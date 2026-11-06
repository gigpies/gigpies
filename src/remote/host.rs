#[path = "source_recovery.rs"]
mod source_recovery;
pub use source_recovery::SourceClockRecovery;
#[path = "brain_handoff.rs"]
mod brain_handoff;
#[path = "source_clock.rs"]
mod source_clock;
pub use brain_handoff::SourceBrainMedia;
use brain_handoff::{BrainHandoffState, MonitorConsumer, TalkbackProducer};
pub use source_clock::{SourceClockReport, SourceClockStatus};
#[path = "fx_aux_relay.rs"]
mod fx_aux_relay;
#[path = "fx_relay.rs"]
mod fx_relay;
// The authenticated endpoint for the existing composed LocalAudio graph. It
// never starts a device or advances time from packet arrival. The source owner
// calls `process_source` with the device/fake-device epoch and first frame.
use super::*;
use crate::brain_audio::bridge::{Bridge, BridgeConfig, BridgeEpochs};
use crate::{
    local_audio::LocalAudio,
    show::Counter,
    transport::{JitterBuffer, Packet, Role, StreamSpec, WetGate},
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
struct DeviceChange {
    fingerprint: String,
    request: crate::control_model::Request,
    config: crate::brain_audio::device::DeviceConfig,
    ticket: u64,
    deadline: u64,
    sent: bool,
}
#[derive(Clone, Copy)]
struct MonitorRetirement {
    first_ms: u64,
    first_frame: u64,
}
struct BrainMediaState {
    context: AuthenticatedContext,
    descriptor: BrainMediaDescriptor,
    outgoing: MonitorConsumer,
    handoff: std::sync::Arc<BrainHandoffState>,
    exported: bool,
    monitor_retirement: Option<MonitorRetirement>,
    last_queued_talkback: Option<u64>,
}
impl Drop for BrainMediaState {
    fn drop(&mut self) {
        self.handoff.close();
    }
}
struct TalkbackSource {
    bridge: Bridge,
    epochs: BridgeEpochs,
    last_received: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct MonitorBlock {
    frame: u64,
    samples: [f64; 96],
}
const MONITOR_BLOCKS: usize = 64;
fn prepare_monitor_lane() -> (rtrb::Producer<MonitorBlock>, rtrb::Consumer<MonitorBlock>) {
    let (mut producer, mut consumer) = rtrb::RingBuffer::new(MONITOR_BLOCKS);
    // Initialize every payload page before handing the producer to rendering.
    for _ in 0..MONITOR_BLOCKS {
        producer
            .push(MonitorBlock {
                frame: 0,
                samples: [0.; 96],
            })
            .unwrap();
    }
    for _ in 0..MONITOR_BLOCKS {
        consumer.pop().unwrap();
    }
    (producer, consumer)
}
// Preserve the existing proxy ingress bounds. Payload-only prepared memory is
// 256 * (size_of::<WetIngress>() + size_of::<WetReceipt>()) +
// 64 * (size_of::<TalkbackIngress>() + size_of::<TalkbackReceipt>()). Rings are
// prefaulted here; these additional queues do not increase the playout window.
const WET_INGRESS_PACKETS: usize = 256;
const TALKBACK_INGRESS_BLOCKS: usize = 64;
#[derive(Clone, Copy, Debug)]
struct WetIngress {
    generation: u64,
    length: usize,
    bytes: [u8; crate::transport::MAX_DATAGRAM],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WetAdmission {
    Accepted,
    Late,
    Rejected,
}
#[derive(Clone, Copy, Debug)]
struct WetReceipt {
    packet: WetIngress,
    result: WetAdmission,
    epoch: u64,
    frame: u64,
}
#[derive(Clone, Copy, Debug)]
struct TalkbackIngress {
    generation: u64,
    frame: u64,
    deadline_ms: u64,
    samples: [f64; 48],
}
#[derive(Clone, Copy, Debug)]
struct TalkbackReceipt {
    accepted: bool,
}
// Only network/controller endpoints lock this producer. Source owns the separate
// consumer and never locks. Closing serializes with submission before final drain.
#[derive(Clone)]
struct WetProducer(std::sync::Arc<std::sync::Mutex<WetProducerState>>);
struct WetProducerState {
    producer: rtrb::Producer<WetIngress>,
    closed: bool,
    rejected: u64,
}
impl WetProducer {
    fn push(&self, block: WetIngress) -> std::result::Result<(), ()> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            // Closed endpoints refuse before admission; shutdown accounting is final.
            return Err(());
        }
        let result = state.producer.push(block).map_err(|_| ());
        if result.is_err() {
            state.rejected = state.rejected.saturating_add(1);
        }
        result
    }
    #[cfg(test)]
    fn slots(&self) -> usize {
        self.0.lock().unwrap().producer.slots()
    }
    #[cfg(test)]
    fn is_full(&self) -> bool {
        self.slots() == 0
    }
}
/// Authenticated prepared ingress. No controller service or source mutex is
/// required to enqueue; the source still validates generation, policy and time.
pub struct SourceWetInput {
    producer: WetProducer,
    context: AuthenticatedContext,
    generation: u64,
    specs: Vec<StreamSpec>,
    source_frame: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl SourceWetInput {
    pub(crate) fn source_frame(&self) -> Option<u64> {
        let frame = self.source_frame.load(std::sync::atomic::Ordering::Acquire);
        (frame != u64::MAX).then_some(frame)
    }
    pub(crate) fn submit(
        &mut self,
        context: &AuthenticatedContext,
        packet: Packet<'_>,
    ) -> Result<()> {
        if context != &self.context {
            return Err("wet owner/direction".into());
        }
        context.require(&Permission::Fx)?;
        if !self.specs.contains(&packet.spec()) {
            return Err("wet stream identity".into());
        }
        let mut block = WetIngress {
            generation: self.generation,
            length: packet.bytes().len(),
            bytes: [0; crate::transport::MAX_DATAGRAM],
        };
        block.bytes[..block.length].copy_from_slice(packet.bytes());
        let mut state = self.producer.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err("source ingress stopped".into());
        }
        if state.producer.push(block).is_err() {
            state.rejected = state.rejected.saturating_add(1);
        }
        Ok(())
    }
}
struct HostMediaIngress {
    wet: WetProducer,
    exported_wet_generation: u64,
    source_frame: std::sync::Arc<std::sync::atomic::AtomicU64>,
    wet_receipts: rtrb::Consumer<WetReceipt>,
    talkback: TalkbackProducer,
    talkback_receipts: rtrb::Consumer<TalkbackReceipt>,
    wet_generation: u64,
    talkback_generation: u64,
}
struct SourceMediaIngress {
    source_frame: std::sync::Arc<std::sync::atomic::AtomicU64>,
    wet: rtrb::Consumer<WetIngress>,
    wet_receipts: rtrb::Producer<WetReceipt>,
    talkback: rtrb::Consumer<TalkbackIngress>,
    talkback_receipts: rtrb::Producer<TalkbackReceipt>,
}
fn prefault_lane<T: Copy>(capacity: usize, value: T) -> (rtrb::Producer<T>, rtrb::Consumer<T>) {
    let (mut producer, mut consumer) = rtrb::RingBuffer::new(capacity);
    for _ in 0..capacity {
        assert!(producer.push(value).is_ok());
    }
    for _ in 0..capacity {
        assert!(consumer.pop().is_ok());
    }
    (producer, consumer)
}
impl HostMediaIngress {
    fn prepare() -> (Self, SourceMediaIngress) {
        let source_frame = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
        let empty = WetIngress {
            generation: 0,
            length: 0,
            bytes: [0; crate::transport::MAX_DATAGRAM],
        };
        let (wet, source_wet) = prefault_lane(WET_INGRESS_PACKETS, empty);
        let (source_wet_receipts, wet_receipts) = prefault_lane(
            WET_INGRESS_PACKETS,
            WetReceipt {
                packet: empty,
                result: WetAdmission::Rejected,
                epoch: 0,
                frame: 0,
            },
        );
        let (talkback, source_talkback) = prefault_lane(
            TALKBACK_INGRESS_BLOCKS,
            TalkbackIngress {
                generation: 0,
                frame: 0,
                deadline_ms: 0,
                samples: [0.; 48],
            },
        );
        let (source_talkback_receipts, talkback_receipts) =
            prefault_lane(TALKBACK_INGRESS_BLOCKS, TalkbackReceipt { accepted: false });
        (
            Self {
                wet: WetProducer(std::sync::Arc::new(std::sync::Mutex::new(
                    WetProducerState {
                        producer: wet,
                        closed: false,
                        rejected: 0,
                    },
                ))),
                exported_wet_generation: 0,
                source_frame: source_frame.clone(),
                wet_receipts,
                talkback: TalkbackProducer::new(talkback),
                talkback_receipts,
                wet_generation: 0,
                talkback_generation: 0,
            },
            SourceMediaIngress {
                source_frame,
                wet: source_wet,
                wet_receipts: source_wet_receipts,
                talkback: source_talkback,
                talkback_receipts: source_talkback_receipts,
            },
        )
    }
}
/// Exclusive source media storage. It contains no authenticated policy handle,
/// packet encoder, JSON state, worker join handle or controller reference.
/// Prepared controller commands install storage and fixed policy fences on the
/// dedicated source executor. Ingress and numeric readback are bounded; autonomous
/// source pacing and controller-stall acceptance remain separate requirements.
#[derive(Clone, Copy, Default)]
struct ClockMediaCounts {
    samples: u64,
    energy: f64,
    egress_drops: u64,
    monitor_drops: u64,
    monitor_packets: u64,
}
struct AnalysisSource {
    indices: Vec<usize>,
    samples: Vec<f64>,
    input: super::source_egress::SourceEgressInput,
    fence: super::policy::PolicyFence,
    epoch: u64,
    map: u64,
}
impl AnalysisSource {
    fn offer(&mut self, epoch: u64, map: u64, frame: u64, capture: &[f64], width: usize) {
        if epoch != self.epoch || map != self.map || !self.fence.valid() {
            self.input.clear();
            return;
        }
        for f in 0..48 {
            for (i, slot) in self.indices.iter().enumerate() {
                self.samples[f * self.indices.len() + i] = capture[f * width + slot];
            }
        }
        self.input.offer(frame, &self.samples, &[]);
    }
}
struct HostSourceMedia {
    observer: Option<AnalysisSource>,
    clock_frame: u64,
    clock_counts: ClockMediaCounts,
    clock_monitor_deadline_ms: Option<u64>,
    clock_monitor_silent: bool,
    clock_monitor_generation: Option<u64>,
    clock_monitor_retirement: Option<(u64, u64)>,
    ingress_stopped: bool,
    ingress: Option<SourceMediaIngress>,
    wet_generation: u64,
    wet_fence: Option<super::policy::PolicyFence>,
    talkback_generation: u64,
    brain_handoff: Option<std::sync::Arc<BrainHandoffState>>,
    clock_talkback_deadline: Option<u64>,
    talkback_fence: Option<super::policy::PolicyFence>,
    talkback_deadline_ms: u64,
    talkback: Option<TalkbackSource>,
    talkback_samples: [f64; 48],
    monitor: Option<rtrb::Producer<MonitorBlock>>,
    wet_groups: Vec<WetGroup>,
    gates: [WetGate; 8],
    wet_channels: usize,
    wet: [f64; 96],
    egress: Option<super::source_egress::SourceEgressInput>,
    analysis_indices: Vec<usize>,
    analysis: Vec<f64>,
    fx_send: Vec<f64>,
}
#[derive(Debug)]
enum WetRenderError {
    Timeline,
    Packet(crate::transport::Error),
}
impl HostSourceMedia {
    fn new() -> Self {
        Self {
            observer: None,
            clock_frame: 0,
            clock_counts: ClockMediaCounts::default(),
            clock_monitor_deadline_ms: None,
            clock_monitor_silent: false,
            clock_monitor_generation: None,
            clock_monitor_retirement: None,
            ingress_stopped: false,
            ingress: None,
            wet_generation: 0,
            wet_fence: None,
            talkback_generation: 0,
            brain_handoff: None,
            clock_talkback_deadline: None,
            talkback_fence: None,
            talkback_deadline_ms: 0,
            talkback: None,
            talkback_samples: [0.; 48],
            monitor: None,
            wet_groups: Vec::new(),
            gates: [WetGate::default(); 8],
            wet_channels: 2,
            wet: [0.; 96],
            egress: None,
            analysis_indices: Vec::new(),
            analysis: Vec::new(),
            fx_send: vec![0.; 96],
        }
    }
    /// Fixed ingress work; occupied receipts stop draining without stopping DSP.
    /// Policy/session closure is checked here independently of controller service.
    fn drain_ingress(&mut self, epoch: u64, frame: u64, now_ms: u64) -> bool {
        self.clock_frame = frame;
        let Some(ingress) = &mut self.ingress else {
            return false;
        };
        ingress
            .source_frame
            .store(frame, std::sync::atomic::Ordering::Release);
        for _ in 0..WET_INGRESS_PACKETS {
            if ingress.wet_receipts.is_full() {
                break;
            }
            let Ok(block) = ingress.wet.pop() else {
                break;
            };
            let mut result = WetAdmission::Rejected;
            if !self.ingress_stopped
                && block.generation == self.wet_generation
                && self.wet_fence.as_ref().is_some_and(|f| f.valid())
                && let Ok(packet) = Packet::parse(&block.bytes[..block.length])
                && let Some(group) = self.wet_groups.iter_mut().find(|g| g.spec == packet.spec())
            {
                result = match group.buffer.insert_wet_at(packet, frame) {
                    Ok(()) => WetAdmission::Accepted,
                    Err(crate::transport::Error::Late) => WetAdmission::Late,
                    Err(_) => WetAdmission::Rejected,
                };
            }
            assert!(
                ingress
                    .wet_receipts
                    .push(WetReceipt {
                        packet: block,
                        result,
                        epoch,
                        frame
                    })
                    .is_ok()
            );
        }
        let mut brain_failed = false;
        for _ in 0..TALKBACK_INGRESS_BLOCKS {
            if ingress.talkback_receipts.is_full() {
                break;
            }
            let Ok(block) = ingress.talkback.pop() else {
                break;
            };
            let mut accepted = false;
            if !self.ingress_stopped
                && self
                    .clock_talkback_deadline
                    .is_none_or(|deadline| now_ms < deadline)
                && block.generation == self.talkback_generation
                && self.talkback_fence.as_ref().is_some_and(|f| f.valid())
                && now_ms < block.deadline_ms
                && let Some(source) = &mut self.talkback
                && source.last_received.is_none_or(|last| block.frame > last)
            {
                if source
                    .bridge
                    .push(source.epochs, block.frame, &block.samples)
                    .is_ok()
                {
                    source.last_received = Some(block.frame);
                    self.talkback_deadline_ms = block.deadline_ms;
                    accepted = true;
                } else {
                    brain_failed = true;
                }
            }
            assert!(
                ingress
                    .talkback_receipts
                    .push(TalkbackReceipt { accepted })
                    .is_ok()
            );
        }
        if self.talkback.is_some()
            && (self.talkback_fence.as_ref().is_some_and(|f| !f.valid())
                || (self.talkback_deadline_ms != 0 && now_ms >= self.talkback_deadline_ms))
        {
            if let Some(source) = &mut self.talkback {
                source.bridge.invalidate();
            }
            self.talkback_samples.fill(0.);
            brain_failed = true;
        }
        brain_failed
    }
    fn render_wet(
        &mut self,
        frame: u64,
        safety: &[f64; 48],
    ) -> std::result::Result<(), WetRenderError> {
        let mut incoming = [None; 384];
        // Session closure also fences packets already admitted to playout. Keep
        // advancing their cursor, but feed missing samples into the bounded fade.
        for group in &mut self.wet_groups {
            if frame
                < group
                    .first
                    .saturating_add(u64::from(group.spec.delay_frames))
            {
                continue;
            }
            if group
                .buffer
                .next_frame()
                .checked_add(u64::from(group.spec.delay_frames))
                != Some(frame)
            {
                return Err(WetRenderError::Timeline);
            }
            let mut bytes = [0; crate::transport::MAX_DATAGRAM];
            if let Some(length) = group
                .buffer
                .pop(&mut bytes)
                .map_err(WetRenderError::Packet)?
            {
                if self.ingress_stopped || self.wet_fence.as_ref().is_some_and(|f| !f.valid()) {
                    continue;
                }
                let packet = Packet::parse(&bytes[..length]).map_err(WetRenderError::Packet)?;
                for f in 0..48 {
                    for ch in 0..usize::from(group.spec.channels) {
                        incoming
                            [f * self.wet_channels + usize::from(group.spec.first_channel) + ch] =
                            Some(
                                packet
                                    .sample(f * usize::from(group.spec.channels) + ch)
                                    .map_err(WetRenderError::Packet)?,
                            );
                    }
                }
            }
        }
        self.wet.fill(0.);
        for f in 0..48 {
            for ch in 0..self.wet_channels {
                self.wet[f * 2 + ch % 2] +=
                    self.gates[ch].step(incoming[f * self.wet_channels + ch]);
            }
            if self.wet_channels == 8 {
                self.wet[f * 2] *= safety[f];
                self.wet[f * 2 + 1] *= safety[f];
            }
        }
        Ok(())
    }
    /// Source-clock ASRC operation using prevalidated numeric control intent.
    /// False asks the controller to publish path closure; it never destroys the
    /// bridge or closes a device on this execution path.
    fn render_talkback(&mut self, frame: u64, enabled: bool, hold_current: bool) -> bool {
        self.talkback_samples.fill(0.);
        let Some(source) = &mut self.talkback else {
            return true;
        };
        if !enabled
            || self.ingress_stopped
            || self.talkback_fence.as_ref().is_some_and(|f| !f.valid())
        {
            return true;
        }
        if !hold_current {
            source.bridge.invalidate();
            return true;
        }
        let status = source.bridge.status();
        if status.fault.is_some() {
            return false;
        }
        if !status.armed && status.ready {
            let _ = source.bridge.arm(source.epochs);
        }
        !source.bridge.status().armed
            || source
                .bridge
                .render(source.epochs, frame, &mut self.talkback_samples)
                .is_ok()
    }
    fn offer_monitor(&mut self, frame: u64, samples: [f64; 96]) -> bool {
        self.monitor
            .as_mut()
            .is_some_and(|lane| lane.push(MonitorBlock { frame, samples }).is_ok())
    }
    fn offer_egress(&mut self, frame: u64, capture: &[f64], capture_channels: usize) -> bool {
        for f in 0..48 {
            for (index, slot) in self.analysis_indices.iter().enumerate() {
                self.analysis[f * self.analysis_indices.len() + index] =
                    capture[f * capture_channels + slot];
            }
        }
        self.egress
            .as_mut()
            .is_some_and(|input| input.offer(frame, &self.analysis, &self.fx_send))
    }
}
/// Numeric source readback. No live media owner is borrowed by the controller.
#[derive(Clone, Copy)]
struct MediaObservation {
    clock_counts: ClockMediaCounts,
    ingress_stopped: bool,
    #[cfg(test)]
    ingress_present: bool,
    #[cfg(test)]
    monitor_present: bool,
    talkback_fence_current: bool,
    #[cfg(test)]
    talkback_fence_present: bool,
    talkback_status: Option<crate::brain_audio::bridge::BridgeStatus>,
    #[cfg(test)]
    talkback_last_received: Option<u64>,
    wet: [f64; 96],
    talkback_samples: [f64; 48],
}
impl MediaObservation {
    fn capture(source: &HostSourceMedia) -> Self {
        Self {
            clock_counts: source.clock_counts,
            ingress_stopped: source.ingress_stopped,
            #[cfg(test)]
            ingress_present: source.ingress.is_some(),
            #[cfg(test)]
            monitor_present: source.monitor.is_some(),
            talkback_fence_current: source.talkback_fence.as_ref().is_some_and(|f| f.valid()),
            #[cfg(test)]
            talkback_fence_present: source.talkback_fence.is_some(),
            talkback_status: source.talkback.as_ref().map(|s| s.bridge.status()),
            #[cfg(test)]
            talkback_last_received: source.talkback.as_ref().and_then(|s| s.last_received),
            wet: source.wet,
            talkback_samples: source.talkback_samples,
        }
    }
}
/// Media and RenderGraph handles share one dedicated source executor. Every
/// operation owns its arguments; displaced bridges, rings and buffers return in
/// the same reserved packet and are destroyed by the controller.
struct SourceMediaController {
    executor: crate::source_executor::SourceExecutor<HostSourceMedia>,
    observed: MediaObservation,
    capture: Option<Vec<f64>>,
}
impl std::ops::Deref for SourceMediaController {
    type Target = MediaObservation;
    fn deref(&self) -> &Self::Target {
        &self.observed
    }
}
impl SourceMediaController {
    fn new(provider: &mut LocalAudio, source: HostSourceMedia) -> Result<Self> {
        let observed = MediaObservation::capture(&source);
        let capture = Some(vec![0.; 48 * provider.topology().capture_channels]);
        Ok(Self {
            executor: provider.source_executor_for(source)?,
            observed,
            capture,
        })
    }
    fn execute<P: Send + 'static>(
        &mut self,
        data: P,
        apply: fn(&mut HostSourceMedia, &mut P),
    ) -> Result<P> {
        let (data, _, observed) =
            self.executor
                .transact((data, apply, self.observed), |source, packet| {
                    (packet.1)(source, &mut packet.0);
                    packet.2 = MediaObservation::capture(source);
                })?;
        self.observed = observed;
        Ok(data)
    }
    fn clear_brain(&mut self) {
        // If the executor has failed, close admission immediately; retained owners
        // remain with executor teardown rather than being destroyed on the source.
        if self
            .execute((None, None), |source, retired| {
                std::mem::swap(&mut source.talkback, &mut retired.0);
                std::mem::swap(&mut source.monitor, &mut retired.1);
                source.talkback_samples.fill(0.);
                source.clock_monitor_generation = None;
                if let Some(handoff) = &source.brain_handoff {
                    handoff.close();
                }
            })
            .is_err()
        {
            self.observed.ingress_stopped = true;
        }
    }
    fn clear_wet(&mut self, reset: bool) {
        if self
            .execute((Vec::new(), reset), |source, packet| {
                std::mem::swap(&mut source.wet_groups, &mut packet.0);
                if packet.1 {
                    source.gates = [WetGate::default(); 8];
                    source.wet.fill(0.);
                }
            })
            .is_err()
        {
            self.observed.ingress_stopped = true;
        }
    }
    fn close_egress(&mut self) {
        if self
            .execute((), |source, _| {
                if let Some(input) = &mut source.egress {
                    input.clear();
                }
            })
            .is_err()
        {
            self.observed.ingress_stopped = true;
        }
    }
    fn retire_egress(&mut self) {
        if self
            .execute(None, |source, retired| {
                std::mem::swap(&mut source.egress, retired);
            })
            .is_err()
        {
            self.observed.ingress_stopped = true;
        }
    }
    fn drain_ingress(&mut self, epoch: u64, frame: u64, now: u64) -> Result<bool> {
        self.execute((epoch, frame, now, false), |source, p| {
            p.3 = source.drain_ingress(p.0, p.1, p.2);
        })
        .map(|p| p.3)
    }
    fn render_wet(
        &mut self,
        frame: u64,
        safety: [f64; 48],
    ) -> Result<std::result::Result<(), WetRenderError>> {
        self.execute((frame, safety, Ok(())), |source, p| {
            p.2 = source.render_wet(p.0, &p.1);
        })
        .map(|p| p.2)
    }
    fn render_talkback(&mut self, frame: u64, enabled: bool, hold: bool) -> Result<bool> {
        self.execute((frame, enabled, hold, false), |source, p| {
            p.3 = source.render_talkback(p.0, p.1, p.2);
        })
        .map(|p| p.3)
    }
    fn offer_monitor(&mut self, frame: u64, samples: [f64; 96]) -> Result<bool> {
        self.execute((frame, samples, false), |source, p| {
            p.2 = source.offer_monitor(p.0, p.1);
        })
        .map(|p| p.2)
    }
    #[allow(clippy::too_many_arguments)]
    fn offer_egress(
        &mut self,
        frame: u64,
        capture: &[f64],
        capture_channels: usize,
        aux: [[f64; 96]; 4],
        send: [f64; 96],
        four_aux: bool,
    ) -> Result<bool> {
        let buffer = self
            .capture
            .as_mut()
            .ok_or("source egress packet unavailable")?;
        if buffer.len() != capture.len() {
            return Err("source egress capture shape".into());
        }
        buffer.copy_from_slice(capture);
        let buffer = self.capture.take().unwrap();
        let packet = self.execute(
            (buffer, frame, capture_channels, aux, send, four_aux, false),
            |source, p| {
                if p.5 {
                    for f in 0..48 {
                        for (bus, samples) in p.3.iter().enumerate() {
                            source.fx_send[f * 8 + bus * 2..f * 8 + bus * 2 + 2]
                                .copy_from_slice(&samples[f * 2..f * 2 + 2]);
                        }
                    }
                } else {
                    source.fx_send.copy_from_slice(&p.4);
                }
                p.6 = source.offer_egress(p.1, &p.0, p.2);
            },
        )?;
        self.capture = Some(packet.0);
        Ok(packet.6)
    }
}
struct PreparedMedia {
    wet_channels: usize,
    fx_send: Vec<f64>,
    analysis: Vec<f64>,
    analysis_indices: Vec<usize>,
    egress: Option<super::source_egress::SourceEgressInput>,
    wet_groups: Vec<WetGroup>,
    wet_generation: u64,
    wet_fence: Option<super::policy::PolicyFence>,
}
struct WetGroup {
    spec: StreamSpec,
    first: u64,
    buffer: JitterBuffer,
}

