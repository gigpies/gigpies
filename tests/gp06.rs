// Linux-only private synthetic IPC regressions.
#![cfg(target_os = "linux")]
use gigpies::{
    control_model::{Command, Edit, Request, Scope, Target, Value},
    local_audio::LocalAudio,
    mixer_control::RenderedReply,
    show::Counter,
};
use std::{
    collections::VecDeque,
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixStream,
    },
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "gp06-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn request(writer: &str, command: Command, id: u64, rev: u64, lease: Option<Counter>) -> Request {
    let snapshot = matches!(command, Command::Snapshot {});
    Request {
        contract: "C-AUDIO".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: (!snapshot).then(|| writer.into()),
        lease,
        request_id: (!snapshot).then_some(Counter(id)),
        expected_revision: (!snapshot).then_some(Counter(rev)),
        command,
    }
}
struct Client {
    socket: UnixStream,
    bytes: Vec<u8>,
    replies: VecDeque<RenderedReply>,
}
impl Client {
    fn new(dir: &Temp) -> Self {
        let socket = UnixStream::connect(dir.0.join("audio.sock")).unwrap();
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            bytes: Vec::new(),
            replies: VecDeque::new(),
        }
    }
    fn send(&mut self, r: &Request) {
        let b = r.encode().unwrap();
        self.socket
            .write_all(&(b.len() as u32).to_be_bytes())
            .unwrap();
        self.socket.write_all(&b).unwrap();
    }
    fn read(&mut self) {
        let mut b = [0; 8192];
        for _ in 0..16 {
            match self.socket.read(&mut b) {
                Ok(0) => break,
                Ok(n) => self.bytes.extend_from_slice(&b[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => panic!("{e}"),
            }
        }
        while self.bytes.len() >= 4 {
            let n = u32::from_be_bytes(self.bytes[..4].try_into().unwrap()) as usize;
            assert!(n <= 65536);
            if self.bytes.len() < n + 4 {
                break;
            }
            self.replies
                .push_back(RenderedReply::decode(&self.bytes[4..n + 4]).unwrap());
            self.bytes.drain(..n + 4);
        }
    }
    fn wait(
        &mut self,
        s: &mut LocalAudio,
        id: Option<Counter>,
        state: &str,
        now: u64,
    ) -> RenderedReply {
        for _ in 0..80 {
            s.tick(now).unwrap();
            self.read();
            if let Some(index) = self
                .replies
                .iter()
                .position(|r| r.context.request_id == id && r.state == state)
            {
                return self.replies.remove(index).unwrap();
            }
        }
        panic!("missing reply {id:?} {state}")
    }
    fn call(&mut self, s: &mut LocalAudio, r: &Request, state: &str, now: u64) -> RenderedReply {
        self.send(r);
        self.wait(s, r.request_id, state, now)
    }
    fn grant(
        &mut self,
        s: &mut LocalAudio,
        writer: &str,
        scope: Scope,
        rev: u64,
        now: u64,
    ) -> Counter {
        self.call(
            s,
            &request("", Command::Snapshot {}, 0, 0, None),
            "final",
            now,
        );
        self.call(
            s,
            &request(writer, Command::Grant { scope }, 1, rev, None),
            "final",
            now,
        )
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap()
    }
}
#[test]
fn actual_ipc_commits_renderer_retries_and_preserves_mix_after_loss() {
    let dir = Temp::new();
    let mut s = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    assert_eq!(
        fs::metadata(dir.0.join("audio.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let mut c = Client::new(&dir);
    let l = c.grant(&mut s, "desk-1", Scope::Foh, 0, 0);
    let set = request(
        "desk-1",
        Command::Set {
            targets: vec![Edit {
                target: Target::Fader {
                    input: "input-01".into(),
                },
                value: Value::Integer(0),
            }],
        },
        41,
        0,
        Some(l),
    );
    let pending = c.call(&mut s, &set, "pending", 0);
    assert!(pending.outcome.is_none());
    let applied = c.wait(&mut s, Some(Counter(41)), "final", 0);
    assert_eq!(applied.outcome.as_ref().unwrap().body.revision, Counter(1));
    assert_eq!(
        c.call(&mut s, &set, "final", 1).encode().unwrap(),
        applied.encode().unwrap()
    );
    for _ in 0..10 {
        s.tick(1).unwrap();
        c.read();
    }
    assert_eq!(
        s.snapshot().unwrap().coefficients[0].current_nanogain[0],
        1_000_000_000
    );
    drop(c);
    let before = s.frame();
    for _ in 0..10 {
        s.tick(2500).unwrap();
    }
    assert!(s.frame() > before);
    assert_eq!(
        s.snapshot().unwrap().coefficients[0].current_nanogain[0],
        1_000_000_000
    );
    let mut fresh = Client::new(&dir);
    let _new = fresh.grant(&mut s, "desk-new", Scope::Foh, 1, 2500);
    assert_eq!(s.snapshot().unwrap().authority.revision, Counter(1));
}
#[test]
fn fragmentation_malformed_lengths_incomplete_timeout_and_capacity() {
    let dir = Temp::new();
    let mut s = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    let mut c = Client::new(&dir);
    let r = request("", Command::Snapshot {}, 0, 0, None)
        .encode()
        .unwrap();
    let header = (r.len() as u32).to_be_bytes();
    for byte in header {
        c.socket.write_all(&[byte]).unwrap();
        s.tick(0).unwrap();
    }
    for bytes in r.chunks(13) {
        c.socket.write_all(bytes).unwrap();
        s.tick(0).unwrap();
    }
    assert!(c.wait(&mut s, None, "final", 0).snapshot.is_some());
    drop(c);
    s.tick(0).unwrap();
    for length in [0_u32, 65537, u32::MAX] {
        let mut socket = UnixStream::connect(dir.0.join("audio.sock")).unwrap();
        socket.write_all(&length.to_be_bytes()).unwrap();
        socket.set_nonblocking(true).unwrap();
        // A nonblocking accept/read may make no progress (WouldBlock or EINTR).
        // Observe the peer closing, rather than assuming one tick consumed the
        // header. Keep provider time fixed: this must be length refusal, not
        // the production incomplete-frame timeout.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            s.tick(0).unwrap();
            match socket.read(&mut [0; 1]) {
                Ok(0) => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => break,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "length {length} was not refused"
                    );
                    std::thread::yield_now();
                }
                other => panic!("unexpected response to malformed length {length}: {other:?}"),
            }
        }
        assert_eq!(s.client_count(), 0, "malformed length {length}");
    }
    let mut partial = UnixStream::connect(dir.0.join("audio.sock")).unwrap();
    partial.write_all(&[0]).unwrap();
    s.tick(0).unwrap();
    assert_eq!(s.client_count(), 1);
    s.tick(2000).unwrap();
    assert_eq!(s.client_count(), 0);
    drop(partial);
    let clients: Vec<_> = (0..5)
        .map(|_| UnixStream::connect(dir.0.join("audio.sock")).unwrap())
        .collect();
    s.tick(2000).unwrap();
    assert_eq!(s.client_count(), 4);
    drop(clients);
    // A nonblocking EOF observation may need another service iteration. Keep
    // provider time fixed so this proves disconnect, not an idle timeout.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while s.client_count() != 0 {
        s.tick(2000).unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "closed clients were not retired"
        );
        std::thread::yield_now();
    }
    assert_eq!(s.client_count(), 0);
}
#[test]
fn new_connection_requires_snapshot_grant_and_cannot_import_old_identity() {
    let dir = Temp::new();
    let mut s = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    let mut c = Client::new(&dir);
    let raw = request("desk-1", Command::Grant { scope: Scope::Foh }, 1, 0, None);
    c.send(&raw);
    s.tick(0).unwrap();
    assert_eq!(s.client_count(), 0);
    drop(c);
    let mut c = Client::new(&dir);
    let l = c.grant(&mut s, "desk-1", Scope::Foh, 0, 0);
    let second = request(
        "desk-1",
        Command::Grant {
            scope: Scope::Monitor1,
        },
        2,
        0,
        None,
    );
    assert_eq!(
        c.call(&mut s, &second, "final", 0).outcome.unwrap().kind,
        "rejected"
    );
    drop(c);
    s.tick(0).unwrap();
    let mut reconnect = Client::new(&dir);
    reconnect.call(
        &mut s,
        &request("", Command::Snapshot {}, 0, 0, None),
        "final",
        0,
    );
    reconnect.send(&raw);
    s.tick(0).unwrap();
    assert_eq!(s.client_count(), 0);
    drop(reconnect);
    let mut reconnect = Client::new(&dir);
    reconnect.call(
        &mut s,
        &request("", Command::Snapshot {}, 0, 0, None),
        "final",
        0,
    );
    reconnect.send(&request("desk-1", Command::Renew {}, 3, 0, Some(l)));
    s.tick(0).unwrap();
    assert_eq!(s.client_count(), 0);
}
#[test]
fn independent_scope_client_cannot_modify_foh_and_lease_expiry_keeps_graph() {
    let dir = Temp::new();
    let mut s = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    let mut foh = Client::new(&dir);
    foh.grant(&mut s, "foh", Scope::Foh, 0, 0);
    let mut monitor = Client::new(&dir);
    let l = monitor.grant(&mut s, "monitor", Scope::Monitor1, 0, 0);
    let bad = request(
        "monitor",
        Command::Set {
            targets: vec![Edit {
                target: Target::Fader {
                    input: "input-01".into(),
                },
                value: Value::Integer(0),
            }],
        },
        2,
        0,
        Some(l),
    );
    assert_eq!(
        monitor
            .call(&mut s, &bad, "final", 0)
            .outcome
            .unwrap()
            .body
            .reason
            .as_deref(),
        Some("scope")
    );
    let before = s.snapshot().unwrap().coefficients[0]
        .ramp_target_nanogain
        .clone();
    let expired = request("monitor", Command::Renew {}, 3, 0, Some(l));
    assert_eq!(
        monitor
            .call(&mut s, &expired, "final", 2000)
            .outcome
            .unwrap()
            .body
            .reason
            .as_deref(),
        Some("lease")
    );
    assert_eq!(
        s.snapshot().unwrap().coefficients[0].ramp_target_nanogain,
        before
    );
}
#[test]
fn filesystem_refusals_preserve_foreign_endpoint_and_cleanup_only_own_inode() {
    let dir = Temp::new();
    let link = dir.0.join("directory-link");
    symlink(&dir.0, &link).unwrap();
    assert!(LocalAudio::bind(&link, "audio.sock", SHOW, Counter(9)).is_err());
    fs::remove_file(link).unwrap();
    assert!(
        LocalAudio::bind(
            std::path::Path::new("/tmp"),
            "gp06-unowned-refusal.sock",
            SHOW,
            Counter(9)
        )
        .is_err()
    );
    let path = dir.0.join("audio.sock");
    fs::write(&path, b"preserve").unwrap();
    assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"preserve");
    fs::remove_file(&path).unwrap();
    symlink("missing", &path).unwrap();
    assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).is_err());
    fs::remove_file(&path).unwrap();
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).is_err());
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700)).unwrap();
    let s = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"replacement").unwrap();
    drop(s);
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
}
#[test]
fn headless_binary_runs_explicitly_and_gracefully_removes_own_socket() {
    let dir = Temp::new();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_gigpies-headless"))
        .args([
            "--directory",
            dir.0.to_str().unwrap(),
            "--show",
            SHOW,
            "--epoch",
            "9",
            "--ticks",
            "300",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while !dir.0.join("audio.sock").exists()
        && started.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(dir.0.join("audio.sock").exists());
    let mut socket = UnixStream::connect(dir.0.join("audio.sock")).unwrap();
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let bytes = request("", Command::Snapshot {}, 0, 0, None)
        .encode()
        .unwrap();
    // The endpoint can answer before its first render tick. Require observed
    // progress within the existing deadline rather than scheduling-dependent
    // progress in the very first valid snapshot.
    loop {
        socket
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        socket.write_all(&bytes).unwrap();
        let mut header = [0; 4];
        socket.read_exact(&mut header).unwrap();
        let length = u32::from_be_bytes(header) as usize;
        assert!(length <= 65536);
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        if RenderedReply::decode(&body)
            .unwrap()
            .snapshot
            .unwrap()
            .frame
            .0
            > 0
        {
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "headless source failed to advance"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    fn exchange(socket: &mut UnixStream, r: &Request) -> RenderedReply {
        let bytes = r.encode().unwrap();
        socket
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        socket.write_all(&bytes).unwrap();
        for _ in 0..32 {
            let mut header = [0; 4];
            socket.read_exact(&mut header).unwrap();
            let length = u32::from_be_bytes(header) as usize;
            assert!(length <= 65536);
            let mut body = vec![0; length];
            socket.read_exact(&mut body).unwrap();
            let reply = RenderedReply::decode(&body).unwrap();
            if reply.context.request_id == r.request_id && reply.state == "final" {
                return reply;
            }
        }
        panic!("missing final child reply")
    }
    let grant = exchange(
        &mut socket,
        &request(
            "binary-desk",
            Command::Grant { scope: Scope::Foh },
            1,
            0,
            None,
        ),
    );
    let lease = grant.outcome.unwrap().body.granted_lease.unwrap();
    let applied = exchange(
        &mut socket,
        &request(
            "binary-desk",
            Command::Set {
                targets: vec![Edit {
                    target: Target::Fader {
                        input: "input-01".into(),
                    },
                    value: Value::Integer(0),
                }],
            },
            2,
            0,
            Some(lease),
        ),
    );
    assert_eq!(applied.outcome.unwrap().body.revision, Counter(1));
    assert!(applied.effective_frame.is_some());
    std::thread::sleep(std::time::Duration::from_millis(20));
    // Discard any older latest telemetry before querying the committed endpoint.
    let snapshot = exchange(&mut socket, &request("", Command::Snapshot {}, 0, 0, None));
    assert_eq!(
        snapshot.snapshot.unwrap().coefficients[0].ramp_target_nanogain[0],
        1_000_000_000
    );
    assert!(child.wait().unwrap().success());
    assert!(!dir.0.join("audio.sock").exists());
}
#[test]
fn stalled_reader_or_reply_overload_disconnects_without_stopping_engine() {
    let dir = Temp::new();
    let mut s = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    let mut socket = UnixStream::connect(dir.0.join("audio.sock")).unwrap();
    let r = request("", Command::Snapshot {}, 0, 0, None)
        .encode()
        .unwrap();
    let mut batch = Vec::new();
    for _ in 0..120 {
        batch.extend_from_slice(&(r.len() as u32).to_be_bytes());
        batch.extend_from_slice(&r);
    }
    socket.write_all(&batch).unwrap();
    for _ in 0..120 {
        s.tick(0).unwrap();
    }
    assert_eq!(s.client_count(), 0);
    let before = s.frame();
    s.tick(2500).unwrap();
    assert_eq!(s.frame(), before + 48);
}
#[test]
fn partially_sent_telemetry_finishes_before_control_reply_without_interleaving() {
    let dir = Temp::new();
    let mut s = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    let mut c = Client::new(&dir);
    let l = c.grant(&mut s, "interleave-desk", Scope::Foh, 0, 0);
    while (s.frame() / 48) % 16 != 15 {
        s.tick(0).unwrap();
        c.read();
    }
    c.replies.clear();
    s.tick(0).unwrap();
    c.read();
    assert!(
        !c.bytes.is_empty(),
        "telemetry is larger than per-tick8KiB writebudget"
    );
    assert!(c.replies.is_empty());
    let renew = request("interleave-desk", Command::Renew {}, 2, 0, Some(l));
    c.send(&renew);
    s.tick(1).unwrap();
    c.read();
    assert!(c.replies.iter().all(|r| r.context.request_id.is_none()));
    let outcome = c.wait(&mut s, Some(Counter(2)), "final", 1);
    assert_eq!(outcome.outcome.unwrap().kind, "applied");
    assert!(c.bytes.is_empty());
}
#[test]
fn durable_epoch_guard_rejects_two_restart_replays_and_preserves_record() {
    let dir = Temp::new();
    let server = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap();
    assert!(LocalAudio::bind(&dir.0, "other.sock", SHOW, Counter(10)).is_err());
    drop(server);
    let record = fs::read(dir.0.join("audio.identity")).unwrap();
    assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).is_err());
    assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(8)).is_err());
    assert_eq!(fs::read(dir.0.join("audio.identity")).unwrap(), record);
    for epoch in [10, 11] {
        let mut server = LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(epoch)).unwrap();
        let mut client = Client::new(&dir);
        client.send(&request("old-desk", Command::Snapshot {}, 0, 0, None));
        let reply = client.wait(&mut server, None, "final", 0);
        assert_eq!(reply.outcome.unwrap().body.reason.as_deref(), Some("epoch"));
        drop(client);
        drop(server);
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(epoch)).is_err());
    }
}

