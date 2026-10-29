use gigpies::{
    measurement_wire::{self as wire, Command, Options},
    show::Counter,
};
fn request(command: Command) -> wire::Request {
    wire::Request {
        contract: wire::CONTRACT.into(),
        version: 1,
        show_id: "11111111-1111-4111-8111-111111111111".into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("desk".into()),
        lease: Some(Counter(1)),
        request_id: Some(Counter(2)),
        expected_revision: Some(Counter(0)),
        command,
    }
}
#[test]
fn measurement_wire_strict_bounds_and_fingerprint() {
    let r = request(Command::CaptureStart {
        id: "a".into(),
        reference_input: "input-01".into(),
        mic_capture_slot: 16,
        output_index: 0,
        position_id: "p1".into(),
        samples: 32768,
        options: Options::default(),
    });
    let bytes = serde_json::to_vec(&r).unwrap();
    assert_eq!(wire::Request::decode(&bytes).unwrap(), r);
    let mut v = serde_json::to_value(&r).unwrap();
    v["extra"] = true.into();
    assert!(wire::Request::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut v = serde_json::to_value(&r).unwrap();
    v["body"]["samples"] = 65537.into();
    assert!(wire::Request::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    let duplicate = String::from_utf8(bytes)
        .unwrap()
        .replace("\"version\":1", "\"version\":1,\"version\":1");
    assert!(wire::Request::decode(duplicate.as_bytes()).is_err());
    let mut changed = r.clone();
    changed.request_id = Some(Counter(3));
    assert_ne!(r.fingerprint().unwrap(), changed.fingerprint().unwrap());
}
#[test]
fn measurement_physical_startup_and_wrong_hash_refused() {
    let c = gigpies::measurement_owner::Config {
        version: 1,
        mode: "physical".into(),
        owner_executable: "/usr/bin/true".into(),
        owner_sha256: "0".repeat(64),
    };
    assert!(c.validate().is_err());
    let mut c = c;
    c.mode = "software-only".into();
    assert!(c.validate().is_err());
}
#[test]
fn unavailable_provider_has_honest_snapshot_and_shared_scope_refusal() {
    use gigpies::{
        control_model::{Command as C, Request as A, Scope},
        local_audio::LocalAudio,
        topology::EngineTopology,
    };
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("gp20-unavailable-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut h = LocalAudio::bind_configured(
        &dir,
        "audio",
        "11111111-1111-4111-8111-111111111111",
        Counter(1),
        EngineTopology::software(16, 2, 0).unwrap(),
    )
    .unwrap();
    assert!(!h.measurement_snapshot().available);
    let mut a = A {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: "11111111-1111-4111-8111-111111111111".into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("desk".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(0)),
        command: C::Grant { scope: Scope::Foh },
    };
    let lease = h
        .engine_mut()
        .handle(&a, 0)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap();
    a.lease = Some(lease);
    let mut r = request(Command::CaptureCancel { id: "x".into() });
    r.lease = Some(lease);
    let p = h.measurement_request(r, 1, true, None).unwrap();
    assert!(p.reason.is_some());
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn new_owner_operations_require_explicit_v2_without_changing_v1() {
    for command in [
        Command::MeasurementOverlap {
            id: "overlap".into(),
            pairs: vec![["a".into(), "b".into()]],
        },
        Command::MeasurementEq {
            id: "eq".into(),
            captures: vec!["a".into()],
            target: "flat".into(),
        },
    ] {
        let mut r = request(command);
        assert!(r.validate().is_err());
        r.version = 2;
        assert_eq!(
            wire::Request::decode(&serde_json::to_vec(&r).unwrap()).unwrap(),
            r
        );
    }
    let mut invalid = request(Command::MeasurementEq {
        id: "eq".into(),
        captures: vec!["a".into()],
        target: "arbitrary".into(),
    });
    invalid.version = 2;
    assert!(invalid.validate().is_err());
    assert!(
        request(Command::MeasurementPropose {
            id: "v1".into(),
            pairs: vec![["a".into(), "b".into()]]
        })
        .validate()
        .is_ok()
    );
}
