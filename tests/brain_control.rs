use gigpies::{
    brain_control::{self, Command as B, MonitorSource},
    control_model::{Command, Request, Scope},
    local_audio::LocalAudio,
    show::Counter,
    topology::EngineTopology,
};
use std::os::unix::fs::PermissionsExt;
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
fn setup(label: &str) -> (std::path::PathBuf, LocalAudio) {
    let dir = std::env::temp_dir().join(format!("gp15-brain-{}-{label}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = LocalAudio::bind_configured(
        &dir,
        "audio",
        SHOW,
        Counter(1),
        EngineTopology::software(48, 5, 0).unwrap(),
    )
    .unwrap();
    (dir, host)
}
fn grant(h: &mut LocalAudio, scope: Scope) -> Counter {
    let r = Request {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("desk".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(0)),
        command: Command::Grant { scope },
    };
    h.engine_mut()
        .handle(&r, 0)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap()
}
fn request(lease: Counter, id: u64, revision: u64, command: B) -> brain_control::Request {
    brain_control::Request {
        contract: "GP15-brain".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("desk".into()),
        lease: Some(lease),
        request_id: Some(Counter(id)),
        expected_revision: Some(Counter(revision)),
        command,
    }
}
fn apply(h: &mut LocalAudio, r: brain_control::Request, now: u64) -> brain_control::Reply {
    let p = h.brain_request(r.clone(), now, true, None).unwrap();
    assert_eq!(p.state, "pending", "{p:?}");
    h.tick(now).unwrap();
    h.tick(now + 1).unwrap();
    h.brain_request(r, now + 2, true, None).unwrap()
}
#[test]
fn selection_boundary_and_independent_high_channel_pfl() {
    let (dir, mut h) = setup("pfl");
    let lease = grant(&mut h, Scope::LocalOperatorMonitor);
    let r = request(
        lease,
        2,
        0,
        B::MonitorSet {
            source: MonitorSource::Pfl { input: 47 },
            gain_cdb: -6000,
            mute: true,
            dim: true,
            armed: false,
        },
    );
    let reply = apply(&mut h, r, 1);
    assert_eq!(reply.reason, None);
    assert_eq!(reply.applied_frame, Some(Counter(48)));
    let mut capture = vec![0.; 48 * 48];
    for row in capture.chunks_exact_mut(48) {
        row[47] = 0.25;
    }
    let mut playback = vec![0.; 48 * h.topology().playback_channels];
    let frame = h.frame();
    h.tick_with_capture(4, 1, frame, &capture, &mut playback)
        .unwrap();
    assert!(
        h.brain_monitor_output()
            .iter()
            .all(|v| (*v - 0.25 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12)
    );
    assert_eq!(h.brain_snapshot().source, MonitorSource::Pfl { input: 47 });
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn held_talkback_releases_and_never_enters_fx_send_or_monitor() {
    let (dir, mut h) = setup("held");
    let lease = grant(&mut h, Scope::TalkbackDestinations);
    h.engine_mut().rearm_outputs().unwrap();
    assert_eq!(
        apply(
            &mut h,
            request(
                lease,
                2,
                0,
                B::TalkbackSet {
                    monitors: vec![4],
                    gain_cdb: 0,
                    mute: false
                }
            ),
            1
        )
        .reason,
        None
    );
    assert_eq!(
        apply(
            &mut h,
            request(
                lease,
                3,
                1,
                B::Hold {
                    generation: Counter(1)
                }
            ),
            4
        )
        .reason,
        None
    );
    let capture = vec![0.; 48 * 48];
    let mut playback = vec![0.; 48 * h.topology().playback_channels];
    for now in 7..15 {
        let frame = h.frame();
        h.tick_with_capture_and_brain(
            now,
            1,
            frame,
            &capture,
            &mut playback,
            None,
            Some(&[0.25; 48]),
        )
        .unwrap();
    }
    assert!(
        h.last_bus_samples()
            .chunks_exact(7)
            .all(|v| v[6] > 0.24 && v[..6].iter().all(|s| *s == 0.))
    );
    assert!(h.brain_fx_send().iter().all(|v| *v == 0.));
    assert!(h.brain_monitor_output().iter().all(|v| *v == 0.));
    assert_eq!(
        apply(
            &mut h,
            request(
                lease,
                4,
                2,
                B::Release {
                    generation: Counter(1)
                }
            ),
            15
        )
        .reason,
        None
    );
    let frame = h.frame();
    h.tick_with_capture_and_brain(
        170,
        1,
        frame,
        &capture,
        &mut playback,
        None,
        Some(&[0.25; 48]),
    )
    .unwrap();
    for now in 171..177 {
        let frame = h.frame();
        h.tick_with_capture_and_brain(
            now,
            1,
            frame,
            &capture,
            &mut playback,
            None,
            Some(&[0.25; 48]),
        )
        .unwrap();
    }
    assert!(h.last_bus_samples().iter().all(|v| *v == 0.));
    assert_eq!(
        apply(
            &mut h,
            request(
                lease,
                5,
                3,
                B::Heartbeat {
                    generation: Counter(1),
                    observed_frame: Counter(0)
                }
            ),
            178
        )
        .reason
        .as_deref(),
        Some("hold released")
    );
    assert_eq!(
        apply(
            &mut h,
            request(
                lease,
                6,
                3,
                B::Hold {
                    generation: Counter(1)
                }
            ),
            181
        )
        .reason
        .as_deref(),
        Some("hold closed/generation/owner")
    );
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn source_change_requires_separate_rearm_and_bad_routes_are_atomic() {
    let (dir, mut h) = setup("rearm");
    let lease = grant(&mut h, Scope::LocalOperatorMonitor);
    let reply = apply(
        &mut h,
        request(
            lease,
            2,
            0,
            B::MonitorSet {
                source: MonitorSource::Main,
                gain_cdb: 0,
                mute: false,
                dim: false,
                armed: true,
            },
        ),
        1,
    );
    assert_eq!(
        reply.reason.as_deref(),
        Some("source change requires disarmed readback then rearm")
    );
    assert_eq!(h.brain_snapshot().source, MonitorSource::None);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn mixer_pfl_pre_mute_and_afl_post_fader_pan_preserve_bus_samples() {
    use gigpies::{
        control_model::{Edit, Target, Value},
        mixer::{Mixer, Prepared},
    };
    let topology = EngineTopology::software(48, 5, 0).unwrap();
    let mut pfl = Mixer::from_topology(0, topology.clone()).unwrap();
    let mut base = Mixer::from_topology(0, topology).unwrap();
    pfl.set_operator_tap(MonitorSource::Pfl { input: 47 });
    let edits = [Edit {
        target: Target::Mute {
            input: "input-48".into(),
        },
        value: Value::Boolean(true),
    }];
    for m in [&mut pfl, &mut base] {
        m.schedule(Prepared::edits_for(48, 5, &edits).unwrap(), 1)
            .unwrap();
    }
    let mut input = vec![0.; 48 * 48];
    for row in input.chunks_exact_mut(48) {
        row[47] = 0.25;
    }
    let mut a = vec![0.; 48 * 7];
    let mut b = a.clone();
    for _ in 0..8 {
        pfl.process_interleaved(&input, &mut a).unwrap();
        base.process_interleaved(&input, &mut b).unwrap();
        assert_eq!(a, b);
    }
    assert!(a.iter().all(|v| *v == 0.));
    assert!(pfl.operator_tap().iter().all(|v| *v > 0.17));
    pfl.set_operator_tap(MonitorSource::Afl { input: 47 });
    pfl.process_interleaved(&input, &mut a).unwrap();
    assert!(pfl.operator_tap().iter().all(|v| *v == 0.));
}
#[test]
fn producer_fixtures_match_strict_wire_and_failures() {
    let (dir, mut h) = setup("fixtures");
    let lease = grant(&mut h, Scope::LocalOperatorMonitor);
    let req = request(
        lease,
        2,
        0,
        B::MonitorSet {
            source: MonitorSource::Pfl { input: 47 },
            gain_cdb: -1200,
            mute: true,
            dim: false,
            armed: false,
        },
    );
    let reply = apply(&mut h, req.clone(), 1);
    assert!(reply.reason.is_none());
    let fail_req = request(
        lease,
        3,
        1,
        B::MonitorSet {
            source: MonitorSource::Afl { input: 48 },
            gain_cdb: -1200,
            mute: true,
            dim: false,
            armed: false,
        },
    );
    let fail = apply(&mut h, fail_req.clone(), 4);
    assert_eq!(fail.reason.as_deref(), Some("monitor source range"));
    for (name, value) in [
        ("monitor-request", serde_json::to_value(&req).unwrap()),
        ("monitor-success", serde_json::to_value(&reply).unwrap()),
        (
            "monitor-failure-request",
            serde_json::to_value(&fail_req).unwrap(),
        ),
        ("monitor-failure", serde_json::to_value(&fail).unwrap()),
    ] {
        let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
        let path = format!("tests/fixtures/brain-v1/{name}.json");
        if std::env::var_os("GIGPIES_UPDATE_BRAIN_FIXTURES").is_some() {
            std::fs::create_dir_all("tests/fixtures/brain-v1").unwrap();
            std::fs::write(&path, &text).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    let reply_bytes = serde_json::to_vec(&reply).unwrap();
    let decoded: gigpies::brain_control::Reply =
        gigpies::brain_control::Reply::decode(&reply_bytes).unwrap();
    assert_eq!(decoded.snapshot.unwrap().monitor_peak_nano, 0);
    let bytes = serde_json::to_vec(&req).unwrap();
    assert_eq!(brain_control::Request::decode(&bytes).unwrap(), req);
    let mut unsupported = req;
    unsupported.version = 99;
    assert!(brain_control::Request::decode(&serde_json::to_vec(&unsupported).unwrap()).is_err());
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deadman_expires_on_source_timeline_even_when_control_time_stalls() {
    let (dir, mut h) = setup("deadman");
    let lease = grant(&mut h, Scope::TalkbackDestinations);
    h.engine_mut().rearm_outputs().unwrap();
    apply(
        &mut h,
        request(
            lease,
            2,
            0,
            B::TalkbackSet {
                monitors: vec![0],
                gain_cdb: 0,
                mute: false,
            },
        ),
        1,
    );
    apply(
        &mut h,
        request(
            lease,
            3,
            1,
            B::Hold {
                generation: Counter(1),
            },
        ),
        4,
    );
    let capture = vec![0.; 48 * 48];
    let mut playback = vec![0.; 48 * h.topology().playback_channels];
    for _ in 0..160 {
        let frame = h.frame();
        h.tick_with_capture_and_brain(
            7,
            1,
            frame,
            &capture,
            &mut playback,
            None,
            Some(&[0.25; 48]),
        )
        .unwrap();
    }
    assert_eq!(h.brain_snapshot().held_generation, None);
    assert!(h.last_bus_samples().iter().all(|v| *v == 0.));
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn intent_restores_settings_but_never_holds_or_output_arm() {
    let (dir, mut h) = setup("persist");
    let lease = grant(&mut h, Scope::TalkbackDestinations);
    h.engine_mut().rearm_outputs().unwrap();
    apply(
        &mut h,
        request(
            lease,
            2,
            0,
            B::TalkbackSet {
                monitors: vec![4],
                gain_cdb: -600,
                mute: false,
            },
        ),
        1,
    );
    apply(
        &mut h,
        request(
            lease,
            3,
            1,
            B::Hold {
                generation: Counter(1),
            },
        ),
        4,
    );
    let intent = h.persisted_intent().unwrap();
    let encoded = intent.encode().unwrap();
    let text = String::from_utf8(encoded.clone()).unwrap();
    assert!(!text.contains("held_generation"));
    assert!(!text.contains("monitor_armed"));
    let decoded = gigpies::structural_control::Intent::decode(&encoded).unwrap();
    drop(h);
    let mut restored = LocalAudio::bind_configured(
        &dir,
        "audio",
        SHOW,
        Counter(2),
        EngineTopology::software(48, 5, 0).unwrap(),
    )
    .unwrap();
    restored.restore_composed_intent(&decoded).unwrap();
    let snapshot = restored.brain_snapshot();
    assert_eq!(snapshot.talkback_monitors, vec![4]);
    assert_eq!(snapshot.held_generation, None);
    assert!(!snapshot.monitor_armed);
    assert!(!snapshot.talkback_foh);
    assert!(!restored.brain_media_authorized(0));
    drop(restored);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_brain_audio_does_not_fault_stagebox_source() {
    let (dir, mut h) = setup("brain-fault");
    h.engine_mut().rearm_outputs().unwrap();
    let capture = vec![0.125; 48 * 48];
    let mut playback = vec![0.; 48 * h.topology().playback_channels];
    h.tick_with_capture_and_brain(
        0,
        1,
        0,
        &capture,
        &mut playback,
        None,
        Some(&[f64::NAN; 48]),
    )
    .unwrap();
    assert_eq!(h.frame(), 48);
    assert_eq!(h.brain_snapshot().held_generation, None);
    assert!(h.brain_fx_send().iter().any(|v| *v > 0.));
    assert_eq!(
        h.structural_snapshot().unwrap().clock.state,
        gigpies::clock_domain::ClockState::Running
    );
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn monitor_disarm_invalidates_stream_generation_and_readiness() {
    let (dir, mut h) = setup("monitor-generation");
    let lease = grant(&mut h, Scope::LocalOperatorMonitor);
    let settings = |armed| B::MonitorSet {
        source: MonitorSource::Main,
        gain_cdb: -1200,
        mute: false,
        dim: false,
        armed,
    };
    apply(&mut h, request(lease, 2, 0, settings(false)), 1);
    let generation = h.brain_snapshot().selection_generation;
    apply(&mut h, request(lease, 3, 1, settings(true)), 4);
    assert_eq!(h.brain_snapshot().selection_generation, generation);
    h.set_brain_path_readiness(false, true);
    assert!(h.brain_snapshot().monitor_path_ready);
    apply(&mut h, request(lease, 4, 2, settings(false)), 7);
    assert_eq!(
        h.brain_snapshot().selection_generation,
        Counter(generation.0 + 1)
    );
    assert!(!h.brain_snapshot().audible_path_ready);
    apply(&mut h, request(lease, 5, 3, settings(true)), 10);
    h.set_brain_path_readiness(false, true);
    h.close_brain_audio();
    assert_eq!(
        h.brain_snapshot().selection_generation,
        Counter(generation.0 + 2)
    );
    assert!(!h.brain_snapshot().monitor_armed);
    assert!(!h.brain_snapshot().audible_path_ready);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

fn fixture(name: &str, value: &impl serde::Serialize) {
    let value = serde_json::to_value(value).unwrap();
    let text = serde_json::to_string_pretty(&value).unwrap() + "\n";
    let path = format!("tests/fixtures/brain-v1/{name}.json");
    if std::env::var_os("GIGPIES_UPDATE_BRAIN_FIXTURES").is_some() {
        std::fs::create_dir_all("tests/fixtures/brain-v1").unwrap();
        std::fs::write(&path, &text).unwrap();
    }
    let original = std::fs::read_to_string(&path).unwrap();
    let mut expected: serde_json::Value = serde_json::from_str(&original).unwrap();
    if let Some(old_bytes) = expected
        .pointer("/resources/admission/estimated_bytes")
        .and_then(|v| v.as_u64())
    {
        let current_bytes = value["resources"]["admission"]["estimated_bytes"]
            .as_u64()
            .unwrap();
        if old_bytes != current_bytes {
            // Immutable pre-gate corpus used 672-byte strips; current strips are
            // 1144 bytes. Admission reserves two strips per input. All other
            // producer fields must still match the original bytes exactly.
            let inputs = expected["authority"]["inputs"].as_array().unwrap().len() as u64;
            expected["resources"]["admission"]["estimated_bytes"] =
                (old_bytes + inputs * 944).into();
        }
    }
    assert_eq!(
        serde_json::to_string_pretty(&expected).unwrap() + "\n",
        text
    );
}
#[test]
fn producer_dynamic_topologies_scope_grants_and_held_flow() {
    use gigpies::mixer_control::OfflineEngine;
    for (inputs, monitors) in [(16, 1), (17, 3), (32, 5), (33, 7), (48, 9)] {
        let mut engine = OfflineEngine::with_topology(
            SHOW,
            Counter(1),
            Counter(0),
            0,
            EngineTopology::software(inputs, monitors, 0).unwrap(),
        )
        .unwrap();
        fixture(
            &format!("raw-snapshot-{inputs}-{monitors}"),
            &engine.snapshot().unwrap(),
        );
        for (scope, writer) in [
            (Scope::LocalOperatorMonitor, "operator"),
            (Scope::TalkbackDestinations, "talkback"),
            (Scope::TalkbackFoh, "protected-foh"),
        ] {
            let r = Request {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(1),
                writer: Some(writer.into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: Command::Grant { scope },
            };
            fixture(&format!("grant-{inputs}-{writer}-request"), &r);
            fixture(
                &format!("grant-{inputs}-{writer}-reply"),
                &engine.handle(&r, 0).unwrap(),
            );
        }
    }
    let (dir, mut h) = setup("held-fixtures");
    let lease = grant(&mut h, Scope::TalkbackDestinations);
    h.engine_mut().rearm_outputs().unwrap();
    let configure = request(
        lease,
        2,
        0,
        B::TalkbackSet {
            monitors: vec![0, 4],
            gain_cdb: -1200,
            mute: false,
        },
    );
    fixture("talkback-configure-request", &configure);
    fixture("talkback-configure-final", &apply(&mut h, configure, 1));
    let hold = request(
        lease,
        3,
        1,
        B::Hold {
            generation: Counter(1),
        },
    );
    fixture("hold-request", &hold);
    fixture(
        "hold-pending",
        &h.brain_request(hold.clone(), 4, true, None).unwrap(),
    );
    h.tick(4).unwrap();
    h.tick(5).unwrap();
    fixture("hold-final", &h.brain_request(hold, 6, true, None).unwrap());
    let beat = request(
        lease,
        4,
        2,
        B::Heartbeat {
            generation: Counter(1),
            observed_frame: Counter(h.frame()),
        },
    );
    fixture("heartbeat-request", &beat);
    fixture("heartbeat-final", &apply(&mut h, beat, 7));
    let release = request(
        lease,
        5,
        3,
        B::Release {
            generation: Counter(1),
        },
    );
    fixture("release-request", &release);
    fixture("release-final", &apply(&mut h, release, 10));
    let replay = request(
        lease,
        6,
        4,
        B::Heartbeat {
            generation: Counter(1),
            observed_frame: Counter(h.frame()),
        },
    );
    fixture("heartbeat-after-release-request", &replay);
    fixture(
        "heartbeat-after-release-refusal",
        &apply(&mut h, replay, 13),
    );
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn delayed_heartbeat_uses_observed_source_deadline_and_expired_pending_cannot_extend() {
    let (dir, mut h) = setup("heartbeat-age");
    let lease = grant(&mut h, Scope::TalkbackDestinations);
    h.engine_mut().rearm_outputs().unwrap();
    apply(
        &mut h,
        request(
            lease,
            2,
            0,
            B::TalkbackSet {
                monitors: vec![0],
                gain_cdb: 0,
                mute: false,
            },
        ),
        1,
    );
    apply(
        &mut h,
        request(
            lease,
            3,
            1,
            B::Hold {
                generation: Counter(1),
            },
        ),
        4,
    );
    let observed = h.frame();
    let now = 7;
    // Simulate an in-flight heartbeat received 40 source milliseconds later.
    for _ in 0..40 {
        h.tick(now).unwrap();
    }
    let r = request(
        lease,
        4,
        2,
        B::Heartbeat {
            generation: Counter(1),
            observed_frame: Counter(observed),
        },
    );
    let reply = apply(&mut h, r, now + 1);
    assert_eq!(reply.reason, None);
    let expiry = h.brain_snapshot().hold_deadline_ms.unwrap().0;
    assert!(
        expiry <= now + 1 + 111,
        "remaining age must be deducted, not a fresh150ms"
    );
    // Monotonic expiry while a freshly submitted renewal waits for the boundary.
    let r = request(
        lease,
        5,
        3,
        B::Heartbeat {
            generation: Counter(1),
            observed_frame: Counter(h.frame()),
        },
    );
    h.brain_request(r.clone(), expiry - 1, true, None).unwrap();
    h.tick(expiry - 1).unwrap();
    h.tick(expiry).unwrap();
    let reply = h.brain_request(r, expiry + 1, true, None).unwrap();
    assert_eq!(reply.reason.as_deref(), Some("hold stale/owner"));
    assert_eq!(h.brain_snapshot().held_generation, None);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn source_scheduler_stall_cannot_make_old_heartbeat_fresh_again() {
    let (dir, mut h) = setup("heartbeat-wall-age");
    let lease = grant(&mut h, Scope::TalkbackDestinations);
    h.engine_mut().rearm_outputs().unwrap();
    apply(
        &mut h,
        request(
            lease,
            2,
            0,
            B::TalkbackSet {
                monitors: vec![0],
                gain_cdb: 0,
                mute: false,
            },
        ),
        1,
    );
    apply(
        &mut h,
        request(
            lease,
            3,
            1,
            B::Hold {
                generation: Counter(1),
            },
        ),
        4,
    );
    let observed = Counter(h.frame());
    let deadline = h.brain_snapshot().hold_deadline_ms;
    let beat = request(
        lease,
        4,
        2,
        B::Heartbeat {
            generation: Counter(1),
            observed_frame: observed,
        },
    );
    let reply = apply(&mut h, beat, 100);
    assert_eq!(
        reply.reason.as_deref(),
        Some("heartbeat frame observation stale")
    );
    assert_eq!(h.brain_snapshot().hold_deadline_ms, deadline);
    h.tick(deadline.unwrap().0).unwrap();
    assert_eq!(h.brain_snapshot().held_generation, None);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
