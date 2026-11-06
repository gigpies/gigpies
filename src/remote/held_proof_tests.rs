//! Crate-private authenticated producer fixture boundary; no public auth bypass.
use crate::{
    control_model::Scope,
    held_proof::{self, Configuration, Request},
    show::Counter,
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
fn digest(inputs: u32, destinations: &[usize], gain: i32, mute: bool, foh: bool) -> String {
    held_proof::config_digest(Configuration {
        show_id: SHOW,
        epoch: 1,
        capability: 1,
        map: 1,
        dimensions: [inputs, 5, 0, inputs, 7, 48000],
        destinations: held_proof::destination_hash(destinations).unwrap(),
        gain_cdb: gain,
        mute,
        foh,
    })
}
fn request(writer: String, session: u64, lease: Counter, inputs: u32) -> Request {
    Request {
        contract: held_proof::CONTRACT.into(),
        version: 1,
        kind: "held_proof".into(),
        query_id: Counter(1),
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        authenticated_session: Counter(session),
        writer,
        capability_generation: Counter(1),
        map_generation: Counter(1),
        scope: Scope::TalkbackDestinations,
        lease,
        expected_config_digest: digest(inputs, &[], -1200, true, false),
    }
}

use crate::{
    control_model::{Command, Request as AudioRequest},
    local_audio::LocalAudio,
    remote::{AuthorityEndpoint, HostAuthority, Peer, Permission, PolicyStore, fingerprint},
    topology::EngineTopology,
};
use std::os::unix::fs::PermissionsExt;
fn fixture(name: &str, value: &serde_json::Value) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/held-proof-v1")
        .join(format!("{name}.json"));
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    if std::env::var_os("GIGPIES_UPDATE_HELD_PROOF_FIXTURES").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [bytes.as_slice(), b"\n"].concat()).unwrap();
    }
    let mut saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&path)
            .expect("generate explicit held-proof fixtures before normal validation"),
    )
    .unwrap();
    // Historical producer bytes retain their original 672-byte strip reservation.
    // Only the exact additional gate storage is normalized; other drift still fails.
    let snapshot = &mut saved;
    if let Some(n) = snapshot["topology"]["inputs"].as_array().map(Vec::len) {
        let extra = n * (std::mem::size_of::<crate::channel_processing::Strip>() - 672) * 2;
        let field = &mut snapshot["resources"]["admission"]["estimated_bytes"];
        if let Some(old) = field.as_u64() {
            let actual = value["resources"]["admission"]["estimated_bytes"].as_u64();
            if actual == Some(old + extra as u64) {
                *field = serde_json::json!(old + extra as u64);
            }
        }
    }
    assert_eq!(&saved, value);
}
fn brain_change(
    host: &mut HostAuthority,
    writer: &str,
    lease: Counter,
    id: u64,
    now: u64,
    command: crate::brain_control::Command,
) {
    let revision = host.provider_mut().engine_mut().revision();
    let r = crate::brain_control::Request {
        contract: "GP15-brain".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some(writer.into()),
        lease: Some(lease),
        request_id: Some(Counter(id)),
        expected_revision: Some(revision),
        command,
    };
    assert_eq!(
        host.provider_mut()
            .brain_request(r.clone(), now, true, None)
            .unwrap()
            .state,
        "pending"
    );
    host.provider_mut().tick(now).unwrap();
    host.provider_mut().tick(now + 1).unwrap();
    assert!(
        host.provider_mut()
            .brain_request(r, now + 2, true, None)
            .unwrap()
            .reason
            .is_none()
    );
}
fn scoped_grant(host: &mut HostAuthority, writer: &str, scope: Scope, now: u64) -> Counter {
    let revision = host.provider_mut().engine_mut().revision();
    host.provider_mut()
        .engine_mut()
        .handle(
            &AudioRequest {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(1),
                writer: Some(writer.into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(revision),
                command: Command::Grant { scope },
            },
            now,
        )
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap()
}
#[test]
fn held_proof_host_renewal_preserves_active_hold_beyond_full_readback_age() {
    let dir = std::env::temp_dir().join(format!("held-proof-renew-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let policy = PolicyStore::new(vec![Peer {
        id: "held-renew".into(),
        certificate_sha256: fingerprint(b"held-renew"),
        permissions: [Permission::TalkbackDestinations].into_iter().collect(),
    }])
    .unwrap();
    let context = policy.authenticate(b"held-renew", 93).unwrap();
    let local = LocalAudio::bind_configured(
        &dir,
        "audio",
        SHOW,
        Counter(1),
        EngineTopology::software(16, 5, 0).unwrap(),
    )
    .unwrap();
    let mut host = HostAuthority::new(local, 1).unwrap();
    // Exactly one full raw attach at0ms. No full snapshot dispatch in loop.
    let raw = AudioRequest {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: Command::Snapshot {},
    };
    assert!(
        host.dispatch(&context, serde_json::to_value(raw).unwrap(), 0)
            .unwrap()
            .unwrap()["snapshot"]
            .is_object()
    );
    let grant = AudioRequest {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some(context.writer().into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(0)),
        command: Command::Grant {
            scope: Scope::TalkbackDestinations,
        },
    };
    let granted = host
        .dispatch(&context, serde_json::to_value(grant).unwrap(), 0)
        .unwrap()
        .unwrap();
    let lease = Counter(
        granted["outcome"]["body"]["granted_lease"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    host.provider_mut().rearm().unwrap();
    brain_change(
        &mut host,
        context.writer(),
        lease,
        2,
        1,
        crate::brain_control::Command::TalkbackSet {
            monitors: vec![0, 4],
            gain_cdb: -1200,
            mute: false,
        },
    );
    brain_change(
        &mut host,
        context.writer(),
        lease,
        3,
        4,
        crate::brain_control::Command::Hold {
            generation: Counter(1),
        },
    );
    let mut proof_request = request(context.writer().into(), context.session(), lease, 16);
    proof_request.expected_config_digest = digest(16, &[0, 4], -1200, false, false);
    let mut query_id = 0;
    let mut mutation_id = 3;
    let mut renewals = 0;
    let mut beats = 0;
    let capture = vec![0.; 48 * host.provider().topology().capture_channels];
    let mut playback = vec![0.; 48 * host.provider().topology().playback_channels];
    for now in 7..=2207 {
        if now % 40 == 0 || [410, 1110, 1810].contains(&now) {
            query_id += 1;
            proof_request.query_id = Counter(query_id);
            let value = host
                .dispatch(&context, serde_json::to_value(&proof_request).unwrap(), now)
                .unwrap()
                .unwrap();
            let proof = held_proof::Reply::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
            let witness = proof.witness.expect("own live proof");
            assert_eq!(witness.brain.held_generation, Some(Counter(1)));
            assert_eq!(witness.config_digest, proof_request.expected_config_digest);
            mutation_id += 1;
            if now % 40 == 0 {
                let heartbeat = crate::brain_control::Request {
                    contract: "GP15-brain".into(),
                    version: 1,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(1),
                    writer: Some(context.writer().into()),
                    lease: Some(lease),
                    request_id: Some(Counter(mutation_id)),
                    expected_revision: Some(witness.revision),
                    command: crate::brain_control::Command::Heartbeat {
                        generation: Counter(1),
                        observed_frame: witness.source_frame,
                    },
                };
                let reply = host
                    .dispatch(&context, serde_json::to_value(heartbeat).unwrap(), now)
                    .unwrap()
                    .unwrap();
                assert_eq!(reply["state"], "pending");
                beats += 1;
            } else {
                assert!(now > 250);
                let renew = AudioRequest {
                    contract: "C-AUDIO".into(),
                    version: 2,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(1),
                    writer: Some(context.writer().into()),
                    lease: Some(lease),
                    request_id: Some(Counter(mutation_id)),
                    expected_revision: Some(witness.revision),
                    command: Command::Renew {},
                };
                let reply = host
                    .dispatch(&context, serde_json::to_value(renew).unwrap(), now)
                    .unwrap()
                    .unwrap();
                assert!(reply["outcome"]["body"]["reason"].is_null());
                assert_eq!(reply["outcome"]["body"]["lease_remaining_ms"], 2000);
                renewals += 1;
            }
        }
        let frame = host.provider().frame();
        host.process_source(now, 1, frame, &capture, &mut playback)
            .unwrap();
        while let Some(reply) = host.poll_reply(&context, now).unwrap() {
            assert!(reply["reason"].is_null(), "{reply}");
        }
        assert_eq!(
            host.provider().brain_snapshot().held_generation,
            Some(Counter(1)),
            "closed at{now}"
        );
    }
    assert_eq!(renewals, 3);
    assert!(beats >= 55);
    assert_eq!(host.provider().frame(), (4 + 2201) * 48);
    drop(host);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn held_proof_actual_authority_fixtures_and_refusals() {
    for inputs in [16, 32, 48] {
        let dir = std::env::temp_dir().join(format!("held-proof-{}-{inputs}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let policy = PolicyStore::new(vec![Peer {
            id: "held-fixture".into(),
            certificate_sha256: fingerprint(b"held-proof"),
            permissions: [Permission::TalkbackDestinations].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"held-proof", 91).unwrap();
        let mut local = LocalAudio::bind_configured(
            &dir,
            "audio",
            SHOW,
            Counter(1),
            EngineTopology::software(inputs, 5, 0).unwrap(),
        )
        .unwrap();
        let grant = AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some(context.writer().into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant {
                scope: Scope::TalkbackDestinations,
            },
        };
        let lease = local
            .engine_mut()
            .handle(&grant, 0)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap();
        let mut host = HostAuthority::new(local, 1).unwrap();
        let r = request(
            context.writer().into(),
            context.session(),
            lease,
            inputs as u32,
        );
        let before = host.provider().brain_snapshot();
        fixture(
            &format!("baseline-raw-{inputs}"),
            &serde_json::to_value(host.provider_mut().snapshot().unwrap()).unwrap(),
        );
        fixture(
            &format!("baseline-brain-{inputs}"),
            &serde_json::to_value(&before).unwrap(),
        );
        let reply = host
            .dispatch(&context, serde_json::to_value(&r).unwrap(), 1)
            .unwrap()
            .unwrap();
        fixture(&format!("proof-{inputs}"), &reply);
        fixture(
            &format!("request-{inputs}"),
            &serde_json::to_value(&r).unwrap(),
        );
        let proof = held_proof::Reply::decode(&serde_json::to_vec(&reply).unwrap()).unwrap();
        assert!(proof.witness.is_some());
        assert_eq!(proof.witness.unwrap().lease_remaining_ms, 1999);
        assert_eq!(before.revision, host.provider().brain_snapshot().revision);
        assert_eq!(before.frame, host.provider().brain_snapshot().frame);
        for (label, mut bad, now, reason) in [
            ("duplicate", r.clone(), 2, "query_id"),
            ("expired", r.clone(), 2000, "lease"),
            ("map", r.clone(), 2, "identity"),
            ("epoch", r.clone(), 2, "identity"),
            ("show", r.clone(), 2, "identity"),
            ("capability", r.clone(), 2, "identity"),
            ("scope", r.clone(), 2, "scope"),
            ("configuration", r.clone(), 2, "config_changed"),
            ("session", r.clone(), 2, "identity"),
        ] {
            if label != "duplicate" {
                bad.query_id = Counter(2);
            }
            match label {
                "map" => bad.map_generation = Counter(2),
                "epoch" => bad.epoch = Counter(2),
                "show" => bad.show_id = "22222222-2222-4222-8222-222222222222".into(),
                "capability" => bad.capability_generation = Counter(2),
                "scope" => bad.scope = Scope::TalkbackFoh,
                "configuration" => {
                    bad.expected_config_digest = digest(inputs as u32, &[0], -1200, true, false)
                }
                "session" => bad.authenticated_session = Counter(92),
                _ => {}
            }
            let reply = host
                .dispatch(&context, serde_json::to_value(&bad).unwrap(), now)
                .unwrap()
                .unwrap();
            assert_eq!(reply["reason"], reason);
            assert!(reply["witness"].is_null());
            if inputs == 16 {
                fixture(label, &reply);
            }
        }
        let no_permissions = PolicyStore::new(vec![Peer {
            id: "held-refused".into(),
            certificate_sha256: fingerprint(b"held-denied"),
            permissions: std::collections::BTreeSet::new(),
        }])
        .unwrap();
        let denied_context = no_permissions.authenticate(b"held-denied", 92).unwrap();
        let denied_request = request(
            denied_context.writer().into(),
            denied_context.session(),
            lease,
            inputs as u32,
        );
        let denied = host
            .dispatch(
                &denied_context,
                serde_json::to_value(denied_request).unwrap(),
                2,
            )
            .unwrap()
            .unwrap();
        assert_eq!(denied["reason"], "permission");
        if inputs == 16 {
            fixture("permission", &denied);
        }
        let mut wrong_writer = r.clone();
        wrong_writer.writer = "some-other-writer".into();
        assert!(
            host.dispatch(&context, serde_json::to_value(wrong_writer).unwrap(), 2)
                .is_err()
        );
        // Refusal neither consumes the next successful nonce nor extends the lease.
        let mut next = r;
        next.query_id = Counter(2);
        let p = host
            .dispatch(&context, serde_json::to_value(next).unwrap(), 3)
            .unwrap()
            .unwrap();
        assert_eq!(p["witness"]["lease_remaining_ms"], 1997);
        // Configuration prepares its cached destination hash off render; the
        // actual source-boundary reapply must preserve that exact cache.
        host.provider_mut().rearm().unwrap();
        let revision = host.provider_mut().engine_mut().revision();
        let config = crate::brain_control::Request {
            contract: "GP15-brain".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some(context.writer().into()),
            lease: Some(lease),
            request_id: Some(Counter(2)),
            expected_revision: Some(revision),
            command: crate::brain_control::Command::TalkbackSet {
                monitors: vec![4, 0],
                gain_cdb: -1200,
                mute: false,
            },
        };
        assert_eq!(
            host.provider_mut()
                .brain_request(config.clone(), 4, true, None)
                .unwrap()
                .state,
            "pending"
        );
        host.provider_mut().tick(4).unwrap();
        host.provider_mut().tick(5).unwrap();
        assert!(
            host.provider_mut()
                .brain_request(config, 6, true, None)
                .unwrap()
                .reason
                .is_none()
        );
        let mut configured = request(
            context.writer().into(),
            context.session(),
            lease,
            inputs as u32,
        );
        configured.query_id = Counter(3);
        let refused = host
            .dispatch(&context, serde_json::to_value(&configured).unwrap(), 6)
            .unwrap()
            .unwrap();
        assert_eq!(refused["reason"], "config_changed");
        configured.expected_config_digest = digest(inputs as u32, &[0, 4], -1200, false, false);
        let reply = host
            .dispatch(&context, serde_json::to_value(&configured).unwrap(), 6)
            .unwrap()
            .unwrap();
        assert!(reply["witness"].is_object());
        if inputs == 16 {
            fixture("configured-source-boundary", &reply);
        }
        let revision = host.provider_mut().engine_mut().revision();
        let hold = crate::brain_control::Request {
            contract: "GP15-brain".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some(context.writer().into()),
            lease: Some(lease),
            request_id: Some(Counter(3)),
            expected_revision: Some(revision),
            command: crate::brain_control::Command::Hold {
                generation: Counter(1),
            },
        };
        assert_eq!(
            host.provider_mut()
                .brain_request(hold.clone(), 7, true, None)
                .unwrap()
                .state,
            "pending"
        );
        host.provider_mut().tick(7).unwrap();
        host.provider_mut().tick(8).unwrap();
        assert!(
            host.provider_mut()
                .brain_request(hold, 9, true, None)
                .unwrap()
                .reason
                .is_none()
        );
        configured.query_id = Counter(4);
        let held = host
            .dispatch(&context, serde_json::to_value(&configured).unwrap(), 9)
            .unwrap()
            .unwrap();
        assert_eq!(held["witness"]["brain"]["held_generation"], "1");
        assert_eq!(
            held["witness"]["config_digest"],
            configured.expected_config_digest
        );
        if inputs == 16 {
            fixture("held", &held);
        }
        let revision = host.provider_mut().engine_mut().revision();
        let renew = AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some(context.writer().into()),
            lease: Some(lease),
            request_id: Some(Counter(4)),
            expected_revision: Some(revision),
            command: Command::Renew {},
        };
        // A compact proof did not authorize the full raw attach prerequisite.
        assert!(
            host.dispatch(&context, serde_json::to_value(&renew).unwrap(), 10)
                .is_err()
        );
        assert!(
            host.provider_mut()
                .engine_mut()
                .handle(&renew, 10)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .reason
                .is_none()
        );
        configured.query_id = Counter(5);
        let renewed = host
            .dispatch(&context, serde_json::to_value(&configured).unwrap(), 11)
            .unwrap()
            .unwrap();
        assert_eq!(renewed["witness"]["lease_remaining_ms"], 1999);
        assert_eq!(
            renewed["witness"]["config_digest"],
            configured.expected_config_digest
        );
        if inputs == 16 {
            fixture("renewed", &renewed);
        }
        if inputs == 16 {
            configured.query_id = Counter(6);
            let clock = host
                .dispatch(&context, serde_json::to_value(&configured).unwrap(), 9)
                .unwrap()
                .unwrap();
            assert_eq!(clock["reason"], "clock");
            fixture("clock", &clock);
            let monitor_lease = scoped_grant(
                &mut host,
                "monitor-fixture",
                Scope::LocalOperatorMonitor,
                12,
            );
            brain_change(
                &mut host,
                "monitor-fixture",
                monitor_lease,
                2,
                13,
                crate::brain_control::Command::MonitorSet {
                    source: crate::brain_control::MonitorSource::Pfl { input: 15 },
                    gain_cdb: -600,
                    mute: true,
                    dim: true,
                    armed: false,
                },
            );
            let monitor = host
                .dispatch(&context, serde_json::to_value(&configured).unwrap(), 16)
                .unwrap()
                .unwrap();
            assert!(monitor["witness"].is_object());
            assert_eq!(
                monitor["witness"]["config_digest"],
                configured.expected_config_digest
            );
            fixture("unrelated-monitor-change", &monitor);
            let foh_lease = scoped_grant(&mut host, "foh-fixture", Scope::TalkbackFoh, 17);
            brain_change(
                &mut host,
                "foh-fixture",
                foh_lease,
                2,
                18,
                crate::brain_control::Command::TalkbackFoh { enabled: true },
            );
            configured.query_id = Counter(7);
            let changed = host
                .dispatch(&context, serde_json::to_value(&configured).unwrap(), 21)
                .unwrap()
                .unwrap();
            assert_eq!(changed["reason"], "config_changed");
            fixture("foh-config-changed", &changed);
            configured.expected_config_digest = digest(16, &[0, 4], -1200, false, true);
            let foh = host
                .dispatch(&context, serde_json::to_value(&configured).unwrap(), 21)
                .unwrap()
                .unwrap();
            assert_eq!(foh["witness"]["foh_authorized"], true);
            fixture("foh-authorized", &foh);
            let revision = host.provider_mut().engine_mut().revision();
            let mut later = renew;
            later.request_id = Some(Counter(5));
            later.expected_revision = Some(revision);
            assert!(
                host.provider_mut()
                    .engine_mut()
                    .handle(&later, 1990)
                    .unwrap()
                    .outcome
                    .unwrap()
                    .body
                    .reason
                    .is_none()
            );
            configured.query_id = Counter(8);
            let expired = host
                .dispatch(&context, serde_json::to_value(&configured).unwrap(), 2018)
                .unwrap()
                .unwrap();
            assert_eq!(expired["witness"]["foh_authorized"], false);
            assert_eq!(expired["witness"]["brain"]["talkback_foh"], true);
            fixture("foh-expired-independent", &expired);
        }
        drop(host);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
