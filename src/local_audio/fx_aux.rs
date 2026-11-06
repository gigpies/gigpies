//! Four source-clock auxes. Preparation, query and retirement are control-side.
use super::*;
use crate::fx_aux_wire as wire;
#[cfg(feature = "hardware-host")]
use crate::host::fx_palette::{Owner, Settings};
use serde_json::{Value, json};
#[cfg(feature = "hardware-host")]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub settings: Settings,
    pub sends: Vec<f64>,
    #[serde(default)]
    pub tempo: Option<Tempo>,
}
#[cfg(feature = "hardware-host")]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tempo {
    pub bpm: f64,
    pub denominator: u32,
    pub modifier: u32,
}
#[cfg(feature = "hardware-host")]
struct Bus {
    owner: Owner,
    output: [f64; 96],
}
#[cfg(feature = "hardware-host")]
pub(super) struct Rack {
    buses: [Bus; 4],
    wet: [f64; 96],
    next_frame: u64,
}
#[cfg(feature = "hardware-host")]
struct BusControl {
    preparer: crate::host::fx_palette::Preparer,
    status: crate::host::fx_palette::Status,
    generation: u64,
    applied: u64,
    settle_frame: u64,
    owner_request: Option<u64>,
    config: Configuration,
}
/// Instance-free controller state. The active native owners leave this facade
/// once, and observations/retired tokens return through bounded owned packets.
#[cfg(feature = "hardware-host")]
pub(super) struct Controller {
    source: Option<Rack>,
    buses: [BusControl; 4],
    hash: String,
    _library_file: std::fs::File,
}
#[cfg(feature = "hardware-host")]
pub(super) struct Observation {
    status: [crate::host::fx_palette::Status; 4],
    valid: [bool; 4],
    retired: [crate::host::fx_palette::PreparedToken; 4],
}
#[cfg(feature = "hardware-host")]
struct PreparedCommand {
    bus: usize,
    panic: bool,
    token: Option<crate::host::fx_palette::PreparedToken>,
    sends: crate::mixer::PreparedFxSends,
    owner_request: Option<u64>,
    revision: u64,
    observation: Observation,
}
#[cfg(feature = "hardware-host")]
pub(super) struct Pending {
    request: crate::control_model::Request,
    fingerprint: String,
    mutation: wire::Mutation,
    config: Option<Configuration>,
    token: Option<crate::host::fx_palette::PreparedToken>,
    frame: u64,
    client: Option<u64>,
}
#[cfg(feature = "hardware-host")]
impl Controller {
    pub(super) fn take_source(&mut self) -> Result<Rack> {
        self.source
            .take()
            .ok_or("FX source already detached".into())
    }
    pub(super) fn restore_source(&mut self, source: Rack) -> Result<()> {
        if self.source.is_some() {
            return Err("FX source already attached".into());
        }
        self.source = Some(source);
        Ok(())
    }
    pub(super) fn prepare_observation(&self) -> Observation {
        Observation {
            status: [crate::host::fx_palette::Status::default(); 4],
            valid: [false; 4],
            retired: std::array::from_fn(|i| self.buses[i].preparer.empty_token()),
        }
    }
    pub(super) fn finish_observation(
        &mut self,
        observation: Observation,
        frame: u64,
    ) -> Result<()> {
        if observation.valid.iter().any(|valid| !valid) {
            return Err("FX palette status ABI".into());
        }
        for (i, b) in self.buses.iter_mut().enumerate() {
            b.status = observation.status[i];
            if frame >= b.settle_frame
                && b.status.transition == 0
                && b.owner_request
                    .is_none_or(|r| r == b.status.applied_request)
                && b.status.current == b.config.settings
            {
                b.applied = b.generation;
            }
        }
        // `observation` owns at most one retired token per bus. Drop happens here
        // on the controller, after the source relinquished every native pointer.
        Ok(())
    }
}
impl LocalAudio {
    #[cfg(feature = "hardware-host")]
    pub fn enable_fx_aux(&mut self, path: &Path, expected_hash: &str) -> Result<()> {
        use sha2::{Digest, Sha256};
        use std::os::fd::AsRawFd;
        if self.frame() != 0
            || self.render_graph.is_some()
            || self.source_executor.is_some()
            || self.fx_aux.is_some()
            || self.remote_fx_aux
            || !path.is_absolute()
        {
            return Err("FX aux startup only, absolute library".into());
        }
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut digest = Sha256::new();
        std::io::copy(&mut file, &mut digest).map_err(|e| e.to_string())?;
        // Reserve bounded wire space for four configurations and actual input identities.
        let map_bytes = serde_json::to_vec(&self.topology().inputs)
            .map_err(|e| e.to_string())?
            .len();
        if self
            .topology()
            .inputs
            .len()
            .checked_mul(128)
            .and_then(|n| n.checked_add(map_bytes + 32768))
            .is_none_or(|n| n > MAX_FRAME)
        {
            return Err("FX aux observation capacity for configured input map".into());
        }
        let hash = format!("{:x}", digest.finalize());
        if hash != expected_hash {
            return Err("FX aux library hash mismatch".into());
        }
        let pinned = std::path::PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
        let mut buses = Vec::with_capacity(4);
        let mut controls = Vec::with_capacity(4);
        for _ in 0..4 {
            let mut owner = Owner::load(&pinned)?;
            let config = Configuration {
                settings: owner.defaults(1)?,
                sends: vec![0.; self.topology().inputs.len()],
                tempo: None,
            };
            controls.push(BusControl {
                preparer: owner.preparer(),
                status: owner.status()?,
                config,
                generation: 0,
                applied: 0,
                settle_frame: 0,
                owner_request: None,
            });
            buses.push(Bus {
                owner,
                output: [0.; 96],
            });
        }
        self.engine.enable_fx_aux()?;
        self.fx_aux = Some(Controller {
            source: Some(Rack {
                buses: buses.try_into().map_err(|_| "FX buses")?,
                wet: [0.; 96],
                next_frame: 0,
            }),
            buses: controls.try_into().map_err(|_| "FX controls")?,
            hash,
            _library_file: file,
        });
        Ok(())
    }
    pub fn fx_aux_enabled(&self) -> bool {
        #[cfg(feature = "hardware-host")]
        {
            self.fx_aux.is_some()
        }
        #[cfg(not(feature = "hardware-host"))]
        {
            false
        }
    }
    pub fn fx_aux_request(
        &mut self,
        value: Value,
        now: u64,
        fresh: bool,
        client: Option<u64>,
    ) -> Result<Value> {
        let (r, m) = wire::request(&value, self.engine.wire_version())?;
        if r.show_id != self.show || r.epoch != self.epoch {
            return Err("FX aux attachment".into());
        }
        if m.is_none() {
            return self.fx_aux_snapshot(&r);
        }
        let _ = (now, fresh, client);
        #[cfg(not(feature = "hardware-host"))]
        {
            Ok(Self::fx_aux_reply(
                &r,
                "rejected",
                Some("FX aux unavailable".into()),
                None,
            ))
        }
        #[cfg(feature = "hardware-host")]
        {
            let m = m.unwrap();
            let result = (|| -> Result<()> {
                if !fresh || self.fx_aux_pending.is_some() {
                    return Err("FX aux fresh read required or busy".into());
                }
                let rack = self.fx_aux.as_mut().ok_or("FX aux unavailable")?;
                let bus = &mut rack.buses[m.bus];
                let status = bus.status;
                if m.owner_binding.is_some()
                    || m.library_sha256 != rack.hash
                    || m.map_revision.0 != self.engine.topology().map_revision
                    || m.expected_generation.0 != bus.generation
                    || (!m.panic && (bus.applied != bus.generation || status.transition != 0))
                    || bus.generation == u64::MAX
                {
                    return Err("FX aux stale/busy identity".into());
                }
                let config: Option<Configuration> = m
                    .configuration_json
                    .as_ref()
                    .map(|s| serde_json::from_str(s).map_err(|e| e.to_string()))
                    .transpose()?;
                let mut token = None;
                if let Some(c) = &config {
                    if c.sends.len() != bus.config.sends.len()
                        || c.sends
                            .iter()
                            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                    {
                        return Err("FX aux sends shape/range".into());
                    }
                    if let Some(t) = &c.tempo
                        && (!matches!(c.settings.algorithm, 1 | 8)
                            || (bus.preparer.delay_time(t.bpm, t.denominator, t.modifier)?
                                - c.settings.time_ms)
                                .abs()
                                > 1e-9)
                    {
                        return Err("FX tempo/time mismatch".into());
                    }
                    // Validation/preparation precedes any reservation or sounding change.
                    // Always validate, even when only sends changed. The token
                    // is caller-owned and can be discarded here off the source.
                    let prepared = bus.preparer.prepare(&c.settings)?;
                    if c.settings != bus.config.settings {
                        token = Some(prepared);
                    }
                }
                use sha2::{Digest, Sha256};
                let fingerprint = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&value).map_err(|e| e.to_string())?)
                );
                let reservation = self.engine.begin_external(
                    &r,
                    &fingerprint,
                    crate::control_model::Scope::FxConfiguration,
                    now,
                );
                let frame = match reservation {
                    Ok((frame, None)) => frame,
                    Ok((_, Some(reply))) => {
                        return Err(reply
                            .body
                            .reason
                            .unwrap_or("request already completed; refresh".into()));
                    }
                    Err(e) => {
                        return Err(e);
                    }
                };
                if frame.checked_add(240).is_none() {
                    self.engine.cancel_fx_external(&r, &fingerprint);
                    return Err("FX source frame exhausted".into());
                }
                self.fx_aux_pending = Some(Pending {
                    request: r.clone(),
                    fingerprint,
                    mutation: m,
                    token,
                    config,
                    frame,
                    client,
                });
                Ok(())
            })();
            Ok(match result {
                Ok(()) => Self::fx_aux_reply(&r, "prepared", None, None),
                Err(e) => Self::fx_aux_reply(&r, "rejected", Some(e), None),
            })
        }
    }
    fn fx_aux_reply(
        r: &crate::control_model::Request,
        state: &str,
        reason: Option<String>,
        frame: Option<u64>,
    ) -> Value {
        json!({"contract":wire::CONTRACT,"version":1,"show_id":r.show_id,"module":"audio","epoch":r.epoch,
            "writer":r.writer,"lease":r.lease,"request_id":r.request_id,"state":state,"reason":reason,
            "effective_frame":frame.map(|f|f.to_string())})
    }
    fn fx_aux_snapshot(&mut self, r: &crate::control_model::Request) -> Result<Value> {
        #[allow(unused_mut)]
        let mut value = Self::fx_aux_reply(r, "snapshot", None, None);
        value["revision"] = json!(self.brain_snapshot().revision);
        value["available"] = json!(false);
        #[cfg(feature = "hardware-host")]
        if let Some(rack) = &mut self.fx_aux {
            let mut buses = Vec::new();
            for (i, bus) in rack.buses.iter_mut().enumerate() {
                let status = bus.status;
                buses.push(json!({"id":format!("fx-{}",i+1),"generation":bus.generation.to_string(),
                    "applied_generation":bus.applied.to_string(),"configuration_json":serde_json::to_string(&bus.config).map_err(|e|e.to_string())?,
                    "owner_json":serde_json::to_string(&status).map_err(|e|e.to_string())?}));
            }
            let mut palette = Vec::new();
            let owner = &rack.buses[0].preparer;
            for algorithm in 1..32 {
                if owner.capabilities.algorithm_mask & (1 << algorithm) != 0 {
                    palette.push(json!({"algorithm":algorithm,"defaults":owner.defaults(algorithm)?,"parameters":owner.parameters(algorithm)?}));
                }
            }
            value["available"] = json!(true);
            value["pending"] = json!(self.fx_aux_pending.as_ref().map(|p| p.mutation.bus));
            value["library_sha256"] = json!(rack.hash);
            value["map_revision"] = json!(self.engine.topology().map_revision.to_string());
            value["frame"] = json!(self.engine.frame().to_string());
            value["inputs"] =
                serde_json::to_value(&self.engine.topology().inputs).map_err(|e| e.to_string())?;
            value["buses"] = json!(buses);
            value["palette_json"] =
                json!(serde_json::to_string(&palette).map_err(|e| e.to_string())?);
            value["capabilities_json"] =
                json!(serde_json::to_string(&owner.capabilities).map_err(|e| e.to_string())?);
        }
        Ok(value)
    }
    /// Return numeric observations and native retirement without borrowing the
    /// source from snapshots or preparation. Called after a render quantum.
    #[cfg(feature = "hardware-host")]
    pub(super) fn observe_fx_aux(&mut self) -> Result<()> {
        let Some(control) = &self.fx_aux else {
            return Ok(());
        };
        let observation = control.prepare_observation();
        let (observation, present) =
            self.graph_command((observation, false), |graph, packet| {
                let Some(rack) = graph.aux.as_mut() else {
                    return;
                };
                #[cfg(test)]
                crate::remote::assert_no_allocations(|| rack.observe(&mut packet.0));
                #[cfg(not(test))]
                rack.observe(&mut packet.0);
                packet.1 = true;
            })?;
        if !present {
            return Err("FX source missing".into());
        }
        let frame = self.engine.frame();
        self.fx_aux
            .as_mut()
            .unwrap()
            .finish_observation(observation, frame)
    }
    #[cfg(feature = "hardware-host")]
    pub(super) fn poll_fx_aux(&mut self, now: u64) -> Result<()> {
        if self.fx_aux_pending.as_ref().is_none_or(|p| {
            self.engine
                .admission_frame()
                .unwrap_or_else(|| self.engine.frame())
                < p.frame
        }) {
            return Ok(());
        }
        let mut p = self.fx_aux_pending.take().unwrap();
        let control = self.fx_aux.as_ref().ok_or("FX aux owner lost")?;
        let b = &control.buses[p.mutation.bus];
        let mut sends = crate::mixer::PreparedFxSends::default();
        if let Some(config) = &p.config {
            sends.set(
                self.engine.topology().inputs.len(),
                p.mutation.bus,
                &config.sends,
            )?;
        }
        let mut command = PreparedCommand {
            bus: p.mutation.bus,
            panic: p.mutation.panic,
            owner_request: if p.token.is_some() {
                b.status.accepted_request.checked_add(1)
            } else {
                None
            },
            revision: b.status.revision,
            token: p.token.take(),
            sends,
            observation: control.prepare_observation(),
        };
        let result = if !self.engine.external_matches(&p.request, &p.fingerprint)
            || self
                .engine
                .admission_frame()
                .unwrap_or_else(|| self.engine.frame())
                != p.frame
        {
            self.engine.cancel_fx_external(&p.request, &p.fingerprint);
            Err("FX aux reservation lost".into())
        } else {
            let permit = self.prepare_source_external(now)?;
            let (_, returned, receipt) =
                self.graph_boundary_command(p.frame, (permit, command, None), |graph, packet| {
                    let Some(rack) = graph.aux.as_mut() else {
                        return;
                    };
                    #[cfg(test)]
                    crate::remote::assert_no_allocations(|| {
                        packet.2 = Some(rack.apply(&packet.0, &mut packet.1, &mut graph.mixer));
                    });
                    #[cfg(not(test))]
                    {
                        packet.2 = Some(rack.apply(&packet.0, &mut packet.1, &mut graph.mixer));
                    }
                })?;
            command = returned;
            let receipt = receipt.ok_or("FX source missing")?;
            let code = receipt.code;
            let outcome = self.engine.finish_external(receipt)?;
            if outcome.kind == "applied" && code == 0 {
                Ok(())
            } else {
                Err(outcome
                    .body
                    .reason
                    .unwrap_or_else(|| format!("FX source refused {code}")))
            }
        };
        let applied = result.is_ok();
        let control = self.fx_aux.as_mut().unwrap();
        // A refused command may not have entered Rack::apply. Observe validity
        // is explicit, so stale control state is retained instead of invented.
        if command.observation.valid.iter().all(|valid| *valid) {
            control.finish_observation(command.observation, self.engine.frame())?;
        }
        let b = &mut control.buses[p.mutation.bus];
        if applied {
            if let Some(c) = p.config {
                b.config = c;
            }
            if p.mutation.panic {
                b.owner_request = None;
                if b.config.settings != b.status.current {
                    b.config.settings = b.status.current;
                    b.config.tempo = None;
                }
            } else {
                b.owner_request = command.owner_request;
            }
            b.generation += 1;
            b.settle_frame = p.frame.saturating_add(240);
        }
        let reply = Self::fx_aux_reply(
            &p.request,
            if applied { "accepted" } else { "rejected" },
            result.err(),
            applied.then_some(p.frame),
        );
        if let Some(id) = p.client {
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
                let _ = c.queue_module(&reply, now);
            }
        } else if self.remote_completions.len() < 64 {
            self.remote_completions.push_back(reply);
        }
        Ok(())
    }
    #[cfg(feature = "hardware-host")]
    pub(super) fn revoke_fx_aux_writer(&mut self, writer: &str) {
        if self
            .fx_aux_pending
            .as_ref()
            .is_some_and(|p| p.request.writer.as_deref() == Some(writer))
        {
            let p = self.fx_aux_pending.take().unwrap();
            self.engine.cancel_fx_external(&p.request, &p.fingerprint);
            // Prepared token destruction stays on the controller.
        }
    }
    #[cfg(feature = "hardware-host")]
    pub(super) fn reset_fx_aux(&mut self) {
        if let Some(p) = self.fx_aux_pending.take() {
            self.engine.cancel_fx_external(&p.request, &p.fingerprint);
        }
        let Some(control) = &self.fx_aux else {
            return;
        };
        let mut observation = control.prepare_observation();
        let frame = self.engine.frame();
        // Startup/recovery may have explicitly reattached the whole graph. Do
        // not detach it in the middle of rebuilding its module/source epochs.
        if let Some(rack) = self.fx_aux.as_mut().and_then(|c| c.source.as_mut()) {
            rack.reset(frame);
            rack.observe(&mut observation);
        } else {
            let returned = self.graph_command((observation, frame, false), |graph, packet| {
                if let Some(rack) = graph.aux.as_mut() {
                    rack.reset(packet.1);
                    rack.observe(&mut packet.0);
                    packet.2 = true;
                }
            });
            let Ok((returned, _, true)) = returned else {
                return;
            };
            observation = returned;
        }
        let control = self.fx_aux.as_mut().unwrap();
        if control.finish_observation(observation, frame).is_err() {
            return;
        }
        for b in &mut control.buses {
            b.owner_request = None;
            if b.config.settings != b.status.current {
                b.config.settings = b.status.current;
                b.config.tempo = None;
            }
            b.generation = b.generation.saturating_add(1);
            b.applied = b.generation;
        }
    }
}
#[cfg(feature = "hardware-host")]
impl Rack {
    pub(super) fn observe(&mut self, observation: &mut Observation) {
        for (i, b) in self.buses.iter_mut().enumerate() {
            if let Ok(status) = b.owner.status_bounded() {
                observation.status[i] = status;
                observation.valid[i] = true;
            }
            b.owner.take_retired(&mut observation.retired[i]);
        }
    }
    fn reset(&mut self, frame: u64) {
        self.wet.fill(0.);
        self.next_frame = frame;
        for b in &mut self.buses {
            let _ = b.owner.reset_bounded();
            b.output.fill(0.);
        }
    }
    fn apply(
        &mut self,
        permit: &crate::mixer_control::source::ExternalPermit,
        command: &mut PreparedCommand,
        source: &mut crate::mixer_control::source::SourceState,
    ) -> crate::mixer_control::source::ExternalReceipt {
        let b = &mut self.buses[command.bus];
        let receipt = source.commit_external(permit, |mixer| {
            if command.panic {
                return if b.owner.reset_bounded().is_ok() {
                    0
                } else {
                    -20
                };
            }
            let Ok(status) = b.owner.status_bounded() else {
                return -21;
            };
            if status.revision != command.revision || status.transition != 0 {
                return -22;
            }
            if !mixer.accepts_fx_sends(&command.sends) {
                return -24;
            }
            if let Some(token) = &mut command.token {
                let Some(request) = command.owner_request else {
                    return -23;
                };
                if let Err(code) = b.owner.publish_token(token, request, command.revision) {
                    return code;
                }
            }
            let applied = mixer.apply_fx_sends(&mut command.sends);
            debug_assert!(applied);
            0
        });
        self.observe(&mut command.observation);
        receipt
    }