#[test]
fn lost_identity_and_interrupted_first_reservation_fail_closed() {
    for initialized in [false, true] {
        let dir = Temp::new();
        if initialized {
            drop(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap());
            fs::remove_file(dir.0.join("audio.identity")).unwrap();
        } else {
            // Crash after durable owner creation but before first registry rename.
            fs::write(dir.0.join("audio.owner"), []).unwrap();
            fs::set_permissions(dir.0.join("audio.owner"), fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        for epoch in [9, 10] {
            assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(epoch)).is_err());
            assert!(!dir.0.join("audio.sock").exists());
            assert!(!dir.0.join("audio.identity").exists());
            assert!(dir.0.join("audio.owner").exists());
        }
    }
}

#[test]
fn strict_identity_corruption_and_exhaustion_never_listen_or_reset() {
    for record in [
        r#"{"version":2,"show":"11111111-1111-4111-8111-111111111111","high_epoch":"9"}"#,
        r#"{"version":1,"show":"22222222-2222-4222-8222-222222222222","high_epoch":"9"}"#,
        r#"{"version":1,"show":"11111111-1111-4111-8111-111111111111","high_epoch":"0"}"#,
        r#"{"version":1,"show":"11111111-1111-4111-8111-111111111111","high_epoch":"09"}"#,
        r#"{"version":1,"show":"11111111-1111-4111-8111-111111111111","high_epoch":"18446744073709551616"}"#,
        r#"{"version":1,"show":"11111111-1111-4111-8111-111111111111","high_epoch":"18446744073709551615"}"#,
        r#"{"version":1,"version":1,"show":"11111111-1111-4111-8111-111111111111","high_epoch":"9"}"#,
        r#"{"version":1,"show":"11111111-1111-4111-8111-111111111111","high_epoch":"9","extra":0}"#,
        "not JSON",
    ] {
        let dir = Temp::new();
        drop(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap());
        let path = dir.0.join("audio.identity");
        fs::write(&path, record).unwrap();
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(u64::MAX)).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), record);
        assert!(!dir.0.join("audio.sock").exists());
    }
    let dir = Temp::new();
    drop(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(u64::MAX)).unwrap());
    let bytes = fs::read(dir.0.join("audio.identity")).unwrap();
    for epoch in [0, 1, u64::MAX] {
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(epoch)).is_err());
        assert_eq!(fs::read(dir.0.join("audio.identity")).unwrap(), bytes);
    }
}

