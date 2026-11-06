//! Opt-in private local analysis publisher. All IPC is outside the raw tap.
use super::*;
use std::{
    collections::VecDeque,
    fs,
    io::{ErrorKind, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
};
struct Frame {
    bytes: Vec<u8>,
    offset: usize,
    expires: u64,
}
impl Frame {
    fn new(bytes: &[u8], expires: u64) -> Self {
        let mut out = Vec::with_capacity(bytes.len() + 4);
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
        Self {
            bytes: out,
            offset: 0,
            expires,
        }
    }
}
struct Client {
    stream: UnixStream,
    request: [u8; 128],
    used: usize,
    attached: bool,
    queue: VecDeque<Frame>,
    losses: u64,
    progress: u64,
}
pub struct LocalAnalysis {
    listener: UnixListener,
    path: PathBuf,
    inode: (u64, u64),
    descriptor: Descriptor,
    input: Consumer<Window>,
    clients: Vec<Client>,
    last_now: u64,
}
impl LocalAnalysis {
    pub fn bind(
        directory: &Path,
        descriptor: Descriptor,
        input: Consumer<Window>,
    ) -> Result<Self, String> {
        descriptor.validate().map_err(String::from)?;
        let m = fs::symlink_metadata(directory).map_err(|e| e.to_string())?;
        if !directory.is_absolute()
            || !m.is_dir()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o777 != 0o700
            || fs::canonicalize(directory).map_err(|e| e.to_string())? != directory
        {
            return Err("private canonical 0700 directory".into());
        }
        let path = directory.join("analysis.sock");
        if fs::symlink_metadata(&path).is_ok() {
            return Err("preexisting analysis endpoint".into());
        }
        let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        Ok(Self {
            listener,
            path,
            inode: (m.dev(), m.ino()),
            descriptor,
            input,
            clients: Vec::with_capacity(8),
            last_now: 0,
        })
    }
    pub fn tick(&mut self, now: u64, tap_losses: u64) -> Result<(), String> {
        if now < self.last_now {
            return Err("clock regression".into());
        }
        self.last_now = now;
        let mono = monotonic_ms()?;
        for _ in 0..8 {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.clients.len() == 8 || !same_uid(&stream) {
                        continue;
                    }
                    let size: libc::c_int = 8192;
                    // SAFETY: connected socket and valid integer socket-option storage.
                    if unsafe {
                        libc::setsockopt(
                            stream.as_raw_fd(),
                            libc::SOL_SOCKET,
                            libc::SO_SNDBUF,
                            (&size as *const libc::c_int).cast(),
                            std::mem::size_of_val(&size) as libc::socklen_t,
                        )
                    } != 0
                    {
                        continue;
                    }
                    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
                    self.clients.push(Client {
                        stream,
                        request: [0; 128],
                        used: 0,
                        attached: false,
                        queue: VecDeque::with_capacity(2),
                        losses: 0,
                        progress: now,
                    });
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        self.clients.retain_mut(|c| {
            if !c.attached {
                match c.stream.read(&mut c.request[c.used..]) {
                    Ok(0) => return false,
                    Ok(n) => c.used += n,
                    Err(e)
                        if e.kind() == ErrorKind::WouldBlock
                            || e.kind() == ErrorKind::Interrupted => {}
                    Err(_) => return false,
                }
                if c.used >= 4 {
                    let n = u32::from_be_bytes(c.request[..4].try_into().unwrap()) as usize;
                    if n == 0 || n > 124 || c.used > n + 4 {
                        return false;
                    }
                    if c.used == n + 4 {
                        if &c.request[4..c.used] != self.descriptor.attach_request() {
                            return false;
                        }
                        c.attached = true;
                        c.queue.push_back(Frame::new(
                            &serde_json::to_vec(&self.descriptor).unwrap(),
                            mono.saturating_add(2000),
                        ));
                        c.progress = now;
                    }
                }
                if !c.attached && now.saturating_sub(c.progress) >= 2000 {
                    return false;
                }
            }
            true
        });
        // Consume at most the two global staging windows each pump.
        for _ in 0..2 {
            let Ok(window) = self.input.pop() else { break };
            for c in self.clients.iter_mut().filter(|c| c.attached) {
                if c.queue.len() >= 2 || mono.saturating_sub(window.produced_mono_ms) > 100 {
                    c.losses = c.losses.saturating_add(1);
                    continue;
                }
                if c.queue.is_empty() {
                    c.progress = now;
                }
                c.queue.push_back(Frame::new(
                    &window.wire(
                        c.losses.saturating_add(tap_losses),
                        self.descriptor.map_revision.0,
                        self.descriptor.calibration_revision.0,
                    ),
                    window.produced_mono_ms.saturating_add(100),
                ));
            }
        }
        self.clients.retain_mut(|c| {
            let Some(p) = c.queue.front_mut() else {
                return true;
            };
            if mono > p.expires || now.saturating_sub(c.progress) >= 2000 {
                return false;
            }
            let end = (p.offset + 8192).min(p.bytes.len());
            match c.stream.write(&p.bytes[p.offset..end]) {
                Ok(0) => return false,
                Ok(n) => {
                    p.offset += n;
                    c.progress = now;
                }
                Err(e)
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::Interrupted => {}
                Err(_) => return false,
            }
            if p.offset == p.bytes.len() {
                c.queue.pop_front();
            }
            true
        });
        Ok(())
    }
}
fn same_uid(stream: &UnixStream) -> bool {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: connected socket, valid sized credential storage.
    unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        ) == 0
            && len as usize == std::mem::size_of::<libc::ucred>()
            && cred.uid == libc::geteuid()
    }
}
impl Drop for LocalAnalysis {
    fn drop(&mut self) {
        if let Ok(m) = fs::symlink_metadata(&self.path)
            && (m.dev(), m.ino()) == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Cross-process same-host age clock; invoked only outside the tap.
pub fn monotonic_ms() -> Result<u64, String> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid timespec storage and read-only monotonic clock query.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err("monotonic clock".into());
    }
    Ok(time.tv_sec as u64 * 1000 + time.tv_nsec as u64 / 1_000_000)
}

