//! Dedicated source-clock palette execution. Controller owns preparation and
//! JSON; the render owner only accepts fixed blocks and prepared ownership.
use super::{MediaDescriptor, Result, fx_aux_owner as wire};
use crate::{
    host::fx_palette::{Owner, PreparedToken, Preparer, Settings, Status},
    show::Counter,
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(crate) struct Block {
    pub frame: u64,
    pub samples: [f64; 384],
}
#[derive(Clone, Copy, Default)]
struct Snapshot {
    next: Option<u64>,
    status: [Status; 4],
    generation: [u64; 4],
    applied: [u64; 4],
    ticket: u64,
    committed_ticket: u64,
    outcome: u8, // 0 pending/none, 1 applied, 2 refused
    refusal: u8, // numeric witness; formatted only by controller
    refusal_native_code: i32,
    resets: u64,
    processed: u64,
    output_drops: u64,
    first_output_drop: Option<u64>,
    last_output_drop: Option<u64>,
    first_source_gap: Option<(u64, u64)>,
    observation_drops: u64,
    closed_blocks: u64,
    unprocessed_send: usize,
    unconsumed_wet: usize,
}
struct Commit {
    ticket: u64,
    bus: usize,
    frame: u64,
    generation: u64,
    revision: u64,
    deadline: Instant,
    closure: u64,
    panic: bool,
    settings: Settings,
    token: Option<PreparedToken>,
}
struct Shared {
    stop: AtomicBool,
    closure: Arc<AtomicU64>,
    permit: AtomicU64,
    cancel: AtomicU64,
    acknowledged: AtomicU64,
}
impl Shared {
    fn close(&self) {
        // This owner never rearms in place. A single store is bounded, cannot
        // wrap, and concurrent closers cannot lose the terminal closure.
        self.closure.store(u64::MAX, Ordering::Release);
    }
}
struct Core {
    owners: [Owner; 4],
    settings: [Settings; 4],
    requests: [Option<u64>; 4],
    state: Snapshot,
    pending: Option<Commit>,
    commits: Consumer<Commit>,
    retired: Producer<Commit>,
    tokens: Producer<PreparedToken>,
    input: Consumer<Block>,
    output: Producer<Block>,
    observations: Producer<Snapshot>,
    shared: Arc<Shared>,
    closure: u64,
    deadline: Instant,
    policy: Option<super::policy::PolicyFence>,
    stranded_token: Option<PreparedToken>,
    was_closed: bool,
    transition_ticket: Option<usize>,
    observation_pending: bool,
}
impl Core {
    fn quantum(&mut self, block: Block) {
        // A consumed commit always has one reserved retirement slot. There is
        // only one producer, and pending occupies that reservation until moved.
        if self.pending.is_none() && self.retired.slots() > 0 {
            self.pending = self.commits.pop().ok();
        }
        let closure = self.shared.closure.load(Ordering::Acquire);
        let gap = self.state.next.is_some_and(|next| next != block.frame);
        if gap {
            self.state
                .first_source_gap
                .get_or_insert((self.state.next.unwrap(), block.frame));
        }
        let valid = block.frame.checked_add(48).is_some()
            && block
                .samples
                .iter()
                .all(|s| s.is_finite() && s.abs() <= 16.);
        let closed = closure != self.closure
            || Instant::now() >= self.deadline
            || self.policy.as_ref().is_some_and(|policy| !policy.valid());
        if gap || (closed && !self.was_closed) || !valid {
            self.transition_ticket = None;
            if self.state.ticket != 0
                && self.shared.acknowledged.load(Ordering::Acquire) != self.state.ticket
            {
                self.state.outcome = 2;
                self.state.refusal_native_code = 0;
                self.state.refusal = if gap {
                    1
                } else if closed {
                    2
                } else {
                    3
                };
            }
            for (i, owner) in self.owners.iter_mut().enumerate() {
                if owner.reset_bounded().is_err() {
                    self.shared.close();
                }
                self.requests[i] = None;
                self.state.generation[i] = self.state.generation[i].saturating_add(1);
            }
            self.state.resets = self.state.resets.saturating_add(1);
        }
        self.was_closed = closed;
        if let Some(p) = &self.pending {
            let refuse = gap
                || closed
                || !valid
                || p.closure != closure
                || p.generation != self.state.generation[p.bus]
                || Instant::now() >= p.deadline
                || block.frame > p.frame
                || self.shared.cancel.load(Ordering::Acquire) == p.ticket;
            if refuse || block.frame == p.frame {
                let mut p = self.pending.take().unwrap();
                let permitted = self.shared.permit.load(Ordering::Acquire) == p.ticket;
                let result = if refuse || !permitted {
                    Err(-1)
                } else if p.panic {
                    self.owners[p.bus].reset_bounded().map_err(|_| -1)
                } else if let Some(token) = &mut p.token {
                    let request = self.state.status[p.bus].accepted_request.checked_add(1);
                    match request {
                        Some(request) => self.owners[p.bus]
                            .publish_token(token, request, p.revision)
                            .map(|()| self.requests[p.bus] = Some(request)),
                        None => Err(-1),
                    }
                } else {
                    Ok(())
                };
                self.state.refusal = if gap {
                    1
                } else if closed {
                    2
                } else if !valid {
                    3
                } else if p.closure != closure {
                    4
                } else if p.generation != self.state.generation[p.bus] {
                    5
                } else if Instant::now() >= p.deadline {
                    6
                } else if block.frame > p.frame {
                    7
                } else if self.shared.cancel.load(Ordering::Acquire) == p.ticket {
                    8
                } else if !permitted {
                    9
                } else if result.is_err() {
                    10
                } else {
                    0
                };
                self.state.refusal_native_code = if self.state.refusal == 10 {
                    result.err().unwrap_or(0)
                } else {
                    0
                };
                self.state.ticket = p.ticket;
                self.state.outcome = if result.is_ok() { 1 } else { 2 };
                if result.is_ok() {
                    self.state.committed_ticket = p.ticket;
                    self.state.generation[p.bus] += 1;
                    self.transition_ticket = Some(p.bus);
                    if p.panic {
                        if let Ok(status) = self.owners[p.bus].status_bounded() {
                            self.settings[p.bus] = status.current;
                            self.requests[p.bus] = None;
                        }
                    } else {
                        self.settings[p.bus] = p.settings;
                    }
                }
                // Capacity was reserved before consuming this commit.
                if let Err(rtrb::PushError::Full(value)) = self.retired.push(p) {
                    // Unreachable with the sole producer/reservation invariant;
                    // retain ownership even if that invariant is broken.
                    self.pending = Some(value);
                    self.shared.close();
                }
            }
        }
        let mut wet = Block {
            frame: block.frame,
            samples: [0.; 384],
        };
        if valid && !gap && !closed {
            for (bus, owner) in self.owners.iter_mut().enumerate() {
                let mut input = [0.; 96];
                let mut output = [0.; 96];
                for f in 0..48 {
                    input[f * 2..f * 2 + 2]
                        .copy_from_slice(&block.samples[f * 8 + bus * 2..f * 8 + bus * 2 + 2]);
                }
                let result = owner.process(&input, &mut output);
                if result != 0 {
                    self.shared.close();
                    self.state.outcome = 2;
                    self.state.refusal = 11;
                    self.state.refusal_native_code = result;
                }
                for f in 0..48 {
                    wet.samples[f * 8 + bus * 2..f * 8 + bus * 2 + 2]
                        .copy_from_slice(&output[f * 2..f * 2 + 2]);
                }
            }
            self.state.processed = self.state.processed.saturating_add(1);
        } else if closed {
            self.state.closed_blocks = self.state.closed_blocks.saturating_add(1);
        }
        self.state.next = block.frame.checked_add(48);
        for (i, owner) in self.owners.iter_mut().enumerate() {
            if let Ok(status) = owner.status_bounded() {
                self.state.status[i] = status;
                if status.fault != 0 && self.transition_ticket == Some(i) {
                    self.state.outcome = 2;
                    self.state.refusal = 12;
                    self.state.refusal_native_code = status.fault as i32;
                }
                if gap || closed {
                    self.settings[i] = status.current;
                }
                if status.transition == 0
                    && self.requests[i].is_none_or(|r| r == status.applied_request)
                    && status.current == self.settings[i]
                {
                    self.state.applied[i] = self.state.generation[i];
                    if self.transition_ticket == Some(i) {
                        self.transition_ticket = None;
                    }
                }
            } else {
                self.shared.close();
            }
            if self.stranded_token.is_none()
                && self.tokens.slots() > 0
                && let Some(token) = owner.retired_token()
            {
                // Sole producer, checked free slot. Never free an object here.
                if let Err(rtrb::PushError::Full(value)) = self.tokens.push(token) {
                    self.stranded_token = Some(value);
                    self.shared.close();
                }
            }
        }
        if self.output.push(wet).is_err() {
            self.state.output_drops = self.state.output_drops.saturating_add(1);
            self.state.first_output_drop.get_or_insert(block.frame);
            self.state.last_output_drop = Some(block.frame);
        }
        self.observation_pending = true;
        if self.observations.push(self.state).is_err() {
            self.state.observation_drops = self.state.observation_drops.saturating_add(1);
        } else {
            self.observation_pending = false;
        }
    }
    fn idle(&mut self) {
        if self.pending.is_none() && self.retired.slots() > 0 {
            self.pending = self.commits.pop().ok();
        }
        // Capture the cause once. Re-evaluating after taking the token could
        // report a different cause, and leaving the previous witness would
        // misattribute this ticket to an unrelated earlier transaction.
        let refusal = self.pending.as_ref().map_or(0, |p| {
            if self.shared.closure.load(Ordering::Acquire) != self.closure
                || self.policy.as_ref().is_some_and(|policy| !policy.valid())
            {
                2
            } else if self.shared.cancel.load(Ordering::Acquire) == p.ticket {
                8
            } else if Instant::now() >= p.deadline {
                6
            } else {
                0
            }
        });
        if refusal != 0 {
            let p = self.pending.take().unwrap();
            self.state.ticket = p.ticket;
            self.state.outcome = 2;
            self.state.refusal = refusal;
            self.state.refusal_native_code = 0;
            if let Err(rtrb::PushError::Full(p)) = self.retired.push(p) {
                self.pending = Some(p);
                self.shared.close();
            }
            self.observation_pending = true;
        }
        // Recover the latest completion even when its final source block filled
        // the observation lane and no further media arrives.
        if self.observation_pending
            && self.observations.slots() > 0
            && self.observations.push(self.state).is_ok()
        {
            self.observation_pending = false;
        }
    }
}

struct Pending {
    ticket: u64,
    bus: usize,
    frame: u64,
    config: Option<wire::Configuration>,
    permitted: bool,
}
/// Controller endpoint. Drop joins first, then destroys owners and all queued
/// storage here; stopping never runs native destruction on the audio thread.
pub(crate) struct Execution {
    descriptor: MediaDescriptor,
    hash: String,
    configs: [wire::Configuration; 4],
    preparers: [Preparer; 4],
    template: wire::Observation,
    snapshot: Snapshot,
    pending: Option<Pending>,
    last_ticket: u64,
    commits: Producer<Commit>,
    retired: Consumer<Commit>,
    tokens: Consumer<PreparedToken>,
    lanes: Option<MediaLanes>,
    observations: Consumer<Snapshot>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<Core>>,
    _file: std::fs::File,
}
pub(crate) struct MediaLanes {
    pub input: Producer<Block>,
    pub output: Consumer<Block>,
    pub wake: std::thread::Thread,
    pub(crate) closure: Arc<AtomicU64>,
}
impl MediaLanes {
    pub(crate) fn close(&self) {
        self.closure.store(u64::MAX, Ordering::Release);
    }
}
/// Media-side adapter; native owners retain their own fixed or topology-sized
/// sample storage. Returning output also returns its reserved input credit.
pub(crate) trait ExecutionLanes: Send + 'static {
    type Output;
    fn capacity(&self) -> usize;
    fn submit(&mut self, frame: u64, samples: &[f64]) -> bool;
    fn output(&mut self) -> Option<Self::Output>;
    fn data(block: &Self::Output) -> (u64, &[f64]);
    fn recycle(&mut self, block: Self::Output);
    fn close(&self);
}
impl ExecutionLanes for MediaLanes {
    type Output = Block;
    fn capacity(&self) -> usize {
        self.input
            .buffer()
            .capacity()
            .min(self.output.buffer().capacity())
    }
    fn submit(&mut self, frame: u64, input: &[f64]) -> bool {
        let mut samples = [0.; 384];
        if input.len() > samples.len() {
            return false;
        }
        samples[..input.len()].copy_from_slice(input);
        let accepted = self.input.push(Block { frame, samples }).is_ok();
        self.wake.unpark();
        accepted
    }
    fn output(&mut self) -> Option<Block> {
        self.output.pop().ok()
    }
    fn data(block: &Block) -> (u64, &[f64]) {
        (block.frame, &block.samples)
    }
    fn recycle(&mut self, _: Block) {}
    fn close(&self) {
        MediaLanes::close(self);
    }
}
// Reserve a wet-output slot before accepting a complete native input. Counting
// the entire pipeline (queued input + executing + queued output) prevents either
// four-block lane from overflowing when one of the two threads runs in bursts.
// Credit returns only after media consumes output, not when native consumes input.
struct NativeWindow {
    outstanding: usize,
    capacity: usize,
}
impl NativeWindow {
    #[cfg(test)]
    fn new(lanes: &MediaLanes) -> Self {
        Self {
            outstanding: 0,
            capacity: lanes
                .input
                .buffer()
                .capacity()
                .min(lanes.output.buffer().capacity()),
        }
    }
    fn available(&self) -> bool {
        self.outstanding < self.capacity
    }
    fn submitted(&mut self) {
        assert!(self.available());
        self.outstanding += 1;
    }
    fn returned(&mut self) {
        assert!(self.outstanding > 0);
        self.outstanding -= 1;
    }
}
// Construct/touch every ring entry before either execution thread is ready.
// Temporary ownership is released here, on the preparing controller.
pub(crate) fn prepared_ring<T>(
    capacity: usize,
    mut empty: impl FnMut() -> T,
) -> (Producer<T>, Consumer<T>) {
    let (mut tx, mut rx) = RingBuffer::new(capacity);
    for _ in 0..capacity {
        assert!(tx.push(empty()).is_ok());
    }
    for _ in 0..capacity {
        drop(rx.pop().ok());
    }
    (tx, rx)
}
impl Execution {
    pub(crate) fn from_owner(
        mut original: wire::BrainAux,
        deadline: Instant,
        policy: Option<super::policy::PolicyFence>,
    ) -> Result<Self> {
        let template = original.observation()?;
        let descriptor = template.descriptor.clone();
        let hash = template.library_sha256.clone();
        let (mut owners, configs, file) = original.into_execution_parts();
        let preparers = owners.each_ref().map(Owner::preparer);
        let mut snapshot = Snapshot::default();
        for (i, owner) in owners.iter_mut().enumerate() {
            snapshot.status[i] = owner.status()?;
        }
        let empty_commit = || Commit {
            ticket: 0,
            bus: 0,
            frame: 0,
            generation: 0,
            revision: 0,
            deadline,
            closure: 0,
            panic: false,
            settings: Settings::default(),
            token: None,
        };
        let (commits, commit_rx) = prepared_ring(16, empty_commit);
        let (retired_tx, retired) = prepared_ring(16, empty_commit);
        let (token_tx, tokens) = prepared_ring(16, || preparers[0].empty_token());
        let (input, input_rx) = prepared_ring(4, || Block {
            frame: 0,
            samples: [0.; 384],
        });
        let (output_tx, output) = prepared_ring(4, || Block {
            frame: 0,
            samples: [0.; 384],
        });
        let (obs_tx, observations) = prepared_ring(128, Snapshot::default);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            closure: Arc::new(AtomicU64::new(1)),
            permit: AtomicU64::new(0),
            cancel: AtomicU64::new(0),
            acknowledged: AtomicU64::new(0),
        });
        let mut core = Core {
            owners,
            settings: configs.each_ref().map(|c| c.settings),
            requests: [None; 4],
            state: snapshot,
            pending: None,
            commits: commit_rx,
            retired: retired_tx,
            tokens: token_tx,
            input: input_rx,
            output: output_tx,
            observations: obs_tx,
            shared: shared.clone(),
            closure: 1,
            deadline,
            policy,
            stranded_token: None,
            was_closed: false,
            transition_ticket: None,
            observation_pending: false,
        };
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("gp-brain-fx".into())
            .spawn(move || {
                let _ = ready_tx.send(());
                while !core.shared.stop.load(Ordering::Acquire) {
                    match core.input.pop() {
                        Ok(block) => {
                            #[cfg(all(test, target_os = "linux"))]
                            super::diagnostics::assert_no_allocations(|| core.quantum(block));
                            #[cfg(not(all(test, target_os = "linux")))]
                            core.quantum(block);
                        }
                        Err(_) => {
                            #[cfg(all(test, target_os = "linux"))]
                            super::diagnostics::assert_no_allocations(|| core.idle());
                            #[cfg(not(all(test, target_os = "linux")))]
                            core.idle();
                            std::thread::park_timeout(Duration::from_micros(250));
                        }
                    }
                }
                core
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|_| "FX owner startup failed")?;
        Ok(Self {
            descriptor,
            hash,
            configs,
            preparers,
            template,
            snapshot,
            pending: None,
            last_ticket: 0,
            commits,
            retired,
            tokens,
            lanes: Some(MediaLanes {
                input,
                output,
                wake: thread.thread().clone(),
                closure: shared.closure.clone(),
            }),
            observations,
            shared,
            thread: Some(thread),
            _file: file,
        })
    }
    pub(crate) fn take_media(&mut self) -> MediaLanes {
        self.lanes.take().expect("sole FX media owner")
    }
    fn refresh(&mut self) {
        for _ in 0..128 {
            let Ok(snapshot) = self.observations.pop() else {
                break;
            };
            if self.thread.is_some() {
                self.snapshot = snapshot;
            }
        }
        for _ in 0..16 {
            if self.retired.pop().is_err() {
                break;
            }
        }
        for _ in 0..16 {
            if self.tokens.pop().is_err() {
                break;
            }
        }
    }
    pub(crate) fn close(&self) {
        self.shared.close();
    }
    pub(crate) fn processed(&self) -> u64 {
        self.snapshot.processed
    }
    pub(crate) fn resets(&self) -> u64 {
        self.snapshot.resets
    }
    pub(crate) fn finish(&mut self) {
        self.close();
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            if let Ok(core) = thread.join() {
                self.snapshot = core.state;
                self.snapshot.unprocessed_send = core.input.slots();
                self.snapshot.unconsumed_wet = 4 - core.output.slots();
                drop(core);
            }
        }
    }
    pub(crate) fn report(&self) -> serde_json::Value {
        serde_json::json!({
            "processed_blocks": self.snapshot.processed,
            "last_ticket": self.snapshot.ticket,
            "last_refusal": self.snapshot.refusal,
            "last_refusal_native_code": self.snapshot.refusal_native_code,
            "output_drops": self.snapshot.output_drops,
            "first_output_drop": self.snapshot.first_output_drop,
            "last_output_drop": self.snapshot.last_output_drop,
            "first_source_gap": self.snapshot.first_source_gap,
            "observation_drops": self.snapshot.observation_drops,
            "closed_blocks": self.snapshot.closed_blocks,
            "unprocessed_send_at_shutdown": self.snapshot.unprocessed_send,
            "unconsumed_wet_at_shutdown": self.snapshot.unconsumed_wet,
            "queue_payload_bytes": 32 * std::mem::size_of::<Commit>()
                + 16 * std::mem::size_of::<PreparedToken>()
                + 8 * std::mem::size_of::<Block>()
                + 128 * std::mem::size_of::<Snapshot>(),
            "resource_exclusions": "ring metadata, controller configurations, native FX storage, assembly, network, stacks and allocator overhead; queue payload bytes are not RSS"
        })
    }
    pub(crate) fn command(&mut self, command: wire::Command) -> Result<Option<wire::Message>> {
        self.refresh();
        match command {
            wire::Command::Prepare {
                ticket,
                descriptor,
                mutation,
                apply_frame,
            } => {
                let result = (|| -> Result<()> {
                    if self.pending.is_some()
                        || descriptor != self.descriptor
                        || mutation.bus >= 4
                        || mutation.library_sha256 != self.hash
                        || mutation.owner_binding.as_deref()
                            != Some(wire::binding(&self.descriptor, &self.hash)?.as_str())
                        || mutation.map_revision != self.descriptor.map_generation
                        || self
                            .snapshot
                            .next
                            .is_none_or(|n| apply_frame.0 <= n || apply_frame.0 - n > 48000)
                        || !apply_frame.0.is_multiple_of(48)
                        || ticket.0 <= self.last_ticket
                    {
                        return Err("FX aux stale/busy preparation".into());
                    }
                    let bus = mutation.bus;
                    let status = self.snapshot.status[bus];
                    if mutation.expected_generation.0 != self.snapshot.generation[bus]
                        || self.snapshot.generation[bus] == u64::MAX
                        || (!mutation.panic
                            && (self.snapshot.generation[bus] != self.snapshot.applied[bus]
                                || status.transition != 0))
                        || mutation.panic == mutation.configuration_json.is_some()
                    {
                        return Err("FX aux generation/transition".into());
                    }
                    let config: Option<wire::Configuration> = mutation
                        .configuration_json
                        .as_ref()
                        .map(|s| serde_json::from_str(s).map_err(|e| e.to_string()))
                        .transpose()?;
                    if let Some(c) = &config {
                        if c.sends.len() != self.configs[bus].sends.len()
                            || c.sends
                                .iter()
                                .any(|v| !v.is_finite() || !(0.0..=1.).contains(v))
                        {
                            return Err("FX aux send bounds".into());
                        }
                        // Tempo resolution remains a controller-only library call.
                        if let Some(t) = &c.tempo
                            && (!matches!(c.settings.algorithm, 1 | 8)
                                || (self.preparers[bus].delay_time(
                                    t.bpm,
                                    t.denominator,
                                    t.modifier,
                                )? - c.settings.time_ms)
                                    .abs()
                                    > 1e-9)
                        {
                            return Err("FX aux tempo/time".into());
                        }
                    }
                    let settings = config.as_ref().map_or(status.current, |c| c.settings);
                    // Validate every settings request, including a sends-only edit.
                    let prepared = config
                        .as_ref()
                        .map(|c| self.preparers[bus].prepare(&c.settings))
                        .transpose()?;
                    let token = if settings != self.configs[bus].settings {
                        prepared
                    } else {
                        None
                    };
                    let commit = Commit {
                        ticket: ticket.0,
                        bus,
                        frame: apply_frame.0,
                        generation: self.snapshot.generation[bus],
                        revision: status.revision,
                        deadline: Instant::now() + Duration::from_millis(1500),
                        closure: self.shared.closure.load(Ordering::Acquire),
                        panic: mutation.panic,
                        settings,
                        token,
                    };
                    self.shared.permit.store(0, Ordering::Release);
                    self.shared.cancel.store(0, Ordering::Release);
                    self.commits
                        .push(commit)
                        .map_err(|_| "FX aux commit capacity")?;
                    self.last_ticket = ticket.0;
                    self.pending = Some(Pending {
                        ticket: ticket.0,
                        bus,
                        frame: apply_frame.0,
                        config,
                        permitted: false,
                    });
                    Ok(())
                })();
                Ok(Some(match result {
                    Ok(()) => wire::Message::Prepared {
                        ticket,
                        mutation,
                        apply_frame,
                    },
                    Err(reason) => wire::Message::Refused { ticket, reason },
                }))
            }
            wire::Command::Permit {
                ticket,
                descriptor,
                apply_frame,
            } => {
                let p = self
                    .pending
                    .as_mut()
                    .ok_or("FX aux permit absent preparation")?;
                if p.ticket != ticket.0
                    || descriptor != self.descriptor
                    || p.frame != apply_frame.0
                    || p.permitted
                    || self.snapshot.next.is_some_and(|n| n > p.frame)
                {
                    return Err("FX aux late/stale permit".into());
                }
                p.permitted = true;
                self.shared.permit.store(ticket.0, Ordering::Release);
                Ok(None)
            }
            wire::Command::Cancel { ticket } => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.ticket == ticket.0 && !p.permitted)
                {
                    self.shared.cancel.store(ticket.0, Ordering::Release);
                }
                Ok(None)
            }
        }
    }
    pub(crate) fn poll(&mut self) -> Result<Option<wire::Message>> {
        self.refresh();
        let mut completed = None;
        if let Some(p) = &mut self.pending
            && self.snapshot.ticket == p.ticket
            && self.snapshot.outcome != 0
        {
            if self.snapshot.committed_ticket == p.ticket
                && let Some(config) = p.config.take()
            {
                self.configs[p.bus] = config;
            }
            if self.snapshot.outcome == 2 {
                let ticket = Counter(p.ticket);
                self.shared.acknowledged.store(p.ticket, Ordering::Release);
                self.pending = None;
                return Ok(Some(wire::Message::Refused {
                    ticket,
                    reason: match self.snapshot.refusal {
                        1 => "FX aux source gap",
                        2 => "FX aux owner closed",
                        3 => "FX aux invalid source block",
                        4 => "FX aux closure changed",
                        5 => "FX aux generation changed",
                        6 => "FX aux commit expired",
                        7 => "FX aux commit frame missed",
                        8 => "FX aux commit cancelled",
                        9 => "FX aux permit missing",
                        10 => "FX aux native publication refused",
                        11 => "FX aux native processing failed",
                        12 => "FX aux native transition fault",
                        _ => "FX aux boundary refused",
                    }
                    .into(),
                }));
            }
            if let Some(config) = p.config.take() {
                self.configs[p.bus] = config;
            }
            completed = Some((Counter(p.ticket), Counter(p.frame)));
            if self.snapshot.generation[p.bus] == self.snapshot.applied[p.bus] {
                self.shared.acknowledged.store(p.ticket, Ordering::Release);
                self.pending = None;
            }
        }
        let mut observation = self.template.clone();
        observation.next_frame = Counter(self.snapshot.next.unwrap_or(0));
        for (i, bus) in observation.buses.iter_mut().enumerate() {
            if self.snapshot.status[i].transition == 0
                && self.pending.as_ref().is_none_or(|p| p.bus != i)
            {
                if self.configs[i].settings != self.snapshot.status[i].current {
                    self.configs[i].tempo = None;
                }
                self.configs[i].settings = self.snapshot.status[i].current;
            }
            bus.generation = Counter(self.snapshot.generation[i]);
            bus.applied_generation = Counter(self.snapshot.applied[i]);
            bus.configuration_json =
                serde_json::to_string(&self.configs[i]).map_err(|e| e.to_string())?;
            bus.owner_json =
                serde_json::to_string(&self.snapshot.status[i]).map_err(|e| e.to_string())?;
        }
        Ok(Some(match completed {
            Some((ticket, apply_frame)) => wire::Message::Completed {
                ticket,
                apply_frame,
                observation,
            },
            None => wire::Message::Observe { observation },
        }))
    }
}
impl Drop for Execution {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Clone, Default)]
pub(crate) struct MediaReport {
    pub stats: super::BrainStats,
    pub hash: String,
    pub nonzero: u64,
    pub energy: Vec<f64>,
    pub input_drops: u64,
    pub first_input_drop: Option<u64>,
    pub last_input_drop: Option<u64>,
    pub analysis_drops: u64,
    pub analysis_unconsumed: usize,
    pub report_drops: u64,
    pub assembly_prepared_bytes: usize,
    pub fault: Option<String>,
}
pub(crate) struct MediaWorker {
    analysis: Consumer<crate::transport::MediaDatagram>,
    reports: Consumer<MediaReport>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<MediaReport>>,
    pub report: MediaReport,
}
impl MediaWorker {
    pub(crate) fn start<L: ExecutionLanes>(
        mut channel: super::MediaChannel,
        mut lanes: L,
        mut receiver: super::GroupedReceiver,
        trace: super::diagnostics::Handle,
    ) -> Result<Self> {
        use sha2::{Digest, Sha256};
        let descriptor = channel.descriptor().clone();
        let channels = descriptor.channels(super::MediaRole::FxSend);
        let samples_per_block = receiver.sample_count();
        if !(1..=256).contains(&channels)
            || ![channels * 48, channels * 96].contains(&samples_per_block)
        {
            return Err("FX execution media shape".into());
        }
        let (mut analysis_tx, analysis) = RingBuffer::new(32);
        let (mut report_tx, reports) = RingBuffer::new(2);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new().name("gp-fx-media".into()).spawn(move || {
            let mut report = MediaReport { energy: vec![0.; channels.div_ceil(2)], assembly_prepared_bytes: receiver.prepared_storage().total_bytes, ..MediaReport::default() };
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => { report.fault = Some(error.to_string()); lanes.close(); return report; }
            };
            let mut hash = Sha256::new();
            let mut window = NativeWindow { outstanding: 0, capacity: lanes.capacity() };
            let mut samples = vec![0.; samples_per_block];
            runtime.block_on(async {
                let mut timer = tokio::time::interval(Duration::from_micros(250));
                timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut last_report = Instant::now();
                while !stopping.load(Ordering::Acquire) {
                    tokio::select! {
                        _ = timer.tick() => {
                            if let Err(error) = channel.check_current() { report.fault = Some(error); break; }
                            for _ in 0..4 {
                                let Some(block) = lanes.output() else { break; };
                                let (frame, output) = L::data(&block);
                                for row in output[..samples_per_block].chunks_exact(channels) {
                                    for (bus, pair) in row.chunks(2).enumerate() { report.energy[bus] += pair.iter().map(|s| s*s).sum::<f64>(); }
                                }
                                let packets = match super::encode_grouped(&descriptor, super::MediaRole::WetReturn, frame, &output[..samples_per_block]) {
                                    Ok(packets) => packets,
                                    Err(error) => { report.fault = Some(error); break; }
                                };
                                lanes.recycle(block);
                                window.returned();
                                for bytes in packets {
                                    if let Ok(packet) = crate::transport::Packet::parse(&bytes) {
                                        for i in 0..packet.spec().samples() {
                                            report.nonzero += u64::from(packet.sample(i).is_ok_and(|s| s != 0.));
                                        }
                                        super::diagnostics::media_record(&trace, super::diagnostics::Stage::BrainSend, packet, 0);
                                    }
                                    hash.update(&bytes);
                                    if let Err(error) = channel.send(bytes) { report.fault = Some(error); break; }
                                    report.stats.returned_packets = report.stats.returned_packets.saturating_add(1);
                                }
                                report.stats.processed_blocks = report.stats.processed_blocks.saturating_add(1);
                            }
                            if last_report.elapsed() >= Duration::from_millis(25) {
                                if report_tx.slots() > 0 {
                                    report.hash = crate::hash::hex(hash.clone().finalize());
                                    let _ = report_tx.push(report.clone());
                                } else { report.report_drops = report.report_drops.saturating_add(1); }
                                last_report = Instant::now();
                            }
                            if report.fault.is_some() { break; }
                        }
                        received = channel.receive(), if window.available() => {
                            let bytes = match received { Ok(bytes) => bytes, Err(error) => { report.fault = Some(error); break; } };
                            let packet = match crate::transport::Packet::parse(&bytes) {
                                Ok(packet) => packet,
                                Err(_) => { report.stats.stale_groups += 1; continue; }
                            };
                            super::diagnostics::media_record(&trace, super::diagnostics::Stage::BrainReceive, packet, 0);
                            if packet.spec().role == crate::transport::Role::Analysis {
                                report.stats.analysis_packets += 1;
                                if let Ok(packet) = crate::transport::MediaDatagram::copy_from(&bytes)
                                    && analysis_tx.push(packet).is_err() { report.analysis_drops += 1; }
                                continue;
                            }
                            report.stats.send_packets += 1;
                            match receiver.push_into(packet, &mut samples[..samples_per_block]) {
                                Ok(Some(frame)) => {
                                    if !lanes.submit(frame, &samples) {
                                        report.input_drops += 1;
                                        report.first_input_drop.get_or_insert(frame);
                                        report.last_input_drop = Some(frame);
                                    } else {
                                        window.submitted();
                                    }
                                }
                                Ok(None) => (),
                                Err(_) => report.stats.stale_groups += 1,
                            }
                            report.stats.incomplete_blocks = receiver.incomplete_dropped;
                        }
                    }
                }
            });
            lanes.close();
            report.hash = crate::hash::hex(hash.finalize());
            report.analysis_unconsumed = 32 - analysis_tx.slots();
            report
        }).map_err(|e| e.to_string())?;
        Ok(Self {
            analysis,
            reports,
            stop,
            thread: Some(thread),
            report: MediaReport::default(),
        })
    }
    pub(crate) fn poll(&mut self) -> Result<Option<Vec<u8>>> {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
        {
            self.finish();
        }
        for _ in 0..2 {
            if let Ok(report) = self.reports.pop()
                && self.thread.is_some()
            {
                self.report = report;
            }
        }
        if let Some(fault) = &self.report.fault {
            return Err(fault.clone());
        }
        Ok(self
            .analysis
            .pop()
            .ok()
            .map(|packet| packet.bytes().to_vec()))
    }
    pub(crate) fn finish(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            match thread.join() {
                Ok(report) => self.report = report,
                Err(_) => self.report.fault = Some("FX media worker panicked".into()),
            }
        }
    }
}
impl Drop for MediaWorker {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::remote::{MediaRole, grouped_streams};
    use sha2::{Digest, Sha256};
    fn setup() -> (Execution, MediaLanes) {
        setup_fenced(None)
    }
    fn setup_fenced(policy: Option<super::super::policy::PolicyFence>) -> (Execution, MediaLanes) {
        let path = std::path::PathBuf::from(
            std::env::var_os("GP_FX_LIBRARY").expect("verified FX library"),
        );
        let hash = crate::hash::hex(Sha256::digest(std::fs::read(&path).unwrap()));
        let mut streams =
            grouped_streams(MediaRole::FxSend, &wire::channel_ids(), 48, 0, 1, 1232).unwrap();
        streams.extend(
            grouped_streams(
                MediaRole::WetReturn,
                &wire::channel_ids(),
                48,
                768,
                10,
                1232,
            )
            .unwrap(),
        );
        let descriptor = MediaDescriptor {
            session: Counter(1),
            source_epoch: Counter(2),
            capability_generation: Counter(3),
            map_generation: Counter(4),
            sample_rate: 48000,
            streams,
        };
        let owner = wire::BrainAux::load(descriptor, &path, &hash, 16).unwrap();
        let mut execution =
            Execution::from_owner(owner, Instant::now() + Duration::from_secs(60), policy).unwrap();
        let lanes = execution.take_media();
        (execution, lanes)
    }
    fn block(lanes: &mut MediaLanes, frame: u64) -> Block {
        assert!(
            lanes
                .input
                .push(Block {
                    frame,
                    samples: [0.25; 384]
                })
                .is_ok()
        );
        lanes.wake.unpark();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(block) = lanes.output.pop() {
                return block;
            }
            assert!(Instant::now() < deadline, "FX output timeout");
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    #[test]
    fn media_native_window_preserves_bursts_across_both_scheduling_boundaries() {
        fn run(reserve: bool) -> (usize, usize, Option<(u64, u64)>, Vec<u64>) {
            let (input, mut native_input) = RingBuffer::new(4);
            let (mut native_output, output) = RingBuffer::new(4);
            let mut lanes = MediaLanes {
                input,
                output,
                wake: std::thread::current(),
                closure: Arc::new(AtomicU64::new(1)),
            };
            let mut window = NativeWindow::new(&lanes);
            let (mut sent, mut input_drops, mut output_drops) = (0, 0, 0);
            let (mut next, mut gap) = (None, None);
            let mut received = Vec::new();
            // Withhold native execution during an eight-block receive burst,
            // then withhold media output service across another receive burst.
            // No clocks, sleeps or scheduler luck are involved.
            for turn in 0..8 {
                for _ in 0..if turn == 0 { 8 } else { 4 } {
                    if sent == 12 || (reserve && !window.available()) {
                        break;
                    }
                    let block = Block {
                        frame: sent * 48,
                        samples: [0.25; 384],
                    };
                    sent += 1;
                    if lanes.input.push(block).is_err() {
                        input_drops += 1;
                    } else if reserve {
                        window.submitted();
                    }
                }
                while let Ok(block) = native_input.pop() {
                    if let Some(expected) = next
                        && expected != block.frame
                    {
                        gap.get_or_insert((expected, block.frame));
                    }
                    next = Some(block.frame + 48);
                    if native_output.push(block).is_err() {
                        output_drops += 1;
                    }
                }
                if turn > 0 {
                    while let Ok(block) = lanes.output.pop() {
                        received.push(block.frame);
                        if reserve {
                            window.returned();
                        }
                    }
                }
            }
            (input_drops, output_drops, gap, received)
        }
        // Negative control reproduces the retained four-input/four-output-drop
        // signature and exact source gap using the former unreserved admission.
        let old = run(false);
        assert_eq!((old.0, old.1, old.2), (4, 4, Some((192, 384))));
        let fixed = run(true);
        assert_eq!((fixed.0, fixed.1, fixed.2), (0, 0, None));
        assert_eq!(fixed.3, (0..12).map(|i| i * 48).collect::<Vec<_>>());
    }

    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_owner_reserved_window_preserves_continuity_without_output_service() {
        let (mut execution, mut lanes) = setup();
        let mut window = NativeWindow::new(&lanes);
        for batch in 0..3 {
            for index in 0..4 {
                assert!(window.available());
                assert!(
                    lanes
                        .input
                        .push(Block {
                            frame: (batch * 4 + index) * 48,
                            samples: [0.25; 384],
                        })
                        .is_ok()
                );
                window.submitted();
            }
            lanes.wake.unpark();
            let until = Instant::now() + Duration::from_secs(2);
            loop {
                execution.refresh();
                if execution.snapshot.next == Some((batch + 1) * 4 * 48) {
                    break;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_micros(100));
            }
            // Even after native frees every input slot, receiving must stay
            // disabled until output is consumed. This is the missing reservation.
            assert_eq!(lanes.input.slots(), 4);
            assert!(!window.available());
            assert_eq!(execution.snapshot.output_drops, 0);
            for index in 0..4 {
                let block = lanes.output.pop().unwrap();
                assert_eq!(block.frame, (batch * 4 + index) * 48);
                window.returned();
            }
        }
        execution.finish();
        assert_eq!(execution.snapshot.processed, 12);
        assert_eq!(execution.snapshot.first_source_gap, None);
        assert_eq!(execution.snapshot.output_drops, 0);
    }

    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_owner_gap_and_output_pressure_keep_exact_frame_witnesses() {
        let (mut execution, mut lanes) = setup();
        for index in 0..8 {
            let frame = (index + u64::from(index >= 4)) * 48;
            assert!(
                lanes
                    .input
                    .push(Block {
                        frame,
                        samples: [0.25; 384]
                    })
                    .is_ok()
            );
            lanes.wake.unpark();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                execution.refresh();
                if execution.snapshot.next == Some(frame + 48) {
                    break;
                }
                assert!(Instant::now() < deadline, "native witness timeout");
                std::thread::sleep(Duration::from_micros(100));
            }
        }
        // Do not drain native output: four retained blocks, then exactly four
        // drops. The deliberate missing source block has an independent witness.
        execution.finish();
        assert_eq!(execution.snapshot.first_source_gap, Some((192, 240)));
        assert_eq!(execution.snapshot.output_drops, 4);
        assert_eq!(execution.snapshot.first_output_drop, Some(240));
        assert_eq!(execution.snapshot.last_output_drop, Some(384));
        assert_eq!(execution.snapshot.unconsumed_wet, 4);
    }

    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_owner_idle_refusal_retains_cause_without_source_or_controller_service() {
        for (cause, reason) in [
            (6, "FX aux commit expired"),
            (8, "FX aux commit cancelled"),
            (2, "FX aux owner closed"),
        ] {
            let (mut execution, _lanes) = setup();
            // No source blocks: refusal must be observed from idle execution,
            // without a later quantum supplying a witness accidentally.
            let ticket = 91;
            execution.pending = Some(Pending {
                ticket,
                bus: 0,
                frame: 48,
                config: None,
                permitted: false,
            });
            if cause == 8 {
                execution.shared.cancel.store(ticket, Ordering::Release);
            } else if cause == 2 {
                execution.close();
            }
            assert!(
                execution
                    .commits
                    .push(Commit {
                        ticket,
                        bus: 0,
                        frame: 48,
                        generation: 0,
                        revision: 0,
                        deadline: if cause == 6 {
                            Instant::now() - Duration::from_secs(1)
                        } else {
                            Instant::now() + Duration::from_secs(60)
                        },
                        closure: 1,
                        panic: false,
                        settings: execution.configs[0].settings,
                        token: None,
                    })
                    .is_ok()
            );
            execution.thread.as_ref().unwrap().thread().unpark();
            let deadline = Instant::now() + Duration::from_secs(2);
            // Wait on the observation lane only; no controller refresh or
            // retirement drain participates in generating the completion.
            loop {
                if let Ok(snapshot) = execution.observations.pop()
                    && snapshot.ticket == ticket
                {
                    assert_eq!(snapshot.outcome, 2);
                    assert_eq!(snapshot.refusal, cause);
                    assert_eq!(snapshot.refusal_native_code, 0);
                    execution.snapshot = snapshot;
                    break;
                }
                assert!(Instant::now() < deadline, "idle refusal timeout");
                std::thread::sleep(Duration::from_micros(100));
            }
            assert!(matches!(execution.poll().unwrap(),
                Some(wire::Message::Refused { ticket: Counter(91), reason: actual })
                    if actual == reason));
        }
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_owner_policy_revocation_does_not_need_controller_or_media_poll() {
        let certificate = b"FX execution policy fixture";
        let policy = crate::remote::PolicyStore::new(vec![crate::remote::Peer {
            id: "brain".into(),
            certificate_sha256: crate::remote::fingerprint(certificate),
            permissions: [crate::remote::Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(certificate, 1).unwrap();
        let fence = context.audio_fence();
        let (mut execution, mut lanes) = setup_fenced(Some(fence.clone()));
        let mut energy = 0.;
        for n in 0..64 {
            energy += block(&mut lanes, n * 48)
                .samples
                .iter()
                .map(|x| x * x)
                .sum::<f64>();
        }
        assert!(energy > 0.);
        policy.replace(vec![]).unwrap();
        assert!(!fence.valid());
        // No call into Execution, no media runtime, no controller queue drain.
        // The actual native render owner observes the published policy fence.
        assert!(block(&mut lanes, 64 * 48).samples.iter().all(|s| *s == 0.));
        execution.refresh();
        assert_eq!(execution.snapshot.closed_blocks, 1);
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_owner_proxy_disconnect_closes_without_controller_service() {
        use crate::remote::AuthorityEndpoint;
        let certificate = b"FX disconnect fixture";
        let policy = crate::remote::PolicyStore::new(vec![crate::remote::Peer {
            id: "brain".into(),
            certificate_sha256: crate::remote::fingerprint(certificate),
            permissions: [crate::remote::Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(certificate, 1).unwrap();
        let (mut execution, mut lanes) = setup_fenced(Some(context.audio_fence()));
        let (mut proxy, _mailbox) = crate::remote::proxy::authority_channel(
            &context,
            crate::remote::EngineIdentity {
                source_epoch: Counter(2),
                capability_generation: Counter(3),
                map_generation: Counter(4),
            },
        )
        .unwrap();
        let mut energy = 0.;
        for n in 0..64 {
            energy += block(&mut lanes, n * 48)
                .samples
                .iter()
                .map(|s| s * s)
                .sum::<f64>();
        }
        assert!(energy > 0.);
        for _ in 0..16 {
            proxy.dispatch(&context, serde_json::json!({}), 0).unwrap();
        }
        proxy.disconnect(&context);
        // The actual owner sees terminal closure despite saturated command ingress
        // and no controller or mailbox service. No new session rearms this owner.
        assert!(block(&mut lanes, 64 * 48).samples.iter().all(|s| *s == 0.));
        let fresh = policy.authenticate(certificate, 2).unwrap();
        assert!(fresh.audio_fence().valid());
        assert!(block(&mut lanes, 65 * 48).samples.iter().all(|s| *s == 0.));
        execution.refresh();
        assert_eq!(execution.snapshot.closed_blocks, 2);
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_owner_retirement_backpressure_preserves_closure() {
        let (mut execution, mut lanes) = setup();
        block(&mut lanes, 0);
        execution.refresh();
        for ticket in 1..=17 {
            let commit = Commit {
                ticket,
                bus: 0,
                frame: ticket * 48,
                generation: 0,
                revision: execution.snapshot.status[0].revision,
                deadline: Instant::now() + Duration::from_secs(2),
                closure: 1,
                panic: false,
                settings: execution.configs[0].settings,
                token: None,
            };
            assert!(execution.commits.push(commit).is_ok());
            block(&mut lanes, ticket * 48);
        }
        // No retirement consumer has run. Sixteen refused ownership objects
        // occupy the retirement queue; the seventeenth remains unconsumed.
        assert_eq!(execution.retired.slots(), 16);
        assert_eq!(execution.commits.slots(), 15);
        execution.close();
        assert!(block(&mut lanes, 18 * 48).samples.iter().all(|s| *s == 0.));
        assert_eq!(execution.commits.slots(), 15);
        execution.refresh();
        block(&mut lanes, 19 * 48);
        execution.refresh();
        assert_eq!(execution.snapshot.ticket, 17);
        assert_eq!(execution.snapshot.outcome, 2);
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_owner_control_stall_closure_and_retirement() {
        let (mut execution, mut lanes) = setup();
        block(&mut lanes, 0);
        execution.refresh();
        let mut config = execution.configs[0].clone();
        config.settings.time_ms = 2.;
        let mutation = crate::fx_aux_wire::Mutation {
            bus: 0,
            map_revision: execution.descriptor.map_generation,
            library_sha256: execution.hash.clone(),
            expected_generation: Counter(0),
            configuration_json: Some(serde_json::to_string(&config).unwrap()),
            panic: false,
            owner_binding: Some(wire::binding(&execution.descriptor, &execution.hash).unwrap()),
        };
        assert!(matches!(
            execution
                .command(wire::Command::Prepare {
                    ticket: Counter(1),
                    descriptor: execution.descriptor.clone(),
                    mutation,
                    apply_frame: Counter(96)
                })
                .unwrap(),
            Some(wire::Message::Prepared { .. })
        ));
        execution
            .command(wire::Command::Permit {
                ticket: Counter(1),
                descriptor: execution.descriptor.clone(),
                apply_frame: Counter(96),
            })
            .unwrap();
        // No controller polling/preparation/retirement through the whole native
        // transition, followed by saturation of the observation lane.
        let mut energy = 0.;
        for n in 1..=200 {
            let wet = block(&mut lanes, n * 48);
            energy += wet.samples.iter().map(|x| x * x).sum::<f64>();
        }
        assert!(energy > 0.);
        assert!(matches!(
            execution.poll().unwrap(),
            Some(wire::Message::Completed { .. })
        ));
        assert_eq!(execution.configs[0].settings.time_ms, 2.);
        block(&mut lanes, 201 * 48);
        execution.refresh();
        assert!(execution.snapshot.observation_drops > 0);
        assert_eq!(
            execution.snapshot.generation[0],
            execution.snapshot.applied[0]
        );
        // Closure bypasses saturated observations and ordinary commands. It
        // cannot be cleared by a later ordinary edit or continued media.
        execution.close();
        for n in 202..=210 {
            assert!(block(&mut lanes, n * 48).samples.iter().all(|s| *s == 0.));
        }
        execution.refresh();
        assert!(execution.snapshot.closed_blocks >= 9);
    }
}
