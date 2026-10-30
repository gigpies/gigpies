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
pub(super) struct Bus {
    owner: Owner,
    generation: u64,
    applied: u64,
    settle_frame: u64,
    owner_request: Option<u64>,
    config: Configuration,
    output: [f64; 96],
}
#[cfg(feature = "hardware-host")]
pub(super) struct Rack {
    buses: [Bus; 4],
    hash: String,
    wet: [f64; 96],
    next_frame: u64,
    _library_file: std::fs::File,
}
#[cfg(feature = "hardware-host")]
pub(super) struct Pending {
    request: crate::control_model::Request,
    fingerprint: String,
    mutation: wire::Mutation,
    config: Option<Configuration>,
    prepared: bool,
    frame: u64,
    client: Option<u64>,
}
impl LocalAudio {
    #[cfg(feature = "hardware-host")]
    pub fn enable_fx_aux(&mut self, path: &Path, expected_hash: &str) -> Result<()> {
        use sha2::{Digest, Sha256};
        use std::os::fd::AsRawFd;
        if self.frame() != 0 || self.fx_aux.is_some() || self.remote_fx_aux || !path.is_absolute() {
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
        for _ in 0..4 {
            let owner = Owner::load(&pinned)?;
            let config = Configuration {
                settings: owner.defaults(1)?,
                sends: vec![0.; self.topology().inputs.len()],
                tempo: None,
            };
            buses.push(Bus {
                owner,
                config,
                generation: 0,
                applied: 0,
                settle_frame: 0,
                owner_request: None,
                output: [0.; 96],
            });
        }
        self.engine.enable_fx_aux()?;
        self.fx_aux = Some(Rack {
            buses: buses.try_into().map_err(|_| "FX buses")?,
            hash,
            wet: [0.; 96],
            next_frame: 0,
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
                bus.owner.reclaim();
                let status = bus.owner.status()?;
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
                            || (bus.owner.delay_time(t.bpm, t.denominator, t.modifier)?
                                - c.settings.time_ms)
                                .abs()
                                > 1e-9)
                    {
                        return Err("FX tempo/time mismatch".into());
                    }
                    // Validation/preparation precedes any reservation or sounding change.
                    bus.owner.prepare(&c.settings)?;
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
                        bus.owner.cancel();
                        return Err(reply
                            .body
                            .reason
                            .unwrap_or("request already completed; refresh".into()));
                    }
                    Err(e) => {
                        bus.owner.cancel();
                        return Err(e);
                    }
                };
                if frame.checked_add(240).is_none() {
                    bus.owner.cancel();
                    self.engine.cancel_fx_external(&r, &fingerprint);
                    return Err("FX source frame exhausted".into());
                }
                let prepared = config
                    .as_ref()
                    .is_some_and(|c| c.settings != bus.config.settings);
                if !prepared {
                    bus.owner.cancel();
                }
                self.fx_aux_pending = Some(Pending {
                    request: r.clone(),
                    fingerprint,
                    mutation: m,
                    config,
                    prepared,
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
                let status = bus.owner.status()?;
                buses.push(json!({"id":format!("fx-{}",i+1),"generation":bus.generation.to_string(),
                    "applied_generation":bus.applied.to_string(),"configuration_json":serde_json::to_string(&bus.config).map_err(|e|e.to_string())?,
                    "owner_json":serde_json::to_string(&status).map_err(|e|e.to_string())?}));
            }
            let mut palette = Vec::new();
            let owner = &rack.buses[0].owner;
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
    #[cfg(feature = "hardware-host")]
    pub(super) fn poll_fx_aux(&mut self, now: u64) -> Result<()> {
        if let Some(rack) = &mut self.fx_aux {
            for b in &mut rack.buses {
                b.owner.reclaim();
                let status = b.owner.status()?;
                if self.engine.frame() >= b.settle_frame
                    && status.transition == 0
                    && b.owner_request.is_none_or(|r| r == status.applied_request)
                    && status.current == b.config.settings
                {
                    b.applied = b.generation;
                }
            }
        }
        if self
            .fx_aux_pending
            .as_ref()
            .is_none_or(|p| self.engine.frame() < p.frame)
        {
            return Ok(());
        }
        let p = self.fx_aux_pending.take().unwrap();
        let rack = self.fx_aux.as_mut().ok_or("FX aux owner lost")?;
        let b = &mut rack.buses[p.mutation.bus];
        let mut applied = false;
        let result = if !self.engine.external_matches(&p.request, &p.fingerprint)
            || self.engine.frame() != p.frame
        {
            self.engine.cancel_fx_external(&p.request, &p.fingerprint);
            Err("FX aux reservation lost".into())
        } else {
            let outcome = self.engine.commit_external_with_engine(now, |engine| {
                let status = b.owner.status()?;
                if p.mutation.panic {
                    b.owner.reset()?;
                    b.owner_request = None;
                    let current = b.owner.status()?.current;
                    if b.config.settings != current {
                        b.config.settings = current;
                        b.config.tempo = None;
                    }
                } else {
                    if p.prepared {
                        let request = status
                            .accepted_request
                            .checked_add(1)
                            .ok_or("FX request exhausted")?;
                        b.owner.publish(request, status.revision)?;
                        b.owner_request = Some(request);
                    }
                    let c = p.config.as_ref().ok_or("FX config missing")?;
                    engine.set_fx_sends(p.mutation.bus, &c.sends)?;
                }
                applied = true;
                Ok(())
            })?;
            if applied {
                Ok(())
            } else {
                Err(outcome.body.reason.unwrap_or("FX authority refused".into()))
            }
        };
        b.owner.cancel();
        if applied {
            if let Some(c) = p.config {
                b.config = c;
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
            if let Some(rack) = &mut self.fx_aux {
                rack.buses[p.mutation.bus].owner.cancel();
            }
        }
    }
    #[cfg(feature = "hardware-host")]
    pub(super) fn reset_fx_aux(&mut self) {
        if let Some(p) = self.fx_aux_pending.take() {
            self.engine.cancel_fx_external(&p.request, &p.fingerprint);
        }
        if let Some(r) = &mut self.fx_aux {
            r.wet.fill(0.);
            r.next_frame = self.engine.frame();
            for b in &mut r.buses {
                b.owner.cancel();
                let _ = b.owner.reset();
                b.owner_request = None;
                if let Ok(status) = b.owner.status()
                    && b.config.settings != status.current
                {
                    b.config.settings = status.current;
                    b.config.tempo = None;
                }
                b.owner.reclaim();
                b.generation = b.generation.saturating_add(1);
                b.applied = b.generation;
                b.output.fill(0.);
            }
        }
    }
    #[cfg(feature = "hardware-host")]
    pub(super) fn process_fx_aux(&mut self, frame: u64) -> Option<[f64; 96]> {
        let r = self.fx_aux.as_mut()?;
        r.wet.fill(0.);
        if r.next_frame != frame {
            for b in &mut r.buses {
                let _ = b.owner.reset();
                b.output.fill(0.);
            }
            r.next_frame = frame.saturating_add(48);
            return Some(r.wet);
        }
        for (b, input) in r.buses.iter_mut().zip(self.engine.mixer().fx_aux_samples()) {
            b.owner.process(input, &mut b.output);
            for (sum, wet) in r.wet.iter_mut().zip(&b.output) {
                *sum += wet;
            }
        }
        for (i, sample) in r.wet.iter_mut().enumerate() {
            let gain = self.engine.output_safety_gain(frame + (i / 2) as u64);
            *sample = if gain == 0. { 0. } else { *sample * gain };
        }
        r.next_frame = frame.saturating_add(48);
        Some(r.wet)
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
            .owner
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
        assert!(p.fx_aux.as_ref().unwrap().wet.iter().all(|v| *v == 0.));
        p.recover_source(Counter(10), 0).unwrap();
        assert!(p.engine.outputs_quiesced());
        assert_eq!(
            p.engine.mixer().fx_sends(0).unwrap(),
            p.fx_aux.as_ref().unwrap().buses[0].config.sends
        );
        assert_eq!(p.fx_aux.as_ref().unwrap().next_frame, 0);
        p.rearm().unwrap();
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
