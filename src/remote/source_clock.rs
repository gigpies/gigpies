//! Bounded source-clock progression over the production graph and media owners.
//! No controller reference, protocol parsing or owner destruction runs here.
use super::HostSourceMedia;
use crate::local_audio::{
    LocalAudio,
    source::{RenderGraph, RenderIntent},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct SourceClockStatus {
    recovery: Option<(Duration, Instant)>,
    recovery_deadline: Option<Instant>,
    closed: Arc<AtomicBool>,
    blocks: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
}
impl SourceClockStatus {
    pub(super) fn recovery_budget(&self) -> Result<(Duration, Instant, u64), String> {
        let (period, deadline) = self
            .recovery
            .ok_or("physical source recovery requires device revalidation")?;
        let blocks = deadline
            .saturating_duration_since(Instant::now())
            .as_nanos()
            / period.as_nanos();
        if blocks == 0 {
            return Err("source recovery run deadline expired".into());
        }
        Ok((period, deadline, blocks as u64))
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    pub fn blocks(&self) -> u64 {
        self.blocks.load(Ordering::Acquire)
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(super) fn mark_stopped(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub(super) fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.closed, &other.closed)
    }
}
/// Numeric identity observed on the source worker for the samples in a window.
/// Revision is the successfully applied source permit/command revision, not a
/// controller snapshot. Expiry/closure may change held state without a revision.
#[derive(Debug, serde::Serialize)]
pub struct SourceWindowIdentity {
    pub remote_fx_applied: Option<(u64, u64)>,
    pub revision: u64,
    pub map_revision: u64,
    pub selection_generation: u64,
    pub monitor: crate::brain_control::MonitorSource,
    pub monitor_audible: bool,
    pub talkback_held: bool,
    pub talkback_foh: bool,
    pub talkback_gain: f64,
    pub talkback_deadline_frame: Option<u64>,
    pub talkback_monitors: Vec<bool>,
}
struct WindowIdentity<'a> {
    remote_fx_applied: Option<(u64, u64)>,
    revision: u64,
    map_revision: u64,
    routing: &'a crate::local_audio::source::ClockRouting,
    held: bool,
    monitor_audible: bool,
}
impl SourceWindowIdentity {
    fn matches(&self, current: &WindowIdentity<'_>) -> bool {
        self.remote_fx_applied == current.remote_fx_applied
            && self.revision == current.revision
            && self.map_revision == current.map_revision
            && self.selection_generation == current.routing.selection_generation
            && self.monitor == current.routing.monitor
            && self.monitor_audible == current.monitor_audible
            && self.talkback_held == current.held
            && self.talkback_foh == current.routing.foh
            && self.talkback_gain == current.routing.gain
            && self.talkback_deadline_frame == current.routing.deadline_frame
            && self
                .talkback_monitors
                .iter()
                .enumerate()
                .all(|(i, enabled)| *enabled == current.routing.monitors.contains(&i))
    }
    fn capture(&mut self, current: &WindowIdentity<'_>) {
        self.remote_fx_applied = current.remote_fx_applied;
        self.revision = current.revision;
        self.map_revision = current.map_revision;
        self.selection_generation = current.routing.selection_generation;
        self.monitor = current.routing.monitor;
        self.monitor_audible = current.monitor_audible;
        self.talkback_held = current.held;
        self.talkback_foh = current.routing.foh;
        self.talkback_gain = current.routing.gain;
        self.talkback_deadline_frame = current.routing.deadline_frame;
        for (i, enabled) in self.talkback_monitors.iter_mut().enumerate() {
            *enabled = current.routing.monitors.contains(&i);
        }
    }
}
/// Exact source-frame windows, independent of controller service cadence.
/// Rows split at identity changes and otherwise cover at most 100 quanta.
#[derive(Debug, serde::Serialize)]
pub struct SourceOutputWindow {
    pub identity: SourceWindowIdentity,
    pub first_frame: u64,
    pub last_frame: u64,
    pub blocks: u64,
    pub peaks: Vec<f64>,
    pub energy: Vec<f64>,
    pub nonzero_wet_samples: u64,
    pub rendered_wet_energy: f64,
}
fn prepare_windows(
    limit: u64,
    width: usize,
    monitors: usize,
    capacity: usize,
) -> Vec<SourceOutputWindow> {
    (0..(limit as usize).min(capacity))
        .map(|_| SourceOutputWindow {
            identity: SourceWindowIdentity {
                remote_fx_applied: None,
                revision: 0,
                map_revision: 0,
                selection_generation: 0,
                monitor: crate::brain_control::MonitorSource::None,
                monitor_audible: false,
                talkback_held: false,
                talkback_foh: false,
                talkback_gain: 0.,
                talkback_deadline_frame: None,
                talkback_monitors: vec![false; monitors],
            },
            first_frame: 0,
            last_frame: 0,
            blocks: 0,
            peaks: vec![0.; width],
            energy: vec![0.; width],
            nonzero_wet_samples: 0,
            rendered_wet_energy: 0.,
        })
        .collect()
}
#[derive(Debug, serde::Serialize)]
pub struct SourceClockReport {
    pub epoch: u64,
    pub first_frame: u64,
    pub next_frame: u64,
    pub blocks: u64,
    pub fault: Option<&'static str>,
    pub peaks: Vec<f64>,
    pub energy: Vec<f64>,
    pub monitor_drops: u64,
    pub egress_drops: u64,
    pub nonzero_wet_samples: u64,
    pub rendered_wet_energy: f64,
    pub output_window_frames: u64,
    pub output_windows: Vec<SourceOutputWindow>,
    pub output_windows_overflow_blocks: u64,
    #[serde(skip)]
    output_windows_used: usize,
}
impl SourceClockReport {
    /// Called only after ownership returns to the controller. Unused rows and
    /// their allocated vectors must never be destroyed on the source worker.
    pub fn finish_observations(&mut self) {
        self.output_windows.truncate(self.output_windows_used);
    }
    fn observe(&mut self, frame: u64, playback: &[f64], wet: &[f64], identity: WindowIdentity<'_>) {
        let wet_energy = wet.iter().map(|s| s * s).sum::<f64>();
        let wet_samples = wet.iter().filter(|s| **s != 0.).count() as u64;
        self.rendered_wet_energy += wet_energy;
        self.nonzero_wet_samples += wet_samples;
        let split = self.output_windows_used == 0
            || self
                .output_windows
                .get(self.output_windows_used - 1)
                .is_none_or(|window| window.blocks == 100 || !window.identity.matches(&identity));
        if split && self.output_windows_overflow_blocks == 0 {
            self.output_windows_used += 1;
        }
        // Once the retained prefix is full, never merge later samples into its
        // last row, even if their identity matches again.
        let window = (self.output_windows_overflow_blocks == 0)
            .then(|| self.output_windows.get_mut(self.output_windows_used - 1))
            .flatten();
        if window.is_none() {
            self.output_windows_overflow_blocks += 1;
        }
        let mut window = window;
        if let Some(window) = window.as_mut() {
            if window.blocks == 0 {
                window.first_frame = frame;
                window.identity.capture(&identity);
            }
            window.last_frame = frame + 47;
            window.blocks += 1;
            window.rendered_wet_energy += wet_energy;
            window.nonzero_wet_samples += wet_samples;
        }
        for row in playback.chunks_exact(self.peaks.len()) {
            for (channel, sample) in row.iter().enumerate() {
                self.peaks[channel] = self.peaks[channel].max(sample.abs());
                self.energy[channel] += sample * sample;
                if let Some(window) = window.as_mut() {
                    window.peaks[channel] = window.peaks[channel].max(sample.abs());
                    window.energy[channel] += sample * sample;
                }
            }
        }
    }
}
pub(super) struct SourceClock {
    pub status: SourceClockStatus,
    pub report: SourceClockReport,
    device: Option<crate::host::duplex::Duplex>,
    playback_pending: bool,
    device_started: bool,
    capture: Vec<f64>,
    playback: Vec<f64>,
    period: Duration,
    limit: u64,
    started: Option<Instant>,
    next: Option<Instant>,
    now_ms: u64,
}
fn clock_fits(period: Duration, limit: u64, first_frame: u64) -> bool {
    period >= Duration::from_millis(1)
        && (1..=60_000).contains(&limit)
        && period
            .checked_mul(limit as u32)
            .is_some_and(|d| d <= Duration::from_secs(60))
        && first_frame.checked_add(limit * 48).is_some()
}
impl SourceClock {
    pub fn prepare(
        provider: &LocalAudio,
        now_ms: u64,
        period: Duration,
        limit: u64,
    ) -> Result<Self, String> {
        // Draining an old epoch can observe a later time than the recovery
        // caller sampled. Carry that time forward without extending its run
        // deadline or advancing the replacement source-frame timeline.
        let now_ms = now_ms.max(provider.source_control_time());
        // Charge preparation and queue delay against copied local deadlines.
        let started = Instant::now();
        if !clock_fits(period, limit, provider.frame()) {
            return Err("source clock duration/frame bound".into());
        }
        let topology = provider.topology();
        Ok(Self {
            status: SourceClockStatus {
                recovery: Some((period, started + period * limit as u32)),
                recovery_deadline: None,
                closed: Arc::new(AtomicBool::new(false)),
                blocks: Arc::new(AtomicU64::new(0)),
                stopped: Arc::new(AtomicBool::new(false)),
            },
            report: SourceClockReport {
                epoch: provider.source_epoch(),
                first_frame: provider.frame(),
                next_frame: provider.frame(),
                blocks: 0,
                fault: None,
                peaks: vec![0.; topology.playback_channels],
                energy: vec![0.; topology.playback_channels],
                monitor_drops: 0,
                egress_drops: 0,
                nonzero_wet_samples: 0,
                rendered_wet_energy: 0.,
                output_window_frames: 4800,
                output_windows: prepare_windows(
                    limit,
                    topology.playback_channels,
                    topology.monitors,
                    600,
                ),
                output_windows_overflow_blocks: 0,
                output_windows_used: 0,
            },
            device: None,
            playback_pending: false,
            device_started: false,
            capture: vec![0.; 48 * topology.capture_channels],
            playback: vec![0.; 48 * topology.playback_channels],
            period,
            limit,
            started: Some(started),
            next: None,
            now_ms,
        })
    }
    pub fn prepare_device(
        provider: &LocalAudio,
        now_ms: u64,
        limit: u64,
        device: crate::host::duplex::Duplex,
    ) -> Result<Self, String> {
        device
            .validate_source_clock(provider.topology())
            .map_err(|e| e.to_string())?;
        if !(1..=86_400_000).contains(&limit)
            || limit
                .checked_mul(48)
                .and_then(|frames| provider.frame().checked_add(frames))
                .is_none()
        {
            return Err("physical source duration/frame bound".into());
        }
        let mut state = Self::prepare(provider, now_ms, Duration::from_millis(1), 1)?;
        state.limit = limit;
        state.report.output_windows = prepare_windows(
            limit,
            provider.topology().playback_channels,
            provider.topology().monitors,
            2048,
        );
        state.device = Some(device);
        state.status.recovery = None;
        Ok(state)
    }
    pub(super) fn retain_run_deadline(&mut self, deadline: Instant) {
        self.status.recovery = Some((self.period, deadline));
        self.status.recovery_deadline = Some(deadline);
    }
    pub(super) fn observed_now_ms(&self) -> u64 {
        self.now_ms.saturating_add(
            self.started
                .map_or(0, |started| started.elapsed().as_millis() as u64),
        )
    }
    fn close(&mut self, graph: &mut RenderGraph, fault: Option<&'static str>) -> Duration {
        self.report.next_frame = graph.mixer.mixer.frame();
        graph.close_numeric();
        if let Some(device) = &mut self.device {
            device.stop_source_clock();
        }
        self.report.fault = fault;
        self.status.stopped.store(true, Ordering::Release);
        Duration::from_millis(1)
    }
    pub fn advance(
        graph: &mut Box<RenderGraph>,
        media: &mut HostSourceMedia,
        state: &mut Self,
        now: Instant,
    ) -> Duration {
        if state.status.stopped() {
            if let Some(handoff) = &media.brain_handoff {
                handoff.close();
            }
            return Duration::from_millis(1);
        }
        if state
            .status
            .recovery_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            return state.close(graph, None);
        }
        if state.status.closed.load(Ordering::Acquire) {
            return state.close(graph, None);
        }
        if graph.closure.load(Ordering::Acquire) {
            return state.close(graph, Some("source closed"));
        }
        if !state.device_started
            && let Some(device) = &mut state.device
        {
            if let Err(fault) = device.start_source_clock() {
                return state.close(graph, Some(fault));
            }
            state.device_started = true;
        }
        if state.playback_pending {
            match state.device.as_mut().unwrap().poll_source_playback(now) {
                Ok(true) => state.playback_pending = false,
                Ok(false) => return Duration::from_micros(50),
                Err(fault) => return state.close(graph, Some(fault)),
            }
        }
        if state.report.blocks == state.limit {
            return state.close(graph, None);
        }
        let started = *state.started.get_or_insert(now);
        let next = *state.next.get_or_insert(now);
        if state.device.is_none() && now < next {
            return next.duration_since(now).min(Duration::from_millis(1));
        }
        if state.device.is_none()
            && now.saturating_duration_since(next) > Duration::from_millis(100)
        {
            return state.close(graph, Some("source scheduling debt exceeded100ms"));
        }
        let now_ms = state
            .now_ms
            .saturating_add(now.saturating_duration_since(started).as_millis() as u64);
        let frame = graph.mixer.mixer.frame();
        if graph.mixer.epoch() != state.report.epoch || frame != state.report.next_frame {
            return state.close(graph, Some("source clock epoch/timeline"));
        }
        let topology = graph.mixer.mixer.topology();
        let width = topology.capture_channels;
        if let Some(device) = &mut state.device {
            if !device.source_map_valid(topology) {
                return state.close(graph, Some("device/provider physical map mismatch"));
            }
            match device.poll_source_capture(now) {
                Ok(true) => state.capture.copy_from_slice(device.last_capture()),
                Ok(false) => return Duration::from_micros(50),
                Err(fault) => return state.close(graph, Some(fault)),
            }
        } else {
            state.capture.fill(0.);
            for f in 0..48 {
                for (index, input) in topology.inputs.iter().enumerate() {
                    let polarity = if ((frame + f as u64) / (24 + index as u64)).is_multiple_of(2) {
                        1.
                    } else {
                        -1.
                    };
                    state.capture[f * width + input.capture_slot] =
                        polarity * ((index + 1) * 128) as f64 / 8_388_608.;
                }
            }
        }
        let Some(routing) = graph.clock_routing.as_ref() else {
            return state.close(graph, Some("source clock routing unavailable"));
        };
        let deadline = routing
            .deadline_ms
            .filter(|deadline| {
                now_ms < *deadline
                    && routing.deadline_frame.is_some_and(|end| frame < end)
                    && routing.hold_fence.as_ref().is_none_or(|f| f.valid())
                    && (!routing.foh || routing.foh_fence.as_ref().is_none_or(|f| f.valid()))
                    && graph.mixer.mixer.output_safety_gain(frame) > 0.
                    && media
                        .brain_handoff
                        .as_ref()
                        .is_none_or(|h| Some(h.hold_generation) == routing.hold_generation)
            })
            .unwrap_or(0);
        media.clock_talkback_deadline = Some(deadline);
        if let Some(handoff) = &media.brain_handoff {
            handoff.publish(
                deadline,
                !media
                    .clock_monitor_deadline_ms
                    .is_some_and(|end| now_ms < end)
                    || media.clock_monitor_silent
                    || routing.monitor_silent
                    || media.clock_monitor_generation != Some(routing.selection_generation),
            );
        }
        let brain_failed = media.drain_ingress(state.report.epoch, frame, now_ms);
        let safety =
            std::array::from_fn(|f| graph.mixer.mixer.output_safety_gain(frame + f as u64));
        if media.render_wet(frame, &safety).is_err() {
            return state.close(graph, Some("source wet timeline"));
        }
        let Some(routing) = graph.clock_routing.take() else {
            return state.close(graph, Some("source clock routing unavailable"));
        };
        let held = !brain_failed
            && deadline != 0
            && routing.hold_fence.as_ref().is_none_or(|f| f.valid())
            && (!routing.foh || routing.foh_fence.as_ref().is_none_or(|f| f.valid()))
            && routing
                .deadline_ms
                .is_some_and(|deadline| now_ms < deadline)
            && routing
                .deadline_frame
                .is_some_and(|deadline| frame < deadline);
        if !media.render_talkback(frame, held, held) {
            media.talkback_samples.fill(0.);
        }
        let intent = RenderIntent {
            monitor: routing.monitor,
            deadline: held.then_some(routing.deadline_frame).flatten(),
            monitors: &routing.monitors,
            foh: routing.foh,
            talkback_gain: routing.gain,
            remote_fx_aux: routing.remote_fx_aux,
        };
        let rendered = graph.render_capture(
            state.report.epoch,
            frame,
            &state.capture,
            &mut state.playback,
            Some(&media.wet),
            Some(&media.talkback_samples),
            Some(now_ms),
            intent,
        );
        graph.clock_routing = Some(routing);
        if rendered.is_err() {
            return state.close(graph, Some("source render refused"));
        }
        if let Some(device) = &mut state.device {
            if !device.source_map_valid(graph.mixer.mixer.topology()) {
                return state.close(graph, Some("device/provider physical map mismatch"));
            }
            if let Err(fault) = device.queue_source_playback(&state.playback) {
                return state.close(graph, Some(fault));
            }
            state.playback_pending = true;
        }
        let monitor_live = media
            .clock_monitor_deadline_ms
            .is_some_and(|deadline| now_ms < deadline)
            && media.talkback_fence.as_ref().is_some_and(|f| f.valid());
        let routing = graph
            .clock_routing
            .as_ref()
            .expect("clock routing retained");
        let generation_changed = media.clock_monitor_generation.is_some()
            && media.clock_monitor_generation != Some(routing.selection_generation);
        if generation_changed && media.clock_monitor_retirement.is_none() {
            media.clock_monitor_retirement = Some((frame, now_ms));
        }
        let retired = media.clock_monitor_retirement.is_some_and(|(first, at)| {
            frame.saturating_sub(first) >= 12_000 || now_ms.saturating_sub(at) >= 250
        });
        let monitor = if media.clock_monitor_silent || routing.monitor_silent || generation_changed
        {
            [0.; 96]
        } else {
            graph.routing.brain_monitor
        };
        if monitor_live && !retired && media.monitor.is_some() {
            if media.offer_monitor(frame, monitor) {
                media.clock_counts.monitor_packets += 1;
            } else {
                state.report.monitor_drops += 1;
                media.clock_counts.monitor_drops += 1;
            }
        }
        if let Some(observer) = &mut media.observer {
            observer.offer(
                state.report.epoch,
                graph.mixer.mixer.topology().map_revision,
                frame,
                &state.capture,
                width,
            );
        }
        if media.egress.is_some() {
            if media.fx_send.len() == 384 {
                for f in 0..48 {
                    for (bus, samples) in graph.mixer.mixer.fx_aux_samples().iter().enumerate() {
                        media.fx_send[f * 8 + bus * 2..f * 8 + bus * 2 + 2]
                            .copy_from_slice(&samples[f * 2..f * 2 + 2]);
                    }
                }
            } else {
                media.fx_send.copy_from_slice(&graph.routing.brain_send);
            }
            if !media.offer_egress(frame, &state.capture, width) {
                state.report.egress_drops += 1;
                media.clock_counts.egress_drops += 1;
            }
        }
        state.report.observe(
            frame,
            &state.playback,
            &media.wet,
            WindowIdentity {
                remote_fx_applied: graph.mixer.remote_fx_applied,
                revision: graph.mixer.applied_revision(),
                map_revision: graph.mixer.mixer.topology().map_revision,
                routing,
                held,
                monitor_audible: monitor_live
                    && !retired
                    && media.monitor.is_some()
                    && !media.clock_monitor_silent
                    && !routing.monitor_silent
                    && !generation_changed,
            },
        );
        media.clock_counts.samples += media.wet.iter().filter(|s| **s != 0.).count() as u64;
        media.clock_counts.energy += media.wet.iter().map(|sample| sample * sample).sum::<f64>();
        state.report.blocks += 1;
        state.report.next_frame = graph.mixer.mixer.frame();
        media.clock_frame = state.report.next_frame;
        state
            .status
            .blocks
            .store(state.report.blocks, Ordering::Release);
        if state.device.is_some() {
            return Duration::ZERO;
        }
        let next = next + state.period;
        state.next = Some(next);
        next.saturating_duration_since(Instant::now())
            .min(Duration::from_millis(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{remote::HostAuthority, show::Counter, topology::EngineTopology};
    use std::{os::unix::fs::PermissionsExt, path::PathBuf};
    const SHOW: &str = "11111111-1111-4111-8111-111111111111";
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn provider(name: &str, pa: usize) -> (Directory, LocalAudio) {
        let path = std::env::temp_dir().join(format!("gp-h1-clock-{name}-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let provider = LocalAudio::bind_configured(
            &path,
            "clock.sock",
            SHOW,
            Counter(9),
            EngineTopology::software(8, 2, pa).unwrap(),
        )
        .unwrap();
        (Directory(path), provider)
    }
    fn warm(host: &mut HostAuthority, now: u64) {
        let frame = host.provider().frame();
        let input = vec![0.125; host.provider().topology().capture_channels * 48];
        let mut output = vec![0.; host.provider().topology().playback_channels * 48];
        host.process_source(
            now,
            host.provider().source_epoch(),
            frame,
            &input,
            &mut output,
        )
        .unwrap();
    }
    fn await_stop(status: &SourceClockStatus) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !status.stopped() {
            assert!(Instant::now() < deadline, "source clock stop timeout");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn source_windows_count_exact_frames_energy_and_overflow_without_controller_sampling() {
        let (_directory, provider) = provider("windows", 0);
        let mut state = SourceClock::prepare(&provider, 0, Duration::from_millis(1), 201).unwrap();
        // Deliberately bound retained evidence to two windows; aggregate metrics
        // must still include every block beyond the retained prefix.
        state.report.output_windows.truncate(2);
        let width = provider.topology().playback_channels;
        let playback = vec![0.25; 48 * width];
        let wet = [0.5; 96];
        let routing = provider.clock_routing(0);
        crate::remote::assert_no_allocations(|| {
            for block in 0..201 {
                state.report.observe(
                    960 + block * 48,
                    &playback,
                    &wet,
                    WindowIdentity {
                        remote_fx_applied: None,
                        revision: 7,
                        map_revision: 1,
                        routing: &routing,
                        held: false,
                        monitor_audible: false,
                    },
                );
                state.report.blocks += 1;
            }
        });
        state.report.finish_observations();
        assert_eq!(state.report.output_windows.len(), 2);
        assert_eq!(state.report.output_windows_overflow_blocks, 1);
        assert_eq!(state.report.output_windows[0].first_frame, 960);
        assert_eq!(state.report.output_windows[0].last_frame, 5759);
        assert_eq!(state.report.output_windows[1].first_frame, 5760);
        assert_eq!(state.report.output_windows[1].last_frame, 10559);
        for window in &state.report.output_windows {
            assert_eq!(window.blocks, 100);
            assert!(window.peaks.iter().all(|peak| *peak == 0.25));
            assert!(window.energy.iter().all(|energy| *energy == 300.));
            assert_eq!(window.nonzero_wet_samples, 9600);
            assert_eq!(window.rendered_wet_energy, 2400.);
        }
        assert_eq!(state.report.nonzero_wet_samples, 201 * 96);
        assert_eq!(state.report.rendered_wet_energy, 201. * 24.);
        assert!(state.report.energy.iter().all(|energy| *energy == 603.));
    }
    #[test]
    fn clock_windows_split_on_source_identity_and_keep_overflow_as_a_prefix() {
        let (_directory, provider) = provider("window-identities", 0);
        let mut state = SourceClock::prepare(&provider, 0, Duration::from_millis(1), 8).unwrap();
        state.report.output_windows.truncate(4);
        let mut routing = provider.clock_routing(0);
        let playback = vec![0.25; 48 * provider.topology().playback_channels];
        // Revision changes, independent hold expiry, and selection changes must
        // all split windows even without controller observation between blocks.
        crate::remote::assert_no_allocations(|| {
            for block in 0..8 {
                routing.selection_generation = u64::from(block >= 4);
                state.report.observe(
                    block * 48,
                    &playback,
                    &[0.; 96],
                    WindowIdentity {
                        remote_fx_applied: None,
                        revision: u64::from(block >= 1 && block != 7),
                        map_revision: 1,
                        routing: &routing,
                        held: (1..3).contains(&block),
                        monitor_audible: false,
                    },
                );
                state.report.blocks += 1;
            }
        });
        state.report.finish_observations();
        assert_eq!(state.report.output_windows.len(), 4);
        assert_eq!(state.report.output_windows_overflow_blocks, 1);
        let windows = &state.report.output_windows;
        assert_eq!(
            windows.iter().map(|w| w.blocks).collect::<Vec<_>>(),
            [1, 2, 1, 3]
        );
        assert_eq!(windows[0].identity.revision, 0);
        assert_eq!(windows[1].identity.revision, 1);
        assert!(windows[1].identity.talkback_held);
        assert!(!windows[2].identity.talkback_held);
        assert_eq!(windows[3].identity.selection_generation, 1);
        assert_eq!(windows[3].last_frame, 335);
        assert!(state.report.energy.iter().all(|e| *e == 24.));
    }
    #[test]
    fn clocked_render_closes_talkback_on_hold_expiry_and_independent_session_disconnect() {
        use crate::control_model::{Command, Request, Scope};
        use crate::remote::{AuthorityEndpoint, BrainMediaDescriptor, BrainMediaRole, BrainPacket};
        use crate::remote::{Peer, Permission, PolicyStore};
        use sha2::{Digest, Sha256};
        for (disconnect, replace_hold) in [(false, false), (true, false), (false, true)] {
            let (_directory, mut provider) = provider(
                if replace_hold {
                    "hold-replace"
                } else if disconnect {
                    "hold-close"
                } else {
                    "hold-expiry"
                },
                0,
            );
            let policy = PolicyStore::new(vec![
                Peer {
                    id: "hold-controller".into(),
                    certificate_sha256: format!("{:x}", Sha256::digest(b"hold-controller")),
                    permissions: [Permission::TalkbackDestinations].into_iter().collect(),
                },
                Peer {
                    id: "speech-media".into(),
                    certificate_sha256: format!("{:x}", Sha256::digest(b"speech-media")),
                    permissions: [Permission::TalkbackDestinations].into_iter().collect(),
                },
            ])
            .unwrap();
            let controller = policy.authenticate(b"hold-controller", 31).unwrap();
            let media_context = policy.authenticate(b"speech-media", 32).unwrap();
            let grant = Request {
                contract: "C-AUDIO".into(),
                version: provider.engine().wire_version(),
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(controller.writer().into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: Command::Grant {
                    scope: Scope::TalkbackDestinations,
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
            provider.rearm().unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let started = Instant::now();
            let status = host
                .start_source_clock(1, Duration::from_millis(3), 160)
                .unwrap();
            for (id, command) in [
                (
                    2,
                    crate::brain_control::Command::TalkbackSet {
                        monitors: vec![0],
                        gain_cdb: -1200,
                        mute: false,
                    },
                ),
                (
                    3,
                    crate::brain_control::Command::Hold {
                        generation: Counter(1),
                    },
                ),
            ] {
                let now = 1 + started.elapsed().as_millis() as u64;
                let request = crate::brain_control::Request {
                    contract: "GP15-brain".into(),
                    version: 1,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some(controller.writer().into()),
                    lease: Some(lease),
                    request_id: Some(Counter(id)),
                    expected_revision: Some(host.provider().engine().revision()),
                    command,
                };
                host.provider_mut()
                    .clocked_control(now, Some(controller.audio_fence()), |provider| {
                        serde_json::to_value(provider.brain_request(request, now, true, None)?)
                            .map_err(|e| e.to_string())
                    })
                    .unwrap();
                assert_eq!(host.provider().engine().revision(), Counter(id - 1));
            }
            let descriptor = BrainMediaDescriptor {
                contract: "GP15-media".into(),
                version: 1,
                session: media_context.session(),
                stagebox_epoch: 9,
                brain_epoch: 77,
                stagebox_map: host.identity().map_generation.0,
                brain_map: 1,
                generation: 1,
                selection_generation: 0,
                hold_generation: 1,
                sample_rate: 48000,
                frames: 48,
                talkback: true,
                monitor: false,
            };
            host.negotiate_brain(
                &media_context,
                serde_json::json!({"contract":"GP15-media","version":1,
                "kind":"negotiate","writer":media_context.writer(),"descriptor":descriptor}),
            )
            .unwrap();
            let (mut proxy, mut mailbox) =
                crate::remote::authority_channel(&media_context, host.identity()).unwrap();
            mailbox
                .service(&mut host, 1 + started.elapsed().as_millis() as u64)
                .unwrap();
            let endpoint_ready = status.blocks() + 1;
            let until = Instant::now() + Duration::from_secs(2);
            while status.blocks() < endpoint_ready {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            // Submit through the production network endpoint with no further mailbox service.
            for block in 0..64 {
                let packet = BrainPacket {
                    descriptor: descriptor.clone(),
                    role: BrainMediaRole::Talkback,
                    frame: block * 48,
                    samples: vec![0.25; 48],
                }
                .encode()
                .unwrap();
                proxy
                    .brain_media(
                        &media_context,
                        &packet,
                        1 + started.elapsed().as_millis() as u64,
                    )
                    .unwrap();
            }
            let before = status.blocks();
            let timeout = Instant::now() + Duration::from_secs(2);
            while status.blocks() < before + 8 {
                assert!(Instant::now() < timeout);
                std::thread::sleep(Duration::from_millis(1));
            }
            let audible = host
                .provider_mut()
                .inspect_source((0., 0., 0), |graph, result| {
                    result.0 = graph
                        .routing
                        .brain_talkback
                        .iter()
                        .map(|s| s * s)
                        .sum::<f64>();
                    result.1 = graph
                        .routing
                        .bus_scratch
                        .chunks_exact(4)
                        .map(|row| row[2] * row[2])
                        .sum::<f64>();
                    result.2 = graph.mixer.mixer.frame();
                })
                .unwrap();
            assert!(
                audible.0 > 0.01 && audible.1 > 0.01,
                "speech must reach actual monitor bus: {audible:?}"
            );
            host.finish_media_ingress();
            assert_eq!(host.media_stats.accepted_talkback_packets, 64);
            assert_eq!(host.media_stats.rejected_talkback_packets, 0);
            if disconnect {
                controller.close_audio();
            }
            if replace_hold {
                // Hold is not an in-place replacement: the current hold must
                // first be released. Depending on incidental deadman expiry made
                // the former fixture alternate between success and refusal -10.
                for (id, command) in [
                    (
                        4,
                        crate::brain_control::Command::Release {
                            generation: Counter(1),
                        },
                    ),
                    (
                        5,
                        crate::brain_control::Command::Hold {
                            generation: Counter(2),
                        },
                    ),
                ] {
                    let now = 1 + started.elapsed().as_millis() as u64;
                    let request = crate::brain_control::Request {
                        contract: "GP15-brain".into(),
                        version: 1,
                        show_id: SHOW.into(),
                        module: "audio".into(),
                        epoch: Counter(9),
                        writer: Some(controller.writer().into()),
                        lease: Some(lease),
                        request_id: Some(Counter(id)),
                        expected_revision: Some(Counter(id - 2)),
                        command,
                    };
                    host.provider_mut().last_brain_source_receipt = None;
                    let reply = host
                        .provider_mut()
                        .clocked_control(now, Some(controller.audio_fence()), |provider| {
                            serde_json::to_value(provider.brain_request(request, now, true, None)?)
                                .map_err(|e| e.to_string())
                        })
                        .unwrap();
                    assert_eq!(
                        host.provider().last_brain_source_receipt.map(|r| r.code),
                        Some(0),
                        "reply={reply}, receipt={:?}",
                        host.provider().last_brain_source_receipt
                    );
                }
                assert_eq!(
                    host.provider().brain_snapshot().held_generation,
                    Some(Counter(2))
                );
            }
            // Neither controller service nor readback advances the following
            // blocks. The still-live media session cannot reopen the held route.
            let idle_blocks = if disconnect || replace_hold { 8 } else { 55 };
            let before = status.blocks();
            while status.blocks() < before + idle_blocks {
                assert!(Instant::now() < timeout);
                std::thread::sleep(Duration::from_millis(1));
            }
            let silent = host
                .provider_mut()
                .inspect_source((0., 0., 0), |graph, result| {
                    result.0 = graph
                        .routing
                        .brain_talkback
                        .iter()
                        .map(|s| s * s)
                        .sum::<f64>();
                    result.1 = graph
                        .routing
                        .bus_scratch
                        .chunks_exact(4)
                        .map(|row| row[2] * row[2])
                        .sum::<f64>();
                    result.2 = graph.mixer.mixer.frame();
                })
                .unwrap();
            assert_eq!(silent.0, 0.);
            // The monitor retains the low-level synthetic band. Speech is more
            // than two orders of magnitude above that bounded source baseline.
            assert!(
                silent.1 < 0.0001,
                "speech remained on monitor bus: {silent:?}"
            );
            assert!(silent.2 > audible.2);
            assert!(media_context.audio_fence().valid());
            let packet = BrainPacket {
                descriptor,
                role: BrainMediaRole::Talkback,
                frame: 64 * 48,
                samples: vec![0.25; 48],
            }
            .encode()
            .unwrap();
            assert!(
                proxy
                    .brain_media(
                        &media_context,
                        &packet,
                        1 + started.elapsed().as_millis() as u64
                    )
                    .is_err()
            );
            host.finish_media_ingress();
            assert_eq!(host.media_stats.rejected_talkback_packets, 1);
            status.close();
            let report = host.finish_source_clock(&status).unwrap();
            assert_eq!(report.fault, None);
            let held_window = report
                .output_windows
                .iter()
                .position(|w| w.identity.talkback_held)
                .expect("source evidence must retain the actual held interval");
            assert!(
                report.output_windows[held_window + 1..]
                    .iter()
                    .any(|w| !w.identity.talkback_held
                        && w.identity.revision == if replace_hold { 3 } else { 2 }),
                "independent closure must be correlated without inventing a revision"
            );
        }
    }
    #[test]
    fn clock_bounds_refuse_frame_wrap_before_capture_preparation() {
        let period = Duration::from_millis(1);
        assert!(clock_fits(period, 60_000, 0));
        assert!(!clock_fits(period, 60_001, 0));
        assert!(!clock_fits(period, u64::MAX, 0));
        assert!(!clock_fits(period, 1, u64::MAX / 48 * 48));
        assert!(!clock_fits(Duration::ZERO, 1, 0));
        assert!(!clock_fits(Duration::from_secs(61), 1, 0));
    }
    #[test]
    fn production_clock_advances_without_controller_and_closes_independently() {
        let (_directory, mut provider) = provider("closure", 0);
        provider.rearm().unwrap();
        let mut host = HostAuthority::new(provider, 1).unwrap();
        warm(&mut host, 0);
        let observed = host.provider().frame();
        let status = host
            .start_source_clock(1, Duration::from_millis(2), 20)
            .unwrap();
        await_stop(&status);
        assert_eq!(status.blocks(), 20);
        // No controller pump fabricated current readback while source progressed.
        assert_eq!(host.provider().frame(), observed);
        let report = host.finish_source_clock(&status).unwrap();
        assert_eq!(report.first_frame, 48);
        assert_eq!(report.next_frame, 1008);
        assert_eq!(report.fault, None);
        assert!(report.energy.iter().any(|e| *e > 0.));
        assert_eq!(report.output_windows.len(), 1);
        assert_eq!(report.output_windows[0].blocks, 20);
        assert_eq!(report.output_windows[0].energy, report.energy);
        assert_eq!(report.output_windows[0].first_frame, 48);
        assert_eq!(report.output_windows[0].last_frame, 1007);
        assert!(host.provider_mut().rearm().is_err());
        host.provider_mut().recover_source(Counter(10), 0).unwrap();
        host.provider_mut().rearm().unwrap();
        warm(&mut host, 2000);
        let replacement = host
            .start_source_clock(2001, Duration::from_millis(2), 8)
            .unwrap();
        assert!(host.finish_source_clock(&status).is_err());
        status.close(); // Old epoch closure cannot stop the replacement clock.
        await_stop(&replacement);
        let replacement = host.finish_source_clock(&replacement).unwrap();
        assert_eq!(replacement.epoch, 10);
        assert_eq!(replacement.blocks, 8);
        assert_eq!(replacement.fault, None);
    }
    #[test]
    fn source_clock_close_does_not_wait_for_next_capture_and_wrong_status_cannot_stop_it() {
        let (_directory, mut provider) = provider("stop", 0);
        provider.rearm().unwrap();
        let mut host = HostAuthority::new(provider, 1).unwrap();
        warm(&mut host, 0);
        let wrong = SourceClockStatus {
            recovery: None,
            recovery_deadline: None,
            closed: Arc::new(AtomicBool::new(false)),
            blocks: Arc::new(AtomicU64::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
        };
        let status = host
            .start_source_clock(1, Duration::from_secs(2), 2)
            .unwrap();
        assert!(host.finish_source_clock(&wrong).is_err());
        let deadline = Instant::now() + Duration::from_secs(1);
        while status.blocks() == 0 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        status.close();
        await_stop(&status);
        assert!(Instant::now() < deadline);
        let report = host.finish_source_clock(&status).unwrap();
        assert_eq!(report.blocks, 1);
        assert_eq!(report.fault, None);
    }
    #[test]
    fn future_engine_preparation_keeps_clock_running_and_refuses_expired_packet() {
        use crate::control_model::{Command, Edit, Request, Scope, Target, Value};
        let (_directory, mut provider) = provider("future", 0);
        provider.rearm().unwrap();
        let mut request = Request {
            contract: "C-AUDIO".into(),
            version: provider.engine().wire_version(),
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some("clock-writer".into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant { scope: Scope::Foh },
        };
        request.lease = provider
            .engine_mut()
            .handle(&request, 0)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease;
        assert!(request.lease.is_some());
        let mut host = HostAuthority::new(provider, 1).unwrap();
        warm(&mut host, 0);
        let status = host
            .start_source_clock(1, Duration::from_millis(2), 200)
            .unwrap();
        request.request_id = Some(Counter(2));
        request.command = Command::Set {
            targets: vec![Edit {
                target: Target::Fader {
                    input: "input-01".into(),
                },
                value: Value::Integer(-900),
            }],
        };
        let pending = host
            .provider
            .clocked_engine_command(2, None, |engine| {
                serde_json::to_value(engine.handle(&request, 2)?).map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(pending["state"], "pending");
        assert_eq!(host.provider.engine().revision(), Counter(1));
        let replies = host.provider.take_remote_completions();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["outcome"]["kind"], "applied");
        let before = status.blocks();
        request.expected_revision = Some(Counter(1));
        request.request_id = Some(Counter(3));
        host.provider
            .clocked_engine_command(3, None, |engine| {
                let reply = engine.handle(&request, 3)?;
                // Parsing/coefficient preparation can stall; source remains active.
                std::thread::sleep(Duration::from_millis(40));
                serde_json::to_value(reply).map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(status.blocks() > before + 10);
        assert_eq!(host.provider.engine().revision(), Counter(1));
        let replies = host.provider.take_remote_completions();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["outcome"]["body"]["reason"], "source_admission");
        status.close();
        let report = host.finish_source_clock(&status).unwrap();
        assert_eq!(report.fault, None);
        assert_eq!(report.output_windows[0].identity.revision, 0);
        assert!(
            report
                .output_windows
                .iter()
                .any(|w| w.identity.revision == 1)
        );
        assert!(
            report
                .output_windows
                .iter()
                .all(|w| w.identity.revision <= 1),
            "expired preparation must never appear in source evidence"
        );
    }
    #[test]
    #[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
    fn actual_clocked_pa_fx_rec_survives_controller_stall_and_finalizes() {
        let (directory, mut provider) = provider("actual", 8);
        provider
            .enable_modules(std::path::Path::new(
                &std::env::var_os("GP05_MANIFEST").unwrap(),
            ))
            .unwrap();
        provider
            .configure_pa(&std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap())
            .unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(std::env::var_os("GP05_MANIFEST").unwrap()).unwrap(),
        )
        .unwrap();
        provider
            .enable_fx_aux(
                std::path::Path::new(manifest["fx"]["library"].as_str().unwrap()),
                manifest["fx"]["library_sha256"].as_str().unwrap(),
            )
            .unwrap();
        crate::remote::runner::start_recording(&mut provider, SHOW, "clock-take", 0).unwrap();
        provider.rearm().unwrap();
        let mut host = HostAuthority::new(provider, 1).unwrap();
        // Controller prepares/activates REC before ownership of pacing changes.
        for now in 0..1000 {
            warm(&mut host, now);
            if host.provider_mut().module_status(now).unwrap()["recording"]["state"] == "recording"
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            host.provider_mut().module_status(1000).unwrap()["recording"]["state"],
            "recording"
        );
        {
            let provider = host.provider_mut();
            let grant = crate::control_model::Request {
                contract: "C-AUDIO".into(),
                version: provider.engine().wire_version(),
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some("clock-aux".into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(provider.engine().revision()),
                command: crate::control_model::Command::Grant {
                    scope: crate::control_model::Scope::FxConfiguration,
                },
            };
            let lease = provider
                .engine_mut()
                .handle(&grant, 1000)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .granted_lease
                .unwrap();
            let read = serde_json::json!({"contract":crate::fx_aux_wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_aux_snapshot","body":{}});
            let snapshot = provider.fx_aux_request(read, 1000, false, None).unwrap();
            let mut config: serde_json::Value =
                serde_json::from_str(snapshot["buses"][0]["configuration_json"].as_str().unwrap())
                    .unwrap();
            config["sends"][0] = serde_json::json!(1.);
            config["settings"]["time_ms"] = serde_json::json!(2.);
            config["settings"]["amount"] = serde_json::json!(0.);
            config["settings"]["damping"] = serde_json::json!(0.);
            config["settings"]["gain"] = serde_json::json!(1.);
            let edit = serde_json::json!({"contract":crate::fx_aux_wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":"clock-aux","lease":lease,"request_id":"2","expected_revision":snapshot["revision"],"kind":"fx_aux_configure","body":{"bus":0,"map_revision":"1","library_sha256":manifest["fx"]["library_sha256"],"expected_generation":"0","configuration_json":serde_json::to_string(&config).unwrap(),"panic":false}});
            assert_eq!(
                provider.fx_aux_request(edit, 1000, true, None).unwrap()["state"],
                "prepared"
            );
        }
        for now in 1001..1033 {
            warm(&mut host, now);
        }
        let first = host.provider().frame();
        let status = host
            .start_source_clock(1033, Duration::from_millis(2), 64)
            .unwrap();
        host.source_media
            .executor
            .with_saturated_returns(false, || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while status.blocks() < 16 {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
        host.source_media
            .executor
            .with_saturated_returns(true, || await_stop(&status));
        assert_eq!(status.blocks(), 64);
        assert_eq!(host.provider().frame(), first);
        let report = host.finish_source_clock(&status).unwrap();
        assert_eq!(report.fault, None);
        assert_eq!(report.next_frame, first + 64 * 48);
        assert!(report.energy.iter().any(|e| *e > 0.));
        let mut result = serde_json::Value::Null;
        for _ in 0..1000 {
            result = host.provider_mut().module_status(2000).unwrap();
            if result["recording"]["outcome"] == "incomplete" {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(result["recording"]["outcome"], "incomplete", "{result}");
        assert_eq!(result["recording"]["dropped_frames"], "0", "{result}");
        // Raw recording remains on the source timeline; inspect actual PCM24.
        let mut paths: Vec<_> = std::fs::read_dir(directory.0.join("takes/clock-take"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .collect();
        assert_eq!(paths.len(), 8);
        paths.sort();
        for (channel, path) in paths.into_iter().enumerate() {
            let mut wav = hound::WavReader::open(path).unwrap();
            assert_eq!(wav.spec().bits_per_sample, 24);
            let samples: Vec<_> = wav.samples::<i32>().map(Result::unwrap).collect();
            assert!(samples.len() >= 64 * 48);
            let wanted: Vec<i32> = (0..64 * 48)
                .map(|offset| {
                    let polarity =
                        if ((first + offset as u64) / (24 + channel as u64)).is_multiple_of(2) {
                            1
                        } else {
                            -1
                        };
                    polarity * (channel as i32 + 1) * 128
                })
                .collect();
            assert_eq!(&samples[samples.len() - 64 * 48..], wanted.as_slice());
        }
        host.provider_mut().recover_source(Counter(10), 0).unwrap();
        host.provider_mut().rearm().unwrap();
        warm(&mut host, 2001);
        let replacement = host
            .start_source_clock(2002, Duration::from_millis(2), 16)
            .unwrap();
        assert!(host.finish_source_clock(&status).is_err());
        status.close();
        await_stop(&replacement);
        let replacement = host.finish_source_clock(&replacement).unwrap();
        assert_eq!(replacement.epoch, 10);
        assert_eq!(replacement.blocks, 16);
        assert_eq!(replacement.fault, None);
        assert!(replacement.energy.iter().any(|e| *e > 0.));
    }
}