#[test]
fn private_identity_files_refuse_symlinks_permissions_and_oversize() {
    for name in ["audio.owner", "audio.identity"] {
        let dir = Temp::new();
        drop(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(9)).unwrap());
        let path = dir.0.join(name);
        let original = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(10)).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let other = dir.0.join("preserved");
        fs::rename(&path, &other).unwrap();
        symlink(&other, &path).unwrap();
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(10)).is_err());
        assert_eq!(fs::read(&other).unwrap(), original);
        assert!(!dir.0.join("audio.sock").exists());
        fs::remove_file(&path).unwrap();
        fs::rename(&other, &path).unwrap();
        fs::hard_link(&path, &other).unwrap();
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(10)).is_err());
        fs::remove_file(&other).unwrap();
        fs::write(&path, vec![b' '; 4097]).unwrap();
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(10)).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), 4097);
    }
}

struct ChildService(std::process::Child);
impl ChildService {
    fn start(dir: &Temp, epoch: u64) -> Self {
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_gigpies-headless"))
            .args([
                "--directory",
                dir.0.to_str().unwrap(),
                "--show",
                SHOW,
                "--epoch",
                &epoch.to_string(),
                "--ticks",
                "5000",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut service = Self(child);
        let started = std::time::Instant::now();
        while !dir.0.join("audio.sock").exists()
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            assert!(
                service.0.try_wait().unwrap().is_none(),
                "child startup failed"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(dir.0.join("audio.sock").exists());
        service
    }
    fn crash(&mut self) {
        self.0.kill().unwrap();
        assert!(!self.0.wait().unwrap().success());
    }
}
impl Drop for ChildService {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child_exchange(socket: &mut UnixStream, request: &Request) -> RenderedReply {
    let bytes = request.encode().unwrap();
    socket
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    socket.write_all(&bytes).unwrap();
    for _ in 0..64 {
        let mut header = [0; 4];
        socket.read_exact(&mut header).unwrap();
        let length = u32::from_be_bytes(header) as usize;
        assert!((1..=65536).contains(&length));
        let mut body = vec![0; length];
        socket.read_exact(&mut body).unwrap();
        let reply = RenderedReply::decode(&body).unwrap();
        if reply.context.request_id == request.request_id && reply.state == "final" {
            return reply;
        }
    }
    panic!("missing final child response");
}
#[test]
fn actual_two_crashes_reject_old_commands_and_preserve_stale_endpoint() {
    use std::os::unix::fs::MetadataExt;
    let dir = Temp::new();
    let old_grant = request(
        "crashed-desk",
        Command::Grant { scope: Scope::Foh },
        1,
        0,
        None,
    );
    let old_set = request(
        "crashed-desk",
        Command::Set {
            targets: vec![Edit {
                target: Target::Fader {
                    input: "input-01".into(),
                },
                value: Value::Integer(-3000),
            }],
        },
        41,
        0,
        Some(Counter(1)),
    );
    for epoch in [9, 10, 11] {
        let mut child = ChildService::start(&dir, epoch);
        let mut socket = UnixStream::connect(dir.0.join("audio.sock")).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        socket
            .set_write_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut snapshot = request("", Command::Snapshot {}, 0, 0, None);
        snapshot.epoch = Counter(epoch);
        assert!(child_exchange(&mut socket, &snapshot).snapshot.is_some());
        if epoch == 9 {
            assert_eq!(
                child_exchange(&mut socket, &old_grant)
                    .outcome
                    .unwrap()
                    .body
                    .granted_lease,
                Some(Counter(1))
            );
            assert_eq!(
                child_exchange(&mut socket, &old_set).outcome.unwrap().kind,
                "applied"
            );
        } else {
            assert_eq!(
                child_exchange(&mut socket, &old_grant)
                    .outcome
                    .unwrap()
                    .body
                    .reason
                    .as_deref(),
                Some("epoch")
            );
            // Fresh current-epoch permission may deliberately recreate writer/lease1.
            // Even then an uncertain prior-epoch command can never regain validity.
            let mut fresh_grant = old_grant.clone();
            fresh_grant.epoch = Counter(epoch);
            assert_eq!(
                child_exchange(&mut socket, &fresh_grant)
                    .outcome
                    .unwrap()
                    .body
                    .granted_lease,
                Some(Counter(1))
            );
            assert_eq!(
                child_exchange(&mut socket, &old_set)
                    .outcome
                    .unwrap()
                    .body
                    .reason
                    .as_deref(),
                Some("epoch")
            );
            let observed = child_exchange(&mut socket, &snapshot).snapshot.unwrap();
            assert_eq!(observed.authority.revision, Counter(0));
        }
        child.crash();
        drop(socket);
        let path = dir.0.join("audio.sock");
        let inode = fs::symlink_metadata(&path).unwrap().ino();
        let registry = fs::read(dir.0.join("audio.identity")).unwrap();
        // Stale endpoint is preserved, never implicitly unlinked/replaced by startup.
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(epoch + 1)).is_err());
        assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), inode);
        assert_eq!(fs::read(dir.0.join("audio.identity")).unwrap(), registry);
        // Only this test's confirmed dead child's exact endpoint is removed.
        fs::remove_file(&path).unwrap();
        assert!(LocalAudio::bind(&dir.0, "audio.sock", SHOW, Counter(epoch)).is_err());
        assert!(!path.exists());
    }
}
