//! Explicit ALSA bench host. Driver waits and status queries are outside render.
//! Negotiation/priming follows SHR PA's standalone transport (MIT), retaining its
//! raw-hardware, no-resampling and stop-on-xrun boundaries.
use super::{
    adapters::{Dsp, Recorder, Result},
    network::{self, ClientConfig},
    *,
};
use alsa::{
    Direction, ValueOr,
    pcm::{Access, Format, HwParams, IO, PCM, TstampType},
};
use rtrb::RingBuffer;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaConfig {
    pub device: String,
    pub expected_usb_id: String,
    pub card_stream_file: PathBuf,
    pub capture_with_probe: bool,
    pub probe: Probe,
    pub stall_device_after_frames: Option<u64>,
    pub seconds: u32,
    pub period: usize,
    pub buffer_periods: u32,
    pub prefill_periods: u32,
    pub audio_fifo_priority: Option<i32>,
    pub audio_cpu: Option<usize>,
    #[serde(default)]
    pub audio_memory_lock: bool,
    pub epoch: u64,
    pub return_delay_frames: u32,
    pub pa_library: PathBuf,
    pub recorder_library: PathBuf,
    pub take: PathBuf,
    pub report: PathBuf,
    pub audio_bind: String,
    pub audio_peer: String,
    pub control_bind: String,
    pub control_peer: String,
    pub faults: bool,
}
#[derive(Serialize)]
struct Negotiated {
    rate: u32,
    channels: u32,
    period: usize,
    buffer: usize,
    format: String,
    period_min: i64,
    period_max: i64,
    buffer_min: i64,
    buffer_max: i64,
}

/// Preallocated diagnostic samples; serialize only after PCM stops.
#[derive(Serialize)]
struct TimestampSample {
    frame: u64,
    monotonic_elapsed_s: f64,
    capture_status_sec: i64,
    capture_status_ns: i64,
    audio_sec: i64,
    audio_ns: i64,
    capture_avail: i64,
    playback_delay: i64,
}

const RECENT_BLOCK_EVENTS: usize = 4096;
const OUTLIER_BLOCK_EVENTS: usize = 64;

/// Host observations, not physical ADC or DAC presentation timestamps.
#[derive(Clone, Copy, Default, Serialize)]
struct StageTiming {
    begin_ns: u64,
    end_ns: u64,
    thread_cpu_ns: Option<u64>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum BlockErrorStage {
    Read,
    Render,
    InputLevel,
    Write,
    PlaybackStart,
    CaptureStatus,
    PlaybackStatus,
}

#[derive(Clone, Copy, Default, Serialize)]
struct BlockEvent {
    frame: u64,
    shutdown_flush: bool,
    begin_ns: u64,
    end_ns: u64,
    thread_cpu_ns: Option<u64>,
    read: Option<StageTiming>,
    previous_event_gap_ns: Option<u64>,
    render: Option<StageTiming>,
    render_usage: Option<super::usage::ThreadUsage>,
    usage_errors: u32,
    usage_errno: Option<i32>,
    write: Option<StageTiming>,
    capture_avail: Option<i64>,
    capture_delay: Option<i64>,
    playback_avail: Option<i64>,
    playback_delay: Option<i64>,
    error_stage: Option<BlockErrorStage>,
    cpu_clock_errors: u32,
    cpu_clock_errno: Option<i32>,
}

impl BlockEvent {
    fn usage_now(&mut self) -> Option<super::usage::ThreadUsage> {
        match super::usage::ThreadUsage::sample() {
            Ok(value) => Some(value),
            Err(error) => {
                self.usage_errors += 1;
                self.usage_errno = error.raw_os_error();
                None
            }
        }
    }

    fn sample_missing_status(&mut self, capture: &PCM, playback: &PCM) {
        if self.capture_avail.is_none()
            && let Ok(status) = capture.status()
        {
            self.capture_avail = Some(status.get_avail());
            self.capture_delay = Some(status.get_delay());
        }
        if self.playback_avail.is_none()
            && let Ok(status) = playback.status()
        {
            self.playback_avail = Some(status.get_avail());
            self.playback_delay = Some(status.get_delay());
        }
    }

    fn cpu_now(&mut self) -> Option<u64> {
        match super::scheduling::thread_cpu_ns() {
            Ok(value) => Some(value),
            Err(error) => {
                self.cpu_clock_errors += 1;
                self.cpu_clock_errno = error.raw_os_error();
                None
            }
        }
    }

    fn finish_stage(&mut self, start: Instant, begin_ns: u64, cpu: Option<u64>) -> StageTiming {
        let end_cpu = self.cpu_now();
        StageTiming {
            begin_ns,
            end_ns: elapsed_ns(start),
            thread_cpu_ns: cpu.zip(end_cpu).and_then(|(a, b)| b.checked_sub(a)),
        }
    }

