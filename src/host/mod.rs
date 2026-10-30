//! Source-frame integration contracts. Hardware activation is a separate opt-in binary.
use crate::transport::{
    Encoding, Error, JitterBuffer, MAX_DATAGRAM, Packet, Role, StreamSpec, WetGate,
};
pub const PACKET_FRAMES: usize = 48;
pub const MAX_BLOCK: usize = 384;
pub const RETURN_DELAY: u32 = 384;
pub mod transfer;

/// Explicit bounded device buffering, independent of the wet-return deadline.
pub fn device_buffer_frames(period: usize, periods: u32) -> Result<usize, Error> {
    if ![48, 96, 192, 384].contains(&period) || ![2, 3, 4, 8].contains(&periods) {
        return Err(Error::Format);
    }
    Ok(period * periods as usize)
}

/// Playback occupancy is explicit and independent of ring capacity.
pub fn device_prefill_frames(period: usize, periods: u32, prefill: u32) -> Result<usize, Error> {
    device_buffer_frames(period, periods)?;
    if prefill >= periods {
        return Err(Error::Format);
    }
    Ok(period * prefill as usize)
}

pub fn valid_return_delay(delay: u32) -> bool {
    (48..=768).contains(&delay) && delay.is_multiple_of(48)
}

/// Sends from the current capture block cannot return before that block renders.
pub fn return_delay_fits_period(period: usize, delay: u32) -> bool {
    valid_return_delay(delay) && delay as usize >= period
}

pub fn spec(epoch: u64, role: Role) -> StreamSpec {
    spec_with_delay(epoch, role, RETURN_DELAY)
}

pub fn spec_with_delay(epoch: u64, role: Role, delay: u32) -> StreamSpec {
    StreamSpec {
        session: epoch,
        stream: role as u32,
        first_channel: 0,
        channels: 2,
        frames: PACKET_FRAMES as u16,
        encoding: Encoding::Float32,
        role,
        delay_frames: if role == Role::WetReturn { delay } else { 0 },
    }
}
#[derive(Clone, Copy)]
pub struct Datagram {
    pub len: usize,
    pub bytes: [u8; MAX_DATAGRAM],
}
impl Default for Datagram {
    fn default() -> Self {
        Self {
            len: 0,
            bytes: [0; MAX_DATAGRAM],
        }
    }
}

/// Insert only before rendering a block. A queued return due at this block's first
/// frame is usable; an earlier output frame is expired. The audio thread owns this
/// object, so network arrival can never advance the output cursor.
pub struct WetRender {
    buffer: JitterBuffer,
    gates: [WetGate; 2],
    packet: [u8; MAX_DATAGRAM],
    pub expired: u64,
    pub rejected: u64,
    pub missing: u64,
    pub present: u64,
    next_output: u64,
    delay: u32,
}
impl WetRender {
    pub fn new(epoch: u64) -> Result<Self, Error> {
        Self::with_delay(epoch, RETURN_DELAY)
    }
    pub fn with_delay(epoch: u64, delay: u32) -> Result<Self, Error> {
        if !valid_return_delay(delay) {
            return Err(Error::Format);
        }
        Ok(Self {
            buffer: JitterBuffer::new(spec_with_delay(epoch, Role::WetReturn, delay), 0, 0, 32)?,
            delay,
            gates: [WetGate::default(); 2],
            packet: [0; MAX_DATAGRAM],
            expired: 0,
            rejected: 0,
            missing: 0,
            present: 0,
            next_output: 0,
        })
    }
    pub fn admit(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let p = Packet::parse(bytes)?;
        if p.output_frame() < self.next_output {
            self.expired += 1;
            return Err(Error::Late);
        }
        let result = self.buffer.insert(p);
        if result.is_err() {
            self.rejected += 1;
        }
        result
    }
    pub fn render(&mut self, first: u64, out: &mut [f64]) -> Result<(), Error> {
        if first != self.next_output
            || out.is_empty()
            || !out.len().is_multiple_of(96)
            || out.len() > MAX_BLOCK * 2
        {
            return Err(Error::Timeline);
        }
        for (index, stereo) in out.chunks_exact_mut(96).enumerate() {
            let frame = first + (index * 48) as u64;
            let present = if frame >= u64::from(self.delay) {
                let found = self.buffer.pop(&mut self.packet)?.is_some();
                if found {
                    self.present += 1;
                } else {
                    self.missing += 1;
                }
                found
            } else {
                false
            };
            // Every channel gets its own envelope, including complete packet loss.
            for (i, x) in stereo.iter_mut().enumerate() {
                let value = if present {
                    Some(
                        f32::from_be_bytes(self.packet[48 + i * 4..52 + i * 4].try_into().unwrap())
                            as f64,
                    )
                } else {
                    None
                };
                *x = self.gates[i % 2].step(value);
            }
        }
        self.next_output += (out.len() / 2) as u64;
        Ok(())
    }
    pub fn occupancy(&self) -> usize {
        self.buffer.occupancy()
    }
}

