//! Offline tests at the real authenticated producer boundary; no network or PCM.
use super::*;
use crate::{
    control_model::{Command, Request as AudioRequest, Scope},
    lease_maintenance as maintenance,
    local_audio::LocalAudio,
    paired_readback as paired,
    show::Counter,
    topology::EngineTopology,
};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
// Named monitor scopes and multiple configured dynamic scopes exercise the same
// live-lease path. These are fixture dimensions, never a product monitor cap.
const MAINTENANCE_SCOPES: [Scope; 10] = [
    Scope::PaConfiguration,
    Scope::Foh,
    Scope::Monitor1,
    Scope::Monitor2,
    Scope::Monitor(3),
    Scope::Monitor(5),
    Scope::OutputRoutes,
    Scope::LocalOperatorMonitor,
    Scope::TalkbackDestinations,
    Scope::TalkbackFoh,
];

struct Fixture {
    host: HostAuthority,
    context: AuthenticatedContext,
    dir: std::path::PathBuf,
}
impl Fixture {
    fn new(label: &str, inputs: usize) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "atomic-control-{label}-{}-{inputs}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let policy = PolicyStore::new(vec![Peer {
            id: "atomic-fixture".into(),
            certificate_sha256: fingerprint(b"atomic-fixture"),
            permissions: [
                Permission::TalkbackDestinations,
                Permission::TalkbackFoh,
                Permission::LocalOperatorMonitor,
                Permission::Foh,
                Permission::Monitor(1),
                Permission::Monitor(2),
                Permission::Monitor(3),
                Permission::Monitor(5),
                Permission::PaConfiguration,
                Permission::OutputRoutes,
            ]
            .into_iter()
            .collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"atomic-fixture", 95).unwrap();
        let local = LocalAudio::bind_configured(
            &dir,
            "audio",
            SHOW,
            Counter(1),
            EngineTopology::software(inputs, 5, 0).unwrap(),
        )
        .unwrap();
        Self {
            host: HostAuthority::new(local, 1).unwrap(),
            context,
            dir,
        }
    }
    fn read(&self, id: u64) -> paired::Request {
        paired::Request {
            contract: paired::CONTRACT.into(),
            version: 1,
            kind: "readback".into(),
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            authenticated_session: Counter(self.context.session()),
            writer: self.context.writer().into(),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            query_id: Counter(id),
        }
    }
    fn audio(&mut self, command: Command, lease: Option<Counter>, id: u64) -> AudioRequest {
        AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some(self.context.writer().into()),
            lease,
            request_id: Some(Counter(id)),
            expected_revision: Some(self.host.provider_mut().engine_mut().revision()),
            command,
        }
    }
    fn send(&mut self, r: &impl serde::Serialize, now: u64) -> Value {
        self.host
            .dispatch(&self.context, serde_json::to_value(r).unwrap(), now)
            .unwrap()
            .unwrap()
    }
    fn grant(&mut self, scope: Scope) -> maintenance::Request {
        let r = self.read(1);
        assert_eq!(self.send(&r, 0)["state"], "snapshot");
        let grant = self.audio(Command::Grant { scope }, None, 1);
        let value = self.send(&grant, 0);
        let lease: Counter =
            serde_json::from_value(value["outcome"]["body"]["granted_lease"].clone()).unwrap();
        maintenance::Request {
            contract: maintenance::CONTRACT.into(),
            version: 1,
            kind: "maintain".into(),
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            authenticated_session: Counter(self.context.session()),
            writer: self.context.writer().into(),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            maintenance_id: Counter(1),
            scope,
            lease,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn fixture(name: &str, value: &Value) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/atomic-control-v1")
        .join(format!("{name}.json"));
    if std::env::var_os("GIGPIES_UPDATE_ATOMIC_FIXTURES").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
        )
        .unwrap();
    }
    let mut saved: Value = serde_json::from_slice(
        &std::fs::read(path).expect("explicitly generate producer atomic fixtures"),
    )
    .unwrap();
    // Historical producer bytes retain their original 672-byte strip reservation.
    // Only the exact additional gate storage is normalized; other drift still fails.
    if let Some(snapshot) = saved.get_mut("raw")
        && let Some(n) = snapshot["topology"]["inputs"].as_array().map(Vec::len)
    {
        let extra = n * (std::mem::size_of::<crate::channel_processing::Strip>() - 672) * 2;
        let field = &mut snapshot["resources"]["admission"]["estimated_bytes"];
        if let Some(old) = field.as_u64() {
            let actual = value["raw"]["resources"]["admission"]["estimated_bytes"].as_u64();
            if actual == Some(old + extra as u64) {
                *field = serde_json::json!(old + extra as u64);
            }
        }
    }
    assert_eq!(&saved, value, "fixture {name}");
}
#[test]
fn atomic_producer_fixtures() {
    for inputs in [16, 32, 48] {
        let mut f = Fixture::new("fixtures", inputs);
        let request = f.read(1);
        fixture(
            &format!("read-request-{inputs}"),
            &serde_json::to_value(&request).unwrap(),
        );
        let reply = f.send(&request, 0);
        paired::Reply::decode(&serde_json::to_vec(&reply).unwrap()).unwrap();
        fixture(&format!("read-snapshot-{inputs}"), &reply);
        let duplicate = f.send(&request, 0);
        assert_eq!(duplicate["reason"], "query_id");
        if inputs == 16 {
            fixture("read-duplicate", &duplicate);
        }
        // Grant through the latch admitted by this paired read, without a separate raw read.
        let grant = f.audio(
            Command::Grant {
                scope: Scope::TalkbackDestinations,
            },
            None,
            1,
        );
        let granted = f.send(&grant, 0);
        let r = maintenance::Request {
            contract: maintenance::CONTRACT.into(),
            version: 1,
            kind: "maintain".into(),
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            authenticated_session: Counter(f.context.session()),
            writer: f.context.writer().into(),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            maintenance_id: Counter(1),
            scope: Scope::TalkbackDestinations,
            lease: serde_json::from_value(granted["outcome"]["body"]["granted_lease"].clone())
                .unwrap(),
        };
        fixture(
            &format!("maintain-request-{inputs}"),
            &serde_json::to_value(&r).unwrap(),
        );
        let success = f.send(&r, 10);
        maintenance::Reply::decode(&serde_json::to_vec(&success).unwrap()).unwrap();
        assert_eq!(success["state"], "maintained");
        fixture(&format!("maintained-{inputs}"), &success);
        assert_eq!(f.send(&r, 20), success);
        for (label, expected) in [
            ("identity", "identity"),
            ("scope", "scope"),
            ("lease", "lease"),
            ("clock", "clock"),
            ("expired", "lease"),
        ] {
            let mut bad = r.clone();
            let now = match label {
                "identity" => {
                    bad.map_generation = Counter(2);
                    20
                }
                "scope" => {
                    // Canonical and configured, but not the scope of this lease.
                    bad.scope = Scope::Monitor(3);
                    20
                }
                "lease" => {
                    bad.lease = Counter(999);
                    20
                }
                "clock" => 9,
                "expired" => 2010,
                _ => unreachable!(),
            };
            let refusal = f.send(&bad, now);
            assert_eq!(refusal["reason"], expected);
            if inputs == 16 {
                fixture(&format!("maintain-{label}"), &refusal);
            }
        }
    }
}
#[test]
fn atomic_maintenance_every_existing_scope_admitted_by_live_lease() {
    for scope in MAINTENANCE_SCOPES {
        let mut f = Fixture::new(&format!("admission-{scope:?}"), 16);
        let r = f.grant(scope);
        let reply = f.send(&r, 10);
        assert_eq!(reply["state"], "maintained", "{scope:?}: {reply}");
        maintenance::Reply::decode(&serde_json::to_vec(&reply).unwrap()).unwrap();
        let label = match scope {
            Scope::Monitor(n) => format!("monitor-{n}"),
            _ => serde_json::to_value(scope)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned(),
        };
        fixture(
            &format!("scope-{label}-request"),
            &serde_json::to_value(&r).unwrap(),
        );
        fixture(&format!("scope-{label}-maintained"), &reply);
        let mut bad = r.clone();
        bad.scope = if scope == Scope::Foh {
            Scope::Monitor(3)
        } else {
            Scope::Foh
        };
        let refusal = f.send(&bad, 11);
        assert_eq!(refusal["reason"], "scope");
        fixture(&format!("scope-{label}-mismatch"), &refusal);
        bad = r.clone();
        bad.lease = Counter(999);
        assert_eq!(f.send(&bad, 11)["reason"], "lease");
        assert_eq!(f.send(&r, 12), reply, "mismatches cannot overwrite success");
    }
}