    fn is_outlier(self) -> bool {
        // Service excludes the expected capture wait. A normal one-ms capture
        // period must not exhaust the separate history at startup.
        self.error_stage.is_some()
            || self.usage_errors != 0
            || self
                .render_usage
                .is_some_and(super::usage::ThreadUsage::changed)
            || self
                .previous_event_gap_ns
                .is_some_and(|gap| gap > 1_000_000)
            || self.read.is_some_and(|read| {
                read.end_ns.saturating_sub(read.begin_ns) > 2_000_000
                    || self.end_ns.saturating_sub(read.end_ns) > 1_000_000
            })
    }
}

fn elapsed_ns(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

fn physical_probe_input_over_limit(probe: Probe, input: &[i32]) -> bool {
    matches!(probe, Probe::LeftCoded | Probe::LeftContinuous)
        && input
            .chunks_exact(2)
            .any(|pair| capture_msb24(pair[0]).abs() > 0.02)
}

/// A single alarm block, including shutdown captures outside the normal take.
struct InputAlarmEvidence {
    frame: Option<u64>,
    shutdown: bool,
    raw: Vec<i32>,
}
impl InputAlarmEvidence {
    fn new(period: usize) -> Self {
        Self {
            frame: None,
            shutdown: false,
            raw: vec![0; period * 2],
        }
    }
    fn retain(&mut self, frame: u64, shutdown: bool, raw: &[i32]) {
        self.raw.copy_from_slice(raw);
        self.frame = Some(frame);
        self.shutdown = shutdown;
    }
    /// Called only after PCM stops. Hash native interleaved little-endian bytes.
    fn report(&self) -> Option<serde_json::Value> {
        self.frame.map(|frame| {
            let mut hash = Sha256::new();
            for sample in &self.raw { hash.update(sample.to_le_bytes()); }
            json!({"source_frame": frame, "shutdown_flush": self.shutdown,
                "outside_normal_take": self.shutdown, "channels": 2, "rate": 48000,
                "format": "interleaved S32LE, descriptor upper24 valid bits",
                "native_s32_samples": self.raw, "native_s32_le_sha256": crate::hash::hex(hash.finalize())})
        })
    }
}

#[derive(Serialize)]
struct BlockDiagnostics {
    recent: Vec<BlockEvent>,
    first_outliers: Vec<BlockEvent>,
    total_events: u64,
    total_outliers: u64,
    recent_overwritten: u64,
    outliers_not_retained: u64,
    cpu_clock_errors: u64,
    render_usage_total: super::usage::ThreadUsage,
    usage_errors: u64,
    #[serde(skip)]
    next: usize,
    #[serde(skip)]
    recent_valid: usize,
    #[serde(skip)]
    outliers_valid: usize,
    #[serde(skip)]
    previous_end_ns: Option<u64>,
}

impl BlockDiagnostics {
    fn new() -> Self {
        Self {
            recent: vec![BlockEvent::default(); RECENT_BLOCK_EVENTS],
            first_outliers: vec![BlockEvent::default(); OUTLIER_BLOCK_EVENTS],
            total_events: 0,
            total_outliers: 0,
            render_usage_total: super::usage::ThreadUsage::default(),
            usage_errors: 0,
            recent_overwritten: 0,
            outliers_not_retained: 0,
            cpu_clock_errors: 0,
            next: 0,
            recent_valid: 0,
            outliers_valid: 0,
            previous_end_ns: None,
        }
    }

    fn push(&mut self, mut event: BlockEvent) {
        event.previous_event_gap_ns = self
            .previous_end_ns
            .and_then(|previous| event.begin_ns.checked_sub(previous));
        self.previous_end_ns = Some(event.end_ns);
        if self.recent_valid == RECENT_BLOCK_EVENTS {
            self.recent_overwritten += 1;
        }
        self.recent[self.next] = event;
        self.recent_valid = (self.recent_valid + 1).min(RECENT_BLOCK_EVENTS);
        self.next = (self.next + 1) % RECENT_BLOCK_EVENTS;
        self.total_events += 1;
        self.cpu_clock_errors += u64::from(event.cpu_clock_errors);
        self.usage_errors += u64::from(event.usage_errors);
        if let Some(delta) = event.render_usage {
            self.render_usage_total.accumulate(delta);
        }
        if event.is_outlier() {
            self.total_outliers += 1;
            if self.outliers_valid < OUTLIER_BLOCK_EVENTS {
                self.first_outliers[self.outliers_valid] = event;
                self.outliers_valid += 1;
            } else {
                self.outliers_not_retained += 1;
            }
        }
    }

    /// Reorder only after PCM stops, so the report is chronological.
    fn finish(&mut self) {
        if self.recent_valid == RECENT_BLOCK_EVENTS {
            self.recent.rotate_left(self.next);
        }
        self.recent.truncate(self.recent_valid);
        self.first_outliers.truncate(self.outliers_valid);
        self.next = 0;
    }
}

fn open(
    device: &str,
    direction: Direction,
    block: usize,
    buffer: usize,
) -> Result<(PCM, Negotiated)> {
    let pcm = PCM::new(device, direction, true)?;
    let p = HwParams::any(&pcm)?;
    p.set_access(Access::RWInterleaved)?;
    p.set_rate_resample(false)?;
    p.set_format(Format::S32LE)?;
    p.set_channels(2)?;
    p.set_rate(48000, ValueOr::Nearest)?;
    let bounds = (
        p.get_period_size_min()?,
        p.get_period_size_max()?,
        p.get_buffer_size_min()?,
        p.get_buffer_size_max()?,
    );
    p.set_period_size(block as i64, ValueOr::Nearest)?;
    p.set_buffer_size(buffer as i64)?;
    pcm.hw_params(&p)?;
    let actual = pcm.hw_params_current()?;
    let n = Negotiated {
        rate: actual.get_rate()?,
        channels: actual.get_channels()?,
        period: actual.get_period_size()? as usize,
        buffer: actual.get_buffer_size()? as usize,
        format: format!("{:?}", actual.get_format()?),
        period_min: bounds.0,
        period_max: bounds.1,
        buffer_min: bounds.2,
        buffer_max: bounds.3,
    };
    if n.rate != 48000 || n.channels != 2 || n.period != block || n.buffer != buffer {
        return Err("negotiated configuration differs from requested bounds".into());
    }
    let sw = pcm.sw_params_current()?;
    sw.set_start_threshold(sw.get_boundary()?)?;
    sw.set_avail_min(block as i64)?;
    sw.set_tstamp_mode(true)?;
    sw.set_tstamp_type(TstampType::Monotonic)?;
    pcm.sw_params(&sw)?;
    drop(sw);
    drop(actual);
    drop(p);
    pcm.prepare()?;
    Ok((pcm, n))
}
fn transfer(pcm: &PCM, io: &IO<'_, i32>, data: &mut [i32], capture: bool) -> Result<()> {
    let begin = Instant::now();
    let frames = data.len() / 2;
    let result = super::transfer::transfer_frames(
        frames,
        || begin.elapsed() > Duration::from_secs(2),
        || match pcm.avail_update() {
            Ok(n) => Ok(n as usize),
            Err(e) if matches!(e.errno(), 4 | 11) => Ok(0),
            Err(e) => Err(e),
        },
        |offset| {
            let result = if capture {
                io.readi(&mut data[offset * 2..])
            } else {
                io.writei(&data[offset * 2..])
            };
            match result {
                Ok(n) => Ok(Some(n)),
                Err(e) if matches!(e.errno(), 4 | 11) => Ok(None),
                Err(e) => Err(e),
            }
        },
        |timeout| match pcm.wait(Some(timeout)) {
            Ok(_) => Ok(()),
            Err(e) if matches!(e.errno(), 4 | 11) => Ok(()),
            Err(e) => Err(e),
        },
    );
    match result {
        Ok(()) => Ok(()),
        Err(super::transfer::TransferError::Device(e)) => Err(e.into()),
        Err(super::transfer::TransferError::Deadline) => {
            Err("ALSA transfer exceeded bounded two-second deadline".into())
        }
        Err(super::transfer::TransferError::InvalidProgress) => {
            Err("invalid ALSA transfer length".into())
        }
    }
}
pub fn run(c: PaConfig) -> Result<serde_json::Value> {
    let buffer = device_buffer_frames(c.period, c.buffer_periods)
        .map_err(|_| "supported period and explicit two/three/four/eight-period buffer required")?;
    let prefill = device_prefill_frames(c.period, c.buffer_periods, c.prefill_periods)
        .map_err(|_| "silence prefill must be less than buffer capacity")?;
    if !c.device.starts_with("hw:CARD=")
        || !c.device.ends_with(",DEV=0")
        || !(1..=600).contains(&c.seconds)
        || c.epoch == 0
        || !return_delay_fits_period(c.period, c.return_delay_frames)
        || (c.capture_with_probe && matches!(c.probe, Probe::LeftCoded | Probe::LeftContinuous))
        || c.audio_fifo_priority.is_some_and(|priority| priority != 20)
    {
        return Err("explicit raw card, bounded duration/period, fresh epoch and wet delay >= capture period required".into());
    }
    network::validate_addresses(&c.audio_bind, &c.audio_peer)?;
    network::validate_addresses(&c.control_bind, &c.control_peer)?;
    // A caller-supplied descriptor is insufficient: match the selected card ID to
    // the kernel USB descriptor itself before opening either direction.
    let card_id = c
        .device
        .trim_start_matches("hw:CARD=")
        .trim_end_matches(",DEV=0");
    if card_id.is_empty()
        || !card_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("invalid stable card ID".into());
    }
    let card_path = std::fs::canonicalize(format!("/proc/asound/{card_id}"))?;
    if std::fs::read_to_string(card_path.join("usbid"))?.trim() != c.expected_usb_id {
        return Err("USB identity mismatch".into());
    }
    let stream_path = std::fs::canonicalize(&c.card_stream_file)?;
    if stream_path != card_path.join("stream0") {
        return Err("stream descriptor does not belong to selected device".into());
    }
    let descriptor = std::fs::read_to_string(&stream_path)?;
    if !native_stereo_24(&descriptor) {
        return Err("this host requires native 24-valid-bit S32 USB capture".into());
    }
    let (capture, cn) = open(&c.device, Direction::Capture, c.period, buffer)?;
    let (playback, pn) = open(&c.device, Direction::Playback, c.period, buffer)?;
    // Verify typed formats once, outside the transfer loop. io_i32() queries and
    // allocates hardware parameters; holding the IO object avoids that hot path.
    let capture_io = capture.io_i32()?;
    let playback_io = playback.io_i32()?;
    let mut pa = Dsp::load(&c.pa_library, "pa", c.period as u32)?;
    let mut final_pa = Dsp::load(&c.pa_library, "pa", c.period as u32)?;
    let mut recorder = Recorder::create(&c.recorder_library, &c.take, c.period as u32, c.epoch)?;
    let (mut send_producer, send_consumer) = RingBuffer::new(256);
    let (return_producer, mut return_consumer) = RingBuffer::new(256);
    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let worker = network::client(
        ClientConfig {
            audio_bind: c.audio_bind,
            audio_peer: c.audio_peer,
            control_bind: c.control_bind,
            control_peer: c.control_peer,
            epoch: c.epoch,
            return_delay_frames: c.return_delay_frames,
            faults: c.faults,
        },
        send_consumer,
        return_producer,
        Arc::clone(&stop),
        Arc::clone(&ready),
    )?;
    let worker = thread::spawn(worker);
    let n = c.period;
    let mut raw = vec![0i32; n * 2];
    let mut alarm_evidence = InputAlarmEvidence::new(n);
    let mut dac = vec![0i32; n * 2];
    let mut source = vec![0.; n * 2];
    let mut dry = vec![0.; n * 6];
    let mut wet = vec![0.; n * 2];
    let mut mixed = vec![0.; n * 2];
    let mut protected = vec![0.; n * 6];
    let mut record = vec![0.; n * 8];
    let mut prime = vec![0; prefill * 2];
    let mut wet_render =
        WetRender::with_delay(c.epoch, c.return_delay_frames).map_err(|_| "wet setup")?;
    let mut hashes: [Sha256; 8] = std::array::from_fn(|_| Sha256::new());
    let mut adc_hashes: [Sha256; 2] = std::array::from_fn(|_| Sha256::new());
    let mut low_byte_histograms = [vec![0u64; 256], vec![0u64; 256]];
    let mut dsp_time = Histogram::default();
    let mut cycle_time = Histogram::default();
    let mut read_time = Histogram::default();
    let mut write_time = Histogram::default();
    let mut interval = Histogram::default();
    let mut frames = 0u64;
    let mut captured_frames = 0u64;
    let mut record_accepted_frames = 0u64;
    let mut network_drops = 0;
    let mut recorder_drops = 0;
    let mut low_byte_nonzero = 0u64;
    let mut clipped = [0u64; 2];
    let mut peaks = [0.0f64; 8];
    let mut occupancy = 0;
    let mut capture_avail_max = 0;
    let mut playback_delay_min = i64::MAX;
    let mut playback_delay_max = 0;
    let mut timestamps = Vec::with_capacity(c.seconds as usize + 4);
    let mut block_diagnostics = BlockDiagnostics::new();
    let start = Instant::now();
    let mut previous = None;
    let mut xruns = 0;
    let mut failed_playback_cycle_us = None;
    let mut failed_playback_start_cycle_us = None;
    let mut playback_started = false;
    let mut initial_playback_queued_frames = None;
    let mut probe_input_level_alarm = false;
    eprintln!(
        "ready pid={} device={} period={} buffer={} epoch={}",
        std::process::id(),
        c.device,
        n,
        pn.buffer,
        c.epoch
    );
    let mut scheduling = None;
    let mut memory_lock = None;
    let mut memory_enter_completed = false;
    let mut memory_lock_applied = false;
    let mut locked_kib_before = None;
    let mut locked_kib_active = None;
    let mut scheduling_enter_completed = false;
    let mut scheduling_applied = false;
    let mut audio_cpu_applied = None;
    let mut outcome: Result<()> = (|| {
        let ready_deadline = Instant::now();
        while !ready.load(Ordering::Acquire) {
            if ready_deadline.elapsed() > Duration::from_secs(2) {
                return Err("Brain fresh control snapshot not ready".into());
            }
            thread::sleep(Duration::from_millis(2));
        }
        if c.audio_memory_lock {
            locked_kib_before = Some(super::memory::process_locked_kib()?);
        }
        memory_lock = super::memory::AudioMemoryLock::enter(c.audio_memory_lock)?;
        memory_enter_completed = true;
        memory_lock_applied = memory_lock.is_some();
        if c.audio_memory_lock {
            locked_kib_active = Some(super::memory::process_locked_kib()?);
        }
        scheduling = super::scheduling::AudioScheduling::enter(c.audio_fifo_priority, c.audio_cpu)?;
        scheduling_enter_completed = true;
        scheduling_applied = c.audio_fifo_priority.is_some() && scheduling.is_some();
        audio_cpu_applied = c.audio_cpu.filter(|_| scheduling.is_some());
        transfer(&playback, &playback_io, &mut prime, false)?;
        initial_playback_queued_frames = Some(prefill);
        capture.start()?;
        while frames < u64::from(c.seconds) * 48000 {
            let mut event = BlockEvent {
                frame: frames,
                begin_ns: elapsed_ns(start),
                ..BlockEvent::default()
            };
            let block_cpu = event.cpu_now();
            let mut cycle = None;
            let block_result: Result<()> = (|| {
                event.error_stage = Some(BlockErrorStage::Read);
                let begin_ns = elapsed_ns(start);
                let cpu = event.cpu_now();
                let read_result = transfer(&capture, &capture_io, &mut raw, true);
                let read = event.finish_stage(start, begin_ns, cpu);
                read_time.observe((read.end_ns - read.begin_ns) as f64 / 1000.0);
                event.read = Some(read);
                read_result?;
                captured_frames += n as u64;
                let cycle_start = Instant::now();
                cycle = Some(cycle_start);
                if let Some(p) = previous {
                    interval.observe(cycle_start.duration_since(p).as_secs_f64() * 1e6);
                }
                previous = Some(cycle_start);
                for _ in 0..64 {
                    let Ok(d) = return_consumer.pop() else { break };
                    let _ = wet_render.admit(&d.bytes[..d.len]);
                }
                occupancy = occupancy.max(wet_render.occupancy());
                for (f, pair) in source.chunks_exact_mut(2).enumerate() {
                    for ch in 0..2 {
                        let captured = if c.capture_with_probe {
                            capture_msb24(raw[f * 2 + ch]) * 0.125
                        } else {
                            0.
                        };
                        pair[ch] = ((probe_sample(c.probe, frames + f as u64, ch) + captured)
                            * 8388608.)
                            .round()
                            / 8388608.;
                    }
                }
                // Render section: bounded owner calls, ring operations and arithmetic only.
                event.error_stage = Some(BlockErrorStage::Render);
                let usage_before = event.usage_now();
                let begin_ns = elapsed_ns(start);
                let cpu = event.cpu_now();
                let render_result = (|| -> std::result::Result<(), &'static str> {
                    if !pa.process(&source, &mut dry, 6) {
                        return Err("PA fault");
                    }
                    wet_render
                        .render(frames, &mut wet)
                        .map_err(|_| "wet timeline fault")?;
                    for (index, pair) in source.chunks_exact(96).enumerate() {
                        let mut d = Datagram::default();
                        let mut wire = [0f32; 96];
                        for (x, s) in wire.iter_mut().zip(pair) {
                            *x = *s as f32;
                        }
                        let f = frames + (index * 48) as u64;
                        d.len = spec(c.epoch, crate::transport::Role::FxSend)
                            .encode_float(f, (f / 48) as u32, &wire, &mut d.bytes)
                            .map_err(|_| "send format")?;
                        if send_producer.push(d).is_err() {
                            network_drops += 1;
                        }
                    }
                    for i in 0..n * 2 {
                        mixed[i] = source[i] + wet[i] * 0.25;
                    }
                    if !final_pa.process(&mixed, &mut protected, 6) {
                        return Err("final PA protection fault");
                    }
                    for f in 0..n {
                        for ch in 0..2 {
                            let adc = raw[f * 2 + ch];
                            low_byte_nonzero += u64::from(adc & 255 != 0);
                            clipped[ch] += u64::from(capture_fullscale(adc));
                            let output = protected[f * 6 + ch].clamp(-0.25, 0.25);
                            dac[f * 2 + ch] =
                                (output * 8388608.).round().clamp(-8388608., 8388607.) as i32 * 256;
                            record[f * 8 + ch] = capture_msb24(adc);
                            record[f * 8 + 2 + ch] = source[f * 2 + ch];
                            record[f * 8 + 4 + ch] = dry[f * 6 + ch];
                            record[f * 8 + 6 + ch] = f64::from(dac[f * 2 + ch]) / 2147483648.;
                        }
                    }
                    let pushed = recorder.push(frames, &record);
                    if pushed != 0 {
                        recorder_drops += 1;
                        if pushed < 0 {
                            return Err("recorder worker fault");
                        }
                    }
                    if pushed == 0 {
                        record_accepted_frames += n as u64;
                    }
                    Ok(())
                })();
                let render = event.finish_stage(start, begin_ns, cpu);
                let usage_after = event.usage_now();
                event.render_usage = usage_before.zip(usage_after).and_then(|(before, after)| {
                    let delta = after.since(before);
                    if delta.is_none() {
                        event.usage_errors += 1;
                        event.usage_errno = Some(libc::ERANGE);
                    }
                    delta
                });
                dsp_time.observe((render.end_ns - render.begin_ns) as f64 / 1000.0);
                event.render = Some(render);
                render_result?;
                // Audit bytes are exact PCM24 submitted to the recorder; file verification is
                // performed independently after finalization, never inferred from counters.
                for row in raw.chunks_exact(2) {
                    for ch in 0..2 {
                        let bytes = row[ch].to_le_bytes();
                        adc_hashes[ch].update(&bytes[1..]);
                        low_byte_histograms[ch][usize::from(bytes[0])] += 1;
                    }
                }
                for row in record.chunks_exact(8) {
                    for (ch, x) in row.iter().enumerate() {
                        peaks[ch] = peaks[ch].max(x.abs());
                        let pcm = (*x * 8388608.).round().clamp(-8388608., 8388607.) as i32;
                        hashes[ch].update(&pcm.to_le_bytes()[..3]);
                    }
                }
                if physical_probe_input_over_limit(c.probe, &raw) {
                    alarm_evidence.retain(event.frame, false, &raw);
                    event.error_stage = Some(BlockErrorStage::InputLevel);
                    probe_input_level_alarm = true;
                    return Err("physical probe input level exceeded 0.02 FS".into());
                }
                event.error_stage = Some(BlockErrorStage::Write);
                let begin_ns = elapsed_ns(start);
                let cpu = event.cpu_now();
                let write_result = transfer(&playback, &playback_io, &mut dac, false);
                let write = event.finish_stage(start, begin_ns, cpu);
                write_time.observe((write.end_ns - write.begin_ns) as f64 / 1000.0);
                event.write = Some(write);
                if let Err(error) = write_result {
                    failed_playback_cycle_us = Some(cycle_start.elapsed().as_secs_f64() * 1e6);
                    return Err(error);
                }
                frames += n as u64;
                if !playback_started {
                    initial_playback_queued_frames = Some(prefill + n);
                    // Start with the first processed block already queued. Starting
                    // playback before capture would exhaust a one-period prefill
                    // while waiting for the very first input block and rendering it.
                    event.error_stage = Some(BlockErrorStage::PlaybackStart);
                    if let Err(error) = playback.start() {
                        failed_playback_start_cycle_us =
                            Some(cycle_start.elapsed().as_secs_f64() * 1e6);
                        return Err(error.into());
                    }
                    playback_started = true;
                }
                event.error_stage = Some(BlockErrorStage::CaptureStatus);
                let cs = capture.status()?;
                event.capture_avail = Some(cs.get_avail());
                event.capture_delay = Some(cs.get_delay());
                event.error_stage = Some(BlockErrorStage::PlaybackStatus);
                let ps = playback.status()?;
                event.playback_avail = Some(ps.get_avail());
                event.playback_delay = Some(ps.get_delay());
                capture_avail_max = capture_avail_max.max(cs.get_avail_max());
                playback_delay_min = playback_delay_min.min(ps.get_delay());
                playback_delay_max = playback_delay_max.max(ps.get_delay());
                if frames % 48000 < (n as u64) {
                    let stamp = cs.get_htstamp();
                    let audio = cs.get_audio_htstamp();
                    timestamps.push(TimestampSample {
                        frame: frames,
                        monotonic_elapsed_s: start.elapsed().as_secs_f64(),
                        capture_status_sec: stamp.tv_sec,
                        capture_status_ns: stamp.tv_nsec,
                        audio_sec: audio.tv_sec,
                        audio_ns: audio.tv_nsec,
                        capture_avail: cs.get_avail(),
                        playback_delay: ps.get_delay(),
                    });
                }
                event.error_stage = None;
                Ok(())
            })();
            // Failed transfers retain timing and any status still available.
            // These queries are outside render and never replace the first fault.
            if block_result.is_err() {
                event.sample_missing_status(&capture, &playback);
            }
            let end_cpu = event.cpu_now();
            event.thread_cpu_ns = block_cpu.zip(end_cpu).and_then(|(a, b)| b.checked_sub(a));
            event.end_ns = elapsed_ns(start);
            if let Some(cycle) = cycle {
                cycle_time.observe(cycle.elapsed().as_secs_f64() * 1e6);
            }
            block_diagnostics.push(event);
            block_result?;
            if c.stall_device_after_frames == Some(frames) {
                thread::sleep(Duration::from_millis(100));
            }
        }
        // Bounded quiet shutdown: continue clocked transfers until queued audio clears.
        dac.fill(0);
        for index in 0..c.buffer_periods + 1 {
            let mut event = BlockEvent {
                frame: frames + u64::from(index) * n as u64,
                shutdown_flush: true,
                begin_ns: elapsed_ns(start),
                ..BlockEvent::default()
            };
            let block_cpu = event.cpu_now();
            let flush_result: Result<()> = (|| {
                event.error_stage = Some(BlockErrorStage::Read);
                let begin_ns = elapsed_ns(start);
                let cpu = event.cpu_now();
                let result = transfer(&capture, &capture_io, &mut raw, true);
                let read = event.finish_stage(start, begin_ns, cpu);
                read_time.observe((read.end_ns - read.begin_ns) as f64 / 1000.0);
                event.read = Some(read);
                result?;
                if physical_probe_input_over_limit(c.probe, &raw) {
                    alarm_evidence.retain(event.frame, true, &raw);
                    event.error_stage = Some(BlockErrorStage::InputLevel);
                    probe_input_level_alarm = true;
                    return Err("physical probe input level exceeded 0.02 FS".into());
                }
                event.error_stage = Some(BlockErrorStage::Write);
                let begin_ns = elapsed_ns(start);
                let cpu = event.cpu_now();
                let result = transfer(&playback, &playback_io, &mut dac, false);
                let write = event.finish_stage(start, begin_ns, cpu);
                write_time.observe((write.end_ns - write.begin_ns) as f64 / 1000.0);
                event.write = Some(write);
                result?;
                event.error_stage = None;
                Ok(())
            })();
            event.sample_missing_status(&capture, &playback);
            let end_cpu = event.cpu_now();
            event.thread_cpu_ns = block_cpu.zip(end_cpu).and_then(|(a, b)| b.checked_sub(a));
            event.end_ns = elapsed_ns(start);
            block_diagnostics.push(event);
            flush_result?;
        }
        Ok(())
    })();
    let _ = capture.drop();
    let _ = playback.drop();
    let scheduling_restore_error = scheduling.as_mut().and_then(|s| s.restore().err());
    let scheduling_restore_retry_error = if scheduling_restore_error.is_some() {
        scheduling.as_mut().and_then(|s| s.restore().err())
    } else {
        None
    };
    // Failed entry can include unsuccessful rollback; no guard means no proof
    // of restoration in that case. Preserve an unknown state instead of true.
    let scheduling_restored = scheduling_enter_completed
        .then_some(scheduling_restore_error.is_none() || scheduling_restore_retry_error.is_none());
    // Retry any pending restoration before joins, disk finalization or reporting.
    drop(scheduling.take());
    let memory_restore_error = memory_lock.as_mut().and_then(|guard| guard.restore().err());
    let memory_restore_retry_error = if memory_restore_error.is_some() {
        memory_lock.as_mut().and_then(|guard| guard.restore().err())
    } else {
        None
    };
    let locked_after = c.audio_memory_lock.then(super::memory::process_locked_kib);
    let locked_kib_after = locked_after
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .copied();
    let memory_restored = memory_enter_completed.then_some(
        (memory_restore_error.is_none() || memory_restore_retry_error.is_none())
            && (!c.audio_memory_lock || locked_kib_after == Some(0)),
    );
    drop(memory_lock.take());
    if outcome.is_ok() && (memory_restore_error.is_some() || memory_restored == Some(false)) {
        outcome = Err("audio process memory-lock restoration failed or unconfirmed".into());
    }
    if outcome.is_ok()
        && let Some(ref error) = scheduling_restore_error
    {
        outcome = Err(format!("audio scheduling restoration failed: {error}").into());
    }
    if let Err(ref e) = outcome {
        if e.downcast_ref::<alsa::Error>()
            .is_some_and(|e| matches!(e.errno(), 32 | 86))
        {
            xruns += 1;
        }
        recorder.fault(2);
    }
    stop.store(true, Ordering::Release);
    let net = worker.join().map_err(|_| "network worker panic")?;
    let finalization = recorder.finish();
    block_diagnostics.finish();
    let mut report = json!({"capture_with_probe":c.capture_with_probe,"epoch":c.epoch,"device":c.device,"capture":cn,"playback":pn,"source_valid_bits":24,"usb_descriptor_bits":24,"alsa_container_bits":32,"adc_native_msb24_sha256":adc_hashes.map(|h|crate::hash::hex(h.finalize())),"low_byte_histograms":low_byte_histograms,"return_delay_frames":c.return_delay_frames,"frames":frames,"captured_frames":captured_frames,"record_accepted_frames":record_accepted_frames,"playback_complete_block_frames":frames,"elapsed_s":start.elapsed().as_secs_f64(),"fault":outcome.err().map(|e|e.to_string()),"xruns":xruns,"record_finalization":finalization,"record_queue_drops":recorder_drops,"network_queue_drops":network_drops,"low_byte_nonzero":low_byte_nonzero,"adc_fullscale_samples":clipped,"peaks":peaks,"record_pcm_sha256":hashes.map(|h|crate::hash::hex(h.finalize())),"record_channels":["adc_left","adc_right","pa_source_left","pa_source_right","pa_dry_left","pa_dry_right","dac_submitted_left","dac_submitted_right"],"wet_present_packets":wet_render.present,"wet_missing_packets":wet_render.missing,"wet_expired":wet_render.expired,"wet_rejected":wet_render.rejected,"wet_peak_occupancy":occupancy,"render":dsp_time.summary(),"cycle":cycle_time.summary(),"read_wait":read_time.summary(),"write_wait":write_time.summary(),"capture_interval":interval.summary(),"capture_avail_max":capture_avail_max,"playback_delay_min":playback_delay_min,"playback_delay_max":playback_delay_max,"timestamps":timestamps,"network":net});
    report["failed_playback_cycle_us"] = json!(failed_playback_cycle_us);
    report["prefill_frames"] = json!(prefill);
    report["configured_initial_playback_frames"] = json!(prefill + n);
    report["initial_playback_queued_frames"] = json!(initial_playback_queued_frames);
    report["playback_delay_min"] =
        json!((playback_delay_min != i64::MAX).then_some(playback_delay_min));
    report["playback_started"] = json!(playback_started);
    report["failed_playback_start_cycle_us"] = json!(failed_playback_start_cycle_us);
    report["probe"] = json!(c.probe);
    report["audio_fifo_priority_requested"] = json!(c.audio_fifo_priority);
    report["audio_fifo_applied"] = json!(scheduling_applied);
    report["audio_cpu_requested"] = json!(c.audio_cpu);
    report["audio_cpu_applied"] = json!(audio_cpu_applied);
    report["audio_memory_lock"] = json!({
        "requested": c.audio_memory_lock,
        "enter_completed": memory_enter_completed,
        "applied": memory_lock_applied,
        "locked_kib_before": locked_kib_before,
        "locked_kib_active": locked_kib_active,
        "locked_kib_after": locked_kib_after,
        "restored_before_guard_drop": memory_restored,
        "restore_error": memory_restore_error.map(|error| error.to_string()),
        "restore_retry_error": memory_restore_retry_error.map(|error| error.to_string()),
        "final_status_error": locked_after.and_then(|result| result.err()).map(|error| error.to_string()),
        "scope": "only this fresh CLI process, with exclusive memory-lock API ownership; no resource-limit changes",
    });
    report["probe_input_level_alarm"] = json!(probe_input_level_alarm);
    report["input_alarm_evidence"] = json!(alarm_evidence.report());
    report["dac_recording_contract"] = json!({
        "channels": "dac_submitted_left/right retain prepared, intended PCM before the ALSA write",
        "unsubmitted_final_block_possible": true,
        "complete_alsa_write_frames": frames,
        "captured_frames_without_complete_write": captured_frames.saturating_sub(frames),
        "partial_write_delivery_unknown": true,
        "physical_output_proven_by_hashes": false
    });
    report["audio_settings_restored_before_guard_drop"] = json!(scheduling_restored);
    report["audio_scheduling_enter_completed"] = json!(scheduling_enter_completed);
    report["audio_restore_retry_error"] =
        json!(scheduling_restore_retry_error.map(|e| e.to_string()));
    report["block_diagnostics"] = json!(block_diagnostics);
    report["block_diagnostics_contract"] = json!({
        "clock": "elapsed monotonic host wall ns; thread CPU deltas are per calling audio thread",
        "physical_presentation_timestamps": false,
        "pcm_status": "end-of-block host status; best-effort after a failed stage",
        "recent_capacity": RECENT_BLOCK_EVENTS,
        "first_outlier_capacity": OUTLIER_BLOCK_EVENTS,
        "outlier_service_after_read_ns": 1_000_000,
        "outlier_read_ns": 2_000_000,
        "outlier_between_events_ns": 1_000_000,
        "faults_are_outliers": true,
        "render_usage": "RUSAGE_THREAD deltas bracket render and its wall/CPU timing calls; getrusage calls remain outside DSP",
        "render_usage_changes_are_outliers": true,
        "render_usage_totals_cover_all_blocks_even_after_history_overflow": true,
        "render_usage_interpretation": "major faults require I/O; voluntary switches indicate blocking/yield; involuntary switches indicate preemption, not its cause; minor fault count does not measure duration",
        "shutdown_flush_included": true,
        "read_write_histograms_include_failed_transfers_and_shutdown_flush": true
    });
    report["audio_scheduler_restore_error"] =
        json!(scheduling_restore_error.map(|e| e.to_string()));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_fault_and_switch_totals_survive_bounded_history() {
        let mut events = BlockDiagnostics::new();
        let change = super::super::usage::ThreadUsage {
            minor_faults: 1,
            major_faults: 2,
            voluntary_switches: 3,
            involuntary_switches: 4,
        };
        for frame in 0..RECENT_BLOCK_EVENTS + 1 {
            events.push(BlockEvent {
                frame: frame as u64 * 48,
                render_usage: Some(change),
                ..BlockEvent::default()
            });
        }
        events.finish();
        let count = RECENT_BLOCK_EVENTS as u64 + 1;
        assert_eq!(events.total_outliers, count);
        assert_eq!(events.first_outliers.len(), OUTLIER_BLOCK_EVENTS);
        assert_eq!(events.render_usage_total.minor_faults, count);
        assert_eq!(events.render_usage_total.major_faults, count * 2);
        assert_eq!(events.render_usage_total.voluntary_switches, count * 3);
        assert_eq!(events.render_usage_total.involuntary_switches, count * 4);
        assert_eq!(events.recent[0].frame, 48);
        assert!(
            BlockEvent {
                usage_errors: 1,
                ..BlockEvent::default()
            }
            .is_outlier()
        );
    }

