use super::{Error, MAX_DATAGRAM, Packet, StreamSpec};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferStats {
    pub accepted: u64,
    pub missing: u64,
    pub late: u64,
    pub future: u64,
    pub duplicate: u64,
    pub reordered: u64,
    pub rejected: u64,
    pub peak_occupancy: usize,
}
struct Slot {
    frame: Option<u64>,
    bytes: [u8; MAX_DATAGRAM],
}
/// Fixed storage allocated at setup. Every insertion/pop is bounded by one packet.
/// Caller advances pop from the PA timeline, even if the network worker stalls.
/// Prefill is the caller's explicit deadline; arrival never moves the read cursor.
pub struct JitterBuffer {
    spec: StreamSpec,
    next: u64,
    origin: u64,
    sequence: u32,
    slots: Box<[Slot]>,
    occupancy: usize,
    high_frame: u64,
    stats: BufferStats,
}
impl JitterBuffer {
    pub fn new(
        spec: StreamSpec,
        first_frame: u64,
        first_sequence: u32,
        capacity: usize,
    ) -> Result<Self, Error> {
        spec.validate()?;
        if !(2..=32).contains(&capacity) {
            return Err(Error::Capacity);
        }
        if !first_frame.is_multiple_of(u64::from(spec.frames)) {
            return Err(Error::Timeline);
        }
        Ok(Self {
            spec,
            next: first_frame,
            origin: first_frame,
            sequence: first_sequence,
            slots: (0..capacity)
                .map(|_| Slot {
                    frame: None,
                    bytes: [0; MAX_DATAGRAM],
                })
                .collect(),
            occupancy: 0,
            high_frame: first_frame,
            stats: BufferStats::default(),
        })
    }
    /// Includes actual slot tags/alignment, not just wire packet payloads.
    pub fn prepared_storage(&self) -> super::PreparedStorage {
        super::PreparedStorage::checked(
            std::mem::size_of::<Self>(),
            std::mem::size_of_val(self.slots.as_ref()),
        )
        .expect("admitted bounded jitter storage")
    }
    /// Enforce the real PA deadline even if the receiving worker has not yet
    /// caught its read cursor up after a scheduling stall. Call after validation.
    pub fn insert_wet_at(&mut self, packet: Packet<'_>, pa_frame_now: u64) -> Result<(), Error> {
        if packet.spec().role != super::Role::WetReturn {
            return Err(Error::Identity);
        }
        if pa_frame_now >= packet.output_frame() {
            self.stats.late += 1;
            return Err(Error::Late);
        }
        self.insert(packet)
    }
    pub fn insert(&mut self, packet: Packet<'_>) -> Result<(), Error> {
        if packet.spec() != self.spec {
            self.stats.rejected += 1;
            return Err(Error::Identity);
        }
        let f = packet.source_frame();
        if f < self.next {
            self.stats.late += 1;
            return Err(Error::Late);
        }
        let blocks = (f - self.next) / u64::from(self.spec.frames);
        if blocks >= self.slots.len() as u64 {
            self.stats.future += 1;
            return Err(Error::Future);
        }
        let expected = self
            .sequence
            .wrapping_add(((f - self.origin) / u64::from(self.spec.frames)) as u32);
        if packet.sequence() != expected {
            self.stats.rejected += 1;
            return Err(Error::Timeline);
        }
        let index = (f / u64::from(self.spec.frames) % self.slots.len() as u64) as usize;
        let slot = &mut self.slots[index];
        if slot.frame.is_some() {
            self.stats.duplicate += 1;
            return Err(Error::Duplicate);
        }
        if f < self.high_frame {
            self.stats.reordered += 1;
        }
        self.high_frame = self.high_frame.max(f);
        slot.bytes[..packet.bytes().len()].copy_from_slice(packet.bytes());
        slot.frame = Some(f);
        self.occupancy += 1;
        self.stats.accepted += 1;
        self.stats.peak_occupancy = self.stats.peak_occupancy.max(self.occupancy);
        Ok(())
    }
    /// Returns byte length, or None on loss (output is cleared). No waiting.
    pub fn pop(&mut self, out: &mut [u8; MAX_DATAGRAM]) -> Result<Option<usize>, Error> {
        let after = self
            .next
            .checked_add(u64::from(self.spec.frames))
            .ok_or(Error::Timeline)?;
        let index = (self.next / u64::from(self.spec.frames) % self.slots.len() as u64) as usize;
        let slot = &mut self.slots[index];
        let result = if slot.frame == Some(self.next) {
            let len = self.spec.packet_bytes();
            out[..len].copy_from_slice(&slot.bytes[..len]);
            slot.frame = None;
            self.occupancy -= 1;
            Some(len)
        } else {
            out.fill(0);
            self.stats.missing += 1;
            None
        };
        self.next = after;
        Ok(result)
    }
    pub fn occupancy(&self) -> usize {
        self.occupancy
    }
    pub fn stats(&self) -> BufferStats {
        self.stats
    }
    pub fn next_frame(&self) -> u64 {
        self.next
    }
}

/// Per-channel loss envelope for a wet return. Hold the last valid sample only
/// during a 5 ms fade, then silence. Recovery ramps from the current gain.
/// Hosts must reset this on a new session and prefill before passing valid data.
#[derive(Clone, Copy, Debug, Default)]
pub struct WetGate {
    gain: f64,
    last: f64,
}
impl WetGate {
    pub fn step(&mut self, sample: Option<f64>) -> f64 {
        match sample.filter(|s| s.is_finite() && s.abs() <= 16.0) {
            Some(x) => {
                self.gain = (self.gain + 1.0 / 240.0).min(1.0);
                self.last = x;
            }
            None => {
                self.gain = (self.gain - 1.0 / 240.0).max(0.0);
            }
        }
        self.gain * self.last
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Diagnostic only: compares source-frame progress with receiver monotonic time.
/// Network arrival variation contaminates this estimate; it never steers PA audio.
#[derive(Clone, Copy, Debug, Default)]
pub struct DriftObserver {
    first: Option<(u64, u64)>,
    last: Option<(u64, u64)>,
}
impl DriftObserver {
    pub fn observe(&mut self, frame: u64, nanos: u64) -> Result<Option<f64>, Error> {
        if let Some((f, t)) = self.last
            && (frame <= f || nanos <= t)
        {
            return Err(Error::Timeline);
        }
        let (f, t) = *self.first.get_or_insert((frame, nanos));
        self.last = Some((frame, nanos));
        if nanos - t < 1_000_000_000 {
            return Ok(None);
        }
        Ok(Some(
            ((frame - f) as f64 / ((nanos - t) as f64 * 48_000.0 / 1e9) - 1.0) * 1e6,
        ))
    }
}
