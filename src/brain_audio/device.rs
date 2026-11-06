//! One duplex endpoint, explicit physical maps, and a shared fake/raw boundary.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleFormat {
    S16Le,
    S32Le,
    Float32Le,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortMap {
    pub id: String,
    pub socket: String,
    pub slot: usize,
}
/// Stable intent only. No epoch, grant, talkback hold or playback permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    pub device_id: String,
    pub endpoint: String,
    pub sample_rate: u32,
    pub capture_channels: usize,
    pub playback_channels: usize,
    pub format: SampleFormat,
    pub period_frames: usize,
    pub buffer_frames: usize,
    pub microphone: PortMap,
    /// Exactly two distinct slots. Mono is not silently substituted for stereo.
    pub monitor: [PortMap; 2],
    /// Operator-supplied observations; empty means physically unverified.
    pub socket_mapping_record: Option<String>,
    pub shared_clock_record: Option<String>,
    pub hardware_monitoring_record: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceCapabilities {
    pub device_id: String,
    pub endpoint: String,
    pub sample_rate: u32,
    pub capture_channels: usize,
    pub playback_channels: usize,
    pub format: SampleFormat,
    pub period_frames: usize,
    pub buffer_frames: usize,
    pub shared_duplex_clock: bool,
    pub physical: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceError {
    Configuration,
    Mapping,
    CapabilityMismatch,
    UnsupportedFormat,
    UnsupportedRate,
    UnsupportedWidth,
    UnsupportedPeriod,
    UnsupportedBuffer,
    PhysicalEvidence,
    NotArmed,
    StaleReadback,
    WouldBlock,
    Xrun,
    Disconnected,
    ClockDiscontinuity,
    Driver,
    InvalidProgress,
    Nonfinite,
    Deadline,
}
impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "brain device {self:?}")
    }
}
impl std::error::Error for DeviceError {}
fn valid_label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
impl DeviceConfig {
    pub fn validate(&self) -> Result<(), DeviceError> {
        if !valid_label(&self.device_id)
            || !valid_label(&self.endpoint)
            || self.sample_rate != 48_000
            || !(1..=256).contains(&self.capture_channels)
            || !(2..=256).contains(&self.playback_channels)
            || !(48..=1024).contains(&self.period_frames)
            || !self.period_frames.is_multiple_of(48)
            || self.buffer_frames < 2 * self.period_frames
            || self.buffer_frames > 48_000
        {
            return Err(DeviceError::Configuration);
        }
        for p in [&self.microphone, &self.monitor[0], &self.monitor[1]] {
            if !valid_label(&p.id) || !valid_label(&p.socket) {
                return Err(DeviceError::Mapping);
            }
        }
        if self.microphone.slot >= self.capture_channels
            || self
                .monitor
                .iter()
                .any(|p| p.slot >= self.playback_channels)
            || self.monitor[0].slot == self.monitor[1].slot
            || self.monitor[0].id == self.monitor[1].id
            || self.monitor[0].socket == self.monitor[1].socket
            || self.monitor.iter().any(|p| p.id == self.microphone.id)
        {
            return Err(DeviceError::Mapping);
        }
        for record in [
            &self.socket_mapping_record,
            &self.shared_clock_record,
            &self.hardware_monitoring_record,
        ]
        .into_iter()
        .flatten()
        {
            if record.trim().is_empty() || record.len() > 4096 {
                return Err(DeviceError::PhysicalEvidence);
            }
        }
        Ok(())
    }
    pub fn physically_verified(&self) -> bool {
        self.socket_mapping_record.is_some()
            && self.shared_clock_record.is_some()
            && self.hardware_monitoring_record.is_some()
    }
    pub fn validate_capabilities(&self, c: &DeviceCapabilities) -> Result<(), DeviceError> {
        self.validate()?;
        if self.device_id != c.device_id
            || self.endpoint != c.endpoint
            || self.sample_rate != c.sample_rate
            || self.capture_channels != c.capture_channels
            || self.playback_channels != c.playback_channels
            || self.format != c.format
            || self.period_frames != c.period_frames
            || self.buffer_frames != c.buffer_frames
            || !c.shared_duplex_clock
        {
            return Err(DeviceError::CapabilityMismatch);
        }
        if c.physical && !self.physically_verified() {
            return Err(DeviceError::PhysicalEvidence);
        }
        Ok(())
    }
}
/// Driver I/O runs outside the bounded renderer. Each method makes at most one
/// nonblocking transfer; zero progress is ordinary WouldBlock, never busy-waited.
pub trait DuplexDevice {
    fn capabilities(&self) -> &DeviceCapabilities;
    fn start(&mut self) -> Result<(), DeviceError>;
    fn read_capture(&mut self, interleaved: &mut [f64]) -> Result<usize, DeviceError>;
    fn write_playback(&mut self, interleaved: &[f64]) -> Result<usize, DeviceError>;
    fn stop(&mut self);
}