/// Owns only the bounded IPC pump, never the engine. Drop joins outside processing.
/// Source-side liveness/loss signaling, without worker teardown ownership.
#[derive(Clone)]
pub struct SourceSignal {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    losses: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl SourceSignal {
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn alive(&self) -> bool {
        !self.stop.load(std::sync::atomic::Ordering::Acquire)
            && self.alive.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn update_losses(&self, losses: u64) {
        self.losses
            .store(losses, std::sync::atomic::Ordering::Relaxed);
    }
}
pub struct Worker {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    losses: std::sync::Arc<std::sync::atomic::AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(mut service: LocalAnalysis) -> Result<Self, String> {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        };
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let losses = Arc::new(AtomicU64::new(0));
        let (s, a, l) = (stop.clone(), alive.clone(), losses.clone());
        let thread = std::thread::Builder::new()
            .name("gp04-analysis".into())
            .spawn(move || {
                struct AliveGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
                impl Drop for AliveGuard {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let _alive_guard = AliveGuard(a.clone());
                let origin = std::time::Instant::now();
                while !s.load(Ordering::Relaxed) {
                    if service
                        .tick(
                            origin.elapsed().as_millis() as u64,
                            l.load(Ordering::Relaxed),
                        )
                        .is_err()
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                a.store(false, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            alive,
            losses,
            thread: Some(thread),
        })
    }
    pub fn source_signal(&self) -> SourceSignal {
        SourceSignal {
            stop: self.stop.clone(),
            alive: self.alive.clone(),
            losses: self.losses.clone(),
        }
    }
    pub fn update_losses(&self, losses: u64) {
        self.losses
            .store(losses, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn alive(&self) -> bool {
        !self.stop.load(std::sync::atomic::Ordering::Relaxed)
            && self.alive.load(std::sync::atomic::Ordering::Acquire)
            && self
                .thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
    }
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finished_panicking_worker_is_unavailable_without_joining_engine() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64},
        };
        let worker = Worker {
            stop: Arc::new(AtomicBool::new(false)),
            alive: Arc::new(AtomicBool::new(true)),
            losses: Arc::new(AtomicU64::new(0)),
            thread: Some(std::thread::spawn(|| panic!("invented transport failure"))),
        };
        // Wait for the injected failure itself before checking the observer.
        // Panic hooks and concurrent tests may outlive a fixed 100ms sleep loop.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !worker.thread.as_ref().unwrap().is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "panic fixture did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!worker.alive());
    }
    #[test]
    fn slow_reader_drops_independently_and_partial_age_is_hard_deadline() {
        let path = std::env::temp_dir().join(format!("gp04-pump-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let d = Descriptor::new(9, 0);
        let (mut tap, input) = Tap::new(&d).unwrap();
        let mut service = LocalAnalysis::bind(&path, d, input).unwrap();
        let (slow, _slow_peer) = UnixStream::pair().unwrap();
        slow.set_nonblocking(true).unwrap();
        let size: libc::c_int = 1024;
        // SAFETY: valid socket and integer option storage.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    slow.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&size as *const libc::c_int).cast(),
                    std::mem::size_of_val(&size) as libc::socklen_t,
                )
            },
            0
        );
        let (fast, mut fast_peer) = UnixStream::pair().unwrap();
        fast.set_nonblocking(true).unwrap();
        fast_peer
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        for stream in [slow, fast] {
            service.clients.push(Client {
                stream,
                request: [0; 128],
                used: 0,
                attached: true,
                queue: VecDeque::with_capacity(2),
                losses: 0,
                progress: 0,
            });
        }
        for window in 0..20 {
            for packet in 0..10 {
                let f = (window * 10 + packet) * 48;
                tap.offer(f, &synthetic_inputs(f), monotonic_ms().unwrap())
                    .unwrap();
            }
            service.tick(window, 0).unwrap();
            let mut bytes = [0; WIRE_BYTES + 4];
            fast_peer.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes[4..8], b"GAW1");
            assert_eq!(u64::from_be_bytes(bytes[8..16].try_into().unwrap()), 0);
        }
        assert!(service.clients[0].losses > 0);
        assert_eq!(service.clients[1].losses, 0);
        // A partial stalled frame expires even when last-write progress is fresh.
        let partial = service.clients[0].queue.front_mut().unwrap();
        assert!(partial.offset > 0);
        partial.expires = 0;
        service.clients[0].progress = 20;
        service.tick(20, 0).unwrap();
        assert_eq!(service.clients.len(), 1);
        drop(service);
        fs::remove_dir(&path).unwrap();
    }
}
