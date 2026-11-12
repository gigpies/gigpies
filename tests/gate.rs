use gigpies::{
    channel_processing::Config,
    gate,
    mixer_control::OfflineEngine,
    processing_wire::{ProcessingCommand, ProcessingReply, ProcessingRequest},
    show::Counter,
    topology::EngineTopology,
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
#[test]
fn persistence_v5_readback_and_legacy_refusal() {
    let mut engine = OfflineEngine::with_topology(
        SHOW,
        Counter(1),
        Counter(0),
        0,
        EngineTopology::software(16, 2, 0).unwrap(),
    )
    .unwrap();
    let mut intent = engine.persisted_intent().unwrap();
    assert_eq!(intent.version, 2);
    let legacy = serde_json::to_vec(&intent).unwrap();
    let restored = gigpies::mixer_control::EngineIntent::decode(&legacy).unwrap();
    assert!(restored.processing.iter().all(|c| c.gate.bypass));
    intent.version = 3;
    intent.processing[0].gate = gate::Config {
        bypass: false,
        hold_ms: 1,
        release_ms: 10,
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&intent).unwrap();
    let intent = gigpies::mixer_control::EngineIntent::decode(&bytes).unwrap();
    engine = OfflineEngine::restore_intent(&intent, Counter(2), 0).unwrap();
    engine
        .process_interleaved(&vec![0.; 16 * 1024], &mut vec![0.; 4 * 1024], 0)
        .unwrap();
    let request = ProcessingRequest {
        contract: "GP07-processing".into(),
        version: 5,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(2),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: ProcessingCommand::ProcessingSnapshot {},
    };
    let reply = engine.handle_processing(&request, 1).unwrap();
    let encoded = reply.encode().unwrap();
    if let Ok(path) = std::env::var("GP_GATE_FIXTURE") {
        std::fs::write(path, &encoded).unwrap();
    }
    let decoded = ProcessingReply::decode(&encoded).unwrap();
    let c = &decoded.snapshot.unwrap().channels[0];
    assert_eq!(c.gate.unwrap().state, gate::State::Closed);
    assert_eq!(c.gate.unwrap().attenuation_mdb, 24000);
    assert_eq!(c.gain_reduction_mdb, None);
    let old = ProcessingRequest {
        version: 4,
        ..request
    };
    assert_eq!(
        engine.handle_processing(&old, 2).unwrap().reason.as_deref(),
        Some("unsupported_version")
    );
}
#[test]
fn gate_configuration_requires_successor_writes() {
    let request = ProcessingRequest {
        contract: "GP07-processing".into(),
        version: 4,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("desk".into()),
        lease: Some(Counter(1)),
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(0)),
        command: ProcessingCommand::ProcessingSet {
            input: "input-01".into(),
            config: Config {
                gate: gate::Config {
                    bypass: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        },
    };
    assert!(request.encode().is_err());
    let request = ProcessingRequest {
        version: 5,
        ..request
    };
    assert_eq!(
        ProcessingRequest::decode(&request.encode().unwrap()).unwrap(),
        request
    );
}
#[test]
fn rates_tom_detector_and_bypass_have_no_delay() {
    for rate in [44100, 48000, 96000] {
        let mut gate = gate::Gate::new(gate::Prepared::new(gate::Config::default(), rate).unwrap());
        assert_eq!(gate.tick(1.), 1.);
        assert_eq!(gate.tick(0.), 0.);
        let c = gate::Config {
            bypass: false,
            detector_hz: 180,
            ..Default::default()
        };
        gate.apply(gate::Prepared::new(c, rate).unwrap());
        for n in 0..rate {
            let x = 0.1 * (n as f64 * std::f64::consts::TAU * 180. / rate as f64).sin();
            assert!(gate.tick(x).is_finite());
        }
        assert!(gate.linear_gain() > 0.99);
    }
}

#[test]
fn preparation_cli_exports_identified_frozen_settings_without_overwriting() {
    let root = std::env::temp_dir().join(format!("gigpies-gate-prepare-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let evidence = serde_json::json!({"version":1,"input":"input-01","calibration_id":"train",
        "evaluation_id":"eval","hits_db":vec![-20.;8],"bleed_db":vec![-40.;8],"decay_ms":vec![400.;8],
        "evaluation_hits_db":vec![-22.;8],"evaluation_bleed_db":vec![-40.;8]});
    std::fs::write(
        root.join("measurements.json"),
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_gigpies"))
            .arg("gate-prepare")
            .arg(root.join("measurements.json"))
            .arg(root.join("prepared.json"))
            .output()
            .unwrap()
    };
    let first = run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let bytes = std::fs::read(root.join("prepared.json")).unwrap();
    let p: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(p["input"], "input-01");
    assert_eq!(p["proposal"]["reason"], "prepared");
    assert_eq!(p["proposal"]["config"]["hold_ms"], 400);
    assert!(!run().status.success());
    assert_eq!(bytes, std::fs::read(root.join("prepared.json")).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn causal_stream_preserves_quiet_rolls_retriggers_and_tails_at_every_partition() {
    let rate = 48000usize;
    let config = gate::Config {
        bypass: false,
        threshold_mdb: -36000,
        hold_ms: 120,
        release_ms: 180,
        ..Default::default()
    };
    let mut signal = vec![0.; rate * 3];
    // Original synthetic signal is the sole preservation reference. Quiet roll,
    // stronger isolated hit, ringing tail, then isolated non-target bleed.
    let hits = [24000, 26400, 28800, 31200, 48000];
    for (index, start) in hits.into_iter().enumerate() {
        let amplitude = if index == 4 { 0.2 } else { 0.04 };
        for n in 0..12000 {
            signal[start + n] += amplitude
                * (-(n as f64) / 3000.).exp()
                * (std::f64::consts::TAU * 180. * n as f64 / rate as f64).sin();
        }
    }
    for (n, x) in signal.iter_mut().enumerate() {
        *x += 0.001 * (n as f64 * 1.71).sin();
    }
    let run = |block| {
        let mut output = signal.clone();
        let mut core = gate::Gate::new(gate::Prepared::new(config, rate as u32).unwrap());
        for part in output.chunks_mut(block) {
            core.process_block(part);
        }
        output
    };
    let output = run(48);
    for block in [1, 47, 128, 256, 1024] {
        assert_eq!(output, run(block));
    }
    let energy = |data: &[f64]| data.iter().map(|x| x * x).sum::<f64>();
    let mut worst_hit_db = 0f64;
    for start in hits {
        let loss = 10.
            * (energy(&output[start..start + 2400]) / energy(&signal[start..start + 2400])).log10();
        worst_hit_db = worst_hit_db.min(loss);
        assert!(loss > -0.1, "intended hit preservation {loss}");
    }
    let tail_db = 10. * (energy(&output[50400..60000]) / energy(&signal[50400..60000])).log10();
    assert!(tail_db > -0.1, "ringing tail {tail_db}");
    let bleed_db = 10. * (energy(&output[96000..]) / energy(&signal[96000..])).log10();
    assert!((bleed_db + 24.).abs() < 1e-9);
    println!(
        "causal preservation: 5/5 hits, worst {worst_hit_db:.6} dB; tail {tail_db:.6} dB; isolated bleed {bleed_db:.6} dB; partitions 1/47/48/128/256/1024 identical"
    );
}

#[test]
fn single_held_out_bleed_transient_refuses_automatic_gate() {
    let mut bleed = vec![-50.; 128];
    bleed[64] = -20.;
    let proposal = gate::prepare_soundcheck(gate::Evidence {
        hits_db: &[-20.; 8],
        bleed_db: &[-50.; 8],
        decay_ms: &[100.; 8],
        evaluation_hits_db: &[-22.; 8],
        evaluation_bleed_db: &bleed,
    });
    assert!(proposal.config.bypass);
    assert_eq!(proposal.reason, gate::Reason::EvaluationOverlap);
    assert_eq!(proposal.measurements.unwrap().evaluation_false_openings, 1);
}

#[test]
fn captured_band_is_calibration_selected_and_held_out_overlap_bypasses() {
    use gigpies::gate_preparation::{Capture, prepare_captures};
    let capture = |hit: bool, variation: usize| {
        let mut capture = Capture::new(48000).unwrap();
        let signal: Vec<_> = (0..24000)
            .map(|n| {
                let t = n as f64 / 48000.;
                let target = if hit {
                    (0.12 + variation as f64 * 0.001)
                        * (-t / 0.04).exp()
                        * (std::f64::consts::TAU * 200. * t).sin()
                } else {
                    0.
                };
                target + 0.003 * (std::f64::consts::TAU * 4000. * t).sin()
            })
            .collect();
        for block in signal.chunks(137) {
            capture.push(block).unwrap();
        }
        capture.finish().unwrap()
    };
    let train: Vec<_> = (0..8).map(|i| capture(true, i)).collect();
    let bleed: Vec<_> = (0..8).map(|i| capture(false, i)).collect();
    let eval: Vec<_> = (8..16).map(|i| capture(true, i)).collect();
    let result = prepare_captures(&train, &bleed, &eval, &bleed);
    assert_eq!(result.reason, gate::Reason::Prepared);
    assert!(result.config.detector_hz > 0);
    let refused = prepare_captures(&train, &bleed, &eval, &eval);
    assert!(refused.config.bypass);
    assert_eq!(refused.reason, gate::Reason::EvaluationOverlap);
    println!(
        "captured detector {} Hz; hold {} ms; release {} ms; confidence {}",
        result.config.detector_hz,
        result.config.hold_ms,
        result.config.release_ms,
        result.confidence
    );
}

#[test]
fn raw_cue_cli_checks_independence_and_causal_preservation() {
    let root = std::env::temp_dir().join(format!("gigpies-captured-gate-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let mut groups = Vec::new();
    for group in 0..4 {
        let mut paths = Vec::new();
        for index in 0..8 {
            let path = root.join(format!("cue-{group}-{index}.wav"));
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for n in 0..24000 {
                let t = n as f64 / 48000.;
                let target = if group % 2 == 0 {
                    (0.12 + (index + group) as f64 * 0.001)
                        * (-t / 0.04).exp()
                        * (std::f64::consts::TAU * 200. * t).sin()
                } else {
                    0.
                };
                let bleed = (0.001 + (group * 8 + index) as f64 * 0.00001)
                    * (std::f64::consts::TAU * 4000. * t).sin();
                writer
                    .write_sample(((target + bleed) * 8388608.).round() as i32)
                    .unwrap();
            }
            writer.finalize().unwrap();
            paths.push(path);
        }
        groups.push(paths);
    }
    let evidence = serde_json::json!({"version":2,"input":"input-01","calibration_id":"calibration","evaluation_id":"held-out",
        "hits":groups[0],"bleed":groups[1],"evaluation_hits":groups[2],"evaluation_bleed":groups[3]});
    let source = root.join("cues.json");
    std::fs::write(&source, serde_json::to_vec(&evidence).unwrap()).unwrap();
    let output = root.join("proposal.json");
    gigpies::gate_preparation::prepare_file(&source, &output).unwrap();
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(result["proposal"]["reason"], "prepared", "{result}");
    assert_eq!(result["causal_evaluation"]["missed_hits"], 0);
    assert_eq!(result["causal_evaluation"]["false_opening_passages"], 0);
    println!("raw cue acceptance: {}", result["causal_evaluation"]);
    // The Desk process protocol uses the same capture and causal evaluation, with
    // bounded progress and file identity receipts; it writes no proposal file.
    use std::io::Write;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_gigpies"))
        .arg("gate-prepare-stream")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&evidence).unwrap())
        .unwrap();
    let stream = child.wait_with_output().unwrap();
    assert!(stream.status.success());
    let messages: Vec<serde_json::Value> = String::from_utf8(stream.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.first().unwrap()["progress"], 0);
    assert_eq!(messages[messages.len() - 2]["progress"], 32);
    let final_message = messages.last().unwrap();
    assert_eq!(final_message["document"], result);
    assert_eq!(final_message["files"].as_array().unwrap().len(), 32);
    let mut duplicate = evidence;
    duplicate["evaluation_hits"][0] = duplicate["hits"][0].clone();
    std::fs::write(&source, serde_json::to_vec(&duplicate).unwrap()).unwrap();
    assert!(gigpies::gate_preparation::prepare_file(&source, &root.join("refused.json")).is_err());
    assert!(!root.join("refused.json").exists());
}
