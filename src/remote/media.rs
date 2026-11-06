use super::{AuthenticatedContext, EngineIdentity, Permission, Result};
use crate::{
    show::Counter,
    transport::{self, Encoding, Packet, Role, StreamSpec},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaRole {
    Analysis,
    FxSend,
    WetReturn,
}
impl MediaRole {
    pub fn packet_role(self) -> Role {
        match self {
            Self::Analysis => Role::Analysis,
            Self::FxSend => Role::FxSend,
            Self::WetReturn => Role::WetReturn,
        }
    }
    fn permission(self) -> Permission {
        if self == Self::Analysis {
            Permission::Analysis
        } else {
            Permission::Fx
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaEncoding {
    Pcm24,
    Float32,
}
impl MediaEncoding {
    fn packet_encoding(self) -> Encoding {
        match self {
            Self::Pcm24 => Encoding::Pcm24,
            Self::Float32 => Encoding::Float32,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaStream {
    pub stream: u32,
    pub role: MediaRole,
    pub first_channel: u16,
    pub channel_ids: Vec<String>,
    pub frames: u16,
    pub encoding: MediaEncoding,
    pub delay_frames: u32,
}
impl MediaStream {
    pub fn spec(&self, session: u64) -> Result<StreamSpec> {
        let spec = StreamSpec {
            session,
            stream: self.stream,
            first_channel: self.first_channel,
            channels: u16::try_from(self.channel_ids.len()).map_err(|_| "media channel count")?,
            frames: self.frames,
            encoding: self.encoding.packet_encoding(),
            role: self.role.packet_role(),
            delay_frames: self.delay_frames,
        };
        spec.validate().map_err(|e| format!("media spec: {e:?}"))?;
        Ok(spec)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaDescriptor {
    pub session: Counter,
    pub source_epoch: Counter,
    pub capability_generation: Counter,
    pub map_generation: Counter,
    pub sample_rate: u32,
    pub streams: Vec<MediaStream>,
}
impl MediaDescriptor {
    pub fn validate(
        &self,
        context: &AuthenticatedContext,
        identity: &EngineIdentity,
        max_datagram: usize,
    ) -> Result<()> {
        context.check_current()?;
        for stream in &self.streams {
            context.require(&stream.role.permission())?;
        }
        self.validate_shape(context.session, identity, max_datagram)
    }
    pub(crate) fn validate_shape(
        &self,
        session: u64,
        identity: &EngineIdentity,
        max_datagram: usize,
    ) -> Result<()> {
        identity.validate()?;
        if self.session.0 != session
            || self.source_epoch != identity.source_epoch
            || self.capability_generation != identity.capability_generation
            || self.map_generation != identity.map_generation
            || self.sample_rate != transport::SAMPLE_RATE
            || self.streams.is_empty()
            || self.streams.len() > 64
        {
            return Err("media identity or capacity".into());
        }
        let mut specs = Vec::with_capacity(self.streams.len());
        let mut names = BTreeSet::new();
        for stream in &self.streams {
            if (stream.role == MediaRole::Analysis) != (stream.encoding == MediaEncoding::Pcm24) {
                return Err("media role encoding".into());
            }
            let spec = stream.spec(self.session.0)?;
            if spec.packet_bytes() > max_datagram {
                return Err("media QUIC datagram capacity".into());
            }
            for id in &stream.channel_ids {
                if !crate::show::id(id) || !names.insert((stream.role, id)) {
                    return Err("media channel identity".into());
                }
            }
            specs.push(spec);
        }
        transport::validate_session(&specs).map_err(|e| format!("media session: {e:?}"))
    }
    pub fn channels(&self, role: MediaRole) -> usize {
        self.streams
            .iter()
            .filter(|s| s.role == role)
            .map(|s| s.channel_ids.len())
            .sum()
    }
}
/// Computes groups against the QUIC payload limit, not the raw UDP maximum.
/// 256 selected channels and 64 total groups are explicit GPA1 transport bounds;
/// they do not constrain the engine's admitted channel count.
pub fn grouped_streams(
    role: MediaRole,
    channel_ids: &[String],
    frames: u16,
    delay_frames: u32,
    first_stream: u32,
    max_datagram: usize,
) -> Result<Vec<MediaStream>> {
    if channel_ids.is_empty()
        || channel_ids.len() > 256
        || !matches!(frames, 48 | 96)
        || first_stream == 0
    {
        return Err("media grouping capacity".into());
    }
    let encoding = if role == MediaRole::Analysis {
        MediaEncoding::Pcm24
    } else {
        MediaEncoding::Float32
    };
    let capacity = max_datagram
        .min(transport::MAX_DATAGRAM)
        .checked_sub(transport::HEADER_BYTES)
        .ok_or("media datagram too small")?
        / (usize::from(frames) * encoding.packet_encoding().bytes());
    if capacity == 0 {
        return Err("media datagram too small".into());
    }
    let mut out = Vec::new();
    for (group, ids) in channel_ids.chunks(capacity).enumerate() {
        let stream = MediaStream {
            stream: first_stream
                .checked_add(group as u32)
                .ok_or("media stream ID exhaustion")?,
            role,
            first_channel: u16::try_from(group * capacity).map_err(|_| "media channel index")?,
            channel_ids: ids.to_vec(),
            frames,
            encoding,
            delay_frames,
        };
        stream.spec(1)?;
        out.push(stream);
    }
    if out.len() > 64 {
        return Err("media group capacity".into());
    }
    Ok(out)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSide {
    ProcessingNode,
    Brain,
}
impl MediaSide {
    fn accepts(self, role: Role) -> bool {
        match self {
            Self::ProcessingNode => role == Role::WetReturn,
            Self::Brain => role != Role::WetReturn,
        }
    }
}
#[derive(Debug, Default)]
struct ReplayWindow {
    latest: Option<u64>,
    seen: u64,
}
impl ReplayWindow {
    fn accept(&mut self, frame: u64, frames: u16) -> Result<()> {
        let index = frame / u64::from(frames);
        match self.latest {
            None => {
                self.latest = Some(index);
                self.seen = 1;
            }
            Some(last) if index > last => {
                let distance = index - last;
                self.seen = if distance >= 64 {
                    1
                } else {
                    (self.seen << distance) | 1
                };
                self.latest = Some(index);
            }
            Some(last) => {
                let distance = last - index;
                if distance >= 64 || self.seen & (1 << distance) != 0 {
                    return Err("duplicate or expired media frame".into());
                }
                self.seen |= 1 << distance;
            }
        }
        Ok(())
    }
}
/// Connection-local admission. Replacement cannot resurrect previously used
/// stream identities. Host queue deadlines remain authoritative for playout.
pub struct MediaRegistry {
    descriptor: Option<MediaDescriptor>,
    retired: BTreeSet<u32>,
    windows: BTreeMap<u32, ReplayWindow>,
}
impl Default for MediaRegistry {
    fn default() -> Self {
        Self::new()
    }
}
impl MediaRegistry {
    pub fn new() -> Self {
        Self {
            descriptor: None,
            retired: BTreeSet::new(),
            windows: BTreeMap::new(),
        }
    }
    pub fn descriptor(&self) -> Option<&MediaDescriptor> {
        self.descriptor.as_ref()
    }
    pub fn install(
        &mut self,
        descriptor: MediaDescriptor,
        context: &AuthenticatedContext,
        identity: &EngineIdentity,
        max_datagram: usize,
    ) -> Result<()> {
        descriptor.validate(context, identity, max_datagram)?;
        self.install_validated(descriptor)
    }
    pub(crate) fn install_received(
        &mut self,
        descriptor: MediaDescriptor,
        session: u64,
        identity: &EngineIdentity,
        max_datagram: usize,
    ) -> Result<()> {
        descriptor.validate_shape(session, identity, max_datagram)?;
        self.install_validated(descriptor)
    }
    fn install_validated(&mut self, descriptor: MediaDescriptor) -> Result<()> {
        if self.descriptor.as_ref() == Some(&descriptor) {
            return Ok(());
        }
        self.check_install(&descriptor)?;
        for stream in &descriptor.streams {
            self.retired.insert(stream.stream);
        }
        self.windows = descriptor
            .streams
            .iter()
            .map(|s| (s.stream, ReplayWindow::default()))
            .collect();
        self.descriptor = Some(descriptor);
        Ok(())
    }
    pub fn check_install(&self, descriptor: &MediaDescriptor) -> Result<()> {
        if self.descriptor.as_ref() == Some(descriptor) {
            return Ok(());
        }
        if self.retired.len() + descriptor.streams.len() > 4096 {
            return Err("media history capacity; reconnect required".into());
        }
        if descriptor
            .streams
            .iter()
            .any(|s| self.retired.contains(&s.stream))
        {
            return Err("retired media stream; use fresh IDs".into());
        }
        Ok(())
    }
    pub fn receive<'a>(
        &mut self,
        bytes: &'a [u8],
        side: MediaSide,
        source_cursor: Option<u64>,
    ) -> Result<Packet<'a>> {
        let packet = Packet::parse(bytes).map_err(|e| format!("media packet: {e:?}"))?;
        let descriptor = self.descriptor.as_ref().ok_or("media not negotiated")?;
        let stream = descriptor
            .streams
            .iter()
            .find(|s| s.stream == packet.spec().stream)
            .ok_or("unnegotiated media stream")?;
        if packet.spec() != stream.spec(descriptor.session.0)? || !side.accepts(packet.spec().role)
        {
            return Err("media peer/session/direction mismatch".into());
        }
        if let Some(cursor) = source_cursor {
            if packet.spec().role == Role::WetReturn && packet.output_frame() <= cursor {
                return Err("late wet media".into());
            }
            if packet.source_frame() > cursor.saturating_add(1536) {
                return Err("future media frame".into());
            }
        }
        self.windows
            .get_mut(&stream.stream)
            .ok_or("media replay state")?
            .accept(packet.source_frame(), packet.spec().frames)?;
        Ok(packet)
    }
    pub fn validate_outgoing(&self, bytes: &[u8], side: MediaSide) -> Result<()> {
        let packet = Packet::parse(bytes).map_err(|e| format!("media packet: {e:?}"))?;
        let descriptor = self.descriptor.as_ref().ok_or("media not negotiated")?;
        let stream = descriptor
            .streams
            .iter()
            .find(|s| s.stream == packet.spec().stream)
            .ok_or("unnegotiated media stream")?;
        if packet.spec() != stream.spec(descriptor.session.0)? || side.accepts(packet.spec().role) {
            return Err("outgoing media direction/session".into());
        }
        Ok(())
    }
}
/// Worker-side grouping of frame-major samples. The source identity and frame are
/// preserved; this performs no clock progression and no FX algorithm itself.
pub fn encode_grouped(
    descriptor: &MediaDescriptor,
    role: MediaRole,
    frame: u64,
    samples: &[f64],
) -> Result<Vec<Vec<u8>>> {
    let specs = descriptor
        .streams
        .iter()
        .map(|s| s.spec(descriptor.session.0))
        .collect::<Result<Vec<_>>>()?;
    transport::validate_session(&specs).map_err(|e| format!("group descriptor {e:?}"))?;
    let channels = descriptor.channels(role);
    let streams: Vec<_> = descriptor
        .streams
        .iter()
        .filter(|s| s.role == role)
        .collect();
    let frames = usize::from(streams.first().ok_or("media role unavailable")?.frames);
    if samples.len()
        != frames
            .checked_mul(channels)
            .ok_or("media sample capacity")?
    {
        return Err("media sample dimensions".into());
    }
    let mut packets = Vec::with_capacity(streams.len());
    for stream in streams {
        let spec = stream.spec(descriptor.session.0)?;
        let mut packet = vec![0; spec.packet_bytes()];
        let mut group = Vec::with_capacity(spec.samples());
        for f in 0..frames {
            let first = f * channels + usize::from(spec.first_channel);
            group.extend_from_slice(&samples[first..first + usize::from(spec.channels)]);
        }
        let sequence = (frame / frames as u64) as u32;
        match spec.encoding {
            Encoding::Pcm24 => {
                let mut pcm = Vec::with_capacity(group.len());
                for sample in group {
                    let value = sample * 8_388_608.;
                    if !value.is_finite()
                        || !(-8_388_608. ..=8_388_607.).contains(&value)
                        || value.fract() != 0.
                    {
                        return Err("raw analysis must preserve exact PCM24".into());
                    }
                    pcm.push(value as i32);
                }
                spec.encode_pcm(frame, sequence, &pcm, &mut packet)
                    .map_err(|e| format!("media encode: {e:?}"))?;
            }
            Encoding::Float32 => {
                let values: Vec<f32> = group.into_iter().map(|v| v as f32).collect();
                spec.encode_float(frame, sequence, &values, &mut packet)
                    .map_err(|e| format!("media encode: {e:?}"))?;
            }
        }
        packets.push(packet);
    }
    Ok(packets)
}
