use super::Error;

pub const HEADER_BYTES: usize = 48;
/// Fits IPv6's minimum 1280-byte MTU including its 40-byte header and UDP.
pub const MAX_DATAGRAM: usize = 1232;
pub const MAX_SAMPLES: usize = (MAX_DATAGRAM - HEADER_BYTES) / 3;
pub const SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Encoding {
    Pcm24 = 1,
    Float32 = 2,
}
impl Encoding {
    pub const fn bytes(self) -> usize {
        match self {
            Self::Pcm24 => 3,
            Self::Float32 => 4,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Role {
    Analysis = 1,
    FxSend = 2,
    WetReturn = 3,
}

/// Negotiated out of band before accepting audio. One spec per channel group.
/// A session is a fresh nonzero authenticated media identity, not a device epoch.
/// Reconnect or source/map replacement must negotiate a fresh session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamSpec {
    pub session: u64,
    pub stream: u32,
    pub first_channel: u16,
    pub channels: u16,
    pub frames: u16,
    pub encoding: Encoding,
    pub role: Role,
    /// Only WetReturn uses this: intended output = source frame + delay.
    pub delay_frames: u32,
}
impl StreamSpec {
    pub fn validate(self) -> Result<(), Error> {
        if self.session == 0
            || self.stream == 0
            || self.channels == 0
            || !matches!(self.frames, 48 | 96)
            || u32::from(self.first_channel) + u32::from(self.channels) > 256
            || (self.role != Role::WetReturn && self.delay_frames != 0)
            || (self.role == Role::WetReturn && !(48..=1536).contains(&self.delay_frames))
        {
            return Err(Error::Format);
        }
        if self.packet_bytes() > MAX_DATAGRAM {
            return Err(Error::Size);
        }
        Ok(())
    }
    pub fn samples(self) -> usize {
        usize::from(self.channels) * usize::from(self.frames)
    }
    pub fn packet_bytes(self) -> usize {
        HEADER_BYTES + self.samples() * self.encoding.bytes()
    }
    pub fn header(self, source_frame: u64, sequence: u32, out: &mut [u8]) -> Result<usize, Error> {
        self.validate()?;
        if out.len() < self.packet_bytes() {
            return Err(Error::Size);
        }
        if !source_frame.is_multiple_of(u64::from(self.frames))
            || source_frame
                .checked_add(u64::from(self.frames) + u64::from(self.delay_frames))
                .is_none()
        {
            return Err(Error::Timeline);
        }
        out[..HEADER_BYTES].fill(0);
        out[..4].copy_from_slice(b"GPA1");
        out[4] = self.encoding as u8;
        out[5] = self.role as u8;
        out[6..8].copy_from_slice(&self.channels.to_be_bytes());
        out[8..10].copy_from_slice(&self.frames.to_be_bytes());
        out[10..12].copy_from_slice(&self.first_channel.to_be_bytes());
        out[12..16].copy_from_slice(&self.stream.to_be_bytes());
        out[16..24].copy_from_slice(&self.session.to_be_bytes());
        out[24..32].copy_from_slice(&source_frame.to_be_bytes());
        out[32..36].copy_from_slice(&sequence.to_be_bytes());
        out[36..40].copy_from_slice(&SAMPLE_RATE.to_be_bytes());
        out[40..44].copy_from_slice(&self.delay_frames.to_be_bytes());
        Ok(self.packet_bytes())
    }
    /// Exact packed integer path. Rejects rather than silently clipping.
    pub fn encode_pcm(
        self,
        frame: u64,
        sequence: u32,
        samples: &[i32],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        if self.encoding != Encoding::Pcm24 || samples.len() != self.samples() {
            return Err(Error::Size);
        }
        if samples
            .iter()
            .any(|x| !(-8_388_608..=8_388_607).contains(x))
        {
            return Err(Error::Sample);
        }
        let len = self.header(frame, sequence, out)?;
        for (bytes, sample) in out[HEADER_BYTES..len].chunks_exact_mut(3).zip(samples) {
            bytes.copy_from_slice(&sample.to_be_bytes()[1..]);
        }
        Ok(len)
    }
    /// Float wire values retain FX headroom, bounded to +/-16 and finite.
    pub fn encode_float(
        self,
        frame: u64,
        sequence: u32,
        samples: &[f32],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        if self.encoding != Encoding::Float32 || samples.len() != self.samples() {
            return Err(Error::Size);
        }
        if samples.iter().any(|x| !x.is_finite() || x.abs() > 16.0) {
            return Err(Error::Sample);
        }
        let len = self.header(frame, sequence, out)?;
        for (bytes, sample) in out[HEADER_BYTES..len].chunks_exact_mut(4).zip(samples) {
            bytes.copy_from_slice(&sample.to_be_bytes());
        }
        Ok(len)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Packet<'a> {
    spec: StreamSpec,
    source_frame: u64,
    sequence: u32,
    bytes: &'a [u8],
}
impl<'a> Packet<'a> {
    pub fn spec(self) -> StreamSpec {
        self.spec
    }
    pub fn source_frame(self) -> u64 {
        self.source_frame
    }
    pub fn sequence(self) -> u32 {
        self.sequence
    }

    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if !(HEADER_BYTES..=MAX_DATAGRAM).contains(&bytes.len()) {
            return Err(Error::Size);
        }
        if &bytes[..4] != b"GPA1" || bytes[44..48] != [0; 4] {
            return Err(Error::Format);
        }
        let u16_at = |i| u16::from_be_bytes(bytes[i..i + 2].try_into().unwrap());
        let u32_at = |i| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
        let u64_at = |i| u64::from_be_bytes(bytes[i..i + 8].try_into().unwrap());
        if u32_at(36) != SAMPLE_RATE {
            return Err(Error::Format);
        }
        let spec = StreamSpec {
            session: u64_at(16),
            stream: u32_at(12),
            first_channel: u16_at(10),
            channels: u16_at(6),
            frames: u16_at(8),
            delay_frames: u32_at(40),
            encoding: match bytes[4] {
                1 => Encoding::Pcm24,
                2 => Encoding::Float32,
                _ => return Err(Error::Format),
            },
            role: match bytes[5] {
                1 => Role::Analysis,
                2 => Role::FxSend,
                3 => Role::WetReturn,
                _ => return Err(Error::Format),
            },
        };
        spec.validate()?;
        if bytes.len() != spec.packet_bytes() {
            return Err(Error::Size);
        }
        let source_frame = u64_at(24);
        if !source_frame.is_multiple_of(u64::from(spec.frames))
            || source_frame
                .checked_add(u64::from(spec.frames) + u64::from(spec.delay_frames))
                .is_none()
        {
            return Err(Error::Timeline);
        }
        if spec.encoding == Encoding::Float32
            && bytes[HEADER_BYTES..].chunks_exact(4).any(|b| {
                let x = f32::from_be_bytes(b.try_into().unwrap());
                !x.is_finite() || x.abs() > 16.0
            })
        {
            return Err(Error::Sample);
        }
        Ok(Self {
            spec,
            source_frame,
            sequence: u32_at(32),
            bytes,
        })
    }
    pub fn bytes(self) -> &'a [u8] {
        self.bytes
    }
    pub fn output_frame(self) -> u64 {
        self.source_frame + u64::from(self.spec.delay_frames)
    }
    pub fn pcm(self, index: usize) -> Result<i32, Error> {
        if self.spec.encoding != Encoding::Pcm24 || index >= self.spec.samples() {
            return Err(Error::Size);
        }
        let i = HEADER_BYTES + index * 3;
        Ok(i32::from_be_bytes([
            if self.bytes[i] & 128 != 0 { 255 } else { 0 },
            self.bytes[i],
            self.bytes[i + 1],
            self.bytes[i + 2],
        ]))
    }
    pub fn sample(self, index: usize) -> Result<f64, Error> {
        if index >= self.spec.samples() {
            return Err(Error::Size);
        }
        match self.spec.encoding {
            Encoding::Pcm24 => Ok(f64::from(self.pcm(index)?) / 8_388_608.0),
            Encoding::Float32 => {
                let i = HEADER_BYTES + index * 4;
                Ok(f64::from(f32::from_be_bytes(
                    self.bytes[i..i + 4].try_into().unwrap(),
                )))
            }
        }
    }
}

/// Validate an explicitly agreed complete session map before socket admission.
/// Every role uses dense channel indices, stream IDs are globally unique, and
/// send/return channel counts match. No discovery or remote allocation occurs.
pub fn validate_session(specs: &[StreamSpec]) -> Result<(), Error> {
    if specs.is_empty() || specs.len() > 64 {
        return Err(Error::Capacity);
    }
    let mut channels = [[false; 256]; 3];
    for (i, &spec) in specs.iter().enumerate() {
        spec.validate()?;
        if spec.session != specs[0].session
            || spec.frames != specs[0].frames
            || specs[..i].iter().any(|prior| prior.stream == spec.stream)
        {
            return Err(Error::Identity);
        }
        let role = spec.role as usize - 1;
        for channel in spec.first_channel..spec.first_channel + spec.channels {
            if channels[role][usize::from(channel)] {
                return Err(Error::Identity);
            }
            channels[role][usize::from(channel)] = true;
        }
    }
    let mut counts = [0; 3];
    for (i, role) in channels.iter().enumerate() {
        counts[i] = role.iter().filter(|&&x| x).count();
        if role[..counts[i]].iter().any(|&x| !x) {
            return Err(Error::Identity);
        }
    }
    if counts[1] != counts[2] {
        return Err(Error::Identity);
    }
    Ok(())
}
