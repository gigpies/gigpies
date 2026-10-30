//! Brain follows source-indexed complete send blocks. It has no audio device or
//! free-running render clock, and calls the actual SHR FX library through its
//! existing host adapter. All allocation/network operations are worker-side.
use super::*;
use crate::transport::{Packet, Role};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
struct Block {
    samples: Vec<f64>,
    groups: BTreeSet<u32>,
}
pub struct GroupedReceiver {
    descriptor: MediaDescriptor,
    role: MediaRole,
    blocks: BTreeMap<u64, Block>,
    next: Option<u64>,
    pub incomplete_dropped: u64,
}
impl GroupedReceiver {
    pub fn new(descriptor: MediaDescriptor, role: MediaRole) -> Result<Self> {
        let specs = descriptor
            .streams
            .iter()
            .map(|s| s.spec(descriptor.session.0))
            .collect::<Result<Vec<_>>>()?;
        crate::transport::validate_session(&specs)
            .map_err(|e| format!("group descriptor {e:?}"))?;
        let channels = descriptor.channels(role);
        if channels == 0 || channels > 256 || descriptor.streams.len() > 64 {
            return Err("group receiver capacity".into());
        }
        Ok(Self {
            descriptor,
            role,
            blocks: BTreeMap::new(),
            next: None,
            incomplete_dropped: 0,
        })
    }
    pub fn push(&mut self, packet: Packet<'_>) -> Result<Option<(u64, Vec<f64>)>> {
        let stream = self
            .descriptor
            .streams
            .iter()
            .find(|s| s.stream == packet.spec().stream && s.role == self.role)
            .ok_or("group stream")?;
        if stream.spec(self.descriptor.session.0)? != packet.spec()
            || self.next.is_some_and(|n| packet.source_frame() < n)
        {
            return Err("stale or mismatched group".into());
        }
        if self.blocks.len() == 32 && !self.blocks.contains_key(&packet.source_frame()) {
            self.blocks.pop_first();
            self.incomplete_dropped = self.incomplete_dropped.saturating_add(1);
        }
        let channels = self.descriptor.channels(self.role);
        let block = self
            .blocks
            .entry(packet.source_frame())
            .or_insert_with(|| Block {
                samples: vec![0.; channels * usize::from(packet.spec().frames)],
                groups: BTreeSet::new(),
            });
        if !block.groups.insert(packet.spec().stream) {
            return Err("duplicate group".into());
        }
        for frame in 0..usize::from(packet.spec().frames) {
            for channel in 0..usize::from(packet.spec().channels) {
                block.samples
                    [frame * channels + usize::from(packet.spec().first_channel) + channel] =
                    packet
                        .sample(frame * usize::from(packet.spec().channels) + channel)
                        .map_err(|e| format!("group sample {e:?}"))?;
            }
        }
        let expected = self
            .descriptor
            .streams
            .iter()
            .filter(|s| s.role == self.role)
            .count();
        let complete = self
            .blocks
            .iter()
            .find(|(_, block)| block.groups.len() == expected)
            .map(|(frame, _)| *frame);
        let Some(frame) = complete else {
            return Ok(None);
        };
        let block = self.blocks.remove(&frame).ok_or("group completion")?;
        while self
            .blocks
            .first_key_value()
            .is_some_and(|(old, _)| *old < frame)
        {
            self.blocks.pop_first();
            self.incomplete_dropped = self.incomplete_dropped.saturating_add(1);
        }
        self.next = Some(
            frame
                .checked_add(u64::from(packet.spec().frames))
                .ok_or("source frame exhausted")?,
        );
        Ok(Some((frame, block.samples)))
    }
}
#[derive(Debug, Default, Clone)]
pub struct BrainStats {
    pub analysis_packets: u64,
    pub send_packets: u64,
    pub processed_blocks: u64,
    pub returned_packets: u64,
    pub stale_groups: u64,
    pub incomplete_blocks: u64,
}
/// A caller may consume analysis packets through `step` while FX remains backed
/// by the independent owner. Analysis consumers cannot mutate the source cursor.
struct FxTicket {
    ticket: u64,
    apply_frame: u64,
    deadline: std::time::Instant,
    permitted: bool,
    applied: bool,
    panic_mask: Option<u32>,
}
pub struct BrainWorker {
    fx_binding: Option<crate::fx_wire::Binding>,
    fx_ticket: Option<FxTicket>,
    fx_refused: Option<(u64, &'static str)>,
    media: MediaChannel,
    receiver: GroupedReceiver,
    fx: Option<crate::host::brain_fx::BrainFx>,
    aux: Option<super::fx_aux_owner::BrainAux>,
    wet: Vec<f64>,
    pub stats: BrainStats,
    returned_hash: Sha256,
    nonzero_return_samples: u64,
    fx_library_file: Option<std::fs::File>,
}
impl BrainWorker {
    pub fn prepare(media: MediaChannel, fx_library: &Path) -> Result<Self> {
        let descriptor = media.descriptor().clone();
        let channels = descriptor.channels(MediaRole::FxSend);
        let frames = descriptor
            .streams
            .iter()
            .find(|s| s.role == MediaRole::FxSend)
            .ok_or("FX send not negotiated")?
            .frames;
        if descriptor.channels(MediaRole::WetReturn) != channels {
            return Err("FX return dimensions".into());
        }
        let fx = crate::host::brain_fx::BrainFx::prepare(
            fx_library,
            channels,
            usize::from(frames),
            descriptor.source_epoch.0,
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            fx_binding: None,
            fx_ticket: None,
            fx_refused: None,
            media,
            receiver: GroupedReceiver::new(descriptor, MediaRole::FxSend)?,
            fx: Some(fx),
            aux: None,
            wet: vec![0.; usize::from(frames) * channels],
            stats: BrainStats::default(),
            returned_hash: Sha256::new(),
            nonzero_return_samples: 0,
            fx_library_file: None,
        })
    }
    pub fn prepare_aux(
        media: MediaChannel,
        path: &Path,
        hash: &str,
        inputs: usize,
    ) -> Result<Self> {
        let descriptor = media.descriptor().clone();
        let aux = super::fx_aux_owner::BrainAux::load(descriptor.clone(), path, hash, inputs)?;
        Ok(Self {
            fx_binding: None,
            fx_ticket: None,
            fx_refused: None,
            receiver: GroupedReceiver::new(descriptor, MediaRole::FxSend)?,
            media,
            fx: None,
            aux: Some(aux),
            wet: vec![0.; 384],
            stats: BrainStats::default(),
            returned_hash: Sha256::new(),
            nonzero_return_samples: 0,
            fx_library_file: None,
        })
    }
    pub fn aux_command(
        &mut self,
        c: super::fx_aux_owner::Command,
    ) -> Result<Option<super::fx_aux_owner::Message>> {
        self.aux
            .as_mut()
            .ok_or("four aux owner unavailable")?
            .command(c)
    }
    pub fn poll_aux_observation(&mut self) -> Result<Option<super::fx_aux_owner::Message>> {
        self.aux
            .as_mut()
            .ok_or("four aux owner unavailable")?
            .poll()
    }
    pub fn enable_fx_control(&mut self, path: &Path, expected_sha256: &str) -> Result<bool> {
        if self.aux.is_some() {
            return Err("four aux owner excludes legacy delay control".into());
        }
        let d = self.media.descriptor();
        if ![MediaRole::FxSend, MediaRole::WetReturn]
            .iter()
            .all(|role| {
                d.streams
                    .iter()
                    .filter(|s| s.role == *role)
                    .flat_map(|s| s.channel_ids.iter())
                    .map(String::as_str)
                    .eq(["foh-left", "foh-right"])
            })
        {
            return Err("FX control requires ordered FOH pair".into());
        }
        if self.fx_binding.is_some() {
            return Err("FX owner already configured".into());
        }
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != expected_sha256 {
            return Err("FX library hash mismatch".into());
        }
        // On Linux dlopen the same open inode whose bytes were hashed, so a
        // concurrent artifact rename cannot substitute a different owner.
        #[cfg(target_os = "linux")]
        let load_path = {
            use std::os::fd::AsRawFd;
            std::path::PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
        };
        #[cfg(not(target_os = "linux"))]
        let load_path = path.to_path_buf();
        if !self.fx.as_mut().unwrap().enable_configured(&load_path)? {
            return Ok(false);
        }
        self.fx_library_file = Some(file);
        self.fx_binding = Some(crate::fx_wire::Binding {
            session: d.session,
            source_epoch: d.source_epoch,
            capability_generation: d.capability_generation,
            map_generation: d.map_generation,
            owner_instance: crate::show::Counter(1),
            library_sha256: actual,
            abi_version: 2,
            config_size: 104,
            status_size: 168,
            capabilities_size: 128,
            channels: ["foh-left".into(), "foh-right".into()],
        });
        Ok(true)
    }
    pub(crate) fn fx_observation(&mut self) -> Result<crate::fx_wire::Observation> {
        use crate::show::Counter;
        let binding = self.fx_binding.clone().ok_or("FX controls unavailable")?;
        let s = self
            .fx
            .as_mut()
            .unwrap()
            .configured_owner()
            .ok_or("FX owner unavailable")?
            .status()?;
        Ok(crate::fx_wire::Observation {
            binding,
            generation: Counter(s.applied_generation),
            settled_generation: Counter(s.settled_generation),
            applied_source_frame: Counter(s.applied_source_frame),
            settled_source_frame: Counter(s.settled_source_frame),
            next_source_frame: Counter(s.next_source_frame),
            reset_count: Counter(s.reset_count),
            remaining_frames: s.remaining_frames,
            owner_json: serde_json::to_string(&s.configuration()).map_err(|e| e.to_string())?,
        })
    }
    /// Control-worker entry. No token allocation/retirement occurs in process.
    pub fn fx_command(
        &mut self,
        command: crate::fx_wire::OwnerCommand,
    ) -> Result<Option<crate::fx_wire::OwnerMessage>> {
        use crate::fx_wire::{Configuration, OwnerCommand as C, OwnerMessage as M};
        match command {
            C::Prepare {
                ticket,
                mutation,
                apply_frame,
            } => {
                mutation.validate()?;
                if self.fx.as_ref().unwrap().next_source_frame().is_none() {
                    return Ok(Some(M::Refused {
                        ticket,
                        reason: "FX source continuity not initialized".into(),
                    }));
                }
                if self.fx_ticket.is_some() || self.fx_refused.is_some() {
                    return Ok(Some(M::Refused {
                        ticket,
                        reason: "FX owner busy".into(),
                    }));
                }
                let o = self.fx_observation()?;
                if mutation.binding != o.binding
                    || mutation.expected_generation != o.generation
                    || mutation.expected_reset_count != o.reset_count
                    || o.remaining_frames != [0, 0]
                    || apply_frame.0 <= o.next_source_frame.0
                    || !apply_frame.0.is_multiple_of(48)
                    || apply_frame.0 - o.next_source_frame.0 > 48_000
                {
                    return Ok(Some(M::Refused {
                        ticket,
                        reason: "FX stale preparation".into(),
                    }));
                }
                if let Some(text) = &mutation.configuration_json {
                    let config = Configuration::decode(text)?;
                    if let Err(reason) = self
                        .fx
                        .as_mut()
                        .unwrap()
                        .configured_owner()
                        .unwrap()
                        .prepare(&config, o.generation.0)
                    {
                        return Ok(Some(M::Refused { ticket, reason }));
                    }
                }
                self.fx_ticket = Some(FxTicket {
                    ticket: ticket.0,
                    apply_frame: apply_frame.0,
                    deadline: std::time::Instant::now()
                        + std::time::Duration::from_millis(crate::fx_wire::PREPARE_MS),
                    permitted: false,
                    applied: false,
                    panic_mask: mutation.panic_mask,
                });
                Ok(Some(M::Prepared {
                    ticket,
                    binding: o.binding,
                    generation: o.generation,
                    configuration_json: mutation.configuration_json,
                    panic_mask: mutation.panic_mask,
                }))
            }
            C::Permit {
                ticket,
                binding,
                apply_frame,
            } => {
                let next = self.fx.as_ref().unwrap().next_source_frame();
                let p = self
                    .fx_ticket
                    .as_mut()
                    .ok_or("FX permit without preparation")?;
                if p.ticket != ticket.0
                    || self.fx_binding.as_ref() != Some(&binding)
                    || p.apply_frame != apply_frame.0
                    || p.permitted
                    || std::time::Instant::now() >= p.deadline
                    || next.is_some_and(|f| f > apply_frame.0)
                {
                    return Err("FX invalid/late permit".into());
                }
                p.permitted = true;
                Ok(None)
            }
            C::Cancel { ticket } => {
                if self
                    .fx_ticket
                    .as_ref()
                    .is_some_and(|p| p.ticket == ticket.0 && !p.permitted)
                {
                    self.fx_ticket = None;
                    self.fx
                        .as_mut()
                        .unwrap()
                        .configured_owner()
                        .unwrap()
                        .retire();
                }
                Ok(None)
            }
        }
    }
    /// Called after processing or on the control timer. Status/JSON and token
    /// retirement are deliberately outside the owner process/commit boundary.
    pub fn poll_fx_observation(&mut self) -> Result<Option<crate::fx_wire::OwnerMessage>> {
        use crate::{fx_wire::OwnerMessage as M, show::Counter};
        if self.fx_binding.is_none() {
            return Ok(None);
        }
        if let Some((ticket, reason)) = self.fx_refused.take() {
            self.fx
                .as_mut()
                .unwrap()
                .configured_owner()
                .unwrap()
                .retire();
            return Ok(Some(M::Refused {
                ticket: Counter(ticket),
                reason: reason.into(),
            }));
        }
        if self
            .fx_ticket
            .as_ref()
            .is_some_and(|p| !p.applied && std::time::Instant::now() >= p.deadline)
        {
            let p = self.fx_ticket.take().unwrap();
            self.fx
                .as_mut()
                .unwrap()
                .configured_owner()
                .unwrap()
                .retire();
            return Ok(Some(M::Refused {
                ticket: Counter(p.ticket),
                reason: "FX preparation deadline".into(),
            }));
        }
        let observation = self.fx_observation()?;
        if self.fx_ticket.as_ref().is_some_and(|p| p.applied) {
            let p = self.fx_ticket.as_ref().unwrap();
            let ticket = Counter(p.ticket);
            let effective_source_frame = Counter(p.apply_frame);
            let panic_mask = p.panic_mask;
            if observation.remaining_frames == [0, 0]
                && observation.generation == observation.settled_generation
            {
                self.fx_ticket = None;
                self.fx
                    .as_mut()
                    .unwrap()
                    .configured_owner()
                    .unwrap()
                    .retire();
            }
            return Ok(Some(M::Completed {
                ticket,
                effective_source_frame,
                panic_mask,
                observation,
            }));
        }
        Ok(Some(M::Observe { observation }))
    }
    fn fx_boundary(&mut self, frame: u64, input: &[f64]) {
        let Some(p) = self.fx_ticket.as_mut() else {
            return;
        };
        if p.applied {
            return;
        }
        let gap = self
            .fx
            .as_ref()
            .unwrap()
            .next_source_frame()
            .is_some_and(|next| next != frame);
        if gap
            || frame > p.apply_frame
            || std::time::Instant::now() >= p.deadline
            || input.iter().any(|s| !s.is_finite() || s.abs() > 16.)
        {
            self.fx_refused = Some((p.ticket, "FX source gap/late boundary"));
            self.fx_ticket = None;
            return;
        }
        if frame != p.apply_frame {
            return;
        }
        if !p.permitted {
            self.fx_refused = Some((p.ticket, "FX permit absent at target"));
            self.fx_ticket = None;
            return;
        }
        let owner = self.fx.as_mut().unwrap().configured_owner().unwrap();
        let rc = if let Some(mask) = p.panic_mask {
            owner.panic(mask)
        } else {
            owner.commit(frame)
        };
        if rc != 0 {
            self.fx_refused = Some((p.ticket, "FX owner commit refused"));
            self.fx_ticket = None;
        } else {
            p.applied = true;
        }
    }
    #[cfg(test)]
    pub(crate) fn test_last_wet(&self) -> &[f64] {
        &self.wet
    }
    pub fn returned_sha256(&self) -> String {
        format!("{:x}", self.returned_hash.clone().finalize())
    }
    pub fn nonzero_return_samples(&self) -> u64 {
        self.nonzero_return_samples
    }
    pub fn intentional_delay_frames(&self) -> u32 {
        self.fx
            .as_ref()
            .map_or(0, |fx| fx.intentional_delay_frames())
    }
    pub fn owner_resets(&self) -> u64 {
        self.fx.as_ref().map_or_else(
            || self.aux.as_ref().map_or(0, |a| a.resets()),
            |fx| fx.resets,
        )
    }
    pub async fn step(&mut self) -> Result<Option<Vec<u8>>> {
        let bytes = self.media.receive().await?;
        let packet = Packet::parse(&bytes).map_err(|e| format!("Brain packet {e:?}"))?;
        if packet.spec().role == Role::Analysis {
            self.stats.analysis_packets = self.stats.analysis_packets.saturating_add(1);
            return Ok(Some(bytes));
        }
        self.stats.send_packets = self.stats.send_packets.saturating_add(1);
        let assembled = match self.receiver.push(packet) {
            Ok(block) => block,
            Err(_) => {
                self.stats.stale_groups = self.stats.stale_groups.saturating_add(1);
                return Ok(None);
            }
        };
        if let Some((frame, input)) = assembled {
            if let Some(aux) = &mut self.aux {
                aux.process(frame, &input, &mut self.wet)?;
            } else {
                self.fx_boundary(frame, &input);
                self.fx
                    .as_mut()
                    .unwrap()
                    .process(
                        self.media.descriptor().source_epoch.0,
                        frame,
                        &input,
                        &mut self.wet,
                    )
                    .map_err(|e| format!("Brain FX {e:?}"))?;
            }
            let descriptor = self.media.descriptor();
            let packets = encode_grouped(descriptor, MediaRole::WetReturn, frame, &self.wet)?;
            for packet in packets {
                let parsed =
                    Packet::parse(&packet).map_err(|e| format!("return evidence {e:?}"))?;
                for index in 0..parsed.spec().samples() {
                    self.nonzero_return_samples +=
                        u64::from(parsed.sample(index).is_ok_and(|sample| sample != 0.));
                }
                self.returned_hash.update(&packet);
                self.media.send(packet)?;
                self.stats.returned_packets = self.stats.returned_packets.saturating_add(1);
            }
            self.stats.processed_blocks = self.stats.processed_blocks.saturating_add(1);
            self.stats.incomplete_blocks = self.receiver.incomplete_dropped;
        }
        Ok(None)
    }
}
