use gigpies::{
    channel_processing::Config,
    control_model::{Command, Edit, Request, Scope, Target, Value},
    mixer::{Mixer, Prepared},
    mixer_control::{EngineIntent, OfflineEngine},
    sends_wire::{SendsCommand, SendsReply, SendsRequest, Tap},
    show::Counter,
    topology::EngineTopology,
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
fn engine() -> OfflineEngine {
    OfflineEngine::with_topology(
        SHOW,
        Counter(9),
        Counter(0),
        0,
        EngineTopology::software(17, 3, 0).unwrap(),
    )
    .unwrap()
}
fn audio(writer: &str, lease: Option<Counter>, id: u64, rev: u64, command: Command) -> Request {
    Request {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: Some(writer.into()),
        lease,
        request_id: Some(Counter(id)),
        expected_revision: Some(Counter(rev)),
        command,
    }
}
fn grant(e: &mut OfflineEngine, scope: Scope) -> Counter {
    let revision = e.revision().0;
    e.handle(
        &audio("sends-writer", None, 1, revision, Command::Grant { scope }),
        0,
    )
    .unwrap()
    .outcome
    .unwrap()
    .body
    .granted_lease
    .unwrap()
}
fn read() -> SendsRequest {
    SendsRequest {
        contract: "GP18-sends".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: SendsCommand::SendsSnapshot {},
    }
}
fn set(lease: Counter, id: u64, revision: u64, tap: Tap) -> SendsRequest {
    SendsRequest {
        writer: Some("sends-writer".into()),
        lease: Some(lease),
        request_id: Some(Counter(id)),
        expected_revision: Some(Counter(revision)),
        command: SendsCommand::SendTapSet {
            input: "input-17".into(),
            monitor: "monitor-3".into(),
            tap,
        },
        ..read()
    }
}
fn render(e: &mut OfflineEngine, frames: usize, now: u64) {
    e.process_interleaved(&vec![0.25; frames * 17], &mut vec![0.; frames * 5], now)
        .unwrap();
}
fn mixer(config: Config, fader: i32, pan: i32, tap: Tap) -> Mixer {
    let mut e = engine();
    let mut intent = e.persisted_intent().unwrap();
    intent.processing[0] = config;
    intent.send_taps.as_mut().unwrap()[0][1] = tap;
    for edit in &mut intent.parameters {
        if edit.target
            == (Target::Fader {
                input: "input-01".into(),
            })
        {
            edit.value = Value::Integer(fader);
        }
        if edit.target
            == (Target::Pan {
                input: "input-01".into(),
            })
        {
            edit.value = Value::Integer(pan);
        }
        if edit.target
            == (Target::Send {
                input: "input-01".into(),
                monitor: "monitor-2".into(),
            })
        {
            edit.value = Value::Integer(-6000);
        }
    }
    let mut m = Mixer::from_topology(0, intent.topology.clone()).unwrap();
    m.restore_intent(&intent.parameters, &intent.processing)
        .unwrap();
    m.restore_sends(intent.send_taps.as_ref().unwrap()).unwrap();
    m.rearm().unwrap();
    m.process_interleaved(&vec![0.; 336 * 17], &mut vec![0.; 336 * 5])
        .unwrap();
    m
}
fn signal(m: &mut Mixer, x: &[f64]) -> Vec<f64> {
    let mut input = vec![0.; x.len() * 17];
    for (f, v) in x.iter().enumerate() {
        input[f * 17] = *v;
    }
    let mut out = vec![0.; x.len() * 5];
    for (input, output) in input.chunks(1024 * 17).zip(out.chunks_mut(1024 * 5)) {
        m.process_interleaved(input, output).unwrap();
    }
    out
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-12, "{a} != {b}");
}
#[test]
fn independent_eq_compressor_fader_pan_and_levels_at_each_tap() {
    let g = 10_f64.powf(-6. / 20.);
    for tap in [
        Tap::RawPostMute,
        Tap::ProcessedPreFader,
        Tap::ProcessedPostFader,
    ] {
        // At its center, the settled RBJ bell has the requested +6 dB gain.
        let eq = Config {
            eq_bypass: false,
            band3_hz: 2000,
            band3_gain_mdb: 6000,
            ..Config::default()
        };
        let x: Vec<_> = (0..4096)
            .map(|f| 0.2 * (std::f64::consts::TAU * 2000. * f as f64 / 48000.).sin())
            .collect();
        let mut a = mixer(eq, -12000, -100, tap);
        let mut b = mixer(eq, -12000, 100, tap);
        let out = signal(&mut a, &x);
        let panned = signal(&mut b, &x);
        for f in 3000..4096 {
            let processed = x[f] * 10_f64.powf(6. / 20.);
            let selected = match tap {
                Tap::RawPostMute => x[f],
                Tap::ProcessedPreFader => processed,
                Tap::ProcessedPostFader => processed * g * g,
            };
            close(out[f * 5 + 3], selected * g);
            assert_eq!(out[f * 5 + 3], panned[f * 5 + 3]);
            assert_eq!(out[f * 5 + 1], 0.);
            assert_eq!(panned[f * 5], 0.);
            // A separate raw destination retains unity independently of EQ/fader.
            assert_eq!(out[f * 5 + 2], x[f]);
        }
        let c = Config {
            compressor_bypass: false,
            threshold_mdb: -12000,
            ratio_milli: 4000,
            knee_mdb: 0,
            attack_us: 10000,
            ..Default::default()
        };
        let out = signal(&mut mixer(c, -12000, 0, tap), &vec![1.; 480]);
        for f in 0..480 {
            let reduction = -9. * (1. - (-((f + 1) as f64) / 480.).exp());
            let processed = 10_f64.powf(reduction / 20.);
            let selected = match tap {
                Tap::RawPostMute => 1.,
                Tap::ProcessedPreFader => processed,
                Tap::ProcessedPostFader => processed * g * g,
            };
            close(out[f * 5 + 3], selected * g);
        }
        let a = signal(&mut mixer(c, 0, 0, tap), &[1.; 48]);
        let b = signal(&mut mixer(c, -12000, 0, tap), &[1.; 48]);
        let ratio = if tap == Tap::ProcessedPostFader {
            g * g
        } else {
            1.
        };
        for f in 0..48 {
            close(b[f * 5 + 3], a[f * 5 + 3] * ratio);
        }
    }
}
#[test]
fn common_mute_and_global_safety_control_every_tap() {
    for tap in [
        Tap::RawPostMute,
        Tap::ProcessedPreFader,
        Tap::ProcessedPostFader,
    ] {
        let mut m = mixer(Config::default(), 0, 0, tap);
        m.schedule(
            Prepared::edits_for(
                17,
                3,
                &[Edit {
                    target: Target::Mute {
                        input: "input-01".into(),
                    },
                    value: Value::Boolean(true),
                }],
            )
            .unwrap(),
            1,
        )
        .unwrap();
        let out = signal(&mut m, &[0.5; 400]);
        assert!(out[350 * 5..].iter().all(|v| *v == 0.));
        m.take_completion();
        m.quiesce();
        assert!(signal(&mut m, &[0.5; 48]).iter().all(|v| *v == 0.));
        let mut m = mixer(Config::default(), 0, 0, tap);
        m.mute_outputs().unwrap();
        let out = signal(&mut m, &[0.5; 300]);
        assert!(out[240 * 5..].iter().all(|v| *v == 0.));
        let mut m = mixer(Config::default(), 0, 0, tap);
        let out = signal(&mut m, &[f64::NAN; 48]);
        assert!(m.faulted());
        assert!(out.iter().all(|v| *v == 0.));
    }
}
#[test]
fn tap_transition_exact_endpoints_partition_equivalence_and_backpressure() {
    let mut one = mixer(Config::default(), -12000, 0, Tap::RawPostMute);
    let boundary = one
        .schedule(
            Prepared::send_for(17, 3, 0, 1, Tap::ProcessedPostFader).unwrap(),
            1,
        )
        .unwrap();
    assert_eq!(boundary, 384); // strict next boundary even when already aligned
    let mut split = mixer(Config::default(), -12000, 0, Tap::RawPostMute);
    split
        .schedule(
            Prepared::send_for(17, 3, 0, 1, Tap::ProcessedPostFader).unwrap(),
            1,
        )
        .unwrap();
    let x: Vec<_> = (0..1000).map(|f| (f as f64 * 0.13).sin() * 0.25).collect();
    let out = signal(&mut one, &x);
    let mut partition = Vec::new();
    let mut start = 0;
    for count in [1, 47, 1, 37, 153, 1, 48, 7, 705] {
        partition.extend(signal(&mut split, &x[start..start + count]));
        start += count;
    }
    assert_eq!(out, partition);
    let g = 10_f64.powf(-6. / 20.);
    for f in 0..1000 {
        let w = ((f as f64 - 48.) / 240.).clamp(0., 1.);
        let expected = if f <= 48 {
            x[f] * g
        } else if f >= 288 {
            x[f] * g * g * g
        } else {
            (x[f] * (1. - w) + x[f] * g * g * w) * g
        };
        close(out[f * 5 + 3], expected);
    }
    assert_eq!(out[48 * 5 + 3], x[48] * g);
    assert_eq!(out[288 * 5 + 3], x[288] * 10_f64.powf(-12. / 20.) * g);
    // Even after draining the ownership slot, an active fade cannot retarget.
    let mut m = mixer(Config::default(), -12000, 0, Tap::RawPostMute);
    m.schedule(
        Prepared::send_for(17, 3, 0, 1, Tap::ProcessedPostFader).unwrap(),
        1,
    )
    .unwrap();
    signal(&mut m, &[1.; 49]);
    m.take_completion();
    assert!(
        m.schedule(
            Prepared::send_for(17, 3, 0, 2, Tap::ProcessedPreFader).unwrap(),
            2
        )
        .is_err()
    );
}
#[test]
fn producer_midfade_snapshots_reject_impossible_transition_states() {
    let mut e = engine();
    let lease = grant(&mut e, Scope::Monitor(3));
    e.handle_sends(&set(lease, 2, 0, Tap::ProcessedPostFader), 1)
        .unwrap();
    render(&mut e, 49, 2);
    let reply = e.handle_sends(&read(), 2).unwrap();
    let snapshot = reply.snapshot.as_ref().unwrap();
    // Real producer reads are legal at arbitrary sample positions during a fade.
    assert_eq!(snapshot.frame, Counter(49));
    assert_eq!(
        snapshot.channels[16].sends[2].transition_remaining_frames,
        239
    );
    SendsReply::decode(&serde_json::to_vec(&reply).unwrap()).unwrap();
    let reject = |snapshot: gigpies::sends_wire::SendsSnapshot| {
        let mut malformed = reply.clone();
        malformed.snapshot = Some(snapshot);
        assert!(SendsReply::decode(&serde_json::to_vec(&malformed).unwrap()).is_err());
    };
    let mut equal = snapshot.clone();
    equal.channels[16].sends[2].current = Tap::ProcessedPostFader;
    reject(equal);
    let mut unaligned = snapshot.clone();
    unaligned.channels[16].sends[2].transition_remaining_frames = 238;
    reject(unaligned);
    let mut overflow = snapshot.clone();
    overflow.frame = Counter(u64::MAX - 238);
    reject(overflow);
    let mut simultaneous = snapshot.clone();
    simultaneous.channels[0].sends[0].target = Tap::ProcessedPreFader;
    simultaneous.channels[0].sends[0].transition_remaining_frames = 239;
    simultaneous.channels[0].sends[0].ready = false;
    reject(simultaneous);
    // Completed and no-op mutations settle immediately; no manufactured fade.
    assert_eq!(e.take_sends_completions().len(), 1);
    render(&mut e, 239, 3);
    assert_eq!(
        e.handle_sends(&set(lease, 3, 1, Tap::ProcessedPostFader), 4)
            .unwrap()
            .state,
        "pending"
    );
    render(&mut e, 49, 5);
    let settled = e.handle_sends(&read(), 5).unwrap();
    assert!(
        settled
            .snapshot
            .as_ref()
            .unwrap()
            .channels
            .iter()
            .flat_map(|c| &c.sends)
            .all(|s| s.ready && s.current == s.target && s.transition_remaining_frames == 0)
    );
    SendsReply::decode(&serde_json::to_vec(&settled).unwrap()).unwrap();
}

