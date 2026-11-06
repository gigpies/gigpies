//! Four Brain-owned palette instances on the Stagebox source timeline.
use super::{MediaDescriptor, MediaRole, Result};
use crate::{host::fx_palette::Owner, show::Counter};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::Path,
    time::{Duration, Instant},
};
pub const CONTRACT: &str = "GP-FX-aux-owner";
pub fn channel_ids() -> Vec<String> {
    (1..=4)
        .flat_map(|n| [format!("fx-{n}-left"), format!("fx-{n}-right")])
        .collect()
}
pub fn binding(d: &MediaDescriptor, hash: &str) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(crate::hash::hex(Sha256::digest(
        serde_json::to_vec(&(d, hash)).map_err(|e| e.to_string())?,
    )))
}
pub use crate::fx_aux_wire::{Configuration, Tempo};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusState {
    pub id: String,
    pub generation: Counter,
    pub applied_generation: Counter,
    pub configuration_json: String,
    pub owner_json: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub descriptor: MediaDescriptor,
    pub library_sha256: String,
    pub next_frame: Counter,
    pub capabilities_json: String,
    pub palette_json: String,
    pub buses: Vec<BusState>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Prepare {
        ticket: Counter,
        descriptor: MediaDescriptor,
        mutation: crate::fx_aux_wire::Mutation,
        apply_frame: Counter,
    },
    Permit {
        ticket: Counter,
        descriptor: MediaDescriptor,
        apply_frame: Counter,
    },
    Cancel {
        ticket: Counter,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Observe {
        observation: Observation,
    },
    Prepared {
        ticket: Counter,
        mutation: crate::fx_aux_wire::Mutation,
        apply_frame: Counter,
    },
    Completed {
        ticket: Counter,
        apply_frame: Counter,
        observation: Observation,
    },
    Refused {
        ticket: Counter,
        reason: String,
    },
}
struct Bus {
    owner: Owner,
    config: Configuration,
    generation: u64,
    applied: u64,
    owner_request: Option<u64>,
}
struct Pending {
    bundle: Option<Box<crate::host::fx_palette::PreparedBundle>>,
    configs: Option<[Configuration; 4]>,
    ticket: u64,
    mutation: crate::fx_aux_wire::Mutation,
    config: Option<Configuration>,
    frame: u64,
    deadline: Instant,
    permitted: bool,
    applied: bool,
    token: Option<crate::host::fx_palette::PreparedToken>,
    failed: bool,
}
pub struct BrainAux {
    descriptor: MediaDescriptor,
    hash: String,
    buses: [Bus; 4],
    next: Option<u64>,
    pending: Option<Pending>,
    refused: Option<(u64, &'static str)>,
    resets: u64,
    _library_file: std::fs::File,
}
impl BrainAux {
    pub fn load(
        descriptor: MediaDescriptor,
        path: &Path,
        hash: &str,
        inputs: usize,
    ) -> Result<Self> {
        use sha2::{Digest, Sha256};
        use std::os::fd::AsRawFd;
        if !path.is_absolute() || inputs == 0 || inputs > 256 {
            return Err("FX aux owner admission".into());
        }
        for role in [MediaRole::FxSend, MediaRole::WetReturn] {
            let mut streams: Vec<_> = descriptor
                .streams
                .iter()
                .filter(|s| s.role == role)
                .collect();
            streams.sort_by_key(|s| s.first_channel);
            if streams.iter().any(|s| s.frames != 48)
                || streams
                    .iter()
                    .flat_map(|s| s.channel_ids.iter())
                    .ne(channel_ids().iter())
            {
                return Err("FX aux ordered eight-channel map".into());
            }
        }
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut digest = Sha256::new();
        std::io::copy(&mut file, &mut crate::hash::DigestWriter(&mut digest))
            .map_err(|e| e.to_string())?;
        if crate::hash::hex(digest.finalize()) != hash {
            return Err("FX aux library hash mismatch".into());
        }
        let pinned = std::path::PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
        let mut buses = Vec::new();
        for _ in 0..4 {
            let owner = Owner::load(&pinned)?;
            let config = Configuration {
                settings: owner.defaults(1)?,
                sends: vec![0.; inputs],
                returns: None,
                tempo: None,
            };
            let generation = 0;
            buses.push(Bus {
                owner,
                config,
                generation,
                applied: generation,
                owner_request: None,
            });
        }
        Ok(Self {
            descriptor,
            hash: hash.into(),
            buses: buses.try_into().map_err(|_| "four FX owners")?,
            next: None,
            pending: None,
            refused: None,
            resets: 0,
            _library_file: file,
        })
    }
    pub(crate) fn into_execution_parts(self) -> ([Owner; 4], [Configuration; 4], std::fs::File) {
        let configs = self.buses.each_ref().map(|b| b.config.clone());
        (self.buses.map(|b| b.owner), configs, self._library_file)
    }
    pub fn observation(&mut self) -> Result<Observation> {
        let mut states = Vec::new();
        for (i, b) in self.buses.iter_mut().enumerate() {
            b.owner.reclaim();
            let s = b.owner.status()?;
            if s.transition == 0
                && b.owner_request.is_none_or(|r| r == s.applied_request)
                && s.current == b.config.settings
            {
                b.applied = b.generation;
            }
            states.push(BusState {
                id: format!("fx-{}", i + 1),
                generation: Counter(b.generation),
                applied_generation: Counter(b.applied),
                configuration_json: serde_json::to_string(&b.config).map_err(|e| e.to_string())?,
                owner_json: serde_json::to_string(&s).map_err(|e| e.to_string())?,
            });
        }
        let owner = &self.buses[0].owner;
        let mut palette = Vec::new();
        for id in 1..32 {
            if owner.capabilities.algorithm_mask & (1 << id) != 0 {
                palette.push(json!({"algorithm":id,"defaults":owner.defaults(id)?,"parameters":owner.parameters(id)?}));
            }
        }
        Ok(Observation {
            descriptor: self.descriptor.clone(),
            library_sha256: self.hash.clone(),
            next_frame: Counter(self.next.unwrap_or(0)),
            capabilities_json: serde_json::to_string(&owner.capabilities)
                .map_err(|e| e.to_string())?,
            palette_json: serde_json::to_string(&palette).map_err(|e| e.to_string())?,
            buses: states,
        })
    }
    pub fn command(&mut self, c: Command) -> Result<Option<Message>> {
        match c {
            Command::Prepare {
                ticket,
                descriptor,
                mutation,
                apply_frame,
            } => {
                let result = (|| -> Result<()> {
                    if self.pending.is_some()
                        || self.refused.is_some()
                        || descriptor != self.descriptor
                        || mutation.bus >= 4
                        || mutation.library_sha256 != self.hash
                        || mutation.owner_binding.as_deref()
                            != Some(binding(&self.descriptor, &self.hash)?.as_str())
                        || mutation.map_revision != self.descriptor.map_generation
                        || self
                            .next
                            .is_none_or(|n| apply_frame.0 <= n || apply_frame.0 - n > 48000)
                        || !apply_frame.0.is_multiple_of(48)
                    {
                        return Err("FX aux stale/busy preparation".into());
                    }
                    mutation.validate_restore()?;
                    let configs = mutation
                        .restore
                        .as_ref()
                        .map(|r| r.saved.validate())
                        .transpose()?;
                    let bundle = if let Some(configs) = &configs {
                        let restore = mutation.restore.as_ref().unwrap();
                        let mut statuses = [crate::host::fx_palette::Status::default(); 4];
                        for (i, (status, bus)) in
                            statuses.iter_mut().zip(&mut self.buses).enumerate()
                        {
                            if restore.expected_generations[i].0 != bus.generation
                                || bus.generation != bus.applied
                            {
                                return Err("FX bundle stale generation".into());
                            }
                            *status = bus.owner.status()?;
                        }
                        let preparers = self.buses.each_ref().map(|b| b.owner.preparer());
                        Some(Box::new(crate::host::fx_palette::PreparedBundle::prepare(
                            preparers.each_ref(),
                            statuses,
                            self.buses.each_ref().map(|b| b.generation),
                            configs,
                        )?))
                    } else {
                        None
                    };
                    let b = &mut self.buses[mutation.bus];
                    b.owner.reclaim();
                    let status = b.owner.status()?;
                    if mutation.expected_generation.0 != b.generation
                        || b.generation == u64::MAX
                        || (!mutation.panic
                            && (b.generation != b.applied || status.transition != 0))
                        || mutation.panic == mutation.configuration_json.is_some()
                    {
                        return Err("FX aux generation/transition".into());
                    }
                    let config: Option<Configuration> = mutation
                        .configuration_json
                        .as_ref()
                        .map(|s| serde_json::from_str(s).map_err(|e| e.to_string()))
                        .transpose()?;
                    if let Some(c) = &config {
                        if let Some(r) = &c.returns {
                            r.validate(r.monitors.len())?;
                        }
                        if c.sends.len() != b.config.sends.len()
                            || c.sends
                                .iter()
                                .any(|v| !v.is_finite() || !(0.0..=1.).contains(v))
                        {
                            return Err("FX aux send bounds".into());
                        }
                        if let Some(t) = &c.tempo
                            && (!matches!(c.settings.algorithm, 1 | 8)
                                || (b.owner.delay_time(t.bpm, t.denominator, t.modifier)?
                                    - c.settings.time_ms)
                                    .abs()
                                    > 1e-9)
                        {
                            return Err("FX aux tempo/time".into());
                        }
                    }
                    let prepared = config
                        .as_ref()
                        .is_some_and(|c| c.settings != b.config.settings);
                    let token = if prepared && bundle.is_none() {
                        Some(
                            b.owner
                                .preparer()
                                .prepare(&config.as_ref().unwrap().settings)?,
                        )
                    } else {
                        None
                    };
                    self.pending = Some(Pending {
                        bundle,
                        configs,
                        ticket: ticket.0,
                        mutation: mutation.clone(),
                        config,
                        frame: apply_frame.0,
                        deadline: Instant::now() + Duration::from_millis(1500),
                        permitted: false,
                        applied: false,
                        token,
                        failed: false,
                    });
                    Ok(())
                })();
                Ok(Some(match result {
                    Ok(()) => Message::Prepared {
                        ticket,
                        mutation,
                        apply_frame,
                    },
                    Err(reason) => Message::Refused { ticket, reason },
                }))
            }
            Command::Permit {
                ticket,
                descriptor,
                apply_frame,
            } => {
                let p = self
                    .pending
                    .as_mut()
                    .ok_or("FX aux permit absent preparation")?;
                if p.failed
                    || p.ticket != ticket.0
                    || descriptor != self.descriptor
                    || p.frame != apply_frame.0
                    || p.permitted
                    || Instant::now() >= p.deadline
                    || self.next.is_some_and(|n| n > p.frame)
                {
                    return Err("FX aux late/stale permit".into());
                }
                p.permitted = true;
                Ok(None)
            }
            Command::Cancel { ticket } => {
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.ticket == ticket.0 && !p.permitted)
                {
                    let p = self.pending.take().unwrap();
                    self.buses[p.mutation.bus].owner.cancel();
                }
                Ok(None)
            }
        }
    }
    fn refuse_pending(&mut self, reason: &'static str) {
        if let Some(p) = &mut self.pending {
            // Retain the token, request and both configurations until control
            // observes the refusal. A full retirement slot rejects new preparation.
            if !p.failed {
                p.failed = true;
                self.refused = Some((p.ticket, reason));
            }
        }
    }
    pub fn process(
        &mut self,
        frame: u64,
        input: &[f64],
        wet: &mut [f64],
    ) -> std::result::Result<(), &'static str> {
        wet.fill(0.);
        if input.len() != 384 || wet.len() != 384 {
            return Err("FX aux media shape");
        }
        let next_frame = frame.checked_add(48).ok_or("FX frame exhausted")?;
        if self.next.is_some_and(|n| n != frame) {
            self.refuse_pending("FX aux source gap");
            for b in &mut self.buses {
                b.owner.reset_bounded()?;
                b.owner_request = None;
                let s = b.owner.status_bounded()?;
                if b.config.settings != s.current {
                    b.config.settings = s.current;
                    b.config.tempo = None;
                }
                b.generation = b
                    .generation
                    .checked_add(1)
                    .ok_or("FX aux generation exhausted")?;
                b.applied = b.generation;
            }
            self.resets += 1;
            self.next = Some(next_frame);
            return Ok(());
        }
        if self.pending.as_ref().is_some_and(|p| {
            !p.failed && !p.applied && (frame > p.frame || Instant::now() >= p.deadline)
        }) {
            self.refuse_pending("FX aux missed boundary/deadline");
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| !p.failed && !p.applied && frame == p.frame)
        {
            let p = self.pending.as_mut().unwrap();
            if !p.permitted {
                self.refuse_pending("FX aux absent permit");
            } else {
                if let Some(bundle) = &mut p.bundle {
                    let [a, b, c, d] = &mut self.buses;
                    let result =
                        bundle.publish([&mut a.owner, &mut b.owner, &mut c.owner, &mut d.owner]);
                    match result {
                        Ok(requests) => {
                            for (i, config) in p.configs.as_mut().unwrap().iter_mut().enumerate() {
                                std::mem::swap(&mut self.buses[i].config, config);
                                self.buses[i].generation += 1;
                                self.buses[i].owner_request = requests[i];
                            }
                            p.applied = true;
                        }
                        Err(_) => {
                            self.refuse_pending("FX bundle publication refused or uncertain");
                        }
                    }
                } else {
                    let b = &mut self.buses[p.mutation.bus];
                    let result = (|| -> std::result::Result<(), &'static str> {
                        if p.mutation.panic {
                            b.owner.reset_bounded()?;
                            b.owner_request = None;
                            let s = b.owner.status_bounded()?;
                            if b.config.settings != s.current {
                                b.config.settings = s.current;
                                b.config.tempo = None;
                            }
                        } else {
                            if let Some(token) = &mut p.token {
                                let s = b.owner.status_bounded()?;
                                let id = s
                                    .accepted_request
                                    .checked_add(1)
                                    .ok_or("FX request exhausted")?;
                                b.owner
                                    .publish_token(token, id, s.revision)
                                    .map_err(|_| "FX aux publish refused")?;
                                b.owner_request = Some(id);
                            }
                            // Keep the retired Vec in the occupied pending slot;
                            // only poll/command on the controller may destroy it.
                            let prepared = p.config.as_mut().ok_or("FX aux configuration")?;
                            std::mem::swap(&mut b.config, prepared);
                        }
                        b.generation += 1;
                        p.applied = true;
                        Ok(())
                    })();
                    if let Err(e) = result {
                        self.refuse_pending(e);
                    }
                }
            }
        }
        for (bus, b) in self.buses.iter_mut().enumerate() {
            let mut source = [0.; 96];
            let mut out = [0.; 96];
            for f in 0..48 {
                source[f * 2..f * 2 + 2]
                    .copy_from_slice(&input[f * 8 + bus * 2..f * 8 + bus * 2 + 2]);
            }
            b.owner.process(&source, &mut out);
            for f in 0..48 {
                wet[f * 8 + bus * 2..f * 8 + bus * 2 + 2].copy_from_slice(&out[f * 2..f * 2 + 2]);
            }
        }
        self.next = Some(next_frame);
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Option<Message>> {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| !p.applied && Instant::now() >= p.deadline)
        {
            self.refuse_pending("FX aux preparation deadline");
        }
        if let Some(p) = self.pending.as_ref().filter(|p| p.applied) {
            let mut fault = false;
            for (i, bus) in self.buses.iter_mut().enumerate() {
                if p.bundle.is_some() || i == p.mutation.bus {
                    fault |= bus.owner.status()?.fault != 0;
                }
            }
            if fault {
                self.refuse_pending("FX aux owner fault after publication");
            }
        }
        if let Some((ticket, reason)) = self.refused.take() {
            if let Some(p) = self.pending.take() {
                self.buses[p.mutation.bus].owner.cancel();
            }
            return Ok(Some(Message::Refused {
                ticket: Counter(ticket),
                reason: reason.into(),
            }));
        }
        if let Some(p) = &mut self.pending
            && let Some(token) = &mut p.token
        {
            self.buses[p.mutation.bus].owner.take_retired(token);
        }
        let observation = self.observation()?;
        if let Some(p) = self.pending.as_ref().filter(|p| p.applied) {
            let ticket = Counter(p.ticket);
            let apply_frame = Counter(p.frame);
            let b = &observation.buses[p.mutation.bus];
            if if p.bundle.is_some() {
                observation
                    .buses
                    .iter()
                    .all(|b| b.generation == b.applied_generation)
            } else {
                b.generation == b.applied_generation
            } {
                self.pending = None;
            }
            return Ok(Some(Message::Completed {
                ticket,
                apply_frame,
                observation,
            }));
        }
        Ok(Some(Message::Observe { observation }))
    }
    pub fn resets(&self) -> u64 {
        self.resets
    }
}
