//! Brain follows source-indexed complete send blocks. It has no audio device or
//! free-running render clock, and calls the actual SHR FX library through its
//! existing host adapter. All allocation/network operations are worker-side.
use super::*;
use crate::transport::{Packet, Role};
use sha2::{Digest, Sha256};
use std::path::Path;
struct AssemblySlot {
    frame: Option<u64>,
    groups: u64,
    samples: Box<[f64]>,
}
/// Prepared assembly storage. Frames, not arrival order, define block identity.
/// Completion copies into caller-owned storage; no allocation/destruction occurs
/// in `push_into`, including stale, duplicate and capacity paths.
pub struct GroupedReceiver {
    specs: Vec<crate::transport::StreamSpec>,
    slots: Box<[AssemblySlot]>,
    channels: usize,
    frames: usize,
    complete_mask: u64,
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
        let specs: Vec<_> = specs
            .into_iter()
            .filter(|s| s.role == role.packet_role())
            .collect();
        let frames = usize::from(specs[0].frames);
        let complete_mask = if specs.len() == 64 {
            u64::MAX
        } else {
            (1u64 << specs.len()) - 1
        };
        Ok(Self {
            specs,
            channels,
            frames,
            complete_mask,
            slots: (0..32)
                .map(|_| AssemblySlot {
                    frame: None,
                    groups: 0,
                    samples: vec![0.; channels * frames].into_boxed_slice(),
                })
                .collect(),
            next: None,
            incomplete_dropped: 0,
        })
    }
    /// Exact requested Rust layouts for this prepared receiver, including slot
    /// metadata and descriptor-spec capacity. Excludes the caller's output block.
    pub fn prepared_storage(&self) -> crate::transport::PreparedStorage {
        let heap = self.specs.capacity() * std::mem::size_of::<crate::transport::StreamSpec>()
            + std::mem::size_of_val(self.slots.as_ref())
            + self
                .slots
                .iter()
                .map(|s| std::mem::size_of_val(s.samples.as_ref()))
                .sum::<usize>();
        crate::transport::PreparedStorage::checked(std::mem::size_of::<Self>(), heap)
            .expect("admitted bounded assembly storage")
    }
    pub fn sample_count(&self) -> usize {
        self.channels * self.frames
    }
    /// Compatibility API for non-audio observers. Audio callers use `push_into`.
    pub fn push(&mut self, packet: Packet<'_>) -> Result<Option<(u64, Vec<f64>)>> {
        let mut samples = vec![0.; self.sample_count()];
        self.push_into(packet, &mut samples)
            .map(|frame| frame.map(|f| (f, samples)))
            .map_err(str::to_owned)
    }
    pub fn push_into(
        &mut self,
        packet: Packet<'_>,
        output: &mut [f64],
    ) -> std::result::Result<Option<u64>, &'static str> {
        if output.len() != self.sample_count() {
            return Err("group output shape");
        }
        let group = self
            .specs
            .iter()
            .position(|s| *s == packet.spec())
            .ok_or("group stream")?;
        let frame = packet.source_frame();
        if self.next.is_some_and(|n| frame < n) {
            return Err("stale or mismatched group");
        }
        let index = if let Some(i) = self.slots.iter().position(|s| s.frame == Some(frame)) {
            i
        } else if let Some(i) = self.slots.iter().position(|s| s.frame.is_none()) {
            i
        } else {
            let (i, _) = self
                .slots
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.frame)
                .unwrap();
            self.incomplete_dropped = self.incomplete_dropped.saturating_add(1);
            self.slots[i].frame = None;
            i
        };
        let slot = &mut self.slots[index];
        if slot.frame.is_none() {
            slot.frame = Some(frame);
            slot.groups = 0;
        }
        let bit = 1u64 << group;
        if slot.groups & bit != 0 {
            return Err("duplicate group");
        }
        for f in 0..self.frames {
            for c in 0..usize::from(packet.spec().channels) {
                slot.samples[f * self.channels + usize::from(packet.spec().first_channel) + c] =
                    packet
                        .sample(f * usize::from(packet.spec().channels) + c)
                        .map_err(|_| "group sample")?;
            }
        }
        slot.groups |= bit;
        if slot.groups != self.complete_mask {
            return Ok(None);
        }
        output.copy_from_slice(&slot.samples);
        slot.frame = None;
        for old in &mut self.slots {
            if old.frame.is_some_and(|f| f < frame) {
                old.frame = None;
                self.incomplete_dropped = self.incomplete_dropped.saturating_add(1);
            }
        }
        self.next = Some(
            frame
                .checked_add(self.frames as u64)
                .ok_or("source frame exhausted")?,
        );
        Ok(Some(frame))
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
    trace: super::diagnostics::Handle,
    fx_binding: Option<crate::fx_wire::Binding>,
    fx_ticket: Option<FxTicket>,
    fx_refused: Option<(u64, &'static str)>,
    media: Option<MediaChannel>,
    execution: Option<super::aux_execution::Execution>,
    legacy_execution: Option<super::fx_execution::Execution>,
    legacy_delay: u32,
    media_worker: Option<super::aux_execution::MediaWorker>,
    receiver: Option<GroupedReceiver>,
    fx: Option<crate::host::brain_fx::BrainFx>,
    aux: Option<super::fx_aux_owner::BrainAux>,
    wet: Vec<f64>,
    input: Vec<f64>,
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
            trace: None,
            fx_binding: None,
            fx_ticket: None,
            fx_refused: None,
            media: Some(media),
            execution: None,
            legacy_execution: None,
            legacy_delay: 0,
            media_worker: None,
            receiver: Some(GroupedReceiver::new(descriptor, MediaRole::FxSend)?),
            fx: Some(fx),
            aux: None,
            wet: vec![0.; usize::from(frames) * channels],
            input: vec![0.; usize::from(frames) * channels],
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
            trace: None,
            fx_binding: None,
            fx_ticket: None,
            fx_refused: None,
            receiver: Some(GroupedReceiver::new(descriptor, MediaRole::FxSend)?),
            media: Some(media),
            execution: None,
            legacy_execution: None,
            legacy_delay: 0,
            media_worker: None,
            fx: None,
            aux: Some(aux),
            wet: vec![0.; 384],
            input: vec![0.; 384],
            stats: BrainStats::default(),
            returned_hash: Sha256::new(),
            nonzero_return_samples: 0,
            fx_library_file: None,
        })
    }
    /// Production four-aux execution: sole DSP thread and independent media
    /// runtime; controller stalls cannot stop either owner.
    pub(crate) fn start_aux_execution(&mut self, deadline: std::time::Instant) -> Result<()> {
        let media = self.media.take().ok_or("FX media already moved")?;
        let mut execution = super::aux_execution::Execution::from_owner(
            self.aux.take().ok_or("four aux owner unavailable")?,
            deadline,
            Some(media.audio_fence()),
        )?;
        let worker = super::aux_execution::MediaWorker::start(
            media,
            execution.take_media(),
            self.receiver.take().ok_or("FX assembly already moved")?,
            self.trace.clone(),
        )?;
        self.input = Vec::new();
        self.wet = Vec::new();
        self.execution = Some(execution);
        self.media_worker = Some(worker);
        Ok(())
    }
    /// Every admitted legacy shape uses independent native and media owners.
    pub(crate) fn start_fx_execution(&mut self, deadline: std::time::Instant) -> Result<()> {
        let media = self.media.as_ref().ok_or("FX media already moved")?;
        if self.fx_ticket.is_some() || self.fx_refused.is_some() {
            return Err("FX execution must start before control preparation".into());
        }
        let epoch = media.descriptor().source_epoch.0;
        let policy = media.audio_fence();
        let fx = self.fx.take().ok_or("legacy FX owner unavailable")?;
        self.legacy_delay = fx.intentional_delay_frames();
        let mut execution = super::fx_execution::Execution::from_owner(
            fx,
            self.fx_binding.clone(),
            epoch,
            deadline,
            Some(policy),
        )?;
        let worker = super::aux_execution::MediaWorker::start(
            self.media.take().unwrap(),
            execution.take_media(),
            self.receiver.take().ok_or("FX assembly already moved")?,
            self.trace.clone(),
        )?;
        self.input = Vec::new();
        self.wet = Vec::new();
        self.legacy_execution = Some(execution);
        self.media_worker = Some(worker);
        Ok(())
    }
    pub(crate) fn finish_execution(&mut self) {
        if let Some(worker) = &mut self.media_worker {
            worker.finish();
            self.stats = worker.report.stats.clone();
            self.nonzero_return_samples = worker.report.nonzero;
        }
        if let Some(execution) = &mut self.execution {
            execution.finish();
            self.stats.processed_blocks = execution.processed();
        }
        if let Some(execution) = &mut self.legacy_execution {
            execution.finish();
            self.stats.processed_blocks = execution.processed();
        }
    }
    pub(crate) fn execution_report(&self) -> Option<serde_json::Value> {
        let mut report = if let Some(execution) = &self.execution {
            execution.report()
        } else {
            self.legacy_execution.as_ref()?.report()
        };
        if let Some(worker) = &self.media_worker {
            report["assembly_prepared_bytes"] =
                serde_json::json!(worker.report.assembly_prepared_bytes);
            report["input_drops"] = serde_json::json!(worker.report.input_drops);
            report["first_input_drop"] = serde_json::json!(worker.report.first_input_drop);
            report["last_input_drop"] = serde_json::json!(worker.report.last_input_drop);
            report["analysis_drops"] = serde_json::json!(worker.report.analysis_drops);
            report["report_drops"] = serde_json::json!(worker.report.report_drops);
            report["analysis_unconsumed_at_shutdown"] =
                serde_json::json!(worker.report.analysis_unconsumed);
            report["media_fault"] = serde_json::json!(worker.report.fault);
        }
        Some(report)
    }
    pub fn aux_command(
        &mut self,
        c: super::fx_aux_owner::Command,
    ) -> Result<Option<super::fx_aux_owner::Message>> {
        if let Some(execution) = &mut self.execution {
            return execution.command(c);
        }
        self.aux
            .as_mut()
            .ok_or("four aux owner unavailable")?
            .command(c)
    }
    pub fn poll_aux_observation(&mut self) -> Result<Option<super::fx_aux_owner::Message>> {
        if let Some(execution) = &mut self.execution {
            return execution.poll();
        }
        self.aux
            .as_mut()
            .ok_or("four aux owner unavailable")?
            .poll()
    }
    pub fn enable_fx_control(&mut self, path: &Path, expected_sha256: &str) -> Result<bool> {
        if self.aux.is_some() || self.execution.is_some() {
            return Err("four aux owner excludes legacy delay control".into());
        }
        if self.legacy_execution.is_some() {
            return Err("FX controls must be configured before execution".into());
        }
        let d = self
            .media
            .as_ref()
            .ok_or("FX media unavailable")?
            .descriptor();
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
        if let Some(execution) = &mut self.legacy_execution {
            return execution.observation();
        }
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
        if let Some(execution) = &mut self.legacy_execution {
            return execution.command(command);
        }
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
        if let Some(execution) = &mut self.legacy_execution {
            return execution.poll();
        }
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
    fn fx_boundary(&mut self, frame: u64) {
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
            || self.input.iter().any(|s| !s.is_finite() || s.abs() > 16.)
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
    pub(crate) fn test_poll_media_report(&mut self) {
        if let Some(worker) = &mut self.media_worker {
            let _ = worker.poll();
        }
    }
    #[cfg(test)]
    pub(crate) fn test_execution_energy(&self) -> Option<[f64; 4]> {
        self.media_worker
            .as_ref()
            .and_then(|worker| worker.report.energy.as_slice().try_into().ok())
    }
    #[cfg(test)]
    pub(crate) fn test_last_wet(&self) -> &[f64] {
        &self.wet
    }
    pub fn returned_sha256(&self) -> String {
        if let Some(worker) = &self.media_worker {
            return worker.report.hash.clone();
        }
        format!("{:x}", self.returned_hash.clone().finalize())
    }
    pub fn nonzero_return_samples(&self) -> u64 {
        self.nonzero_return_samples
    }
    pub fn intentional_delay_frames(&self) -> u32 {
        if self.legacy_execution.is_some() {
            return self.legacy_delay;
        }
        self.fx
            .as_ref()
            .map_or(0, |fx| fx.intentional_delay_frames())
    }
    pub fn owner_resets(&self) -> u64 {
        if let Some(execution) = &self.legacy_execution {
            return execution.resets();
        }
        if let Some(execution) = &self.execution {
            return execution.resets();
        }
        self.fx.as_ref().map_or_else(
            || self.aux.as_ref().map_or(0, |a| a.resets()),
            |fx| fx.resets,
        )
    }
    pub(crate) fn set_diagnostics(&mut self, trace: super::diagnostics::Handle) {
        self.trace = trace;
    }
    pub async fn step(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(worker) = &mut self.media_worker {
            tokio::time::sleep(std::time::Duration::from_micros(250)).await;
            let packet = worker.poll()?;
            self.stats = worker.report.stats.clone();
            self.nonzero_return_samples = worker.report.nonzero;
            return Ok(packet);
        }
        let bytes = self.media.as_mut().unwrap().receive().await?;
        let packet = Packet::parse(&bytes).map_err(|e| format!("Brain packet {e:?}"))?;
        super::diagnostics::media_record(
            &self.trace,
            super::diagnostics::Stage::BrainReceive,
            packet,
            0,
        );
        if packet.spec().role == Role::Analysis {
            self.stats.analysis_packets = self.stats.analysis_packets.saturating_add(1);
            return Ok(Some(bytes));
        }
        self.stats.send_packets = self.stats.send_packets.saturating_add(1);
        let assembled = match self
            .receiver
            .as_mut()
            .unwrap()
            .push_into(packet, &mut self.input)
        {
            Ok(block) => block,
            Err(_) => {
                self.stats.stale_groups = self.stats.stale_groups.saturating_add(1);
                return Ok(None);
            }
        };
        if let Some(frame) = assembled {
            let token = super::diagnostics::Token {
                session: packet.spec().session,
                ordinal: frame,
                request: frame,
                kind: 0,
            };
            super::diagnostics::record(
                &self.trace,
                token,
                super::diagnostics::Stage::BrainProcessBegin,
                0,
                0,
            );
            if let Some(aux) = &mut self.aux {
                aux.process(frame, &self.input, &mut self.wet)?;
            } else {
                self.fx_boundary(frame);
                self.fx
                    .as_mut()
                    .unwrap()
                    .process(
                        self.media.as_ref().unwrap().descriptor().source_epoch.0,
                        frame,
                        &self.input,
                        &mut self.wet,
                    )
                    .map_err(|e| format!("Brain FX {e:?}"))?;
            }
            super::diagnostics::record(
                &self.trace,
                token,
                super::diagnostics::Stage::BrainProcessEnd,
                0,
                0,
            );
            let descriptor = self.media.as_ref().unwrap().descriptor();
            let packets = encode_grouped(descriptor, MediaRole::WetReturn, frame, &self.wet)?;
            for packet in packets {
                let parsed =
                    Packet::parse(&packet).map_err(|e| format!("return evidence {e:?}"))?;
                for index in 0..parsed.spec().samples() {
                    self.nonzero_return_samples +=
                        u64::from(parsed.sample(index).is_ok_and(|sample| sample != 0.));
                }
                self.returned_hash.update(&packet);
                super::diagnostics::media_record(
                    &self.trace,
                    super::diagnostics::Stage::BrainSend,
                    parsed,
                    0,
                );
                self.media.as_ref().unwrap().send(packet)?;
                self.stats.returned_packets = self.stats.returned_packets.saturating_add(1);
            }
            if self.trace.is_some() && frame.is_multiple_of(4800) {
                self.media
                    .as_ref()
                    .unwrap()
                    .diagnostic_stats(&self.trace, frame);
            }
            self.stats.processed_blocks = self.stats.processed_blocks.saturating_add(1);
            self.stats.incomplete_blocks = self.receiver.as_ref().unwrap().incomplete_dropped;
        }
        Ok(None)
    }
}