#[test]
fn atomic_maintenance_scopes_replay_eviction_and_expiry() {
    for scope in MAINTENANCE_SCOPES {
        let mut f = Fixture::new(&format!("scope-{scope:?}"), 16);
        let mut r = f.grant(scope);
        let revision = f.host.provider_mut().engine_mut().revision();
        let frame = f.host.provider().frame();
        let first = f.send(&r, 100);
        assert_eq!(first["state"], "maintained");
        assert_eq!(f.send(&r, 200), first);
        assert_eq!(f.host.provider_mut().engine_mut().revision(), revision);
        assert_eq!(f.host.provider().frame(), frame);
        for id in 2..=65 {
            r.maintenance_id = Counter(id);
            assert_eq!(f.send(&r, 200)["state"], "maintained");
        }
        r.maintenance_id = Counter(1);
        assert_eq!(f.send(&r, 201)["reason"], "expired_id");
        r.maintenance_id = Counter(65);
        assert_eq!(f.send(&r, 2200)["reason"], "lease");
        assert_eq!(
            f.send(&r, 2199)["reason"],
            "clock",
            "expired lease cannot revive through regressed time"
        );
    }
}
#[test]
fn atomic_all_scope_permissions_contexts_revocation_and_replaced_leases() {
    for scope in MAINTENANCE_SCOPES {
        let mut f = Fixture::new(&format!("permission-{scope:?}"), 16);
        let peer = Peer {
            id: "scope-owner".into(),
            certificate_sha256: fingerprint(b"scope-owner"),
            permissions: [scope_permission(scope)].into_iter().collect(),
        };
        let policy = PolicyStore::new(vec![peer.clone()]).unwrap();
        f.context = policy.authenticate(b"scope-owner", 120).unwrap();
        let r = f.grant(scope);
        let first = f.send(&r, 10);
        assert_eq!(first["state"], "maintained", "{scope:?}");
        // Only the requested scope's permission works; no Brain/FOH/PA or
        // neighboring monitor permission substitutes for it, even on a retry.
        for other in MAINTENANCE_SCOPES {
            if other != scope {
                let mut wrong = r.clone();
                wrong.scope = other;
                assert_eq!(
                    f.send(&wrong, 11)["reason"],
                    "permission",
                    "{scope:?} -> {other:?}"
                );
            }
        }
        for field in [
            "authenticated_session",
            "epoch",
            "capability_generation",
            "map_generation",
            "show_id",
        ] {
            let mut wrong = serde_json::to_value(&r).unwrap();
            wrong[field] = if field == "show_id" {
                json!("22222222-2222-4222-8222-222222222222")
            } else {
                json!("999")
            };
            assert_eq!(
                f.send(&wrong, 11)["reason"],
                "identity",
                "{scope:?}: {field}"
            );
        }
        let mut wrong = r.clone();
        wrong.writer = "other-writer".into();
        assert!(
            f.host
                .dispatch(&f.context, serde_json::to_value(wrong).unwrap(), 11)
                .is_err()
        );
        assert_eq!(f.send(&r, 12), first);
        policy
            .replace(vec![Peer {
                permissions: Default::default(),
                ..peer.clone()
            }])
            .unwrap();
        assert!(
            f.host
                .dispatch(&f.context, serde_json::to_value(&r).unwrap(), 13)
                .is_err(),
            "cached success must not bypass revoked policy: {scope:?}"
        );
        f.host.disconnect(&f.context);
        f.context = policy.authenticate(b"scope-owner", 121).unwrap();
        let mut replacement = r.clone();
        replacement.writer = f.context.writer().into();
        replacement.authenticated_session = Counter(121);
        let refused = f.send(&replacement, 14);
        assert_eq!(refused["reason"], "permission");
        let label = match scope {
            Scope::Monitor(n) => format!("monitor-{n}"),
            _ => serde_json::to_value(scope)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned(),
        };
        fixture(&format!("scope-{label}-permission"), &refused);
        policy.replace(vec![peer]).unwrap();
        f.context = policy.authenticate(b"scope-owner", 122).unwrap();
        replacement.writer = f.context.writer().into();
        replacement.authenticated_session = Counter(122);
        assert_eq!(f.send(&replacement, 15)["reason"], "lease");
        // Explicit new grant is a test of replacement, never automatic recovery.
        let read = f.read(1);
        assert_eq!(f.send(&read, 16)["state"], "snapshot");
        let grant = f.audio(Command::Grant { scope }, None, 1);
        let granted = f.send(&grant, 16);
        let new_lease: Counter =
            serde_json::from_value(granted["outcome"]["body"]["granted_lease"].clone()).unwrap();
        assert_ne!(new_lease, r.lease);
        assert_eq!(f.send(&replacement, 17)["reason"], "lease");
        replacement.lease = new_lease;
        assert_eq!(f.send(&replacement, 17)["state"], "maintained");
        assert!(
            f.host
                .dispatch(&f.context, serde_json::to_value(&r).unwrap(), 18)
                .is_err()
        );
    }
}

