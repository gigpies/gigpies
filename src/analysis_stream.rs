//! C-ANALYSIS:1 named raw input tap. Construction/control are outside callbacks.
use crate::transport::{Encoding, Packet, Role, StreamSpec};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};
pub const PACKET_BYTES: usize = 624;
pub const WINDOW_BYTES: usize = PACKET_BYTES * 10;
pub const MAX_SUBSCRIBERS: usize = 8;
pub const WIRE_BYTES: usize = WINDOW_BYTES + 40;
/// Eight PCM24 feeds fit the existing 1200-byte GPA1 packet budget.
pub const MAX_SOURCES: usize = 8;
pub const MAX_WINDOW_BYTES: usize = (48 + MAX_SOURCES * 144) * 10;
pub const MAX_WIRE_BYTES: usize = MAX_WINDOW_BYTES + 40;
/// Explicit startup binding for legacy four-role or extended named analysis.
/// Input IDs identify raw logical strips, never capture slots or screen banks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    pub inputs: Vec<String>,
}
impl Mapping {
    pub fn descriptor(
        &self,
        topology: &crate::topology::EngineTopology,
        epoch: u64,
        frame: u64,
    ) -> Result<Descriptor, String> {
        topology.validate(crate::topology::ResourceBudget::default())?;
        if !matches!(self.version, 1 | 2) {
            return Err("analysis mapping version".into());
        }
        let mut d = Descriptor::new(epoch, frame);
        d.version = 2;
        d.subscription = "lux.aux.v2".into();
        d.inputs = self.inputs.clone();
        if self.version == 2 {
            d.version = 3;
            d.subscription = "lux.aux.v3".into();
            d.sources = self.sources.clone();
        } else if !self.sources.is_empty() {
            return Err("legacy mapping sources".into());
        }
        d.map_revision = crate::show::Counter(topology.map_revision);
        d.calibration_revision = crate::show::Counter(1);
        d.validate().map_err(String::from)?;
        if d.inputs
            .iter()
            .any(|id| !topology.inputs.iter().any(|p| &p.id == id))
        {
            return Err("analysis input absent from configured topology".into());
        }
        Ok(d)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub contract: String,
    pub version: u32,
    pub subscription: String,
    pub source_epoch: crate::show::Counter,
    pub stream: u32,
    pub sample_rate: u32,
    pub first_frame: crate::show::Counter,
    pub sources: Vec<String>,
    pub inputs: Vec<String>,
    pub tap: String,
    pub clock: String,
    pub map_revision: crate::show::Counter,
    pub calibration_revision: crate::show::Counter,
}
impl Descriptor {
    pub fn new(epoch: u64, frame: u64) -> Self {
        Self {
            contract: "C-ANALYSIS".into(),
            version: 1,
            subscription: "lux.aux.v1".into(),
            source_epoch: crate::show::Counter(epoch),
            stream: 3,
            sample_rate: 48000,
            first_frame: crate::show::Counter(frame),
            sources: ["kick", "bass", "guitar-1", "guitar-2"]
                .map(String::from)
                .to_vec(),
            inputs: ["input-01", "input-02", "input-03", "input-04"]
                .map(String::from)
                .to_vec(),
            tap: "raw-pre-fader".into(),
            clock: "linux-clock-monotonic-ms".into(),
            map_revision: crate::show::Counter(1),
            calibration_revision: crate::show::Counter(2),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contract != "C-ANALYSIS"
            || !matches!(
                (self.version, self.subscription.as_str()),
                (1, "lux.aux.v1") | (2, "lux.aux.v2") | (3, "lux.aux.v3")
            )
            || self.source_epoch.0 == 0
            || self.stream != 3
            || self.sample_rate != 48000
            || !self.first_frame.0.is_multiple_of(48)
            || self.sources.len() != self.inputs.len()
            || !(4..=MAX_SOURCES).contains(&self.inputs.len())
            || (self.version < 3 && self.inputs.len() != 4)
            || self.sources[..4] != ["kick", "bass", "guitar-1", "guitar-2"]
            || self.sources.iter().enumerate().any(|(i, s)| {
                s.is_empty()
                    || s.len() > 32
                    || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || self.sources[..i].contains(s)
            })
            || self.clock != "linux-clock-monotonic-ms"
            || self.tap != "raw-pre-fader"
            || self.map_revision.0 == 0
            || self.calibration_revision.0 == 0
        {
            return Err("descriptor identity/rate");
        }
        for (i, input) in self.inputs.iter().enumerate() {
            let index = input
                .strip_prefix("input-")
                .and_then(|n| n.parse::<u16>().ok());
            if !index.is_some_and(|n| {
                n > 0 && (self.version >= 2 || n <= 8) && input == &format!("input-{n:02}")
            }) || self.inputs[..i].contains(input)
            {
                return Err("explicit input mapping");
            }
        }
        Ok(())
    }
    pub fn attach_request(&self) -> &'static [u8] {
        if self.version == 3 {
            br#"{"subscription":"lux.aux.v3","version":3}"#
        } else if self.version == 2 {
            br#"{"subscription":"lux.aux.v2","version":2}"#
        } else {
            br#"{"subscription":"lux.aux.v1","version":1}"#
        }
    }
    pub fn spec(&self) -> StreamSpec {
        StreamSpec {
            session: self.source_epoch.0,
            stream: 3,
            first_channel: 0,
            channels: self.inputs.len() as u16,
            frames: 48,
            encoding: Encoding::Pcm24,
            role: Role::Analysis,
            delay_frames: 0,
        }
    }
}
#[derive(Clone, Copy)]
pub struct PacketBuffer {
    bytes: [u8; MAX_WINDOW_BYTES],
    length: usize,
}
impl PacketBuffer {
    fn new(length: usize) -> Self {
        Self {
            bytes: [0; MAX_WINDOW_BYTES],
            length,
        }
    }
}
impl std::ops::Deref for PacketBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}
impl std::ops::DerefMut for PacketBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.bytes[..self.length]
    }
}
#[derive(Clone, Copy)]
pub struct Window {
    pub packets: PacketBuffer,
    pub produced_mono_ms: u64,
}
impl Window {
    pub fn validate(
        &self,
        descriptor: &Descriptor,
        expected_frame: u64,
        expected_sequence: u32,
    ) -> Result<(), &'static str> {
        descriptor.validate()?;
        if self.packets.len() != descriptor.spec().packet_bytes() * 10 {
            return Err("window packet shape");
        }
        for (i, bytes) in self
            .packets
            .chunks_exact(descriptor.spec().packet_bytes())
            .enumerate()
        {
            let p = Packet::parse(bytes).map_err(|_| "packet")?;
            if p.spec() != descriptor.spec()
                || p.source_frame()
                    != expected_frame
                        .checked_add(i as u64 * 48)
                        .ok_or("frame overflow")?
                || p.sequence()
                    != expected_sequence
                        .checked_add(i as u32)
                        .ok_or("sequence overflow")?
            {
                return Err("window identity/timeline");
            }
        }
        Ok(())
    }
    pub fn wire(&self, losses: u64, map: u64, calibration: u64) -> Vec<u8> {
        let mut out = vec![0; self.packets.len() + 40];
        out[..4].copy_from_slice(b"GAW1");
        out[4..12].copy_from_slice(&losses.to_be_bytes());
        out[12..20].copy_from_slice(&map.to_be_bytes());
        out[20..28].copy_from_slice(&calibration.to_be_bytes());
        out[28..36].copy_from_slice(&self.produced_mono_ms.to_be_bytes());
        out[40..].copy_from_slice(&self.packets);
        out
    }
}
/// Fixed-storage raw tap; offer does no allocation, locks, I/O or retries.
pub struct Tap {
    spec: StreamSpec,
    mapping: Vec<usize>,
    frame: u64,
    sequence: u32,
    count: usize,
    last_mono_ms: Option<u64>,
    window: Window,
    output: Producer<Window>,
    pub dropped_windows: u64,
}
impl Tap {
    pub fn new(d: &Descriptor) -> Result<(Self, Consumer<Window>), &'static str> {
        d.validate()?;
        let mapping = d
            .inputs
            .iter()
            .map(|id| id[6..].parse::<usize>().unwrap() - 1)
            .collect();
        let (output, consumer) = RingBuffer::new(2);
        Ok((
            Self {
                spec: d.spec(),
                mapping,
                frame: d.first_frame.0,
                sequence: 0,
                count: 0,
                last_mono_ms: None,
                window: Window {
                    packets: PacketBuffer::new(d.spec().packet_bytes() * 10),
                    produced_mono_ms: 0,
                },
                output,
                dropped_windows: 0,
            },
            consumer,
        ))
    }
    pub fn offer(
        &mut self,
        frame: u64,
        raw: &[[i32; 8]; 48],
        produced_mono_ms: u64,
    ) -> Result<(), &'static str> {
        self.offer_interleaved(frame, raw.as_flattened(), 8, produced_mono_ms)
    }
    pub fn offer_interleaved(
        &mut self,
        frame: u64,
        raw: &[i32],
        inputs: usize,
        produced_mono_ms: u64,
    ) -> Result<(), &'static str> {
        if inputs == 0 || raw.len() != inputs * 48 || self.mapping.iter().any(|&i| i >= inputs) {
            return Err("tap shape");
        }
        if frame != self.frame {
            self.count = 0;
            return Err("tap timeline");
        }
        if self
            .last_mono_ms
            .is_some_and(|previous| produced_mono_ms < previous)
        {
            self.count = 0;
            return Err("tap clock regression");
        }
        let next_frame = frame.checked_add(48).ok_or("frame exhausted")?;
        let next_sequence = self.sequence.checked_add(1).ok_or("sequence exhausted")?;
        let mut samples = [0; MAX_SOURCES * 48];
        for (f, inputs) in raw.chunks_exact(inputs).enumerate() {
            for (c, &input) in self.mapping.iter().enumerate() {
                samples[f * self.mapping.len() + c] = inputs[input];
            }
        }
        self.spec
            .encode_pcm(
                frame,
                self.sequence,
                &samples[..self.mapping.len() * 48],
                &mut self.window.packets[self.count * self.spec.packet_bytes()
                    ..(self.count + 1) * self.spec.packet_bytes()],
            )
            .map_err(|_| "PCM24")?;
        if self.count == 0 {
            self.window.produced_mono_ms = produced_mono_ms;
        }
        self.last_mono_ms = Some(produced_mono_ms);
        self.frame = next_frame;
        self.sequence = next_sequence;
        self.count += 1;
        if self.count == 10 {
            if self.output.push(self.window).is_err() {
                self.dropped_windows = self.dropped_windows.saturating_add(1);
            }
            self.count = 0;
        }
        Ok(())
    }
}
/// Invented exact signed PCM24 inputs, shared by synthetic mixer and raw tap.
pub fn synthetic_inputs(frame: u64) -> [[i32; 8]; 48] {
    std::array::from_fn(|f| {
        std::array::from_fn(|c| {
            let n = ((frame % 4096 + f as u64) % 4096) as i32;
            (n - 2048) * (c as i32 + 1) * 32
        })
    })
}
#[cfg(target_os = "linux")]
pub mod local;

