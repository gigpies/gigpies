//! Source-frame execution for the legacy stereo delay. Preparation and native
//! destruction stay with the controller; media only transfers fixed sample blocks.
use super::{
    Result,
    aux_execution::{ExecutionLanes, prepared_ring},
};
use crate::{
    fx_wire::{self as wire, OwnerCommand as C, OwnerMessage as M},
    host::{
        brain_fx::BrainFx,
        fx_v2::{PreparedToken, Preparer, Status},
    },
    show::Counter,
};
use rtrb::{Consumer, Producer};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub(crate) struct Block {
    pub(crate) frame: u64,
    pub(crate) samples: Box<[f64]>,
}
pub(crate) struct MediaLanes {
    input: Producer<Block>,
    output: Consumer<Block>,
    pool: Vec<Block>,
    wake: std::thread::Thread,
    closure: Arc<AtomicU64>,
}
impl MediaLanes {
    fn close(&self) {
        self.closure.store(u64::MAX, Ordering::Release);
    }
}
impl ExecutionLanes for MediaLanes {
    type Output = Block;
    fn capacity(&self) -> usize {
        self.input
            .buffer()
            .capacity()
            .min(self.output.buffer().capacity())
    }
    fn submit(&mut self, frame: u64, samples: &[f64]) -> bool {
        let Some(mut block) = self.pool.pop() else {
            return false;
        };
        block.frame = frame;
        block.samples.copy_from_slice(samples);
        if let Err(rtrb::PushError::Full(block)) = self.input.push(block) {
            self.pool.push(block);
            return false;
        }
        self.wake.unpark();
        true
    }
    fn output(&mut self) -> Option<Block> {
        self.output.pop().ok()
    }
    fn data(block: &Block) -> (u64, &[f64]) {
        (block.frame, &block.samples)
    }
    fn recycle(&mut self, block: Block) {
        self.pool.push(block);
    }
    fn close(&self) {
        MediaLanes::close(self);
    }
}