#[test]
fn atomic_canonical_monitor_scopes_require_an_existing_configured_lease() {
    let mut f = Fixture::new("canonical-monitor", 16);
    let r = f.grant(Scope::Monitor(5));
    for n in 0..=2 {
        let mut invalid = r.clone();
        invalid.scope = Scope::Monitor(n);
        let refused = f.send(&invalid, 10);
        assert_eq!(refused["reason"], "scope");
        fixture(&format!("scope-noncanonical-monitor-{n}"), &refused);
    }
    for invalid in [
        json!({"foh": null}),
        json!({"monitor1": null}),
        json!({"talkback_destinations": null}),
        json!("monitor3"),
        json!({"monitor": -1}),
        json!({"monitor": 65536}),
        json!({"monitor": "3"}),
        json!({"monitor": 3, "extra": 0}),
        json!("unknown"),
    ] {
        let mut wrong = serde_json::to_value(&r).unwrap();
        wrong["scope"] = invalid;
        assert!(f.host.dispatch(&f.context, wrong, 10).is_err());
    }
    // Canonical u16 syntax is not a topology grant, including the wire endpoint.
    // The configured five-monitor fixture must reject both out-of-topology grants.
    for n in [6, u16::MAX] {
        let policy = PolicyStore::new(vec![Peer {
            id: "out-of-topology".into(),
            certificate_sha256: fingerprint(b"out-of-topology"),
            permissions: [Permission::Monitor(u32::from(n))].into_iter().collect(),
        }])
        .unwrap();
        f.context = policy
            .authenticate(b"out-of-topology", u64::from(n) + 200)
            .unwrap();
        let read = f.read(1);
        assert_eq!(f.send(&read, 12)["state"], "snapshot");
        let grant = f.audio(
            Command::Grant {
                scope: Scope::Monitor(n),
            },
            None,
            1,
        );
        let refused = f.send(&grant, 12);
        assert_eq!(refused["outcome"]["body"]["reason"], "scope");
        let mut ungranted = r.clone();
        ungranted.writer = f.context.writer().into();
        ungranted.authenticated_session = Counter(f.context.session());
        ungranted.scope = Scope::Monitor(n);
        assert!(ungranted.supported_scope());
        assert_eq!(f.send(&ungranted, 12)["reason"], "lease");
    }
}