/// Validates an attachment's windows without interpreting missing audio as silence.
/// On any refusal discard this cursor and reattach/recalibrate before use.
pub struct WindowCursor {
    descriptor: Descriptor,
    next: Option<(u64, u32)>,
    losses: u64,
    invalid: bool,
    last_now: Option<u64>,
    last_produced: Option<u64>,
}
impl WindowCursor {
    pub fn new(descriptor: Descriptor) -> Result<Self, &'static str> {
        descriptor.validate()?;
        Ok(Self {
            descriptor,
            next: None,
            losses: 0,
            invalid: false,
            last_now: None,
            last_produced: None,
        })
    }
    pub fn accept(&mut self, bytes: &[u8], now_mono_ms: u64) -> Result<Window, &'static str> {
        let result = self.check(bytes, now_mono_ms);
        if result.is_err() {
            self.invalid = true;
        }
        result
    }
    fn check(&mut self, bytes: &[u8], now: u64) -> Result<Window, &'static str> {
        if self.invalid
            || bytes.len() != self.descriptor.spec().packet_bytes() * 10 + 40
            || &bytes[..4] != b"GAW1"
            || bytes[36..40] != [0; 4]
        {
            return Err("window framing/invalid cursor");
        }
        let n = |i| u64::from_be_bytes(bytes[i..i + 8].try_into().unwrap());
        if self.last_now.is_some_and(|previous| now < previous) {
            return Err("receipt clock regression");
        }
        let losses = n(4);
        let produced = n(28);
        if self
            .last_produced
            .is_some_and(|previous| produced < previous)
        {
            return Err("source clock regression");
        }
        if n(12) != self.descriptor.map_revision.0
            || n(20) != self.descriptor.calibration_revision.0
            || produced > now
            || now - produced > 100
            || losses < self.losses
        {
            return Err("window map/calibration/age/loss");
        }
        let mut w = Window {
            packets: PacketBuffer::new(self.descriptor.spec().packet_bytes() * 10),
            produced_mono_ms: produced,
        };
        w.packets.copy_from_slice(&bytes[40..]);
        let first = Packet::parse(&w.packets[..self.descriptor.spec().packet_bytes()])
            .map_err(|_| "packet")?;
        let frame = first.source_frame();
        let sequence = first.sequence();
        let offset = frame
            .checked_sub(self.descriptor.first_frame.0)
            .ok_or("stale origin")?;
        if offset / 48 != u64::from(sequence) || !offset.is_multiple_of(480) {
            return Err("source origin/sequence");
        }
        if let Some((f, s)) = self.next
            && (frame != f || sequence != s || losses != self.losses)
        {
            return Err("gap/overlap/loss");
        }
        w.validate(&self.descriptor, frame, sequence)?;
        self.next = Some((
            frame.checked_add(480).ok_or("frame exhausted")?,
            sequence.checked_add(10).ok_or("sequence exhausted")?,
        ));
        self.losses = losses;
        self.last_now = Some(now);
        self.last_produced = Some(produced);
        Ok(w)
    }
}