#[derive(Clone, Copy, Default)]
struct Snapshot {
    status: Status,
    completion_status: Status,
    next: Option<u64>,
    ticket: u64,
    outcome: u8,
    settled: bool,
    refusal: u8,
    native_code: i32,
    terminal_fault: u8,
    terminal_native_code: i32,
    processed: u64,
    resets: u64,
    closed_blocks: u64,
    output_drops: u64,
    observation_drops: u64,
}
struct Commit {
    ticket: u64,
    frame: u64,
    expected_generation: u64,
    expected_reset: u64,
    deadline: Instant,
    panic: Option<u32>,
    token: Option<PreparedToken>,
}
struct Shared {
    closure: Arc<AtomicU64>,
    stop: AtomicBool,
    permit: AtomicU64,
    cancel: AtomicU64,
    acknowledged: AtomicU64,
}
struct Core {
    fx: BrainFx,
    frames: usize,
    wet: Box<[f64]>,
    stranded: Option<Block>,
    epoch: u64,
    shared: Arc<Shared>,
    policy: Option<super::policy::PolicyFence>,
    deadline: Instant,
    commits: Consumer<Commit>,
    retired: Producer<Commit>,
    pending: Option<Commit>,
    input: Consumer<Block>,
    output: Producer<Block>,
    observations: Producer<Snapshot>,
    state: Snapshot,
    dirty: bool,
    was_closed: bool,
}
impl Core {
    fn closed(&self) -> bool {
        self.shared.closure.load(Ordering::Acquire) != 1
            || Instant::now() >= self.deadline
            || self.policy.as_ref().is_some_and(|p| !p.valid())
    }
    fn close(&self) {
        self.shared.closure.store(u64::MAX, Ordering::Release);
    }
    fn admit(&mut self) {
        if self.pending.is_none() && self.retired.slots() > 0 {
            self.pending = self.commits.pop().ok();
        }
    }
    fn refuse_reason(&self, p: &Commit) -> u8 {
        if self.closed() {
            1
        } else if self.shared.cancel.load(Ordering::Acquire) == p.ticket {
            2
        } else if Instant::now() >= p.deadline {
            3
        } else {
            0
        }
    }
    fn complete(&mut self, p: Commit, refusal: u8, native_code: i32) {
        self.state.ticket = p.ticket;
        self.state.outcome = if refusal == 0 { 1 } else { 2 };
        self.state.refusal = refusal;
        self.state.settled = false;
        self.state.native_code = native_code;
        self.dirty = true;
        // This sole producer reserved a slot before consuming the commit.
        if let Err(rtrb::PushError::Full(p)) = self.retired.push(p) {
            self.pending = Some(p);
            self.close();
        }
    }
    fn publish(&mut self) {
        if self.dirty {
            if self.observations.push(self.state).is_ok() {
                self.dirty = false;
            } else {
                self.state.observation_drops = self.state.observation_drops.saturating_add(1);
            }
        }
    }
    fn unsettled(&self) -> bool {
        self.state.outcome == 1 && !self.state.settled
    }
    fn idle(&mut self) {
        self.admit();
        let reason = self.pending.as_ref().map_or(0, |p| self.refuse_reason(p));
        if reason != 0 {
            let p = self.pending.take().unwrap();
            self.complete(p, reason, 0);
        }
        if self.closed()
            && self.unsettled()
            && self.shared.acknowledged.load(Ordering::Acquire) != self.state.ticket
        {
            self.state.outcome = 2;
            self.state.refusal = 1;
            self.state.native_code = 0;
            self.dirty = true;
        }
        self.publish();
    }
    fn quantum(&mut self, mut block: Block) {
        self.admit();
        let closed = self.closed();
        let unsettled = self.unsettled();
        let gap = self.state.next.is_some_and(|f| f != block.frame);
        let valid = block.samples.len() == self.wet.len()
            && block.frame.checked_add(self.frames as u64).is_some()
            && block
                .samples
                .iter()
                .all(|s| s.is_finite() && s.abs() <= 16.);
        if !valid {
            self.state.terminal_fault = 5;
            self.state.terminal_native_code = 0;
            self.close();
        }
        if (closed && !self.was_closed) || !valid {
            // Reset is bounded and retains native ownership on this thread.
            if self.fx.reset_session(self.epoch).is_err() {
                self.close();
            }
        }
        self.was_closed = closed;
        if let Some(p) = self.pending.as_ref() {
            let mut reason = self.refuse_reason(p);
            if reason == 0 {
                reason = if gap {
                    4
                } else if !valid {
                    5
                } else if block.frame > p.frame {
                    6
                } else if self.state.status.applied_generation != p.expected_generation
                    || self.state.status.reset_count != p.expected_reset
                {
                    7
                } else {
                    0
                };
            }
            if reason != 0 || block.frame == p.frame {
                let p = self.pending.take().unwrap();
                if reason == 0 && self.shared.permit.load(Ordering::Acquire) != p.ticket {
                    reason = 8;
                }
                let rc = if reason != 0 {
                    0
                } else if let Some(owner) = self.fx.configured_owner() {
                    if let Some(mask) = p.panic {
                        owner.panic(mask)
                    } else if let Some(token) = &p.token {
                        owner.commit_token(token, block.frame)
                    } else {
                        -1
                    }
                } else {
                    -1
                };
                if rc != 0 {
                    reason = 9;
                }
                self.complete(p, reason, rc);
            }
        }
        self.wet.fill(0.);
        if !closed && valid {
            if let Err(error) =
                self.fx
                    .process(self.epoch, block.frame, &block.samples, &mut self.wet)
            {
                self.close();
                self.state.terminal_fault = 10;
                self.state.terminal_native_code = match error {
                    crate::host::brain_fx::FxError::Owner(code) => code,
                    _ => 0,
                };
                self.wet.fill(0.);
                if self.unsettled() {
                    self.state.outcome = 2;
                    self.state.refusal = 10;
                    self.state.native_code = match error {
                        crate::host::brain_fx::FxError::Owner(code) => code,
                        _ => 0,
                    };
                }
            } else {
                self.state.processed = self.state.processed.saturating_add(1);
            }
        } else if closed {
            self.state.closed_blocks = self.state.closed_blocks.saturating_add(1);
        }
        if let Some(owner) = self.fx.configured_owner() {
            match owner.status_bounded() {
                Ok(status) => {
                    self.state.status = status;
                    if self.state.outcome == 1 && !self.state.settled {
                        self.state.settled = status.remaining_frames == [0, 0]
                            && status.applied_generation == status.settled_generation;
                        if self.state.settled {
                            self.state.completion_status = status;
                        }
                    }
                }
                Err(_) => {
                    self.close();
                    self.wet.fill(0.);
                    self.state.terminal_fault = 11;
                    self.state.terminal_native_code = 0;
                    if self.unsettled() {
                        self.state.outcome = 2;
                        self.state.refusal = 11;
                        self.state.native_code = 0;
                    }
                }
            }
        }
        if (gap || closed || !valid)
            && unsettled
            && self.shared.acknowledged.load(Ordering::Acquire) != self.state.ticket
        {
            self.state.outcome = 2;
            self.state.refusal = if closed {
                1
            } else if gap {
                4
            } else {
                5
            };
            self.state.native_code = 0;
        }
        self.state.next = self.fx.next_source_frame();
        self.state.resets = self.fx.resets;
        block.samples.copy_from_slice(&self.wet);
        if let Err(rtrb::PushError::Full(block)) = self.output.push(block) {
            // Retain ownership on exceptional pressure. Never destroy sample
            // storage in native execution or accept another block over it.
            self.stranded = Some(block);
            self.state.output_drops = self.state.output_drops.saturating_add(1);
            self.close();
        }
        self.dirty = true;
        self.publish();
    }
}
struct Pending {
    ticket: u64,
    frame: u64,
    panic: Option<u32>,
    deadline: Instant,
    permitted: bool,
}
/// The controller owns this endpoint and joins before destroying native owners.
pub(crate) struct Execution {
    binding: Option<wire::Binding>,
    frames: usize,
    samples: usize,
    preparer: Option<Preparer>,
    state: Snapshot,
    pending: Option<Pending>,
    last_ticket: u64,
    shared: Arc<Shared>,
    commits: Producer<Commit>,
    retired: Consumer<Commit>,
    observations: Consumer<Snapshot>,
    lanes: Option<MediaLanes>,
    thread: Option<JoinHandle<Core>>,
}
impl Execution {
    pub(crate) fn from_owner(
        mut fx: BrainFx,
        binding: Option<wire::Binding>,
        epoch: u64,
        deadline: Instant,
        policy: Option<super::policy::PolicyFence>,
    ) -> Result<Self> {
        let frames = fx.max_block();
        if !matches!(frames, 48 | 96) {
            return Err("legacy FX execution frame shape".into());
        }
        let samples = frames
            .checked_mul(fx.channels())
            .ok_or("FX sample storage overflow")?;
        let preparer = fx.configured_owner().map(|o| o.preparer());
        let mut state = Snapshot::default();
        if let Some(owner) = fx.configured_owner() {
            state.status = owner.status()?;
        }
        let empty = || Commit {
            ticket: 0,
            frame: 0,
            expected_generation: 0,
            expected_reset: 0,
            deadline,
            panic: None,
            token: None,
        };
        let (commits, commit_rx) = prepared_ring(16, empty);
        let (retire_tx, retired) = prepared_ring(16, empty);
        let make_block = || Block {
            frame: 0,
            samples: vec![0.; samples].into_boxed_slice(),
        };
        let (input, input_rx) = prepared_ring(4, make_block);
        let (output_tx, output) = prepared_ring(4, make_block);
        let mut pool: Vec<Block> = (0..4).map(|_| make_block()).collect();
        for block in &mut pool {
            for sample in &mut block.samples {
                // SAFETY: exclusively owned initialized sample storage.
                unsafe {
                    std::ptr::write_volatile(sample, 0.);
                }
            }
        }
        let mut wet = vec![0.; samples].into_boxed_slice();
        for sample in &mut wet {
            // SAFETY: exclusively owned initialized sample storage.
            unsafe {
                std::ptr::write_volatile(sample, 0.);
            }
        }
        let (obs_tx, observations) = prepared_ring(128, Snapshot::default);
        let shared = Arc::new(Shared {
            closure: Arc::new(AtomicU64::new(1)),
            stop: AtomicBool::new(false),
            permit: AtomicU64::new(0),
            cancel: AtomicU64::new(0),
            acknowledged: AtomicU64::new(0),
        });
        let mut core = Core {
            fx,
            frames,
            wet,
            stranded: None,
            epoch,
            shared: shared.clone(),
            policy,
            deadline,
            commits: commit_rx,
            retired: retire_tx,
            pending: None,
            input: input_rx,
            output: output_tx,
            observations: obs_tx,
            state,
            dirty: false,
            was_closed: false,
        };
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("gp-brain-delay".into())
            .spawn(move || {
                let _ = ready_tx.send(());
                while !core.shared.stop.load(Ordering::Acquire) {
                    if core.stranded.is_some() {
                        core.idle();
                        std::thread::park_timeout(Duration::from_micros(250));
                        continue;
                    }
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
                core.close();
                #[cfg(all(test, target_os = "linux"))]
                super::diagnostics::assert_no_allocations(|| core.idle());
                #[cfg(not(all(test, target_os = "linux")))]
                core.idle();
                core
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|_| "legacy FX worker startup")?;
        let lanes = MediaLanes {
            pool,
            input,
            output,
            wake: thread.thread().clone(),
            closure: shared.closure.clone(),
        };
        Ok(Self {
            binding,
            frames,
            samples,
            preparer,
            state,
            pending: None,
            last_ticket: 0,
            shared,
            commits,
            retired,
            observations,
            lanes: Some(lanes),
            thread: Some(thread),
        })
    }
    pub(crate) fn take_media(&mut self) -> MediaLanes {
        self.lanes.take().expect("sole legacy FX media owner")
    }
    fn refresh(&mut self) {
        for _ in 0..128 {
            let Ok(state) = self.observations.pop() else {
                break;
            };
            if self.thread.is_some() {
                self.state = state;
            }
        }
        for _ in 0..16 {
            if self.retired.pop().is_err() {
                break;
            }
        }
    }
    pub(crate) fn observation(&mut self) -> Result<wire::Observation> {
        self.refresh();
        self.observation_for(self.state.status)
    }
    fn observation_for(&self, s: Status) -> Result<wire::Observation> {
        Ok(wire::Observation {
            binding: self.binding.clone().ok_or("FX controls unavailable")?,
            generation: Counter(s.applied_generation),
            settled_generation: Counter(s.settled_generation),
            applied_source_frame: Counter(s.applied_source_frame),
            settled_source_frame: Counter(s.settled_source_frame),
            next_source_frame: Counter(s.next_source_frame),
            reset_count: Counter(s.reset_count),
            remaining_frames: s.remaining_frames,
            owner_json: serde_json::to_string(&s.configuration()).map_err(|e| e.to_string())?,
        })
    }
    pub(crate) fn command(&mut self, command: C) -> Result<Option<M>> {
        self.refresh();
        match command {
            C::Prepare {
                ticket,
                mutation,
                apply_frame,
            } => {
                mutation.validate()?;
                let refuse = |reason: &str| {
                    Ok(Some(M::Refused {
                        ticket,
                        reason: reason.into(),
                    }))
                };
                if self.state.next.is_none() {
                    return refuse("FX source continuity not initialized");
                }
                if self.pending.is_some() {
                    return refuse("FX owner busy");
                }
                let o = self.observation()?;
                if self.shared.closure.load(Ordering::Acquire) != 1
                    || self.thread.is_none()
                    || ticket.0 == 0
                    || ticket.0 <= self.last_ticket
                    || mutation.binding != o.binding
                    || mutation.expected_generation != o.generation
                    || mutation.expected_reset_count != o.reset_count
                    || o.remaining_frames != [0, 0]
                    || apply_frame.0 <= o.next_source_frame.0
                    || !apply_frame.0.is_multiple_of(self.frames as u64)
                    || apply_frame.0 - o.next_source_frame.0 > 48_000
                {
                    return refuse("FX stale preparation");
                }
                if self.commits.slots() == 0 {
                    return refuse("FX commit lane full");
                }
                let token = if let Some(text) = &mutation.configuration_json {
                    let config = wire::Configuration::decode(text)?;
                    match self
                        .preparer
                        .as_ref()
                        .ok_or("FX controls unavailable")?
                        .prepare(&config, o.generation.0)
                    {
                        Ok(token) => Some(token),
                        Err(reason) => return Ok(Some(M::Refused { ticket, reason })),
                    }
                } else {
                    None
                };
                let deadline = Instant::now() + Duration::from_millis(wire::PREPARE_MS);
                let commit = Commit {
                    ticket: ticket.0,
                    frame: apply_frame.0,
                    expected_generation: o.generation.0,
                    expected_reset: o.reset_count.0,
                    deadline,
                    panic: mutation.panic_mask,
                    token,
                };
                self.commits
                    .push(commit)
                    .map_err(|_| "FX commit lane full")?;
                self.last_ticket = ticket.0;
                self.pending = Some(Pending {
                    ticket: ticket.0,
                    frame: apply_frame.0,
                    panic: mutation.panic_mask,
                    deadline,
                    permitted: false,
                });
                self.wake();
                Ok(Some(M::Prepared {
                    ticket,
                    binding: o.binding,
                    generation: o.generation,
                    configuration_json: mutation.configuration_json,
                    panic_mask: mutation.panic_mask,
                }))
            }
            C::Permit {
                ticket,
                binding,
                apply_frame,
            } => {
                let p = self
                    .pending
                    .as_mut()
                    .ok_or("FX permit without preparation")?;
                if p.ticket != ticket.0
                    || self.binding.as_ref() != Some(&binding)
                    || p.frame != apply_frame.0
                    || p.permitted
                    || Instant::now() >= p.deadline
                    || self.shared.closure.load(Ordering::Acquire) != 1
                    || self.state.next.is_some_and(|f| f > apply_frame.0)
                    || (self.state.ticket == ticket.0 && self.state.outcome != 0)
                {
                    return Err("FX invalid/late permit".into());
                }
                p.permitted = true;
                self.shared.permit.store(ticket.0, Ordering::Release);
                self.wake();
                Ok(None)
            }
            C::Cancel { ticket } => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.ticket == ticket.0 && !p.permitted)
                {
                    self.shared.cancel.store(ticket.0, Ordering::Release);
                    self.wake();
                }
                Ok(None)
            }
        }
    }
    fn wake(&self) {
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
    pub(crate) fn poll(&mut self) -> Result<Option<M>> {
        if self.binding.is_none() {
            self.refresh();
            return Ok(None);
        }
        let observation = self.observation()?;
        if let Some(p) = &self.pending {
            if self.state.ticket == p.ticket && self.state.outcome == 2 {
                let ticket = Counter(p.ticket);
                let reason = refusal(self.state.refusal).to_owned();
                self.shared.acknowledged.store(p.ticket, Ordering::Release);
                self.pending = None;
                return Ok(Some(M::Refused { ticket, reason }));
            }
            if self.state.ticket == p.ticket && self.state.outcome == 1 {
                let observation = if self.state.settled {
                    self.observation_for(self.state.completion_status)?
                } else {
                    observation
                };
                let ticket = Counter(p.ticket);
                let effective_source_frame = Counter(p.frame);
                let panic_mask = p.panic;
                if observation.remaining_frames == [0, 0]
                    && observation.generation == observation.settled_generation
                {
                    self.shared.acknowledged.store(p.ticket, Ordering::Release);
                    self.pending = None;
                }
                return Ok(Some(M::Completed {
                    ticket,
                    effective_source_frame,
                    panic_mask,
                    observation,
                }));
            }
        }
        Ok(Some(M::Observe { observation }))
    }
    pub(crate) fn close(&self) {
        self.shared.closure.store(u64::MAX, Ordering::Release);
        self.wake();
    }
    pub(crate) fn finish(&mut self) {
        self.close();
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            match thread.join() {
                Ok(core) => {
                    self.state = core.state;
                    drop(core);
                }
                Err(_) => {
                    self.state.terminal_fault = 12;
                    self.state.terminal_native_code = 0;
                }
            }
        }
        self.refresh();
    }
    pub(crate) fn processed(&self) -> u64 {
        self.state.processed
    }
    pub(crate) fn resets(&self) -> u64 {
        self.state.resets
    }
    pub(crate) fn report(&self) -> serde_json::Value {
        serde_json::json!({"owner":"legacy_delay", "processed_blocks":self.state.processed,
            "last_ticket":self.state.ticket, "last_refusal":self.state.refusal,
            "last_refusal_native_code":self.state.native_code,
            "terminal_fault":self.state.terminal_fault,"terminal_native_code":self.state.terminal_native_code,
            "closed_blocks":self.state.closed_blocks,"output_drops":self.state.output_drops,
            "observation_drops":self.state.observation_drops,
            "frames":self.frames,"sample_storage_bytes":5*self.samples*size_of::<f64>(),
            "queue_payload_bytes":32*size_of::<Commit>()+12*size_of::<Block>()+128*size_of::<Snapshot>()})
    }
}
impl Drop for Execution {
    fn drop(&mut self) {
        self.finish();
    }
}
fn refusal(code: u8) -> &'static str {
    match code {
        1 => "FX owner closed",
        2 => "FX preparation cancelled",
        3 => "FX preparation deadline",
        4 => "FX source gap/late boundary",
        5 => "FX invalid source samples",
        6 => "FX source gap/late boundary",
        7 => "FX stale preparation",
        8 => "FX permit absent at target",
        9 => "FX owner commit refused",
        10 => "FX owner process refused",
        11 => "FX owner status refused",
        _ => "FX owner refused",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    #[ignore = "verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_legacy_shapes_keep_samples_and_bounded_ownership() {
        let path = std::path::PathBuf::from(std::env::var_os("GP_FX_LIBRARY").unwrap());
        for frames in [48, 96] {
            for channels in [1, 2, 3, 16, 48, 256] {
                let mut owner = BrainFx::prepare(&path, channels, frames, 2).unwrap();
                let mut reference = BrainFx::prepare(&path, channels, frames, 2).unwrap();
                if channels == 2 && frames == 96 {
                    assert!(owner.enable_configured(&path).unwrap());
                    assert!(reference.enable_configured(&path).unwrap());
                }
                let mut execution = Execution::from_owner(
                    owner,
                    None,
                    2,
                    Instant::now() + Duration::from_secs(3),
                    None,
                )
                .unwrap();
                let mut lanes = execution.take_media();
                let input: Vec<_> = (0..frames * channels)
                    .map(|i| ((i % 17) as f64 - 8.) / 128.)
                    .collect();
                let mut expected = vec![0.; input.len()];
                let mut nonzero = 0;
                for n in 0..32 {
                    let frame = n * frames as u64;
                    reference.process(2, frame, &input, &mut expected).unwrap();
                    assert!(lanes.submit(frame, &input));
                    let until = Instant::now() + Duration::from_secs(1);
                    let block = loop {
                        if let Some(block) = lanes.output() {
                            break block;
                        }
                        assert!(Instant::now() < until, "{channels}x{frames} stalled");
                        std::thread::sleep(Duration::from_micros(100));
                    };
                    assert_eq!(block.frame, frame);
                    assert_eq!(&*block.samples, expected);
                    nonzero += block.samples.iter().filter(|s| **s != 0.).count();
                    lanes.recycle(block);
                }
                assert!(nonzero > 0);
                assert_eq!(lanes.pool.len(), 4);
                lanes.close();
                assert!(lanes.submit(32 * frames as u64, &input));
                let until = Instant::now() + Duration::from_secs(1);
                let closed = loop {
                    if let Some(block) = lanes.output() {
                        break block;
                    }
                    assert!(Instant::now() < until);
                    std::thread::sleep(Duration::from_micros(100));
                };
                assert!(closed.samples.iter().all(|s| *s == 0.));
                lanes.recycle(closed);
                execution.finish();
                assert_eq!(execution.state.processed, 32);
                assert_eq!(execution.state.output_drops, 0);
                assert_eq!(execution.state.terminal_fault, 0);
                assert_eq!(lanes.pool.len(), 4);
            }
        }
    }
    fn setup(deadline: Instant) -> (Execution, MediaLanes) {
        let path = std::path::PathBuf::from(
            std::env::var_os("GP_FX_LIBRARY").expect("verified FX library"),
        );
        let hash = crate::hash::hex(Sha256::digest(std::fs::read(&path).unwrap()));
        let mut fx = BrainFx::prepare(&path, 2, 48, 2).unwrap();
        assert!(fx.enable_configured(&path).unwrap());
        let binding = wire::Binding {
            session: Counter(1),
            source_epoch: Counter(2),
            capability_generation: Counter(3),
            map_generation: Counter(4),
            owner_instance: Counter(1),
            library_sha256: hash,
            abi_version: 2,
            config_size: 104,
            status_size: 168,
            capabilities_size: 128,
            channels: ["foh-left".into(), "foh-right".into()],
        };
        let mut execution = Execution::from_owner(fx, Some(binding), 2, deadline, None).unwrap();
        let lanes = execution.take_media();
        (execution, lanes)
    }
    fn block(lanes: &mut MediaLanes, frame: u64) -> Block {
        assert!(
            lanes
                .input
                .push(Block {
                    frame,
                    samples: vec![0.25; 96].into_boxed_slice()
                })
                .is_ok()
        );
        lanes.wake.unpark();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(block) = lanes.output.pop() {
                return block;
            }
            assert!(Instant::now() < deadline, "legacy FX output timeout");
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    fn prepare(execution: &mut Execution, ticket: u64, frame: u64) -> wire::Binding {
        let deadline = Instant::now() + Duration::from_secs(2);
        let observation = loop {
            let observation = execution.observation().unwrap();
            if execution.state.next.is_some() {
                break observation;
            }
            assert!(
                Instant::now() < deadline,
                "legacy FX initial observation timeout"
            );
            std::thread::sleep(Duration::from_micros(100));
        };
        let mut config = wire::Configuration::decode(&observation.owner_json).unwrap();
        config.channels[0].delay_ms = 2.;
        config.channels[1].delay_ms = 3.;
        let binding = observation.binding;
        let mutation = wire::Mutation {
            binding: binding.clone(),
            expected_generation: observation.generation,
            expected_reset_count: observation.reset_count,
            lead_frames: Counter(48),
            configuration_json: Some(serde_json::to_string(&config).unwrap()),
            panic_mask: None,
        };
        assert!(matches!(
            execution
                .command(C::Prepare {
                    ticket: Counter(ticket),
                    mutation,
                    apply_frame: Counter(frame)
                })
                .unwrap(),
            Some(M::Prepared { .. })
        ));
        binding
    }
    fn await_outcome(execution: &mut Execution, ticket: u64) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            execution.refresh();
            if execution.state.ticket == ticket && execution.state.outcome != 0 {
                return;
            }
            assert!(Instant::now() < deadline, "legacy FX completion timeout");
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_legacy_fx_retains_settled_completion_through_saturation_and_closure() {
        let (mut execution, mut lanes) = setup(Instant::now() + Duration::from_secs(60));
        block(&mut lanes, 0);
        let binding = prepare(&mut execution, 1, 96);
        execution
            .command(C::Permit {
                ticket: Counter(1),
                binding,
                apply_frame: Counter(96),
            })
            .unwrap();
        let mut energy = 0.;
        // No controller operation through transition and observation saturation.
        for n in 1..=180 {
            energy += block(&mut lanes, n * 48).samples[..96]
                .iter()
                .map(|s| s * s)
                .sum::<f64>();
        }
        assert!(energy > 0.);
        execution.close();
        assert!(block(&mut lanes, 181 * 48).samples.iter().all(|s| *s == 0.));
        execution.finish();
        assert!(execution.state.observation_drops > 0);
        assert!(matches!(execution.poll().unwrap(), Some(M::Completed {
            ticket:Counter(1), effective_source_frame:Counter(96), observation, ..
        }) if observation.generation == Counter(1) && observation.settled_generation == Counter(1)
            && observation.applied_source_frame == Counter(96)));
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_legacy_fx_idle_expiry_cancel_and_closure_retain_refusal() {
        for cause in [1, 2, 3] {
            let (mut execution, mut lanes) = setup(Instant::now() + Duration::from_secs(60));
            block(&mut lanes, 0);
            prepare(&mut execution, 1, 96);
            if cause == 1 {
                lanes.close();
            } else if cause == 2 {
                execution.command(C::Cancel { ticket: Counter(1) }).unwrap();
            }
            // For expiry, use the actual copied production PREPARE_MS deadline.
            // Neither source blocks nor controller polling generates the receipt.
            let deadline = Instant::now() + Duration::from_millis(wire::PREPARE_MS + 1_000);
            loop {
                if let Ok(state) = execution.observations.pop()
                    && state.ticket == 1
                    && state.outcome == 2
                {
                    execution.state = state;
                    break;
                }
                assert!(Instant::now() < deadline, "legacy idle refusal timeout");
                std::thread::sleep(Duration::from_micros(100));
            }
            assert_eq!(execution.state.refusal, cause);
            assert!(matches!(
                execution.poll().unwrap(),
                Some(M::Refused {
                    ticket: Counter(1),
                    ..
                })
            ));
            assert_eq!(execution.state.status.applied_generation, 0);
        }
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_legacy_fx_absolute_deadline_refuses_without_another_source_block() {
        let (mut execution, mut lanes) = setup(Instant::now() + Duration::from_millis(500));
        block(&mut lanes, 0);
        prepare(&mut execution, 1, 96);
        await_outcome(&mut execution, 1);
        assert_eq!(execution.state.refusal, 1);
        assert_eq!(execution.state.status.applied_generation, 0);
    }
    #[test]
    #[ignore = "requires verified actual SHR FX library via GP_FX_LIBRARY"]
    fn actual_legacy_fx_retirement_pressure_does_not_block_independent_closure() {
        let (mut execution, mut lanes) = setup(Instant::now() + Duration::from_secs(60));
        block(&mut lanes, 0);
        execution.refresh();
        for ticket in 1..=17 {
            assert!(
                execution
                    .commits
                    .push(Commit {
                        ticket,
                        frame: ticket * 48,
                        expected_generation: 0,
                        expected_reset: execution.state.status.reset_count,
                        deadline: Instant::now() + Duration::from_secs(60),
                        panic: None,
                        token: None,
                    })
                    .is_ok()
            );
            block(&mut lanes, ticket * 48);
        }
        assert_eq!(execution.retired.slots(), 16);
        assert_eq!(execution.commits.slots(), 15);
        lanes.close();
        assert!(block(&mut lanes, 18 * 48).samples.iter().all(|s| *s == 0.));
        assert_eq!(execution.commits.slots(), 15);
        execution.refresh();
        await_outcome(&mut execution, 17);
        assert_eq!(execution.state.refusal, 1);
    }
}
