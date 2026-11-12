use gigpies::{
    control_model::{Command, Request, Scope},
    local_audio::LocalAudio,
    show::Counter,
    structural_control::{self, Command as S},
    topology::EngineTopology,
};
use std::os::unix::fs::PermissionsExt;
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
fn setup(label: &str) -> (std::path::PathBuf, LocalAudio) {
    let dir = std::env::temp_dir().join(format!("gp14-structure-{}-{label}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = LocalAudio::bind_configured(
        &dir,
        "audio",
        SHOW,
        Counter(1),
        EngineTopology::software(17, 5, 0).unwrap(),
    )
    .unwrap();
    (dir, host)
}
fn grant(host: &mut LocalAudio, scope: Scope) -> Counter {
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
    host.engine_mut()
        .handle(&r, 0)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap()
}
fn request(lease: Counter, command: S) -> structural_control::Request {
    structural_control::Request {
        contract: "GP14-structure".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("desk".into()),
        lease: Some(lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(Counter(0)),
        command,
    }
}
#[test]
fn patch_uses_same_boundary_revision_and_retry_history() {
    let (dir, mut h) = setup("patch");
    let lease = grant(&mut h, Scope::OutputRoutes);
    let mut outputs = h.topology().outputs.clone();
    outputs[0].source = None;
    let r = request(
        lease,
        S::OutputPatch {
            outputs: outputs.clone(),
        },
    );
    let pending = h.structural_request(r.clone(), 1, true, None).unwrap();
    assert_eq!(pending.state, "pending");
    assert_eq!(pending.effective_frame, Some(Counter(48)));
    h.tick(1).unwrap();
    assert_eq!(h.engine_mut().revision(), Counter(0));
    h.tick(2).unwrap();
    assert_eq!(h.topology().outputs, outputs);
    assert_eq!(h.engine_mut().revision(), Counter(1));
    let retry = h.structural_request(r, 3, false, None).unwrap();
    assert_eq!(retry.state, "final");
    assert_eq!(retry.reason, None);
    assert_eq!(retry.effective_frame, Some(Counter(48)));
    assert_eq!(retry.revision, Counter(1));
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn stale_preparation_fails_atomically_and_consumes_id() {
    let (dir, mut h) = setup("stale");
    let lease = grant(&mut h, Scope::OutputRoutes);
    let original = h.topology().clone();
    let mut outputs = original.outputs.clone();
    outputs[0].source = None;
    let r = request(lease, S::OutputPatch { outputs });
    h.structural_request(r.clone(), 1, false, None).unwrap();
    h.tick(1).unwrap();
    h.tick(2).unwrap();
    assert_eq!(h.topology(), &original);
    assert_eq!(h.engine_mut().revision(), Counter(0));
    let reply = h.structural_request(r, 3, true, None).unwrap();
    assert_eq!(
        reply.reason.as_deref(),
        Some("fresh_structural_snapshot_required")
    );
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn recovery_is_durable_quiesced_and_cancels_permission() {
    let (dir, mut h) = setup("recover");
    grant(&mut h, Scope::PaConfiguration);
    h.tick(1).unwrap();
    h.quiesce_source("injected_lock_loss").unwrap();
    h.recover_source(Counter(9), 0).unwrap();
    assert!(h.structural_snapshot().unwrap().outputs_quiesced);
    assert_eq!(
        h.structural_snapshot().unwrap().clock.adat_lock,
        gigpies::clock_domain::LockEvidence::Unknown
    );
    drop(h);
    assert!(
        LocalAudio::bind_configured(
            &dir,
            "audio",
            SHOW,
            Counter(8),
            EngineTopology::software(17, 5, 0).unwrap()
        )
        .is_err()
    );
    let h = LocalAudio::bind_configured(
        &dir,
        "audio",
        SHOW,
        Counter(10),
        EngineTopology::software(17, 5, 0).unwrap(),
    )
    .unwrap();
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn composed_restore_preserves_intent_but_never_permissions_or_arm() {
    let (dir, mut h) = setup("persist");
    let intent = h.persisted_intent().unwrap();
    let decoded = structural_control::Intent::decode(&intent.encode().unwrap()).unwrap();
    h.restore_composed_intent(&decoded).unwrap();
    assert!(h.structural_snapshot().unwrap().outputs_quiesced);
    assert_eq!(h.structural_snapshot().unwrap().epoch, Counter(1));
    let mut wrong = decoded;
    wrong.engine.topology.monitors += 1;
    assert!(h.restore_composed_intent(&wrong).is_err());
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn revoked_preparation_cannot_apply_under_new_writer() {
    let (dir, mut h) = setup("revoke");
    let lease = grant(&mut h, Scope::OutputRoutes);
    let original = h.topology().outputs.clone();
    let mut old = original.clone();
    old[0].source = None;
    h.structural_request(
        request(lease, S::OutputPatch { outputs: old }),
        1,
        true,
        None,
    )
    .unwrap();
    h.revoke_writer("desk");
    let g = Request {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("other".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(0)),
        command: Command::Grant {
            scope: Scope::OutputRoutes,
        },
    };
    let lease = h
        .engine_mut()
        .handle(&g, 2)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap();
    let mut next = original.clone();
    next[1].source = None;
    let mut r = request(
        lease,
        S::OutputPatch {
            outputs: next.clone(),
        },
    );
    r.writer = Some("other".into());
    h.structural_request(r, 3, true, None).unwrap();
    h.tick(3).unwrap();
    h.tick(4).unwrap();
    assert_eq!(h.topology().outputs, next);
    assert_eq!(h.topology().outputs[0], original[0]);
    assert_eq!(h.engine_mut().revision(), Counter(1));
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
#[cfg(feature = "hardware-host")]
#[test]
fn physical_acceptance_rejects_reference_and_allows_only_signal_patch() {
    use gigpies::host::duplex::{DeviceAcceptance, same_device_map};
    use sha2::{Digest, Sha256};
    let mut t = EngineTopology::reference_16_18(8, 5).unwrap();
    let mut record = DeviceAcceptance {
        device: "hw:unattached".into(),
        topology_sha256: gigpies::hash::hex(Sha256::digest(serde_json::to_vec(&t).unwrap())),
        native_significant_bits: 24,
        socket_mapping_record: "synthetic test record".into(),
        single_clock_setup_record: "synthetic test record".into(),
    };
    assert!(record.validate("hw:unattached", &t).is_err());
    t.mapping_evidence = "operator-verified".into();
    record.topology_sha256 = gigpies::hash::hex(Sha256::digest(serde_json::to_vec(&t).unwrap()));
    assert!(record.validate("hw:unattached", &t).is_ok());
    let mut patch = t.clone();
    patch.map_revision += 1;
    patch.outputs[0].source = Some(gigpies::topology::OutputSource::Pa { index: 0 });
    assert!(same_device_map(&t, &patch));
    patch.outputs[0].playback_slot = 19;
    assert!(!same_device_map(&t, &patch));
    assert!(record.validate("hw:unattached", &patch).is_err());
}

#[cfg(feature = "hardware-host")]
#[test]
fn pump_admission_rejects_initial_provider_main_bypass_on_same_device_map() {
    use gigpies::{
        host::duplex::{same_device_map, validate_provider_map},
        topology::OutputSource,
    };
    let mut accepted = EngineTopology::reference_16_18(8, 5).unwrap();
    accepted.mapping_evidence = "operator-verified".into();
    accepted.outputs[0].source = Some(OutputSource::Pa { index: 0 });
    let mut provider = accepted.clone();
    provider.outputs[0].source = Some(OutputSource::Main { channel: 0 });
    // This is an initial independently constructed provider, not a structural
    // patch that would already pass the LocalAudio routing authority check.
    assert!(same_device_map(&provider, &accepted));
    assert!(validate_provider_map(&provider, &accepted).is_err());
    provider.pa_outputs = 0;
    assert!(validate_provider_map(&provider, &accepted).is_err());
    provider.pa_outputs = accepted.pa_outputs;
    provider.outputs[0].source = Some(OutputSource::Pa { index: 1 });
    assert!(validate_provider_map(&provider, &accepted).is_ok());
    provider.outputs[0].source = None;
    assert!(validate_provider_map(&provider, &accepted).is_ok());
    provider.outputs[0].playback_slot = 19;
    assert!(validate_provider_map(&provider, &accepted).is_err());
}

fn setup_request(
    lease: Counter,
    id: u64,
    revision: u64,
    command: S,
) -> structural_control::Request {
    structural_control::Request {
        request_id: Some(Counter(id)),
        expected_revision: Some(Counter(revision)),
        ..request(lease, command)
    }
}
#[test]
fn setup_hold_survives_writer_loss_and_requires_separate_release_and_rearm() {
    let (dir, mut h) = setup("setup-hold");
    let lease = grant(&mut h, Scope::PaConfiguration);
    let r = setup_request(lease, 2, 0, S::SetupHold {});
    assert_eq!(
        h.structural_request(r.clone(), 0, true, None)
            .unwrap()
            .state,
        "pending"
    );
    h.tick(1).unwrap();
    h.tick(2).unwrap();
    assert!(
        h.structural_request(r, 3, false, None)
            .unwrap()
            .reason
            .is_none()
    );
    let query = structural_control::Request {
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: S::SetupSnapshot {},
        ..setup_request(lease, 3, 1, S::SetupSnapshot {})
    };
    let snapshot = h
        .structural_request(query.clone(), 3, true, None)
        .unwrap()
        .snapshot
        .unwrap();
    assert_eq!(snapshot.setup_hold, Some(true));
    assert!(snapshot.outputs_quiesced);
    assert!(h.rearm().is_err());
    let r = setup_request(lease, 3, 1, S::OutputRearm {});
    assert_eq!(
        h.structural_request(r.clone(), 3, true, None)
            .unwrap()
            .state,
        "pending"
    );
    h.tick(4).unwrap();
    h.tick(5).unwrap();
    assert!(
        h.structural_request(r, 6, false, None)
            .unwrap()
            .reason
            .unwrap()
            .contains("setup")
    );
    h.revoke_writer("desk");
    assert_eq!(
        h.structural_request(query.clone(), 6, true, None)
            .unwrap()
            .snapshot
            .unwrap()
            .setup_hold,
        Some(true)
    );
    let new_grant = Request {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("recovery".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(h.engine_mut().revision()),
        command: Command::Grant {
            scope: Scope::PaConfiguration,
        },
    };
    let lease = h
        .engine_mut()
        .handle(&new_grant, 7)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap();
    let r = structural_control::Request {
        writer: Some("recovery".into()),
        ..setup_request(lease, 2, h.engine_mut().revision().0, S::SetupRelease {})
    };
    h.structural_request(r.clone(), 7, true, None).unwrap();
    h.tick(8).unwrap();
    h.tick(9).unwrap();
    assert!(
        h.structural_request(r, 10, false, None)
            .unwrap()
            .reason
            .is_none()
    );
    let snapshot = h
        .structural_request(query, 10, true, None)
        .unwrap()
        .snapshot
        .unwrap();
    assert_eq!(snapshot.setup_hold, Some(false));
    assert!(snapshot.outputs_quiesced);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn setup_hold_closes_brain_and_blocks_activation_after_source_recovery() {
    use gigpies::brain_control::{Command as B, MonitorSource, Request as BrainRequest};
    let (dir, mut h) = setup("setup-brain");
    assert!(
        !serde_json::to_value(h.structural_snapshot().unwrap())
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("setup_hold")
    );
    let lease = grant(&mut h, Scope::PaConfiguration);
    let r = setup_request(lease, 2, 0, S::SetupHold {});
    h.structural_request(r.clone(), 0, true, None).unwrap();
    h.tick(1).unwrap();
    h.tick(2).unwrap();
    assert!(
        h.structural_request(r, 3, false, None)
            .unwrap()
            .reason
            .is_none()
    );
    let grant = Request {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("monitor".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(h.engine_mut().revision()),
        command: Command::Grant {
            scope: Scope::LocalOperatorMonitor,
        },
    };
    let lease = h
        .engine_mut()
        .handle(&grant, 3)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap();
    let r = BrainRequest {
        contract: "GP15-brain".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("monitor".into()),
        lease: Some(lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(h.engine_mut().revision()),
        command: B::MonitorSet {
            source: MonitorSource::Main,
            gain_cdb: 0,
            mute: false,
            dim: false,
            armed: true,
        },
    };
    h.brain_request(r.clone(), 4, true, None).unwrap();
    h.tick(4).unwrap();
    h.tick(5).unwrap();
    let reply = h.brain_request(r, 6, false, None).unwrap();
    assert_eq!(reply.reason.as_deref(), Some("audio_setup_hold"));
    assert!(!h.brain_snapshot().monitor_armed);
    h.quiesce_source("software interruption").unwrap();
    h.recover_source(Counter(9), 0).unwrap();
    assert!(h.rearm().is_err());
    assert!(h.structural_snapshot().unwrap().outputs_quiesced);
    let query = structural_control::Request {
        contract: "GP14-structure".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: S::SetupSnapshot {},
    };
    assert_eq!(
        h.structural_request(query, 7, true, None)
            .unwrap()
            .snapshot
            .unwrap()
            .setup_hold,
        Some(true)
    );
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