#[test]
fn legacy_default_is_bit_exact_with_processing_and_v1_migration() {
    let mut legacy = Mixer::new(0);
    let mut configured =
        Mixer::from_topology(0, EngineTopology::software(8, 2, 0).unwrap()).unwrap();
    let config = Config {
        eq_bypass: false,
        band3_gain_mdb: 6000,
        compressor_bypass: false,
        ratio_milli: 4000,
        ..Default::default()
    };
    for m in [&mut legacy, &mut configured] {
        m.schedule(
            Prepared::processing_for(
                8,
                2,
                0,
                gigpies::channel_processing::Prepared::new(config).unwrap(),
            )
            .unwrap(),
            1,
        )
        .unwrap();
    }
    let input: Vec<_> = (0..8000)
        .map(|f| ((f * 17 % 503) as f64 - 251.) / 512.)
        .collect();
    let mut a = vec![0.; 4000];
    let mut b = a.clone();
    legacy.process_interleaved(&input, &mut a).unwrap();
    configured.process_interleaved(&input, &mut b).unwrap();
    assert_eq!(a, b);
    for f in 0..1000 {
        assert_eq!(a[f * 4 + 2], input[f * 8..f * 8 + 8].iter().sum::<f64>());
    }
    let mut e = engine();
    let mut old = e.persisted_intent().unwrap();
    old.version = 1;
    old.send_taps = None;
    let restored = OfflineEngine::restore_intent(&old, Counter(10), 0).unwrap();
    assert!(
        restored
            .mixer()
            .send_observations()
            .iter()
            .flatten()
            .all(|s| s.1 == Tap::RawPostMute)
    );
    let decoded = EngineIntent::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(decoded.version, 1);
}
#[test]
fn authority_shared_revision_scope_pending_dedup_and_cross_contract_ids() {
    let mut e = engine();
    let lease = grant(&mut e, Scope::Monitor(3));
    let r = set(lease, 2, 0, Tap::ProcessedPostFader);
    let pending = e.handle_sends(&r, 1).unwrap();
    pending.validate().unwrap();
    assert_eq!(pending.state, "pending");
    assert_eq!(pending.effective_frame, Some(Counter(48)));
    assert_eq!(e.handle_sends(&r, 2).unwrap(), pending);
    let mut changed = r.clone();
    changed.command = SendsCommand::SendTapSet {
        input: "input-01".into(),
        monitor: "monitor-3".into(),
        tap: Tap::ProcessedPreFader,
    };
    assert_eq!(
        e.handle_sends(&changed, 3).unwrap().reason.as_deref(),
        Some("reused_id")
    );
    assert_eq!(
        e.handle(
            &audio("sends-writer", Some(lease), 2, 0, Command::Renew {}),
            3
        )
        .unwrap()
        .outcome
        .unwrap()
        .body
        .reason
        .as_deref(),
        Some("reused_id")
    );
    assert_eq!(
        e.handle_sends(&set(lease, 3, 0, Tap::RawPostMute), 3)
            .unwrap()
            .state,
        "backpressure"
    );
    render(&mut e, 48, 4);
    assert!(e.take_sends_completions().is_empty());
    render(&mut e, 1, 5);
    let final_reply = e.take_sends_completions().pop().unwrap();
    final_reply.validate().unwrap();
    assert_eq!(e.revision(), Counter(1));
    assert_eq!(
        final_reply.snapshot.as_ref().unwrap().channels[16].sends[2].transition_remaining_frames,
        239
    );
    assert_eq!(e.handle_sends(&r, 6).unwrap(), final_reply);
    assert_eq!(
        e.handle_sends(&changed, 6).unwrap().reason.as_deref(),
        Some("reused_id")
    );
    render(&mut e, 239, 7);
    assert_eq!(
        e.handle_sends(&set(lease, 3, 0, Tap::RawPostMute), 8)
            .unwrap()
            .reason
            .as_deref(),
        Some("stale_revision")
    );
    assert_eq!(
        e.handle_sends(&set(lease, 4, 1, Tap::RawPostMute), 8)
            .unwrap()
            .state,
        "pending"
    );
    e.revoke_writer("sends-writer");
    render(&mut e, 96, 9);
    assert!(e.take_sends_completions().is_empty());
    assert_eq!(e.revision(), Counter(1));
    let mut e = engine();
    let lease = grant(&mut e, Scope::Foh);
    assert_eq!(
        e.handle_sends(&set(lease, 2, 0, Tap::ProcessedPreFader), 1)
            .unwrap()
            .reason
            .as_deref(),
        Some("scope")
    );
}
#[test]
fn expiry_boundary_persistence_recovery_and_no_pending_replay() {
    let mut e = engine();
    let lease = grant(&mut e, Scope::Monitor(3));
    let r = set(lease, 2, 0, Tap::ProcessedPreFader);
    assert_eq!(e.handle_sends(&r, 1).unwrap().state, "pending");
    let pre = e.persisted_intent().unwrap();
    assert_eq!(pre.send_taps.as_ref().unwrap()[16][2], Tap::RawPostMute);
    render(&mut e, 49, 3000);
    let refused = e.take_sends_completions().pop().unwrap();
    assert_eq!(refused.reason.as_deref(), Some("lease"));
    assert_eq!(e.revision(), Counter(0));
    let mut e = engine();
    let lease = grant(&mut e, Scope::Monitor(3));
    e.handle_sends(&set(lease, 2, 0, Tap::ProcessedPreFader), 1)
        .unwrap();
    render(&mut e, 49, 2);
    e.take_sends_completions();
    let intent = e.persisted_intent().unwrap();
    assert_eq!(intent.version, 2);
    assert_eq!(
        intent.send_taps.as_ref().unwrap()[16][2],
        Tap::ProcessedPreFader
    );
    let restored = OfflineEngine::restore_intent(&intent, Counter(10), 0).unwrap();
    assert!(restored.outputs_quiesced());
    assert!(restored.mixer().sends_ready());
    assert_eq!(restored.revision(), Counter(0));
    assert_eq!(
        restored.mixer().send_observations()[16][2],
        (Tap::ProcessedPreFader, Tap::ProcessedPreFader, 0)
    );
    e.quiesce("source_discontinuity");
    e.recover(Counter(10), 0).unwrap();
    assert!(e.outputs_quiesced());
    assert_eq!(
        e.mixer().send_observations()[16][2].1,
        Tap::ProcessedPreFader
    );
    assert_eq!(
        e.handle_sends(&r, 3).unwrap().reason.as_deref(),
        Some("epoch")
    );
    let mut invalid = intent.clone();
    invalid.send_taps.as_mut().unwrap().pop();
    assert!(OfflineEngine::restore_intent(&invalid, Counter(10), 0).is_err());
    invalid = intent.clone();
    invalid.send_taps.as_mut().unwrap()[0].pop();
    assert!(EngineIntent::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    invalid = intent.clone();
    invalid.send_taps = None;
    assert!(EngineIntent::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    let composed = gigpies::structural_control::Intent {
        version: 1,
        engine: intent,
        brain: None,
        pa_configuration_json: None,
        pa_program_buses: vec![],
    };
    assert!(gigpies::structural_control::Intent::decode(&composed.encode().unwrap()).is_ok());
}
#[test]
fn strict_envelope_and_readback_reject_invalid_duplicate_unknown_and_overlarge() {
    let good = set(Counter(1), 2, 0, Tap::ProcessedPreFader)
        .encode()
        .unwrap();
    assert!(SendsRequest::decode(&good).is_ok());
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("input", serde_json::json!("input-017")),
        ("monitor", serde_json::json!("monitor-03")),
        ("tap", serde_json::json!("pre")),
        ("tap", serde_json::json!(1)),
    ] {
        let mut v: serde_json::Value = serde_json::from_slice(&good).unwrap();
        if key == "version" {
            v[key] = value;
        } else {
            v["body"][key] = value;
        }
        assert!(SendsRequest::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    let mut v: serde_json::Value = serde_json::from_slice(&good).unwrap();
    v["body"]["level"] = 0.into();
    assert!(SendsRequest::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    let text = String::from_utf8(good).unwrap();
    let duplicate = text.replace("\"version\":1", "\"version\":1,\"version\":1");
    assert!(SendsRequest::decode(duplicate.as_bytes()).is_err());
    assert!(SendsRequest::decode(&vec![b' '; 65537]).is_err());
    let mut e = engine();
    let reply = e.handle_sends(&read(), 0).unwrap();
    assert_eq!(SendsReply::decode(&reply.encode().unwrap()).unwrap(), reply);
    let mut v = serde_json::to_value(reply).unwrap();
    v["snapshot"]["channels"][0]["sends"][0]["ready"] = false.into();
    assert!(SendsReply::decode(&serde_json::to_vec(&v).unwrap()).is_err());
}
#[test]
fn explicit_processing_successor_and_legacy_wire_stay_truthful() {
    use gigpies::processing_wire::{ProcessingCommand, ProcessingRequest};
    let mut e = engine();
    let base = ProcessingRequest {
        contract: "GP07-processing".into(),
        version: 4,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: ProcessingCommand::ProcessingSnapshot {},
    };
    let reply = e.handle_processing(&base, 0).unwrap();
    reply.validate().unwrap();
    assert_eq!(reply.snapshot.unwrap().monitor_tap, "per-send-gp18-v1");
    for version in [2, 3] {
        let mut r = base.clone();
        r.version = version;
        assert_eq!(
            e.handle_processing(&r, 0).unwrap().reason.as_deref(),
            Some("unsupported_version")
        );
    }
    let mut legacy = OfflineEngine::new(SHOW, Counter(9), Counter(0), 0).unwrap();
    let mut r = base;
    r.version = 2;
    let p = legacy.handle_processing(&r, 0).unwrap();
    p.validate().unwrap();
    assert_eq!(p.snapshot.unwrap().monitor_tap, "raw-post-mute-v1");
    assert_eq!(
        legacy.handle_sends(&read(), 0).unwrap().reason.as_deref(),
        Some("unsupported_topology")
    );
}
#[test]
fn send_busy_gates_external_module_processing_and_invalid_inventory() {
    let mut e = engine();
    let lease = grant(&mut e, Scope::Monitor(3));
    let r = set(lease, 2, 0, Tap::ProcessedPreFader);
    assert_eq!(e.handle_sends(&r, 1).unwrap().state, "pending");
    let a = audio("sends-writer", Some(lease), 2, 0, Command::Renew {});
    assert_eq!(
        e.authorize_module(&a, "fixture", 1).unwrap_err(),
        "module authority identity/lease/scope"
    );
    assert_eq!(
        e.begin_external(&a, "fixture", Scope::PaConfiguration, 1)
            .unwrap_err(),
        "reused_id"
    );
    let mut config = gigpies::processing_wire::ProcessingRequest {
        contract: "GP07-processing".into(),
        version: 4,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: r.writer.clone(),
        lease: r.lease,
        request_id: r.request_id,
        expected_revision: r.expected_revision,
        command: gigpies::processing_wire::ProcessingCommand::ProcessingSet {
            input: "input-01".into(),
            config: Default::default(),
        },
    };
    assert_eq!(
        e.handle_processing(&config, 1).unwrap().reason.as_deref(),
        Some("reused_id")
    );
    config.request_id = Some(Counter(3));
    assert_eq!(
        e.handle_processing(&config, 1).unwrap().state,
        "backpressure"
    );
    e.revoke_writer("sends-writer");
    let mut e = engine();
    let lease = grant(&mut e, Scope::Monitor(3));
    let mut r = set(lease, 2, 0, Tap::RawPostMute);
    r.command = SendsCommand::SendTapSet {
        input: "input-18".into(),
        monitor: "monitor-3".into(),
        tap: Tap::RawPostMute,
    };
    assert_eq!(
        e.handle_sends(&r, 1).unwrap().reason.as_deref(),
        Some("target")
    );
    let mut old = e.persisted_intent().unwrap();
    old.version = 1;
    old.send_taps = None;
    let mut value = serde_json::to_value(old).unwrap();
    value["send_taps"] = serde_json::Value::Null;
    assert!(EngineIntent::decode(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
#[ignore = "one-time actual producer corpus; explicit committed revision and output required"]
fn write_sends_producer_corpus() {
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    let revision = std::env::var("GP18_SOURCE_REVISION").unwrap();
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), revision);
    let root = PathBuf::from(std::env::var("GP18_FIXTURES").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    let mut hashes = std::collections::BTreeMap::new();
    let mut write = |name: &str, value: serde_json::Value| {
        let mut bytes = serde_json::to_vec_pretty(&value).unwrap();
        bytes.push(b'\n');
        std::fs::write(root.join(name), &bytes).unwrap();
        hashes.insert(name.to_owned(), gigpies::hash::hex(Sha256::digest(&bytes)));
    };
    let mut e = engine();
    write(
        "snapshot-request.json",
        serde_json::to_value(read()).unwrap(),
    );
    write(
        "baseline.json",
        serde_json::to_value(e.handle_sends(&read(), 0).unwrap()).unwrap(),
    );
    write(
        "raw-baseline.json",
        serde_json::to_value(e.snapshot().unwrap()).unwrap(),
    );
    let lease = grant(&mut e, Scope::Monitor(3));
    let r = set(lease, 2, 0, Tap::ProcessedPostFader);
    write("set-request.json", serde_json::to_value(&r).unwrap());
    write(
        "pending.json",
        serde_json::to_value(e.handle_sends(&r, 1).unwrap()).unwrap(),
    );
    write(
        "pending-read.json",
        serde_json::to_value(e.handle_sends(&read(), 1).unwrap()).unwrap(),
    );
    render(&mut e, 49, 2);
    write(
        "final.json",
        serde_json::to_value(e.take_sends_completions().pop().unwrap()).unwrap(),
    );
    render(&mut e, 119, 3);
    write(
        "transition.json",
        serde_json::to_value(e.handle_sends(&read(), 3).unwrap()).unwrap(),
    );
    write(
        "backpressure.json",
        serde_json::to_value(
            e.handle_sends(&set(lease, 3, 1, Tap::RawPostMute), 3)
                .unwrap(),
        )
        .unwrap(),
    );
    render(&mut e, 120, 4);
    write(
        "ready.json",
        serde_json::to_value(e.handle_sends(&read(), 4).unwrap()).unwrap(),
    );
    write(
        "stale-revision.json",
        serde_json::to_value(
            e.handle_sends(&set(lease, 3, 0, Tap::RawPostMute), 5)
                .unwrap(),
        )
        .unwrap(),
    );
    let mut reused = r.clone();
    reused.command = SendsCommand::SendTapSet {
        input: "input-01".into(),
        monitor: "monitor-3".into(),
        tap: Tap::RawPostMute,
    };
    write(
        "reused-id.json",
        serde_json::to_value(e.handle_sends(&reused, 5).unwrap()).unwrap(),
    );
    let grant = e
        .handle(
            &audio(
                "processing-writer",
                None,
                1,
                1,
                Command::Grant { scope: Scope::Foh },
            ),
            6,
        )
        .unwrap();
    let foh_lease = grant.outcome.as_ref().unwrap().body.granted_lease;
    write("foh-grant.json", serde_json::to_value(grant).unwrap());
    let gp07 = gigpies::processing_wire::ProcessingRequest {
        contract: "GP07-processing".into(),
        version: 4,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: gigpies::processing_wire::ProcessingCommand::ProcessingSnapshot {},
    };
    write(
        "gp07v4-snapshot-request.json",
        serde_json::to_value(&gp07).unwrap(),
    );
    write(
        "gp07v4-baseline.json",
        serde_json::to_value(e.handle_processing(&gp07, 6).unwrap()).unwrap(),
    );
    let mut set07 = gp07.clone();
    set07.writer = Some("processing-writer".into());
    set07.lease = foh_lease;
    set07.request_id = Some(Counter(2));
    set07.expected_revision = Some(Counter(1));
    set07.command = gigpies::processing_wire::ProcessingCommand::ProcessingSet {
        input: "input-17".into(),
        config: Config {
            eq_bypass: false,
            band3_gain_mdb: 6000,
            compressor_bypass: false,
            ratio_milli: 4000,
            ..Default::default()
        },
    };
    write(
        "gp07v4-set-request.json",
        serde_json::to_value(&set07).unwrap(),
    );
    write(
        "gp07v4-pending.json",
        serde_json::to_value(e.handle_processing(&set07, 6).unwrap()).unwrap(),
    );
    render(&mut e, 49, 7);
    write(
        "gp07v4-final.json",
        serde_json::to_value(e.take_processing_completions().pop().unwrap()).unwrap(),
    );
    render(&mut e, 239, 8);
    write(
        "gp07v4-ready.json",
        serde_json::to_value(e.handle_processing(&gp07, 8).unwrap()).unwrap(),
    );
    let mut old = gp07.clone();
    old.version = 3;
    let refused =
        gigpies::processing_wire::UnsupportedRequest::decode_for(&old.encode().unwrap(), 4)
            .unwrap()
            .unwrap()
            .refusal(e.revision());
    write("gp07v3-refusal.json", refused);
    let intent = e.persisted_intent().unwrap();
    write("intent-v2.json", serde_json::to_value(&intent).unwrap());
    write(
        "lease-expired.json",
        serde_json::to_value(
            e.handle_sends(&set(lease, 4, 2, Tap::RawPostMute), 2000)
                .unwrap(),
        )
        .unwrap(),
    );
    e.quiesce("source_reopen");
    e.recover(Counter(10), 0).unwrap();
    let mut recovery = read();
    recovery.epoch = Counter(10);
    write(
        "recovery.json",
        serde_json::to_value(e.handle_sends(&recovery, 2001).unwrap()).unwrap(),
    );
    write(
        "recovery-raw.json",
        serde_json::to_value(e.snapshot().unwrap()).unwrap(),
    );
    for n in [16, 32, 48] {
        let mut profile = OfflineEngine::with_topology(
            SHOW,
            Counter(9),
            Counter(0),
            0,
            EngineTopology::software(n, 5, 0).unwrap(),
        )
        .unwrap();
        write(
            &format!("baseline-{n}.json"),
            serde_json::to_value(profile.handle_sends(&read(), 0).unwrap()).unwrap(),
        );
        write(
            &format!("gp07v4-{n}.json"),
            serde_json::to_value(profile.handle_processing(&gp07, 0).unwrap()).unwrap(),
        );
    }
    let sources = [
        "src/mixer.rs",
        "src/mixer_control.rs",
        "src/sends_wire.rs",
        "src/processing_wire.rs",
        "src/local_audio.rs",
        "src/remote/authority.rs",
        "src/remote/policy.rs",
        "src/analysis_stream.rs",
        "tests/sends_local.rs",
        "tests/gp04_local.rs",
        "src/remote/tests.rs",
        "src/control_model.rs",
        "src/channel_processing.rs",
        "src/topology.rs",
        "tests/sends.rs",
    ];
    let source_hashes: std::collections::BTreeMap<_, _> = sources
        .into_iter()
        .map(|name| {
            (
                name,
                gigpies::hash::hex(Sha256::digest(std::fs::read(name).unwrap())),
            )
        })
        .collect();
    std::fs::write(root.join("providers.json"),serde_json::to_vec_pretty(&serde_json::json!({"contract":"GP18-sends","version":1,"processing_version":4,"source_revision":revision,"source_sha256":source_hashes,"fixture_sha256":hashes,"provenance":"Actual committed OfflineEngine producer, bounded synthetic samples; no hardware. Coordinator review required before consumer freeze."})).unwrap()).unwrap();
    std::fs::write(
        root.join("SHA256SUMS"),
        hashes
            .iter()
            .map(|(n, h)| format!("{h}  {n}\n"))
            .collect::<String>(),
    )
    .unwrap();
}

#[test]
#[ignore = "producer corpus verification after explicit generation; no output mutations"]
fn verify_sends_producer_corpus() {
    use sha2::{Digest, Sha256};
    let root_path =
        std::env::var("GP18_FIXTURES").unwrap_or_else(|_| "tests/fixtures/gp18/v1".into());
    let root = std::path::Path::new(&root_path);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("providers.json")).unwrap()).unwrap();
    assert_eq!(manifest["source_revision"].as_str().unwrap().len(), 40);
    for (name, hash) in manifest["fixture_sha256"].as_object().unwrap() {
        let bytes = std::fs::read(root.join(name)).unwrap();
        assert_eq!(
            gigpies::hash::hex(Sha256::digest(&bytes)),
            hash.as_str().unwrap()
        );
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if value["contract"] == "GP18-sends" {
            if value.get("kind").is_some() {
                SendsRequest::decode(&bytes).unwrap();
            } else {
                SendsReply::decode(&bytes).unwrap();
            }
        } else if value["contract"] == "GP07-processing" {
            if value["reason"] == "unsupported_version" {
                assert_eq!(value["version"], 3);
                assert!(value["snapshot"].is_null());
            } else if value.get("kind").is_some() {
                gigpies::processing_wire::ProcessingRequest::decode(&bytes).unwrap();
            } else {
                gigpies::processing_wire::ProcessingReply::decode_assembled(&bytes).unwrap();
            }
        } else if name == "intent-v2.json" {
            EngineIntent::decode(&bytes).unwrap();
        } else if value["capability"] == "GP03-rendered" && value.get("context").is_some() {
            gigpies::mixer_control::RenderedReply::decode_assembled(&bytes).unwrap();
        } else if value["capability"] == "GP03-rendered" {
            let snapshot: gigpies::mixer_control::RenderedSnapshot =
                serde_json::from_slice(&bytes).unwrap();
            snapshot.validate().unwrap();
        }
    }
    for (name, hash) in manifest["source_sha256"].as_object().unwrap() {
        assert_eq!(
            {
                let revision = manifest["source_revision"].as_str().unwrap();
                let blob = std::process::Command::new("git")
                    .args(["show", &format!("{revision}:{name}")])
                    .output()
                    .unwrap();
                assert!(blob.status.success());
                gigpies::hash::hex(Sha256::digest(blob.stdout))
            },
            hash.as_str().unwrap()
        );
    }
}