#[test]
fn atomic_read_only_context_and_unsupported_envelopes() {
    let mut f = Fixture::new("readonly", 17);
    let policy = PolicyStore::new(vec![Peer {
        id: "observer".into(),
        certificate_sha256: fingerprint(b"observer"),
        permissions: Default::default(),
    }])
    .unwrap();
    f.context = policy.authenticate(b"observer", 96).unwrap();
    let r = f.read(1);
    assert_eq!(f.send(&r, 10)["state"], "snapshot");
    let mut bad = serde_json::to_value(&r).unwrap();
    bad["version"] = json!(2);
    assert!(f.host.dispatch(&f.context, bad, 10).is_err());
    let mut changed = r.clone();
    changed.query_id = Counter(2);
    changed.epoch = Counter(2);
    assert_eq!(f.send(&changed, 10)["reason"], "identity");
    let r2 = f.read(2);
    assert_eq!(f.send(&r2, 10)["state"], "snapshot");
    f.host.disconnect(&f.context);
    // New TLS exporter identity has an independent namespace and no lease rights.
    f.context = policy.authenticate(b"observer", 97).unwrap();
    let r = f.read(1);
    assert_eq!(f.send(&r, 11)["state"], "snapshot");
    let mut m = maintenance::Request {
        contract: maintenance::CONTRACT.into(),
        version: 1,
        kind: "maintain".into(),
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        authenticated_session: Counter(97),
        writer: f.context.writer().into(),
        capability_generation: Counter(1),
        map_generation: Counter(1),
        maintenance_id: Counter(1),
        scope: Scope::TalkbackFoh,
        lease: Counter(1),
    };
    assert_eq!(f.send(&m, 11)["reason"], "permission");
    m.version = 2;
    assert!(
        f.host
            .dispatch(&f.context, serde_json::to_value(m).unwrap(), 11)
            .is_err()
    );
}

