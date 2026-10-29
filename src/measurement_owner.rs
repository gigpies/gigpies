//! One startup-created offline owner worker. Bounded pipes, child cancellation and reap.
use crate::show::Result;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::fd::AsRawFd,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub mode: String,
    pub owner_executable: PathBuf,
    pub owner_sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offline_evidence: Vec<OfflineEvidence>,
}
/// Explicit operator-supplied offline files, never capture authentication.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineEvidence {
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
}
impl OfflineEvidence {
    pub fn load(&self) -> Result<serde_json::Value> {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::OpenOptionsExt;
        if !crate::measurement_wire::identifier(&self.id) || !self.path.is_absolute() {
            return Err("offline evidence identity/path".into());
        }
        // Reject symlinks, special files and FIFOs without blocking or opening PCM.
        let metadata = std::fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err("offline evidence regular file/capacity".into());
        }
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
            .map_err(|e| e.to_string())?;
        if !f.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("offline evidence regular file".into());
        }
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut f)
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if format!("{:x}", Sha256::digest(&bytes)) != self.sha256 {
            return Err("offline evidence hash".into());
        }
        let v: serde_json::Value = decode_document(&bytes, 1024 * 1024)?;
        if v["contract"] != "C-PA-MEASUREMENT" || v["version"] != 2 || v["operation"] != "eq_sweeps"
        {
            return Err("offline evidence requires owner v2 eq_sweeps request".into());
        }
        Ok(v)
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.version, 1 | 2)
            || self.offline_evidence.len() > 4
            || (self.version == 1 && !self.offline_evidence.is_empty())
            || self.mode != "software-only"
            || !self.owner_executable.is_absolute()
            || self.owner_sha256.len() != 64
            || !self
                .owner_sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("measurement startup identity".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for evidence in &self.offline_evidence {
            if !ids.insert(&evidence.id) {
                return Err("duplicate offline evidence ID".into());
            }
        }
        self.verify()
    }
    fn verify(&self) -> Result<()> {
        use sha2::{Digest, Sha256};
        let mut f = std::fs::File::open(&self.owner_executable).map_err(|e| e.to_string())?;
        let meta = f.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > 32 * 1024 * 1024 {
            return Err("measurement executable capacity".into());
        }
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut f)
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if format!("{:x}", Sha256::digest(&bytes)) != self.owner_sha256 {
            return Err("measurement executable hash".into());
        }
        Ok(())
    }
}
pub enum Payload {
    Json(serde_json::Value),
    Samples {
        capture: serde_json::Value,
        options: crate::measurement_wire::Options,
        reference: Vec<f64>,
        mic: Vec<f64>,
    },
}
impl Payload {
    fn value(self) -> serde_json::Value {
        match self {
            Self::Json(v) => v,
            Self::Samples {
                capture,
                options,
                reference,
                mic,
            } => {
                serde_json::json!({"contract":"C-PA-MEASUREMENT","version":1,"operation":"analyze","capture":capture,"options":options,"reference":reference,"mic":mic})
            }
        }
    }
}
pub struct Job {
    pub token: u64,
    pub request: Payload,
    pub candidate: Option<(serde_json::Value, String)>,
}
pub struct Completion {
    pub token: u64,
    pub result: Result<(serde_json::Value, Option<serde_json::Value>)>,
}
pub struct Worker {
    tx: Option<SyncSender<Job>>,
    rx: Receiver<Completion>,
    generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(config: Config) -> Result<Self> {
        config.validate()?;
        let (tx, input) = mpsc::sync_channel::<Job>(1);
        let (output, rx) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(1));
        let stop = Arc::new(AtomicBool::new(false));
        let (g, s) = (generation.clone(), stop.clone());
        let thread = thread::Builder::new()
            .name("gp20-pa-measure".into())
            .spawn(move || {
                while !s.load(Ordering::Acquire) {
                    let job = match input.recv_timeout(Duration::from_millis(10)) {
                        Ok(job) => job,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    let result = (|| {
                        let request = job.request.value();
                        let result = run(&config, &request, job.token, &g, &s)?;
                        let proposal = if result["contract"] == "C-PA-OVERLAP-REVIEW" { &result["proposal"] } else { &result };
                        let candidate = if let Some((configuration, revision)) = job.candidate {
                            if matches!(
                                proposal.get("status").and_then(|v| v.as_str()),
                                Some("proposed" | "no_change")
                            ) && !(proposal["contract"]=="C-PA-EQ-PROPOSAL" && proposal["status"]=="no_change") {
                                let eq=proposal["contract"]=="C-PA-EQ-PROPOSAL";
                                let request = serde_json::json!({
                                    "contract":"C-PA-MEASUREMENT", "version":if eq {2} else {1},
                                    "operation":if eq {"eq_candidate"} else {"candidate"}, "configuration_revision":revision,
                                    "configuration":configuration, "proposal":proposal
                                });
                                Some(run(&config, &request, job.token, &g, &s)?)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        Ok((result, candidate))
                    })();
                    // A cancelled consumer may retain an old completion. Never block this
                    // worker on it; the host has a bounded terminal deadline independently.
                    let _ = output.try_send(Completion {
                        token: job.token,
                        result,
                    });
                }
            })
            .map_err(|e| e.to_string())?;

        Ok(Self {
            tx: Some(tx),
            rx,
            generation,
            stop,
            thread: Some(thread),
        })
    }
    pub fn token(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        if self
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
            .is_err()
        {
            self.stop.store(true, Ordering::Release);
        }
    }
    pub fn submit(&self, job: Job) -> Result<()> {
        if self.stop.load(Ordering::Acquire) || job.token != self.token() {
            return Err("measurement worker stopped or stale job".into());
        }
        self.tx
            .as_ref()
            .ok_or("worker closed")?
            .try_send(job)
            .map_err(|_| "measurement worker busy".into())
    }
    pub fn poll(&self) -> Option<Completion> {
        self.rx.try_recv().ok()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.cancel();
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
fn nonblocking(fd: i32) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("owner pipe flags".into());
    }
    Ok(())
}
fn run(
    config: &Config,
    request: &serde_json::Value,
    token: u64,
    generation: &AtomicU64,
    stop: &AtomicBool,
) -> Result<serde_json::Value> {
    if stop.load(Ordering::Acquire) || generation.load(Ordering::Acquire) != token {
        return Err("cancelled".into());
    }
    config.verify()?;
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("owner input capacity".into());
    }
    let mut child = Command::new(&config.owner_executable)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or("owner stdout")?;
        nonblocking(stdin.as_ref().ok_or("owner stdin")?.as_raw_fd())?;
        nonblocking(stdout.as_raw_fd())?;
        let start = Instant::now();
        let mut written = 0;
        let mut output = Vec::new();
        let mut eof = false;
        loop {
            if stop.load(Ordering::Acquire) || generation.load(Ordering::Acquire) != token {
                return Err("cancelled".into());
            }
            if start.elapsed() > Duration::from_secs(5) {
                return Err("owner deadline".into());
            }
            if let Some(input) = stdin.as_mut() {
                match input.write(&bytes[written..(written + 8192).min(bytes.len())]) {
                    Ok(0) => return Err("owner stdin closed".into()),
                    Ok(n) => written += n,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => return Err(e.to_string()),
                }
                if written == bytes.len() {
                    stdin = None;
                }
            }
            let mut buf = [0; 8192];
            match stdout.read(&mut buf) {
                Ok(0) => eof = true,
                Ok(n) => {
                    if output.len() + n > 256 * 1024 {
                        return Err("owner result capacity".into());
                    }
                    output.extend_from_slice(&buf[..n]);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e.to_string()),
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())?
                && eof
            {
                let v: serde_json::Value = decode_document(&output, 256 * 1024)?;
                if !status.success() && status.code() != Some(2) {
                    return Err("owner exit".into());
                }
                return Ok(v);
            }
            thread::sleep(Duration::from_millis(1));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}
/// Separate owner-document decoder: finite floats allowed, duplicates still refused.
/// Never used for the existing integer-only control envelope.
pub fn decode_document<T: serde::de::DeserializeOwned>(bytes: &[u8], maximum: usize) -> Result<T> {
    if bytes.len() > maximum {
        return Err("owner document capacity".into());
    }
    struct Strict(serde_json::Value);
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct V;
            impl<'de> serde::de::Visitor<'de> for V {
                type Value = Strict;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("bounded duplicate-free finite owner JSON")
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    v: bool,
                ) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Strict, E> {
                    serde_json::Number::from_f64(v)
                        .map(|n| Strict(n.into()))
                        .ok_or_else(|| E::custom("nonfinite"))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                    self.visit_string(v.into())
                }
                fn visit_string<E: serde::de::Error>(
                    self,
                    v: String,
                ) -> std::result::Result<Strict, E> {
                    if v.len() > 262144 {
                        return Err(E::custom("owner string capacity"));
                    }
                    Ok(Strict(v.into()))
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Strict, E> {
                    Ok(Strict(serde_json::Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut v = Vec::new();
                    while let Some(Strict(x)) = a.next_element()? {
                        if v.len() == 65536 {
                            return Err(serde::de::Error::custom("owner array capacity"));
                        }
                        v.push(x);
                    }
                    Ok(Strict(v.into()))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut v = serde_json::Map::new();
                    while let Some(k) = a.next_key::<String>()? {
                        if k.len() > 128 || v.len() == 64 || v.contains_key(&k) {
                            return Err(serde::de::Error::custom(
                                "owner object capacity or duplicate key",
                            ));
                        }
                        let Strict(x) = a.next_value()?;
                        v.insert(k, x);
                    }
                    Ok(Strict(v.into()))
                }
            }
            d.deserialize_any(V)
        }
    }
    let Strict(v) = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    serde_json::from_value(v).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_document_preserves_floats_but_rejects_duplicate_nonfinite_and_unknown_fields() {
        let v: serde_json::Value = decode_document(br#"{"bins":[0.25,-1.5,2]}"#, 100).unwrap();
        assert_eq!(v["bins"][0], 0.25);
        for bad in [
            br#"{"x":{"a":1,"a":2}}"#.as_slice(),
            br#"{"x":1e999}"#,
            br#"{"x":NaN}"#,
        ] {
            assert!(decode_document::<serde_json::Value>(bad, 100).is_err());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Strict {
            _x: f64,
        }
        assert!(decode_document::<Strict>(br#"{"_x":0.1,"extra":2}"#, 100).is_err());
        assert!(decode_document::<serde_json::Value>(b"[1,2]", 4).is_err());
    }
    #[test]
    fn worker_generation_exhaustion_and_stale_submission_fail_closed() {
        use sha2::{Digest, Sha256};
        let path = std::fs::canonicalize("/usr/bin/true").unwrap();
        let hash = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
        let w = Worker::start(Config {
            version: 1,
            mode: "software-only".into(),
            owner_executable: path,
            owner_sha256: hash,
            offline_evidence: vec![],
        })
        .unwrap();
        let token = w.token();
        w.cancel();
        assert!(
            w.submit(Job {
                token,
                request: Payload::Json(serde_json::json!({})),
                candidate: None
            })
            .is_err()
        );
        w.generation.store(u64::MAX, Ordering::Release);
        w.cancel();
        assert_eq!(w.token(), u64::MAX);
        assert!(w.stop.load(Ordering::Acquire));
        assert!(
            w.submit(Job {
                token: u64::MAX,
                request: Payload::Json(serde_json::json!({})),
                candidate: None
            })
            .is_err()
        );
    }
}