    #[test]
    fn alarm_evidence_preserves_shutdown_samples_without_extending_the_take() {
        let mut evidence = InputAlarmEvidence::new(2);
        let allocation = evidence.raw.as_ptr();
        assert!(evidence.report().is_none());
        let raw = [i32::MIN, 3, 167_773 * 256, -3];
        evidence.retain(48_000, true, &raw);
        assert_eq!(evidence.raw.as_ptr(), allocation);
        let report = evidence.report().unwrap();
        assert_eq!(report["source_frame"], 48_000);
        assert_eq!(report["outside_normal_take"], true);
        assert_eq!(report["native_s32_samples"], json!(raw));
        assert_eq!(report["native_s32_le_sha256"].as_str().unwrap().len(), 64);
        evidence.retain(47_998, false, &raw);
        assert_eq!(evidence.report().unwrap()["outside_normal_take"], false);
    }

    #[test]
    fn physical_probe_level_alarm_is_left_only_and_keeps_tone_mode_unchanged() {
        for probe in [Probe::LeftCoded, Probe::LeftContinuous] {
            assert!(!physical_probe_input_over_limit(
                probe,
                &[167_772 * 256, i32::MAX]
            ));
            assert!(physical_probe_input_over_limit(probe, &[167_773 * 256, 0]));
            assert!(physical_probe_input_over_limit(probe, &[-167_773 * 256, 0]));
        }
        assert!(!physical_probe_input_over_limit(
            Probe::StereoTones,
            &[i32::MAX, i32::MAX]
        ));
    }