/// Deterministic boundary device. External virtual device time supplies capture
/// and playback credit independently of the Stagebox clock; no physical activation.
pub struct FakeDuplex {
    capabilities: DeviceCapabilities,
    capture: Vec<f64>,
    playback: Vec<f64>,
    capture_read: usize,
    capture_len: usize,
    playback_read: usize,
    playback_len: usize,
    credit: usize,
    started: bool,
    fault: Option<DeviceError>,
    max_transfer: usize,
}
impl FakeDuplex {
    pub fn prepare(config: &DeviceConfig) -> Result<Self, DeviceError> {
        config.validate()?;
        let capabilities = DeviceCapabilities {
            device_id: config.device_id.clone(),
            endpoint: config.endpoint.clone(),
            sample_rate: config.sample_rate,
            capture_channels: config.capture_channels,
            playback_channels: config.playback_channels,
            format: config.format,
            period_frames: config.period_frames,
            buffer_frames: config.buffer_frames,
            shared_duplex_clock: true,
            physical: false,
        };
        Ok(Self {
            capture: vec![0.; config.buffer_frames * config.capture_channels],
            playback: vec![0.; config.buffer_frames * config.playback_channels],
            capabilities,
            capture_read: 0,
            capture_len: 0,
            playback_read: 0,
            playback_len: 0,
            credit: 0,
            started: false,
            fault: None,
            max_transfer: config.period_frames,
        })
    }
    /// Requested sample allocations only; excludes allocator overhead and any
    /// library/device state not represented by these owned sample vectors.
    pub fn prepared_sample_bytes(&self) -> usize {
        [self.capture.capacity(), self.playback.capacity()]
            .into_iter()
            .map(|n| n * std::mem::size_of::<f64>())
            .sum()
    }
    /// Startup only: touch prepared storage before the first service quantum.
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(crate) fn prefault_prepared(&mut self) {
        super::prefault_samples(&mut self.capture);
        super::prefault_samples(&mut self.playback);
    }
    pub fn set_max_transfer(&mut self, frames: usize) {
        self.max_transfer = frames;
    }
    pub fn inject_fault(&mut self, error: DeviceError) {
        self.fault = Some(error);
    }
    pub fn advance(&mut self, capture: &[f64], playback_frames: usize) -> Result<(), DeviceError> {
        let channels = self.capabilities.capture_channels;
        if !capture.len().is_multiple_of(channels) || capture.iter().any(|x| !x.is_finite()) {
            return Err(DeviceError::Nonfinite);
        }
        if capture.len() > self.capture.len() - self.capture_len {
            self.fault = Some(DeviceError::Xrun);
            return Err(DeviceError::Xrun);
        }
        for (n, v) in capture.iter().enumerate() {
            let p = (self.capture_read + self.capture_len + n) % self.capture.len();
            self.capture[p] = *v;
        }
        self.capture_len += capture.len();
        self.credit = self
            .credit
            .saturating_add(playback_frames)
            .min(self.capabilities.buffer_frames);
        Ok(())
    }
    pub fn drain_playback(&mut self, out: &mut [f64]) -> usize {
        let n = out.len().min(self.playback_len) / self.capabilities.playback_channels
            * self.capabilities.playback_channels;
        for (i, v) in out[..n].iter_mut().enumerate() {
            *v = self.playback[(self.playback_read + i) % self.playback.len()];
        }
        self.playback_read = (self.playback_read + n) % self.playback.len();
        self.playback_len -= n;
        n / self.capabilities.playback_channels
    }
}
impl DuplexDevice for FakeDuplex {
    fn capabilities(&self) -> &DeviceCapabilities {
        &self.capabilities
    }
    fn start(&mut self) -> Result<(), DeviceError> {
        if let Some(e) = self.fault {
            return Err(e);
        }
        self.started = true;
        Ok(())
    }
    fn read_capture(&mut self, out: &mut [f64]) -> Result<usize, DeviceError> {
        if let Some(e) = self.fault {
            return Err(e);
        }
        if !self.started {
            return Err(DeviceError::NotArmed);
        }
        let c = self.capabilities.capture_channels;
        let frames = (out.len() / c)
            .min(self.capture_len / c)
            .min(self.max_transfer);
        for (n, v) in out[..frames * c].iter_mut().enumerate() {
            *v = self.capture[(self.capture_read + n) % self.capture.len()];
        }
        self.capture_read = (self.capture_read + frames * c) % self.capture.len();
        self.capture_len -= frames * c;
        Ok(frames)
    }
    fn write_playback(&mut self, input: &[f64]) -> Result<usize, DeviceError> {
        if let Some(e) = self.fault {
            return Err(e);
        }
        if !self.started {
            return Err(DeviceError::NotArmed);
        }
        let c = self.capabilities.playback_channels;
        let frames = (input.len() / c)
            .min((self.playback.len() - self.playback_len) / c)
            .min(self.credit)
            .min(self.max_transfer);
        for (n, v) in input[..frames * c].iter().enumerate() {
            let p = (self.playback_read + self.playback_len + n) % self.playback.len();
            self.playback[p] = *v;
        }
        self.playback_len += frames * c;
        self.credit -= frames;
        Ok(frames)
    }
    fn stop(&mut self) {
        self.started = false;
        self.capture_len = 0;
        self.playback_len = 0;
        self.credit = 0;
    }
}

