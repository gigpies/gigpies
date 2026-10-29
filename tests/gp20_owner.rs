#![cfg(feature = "hardware-host")]
//! Explicit independently built PA owner and reserved-slot software acceptance.
use gigpies::{
    control_model::{Command as A, Request as AR, Scope},
    host::pa_v2::Pa,
    local_audio::LocalAudio,
    measurement_owner::Config,
    measurement_wire::{self as wire, Command as M, Options},
    module_graph::Manifest,
    show::Counter,
    topology::EngineTopology,
};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
struct Pump {
    host: LocalAudio,
    pa: Pa,
    pa_frame: u64,
    noise: u64,
    started: Instant,
    lease: Counter,
    request: u64,
    last_renew: u64,
    exchanges: Vec<Value>,
    mic_output: usize,
}
impl Pump {
    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
    fn request(&mut self, command: M) -> wire::Request {
        self.request += 1;
        let read = matches!(
            command,
            M::MeasurementSnapshot {} | M::MeasurementResult { .. }
        );
        wire::Request {
            contract: wire::CONTRACT.into(),
            version: if matches!(
                command,
                M::MeasurementOverlap { .. } | M::MeasurementEq { .. }
            ) {
                2
            } else {
                1
            },
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: if read { None } else { Some("desk".into()) },
            lease: if read { None } else { Some(self.lease) },
            request_id: if read {
                None
            } else {
                Some(Counter(self.request))
            },
            expected_revision: if read {
                None
            } else {
                Some(self.host.engine_mut().revision())
            },
            command,
        }
    }
    fn send(&mut self, command: M) -> wire::Reply {
        let r = self.request(command);
        let reply = self
            .host
            .measurement_request(r.clone(), self.now(), true, None)
            .unwrap();
        self.exchanges.push(json!({"request":r,"reply":reply}));
        reply
    }
    fn block(&mut self) {
        let now = self.now();
        if now - self.last_renew >= 800 {
            self.request += 1;
            let r = AR {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(1),
                writer: Some("desk".into()),
                lease: Some(self.lease),
                request_id: Some(Counter(self.request)),
                expected_revision: Some(self.host.engine_mut().revision()),
                command: A::Renew {},
            };
            assert!(
                self.host
                    .engine_mut()
                    .handle(&r, now)
                    .unwrap()
                    .outcome
                    .is_some()
            );
            self.last_renew = now;
        }
        let mut input = vec![0.; 96];
        let mut raw = vec![0.; 48 * 17];
        for f in 0..48 {
            self.noise ^= self.noise << 13;
            self.noise ^= self.noise >> 7;
            self.noise ^= self.noise << 17;
            let v =
                (self.noise as i64 as f64 / i64::MAX as f64 * 0.03 * 8388608.).round() / 8388608.;
            input[f * 2] = v;
            input[f * 2 + 1] = v;
            raw[f * 17] = v;
        }
        let mut mic = vec![0.; 96];
        assert_eq!(self.pa.process(&input, &mut mic, 1, self.pa_frame), 0);
        self.pa_frame += 48;
        for f in 0..48 {
            raw[f * 17 + 16] = mic[f * 2 + self.mic_output];
        }
        let frame = self.host.frame();
        let mut output = vec![0.; 48 * self.host.topology().playback_channels];
        self.host
            .tick_with_capture(now, 1, frame, &raw, &mut output)
            .unwrap();
        assert!(
            output.iter().all(|v| *v == 0.),
            "software source remains disarmed; mic never leaks to outputs"
        );
        for completion in self.host.take_remote_completions() {
            self.exchanges.push(json!({"completion":completion}));
        }
    }
    fn capture(&mut self, id: &str, output: usize, position: &str) -> wire::Record {
        self.mic_output = output;
        let reply = self.send(M::CaptureStart {
            id: id.into(),
            reference_input: "input-01".into(),
            mic_capture_slot: 16,
            output_index: output,
            position_id: position.into(),
            samples: 32768,
            options: Options::default(),
        });
        assert_eq!(reply.state, "pending");
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            self.block();
            if let Some(r) = self
                .host
                .measurement_snapshot()
                .results
                .iter()
                .find(|r| r.id == id)
            {
                if r.state == "measured" {
                    return self
                        .send(M::MeasurementResult { id: id.into() })
                        .result
                        .unwrap();
                }
                assert!(
                    !matches!(r.state.as_str(), "invalidated" | "refused"),
                    "{r:?}"
                );
            }
            assert!(
                Instant::now() < deadline,
                "capture deadline {:?}",
                self.host.measurement_snapshot()
            );
            std::thread::sleep(Duration::from_micros(100));
        }
    }
}
#[test]
#[ignore = "explicit trusted GP05_MANIFEST, GP20_OWNER_CONFIG, GP20_GRAPH and GP20_CORPUS_DIR required; no devices"]
fn actual_pa_reserved_capture_two_positions_owner_candidate_and_cancel() {
    let manifest_path = PathBuf::from(std::env::var("GP05_MANIFEST").unwrap());
    let manifest = Manifest::load(&manifest_path).unwrap();
    let owner: Config = serde_json::from_slice(
        &std::fs::read(std::env::var("GP20_OWNER_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let graph = std::fs::read(std::env::var("GP20_GRAPH").unwrap()).unwrap();
    let corpus = PathBuf::from(std::env::var("GP20_CORPUS_DIR").unwrap());
    std::fs::create_dir_all(&corpus).unwrap();
    let root = std::env::temp_dir().join(format!("gp20-real-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut topology = EngineTopology::software(16, 2, 2).unwrap();
    topology.capture_channels += 1;
    topology.measurement_slots = vec![16];
    let mut host =
        LocalAudio::bind_configured(&root, "audio", SHOW, Counter(1), topology.clone()).unwrap();
    host.enable_modules(&manifest_path).unwrap();
    host.configure_pa(&graph).unwrap();
    host.enable_software_measurement(owner).unwrap();
    let grant = AR {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("desk".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(0)),
        command: A::Grant {
            scope: Scope::PaConfiguration,
        },
    };
    let lease = host
        .engine_mut()
        .handle(&grant, 0)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap();
    let mut pa = Pa::load(&manifest.pa.library, &graph, 1, 0).unwrap();
    assert_eq!(pa.rearm(1, 0), 0);
    let mut p = Pump {
        host,
        pa,
        pa_frame: 0,
        noise: 17,
        started: Instant::now(),
        lease,
        request: 1,
        last_renew: 0,
        exchanges: Vec::new(),
        mic_output: 0,
    };
    for _ in 0..8 {
        p.block();
    }
    let snapshot = p.send(M::MeasurementSnapshot {});
    assert!(snapshot.snapshot.as_ref().unwrap().available);
    let a = p.capture("anchor-1", 0, "p1");
    let b = p.capture("target-1", 1, "p1");
    assert_eq!(
        serde_json::from_str::<Value>(a.owner_result_json.as_ref().unwrap()).unwrap()["arrival_samples"],
        0
    );
    assert_eq!(
        serde_json::from_str::<Value>(b.owner_result_json.as_ref().unwrap()).unwrap()["arrival_samples"],
        24
    );
    p.capture("anchor-2", 0, "p2");
    p.capture("target-2", 1, "p2");
    let reply = p.send(M::MeasurementPropose {
        id: "alignment".into(),
        pairs: vec![
            ["anchor-1".into(), "target-1".into()],
            ["anchor-2".into(), "target-2".into()],
        ],
    });
    assert_eq!(reply.state, "pending");
    let deadline = Instant::now() + Duration::from_secs(8);
    let proposal = loop {
        p.block();
        if let Some(r) = p
            .host
            .measurement_snapshot()
            .results
            .iter()
            .find(|r| r.id == "alignment")
        {
            if r.state == "proposed" {
                break p
                    .send(M::MeasurementResult {
                        id: "alignment".into(),
                    })
                    .result
                    .unwrap();
            }
            assert!(
                !matches!(r.state.as_str(), "invalidated" | "refused"),
                "{r:?}"
            );
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    let candidate: Value =
        serde_json::from_str(proposal.candidate_configuration_json.as_ref().unwrap()).unwrap();
    assert_eq!(candidate["outputs"][0]["processing"]["delay_ms"], 0.5);
    assert_eq!(candidate["outputs"][1]["processing"]["inverted"], false);
    assert_eq!(
        p.host
            .measurement_current_basis()
            .unwrap()
            .configuration_json,
        std::str::from_utf8(&graph).unwrap()
    );
    for (command, id, expected) in [
        (
            M::MeasurementOverlap {
                id: "overlap".into(),
                pairs: vec![["anchor-1".into(), "target-1".into()]],
            },
            "overlap",
            "proposed",
        ),
        (
            M::MeasurementEq {
                id: "eq-flat".into(),
                captures: vec!["anchor-1".into(), "anchor-2".into()],
                target: "flat".into(),
            },
            "eq-flat",
            "no_change",
        ),
        (
            M::MeasurementEq {
                id: "eq-refused".into(),
                captures: vec!["anchor-1".into(), "target-1".into()],
                target: "flat".into(),
            },
            "eq-refused",
            "refused",
        ),
        (
            M::MeasurementEq {
                id: "eq-shaped".into(),
                captures: vec!["anchor-1".into()],
                target: "damped_high_frequency".into(),
            },
            "eq-shaped",
            "proposed",
        ),
    ] {
        assert_eq!(p.send(command).state, "pending");
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            p.block();
            let reply = p.send(M::MeasurementResult { id: id.into() });
            if let Some(r) = reply.result {
                if r.summary.state == expected {
                    if expected == "proposed" {
                        assert!(r.candidate_configuration_json.is_some());
                    }
                    break;
                }
                assert!(
                    !matches!(r.summary.state.as_str(), "invalidated" | "refused"),
                    "{r:?}"
                );
            }
            assert!(Instant::now() < deadline, "{id} timeout");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    p.send(M::CaptureStart {
        id: "cancelled".into(),
        reference_input: "input-01".into(),
        mic_capture_slot: 16,
        output_index: 0,
        position_id: "p3".into(),
        samples: 32768,
        options: Options::default(),
    });
    p.block();
    p.block();
    p.send(M::CaptureCancel {
        id: "cancelled".into(),
    });
    p.block();
    p.block();
    assert_eq!(
        p.send(M::MeasurementResult {
            id: "cancelled".into()
        })
        .result
        .unwrap()
        .summary
        .state,
        "cancelled"
    );
    std::fs::write(corpus.join("producer.json"),serde_json::to_vec_pretty(&json!({"version":1,"exchanges":p.exchanges,"topology":topology,"proposal":proposal,"physical":false})).unwrap()).unwrap();
    p.host.revoke_writer("desk");
    assert!(
        p.host
            .measurement_snapshot()
            .results
            .iter()
            .all(|r| matches!(r.state.as_str(), "invalidated" | "cancelled"))
    );
    drop(p);
    std::fs::remove_dir_all(root).unwrap();
}