    #[test]
    fn block_event_ring_retains_chronological_tail_without_growing() {
        let mut events = BlockDiagnostics::new();
        let allocation = events.recent.as_ptr();
        for frame in 0..RECENT_BLOCK_EVENTS + 7 {
            events.push(BlockEvent {
                frame: frame as u64,
                ..BlockEvent::default()
            });
        }
        assert_eq!(events.recent.as_ptr(), allocation);
        assert_eq!(events.recent.len(), RECENT_BLOCK_EVENTS);
        assert_eq!(events.total_events, (RECENT_BLOCK_EVENTS + 7) as u64);
        assert_eq!(events.recent_overwritten, 7);
        events.finish();
        assert_eq!(events.recent.first().unwrap().frame, 7);
        assert_eq!(
            events.recent.last().unwrap().frame,
            (RECENT_BLOCK_EVENTS + 6) as u64
        );
        assert!(
            events
                .recent
                .windows(2)
                .all(|pair| pair[0].frame + 1 == pair[1].frame)
        );
        assert!(events.first_outliers.is_empty());
    }

    #[test]
    fn block_outliers_preserve_first_events_and_count_overflow() {
        let mut events = BlockDiagnostics::new();
        let allocation = events.first_outliers.as_ptr();
        for frame in 0..OUTLIER_BLOCK_EVENTS + 3 {
            events.push(BlockEvent {
                frame: frame as u64,
                error_stage: Some(BlockErrorStage::Read),
                cpu_clock_errors: 1,
                ..BlockEvent::default()
            });
        }
        assert_eq!(events.first_outliers.as_ptr(), allocation);
        events.finish();
        assert_eq!(events.first_outliers.len(), OUTLIER_BLOCK_EVENTS);
        assert_eq!(events.first_outliers[0].frame, 0);
        assert_eq!(
            events.first_outliers[OUTLIER_BLOCK_EVENTS - 1].frame,
            (OUTLIER_BLOCK_EVENTS - 1) as u64
        );
        assert_eq!(events.total_outliers, (OUTLIER_BLOCK_EVENTS + 3) as u64);
        assert_eq!(events.outliers_not_retained, 3);
        assert_eq!(events.cpu_clock_errors, (OUTLIER_BLOCK_EVENTS + 3) as u64);
        assert_eq!(events.recent.len(), OUTLIER_BLOCK_EVENTS + 3);
    }

