#![cfg(all(target_os = "linux", feature = "hardware-host"))]
use crate::{
    control_model::{Command, Request as Audio, Scope},
    local_audio::LocalAudio,
    remote::{AuthorityEndpoint, HostAuthority, Peer, Permission, PolicyStore},
    show::Counter,
    structural_control::{Command as S, Request as R},
    topology::EngineTopology,
};
use serde_json::{Value, json};
use std::{
    io::{ErrorKind, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::PathBuf,
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
struct Pump {
    host: HostAuthority,
}
impl Pump {
    fn block(&mut self, now: u64) {
        let provider = self.host.provider();
        let t = provider.topology();
        let frame = provider.frame();
        let epoch = provider.source_epoch();
        let capture: Vec<_> = (0..48 * t.capture_channels)
            .map(|i| {
                let phase =
                    (frame + (i / t.capture_channels) as u64) as f64 * 173. * std::f64::consts::TAU
                        / 48000.;
                (0.005 * phase.sin() * 8388608.).round() / 8388608.
            })
            .collect();
        let mut playback = vec![0.; 48 * t.playback_channels];
        self.host
            .process_source(now, epoch, frame, &capture, &mut playback)
            .unwrap();
        assert!(playback.iter().all(|x| x.is_finite() && x.abs() <= 1.));
    }
}
struct Client {
    stream: UnixStream,
    bytes: Vec<u8>,
    assembly: crate::snapshot_pages::Assembly,
    inbox: Vec<serde_json::Value>,
    pages: usize,
    assembled_bytes: usize,
}
impl Client {
    fn new(path: &std::path::Path) -> Self {
        let stream = UnixStream::connect(path).unwrap();
        stream.set_nonblocking(true).unwrap();
        Self {
            stream,
            bytes: vec![],
            assembly: Default::default(),
            inbox: vec![],
            pages: 0,
            assembled_bytes: 0,
        }
    }
    fn send(&mut self, value: &impl serde::Serialize) {
        let b = serde_json::to_vec(value).unwrap();
        self.stream
            .write_all(&(b.len() as u32).to_be_bytes())
            .unwrap();
        self.stream.write_all(&b).unwrap();
    }
    fn wait(
        &mut self,
        server: &mut Pump,
        now: &mut u64,
        predicate: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        for _ in 0..200 {
            if let Some(i) = self.inbox.iter().position(&predicate) {
                return self.inbox.remove(i);
            }
            *now += 1;
            server.block(*now);
            let mut b = [0; 8192];
            loop {
                match self.stream.read(&mut b) {
                    Ok(0) => break,
                    Ok(n) => self.bytes.extend_from_slice(&b[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => panic!("{e}"),
                }
            }
            while self.bytes.len() >= 4 {
                let n = u32::from_be_bytes(self.bytes[..4].try_into().unwrap()) as usize;
                if self.bytes.len() < n + 4 {
                    break;
                }
                let mut v: serde_json::Value =
                    serde_json::from_slice(&self.bytes[4..n + 4]).unwrap();
                self.bytes.drain(..n + 4);
                if v["contract"] == "GP14-snapshot-pages" {
                    self.pages += 1;
                    let page = serde_json::from_value(v).unwrap();
                    let Some(bytes) = self.assembly.offer(page, *now).unwrap() else {
                        continue;
                    };
                    self.assembled_bytes = bytes.len();
                    v = serde_json::from_slice(&bytes).unwrap();
                }
                if predicate(&v) {
                    return v;
                }
                self.inbox.push(v);
            }
        }
        panic!("bounded reply timeout")
    }
}

fn read() -> R {
    R {
        contract: "GP18-master-eq".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: S::MasterEqSnapshot {},
    }
}
fn audio(
    writer: Option<&str>,
    lease: Option<Counter>,
    id: Option<u64>,
    revision: Option<Counter>,
    command: Command,
) -> Audio {
    Audio {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: writer.map(str::to_owned),
        lease,
        request_id: id.map(Counter),
        expected_revision: revision,
        command,
    }
}
fn eqset(snapshot: &Value, writer: &str, lease: Counter, id: u64, db: f64) -> R {
    let snap = &snapshot["master_eq"];
    let bank: Value = serde_json::from_str(snap["owner_json"].as_str().unwrap()).unwrap();
    let mut settings = bank["target"].clone();
    settings[0]["eq"][0]["db"] = json!(db);
    R {
        writer: Some(writer.into()),
        lease: Some(lease),
        request_id: Some(Counter(id)),
        expected_revision: Some(serde_json::from_value(snap["revision"].clone()).unwrap()),
        command: S::MasterEqSet {
            patch_json: json!({"version":1,"inputs":settings}).to_string(),
            program_buses: serde_json::from_value(snap["program_buses"].clone()).unwrap(),
            owner_instance: serde_json::from_value(snap["owner_instance"].clone()).unwrap(),
            graph_generation: serde_json::from_value(snap["graph_generation"].clone()).unwrap(),
            eq_generation: serde_json::from_value(snap["eq_generation"].clone()).unwrap(),
            map_revision: serde_json::from_value(snap["map_revision"].clone()).unwrap(),
        },
        ..read()
    }
}
fn attach(c: &mut Client, p: &mut Pump, now: &mut u64, writer: &str) -> Counter {
    c.send(&audio(None, None, None, None, Command::Snapshot {}));
    c.wait(p, now, |v| {
        v["capability"] == "GP03-rendered" && v["snapshot"].is_object()
    });
    let revision = p.host.provider_mut().engine_mut().revision();
    c.send(&audio(
        Some(writer),
        None,
        Some(1),
        Some(revision),
        Command::Grant {
            scope: Scope::PaConfiguration,
        },
    ));
    let grant = c.wait(p, now, |v| {
        v["outcome"]["body"]["granted_lease"].is_string()
    });
    serde_json::from_value(grant["outcome"]["body"]["granted_lease"].clone()).unwrap()
}
fn remote(
    p: &mut Pump,
    ctx: &crate::remote::AuthenticatedContext,
    r: &impl serde::Serialize,
    now: u64,
) -> crate::remote::Result<Option<Value>> {
    let bytes = serde_json::to_vec(r).unwrap();
    let v = crate::remote::decode(&bytes)?;
    p.host.dispatch(ctx, v, now)
}
#[test]
#[ignore = "bounded actual owner Unix/authenticated endpoint campaign; requires GP_EQ_MANIFEST and GP_PA_V2_FIXTURES"]
fn actual_new_owner_unix_remote_success_cancellation_completion_and_cached_final_ownership() {
    let dir = std::env::temp_dir().join(format!("gp18-eq-transports-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut local = LocalAudio::bind_configured(
        &dir,
        "audio",
        SHOW,
        Counter(1),
        EngineTopology::software(17, 3, 8).unwrap(),
    )
    .unwrap();
    local
        .enable_modules(&PathBuf::from(std::env::var_os("GP_EQ_MANIFEST").unwrap()))
        .unwrap();
    let config = std::fs::read(
        PathBuf::from(std::env::var_os("GP_PA_V2_FIXTURES").unwrap()).join("stereo4way.json"),
    )
    .unwrap();
    local.configure_pa(&config).unwrap();
    let mut p = Pump {
        host: HostAuthority::new(local, 1).unwrap(),
    };
    let mut now = 0;
    let mut c = Client::new(&dir.join("audio"));
    let lease = attach(&mut c, &mut p, &mut now, "eq-uds");
    c.send(&read());
    let baseline = c.wait(&mut p, &mut now, |v| v["master_eq"].is_object());
    let first = eqset(&baseline, "eq-uds", lease, 2, 6.125);
    c.send(&first);
    c.wait(&mut p, &mut now, |v| {
        v["state"] == "pending" && v["contract"] == "GP18-master-eq"
    });
    let final_reply = c.wait(&mut p, &mut now, |v| {
        v["state"] == "final" && v["contract"] == "GP18-master-eq"
    });
    assert!(final_reply["reason"].is_null());
    for _ in 0..6 {
        now += 1;
        p.block(now);
    }
    c.send(&read());
    let settled = c.wait(&mut p, &mut now, |v| v["master_eq"].is_object());
    assert_eq!(settled["master_eq"]["settled"], true);
    let next = eqset(&settled, "eq-uds", lease, 3, -3.125);
    c.send(&next);
    c.wait(&mut p, &mut now, |v| v["state"] == "pending");
    c.send(&first);
    let cached = c.wait(&mut p, &mut now, |v| {
        v["state"] == "final" && v["context"]["request_id"] == "2"
    });
    assert_eq!(cached, final_reply);
    let next_final = c.wait(&mut p, &mut now, |v| {
        v["state"] == "final" && v["context"]["request_id"] == "3"
    });
    assert!(next_final["reason"].is_null(), "{next_final}");
    assert!(p.host.provider_mut().take_remote_completions().is_empty());
    for _ in 0..6 {
        now += 1;
        p.block(now);
    }
    c.send(&read());
    let before = c.wait(&mut p, &mut now, |v| v["master_eq"].is_object());
    let cancel = eqset(&before, "eq-uds", lease, 4, 9.);
    c.send(&cancel);
    c.wait(&mut p, &mut now, |v| v["state"] == "pending");
    drop(c);
    now += 1;
    p.block(now);
    assert!(
        p.host
            .provider_mut()
            .engine_mut()
            .external_boundary()
            .is_none()
    );
    let policy = PolicyStore::new(vec![Peer {
        id: "eq-remote".into(),
        certificate_sha256: crate::remote::fingerprint(b"eq-certificate"),
        permissions: [Permission::PaConfiguration].into_iter().collect(),
    }])
    .unwrap();
    let ctx = policy.authenticate(b"eq-certificate", 105).unwrap();
    remote(
        &mut p,
        &ctx,
        &audio(None, None, None, None, Command::Snapshot {}),
        now,
    )
    .unwrap();
    let revision = p.host.provider_mut().engine_mut().revision();
    let grant = remote(
        &mut p,
        &ctx,
        &audio(
            Some(ctx.writer()),
            None,
            Some(1),
            Some(revision),
            Command::Grant {
                scope: Scope::PaConfiguration,
            },
        ),
        now,
    )
    .unwrap()
    .unwrap();
    let remote_lease: Counter =
        serde_json::from_value(grant["outcome"]["body"]["granted_lease"].clone()).unwrap();
    let stale_read = eqset(&before, ctx.writer(), remote_lease, 2, 2.125);
    assert_eq!(
        remote(&mut p, &ctx, &stale_read, now).unwrap().unwrap()["state"],
        "pending"
    );
    for _ in 0..2 {
        now += 1;
        p.block(now);
    }
    let stale_final = p.host.poll_reply(&ctx, now).unwrap().unwrap();
    assert_eq!(stale_final["reason"], "fresh_structural_snapshot_required");
    let snap = remote(&mut p, &ctx, &read(), now).unwrap().unwrap();
    let no_permission = PolicyStore::new(vec![Peer {
        id: "observer".into(),
        certificate_sha256: crate::remote::fingerprint(b"observer"),
        permissions: Default::default(),
    }])
    .unwrap();
    let observer = no_permission.authenticate(b"observer", 106).unwrap();
    let denied = eqset(&snap, observer.writer(), remote_lease, 2, 2.125);
    assert!(remote(&mut p, &observer, &denied, now).is_err());
    let update = eqset(&snap, ctx.writer(), remote_lease, 3, 2.125);
    assert_eq!(
        remote(&mut p, &ctx, &update, now).unwrap().unwrap()["state"],
        "pending"
    );
    p.host.provider_mut().revoke_writer("unrelated");
    for _ in 0..2 {
        now += 1;
        p.block(now);
    }
    let completed = p.host.poll_reply(&ctx, now).unwrap().unwrap();
    assert!(completed["reason"].is_null());
    assert_eq!(completed["context"]["writer"], ctx.writer());
    assert_eq!(
        remote(&mut p, &ctx, &update, now).unwrap().unwrap(),
        completed
    );
    for _ in 0..6 {
        now += 1;
        p.block(now);
    }
    let snap = remote(&mut p, &ctx, &read(), now).unwrap().unwrap();
    let cancel = eqset(&snap, ctx.writer(), remote_lease, 4, 11.);
    assert_eq!(
        remote(&mut p, &ctx, &cancel, now).unwrap().unwrap()["state"],
        "pending"
    );
    p.host.disconnect(&ctx);
    let mut c = Client::new(&dir.join("audio"));
    let lease = attach(&mut c, &mut p, &mut now, "eq-uds-next");
    c.send(&read());
    let snap = c.wait(&mut p, &mut now, |v| v["master_eq"].is_object());
    let update = eqset(&snap, "eq-uds-next", lease, 2, -2.125);
    c.send(&update);
    c.wait(&mut p, &mut now, |v| v["state"] == "pending");
    let final_reply = c.wait(&mut p, &mut now, |v| v["state"] == "final");
    assert!(final_reply["reason"].is_null());
    assert!(p.host.poll_reply(&ctx, now).unwrap().is_none());
    drop(c);
    drop(p);
    std::fs::remove_dir_all(dir).unwrap();
}
