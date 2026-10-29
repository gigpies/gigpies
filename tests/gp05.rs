use gigpies::{
    control_model::{Command, Request, Scope},
    mixer_control::OfflineEngine,
    module_wire::{ModuleCommand, ModuleRequest},
    show::Counter,
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
fn request(kind: Command, n: u64, lease: Option<Counter>) -> Request {
    Request {
        contract: "C-AUDIO".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: Some("gp05-writer".into()),
        lease,
        request_id: Some(Counter(n)),
        expected_revision: Some(Counter(0)),
        command: kind,
    }
}
#[test]
fn module_codec_is_separate_strict_and_decimal() {
    let r = ModuleRequest::decode(include_bytes!("fixtures/gp05/v1/status-request.json")).unwrap();
    assert!(matches!(r.command, ModuleCommand::ModuleStatus {}));
    assert!(Request::decode(&r.encode().unwrap()).is_err());
    let mut v: serde_json::Value = serde_json::from_slice(&r.encode().unwrap()).unwrap();
    v["extra"] = true.into();
    assert!(ModuleRequest::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    v.as_object_mut().unwrap().remove("extra");
    v["epoch"] = 9.into();
    assert!(ModuleRequest::decode(&serde_json::to_vec(&v).unwrap()).is_err());
}
#[test]
fn shared_authority_rejects_cross_contract_reuse_and_fences_retries() {
    let mut engine = OfflineEngine::new(SHOW, Counter(9), Counter(0), 0).unwrap();
    let grant = engine
        .handle(&request(Command::Grant { scope: Scope::Foh }, 1, None), 0)
        .unwrap();
    let lease = grant.outcome.unwrap().body.granted_lease;
    let r = request(Command::Renew {}, 2, lease);
    assert_eq!(
        engine.authorize_module(&r, "start-take-op17", 1).unwrap(),
        (false, None)
    );
    assert_eq!(
        engine.authorize_module(&r, "start-take-op17", 2).unwrap(),
        (true, None)
    );
    assert!(engine.authorize_module(&r, "stop-take-op17", 3).is_err());
    assert_eq!(
        engine
            .handle(&r, 4)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .reason
            .as_deref(),
        Some("reused_id")
    );
    let mut refused = request(Command::Renew {}, 3, lease);
    refused.expected_revision = Some(Counter(99));
    assert_eq!(
        engine.authorize_module(&refused, "start-stale", 5).unwrap(),
        (false, Some("stale_revision".into()))
    );
    assert_eq!(
        engine.authorize_module(&refused, "start-stale", 6).unwrap(),
        (true, Some("stale_revision".into()))
    );
    assert!(
        engine
            .authorize_module(&refused, "changed-stale", 7)
            .is_err()
    );
    assert_eq!(
        engine
            .handle(&refused, 8)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .reason
            .as_deref(),
        Some("reused_id")
    );
    let normal = request(Command::Renew {}, 4, lease);
    assert_eq!(
        engine.handle(&normal, 9).unwrap().outcome.unwrap().kind,
        "applied"
    );
    assert!(engine.authorize_module(&normal, "start", 10).is_err());
    let mut wrong = r.clone();
    wrong.epoch = Counter(8);
    assert!(
        engine
            .authorize_module(&wrong, "start-take-op17", 7)
            .is_err()
    );
    assert!(
        engine
            .authorize_module(&r, "start-take-op17", 3000)
            .is_err()
    );
}
#[test]
fn monitor_lease_cannot_record() {
    let mut engine = OfflineEngine::new(SHOW, Counter(9), Counter(0), 0).unwrap();
    let lease = engine
        .handle(
            &request(
                Command::Grant {
                    scope: Scope::Monitor1,
                },
                1,
                None,
            ),
            0,
        )
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease;
    assert!(
        engine
            .authorize_module(&request(Command::Renew {}, 2, lease), "start", 1)
            .is_err()
    );
}
#[cfg(target_os = "linux")]
#[test]
fn default_service_explicitly_reports_modules_unavailable() {
    use std::os::unix::fs::PermissionsExt;
    let p = std::env::temp_dir().join(format!("gp05-default-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    {
        let mut s =
            gigpies::local_audio::LocalAudio::bind(&p, "audio.sock", SHOW, Counter(9)).unwrap();
        let v = s.module_status(0).unwrap();
        let fixture: serde_json::Value =
            serde_json::from_slice(include_bytes!("fixtures/gp05/v1/status-unavailable.json"))
                .unwrap();
        assert_eq!(v, fixture);
        assert_eq!(v["available"], false);
        assert_eq!(v["readiness"], "unavailable");
        assert_eq!(v["physical"], "unverified");
        assert!(v["pa"].is_null());
    }
    std::fs::remove_dir_all(p).unwrap();
}
#[cfg(feature = "hardware-host")]
mod actual {
    use super::*;
    use gigpies::module_graph::{Manifest, ModuleGraph, ProcessError};
    use std::{path::PathBuf, time::Duration};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            let p = std::env::temp_dir().join(format!(
                "gp05-owner-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self(p)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn manifest() -> Manifest {
        Manifest::load(&PathBuf::from(
            std::env::var("GP05_MANIFEST").expect("explicit accepted owner manifest required"),
        ))
        .unwrap()
    }
    fn ready(g: &mut ModuleGraph) {
        for _ in 0..1000 {
            g.poll_lifecycle().unwrap();
            if g.status(SHOW, 0)
                .unwrap()
                .recording
                .as_ref()
                .is_some_and(|t| t.state == "recording")
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("prepare timeout");
    }
    fn finished(g: &mut ModuleGraph) {
        for _ in 0..1000 {
            g.poll_lifecycle().unwrap();
            if g.status(SHOW, 0)
                .unwrap()
                .recording
                .as_ref()
                .is_some_and(|t| t.state == "finalized")
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("finish timeout");
    }
    #[test]
    fn abi_layouts_match_frozen_headers() {
        use gigpies::host::adapters::*;
        assert_eq!(std::mem::size_of::<FxCapabilities>(), 112);
        assert_eq!(std::mem::size_of::<FxStatus>(), 40);
        assert_eq!(std::mem::size_of::<PaDescriptor>(), 80);
        assert_eq!(std::mem::size_of::<PaStatus>(), 24);
        assert_eq!(std::mem::size_of::<RecorderProgress>(), 96);
        let fixture: gigpies::module_graph::ModuleStatus =
            serde_json::from_slice(include_bytes!("fixtures/gp05/v1/status-finalized.json"))
                .unwrap();
        assert_eq!(fixture.recording.unwrap().written_frames, Counter(1440));
    }
    #[test]
    #[ignore = "explicit accepted owner libraries; finite synthetic REC storage/finalizer checks"]
    fn recording_storage_refusal_and_finalizer_progress_remain_truthful() {
        let dir = Directory::new();
        let mut g = ModuleGraph::load(manifest(), 9, 0).unwrap();
        std::fs::create_dir(dir.0.join("occupied")).unwrap();
        g.start(&dir.0, "occupied", Counter(70)).unwrap();
        finished(&mut g);
        assert_eq!(
            g.status(SHOW, 100).unwrap().recording.unwrap().outcome,
            "error"
        );
        g.start(&dir.0, "publish-collision", Counter(71)).unwrap();
        ready(&mut g);
        for frame in (0..480).step_by(48) {
            g.process(9, frame, &[[0.125; 8]; 48], &[[0.; 4]; 48])
                .unwrap();
        }
        // Create-new publication must preserve this unrelated file and report
        // an actual owner finalization failure, even if all PCM was written.
        std::fs::write(dir.0.join("publish-collision/channel-001.wav"), b"preserve").unwrap();
        g.stop("publish-collision", Counter(71)).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        let draining = g.status(SHOW, 200).unwrap().recording.unwrap();
        assert_eq!(draining.state, "finalizing");
        assert_eq!(draining.outcome, "pending");
        assert_eq!(draining.written_frames, Counter(480));
        assert!(draining.durable_frames.is_none());
        finished(&mut g);
        for now in [300, 400, 500] {
            let failed = g.status(SHOW, now).unwrap().recording.unwrap();
            assert_eq!(failed.state, "finalized");
            assert_eq!(failed.outcome, "error");
            assert!(failed.writer_fault);
        }
        assert_eq!(
            std::fs::read(dir.0.join("publish-collision/channel-001.wav")).unwrap(),
            b"preserve"
        );
    }
    #[test]
    #[ignore = "requires explicitly supplied independently accepted owner libraries"]
    fn accepted_owner_graph_exact_pcm_lifecycle_faults_and_discontinuity() {
        let dir = Directory::new();
        let mut g = ModuleGraph::load(manifest(), 9, 0).unwrap();
        let before = g.status(SHOW, 0).unwrap();
        assert_eq!(before.fx.unwrap().status.intentional_delay_frames, 960);
        assert_eq!(before.pa.unwrap().descriptor.unavailable_capabilities, 15);
        g.start(&dir.0, "take-01", Counter(17)).unwrap();
        assert!(g.start(&dir.0, "take-02", Counter(18)).is_err());
        ready(&mut g);
        let mut expected = Vec::new();
        let mut mixer = gigpies::mixer::Mixer::default();
        let mut first = 0;
        for _ in 0..30 {
            let raw = gigpies::analysis_stream::synthetic_inputs(first)
                .map(|f| f.map(|x| f64::from(x) / 8388608.));
            expected.extend(raw);
            let mut mixed = [[0.; 4]; 48];
            mixer.process(&raw, &mut mixed).unwrap();
            g.process(9, first, &raw, &mixed).unwrap();
            assert!(
                g.output()
                    .chunks_exact(6)
                    .all(|f| f[2..].iter().all(|v| *v == 0.))
            );
            assert!(g.output().iter().all(|v| v.abs() <= 10f64.powf(-1. / 20.)));
            first += 48;
        }
        assert!(g.stop("take-01", Counter(16)).is_err());
        g.stop("take-01", Counter(17)).unwrap();
        finished(&mut g);
        let terminal = g.status(SHOW, 100).unwrap();
        let rec = terminal.recording.as_ref().unwrap();
        assert_eq!(rec.outcome, "complete");
        assert_eq!(rec.accepted_frames, Counter(first));
        assert_eq!(rec.written_frames, Counter(first));
        assert_eq!(rec.durable_frames, None);
        // Inspect exact owner-produced PCM24, not packet or journal counts.
        let take = dir.0.join("take-01");
        let mut wavs: Vec<_> = std::fs::read_dir(&take)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "wav"))
            .collect();
        wavs.sort();
        assert_eq!(wavs.len(), 8);
        for (ch, path) in wavs.iter().enumerate() {
            let mut wav = hound::WavReader::open(path).unwrap();
            assert_eq!(wav.spec().bits_per_sample, 24);
            let actual: Vec<_> = wav.samples::<i32>().map(|x| x.unwrap()).collect();
            let wanted: Vec<_> = expected
                .iter()
                .map(|f| (f[ch] * 8388608.).round() as i32)
                .collect();
            assert_eq!(actual, wanted);
        }
        if let Ok(p) = std::env::var("GP05_HEALTH_EVIDENCE") {
            std::fs::write(p, serde_json::to_vec_pretty(&terminal).unwrap()).unwrap();
        }
        assert_eq!(
            g.process(9, 0, &[[0.; 8]; 48], &[[0.; 4]; 48]),
            Err(ProcessError::Timeline)
        );
        g.discontinuity(9, first).unwrap();
        let retained = g.status(SHOW, 100).unwrap().recording.unwrap();
        assert_eq!(retained.state, "finalized");
        assert_eq!(retained.outcome, "complete");
        g.start(&dir.0, "cancelled", Counter(18)).unwrap();
        g.stop("cancelled", Counter(18)).unwrap();
        finished(&mut g);
        assert_ne!(
            g.status(SHOW, 200).unwrap().recording.unwrap().outcome,
            "complete"
        );
        g.start(&dir.0, "prepare-discontinuity", Counter(19))
            .unwrap();
        g.discontinuity(10, 2400).unwrap();
        finished(&mut g);
        assert_eq!(
            g.status(SHOW, 250).unwrap().recording.unwrap().outcome,
            "incomplete"
        );
        g.start(&dir.0, "invalid-source", Counter(20)).unwrap();
        ready(&mut g);
        assert_eq!(
            g.process(10, 2400, &[[0.1; 8]; 48], &[[0.; 4]; 48]),
            Err(ProcessError::Source)
        );
        g.stop("invalid-source", Counter(20)).unwrap();
        finished(&mut g);
        assert_eq!(
            g.status(SHOW, 300).unwrap().recording.unwrap().outcome,
            "incomplete"
        );
        g.discontinuity(10, 2400).unwrap();
        g.start(&dir.0, "prepare-source-error", Counter(21))
            .unwrap();
        assert_eq!(
            g.process(10, 2400, &[[f64::NAN; 8]; 48], &[[0.; 4]; 48]),
            Err(ProcessError::Source)
        );
        for attempt in 0..1000 {
            g.poll_lifecycle().unwrap();
            let state = g
                .status(SHOW, 300 + attempt * 100)
                .unwrap()
                .recording
                .unwrap();
            assert_ne!(
                state.state, "recording",
                "cancelled preparation reactivated"
            );
            assert_eq!(state.first_source_frame, None);
            if state.state == "finalized" {
                break;
            }
            assert!(attempt < 999, "cancelled preparation finish timeout");
            std::thread::sleep(Duration::from_millis(1));
        }
        let rejected = g.status(SHOW, 300).unwrap().recording.unwrap();
        assert_eq!(rejected.host_fault, 7);
        assert_eq!(rejected.outcome, "incomplete");
        g.discontinuity(10, 2400).unwrap();
        g.start(&dir.0, "epoch-change", Counter(22)).unwrap();
        ready(&mut g);
        g.discontinuity(10, 4800).unwrap();
        finished(&mut g);
        assert_eq!(
            g.status(SHOW, 300).unwrap().recording.unwrap().outcome,
            "incomplete"
        );
        let raw = [[0.; 8]; 48];
        let bad = [[f64::NAN, 0., 0., 0.]; 48];
        assert!(matches!(
            g.process(10, 4800, &raw, &bad),
            Err(ProcessError::Pa(-2))
        ));
        assert_eq!(g.status(SHOW, 400).unwrap().readiness, "faulted");
        assert!(g.output().iter().all(|v| *v == 0.));
        g.discontinuity(11, 0).unwrap();
        g.process(11, 0, &raw, &[[0.; 4]; 48]).unwrap();
        assert_eq!(g.status(SHOW, 500).unwrap().readiness, "ready");
        assert_eq!(
            g.process(11, 48, &raw, &[[17., 0., 0., 0.]; 48]),
            Err(ProcessError::Fx(-3))
        );
        let degraded = g.status(SHOW, 600).unwrap();
        assert_eq!(degraded.readiness, "degraded");
        assert_eq!(degraded.pa.unwrap().status.fault_latched, 0);
        assert!(g.output().iter().all(|v| v.abs() <= 10f64.powf(-1. / 20.)));
        g.process(11, 96, &raw, &[[0.; 4]; 48]).unwrap();
        assert_eq!(g.status(SHOW, 700).unwrap().readiness, "ready");
    }
    #[test]
    #[ignore = "requires explicitly supplied independently accepted owner libraries"]
    fn wet_only_echo_is_summed_once_and_enters_actual_pa() {
        let m = manifest();
        let mut graph = ModuleGraph::load(m.clone(), 9, 0).unwrap();
        let mut pa = gigpies::host::adapters::Dsp::load(&m.pa.library, "pa", 48).unwrap();
        let mut fx = gigpies::host::adapters::Dsp::load(&m.fx.library, "fx", 48).unwrap();
        let mut heard = false;
        for frame in (0..1200).step_by(48) {
            let mut input = [[0.; 4]; 48];
            if frame == 0 {
                input[0][0] = 0.5;
            }
            let flat: Vec<_> = input.iter().flat_map(|f| f[..2].iter().copied()).collect();
            let mut wet = [0.; 96];
            assert_eq!(fx.process_result(&flat, &mut wet, 2), 0);
            if frame == 960 {
                assert!(wet[0] > 0.);
                heard = true;
            }
            let sum: Vec<_> = flat.iter().zip(wet).map(|(d, w)| d + w).collect();
            let mut output = [0.; 288];
            assert_eq!(pa.process_result(&sum, &mut output, 6), 0);
            graph.process(9, frame, &[[0.; 8]; 48], &input).unwrap();
            assert_eq!(graph.output(), &output);
        }
        assert!(heard);
    }
    #[test]
    fn manifest_rejects_wrong_version_and_no_artifact_search() {
        let empty = gigpies::module_graph::Artifact {
            library: "/not-a-provider".into(),
            library_sha256: "0".repeat(64),
            header: "/none".into(),
            header_sha256: "0".repeat(64),
            owner_manifest: "/none".into(),
            owner_manifest_sha256: "0".repeat(64),
        };
        assert!(
            Manifest {
                version: 2,
                rec: empty.clone(),
                fx: empty.clone(),
                pa: empty
            }
            .verify()
            .is_err()
        );
    }
}
