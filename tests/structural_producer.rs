//! Explicit software producer evidence with trusted actual owner libraries.
//! GP05_MANIFEST, GP_PA_V2_FIXTURES, GP14_PRODUCER_DIR are required.
#![cfg(feature = "hardware-host")]
use gigpies::{
    control_model::{Command, Request, Scope},
    local_audio::LocalAudio,
    show::Counter,
    structural_control::{self, Command as S},
    topology::{EngineTopology, OutputSource},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
struct Pump {
    host: LocalAudio,
    now: u64,
    exchanges: Vec<Value>,
}
impl Pump {
    fn snapshot_reply(&mut self) {
        let request = structural_control::Request {
            contract: "GP14-structure".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(self.host.source_epoch()),
            writer: None,
            lease: None,
            request_id: None,
            expected_revision: None,
            command: S::StructuralSnapshot {},
        };
        let reply = self
            .host
            .structural_request(request.clone(), self.now, false, None)
            .unwrap();
        assert_eq!(reply.state, "snapshot");
        assert!(reply.snapshot.is_some());
        self.exchanges
            .push(json!({"request":request,"reply":reply}));
    }

    fn block(&mut self) -> Vec<f64> {
        self.now += 1;
        let t = self.host.topology();
        let mut capture = vec![0.; 48 * t.capture_channels];
        for f in 0..48 {
            for (i, p) in t.inputs.iter().enumerate() {
                let phase =
                    (self.host.frame() + f as u64) as f64 * (173. + i as f64 * 19.) / 48000.;
                capture[f * t.capture_channels + p.capture_slot] =
                    (0.005 * (phase * std::f64::consts::TAU).sin() * 8388608.).round() / 8388608.;
            }
        }
        let mut out = vec![0.; 48 * t.playback_channels];
        self.host
            .tick_with_capture(
                self.now,
                self.host.source_epoch(),
                self.host.frame(),
                &capture,
                &mut out,
            )
            .unwrap();
        out
    }
    fn grant(&mut self, writer: &str, scope: Scope) -> Counter {
        let r = Request {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(self.host.source_epoch()),
            writer: Some(writer.into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(self.host.engine_mut().revision()),
            command: Command::Grant { scope },
        };
        self.host
            .engine_mut()
            .handle(&r, self.now)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap()
    }
    fn command(
        &mut self,
        writer: &str,
        lease: Counter,
        id: u64,
        command: S,
    ) -> structural_control::Reply {
        let revision = self.host.engine_mut().revision();
        let r = structural_control::Request {
            contract: "GP14-structure".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(self.host.source_epoch()),
            writer: Some(writer.into()),
            lease: Some(lease),
            request_id: Some(Counter(id)),
            expected_revision: Some(revision),
            command,
        };
        let pending = self
            .host
            .structural_request(r.clone(), self.now, true, None)
            .unwrap();
        assert_eq!(pending.state, "pending");
        self.exchanges.push(json!({"request":r,"reply":pending}));
        self.block();
        assert_eq!(self.host.engine_mut().revision(), revision);
        self.block();
        let final_reply = self
            .host
            .structural_request(r.clone(), self.now, false, None)
            .unwrap();
        assert_eq!(final_reply.state, "final");
        if final_reply.reason.is_none() {
            assert_eq!(final_reply.effective_frame, pending.effective_frame);
        }
        self.exchanges
            .push(json!({"request":r,"reply":final_reply}));
        final_reply
    }
    fn settle(&mut self) -> Vec<f64> {
        let mut out = vec![];
        for _ in 0..12 {
            out = self.block();
        }
        out
    }
}
fn hash(bytes: &[u8]) -> String {
    gigpies::hash::hex(Sha256::digest(bytes))
}
#[test]
#[ignore = "requires explicit trusted owner manifest, fixtures and private producer output directory"]
fn actual_structural_producer_16_32_48() {
    let manifest = PathBuf::from(std::env::var_os("GP05_MANIFEST").expect("GP05_MANIFEST"));
    let fixture = PathBuf::from(std::env::var_os("GP_PA_V2_FIXTURES").expect("GP_PA_V2_FIXTURES"))
        .join("stereo4way.json");
    let destination =
        PathBuf::from(std::env::var_os("GP14_PRODUCER_DIR").expect("GP14_PRODUCER_DIR"));
    std::fs::create_dir_all(&destination).unwrap();
    let original = std::fs::read(&fixture).unwrap();
    let mut changed: Value = serde_json::from_slice(&original).unwrap();
    for output in changed["outputs"].as_array_mut().unwrap() {
        output["processing"]["gain_db"] = json!(-6.);
    }
    let changed = serde_json::to_string(&changed).unwrap();
    let mut evidence = vec![];
    for inputs in [16, 32, 48] {
        let dir =
            std::env::temp_dir().join(format!("gp14-producer-{}-{inputs}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut topology = if inputs == 16 {
            EngineTopology::reference_16_18(8, 5).unwrap()
        } else {
            EngineTopology::software(inputs, 5, 8).unwrap()
        };
        for p in &mut topology.outputs {
            p.source = None;
        }
        for (i, p) in topology.outputs.iter_mut().take(8).enumerate() {
            p.source = Some(OutputSource::Pa { index: i });
        }
        let mut host =
            LocalAudio::bind_configured(&dir, "audio", SHOW, Counter(1), topology).unwrap();
        host.enable_modules(&manifest).unwrap();
        host.configure_pa(&original).unwrap();
        let mut p = Pump {
            host,
            now: 0,
            exchanges: Vec::new(),
        };
        p.snapshot_reply();
        let pa = p.grant("pa-desk", Scope::PaConfiguration);
        let routes = p.grant("route-desk", Scope::OutputRoutes);
        assert!(
            p.command("pa-desk", pa, 2, S::OutputRearm {})
                .reason
                .is_none()
        );
        let before = p.settle();
        assert!(before.iter().any(|x| x.abs() > 1e-8));
        assert!(
            p.command("pa-desk", pa, 3, S::OutputMute {})
                .reason
                .is_none()
        );
        assert!(p.settle().iter().all(|x| *x == 0.));
        assert!(p.host.structural_snapshot().unwrap().outputs_quiesced);
        assert!(
            p.command(
                "pa-desk",
                pa,
                4,
                S::PaSet {
                    configuration_json: changed.clone(),
                    program_buses: vec![2, 3]
                }
            )
            .reason
            .is_none()
        );
        let snapshot = p.host.structural_snapshot().unwrap();
        assert_eq!(
            snapshot.pa_configuration_json.as_deref(),
            Some(changed.as_str())
        );
        assert_eq!(snapshot.pa_program_buses, vec![2, 3]);
        assert!(p.block().iter().all(|x| *x == 0.));
        assert!(
            p.command("pa-desk", pa, 5, S::OutputRearm {})
                .reason
                .is_none()
        );
        let after = p.settle();
        assert!(after.iter().any(|x| x.abs() > 1e-8));
        assert_ne!(before, after);
        assert!(
            p.command("pa-desk", pa, 6, S::OutputMute {})
                .reason
                .is_none()
        );
        p.settle();
        let mut outputs = p.host.topology().outputs.clone();
        outputs[0].source = Some(OutputSource::Pa { index: 1 });
        outputs[1].source = Some(OutputSource::Pa { index: 0 });
        assert!(
            p.command(
                "route-desk",
                routes,
                2,
                S::OutputPatch {
                    outputs: outputs.clone()
                }
            )
            .reason
            .is_none()
        );
        assert_eq!(p.host.topology().outputs, outputs);
        let revision = p.host.engine_mut().revision();
        let mut invalid = outputs.clone();
        invalid[1].playback_slot = invalid[0].playback_slot;
        assert!(
            p.command("route-desk", routes, 3, S::OutputPatch { outputs: invalid })
                .reason
                .is_some()
        );
        assert_eq!(p.host.topology().outputs, outputs);
        assert_eq!(p.host.engine_mut().revision(), revision);
        assert!(
            p.command("pa-desk", pa, 7, S::OutputRearm {})
                .reason
                .is_none()
        );
        let patched = p.settle();
        assert!(patched.iter().any(|x| x.abs() > 1e-8));
        let capture = vec![0.; 48 * p.host.topology().capture_channels];
        let mut playback = vec![1.; 48 * p.host.topology().playback_channels];
        assert!(
            p.host
                .tick_with_capture(p.now, 1, p.host.frame() + 1, &capture, &mut playback)
                .is_err()
        );
        assert!(playback.iter().all(|x| *x == 0.));
        p.host.recover_source(Counter(9), 0).unwrap();
        assert!(p.block().iter().all(|x| *x == 0.));
        let recovered = p.host.structural_snapshot().unwrap();
        assert!(recovered.outputs_quiesced);
        assert_eq!(
            recovered.pa_configuration_json.as_deref(),
            Some(changed.as_str())
        );
        let lease = p.grant("recovered-desk", Scope::PaConfiguration);
        assert!(
            p.command("recovered-desk", lease, 2, S::OutputRearm {})
                .reason
                .is_none()
        );
        assert!(p.settle().iter().any(|x| x.abs() > 1e-8));
        p.snapshot_reply();
        let snapshot = p.host.structural_snapshot().unwrap();
        assert!(snapshot.pa_capabilities.is_some());
        assert!(snapshot.pa_status.is_some());
        let bytes = serde_json::to_vec_pretty(&snapshot).unwrap();
        let filename = format!("structure-{inputs}.json");
        std::fs::write(destination.join(&filename), &bytes).unwrap();
        assert!(p.exchanges.iter().any(|e| e["reply"]["state"] == "pending"));
        assert!(
            p.exchanges
                .iter()
                .any(|e| e["reply"]["state"] == "final" && e["reply"]["reason"].is_null())
        );
        assert!(
            p.exchanges
                .iter()
                .any(|e| e["reply"]["state"] == "final" && e["reply"]["reason"].is_string())
        );
        let reply_filename = format!("structure-replies-{inputs}.json");
        let reply_bytes =
            serde_json::to_vec_pretty(&json!({"inputs":inputs,"exchanges":p.exchanges})).unwrap();
        std::fs::write(destination.join(&reply_filename), &reply_bytes).unwrap();
        evidence.push(json!({"replies":reply_filename,"replies_sha256":hash(&reply_bytes),"inputs":inputs,"snapshot":filename,"sha256":hash(&bytes),"before_samples":before,"after_samples":after,"patched_samples":patched,"hardware_verified":false}));
        drop(p);
        std::fs::remove_dir_all(dir).unwrap();
    }
    let mut pins = serde_json::Map::new();
    for name in [
        "src/local_audio.rs",
        "src/module_graph.rs",
        "src/host/pa_v2.rs",
        "src/structural_control.rs",
        "src/mixer.rs",
        "src/mixer_control.rs",
        "src/topology.rs",
        "tests/structural_producer.rs",
    ] {
        pins.insert(
            name.into(),
            json!(hash(
                &std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)).unwrap()
            )),
        );
    }
    let report = json!({"software_only":true,"manifest_sha256":hash(&std::fs::read(manifest).unwrap()),"owner_fixture_sha256":hash(&original),"source_sha256":pins,"evidence":evidence});
    std::fs::write(
        destination.join("structure-manifest.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