#[cfg(all(feature = "hardware-host", target_os = "linux"))]
mod alsa_adapter {
    use super::*;
    use alsa::{
        Direction, ValueOr,
        pcm::{Access, Format, HwParams, PCM},
    };
    pub struct AlsaDuplex {
        capture: PCM,
        playback: PCM,
        caps: DeviceCapabilities,
        capture_i32: Vec<i32>,
        playback_i32: Vec<i32>,
        capture_i16: Vec<i16>,
        playback_i16: Vec<i16>,
        capture_f32: Vec<f32>,
        playback_f32: Vec<f32>,
        started: bool,
        playback_started: bool,
        playback_prefill: usize,
    }
    fn error(e: alsa::Error) -> DeviceError {
        match e.errno() {
            libc::EAGAIN | libc::EINTR => DeviceError::WouldBlock,
            libc::EPIPE | libc::ESTRPIPE => DeviceError::Xrun,
            libc::ENODEV | libc::ENXIO => DeviceError::Disconnected,
            _ => DeviceError::Driver,
        }
    }
    /// Detailed, controller-only opening failure. Strings are allocated only while
    /// preparing a stream, never by the capture/playback callback methods.
    #[derive(Debug, Clone, Serialize)]
    pub struct AlsaOpenError {
        pub code: DeviceError,
        pub direction: &'static str,
        pub operation: &'static str,
        pub alsa_errno: Option<i32>,
        pub message: String,
    }
    impl std::fmt::Display for AlsaOpenError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "Brain {} {}: {} ({:?}, ALSA errno {:?})",
                self.direction, self.operation, self.message, self.code, self.alsa_errno
            )
        }
    }
    impl std::error::Error for AlsaOpenError {}
    fn direction_name(direction: Direction) -> &'static str {
        match direction {
            Direction::Capture => "capture",
            Direction::Playback => "playback",
        }
    }
    fn diagnostic(
        direction: Direction,
        operation: &'static str,
        value: alsa::Error,
        unsupported: Option<DeviceError>,
    ) -> AlsaOpenError {
        AlsaOpenError {
            code: if value.errno() == libc::EINVAL {
                unsupported.unwrap_or_else(|| error(value))
            } else {
                error(value)
            },
            direction: direction_name(direction),
            operation,
            alsa_errno: Some(value.errno()),
            message: value.to_string(),
        }
    }
    fn mismatch(
        direction: Direction,
        code: DeviceError,
        operation: &'static str,
        expected: impl std::fmt::Debug,
        actual: impl std::fmt::Debug,
    ) -> AlsaOpenError {
        AlsaOpenError {
            code,
            direction: direction_name(direction),
            operation,
            alsa_errno: None,
            message: format!("requested {expected:?}, negotiated {actual:?}; no fallback"),
        }
    }
    fn open(
        config: &DeviceConfig,
        direction: Direction,
        channels: usize,
    ) -> Result<PCM, AlsaOpenError> {
        let d = |operation, e| diagnostic(direction, operation, e, None);
        let pcm = PCM::new(&config.endpoint, direction, true).map_err(|e| d("open endpoint", e))?;
        let h = HwParams::any(&pcm).map_err(|e| d("query hardware parameters", e))?;
        h.set_access(Access::RWInterleaved)
            .map_err(|e| d("set interleaved access", e))?;
        h.set_rate_resample(false)
            .map_err(|e| d("disable driver resampling", e))?;
        let format = match config.format {
            SampleFormat::S16Le => Format::S16LE,
            SampleFormat::S32Le => Format::S32LE,
            SampleFormat::Float32Le => Format::FloatLE,
        };
        h.set_format(format).map_err(|e| {
            diagnostic(
                direction,
                "set format",
                e,
                Some(DeviceError::UnsupportedFormat),
            )
        })?;
        h.set_channels(channels as u32).map_err(|e| {
            diagnostic(
                direction,
                "set channels",
                e,
                Some(DeviceError::UnsupportedWidth),
            )
        })?;
        h.set_rate(config.sample_rate, ValueOr::Nearest)
            .map_err(|e| {
                diagnostic(direction, "set rate", e, Some(DeviceError::UnsupportedRate))
            })?;
        h.set_period_size(config.period_frames as i64, ValueOr::Nearest)
            .map_err(|e| {
                diagnostic(
                    direction,
                    "set period",
                    e,
                    Some(DeviceError::UnsupportedPeriod),
                )
            })?;
        h.set_buffer_size(config.buffer_frames as i64)
            .map_err(|e| {
                diagnostic(
                    direction,
                    "set buffer",
                    e,
                    Some(DeviceError::UnsupportedBuffer),
                )
            })?;
        pcm.hw_params(&h)
            .map_err(|e| d("apply hardware parameters", e))?;
        let actual = pcm
            .hw_params_current()
            .map_err(|e| d("read hardware parameters", e))?;
        let rate = actual.get_rate().map_err(|e| d("read rate", e))?;
        let width = actual.get_channels().map_err(|e| d("read channels", e))?;
        let actual_format = actual.get_format().map_err(|e| d("read format", e))?;
        let period = actual.get_period_size().map_err(|e| d("read period", e))?;
        let buffer = actual.get_buffer_size().map_err(|e| d("read buffer", e))?;
        if rate != config.sample_rate {
            return Err(mismatch(
                direction,
                DeviceError::UnsupportedRate,
                "verify rate",
                config.sample_rate,
                rate,
            ));
        }
        if width != channels as u32 {
            return Err(mismatch(
                direction,
                DeviceError::UnsupportedWidth,
                "verify channels",
                channels,
                width,
            ));
        }
        if actual_format != format {
            return Err(mismatch(
                direction,
                DeviceError::UnsupportedFormat,
                "verify format",
                format,
                actual_format,
            ));
        }
        if period != config.period_frames as i64 {
            return Err(mismatch(
                direction,
                DeviceError::UnsupportedPeriod,
                "verify period",
                config.period_frames,
                period,
            ));
        }
        if buffer != config.buffer_frames as i64 {
            return Err(mismatch(
                direction,
                DeviceError::UnsupportedBuffer,
                "verify buffer",
                config.buffer_frames,
                buffer,
            ));
        }
        let sw = pcm
            .sw_params_current()
            .map_err(|e| d("query software parameters", e))?;
        sw.set_start_threshold(
            sw.get_boundary()
                .map_err(|e| d("query software boundary", e))?,
        )
        .map_err(|e| d("set explicit playback start", e))?;
        sw.set_avail_min(config.period_frames as i64)
            .map_err(|e| d("set available minimum", e))?;
        pcm.sw_params(&sw)
            .map_err(|e| d("apply software parameters", e))?;
        drop(sw);
        drop(actual);
        drop(h);
        pcm.prepare().map_err(|e| d("prepare endpoint", e))?;
        Ok(pcm)
    }
    impl AlsaDuplex {
        /// Only this explicit call activates the physical PCM. The executable must
        /// require separately authorized device activation and exact observed intent.
        pub fn open(config: &DeviceConfig) -> Result<Self, DeviceError> {
            Self::open_detailed(config).map_err(|e| e.code)
        }
        pub fn open_detailed(config: &DeviceConfig) -> Result<Self, AlsaOpenError> {
            let reject = |code, operation, message: &str| AlsaOpenError {
                code,
                direction: "duplex",
                operation,
                alsa_errno: None,
                message: message.into(),
            };
            config.validate().map_err(|code| {
                reject(
                    code,
                    "validate configuration",
                    "invalid explicit device configuration",
                )
            })?;
            if !config.endpoint.starts_with("hw:")
                || config.endpoint.len() <= 3
                || !config.physically_verified()
            {
                return Err(reject(
                    DeviceError::PhysicalEvidence,
                    "validate physical evidence",
                    "raw endpoint plus socket/shared-clock/direct-monitoring records required",
                ));
            }
            let capture = open(config, Direction::Capture, config.capture_channels)?;
            let playback = open(config, Direction::Playback, config.playback_channels)?;
            let ci = capture
                .info()
                .map_err(|e| diagnostic(Direction::Capture, "read identity", e, None))?;
            let pi = playback
                .info()
                .map_err(|e| diagnostic(Direction::Playback, "read identity", e, None))?;
            if ci.get_card() != pi.get_card()
                || ci.get_device() != pi.get_device()
                || ci.get_subdevice() != pi.get_subdevice()
            {
                return Err(reject(
                    DeviceError::CapabilityMismatch,
                    "verify duplex identity",
                    "capture/playback card/device/subdevice differ",
                ));
            }
            let caps = DeviceCapabilities {
                device_id: config.device_id.clone(),
                endpoint: config.endpoint.clone(),
                sample_rate: config.sample_rate,
                capture_channels: config.capture_channels,
                playback_channels: config.playback_channels,
                format: config.format,
                period_frames: config.period_frames,
                buffer_frames: config.buffer_frames,
                shared_duplex_clock: true,
                physical: true,
            };
            let n = config.period_frames * config.capture_channels;
            let m = config.period_frames * config.playback_channels;
            Ok(Self {
                capture,
                playback,
                caps,
                capture_i32: vec![0; n],
                playback_i32: vec![0; m],
                capture_i16: vec![0; n],
                playback_i16: vec![0; m],
                capture_f32: vec![0.; n],
                playback_f32: vec![0.; m],
                started: false,
                playback_started: false,
                playback_prefill: 0,
            })
        }
    }
    impl DuplexDevice for AlsaDuplex {
        fn capabilities(&self) -> &DeviceCapabilities {
            &self.caps
        }
        fn start(&mut self) -> Result<(), DeviceError> {
            self.capture.start().map_err(error)?;
            self.started = true;
            Ok(())
        }
        fn read_capture(&mut self, out: &mut [f64]) -> Result<usize, DeviceError> {
            if !self.started {
                return Err(DeviceError::NotArmed);
            }
            let count = out
                .len()
                .min(self.caps.period_frames * self.caps.capture_channels);
            let frames = match self.caps.format {
                SampleFormat::S16Le => {
                    let n = self
                        .capture
                        .io_i16()
                        .map_err(error)?
                        .readi(&mut self.capture_i16[..count])
                        .map_err(error)?;
                    for (o, v) in out
                        .iter_mut()
                        .zip(&self.capture_i16)
                        .take(n * self.caps.capture_channels)
                    {
                        *o = f64::from(*v) / 32768.;
                    }
                    n
                }
                SampleFormat::S32Le => {
                    let n = self
                        .capture
                        .io_i32()
                        .map_err(error)?
                        .readi(&mut self.capture_i32[..count])
                        .map_err(error)?;
                    for (o, v) in out
                        .iter_mut()
                        .zip(&self.capture_i32)
                        .take(n * self.caps.capture_channels)
                    {
                        *o = f64::from(*v) / 2147483648.;
                    }
                    n
                }
                SampleFormat::Float32Le => {
                    let n = self
                        .capture
                        .io_f32()
                        .map_err(error)?
                        .readi(&mut self.capture_f32[..count])
                        .map_err(error)?;
                    for (o, v) in out
                        .iter_mut()
                        .zip(&self.capture_f32)
                        .take(n * self.caps.capture_channels)
                    {
                        *o = f64::from(*v);
                    }
                    n
                }
            };
            Ok(frames)
        }
        fn write_playback(&mut self, input: &[f64]) -> Result<usize, DeviceError> {
            if !self.started {
                return Err(DeviceError::NotArmed);
            }
            let count = input
                .len()
                .min(self.caps.period_frames * self.caps.playback_channels);
            if input[..count].iter().any(|x| !x.is_finite()) {
                return Err(DeviceError::Nonfinite);
            }
            let frames = match self.caps.format {
                SampleFormat::S16Le => {
                    for (v, x) in self.playback_i16.iter_mut().zip(input).take(count) {
                        *v = (x * 32768.).round().clamp(-32768., 32767.) as i16;
                    }
                    self.playback
                        .io_i16()
                        .map_err(error)?
                        .writei(&self.playback_i16[..count])
                        .map_err(error)?
                }
                SampleFormat::S32Le => {
                    for (v, x) in self.playback_i32.iter_mut().zip(input).take(count) {
                        *v = (x * 2147483648.).round().clamp(-2147483648., 2147483647.) as i32;
                    }
                    self.playback
                        .io_i32()
                        .map_err(error)?
                        .writei(&self.playback_i32[..count])
                        .map_err(error)?
                }
                SampleFormat::Float32Le => {
                    for (v, x) in self.playback_f32.iter_mut().zip(input).take(count) {
                        *v = x.clamp(-1., 1.) as f32;
                    }
                    self.playback
                        .io_f32()
                        .map_err(error)?
                        .writei(&self.playback_f32[..count])
                        .map_err(error)?
                }
            };
            if !self.playback_started {
                self.playback_prefill += frames;
                if self.playback_prefill >= self.caps.period_frames {
                    self.playback.start().map_err(error)?;
                    self.playback_started = true;
                }
            }
            Ok(frames)
        }
        fn stop(&mut self) {
            let _ = self.capture.drop();
            let _ = self.playback.drop();
            self.started = false;
        }
    }
    impl Drop for AlsaDuplex {
        fn drop(&mut self) {
            self.stop();
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn opening_diagnostics_preserve_direction_operation_errno_without_pcm() {
            let e = diagnostic(
                Direction::Capture,
                "set rate",
                alsa::Error::new("snd_pcm_hw_params_set_rate", libc::EINVAL),
                Some(DeviceError::UnsupportedRate),
            );
            assert_eq!(e.code, DeviceError::UnsupportedRate);
            assert_eq!(e.direction, "capture");
            assert_eq!(e.operation, "set rate");
            assert_eq!(e.alsa_errno, Some(libc::EINVAL));
            assert!(e.message.contains("snd_pcm_hw_params_set_rate"));
            let e = mismatch(
                Direction::Playback,
                DeviceError::UnsupportedPeriod,
                "verify period",
                48,
                96,
            );
            assert_eq!(e.direction, "playback");
            assert!(e.message.contains("48"));
            assert!(e.message.contains("96"));
            let e = diagnostic(
                Direction::Playback,
                "open endpoint",
                alsa::Error::new("snd_pcm_open", libc::ENODEV),
                None,
            );
            assert_eq!(e.code, DeviceError::Disconnected);
            assert_eq!(e.alsa_errno, Some(libc::ENODEV));
        }
    }
}
#[cfg(all(feature = "hardware-host", target_os = "linux"))]
pub use alsa_adapter::{AlsaDuplex, AlsaOpenError};
