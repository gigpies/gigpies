//! Explicit opt-in private Unix stream binding for the offline synthetic graph.
//! One control owner; no device, TCP, service installation or realtime deadline.
use crate::{
    control_model::{Command, Request},
    mixer_control::{OfflineEngine, RenderedReply},
    show::{Counter, Result},
};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{ErrorKind, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
};
mod fx_aux;
mod measurement;
pub(crate) mod source;
pub const MAX_CLIENTS: usize = 4;
pub const MAX_FRAME: usize = 65536;
pub const MAX_REPLIES: usize = 32;
const IO_BUDGET: usize = 8192;
const TIMEOUT_MS: u64 = 2000;
struct Endpoint {
    path: PathBuf,
    dev: u64,
    ino: u64,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Ok(m) = fs::symlink_metadata(&self.path)
            && m.file_type().is_socket()
            && m.dev() == self.dev
            && m.ino() == self.ino
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
struct Packet {
    bytes: Vec<u8>,
    offset: usize,
}
impl Packet {
    fn paged(bytes: Vec<u8>) -> Result<Self> {
        let pages = crate::snapshot_pages::encode(bytes)?;
        let size = pages
            .iter()
            .try_fold(0usize, |sum, p| sum.checked_add(p.len() + 4))
            .ok_or("paged reply overflow")?;
        let mut bytes = Vec::with_capacity(size);
        for page in pages {
            bytes.extend_from_slice(&(page.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&page);
        }
        Ok(Self { bytes, offset: 0 })
    }

    fn new(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME {
            return Err("frame capacity".into());
        }
        let mut framed = Vec::with_capacity(bytes.len() + 4);
        framed.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        framed.extend_from_slice(&bytes);
        Ok(Self {
            bytes: framed,
            offset: 0,
        })
    }
}
enum Incoming {
    FxAux(serde_json::Value),
    Measurement(crate::measurement_wire::Request),
    Meter(crate::meter_wire::Request),
    Spectrum(crate::spectrum::Request),
    Audio(Request),
    Structural(crate::structural_control::Request),
    Brain(crate::brain_control::Request),
    Modules(crate::module_wire::ModuleRequest),
    Sends(crate::sends_wire::SendsRequest),
    Processing(crate::processing_wire::ProcessingRequest),
    UnsupportedProcessing(crate::processing_wire::UnsupportedRequest),
}
struct Client {
    id: u64,
    stream: UnixStream,
    buffer: Box<[u8; MAX_FRAME + 4]>,
    used: usize,
    started: Option<u64>,
    replies: VecDeque<Packet>,
    telemetry: Option<Packet>,
    meter_deadline: Option<u64>,
    meter_last_query: Option<u64>,
    last_write: u64,
    snapshot: bool,
    sends_snapshot_ms: Option<u64>,
    processing_snapshot_ms: Option<u64>,
    structural_snapshot_ms: Option<u64>,
    master_eq_snapshot_ms: Option<u64>,
    measurement_snapshot_ms: Option<u64>,
    fx_aux_snapshot_ms: Option<u64>,
    brain_snapshot_ms: Option<u64>,
    writer: Option<String>,
    lease: Option<Counter>,
}
impl Client {
    fn queue(&mut self, reply: &RenderedReply, now: u64) -> Result<()> {
        if self.replies.len() >= MAX_REPLIES {
            return Err("reply capacity".into());
        }
        if self.replies.is_empty() && self.telemetry.is_none() {
            self.last_write = now;
        }
        reply.validate()?;
        self.replies.push_back(Packet::paged(
            serde_json::to_vec(reply).map_err(|e| e.to_string())?,
        )?);
        Ok(())
    }
    fn receive(&mut self, now: u64, processing_version: u32) -> Result<Option<Incoming>> {
        if self
            .started
            .is_some_and(|t| now.saturating_sub(t) >= TIMEOUT_MS)
        {
            return Err("frame timeout".into());
        }
        let count = (self.buffer.len() - self.used).min(IO_BUDGET);
        if count > 0 {
            match self
                .stream
                .read(&mut self.buffer[self.used..self.used + count])
            {
                Ok(0) => return Err("disconnected".into()),
                Ok(n) => {
                    if self.used == 0 {
                        self.started = Some(now);
                    }
                    self.used += n;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => (),
                Err(e) if e.kind() == ErrorKind::Interrupted => (),
                Err(e) => return Err(e.to_string()),
            }
        }
        if self.used < 4 {
            return Ok(None);
        }
        let length = u32::from_be_bytes(self.buffer[..4].try_into().unwrap()) as usize;
        if length == 0 || length > MAX_FRAME {
            return Err("frame length".into());
        }
        if self.used < length + 4 {
            return Ok(None);
        }
        let bytes = &self.buffer[4..length + 4];
        let value: serde_json::Value = crate::show::decode(bytes)?;
        let request = if value.get("contract").and_then(|v| v.as_str())
            == Some(crate::fx_aux_wire::CONTRACT)
        {
            Incoming::FxAux(value)
        } else if value.get("contract").and_then(|v| v.as_str())
            == Some(crate::measurement_wire::CONTRACT)
        {
            Incoming::Measurement(crate::measurement_wire::Request::decode(bytes)?)
        } else if value.get("contract").and_then(|v| v.as_str()) == Some("GP-SPECTRUM") {
            Incoming::Spectrum(serde_json::from_value(value).map_err(|e| e.to_string())?)
        } else if value.get("contract").and_then(|v| v.as_str()) == Some("GP-METER") {
            Incoming::Meter(crate::meter_wire::Request::decode(bytes)?)
        } else if value.get("contract").and_then(|v| v.as_str()) == Some("GP15-brain") {
            Incoming::Brain(crate::brain_control::Request::decode(bytes)?)
        } else if value.get("contract").and_then(|v| v.as_str()) == Some("GP14-structure")
            || value.get("contract").and_then(|v| v.as_str())
                == Some(crate::master_eq_wire::CONTRACT)
        {
            Incoming::Structural(crate::structural_control::Request::decode(bytes)?)
        } else if value.get("contract").and_then(|v| v.as_str()) == Some("GP05-modules") {
            Incoming::Modules(crate::module_wire::ModuleRequest::decode(bytes)?)
        } else if value.get("contract").and_then(|v| v.as_str())
            == Some(crate::sends_wire::CONTRACT)
        {
            Incoming::Sends(crate::sends_wire::SendsRequest::decode(bytes)?)
        } else if value.get("contract").and_then(|v| v.as_str()) == Some("GP07-processing") {
            if let Some(request) =
                crate::processing_wire::UnsupportedRequest::decode_for(bytes, processing_version)?
            {
                Incoming::UnsupportedProcessing(request)
            } else {
                Incoming::Processing(crate::processing_wire::ProcessingRequest::decode(bytes)?)
            }
        } else {
            Incoming::Audio(Request::decode(bytes)?)
        };
        self.buffer.copy_within(length + 4..self.used, 0);
        self.used -= length + 4;
        if self.used == 0 {
            self.started = None;
        }
        Ok(Some(request))
    }
    fn queue_module(&mut self, value: &impl serde::Serialize, now: u64) -> Result<()> {
        if self.replies.len() >= MAX_REPLIES {
            return Err("reply capacity".into());
        }
        if self.replies.is_empty() && self.telemetry.is_none() {
            self.last_write = now;
        }
        self.replies.push_back(Packet::paged(
            serde_json::to_vec(value).map_err(|e| e.to_string())?,
        )?);
        Ok(())
    }
    fn flush(&mut self, now: u64) -> Result<()> {
        if self.meter_deadline.is_some_and(|d| now >= d) {
            return Err("meter reply deadline".into());
        }
        // A partly sent telemetry frame cannot be interleaved with a control frame.
        let partial = self.telemetry.as_ref().is_some_and(|p| p.offset > 0);
        let packet = if partial {
            self.telemetry.as_mut()
        } else {
            self.replies.front_mut().or(self.telemetry.as_mut())
        };
        let Some(packet) = packet else {
            return Ok(());
        };
        if now.saturating_sub(self.last_write) >= TIMEOUT_MS {
            return Err("stalled reader".into());
        }
        let end = (packet.offset + IO_BUDGET).min(packet.bytes.len());
        match self.stream.write(&packet.bytes[packet.offset..end]) {
            Ok(0) => return Err("disconnected".into()),
            Ok(n) => {
                packet.offset += n;
                self.last_write = now;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::Interrupted => (),
            Err(e) => return Err(e.to_string()),
        }
        let done = packet.offset == packet.bytes.len();
        if done {
            if partial || self.replies.is_empty() {
                self.telemetry = None;
            } else {
                self.replies.pop_front();
            }
        }
        if self.replies.is_empty() && self.telemetry.is_none() {
            self.meter_deadline = None;
        }
        Ok(())
    }
}
/// Same effective UID only; no hardware-host feature or device opens.
fn same_uid(stream: &UnixStream) -> Result<bool> {
    let mut credential = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: valid connected socket and correctly sized writable ucred/length.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credential as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if rc != 0 || length as usize != std::mem::size_of::<libc::ucred>() {
        return Err("peer credential".into());
    }
    Ok(credential.uid == unsafe { libc::geteuid() })
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EpochRecord {
    version: u32,
    show: String,
    high_epoch: Counter,
}
struct EpochOwner {
    _file: fs::File,
    directory: PathBuf,
    show: String,
    epoch: Counter,
}
impl EpochOwner {
    fn advance(&mut self, epoch: Counter) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        if epoch.0 <= self.epoch.0 {
            return Err("fresh durable epoch required".into());
        }
        let bytes = serde_json::to_vec(&EpochRecord {
            version: 1,
            show: self.show.clone(),
            high_epoch: epoch,
        })
        .map_err(|e| e.to_string())?;
        let path = self.directory.join("audio.identity");
        let temp = self
            .directory
            .join(format!("audio.identity.recovery-{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temp, &path).map_err(|e| e.to_string())?;
            fs::File::open(&self.directory)
                .and_then(|d| d.sync_all())
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        } else {
            self.epoch = epoch;
        }
        result
    }
    fn reserve(directory: &Path, show: &str, epoch: Counter) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let lockpath = directory.join("audio.owner");
        // A persistent owner without its registry means reservation was interrupted or
        // identity metadata was lost. Never reinterpret it as a new authority.
        let options = || {
            let mut o = fs::OpenOptions::new();
            o.read(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            o
        };
        let (lock, fresh_owner) = match options().create_new(true).open(&lockpath) {
            Ok(file) => (file, true),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                (options().open(&lockpath).map_err(|e| e.to_string())?, false)
            }
            Err(e) => return Err(e.to_string()),
        };
        let metadata = lock.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
            || metadata.len() != 0
        {
            return Err("private epoch owner".into());
        }
        // SAFETY: flock applies only to this owned regular file descriptor.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("authority already owned".into());
        }
        if fresh_owner {
            // Make the fail-closed marker durable before the first identity reservation.
            lock.sync_all().map_err(|e| e.to_string())?;
            fs::File::open(directory)
                .and_then(|d| d.sync_all())
                .map_err(|e| e.to_string())?;
        }
        let path = directory.join("audio.identity");
        match fs::symlink_metadata(&path) {
            Ok(m) => {
                if !m.is_file()
                    || m.file_type().is_symlink()
                    || m.uid() != unsafe { libc::geteuid() }
                    || m.mode() & 0o777 != 0o600
                    || m.nlink() != 1
                    || m.len() > 4096
                {
                    return Err("private epoch record".into());
                }
                let file = fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                let opened = file.metadata().map_err(|e| e.to_string())?;
                if opened.dev() != m.dev() || opened.ino() != m.ino() || opened.len() > 4096 {
                    return Err("changed epoch record".into());
                }
                let mut bytes = Vec::new();
                file.take(4097)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                let record: EpochRecord = crate::show::decode(&bytes)?;
                if record.version != 1
                    || record.high_epoch.0 == 0
                    || record.show != show
                    || epoch.0 <= record.high_epoch.0
                {
                    return Err("reused or regressed authority epoch".into());
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound && fresh_owner => (),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err("lost authority identity".into());
            }
            Err(e) => return Err(e.to_string()),
        }
        let bytes = serde_json::to_vec(&EpochRecord {
            version: 1,
            show: show.into(),
            high_epoch: epoch,
        })
        .map_err(|e| e.to_string())?;
        let temp = directory.join(format!("audio.identity.pending-{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let saved = (|| {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temp, &path).map_err(|e| e.to_string())?;
            fs::File::open(directory)
                .and_then(|d| d.sync_all())
                .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })();
        if saved.is_err() {
            let _ = fs::remove_file(&temp);
        }
        saved?;
        Ok(Self {
            _file: lock,
            directory: directory.into(),
            show: show.into(),
            epoch,
        })
    }
}
// One bounded pending slot retains prepared state through the render boundary.
// Keep it inline so commit does not introduce an additional heap owner to retire.
#[allow(clippy::large_enum_variant)]
enum StructuralAction {
    #[cfg(feature = "hardware-host")]
    Pa(crate::module_graph::GraphPaPrepared),
    #[cfg(feature = "hardware-host")]
    MasterEq(crate::module_graph::GraphEqPrepared),
    Patch(crate::mixer::PreparedOutputPatch),
    Mute,
    Rearm,
    Failed(String),
}
pub struct LocalAudio {
    source: source::RoutingSlot,
    render_graph: Option<Box<source::RenderGraph>>,
    source_executor: Option<crate::source_executor::SourceExecutor<Box<source::RenderGraph>>>,
    source_quantum: Option<Box<source::Quantum>>,
    persistent_source: bool,
    #[cfg(feature = "hardware-host")]
    clocked_source: bool,
    #[cfg(feature = "hardware-host")]
    clocked_fence: Option<crate::remote::PolicyFence>,
    #[cfg(feature = "hardware-host")]
    clocked_preparation_started: Option<(u64, std::time::Instant)>,
    #[cfg(all(test, feature = "hardware-host"))]
    pub(crate) last_brain_source_receipt: Option<crate::mixer_control::source::ExternalReceipt>,
    routing_observed: source::RoutingObservation,
    source_closure: std::sync::Arc<std::sync::atomic::AtomicBool>,
    spectrum: crate::spectrum::Spectrum,
    metering: Option<crate::metering::Latest>,
    brain: crate::brain_control::State,
    #[cfg(feature = "hardware-host")]
    brain_hold_fence: Option<crate::remote::PolicyFence>,
    #[cfg(feature = "hardware-host")]
    brain_foh_fence: Option<crate::remote::PolicyFence>,
    brain_pending: Option<(
        crate::brain_control::Request,
        crate::brain_control::State,
        Option<String>,
        Option<u64>,
    )>,
    brain_cache: VecDeque<(crate::brain_control::Request, crate::brain_control::Reply)>,
    brain_path_readiness: (bool, bool),
    brain_frame_times: [Option<(u64, u64)>; 64],
    listener: UnixListener,
    _epoch_owner: EpochOwner,
    _endpoint: Endpoint,
    engine: OfflineEngine,
    source_completion: crate::mixer_control::source::SourceCompletion,
    clients: Vec<Client>,
    next_client: u64,
    ticks: u64,
    last_now: u64,
    pending_owner: Option<(u64, u64)>,
    sends_owner: Option<(u64, u64)>,
    processing_owner: Option<(u64, u64)>,
    writer_connections: BTreeMap<String, u64>,
    show: String,
    epoch: Counter,
    synthetic_fouraux: bool,
    remote_completions: VecDeque<serde_json::Value>,
    #[cfg(feature = "hardware-host")]
    fx_aux: Option<fx_aux::Controller>,
    #[cfg(feature = "hardware-host")]
    remote_fx_aux: bool,
    #[cfg(feature = "hardware-host")]
    fx_aux_pending: Option<fx_aux::Pending>,
    measurement: Option<crate::measurement_capture::State>,
    measurement_pending: Option<measurement::Pending>,
    measurement_cache: VecDeque<(
        crate::measurement_wire::Request,
        crate::measurement_wire::Reply,
    )>,
    structural_pending: Option<(
        crate::structural_control::Request,
        StructuralAction,
        Option<u64>,
    )>,
    structural_cache: VecDeque<(
        crate::structural_control::Request,
        crate::structural_control::Reply,
    )>,
    #[cfg(feature = "hardware-host")]
    private_directory: PathBuf,
    module_cache: VecDeque<(
        crate::module_wire::ModuleRequest,
        crate::module_wire::ModuleReply,
    )>,
    #[cfg(feature = "hardware-host")]
    modules: Option<crate::module_graph::ModuleGraph>,
    #[cfg(feature = "hardware-host")]
    module_pending: Vec<(crate::module_wire::ModuleRequest, u64)>,
    analysis: Option<crate::analysis_stream::local::Worker>,
}
impl Drop for LocalAudio {
    fn drop(&mut self) {
        // A detached owner must observe teardown even if every ordinary command
        // or completion slot is occupied. Never wait for it from this signal.
        self.source_closure
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(all(test, feature = "hardware-host"))]
mod executor_tests {
    use super::*;
    #[test]
    fn recovery_retains_shared_worker_and_refuses_unaligned_epoch_before_mutation() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("gp-source-recovery-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _directory = Directory(path.clone());
        let mut provider = LocalAudio::bind(
            &path,
            "audio",
            "11111111-1111-4111-8111-111111111111",
            Counter(9),
        )
        .unwrap();
        provider.enable_persistent_source();
        let media = provider.source_executor_for(0u8).unwrap();
        let before = provider
            .graph_command(None, |_, p| *p = Some(std::thread::current().id()))
            .unwrap();
        assert_eq!(
            before,
            media
                .transact(None, |_, p| *p = Some(std::thread::current().id()))
                .unwrap()
        );
        provider.rearm().unwrap();
        provider.tick(0).unwrap();
        assert!(provider.recover_source(Counter(10), 1).is_err());
        assert_eq!(provider.source_epoch(), 9);
        assert_eq!(provider.frame(), 48);
        provider.recover_source(Counter(10), 0).unwrap();
        assert_eq!(
            before,
            provider
                .graph_command(None, |_, p| *p = Some(std::thread::current().id()))
                .unwrap()
        );
        provider.rearm().unwrap();
        provider.tick(1).unwrap();
        assert_eq!(provider.source_epoch(), 10);
        assert_eq!(provider.frame(), 48);
        drop(provider);
        drop(media);
        std::fs::remove_dir_all(path).unwrap();
    }
}
impl LocalAudio {
    pub fn bind(directory: &Path, name: &str, show: &str, epoch: Counter) -> Result<Self> {
        Self::bind_configured(
            directory,
            name,
            show,
            epoch,
            crate::topology::EngineTopology::legacy(),
        )
    }
    pub fn bind_configured(
        directory: &Path,
        name: &str,
        show: &str,
        epoch: Counter,
        topology: crate::topology::EngineTopology,
    ) -> Result<Self> {
        if name.is_empty()
            || name.len() > 40
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err("endpoint name".into());
        }
        let metadata = fs::symlink_metadata(directory).map_err(|e| e.to_string())?;
        if !directory.is_absolute()
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
            || fs::canonicalize(directory).map_err(|e| e.to_string())? != directory
        {
            return Err("private owned directory required".into());
        }
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => (),
            _ => return Err("preexisting endpoint".into()),
        }
        let ni = topology.inputs.len();
        let mut spectrum = crate::spectrum::Spectrum::new(topology.sample_rate);
        let metering = topology
            .meter_admission(crate::topology::ResourceBudget::default())
            .ok()
            .and_then(|_| crate::metering::prepare(ni, topology.monitors, topology.sample_rate));
        let (meter_tap, metering) = match metering {
            Some((tap, latest)) => (Some(tap), Some(latest)),
            None => (None, None),
        };
        let source = source::SourceRouting::with_spectrum(
            &topology,
            meter_tap,
            spectrum.take_acquisition().unwrap(),
        );
        let engine = OfflineEngine::with_topology(show, epoch, Counter(0), 0, topology)?;
        let source_completion = engine.source_completion();
        let routing_observed = source::RoutingObservation::new(engine.topology());
        let epoch_owner = EpochOwner::reserve(directory, show, epoch)?;
        let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
        let m = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        let endpoint = Endpoint {
            path,
            dev: m.dev(),
            ino: m.ino(),
        };
        fs::set_permissions(&endpoint.path, fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self {
            source: source::RoutingSlot::new(source),
            render_graph: None,
            source_executor: None,
            source_quantum: None,
            persistent_source: false,
            #[cfg(feature = "hardware-host")]
            clocked_source: false,
            #[cfg(feature = "hardware-host")]
            clocked_fence: None,
            #[cfg(feature = "hardware-host")]
            clocked_preparation_started: None,
            #[cfg(all(test, feature = "hardware-host"))]
            last_brain_source_receipt: None,
            routing_observed,
            source_closure: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            spectrum,
            metering,
            brain: crate::brain_control::State::default(),
            #[cfg(feature = "hardware-host")]
            brain_hold_fence: None,
            #[cfg(feature = "hardware-host")]
            brain_foh_fence: None,
            brain_pending: None,
            brain_cache: VecDeque::with_capacity(256),
            brain_path_readiness: (false, false),
            brain_frame_times: {
                let mut times = [None; 64];
                times[0] = Some((0, 0));
                times
            },
            listener,
            _epoch_owner: epoch_owner,
            _endpoint: endpoint,
            engine,
            source_completion,
            clients: Vec::with_capacity(MAX_CLIENTS),
            next_client: 1,
            ticks: 0,
            last_now: 0,
            pending_owner: None,
            sends_owner: None,
            processing_owner: None,
            writer_connections: BTreeMap::new(),
            show: show.into(),
            epoch,
            analysis: None,
            synthetic_fouraux: false,
            remote_completions: VecDeque::with_capacity(64),
            #[cfg(feature = "hardware-host")]
            fx_aux: None,
            #[cfg(feature = "hardware-host")]
            remote_fx_aux: false,
            #[cfg(feature = "hardware-host")]
            fx_aux_pending: None,
            measurement: None,
            measurement_pending: None,
            measurement_cache: VecDeque::with_capacity(256),
            structural_pending: None,
            structural_cache: VecDeque::with_capacity(256),
            #[cfg(feature = "hardware-host")]
            private_directory: directory.into(),
            module_cache: VecDeque::with_capacity(256),
            #[cfg(feature = "hardware-host")]
            modules: None,
            #[cfg(feature = "hardware-host")]
            module_pending: Vec::with_capacity(2),
        })
    }
    /// Read-only versioned analysis. `input` is the legacy wire name for source ID.
    pub fn spectrum_snapshot(
        &self,
        request: &crate::spectrum::Request,
    ) -> Result<crate::spectrum::Snapshot> {
        use crate::spectrum::{MAIN, OUTPUT, Source};
        let t = self.topology();
        if request.contract != "GP-SPECTRUM"
            || !matches!(request.version, 1 | 2)
            || request.query == 0
            || request.show_id != self.show
            || request.epoch != self.epoch.0
            || request.map != t.map_revision
        {
            return Err("spectrum identity/version".into());
        }
        let mut sources: Vec<_> = t
            .inputs
            .iter()
            .map(|p| Source {
                id: p.id.clone(),
                label: format!("{} / {}", p.id, p.physical_port)
                    .escape_debug()
                    .to_string(),
                tap: "raw-pre-processing",
            })
            .collect();
        let mut index = t.inputs.iter().position(|p| p.id == request.input);
        if request.version == 2 {
            for (i, side) in ["left", "right"].iter().enumerate() {
                let id = format!("main:{side}");
                if request.input == id {
                    index = Some(MAIN + i);
                }
                sources.push(Source {
                    id,
                    label: format!("Main {}", side),
                    tap: "main-pre-pa",
                });
            }
            for p in &t.outputs {
                let id = format!("output:{}", p.id);
                if request.input == id {
                    index = Some(OUTPUT + p.playback_slot);
                }
                let route = match &p.source {
                    Some(crate::topology::OutputSource::Main { channel: 0 }) => "Main L".into(),
                    Some(crate::topology::OutputSource::Main { .. }) => "Main R".into(),
                    Some(crate::topology::OutputSource::Monitor { index }) => {
                        format!("Monitor {}", index + 1)
                    }
                    Some(crate::topology::OutputSource::Pa { index }) => {
                        format!("PA {}", index + 1)
                    }
                    None => "Unassigned (digital zero)".into(),
                };
                sources.push(Source {
                    id,
                    label: format!("{} / {} / {}", p.id, p.physical_port, route)
                        .escape_debug()
                        .to_string(),
                    tap: "physical-output-post-patch",
                });
            }
        }
        let mut reply = self.spectrum.snapshot(
            request,
            index.ok_or("spectrum source")?,
            crate::analysis_stream::local::monotonic_ms()?,
        );
        if request.version == 2 {
            reply.sources = Some(sources);
        }
        Ok(reply)
    }
    pub fn meter_snapshot(
        &mut self,
        request: &crate::meter_wire::Request,
        generation: u64,
    ) -> Result<crate::meter_wire::Snapshot> {
        request.validate()?;
        if request.show_id != self.show {
            return Err("meter show identity".into());
        }
        let now = crate::analysis_stream::local::monotonic_ms()?;
        let topology = self.topology().clone();
        let quiesced = false; // Raw observations remain available through downstream safety zeros.
        let s = if let Some(latest) = &mut self.metering {
            latest.snapshot(
                request,
                &topology,
                self.epoch.0,
                generation,
                now,
                self.routing_observed.meter_losses,
                quiesced,
            )
        } else {
            crate::meter_wire::Snapshot {
                contract: "GP-METER".into(),
                version: 1,
                kind: "meter_snapshot".into(),
                show_id: self.show.clone(),
                module: "audio".into(),
                source_epoch: self.epoch,
                query_id: request.query_id,
                capability_generation: Counter(generation),
                map_generation: Counter(topology.map_revision),
                topology: topology.identity,
                sample_rate: topology.sample_rate,
                inputs: topology.inputs.len(),
                monitors: topology.monitors,
                sequence: Counter(0),
                first_frame: None,
                end_frame: None,
                acquisition_age_ms: None,
                publication_loss: Counter(0),
                valid: false,
                reason: Some("capacity".into()),
                taps: Vec::new(),
            }
        };
        s.validate()?;
        Ok(s)
    }
    pub fn brain_monitor_output(&self) -> &[f64] {
        &self.routing_observed.monitor
    }
    pub fn brain_fx_send(&self) -> &[f64] {
        &self.routing_observed.send
    }
    pub fn close_brain_audio(&mut self) {
        self.brain.fault();
        self.brain_path_readiness = (false, false);
        self.routing_observed.monitor.fill(0.);
        #[cfg(feature = "hardware-host")]
        if self.clocked_source {
            let generation = self.brain.selection_generation;
            let _ = self.graph_command(generation, |graph, generation| {
                graph.routing.close_brain_audio();
                if let Some(routing) = &mut graph.clock_routing {
                    routing.monitor = crate::brain_control::MonitorSource::None;
                    routing.monitor_silent = true;
                    routing.selection_generation = *generation;
                    routing.deadline_frame = None;
                    routing.deadline_ms = None;
                    routing.foh = false;
                }
            });
            return;
        }
        let _ = self.routing_command((), |routing, _| routing.close_brain_audio());
    }
    pub fn brain_media_authorized(&self, now: u64) -> bool {
        self.brain_media_authorized_at(now, self.frame())
    }
    fn brain_media_authorized_at(&self, now: u64, source_frame: u64) -> bool {
        self.brain
            .hold
            .as_ref()
            .is_some_and(|(r, _, deadline, frame)| {
                now < *deadline
                    && source_frame < *frame
                    && self.engine.writer_scope(r, now)
                        == Some(crate::control_model::Scope::TalkbackDestinations)
            })
            && !self.brain.mute
            && (!self.brain.foh
                || self.brain.foh_authority.as_ref().is_some_and(|r| {
                    self.engine.writer_scope(r, now)
                        == Some(crate::control_model::Scope::TalkbackFoh)
                }))
            && self.engine.output_safety_gain(source_frame) > 0.
    }
    /// Authenticated media owner supplies observations after checking current
    /// descriptors, device status and bridge acquisition. Configuration alone is
    /// never readiness. Device/source/session closure clears both observations.
    pub fn set_brain_path_readiness(&mut self, talkback: bool, monitor: bool) {
        self.brain_path_readiness = (talkback, monitor);
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn show_id(&self) -> &str {
        &self.show
    }
    /// Authenticated owner operation; does not change source or hold state.
    pub fn maintain_lease(
        &mut self,
        r: &crate::lease_maintenance::Request,
        now: u64,
        capability: u64,
    ) -> std::result::Result<crate::lease_maintenance::Maintained, crate::lease_maintenance::Reason>
    {
        use crate::lease_maintenance::Reason;
        if r.validate().is_err()
            || r.show_id != self.show
            || r.epoch != self.epoch
            || r.capability_generation.0 != capability
            || r.map_generation.0 != self.topology().map_revision
        {
            return Err(Reason::Identity);
        }
        self.engine.maintain_lease(r, now)
    }
    /// No await, tick, pending-state inspection, or second raw snapshot between halves.
    pub fn paired_readback(
        &mut self,
        r: &crate::paired_readback::Request,
        now: u64,
        capability: u64,
    ) -> std::result::Result<
        (
            crate::mixer_control::RenderedSnapshot,
            crate::brain_control::Snapshot,
        ),
        crate::paired_readback::Reason,
    > {
        use crate::paired_readback::Reason;
        if r.validate().is_err()
            || r.show_id != self.show
            || r.epoch != self.epoch
            || r.capability_generation.0 != capability
            || r.map_generation.0 != self.topology().map_revision
        {
            return Err(Reason::Identity);
        }
        self.engine
            .observe_atomic_read_time(now)
            .map_err(|_| Reason::Clock)?;
        let raw = self.snapshot().map_err(|_| Reason::Capacity)?;
        let brain = self.brain_snapshot();
        if raw.authority.revision != brain.revision || raw.frame != brain.frame {
            return Err(Reason::Clock);
        }
        Ok((raw, brain))
    }
    /// Fixed-size controller witness: no full mixer snapshot or authority clone.
    pub fn held_proof(
        &self,
        r: &crate::held_proof::Request,
        now: u64,
        capability: u64,
    ) -> std::result::Result<crate::held_proof::Witness, crate::held_proof::Reason> {
        use crate::held_proof::{Brain, Configuration, Reason, Witness};
        if r.show_id != self.show
            || r.epoch != self.epoch
            || r.capability_generation.0 != capability
            || r.map_generation.0 != self.topology().map_revision
        {
            return Err(Reason::Identity);
        }
        if r.scope != crate::control_model::Scope::TalkbackDestinations {
            return Err(Reason::Scope);
        }
        let (scope, remaining) = self
            .engine
            .live_lease_witness(&r.authority_request(), now)?;
        if scope != r.scope {
            return Err(Reason::Scope);
        }
        let topology = self.topology();
        let dims = [
            topology.inputs.len(),
            topology.monitors,
            topology.pa_outputs,
            topology.capture_channels,
            topology.playback_channels,
            topology.sample_rate as usize,
        ];
        let mut dimensions = [0u32; 6];
        for (dst, value) in dimensions.iter_mut().zip(dims) {
            *dst = u32::try_from(value).map_err(|_| Reason::Capacity)?;
        }
        let digest = crate::held_proof::config_digest(Configuration {
            show_id: &self.show,
            epoch: self.epoch.0,
            capability,
            map: topology.map_revision,
            dimensions,
            destinations: self.brain.destination_hash,
            gain_cdb: self.brain.gain_cdb,
            mute: self.brain.mute,
            foh: self.brain.foh,
        });
        if digest != r.expected_config_digest {
            return Err(Reason::ConfigChanged);
        }
        let held = self.brain.hold.as_ref();
        let foh_authorized = self.brain.foh_authority.as_ref().is_some_and(|request| {
            self.engine.writer_scope(request, now) == Some(crate::control_model::Scope::TalkbackFoh)
        });
        let media_authorized = self.brain_media_authorized(now);
        Ok(Witness {
            revision: self.engine.revision(),
            source_frame: crate::show::Counter(self.frame()),
            config_digest: digest,
            lease_remaining_ms: remaining,
            brain: Brain {
                selection_generation: crate::show::Counter(self.brain.selection_generation),
                hold_generation_counter: crate::show::Counter(self.brain.high_generation),
                held_generation: held.map(|h| crate::show::Counter(h.1)),
                hold_deadline_ms: held.map(|h| crate::show::Counter(h.2)),
                source: self.brain.source,
                monitor_armed: self.brain.monitor_armed,
                monitor_mute: self.brain.monitor_mute,
                monitor_dim: self.brain.monitor_dim,
                talkback_mute: self.brain.mute,
                talkback_foh: self.brain.foh,
                monitor_path_ready: self.brain_path_readiness.1
                    && self.brain.monitor_armed
                    && !self.brain.monitor_mute
                    && self.brain.source != crate::brain_control::MonitorSource::None,
                talkback_path_ready: self.brain_path_readiness.0 && media_authorized,
            },
            foh_authorized,
            media_authorized,
            heartbeat_ms: crate::brain_control::HEARTBEAT_MS,
            deadman_ms: crate::brain_control::DEADMAN_MS,
            fade_frames: crate::brain_control::FADE_FRAMES,
        })
    }
    pub fn brain_snapshot(&self) -> crate::brain_control::Snapshot {
        let mut snapshot = self
            .brain
            .snapshot(self.frame(), self.engine.revision(), false);
        snapshot.talkback_path_ready =
            self.brain_path_readiness.0 && self.brain_media_authorized(self.last_now);
        snapshot.monitor_path_ready = self.brain_path_readiness.1
            && self.brain.monitor_armed
            && !self.brain.monitor_mute
            && self.brain.source != crate::brain_control::MonitorSource::None;
        snapshot.audible_path_ready = snapshot.talkback_path_ready || snapshot.monitor_path_ready;
        snapshot
    }
    pub fn brain_request(
        &mut self,
        r: crate::brain_control::Request,
        now: u64,
        fresh: bool,
        owner: Option<u64>,
    ) -> Result<crate::brain_control::Reply> {
        use crate::brain_control::{Command as B, Reply};
        r.validate()?;
        if r.show_id != self.show || r.epoch != self.epoch {
            return Err("brain attachment identity".into());
        }
        if matches!(r.command, B::BrainSnapshot {}) {
            return Ok(Reply::new(
                &r,
                "snapshot",
                None,
                self.engine.revision(),
                None,
                Some(self.brain_snapshot()),
            ));
        }
        if self.engine.external_boundary().is_none() {
            self.brain_pending = None;
        }
        let (frame, cached) = match self.engine.begin_external(
            &r.authority_request(),
            &r.fingerprint()?,
            r.scope(),
            now,
        ) {
            Ok(v) => v,
            Err(reason) => {
                return Ok(Reply::new(
                    &r,
                    "final",
                    Some(reason),
                    self.engine.revision(),
                    None,
                    None,
                ));
            }
        };
        if let Some(cached) = cached {
            return Ok(self
                .brain_cache
                .iter()
                .find(|(old, _)| old == &r)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| {
                    Reply::new(
                        &r,
                        "final",
                        cached
                            .body
                            .reason
                            .or_else(|| Some("expired_outcome".into())),
                        cached.body.revision,
                        cached.body.effective_frame.map(|f| f.0),
                        None,
                    )
                }));
        }
        if self.brain_pending.is_none() {
            let mut prepared = self.brain.clone();
            let failure = if !fresh {
                Some("stale_snapshot".into())
            } else {
                prepared
                    .apply(
                        &r,
                        now,
                        frame,
                        self.topology().inputs.len(),
                        self.topology().monitors,
                    )
                    .err()
            };
            if failure.is_none()
                && matches!(r.command, crate::brain_control::Command::TalkbackSet { .. })
            {
                prepared.destination_hash =
                    crate::held_proof::destination_hash(&prepared.monitors)?;
            }
            self.brain_pending = Some((r.clone(), prepared, failure, owner));
        }
        Ok(Reply::new(
            &r,
            "pending",
            None,
            self.engine.revision(),
            Some(frame),
            None,
        ))
    }
    fn heartbeat_observation_time(
        &self,
        request: &crate::brain_control::Request,
        now: u64,
    ) -> Result<Option<u64>> {
        if let crate::brain_control::Command::Heartbeat { observed_frame, .. } = request.command {
            let Some((frame, observed_ms)) =
                self.brain_frame_times[(observed_frame.0 / 48 % 64) as usize]
            else {
                return Err("heartbeat frame observation missing".into());
            };
            if frame != observed_frame.0
                || now < observed_ms
                || now - observed_ms > crate::brain_control::HEARTBEAT_MS
            {
                return Err("heartbeat frame observation stale".into());
            }
            Ok(Some(observed_ms))
        } else {
            Ok(None)
        }
    }
    fn commit_brain(&mut self, now: u64) -> Result<()> {
        if self.engine.external_boundary()
            != Some(
                self.engine
                    .admission_frame()
                    .unwrap_or_else(|| self.frame()),
            )
            || self.brain_pending.is_none()
        {
            return Ok(());
        }
        let (r, mut prepared, failure, owner) = self.brain_pending.take().unwrap();
        if !self
            .engine
            .external_matches(&r.authority_request(), &r.fingerprint()?)
        {
            self.close_brain_audio();
            return Err("brain prepared identity".into());
        }
        let frame = self
            .engine
            .admission_frame()
            .unwrap_or_else(|| self.frame());
        // Re-evaluate held deadlines at the actual consumer boundary, never extend
        // a delayed heartbeat using preparation-time permission.
        let mut failure = failure;
        if failure.is_none() {
            let destination_hash = prepared.destination_hash;
            prepared = self.brain.clone();
            if matches!(r.command, crate::brain_control::Command::TalkbackSet { .. }) {
                prepared.destination_hash = destination_hash;
            }
            failure = prepared
                .apply(
                    &r,
                    now,
                    frame,
                    self.topology().inputs.len(),
                    self.topology().monitors,
                )
                .err();
        }
        if failure.is_none() {
            match self.heartbeat_observation_time(&r, now) {
                Ok(Some(observed_ms)) => {
                    if let Some((_, _, deadline, _)) = &mut prepared.hold {
                        *deadline = (*deadline)
                            .min(observed_ms.saturating_add(crate::brain_control::DEADMAN_MS));
                    }
                }
                Ok(None) => (),
                Err(reason) => failure = Some(reason),
            }
        }
        #[cfg(feature = "hardware-host")]
        let result = if self.clocked_source {
            // Preparing routing may allocate, but publication and replacement are
            // one retained source packet. The old routing returns for destruction.
            std::mem::swap(&mut self.brain, &mut prepared);
            let mut routing = self.clock_routing(now);
            if matches!(
                r.command,
                crate::brain_control::Command::Hold { .. }
                    | crate::brain_control::Command::Heartbeat { .. }
            ) {
                routing.hold_fence = self.clocked_fence.clone();
            }
            if matches!(
                r.command,
                crate::brain_control::Command::TalkbackFoh { enabled: true }
            ) {
                routing.foh_fence = self.clocked_fence.clone();
            }
            std::mem::swap(&mut self.brain, &mut prepared);
            let permit = self.prepare_source_external(now)?;
            let (_, _, _, receipt) = self.graph_boundary_command(
                frame,
                (permit, Some(routing), failure.is_some(), None),
                |graph, packet| {
                    let mixer = &mut graph.mixer;
                    let routing = &mut graph.clock_routing;
                    packet.3 = Some(mixer.commit_external(&packet.0, |mixer| {
                        if packet.2 {
                            return -10;
                        }
                        mixer
                            .set_operator_tap(packet.1.as_ref().expect("prepared routing").monitor);
                        std::mem::swap(routing, &mut packet.1);
                        0
                    }));
                },
            )?;
            let receipt = receipt.ok_or("Brain source receipt")?;
            #[cfg(test)]
            {
                self.last_brain_source_receipt = Some(receipt);
            }
            self.engine.finish_external(receipt)?
        } else {
            self.engine
                .commit_external(now, || failure.clone().map_or(Ok(()), Err))?
        };
        #[cfg(not(feature = "hardware-host"))]
        let result = self
            .engine
            .commit_external(now, || failure.clone().map_or(Ok(()), Err))?;
        let applied = result.kind == "applied";
        if applied {
            if self.brain.selection_generation != prepared.selection_generation
                || self.brain.hold.as_ref().map(|h| h.1) != prepared.hold.as_ref().map(|h| h.1)
            {
                self.brain_path_readiness = (false, false);
            }
            self.brain = prepared;
            #[cfg(feature = "hardware-host")]
            {
                if self.brain.hold.is_none() {
                    self.brain_hold_fence = None;
                } else if matches!(
                    r.command,
                    crate::brain_control::Command::Hold { .. }
                        | crate::brain_control::Command::Heartbeat { .. }
                ) {
                    self.brain_hold_fence = self.clocked_fence.clone();
                }
                if !self.brain.foh {
                    self.brain_foh_fence = None;
                } else if matches!(
                    r.command,
                    crate::brain_control::Command::TalkbackFoh { enabled: true }
                ) {
                    self.brain_foh_fence = self.clocked_fence.clone();
                }
            }
            #[cfg(feature = "hardware-host")]
            if !self.clocked_source {
                self.engine.set_operator_tap(self.brain.source);
            }
            #[cfg(not(feature = "hardware-host"))]
            self.engine.set_operator_tap(self.brain.source);
        }
        let reply = crate::brain_control::Reply::new(
            &r,
            "final",
            failure.or(result.body.reason),
            result.body.revision,
            applied.then_some(frame),
            Some(self.brain_snapshot()),
        );
        if self.brain_cache.len() == 256 {
            self.brain_cache.pop_front();
        }
        self.brain_cache.push_back((r, reply.clone()));
        if let Some(id) = owner {
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
                let _ = c.queue_module(&reply, now);
            }
        } else if self.remote_completions.len() < 64 {
            self.remote_completions
                .push_back(serde_json::to_value(reply).map_err(|e| e.to_string())?);
        }
        Ok(())
    }
    #[cfg(feature = "hardware-host")]
    fn enforce_clock_brain_authority(&mut self, now: u64) -> Result<()> {
        if !self.clocked_source {
            return Ok(());
        }
        if self.brain_hold_fence.as_ref().is_some_and(|f| !f.valid())
            || (self.brain.foh && self.brain_foh_fence.as_ref().is_some_and(|f| !f.valid()))
        {
            self.brain.close();
        }
        self.prepare_brain_authority(now, self.frame());
        let held = self.brain_media_authorized(now);
        let foh = self.brain.foh;
        self.graph_command((held, foh), |graph, p| {
            if let Some(routing) = &mut graph.clock_routing {
                // Closure only: a service tick cannot renew a copied hold.
                if !p.0 {
                    routing.deadline_ms = None;
                    routing.deadline_frame = None;
                }
                if !p.1 {
                    routing.foh = false;
                }
            }
        })?;
        Ok(())
    }
    fn prepare_brain_authority(&mut self, now: u64, end_frame: u64) {
        if !self.brain_media_authorized_at(now, end_frame) {
            self.brain.close();
        }
        if self.brain.foh_authority.as_ref().is_some_and(|r| {
            self.engine.writer_scope(r, now) != Some(crate::control_model::Scope::TalkbackFoh)
        }) {
            self.brain.foh = false;
            self.brain.foh_authority = None;
            self.brain.close();
        }
    }
    pub fn persisted_intent(&mut self) -> Result<crate::structural_control::Intent> {
        let snapshot = self.structural_snapshot()?;
        Ok(crate::structural_control::Intent {
            version: 1,
            engine: self.engine.persisted_intent()?,
            brain: Some(self.brain.intent()),
            pa_configuration_json: snapshot.pa_configuration_json,
            pa_program_buses: snapshot.pa_program_buses,
        })
    }
    /// Restore only on a newly bound, unprocessed authority. Libraries have been
    /// explicitly loaded by the caller; all owner validation precedes replacement.
    pub fn restore_composed_intent(
        &mut self,
        intent: &crate::structural_control::Intent,
    ) -> Result<()> {
        intent.validate()?;
        if self.frame() != 0
            || self.render_graph.is_some()
            || self.source_executor.is_some()
            || !self.clients.is_empty()
            || self.engine.revision() != Counter(0)
            || self.show != intent.engine.show_id
        {
            return Err("fresh unattached provider required".into());
        }
        let restored =
            OfflineEngine::restore_intent_for(&intent.engine, self.topology(), self.epoch, 0)?;
        #[cfg(feature = "hardware-host")]
        if let Some(json) = &intent.pa_configuration_json {
            let capacity = self.topology().pa_outputs;
            let total = self.topology().monitors + 2;
            let g = self
                .modules
                .as_mut()
                .ok_or("persisted PA requires explicit owner library")?;
            let mut prepared =
                g.prepare_pa_change(json.as_bytes(), intent.pa_program_buses.clone(), total)?;
            if prepared.output_channels() != capacity {
                return Err("persisted PA capability mismatch".into());
            }
            let result = g.commit_pa_change(&mut prepared, self.epoch.0, 0);
            if result != 0 {
                return Err(format!("persisted PA commit {result}"));
            }
            g.retire_pa_changes();
        }
        #[cfg(not(feature = "hardware-host"))]
        if intent.pa_configuration_json.is_some() {
            return Err("persisted PA unavailable".into());
        }
        self.engine = restored;
        self.source_completion = self.engine.source_completion();
        self.routing_command((), |routing, _| routing.close_brain_audio())?;
        self.brain = intent
            .brain
            .as_ref()
            .map(crate::brain_control::State::from_intent)
            .unwrap_or_default();
        self.engine.set_operator_tap(self.brain.source);
        Ok(())
    }
    pub fn structural_snapshot(&self) -> Result<crate::structural_control::Snapshot> {
        #[cfg(feature = "hardware-host")]
        let (configuration, buses, status, capabilities) = if let Some(g) = &self.modules {
            (
                g.pa_configuration_json().map(str::to_owned),
                g.pa_program_buses().to_vec(),
                g.pa_v2_status()
                    .transpose()
                    .map_err(|e| format!("PA status {e}"))?
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|e| e.to_string())?,
                g.pa_v2_capabilities()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|e| e.to_string())?,
            )
        } else {
            (None, Vec::new(), None, None)
        };
        #[cfg(not(feature = "hardware-host"))]
        let (configuration, buses, status, capabilities) = (None, Vec::new(), None, None);
        Ok(crate::structural_control::Snapshot {
            show_id: self.show.clone(),
            epoch: self.epoch,
            revision: self.engine.revision(),
            frame: Counter(self.frame()),
            topology: self.topology().clone(),
            clock: self.engine.clock_status().clone(),
            outputs_quiesced: self.engine.outputs_quiesced(),
            pa_configuration_json: configuration,
            pa_program_buses: buses,
            pa_status: status,
            pa_capabilities: capabilities,
        })
    }
    /// Control-side preparation; exact identity, scopes, retry history and final
    /// revision remain owned by the same engine authority as channel controls.
    pub fn master_eq_snapshot(&self) -> Result<crate::master_eq_wire::Snapshot> {
        let mut snapshot = crate::master_eq_wire::Snapshot {
            show_id: self.show.clone(),
            epoch: self.epoch,
            revision: self.engine.revision(),
            frame: Counter(self.frame()),
            map_revision: Counter(self.topology().map_revision),
            live_supported: false,
            live_available: false,
            fault_latched: false,
            source_recovery_required: false,
            settled: false,
            unavailable_reason: Some("live EQ unavailable".into()),
            owner_instance: Counter(0),
            graph_generation: Counter(0),
            eq_generation: Counter(0),
            program_buses: Vec::new(),
            master_input_indices: None,
            transition_remaining_frames: Counter(0),
            retirement_occupied: false,
            owner_json: None,
        };
        #[cfg(feature = "hardware-host")]
        if let Some(g) = &self.modules {
            snapshot.program_buses = g.pa_program_buses().to_vec();
            if let Some(Ok(base)) = g.pa_v2_status() {
                snapshot.graph_generation = Counter(base.generation);
            }
            if let Ok(status) = g.master_eq_status() {
                snapshot.live_supported = true;
                snapshot.source_recovery_required = g.master_eq_source_recovery_required()
                    || self.engine.clock_status().fault.is_some()
                    || self.engine.clock_status().state
                        == crate::clock_domain::ClockState::Quiesced;
                let base_status = g.pa_v2_status().and_then(|s| s.ok());
                let base_valid = base_status.is_some_and(|s| {
                    s.version == 2
                        && s.size == 80
                        && s.committed == 1
                        && s.epoch == status.epoch
                        && s.generation == status.graph_generation
                });
                snapshot.fault_latched = !base_valid
                    || base_status.is_some_and(|s| s.fault_latched != 0)
                    || snapshot.source_recovery_required;
                snapshot.settled = status.remaining == 0 && !snapshot.fault_latched;
                snapshot.owner_instance = Counter(status.instance);
                snapshot.graph_generation = Counter(status.graph_generation);
                snapshot.eq_generation = Counter(status.eq_generation);
                snapshot.transition_remaining_frames = Counter(status.remaining);
                snapshot.retirement_occupied = status.retirement_occupied != 0;
                match g.master_eq_indices() {
                    Ok(indices) => {
                        snapshot.master_input_indices = Some(indices);
                        snapshot.owner_json = Some(g.master_eq_readback()?);
                        if snapshot.source_recovery_required {
                            snapshot.unavailable_reason = Some("source recovery required".into());
                        } else if !base_valid {
                            snapshot.unavailable_reason = Some("PA status unavailable".into());
                        } else if snapshot.fault_latched {
                            snapshot.unavailable_reason = Some("PA fault latched".into());
                        } else if status.eligible != 0 {
                            snapshot.live_available = true;
                            snapshot.unavailable_reason = None;
                        } else {
                            snapshot.unavailable_reason = Some("live EQ resource budget".into());
                        }
                    }
                    Err(e) => snapshot.unavailable_reason = Some(e),
                }
            }
        }
        #[cfg(not(feature = "hardware-host"))]
        let _ = &mut snapshot;
        Ok(snapshot)
    }
    pub fn structural_request(
        &mut self,
        r: crate::structural_control::Request,
        now: u64,
        fresh: bool,
        owner: Option<u64>,
    ) -> Result<crate::structural_control::Reply> {
        use crate::structural_control::{Command as S, Reply};
        r.validate()?;
        if r.show_id != self.show || r.epoch != self.epoch {
            return Err("structural attachment identity".into());
        }
        if matches!(r.command, S::MasterEqSnapshot {}) {
            let mut reply = Reply::new(&r, "snapshot", None, None, self.engine.revision(), None);
            reply.master_eq = Some(self.master_eq_snapshot()?);
            return Ok(reply);
        }
        if matches!(r.command, S::StructuralSnapshot {}) {
            return Ok(Reply::new(
                &r,
                "snapshot",
                None,
                None,
                self.engine.revision(),
                Some(self.structural_snapshot()?),
            ));
        }
        if self.engine.external_boundary().is_none() {
            self.structural_pending = None;
        }
        let auth = r.authority_request();
        let (frame, cached) =
            match self
                .engine
                .begin_external(&auth, &r.fingerprint()?, r.scope().unwrap(), now)
            {
                Ok(v) => v,
                Err(reason) => {
                    return Ok(Reply::new(
                        &r,
                        "final",
                        Some(reason),
                        None,
                        self.engine.revision(),
                        None,
                    ));
                }
            };
        if let Some(cached) = cached {
            return Ok(self
                .structural_cache
                .iter()
                .find(|(old, _)| old == &r)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| {
                    Reply::new(
                        &r,
                        "final",
                        cached
                            .body
                            .reason
                            .clone()
                            .or_else(|| Some("expired_outcome".into())),
                        cached.body.effective_frame.map(|f| f.0),
                        cached.body.revision,
                        None,
                    )
                }));
        }
        if let Some((old, _, _)) = &self.structural_pending {
            if old == &r {
                return Ok(Reply::new(
                    &r,
                    "pending",
                    None,
                    Some(frame),
                    self.engine.revision(),
                    None,
                ));
            }
            return Err("structural pending ownership".into());
        }
        let prepared = (|| -> Result<StructuralAction> {
            if !fresh {
                return Err("fresh_structural_snapshot_required".into());
            }
            match &r.command {
                S::MasterEqSet {
                    patch_json,
                    program_buses,
                    owner_instance,
                    graph_generation,
                    eq_generation,
                    map_revision,
                } => {
                    if map_revision.0 != self.topology().map_revision {
                        return Err("stale output map revision".into());
                    }
                    #[cfg(feature = "hardware-host")]
                    {
                        let g = self.modules.as_mut().ok_or("live EQ unavailable")?;
                        let expected = crate::host::pa_v2::EqStatus {
                            instance: owner_instance.0,
                            graph_generation: graph_generation.0,
                            eq_generation: eq_generation.0,
                            epoch: self.epoch.0,
                            ..Default::default()
                        };
                        Ok(StructuralAction::MasterEq(g.prepare_master_eq(
                            patch_json.as_bytes(),
                            expected,
                            program_buses,
                            frame,
                        )?))
                    }
                    #[cfg(not(feature = "hardware-host"))]
                    {
                        let _ = (
                            patch_json,
                            program_buses,
                            owner_instance,
                            graph_generation,
                            eq_generation,
                        );
                        Err("live EQ unavailable".into())
                    }
                }
                S::OutputMute {} => Ok(StructuralAction::Mute),
                S::OutputRearm {} => Ok(StructuralAction::Rearm),
                S::OutputPatch { outputs } => {
                    if !self.engine.outputs_quiesced() {
                        return Err("outputs_must_be_quiesced".into());
                    }
                    if self.topology().mapping_evidence == "operator-verified"
                        && self.topology().pa_outputs > 0
                        && outputs.iter().any(|p| {
                            matches!(p.source, Some(crate::topology::OutputSource::Main { .. }))
                        })
                    {
                        return Err("physical main route must pass through PA protection".into());
                    }
                    let current = &self.topology().outputs;
                    if outputs.len() != current.len()
                        || outputs.iter().zip(current).any(|(a, b)| {
                            a.id != b.id
                                || a.playback_slot != b.playback_slot
                                || a.physical_port != b.physical_port
                        })
                    {
                        return Err("physical remap requires reviewed topology reopen".into());
                    }
                    Ok(StructuralAction::Patch(
                        self.engine.prepare_output_patch(outputs.clone())?,
                    ))
                }
                S::PaSet {
                    configuration_json,
                    program_buses,
                } => {
                    if !self.engine.outputs_quiesced() {
                        return Err("outputs_must_be_quiesced".into());
                    }
                    #[cfg(feature = "hardware-host")]
                    {
                        let total = self.topology().monitors + 2;
                        let capacity = self.topology().pa_outputs;
                        let g = self.modules.as_mut().ok_or("modules unavailable")?;
                        let prepared = g.prepare_pa_change(
                            configuration_json.as_bytes(),
                            program_buses.clone(),
                            total,
                        )?;
                        if prepared.output_channels() != capacity {
                            return Err("PA outputs must match admitted module ports".into());
                        }
                        Ok(StructuralAction::Pa(prepared))
                    }
                    #[cfg(not(feature = "hardware-host"))]
                    {
                        let _ = (configuration_json, program_buses);
                        Err("modules unavailable".into())
                    }
                }
                S::StructuralSnapshot {} | S::MasterEqSnapshot {} => unreachable!(),
            }
        })();
        self.structural_pending = Some((
            r.clone(),
            prepared.unwrap_or_else(StructuralAction::Failed),
            owner,
        ));
        Ok(Reply::new(
            &r,
            "pending",
            None,
            Some(frame),
            self.engine.revision(),
            None,
        ))
    }
    fn commit_structure(&mut self, now: u64) -> Result<()> {
        if self.engine.external_boundary()
            != Some(
                self.engine
                    .admission_frame()
                    .unwrap_or_else(|| self.frame()),
            )
            || self.structural_pending.is_none()
        {
            return Ok(());
        }
        let (r, mut action, owner) = self
            .structural_pending
            .take()
            .ok_or("missing structural prepared state")?;
        if !self
            .engine
            .external_matches(&r.authority_request(), &r.fingerprint()?)
        {
            self.quiesce_source("structural_identity_mismatch")?;
            return Err("structural prepared identity mismatch".into());
        }
        let frame = self
            .engine
            .admission_frame()
            .unwrap_or_else(|| self.frame());
        let epoch = self.epoch.0;
        let rearm_allowed = matches!(
            self.engine.clock_status().state,
            crate::clock_domain::ClockState::Running | crate::clock_domain::ClockState::Disarmed
        );
        let permit = self.prepare_source_external(now)?;
        let (permit, returned_action, _, _, _, receipt) = self.graph_boundary_command(
            frame,
            (permit, action, epoch, frame, rearm_allowed, None),
            |graph, packet| {
                packet.5 = Some(graph.apply_structure(
                    &packet.0,
                    &mut packet.1,
                    packet.2,
                    packet.3,
                    packet.4,
                ));
            },
        )?;
        let _ = permit;
        action = returned_action;
        let receipt = receipt.expect("structural receipt");
        let code = receipt.code;
        #[cfg(feature = "hardware-host")]
        if let Some(modules) = &mut self.modules {
            match &mut action {
                StructuralAction::Pa(prepared) => modules.finish_pa_change(prepared, code),
                StructuralAction::MasterEq(prepared) => modules.finish_master_eq(prepared, code),
                _ => (),
            }
        }
        let result = self.engine.finish_external(receipt);
        let publication = if code == 0
            && let StructuralAction::Patch(prepared) = &action
        {
            self.engine.acknowledge_output_patch(prepared)
        } else {
            Ok(())
        };
        self.finish_compat_render()?;
        let result = result?;
        publication?;
        if result.kind == "applied" && matches!(action, StructuralAction::Rearm) {
            self.engine.acknowledge_output_rearm()?;
        }
        let failure = if code == 0 || result.body.reason.as_deref() != Some("configuration_failed")
        {
            None
        } else {
            Some(match &action {
                StructuralAction::Failed(reason) => reason.clone(),
                StructuralAction::Patch(_) => "patch quiescence/identity".into(),
                StructuralAction::Rearm if !rearm_allowed => "clock recovery required".into(),
                #[cfg(feature = "hardware-host")]
                StructuralAction::MasterEq(_) => format!("EQ commit {code}"),
                #[cfg(feature = "hardware-host")]
                StructuralAction::Pa(_) => format!("PA commit {code}"),
                _ => format!("structural commit {code}"),
            })
        };
        #[cfg(feature = "hardware-host")]
        if let Some(g) = &mut self.modules {
            g.retire_pa_changes();
        }
        let applied_frame = if result.kind == "applied" {
            Some(frame)
        } else {
            None
        };
        let reply = crate::structural_control::Reply::new(
            &r,
            "final",
            failure.or(result.body.reason),
            applied_frame,
            result.body.revision,
            None,
        );
        let writer = r.writer.clone();
        self.structural_cache.push_back((r, reply.clone()));
        if self
            .structural_cache
            .iter()
            .filter(|(r, _)| r.writer == writer)
            .count()
            > 64
            && let Some(index) = self
                .structural_cache
                .iter()
                .position(|(r, _)| r.writer == writer)
        {
            self.structural_cache.remove(index);
        }
        if self.structural_cache.len() > 256 {
            self.structural_cache.pop_front();
        }
        if let Some(id) = owner {
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
                let _ = c.queue_module(&reply, now);
            }
        } else if self.remote_completions.len() < 64 {
            self.remote_completions
                .push_back(serde_json::to_value(reply).map_err(|e| e.to_string())?);
        }
        Ok(())
    }
    pub fn take_remote_completions(&mut self) -> Vec<serde_json::Value> {
        self.remote_completions.drain(..).collect()
    }
    pub fn source_epoch(&self) -> u64 {
        self.epoch.0
    }
    pub fn quiesce_source(&mut self, reason: &str) -> Result<()> {
        self.source_closure
            .store(true, std::sync::atomic::Ordering::Release);
        self.engine.quiesce(reason);
        #[cfg(feature = "hardware-host")]
        let module_closure = self
            .modules
            .as_mut()
            .map(|g| g.quiesce_source())
            .transpose();
        #[cfg(feature = "hardware-host")]
        self.reset_fx_aux();
        if let Some(s) = &mut self.measurement {
            s.invalidate_all(reason);
        }
        self.invalidate_pending_measurement(reason);
        self.stop_analysis();
        self.routing_command((), |routing, _| {
            if let Some(tap) = &mut routing.metering {
                tap.reset();
            }
        })?;
        if let Some(latest) = &mut self.metering {
            latest.clear();
        }
        self.close_brain_audio();
        self.brain_pending = None;
        self.brain_cache.clear();
        if self.source_executor.is_some() || self.render_graph.is_some() {
            let observation = self.graph_command(None, |graph, observation| {
                graph.close_numeric();
                *observation = Some(graph.mixer.mixer.output_observation());
            })?;
            self.engine
                .acknowledge_source_quiescence(observation.expect("source quiescence"));
        }
        #[cfg(feature = "hardware-host")]
        if let Some(g) = &mut self.modules {
            module_closure?;
            if self.render_graph.is_some() || self.source_executor.is_some() {
                let packet = g.prepare_exchange()?;
                let packet = self.graph_command(packet, |graph, packet| {
                    if let Some(source) = &mut graph.modules {
                        source.exchange(packet);
                    }
                })?;
                self.modules
                    .as_mut()
                    .expect("module controller")
                    .finish_exchange(packet)?;
            }
        }
        self.structural_pending = None;
        self.structural_cache.clear();
        self.clients.clear();
        self.writer_connections.clear();
        self.pending_owner = None;
        self.processing_owner = None;
        self.sends_owner = None;
        Ok(())
    }
    pub fn recover_source(&mut self, epoch: Counter, frame: u64) -> Result<()> {
        if epoch.0 <= self.epoch.0 || !frame.is_multiple_of(48) {
            return Err("fresh epoch and aligned source frame required".into());
        }
        #[cfg(feature = "hardware-host")]
        if self.clocked_source {
            return Err("stop source clock before epoch recovery".into());
        }
        self.quiesce_source("source_reopen")?;
        let mut executor = self.source_executor.take();
        if let Some(worker) = &mut executor {
            match worker.suspend() {
                Ok(graph) => self.render_graph = Some(graph),
                Err(error) => {
                    self.source_executor = executor;
                    return Err(error);
                }
            }
        }
        let recovery = (|| -> Result<()> {
            self.source_quantum = None;
            if let Some(graph) = self.render_graph.take() {
                self.attach_render_graph(*graph)?;
            }
            self._epoch_owner.advance(epoch)?;
            self.engine.recover(epoch, frame)?;
            self.source_completion = self.engine.source_completion();
            #[cfg(feature = "hardware-host")]
            self.reset_fx_aux();
            #[cfg(feature = "hardware-host")]
            if let Some(g) = &mut self.modules {
                g.discontinuity(epoch.0, frame)?;
            }
            self.epoch = epoch;
            self.source_closure = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.spectrum.reset();
            self.brain_frame_times.fill(None);
            self.brain_frame_times[(frame / 48 % 64) as usize] = Some((frame, self.last_now));
            self.analysis = None;
            self.routing_command(None, |routing, replacement| {
                std::mem::swap(&mut routing.analysis, replacement);
            })?;
            self.remote_completions.clear();
            Ok(())
        })();
        // Even a failed rebuild retains the original worker reservation. Never
        // silently fall back to inline rendering while sibling media is detached.
        let reinstalled = if let Some(mut worker) = executor {
            let result = self
                .take_render_graph()
                .and_then(|graph| worker.resume(Box::new(graph)));
            self.source_executor = Some(worker);
            result
        } else {
            Ok(())
        };
        recovery.and(reinstalled)
    }

    pub fn rearm(&mut self) -> Result<()> {
        if self.render_graph.is_some() || self.source_executor.is_some() {
            if self.frame() != 0
                || self
                    .source_closure
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err("persistent source requires structural rearm transaction".into());
            }
            let mut clock = self.engine.clock_status().clone();
            if clock.state == crate::clock_domain::ClockState::Disarmed {
                clock.rearm().map_err(String::from)?;
            } else if clock.state != crate::clock_domain::ClockState::Running {
                return Err("clock recovery required".into());
            }
            let (result, observation, module_code) =
                self.graph_command((None, None, 0), |graph, p| {
                    #[cfg(feature = "hardware-host")]
                    if let Some(modules) = &mut graph.modules {
                        p.2 = modules.rearm_pa();
                    }
                    if p.2 == 0 {
                        p.0 = Some(graph.mixer.mixer.rearm());
                        p.1 = Some(graph.mixer.mixer.output_observation());
                    }
                })?;
            if module_code != 0 {
                return Err(format!("PA rearm {module_code}"));
            }
            result
                .expect("startup rearm result")
                .map_err(|e| format!("source rearm {e:?}"))?;
            return self
                .engine
                .acknowledge_startup_rearm(observation.expect("startup output observation"));
        }
        #[cfg(feature = "hardware-host")]
        if let Some(g) = &mut self.modules {
            g.rearm_pa()?;
        }
        self.engine.rearm()
    }
    #[cfg(feature = "hardware-host")]
    pub fn configure_pa(&mut self, json: &[u8]) -> Result<()> {
        if self.frame() != 0 || self.render_graph.is_some() || self.source_executor.is_some() {
            return Err("initial PA setup requires unprocessed provider".into());
        }
        let capacity = self.topology().pa_outputs;
        let total = self.topology().monitors + 2;
        let g = self.modules.as_mut().ok_or("modules unavailable")?;
        if g.pa_configuration_json().is_some() {
            return Err("PA already configured; use structural transaction".into());
        }
        let mut prepared = g.prepare_pa_change(json, vec![0, 1], total)?;
        if prepared.output_channels() != capacity {
            return Err("initial PA output capability mismatch".into());
        }
        let code = g.commit_pa_change(&mut prepared, self.epoch.0, 0);
        if code != 0 {
            return Err(format!("initial PA commit {code}"));
        }
        g.retire_pa_changes();
        Ok(())
    }
    pub fn last_bus_samples(&self) -> &[f64] {
        &self.routing_observed.buses
    }
    pub fn revoke_writer(&mut self, writer: &str) {
        #[cfg(feature = "hardware-host")]
        self.revoke_fx_aux_writer(writer);
        if let Some(s) = &mut self.measurement {
            let ids: Vec<_> = s
                .guards()
                .filter(|(_, r, _)| r.writer.as_deref() == Some(writer))
                .map(|(id, _, _)| id.to_owned())
                .collect();
            for id in ids {
                s.invalidate(&id, "measurement writer revoked");
            }
        }
        if self
            .measurement_pending
            .as_ref()
            .is_some_and(|p| p.request.writer.as_deref() == Some(writer))
        {
            self.invalidate_pending_measurement("measurement writer revoked");
        }
        self.measurement_cache
            .retain(|(r, _)| r.writer.as_deref() != Some(writer));
        if self
            .brain
            .hold
            .as_ref()
            .is_some_and(|h| h.0.writer.as_deref() == Some(writer))
        {
            self.brain.close();
        }
        if self
            .brain_pending
            .as_ref()
            .is_some_and(|p| p.0.writer.as_deref() == Some(writer))
        {
            self.brain_pending = None;
        }
        let cancelled_send = self.engine.pending_sends_ticket_for(writer);
        self.engine.revoke_writer(writer);
        #[cfg(feature = "hardware-host")]
        if self.enforce_clock_brain_authority(self.last_now).is_err() {
            self.source_closure
                .store(true, std::sync::atomic::Ordering::Release);
        }
        if cancelled_send
            .is_some_and(|ticket| self.sends_owner.is_some_and(|owner| owner.0 == ticket))
        {
            self.sends_owner = None;
        }
        if self
            .structural_pending
            .as_ref()
            .is_some_and(|(r, _, _)| r.writer.as_deref() == Some(writer))
        {
            self.structural_pending = None;
        }
        self.structural_cache
            .retain(|(r, _)| r.writer.as_deref() != Some(writer));
        // Keep the connection tombstone for this epoch: reconnect must use a
        // fresh writer identity, even after its old lease is revoked.
        self.remote_completions.retain(|r| {
            !(r["contract"] == crate::fx_aux_wire::CONTRACT && r["writer"].as_str() == Some(writer))
                && r.get("context")
                    .and_then(|c| c.get("writer"))
                    .and_then(|w| w.as_str())
                    != Some(writer)
        });
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn engine(&self) -> &OfflineEngine {
        &self.engine
    }
    pub fn engine_mut(&mut self) -> &mut OfflineEngine {
        &mut self.engine
    }
    pub fn topology(&self) -> &crate::topology::EngineTopology {
        self.engine.topology()
    }
    pub fn enable_synthetic_fouraux(&mut self) -> Result<()> {
        if self.frame() != 0 || self.analysis.is_some() {
            return Err("source must be selected before processing".into());
        }
        self.synthetic_fouraux = true;
        Ok(())
    }
    pub fn analysis_alive(&self) -> bool {
        self.analysis.as_ref().is_some_and(|worker| worker.alive())
    }
    pub fn stop_analysis(&self) {
        if let Some(worker) = &self.analysis {
            worker.stop();
        }
    }
    pub fn enable_analysis(&mut self, directory: &Path) -> Result<()> {
        if !self.synthetic_fouraux {
            return Err(
                "four named inputs unavailable in default source; select explicit fouraux".into(),
            );
        }
        if self.analysis.is_some() {
            return Err("analysis already enabled".into());
        }
        let descriptor = crate::analysis_stream::Descriptor::new(self.epoch.0, self.frame());
        self.start_analysis(directory, descriptor)
    }
    /// Explicit raw-strip attachment to the actual configured source path.
    /// Selection is startup-only and never opens a second PCM owner.
    pub fn enable_configured_analysis(
        &mut self,
        directory: &Path,
        mapping: &crate::analysis_stream::Mapping,
    ) -> Result<()> {
        if self.frame() != 0 || self.analysis.is_some() {
            return Err("analysis mapping requires fresh provider startup".into());
        }
        let descriptor = mapping.descriptor(self.topology(), self.epoch.0, self.frame())?;
        self.start_analysis(directory, descriptor)
    }
    fn start_analysis(
        &mut self,
        directory: &Path,
        descriptor: crate::analysis_stream::Descriptor,
    ) -> Result<()> {
        let (tap, input) = crate::analysis_stream::Tap::new(&descriptor).map_err(String::from)?;
        let service =
            crate::analysis_stream::local::LocalAnalysis::bind(directory, descriptor, input)?;
        let worker = crate::analysis_stream::local::Worker::start(service)?;
        self.routing_command(
            Some((tap, worker.source_signal(), self.topology().map_revision)),
            |routing, replacement| std::mem::swap(&mut routing.analysis, replacement),
        )?;
        self.analysis = Some(worker);
        Ok(())
    }
    #[cfg(feature = "hardware-host")]
    pub fn enable_modules(&mut self, manifest: &Path) -> Result<()> {
        if self.frame() != 0
            || self.modules.is_some()
            || self.render_graph.is_some()
            || self.source_executor.is_some()
        {
            return Err("module preparation before processing required".into());
        }
        let root = self.private_directory.join("takes");
        match fs::create_dir(&root) {
            Ok(()) => fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.to_string()),
        }
        let m = fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
        if !m.is_dir()
            || m.file_type().is_symlink()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o777 != 0o700
            || fs::canonicalize(&root).map_err(|e| e.to_string())? != root
        {
            return Err("owned recording root".into());
        }
        let manifest = crate::module_graph::Manifest::load(manifest)?;
        self.modules = Some(crate::module_graph::ModuleGraph::load_configured(
            manifest,
            self.epoch.0,
            self.frame(),
            self.topology().inputs.len(),
        )?);
        Ok(())
    }
    pub fn module_status(&mut self, now: u64) -> Result<serde_json::Value> {
        #[cfg(feature = "hardware-host")]
        if let Some(graph) = &mut self.modules {
            graph.poll_lifecycle()?;
            return serde_json::to_value(graph.status(&self.show, now)?).map_err(|e| e.to_string());
        }
        let _ = now;
        Ok(crate::module_wire::unavailable_status(
            &self.show,
            self.epoch.0,
            self.frame(),
        ))
    }
    fn module_request(
        &mut self,
        index: usize,
        r: crate::module_wire::ModuleRequest,
        now: u64,
    ) -> Result<()> {
        if !matches!(
            r.command,
            crate::module_wire::ModuleCommand::ModuleStatus {}
        ) {
            let c = &self.clients[index];
            if !c.snapshot || c.writer != r.writer || c.lease != r.lease {
                return Err("module connection authority".into());
            }
        }
        let result = self.dispatch_module(r, now, Some(self.clients[index].id))?;
        self.clients[index].queue_module(&result, now)
    }
    pub fn dispatch_module(
        &mut self,
        r: crate::module_wire::ModuleRequest,
        now: u64,
        owner: Option<u64>,
    ) -> Result<serde_json::Value> {
        use crate::module_wire::{ModuleCommand, ModuleReply};
        #[cfg(not(feature = "hardware-host"))]
        let _ = owner;
        if r.version != self.engine.wire_version() {
            return Err("module wire version mismatch".into());
        }
        if r.show_id != self.show || r.epoch != self.epoch {
            return Err("module attachment identity".into());
        }
        if matches!(r.command, ModuleCommand::ModuleStatus {}) {
            let status = self.module_status(now)?;
            return Ok(status);
        }
        let auth = self
            .engine
            .authorize_module(&r.authority_request(), &r.fingerprint()?, now);
        let reply = match auth {
            Err(reason) => ModuleReply::new(&r, "rejected", Some(reason)),
            Ok((true, _)) => self
                .module_cache
                .iter()
                .find(|(old, _)| old == &r)
                .map(|(_, p)| p.clone())
                .unwrap_or_else(|| {
                    ModuleReply::new(&r, "rejected", Some("expired_outcome".into()))
                }),
            Ok((false, Some(reason))) => {
                let reply = ModuleReply::new(&r, "rejected", Some(reason));
                self.module_cache.push_back((r.clone(), reply.clone()));
                if self.module_cache.len() > 256 {
                    self.module_cache.pop_front();
                }
                reply
            }
            Ok((false, None)) => {
                #[cfg(feature = "hardware-host")]
                let result = if let Some(graph) = &mut self.modules {
                    if self.module_pending.len() >= 2 {
                        Err("lifecycle backpressure".into())
                    } else {
                        match &r.command {
                            ModuleCommand::RecordStart {
                                take_id,
                                operation_id,
                            } => graph.start(
                                &self.private_directory.join("takes"),
                                take_id,
                                *operation_id,
                            ),
                            ModuleCommand::RecordStop {
                                take_id,
                                operation_id,
                            } => graph.stop(take_id, *operation_id),
                            _ => unreachable!(),
                        }
                    }
                } else {
                    Err("modules unavailable".into())
                };
                #[cfg(not(feature = "hardware-host"))]
                let result: Result<()> = Err("modules unavailable".into());
                let reply = match result {
                    Ok(()) => {
                        #[cfg(feature = "hardware-host")]
                        self.module_pending.push((r.clone(), owner.unwrap_or(0)));
                        ModuleReply::new(&r, "accepted_pending", None)
                    }
                    Err(reason) => ModuleReply::new(&r, "rejected", Some(reason)),
                };
                self.module_cache.push_back((r.clone(), reply.clone()));
                if self.module_cache.len() > 256 {
                    self.module_cache.pop_front();
                }
                reply
            }
        };
        serde_json::to_value(reply).map_err(|e| e.to_string())
    }
    fn processing_request(
        &mut self,
        index: usize,
        r: crate::processing_wire::ProcessingRequest,
        now: u64,
    ) -> Result<()> {
        use crate::processing_wire::{ProcessingCommand, ProcessingReply};
        let read = matches!(r.command, ProcessingCommand::ProcessingSnapshot {});
        let client = &self.clients[index];
        let reply = if !read
            && (!client.snapshot || client.writer != r.writer || client.lease != r.lease)
        {
            ProcessingReply::new(&r, "final", Some("lease".into()), self.engine.revision())
        } else {
            self.engine.handle_processing_with_freshness(
                &r,
                now,
                client
                    .processing_snapshot_ms
                    .is_some_and(|t| now.saturating_sub(t) <= 250),
            )?
        };
        if read && reply.snapshot.is_some() {
            self.clients[index].processing_snapshot_ms = Some(now);
        }
        if reply.state == "pending" {
            self.processing_owner = Some((
                reply.ticket.expect("pending ticket").0,
                self.clients[index].id,
            ));
        }
        reply.validate()?;
        self.clients[index].queue_module(&reply, now)
    }
    fn sends_request(
        &mut self,
        index: usize,
        r: crate::sends_wire::SendsRequest,
        now: u64,
    ) -> Result<()> {
        use crate::sends_wire::{SendsCommand, SendsReply};
        let read = matches!(r.command, SendsCommand::SendsSnapshot {});
        let client = &self.clients[index];
        let reply = if !read
            && (!client.snapshot || client.writer != r.writer || client.lease != r.lease)
        {
            SendsReply::new(&r, "final", Some("lease".into()), self.engine.revision())
        } else {
            self.engine.handle_sends_with_freshness(
                &r,
                now,
                client
                    .sends_snapshot_ms
                    .is_some_and(|t| now.saturating_sub(t) <= 250),
            )?
        };
        if read && reply.snapshot.is_some() {
            self.clients[index].sends_snapshot_ms = Some(now);
        }
        if reply.state == "pending" {
            self.sends_owner = Some((
                reply.ticket.expect("pending ticket").0,
                self.clients[index].id,
            ));
        }
        reply.validate()?;
        self.clients[index].queue_module(&reply, now)
    }
    /// Captured final PA output from the actual owner graph, when enabled.
    #[cfg(feature = "hardware-host")]
    pub fn module_output(&self) -> Option<&[f64; 48 * 6]> {
        self.modules.as_ref().map(|g| g.output())
    }
    #[cfg(feature = "hardware-host")]
    fn module_completions(&mut self, now: u64) -> Result<()> {
        use crate::module_wire::{ModuleCommand, ModuleReply};
        let Some(graph) = &mut self.modules else {
            return Ok(());
        };
        graph.poll_lifecycle()?;
        if self.module_pending.is_empty() {
            return Ok(());
        }
        let status = graph.status(&self.show, now)?;
        let Some(recording) = status.recording else {
            return Ok(());
        };
        let mut i = 0;
        while i < self.module_pending.len() {
            let (r, owner) = &self.module_pending[i];
            let start = matches!(r.command, ModuleCommand::RecordStart { .. });
            let done = if start {
                recording.state == "recording" || recording.state == "finalized"
            } else {
                recording.state == "finalized"
            };
            if !done {
                i += 1;
                continue;
            }
            let state = if start && recording.state != "recording" {
                "cancelled"
            } else {
                "completed"
            };
            let reply = ModuleReply::new(r, state, Some(recording.outcome.clone()));
            if let Some((_, cached)) = self.module_cache.iter_mut().find(|(old, _)| old == r) {
                *cached = reply.clone();
            }
            if *owner == 0 && self.remote_completions.len() < 64 {
                self.remote_completions
                    .push_back(serde_json::to_value(&reply).map_err(|e| e.to_string())?);
            }
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == *owner) {
                let _ = c.queue_module(&reply, now);
            }
            self.module_pending.remove(i);
        }
        Ok(())
    }
    pub fn frame(&self) -> u64 {
        self.engine.frame()
    }
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
    pub fn snapshot(&mut self) -> Result<crate::mixer_control::RenderedSnapshot> {
        self.engine.snapshot()
    }
    /// Control-side observation, outside the bounded mixer callback.
    pub fn processing_snapshot(&mut self) -> Result<crate::processing_wire::ProcessingSnapshot> {
        self.engine.processing_snapshot()
    }
    /// Bounded synthetic work: <=4 accepts, one frame/client,8KiB read/write/client,
    ///48 render frames. All parsing/IPC/heap work is outside Mixer::process.
    pub fn tick(&mut self, now: u64) -> Result<()> {
        let (mut capture, mut playback) =
            self.routing_command((Vec::new(), Vec::new()), |routing, packet| {
                std::mem::swap(&mut routing.capture_scratch, &mut packet.0);
                std::mem::swap(&mut routing.playback_scratch, &mut packet.1);
            })?;
        capture.fill(0.);
        let topology = self.topology();
        let mut synthetic = self
            .synthetic_fouraux
            .then(|| SyntheticBlock::new(self.frame()));
        for f in 0..48 {
            for (i, p) in topology.inputs.iter().enumerate() {
                capture[f * topology.capture_channels + p.capture_slot] =
                    if let Some(source) = &mut synthetic {
                        source.sample(self.frame() + f as u64, i)
                    } else if i == 0 {
                        0.125
                    } else {
                        0.
                    };
            }
        }
        let result =
            self.tick_with_capture(now, self.epoch.0, self.frame(), &capture, &mut playback);
        self.routing_command((capture, playback), |routing, packet| {
            std::mem::swap(&mut routing.capture_scratch, &mut packet.0);
            std::mem::swap(&mut routing.playback_scratch, &mut packet.1);
        })?;
        result
    }
    /// Same provider pump, with caller-owned observation of the FOH/monitor block.
    /// This is software evidence; it does not open a hardware endpoint.
    pub fn tick_with_output(&mut self, now: u64, output: &mut [[f64; 4]; 48]) -> Result<()> {
        if !self.topology().is_legacy() {
            return Err("legacy output shape".into());
        }
        let mut inputs = [[0.; 8]; 48];
        let mut synthetic = self
            .synthetic_fouraux
            .then(|| SyntheticBlock::new(self.frame()));
        for (f, row) in inputs.iter_mut().enumerate() {
            for (i, v) in row.iter_mut().enumerate() {
                *v = if let Some(source) = &mut synthetic {
                    source.sample(self.frame() + f as u64, i)
                } else if i == 0 {
                    0.125
                } else {
                    0.
                };
            }
        }
        self.tick_raw(now, inputs.as_flattened(), output.as_flattened_mut())
    }
    /// Capture and playback use configured transport slots. Logical strip ordering
    /// is independent of that permutation. This control pump is outside render.
    pub fn tick_with_capture(
        &mut self,
        now: u64,
        epoch: u64,
        first_frame: u64,
        capture: &[f64],
        playback: &mut [f64],
    ) -> Result<()> {
        self.tick_with_capture_and_wet(now, epoch, first_frame, capture, playback, None)
    }
    pub fn tick_with_capture_and_wet(
        &mut self,
        now: u64,
        epoch: u64,
        first_frame: u64,
        capture: &[f64],
        playback: &mut [f64],
        wet: Option<&[f64]>,
    ) -> Result<()> {
        self.tick_with_capture_and_brain(now, epoch, first_frame, capture, playback, wet, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn tick_with_capture_and_brain(
        &mut self,
        now: u64,
        epoch: u64,
        first_frame: u64,
        capture: &[f64],
        playback: &mut [f64],
        wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
    ) -> Result<()> {
        #[cfg(feature = "hardware-host")]
        if self.clocked_source {
            return Err("source clock owns capture progression".into());
        }
        let talkback =
            if talkback.is_some_and(|v| v.len() != 48 || v.iter().any(|s| !s.is_finite())) {
                self.close_brain_audio();
                None
            } else {
                talkback
            };
        let topology = self.topology();
        if capture.len() != 48 * topology.capture_channels
            || playback.len() != 48 * topology.playback_channels
        {
            return Err("device block shape".into());
        }
        playback.fill(0.);
        if epoch != self.epoch.0 || first_frame != self.frame() {
            self.quiesce_source("source_discontinuity")?;
            return Err("source timeline".into());
        }
        if let Err(e) = self.engine.admit_source(epoch, first_frame, 48) {
            self.quiesce_source("source_discontinuity")?;
            return Err(e);
        }
        let result = self.tick_block(now, capture, playback, wet, talkback, true);

        if result.is_err() {
            self.quiesce_source("processing_failure")?;
        }
        result
    }

    /// Keep the exclusive numeric graph across production source quanta. The
    /// controller facade never regains its active mixer/module owners between blocks.
    #[cfg(any(feature = "hardware-host", test))]
    pub(crate) fn enable_persistent_source(&mut self) {
        self.persistent_source = true;
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn source_executor_for<T: Send + 'static>(
        &mut self,
        owner: T,
    ) -> Result<crate::source_executor::SourceExecutor<T>> {
        self.ensure_render_graph()?;
        if self.source_executor.is_none() {
            let graph = self.render_graph.take().expect("source graph");
            self.source_executor = Some(crate::source_executor::SourceExecutor::new(
                "gp-source",
                graph,
            )?);
        }
        self.source_executor.as_ref().unwrap().sibling(owner)
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn source_pair_command<U: Send + 'static, P: Send + 'static>(
        &mut self,
        other: &crate::source_executor::SourceExecutor<U>,
        packet: P,
        advance: fn(&mut Box<source::RenderGraph>, &mut U, &mut P) -> bool,
    ) -> Result<P> {
        self.source_executor
            .as_ref()
            .ok_or("source worker absent")?
            .transact_pair_until(other, packet, advance)
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn finish_remote_fx_aux_mode(&mut self, enabled: bool) {
        self.remote_fx_aux = enabled;
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn source_control_time(&self) -> u64 {
        self.last_now
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn start_source_progress<U: Send + 'static, P: Send + 'static>(
        &mut self,
        now: u64,
        media: &crate::source_executor::SourceExecutor<U>,
        state: P,
        advance: fn(
            &mut Box<source::RenderGraph>,
            &mut U,
            &mut P,
            std::time::Instant,
        ) -> std::time::Duration,
    ) -> Result<()> {
        if self.clocked_source || self.engine.external_boundary().is_some() {
            return Err(
                "source clock requires completed startup and no structural reservation".into(),
            );
        }
        if now < self.last_now {
            return Err("source startup clock regression".into());
        }
        // Startup control (including REC preparation) may already have advanced
        // authority time without rendering a block. Use the caller's current
        // monotonic time for the first source admission, never the initial zero.
        self.last_now = now;
        let routing = self.clock_routing(now);
        self.graph_command(Some(routing), |graph, routing| {
            std::mem::swap(&mut graph.clock_routing, routing);
        })?;
        if self.frame() == 0 {
            self.engine.prepare_render(48, self.last_now)?;
            let command = self
                .engine
                .take_source_command()
                .ok_or("startup source command")?;
            let (_, result) = self.graph_command((command, None), |graph, packet| {
                packet.1 = Some(graph.mixer.install_command(&mut packet.0));
            })?;
            result
                .ok_or("startup source result")?
                .map_err(|e| format!("startup source {e:?}"))?;
        }
        self.source_executor
            .as_ref()
            .ok_or("source worker absent")?
            .start_progress(media, state, advance)?;
        self.clocked_source = true;
        Ok(())
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn stop_source_progress<P: Send + 'static>(&mut self) -> Result<P> {
        let executor = self
            .source_executor
            .as_ref()
            .ok_or("source worker absent")?;
        executor.transact_until((), |graph, _| {
            graph.closure.load(std::sync::atomic::Ordering::Acquire)
        })?;
        let state = executor.stop_progress()?;
        self.clocked_source = false;
        Ok(state)
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn clocked_completion_reservation(&self) -> usize {
        self.remote_completions.len() + self.engine.queued_render_replies() + 1
    }
    /// Observe and retire source work without pacing capture. The complete
    /// module context exchange follows any synchronous structural completion.
    #[cfg(feature = "hardware-host")]
    pub(crate) fn service_source_clock(&mut self, now: u64) -> Result<()> {
        self.last_now = now;
        self.enforce_clock_brain_authority(now)?;
        self.module_completions(now)?;
        let modules = self
            .modules
            .as_mut()
            .map(|m| m.prepare_exchange())
            .transpose()?;
        let packet = (
            self.engine.source_completion(),
            source::RoutingObservation::new(self.topology()),
            modules,
            false,
            false,
        );
        let (mut completion, routing, modules, completed, observed) =
            self.graph_command(packet, |graph, p| {
                if let (Some(source), Some(exchange)) = (&mut graph.modules, &mut p.2) {
                    source.exchange(exchange);
                }
                if graph.closure.load(std::sync::atomic::Ordering::Acquire) && graph.mixer.busy() {
                    graph.mixer.finish_closed_command();
                }
                p.3 = graph.mixer.publish_completion(&mut p.0);
                p.4 = p.3 || graph.mixer.observe_latest(&mut p.0);
                graph.observe_routing(&mut p.1);
            })?;
        if let Some(exchange) = modules {
            self.modules
                .as_mut()
                .expect("module controller")
                .finish_exchange(exchange)?;
        }
        if !observed {
            return Ok(());
        }
        self.routing_observed = routing;
        if completed {
            for reply in self.engine.finish_source_completion(&mut completion)? {
                if self.remote_completions.len() >= 64 {
                    return Err("remote final capacity".into());
                }
                self.remote_completions
                    .push_back(serde_json::to_value(reply).map_err(|e| e.to_string())?);
            }
        } else {
            self.engine.finish_source_observation(&mut completion)?;
        }
        self.observe_fx_aux()?;
        self.brain_frame_times[(self.frame() / 48 % 64) as usize] = Some((self.frame(), now));
        if let Some(latest) = &mut self.metering {
            latest.drain();
        }
        self.validate_measurement(now);
        self.module_completions(now)
    }
    /// Reserve source time before allocating structural/Brain/local-aux state.
    /// Final publication waits for the retained source outcome; stale preparation
    /// returns its storage without mutating the graph.
    #[cfg(feature = "hardware-host")]
    pub(crate) fn clocked_control(
        &mut self,
        now: u64,
        fence: Option<crate::remote::PolicyFence>,
        prepare: impl FnOnce(&mut Self) -> Result<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let entered = std::time::Instant::now();
        if !self.clocked_source
            || self.engine.external_boundary().is_some()
            || self.clocked_completion_reservation() > 64
        {
            return Err("clocked control preparation state".into());
        }
        self.service_source_clock(now)?;
        let admission = self
            .graph_command(None, |graph, p| {
                *p = Some(
                    graph
                        .mixer
                        .reserve_admission(graph.mixer.mixer.frame().saturating_add(480)),
                );
            })?
            .ok_or("source admission absent")?
            .map_err(|e| format!("source admission {e:?}"))?;
        self.engine.begin_source_admission(admission)?;
        self.clocked_fence = fence;
        self.clocked_preparation_started = Some((now, entered));
        let result = (|| {
            let reply = prepare(self)?;
            let now = now.saturating_add(entered.elapsed().as_millis() as u64);
            self.commit_structure(now)?;
            self.commit_brain(now)?;
            self.poll_fx_aux(now)?;
            Ok(reply)
        })();
        self.engine.end_source_admission();
        self.clocked_fence = None;
        self.clocked_preparation_started = None;
        if result.is_err() {
            if self.engine.abort_external_preparation().is_ok() {
                self.structural_pending = None;
                self.brain_pending = None;
                self.fx_aux_pending = None;
            } else {
                // A worker failure after transfer has no trusted numeric receipt.
                // Close this epoch rather than publish speculative application.
                self.quiesce_source("clocked_control_outcome_lost")?;
            }
        }
        result
    }
    /// Complete a prepared authority-only external intent on the same ordered
    /// source boundary as structural and Brain routing commits. The caller runs
    /// this inside clocked_control after begin_external, then acts on its receipt.
    #[cfg(feature = "hardware-host")]
    pub(crate) fn commit_clocked_external(
        &mut self,
        now: u64,
    ) -> Result<crate::control_model::Reply> {
        let frame = self
            .engine
            .admission_frame()
            .ok_or("clocked external admission absent")?;
        if !self.clocked_source || self.engine.external_boundary() != Some(frame) {
            return Err("clocked external boundary".into());
        }
        let permit = self.prepare_source_external(now)?;
        let (_, receipt) =
            self.graph_boundary_command(frame, (permit, None), |graph, packet| {
                packet.1 = Some(graph.mixer.commit_external(&packet.0, |_| 0));
            })?;
        self.engine
            .finish_external(receipt.ok_or("clocked external receipt")?)
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn remote_target_frame(&mut self, lead: u64) -> Result<u64> {
        self.graph_command((lead, None), |graph, p| {
            p.1 = graph.mixer.mixer.frame().checked_add(p.0);
        })?
        .1
        .ok_or_else(|| "remote target frame exhausted".into())
    }
    /// Publish the remote owner's permission at a source-issued boundary. An aux
    /// send packet is retained on the source until its later negotiated frame;
    /// sending the Permit therefore never depends on polling that exact frame.
    #[cfg(feature = "hardware-host")]
    pub(crate) fn clocked_remote_permission(
        &mut self,
        now: u64,
        request: &crate::control_model::Request,
        fingerprint: &str,
        target: u64,
        fences: [crate::remote::PolicyFence; 2],
        mut action: Option<crate::mixer_control::source::RemoteFxSends>,
    ) -> Result<crate::control_model::Reply> {
        let entered = std::time::Instant::now();
        self.service_source_clock(now)?;
        let admission = self
            .graph_command(None, |graph, p| {
                *p = Some(
                    graph
                        .mixer
                        .reserve_admission(graph.mixer.mixer.frame().saturating_add(480)),
                );
            })?
            .ok_or("remote source admission")?
            .map_err(|e| format!("remote source admission {e:?}"))?;
        let frame = admission.frame();
        if frame >= target {
            return Err("remote permission preparation expired".into());
        }
        self.engine.begin_source_admission(admission)?;
        let result = (|| {
            self.engine
                .schedule_fx_external(request, fingerprint, frame)?;
            let checked_now = now.saturating_add(entered.elapsed().as_millis() as u64);
            let mut permit = self.engine.prepare_external(checked_now)?;
            permit.authority_fence = Some(fences[0].clone());
            if let (Some(action), Some(expiry)) = (&mut action, permit.expires) {
                action.expires = action.expires.min(expiry);
            }
            let (_, _, _, receipt) =
                self.graph_boundary_command(frame, (permit, action, fences, None), |graph, p| {
                    let available = p.1.is_none() || graph.mixer.remote_fx.is_none();
                    let valid = p.2.iter().all(|f| f.valid());
                    let receipt = graph
                        .mixer
                        .commit_external(&p.0, |_| if !available || !valid { -3 } else { 0 });
                    if receipt.code == 0 && p.1.is_some() {
                        std::mem::swap(&mut graph.mixer.remote_fx, &mut p.1);
                    }
                    p.3 = Some(receipt);
                })?;
            self.engine
                .finish_external(receipt.ok_or("remote permission receipt")?)
        })();
        self.engine.end_source_admission();
        if result.is_err() && self.engine.abort_external_preparation().is_err() {
            self.quiesce_source("remote_permission_outcome_lost")?;
        }
        result
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn take_remote_fx_completion(&mut self) -> Result<Option<(u64, bool)>> {
        let packet = self.graph_command(None, |graph, p| {
            if graph
                .mixer
                .remote_fx
                .as_ref()
                .is_some_and(|r| r.result.is_some())
                || (graph.closure.load(std::sync::atomic::Ordering::Acquire)
                    && graph.mixer.remote_fx.is_some())
            {
                std::mem::swap(&mut graph.mixer.remote_fx, p);
            }
        })?;
        match packet {
            Some(p) => Ok(Some((p.ticket, p.result.unwrap_or(false)))),
            None => Ok(None),
        }
    }

    /// Controller-side preparation against a source-issued future boundary. The
    /// worker keeps rendering while this closure allocates/parses/validates.
    #[cfg(feature = "hardware-host")]
    pub(crate) fn clocked_engine_command(
        &mut self,
        now: u64,
        fence: Option<crate::remote::PolicyFence>,
        prepare: impl FnOnce(&mut OfflineEngine) -> Result<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let entered = std::time::Instant::now();
        if !self.clocked_source
            || self.engine.external_boundary().is_some()
            || self.clocked_completion_reservation() > 64
        {
            return Err("clocked engine preparation state".into());
        }
        let admission = self
            .graph_command(None, |graph, packet| {
                *packet = Some(
                    graph
                        .mixer
                        .reserve_admission(graph.mixer.mixer.frame().saturating_add(480)),
                );
            })?
            .ok_or("source admission absent")?
            .map_err(|e| format!("source admission {e:?}"))?;
        let frame = admission.frame();
        self.engine.begin_source_admission(admission)?;
        let prepared: Result<_> = (|| {
            let reply = prepare(&mut self.engine)?;
            // Release/revocation takes effect on copied held authorization before
            // waiting for an unrelated future no-op render completion.
            self.enforce_clock_brain_authority(now)?;
            self.engine.prepare_render(48, now)?;
            let expiry = entered
                + std::time::Duration::from_millis(u64::from(
                    self.engine.pending_lease_remaining(now).unwrap_or(0),
                ));
            let mut command = self
                .engine
                .take_source_command()
                .ok_or("clock command absent")?;
            command.expires = Some(expiry);
            command.authority_fence = fence;
            // Read-only operations have no lease and no scheduled mutation.
            if self.engine.pending_lease_remaining(now).is_none() && reply["state"] != "pending" {
                command.expires = None;
            }
            Ok((reply, command))
        })();
        self.engine.end_source_admission();
        let (reply, command) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                // Nothing was published to the source. Dispose prepared storage
                // here and remember refusal instead of stranding a pending edge.
                self.engine.abort_source_preparation()?;
                return Err(error);
            }
        };
        let packet = source::ClockCommand {
            frame,
            command,
            completion: self.engine.source_completion(),
            routing: source::RoutingObservation::new(self.topology()),
            installed: false,
            refused: false,
        };
        let mut packet = self
            .source_executor
            .as_ref()
            .ok_or("source worker absent")?
            .transact_until(packet, |graph, packet| {
                source::ClockCommand::advance(graph, packet)
            })?;
        let replies = if packet.refused {
            self.engine.reject_source_command(&packet.command)?
        } else {
            std::mem::swap(&mut self.routing_observed, &mut packet.routing);
            self.engine
                .finish_source_completion(&mut packet.completion)?
        };
        for final_reply in replies {
            if self.remote_completions.len() >= 64 {
                return Err("remote final capacity".into());
            }
            self.remote_completions
                .push_back(serde_json::to_value(final_reply).map_err(|e| e.to_string())?);
        }
        for final_reply in self.engine.take_processing_completions() {
            self.remote_completions
                .push_back(serde_json::to_value(final_reply).map_err(|e| e.to_string())?);
        }
        for final_reply in self.engine.take_sends_completions() {
            self.remote_completions
                .push_back(serde_json::to_value(final_reply).map_err(|e| e.to_string())?);
        }
        Ok(reply)
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn clock_routing(&self, now: u64) -> source::ClockRouting {
        let hold = self
            .brain
            .hold
            .as_ref()
            .filter(|_| self.brain_media_authorized_at(now, self.frame()));
        let expiry = hold.and_then(|(request, _, deadline, _)| {
            self.engine
                .live_lease_witness(request, now)
                .ok()
                .map(|(_, remaining)| (*deadline).min(now.saturating_add(u64::from(remaining))))
        });
        let expiry = if self.brain.foh {
            expiry
                .zip(self.brain.foh_authority.as_ref().and_then(|request| {
                    self.engine
                        .live_lease_witness(request, now)
                        .ok()
                        .map(|(_, remaining)| now.saturating_add(u64::from(remaining)))
                }))
                .map(|(hold, foh)| hold.min(foh))
        } else {
            expiry
        };
        source::ClockRouting {
            monitor: self.brain.source,
            selection_generation: self.brain.selection_generation,
            monitor_silent: !self.brain.monitor_armed || self.brain.monitor_mute,
            hold_generation: hold.map(|h| h.1),
            hold_fence: hold.and(self.brain_hold_fence.clone()),
            foh_fence: self
                .brain
                .foh
                .then(|| self.brain_foh_fence.clone())
                .flatten(),
            deadline_frame: hold.map(|h| h.3),
            deadline_ms: expiry,
            monitors: self.brain.monitors.clone(),
            foh: self.brain.foh && self.topology().pa_outputs > 0,
            gain: 10_f64.powf(self.brain.gain_cdb as f64 / 2000.),
            remote_fx_aux: self.remote_fx_aux,
        }
    }
    fn prepare_source_external(
        &mut self,
        now: u64,
    ) -> Result<crate::mixer_control::source::ExternalPermit> {
        #[cfg(feature = "hardware-host")]
        let now = self
            .clocked_preparation_started
            .map_or(now, |(base, entered)| {
                now.max(base.saturating_add(entered.elapsed().as_millis() as u64))
            });
        let permit = self.engine.prepare_external(now)?;
        #[cfg(feature = "hardware-host")]
        let permit = {
            let mut permit = permit;
            permit.authority_fence = self.clocked_fence.clone();
            permit
        };
        Ok(permit)
    }
    fn graph_boundary_command<P: Send + 'static>(
        &mut self,
        frame: u64,
        packet: P,
        apply: fn(&mut source::RenderGraph, &mut P),
    ) -> Result<P> {
        #[cfg(feature = "hardware-host")]
        if self.clocked_source {
            let observation = self.engine.source_completion();
            let routing = source::RoutingObservation::new(self.topology());
            let (_, packet, _, mut observation, routing, observed) = self
                .source_executor
                .as_ref()
                .ok_or("source worker absent")?
                .transact_until(
                    (frame, packet, apply, observation, routing, false),
                    |graph, p| {
                        if graph.mixer.mixer.frame() < p.0
                            && !graph.closure.load(std::sync::atomic::Ordering::Acquire)
                        {
                            return false;
                        }
                        (p.2)(graph, &mut p.1);
                        p.5 = graph.mixer.observe_latest(&mut p.3);
                        graph.observe_routing(&mut p.4);
                        true
                    },
                )?;
            if observed {
                // This is actual source state at the commit/refusal boundary,
                // not the future admission frame promoted into readback.
                self.engine.finish_source_observation(&mut observation)?;
                self.routing_observed = routing;
            }
            return Ok(packet);
        }
        let _ = frame;
        self.graph_command_retained(packet, apply)
    }
    #[cfg(all(test, feature = "hardware-host"))]
    pub(crate) fn inspect_source<P: Send + 'static>(
        &mut self,
        packet: P,
        inspect: fn(&mut source::RenderGraph, &mut P),
    ) -> Result<P> {
        self.graph_command(packet, inspect)
    }
    fn graph_command<P: Send + 'static>(
        &mut self,
        packet: P,
        apply: fn(&mut source::RenderGraph, &mut P),
    ) -> Result<P> {
        let packet = self.graph_command_retained(packet, apply)?;
        self.finish_compat_render()?;
        Ok(packet)
    }
    fn graph_command_retained<P: Send + 'static>(
        &mut self,
        mut packet: P,
        apply: fn(&mut source::RenderGraph, &mut P),
    ) -> Result<P> {
        if let Some(executor) = &self.source_executor {
            return executor
                .transact((packet, apply), |graph, p| (p.1)(graph, &mut p.0))
                .map(|p| p.0);
        }
        self.ensure_render_graph()?;
        apply(
            self.render_graph.as_mut().expect("source graph"),
            &mut packet,
        );
        Ok(packet)
    }
    fn routing_command<P: Send + 'static>(
        &mut self,
        mut packet: P,
        apply: fn(&mut source::SourceRouting, &mut P),
    ) -> Result<P> {
        if self.source_executor.is_some() || self.render_graph.is_some() {
            let (packet, _) = self.graph_command((packet, apply), |graph, p| {
                (p.1)(&mut graph.routing, &mut p.0);
            })?;
            Ok(packet)
        } else {
            apply(&mut self.source, &mut packet);
            Ok(packet)
        }
    }
    fn ensure_render_graph(&mut self) -> Result<()> {
        if self.render_graph.is_none() && self.source_executor.is_none() {
            self.render_graph = Some(Box::new(self.take_render_graph()?));
        }
        Ok(())
    }
    fn finish_compat_render(&mut self) -> Result<()> {
        if !self.persistent_source
            && let Some(graph) = self.render_graph.take()
        {
            self.attach_render_graph(*graph)?;
        }
        Ok(())
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn fx_aux_samples(&self) -> &[[f64; 96]; 4] {
        &self.routing_observed.aux
    }
    /// Transfer all DSP/acquisition storage together. Controller-only state and
    /// lifecycle workers remain here. Compatibility calls restore after rendering.
    fn take_render_graph(&mut self) -> Result<source::RenderGraph> {
        let mixer = self.engine.take_source()?;
        #[cfg(feature = "hardware-host")]
        let modules = match self.modules.as_mut().map(|g| g.take_source()).transpose() {
            Ok(modules) => modules,
            Err(error) => {
                self.engine.restore_source(mixer)?;
                return Err(error);
            }
        };
        #[cfg(feature = "hardware-host")]
        let aux = match self
            .fx_aux
            .as_mut()
            .map(|controller| controller.take_source())
            .transpose()
        {
            Ok(aux) => aux,
            Err(error) => {
                if let Some(modules) = modules {
                    self.modules
                        .as_mut()
                        .expect("module controller")
                        .restore_source(modules)?;
                }
                self.engine.restore_source(mixer)?;
                return Err(error);
            }
        };
        Ok(source::RenderGraph {
            #[cfg(feature = "hardware-host")]
            clock_routing: None,
            mixer,
            routing: self.source.take(),
            closure: self.source_closure.clone(),
            #[cfg(feature = "hardware-host")]
            modules,
            #[cfg(feature = "hardware-host")]
            aux,
        })
    }
    fn attach_render_graph(&mut self, graph: source::RenderGraph) -> Result<()> {
        self.engine.restore_source(graph.mixer)?;
        self.source.restore(graph.routing);
        #[cfg(feature = "hardware-host")]
        {
            if let Some(module_source) = graph.modules {
                self.modules
                    .as_mut()
                    .expect("module controller")
                    .restore_source(module_source)?;
            }
            if let Some(aux) = graph.aux {
                self.fx_aux
                    .as_mut()
                    .ok_or("aux controller missing")?
                    .restore_source(aux)?;
            }
        }
        Ok(())
    }
    fn tick_raw(&mut self, now: u64, inputs: &[f64], output: &mut [f64]) -> Result<()> {
        self.tick_raw_with_wet(now, inputs, output, None)
    }
    fn tick_raw_with_wet(
        &mut self,
        now: u64,
        inputs: &[f64],
        output: &mut [f64],
        wet: Option<&[f64]>,
    ) -> Result<()> {
        self.tick_raw_with_brain(now, inputs, output, wet, None)
    }
    fn tick_raw_with_brain(
        &mut self,
        now: u64,
        inputs: &[f64],
        output: &mut [f64],
        wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
    ) -> Result<()> {
        self.tick_block(now, inputs, output, wet, talkback, false)
    }
    fn tick_block(
        &mut self,
        now: u64,
        inputs: &[f64],
        output: &mut [f64],
        wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
        capture: bool,
    ) -> Result<()> {
        // Preserve the receipt time of this borrowed source block across control
        // work. This is not an ALSA converter acquisition timestamp.
        let source_received = crate::analysis_stream::local::monotonic_ms();
        let meter_mono = source_received.as_ref().ok().copied();
        if now < self.last_now {
            return Err("clock".into());
        }
        self.last_now = now;
        self.poll_measurement(now);
        #[cfg(feature = "hardware-host")]
        self.poll_fx_aux(now)?;
        self.ticks = self.ticks.checked_add(1).ok_or("tick exhausted")?;
        for _ in 0..MAX_CLIENTS {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.clients.len() >= MAX_CLIENTS || !same_uid(&stream).unwrap_or(false) {
                        continue;
                    }
                    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
                    let id = self.next_client;
                    self.next_client = self.next_client.checked_add(1).ok_or("client exhausted")?;
                    self.clients.push(Client {
                        id,
                        stream,
                        buffer: Box::new([0; MAX_FRAME + 4]),
                        used: 0,
                        started: None,
                        replies: VecDeque::with_capacity(MAX_REPLIES),
                        telemetry: None,
                        meter_deadline: None,
                        meter_last_query: None,
                        last_write: now,
                        snapshot: false,
                        sends_snapshot_ms: None,
                        processing_snapshot_ms: None,
                        structural_snapshot_ms: None,
                        master_eq_snapshot_ms: None,
                        measurement_snapshot_ms: None,
                        fx_aux_snapshot_ms: None,
                        brain_snapshot_ms: None,
                        writer: None,
                        lease: None,
                    });
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        let mut index = 0;
        while index < self.clients.len() {
            let result = self.clients[index].receive(now, self.engine.processing_version());
            let mut keep = true;
            match result {
                Err(_) => keep = false,
                Ok(None) => (),
                Ok(Some(Incoming::FxAux(value))) => {
                    let handled = (|| -> Result<()> {
                        let (request, mutation) =
                            crate::fx_aux_wire::request(&value, self.engine.wire_version())?;
                        let c = &self.clients[index];
                        let fresh = c
                            .fx_aux_snapshot_ms
                            .is_some_and(|t| now.saturating_sub(t) <= 1000);
                        if mutation.is_some()
                            && (!c.snapshot
                                || c.writer != request.writer
                                || c.lease != request.lease)
                        {
                            return Err("FX aux connection authority".into());
                        }
                        let read = mutation.is_none();
                        let reply = self.fx_aux_request(value, now, fresh, Some(c.id))?;
                        if read {
                            self.clients[index].fx_aux_snapshot_ms = Some(now);
                        }
                        self.clients[index].queue_module(&reply, now)
                    })();
                    if handled.is_err() {
                        keep = false;
                    }
                }
                Ok(Some(Incoming::Spectrum(request))) => {
                    if !self.clients[index].replies.is_empty()
                        || self.clients[index]
                            .meter_last_query
                            .is_some_and(|t| now.saturating_sub(t) < 80)
                    {
                        keep = false;
                    } else {
                        self.clients[index].meter_last_query = Some(now);
                        self.clients[index].meter_deadline = now.checked_add(100);
                        match self.spectrum_snapshot(&request) {
                            Ok(reply) => {
                                if self.clients[index].queue_module(&reply, now).is_err() {
                                    keep = false;
                                }
                            }
                            Err(_) => keep = false,
                        }
                    }
                }
                Ok(Some(Incoming::Meter(request))) => {
                    if !self.clients[index].replies.is_empty()
                        || self.clients[index].telemetry.is_some()
                        || self.clients[index]
                            .meter_last_query
                            .is_some_and(|t| now.saturating_sub(t) < 40)
                    {
                        keep = false;
                    } else {
                        self.clients[index].meter_last_query = Some(now);
                        self.clients[index].meter_deadline = now.checked_add(100);
                        match self.meter_snapshot(&request, 1) {
                            Ok(reply) => {
                                if self.clients[index].queue_module(&reply, now).is_err() {
                                    keep = false;
                                }
                            }
                            Err(_) => keep = false,
                        }
                    }
                }
                Ok(Some(Incoming::UnsupportedProcessing(request))) => {
                    let reply = request.refusal(self.engine.revision());
                    if self.clients[index].queue_module(&reply, now).is_err() {
                        keep = false;
                    }
                }
                Ok(Some(Incoming::Brain(request))) => {
                    let read = matches!(
                        request.command,
                        crate::brain_control::Command::BrainSnapshot {}
                    );
                    let c = &self.clients[index];
                    let authorized = read
                        || (c.snapshot && c.writer == request.writer && c.lease == request.lease);
                    let fresh = c
                        .brain_snapshot_ms
                        .is_some_and(|t| now.saturating_sub(t) <= 250);
                    if !authorized {
                        keep = false;
                    } else {
                        let owner = Some(c.id);
                        let reply = self.brain_request(request, now, fresh, owner)?;
                        if read && reply.snapshot.is_some() {
                            self.clients[index].brain_snapshot_ms = Some(now);
                        }
                        if self.clients[index].queue_module(&reply, now).is_err() {
                            keep = false;
                        }
                    }
                }
                Ok(Some(Incoming::Measurement(request))) => {
                    let read = request.read();
                    let c = &self.clients[index];
                    let authorized = read
                        || (c.snapshot && c.writer == request.writer && c.lease == request.lease);
                    let fresh = c
                        .measurement_snapshot_ms
                        .is_some_and(|t| now >= t && now - t <= 250);
                    if !authorized {
                        keep = false;
                    } else {
                        let owner = Some(c.id);
                        let snapshot = matches!(
                            request.command,
                            crate::measurement_wire::Command::MeasurementSnapshot {}
                        );
                        let reply = self.measurement_request(request, now, fresh, owner)?;
                        if snapshot && reply.snapshot.is_some() {
                            self.clients[index].measurement_snapshot_ms = Some(now);
                        }
                        if self.clients[index].queue_module(&reply, now).is_err() {
                            keep = false;
                        }
                    }
                }
                Ok(Some(Incoming::Structural(request))) => {
                    let eq = request.contract == crate::master_eq_wire::CONTRACT;
                    let read = matches!(
                        request.command,
                        crate::structural_control::Command::StructuralSnapshot {}
                            | crate::structural_control::Command::MasterEqSnapshot {}
                    );
                    let c = &self.clients[index];
                    let authorized = read
                        || (c.snapshot && c.writer == request.writer && c.lease == request.lease);
                    let fresh = (if eq {
                        c.master_eq_snapshot_ms
                    } else {
                        c.structural_snapshot_ms
                    })
                    .is_some_and(|t| now >= t && now - t <= 250);
                    if !authorized {
                        keep = false;
                    } else {
                        let owner = Some(c.id);
                        let reply = self.structural_request(request, now, fresh, owner)?;
                        if read {
                            if eq && reply.master_eq.is_some() {
                                self.clients[index].master_eq_snapshot_ms = Some(now);
                            } else if reply.snapshot.is_some() {
                                self.clients[index].structural_snapshot_ms = Some(now);
                            }
                        }
                        if self.clients[index].queue_module(&reply, now).is_err() {
                            keep = false;
                        }
                    }
                }
                Ok(Some(Incoming::Processing(request))) => {
                    if self.processing_request(index, request, now).is_err() {
                        keep = false;
                    }
                }
                Ok(Some(Incoming::Sends(request))) => {
                    if self.sends_request(index, request, now).is_err() {
                        keep = false;
                    }
                }
                Ok(Some(Incoming::Modules(request))) => {
                    if self.module_request(index, request, now).is_err() {
                        keep = false;
                    }
                }
                Ok(Some(Incoming::Audio(request))) => {
                    let client = &mut self.clients[index];
                    let read = matches!(request.command, Command::Snapshot {});
                    let grant = matches!(request.command, Command::Grant { .. });
                    if !read
                        && (!client.snapshot
                            || (!grant
                                && (client.writer != request.writer
                                    || client.lease != request.lease))
                            || (grant
                                && request
                                    .writer
                                    .as_ref()
                                    .and_then(|w| self.writer_connections.get(w))
                                    .is_some_and(|owner| *owner != client.id)))
                    {
                        keep = false;
                    } else {
                        let reply = self.engine.handle(&request, now)?;
                        if read && reply.outcome.as_ref().is_some_and(|p| p.kind == "applied") {
                            client.snapshot = true;
                        }
                        if grant
                            && let Some(lease) =
                                reply.outcome.as_ref().and_then(|p| p.body.granted_lease)
                        {
                            client.writer = request.writer.clone();
                            client.lease = Some(lease);
                            self.writer_connections
                                .insert(request.writer.clone().unwrap(), client.id);
                        }
                        if reply.state == "pending" {
                            self.pending_owner = Some((reply.ticket.unwrap().0, client.id));
                        }
                        if client.queue(&reply, now).is_err() {
                            keep = false;
                        }
                    }
                }
            }
            if keep {
                index += 1;
            } else {
                let removed = self.clients.remove(index);
                if let Some(writer) = removed.writer {
                    self.revoke_writer(&writer);
                }
            }
        }
        #[cfg(feature = "hardware-host")]
        self.module_completions(now)?;
        self.commit_brain(now)?;
        self.commit_structure(now)?;
        self.commit_measurement(now)?;
        // Basis/authority checks are controller work. Invalidation closes the
        // source capture independently, including a just-queued start.
        self.validate_measurement(now);
        let source_frame = self.frame();
        let ni = if capture {
            self.topology().capture_channels
        } else {
            self.topology().inputs.len()
        };
        if let Some(latest) = &mut self.metering {
            latest.drain();
        }
        output.fill(0.);
        let end_frame = source_frame
            .checked_add((inputs.len() / ni) as u64)
            .ok_or("clock exhausted")?;
        // Preserve the historical completed-quantum deadline/safety check while
        // keeping authority/lease destruction outside the numeric DSP boundary.
        self.prepare_brain_authority(now, end_frame);
        self.ensure_render_graph()?;
        if self.source_executor.is_some() {
            let (_, admission) = self.graph_command((source_frame, None), |graph, packet| {
                packet.1 = Some(graph.mixer.reserve_admission(packet.0));
            })?;
            let admission = admission
                .expect("source admission reply")
                .map_err(|error| format!("source admission {error:?}"))?;
            if admission.epoch() != self.epoch.0 || admission.ticket() == 0 {
                return Err("source admission identity".into());
            }
            self.engine.begin_source_admission(admission)?;
        }
        let prepared = self.engine.prepare_render(inputs.len() / ni, now);
        self.engine.end_source_admission();
        if let Err(error) = prepared {
            self.finish_compat_render()?;
            return Err(error);
        }
        let Some(command) = self.engine.take_source_command() else {
            self.finish_compat_render()?;
            self.quiesce_source("source_command_missing")?;
            return Err("source command missing".into());
        };
        let mut quantum = self.source_quantum.take().unwrap_or_else(|| {
            Box::new(source::Quantum::new(
                self.topology(),
                self.engine.source_completion(),
            ))
        });
        quantum.epoch = self.epoch.0;
        quantum.frame = source_frame;
        quantum.capture = capture;
        quantum.received = meter_mono;
        quantum.input.clear();
        quantum.input.extend_from_slice(inputs);
        quantum.output.resize(output.len(), 0.);
        quantum.output.fill(0.);
        quantum.wet.clear();
        quantum.wet.extend_from_slice(wet.unwrap_or(&[]));
        quantum.talkback.clear();
        quantum.talkback.extend_from_slice(talkback.unwrap_or(&[]));
        quantum.monitor = self.brain.source;
        quantum.deadline = talkback.and(self.brain.hold.as_ref().map(|h| h.3));
        quantum.monitors.clear();
        quantum.monitors.extend_from_slice(&self.brain.monitors);
        quantum.foh = self.brain.foh && self.topology().pa_outputs > 0;
        quantum.gain = 10_f64.powf(self.brain.gain_cdb as f64 / 2000.);
        #[cfg(feature = "hardware-host")]
        {
            quantum.remote_fx_aux = self.remote_fx_aux;
            // Structural completion above has already installed the new PA
            // context. Never prepare this exchange before that completion.
            quantum.modules = self
                .modules
                .as_mut()
                .map(|g| g.prepare_exchange())
                .transpose()?;
        }
        quantum.command = Some(command);
        std::mem::swap(&mut quantum.completion, &mut self.source_completion);
        std::mem::swap(&mut quantum.routing, &mut self.routing_observed);
        let mut quantum = if let Some(executor) = &self.source_executor {
            executor.transact(quantum, |graph, packet| {
                source::Quantum::execute(graph, packet)
            })?
        } else {
            source::Quantum::execute(
                self.render_graph.as_mut().expect("source graph"),
                &mut quantum,
            );
            quantum
        };
        output.copy_from_slice(&quantum.output);
        std::mem::swap(&mut quantum.completion, &mut self.source_completion);
        std::mem::swap(&mut quantum.routing, &mut self.routing_observed);
        let result = quantum.result.take().expect("source quantum result");
        // Retired prepared command storage is destroyed only on this controller.
        let completion = if quantum.refused {
            self.engine
                .reject_source_command(quantum.command.as_ref().expect("refused command"))
        } else {
            self.engine
                .finish_source_completion(&mut self.source_completion)
        };
        quantum.command = None;
        #[cfg(feature = "hardware-host")]
        let module_completion = match quantum.modules.take() {
            Some(packet) => {
                let modules = self.modules.as_mut().expect("module controller");
                if quantum.refused {
                    modules.cancel_exchange(packet)
                } else {
                    modules.finish_exchange(packet)
                }
            }
            None => Ok(()),
        };
        let command_refused = quantum.refused;
        self.source_quantum = Some(quantum);
        self.finish_compat_render()?;
        #[cfg(feature = "hardware-host")]
        module_completion?;
        if command_refused {
            completion?;
            return Err("source command refused before rendering".into());
        }
        #[cfg(feature = "hardware-host")]
        self.observe_fx_aux()?;
        let peaks = match result {
            Ok(peaks) => peaks,
            Err(source::RenderError::Mixer) => return completion.map(|_| ()),
            Err(source::RenderError::Closed | source::RenderError::Mapping) => {
                self.quiesce_source("source_render_refused")?;
                return Err("source render refused".into());
            }
            #[cfg(feature = "hardware-host")]
            Err(source::RenderError::Module(error)) => {
                let _ = completion;
                self.quiesce_source("module_processing_failure")?;
                return Err(format!("module processing {error:?}"));
            }
        };
        let rendered_frame = self.frame();
        debug_assert_eq!(
            rendered_frame,
            source_frame + inputs.len() as u64 / ni as u64
        );
        self.brain_frame_times[(rendered_frame / 48 % 64) as usize] = Some((rendered_frame, now));
        if !self.brain_media_authorized(now) {
            self.brain.close();
        }
        self.brain.microphone_peak = peaks.microphone;
        self.brain.outgoing_peak = peaks.outgoing;
        self.brain.monitor_peak = peaks.monitor;
        let completions = completion?;
        for reply in completions {
            if self.pending_owner.is_none() && self.remote_completions.len() < 64 {
                self.remote_completions
                    .push_back(serde_json::to_value(&reply).map_err(|e| e.to_string())?);
            }
            if let Some((ticket, owner)) = self.pending_owner.take()
                && reply.ticket.is_none_or(|t| t.0 == ticket)
                && let Some(index) = self.clients.iter().position(|c| c.id == owner)
                && self.clients[index].queue(&reply, now).is_err()
            {
                self.clients.remove(index);
            }
        }
        for reply in self.engine.take_processing_completions() {
            if self.processing_owner.is_none() && self.remote_completions.len() < 64 {
                self.remote_completions
                    .push_back(serde_json::to_value(&reply).map_err(|e| e.to_string())?);
            }
            if let Some((ticket, owner)) = self.processing_owner.take()
                && reply.ticket.is_none_or(|t| t.0 == ticket)
                && let Some(index) = self.clients.iter().position(|c| c.id == owner)
                && self.clients[index].queue_module(&reply, now).is_err()
            {
                self.clients.remove(index);
            }
        }
        for reply in self.engine.take_sends_completions() {
            if self.sends_owner.is_none() && self.remote_completions.len() < 64 {
                self.remote_completions
                    .push_back(serde_json::to_value(&reply).map_err(|e| e.to_string())?);
            }
            if let Some((ticket, owner)) = self.sends_owner.take()
                && reply.ticket.is_none_or(|t| t.0 == ticket)
                && let Some(index) = self.clients.iter().position(|c| c.id == owner)
                && self.clients[index].queue_module(&reply, now).is_err()
            {
                let removed = self.clients.remove(index);
                if let Some(writer) = removed.writer {
                    self.revoke_writer(&writer);
                }
            }
        }
        // Latest-only telemetry never replaces a partially transmitted frame.
        if self.ticks.is_multiple_of(16) {
            for client in &mut self.clients {
                if client.snapshot && !client.telemetry.as_ref().is_some_and(|p| p.offset > 0) {
                    let request = Request {
                        contract: "C-AUDIO".into(),
                        version: self.engine.wire_version(),
                        show_id: self.show.clone(),
                        module: "audio".into(),
                        epoch: self.epoch,
                        writer: None,
                        lease: None,
                        request_id: None,
                        expected_revision: None,
                        command: Command::Snapshot {},
                    };
                    let reply = self.engine.handle(&request, now)?;
                    if client.replies.is_empty() && client.telemetry.is_none() {
                        client.last_write = now;
                    }
                    let bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
                    if bytes.len() <= MAX_FRAME {
                        client.telemetry = Some(Packet::new(bytes)?);
                    } // expanded clients request coherent pages; never partial periodic snapshots
                }
            }
        }
        let mut index = 0;
        while index < self.clients.len() {
            if self.clients[index].flush(now).is_ok() {
                index += 1;
            } else {
                let c = self.clients.remove(index);
                if let Some(writer) = c.writer {
                    self.revoke_writer(&writer);
                }
            }
        }
        Ok(())
    }
}

// Generate each containing legacy block once per provider tick, including a
// possible second block after unaligned source recovery. Extra logical inputs
// retain their original independent PCM24 mapping. No source bytes change.
struct SyntheticBlock {
    first: u64,
    pcm: [[i32; 8]; 48],
}
impl SyntheticBlock {
    fn new(frame: u64) -> Self {
        let first = frame - frame % 48;
        Self {
            first,
            pcm: crate::analysis_stream::synthetic_inputs(first),
        }
    }
    fn sample(&mut self, frame: u64, input: usize) -> f64 {
        if input >= 8 {
            return (((frame as i64 * 17 + input as i64 * 7919) % 65537) - 32768) as f64
                / 8_388_608.;
        }
        let first = frame - frame % 48;
        if first != self.first {
            *self = Self::new(frame);
        }
        f64::from(self.pcm[(frame % 48) as usize][input]) / 8_388_608.
    }
}

#[cfg(test)]
mod synthetic_source_tests {
    use super::*;
    // Independent scalar reference, rather than calling the block generator.
    fn reference(frame: u64, input: usize) -> f64 {
        let pcm = if input < 8 {
            ((frame % 4096) as i32 - 2048) * (input as i32 + 1) * 32
        } else {
            (((frame as i64 * 17 + input as i64 * 7919) % 65537) - 32768) as i32
        };
        f64::from(pcm) / 8_388_608.
    }
    #[test]
    fn exact_raw_source_across_channels_blocks_and_capture_permutation() {
        for first in [0, 1, 47, 48, 49, 4080, 4095, 4096, 65520] {
            let mut source = SyntheticBlock::new(first);
            for frame in first..first + 96 {
                for input in 0..48 {
                    assert_eq!(
                        source.sample(frame, input).to_bits(),
                        reference(frame, input).to_bits(),
                        "frame={frame} input={input}"
                    );
                }
            }
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gp04-raw-equivalence-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut topology = crate::topology::EngineTopology::software(16, 3, 0).unwrap();
        for (i, port) in topology.inputs.iter_mut().enumerate() {
            port.capture_slot = 15 - i;
        }
        let mut audio = LocalAudio::bind_configured(
            &dir,
            "audio.sock",
            "11111111-1111-4111-8111-111111111111",
            Counter(9),
            topology,
        )
        .unwrap();
        audio.enable_synthetic_fouraux().unwrap();
        for tick in 0..88 {
            let first = audio.frame();
            audio.tick(tick).unwrap();
            for f in 0..48 {
                for input in 0..16 {
                    let expected = reference(first + f as u64, input);
                    assert_eq!(
                        audio.source.raw_scratch[f * 16 + input].to_bits(),
                        expected.to_bits()
                    );
                    assert_eq!(
                        audio.source.capture_scratch[f * 16 + 15 - input].to_bits(),
                        expected.to_bits()
                    );
                }
            }
        }
        drop(audio);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod held_proof_boundary_tests {
    use super::*;
    use crate::{
        brain_control,
        control_model::{Command, Request, Scope},
    };
    #[test]
    fn held_proof_destination_hash_is_prepared_not_recomputed_at_commit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("held-proof-boundary-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        let mut h = LocalAudio::bind_configured(
            &dir,
            "audio",
            show,
            Counter(1),
            crate::topology::EngineTopology::software(16, 5, 0).unwrap(),
        )
        .unwrap();
        let grant = Request {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some("desk".into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant {
                scope: Scope::TalkbackDestinations,
            },
        };
        let lease = h
            .engine_mut()
            .handle(&grant, 0)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap();
        let request = brain_control::Request {
            contract: "GP15-brain".into(),
            version: 1,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some("desk".into()),
            lease: Some(lease),
            request_id: Some(Counter(2)),
            expected_revision: Some(h.engine_mut().revision()),
            command: brain_control::Command::TalkbackSet {
                monitors: vec![4, 0],
                gain_cdb: -1200,
                mute: false,
            },
        };
        let expected = crate::held_proof::destination_hash(&[0, 4]).unwrap();
        crate::held_proof::HASH_CALLS.with(|n| n.set(0));
        assert_eq!(
            h.brain_request(request, 1, true, None).unwrap().state,
            "pending"
        );
        assert_eq!(crate::held_proof::HASH_CALLS.with(|n| n.get()), 1);
        crate::held_proof::HASH_CALLS.with(|n| n.set(0));
        h.tick(1).unwrap();
        h.tick(2).unwrap();
        assert_eq!(crate::held_proof::HASH_CALLS.with(|n| n.get()), 0);
        assert_eq!(h.brain.destination_hash, expected);
        let restored = brain_control::State::from_intent(&h.brain.intent());
        assert_eq!(restored.destination_hash, expected);
        drop(h);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(all(test, feature = "hardware-host"))]
mod clocked_control_tests {
    use super::*;
    use crate::control_model::{Command, Request, Scope};
    use crate::remote::HostAuthority;
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    const SHOW: &str = "11111111-1111-4111-8111-111111111111";
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn provider(name: &str, outputs: usize) -> (Directory, LocalAudio) {
        let dir =
            std::env::temp_dir().join(format!("gp-h1-clock-control-{name}-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let provider = LocalAudio::bind_configured(
            &dir,
            "audio.sock",
            SHOW,
            Counter(9),
            crate::topology::EngineTopology::software(8, 2, outputs).unwrap(),
        )
        .unwrap();
        (Directory(dir), provider)
    }
    fn grant(provider: &mut LocalAudio, writer: &str, scope: Scope) -> Counter {
        let revision = provider.engine().revision();
        provider
            .engine_mut()
            .handle(
                &Request {
                    contract: "C-AUDIO".into(),
                    version: 2,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some(writer.into()),
                    lease: None,
                    request_id: Some(Counter(1)),
                    expected_revision: Some(revision),
                    command: Command::Grant { scope },
                },
                0,
            )
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap()
    }
    fn structure(
        lease: Counter,
        id: u64,
        revision: Counter,
        command: crate::structural_control::Command,
    ) -> crate::structural_control::Request {
        crate::structural_control::Request {
            contract: "GP14-structure".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some("structure".into()),
            lease: Some(lease),
            request_id: Some(Counter(id)),
            expected_revision: Some(revision),
            command,
        }
    }
    fn wait_blocks(status: &crate::remote::SourceClockStatus, blocks: u64) {
        let until = Instant::now() + Duration::from_secs(3);
        while status.blocks() < blocks {
            assert!(
                Instant::now() < until && !status.stopped(),
                "source stopped before requested frame"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn clocked_structure_refuses_late_expired_and_disconnected_preparation() {
        for mode in 0..4 {
            let (_directory, mut provider) = provider(&format!("structure-{mode}"), 0);
            let lease = grant(&mut provider, "structure", Scope::PaConfiguration);
            provider.rearm().unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let now = if mode == 2 {
                crate::control_model::LEASE_MS - 5
            } else {
                1
            };
            let status = host
                .start_source_clock(now, Duration::from_millis(2), 200)
                .unwrap();
            wait_blocks(&status, 1);
            // Startup charges preparation time. Reusing the requested origin
            // can move authority time backwards before the tested preparation.
            let now = host.provider().source_control_time();
            let certificate = b"clocked-structure-fence";
            use sha2::{Digest, Sha256};
            let policy = crate::remote::PolicyStore::new(vec![crate::remote::Peer {
                id: "clock-test".into(),
                certificate_sha256: format!("{:x}", Sha256::digest(certificate)),
                permissions: [crate::remote::Permission::PaConfiguration]
                    .into_iter()
                    .collect(),
            }])
            .unwrap();
            let context = policy.authenticate(certificate, 1).unwrap();
            let request = structure(
                lease,
                2,
                Counter(0),
                crate::structural_control::Command::OutputMute {},
            );
            let reply = host
                .provider_mut()
                .clocked_control(now, Some(context.audio_fence()), |provider| {
                    let reply = provider.structural_request(request, now, true, None)?;
                    if mode == 1 {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    if mode == 3 {
                        context.close_audio();
                    }
                    serde_json::to_value(reply).map_err(|e| e.to_string())
                })
                .unwrap();
            assert_eq!(reply["state"], "pending", "mode={mode}: {reply}");
            let replies = host.provider_mut().take_remote_completions();
            assert_eq!(replies.len(), 1, "mode={mode}: {replies:?}");
            assert_eq!(
                host.provider().engine().revision(),
                Counter(u64::from(mode == 0))
            );
            assert_eq!(
                replies[0]["reason"].is_null(),
                mode == 0,
                "mode={mode}: {:?}",
                replies[0]
            );
            // Failed preparation cannot strand the global reservation.
            assert!(host.provider().engine().external_boundary().is_none());
            status.close();
            host.finish_source_clock(&status).unwrap();
        }
    }
    #[test]
    fn clocked_brain_selection_installs_routing_at_source_boundary() {
        let (_directory, mut provider) = provider("brain", 0);
        let lease = grant(&mut provider, "listen", Scope::LocalOperatorMonitor);
        provider.rearm().unwrap();
        let mut host = HostAuthority::new(provider, 1).unwrap();
        let status = host
            .start_source_clock(1, Duration::from_millis(2), 100)
            .unwrap();
        wait_blocks(&status, 1);
        let request = crate::brain_control::Request {
            contract: "GP15-brain".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some("listen".into()),
            lease: Some(lease),
            request_id: Some(Counter(2)),
            expected_revision: Some(Counter(0)),
            command: crate::brain_control::Command::MonitorSet {
                source: crate::brain_control::MonitorSource::Pfl { input: 3 },
                gain_cdb: -600,
                mute: false,
                dim: false,
                armed: false,
            },
        };
        host.provider_mut()
            .clocked_control(2, None, |provider| {
                serde_json::to_value(provider.brain_request(request, 2, true, None)?)
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(host.provider().engine().revision(), Counter(1));
        let source = host
            .provider_mut()
            .graph_command(None, |graph, p| {
                *p = graph.clock_routing.as_ref().map(|routing| routing.monitor);
            })
            .unwrap();
        assert_eq!(
            source,
            Some(crate::brain_control::MonitorSource::Pfl { input: 3 })
        );
        host.provider_mut().close_brain_audio();
        let closed = host
            .provider_mut()
            .graph_command(false, |graph, p| {
                *p = graph.clock_routing.as_ref().is_some_and(|r| {
                    r.monitor == crate::brain_control::MonitorSource::None
                        && r.deadline_ms.is_none()
                        && r.deadline_frame.is_none()
                });
            })
            .unwrap();
        assert!(closed);
        status.close();
        host.finish_source_clock(&status).unwrap();
    }
    #[test]
    fn clocked_authority_release_and_writer_revocation_close_copied_hold() {
        for mode in 0..3 {
            let release = mode == 1;
            let (_directory, mut provider) = provider(&format!("hold-{mode}"), 0);
            let lease = grant(&mut provider, "talk", Scope::TalkbackDestinations);
            let listen_lease = grant(&mut provider, "listen", Scope::LocalOperatorMonitor);
            use sha2::{Digest, Sha256};
            let certificate = b"held-controller-session";
            let policy = crate::remote::PolicyStore::new(vec![crate::remote::Peer {
                id: "held-controller".into(),
                certificate_sha256: format!("{:x}", Sha256::digest(certificate)),
                permissions: [crate::remote::Permission::TalkbackDestinations]
                    .into_iter()
                    .collect(),
            }])
            .unwrap();
            let context = policy.authenticate(certificate, 1).unwrap();
            provider.rearm().unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let status = host
                .start_source_clock(1, Duration::from_millis(2), 200)
                .unwrap();
            wait_blocks(&status, 8);
            for (id, command) in [
                (
                    2,
                    crate::brain_control::Command::TalkbackSet {
                        monitors: vec![0],
                        gain_cdb: -1200,
                        mute: false,
                    },
                ),
                (
                    3,
                    crate::brain_control::Command::Hold {
                        generation: Counter(1),
                    },
                ),
            ] {
                let request = crate::brain_control::Request {
                    contract: "GP15-brain".into(),
                    version: 1,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some("talk".into()),
                    lease: Some(lease),
                    request_id: Some(Counter(id)),
                    expected_revision: Some(host.provider().engine().revision()),
                    command,
                };
                host.provider_mut().last_brain_source_receipt = None;
                let reply = host
                    .provider_mut()
                    .clocked_control(id, Some(context.audio_fence()), |provider| {
                        serde_json::to_value(provider.brain_request(request, id, true, None)?)
                            .map_err(|e| e.to_string())
                    })
                    .unwrap();
                assert_eq!(
                    host.provider().engine().revision(),
                    Counter(id - 1),
                    "mode={mode}, request={id}, reply={reply}, source_receipt={:?}",
                    host.provider().last_brain_source_receipt
                );
            }
            let held = host
                .provider_mut()
                .graph_command(false, |graph, p| {
                    *p = graph
                        .clock_routing
                        .as_ref()
                        .is_some_and(|r| r.deadline_ms.is_some());
                })
                .unwrap();
            assert!(held);
            if mode == 2 {
                // A separate writer changing monitor intent must preserve the
                // talkback controller's independent authenticated closure signal.
                let request = crate::brain_control::Request {
                    contract: "GP15-brain".into(),
                    version: 1,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some("listen".into()),
                    lease: Some(listen_lease),
                    request_id: Some(Counter(2)),
                    expected_revision: Some(host.provider().engine().revision()),
                    command: crate::brain_control::Command::MonitorSet {
                        source: crate::brain_control::MonitorSource::Main,
                        gain_cdb: -1200,
                        mute: true,
                        dim: false,
                        armed: false,
                    },
                };
                host.provider_mut()
                    .clocked_control(4, None, |provider| {
                        serde_json::to_value(provider.brain_request(request, 4, true, None)?)
                            .map_err(|e| e.to_string())
                    })
                    .unwrap();
                context.close_audio();
                // No controller service/readback: source sees closure itself.
                wait_blocks(&status, status.blocks() + 2);
                let fenced = host
                    .provider_mut()
                    .graph_command(false, |graph, p| {
                        *p = graph
                            .clock_routing
                            .as_ref()
                            .is_some_and(|r| r.hold_fence.as_ref().is_some_and(|f| !f.valid()));
                    })
                    .unwrap();
                assert!(fenced);
                status.close();
                host.finish_source_clock(&status).unwrap();
                continue;
            }
            if release {
                let request = Request {
                    contract: "C-AUDIO".into(),
                    version: 2,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some("talk".into()),
                    lease: Some(lease),
                    request_id: Some(Counter(4)),
                    expected_revision: Some(host.provider().engine().revision()),
                    command: Command::Release {},
                };
                host.provider_mut()
                    .clocked_engine_command(4, None, |engine| {
                        serde_json::to_value(engine.handle(&request, 4)?).map_err(|e| e.to_string())
                    })
                    .unwrap();
            } else {
                host.provider_mut().revoke_writer("talk");
            }
            let closed = host
                .provider_mut()
                .graph_command(false, |graph, p| {
                    *p = graph
                        .clock_routing
                        .as_ref()
                        .is_some_and(|r| r.deadline_ms.is_none() && r.deadline_frame.is_none());
                })
                .unwrap();
            assert!(closed, "release={release}");
            status.close();
            host.finish_source_clock(&status).unwrap();
        }
    }
    #[test]
    #[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
    fn clocked_pa_replacement_completes_before_next_context_exchange() {
        let (_directory, mut provider) = provider("pa-context", 8);
        provider
            .enable_modules(std::path::Path::new(
                &std::env::var_os("GP05_MANIFEST").unwrap(),
            ))
            .unwrap();
        let config = std::fs::read_to_string(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap();
        provider.configure_pa(config.as_bytes()).unwrap();
        let lease = grant(&mut provider, "structure", Scope::PaConfiguration);
        provider.rearm().unwrap();
        let mut host = HostAuthority::new(provider, 1).unwrap();
        let started = Instant::now();
        let current_time = || 1 + started.elapsed().as_millis() as u64;
        let status = host
            .start_source_clock(1, Duration::from_millis(2), 200)
            .unwrap();
        wait_blocks(&status, 1);
        let mute = structure(
            lease,
            2,
            Counter(0),
            crate::structural_control::Command::OutputMute {},
        );
        let now = current_time();
        let reply = host
            .provider_mut()
            .clocked_control(now, None, |provider| {
                serde_json::to_value(provider.structural_request(mute, now, true, None)?)
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(reply["state"], "pending", "{reply}");
        let replies = host.provider_mut().take_remote_completions();
        assert_eq!(replies.len(), 1);
        assert!(replies[0]["reason"].is_null(), "{replies:?}");
        wait_blocks(&status, status.blocks() + 6);
        for id in 3..5 {
            let request = structure(
                lease,
                id,
                host.provider().engine().revision(),
                crate::structural_control::Command::PaSet {
                    configuration_json: config.clone(),
                    program_buses: vec![0, 1],
                },
            );
            let now = current_time();
            let reply = host
                .provider_mut()
                .clocked_control(now, None, |provider| {
                    serde_json::to_value(provider.structural_request(request, now, true, None)?)
                        .map_err(|e| e.to_string())
                })
                .unwrap();
            assert_eq!(reply["state"], "pending", "{reply}");
            let replies = host.provider_mut().take_remote_completions();
            assert_eq!(replies.len(), 1);
            assert!(replies[0]["reason"].is_null(), "{replies:?}");
            assert_eq!(host.provider().engine().revision(), Counter(id - 1));
            let applied: u64 = replies[0]["effective_frame"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            assert!(host.provider().frame() >= applied);
            // This exchange must use the new context installed by finish_pa_change.
            host.provider_mut()
                .service_source_clock(current_time())
                .unwrap();
            host.provider_mut().master_eq_snapshot().unwrap();
        }
        status.close();
        host.finish_source_clock(&status).unwrap();
    }
}
