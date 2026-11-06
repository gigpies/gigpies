//! Controller-side nonblocking duplex pump shared by raw ALSA and fake devices.
use super::device::{DeviceCapabilities, DeviceConfig, DeviceError, DuplexDevice};
use serde::Serialize;

/// Actual media adapters implement this boundary. Capture is mono after mapping
/// and local gain; playback is stereo before local mute/dim/gain and physical map.
/// The callbacks must be bounded, allocation/free/lock/I/O-free prepared renderers.
pub trait BrainRender {
    fn capture(&mut self, device_epoch: u64, first_frame: u64, mono: &[f64]);
    fn playback(&mut self, device_epoch: u64, first_frame: u64, stereo: &mut [f64]);
    fn fault(&mut self, device_epoch: u64, error: DeviceError);
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DeviceReadback {
    pub epoch: u64,
    pub configuration_generation: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct DeviceStatus {
    pub readback: DeviceReadback,
    pub capabilities: DeviceCapabilities,
    pub physical_mapping_verified: bool,
    pub armed: bool,
    pub fault: Option<DeviceError>,
    pub capture_frame: u64,
    pub playback_frame: u64,
    pub captured_peak: f64,
    pub outgoing_peak: f64,
    pub playback_peak: f64,
    pub capture_stalls: u64,
    pub playback_stalls: u64,
    pub faults: u64,
}
/// Allocation-free observation copied by the sole service owner. Static device
/// descriptions are joined to this value by the controller, never by service.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ServiceStatus {
    pub readback: DeviceReadback,
    pub armed: bool,
    pub fault: Option<DeviceError>,
    pub capture_frame: u64,
    pub playback_frame: u64,
    pub captured_peak: f64,
    pub outgoing_peak: f64,
    pub playback_peak: f64,
    pub capture_stalls: u64,
    pub playback_stalls: u64,
    pub faults: u64,
}
impl ServiceStatus {
    pub fn with_description(
        self,
        capabilities: DeviceCapabilities,
        physical_mapping_verified: bool,
    ) -> DeviceStatus {
        DeviceStatus {
            readback: self.readback,
            capabilities,
            physical_mapping_verified,
            armed: self.armed,
            fault: self.fault,
            capture_frame: self.capture_frame,
            playback_frame: self.playback_frame,
            captured_peak: self.captured_peak,
            outgoing_peak: self.outgoing_peak,
            playback_peak: self.playback_peak,
            capture_stalls: self.capture_stalls,
            playback_stalls: self.playback_stalls,
            faults: self.faults,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct LocalLevels {
    pub microphone_gain_db: f64,
    pub microphone_muted: bool,
    pub monitor_gain_db: f64,
    pub monitor_muted: bool,
    pub monitor_dim: bool,
}
impl Default for LocalLevels {
    fn default() -> Self {
        Self {
            microphone_gain_db: 0.,
            microphone_muted: true,
            monitor_gain_db: -20.,
            monitor_muted: true,
            monitor_dim: false,
        }
    }
}
impl LocalLevels {
    fn valid(self) -> bool {
        self.microphone_gain_db.is_finite()
            && (-60.0..=20.0).contains(&self.microphone_gain_db)
            && self.monitor_gain_db.is_finite()
            && (-90.0..=0.0).contains(&self.monitor_gain_db)
    }
}
pub struct BrainHost<D: DuplexDevice> {
    device: D,
    config: DeviceConfig,
    epoch: u64,
    generation: u64,
    armed: bool,
    fault: Option<DeviceError>,
    capture: Vec<f64>,
    playback: Vec<f64>,
    mono: Vec<f64>,
    stereo: Vec<f64>,
    captured: usize,
    playback_offset: usize,
    playback_pending: bool,
    capture_frame: u64,
    playback_frame: u64,
    microphone_target: f64,
    monitor_target: f64,
    mic_gain: f64,
    monitor_gain: f64,
    now_ms: Option<u64>,
    capture_progress_ms: u64,
    playback_progress_ms: u64,
    captured_peak: f64,
    outgoing_peak: f64,
    playback_peak: f64,
    capture_stalls: u64,
    playback_stalls: u64,
    faults: u64,
}
impl<D: DuplexDevice> BrainHost<D> {
    /// Preparation validates exact negotiated capabilities before allocating. Fresh
    /// nonzero device epochs/generations are supplied by the durable process owner.
    pub fn prepare(
        device: D,
        config: DeviceConfig,
        epoch: u64,
        generation: u64,
    ) -> Result<Self, DeviceError> {
        config.validate_capabilities(device.capabilities())?;
        if epoch == 0 || generation == 0 {
            return Err(DeviceError::Configuration);
        }
        let n = config.period_frames;
        Ok(Self {
            capture: vec![0.; n * config.capture_channels],
            playback: vec![0.; n * config.playback_channels],
            mono: vec![0.; n],
            stereo: vec![0.; n * 2],
            device,
            config,
            epoch,
            generation,
            armed: false,
            fault: None,
            captured: 0,
            playback_offset: 0,
            playback_pending: false,
            capture_frame: 0,
            playback_frame: 0,
            microphone_target: 0.,
            monitor_target: 0.,
            mic_gain: 0.,
            monitor_gain: 0.,
            now_ms: None,
            capture_progress_ms: 0,
            playback_progress_ms: 0,
            captured_peak: 0.,
            outgoing_peak: 0.,
            playback_peak: 0.,
            capture_stalls: 0,
            playback_stalls: 0,
            faults: 0,
        })
    }
    /// Requested sample allocations only; excludes allocator overhead and any
    /// library/device state not represented by these owned sample vectors.
    pub fn prepared_sample_bytes(&self) -> usize {
        [
            self.capture.capacity(),
            self.playback.capacity(),
            self.mono.capacity(),
            self.stereo.capacity(),
        ]
        .into_iter()
        .map(|n| n * std::mem::size_of::<f64>())
        .sum()
    }
    /// Startup only: touch prepared storage before the first service quantum.
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(crate) fn prefault_prepared(&mut self) {
        super::prefault_samples(&mut self.capture);
        super::prefault_samples(&mut self.playback);
        super::prefault_samples(&mut self.mono);
        super::prefault_samples(&mut self.stereo);
    }
    pub fn is_armed(&self) -> bool {
        self.armed
    }
    pub fn fault_reason(&self) -> Option<DeviceError> {
        self.fault
    }
    pub fn config(&self) -> &DeviceConfig {
        &self.config
    }
    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }
    pub fn readback(&self) -> DeviceReadback {
        DeviceReadback {
            epoch: self.epoch,
            configuration_generation: self.generation,
        }
    }
    /// Controller-only convenience: joins copied status to static descriptions.
    pub fn status(&self) -> DeviceStatus {
        self.service_status().with_description(
            self.device.capabilities().clone(),
            self.device.capabilities().physical && self.config.physically_verified(),
        )
    }
    pub fn service_status(&self) -> ServiceStatus {
        ServiceStatus {
            readback: self.readback(),
            armed: self.armed,
            fault: self.fault,
            capture_frame: self.capture_frame,
            playback_frame: self.playback_frame,
            captured_peak: self.captured_peak,
            outgoing_peak: self.outgoing_peak,
            playback_peak: self.playback_peak,
            capture_stalls: self.capture_stalls,
            playback_stalls: self.playback_stalls,
            faults: self.faults,
        }
    }
    pub fn set_levels(&mut self, levels: LocalLevels) -> Result<(), DeviceError> {
        if !levels.valid() {
            return Err(DeviceError::Configuration);
        }
        self.microphone_target = if levels.microphone_muted {
            0.
        } else {
            10_f64.powf(levels.microphone_gain_db / 20.)
        };
        self.monitor_target = if levels.monitor_muted {
            0.
        } else {
            10_f64.powf((levels.monitor_gain_db - if levels.monitor_dim { 20. } else { 0. }) / 20.)
        };
        Ok(())
    }
    pub fn arm(&mut self, readback: DeviceReadback, now_ms: u64) -> Result<(), DeviceError> {
        if readback != self.readback() {
            return Err(DeviceError::StaleReadback);
        }
        if self.fault.is_some() || self.armed {
            return Err(DeviceError::NotArmed);
        }
        if let Err(error) = self.device.start() {
            self.device.stop();
            self.fault = Some(error);
            self.faults += 1;
            return Err(error);
        }
        self.armed = true;
        self.now_ms = Some(now_ms);
        self.capture_progress_ms = now_ms;
        self.playback_progress_ms = now_ms;
        Ok(())
    }
    /// Route/selection barrier: silence every not-yet-submitted frame of the
    /// prepared playback period, preserving duplex clocks and capture. Returns
    /// the first unsubmitted device frame; an already submitted driver tail is
    /// bounded separately by configured buffer_frames and cannot be withdrawn
    /// here without discontinuing the whole duplex epoch. The caller also resets
    /// or invalidates its monitor bridge at this same control boundary.
    pub fn invalidate_monitor_output(&mut self) -> Option<u64> {
        let offset = if self.playback_pending {
            self.playback_offset
        } else {
            0
        };
        let first = self.playback_frame.checked_add(offset as u64);
        if self.playback_pending {
            self.playback[self.playback_offset * self.config.playback_channels..].fill(0.);
        }
        self.stereo.fill(0.);
        self.monitor_gain = 0.;
        first
    }
    /// Stop consumes this epoch's ability to arm. Reopen is a newly prepared host,
    /// fresh epoch/configuration readback, and explicit arm; never automatic recover.
    pub fn stop(&mut self) {
        self.device.stop();
        self.armed = false;
        self.fault = Some(DeviceError::NotArmed);
        self.capture.fill(0.);
        self.playback.fill(0.);
        self.mono.fill(0.);
        self.stereo.fill(0.);
        self.captured = 0;
        self.playback_pending = false;
        self.mic_gain = 0.;
        self.monitor_gain = 0.;
    }
    fn fail<R: BrainRender>(&mut self, renderer: &mut R, error: DeviceError) -> DeviceError {
        self.device.stop();
        self.armed = false;
        self.fault = Some(error);
        self.faults += 1;
        self.playback_pending = false;
        self.playback.fill(0.);
        self.mono.fill(0.);
        self.stereo.fill(0.);
        self.mic_gain = 0.;
        self.monitor_gain = 0.;
        renderer.fault(self.epoch, error);
        error
    }
    /// One read and one write per call, at most one period rendered per direction.
    /// No socket or file work belongs in the render callbacks. A driver stall of
    /// 100 ms invalidates the whole duplex epoch; unrelated Stagebox work survives.
    pub fn service<R: BrainRender>(
        &mut self,
        now_ms: u64,
        renderer: &mut R,
    ) -> Result<(), DeviceError> {
        if !self.armed {
            return Err(self.fault.unwrap_or(DeviceError::NotArmed));
        }
        if self.now_ms.is_some_and(|last| now_ms < last) {
            return Err(self.fail(renderer, DeviceError::ClockDiscontinuity));
        }
        self.now_ms = Some(now_ms);
        let channels = self.config.capture_channels;
        let period = self.config.period_frames;
        match self
            .device
            .read_capture(&mut self.capture[self.captured * channels..])
        {
            Ok(n) if n <= period - self.captured => {
                if n > 0 {
                    self.capture_progress_ms = now_ms;
                } else {
                    self.capture_stalls += 1;
                }
                self.captured += n;
            }
            Ok(_) => return Err(self.fail(renderer, DeviceError::InvalidProgress)),
            Err(DeviceError::WouldBlock) => self.capture_stalls += 1,
            Err(e) => return Err(self.fail(renderer, e)),
        }
        if self.captured == period {
            if self.capture.iter().any(|x| !x.is_finite()) {
                return Err(self.fail(renderer, DeviceError::Nonfinite));
            }
            let gain = self.microphone_target;
            self.captured_peak = 0.;
            self.outgoing_peak = 0.;
            for n in 0..period {
                let sample = self.capture[n * channels + self.config.microphone.slot];
                self.captured_peak = self.captured_peak.max(sample.abs());
                self.mic_gain += (gain - self.mic_gain).clamp(-10. / 240., 10. / 240.);
                self.mono[n] = (sample * self.mic_gain).clamp(-1., 1.);
                self.outgoing_peak = self.outgoing_peak.max(self.mono[n].abs());
            }
            renderer.capture(self.epoch, self.capture_frame, &self.mono);
            let Some(next) = self.capture_frame.checked_add(period as u64) else {
                return Err(self.fail(renderer, DeviceError::ClockDiscontinuity));
            };
            self.capture_frame = next;
            self.captured = 0;
        }
        if !self.playback_pending {
            self.stereo.fill(0.);
            renderer.playback(self.epoch, self.playback_frame, &mut self.stereo);
            if self.stereo.iter().any(|x| !x.is_finite()) {
                return Err(self.fail(renderer, DeviceError::Nonfinite));
            }
            let gain = self.monitor_target;
            self.playback.fill(0.);
            self.playback_peak = 0.;
            for n in 0..period {
                self.monitor_gain += (gain - self.monitor_gain).clamp(-1. / 240., 1. / 240.);
                for c in 0..2 {
                    let v = (self.stereo[n * 2 + c] * self.monitor_gain).clamp(-1., 1.);
                    self.playback
                        [n * self.config.playback_channels + self.config.monitor[c].slot] = v;
                    self.playback_peak = self.playback_peak.max(v.abs());
                }
            }
            self.playback_pending = true;
            self.playback_offset = 0;
        }
        match self
            .device
            .write_playback(&self.playback[self.playback_offset * self.config.playback_channels..])
        {
            Ok(n) if n <= period - self.playback_offset => {
                if n > 0 {
                    self.playback_progress_ms = now_ms;
                } else {
                    self.playback_stalls += 1;
                }
                self.playback_offset += n;
            }
            Ok(_) => return Err(self.fail(renderer, DeviceError::InvalidProgress)),
            Err(DeviceError::WouldBlock) => self.playback_stalls += 1,
            Err(e) => return Err(self.fail(renderer, e)),
        }
        if self.playback_offset == period {
            self.playback_pending = false;
            let Some(next) = self.playback_frame.checked_add(period as u64) else {
                return Err(self.fail(renderer, DeviceError::ClockDiscontinuity));
            };
            self.playback_frame = next;
        }
        if now_ms.saturating_sub(self.capture_progress_ms) > 100
            || now_ms.saturating_sub(self.playback_progress_ms) > 100
        {
            return Err(self.fail(renderer, DeviceError::Deadline));
        }
        Ok(())
    }
}
impl<D: DuplexDevice> Drop for BrainHost<D> {
    fn drop(&mut self) {
        self.device.stop();
    }
}