/// Deterministic distinct-channel, peak-bounded test stimulus, quantized to PCM24.
/// It is an explicit bench source, never an implicit microphone monitoring route.
pub fn stimulus(frame: u64, channel: usize) -> f64 {
    let frequency = if channel == 0 { 997.0 } else { 1499.0 };
    let ramp = (frame as f64 / 240.0).min(1.0);
    ((std::f64::consts::TAU * frequency * frame as f64 / 48_000.0).sin()
        * 0.015848931924611134
        * ramp
        * 8388608.0)
        .round()
        / 8388608.0
}

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Probe {
    StereoTones,
    LeftCoded,
    LeftContinuous,
}

/// Quiet deterministic probes for correlating physical input with recorded output.
/// Only channel 1 emits; continuous mode becomes active after one quiet second.
pub fn probe_sample(probe: Probe, frame: u64, channel: usize) -> f64 {
    match probe {
        Probe::StereoTones => stimulus(frame, channel),
        Probe::LeftCoded | Probe::LeftContinuous => {
            let quiet = match probe {
                Probe::LeftCoded => frame % 96_000 < 48_000,
                _ => frame < 48_000,
            };
            if channel != 0 || quiet {
                return 0.0;
            }
            let mut bits = frame.wrapping_add(0x9e3779b97f4a7c15);
            bits = (bits ^ (bits >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            bits = (bits ^ (bits >> 27)).wrapping_mul(0x94d049bb133111eb);
            bits ^= bits >> 31;
            let unit = (bits >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
            (unit * 0.0019952623149688795 * 8388608.0).round() / 8388608.0
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub struct Histogram {
    bins: Vec<u64>,
    pub count: u64,
    pub max_us: f64,
    pub overflow: u64,
}
impl Default for Histogram {
    fn default() -> Self {
        Self {
            bins: vec![0; 10001],
            count: 0,
            max_us: 0.0,
            overflow: 0,
        }
    }
}
impl Histogram {
    pub fn observe(&mut self, us: f64) {
        self.count += 1;
        self.max_us = self.max_us.max(us);
        let i = us.ceil().max(0.0) as usize;
        if i < self.bins.len() {
            self.bins[i] += 1;
        } else {
            self.overflow += 1;
        }
    }
    pub fn percentile(&self, p: f64) -> Option<f64> {
        if self.count == 0 || !(0.0..=1.0).contains(&p) {
            return None;
        }
        let rank = (self.count as f64 * p).ceil() as u64;
        let mut n = 0;
        for (i, v) in self.bins.iter().enumerate() {
            n += v;
            if n >= rank {
                return Some(i as f64);
            }
        }
        None // Rank is outside the bounded histogram; maximum is not a percentile.
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"count":self.count,"p50_us":self.percentile(0.50),"p95_us":self.percentile(0.95),"p99_us":self.percentile(0.99),"max_us":self.max_us,"overflow":self.overflow})
    }
}
#[cfg(feature = "hardware-host")]
pub mod adapters;
#[cfg(feature = "hardware-host")]
pub mod device;
#[cfg(feature = "hardware-host")]
mod memory;
#[cfg(feature = "hardware-host")]
pub mod network;
#[cfg(feature = "hardware-host")]
mod scheduling;
#[cfg(feature = "hardware-host")]
mod usage;

/// Restrict this first host to one unambiguous native format in both directions.
/// Descriptor bits declare nominal payload width, not measured converter resolution.
pub fn native_stereo_24(descriptor: &str) -> bool {
    ["Capture:", "Playback:"].iter().all(|direction| {
        let Some((_, section)) = descriptor.split_once(direction) else {
            return false;
        };
        let section = section.split("\n\n").next().unwrap_or("");
        section.matches("Format:").count() == 1
            && section.contains("Format: S32_LE")
            && section.contains("Bits: 24")
            && section.contains("Channels: 2")
    })
}

/// Extract the declared upper 24 USB sample bits without rounding unused container
/// bits into the payload. The AudioBox descriptor says 24, while ALSA reports S32.
pub fn capture_msb24(container: i32) -> f64 {
    f64::from(container >> 8) / 8_388_608.0
}

/// Count either nominal PCM24 endpoint regardless of unused container bits.
pub fn capture_fullscale(container: i32) -> bool {
    matches!(container >> 8, -8_388_608 | 8_388_607)
}

/// Configurable trusted PA owner ABI, prepared and retired off rendering.
#[cfg(feature = "hardware-host")]
pub mod pa_v2;

/// Explicit raw duplex adapter invoking the common configured provider graph.
#[cfg(all(feature = "hardware-host", target_os = "linux"))]
pub mod duplex;

/// Brain owner effects follow source frames, independently of network arrival.
#[cfg(feature = "hardware-host")]
pub mod brain_fx;

#[cfg(feature = "hardware-host")]
pub mod fx_palette;
#[cfg(feature = "hardware-host")]
pub mod fx_v2;