    #[test]
    fn outlier_thresholds_distinguish_capture_wait_from_service_time() {
        let mut event = BlockEvent {
            end_ns: 1_100_000,
            read: Some(StageTiming {
                begin_ns: 0,
                end_ns: 1_000_000,
                thread_cpu_ns: Some(1000),
            }),
            ..BlockEvent::default()
        };
        assert!(!event.is_outlier());
        event.end_ns = 2_000_001;
        assert!(event.is_outlier());
        event.read.as_mut().unwrap().end_ns = 2_000_001;
        assert!(event.is_outlier());
        event.read = None;
        event.error_stage = Some(BlockErrorStage::Read);
        assert!(event.is_outlier());
    }

    #[test]
    fn pauses_between_blocks_survive_the_recent_history() {
        let mut events = BlockDiagnostics::new();
        events.push(BlockEvent {
            end_ns: 1000,
            ..BlockEvent::default()
        });
        events.push(BlockEvent {
            frame: 48,
            begin_ns: 3_001_000,
            end_ns: 3_002_000,
            ..BlockEvent::default()
        });
        for frame in 2..RECENT_BLOCK_EVENTS + 3 {
            events.push(BlockEvent {
                frame: frame as u64 * 48,
                begin_ns: 3_002_000,
                end_ns: 3_002_000,
                ..BlockEvent::default()
            });
        }
        events.finish();
        assert_eq!(events.first_outliers.len(), 1);
        assert_eq!(events.first_outliers[0].frame, 48);
        assert_eq!(
            events.first_outliers[0].previous_event_gap_ns,
            Some(3_000_000)
        );
        assert!(events.recent.iter().all(|event| event.frame > 48));
    }
}