#[test]
fn atomic_pair_observes_old_then_new_committed_brain_and_raw() {
    let mut f = Fixture::new("committed", 32);
    let mut m = f.grant(Scope::TalkbackDestinations);
    let initial = f.read(2);
    let before = f.send(&initial, 1);
    let request = crate::brain_control::Request {
        contract: "GP15-brain".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some(f.context.writer().into()),
        lease: Some(m.lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(f.host.provider_mut().engine_mut().revision()),
        command: crate::brain_control::Command::TalkbackSet {
            monitors: vec![0, 4],
            gain_cdb: -600,
            mute: false,
        },
    };
    assert_eq!(f.send(&request, 2)["state"], "pending");
    let read = f.read(3);
    let pending = f.send(&read, 3);
    assert_eq!(pending["brain"], before["brain"]);
    assert_eq!(
        pending["raw"]["authority"]["revision"],
        before["raw"]["authority"]["revision"]
    );
    assert_eq!(f.send(&m, 4)["reason"], "unavailable");
    f.host.provider_mut().tick(5).unwrap();
    f.host.provider_mut().tick(6).unwrap();
    let read = f.read(4);
    let after = f.send(&read, 7);
    assert_ne!(after["brain"]["revision"], before["brain"]["revision"]);
    assert_eq!(
        after["raw"]["authority"]["revision"],
        after["brain"]["revision"]
    );
    assert_eq!(after["raw"]["frame"], after["brain"]["frame"]);
    assert_eq!(after["brain"]["talkback_monitors"], json!([0, 4]));
    assert_eq!(f.send(&m, 8)["reason"], "unavailable");
    m.maintenance_id = Counter(2);
    assert_eq!(f.send(&m, 8)["state"], "maintained");
    fixture(
        "maintain-busy",
        &f.send(
            &{
                let mut r = m.clone();
                r.maintenance_id = Counter(1);
                r
            },
            8,
        ),
    );
    fixture("read-pending-old", &pending);
    fixture("read-committed-new", &after);
}

use crate::held_proof::{self, Configuration, Request};
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
fn held_request(writer: String, session: u64, lease: Counter, inputs: u32) -> Request {
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

#[test]
fn atomic_maintenance_under_heartbeat_churn_preserves_legacy_revision_conflicts() {
    let dir = std::env::temp_dir().join(format!(
        "atomic-maintenance-heartbeat-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut peers = vec![Peer {
        id: "held-renew".into(),
        certificate_sha256: fingerprint(b"held-renew"),
        permissions: [Permission::TalkbackDestinations].into_iter().collect(),
    }];
    for (label, scope) in [
        ("passive-pa", Scope::PaConfiguration),
        ("passive-foh", Scope::Foh),
        ("passive-monitor", Scope::Monitor(3)),
    ] {
        peers.push(Peer {
            id: label.into(),
            certificate_sha256: fingerprint(label.as_bytes()),
            permissions: [scope_permission(scope)].into_iter().collect(),
        });
    }
    let policy = PolicyStore::new(peers).unwrap();
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
    let mut passive = Vec::new();
    for (i, (label, scope)) in [
        ("passive-pa", Scope::PaConfiguration),
        ("passive-foh", Scope::Foh),
        ("passive-monitor", Scope::Monitor(3)),
    ]
    .into_iter()
    .enumerate()
    {
        let ctx = policy
            .authenticate(label.as_bytes(), 130 + i as u64)
            .unwrap();
        let read = paired::Request {
            contract: paired::CONTRACT.into(),
            version: 1,
            kind: "readback".into(),
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            authenticated_session: Counter(ctx.session()),
            writer: ctx.writer().into(),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            query_id: Counter(1),
        };
        assert_eq!(
            host.dispatch(&ctx, serde_json::to_value(read).unwrap(), 0)
                .unwrap()
                .unwrap()["state"],
            "snapshot"
        );
        let grant = AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some(ctx.writer().into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant { scope },
        };
        let granted = host
            .dispatch(&ctx, serde_json::to_value(grant).unwrap(), 0)
            .unwrap()
            .unwrap();
        let passive_lease: Counter =
            serde_json::from_value(granted["outcome"]["body"]["granted_lease"].clone()).unwrap();
        passive.push((ctx, scope, passive_lease));
    }
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
    let mut proof_request = held_request(context.writer().into(), context.session(), lease, 16);
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
                    expected_revision: Some(Counter(0)),
                    command: Command::Renew {},
                };
                let legacy = host
                    .dispatch(&context, serde_json::to_value(renew).unwrap(), now)
                    .unwrap()
                    .unwrap();
                assert_eq!(legacy["outcome"]["body"]["reason"], "stale_revision");
                let maintain = maintenance::Request {
                    contract: maintenance::CONTRACT.into(),
                    version: 1,
                    kind: "maintain".into(),
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(1),
                    authenticated_session: Counter(context.session()),
                    writer: context.writer().into(),
                    capability_generation: Counter(1),
                    map_generation: Counter(1),
                    maintenance_id: Counter(renewals + 1),
                    scope: Scope::TalkbackDestinations,
                    lease,
                };
                let reply = host
                    .dispatch(&context, serde_json::to_value(&maintain).unwrap(), now)
                    .unwrap()
                    .unwrap();
                assert_eq!(reply["state"], "maintained");
                assert_eq!(reply["result"]["revision"], json!(witness.revision));
                assert_eq!(reply["result"]["lease_remaining_ms"], 2000);
                for (ctx, scope, passive_lease) in &passive {
                    let legacy = AudioRequest {
                        contract: "C-AUDIO".into(),
                        version: 2,
                        show_id: SHOW.into(),
                        module: "audio".into(),
                        epoch: Counter(1),
                        writer: Some(ctx.writer().into()),
                        lease: Some(*passive_lease),
                        request_id: Some(Counter(renewals + 2)),
                        expected_revision: Some(Counter(0)),
                        command: Command::Renew {},
                    };
                    let refused = host
                        .dispatch(ctx, serde_json::to_value(legacy).unwrap(), now)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        refused["outcome"]["body"]["reason"], "stale_revision",
                        "{scope:?}"
                    );
                    let request = maintenance::Request {
                        authenticated_session: Counter(ctx.session()),
                        writer: ctx.writer().into(),
                        scope: *scope,
                        lease: *passive_lease,
                        ..maintain.clone()
                    };
                    let before = host.provider().brain_snapshot();
                    let maintained = host
                        .dispatch(ctx, serde_json::to_value(request).unwrap(), now)
                        .unwrap()
                        .unwrap();
                    assert_eq!(maintained["state"], "maintained", "{scope:?}: {maintained}");
                    assert_eq!(maintained["result"]["revision"], json!(witness.revision));
                    assert_eq!(
                        serde_json::to_value(host.provider().brain_snapshot()).unwrap(),
                        serde_json::to_value(before).unwrap()
                    );
                }
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
fn atomic_cached_success_cannot_bypass_current_policy_attachment_or_session() {
    let mut f = Fixture::new("current-policy", 16);
    let policy = PolicyStore::new(vec![Peer {
        id: "maintainer".into(),
        certificate_sha256: fingerprint(b"policy-maintainer"),
        permissions: [Permission::TalkbackDestinations].into_iter().collect(),
    }])
    .unwrap();
    f.context = policy.authenticate(b"policy-maintainer", 110).unwrap();
    let r = f.grant(Scope::TalkbackDestinations);
    assert_eq!(f.send(&r, 10)["state"], "maintained");
    let before = f.host.provider().brain_snapshot();
    let mut wrong = r.clone();
    wrong.authenticated_session = Counter(111);
    assert_eq!(f.send(&wrong, 11)["reason"], "identity");
    wrong = r.clone();
    wrong.capability_generation = Counter(2);
    assert_eq!(f.send(&wrong, 11)["reason"], "identity");
    wrong = r.clone();
    wrong.epoch = Counter(2);
    assert_eq!(f.send(&wrong, 11)["reason"], "identity");
    wrong = r.clone();
    wrong.writer = "invented-writer".into();
    assert!(
        f.host
            .dispatch(&f.context, serde_json::to_value(wrong).unwrap(), 11)
            .is_err()
    );
    assert_eq!(f.host.provider().brain_snapshot().revision, before.revision);
    policy
        .replace(vec![Peer {
            id: "maintainer".into(),
            certificate_sha256: fingerprint(b"policy-maintainer"),
            permissions: Default::default(),
        }])
        .unwrap();
    assert!(
        f.host
            .dispatch(&f.context, serde_json::to_value(&r).unwrap(), 12)
            .is_err()
    );
    f.host.disconnect(&f.context);
    f.context = policy.authenticate(b"policy-maintainer", 111).unwrap();
    let mut next = r;
    next.writer = f.context.writer().into();
    next.authenticated_session = Counter(111);
    let reply = f.send(&next, 13);
    assert_eq!(reply["reason"], "permission");
    fixture("maintain-permission", &reply);
    let read = f.read(1);
    assert_eq!(f.send(&read, 13)["state"], "snapshot");
    let mut clock = f.read(2);
    let reply = f.send(&clock, 12);
    assert_eq!(reply["reason"], "clock");
    fixture("read-clock", &reply);
    assert_eq!(
        f.send(&clock, 13)["reason"],
        "query_id",
        "authenticated nonce cannot be replayed after operational refusal"
    );
    clock.epoch = Counter(2);
    let reply = f.send(&clock, 13);
    assert_eq!(reply["reason"], "identity");
    fixture("read-identity", &reply);
}

#[test]
fn atomic_namespaces_are_independent_and_maintenance_never_refreshes_brain_admission() {
    let mut f = Fixture::new("namespaces", 16);
    let r = f.grant(Scope::TalkbackDestinations); // paired query 1
    let held = held_request(f.context.writer().into(), f.context.session(), r.lease, 16);
    assert_eq!(f.send(&held, 1)["state"], "proof"); // independent held query 1
    let maintained = f.send(&r, 1000); // independent maintenance 1
    assert_eq!(maintained["state"], "maintained");
    let brain = crate::brain_control::Request {
        contract: "GP15-brain".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some(f.context.writer().into()),
        lease: Some(r.lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(f.host.provider_mut().engine_mut().revision()),
        command: crate::brain_control::Command::TalkbackSet {
            monitors: vec![0],
            gain_cdb: -1200,
            mute: false,
        },
    };
    assert_eq!(f.send(&brain, 1001)["state"], "pending");
    f.host.provider_mut().tick(1002).unwrap();
    f.host.provider_mut().tick(1003).unwrap();
    assert_eq!(f.send(&brain, 1004)["reason"], "stale_snapshot");
    let read = f.read(2);
    assert_eq!(f.send(&read, 1005)["state"], "snapshot");
    // The old outcome survives the same-lease failed staged commit; no renewal.
    assert_eq!(f.send(&r, 1006), maintained);
}

#[test]
fn master_eq_remote_permission_freshness_maintenance_completion_and_disconnect() {
    use crate::structural_control::{Command as S, Request as R};
    let mut f = Fixture::new("master-eq", 17);
    let maintenance = f.grant(Scope::PaConfiguration);
    let read = R {
        contract: crate::master_eq_wire::CONTRACT.into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: S::MasterEqSnapshot {},
    };
    let reply = f.send(&read, 0);
    assert_eq!(reply["master_eq"]["live_available"], false);
    let command = |revision| R {
        writer: Some(f.context.writer().into()),
        lease: Some(maintenance.lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(revision),
        command: S::MasterEqSet {
            patch_json: "{}".into(),
            program_buses: vec![0, 1],
            owner_instance: Counter(1),
            graph_generation: Counter(1),
            eq_generation: Counter(0),
            map_revision: Counter(1),
        },
        ..read.clone()
    };
    let r = command(f.host.provider_mut().engine_mut().revision());
    assert_eq!(f.send(&r, 0)["state"], "pending");
    assert_eq!(f.send(&maintenance, 0)["reason"], "unavailable");
    let inputs = vec![0.; 17 * 48];
    let mut output = vec![0.; 7 * 48];
    f.host
        .process_source(1, 1, 0, &inputs, &mut output)
        .unwrap();
    f.host
        .process_source(2, 1, 48, &inputs, &mut output)
        .unwrap();
    let final_reply = f.host.poll_reply(&f.context, 2).unwrap().unwrap();
    assert_eq!(final_reply["contract"], crate::master_eq_wire::CONTRACT);
    assert_eq!(final_reply["reason"], "live EQ unavailable");
    assert_eq!(f.send(&r, 2)["reason"], "live EQ unavailable"); // cached final never acquires a pending owner
    f.host.disconnect(&f.context);
    assert!(
        f.host
            .provider_mut()
            .engine_mut()
            .external_boundary()
            .is_none()
    );
}
