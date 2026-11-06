//! Preallocated asynchronous clock crossing. Ratio means output/input frames.
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use serde::{Deserialize, Serialize};

const QUANTUM: usize = 48;
const FADE: usize = 240;
const MAX_CHANNELS: usize = 8;
const MAX_CAPACITY: usize = 48_000;
const MAX_PPM: f64 = 1500.;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeEpochs {
    pub source: u64,
    pub destination: u64,
    pub route: u64,
}
impl BridgeEpochs {
    fn valid(self) -> bool {
        self.source != 0 && self.destination != 0 && self.route != 0
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeConfig {
    pub channels: usize,
    pub sample_rate: u32,
    pub capacity_frames: usize,
    pub target_frames: usize,
    pub max_block_frames: usize,
}
impl BridgeConfig {
    pub fn voice(channels: usize) -> Self {
        Self {
            channels,
            sample_rate: 48_000,
            capacity_frames: 4096,
            target_frames: 960,
            max_block_frames: 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeError {
    Configuration,
    Identity,
    Timeline,
    Shape,
    Nonfinite,
    Overflow,
    Starved,
    Skew,
    NotReady,
    Resampler,
}
impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "brain bridge {self:?}")
    }
}
impl std::error::Error for BridgeError {}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BridgeStatus {
    pub epochs: BridgeEpochs,
    pub armed: bool,
    pub ready: bool,
    pub fault: Option<BridgeError>,
    pub occupancy_frames: usize,
    pub target_frames: usize,
    pub source_frame: Option<u64>,
    pub destination_frame: Option<u64>,
    pub ratio_output_per_input: f64,
    pub estimated_source_skew_ppm: f64,
    pub filter_delay_output_frames: usize,
    pub queue_latency_nominal_ms: f64,
    pub filter_latency_nominal_ms: f64,
    /// No physical timestamp calibration has been performed by this bridge.
    pub physical_mapping_uncertainty_frames: Option<f64>,
    pub physical_clock_lock_verified: bool,
    pub underruns: u64,
    pub overflows: u64,
    pub rejected: u64,
}

pub struct Bridge {
    config: BridgeConfig,
    epochs: BridgeEpochs,
    resampler: Async<f64>,
    ring: Vec<f64>,
    input: Vec<f64>,
    output: Vec<f64>,
    read: usize,
    queued: usize,
    next_source: Option<u64>,
    consumed_source: Option<u64>,
    next_destination: Option<u64>,
    output_cursor: usize,
    armed: bool,
    running: bool,
    fault: Option<BridgeError>,
    last: [f64; MAX_CHANNELS],
    fade: usize,
    attack: usize,
    filtered_error: f64,
    integral_ppm: f64,
    correction_ppm: f64,
    saturation_frames: u64,
    underruns: u64,
    overflows: u64,
    rejected: u64,
}
impl Bridge {
    pub fn prepare(config: BridgeConfig, epochs: BridgeEpochs) -> Result<Self, BridgeError> {
        if !epochs.valid()
            || !(1..=MAX_CHANNELS).contains(&config.channels)
            || config.sample_rate != 48_000
            || config.target_frames < 480
            || config.capacity_frames > MAX_CAPACITY
            || config.capacity_frames < config.target_frames.saturating_add(512)
            || config.max_block_frames == 0
            || config.max_block_frames > 4096
        {
            return Err(BridgeError::Configuration);
        }
        let parameters = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: Some(0.875),
            oversampling_factor: 128,
            interpolation: SincInterpolationType::Cubic,
            window: WindowFunction::BlackmanHarris2,
        };
        let resampler = Async::new_sinc(
            1.,
            1.002,
            &parameters,
            QUANTUM,
            config.channels,
            FixedAsync::Output,
        )
        .map_err(|_| BridgeError::Resampler)?;
        let input = vec![0.; resampler.input_frames_max() * config.channels];
        Ok(Self {
            config,
            epochs,
            resampler,
            ring: vec![0.; config.capacity_frames * config.channels],
            input,
            output: vec![0.; QUANTUM * config.channels],
            read: 0,
            queued: 0,
            next_source: None,
            consumed_source: None,
            next_destination: None,
            output_cursor: QUANTUM,
            armed: false,
            running: false,
            fault: None,
            last: [0.; MAX_CHANNELS],
            fade: 0,
            attack: 0,
            filtered_error: 0.,
            integral_ppm: 0.,
            correction_ppm: 0.,
            saturation_frames: 0,
            underruns: 0,
            overflows: 0,
            rejected: 0,
        })
    }
    pub fn config(&self) -> BridgeConfig {
        self.config
    }
    /// Requested sample allocations only; excludes allocator overhead and any
    /// library/device state not represented by these owned sample vectors.
    pub fn prepared_sample_bytes(&self) -> usize {
        [
            self.ring.capacity(),
            self.input.capacity(),
            self.output.capacity(),
        ]
        .into_iter()
        .map(|n| n * std::mem::size_of::<f64>())
        .sum()
    }
    /// Startup only: touch prepared storage before the first service quantum.
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(crate) fn prefault_prepared(&mut self) {
        super::prefault_samples(&mut self.ring);
        super::prefault_samples(&mut self.input);
        super::prefault_samples(&mut self.output);
        std::hint::black_box(&mut self.resampler).reset();
    }
    pub fn status(&self) -> BridgeStatus {
        BridgeStatus {
            epochs: self.epochs,
            armed: self.armed,
            ready: self.running || self.prefilled(),
            fault: self.fault,
            occupancy_frames: self.queued,
            target_frames: self.config.target_frames,
            source_frame: self.consumed_source,
            destination_frame: self.next_destination,
            ratio_output_per_input: 1. / (1. + self.correction_ppm * 1e-6),
            estimated_source_skew_ppm: self.correction_ppm,
            filter_delay_output_frames: self.resampler.output_delay(),
            queue_latency_nominal_ms: self.queued as f64 * 1000. / self.config.sample_rate as f64,
            filter_latency_nominal_ms: self.resampler.output_delay() as f64 * 1000.
                / self.config.sample_rate as f64,
            physical_mapping_uncertainty_frames: None,
            physical_clock_lock_verified: false,
            underruns: self.underruns,
            overflows: self.overflows,
            rejected: self.rejected,
        }
    }
    fn prefilled(&self) -> bool {
        self.fault.is_none()
            && self.queued >= self.config.target_frames + self.resampler.input_frames_next()
    }
    pub fn arm(&mut self, epochs: BridgeEpochs) -> Result<(), BridgeError> {
        if epochs != self.epochs {
            return Err(BridgeError::Identity);
        }
        if !self.prefilled() {
            return Err(BridgeError::NotReady);
        }
        self.armed = true;
        self.attack = 0;
        Ok(())
    }
    /// Clears stale content without releasing storage. A changed identity and fresh
    /// explicit arm are mandatory; callers retire the whole instance off render.
    pub fn reset(&mut self, epochs: BridgeEpochs) -> Result<(), BridgeError> {
        if !epochs.valid() || epochs == self.epochs {
            return Err(BridgeError::Identity);
        }
        self.epochs = epochs;
        self.resampler.reset();
        self.read = 0;
        self.queued = 0;
        self.next_source = None;
        self.consumed_source = None;
        self.next_destination = None;
        self.output_cursor = QUANTUM;
        self.armed = false;
        self.running = false;
        self.fault = None;
        self.last.fill(0.);
        self.fade = 0;
        self.attack = 0;
        self.filtered_error = 0.;
        self.integral_ppm = 0.;
        self.correction_ppm = 0.;
        self.saturation_frames = 0;
        Ok(())
    }
    pub fn invalidate(&mut self) {
        self.fail(BridgeError::Identity);
    }
    pub fn disarm(&mut self) {
        self.fail(BridgeError::NotReady);
    }
    fn fail(&mut self, error: BridgeError) {
        if self.fault.is_none() {
            self.fault = Some(error);
            self.fade = FADE;
        }
        self.armed = false;
        self.running = false;
        self.queued = 0;
        self.output_cursor = QUANTUM;
    }
    pub fn push(
        &mut self,
        epochs: BridgeEpochs,
        first: u64,
        samples: &[f64],
    ) -> Result<(), BridgeError> {
        if epochs != self.epochs {
            self.rejected += 1;
            return Err(BridgeError::Identity);
        }
        if let Some(e) = self.fault {
            return Err(e);
        }
        if samples.is_empty()
            || !samples.len().is_multiple_of(self.config.channels)
            || samples.len() / self.config.channels > self.config.max_block_frames
        {
            self.rejected += 1;
            return Err(BridgeError::Shape);
        }
        let frames = samples.len() / self.config.channels;
        let Some(end) = first.checked_add(frames as u64) else {
            self.fail(BridgeError::Timeline);
            return Err(BridgeError::Timeline);
        };
        if let Some(next) = self.next_source {
            if first < next {
                self.rejected += 1;
                return Err(BridgeError::Timeline);
            }
            if first != next {
                self.fail(BridgeError::Timeline);
                return Err(BridgeError::Timeline);
            }
        }
        if samples.iter().any(|x| !x.is_finite()) {
            self.fail(BridgeError::Nonfinite);
            return Err(BridgeError::Nonfinite);
        }
        if frames > self.config.capacity_frames - self.queued {
            self.overflows += 1;
            self.fail(BridgeError::Overflow);
            return Err(BridgeError::Overflow);
        }
        for (n, frame) in samples.chunks_exact(self.config.channels).enumerate() {
            let index =
                (self.read + self.queued + n) % self.config.capacity_frames * self.config.channels;
            self.ring[index..index + self.config.channels].copy_from_slice(frame);
        }
        self.queued += frames;
        self.next_source = Some(end);
        if self.consumed_source.is_none() {
            self.consumed_source = Some(first);
        }
        Ok(())
    }
    fn quantum(&mut self) -> Result<(), BridgeError> {
        let required = self.resampler.input_frames_next();
        if self.queued < required {
            self.underruns += 1;
            return Err(BridgeError::Starved);
        }
        for n in 0..required {
            let index = (self.read + n) % self.config.capacity_frames * self.config.channels;
            self.input[n * self.config.channels..(n + 1) * self.config.channels]
                .copy_from_slice(&self.ring[index..index + self.config.channels]);
        }
        let input = InterleavedSlice::new(&self.input, self.config.channels, required)
            .map_err(|_| BridgeError::Resampler)?;
        let mut output = InterleavedSlice::new_mut(&mut self.output, self.config.channels, QUANTUM)
            .map_err(|_| BridgeError::Resampler)?;
        let (used, made) = self
            .resampler
            .process_into_buffer(&input, &mut output, None)
            .map_err(|_| BridgeError::Resampler)?;
        if used != required || made != QUANTUM {
            return Err(BridgeError::Resampler);
        }
        if self.output.iter().any(|sample| !sample.is_finite()) {
            return Err(BridgeError::Nonfinite);
        }
        self.read = (self.read + used) % self.config.capacity_frames;
        self.queued -= used;
        self.consumed_source = self
            .consumed_source
            .and_then(|v| v.checked_add(used as u64));
        self.output_cursor = 0;
        // Servo observes occupancy only once per fixed output quantum. Network
        // delivery scheduling is filtered independently of interpolation phase.
        let dt = QUANTUM as f64 / self.config.sample_rate as f64;
        let error = self.queued as f64 - self.config.target_frames as f64;
        self.filtered_error += (error - self.filtered_error) * (dt / (2. + dt));
        self.integral_ppm =
            (self.integral_ppm + self.filtered_error * 0.2 * dt).clamp(-MAX_PPM, MAX_PPM);
        let desired = 6. * self.filtered_error + self.integral_ppm;
        if desired.abs() >= MAX_PPM {
            self.saturation_frames += QUANTUM as u64;
        } else {
            self.saturation_frames = 0;
        }
        if self.saturation_frames > 30 * self.config.sample_rate as u64 {
            return Err(BridgeError::Skew);
        }
        self.correction_ppm +=
            (desired.clamp(-MAX_PPM, MAX_PPM) - self.correction_ppm).clamp(-100. * dt, 100. * dt);
        self.resampler
            .set_resample_ratio(1. / (1. + self.correction_ppm * 1e-6), true)
            .map_err(|_| BridgeError::Skew)?;
        Ok(())
    }
    /// Bounded output buffers are initialized even on faults. An oversized buffer
    /// is rejected untouched, so invalid caller lengths cannot cause unbounded
    /// clearing work; callers must discard it. Frame progression is independent
    /// of packet arrival and push partitioning.
    pub fn render(
        &mut self,
        epochs: BridgeEpochs,
        first: u64,
        out: &mut [f64],
    ) -> Result<(), BridgeError> {
        if out.len() > self.config.max_block_frames * self.config.channels {
            return Err(BridgeError::Shape);
        }
        out.fill(0.);
        if epochs != self.epochs {
            return Err(BridgeError::Identity);
        }
        if out.is_empty()
            || !out.len().is_multiple_of(self.config.channels)
            || out.len() / self.config.channels > self.config.max_block_frames
        {
            return Err(BridgeError::Shape);
        }
        let frames = out.len() / self.config.channels;
        let Some(end) = first.checked_add(frames as u64) else {
            self.fail(BridgeError::Timeline);
            return Err(BridgeError::Timeline);
        };
        if self.next_destination.is_some_and(|v| v != first) {
            self.fail(BridgeError::Timeline);
            return Err(BridgeError::Timeline);
        }
        self.next_destination = Some(end);
        if self.armed && !self.running && self.prefilled() {
            self.running = true;
        }
        for frame in out.chunks_exact_mut(self.config.channels) {
            if self.running
                && self.output_cursor == QUANTUM
                && let Err(e) = self.quantum()
            {
                self.fail(e);
            }
            if self.running {
                self.attack = (self.attack + 1).min(FADE);
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = self.output[self.output_cursor * self.config.channels + channel]
                        * (self.attack as f64 / FADE as f64);
                    self.last[channel] = *sample;
                }
                self.output_cursor += 1;
            } else if self.fade > 0 {
                for (channel, sample) in frame.iter_mut().enumerate() {
                    *sample = self.last[channel] * ((self.fade - 1) as f64 / FADE as f64);
                }
                self.fade -= 1;
            }
        }
        self.fault.map_or(Ok(()), Err)
    }
}