    pub(super) fn process(&mut self, frame: u64, mixer: &crate::mixer::Mixer) -> [f64; 96] {
        let r = self;
        r.wet.fill(0.);
        if r.next_frame != frame {
            for b in &mut r.buses {
                let _ = b.owner.reset_bounded();
                b.output.fill(0.);
            }
            r.next_frame = frame.saturating_add(48);
            return r.wet;
        }
        for (b, input) in r.buses.iter_mut().zip(mixer.fx_aux_samples()) {
            b.owner.process(input, &mut b.output);
            for (sum, wet) in r.wet.iter_mut().zip(&b.output) {
                *sum += wet;
            }
        }
        for (i, sample) in r.wet.iter_mut().enumerate() {
            let gain = mixer.output_safety_gain(frame + (i / 2) as u64);
            *sample = if gain == 0. { 0. } else { *sample * gain };
        }
        r.next_frame = frame.saturating_add(48);
        r.wet
    }
}
#[cfg(all(test, feature = "hardware-host"))]
mod actual_tests {
    use super::*;
    #[test]
    #[ignore = "GP_FX_LIBRARY points to actual native-f64 provider; software only"]
    fn actual_aux_authority_boundary_samples_failed_edit_and_panic() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;
        let path = std::path::PathBuf::from(std::env::var("GP_FX_LIBRARY").unwrap());
        let hash = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
        let dir = std::env::temp_dir().join(format!("gigpies-fx-aux-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let show = "11111111-1111-4111-8111-111111111111";
        let mut p = LocalAudio::bind_configured(
            &dir,
            "aux.sock",
            show,
            Counter(9),
            crate::topology::EngineTopology::software(4, 2, 0).unwrap(),
        )
        .unwrap();
        p.enable_fx_aux(&path, &hash).unwrap();
        p.rearm().unwrap();
        p.enable_persistent_source();
        let _media = p.source_executor_for(()).unwrap();
        let grant = crate::control_model::Request {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some("aux-test".into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant {
                scope: crate::control_model::Scope::FxConfiguration,
            },
        };
        let lease = p
            .engine
            .handle(&grant, 0)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap();
        let read = json!({"contract":wire::CONTRACT,"version":1,"show_id":show,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_aux_snapshot","body":{}});
        let snapshot = p.fx_aux_request(read.clone(), 0, false, None).unwrap();
        let mut c: Configuration =
            serde_json::from_str(snapshot["buses"][0]["configuration_json"].as_str().unwrap())
                .unwrap();
        c.sends[0] = 1.;
        c.settings.time_ms = 2.;
        c.settings.amount = 0.;
        c.settings.damping = 0.;
        c.settings.gain = 1.;
        let request = |id: u64,
                       generation: u64,
                       revision: Value,
                       config: Option<&Configuration>| json!({"contract":wire::CONTRACT,"version":1,"show_id":show,"module":"audio","epoch":"9","writer":"aux-test","lease":lease,"request_id":id.to_string(),"expected_revision":revision,"kind":"fx_aux_configure","body":{"bus":0,"map_revision":"1","library_sha256":hash,"expected_generation":generation.to_string(),"configuration_json":config.map(|c|serde_json::to_string(c).unwrap()),"panic":config.is_none()}});
        let r = request(2, 0, snapshot["revision"].clone(), Some(&c));
        assert_eq!(
            p.fx_aux_request(r.clone(), 0, true, None).unwrap()["state"],
            "prepared"
        );
        let mut playback = [0.; 48 * 4];
        let mut input = [0.; 48 * 4];
        for i in 0..48 {
            input[i * 4] = 0.125;
        }
        for tick in 0..32 {
            let frame = p.frame();
            p.tick_with_capture(tick, 9, frame, &input, &mut playback)
                .unwrap();
            if tick == 1 {
                assert!(p.render_graph.is_none());
                assert!(p.source_executor.is_some());
                assert!(p.fx_aux.as_ref().unwrap().source.is_none());
                let observed = p.fx_aux_request(read.clone(), tick, true, None).unwrap();
                assert_eq!(observed["buses"][0]["generation"], "1");
                assert_eq!(observed["buses"][0]["applied_generation"], "0");
                let owner: Value =
                    serde_json::from_str(observed["buses"][0]["owner_json"].as_str().unwrap())
                        .unwrap();
                assert_eq!(owner["accepted_request"], 1);
                assert_eq!(owner["applied_request"], 0);
                let replies = p.take_remote_completions();
                assert!(replies.iter().any(|r| r["state"] == "accepted"
                    && r["request_id"] == "2"
                    && r["effective_frame"] == "48"));
            }
        }
        let s = p.fx_aux_request(read.clone(), 32, true, None).unwrap();
        if let Ok(path) = std::env::var("GP_FX_AUX_FIXTURE") {
            std::fs::write(path, serde_json::to_vec_pretty(&s).unwrap()).unwrap();
        }
        assert_eq!(s["buses"][0]["generation"], "1");
        assert_eq!(s["buses"][0]["applied_generation"], "1");
        for i in 1..4 {
            assert_eq!(s["buses"][i]["generation"], "0");
        }
        let dry = 0.125 * 10_f64.powf(-6. / 20.) * std::f64::consts::FRAC_1_SQRT_2;
        for f in playback.chunks_exact(4) {
            assert!(
                (f[0] - 2. * dry).abs() < 1e-12,
                "Main dry + wet {} vs {}",
                f[0],
                2. * dry
            );
            assert!((f[1] - 2. * dry).abs() < 1e-12);
            assert!((f[2] - 0.125).abs() < 1e-12, "monitor unchanged");
        }
        assert_eq!(
            p.fx_aux_request(r, 33, true, None).unwrap()["state"],
            "rejected"
        );
        let mut bad = c.clone();
        bad.settings.amount = 3.;
        assert_eq!(
            p.fx_aux_request(
                request(3, 1, s["revision"].clone(), Some(&bad)),
                33,
                true,
                None
            )
            .unwrap()["state"],
            "rejected"
        );
        let after = p.fx_aux_request(read.clone(), 33, true, None).unwrap();
        assert_eq!(after["buses"], s["buses"]);
        assert_eq!(
            p.fx_aux_request(request(4, 1, s["revision"].clone(), None), 34, true, None)
                .unwrap()["state"],
            "prepared"
        );
        for tick in 34..42 {
            let frame = p.frame();
            p.tick_with_capture(tick, 9, frame, &[0.; 48 * 4], &mut playback)
                .unwrap();
        }
        assert!(playback.iter().all(|v| *v == 0.));
        // A finite over-range aux input faults the actual owner during a replacement.
        // Panic must remain reachable and retire the interrupted token, not deadlock on settlement.
        let s = p.fx_aux_request(read.clone(), 42, true, None).unwrap();
        let mut replacement = c.clone();
        replacement.settings = p.fx_aux.as_ref().unwrap().buses[0]
            .preparer
            .defaults(6)
            .unwrap();
        assert_eq!(
            p.fx_aux_request(
                request(5, 2, s["revision"].clone(), Some(&replacement)),
                42,
                true,
                None
            )
            .unwrap()["state"],
            "prepared"
        );
        let mut loud = [0.; 48 * 4];
        for f in 0..48 {
            loud[f * 4] = 100.;
        }
        for tick in 42..45 {
            let frame = p.frame();
            p.tick_with_capture(tick, 9, frame, &loud, &mut playback)
                .unwrap();
        }
        let s = p.fx_aux_request(read.clone(), 45, true, None).unwrap();
        let status: Value =
            serde_json::from_str(s["buses"][0]["owner_json"].as_str().unwrap()).unwrap();
        assert_ne!(status["fault"], 0);
        assert_eq!(
            p.fx_aux_request(request(6, 3, s["revision"].clone(), None), 45, true, None)
                .unwrap()["state"],
            "prepared"
        );
        for tick in 45..54 {
            let frame = p.frame();
            p.tick_with_capture(tick, 9, frame, &[0.; 48 * 4], &mut playback)
                .unwrap();
        }
        let s = p.fx_aux_request(read.clone(), 54, true, None).unwrap();
        assert_eq!(s["buses"][0]["applied_generation"], "4");
        let status: Value =
            serde_json::from_str(s["buses"][0]["owner_json"].as_str().unwrap()).unwrap();
        assert_eq!(status["fault"], 0);
        p.quiesce_source("test source loss").unwrap();
        p.graph_command((), |g, _| {
            assert!(g.aux.as_ref().unwrap().wet.iter().all(|v| *v == 0.));
        })
        .unwrap();
        p.recover_source(Counter(10), 0).unwrap();
        assert!(p.engine.outputs_quiesced());
        let recovered_sends = p.fx_aux.as_ref().unwrap().buses[0].config.sends.clone();
        p.graph_command((), |graph, _| {
            assert_eq!(graph.aux.as_ref().unwrap().next_frame, 0)
        })
        .unwrap();
        p.rearm().unwrap();
        p.graph_command(recovered_sends, |graph, recovered_sends| {
            graph.mixer.mixer.observe_into(&mut graph.mixer.observation);
            for (row, expected) in graph.mixer.observation.fx_sends.iter().zip(recovered_sends) {
                assert_eq!(row[0], *expected);
            }
        })
        .unwrap();
        for tick in 54..86 {
            let frame = p.frame();
            p.tick_with_capture(tick, 10, frame, &input, &mut playback)
                .unwrap();
        }
        for f in playback.chunks_exact(4) {
            assert!((f[0] - 2. * dry).abs() < 1e-12, "recovered dry + wet");
        }
        drop(p);
    }
}
