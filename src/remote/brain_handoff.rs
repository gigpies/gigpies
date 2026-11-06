//! Network/controller endpoints for the existing fixed source sample lanes.
//! Only these endpoints lock; source publishes numeric authorization atomically.
use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Clone)]
pub(super) struct MonitorConsumer(Arc<Mutex<rtrb::Consumer<MonitorBlock>>>);
impl MonitorConsumer {
    pub(super) fn new(consumer: rtrb::Consumer<MonitorBlock>) -> Self {
        Self(Arc::new(Mutex::new(consumer)))
    }
    pub(super) fn pop(&self) -> std::result::Result<MonitorBlock, rtrb::PopError> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).pop()
    }
}
#[derive(Clone)]
pub(super) struct TalkbackProducer(Arc<Mutex<TalkbackProducerState>>);
struct TalkbackProducerState {
    producer: rtrb::Producer<TalkbackIngress>,
    last: Option<(u64, u64)>,
    closed: bool,
    rejected: u64,
}
impl TalkbackProducer {
    pub(super) fn new(producer: rtrb::Producer<TalkbackIngress>) -> Self {
        Self(Arc::new(Mutex::new(TalkbackProducerState {
            producer,
            last: None,
            closed: false,
            rejected: 0,
        })))
    }
    // Compatibility callers account refusals themselves.
    pub(super) fn push(&self, block: TalkbackIngress) -> std::result::Result<(), ()> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(());
        }
        state.producer.push(block).map_err(|_| ())?;
        state.last = Some((block.generation, block.frame));
        Ok(())
    }
    pub(super) fn last(&self, generation: u64) -> Option<u64> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last
            .filter(|(g, _)| *g == generation)
            .map(|(_, f)| f)
    }
    pub(super) fn close(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
    }
    pub(super) fn take_rejected(&self) -> u64 {
        std::mem::take(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()).rejected)
    }
}
pub(super) struct BrainHandoffState {
    pub(super) hold_generation: u64,
    live: AtomicBool,
    deadline: AtomicU64,
    silent: AtomicBool,
}
impl BrainHandoffState {
    pub(super) fn new(hold_generation: u64) -> Self {
        Self {
            hold_generation,
            live: AtomicBool::new(true),
            deadline: AtomicU64::new(0),
            silent: AtomicBool::new(true),
        }
    }
    pub(super) fn close(&self) {
        self.live.store(false, Ordering::Release);
    }
    pub(super) fn publish(&self, deadline: u64, silent: bool) {
        self.deadline.store(deadline, Ordering::Release);
        self.silent.store(silent, Ordering::Release);
    }
}
/// Prepared authenticated Brain handoff; no authority mailbox service on media.
pub struct SourceBrainMedia {
    context: AuthenticatedContext,
    descriptor: BrainMediaDescriptor,
    generation: u64,
    producer: TalkbackProducer,
    monitor: MonitorConsumer,
    state: Arc<BrainHandoffState>,
}
impl SourceBrainMedia {
    pub(super) fn new(
        context: AuthenticatedContext,
        descriptor: BrainMediaDescriptor,
        generation: u64,
        producer: TalkbackProducer,
        monitor: MonitorConsumer,
        state: Arc<BrainHandoffState>,
    ) -> Self {
        Self {
            context,
            descriptor,
            generation,
            producer,
            monitor,
            state,
        }
    }
    pub(crate) fn submit(
        &mut self,
        context: &AuthenticatedContext,
        bytes: &[u8],
        now: u64,
    ) -> Result<()> {
        let mut producer = self.producer.0.lock().unwrap_or_else(|e| e.into_inner());
        if producer.closed {
            return Err("source ingress stopped".into());
        }
        let result = (|| {
            context.require(&Permission::TalkbackDestinations)?;
            if context != &self.context || !self.state.live.load(Ordering::Acquire) {
                return Err("Brain media owner retired".into());
            }
            let deadline = self.state.deadline.load(Ordering::Acquire);
            if now >= deadline {
                return Err("Brain hold closed".into());
            }
            let packet = BrainPacket::decode(bytes, &self.descriptor, BrainMediaRole::Talkback)?;
            if producer
                .last
                .is_some_and(|(g, f)| g == self.generation && packet.frame <= f)
            {
                return Err("Brain replay/reordered media".into());
            }
            let block = TalkbackIngress {
                generation: self.generation,
                frame: packet.frame,
                deadline_ms: deadline,
                samples: packet
                    .samples
                    .as_slice()
                    .try_into()
                    .map_err(|_| "Brain talkback sample shape")?,
            };
            producer
                .producer
                .push(block)
                .map_err(|_| "Brain talkback ingress full")?;
            producer.last = Some((self.generation, packet.frame));
            Ok(())
        })();
        if result.is_err() {
            producer.rejected = producer.rejected.saturating_add(1);
        }
        result
    }
    pub(crate) fn poll(&mut self, context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        if !self.descriptor.monitor {
            return Ok(None);
        }
        context.require(&Permission::LocalOperatorMonitor)?;
        if context != &self.context || !self.state.live.load(Ordering::Acquire) {
            return Ok(None);
        }
        let Ok(block) = self.monitor.pop() else {
            return Ok(None);
        };
        let silent = self.state.silent.load(Ordering::Acquire);
        BrainPacket {
            descriptor: self.descriptor.clone(),
            role: BrainMediaRole::Monitor,
            frame: block.frame,
            samples: if silent {
                vec![0.; 96]
            } else {
                block.samples.to_vec()
            },
        }
        .encode()
        .map(Some)
    }
}