/// Raw selected-input stream separate from the unchanged four-name Lux adapter.
/// Four PCM24 channels per GPA1 packet, including a smaller final group. Each
/// whole 48-frame source block enters a two-block queue atomically or is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawDescriptor {
    pub version: u32,
    pub source_epoch: crate::show::Counter,
    pub first_frame: crate::show::Counter,
    pub map_revision: crate::show::Counter,
    pub inputs: Vec<String>,
    pub tap: String,
}
impl RawDescriptor {
    pub fn new(
        epoch: u64,
        frame: u64,
        map: u64,
        inputs: Vec<String>,
    ) -> Result<Self, &'static str> {
        let d = Self {
            version: 2,
            source_epoch: crate::show::Counter(epoch),
            first_frame: crate::show::Counter(frame),
            map_revision: crate::show::Counter(map),
            inputs,
            tap: "raw-pre-fader".into(),
        };
        d.validate()?;
        Ok(d)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != 2
            || self.source_epoch.0 == 0
            || self.map_revision.0 == 0
            || !self.first_frame.0.is_multiple_of(48)
            || self.tap != "raw-pre-fader"
            || self.inputs.is_empty()
            || self.inputs.len() > 256
        {
            return Err("raw descriptor exceeds GPA1 stream channel domain (256)");
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in &self.inputs {
            if crate::processing_wire::input_index(id).is_err() || !seen.insert(id) {
                return Err("raw input identity");
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub struct RawPacket {
    pub bytes: [u8; PACKET_BYTES],
    pub length: u16,
    pub map_revision: u64,
}
pub struct RawTap {
    mapping: Vec<usize>,
    specs: Vec<StreamSpec>,
    frame: u64,
    sequence: u32,
    map_revision: u64,
    output: Producer<RawPacket>,
    pub dropped_blocks: u64,
}
impl RawTap {
    pub fn new(
        descriptor: &RawDescriptor,
        source_inputs: usize,
    ) -> Result<(Self, Consumer<RawPacket>), &'static str> {
        descriptor.validate()?;
        let mapping: Vec<_> = descriptor
            .inputs
            .iter()
            .map(|s| crate::processing_wire::input_index(s).unwrap())
            .collect();
        if mapping.iter().any(|&i| i >= source_inputs) {
            return Err("raw map outside topology");
        }
        let mut specs = Vec::new();
        for first in (0..mapping.len()).step_by(4) {
            let spec = StreamSpec {
                session: descriptor.source_epoch.0,
                stream: 4,
                first_channel: first as u16,
                channels: (mapping.len() - first).min(4) as u16,
                frames: 48,
                encoding: Encoding::Pcm24,
                role: Role::Analysis,
                delay_frames: 0,
            };
            spec.validate().map_err(|_| "raw stream admission")?;
            specs.push(spec);
        }
        let (output, input) = RingBuffer::new(specs.len() * 2);
        Ok((
            Self {
                mapping,
                specs,
                frame: descriptor.first_frame.0,
                sequence: 0,
                map_revision: descriptor.map_revision.0,
                output,
                dropped_blocks: 0,
            },
            input,
        ))
    }
    pub fn offer(
        &mut self,
        epoch: u64,
        frame: u64,
        raw: &[i32],
        source_inputs: usize,
    ) -> Result<(), &'static str> {
        if source_inputs == 0
            || raw.len() != source_inputs * 48
            || self.mapping.iter().any(|&i| i >= source_inputs)
            || epoch != self.specs[0].session
            || frame != self.frame
        {
            return Err("raw source identity/shape");
        }
        if raw.iter().any(|v| !(-8_388_608..=8_388_607).contains(v)) {
            return Err("raw PCM24");
        }
        let next = frame.checked_add(48).ok_or("raw frame exhausted")?;
        let seq = self
            .sequence
            .checked_add(1)
            .ok_or("raw sequence exhausted")?;
        if self.output.slots() < self.specs.len() {
            self.dropped_blocks = self.dropped_blocks.saturating_add(1);
            self.frame = next;
            self.sequence = seq;
            return Ok(());
        }
        for spec in &self.specs {
            let mut samples = [0; 192];
            let count = usize::from(spec.channels);
            for f in 0..48 {
                for c in 0..count {
                    samples[f * count + c] =
                        raw[f * source_inputs + self.mapping[usize::from(spec.first_channel) + c]];
                }
            }
            let mut packet = RawPacket {
                bytes: [0; PACKET_BYTES],
                length: 0,
                map_revision: self.map_revision,
            };
            packet.length = spec
                .encode_pcm(
                    frame,
                    self.sequence,
                    &samples[..48 * count],
                    &mut packet.bytes,
                )
                .map_err(|_| "raw packet")? as u16;
            if self.output.push(packet).is_err() {
                return Err("raw queue invariant");
            }
        }
        self.frame = next;
        self.sequence = seq;
        Ok(())
    }
}
