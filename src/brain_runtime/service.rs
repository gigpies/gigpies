//! Sole duplex service execution. All lanes contain fixed Copy values; the
//! controller owns negotiation, encoding, hashing, JSON and device descriptions.
use super::*;
use crate::brain_audio::{bridge::BridgeStatus, host::ServiceStatus};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

const CAPACITY: usize = 4;
#[derive(Clone, Copy)]
pub(super) struct State {
    pub closure: u64,
    pub revision: u64,
    pub epochs: BridgeEpochs,
    pub hold: u64,
    pub monitor: bool,
    pub levels: LocalLevels,
    pub expires: Instant,
}
#[derive(Clone, Copy)]
pub(super) struct Monitor {
    pub epochs: BridgeEpochs,
    pub frame: u64,
    pub samples: [f64; 96],
}
#[derive(Clone, Copy)]
pub(super) struct Status {
    pub observed_at: Instant,
    pub host: ServiceStatus,
    pub service_fault: Option<DeviceError>,
    pub bridge: BridgeStatus,
    pub capture_dropped: u64,
    pub monitor_rejected: u64,
    pub playback_dropped: u64,
    pub status_dropped: u64,
    pub state_refused: u64,
    pub ticks: u64,
    pub applied_revision: Option<u64>,
    pub closure: u64,
}
// Lossless sparse evidence for one physical-output chunk. BrainHost writes only
// the two mapped monitor slots; all other slots are checked for exact zero before
// compression. The controller reconstructs the configured width for hashing.
#[derive(Clone, Copy)]
pub(super) struct Playback {
    pub frame: u64,
    pub frames: usize,
    pub width: usize,
    pub revision: u64,
    pub slots: [usize; 2],
    pub samples: [f64; 96],
}
/// Requested layouts; intentionally not RSS or a complete process budget.
#[derive(Clone, Copy, Serialize)]
pub(super) struct PreparedStorage {
    pub core_inline_bytes: usize,
    pub copy_lane_slots_bytes: usize,
    pub host_sample_bytes: usize,
    pub bridge_sample_bytes: usize,
    pub fake_driver_sample_bytes: usize,
    pub fake_service_sample_bytes: usize,
    pub known_total_bytes: usize,
}
pub(super) struct Service {
    state: rtrb::Producer<State>,
    monitor: Option<rtrb::Producer<Monitor>>,
    captured: Option<rtrb::Consumer<Captured>>,
    pub playback: rtrb::Consumer<Playback>,
    status: rtrb::Consumer<Status>,
    closure: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<Status>>>,
    pub latest: Status,
    pub prepared_storage: PreparedStorage,
    pub capabilities: crate::brain_audio::device::DeviceCapabilities,
    pub state_dropped: u64,
    pub monitor_dropped: Arc<AtomicU64>,
}
pub(super) struct MediaLanes {
    pub captured: rtrb::Consumer<Captured>,
    pub monitor: rtrb::Producer<Monitor>,
    pub dropped: Arc<AtomicU64>,
}
fn prepared_lane<T: Copy>(value: T) -> (rtrb::Producer<T>, rtrb::Consumer<T>) {
    let (mut tx, mut rx) = rtrb::RingBuffer::new(CAPACITY);
    for _ in 0..CAPACITY {
        let _ = tx.push(value);
    }
    std::hint::black_box(&mut tx);
    for _ in 0..CAPACITY {
        let _ = rx.pop();
    }
    (tx, rx)
}
impl Service {
    pub fn start(
        config: DeviceConfig,
        physical: bool,
        epoch: u64,
        source: u64,
        skew: i32,
        deadline: Instant,
    ) -> Result<Self> {
        let epochs = BridgeEpochs {
            source,
            destination: epoch,
            route: u64::MAX,
        };
        let (state, states) = prepared_lane(State {
            closure: 0,
            revision: 0,
            epochs,
            hold: 0,
            monitor: false,
            levels: LocalLevels::default(),
            expires: Instant::now(),
        });
        let (monitor, monitors) = prepared_lane(Monitor {
            epochs,
            frame: 0,
            samples: [0.; 96],
        });
        let (capture, captured) = prepared_lane(Captured {
            epoch,
            frame: 0,
            hold_generation: 0,
            samples: [0.; 48],
        });
        let (playback_tx, playback) = prepared_lane(Playback {
            frame: 0,
            frames: 0,
            width: config.playback_channels,
            revision: 0,
            slots: [config.monitor[0].slot, config.monitor[1].slot],
            samples: [0.; 96],
        });
        let (status_tx, status) = rtrb::RingBuffer::new(CAPACITY);
        let closure = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_closure = closure.clone();
        let thread_stop = stop.clone();
        // Setup may allocate/open the explicitly authorized device, but it is
        // complete before readiness; no controller can access the live instance.
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        // The same absolute run deadline survives preparation and replacement.
        let thread = std::thread::Builder::new()
            .name("gp-brain-duplex".into())
            .spawn(move || {
                let prepared = Core::prepare(
                    config,
                    physical,
                    epoch,
                    source,
                    skew,
                    capture,
                    playback_tx,
                    status_tx,
                );
                let mut core = match prepared {
                    Ok(mut core) => {
                        for _ in 0..CAPACITY {
                            let _ = core.statuses.push(core.status());
                        }
                        let _ = ready_tx.send(Ok((
                            core.status(),
                            core.host.device_mut().capabilities().clone(),
                            core.prepared_storage(),
                        )));
                        core
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                let mut states = states;
                let mut monitors = monitors;
                while !thread_stop.load(Ordering::Acquire) && Instant::now() < deadline {
                    let now = Instant::now();
                    core.tick(
                        now,
                        thread_closure.load(Ordering::Acquire),
                        &mut states,
                        &mut monitors,
                    );
                    std::thread::sleep(Duration::from_micros(250));
                }
                core.host.stop();
                core.renderer.monitor.invalidate();
                Ok(core.status())
            })
            .map_err(|e| e.to_string())?;
        let (latest, capabilities, prepared_storage) =
            match ready_rx.recv().map_err(|e| e.to_string())? {
                Ok(status) => status,
                Err(error) => {
                    let _ = thread.join();
                    return Err(error);
                }
            };
        Ok(Self {
            state,
            monitor: Some(monitor),
            captured: Some(captured),
            playback,
            status,
            closure,
            stop,
            thread: Some(thread),
            latest,
            capabilities,
            prepared_storage,
            state_dropped: 0,
            monitor_dropped: Arc::new(AtomicU64::new(0)),
        })
    }
    /// Independent safety closure cannot be dropped behind a full ordinary lane.
    pub fn close(&mut self) -> Result<u64> {
        self.close_roles(true, true)
    }
    /// Two checked role generations share one atomic publication. Closing a
    /// monitor selection must never interrupt an unrelated held microphone.
    pub fn close_roles(&mut self, capture: bool, monitor: bool) -> Result<u64> {
        let old = self.closure.load(Ordering::Relaxed);
        let mut capture_generation = old as u32;
        let mut monitor_generation = (old >> 32) as u32;
        if capture {
            capture_generation = capture_generation
                .checked_add(1)
                .ok_or("capture closure exhausted")?;
        }
        if monitor {
            monitor_generation = monitor_generation
                .checked_add(1)
                .ok_or("monitor closure exhausted")?;
        }
        let next = u64::from(capture_generation) | (u64::from(monitor_generation) << 32);
        self.closure.store(next, Ordering::Release);
        Ok(next)
    }
    pub fn publish(&mut self, mut state: State) {
        state.closure = self.closure.load(Ordering::Acquire);
        if self.state.push(state).is_err() {
            self.state_dropped = self.state_dropped.saturating_add(1);
        }
    }
    pub fn take_media(&mut self) -> MediaLanes {
        MediaLanes {
            captured: self.captured.take().expect("single capture consumer"),
            monitor: self.monitor.take().expect("single monitor producer"),
            dropped: self.monitor_dropped.clone(),
        }
    }
    pub fn poll(&mut self) {
        for _ in 0..CAPACITY {
            match self.status.pop() {
                Ok(s) => self.latest = s,
                Err(_) => break,
            }
        }
    }
    pub fn finish(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            self.latest = thread.join().map_err(|_| "duplex service panicked")??;
        }
        Ok(())
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
struct Core {
    host: BrainHost<Device>,
    renderer: Renderer,
    state: Option<State>,
    closure: u64,
    expired_revision: Option<u64>,
    rearm_closed_revision: Option<u64>,
    monitor_arm_revision: Option<u64>,
    monitor_rejected: u64,
    playback_dropped: u64,
    status_dropped: u64,
    state_refused: u64,
    last_revision: u64,
    ticks: u64,
    fake_frame: u64,
    fake_playback_frame: u64,
    fake_origin: Option<Instant>,
    fake_capture: Vec<f64>,
    fake_output: Vec<f64>,
    skew: i32,
    playback: rtrb::Producer<Playback>,
    statuses: rtrb::Producer<Status>,
}
impl Core {
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        config: DeviceConfig,
        physical: bool,
        epoch: u64,
        source: u64,
        skew: i32,
        capture: rtrb::Producer<Captured>,
        playback: rtrb::Producer<Playback>,
        statuses: rtrb::Producer<Status>,
    ) -> Result<Self> {
        let host = BrainHost::prepare(device(&config, physical)?, config.clone(), epoch, epoch)
            .map_err(|e| e.to_string())?;
        let epochs = BridgeEpochs {
            source,
            destination: epoch,
            route: u64::MAX,
        };
        let renderer = Renderer {
            captured: capture,
            monitor: Bridge::prepare(BridgeConfig::voice(2), epochs).map_err(|e| e.to_string())?,
            epochs,
            fault: None,
            dropped: 0,
            hold_generation: 0,
        };
        let mut core = Self {
            host,
            renderer,
            state: None,
            closure: 0,
            expired_revision: None,
            rearm_closed_revision: None,
            monitor_arm_revision: None,
            monitor_rejected: 0,
            playback_dropped: 0,
            status_dropped: 0,
            state_refused: 0,
            last_revision: 0,
            ticks: 0,
            fake_frame: 0,
            fake_playback_frame: 0,
            fake_origin: None,
            fake_capture: vec![0.; config.period_frames * config.capture_channels],
            fake_output: vec![0.; config.period_frames * config.playback_channels],
            skew,
            playback,
            statuses,
        };
        core.host.prefault_prepared();
        if let Device::Fake(device) = core.host.device_mut() {
            device.prefault_prepared();
        }
        core.renderer.monitor.prefault_prepared();
        crate::brain_audio::prefault_samples(&mut core.fake_capture);
        crate::brain_audio::prefault_samples(&mut core.fake_output);
        let _ = monotonic_ms();
        Ok(core)
    }
    fn prepared_storage(&mut self) -> PreparedStorage {
        let core_inline_bytes = std::mem::size_of::<Self>();
        let copy_lane_slots_bytes = CAPACITY
            * (std::mem::size_of::<State>()
                + std::mem::size_of::<Monitor>()
                + std::mem::size_of::<Captured>()
                + std::mem::size_of::<Playback>()
                + std::mem::size_of::<Status>());
        let host_sample_bytes = self.host.prepared_sample_bytes();
        let bridge_sample_bytes = self.renderer.monitor.prepared_sample_bytes();
        let fake_driver_sample_bytes = match self.host.device_mut() {
            Device::Fake(d) => d.prepared_sample_bytes(),
            Device::Physical(_) => 0,
        };
        let fake_service_sample_bytes = (self.fake_capture.capacity()
            + self.fake_output.capacity())
            * std::mem::size_of::<f64>();
        let known_total_bytes = [
            core_inline_bytes,
            copy_lane_slots_bytes,
            host_sample_bytes,
            bridge_sample_bytes,
            fake_driver_sample_bytes,
            fake_service_sample_bytes,
        ]
        .into_iter()
        .try_fold(0usize, |a, b| a.checked_add(b))
        .expect("validated bounded duplex dimensions");
        PreparedStorage {
            core_inline_bytes,
            copy_lane_slots_bytes,
            host_sample_bytes,
            bridge_sample_bytes,
            fake_driver_sample_bytes,
            fake_service_sample_bytes,
            known_total_bytes,
        }
    }
    fn status(&self) -> Status {
        Status {
            observed_at: Instant::now(),
            host: self.host.service_status(),
            service_fault: self.renderer.fault,
            bridge: self.renderer.monitor.status(),
            capture_dropped: self.renderer.dropped,
            monitor_rejected: self.monitor_rejected,
            playback_dropped: self.playback_dropped,
            status_dropped: self.status_dropped,
            state_refused: self.state_refused,
            ticks: self.ticks,
            applied_revision: self.state.map(|s| s.revision),
            closure: self.closure,
        }
    }
    fn close(&mut self) {
        self.close_roles(true, true);
    }
    fn close_roles(&mut self, capture: bool, monitor: bool) {
        let mut levels = self.state.map_or_else(LocalLevels::default, |s| s.levels);
        if capture {
            self.renderer.hold_generation = 0;
            levels.microphone_muted = true;
            if let Some(state) = &mut self.state {
                state.hold = 0;
            }
        }
        if monitor {
            self.host.invalidate_monitor_output();
            levels.monitor_muted = true;
            if let Some(state) = &mut self.state {
                state.monitor = false;
            }
        }
        let _ = self.host.set_levels(levels);
        if capture && monitor {
            self.state = None;
        }
        if let Some(state) = &mut self.state {
            state.levels = levels;
        }
    }
    fn admit_rearm(&mut self, state: State) -> bool {
        let Some(expired) = self.expired_revision else {
            return true;
        };
        if state.revision < expired {
            return false;
        }
        if !state.monitor && state.hold == 0 {
            self.rearm_closed_revision = Some(state.revision);
            return true;
        }
        if self
            .rearm_closed_revision
            .is_some_and(|closed| state.revision > closed)
        {
            self.expired_revision = None;
            self.rearm_closed_revision = None;
            return true;
        }
        false
    }
    fn tick(
        &mut self,
        now: Instant,
        closure: u64,
        states: &mut rtrb::Consumer<State>,
        monitors: &mut rtrb::Consumer<Monitor>,
    ) {
        self.ticks = self.ticks.saturating_add(1);
        if closure != self.closure {
            self.close_roles(
                closure as u32 != self.closure as u32,
                closure >> 32 != self.closure >> 32,
            );
            self.closure = closure;
            if let Some(state) = &mut self.state {
                state.closure = closure;
            }
        }
        // One ordinary copied commit per service quantum, no control parsing.
        if let Ok(state) = states.pop() {
            if state.closure == closure
                && now < state.expires
                && state.epochs.destination == self.host.readback().epoch
                && state.revision >= self.last_revision
                && self.admit_rearm(state)
            {
                if self.host.set_levels(state.levels).is_err() {
                    self.close();
                    self.host.stop();
                    self.renderer.fault = Some(DeviceError::Configuration);
                    self.state = None;
                    self.state_refused = self.state_refused.saturating_add(1);
                } else {
                    if self.renderer.epochs != state.epochs {
                        self.host.invalidate_monitor_output();
                        let _ = self.renderer.monitor.reset(state.epochs);
                        self.renderer.epochs = state.epochs;
                        self.monitor_arm_revision = None;
                    }
                    self.renderer.hold_generation = state.hold;
                    if !self.host.is_armed()
                        && self.host.fault_reason().is_none()
                        && (state.hold != 0 || state.monitor)
                    {
                        let _ = self.host.arm(self.host.readback(), monotonic_ms());
                    }
                    self.last_revision = state.revision;
                    self.state = Some(state);
                }
            } else {
                self.state_refused = self.state_refused.saturating_add(1);
            }
        }
        if self.state.is_some_and(|s| now >= s.expires) {
            self.expired_revision = self.state.map(|s| s.revision);
            self.rearm_closed_revision = None;
            self.close();
            self.renderer.monitor.invalidate();
            self.state = None;
        }
        for _ in 0..CAPACITY {
            let Ok(packet) = monitors.pop() else { break };
            if !self
                .state
                .is_some_and(|s| s.monitor && s.epochs == packet.epochs)
                || self
                    .renderer
                    .monitor
                    .push(packet.epochs, packet.frame, &packet.samples)
                    .is_err()
            {
                self.monitor_rejected = self.monitor_rejected.saturating_add(1);
            }
        }
        if let Some(state) = self.state
            && state.monitor
            && self.monitor_arm_revision != Some(state.revision)
            && self.renderer.monitor.status().ready
            && !self.renderer.monitor.status().armed
            && self.renderer.monitor.arm(state.epochs).is_ok()
        {
            self.monitor_arm_revision = Some(state.revision);
        }
        if self.host.is_armed() {
            self.advance_fake(now);
            if self.host.is_armed() {
                let _ = self.host.service(monotonic_ms(), &mut self.renderer);
            }
        }
        if self.statuses.push(self.status()).is_err() {
            self.status_dropped = self.status_dropped.saturating_add(1);
        }
    }
    fn advance_fake(&mut self, now: Instant) {
        let period = self.host.config().period_frames;
        let width = self.host.config().playback_channels;
        let capture_width = self.host.config().capture_channels;
        let mic = self.host.config().microphone.slot;
        let slots = [
            self.host.config().monitor[0].slot,
            self.host.config().monitor[1].slot,
        ];
        let Device::Fake(fake) = self.host.device_mut() else {
            return;
        };
        let origin = *self.fake_origin.get_or_insert(now);
        let expected = (now.duration_since(origin).as_secs_f64()
            * 48_000.
            * (1. + f64::from(self.skew) * 1e-6)) as u64;
        if expected.saturating_sub(self.fake_frame) > 4800 {
            self.host.stop();
            self.renderer.fault = Some(DeviceError::Deadline);
            return;
        }
        if self.fake_frame + period as u64 <= expected {
            for (n, row) in self
                .fake_capture
                .chunks_exact_mut(capture_width)
                .enumerate()
            {
                row.fill(0.);
                row[mic] = if ((self.fake_frame + n as u64) / 24).is_multiple_of(2) {
                    0.0625
                } else {
                    -0.0625
                };
            }
            if let Err(e) = fake.advance(&self.fake_capture, period) {
                self.renderer.fault = Some(e);
                self.host.stop();
                return;
            }
            self.fake_frame += period as u64;
        }
        let frames = fake.drain_playback(&mut self.fake_output);
        for (i, chunk) in self.fake_output[..frames * width]
            .chunks(48 * width)
            .enumerate()
        {
            let mut block = Playback {
                frame: self.fake_playback_frame + i as u64 * 48,
                frames: chunk.len() / width,
                width,
                revision: self.state.map_or(0, |s| s.revision),
                slots,
                samples: [0.; 96],
            };
            for (frame, row) in chunk.chunks_exact(width).enumerate() {
                for (slot, sample) in row.iter().enumerate() {
                    if !slots.contains(&slot) && sample.to_bits() != 0 {
                        self.renderer.fault = Some(DeviceError::Mapping);
                        self.host.stop();
                        return;
                    }
                }
                block.samples[frame * 2] = row[slots[0]];
                block.samples[frame * 2 + 1] = row[slots[1]];
            }
            if self.playback.push(block).is_err() {
                self.playback_dropped = self.playback_dropped.saturating_add(1);
            }
        }
        self.fake_playback_frame += frames as u64;
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::brain_audio::device::{PortMap, SampleFormat};
    use crate::remote::assert_no_allocations as guarded;
    #[test]
    fn actual_duplex_monitor_asrc_renders_and_closes_without_allocating() {
        let (capture, mut captures) = rtrb::RingBuffer::new(4);
        let (playback, mut playbacks) = rtrb::RingBuffer::new(4);
        let (status, mut statuses) = rtrb::RingBuffer::new(4);
        let mut core = Core::prepare(
            {
                let mut c = config();
                c.capture_channels = 4;
                c.microphone.slot = 3;
                c.playback_channels = 18;
                c.monitor[0].slot = 7;
                c.monitor[1].slot = 17;
                c
            },
            false,
            2,
            1,
            0,
            capture,
            playback,
            status,
        )
        .unwrap();
        let (mut tx, mut states) = rtrb::RingBuffer::new(4);
        let (mut monitor_tx, mut monitors) = rtrb::RingBuffer::new(4);
        let now = Instant::now();
        let mut live = state(now);
        live.monitor = true;
        live.levels.monitor_muted = false;
        live.levels.monitor_gain_db = -6.;
        tx.push(live).unwrap();
        let mut energy = 0.;
        for n in 0..120 {
            monitor_tx
                .push(Monitor {
                    epochs: live.epochs,
                    frame: n * 48,
                    samples: [0.125; 96],
                })
                .unwrap();
            guarded(|| {
                core.tick(
                    now + Duration::from_millis(n),
                    0,
                    &mut states,
                    &mut monitors,
                )
            });
            while let Ok(p) = playbacks.pop() {
                assert_eq!(p.width, 18);
                assert_eq!(p.slots, [7, 17]);
                let mut reconstructed = vec![0.; p.frames * p.width];
                for (frame, row) in reconstructed.chunks_exact_mut(p.width).enumerate() {
                    row[p.slots[0]] = p.samples[frame * 2];
                    row[p.slots[1]] = p.samples[frame * 2 + 1];
                }
                assert_eq!(
                    reconstructed,
                    &core.fake_output[..p.frames * p.width],
                    "compressed evidence differs from actual submitted physical slots"
                );
                energy += p.samples[..p.frames * 2].iter().map(|v| v * v).sum::<f64>();
            }
            while captures.pop().is_ok() {}
            while statuses.pop().is_ok() {}
        }
        assert!(energy > 0.01);
        assert_eq!(core.renderer.monitor.status().overflows, 0);
        assert_eq!(core.renderer.monitor.status().underruns, 0);
        for _ in 0..4 {
            tx.push(live).unwrap();
        }
        guarded(|| {
            core.tick(
                now + Duration::from_millis(120),
                0x1_0000_0001,
                &mut states,
                &mut monitors,
            )
        });
        assert!(core.state.is_none());
        assert_eq!(core.renderer.hold_generation, 0);
        // A partial period already accepted by the device cannot be withdrawn;
        // every subsequently rendered period is muted by the closure.
        while playbacks.pop().is_ok() {}
        for n in 121..126 {
            guarded(|| {
                core.tick(
                    now + Duration::from_millis(n),
                    0x1_0000_0001,
                    &mut states,
                    &mut monitors,
                )
            });
            while let Ok(p) = playbacks.pop() {
                assert!(p.samples[..p.frames * 2].iter().all(|v| *v == 0.));
            }
        }
    }
    pub(crate) fn config() -> DeviceConfig {
        DeviceConfig {
            device_id: "fake-brain-duplex".into(),
            endpoint: "fake:brain".into(),
            sample_rate: 48000,
            capture_channels: 1,
            playback_channels: 2,
            format: SampleFormat::Float32Le,
            period_frames: 48,
            buffer_frames: 192,
            microphone: PortMap {
                id: "mic".into(),
                socket: "mic".into(),
                slot: 0,
            },
            monitor: [
                PortMap {
                    id: "left".into(),
                    socket: "left".into(),
                    slot: 0,
                },
                PortMap {
                    id: "right".into(),
                    socket: "right".into(),
                    slot: 1,
                },
            ],
            socket_mapping_record: None,
            shared_clock_record: None,
            hardware_monitoring_record: None,
        }
    }
    fn state(now: Instant) -> State {
        State {
            closure: 0,
            revision: 1,
            epochs: BridgeEpochs {
                source: 1,
                destination: 2,
                route: 3,
            },
            hold: 11,
            monitor: false,
            levels: LocalLevels {
                microphone_muted: false,
                ..LocalLevels::default()
            },
            expires: now + Duration::from_millis(150),
        }
    }
    #[test]
    fn monitor_closure_preserves_actual_held_capture_and_capture_closure_preserves_monitor() {
        let (capture, mut captured) = rtrb::RingBuffer::new(4);
        let (playback, mut played) = rtrb::RingBuffer::new(4);
        let (status, mut statuses) = rtrb::RingBuffer::new(4);
        let mut core = Core::prepare(config(), false, 2, 1, 0, capture, playback, status).unwrap();
        let (mut tx, mut states) = rtrb::RingBuffer::new(4);
        let (mut monitor_tx, mut monitors) = rtrb::RingBuffer::new(4);
        let now = Instant::now();
        let mut live = state(now);
        live.monitor = true;
        live.levels.monitor_muted = false;
        tx.push(live).unwrap();
        for n in 0..60 {
            monitor_tx
                .push(Monitor {
                    epochs: live.epochs,
                    frame: n * 48,
                    samples: [0.125; 96],
                })
                .unwrap();
            guarded(|| {
                core.tick(
                    now + Duration::from_millis(n),
                    0,
                    &mut states,
                    &mut monitors,
                )
            });
            while captured.pop().is_ok() {}
            while played.pop().is_ok() {}
            while statuses.pop().is_ok() {}
        }
        assert!(core.renderer.monitor.status().armed);
        // Capture-only safety publication must not touch monitor ASRC or gain.
        guarded(|| {
            core.tick(
                now + Duration::from_millis(60),
                1,
                &mut states,
                &mut monitors,
            )
        });
        assert_eq!(core.renderer.hold_generation, 0);
        assert!(core.renderer.monitor.status().armed);
        assert!(core.state.unwrap().monitor);
        assert!(!core.state.unwrap().levels.monitor_muted);
        while played.pop().is_ok() {}
        while statuses.pop().is_ok() {}
        live.closure = 1;
        live.revision = 2;
        live.hold = 12;
        tx.push(live).unwrap();
        guarded(|| {
            core.tick(
                now + Duration::from_millis(61),
                1,
                &mut states,
                &mut monitors,
            )
        });
        let first = captured.pop().unwrap();
        // Monitor-only safety publication preserves capture samples and frames.
        guarded(|| {
            core.tick(
                now + Duration::from_millis(62),
                0x1_0000_0001,
                &mut states,
                &mut monitors,
            )
        });
        let next = captured.pop().unwrap();
        assert_eq!((first.hold_generation, next.hold_generation), (12, 12));
        assert_eq!(next.frame, first.frame + 48);
        assert_eq!(core.renderer.hold_generation, 12);
        assert!(!core.state.unwrap().monitor);
        assert!(!core.state.unwrap().levels.microphone_muted);
    }
    #[test]
    fn service_duration_closes_without_controller_and_replacement_rejects_old_epoch() {
        let before = process_resources().unwrap();
        let mut owner = Service::start(
            config(),
            false,
            2,
            1,
            0,
            Instant::now() + Duration::from_millis(20),
        )
        .unwrap();
        owner.publish(state(Instant::now()));
        std::thread::sleep(Duration::from_millis(60));
        assert!(
            owner.thread.as_ref().unwrap().is_finished(),
            "finite service cap depends on controller progress"
        );
        owner.finish().unwrap();
        assert!(!owner.latest.host.armed);
        observation(&owner, &config(), None, None, None)
            .validate()
            .unwrap();
        let mut replacement = Service::start(
            config(),
            false,
            3,
            1,
            0,
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        replacement.publish(state(Instant::now()));
        // Epoch refusal is independent of the finite-run check above. Wait for
        // its actual observation rather than assuming preparation plus a first
        // service tick fits into a 20 ms wall-clock window on a cold machine.
        let refusal_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            replacement.poll();
            if replacement.latest.state_refused != 0 {
                break;
            }
            assert!(
                Instant::now() < refusal_deadline,
                "stale epoch was not observed"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        replacement.finish().unwrap();
        assert_eq!(replacement.latest.host.capture_frame, 0);
        assert_eq!(replacement.latest.state_refused, 1);
        assert_eq!(replacement.latest.host.readback.epoch, 3);
        // Replacing after the original deadline must not start another period.
        let mut expired = Service::start(config(), false, 4, 1, 0, Instant::now()).unwrap();
        let mut active = state(Instant::now());
        active.epochs.destination = 4;
        expired.publish(active);
        std::thread::sleep(Duration::from_millis(10));
        assert!(expired.thread.as_ref().unwrap().is_finished());
        expired.finish().unwrap();
        assert_eq!(expired.latest.ticks, 0);
        assert_eq!(expired.latest.host.capture_frame, 0);
        eprintln!(
            "duplex requested layouts: {}; test-process resources before: {}; after: {}",
            serde_json::to_string(&owner.prepared_storage).unwrap(),
            before,
            process_resources().unwrap()
        );
    }
    #[test]
    fn production_service_survives_stalled_controller_and_full_observation_lanes() {
        let mut service = Service::start(
            config(),
            false,
            2,
            1,
            0,
            Instant::now() + Duration::from_millis(150),
        )
        .unwrap();
        service.publish(state(Instant::now()));
        // Intentionally do not consume any lane or run an async executor.
        std::thread::sleep(Duration::from_millis(60));
        service.finish().unwrap();
        assert!(service.latest.host.capture_frame >= 48 * 10);
        assert!(service.latest.ticks > 10);
        assert!(service.latest.capture_dropped > 0);
        assert!(service.latest.playback_dropped > 0);
        assert!(service.latest.status_dropped > 0);
        assert_eq!(service.latest.host.faults, 0);
    }
    #[test]
    fn actual_duplex_core_closes_despite_full_commits_and_requires_fresh_revision_after_expiry() {
        let (capture, mut captures) = rtrb::RingBuffer::new(4);
        let (playback, _playbacks) = rtrb::RingBuffer::new(4);
        let (status, _statuses) = rtrb::RingBuffer::new(4);
        let mut core = Core::prepare(config(), false, 2, 1, 0, capture, playback, status).unwrap();
        let (mut tx, mut states) = rtrb::RingBuffer::new(4);
        let (_monitor_tx, mut monitors) = rtrb::RingBuffer::new(4);
        let now = Instant::now();
        tx.push(state(now)).unwrap();
        core.tick(now, 0, &mut states, &mut monitors);
        for n in 1..5 {
            core.tick(
                now + Duration::from_millis(n),
                0,
                &mut states,
                &mut monitors,
            );
        }
        assert!(captures.pop().is_ok());
        for _ in 0..4 {
            tx.push(state(now)).unwrap();
        }
        core.tick(
            now + Duration::from_millis(5),
            0x1_0000_0001,
            &mut states,
            &mut monitors,
        );
        assert_eq!(core.renderer.hold_generation, 0);
        assert!(core.state.is_none());
        for n in 6..9 {
            core.tick(
                now + Duration::from_millis(n),
                0x1_0000_0001,
                &mut states,
                &mut monitors,
            );
        }
        let mut fresh = state(now);
        fresh.closure = 0x1_0000_0001;
        fresh.revision = 2;
        fresh.expires = now + Duration::from_millis(12);
        tx.push(fresh).unwrap();
        core.tick(
            now + Duration::from_millis(10),
            0x1_0000_0001,
            &mut states,
            &mut monitors,
        );
        assert_eq!(core.renderer.hold_generation, 11);
        core.tick(
            now + Duration::from_millis(12),
            0x1_0000_0001,
            &mut states,
            &mut monitors,
        );
        assert_eq!(core.renderer.hold_generation, 0);
        fresh.expires = now + Duration::from_millis(150);
        tx.push(fresh).unwrap();
        core.tick(
            now + Duration::from_millis(13),
            0x1_0000_0001,
            &mut states,
            &mut monitors,
        );
        assert!(core.state.is_none());
        fresh.revision = 3;
        tx.push(fresh).unwrap();
        core.tick(
            now + Duration::from_millis(14),
            0x1_0000_0001,
            &mut states,
            &mut monitors,
        );
        assert_eq!(
            core.renderer.hold_generation, 0,
            "a newer unrelated revision is not rearm"
        );
        fresh.hold = 0;
        tx.push(fresh).unwrap();
        core.tick(
            now + Duration::from_millis(15),
            0x1_0000_0001,
            &mut states,
            &mut monitors,
        );
        fresh.revision = 4;
        fresh.hold = 12;
        tx.push(fresh).unwrap();
        core.tick(
            now + Duration::from_millis(16),
            0x1_0000_0001,
            &mut states,
            &mut monitors,
        );
        assert_eq!(core.renderer.hold_generation, 12);
        assert_eq!(core.host.readback().epoch, 2);
        fresh.levels.monitor_gain_db = f64::NAN;
        fresh.revision = 5;
        tx.push(fresh).unwrap();
        guarded(|| {
            core.tick(
                now + Duration::from_millis(17),
                0x1_0000_0001,
                &mut states,
                &mut monitors,
            )
        });
        assert_eq!(core.renderer.fault, Some(DeviceError::Configuration));
        assert_eq!(core.last_revision, 4);
        assert!(core.state.is_none());
        assert!(!core.host.is_armed());
    }
}