/// Source-clock witness only: this is not a wall-clock or one-way latency sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LateWetWitness {
    pub source_epoch: u64,
    pub packet_source_frame: u64,
    pub packet_output_frame: u64,
    pub observed_source_frame: u64,
}

#[derive(Default, Serialize)]
pub struct HostMediaStats {
    pub source_egress_dropped_blocks: u64,
    pub accepted_talkback_packets: u64,
    pub rejected_talkback_packets: u64,
    pub emitted_monitor_packets: u64,
    pub dropped_monitor_packets: u64,
    pub accepted_wet_packets: u64,
    pub late_wet_packets: u64,
    pub first_late_wet: Option<LateWetWitness>,
    pub last_late_wet: Option<LateWetWitness>,
    pub rejected_wet_packets: u64,
    pub nonzero_received_wet_samples: u64,
    pub nonzero_rendered_wet_samples: u64,
    pub rendered_wet_energy: f64,
    pub first_wet_source_frame: Option<u64>,
    pub last_wet_source_frame: Option<u64>,
    pub first_wet_output_frame: Option<u64>,
    pub last_wet_output_frame: Option<u64>,
}

struct DeviceRead {
    at: u64,
    identity: Option<(u64, u64)>,
}

pub struct HostAuthority {
    fx: fx_relay::FxRelay,
    provider: LocalAudio,
    meter_reads: BTreeMap<u64, u64>,
    pending_meter_reads: BTreeMap<u64, crate::meter_wire::Request>,
    reads: BTreeMap<u64, ReadState>,
    structural_reads: BTreeMap<u64, u64>,
    fx_aux_reads: BTreeMap<u64, u64>,
    master_eq_reads: BTreeMap<u64, u64>,
    brain_reads: BTreeMap<u64, u64>,
    held_queries: BTreeMap<u64, u64>,
    paired_queries: BTreeMap<u64, u64>,
    brain_generations: BTreeMap<u64, u64>,
    device_reads: BTreeMap<u64, DeviceRead>,
    device_owner: Option<AuthenticatedContext>,
    device_observation: Option<BrainDeviceObservation>,
    device_observed_ms: Option<u64>,
    device_pending: Option<DeviceChange>,
    device_messages: VecDeque<(u64, Value)>,
    device_results: VecDeque<Value>,
    device_history: VecDeque<(String, Value)>,
    device_ticket: u64,
    brain: Option<BrainMediaState>,
    descriptor: Option<MediaDescriptor>,
    media_owner: Option<u64>,
    aux: fx_aux_relay::AuxRelay,
    // Field order also joins the controller encoder before freeing its source
    // endpoint during final controller-side HostAuthority destruction.
    source_egress: super::source_egress::SourceEgress,
    analysis_egress: super::source_egress::SourceEgress,
    analysis_owner: Option<u64>,
    retired_analysis: super::source_egress::Counts,
    source_media: SourceMediaController,
    media_ingress: HostMediaIngress,
    retired_egress: super::source_egress::Counts,
    completions: VecDeque<Value>,
    capability_generation: u64,
    media_stats: HostMediaStats,
    accepted_wet_hash: Sha256,
    observed_source_epoch: u64,
    source_clock_status: Option<SourceClockStatus>,
    clock_media_accounted: ClockMediaCounts,
}
// Account for the existing GP-REMOTE Reply wrapper as well as the additive
// envelope. Compute its small overhead through the real serializer; do not clone
// or serialize another topology merely to size the transport wrapper.
fn paired_transport_capacity(payload_bytes: usize, session: u64) -> Result<()> {
    let wrapper = serde_json::to_vec(&Response::Reply {
        session: Counter(session),
        payload: Value::Null,
    })
    .map_err(|e| e.to_string())?;
    let overhead = wrapper.len() - b"null".len();
    if payload_bytes
        .checked_add(overhead)
        .is_none_or(|n| n > crate::snapshot_pages::ASSEMBLY_BYTES)
    {
        return Err("paired transport assembly capacity".into());
    }
    Ok(())
}
impl HostAuthority {
    /// Start a bounded device-free capture clock on the existing source worker.
    /// Frame-zero startup and subsequent control service are independent of
    /// capture progression. The production runner uses this lifecycle directly.
    pub fn start_source_clock(
        &mut self,
        now_ms: u64,
        period: std::time::Duration,
        blocks: u64,
    ) -> Result<SourceClockStatus> {
        if self.source_clock_status.is_some() {
            return Err("source clock already active".into());
        }
        let state = source_clock::SourceClock::prepare(&self.provider, now_ms, period, blocks)?;
        self.install_source_clock(state)
    }
    pub(crate) fn start_device_source_clock(
        &mut self,
        now_ms: u64,
        blocks: u64,
        device: crate::host::duplex::Duplex,
    ) -> Result<SourceClockStatus> {
        if self.source_clock_status.is_some() {
            return Err("source clock already active".into());
        }
        let state =
            source_clock::SourceClock::prepare_device(&self.provider, now_ms, blocks, device)?;
        self.install_source_clock(state)
    }
    fn install_source_clock(
        &mut self,
        state: source_clock::SourceClock,
    ) -> Result<SourceClockStatus> {
        self.refresh_clock_media()?;
        let now_ms = state.observed_now_ms();
        let status = state.status.clone();
        self.provider.start_source_progress(
            now_ms,
            &self.source_media.executor,
            state,
            source_clock::SourceClock::advance,
        )?;
        self.source_clock_status = Some(status.clone());
        Ok(status)
    }
    fn refresh_clock_media(&mut self) -> Result<()> {
        let deadline = self.device_observed_ms.map(|t| t.saturating_add(250));
        let silent = self
            .brain
            .as_ref()
            .is_some_and(|b| b.monitor_retirement.is_some());
        let before = self.clock_media_accounted;
        self.source_media.execute((deadline, silent), |source, p| {
            source.clock_monitor_deadline_ms = p.0;
            source.clock_monitor_silent = p.1;
        })?;
        let after = self.source_media.observed.clock_counts;
        self.clock_media_accounted = after;
        self.media_stats.nonzero_rendered_wet_samples += after.samples - before.samples;
        self.media_stats.rendered_wet_energy += after.energy - before.energy;
        self.media_stats.source_egress_dropped_blocks += after.egress_drops - before.egress_drops;
        self.media_stats.dropped_monitor_packets += after.monitor_drops - before.monitor_drops;
        self.media_stats.emitted_monitor_packets += after.monitor_packets - before.monitor_packets;
        Ok(())
    }
    /// Controller service and copied readback; never advances capture/render time.
    pub(crate) fn service_source_clock(&mut self, now_ms: u64) -> Result<()> {
        if self.source_clock_status.is_none() {
            return Err("source clock absent".into());
        }
        self.provider.service_source_clock(now_ms)?;
        self.finish_media_ingress();
        self.tick_fx(now_ms)?;
        self.tick_aux(now_ms)?;
        self.commit_device_intent(now_ms)?;
        self.refresh_clock_media()?;
        self.update_brain_readiness(now_ms);
        let replies = self.provider.take_remote_completions();
        if self.completions.len() + replies.len() > 128 {
            return Err("remote final queue capacity".into());
        }
        self.completions.extend(replies);
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn with_saturated_source_returns(&self, completions: bool, action: impl FnOnce()) {
        self.source_media
            .executor
            .with_saturated_returns(completions, action);
    }
    /// Stop pacing and retire prepared capture/report storage on the controller.
    /// The stop signal closes numeric owners even if command returns are full.
    pub fn finish_source_clock(&mut self, status: &SourceClockStatus) -> Result<SourceClockReport> {
        if !self
            .source_clock_status
            .as_ref()
            .is_some_and(|active| active.same_owner(status))
        {
            return Err("source clock status identity".into());
        }
        status.close();
        let mut state: source_clock::SourceClock = self.provider.stop_source_progress()?;
        state.report.finish_observations();
        self.source_clock_status = None;
        status.mark_stopped();
        self.provider
            .service_source_clock(state.observed_now_ms())?;
        self.provider.quiesce_source("source_clock_end")?;
        self.clear_aux();
        self.fx_clear_media();
        // Terminal closure needs no further capture to return a retained send
        // packet. Retire it here even when its target frame was never reached.
        let _ = self.provider.take_remote_fx_completion()?;
        self.brain = None;
        self.source_media.clear_brain();
        self.refresh_clock_media()?;
        self.finish_media_ingress();
        Ok(state.report)
    }
    /// Both server read gates commit only after the whole paired envelope exists.
    fn finish_paired_readback(
        &mut self,
        context: &AuthenticatedContext,
        reply: crate::paired_readback::Reply,
        now_ms: u64,
    ) -> Result<Value> {
        use crate::paired_readback::{Reason, Reply};
        let mut reply = reply;
        // The entire envelope (including context) shares the existing 1 MiB cap.
        // A failed construction cannot admit either server read latch. The
        // authenticated query nonce was already consumed without caching a reply.
        let encoded = reply.encode().and_then(|bytes| {
            paired_transport_capacity(bytes.len(), context.session())?;
            Ok(bytes)
        });
        let bytes = match encoded {
            Ok(bytes) => bytes,
            Err(_) if reply.raw.is_some() => {
                reply = Reply::new(reply.context, Err(Reason::Capacity));
                reply.encode()?
            }
            Err(e) => return Err(e),
        };
        let value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if reply.raw.is_some() {
            self.reads
                .entry(context.session())
                .or_default()
                .admit_audio_snapshot();
            self.brain_reads.insert(context.session(), now_ms);
        }
        Ok(value)
    }

    pub fn new(mut provider: LocalAudio, capability_generation: u64) -> Result<Self> {
        if capability_generation == 0 {
            return Err("zero capability generation".into());
        }
        provider.enable_persistent_source();
        let observed_source_epoch = provider.source_epoch();
        let (media_ingress, ingress) = HostMediaIngress::prepare();
        let mut source_media = HostSourceMedia::new();
        source_media.ingress = Some(ingress);
        let source_media = SourceMediaController::new(&mut provider, source_media)?;
        Ok(Self {
            fx: fx_relay::FxRelay::default(),
            provider,
            meter_reads: BTreeMap::new(),
            pending_meter_reads: BTreeMap::new(),
            reads: BTreeMap::new(),
            structural_reads: BTreeMap::new(),
            fx_aux_reads: BTreeMap::new(),
            master_eq_reads: BTreeMap::new(),
            brain_reads: BTreeMap::new(),
            held_queries: BTreeMap::new(),
            paired_queries: BTreeMap::new(),
            brain_generations: BTreeMap::new(),
            device_reads: BTreeMap::new(),
            device_owner: None,
            device_observation: None,
            device_observed_ms: None,
            device_pending: None,
            device_messages: VecDeque::with_capacity(8),
            device_results: VecDeque::with_capacity(16),
            device_history: VecDeque::with_capacity(64),
            device_ticket: 0,
            brain: None,
            descriptor: None,
            media_owner: None,
            aux: Default::default(),
            source_egress: super::source_egress::SourceEgress::default(),
            analysis_egress: super::source_egress::SourceEgress::default(),
            analysis_owner: None,
            retired_analysis: super::source_egress::Counts::default(),
            source_media,
            media_ingress,
            retired_egress: super::source_egress::Counts::default(),
            completions: VecDeque::with_capacity(128),
            capability_generation,
            media_stats: HostMediaStats::default(),
            accepted_wet_hash: Sha256::new(),
            observed_source_epoch,
            source_clock_status: None,
            clock_media_accounted: ClockMediaCounts::default(),
        })
    }
    /// Controller accounting: accepted payload hashing never enters source work.
    fn finish_media_ingress(&mut self) {
        self.media_stats.rejected_talkback_packets += self.media_ingress.talkback.take_rejected();
        {
            let mut state = self
                .media_ingress
                .wet
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            self.media_stats.rejected_wet_packets = self
                .media_stats
                .rejected_wet_packets
                .saturating_add(std::mem::take(&mut state.rejected));
        }
        for _ in 0..WET_INGRESS_PACKETS {
            let Ok(receipt) = self.media_ingress.wet_receipts.pop() else {
                break;
            };
            let block = receipt.packet;
            let Ok(packet) = Packet::parse(&block.bytes[..block.length]) else {
                self.media_stats.rejected_wet_packets =
                    self.media_stats.rejected_wet_packets.saturating_add(1);
                continue;
            };
            match receipt.result {
                WetAdmission::Accepted => {
                    self.media_stats.accepted_wet_packets =
                        self.media_stats.accepted_wet_packets.saturating_add(1);
                    self.media_stats
                        .first_wet_source_frame
                        .get_or_insert(packet.source_frame());
                    self.media_stats.last_wet_source_frame = Some(packet.source_frame());
                    self.media_stats
                        .first_wet_output_frame
                        .get_or_insert(packet.output_frame());
                    self.media_stats.last_wet_output_frame = Some(packet.output_frame());
                    for index in 0..packet.spec().samples() {
                        self.media_stats.nonzero_received_wet_samples +=
                            u64::from(packet.sample(index).is_ok_and(|sample| sample != 0.));
                    }
                    self.accepted_wet_hash.update(packet.bytes());
                }
                WetAdmission::Late => {
                    self.media_stats.late_wet_packets =
                        self.media_stats.late_wet_packets.saturating_add(1);
                    let witness = LateWetWitness {
                        source_epoch: receipt.epoch,
                        packet_source_frame: packet.source_frame(),
                        packet_output_frame: packet.output_frame(),
                        observed_source_frame: receipt.frame,
                    };
                    self.media_stats.first_late_wet.get_or_insert(witness);
                    self.media_stats.last_late_wet = Some(witness);
                }
                WetAdmission::Rejected => {
                    self.media_stats.rejected_wet_packets =
                        self.media_stats.rejected_wet_packets.saturating_add(1);
                }
            }
        }
        for _ in 0..TALKBACK_INGRESS_BLOCKS {
            let Ok(receipt) = self.media_ingress.talkback_receipts.pop() else {
                break;
            };
            if receipt.accepted {
                self.media_stats.accepted_talkback_packets =
                    self.media_stats.accepted_talkback_packets.saturating_add(1);
            } else {
                self.media_stats.rejected_talkback_packets =
                    self.media_stats.rejected_talkback_packets.saturating_add(1);
            }
        }
    }
    fn retire_analysis(&mut self) {
        // Stop source offers before joining/counting the encoder. Retain the
        // returned sample/fence storage until the worker has stopped.
        let retired = self.source_media.execute(None, |source, retired| {
            std::mem::swap(&mut source.observer, retired);
        });
        self.analysis_egress.finish();
        self.retired_analysis.add(self.analysis_egress.counts());
        self.analysis_egress = super::source_egress::SourceEgress::default();
        self.analysis_owner = None;
        drop(retired);
    }
    fn negotiate_analysis(
        &mut self,
        context: &AuthenticatedContext,
        descriptor: &MediaDescriptor,
    ) -> Result<bool> {
        if self.media_owner == Some(context.session())
            || self
                .brain
                .as_ref()
                .is_some_and(|b| b.context.session() == context.session())
        {
            return Err("analysis requires a separate read-only connection".into());
        }
        descriptor.validate(context, &self.identity(), crate::transport::MAX_DATAGRAM)?;
        if self.analysis_owner.is_some()
            || self
                .descriptor
                .as_ref()
                .is_some_and(|d| d.channels(MediaRole::Analysis) > 0)
        {
            return Err("analysis subscription already attached".into());
        }
        if descriptor.streams.iter().any(|s| s.frames != 48) {
            return Err("composed host currently advertises 48-frame media blocks".into());
        }
        let mut streams: Vec<_> = descriptor.streams.iter().collect();
        streams.sort_by_key(|s| s.first_channel);
        let mut indices = Vec::new();
        for id in streams.iter().flat_map(|s| &s.channel_ids) {
            indices.push(
                self.provider
                    .topology()
                    .inputs
                    .iter()
                    .find(|i| &i.id == id)
                    .ok_or("unknown admitted raw source")?
                    .capture_slot,
            );
        }
        let mut egress =
            super::source_egress::SourceEgress::prepare(descriptor.clone(), indices.len() * 48, 0)?;
        let prepared = AnalysisSource {
            samples: vec![0.; indices.len() * 48],
            indices,
            input: egress.take_input().ok_or("analysis input unavailable")?,
            fence: context.audio_fence(),
            epoch: descriptor.source_epoch.0,
            map: descriptor.map_generation.0,
        };
        let first = if self.source_clock_status.is_some() {
            self.provider.remote_target_frame(480)?
        } else {
            self.provider.frame()
        };
        let packet = self.provider.source_pair_command(
            &self.source_media.executor,
            (Some(prepared), first, false),
            |graph, source, p| {
                let frame = graph.mixer.mixer.frame();
                if frame < p.1 {
                    return false;
                }
                if frame != p.1
                    || !p.0.as_ref().is_some_and(|a| a.fence.valid())
                    || graph.closure.load(std::sync::atomic::Ordering::Acquire)
                {
                    return true;
                }
                std::mem::swap(&mut source.observer, &mut p.0);
                p.2 = true;
                true
            },
        )?;
        if !packet.2 {
            return Err("analysis preparation expired or authority closed".into());
        }
        self.analysis_egress = egress;
        self.analysis_owner = Some(context.session());
        Ok(true)
    }
    fn clear_media_history(&mut self) {
        self.retire_analysis();
        self.finish_media_ingress();
        self.fx_clear_media();
        self.clear_aux();
        self.brain = None;
        self.source_media.clear_brain();
        self.provider.close_brain_audio();
        self.descriptor = None;
        self.media_owner = None;
        self.source_media.clear_wet(true);
        self.close_source_egress();
    }
    fn queue_device_result(&mut self, value: Value) {
        if self.device_results.len() == 16 {
            self.device_results.pop_front();
        }
        self.device_results.push_back(value);
    }
    fn finish_device(&mut self, change: &DeviceChange, state: &str, reason: Option<String>) {
        let value = self.device_reply(change, state, reason);
        if self.device_history.len() == 64 {
            self.device_history.pop_front();
        }
        self.device_history
            .push_back((change.fingerprint.clone(), value.clone()));
        self.queue_device_result(value);
    }
    fn device_reply(&self, change: &DeviceChange, state: &str, reason: Option<String>) -> Value {
        serde_json::json!({"contract":"GP15-device","version":1,"state":state,"reason":reason,
            "ticket":change.ticket,"context":crate::mixer_control::RequestContext::request(&change.request),
            "revision":self.provider.brain_snapshot().revision,"stagebox_frame":self.provider.frame(),"observation":self.device_observation})
    }
    fn commit_device_intent(&mut self, now: u64) -> Result<()> {
        if self.device_pending.as_ref().is_some_and(|c| !c.sent)
            && self.provider.engine_mut().external_boundary().is_none()
        {
            let change = self.device_pending.take().unwrap();
            self.finish_device(
                &change,
                "failed_device",
                Some("configuration intent revoked".into()),
            );
        }
        let Some(change) = self.device_pending.as_ref() else {
            return Ok(());
        };
        if !change.sent
            && self.provider.engine_mut().external_boundary() == Some(self.provider.frame())
        {
            let valid = self
                .device_owner
                .as_ref()
                .is_some_and(|o| o.check_current().is_ok());
            let reply = self.provider.engine_mut().commit_external(now, || {
                if valid {
                    Ok(())
                } else {
                    Err("Brain duplex unavailable".into())
                }
            })?;
            let mut change = self.device_pending.take().unwrap();
            if reply.kind != "applied" {
                self.finish_device(&change, "failed_device", reply.body.reason);
                return Ok(());
            }
            self.provider.close_brain_audio();
            self.brain = None;
            self.source_media.clear_brain();
            change.sent = true;
            let owner = self.device_owner.as_ref().unwrap();
            self.device_messages.push_back((owner.session(),serde_json::json!({"contract":"GP15-device","version":1,"kind":"apply_configuration","ticket":change.ticket,"config":change.config})));
            self.queue_device_result(self.device_reply(&change, "accepted_intent", None));
            self.device_pending = Some(change);
        }
        if self
            .device_pending
            .as_ref()
            .is_some_and(|c| c.sent && (now >= c.deadline || self.device_owner.is_none()))
        {
            let change = self.device_pending.take().unwrap();
            self.finish_device(
                &change,
                "failed_device",
                Some("device acknowledgement deadline/session loss".into()),
            );
        }
        Ok(())
    }
    fn dispatch_device(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        now: u64,
    ) -> Result<Value> {
        if payload.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("Brain device version".into());
        }
        match payload.get("kind").and_then(Value::as_str) {
            Some("device_snapshot") => {
                self.device_reads.insert(
                    context.session(),
                    DeviceRead {
                        at: now,
                        identity: self
                            .device_observation
                            .as_ref()
                            .map(|o| (o.brain_epoch, o.brain_map)),
                    },
                );
                Ok(
                    serde_json::json!({"contract":"GP15-device","version":1,"state":"snapshot","observation":self.device_observation,
                    "observed_ms":self.device_observed_ms,"connected":self.device_owner.is_some(),"pending":self.device_pending.as_ref().map(|c|c.ticket),"bridge":self.brain_bridge_status().as_ref().map(brain_bridge_telemetry)}),
                )
            }
            Some("device_observation") => {
                context.require(&Permission::LocalOperatorMonitor)?;
                if self.device_owner.as_ref().is_some_and(|o| o != context) {
                    return Err("Brain device owner attached".into());
                }
                let observation: BrainDeviceObservation = serde_json::from_value(
                    payload
                        .get("observation")
                        .cloned()
                        .ok_or("device observation")?,
                )
                .map_err(|e| e.to_string())?;
                observation.validate()?;
                if let Some(ticket) = observation.ticket {
                    let change = self
                        .device_pending
                        .as_ref()
                        .ok_or("unexpected device ticket")?;
                    if !change.sent || ticket != change.ticket || now >= change.deadline {
                        return Err("stale device acknowledgement".into());
                    }
                    if observation.success == Some(true)
                        && (observation.config != change.config
                            || self.device_observation.as_ref().is_some_and(|o| {
                                observation.brain_epoch <= o.brain_epoch
                                    || observation.brain_map <= o.brain_map
                            }))
                    {
                        return Err(
                            "device configuration readback mismatch/fresh epoch required".into(),
                        );
                    }
                } else if self.device_observation.as_ref().is_some_and(|o| {
                    observation.brain_epoch < o.brain_epoch
                        || observation.brain_map < o.brain_map
                        || observation.brain_epoch == o.brain_epoch && observation.frame < o.frame
                }) {
                    return Err("stale device observation".into());
                }
                let replaced = self.device_observation.as_ref().is_some_and(|o| {
                    o.brain_epoch != observation.brain_epoch || o.brain_map != observation.brain_map
                });
                let device_closed = observation
                    .status
                    .get("fault")
                    .is_some_and(|f| !f.is_null())
                    || (observation.status.get("armed").and_then(Value::as_bool) == Some(false)
                        && self.device_observation.as_ref().is_some_and(|o| {
                            o.status.get("armed").and_then(Value::as_bool) == Some(true)
                        }));
                if replaced || device_closed {
                    self.provider.close_brain_audio();
                    self.brain = None;
                    self.source_media.clear_brain();
                }
                self.device_owner = Some(context.clone());
                self.device_observed_ms = Some(now);
                self.device_observation = Some(observation.clone());
                if observation.ticket.is_some() {
                    let change = self.device_pending.take().unwrap();
                    self.finish_device(
                        &change,
                        if observation.success == Some(true) {
                            "applied_device"
                        } else {
                            "failed_device"
                        },
                        observation.error,
                    );
                }
                Ok(
                    serde_json::json!({"contract":"GP15-device","version":1,"state":"observed","brain_epoch":observation.brain_epoch,"brain_map":observation.brain_map}),
                )
            }
            Some("device_configure") => {
                context.require(&Permission::LocalOperatorMonitor)?;
                let fingerprint = crate::hash::hex(Sha256::digest(super::encode(&payload)?));
                if let Some(change) = self
                    .device_pending
                    .as_ref()
                    .filter(|c| c.fingerprint == fingerprint)
                {
                    return Ok(self.device_reply(
                        change,
                        if change.sent {
                            "accepted_intent"
                        } else {
                            "pending"
                        },
                        None,
                    ));
                }
                if let Some((_, reply)) =
                    self.device_history.iter().find(|(f, _)| f == &fingerprint)
                {
                    return Ok(reply.clone());
                }
                if self.device_pending.is_some() || self.device_messages.len() >= 8 {
                    return Err("device configuration backpressure".into());
                }
                if self.device_owner.is_none() {
                    return Err("Brain device unavailable".into());
                }
                if !self
                    .device_reads
                    .get(&context.session())
                    .is_some_and(|read| {
                        now >= read.at
                            && now - read.at <= 250
                            && read.identity.is_some()
                            && read.identity
                                == self
                                    .device_observation
                                    .as_ref()
                                    .map(|o| (o.brain_epoch, o.brain_map))
                    })
                {
                    return Err("fresh device snapshot required".into());
                }
                let mut request_value = payload.clone();
                let object = request_value.as_object_mut().ok_or("device command")?;
                let config: crate::brain_audio::device::DeviceConfig =
                    serde_json::from_value(object.remove("config").ok_or("device config")?)
                        .map_err(|e| e.to_string())?;
                config.validate().map_err(|e| e.to_string())?;
                object.insert("contract".into(), Value::String("C-AUDIO".into()));
                object.insert("version".into(), serde_json::json!(2));
                object.insert("kind".into(), Value::String("renew".into()));
                object.insert("body".into(), serde_json::json!({}));
                let request =
                    crate::control_model::Request::decode(&super::encode(&request_value)?)?;
                if self.provider.engine_mut().writer_scope(&request, now)
                    != Some(crate::control_model::Scope::LocalOperatorMonitor)
                {
                    return Err("local operator lease required".into());
                }
                let mut clocked_result = None;
                let cached = if self.source_clock_status.is_some() {
                    let value = self.provider.clocked_control(
                        now,
                        Some(context.audio_fence()),
                        |provider| {
                            let (_, cached) = provider.engine_mut().begin_external(
                                &request,
                                &fingerprint,
                                crate::control_model::Scope::LocalOperatorMonitor,
                                now,
                            )?;
                            if let Some(reply) = cached {
                                return Ok(serde_json::json!({"cached":reply}));
                            }
                            let reply = provider.commit_clocked_external(now)?;
                            serde_json::to_value(reply).map_err(|e| e.to_string())
                        },
                    )?;
                    if !value["cached"].is_null() {
                        return Ok(
                            serde_json::json!({"contract":"GP15-device","version":1,"state":"replayed","result":value["cached"]}),
                        );
                    }
                    clocked_result = Some(value);
                    None
                } else {
                    self.provider
                        .engine_mut()
                        .begin_external(
                            &request,
                            &fingerprint,
                            crate::control_model::Scope::LocalOperatorMonitor,
                            now,
                        )?
                        .1
                };
                if let Some(reply) = cached {
                    return Ok(
                        serde_json::json!({"contract":"GP15-device","version":1,"state":"replayed","result":reply}),
                    );
                }
                self.device_ticket = self
                    .device_ticket
                    .checked_add(1)
                    .ok_or("device ticket exhausted")?;
                let mut change = DeviceChange {
                    fingerprint,
                    request,
                    config,
                    ticket: self.device_ticket,
                    deadline: now.saturating_add(2000),
                    sent: false,
                };
                if let Some(result) = clocked_result {
                    let owner = self
                        .device_owner
                        .as_ref()
                        .filter(|owner| owner.check_current().is_ok());
                    if result["kind"] != "applied" || owner.is_none() {
                        let reason = result["body"]["reason"]
                            .as_str()
                            .unwrap_or("device owner unavailable")
                            .to_string();
                        let reply =
                            self.device_reply(&change, "failed_device", Some(reason.clone()));
                        self.finish_device(&change, "failed_device", Some(reason));
                        return Ok(reply);
                    }
                    let session = owner.unwrap().session();
                    self.provider.close_brain_audio();
                    self.brain = None;
                    self.source_media.clear_brain();
                    change.sent = true;
                    self.device_messages.push_back((session, serde_json::json!({"contract":"GP15-device","version":1,"kind":"apply_configuration","ticket":change.ticket,"config":change.config})));
                    let reply = self.device_reply(&change, "accepted_intent", None);
                    self.device_pending = Some(change);
                    return Ok(reply);
                }
                let reply = self.device_reply(&change, "pending", None);
                self.device_pending = Some(change);
                Ok(reply)
            }
            _ => Err("Brain device command unsupported".into()),
        }
    }
    fn update_brain_readiness(&mut self, now: u64) {
        let snapshot = self.provider.brain_snapshot();
        let device = self.device_observation.as_ref().filter(|o| {
            self.device_observed_ms
                .is_some_and(|t| now >= t && now - t <= 250)
                && o.status.get("armed").and_then(Value::as_bool) == Some(true)
                && o.status.get("fault").is_none_or(Value::is_null)
        });
        let valid = self.brain.as_ref().filter(|b| {
            b.context.check_current().is_ok()
                && b.descriptor.stagebox_epoch == self.provider.source_epoch()
                && b.descriptor.stagebox_map == self.provider.topology().map_revision
                && device.is_some_and(|o| {
                    o.brain_epoch == b.descriptor.brain_epoch
                        && o.brain_map == b.descriptor.brain_map
                })
        });
        let talkback = valid.is_some_and(|b| {
            let Some(status) = self.source_media.talkback_status else {
                return false;
            };
            b.descriptor.talkback
                && status.ready
                && status.armed
                && status.fault.is_none()
                && snapshot.held_generation.map(|g| g.0) == Some(b.descriptor.hold_generation)
                && self.provider.brain_media_authorized(now)
        });
        let monitor = valid.is_some_and(|b| {
            b.descriptor.monitor
                && b.descriptor.selection_generation == snapshot.selection_generation.0
        }) && snapshot.monitor_armed
            && !snapshot.monitor_mute
            && device
                .and_then(|o| o.status.get("monitor_bridge"))
                .is_some_and(|s| {
                    valid.is_some_and(|b| brain_monitor_bridge_current(s, &b.descriptor))
                });
        self.provider.set_brain_path_readiness(talkback, monitor);
    }
    fn prepare_brain_block(&mut self, frame: u64) {
        let snapshot = self.provider.brain_snapshot();
        let Some(brain) = &mut self.brain else {
            let _ = self.source_media.render_talkback(frame, false, false);
            return;
        };
        if brain.context.check_current().is_err()
            || brain.descriptor.stagebox_epoch != self.provider.source_epoch()
            || brain.descriptor.stagebox_map != self.provider.topology().map_revision
        {
            self.brain = None;
            self.source_media.clear_brain();
            self.provider.close_brain_audio();
            return;
        }
        if !self
            .source_media
            .render_talkback(
                frame,
                brain.descriptor.talkback,
                snapshot.held_generation.map(|v| v.0) == Some(brain.descriptor.hold_generation),
            )
            .unwrap_or(false)
        {
            self.provider.close_brain_audio();
        }
    }
    fn emit_brain_monitor(&mut self, frame: u64, now: u64) -> Result<()> {
        let snapshot = self.provider.brain_snapshot();
        let Some(brain) = self.brain.as_ref() else {
            return Ok(());
        };
        let retiring = brain.monitor_retirement.is_some()
            || snapshot.selection_generation.0 != brain.descriptor.selection_generation;
        // A silent tail is permitted only for a still-live duplex owner. It is
        // not a grace period for revoked permissions, stale device telemetry,
        // or replaced source/device identities. Disconnect clears device_owner.
        if retiring && brain.descriptor.monitor {
            let live = brain
                .context
                .require(&Permission::LocalOperatorMonitor)
                .is_ok()
                && self.device_owner.as_ref().is_some_and(|owner| {
                    owner == &brain.context
                        && owner.require(&Permission::LocalOperatorMonitor).is_ok()
                })
                && brain.descriptor.stagebox_epoch == self.provider.source_epoch()
                && brain.descriptor.stagebox_map == self.provider.topology().map_revision
                && self
                    .device_observed_ms
                    .is_some_and(|t| now >= t && now - t <= 250)
                && self.device_observation.as_ref().is_some_and(|o| {
                    o.brain_epoch == brain.descriptor.brain_epoch
                        && o.brain_map == brain.descriptor.brain_map
                        && o.status.get("armed").and_then(Value::as_bool) == Some(true)
                        && o.status.get("fault").is_none_or(Value::is_null)
                });
            let expired = brain.monitor_retirement.is_some_and(|r| {
                now < r.first_ms
                    || now - r.first_ms >= 250
                    || frame < r.first_frame
                    || frame - r.first_frame >= 12_000
            });
            if !live || expired {
                self.brain = None;
                self.source_media.clear_brain();
                self.provider.close_brain_audio();
                return Ok(());
            }
        }
        let brain = self.brain.as_mut().unwrap();
        if !brain.descriptor.monitor {
            return Ok(());
        }
        if retiring && brain.monitor_retirement.is_none() {
            // The consumer silences retained blocks before encoding. Preserve
            // source indices and queue order across this retirement boundary.
            brain.monitor_retirement = Some(MonitorRetirement {
                first_ms: now,
                first_frame: frame,
            });
        }
        let mut samples = [0.; 96];
        if !retiring {
            samples.copy_from_slice(self.provider.brain_monitor_output());
        }
        if !frame.is_multiple_of(48)
            || frame.checked_add(48).is_none()
            || samples
                .iter()
                .any(|s| !s.is_finite() || s.abs() > f32::MAX as f64)
        {
            return Err("Brain packet shape".into());
        }
        if self.source_media.offer_monitor(frame, samples)? {
            self.media_stats.emitted_monitor_packets =
                self.media_stats.emitted_monitor_packets.saturating_add(1);
        } else {
            // Never evict storage owned by the consumer. Overflow is observable
            // and fails clean-run acceptance; a later gap closes the peer ASRC.
            self.media_stats.dropped_monitor_packets =
                self.media_stats.dropped_monitor_packets.saturating_add(1);
        }
        Ok(())
    }
    pub fn brain_descriptor(&self) -> Option<&BrainMediaDescriptor> {
        self.brain.as_ref().map(|b| &b.descriptor)
    }
    pub fn brain_bridge_status(&self) -> Option<crate::brain_audio::bridge::BridgeStatus> {
        self.brain.as_ref()?;
        self.source_media.talkback_status
    }
    fn negotiate_brain(&mut self, context: &AuthenticatedContext, payload: Value) -> Result<Value> {
        self.negotiate_brain_prepared(context, payload, || {})
    }
    fn negotiate_brain_prepared(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        after_preparation: impl FnOnce(),
    ) -> Result<Value> {
        if self.analysis_owner == Some(context.session()) {
            return Err("analysis connection cannot attach duplex".into());
        }
        if self.source_media.ingress_stopped {
            return Err("source ingress stopped".into());
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Negotiation {
            contract: String,
            version: u32,
            kind: String,
            writer: String,
            descriptor: BrainMediaDescriptor,
        }
        let request: Negotiation = serde_json::from_value(payload).map_err(|e| e.to_string())?;
        if request.contract != "GP15-media"
            || request.version != 1
            || request.kind != "negotiate"
            || request.writer != context.writer()
        {
            return Err("Brain media envelope".into());
        }
        let descriptor = request.descriptor;
        descriptor.validate(context.session(), &self.identity())?;
        if descriptor.talkback {
            context.require(&Permission::TalkbackDestinations)?;
        }
        if descriptor.monitor {
            context.require(&Permission::LocalOperatorMonitor)?;
        }
        if let Some(old) = &self.brain {
            if old.context.session() != context.session() {
                return Err("Brain duplex media owner attached".into());
            }
            if descriptor == old.descriptor {
                return Ok(
                    serde_json::json!({"contract":"GP15-media","version":1,"state":"accepted","descriptor":descriptor}),
                );
            }
            if descriptor.generation <= old.descriptor.generation {
                return Err("Brain media generation stale".into());
            }
        }
        if self
            .brain_generations
            .get(&context.session())
            .is_some_and(|g| descriptor.generation <= *g)
        {
            return Err("retired Brain media generation".into());
        }
        if self.brain_generations.len() >= MAX_CONNECTIONS
            && !self.brain_generations.contains_key(&context.session())
        {
            return Err("Brain session generation capacity".into());
        }
        if self
            .device_owner
            .as_ref()
            .is_some_and(|owner| owner != context)
        {
            return Err("Brain media/device owner mismatch".into());
        }
        if self.device_observation.as_ref().is_some_and(|o| {
            descriptor.brain_epoch != o.brain_epoch || descriptor.brain_map != o.brain_map
        }) {
            return Err("Brain device epoch/map readback mismatch".into());
        }
        let snapshot = self.provider.brain_snapshot();
        if descriptor.monitor && descriptor.selection_generation != snapshot.selection_generation.0
        {
            return Err("Brain monitor selection stale".into());
        }
        if descriptor.talkback
            && descriptor.hold_generation != snapshot.held_generation.map_or(0, |v| v.0)
        {
            return Err("Brain talkback hold stale".into());
        }
        let epochs = BridgeEpochs {
            source: descriptor.brain_epoch,
            destination: descriptor.stagebox_epoch,
            route: descriptor.generation,
        };
        let preserve = self.brain.as_ref().is_some_and(|old| {
            old.context == *context
                && self.source_media.talkback_fence_current
                && old.descriptor.brain_epoch == descriptor.brain_epoch
                && old.descriptor.stagebox_epoch == descriptor.stagebox_epoch
                && old.descriptor.brain_map == descriptor.brain_map
                && old.descriptor.stagebox_map == descriptor.stagebox_map
                && old.descriptor.hold_generation == descriptor.hold_generation
                && old.descriptor.talkback == descriptor.talkback
                && self
                    .source_media
                    .talkback_status
                    .is_some_and(|status| status.fault.is_none())
        });
        let talkback_generation = if preserve {
            self.media_ingress.talkback_generation
        } else {
            self.media_ingress
                .talkback_generation
                .checked_add(1)
                .ok_or("talkback generation exhausted")?
        };
        let last_queued_talkback = if preserve {
            self.media_ingress
                .talkback
                .last(talkback_generation)
                .or_else(|| self.brain.as_ref().and_then(|b| b.last_queued_talkback))
        } else {
            None
        };
        let prepared = if preserve {
            None
        } else {
            Some(Bridge::prepare(BridgeConfig::voice(1), epochs).map_err(|e| e.to_string())?)
        };
        // Talkback hold changes do not change monitor wire identity. Retain its
        // queued source frames independently of talkback bridge replacement; a
        // gap here would correctly fault the already-running monitor ASRC.
        // All fallible validation/preparation above precedes taking old state.
        let preserve_monitor = self.brain.as_ref().is_some_and(|old| {
            old.context == *context
                && old.descriptor.monitor
                && descriptor.monitor
                && old.descriptor.session == descriptor.session
                && old.descriptor.stagebox_epoch == descriptor.stagebox_epoch
                && old.descriptor.brain_epoch == descriptor.brain_epoch
                && old.descriptor.stagebox_map == descriptor.stagebox_map
                && old.descriptor.brain_map == descriptor.brain_map
                && old.descriptor.selection_generation == descriptor.selection_generation
                && old.descriptor.sample_rate == descriptor.sample_rate
                && old.descriptor.frames == descriptor.frames
        });
        let (new_monitor, new_outgoing) = if preserve_monitor {
            (None, None)
        } else {
            let (producer, consumer) = prepare_monitor_lane();
            (Some(producer), Some(MonitorConsumer::new(consumer)))
        };
        let new_talkback = prepared.map(|bridge| TalkbackSource {
            bridge,
            epochs,
            last_received: None,
        });
        let handoff = std::sync::Arc::new(BrainHandoffState::new(descriptor.hold_generation));
        after_preparation();
        // Reserve only after ASRC/filter and lane preparation. Those controller
        // allocations must not spend an already-issued source commit window.
        let boundary = if self.source_clock_status.is_some() {
            self.source_media.execute(0, |source, frame| {
                *frame = source.clock_frame.saturating_add(480);
            })?
        } else {
            self.provider.frame()
        };
        // All replaced native/ring/fence storage stays in the return packet.
        let packet = self.source_media.executor.transact_until(
            (
                new_monitor,
                new_talkback,
                Some(context.audio_fence()),
                talkback_generation,
                preserve,
                boundary,
                self.source_clock_status.clone(),
                false,
                descriptor
                    .monitor
                    .then_some(descriptor.selection_generation),
                Some(handoff.clone()),
            ),
            |source, p| {
                if let Some(status) = &p.6 {
                    if status.stopped() || source.clock_frame > p.5 {
                        return true;
                    }
                    if source.clock_frame < p.5 {
                        return false;
                    }
                }
                if !p.2.as_ref().is_some_and(|f| f.valid()) {
                    return true;
                }
                if p.0.is_some() {
                    std::mem::swap(&mut source.monitor, &mut p.0);
                }
                if p.1.is_some() {
                    std::mem::swap(&mut source.talkback, &mut p.1);
                }
                std::mem::swap(&mut source.talkback_fence, &mut p.2);
                if let Some(old) = &source.brain_handoff {
                    old.close();
                }
                std::mem::swap(&mut source.brain_handoff, &mut p.9);
                source.talkback_generation = p.3;
                source.clock_monitor_generation = p.8;
                source.clock_monitor_retirement = None;
                if !p.4 {
                    source.talkback_deadline_ms = 0;
                }
                p.7 = true;
                true
            },
        )?;
        if !packet.7 {
            return Err("Brain media preparation expired or authority closed".into());
        }
        drop(packet);
        let old = self.brain.take();
        let monitor_retirement = if preserve_monitor {
            old.as_ref().and_then(|old| old.monitor_retirement)
        } else {
            None
        };
        let outgoing = match new_outgoing {
            Some(outgoing) => outgoing,
            None => old
                .as_ref()
                .expect("preserved monitor owner")
                .outgoing
                .clone(),
        };
        self.source_media.execute((), |_, _| {})?;
        self.media_ingress.talkback_generation = talkback_generation;
        self.brain_generations
            .insert(context.session(), descriptor.generation);
        self.brain = Some(BrainMediaState {
            context: context.clone(),
            descriptor: descriptor.clone(),
            outgoing,
            handoff,
            exported: false,
            monitor_retirement,
            last_queued_talkback,
        });
        Ok(
            serde_json::json!({"contract":"GP15-media","version":1,"state":"accepted","descriptor":descriptor}),
        )
    }
    #[cfg(test)]
    pub(crate) fn test_pending_accepted_wet_receipts(&mut self) -> usize {
        let receipts = &mut self.media_ingress.wet_receipts;
        let count = receipts.slots();
        let chunk = receipts.read_chunk(count).unwrap();
        let (first, second) = chunk.as_slices();
        // Peek only: retain every receipt for normal controller accounting.
        first
            .iter()
            .chain(second)
            .filter(|receipt| receipt.result == WetAdmission::Accepted)
            .count()
    }
    #[cfg(test)]
    pub(crate) fn test_pending_wet_receipt_kinds(&mut self) -> [usize; 3] {
        let receipts = &mut self.media_ingress.wet_receipts;
        let count = receipts.slots();
        let chunk = receipts.read_chunk(count).unwrap();
        let (first, second) = chunk.as_slices();
        let mut counts = [0; 3];
        for receipt in first.iter().chain(second) {
            counts[match receipt.result {
                WetAdmission::Accepted => 0,
                WetAdmission::Late => 1,
                WetAdmission::Rejected => 2,
            }] += 1;
        }
        counts
    }
    #[cfg(test)]
    pub(crate) fn test_drain_media_ingress(&mut self, now_ms: u64) {
        let _ = self.source_media.drain_ingress(
            self.provider.source_epoch(),
            self.provider.frame(),
            now_ms,
        );
        self.finish_media_ingress();
    }
    #[cfg(test)]
    pub(crate) fn test_admitted_wet(&self) -> &[f64; 96] {
        &self.source_media.wet
    }
    /// Bounded source closure retains all endpoint storage for the controller.
    fn close_source_egress(&mut self) {
        self.source_media.close_egress();
    }
    /// Final controller-side retirement after the source and mailbox producers
    /// stop. Preserve completed insertion truth; reject/count queued admissions
    /// without rendering, and do not affect separately owned egress shutdown.
    pub(crate) fn finish_source_ingress(&mut self) {
        self.media_ingress.talkback.close();
        if let Some(brain) = &self.brain {
            brain.handoff.close();
        }
        self.media_ingress
            .wet
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .closed = true;
        let retired = self.source_media.execute((None, None, None), |source, p| {
            source.ingress_stopped = true;
            std::mem::swap(&mut source.ingress, &mut p.0);
            std::mem::swap(&mut source.wet_fence, &mut p.1);
            std::mem::swap(&mut source.talkback_fence, &mut p.2);
            source.talkback_deadline_ms = 0;
            if let Some(talkback) = &mut source.talkback {
                talkback.bridge.invalidate();
            }
            source.talkback_samples.fill(0.);
        });
        self.source_media.observed.ingress_stopped = true;
        self.finish_media_ingress();
        if let Ok((Some(mut ingress), _, _)) = retired {
            for _ in 0..WET_INGRESS_PACKETS {
                if ingress.wet.pop().is_err() {
                    break;
                }
                self.media_stats.rejected_wet_packets =
                    self.media_stats.rejected_wet_packets.saturating_add(1);
            }
            for _ in 0..TALKBACK_INGRESS_BLOCKS {
                if ingress.talkback.pop().is_err() {
                    break;
                }
                self.media_stats.rejected_talkback_packets =
                    self.media_stats.rejected_talkback_packets.saturating_add(1);
            }
        }
    }
    /// Controller-only teardown after source execution has stopped.
    pub fn finish_source_egress(&mut self) {
        self.retire_analysis();
        self.close_source_egress();
        self.source_egress.finish();
        // The worker is joined before endpoint/ring/sample storage is destroyed.
        self.source_media.retire_egress();
    }
    #[cfg(test)]
    pub(crate) fn wait_source_encoded(&self) {
        self.source_egress.wait_encoded();
    }
    #[cfg(test)]
    pub(crate) fn pause_source_encoder(&self, paused: bool) {
        self.source_egress.pause(paused);
    }
    pub fn source_egress_report(&self) -> Value {
        let mut report = self.source_egress.report(self.retired_egress);
        report["independent_analysis"] = self.analysis_egress.report(self.retired_analysis);
        report
    }
    /// Snapshot numeric layout counts on the owner, serialize on the controller.
    /// Each lane/allocation is counted once, independent of its shared endpoints.
    pub fn prepared_media_storage(&mut self) -> Result<Value> {
        let sizes = self.source_media.execute([0usize; 7], |source, sizes| {
            sizes[0] = std::mem::size_of::<HostSourceMedia>();
            sizes[1] = WET_INGRESS_PACKETS
                * (std::mem::size_of::<WetIngress>() + std::mem::size_of::<WetReceipt>())
                + TALKBACK_INGRESS_BLOCKS
                    * (std::mem::size_of::<TalkbackIngress>()
                        + std::mem::size_of::<TalkbackReceipt>());
            sizes[2] = usize::from(source.monitor.is_some())
                * MONITOR_BLOCKS
                * std::mem::size_of::<MonitorBlock>();
            sizes[3] = source.wet_groups.capacity() * std::mem::size_of::<WetGroup>()
                + source
                    .wet_groups
                    .iter()
                    .map(|g| g.buffer.prepared_storage().heap_bytes)
                    .sum::<usize>();
            sizes[4] = source
                .talkback
                .as_ref()
                .map_or(0, |s| s.bridge.prepared_sample_bytes());
            sizes[5] = source.analysis_indices.capacity() * std::mem::size_of::<usize>()
                + (source.analysis.capacity() + source.fx_send.capacity())
                    * std::mem::size_of::<f64>();
            sizes[6] = source.observer.as_ref().map_or(0, |a| {
                a.indices.capacity() * std::mem::size_of::<usize>()
                    + a.samples.capacity() * std::mem::size_of::<f64>()
            });
        })?;
        let egress = self.source_egress_report();
        let encoders = egress["current_prepared_requested_bytes"]
            .as_u64()
            .unwrap_or(0)
            + egress["independent_analysis"]["current_prepared_requested_bytes"]
                .as_u64()
                .unwrap_or(0);
        let total = sizes
            .iter()
            .try_fold(encoders, |sum, size| sum.checked_add(*size as u64))
            .ok_or("prepared media storage overflow")?;
        Ok(
            serde_json::json!({"source_inline":sizes[0],"ingress_and_receipt_lanes":sizes[1],
            "monitor_lane":sizes[2],"wet_groups":sizes[3],"talkback_bridge_samples":sizes[4],
            "source_scratch":sizes[5],"analysis_scratch":sizes[6],"encoder_pools_and_lanes":encoders,
            "known_total_bytes":total,"global_executor":crate::source_executor::prepared_queue_storage(),
            "excludes":"render graph and owner libraries, prepared command payloads, ASRC filter library, controller state, ring/Arc metadata, TLS/socket buffers, stacks, allocator overhead; process RSS accounts for resident aggregate"}),
        )
    }
    pub fn media_stats(&self) -> &HostMediaStats {
        &self.media_stats
    }
    pub fn accepted_wet_sha256(&self) -> String {
        crate::hash::hex(self.accepted_wet_hash.clone().finalize())
    }
    pub fn provider(&self) -> &LocalAudio {
        &self.provider
    }
    pub fn provider_mut(&mut self) -> &mut LocalAudio {
        &mut self.provider
    }
    pub fn descriptor(&self) -> Option<&MediaDescriptor> {
        self.descriptor.as_ref()
    }
    /// Source worker entry, not a socket/TLS callback. The only timeline advance
    /// is inside the same production provider used by local/headless adapters.
    pub fn process_source(
        &mut self,
        now_ms: u64,
        epoch: u64,
        frame: u64,
        capture: &[f64],
        playback: &mut [f64],
    ) -> Result<()> {
        if self.source_clock_status.is_some() {
            return Err("stop source clock before Brain negotiation".into());
        }
        playback.fill(0.);
        if self.source_media.ingress_stopped {
            return Err("source ingress closed".into());
        }
        if self.observed_source_epoch != self.provider.source_epoch() {
            self.clear_media_history();
            self.observed_source_epoch = self.provider.source_epoch();
        }
        if epoch != self.provider.source_epoch() || frame != self.provider.frame() {
            self.clear_media_history();
            self.provider.quiesce_source("source_discontinuity")?;
            return Err("source timeline".into());
        }
        if self.descriptor.as_ref().is_some_and(|d| {
            d.source_epoch.0 != epoch || d.map_generation.0 != self.provider.topology().map_revision
        }) {
            self.clear_media_history();
        }
        if self.source_media.drain_ingress(epoch, frame, now_ms)? {
            self.provider.close_brain_audio();
        }
        self.finish_media_ingress();
        let safety =
            std::array::from_fn(|i| self.provider.engine().output_safety_gain(frame + i as u64));
        if let Err(error) = self.source_media.render_wet(frame, safety)? {
            return match error {
                WetRenderError::Timeline => {
                    self.provider.quiesce_source("wet_source_timeline")?;
                    Err("wet source timeline".into())
                }
                WetRenderError::Packet(error) => Err(format!("wet media {error:?}")),
            };
        }
        for sample in self.source_media.wet {
            self.media_stats.nonzero_rendered_wet_samples += u64::from(sample != 0.);
            self.media_stats.rendered_wet_energy += sample * sample;
        }
        // Always supply the explicit external wet path, including silence after
        // disconnect. This prevents fallback to another local FX instance.
        self.tick_fx(now_ms)?;
        self.tick_aux(now_ms)?;
        self.commit_device_intent(now_ms)?;
        self.prepare_brain_block(frame);
        self.provider.tick_with_capture_and_brain(
            now_ms,
            epoch,
            frame,
            capture,
            playback,
            Some(&self.source_media.wet),
            Some(&self.source_media.talkback_samples),
        )?;
        self.emit_brain_monitor(frame, now_ms)?;
        self.update_brain_readiness(now_ms);
        for reply in self.provider.take_remote_completions() {
            if self.completions.len() == 128 {
                return Err("remote final queue capacity".into());
            }
            self.completions.push_back(reply);
        }
        if self.analysis_owner.is_some() {
            let width = self.provider.topology().capture_channels;
            let map = self.provider.topology().map_revision;
            self.source_media.execute(
                (capture.to_vec(), epoch, map, frame, width),
                |source, p| {
                    if let Some(observer) = &mut source.observer {
                        observer.offer(p.1, p.2, p.3, &p.0, p.4);
                    }
                },
            )?;
        }
        if let Some(descriptor) = &self.descriptor {
            if descriptor.source_epoch.0 != epoch
                || descriptor.map_generation.0 != self.provider.topology().map_revision
            {
                self.descriptor = None;
                self.source_media.clear_wet(false);
                self.close_source_egress();
                return Ok(());
            }
            let capture_channels = self.provider.topology().capture_channels;
            if !self.source_media.offer_egress(
                frame,
                capture,
                capture_channels,
                *self.provider.fx_aux_samples(),
                self.provider
                    .brain_fx_send()
                    .try_into()
                    .map_err(|_| "FX send observation shape")?,
                descriptor.channels(MediaRole::FxSend) == 8,
            )? {
                self.media_stats.source_egress_dropped_blocks = self
                    .media_stats
                    .source_egress_dropped_blocks
                    .saturating_add(1);
            }
        }
        Ok(())
    }
}
impl AuthorityEndpoint for HostAuthority {
    fn identity(&self) -> EngineIdentity {
        EngineIdentity {
            source_epoch: Counter(self.provider.source_epoch()),
            capability_generation: Counter(self.capability_generation),
            map_generation: Counter(self.provider.topology().map_revision),
        }
    }
    fn dispatch(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        now_ms: u64,
    ) -> Result<Option<Value>> {
        context.check_writer(&payload)?;
        if self.analysis_owner == Some(context.session()) {
            return Err("analysis connection permits media observation only".into());
        }
        if self.source_clock_status.is_some()
            && self.completions.len() + self.provider.clocked_completion_reservation() > 128
        {
            return Err("remote final queue capacity".into());
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP-SPECTRUM") {
            if self
                .meter_reads
                .get(&context.session())
                .is_some_and(|t| now_ms.saturating_sub(*t) < 80)
            {
                return Err("spectrum observer poll bound".into());
            }
            let request: crate::spectrum::Request =
                serde_json::from_value(payload).map_err(|e| e.to_string())?;
            self.meter_reads.insert(context.session(), now_ms);
            return serde_json::to_value(self.provider.spectrum_snapshot(&request)?)
                .map(Some)
                .map_err(|e| e.to_string());
        }

        if payload.get("contract").and_then(Value::as_str)
            == Some(crate::remote::fx_aux_owner::CONTRACT)
        {
            return self.dispatch_aux_owner(context, payload, now_ms).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::fx_aux_wire::CONTRACT) {
            if !self.provider.fx_aux_enabled() {
                return self.dispatch_aux(context, payload, now_ms).map(Some);
            }
            let (_, mutation) =
                crate::fx_aux_wire::request(&payload, self.provider.engine_mut().wire_version())?;
            if mutation.is_some() {
                context.require(&Permission::FxConfiguration)?;
            }
            let fresh = self
                .fx_aux_reads
                .get(&context.session())
                .is_some_and(|t| now_ms.saturating_sub(*t) <= 1000);
            let read = mutation.is_none();
            let reply = if self.source_clock_status.is_some() {
                if self.aux_mode() {
                    return Err("autonomous remote aux lifecycle unavailable".into());
                }
                self.provider
                    .clocked_control(now_ms, Some(context.audio_fence()), |provider| {
                        provider.fx_aux_request(payload, now_ms, fresh, None)
                    })?
            } else {
                self.provider.fx_aux_request(payload, now_ms, fresh, None)?
            };
            if read {
                self.fx_aux_reads.insert(context.session(), now_ms);
            }
            return Ok(Some(reply));
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::fx_wire::CONTRACT) {
            return self.dispatch_fx(context, payload, now_ms).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::meter_wire::CONTRACT) {
            let request = crate::meter_wire::Request::decode(&super::encode(&payload)?)?;
            if self.pending_meter_reads.contains_key(&context.session()) {
                return Err("meter observer read pending".into());
            }
            if self
                .meter_reads
                .get(&context.session())
                .is_some_and(|t| now_ms.saturating_sub(*t) < 40)
            {
                // Client cadence includes connection setup and network jitter.
                // Keep the 40ms production bound without forcing a reconnect
                // loop: at most one pending read per authenticated session.
                if self.pending_meter_reads.len() >= MAX_CONNECTIONS {
                    return Err("meter observer pending capacity".into());
                }
                self.pending_meter_reads.insert(context.session(), request);
                return Ok(None);
            }
            self.meter_reads.insert(context.session(), now_ms);
            let reply = self
                .provider
                .meter_snapshot(&request, self.capability_generation)?;
            return serde_json::to_value(reply)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str)
            == Some(crate::lease_maintenance::CONTRACT)
        {
            use crate::lease_maintenance::{Reason, Reply, Request};
            let request = Request::decode(&super::encode(&payload)?)?;
            let result = (|| {
                if request.authenticated_session.0 != context.session()
                    || request.writer != context.writer
                    || request.epoch != self.identity().source_epoch
                    || request.show_id != self.provider.show_id()
                    || request.capability_generation != self.identity().capability_generation
                    || request.map_generation != self.identity().map_generation
                {
                    return Err(Reason::Identity);
                }
                if !request.supported_scope() {
                    return Err(Reason::Scope);
                }
                context
                    .require(&scope_permission(request.scope))
                    .map_err(|_| Reason::Permission)?;
                self.provider
                    .maintain_lease(&request, now_ms, self.capability_generation)
            })();
            // Maintenance is never a readback admission or a heartbeat witness.
            let bytes = Reply::new(request, result).encode()?;
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::paired_readback::CONTRACT)
        {
            use crate::paired_readback::{Reason, Reply, Request};
            let request = Request::decode(&super::encode(&payload)?)?;
            let result = (|| {
                if request.authenticated_session.0 != context.session()
                    || request.writer != context.writer
                    || request.epoch != self.identity().source_epoch
                    || request.show_id != self.provider.show_id()
                    || request.capability_generation != self.identity().capability_generation
                    || request.map_generation != self.identity().map_generation
                {
                    return Err(Reason::Identity);
                }
                if self
                    .paired_queries
                    .get(&context.session())
                    .is_some_and(|last| request.query_id.0 <= *last)
                {
                    return Err(Reason::QueryId);
                }
                if (!self.paired_queries.contains_key(&context.session())
                    && self.paired_queries.len() >= MAX_CONNECTIONS)
                    || (!self.reads.contains_key(&context.session())
                        && self.reads.len() >= MAX_CONNECTIONS)
                {
                    return Err(Reason::Capacity);
                }
                // A read nonce is single-use after authenticated attachment and
                // bounded namespace admission, even when observation later fails.
                // It is not a freshness latch and stores no response body.
                self.paired_queries
                    .insert(context.session(), request.query_id.0);
                self.provider
                    .paired_readback(&request, now_ms, self.capability_generation)
            })();
            return self
                .finish_paired_readback(context, Reply::new(request, result), now_ms)
                .map(Some);
        }
        if self.reads.len() >= MAX_CONNECTIONS && !self.reads.contains_key(&context.session()) {
            return Err("remote read-state capacity".into());
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::held_proof::CONTRACT) {
            use crate::held_proof::{Reason, Reply, Request};
            let request = Request::decode(&super::encode(&payload)?)?;
            let result = (|| {
                if request.authenticated_session.0 != context.session()
                    || request.writer != context.writer
                    || request.capability_generation != self.identity().capability_generation
                    || request.map_generation != self.identity().map_generation
                {
                    return Err(Reason::Identity);
                }
                if request.scope != crate::control_model::Scope::TalkbackDestinations {
                    return Err(Reason::Scope);
                }
                context
                    .require(&Permission::TalkbackDestinations)
                    .map_err(|_| Reason::Permission)?;
                if self
                    .held_queries
                    .get(&context.session())
                    .is_some_and(|last| request.query_id.0 <= *last)
                {
                    return Err(Reason::QueryId);
                }
                if !self.held_queries.contains_key(&context.session())
                    && self.held_queries.len() >= MAX_CONNECTIONS
                {
                    return Err(Reason::Capacity);
                }
                let witness =
                    self.provider
                        .held_proof(&request, now_ms, self.capability_generation)?;
                Ok(witness)
            })();
            let accepted = result.is_ok();
            let query_id = request.query_id.0;
            let reply = Reply::new(request, result);
            reply.validate()?;
            let bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
            if bytes.len() > crate::held_proof::MAX_BYTES {
                return Err("held proof response capacity".into());
            }
            if accepted {
                self.held_queries.insert(context.session(), query_id);
                self.brain_reads.insert(context.session(), now_ms);
            }
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP15-device") {
            return self.dispatch_device(context, payload, now_ms).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP15-media") {
            return self.negotiate_brain(context, payload).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP15-brain") {
            let request = crate::brain_control::Request::decode(&super::encode(&payload)?)?;
            let snapshot = matches!(
                request.command,
                crate::brain_control::Command::BrainSnapshot {}
            );
            if !snapshot {
                context.require(&scope_permission(request.scope()))?;
                if self
                    .provider
                    .engine_mut()
                    .writer_scope(&request.authority_request(), now_ms)
                    != Some(request.scope())
                {
                    return Err("Brain scoped lease required".into());
                }
            }
            let fresh = self
                .brain_reads
                .get(&context.session())
                .is_some_and(|t| now_ms >= *t && now_ms - *t <= 250);
            let reply = if self.source_clock_status.is_some() {
                self.provider
                    .clocked_control(now_ms, Some(context.audio_fence()), |provider| {
                        serde_json::to_value(provider.brain_request(request, now_ms, fresh, None)?)
                            .map_err(|e| e.to_string())
                    })?
            } else {
                serde_json::to_value(self.provider.brain_request(request, now_ms, fresh, None)?)
                    .map_err(|e| e.to_string())?
            };
            if snapshot && !reply["snapshot"].is_null() {
                self.brain_reads.insert(context.session(), now_ms);
            }
            return serde_json::to_value(reply)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP05-modules") {
            let request = crate::module_wire::ModuleRequest::decode(&super::encode(&payload)?)?;
            if request.version != self.provider.engine_mut().wire_version() {
                return Err("module version incompatible with admitted topology".into());
            }
            if !matches!(
                request.command,
                crate::module_wire::ModuleCommand::ModuleStatus {}
            ) {
                if !self
                    .reads
                    .get(&context.session())
                    .is_some_and(ReadState::audio_ready)
                {
                    return Err("remote snapshot required before recorder mutation".into());
                }
                context.require(&Permission::Foh)?;
                if self
                    .provider
                    .engine_mut()
                    .writer_scope(&request.authority_request(), now_ms)
                    != Some(crate::control_model::Scope::Foh)
                {
                    return Err("recorder FOH lease required".into());
                }
            }
            return self
                .provider
                .dispatch_module(request, now_ms, None)
                .map(Some);
        }
        if matches!(
            payload.get("contract").and_then(Value::as_str),
            Some("GP14-structure" | "GP18-master-eq")
        ) {
            let eq = payload["contract"] == crate::master_eq_wire::CONTRACT;
            let request = crate::structural_control::Request::decode(&super::encode(&payload)?)?;
            if let Some(scope) = request.scope() {
                context.require(&scope_permission(scope))?;
                if self
                    .provider
                    .engine_mut()
                    .writer_scope(&request.authority_request(), now_ms)
                    != Some(scope)
                {
                    return Err("structural scoped lease required".into());
                }
            }
            let fresh = (if eq {
                &self.master_eq_reads
            } else {
                &self.structural_reads
            })
            .get(&context.session())
            .is_some_and(|t| now_ms >= *t && now_ms - *t <= 250);
            let snapshot = matches!(
                request.command,
                crate::structural_control::Command::StructuralSnapshot {}
                    | crate::structural_control::Command::MasterEqSnapshot {}
            );
            let reply = if self.source_clock_status.is_some() {
                self.provider
                    .clocked_control(now_ms, Some(context.audio_fence()), |provider| {
                        serde_json::to_value(
                            provider.structural_request(request, now_ms, fresh, None)?,
                        )
                        .map_err(|e| e.to_string())
                    })?
            } else {
                serde_json::to_value(
                    self.provider
                        .structural_request(request, now_ms, fresh, None)?,
                )
                .map_err(|e| e.to_string())?
            };
            if snapshot {
                if eq && !reply["master_eq"].is_null() {
                    self.master_eq_reads.insert(context.session(), now_ms);
                } else if !reply["snapshot"].is_null() {
                    self.structural_reads.insert(context.session(), now_ms);
                }
            }
            return serde_json::to_value(reply)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        let state = self.reads.entry(context.session()).or_default();
        if self.source_clock_status.is_some() {
            // Ordinary engine preparation has a future source reservation; it
            // cannot hold the graph clock while JSON/coefficients are prepared.
            if self.completions.len() + self.provider.clocked_completion_reservation() > 128 {
                return Err("remote final queue capacity".into());
            }
            let reply = self.provider.clocked_engine_command(
                now_ms,
                Some(context.audio_fence()),
                |engine| dispatch_engine(engine, context, payload, now_ms, state),
            )?;
            self.completions
                .extend(self.provider.take_remote_completions());
            Ok(Some(reply))
        } else {
            dispatch_engine(self.provider.engine_mut(), context, payload, now_ms, state).map(Some)
        }
    }
    fn negotiate(
        &mut self,
        context: &AuthenticatedContext,
        descriptor: &MediaDescriptor,
    ) -> Result<bool> {
        if self.source_media.ingress_stopped {
            return Err("source ingress stopped".into());
        }
        if !descriptor.streams.is_empty()
            && descriptor
                .streams
                .iter()
                .all(|s| s.role == MediaRole::Analysis)
        {
            return self.negotiate_analysis(context, descriptor);
        }
        if self.analysis_owner == Some(context.session()) {
            return Err("analysis connection cannot attach FX".into());
        }
        if descriptor.channels(MediaRole::Analysis) > 0 && self.analysis_owner.is_some() {
            return Err("analysis subscription already attached".into());
        }
        if self
            .media_owner
            .is_some_and(|session| session != context.session())
        {
            return Err("Brain media owner already attached".into());
        }
        if self.fx_control_attached(context.session()) || self.aux_mode() {
            return Err("FX owner replacement requires fresh session".into());
        }
        descriptor.validate(context, &self.identity(), crate::transport::MAX_DATAGRAM)?;
        if self.provider.fx_aux_enabled()
            && descriptor
                .streams
                .iter()
                .any(|s| matches!(s.role, MediaRole::FxSend | MediaRole::WetReturn))
        {
            return Err("local four-aux owner excludes legacy remote FX media".into());
        }
        if descriptor.streams.iter().any(|s| s.frames != 48) {
            return Err("composed host currently advertises 48-frame media blocks".into());
        }
        let ordered_ids = |role| {
            let mut streams: Vec<_> = descriptor
                .streams
                .iter()
                .filter(|s| s.role == role)
                .collect();
            streams.sort_by_key(|s| s.first_channel);
            streams
                .into_iter()
                .flat_map(|s| s.channel_ids.iter().cloned())
                .collect::<Vec<_>>()
        };
        let analysis_ids = ordered_ids(MediaRole::Analysis);
        let mut indices = Vec::new();
        for id in &analysis_ids {
            indices.push(
                self.provider
                    .topology()
                    .inputs
                    .iter()
                    .find(|input| &input.id == id)
                    .ok_or("unknown admitted raw source")?
                    .capture_slot,
            );
        }
        let send_ids = ordered_ids(MediaRole::FxSend);
        let return_ids = ordered_ids(MediaRole::WetReturn);
        let four_aux = send_ids == crate::remote::fx_aux_owner::channel_ids();
        if four_aux {
            context.require(&Permission::FxConfiguration)?;
            if return_ids != send_ids {
                return Err("four FX aux return map must match sends".into());
            }
        } else {
            for ids in [&send_ids, &return_ids] {
                if !ids.is_empty() && *ids != ["foh-left", "foh-right"] {
                    return Err("explicit stereo or four-aux FX ports required".into());
                }
            }
        }
        let wet_generation = self
            .media_ingress
            .wet_generation
            .checked_add(1)
            .ok_or("wet generation exhausted")?;
        let first = if self.source_clock_status.is_some() {
            self.provider.remote_target_frame(480)?
        } else {
            self.provider.frame()
        };
        let mut wet_groups = Vec::new();
        for stream in descriptor
            .streams
            .iter()
            .filter(|s| s.role == MediaRole::WetReturn)
        {
            let spec = stream.spec(context.session())?;
            let buffer =
                JitterBuffer::new(spec, first, (first / u64::from(spec.frames)) as u32, 32)
                    .map_err(|e| format!("wet admission {e:?}"))?;
            wet_groups.push(WetGroup {
                spec,
                first,
                buffer,
            });
        }
        let mut source_egress = super::source_egress::SourceEgress::prepare(
            descriptor.clone(),
            48 * indices.len(),
            if four_aux { 384 } else { 96 },
        )?;
        let mut sends = crate::mixer::PreparedFxSends::default();
        if four_aux {
            sends.enable(self.provider.topology())?;
            let zeros = vec![0.; self.provider.topology().inputs.len()];
            for bus in 0..4 {
                sends.set(zeros.len(), bus, &zeros)?;
            }
        }
        let wet_channels = if four_aux { 8 } else { 2 };
        let fx_send = vec![0.; 48 * wet_channels];
        let analysis = vec![0.; 48 * indices.len()];
        // Negotiation is controller work. Detach the single prepared producer;
        // source execution never calls into the controller's encoder owner.
        let source_egress_input = source_egress
            .take_input()
            .ok_or("prepared source egress endpoint missing")?;
        let prepared = PreparedMedia {
            wet_channels,
            fx_send,
            analysis,
            analysis_indices: indices,
            egress: Some(source_egress_input),
            wet_groups,
            wet_generation,
            wet_fence: Some(context.audio_fence()),
        };
        let mut packet = self.provider.source_pair_command(
            &self.source_media.executor,
            (
                prepared,
                first,
                self.source_clock_status.clone(),
                false,
                sends,
                four_aux,
            ),
            |graph, source, p| {
                if let Some(status) = &p.2 {
                    if status.stopped() || graph.mixer.mixer.frame() > p.1 {
                        return true;
                    }
                    if graph.mixer.mixer.frame() < p.1 {
                        return false;
                    }
                }
                if !p.0.wet_fence.as_ref().is_some_and(|f| f.valid()) {
                    return true;
                }
                if graph.closure.load(std::sync::atomic::Ordering::Acquire)
                    || !graph.mixer.mixer.accepts_fx_sends(&p.4)
                {
                    return true;
                }
                if !graph.mixer.mixer.apply_fx_sends(&mut p.4) {
                    return true;
                }
                if let Some(action) = &mut graph.mixer.remote_fx
                    && action.result.is_none()
                {
                    action.result = Some(false);
                }
                if let Some(routing) = &mut graph.clock_routing {
                    routing.remote_fx_aux = p.5;
                }
                let p = &mut p.0;
                std::mem::swap(&mut source.wet_channels, &mut p.wet_channels);
                std::mem::swap(&mut source.fx_send, &mut p.fx_send);
                std::mem::swap(&mut source.analysis, &mut p.analysis);
                std::mem::swap(&mut source.analysis_indices, &mut p.analysis_indices);
                std::mem::swap(&mut source.egress, &mut p.egress);
                std::mem::swap(&mut source.wet_groups, &mut p.wet_groups);
                std::mem::swap(&mut source.wet_generation, &mut p.wet_generation);
                std::mem::swap(&mut source.wet_fence, &mut p.wet_fence);
                source.gates = [WetGate::default(); 8];
                source.wet.fill(0.);
                // Successful swap is marked below outside the payload borrow.
                true
            },
        )?;
        // The old generation in the returned storage proves that the new one swapped.
        packet.3 = packet.0.wet_generation != wet_generation;
        if !packet.3 {
            return Err("media preparation expired or authority closed".into());
        }
        // The displaced endpoint returns before joining its encoder on the controller.
        drop(packet);
        self.source_egress.finish();
        self.retired_egress.add(self.source_egress.counts());
        self.source_egress = source_egress;
        self.provider.finish_remote_fx_aux_mode(four_aux);
        self.media_ingress.wet_generation = wet_generation;
        self.observed_source_epoch = self.provider.source_epoch();
        self.descriptor = Some(descriptor.clone());
        self.media_owner = Some(context.session());
        Ok(true)
    }
    fn media(
        &mut self,
        context: &AuthenticatedContext,
        packet: Packet<'_>,
        _now_ms: u64,
    ) -> Result<()> {
        if self.source_media.ingress_stopped {
            return Err("source ingress stopped".into());
        }
        context.require(&Permission::Fx)?;
        if self.media_owner != Some(context.session()) || packet.spec().role != Role::WetReturn {
            return Err("wet owner/direction".into());
        }
        let spec = packet.spec();
        if !self.descriptor.as_ref().is_some_and(|d| {
            d.streams.iter().any(|stream| {
                stream.role == MediaRole::WetReturn
                    && stream.spec(context.session()).ok() == Some(spec)
            })
        }) {
            return Err("wet stream identity".into());
        }
        let mut block = WetIngress {
            generation: self.media_ingress.wet_generation,
            length: packet.bytes().len(),
            bytes: [0; crate::transport::MAX_DATAGRAM],
        };
        block.bytes[..block.length].copy_from_slice(packet.bytes());
        let _ = self.media_ingress.wet.push(block);
        Ok(())
    }
    fn brain_media(
        &mut self,
        context: &AuthenticatedContext,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<()> {
        let result = (|| {
            if self.source_media.ingress_stopped {
                return Err("source ingress stopped".into());
            }
            context.require(&Permission::TalkbackDestinations)?;
            if !self.provider.brain_media_authorized(now_ms) {
                return Err("Brain talkback lease/deadman/safety closed".into());
            }
            let snapshot = self.provider.brain_snapshot();
            let brain = self.brain.as_mut().ok_or("Brain media not negotiated")?;
            if brain.context != *context
                || !brain.descriptor.talkback
                || snapshot.held_generation.map(|v| v.0) != Some(brain.descriptor.hold_generation)
            {
                return Err("Brain talkback owner/hold/direction".into());
            }
            let packet = BrainPacket::decode(bytes, &brain.descriptor, BrainMediaRole::Talkback)?;
            if brain
                .last_queued_talkback
                .is_some_and(|f| packet.frame <= f)
            {
                return Err("Brain replay/reordered media".into());
            }
            let samples: [f64; 48] = packet
                .samples
                .as_slice()
                .try_into()
                .map_err(|_| "Brain talkback sample shape")?;
            let block = TalkbackIngress {
                generation: self.media_ingress.talkback_generation,
                frame: packet.frame,
                deadline_ms: snapshot.hold_deadline_ms.ok_or("Brain hold closed")?.0,
                samples,
            };
            self.media_ingress
                .talkback
                .push(block)
                .map_err(|_| "Brain talkback ingress full")?;
            brain.last_queued_talkback = Some(packet.frame);
            Ok(())
        })();
        if result.is_err() {
            self.media_stats.rejected_talkback_packets =
                self.media_stats.rejected_talkback_packets.saturating_add(1);
        }
        result
    }
    fn poll_brain_media(&mut self, context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        context.check_current()?;
        let clocked_selection = self
            .source_clock_status
            .as_ref()
            .map(|_| self.provider.brain_snapshot());
        let Some(brain) = self
            .brain
            .as_mut()
            .filter(|brain| brain.context == *context)
        else {
            return Ok(None);
        };
        if brain.exported {
            return Ok(None);
        }
        let Ok(block) = brain.outgoing.pop() else {
            return Ok(None);
        };
        // All descriptor cloning, allocation, validation and serialization is
        // controller/media work; the source offers only a copied sample block.
        let mut packet = BrainPacket {
            descriptor: brain.descriptor.clone(),
            role: BrainMediaRole::Monitor,
            frame: block.frame,
            samples: block.samples.to_vec(),
        }
        .encode()?;
        if brain.monitor_retirement.is_some()
            || clocked_selection.is_some_and(|snapshot| {
                snapshot.selection_generation.0 != brain.descriptor.selection_generation
                    || snapshot.monitor_mute
                    || !snapshot.monitor_armed
            })
        {
            BrainPacket::silence_monitor(&mut packet, &brain.descriptor)?;
        }
        Ok(Some(packet))
    }
    fn disconnect(&mut self, context: &AuthenticatedContext) {
        self.fx_disconnect(context);
        self.disconnect_aux(context);
        self.brain_reads.remove(&context.session());
        self.held_queries.remove(&context.session());
        self.paired_queries.remove(&context.session());
        self.brain_generations.remove(&context.session());
        self.device_reads.remove(&context.session());
        if self
            .device_owner
            .as_ref()
            .is_some_and(|o| o.session() == context.session())
        {
            self.device_owner = None;
            self.device_observed_ms = None;
            self.provider.close_brain_audio();
            self.device_messages
                .retain(|(s, _)| *s != context.session());
        }
        if self
            .brain
            .as_ref()
            .is_some_and(|b| b.context.session() == context.session())
        {
            self.brain = None;
            self.source_media.clear_brain();
            self.provider.close_brain_audio();
        }
        self.provider.revoke_writer(context.writer());
        self.meter_reads.remove(&context.session());
        self.pending_meter_reads.remove(&context.session());
        self.reads.remove(&context.session());
        self.structural_reads.remove(&context.session());
        self.fx_aux_reads.remove(&context.session());
        self.master_eq_reads.remove(&context.session());
        self.completions.retain(|r| {
            !(r["contract"] == crate::fx_aux_wire::CONTRACT
                && r["writer"].as_str() == Some(context.writer()))
                && r.get("context")
                    .and_then(|c| c.get("writer"))
                    .and_then(Value::as_str)
                    != Some(context.writer())
        });
        if self.analysis_owner == Some(context.session()) {
            self.retire_analysis();
        }
        if self.media_owner == Some(context.session()) {
            self.media_owner = None;
            self.descriptor = None;
            self.source_media.clear_wet(false);
            self.close_source_egress();
        }
        // Keep WetGate history solely for its bounded five-ms fade to silence.
    }
    fn source_frame(&self) -> Option<u64> {
        Some(self.provider.frame())
    }
    fn poll_reply(&mut self, context: &AuthenticatedContext, now_ms: u64) -> Result<Option<Value>> {
        self.finish_media_ingress();
        if self.pending_meter_reads.contains_key(&context.session())
            && self
                .meter_reads
                .get(&context.session())
                .is_some_and(|at| now_ms.saturating_sub(*at) >= 40)
        {
            context.check_current()?;
            let request = self.pending_meter_reads.remove(&context.session()).unwrap();
            self.meter_reads.insert(context.session(), now_ms);
            let reply = self
                .provider
                .meter_snapshot(&request, self.capability_generation)?;
            return serde_json::to_value(reply)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if let Some(value) = self.poll_aux(context) {
            return Ok(Some(value));
        }
        if let Some(value) = self.poll_fx(context) {
            return Ok(Some(value));
        }
        if let Some(i) = self
            .device_messages
            .iter()
            .position(|(s, _)| *s == context.session())
        {
            return Ok(self.device_messages.remove(i).map(|(_, v)| v));
        }
        if let Some(index) = self.device_results.iter().position(|r| {
            r.get("context")
                .and_then(|c| c.get("writer"))
                .and_then(Value::as_str)
                == Some(context.writer())
        }) {
            return Ok(self.device_results.remove(index));
        }
        let index = self.completions.iter().position(|r| {
            (r["contract"] == crate::fx_aux_wire::CONTRACT
                && r["writer"].as_str() == Some(context.writer()))
                || r.get("context")
                    .and_then(|c| c.get("writer"))
                    .and_then(Value::as_str)
                    == Some(context.writer())
        });
        Ok(index.and_then(|index| self.completions.remove(index)))
    }
    fn take_brain_handoff(&mut self, context: &AuthenticatedContext) -> Option<SourceBrainMedia> {
        // Compatibility hosts keep their existing synchronous admission checks.
        self.source_clock_status.as_ref()?;
        let brain = self.brain.as_mut()?;
        if brain.context != *context || brain.exported {
            return None;
        }
        brain.exported = true;
        Some(SourceBrainMedia::new(
            context.clone(),
            brain.descriptor.clone(),
            self.media_ingress.talkback_generation,
            self.media_ingress.talkback.clone(),
            brain.outgoing.clone(),
            brain.handoff.clone(),
        ))
    }
    fn take_wet_input(&mut self, context: &AuthenticatedContext) -> Option<SourceWetInput> {
        if self.media_owner != Some(context.session())
            || self.media_ingress.exported_wet_generation == self.media_ingress.wet_generation
        {
            return None;
        }
        let specs = self
            .descriptor
            .as_ref()?
            .streams
            .iter()
            .filter(|stream| stream.role == MediaRole::WetReturn)
            .map(|stream| stream.spec(context.session()))
            .collect::<Result<Vec<_>>>()
            .ok()?;
        self.media_ingress.exported_wet_generation = self.media_ingress.wet_generation;
        Some(SourceWetInput {
            producer: self.media_ingress.wet.clone(),
            context: context.clone(),
            generation: self.media_ingress.wet_generation,
            specs,
            source_frame: self.media_ingress.source_frame.clone(),
        })
    }
    fn take_media_output(&mut self, context: &AuthenticatedContext) -> Option<SourceEgressOutput> {
        if self.analysis_owner == Some(context.session()) && context.check_current().is_ok() {
            return self.analysis_egress.take_output();
        }
        if self.media_owner == Some(context.session()) && context.check_current().is_ok() {
            self.source_egress.take_output()
        } else {
            None
        }
    }
    fn poll_media(&mut self, context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        if self.analysis_owner == Some(context.session()) {
            return Ok(self.analysis_egress.poll());
        }
        if self.media_owner == Some(context.session()) {
            Ok(self.source_egress.poll())
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod source_media_tests {
    use super::*;

    fn admitted_ingress() -> (
        HostMediaIngress,
        HostSourceMedia,
        AuthenticatedContext,
        PolicyStore,
        WetIngress,
    ) {
        let policy = PolicyStore::new(vec![Peer {
            id: "ingress".into(),
            certificate_sha256: super::super::policy::fingerprint(b"ingress"),
            permissions: [Permission::Fx, Permission::TalkbackDestinations]
                .into_iter()
                .collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"ingress", 1).unwrap();
        let (controller, ingress) = HostMediaIngress::prepare();
        let mut source = HostSourceMedia::new();
        source.ingress = Some(ingress);
        source.wet_generation = 2;
        source.wet_fence = Some(context.audio_fence());
        source.talkback_generation = 3;
        source.talkback_fence = Some(context.audio_fence());
        let epochs = BridgeEpochs {
            source: 77,
            destination: 9,
            route: 1,
        };
        source.talkback = Some(TalkbackSource {
            bridge: Bridge::prepare(BridgeConfig::voice(1), epochs).unwrap(),
            epochs,
            last_received: None,
        });
        let spec = StreamSpec {
            session: 1,
            stream: 1,
            first_channel: 0,
            channels: 2,
            frames: 48,
            encoding: crate::transport::Encoding::Float32,
            role: Role::WetReturn,
            delay_frames: 48,
        };
        source.wet_groups.push(WetGroup {
            spec,
            first: 0,
            buffer: JitterBuffer::new(spec, 0, 0, 32).unwrap(),
        });
        let mut packet = WetIngress {
            generation: 2,
            length: 0,
            bytes: [0; crate::transport::MAX_DATAGRAM],
        };
        packet.length = spec
            .encode_float(0, 0, &[0.25; 96], &mut packet.bytes)
            .unwrap();
        (controller, source, context, policy, packet)
    }

    #[test]
    fn network_wet_endpoint_progresses_without_controller_and_accounts_pressure() {
        let (mut controller, mut source, context, policy, packet) = admitted_ingress();
        let spec = Packet::parse(&packet.bytes[..packet.length])
            .unwrap()
            .spec();
        let mut endpoint = SourceWetInput {
            producer: controller.wet.clone(),
            context: context.clone(),
            generation: 2,
            specs: vec![spec],
            source_frame: controller.source_frame.clone(),
        };
        // The controller neither submits nor consumes receipts during this work.
        endpoint = std::thread::spawn(move || {
            for _ in 0..WET_INGRESS_PACKETS {
                endpoint
                    .submit(
                        &context,
                        Packet::parse(&packet.bytes[..packet.length]).unwrap(),
                    )
                    .unwrap();
            }
            (endpoint, context)
        })
        .join()
        .map(|(e, _)| e)
        .unwrap();
        crate::remote::assert_no_allocations(|| assert!(!source.drain_ingress(9, 0, 0)));
        assert_eq!(controller.wet.slots(), WET_INGRESS_PACKETS);
        let context = endpoint.context.clone();
        for _ in 0..=WET_INGRESS_PACKETS {
            endpoint
                .submit(
                    &context,
                    Packet::parse(&packet.bytes[..packet.length]).unwrap(),
                )
                .unwrap();
        }
        assert_eq!(controller.wet.0.lock().unwrap().rejected, 1);
        crate::remote::assert_no_allocations(|| {
            assert!(!source.drain_ingress(9, 48, 1));
            source.render_wet(48, &[1.; 48]).unwrap();
        });
        assert!(source.wet.iter().all(|v| *v > 0.));
        assert!(controller.wet.is_full());
        assert_eq!(endpoint.source_frame(), Some(48));
        // A full receipt lane must not freeze the network admission cursor.
        crate::remote::assert_no_allocations(|| assert!(!source.drain_ingress(9, 96, 2)));
        assert_eq!(endpoint.source_frame(), Some(96));
        let mut receipts = 0;
        while controller.wet_receipts.pop().is_ok() {
            receipts += 1;
        }
        assert_eq!(receipts, WET_INGRESS_PACKETS);
        source.wet_generation += 1;
        crate::remote::assert_no_allocations(|| assert!(!source.drain_ingress(9, 96, 2)));
        for _ in 0..WET_INGRESS_PACKETS {
            assert_eq!(
                controller.wet_receipts.pop().unwrap().result,
                WetAdmission::Rejected
            );
        }
        // Old endpoint cannot inherit a replacement generation; policy revocation
        // also refuses before queue admission without controller service.
        policy.replace(Vec::new()).unwrap();
        assert!(
            endpoint
                .submit(
                    &context,
                    Packet::parse(&packet.bytes[..packet.length]).unwrap()
                )
                .is_err()
        );
        assert_eq!(controller.wet.slots(), WET_INGRESS_PACKETS);
    }

    #[test]
    fn ingress_receipt_saturation_does_not_stop_source_and_rejects_newest() {
        let (mut controller, mut source, _, _, packet) = admitted_ingress();
        for _ in 0..WET_INGRESS_PACKETS {
            controller.wet.push(packet).unwrap();
        }
        assert!(controller.wet.push(packet).is_err());
        crate::remote::assert_no_allocations(|| assert!(!source.drain_ingress(9, 0, 0)));
        assert_eq!(controller.wet.slots(), WET_INGRESS_PACKETS);
        for _ in 0..WET_INGRESS_PACKETS {
            controller.wet.push(packet).unwrap();
        }
        crate::remote::assert_no_allocations(|| {
            assert!(!source.drain_ingress(9, 48, 1));
            source.render_wet(48, &[1.; 48]).unwrap();
        });
        assert!(controller.wet.is_full());
        // Receipt pressure does not alter the existing 240-frame recovery fade.
        for (frame, pair) in source.wet.chunks_exact(2).enumerate() {
            let expected = 0.25 * (frame + 1) as f64 / 240.;
            assert!((pair[0] - expected).abs() < 1e-12);
            assert_eq!(pair[0], pair[1]);
        }
        assert_eq!(
            controller.wet_receipts.pop().unwrap().result,
            WetAdmission::Accepted
        );
        crate::remote::assert_no_allocations(|| assert!(!source.drain_ingress(9, 48, 1)));
        assert_eq!(controller.wet.slots(), 1);
        // Retained receipt payloads are Copy bytes; all queue/native destruction
        // occurs here after the source guard has ended.
    }

    #[test]
    fn ingress_rechecks_generation_policy_and_exact_wet_deadline() {
        let (mut controller, mut source, context, _, packet) = admitted_ingress();
        controller
            .wet
            .push(WetIngress {
                generation: 1,
                ..packet
            })
            .unwrap();
        controller.wet.push(packet).unwrap();
        crate::remote::assert_no_allocations(|| assert!(!source.drain_ingress(9, 48, 1)));
        assert_eq!(
            controller.wet_receipts.pop().unwrap().result,
            WetAdmission::Rejected
        );
        let late = controller.wet_receipts.pop().unwrap();
        assert_eq!(late.result, WetAdmission::Late);
        assert_eq!(late.frame, 48);
        context.close_audio();
        controller.wet.push(packet).unwrap();
        crate::remote::assert_no_allocations(|| assert!(source.drain_ingress(9, 0, 2)));
        assert_eq!(
            controller.wet_receipts.pop().unwrap().result,
            WetAdmission::Rejected
        );
        assert_eq!(source.talkback_samples, [0.; 48]);
    }

    #[test]
    fn network_brain_handoff_preserves_bounded_receipts_replay_and_closure() {
        let (mut controller, mut source, context, _, _) = admitted_ingress();
        let descriptor = BrainMediaDescriptor {
            contract: "GP15-media".into(),
            version: 1,
            session: context.session(),
            stagebox_epoch: 9,
            brain_epoch: 77,
            stagebox_map: 1,
            brain_map: 1,
            generation: 1,
            selection_generation: 1,
            hold_generation: 1,
            sample_rate: 48000,
            frames: 48,
            talkback: true,
            monitor: false,
        };
        let (_, consumer) = prepare_monitor_lane();
        let state = std::sync::Arc::new(BrainHandoffState::new(1));
        state.publish(1000, false);
        let mut endpoint = SourceBrainMedia::new(
            context.clone(),
            descriptor.clone(),
            3,
            controller.talkback.clone(),
            MonitorConsumer::new(consumer),
            state.clone(),
        );
        let packet = |frame| {
            BrainPacket {
                descriptor: descriptor.clone(),
                role: BrainMediaRole::Talkback,
                frame,
                samples: vec![0.25; 48],
            }
            .encode()
            .unwrap()
        };
        let bytes = (0..65).map(|i| packet(i * 48)).collect::<Vec<_>>();
        endpoint = std::thread::spawn({
            let context = context.clone();
            move || {
                for bytes in &bytes[..64] {
                    endpoint.submit(&context, bytes, 1).unwrap();
                }
                assert!(endpoint.submit(&context, &bytes[64], 1).is_err());
                endpoint
            }
        })
        .join()
        .unwrap();
        assert_eq!(controller.talkback.take_rejected(), 1);
        crate::remote::assert_no_allocations(|| {
            source.drain_ingress(9, 0, 1);
        });
        assert_eq!(controller.talkback_receipts.slots(), 64);
        assert_eq!(
            source.talkback.as_ref().unwrap().last_received,
            Some(63 * 48)
        );
        assert!(endpoint.submit(&context, &packet(63 * 48), 1).is_err());
        endpoint.submit(&context, &packet(64 * 48), 1).unwrap();
        // Full receipts retain pending work without consuming it or blocking render.
        crate::remote::assert_no_allocations(|| {
            source.drain_ingress(9, 48, 1);
        });
        assert_eq!(
            source.talkback.as_ref().unwrap().last_received,
            Some(63 * 48)
        );
        for _ in 0..64 {
            assert!(controller.talkback_receipts.pop().unwrap().accepted);
        }
        source.clock_talkback_deadline = Some(0);
        crate::remote::assert_no_allocations(|| {
            source.drain_ingress(9, 96, 1);
        });
        assert!(!controller.talkback_receipts.pop().unwrap().accepted);
        state.close();
        assert!(endpoint.submit(&context, &packet(65 * 48), 1).is_err());
        assert_eq!(controller.talkback.take_rejected(), 2);
        controller.talkback.close();
        assert!(endpoint.submit(&context, &packet(66 * 48), 1).is_err());
        assert_eq!(
            controller.talkback.take_rejected(),
            0,
            "final shutdown accounting stays final"
        );
    }

    #[test]
    fn talkback_ingress_refuses_stale_expired_and_revoked_blocks_without_control_pump() {
        let (mut controller, mut source, _, policy, _) = admitted_ingress();
        let block = TalkbackIngress {
            generation: 3,
            frame: 0,
            deadline_ms: 10,
            samples: [0.25; 48],
        };
        controller
            .talkback
            .push(TalkbackIngress {
                generation: 2,
                ..block
            })
            .unwrap();
        controller.talkback.push(block).unwrap();
        crate::remote::assert_no_allocations(|| assert!(!source.drain_ingress(9, 0, 0)));
        assert!(!controller.talkback_receipts.pop().unwrap().accepted);
        assert!(controller.talkback_receipts.pop().unwrap().accepted);
        assert_eq!(source.talkback.as_ref().unwrap().last_received, Some(0));
        controller
            .talkback
            .push(TalkbackIngress { frame: 48, ..block })
            .unwrap();
        crate::remote::assert_no_allocations(|| assert!(source.drain_ingress(9, 48, 10)));
        assert!(!controller.talkback_receipts.pop().unwrap().accepted);
        policy.replace(Vec::new()).unwrap();
        controller
            .talkback
            .push(TalkbackIngress {
                frame: 96,
                deadline_ms: 100,
                ..block
            })
            .unwrap();
        crate::remote::assert_no_allocations(|| assert!(source.drain_ingress(9, 96, 11)));
        assert!(!controller.talkback_receipts.pop().unwrap().accepted);
        assert_eq!(source.talkback.as_ref().unwrap().last_received, Some(0));
        assert_eq!(source.talkback_samples, [0.; 48]);
    }

    #[test]
    fn owned_media_runs_with_stalled_consumer_and_bounded_monitor_overflow() {
        let mut source = HostSourceMedia::new();
        let epochs = BridgeEpochs {
            source: 77,
            destination: 9,
            route: 1,
        };
        let mut bridge = Bridge::prepare(BridgeConfig::voice(1), epochs).unwrap();
        for block in 0..64 {
            bridge.push(epochs, block * 48, &[0.25; 48]).unwrap();
        }
        source.talkback = Some(TalkbackSource {
            bridge,
            epochs,
            last_received: Some(63 * 48),
        });
        let (producer, mut consumer) = prepare_monitor_lane();
        source.monitor = Some(producer);
        let spec = StreamSpec {
            session: 1,
            stream: 1,
            first_channel: 0,
            channels: 2,
            frames: 48,
            encoding: crate::transport::Encoding::Float32,
            role: Role::WetReturn,
            delay_frames: 48,
        };
        let mut buffer = JitterBuffer::new(spec, 0, 0, 32).unwrap();
        for block in 0..32 {
            let mut bytes = [0; crate::transport::MAX_DATAGRAM];
            let n = spec
                .encode_float(block * 48, block as u32, &[0.25; 96], &mut bytes)
                .unwrap();
            buffer
                .insert_wet_at(Packet::parse(&bytes[..n]).unwrap(), 0)
                .unwrap();
        }
        source.wet_groups.push(WetGroup {
            spec,
            first: 0,
            buffer,
        });
        // Ownership crosses a real thread boundary. The consumer is untouched
        // until all render quanta, overflow and talkback closure have completed.
        let source = std::thread::spawn(move || {
            crate::remote::assert_no_allocations(|| {
                for block in 0..32 {
                    let frame = (block + 1) * 48;
                    source.render_wet(frame, &[1.; 48]).unwrap();
                    assert!(source.render_talkback(frame, true, true));
                    assert!(source.talkback_samples.iter().all(|s| s.is_finite()));
                    assert!(source.offer_monitor(block * 96, source.wet));
                    assert!(source.offer_monitor(block * 96 + 48, source.wet));
                }
                assert!(source.wet.iter().all(|sample| *sample == 0.25));
                assert!(!source.offer_monitor(3072, [0.; 96]));
                assert!(source.render_talkback(1584, true, false));
                assert_eq!(source.talkback_samples, [0.; 48]);
            });
            source
        })
        .join()
        .unwrap();
        for block in 0..64 {
            let packet = consumer.pop().unwrap();
            assert_eq!(packet.frame, block * 48);
            assert!(packet.samples.iter().all(|s| s.is_finite()));
        }
        assert!(consumer.pop().is_err());
        // Native ASRC and all allocating storage are retired on this controller.
        drop(source);
    }
}

#[cfg(test)]
mod monitor_retirement_tests {
    use super::*;
    use crate::brain_audio::{
        bridge::{BridgeConfig, BridgeEpochs},
        device::{DeviceConfig, PortMap, SampleFormat},
    };
    use crate::control_model::{Command, Request, Scope};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn clocked_brain_proxy_drains_monitor_without_mailbox_and_replaces_endpoint() {
        let mut f = Fixture::new("network-monitor");
        let status = f
            .host
            .start_source_clock(0, std::time::Duration::from_millis(1), 1000)
            .unwrap();
        let (mut proxy, mut mailbox) = authority_channel(&f.ctx, f.host.identity()).unwrap();
        mailbox.service(&mut f.host, 0).unwrap();
        assert!(f.host.brain.as_ref().unwrap().exported);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let before = status.blocks();
        while status.blocks() < before + 70 {
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut frames = Vec::new();
        for _ in 0..64 {
            let bytes = proxy
                .poll_brain_media(&f.ctx)
                .unwrap()
                .expect("source monitor independent of mailbox");
            let packet =
                BrainPacket::decode(&bytes, &f.descriptor, BrainMediaRole::Monitor).unwrap();
            frames.push(packet.frame);
            assert!(
                packet.samples.iter().all(|s| *s == 0.),
                "unarmed monitor stays silent"
            );
        }
        assert!(frames.windows(2).all(|f| f[1] == f[0] + 48));
        // Retire the old endpoint at the source boundary, preserving same-stream frames.
        let mut next = f.descriptor.clone();
        next.generation += 1;
        f.host
            .negotiate_brain(
                &f.ctx,
                serde_json::json!({"contract":"GP15-media","version":1,
            "kind":"negotiate","writer":f.ctx.writer(),"descriptor":next}),
            )
            .unwrap();
        assert!(proxy.poll_brain_media(&f.ctx).unwrap().is_none());
        mailbox.service(&mut f.host, 90).unwrap();
        let bytes = loop {
            if let Some(bytes) = proxy.poll_brain_media(&f.ctx).unwrap() {
                break bytes;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        BrainPacket::decode(&bytes, &next, BrainMediaRole::Monitor).unwrap();
        drop(proxy);
        assert!(!f.ctx.audio_fence().valid());
        let report = f.host.finish_source_clock(&status).unwrap();
        assert!(
            report.monitor_drops >= 6,
            "full copied-sample lane must count overflow"
        );
        f.host.finish_source_ingress();
    }

    #[test]
    fn brain_preparation_does_not_spend_the_source_reservation() {
        let mut f = Fixture::new("brain-preparation-reservation");
        let source = f
            .host
            .start_source_clock(0, std::time::Duration::from_millis(1), 500)
            .unwrap();
        let mut descriptor = f.host.brain_descriptor().unwrap().clone();
        descriptor.generation += 1;
        let before = source.blocks();
        let reply = f
            .host
            .negotiate_brain_prepared(
                &f.ctx,
                serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate",
                "writer":f.ctx.writer(),"descriptor":descriptor}),
                || {
                    // Deliberately exceed the unchanged 480-frame reservation lead.
                    // A reservation taken before preparation expires deterministically.
                    let target = source.blocks() + 24;
                    let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
                    while source.blocks() < target {
                        assert!(std::time::Instant::now() < until);
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                },
            )
            .unwrap();
        assert!(reply["reason"].is_null(), "{reply}");
        assert!(source.blocks() >= before + 24);
        assert_eq!(
            f.host.brain_descriptor().unwrap().generation,
            descriptor.generation
        );
        assert_eq!(f.host.finish_source_clock(&source).unwrap().fault, None);
    }

    #[test]
    fn independent_analysis_admission_closure_and_source_progress() {
        let mut f = Fixture::new("independent-analysis");
        let policy = PolicyStore::new(vec![Peer {
            id: "analysis".into(),
            certificate_sha256: fingerprint(b"analysis"),
            permissions: [Permission::Analysis, Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"analysis", 88).unwrap();
        let identity = f.host.identity();
        let ids: Vec<_> = f
            .host
            .provider
            .topology()
            .inputs
            .iter()
            .map(|i| i.id.clone())
            .collect();
        let descriptor = MediaDescriptor {
            session: Counter(88),
            source_epoch: identity.source_epoch,
            capability_generation: identity.capability_generation,
            map_generation: identity.map_generation,
            sample_rate: 48000,
            streams: grouped_streams(MediaRole::Analysis, &ids, 48, 0, 1, 1100).unwrap(),
        };
        let source = f
            .host
            .start_source_clock(0, std::time::Duration::from_millis(2), 200)
            .unwrap();
        assert!(f.host.negotiate(&context, &descriptor).unwrap());
        assert!(f.host.negotiate(&context, &descriptor).is_err());
        assert!(
            f.host.media_owner.is_none(),
            "observer must not reserve the FX owner"
        );
        assert_eq!(
            f.host
                .dispatch(
                    &context,
                    serde_json::json!({"writer":context.writer(),"kind":"grant"}),
                    0
                )
                .unwrap_err(),
            "analysis connection permits media observation only"
        );
        let fx_context = policy.authenticate(b"analysis", 89).unwrap();
        let stereo = ["foh-left".into(), "foh-right".into()];
        let mut fx_streams = grouped_streams(MediaRole::FxSend, &stereo, 48, 0, 1, 1100).unwrap();
        fx_streams
            .extend(grouped_streams(MediaRole::WetReturn, &stereo, 48, 768, 2, 1100).unwrap());
        let fx_descriptor = MediaDescriptor {
            session: Counter(89),
            streams: fx_streams,
            ..descriptor.clone()
        };
        assert!(f.host.negotiate(&fx_context, &fx_descriptor).unwrap());
        assert_eq!(f.host.media_owner, Some(89));
        let mut fx_output = f.host.take_media_output(&fx_context).unwrap();
        let mut output = f.host.take_media_output(&context).unwrap();
        let before = source.blocks();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let mut packets = 0;
        while source.blocks() < before + 32 {
            if let Some(bytes) = output.poll() {
                let packet = Packet::parse(&bytes).unwrap();
                assert_eq!(packet.spec().role, Role::Analysis);
                for index in 0..packet.spec().samples() {
                    let channel = usize::from(packet.spec().first_channel)
                        + index % usize::from(packet.spec().channels);
                    let frame = packet.source_frame()
                        + (index / usize::from(packet.spec().channels)) as u64;
                    let sign = if (frame / (24 + channel as u64)).is_multiple_of(2) {
                        1
                    } else {
                        -1
                    };
                    assert_eq!(
                        packet.pcm(index).unwrap(),
                        sign * (channel as i32 + 1) * 128
                    );
                }
                packets += 1;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        assert!(packets > 0);
        f.host.analysis_egress.pause(true);
        let target = source.blocks() + 32;
        let mut fx_packets = 0;
        while source.blocks() < target {
            if let Some(bytes) = fx_output.poll() {
                assert_eq!(Packet::parse(&bytes).unwrap().spec().role, Role::FxSend);
                fx_packets += 1;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        assert!(fx_packets > 0);
        let counts = f.host.source_egress_report();
        assert!(
            counts["independent_analysis"]["all_sessions"]["offer_refusals"]
                .as_u64()
                .unwrap()
                >= 28,
            "{counts}"
        );
        context.close_audio();
        let target = source.blocks() + 2;
        while source.blocks() < target {
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            output.poll().is_none(),
            "terminal closure bypasses controller service"
        );
        f.host.disconnect(&context);
        assert!(f.host.analysis_owner.is_none());
        assert_eq!(f.host.media_owner, Some(89));
        assert!(fx_context.audio_fence().valid());
        assert!(fx_output.poll().is_some());
        assert_eq!(f.host.finish_source_clock(&source).unwrap().fault, None);
    }

    #[test]
    fn clocked_media_first_packet_matches_reserved_source_frame() {
        let mut f = Fixture::new("media-boundary");
        let policy = PolicyStore::new(vec![Peer {
            id: "fx-boundary".into(),
            certificate_sha256: fingerprint(b"fx-boundary"),
            permissions: [Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"fx-boundary", 88).unwrap();
        let ids = ["foh-left".into(), "foh-right".into()];
        let mut streams = grouped_streams(MediaRole::FxSend, &ids, 48, 0, 1, 1100).unwrap();
        streams.extend(grouped_streams(MediaRole::WetReturn, &ids, 48, 384, 2, 1100).unwrap());
        let identity = f.host.identity();
        let descriptor = MediaDescriptor {
            session: Counter(88),
            source_epoch: identity.source_epoch,
            capability_generation: identity.capability_generation,
            map_generation: identity.map_generation,
            sample_rate: 48000,
            streams,
        };
        let status = f
            .host
            .start_source_clock(0, std::time::Duration::from_millis(5), 100)
            .unwrap();
        assert!(f.host.negotiate(&context, &descriptor).unwrap());
        let first = f
            .host
            .source_media
            .execute(None, |source, out| {
                *out = source.wet_groups.first().map(|group| group.first);
            })
            .unwrap()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(bytes) = f.host.poll_media(&context).unwrap() {
                assert_eq!(Packet::parse(&bytes).unwrap().source_frame(), first);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        f.host.finish_source_clock(&status).unwrap();
    }

    #[test]
    fn clocked_device_intent_commits_at_future_source_boundary() {
        let mut f = Fixture::new("clocked-device");
        let started = std::time::Instant::now();
        let status = f
            .host
            .start_source_clock(0, std::time::Duration::from_millis(2), 200)
            .unwrap();
        f.host
            .dispatch_device(
                &f.ctx,
                serde_json::json!({"version":1,"kind":"device_snapshot"}),
                0,
            )
            .unwrap();
        let before = status.blocks();
        let config = f.host.device_observation.as_ref().unwrap().config.clone();
        let payload = serde_json::json!({"contract":"GP15-device","version":1,"show_id":"11111111-1111-4111-8111-111111111111","module":"audio","epoch":"9","writer":f.ctx.writer(),"lease":f.lease,"request_id":"2","expected_revision":"0","kind":"device_configure","config":config});
        let reply = f
            .host
            .dispatch(&f.ctx, payload, started.elapsed().as_millis() as u64)
            .unwrap()
            .unwrap();
        assert_eq!(reply["state"], "accepted_intent", "{reply}");
        assert!(status.blocks() >= before + 10);
        assert!(f.host.provider.frame() >= (before + 10) * 48);
        assert_eq!(f.host.provider.engine().revision(), Counter(1));
        assert!(f.host.device_pending.as_ref().unwrap().sent);
        assert_eq!(f.host.device_messages.len(), 1);
        assert!(f.host.brain.is_none());
        status.close();
        assert_eq!(f.host.finish_source_clock(&status).unwrap().fault, None);
    }

    #[test]
    fn clocked_selection_silences_already_queued_monitor_samples() {
        let mut f = Fixture::new("clocked-monitor-queue");
        assert!(f.host.source_media.offer_monitor(0, [0.25; 96]).unwrap());
        let status = f
            .host
            .start_source_clock(7, std::time::Duration::from_millis(2), 100)
            .unwrap();
        // Deliberately nonzero: a literal time-zero control would be refused.
        let now = f.host.provider.source_control_time();
        assert!(now >= 7);
        let request = crate::brain_control::Request {
            contract: "GP15-brain".into(),
            version: 1,
            show_id: "11111111-1111-4111-8111-111111111111".into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some(f.ctx.writer().into()),
            lease: Some(f.lease),
            request_id: Some(Counter(2)),
            expected_revision: Some(Counter(0)),
            command: crate::brain_control::Command::MonitorSet {
                source: crate::brain_control::MonitorSource::Pfl { input: 0 },
                gain_cdb: 0,
                mute: false,
                dim: false,
                armed: false,
            },
        };
        f.host.provider.last_brain_source_receipt = None;
        let reply = f
            .host
            .provider
            .clocked_control(now, Some(f.ctx.audio_fence()), |provider| {
                serde_json::to_value(provider.brain_request(request, now, true, None)?)
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        assert_ne!(
            f.host.provider.brain_snapshot().selection_generation.0,
            f.descriptor.selection_generation,
            "reply={reply}, receipt={:?}",
            f.host.provider.last_brain_source_receipt
        );
        assert!(f.host.brain.as_ref().unwrap().monitor_retirement.is_none());
        let bytes = f.host.poll_brain_media(&f.ctx).unwrap().unwrap();
        let packet = BrainPacket::decode(&bytes, &f.descriptor, BrainMediaRole::Monitor).unwrap();
        assert_eq!(packet.frame, 0);
        assert!(packet.samples.iter().all(|sample| *sample == 0.));
        status.close();
        f.host.finish_source_clock(&status).unwrap();
        assert!(f.host.poll_brain_media(&f.ctx).unwrap().is_none());
    }

    struct Fixture {
        host: HostAuthority,
        ctx: AuthenticatedContext,
        policy: PolicyStore,
        descriptor: BrainMediaDescriptor,
        path: std::path::PathBuf,
        lease: Counter,
        request: u64,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    impl Fixture {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("gp-retirement-{}-{label}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut provider = LocalAudio::bind_configured(
                &path,
                "host",
                "11111111-1111-4111-8111-111111111111",
                Counter(9),
                crate::topology::EngineTopology::reference_16_18(0, 4).unwrap(),
            )
            .unwrap();
            provider.rearm().unwrap();
            let policy = PolicyStore::new(vec![Peer {
                id: "duplex".into(),
                certificate_sha256: fingerprint(b"retirement"),
                permissions: [Permission::LocalOperatorMonitor].into_iter().collect(),
            }])
            .unwrap();
            let ctx = policy.authenticate(b"retirement", 12).unwrap();
            let grant = Request {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: "11111111-1111-4111-8111-111111111111".into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(ctx.writer().into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: Command::Grant {
                    scope: Scope::LocalOperatorMonitor,
                },
            };
            let lease = provider
                .engine_mut()
                .handle(&grant, 0)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .granted_lease
                .unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let descriptor = BrainMediaDescriptor {
                contract: "GP15-media".into(),
                version: 1,
                session: 12,
                stagebox_epoch: 9,
                brain_epoch: 77,
                stagebox_map: host.identity().map_generation.0,
                brain_map: 1,
                generation: 1,
                selection_generation: host.provider.brain_snapshot().selection_generation.0,
                hold_generation: 0,
                sample_rate: 48000,
                frames: 48,
                talkback: false,
                monitor: true,
            };
            host.negotiate_brain(&ctx, serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":ctx.writer(),"descriptor":descriptor})).unwrap();
            let port = |id: &str, slot| PortMap {
                id: id.into(),
                socket: id.into(),
                slot,
            };
            host.device_owner = Some(ctx.clone());
            host.device_observed_ms = Some(0);
            host.device_observation = Some(BrainDeviceObservation {
                brain_epoch: 77,
                brain_map: 1,
                frame: 0,
                config: DeviceConfig {
                    device_id: "fake".into(),
                    endpoint: "fake:brain".into(),
                    sample_rate: 48000,
                    capture_channels: 1,
                    playback_channels: 2,
                    format: SampleFormat::Float32Le,
                    period_frames: 48,
                    buffer_frames: 192,
                    microphone: port("mic", 0),
                    monitor: [port("left", 0), port("right", 1)],
                    socket_mapping_record: None,
                    shared_clock_record: None,
                    hardware_monitoring_record: None,
                },
                status: serde_json::json!({"armed":true,"fault":null}),
                ticket: None,
                success: None,
                error: None,
            });
            Self {
                host,
                ctx,
                policy,
                descriptor,
                path,
                lease,
                request: 2,
            }
        }
        fn tick(&mut self, now: u64) {
            let frame = self.host.provider.frame();
            let capture = vec![0.125; 48 * self.host.provider.topology().capture_channels];
            let mut playback = vec![0.; 48 * self.host.provider.topology().playback_channels];
            self.host
                .process_source(now, 9, frame, &capture, &mut playback)
                .unwrap();
        }
        fn select(&mut self, source: crate::brain_control::MonitorSource, now: u64) {
            let request = crate::brain_control::Request {
                contract: "GP15-brain".into(),
                version: 1,
                show_id: "11111111-1111-4111-8111-111111111111".into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(self.ctx.writer().into()),
                lease: Some(self.lease),
                request_id: Some(Counter(self.request)),
                expected_revision: Some(self.host.provider.brain_snapshot().revision),
                command: crate::brain_control::Command::MonitorSet {
                    source,
                    gain_cdb: 0,
                    mute: false,
                    dim: false,
                    armed: false,
                },
            };
            self.request += 1;
            let pending = self
                .host
                .provider
                .brain_request(request.clone(), now, true, None)
                .unwrap();
            assert_eq!(pending.state, "pending", "{pending:?}");
            assert!(pending.reason.is_none(), "{pending:?}");
            let boundary = pending
                .applied_frame
                .expect("declared effective boundary")
                .0;
            let current = self.host.provider.frame();
            assert!(boundary >= current && boundary <= current + 48 * 4);
            while self.host.provider.frame() <= boundary {
                self.tick(now);
            }
            let final_reply = self
                .host
                .provider
                .brain_request(request, now, true, None)
                .unwrap();
            assert_eq!(final_reply.state, "final", "{final_reply:?}");
            assert!(final_reply.reason.is_none(), "{final_reply:?}");
            assert_eq!(final_reply.applied_frame, Some(Counter(boundary)));
            assert_eq!(self.host.provider.brain_snapshot().source, source);
        }
        fn queued_monitor(&mut self) -> Vec<MonitorBlock> {
            let mut blocks = Vec::new();
            let consumer = &mut self.host.brain.as_mut().unwrap().outgoing;
            while let Ok(block) = consumer.pop() {
                blocks.push(block);
            }
            for block in &blocks {
                assert!(
                    self.host
                        .source_media
                        .offer_monitor(block.frame, block.samples)
                        .unwrap()
                );
            }
            blocks
        }
        fn transition(&mut self) -> Vec<u8> {
            self.tick(0);
            let packet = BrainPacket {
                descriptor: self.descriptor.clone(),
                role: BrainMediaRole::Monitor,
                frame: 0,
                samples: vec![0.125; 96],
            }
            .encode()
            .unwrap();
            let mut block = self.host.brain.as_mut().unwrap().outgoing.pop().unwrap();
            block.samples.fill(0.125);
            assert!(
                self.host
                    .source_media
                    .offer_monitor(block.frame, block.samples)
                    .unwrap()
            );
            self.select(crate::brain_control::MonitorSource::Main, 1);
            self.tick(1);
            assert!(
                self.host
                    .brain
                    .as_ref()
                    .unwrap()
                    .monitor_retirement
                    .is_some()
            );
            packet
        }
    }

    #[test]
    fn media_commands_execute_on_source_and_return_displaced_bridge_storage() {
        let mut f = Fixture::new("executor-retirement");
        let controller = std::thread::current().id();
        let epochs = BridgeEpochs {
            source: 78,
            destination: 9,
            route: 2,
        };
        let prepared = TalkbackSource {
            bridge: Bridge::prepare(BridgeConfig::voice(1), epochs).unwrap(),
            epochs,
            last_received: None,
        };
        let (retired, worker) = f
            .host
            .source_media
            .execute((Some(prepared), None), |source, p| {
                p.1 = Some(std::thread::current().id());
                crate::remote::assert_no_allocations(|| {
                    std::mem::swap(&mut source.talkback, &mut p.0);
                });
            })
            .unwrap();
        assert_ne!(worker.unwrap(), controller);
        let retired = retired.expect("previous bridge must be returned for controller retirement");
        assert_eq!(retired.bridge.status().epochs.source, 77);
        assert_eq!(f.host.brain_bridge_status().unwrap().epochs, epochs);
        drop(retired);
        f.host.source_media.clear_brain();
        assert!(f.host.source_media.talkback_status.is_none());
    }

    #[test]
    fn preserving_brain_bridge_keeps_pending_talkback_generation_and_replay_cursor() {
        let mut f = Fixture::new("preserve-ingress");
        let generation = f.host.media_ingress.talkback_generation;
        let block = TalkbackIngress {
            generation,
            frame: 0,
            deadline_ms: 100,
            samples: [0.25; 48],
        };
        f.host.media_ingress.talkback.push(block).unwrap();
        f.host.test_drain_media_ingress(0);
        f.host
            .media_ingress
            .talkback
            .push(TalkbackIngress { frame: 48, ..block })
            .unwrap();
        f.host.brain.as_mut().unwrap().last_queued_talkback = Some(48);
        let mut next = f.descriptor.clone();
        next.generation += 1;
        f.host
            .negotiate_brain(
                &f.ctx,
                serde_json::json!({"contract":"GP15-media","version":1,
            "kind":"negotiate","writer":f.ctx.writer(),"descriptor":next}),
            )
            .unwrap();
        assert_eq!(f.host.media_ingress.talkback_generation, generation);
        assert_eq!(
            f.host.brain.as_ref().unwrap().last_queued_talkback,
            Some(48)
        );
        f.host
            .media_ingress
            .talkback
            .push(TalkbackIngress { frame: 96, ..block })
            .unwrap();
        f.host.test_drain_media_ingress(1);
        assert_eq!(f.host.media_stats.accepted_talkback_packets, 3);
        assert_eq!(f.host.media_stats.rejected_talkback_packets, 0);
        assert_eq!(f.host.source_media.talkback_last_received, Some(96));
        assert!(f.host.source_media.talkback_status.unwrap().fault.is_none());
    }

    #[test]
    fn final_ingress_retirement_accounts_receipts_and_pending_without_rendering() {
        let mut f = Fixture::new("finish-ingress");
        let block = TalkbackIngress {
            generation: f.host.media_ingress.talkback_generation,
            frame: 0,
            deadline_ms: 100,
            samples: [0.25; 48],
        };
        f.host.media_ingress.talkback.push(block).unwrap();
        assert!(!f.host.source_media.drain_ingress(9, 0, 0).unwrap());
        f.host
            .media_ingress
            .talkback
            .push(TalkbackIngress { frame: 48, ..block })
            .unwrap();
        f.host
            .media_ingress
            .wet
            .push(WetIngress {
                generation: 1,
                length: 0,
                bytes: [0; crate::transport::MAX_DATAGRAM],
            })
            .unwrap();
        // One packet is already pending: fill the remaining slots and refuse
        // exactly two newest submissions. Final collection must count each once.
        for _ in 0..=WET_INGRESS_PACKETS {
            let _ = f.host.media_ingress.wet.push(WetIngress {
                generation: 1,
                length: 0,
                bytes: [0; crate::transport::MAX_DATAGRAM],
            });
        }
        let policy = PolicyStore::new(vec![Peer {
            id: "closed-wet".into(),
            certificate_sha256: super::super::policy::fingerprint(b"closed-wet"),
            permissions: [Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"closed-wet", f.ctx.session()).unwrap();
        let producer = f.host.media_ingress.wet.clone();
        let mut endpoint = SourceWetInput {
            producer,
            context: context.clone(),
            generation: 1,
            specs: vec![],
            source_frame: f.host.media_ingress.source_frame.clone(),
        };
        let spec = StreamSpec {
            session: f.ctx.session(),
            stream: 1,
            first_channel: 0,
            channels: 2,
            frames: 48,
            encoding: crate::transport::Encoding::Float32,
            role: Role::WetReturn,
            delay_frames: 48,
        };
        endpoint.specs.push(spec);
        let mut bytes = [0; crate::transport::MAX_DATAGRAM];
        let length = spec.encode_float(0, 0, &[0.; 96], &mut bytes).unwrap();
        let frame = f.host.provider.frame();
        f.host.finish_source_ingress();
        assert_eq!(
            endpoint
                .submit(&context, Packet::parse(&bytes[..length]).unwrap())
                .unwrap_err(),
            "source ingress stopped"
        );
        assert_eq!(f.host.provider.frame(), frame);
        assert_eq!(f.host.media_stats.accepted_talkback_packets, 1);
        assert_eq!(f.host.media_stats.rejected_talkback_packets, 1);
        assert_eq!(
            f.host.media_stats.rejected_wet_packets,
            WET_INGRESS_PACKETS as u64 + 2
        );
        assert!(!f.host.source_media.ingress_present);
        assert!(f.host.source_media.monitor_present);
        assert!(!f.host.source_media.talkback_fence_present);
        let capture = vec![0.; 48 * f.host.provider.topology().capture_channels];
        let mut playback = vec![1.; 48 * f.host.provider.topology().playback_channels];
        assert!(
            f.host
                .process_source(1, 9, frame, &capture, &mut playback)
                .is_err()
        );
        assert!(playback.iter().all(|sample| *sample == 0.));
        assert_eq!(f.host.provider.frame(), frame);
        f.host.finish_source_ingress();
        assert_eq!(f.host.media_stats.rejected_talkback_packets, 1);
        assert_eq!(
            f.host.media_stats.rejected_wet_packets,
            WET_INGRESS_PACKETS as u64 + 2
        );
        let mut next = f.descriptor.clone();
        next.generation += 1;
        assert!(
            f.host
                .negotiate_brain(
                    &f.ctx,
                    serde_json::json!({"contract":"GP15-media","version":1,
            "kind":"negotiate","writer":f.ctx.writer(),"descriptor":next})
                )
                .is_err()
        );
    }

    #[test]
    fn retirement_preserves_indices_and_runs_real_bridge_until_new_identity() {
        let mut f = Fixture::new("continuous");
        let old = f.transition();
        let epochs = BridgeEpochs {
            source: 9,
            destination: 77,
            route: f.descriptor.selection_generation,
        };
        let mut bridge = Bridge::prepare(BridgeConfig::voice(2), epochs).unwrap();
        let mut expected_frame = 0;
        let mut output_frame = 0;
        for now in 2..102 {
            while let Some(bytes) = f.host.poll_brain_media(&f.ctx).unwrap() {
                if expected_frame == 0 {
                    assert_eq!(&bytes[..80], &old[..80]);
                    assert_eq!(bytes.len(), old.len());
                }
                let packet =
                    BrainPacket::decode(&bytes, &f.descriptor, BrainMediaRole::Monitor).unwrap();
                assert_eq!(packet.frame, expected_frame);
                assert!(packet.samples.iter().all(|s| *s == 0. && s.is_finite()));
                bridge.push(epochs, packet.frame, &packet.samples).unwrap();
                expected_frame += 48;
            }
            if bridge.status().ready && !bridge.status().armed {
                bridge.arm(epochs).unwrap();
            }
            if bridge.status().armed {
                let mut samples = [0.; 96];
                bridge.render(epochs, output_frame, &mut samples).unwrap();
                assert!(samples.iter().all(|s| *s == 0.));
                output_frame += 48;
            }
            f.tick(now);
        }
        assert!(output_frame > 3000);
        assert_eq!(bridge.status().underruns, 0);
        assert_eq!(bridge.status().overflows, 0);
        let mut next = f.descriptor.clone();
        next.generation += 1;
        next.selection_generation = f.host.provider.brain_snapshot().selection_generation.0;
        f.host.negotiate_brain(&f.ctx,serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":f.ctx.writer(),"descriptor":next})).unwrap();
        assert!(f.host.brain.as_ref().unwrap().monitor_retirement.is_none());
        assert!(f.host.poll_brain_media(&f.ctx).unwrap().is_none());
        f.tick(102);
        let bytes = f.host.poll_brain_media(&f.ctx).unwrap().unwrap();
        BrainPacket::decode(&bytes, &next, BrainMediaRole::Monitor).unwrap();
        assert!(BrainPacket::decode(&bytes, &f.descriptor, BrainMediaRole::Monitor).is_err());
    }

    #[test]
    fn retirement_payload_rewrite_rejects_wrong_role_and_identity_without_mutation() {
        let f = Fixture::new("packet");
        let mut d = f.descriptor.clone();
        d.talkback = true;
        for role in [BrainMediaRole::Talkback, BrainMediaRole::Monitor] {
            let mut bytes = BrainPacket {
                descriptor: d.clone(),
                role,
                frame: 48,
                samples: vec![0.5; 48 * role.channels()],
            }
            .encode()
            .unwrap();
            if role == BrainMediaRole::Monitor {
                bytes[15] ^= 1;
            }
            let old = bytes.clone();
            assert!(BrainPacket::silence_monitor(&mut bytes, &d).is_err());
            assert_eq!(bytes, old);
        }
        let mut bytes = BrainPacket {
            descriptor: d.clone(),
            role: BrainMediaRole::Monitor,
            frame: 48,
            samples: vec![0.5; 96],
        }
        .encode()
        .unwrap();
        let old = bytes.clone();
        BrainPacket::silence_monitor(&mut bytes, &d).unwrap();
        assert_eq!(&bytes[..80], &old[..80]);
        assert_eq!(bytes.len(), old.len());
        assert!(
            BrainPacket::decode(&bytes, &d, BrainMediaRole::Monitor)
                .unwrap()
                .samples
                .iter()
                .all(|v| *v == 0. && v.is_finite())
        );
    }

    #[test]
    fn retirement_deadlines_do_not_extend_and_identity_loss_closes() {
        for case in 0..11 {
            let mut f = Fixture::new(&format!("expiry-{case}"));
            f.transition();
            let first = f.host.brain.as_ref().unwrap().monitor_retirement.unwrap();
            let queued = f.queued_monitor();
            let mut stale = f.descriptor.clone();
            stale.generation += 1;
            assert!(f.host.negotiate_brain(&f.ctx,serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":f.ctx.writer(),"descriptor":stale})).is_err());
            assert_eq!(f.queued_monitor(), queued);
            assert_eq!(
                f.host
                    .brain
                    .as_ref()
                    .unwrap()
                    .monitor_retirement
                    .unwrap()
                    .first_frame,
                first.first_frame
            );

            // An exact retransmission is idempotent, never a new retirement.
            f.host.negotiate_brain(&f.ctx,serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":f.ctx.writer(),"descriptor":f.descriptor})).unwrap();
            f.select(crate::brain_control::MonitorSource::None, 2);
            f.tick(2);
            assert_eq!(
                f.host
                    .brain
                    .as_ref()
                    .unwrap()
                    .monitor_retirement
                    .unwrap()
                    .first_ms,
                first.first_ms
            );
            let now = match case {
                0 => {
                    f.host.device_observed_ms = Some(251);
                    251
                }
                1 => {
                    let deadline = first.first_frame + 12_000;
                    while f.host.provider.frame() < deadline {
                        while f.host.brain.as_mut().unwrap().outgoing.pop().is_ok() {}
                        f.tick(2);
                        assert!(f.host.brain.is_some());
                    }
                    assert_eq!(f.host.provider.frame(), deadline);
                    2
                }
                2 => {
                    f.host.device_owner = None;
                    3
                }
                3 => {
                    f.host.device_observation.as_mut().unwrap().brain_epoch += 1;
                    3
                }
                4 => {
                    f.host.device_observation.as_mut().unwrap().status["fault"] =
                        serde_json::json!("xrun");
                    3
                }
                5 => {
                    f.host.device_observed_ms = None;
                    3
                }
                6 => {
                    f.host.device_observation.as_mut().unwrap().brain_map += 1;
                    3
                }
                7 => {
                    f.host.disconnect(&f.ctx);
                    3
                }
                8 => {
                    f.policy.replace(vec![]).unwrap();
                    3
                }
                9 => {
                    f.host.brain.as_mut().unwrap().descriptor.stagebox_map += 1;
                    3
                }
                _ => {
                    f.host.brain.as_mut().unwrap().descriptor.stagebox_epoch += 1;
                    3
                }
            };
            f.tick(now);
            assert!(f.host.brain.is_none(), "case {case}");
            assert_eq!(f.host.media_stats.dropped_monitor_packets, 0);
        }
    }
}

#[cfg(test)]
mod atomic_admission_tests {
    use super::*;
    use crate::{paired_readback as paired, topology::EngineTopology};
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn atomic_pair_capacity_includes_actual_remote_wrapper_at_exact_boundary() {
        for session in [1, u64::MAX] {
            let value = serde_json::to_vec(&Response::Reply {
                session: Counter(session),
                payload: Value::Null,
            })
            .unwrap();
            let overhead = value.len() - 4;
            let limit = crate::snapshot_pages::ASSEMBLY_BYTES - overhead;
            assert!(paired_transport_capacity(limit, session).is_ok());
            assert!(paired_transport_capacity(limit + 1, session).is_err());
            assert!(paired_transport_capacity(usize::MAX, session).is_err());
            let payload: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/atomic-control-v1/read-snapshot-48.json"
            ))
            .unwrap();
            let payload_bytes = serde_json::to_vec(&payload).unwrap().len();
            let wrapped = serde_json::to_vec(&Response::Reply {
                session: Counter(session),
                payload,
            })
            .unwrap();
            assert_eq!(wrapped.len(), payload_bytes + overhead);
        }
    }
    #[test]
    fn atomic_failed_pair_construction_changes_neither_read_latch() {
        let dir = std::env::temp_dir().join(format!("atomic-admission-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        let policy = PolicyStore::new(vec![Peer {
            id: "observer".into(),
            certificate_sha256: fingerprint(b"admission"),
            permissions: Default::default(),
        }])
        .unwrap();
        let context = policy.authenticate(b"admission", 101).unwrap();
        let local = LocalAudio::bind_configured(
            &dir,
            "audio",
            show,
            Counter(1),
            EngineTopology::software(48, 5, 0).unwrap(),
        )
        .unwrap();
        let mut host = HostAuthority::new(local, 1).unwrap();
        let request = paired::Request {
            contract: paired::CONTRACT.into(),
            version: 1,
            kind: "readback".into(),
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(1),
            authenticated_session: Counter(context.session()),
            writer: context.writer().into(),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            query_id: Counter(1),
        };
        let (raw, brain) = host.provider.paired_readback(&request, 10, 1).unwrap();
        let mut malformed = paired::Reply::new(request.clone(), Ok((raw.clone(), brain.clone())));
        malformed.brain.as_mut().unwrap().revision = Counter(u64::MAX);
        let refused = host
            .finish_paired_readback(&context, malformed, 10)
            .unwrap();
        assert_eq!(refused["reason"], "capacity");
        assert!(!host.reads.contains_key(&context.session()));
        assert!(!host.brain_reads.contains_key(&context.session()));
        assert!(!host.paired_queries.contains_key(&context.session()));
        let valid = paired::Reply::new(request.clone(), Ok((raw, brain)));
        assert_eq!(
            host.dispatch(&context, serde_json::to_value(&request).unwrap(), 10)
                .unwrap()
                .unwrap()["state"],
            "snapshot"
        );
        assert!(host.reads[&context.session()].audio_ready());
        assert_eq!(host.brain_reads[&context.session()], 10);
        let mut malformed = valid;
        malformed.raw.as_mut().unwrap().frame = Counter(96);
        assert_eq!(
            host.finish_paired_readback(&context, malformed, 99)
                .unwrap()["reason"],
            "capacity"
        );
        assert_eq!(host.brain_reads[&context.session()], 10);
        assert_eq!(host.paired_queries[&context.session()], 1);
        // Authenticated wrong attachment is checked before duplicate nonce admission.
        let mut wrong = request.clone();
        wrong.epoch = Counter(2);
        assert_eq!(
            host.dispatch(&context, serde_json::to_value(wrong).unwrap(), 11)
                .unwrap()
                .unwrap()["reason"],
            "identity"
        );
        for session in 102..102 + MAX_CONNECTIONS as u64 - 1 {
            let ctx = policy.authenticate(b"admission", session).unwrap();
            let mut r = request.clone();
            r.authenticated_session = Counter(session);
            r.writer = ctx.writer().into();
            assert_eq!(
                host.dispatch(&ctx, serde_json::to_value(r).unwrap(), 11)
                    .unwrap()
                    .unwrap()["state"],
                "snapshot"
            );
        }
        let ctx = policy.authenticate(b"admission", 200).unwrap();
        let mut r = request;
        r.authenticated_session = Counter(200);
        r.writer = ctx.writer().into();
        assert_eq!(
            host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 11)
                .unwrap()
                .unwrap()["reason"],
            "capacity"
        );
        host.disconnect(&context);
        assert_eq!(
            host.dispatch(&ctx, serde_json::to_value(r).unwrap(), 11)
                .unwrap()
                .unwrap()["state"],
            "snapshot"
        );
        drop(host);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[cfg(test)]
mod meter_reads_tests {
    use super::*;
    #[test]
    fn meter_reads_do_not_touch_paired_processing_leases_or_held_admission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gp-meter-authority-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        {
            let provider = LocalAudio::bind(&dir, "meter.sock", show, Counter(1)).unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let policy = PolicyStore::new(vec![Peer {
                id: "reader".into(),
                certificate_sha256: fingerprint(b"meter"),
                permissions: Default::default(),
            }])
            .unwrap();
            let ctx = policy.authenticate(b"meter", 1).unwrap();
            let mut r = crate::meter_wire::Request::new(show, 1, 1, 1);
            let result = host
                .dispatch(&ctx, serde_json::to_value(&r).unwrap(), 0)
                .unwrap()
                .unwrap();
            assert_eq!(result["reason"], "missing_window");
            assert!(host.reads.is_empty());
            assert!(host.brain_reads.is_empty());
            assert!(host.structural_reads.is_empty());
            assert!(host.paired_queries.is_empty());
            assert_eq!(host.provider.engine_mut().revision(), Counter(0));
            r.query_id = Counter(2);
            assert!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 39)
                    .unwrap()
                    .is_none()
            );
            assert!(host.poll_reply(&ctx, 39).unwrap().is_none());
            assert!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 39)
                    .is_err()
            );
            let deferred = host.poll_reply(&ctx, 40).unwrap().unwrap();
            assert_eq!(deferred["query_id"], "2");
            assert_eq!(deferred["reason"], "missing_window");
            assert!(host.pending_meter_reads.is_empty());
            assert_eq!(host.meter_reads[&ctx.session()], 40);
            assert!(host.poll_reply(&ctx, 41).unwrap().is_none());
            assert_eq!(host.provider.engine_mut().revision(), Counter(0));
            r.version = 2;
            assert_eq!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 80)
                    .unwrap()
                    .unwrap()["reason"],
                "unsupported"
            );
            // Exercise the real network/source handoff: pending must not become
            // a synthetic refusal before the final owner reply is available.
            let (mut proxy, mut mailbox) = authority_channel(&ctx, host.identity()).unwrap();
            proxy
                .dispatch(&ctx, serde_json::to_value(&r).unwrap(), 81)
                .unwrap();
            mailbox.service(&mut host, 81).unwrap();
            assert!(proxy.poll_refusal().unwrap().is_none());
            assert!(proxy.poll_reply(&ctx, 81).unwrap().is_none());
            mailbox.service(&mut host, 119).unwrap();
            assert!(proxy.poll_reply(&ctx, 119).unwrap().is_none());
            mailbox.service(&mut host, 120).unwrap();
            assert_eq!(
                proxy.poll_reply(&ctx, 120).unwrap().unwrap()["reason"],
                "unsupported"
            );
            assert!(proxy.poll_reply(&ctx, 120).unwrap().is_none());
            assert!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 121)
                    .unwrap()
                    .is_none()
            );
            policy.replace(vec![]).unwrap();
            assert!(host.poll_reply(&ctx, 160).is_err());
            assert!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 160)
                    .is_err()
            );
            host.disconnect(&ctx);
            assert!(host.pending_meter_reads.is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
