//! Explicit-path fixed-owner offline graph. Control preparation/query/lifecycle
//! is separate from bounded processing; no device enumeration or host changes.
use crate::{
    host::adapters::{
        Dsp, FxCapabilities, FxStatus, PaDescriptor, PaStatus, Recorder, RecorderObserver,
        RecorderProgress,
    },
    show::Counter,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;
pub const FRAMES: usize = 48;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub library: PathBuf,
    pub library_sha256: String,
    pub header: PathBuf,
    pub header_sha256: String,
    pub owner_manifest: PathBuf,
    pub owner_manifest_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub rec: Artifact,
    pub fx: Artifact,
    pub pa: Artifact,
}
fn hash(path: &Path, maximum: u64) -> Result<String> {
    use std::io::Read;
    if !path.is_absolute() {
        return Err("artifact requires absolute path".into());
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("artifact must be a regular file".into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > maximum {
            return Err("artifact size bound".into());
        }
        hasher.update(&buffer[..n]);
    }
    Ok(crate::hash::hex(hasher.finalize()))
}
impl Artifact {
    fn verify(&self) -> Result<()> {
        for (path, expected, max) in [
            (&self.library, &self.library_sha256, 64 * 1024 * 1024),
            (&self.header, &self.header_sha256, 65536),
            (&self.owner_manifest, &self.owner_manifest_sha256, 65536),
        ] {
            if expected.len() != 64
                || !expected
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || hash(path, max)? != *expected
            {
                return Err("artifact hash mismatch".into());
            }
        }
        // Exact accepted owner manifest must itself bind both ABI and executable.
        let bytes = fs::read(&self.owner_manifest).map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let files = value
            .get("files")
            .and_then(|v| v.as_object())
            .ok_or("owner manifest files")?;
        for expected in [&self.library_sha256, &self.header_sha256] {
            if !files.values().any(|v| v.as_str() == Some(expected)) {
                return Err("artifact absent from owner manifest".into());
            }
        }
        Ok(())
    }
}
impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > 65536 {
            return Err("manifest capacity".into());
        }
        let m: Self = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        m.verify()?;
        Ok(m)
    }
    pub fn verify(&self) -> Result<()> {
        if self.version != 1 {
            return Err("module manifest version".into());
        }
        self.rec.verify()?;
        self.fx.verify()?;
        self.pa.verify()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingStatus {
    pub take_id: String,
    pub operation_id: Counter,
    pub source_epoch: Counter,
    pub first_source_frame: Option<Counter>,
    pub mapping: Vec<String>,
    pub state: String,
    pub outcome: String,
    pub accepted_frames: Counter,
    pub written_frames: Counter,
    pub durable_frames: Option<Counter>,
    pub dropped_frames: Counter,
    pub overflow_blocks: Counter,
    pub gap_frames: Counter,
    pub invalid_blocks: Counter,
    pub clipped_samples: Counter,
    pub writer_fault: bool,
    pub host_fault: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub sample_rate: u32,
    pub block_frames: u32,
    pub raw_inputs: Vec<String>,
    pub fx_mix: String,
    pub pa_inputs: u32,
    pub pa_outputs: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FxHealth {
    pub capabilities: FxCapabilities,
    pub status: FxStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaHealth {
    pub descriptor: PaDescriptor,
    pub status: PaStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaHealthV2 {
    pub capabilities: crate::host::pa_v2::Capabilities,
    pub status: crate::host::pa_v2::Status,
    pub configuration_json: String,
    pub program_buses: Vec<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryIdentity {
    pub sha256: String,
    pub state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleStatus {
    pub contract: String,
    pub version: u32,
    pub show_id: String,
    pub module: String,
    pub epoch: Counter,
    pub frame: Counter,
    pub available: bool,
    pub physical: String,
    pub protection: String,
    pub readiness: String,
    pub activity: String,
    pub source_fault: u32,
    pub configuration: Option<Configuration>,
    pub libraries: BTreeMap<String, LibraryIdentity>,
    pub fx: Option<FxHealth>,
    pub pa: Option<PaHealth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pa_v2: Option<PaHealthV2>,
    pub recording: Option<RecordingStatus>,
}
impl ModuleStatus {
    pub fn unavailable(show: &str, epoch: u64, frame: u64) -> Self {
        Self {
            contract: "GP05-modules".into(),
            version: 1,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(epoch),
            frame: Counter(frame),
            available: false,
            physical: "unverified".into(),
            protection: "offline-unprotected".into(),
            readiness: "unavailable".into(),
            activity: "idle".into(),
            source_fault: 0,
            configuration: None,
            libraries: BTreeMap::new(),
            fx: None,
            pa: None,
            pa_v2: None,
            recording: None,
        }
    }
}
struct Take {
    observer: Option<RecorderObserver>,
    status: RecordingStatus,
}
enum WorkerResult {
    Prepared(std::result::Result<(Recorder, RecorderObserver), String>),
    Finished(i32),
}
struct Lifecycle {
    receiver: std::sync::mpsc::Receiver<WorkerResult>,
    thread: std::thread::JoinHandle<()>,
    cancel_code: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    Timeline,
    Source,
    Fx(i32),
    Pa(i32),
}
#[derive(Clone)]
struct PaMetadata {
    configuration_json: String,
    program_buses: Vec<usize>,
}
pub(crate) struct PaStorage {
    input: Vec<f64>,
    output: Vec<f64>,
    configuration_json: String,
    program_buses: Vec<usize>,
    total_buses: usize,
}
/// Controller-owned transaction; retain this wrapper until outside rendering,
/// including on refusal. The owner allocation and buffers are prepared offRT.
pub struct GraphPaPrepared {
    owner: crate::host::pa_v2::Prepared,
    initial: Option<crate::host::pa_v2::Pa>,
    storage: Option<PaStorage>,
    expected_generation: u64,
    expected_epoch: u64,
    expected_library_sha256: String,
    preparation: Option<crate::host::pa_v2::PaPreparation>,
    observation: Option<PaEqObservation>,
    metadata: Option<PaMetadata>,
}
impl GraphPaPrepared {
    pub fn output_channels(&self) -> usize {
        self.storage.as_ref().map_or(0, |s| s.output.len() / FRAMES)
    }
}
/// Owner token and complete retained metadata, both prepared offRT.
pub struct GraphEqPrepared {
    owner: crate::host::pa_v2::PreparedEq,
    metadata: String,
    expected_map: Vec<usize>,
    accepted_metadata: String,
}
/// Exclusively owned PA readback/retirement packet. Allocate on the controller,
/// capture on the source, then return ownership before formatting or preparing.
pub(crate) struct PaEqObservation {
    context: crate::host::pa_v2::EqContext,
    retirement: crate::host::pa_v2::PreparedEq,
    indices: Option<[usize; 2]>,
    result: i32,
    pa_status: std::result::Result<crate::host::pa_v2::Status, i32>,
}
impl PaEqObservation {
    fn new(api: &crate::host::pa_v2::PaPreparation, map: &[usize]) -> Result<Self> {
        let indices = (|| {
            let mut indices = [0; 2];
            for (bus, index) in indices.iter_mut().enumerate() {
                let mut matches = map.iter().enumerate().filter(|(_, b)| **b == bus);
                *index = matches.next()?.0;
                if matches.next().is_some() {
                    return None;
                }
            }
            Some(indices)
        })();
        Ok(Self {
            context: api.create_eq_context().map_err(|e| e.to_string())?,
            retirement: api.eq_retirement_slot(),
            indices,
            result: -5,
            pa_status: Err(-5),
        })
    }
    fn status(&self) -> std::result::Result<crate::host::pa_v2::EqStatus, i32> {
        if self.result != 0 {
            return Err(self.result);
        }
        self.context.status()
    }
}
pub(crate) mod source;
use source::{ModuleObservation, ModuleSource};
/// The compatibility facade never shares active state with a detached owner.
/// Direct render methods require the source to have been restored first.
struct SourceSlot(Option<ModuleSource>);
impl std::ops::Deref for SourceSlot {
    type Target = ModuleSource;
    fn deref(&self) -> &ModuleSource {
        self.0.as_ref().expect("module source detached")
    }
}
impl std::ops::DerefMut for SourceSlot {
    fn deref_mut(&mut self) -> &mut ModuleSource {
        self.0.as_mut().expect("module source detached")
    }
}
/// Exclusively transferred bounded lifecycle/observation packet. REC handles and
/// native retirement allocations stay retained until controller consumption.
pub(crate) struct ModuleExchange {
    owner: std::sync::Arc<std::sync::atomic::AtomicU32>,
    epoch: u64,
    start: Option<Recorder>,
    stop: bool,
    retired_recorder: Option<Recorder>,
    activated_frame: Option<u64>,
    observation: ModuleObservation,
    pa: Option<PaEqObservation>,
    pa_retirement: Option<crate::host::pa_v2::Prepared>,
    retired_storage: Option<PaStorage>,
    serviced: bool,
    accepted: bool,
}

pub struct ModuleGraph {
    manifest: Manifest,
    take: Option<Take>,
    lifecycle: Option<Lifecycle>,
    last_poll: Option<u64>,
    source: SourceSlot,
    observed: ModuleObservation,
    fx_capabilities: FxCapabilities,
    pa_descriptor: PaDescriptor,
    pa_preparation: Option<crate::host::pa_v2::PaPreparation>,
    pa_observed: Option<PaEqObservation>,
    pa_metadata: Option<PaMetadata>,
    prepared_recorder: Option<Recorder>,
    returned_recorder: Option<Recorder>,
    recorder_stop: bool,
    exchange_pending: bool,
    closure: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl ModuleGraph {
    /// Exclusive transfer; the facade retains only controller state and snapshots.
    pub(crate) fn take_source(&mut self) -> Result<ModuleSource> {
        if self.exchange_pending {
            return Err("module exchange outstanding".into());
        }
        self.observe_pa();
        self.source
            .0
            .take()
            .ok_or("module source already detached".into())
    }
    pub(crate) fn restore_source(&mut self, source: ModuleSource) -> Result<()> {
        if self.source.0.is_some()
            || self.exchange_pending
            || source.epoch != self.observed.epoch
            || !std::sync::Arc::ptr_eq(&self.closure, &source.closure)
        {
            return Err("module source restore ownership/epoch".into());
        }
        self.source.0 = Some(source);
        self.observe_pa();
        Ok(())
    }
    /// Reserve exactly one return packet before publication. No source reference
    /// is needed; all native storage here is allocated by the controller.
    pub(crate) fn prepare_exchange(&mut self) -> Result<ModuleExchange> {
        if self.exchange_pending {
            return Err("module exchange outstanding".into());
        }
        let packet = ModuleExchange {
            owner: self.closure.clone(),
            epoch: self.observed.epoch,
            start: self.prepared_recorder.take(),
            stop: std::mem::take(&mut self.recorder_stop),
            retired_recorder: None,
            activated_frame: None,
            observation: self.observed.clone(),
            pa: self.pa_observed.take(),
            pa_retirement: self
                .pa_preparation
                .as_ref()
                .map(|api| api.retirement_slot()),
            retired_storage: None,
            serviced: false,
            accepted: false,
        };
        self.exchange_pending = true;
        Ok(packet)
    }
    /// Restore a packet refused before source service. No REC activation or PA
    /// context capture occurred, so its reserved storage remains reusable.
    pub(crate) fn cancel_exchange(&mut self, mut packet: ModuleExchange) -> Result<()> {
        if !self.exchange_pending
            || packet.serviced
            || packet.epoch != self.observed.epoch
            || !std::sync::Arc::ptr_eq(&self.closure, &packet.owner)
            || self.prepared_recorder.is_some()
            || self.pa_observed.is_some()
        {
            return Err("module exchange cancellation identity".into());
        }
        self.prepared_recorder = packet.start.take();
        self.recorder_stop |= packet.stop;
        self.pa_observed = packet.pa.take();
        self.exchange_pending = false;
        Ok(())
    }
    /// Consume only after exclusive source publication. Native destruction and
    /// REC finalizer scheduling occur here, never in ModuleSource::exchange.
    pub(crate) fn finish_exchange(&mut self, mut packet: ModuleExchange) -> Result<()> {
        if !self.exchange_pending
            || !packet.serviced
            || !std::sync::Arc::ptr_eq(&self.closure, &packet.owner)
        {
            return Err("module exchange not completed".into());
        }
        self.exchange_pending = false;
        if packet.accepted {
            self.observed = packet.observation;
        }
        self.pa_observed = packet.pa.take();
        if let (Some(observation), Some(api)) = (&mut self.pa_observed, &self.pa_preparation)
            && !observation.retirement.is_empty()
        {
            // Exclusive return has completed. Dispose native retired ownership
            // here and reserve an empty slot for the next source exchange.
            observation.retirement = api.eq_retirement_slot();
        }
        if let Some(frame) = packet.activated_frame
            && let Some(t) = &mut self.take
        {
            t.status.first_source_frame = Some(Counter(frame));
            if t.status.state != "finalizing" {
                t.status.state = "recording".into();
            }
        }
        // Refused activation is retained in the packet until this controller
        // can schedule finalization, including an epoch/fault cancellation.
        if let Some(mut recorder) = packet.retired_recorder.take().or(packet.start.take()) {
            if self.observed.recorder_cancel_code != 0 {
                recorder.fault(self.observed.recorder_cancel_code);
            }
            if let Some(t) = &mut self.take {
                t.status.state = "finalizing".into();
            }
            self.finish_worker(recorder)?;
        }
        Ok(())
    }
    fn service_attached_exchange(&mut self) -> Result<()> {
        if self.source.0.is_none() || self.exchange_pending {
            return Ok(());
        }
        let mut packet = self.prepare_exchange()?;
        self.source.exchange(&mut packet);
        self.finish_exchange(packet)
    }
    /// Explicit compatibility observation exchange after numeric source work.
    /// Independent execution returns the same owned observation/context packet.
    #[cfg(test)]
    pub(crate) fn observe_after_render(&mut self) {
        self.observe_pa();
    }

    pub fn load(manifest: Manifest, epoch: u64, frame: u64) -> Result<Self> {
        Self::load_configured(manifest, epoch, frame, 8)
    }
    pub fn load_configured(
        manifest: Manifest,
        epoch: u64,
        frame: u64,
        inputs: usize,
    ) -> Result<Self> {
        if inputs == 0 || inputs > 64 {
            return Err("REC owner ABI admits 1..64 raw tracks".into());
        }
        manifest.verify()?;
        if epoch == 0 || !frame.is_multiple_of(FRAMES as u64) {
            return Err("graph timeline".into());
        }
        let fx = Dsp::load(&manifest.fx.library, "fx", FRAMES as u32).map_err(|e| e.to_string())?;
        let pa = Dsp::load(&manifest.pa.library, "pa", FRAMES as u32).map_err(|e| e.to_string())?;
        // Real owner queries establish availability before any capability is exposed.
        let fc = fx.fx_capabilities().map_err(|e| e.to_string())?;
        let pd = pa.pa_descriptor().map_err(|e| e.to_string())?;
        if fc.channels != 2
            || fc.writable_parameters != 0
            || pd.input_channels != 2
            || pd.logical_outputs != 6
        {
            return Err("unsupported owner descriptor".into());
        }
        let closure = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let source = ModuleSource {
            closure: closure.clone(),
            library_identity: manifest.pa.library_sha256.as_bytes().try_into().unwrap(),
            fx,
            pa,
            processed: 0,
            source_fault: 0,
            epoch,
            next: frame,
            wet: [0.; FRAMES * 2],
            sum: [0.; FRAMES * 2],
            program: [0.; FRAMES * 2],
            raw: vec![0.; FRAMES * inputs],
            output: [0.; FRAMES * 6],
            inputs,
            pa_v2: None,
            pa_storage: None,
            pa_retired_storage: None,
            pa_hold: false,
            recorder: None,
            recording: false,
            recorder_cancel_code: 0,
        };
        Ok(Self {
            observed: source.observation(),
            fx_capabilities: fc,
            pa_descriptor: pd,
            source: SourceSlot(Some(source)),
            manifest,
            take: None,
            lifecycle: None,
            last_poll: None,
            pa_preparation: None,
            pa_observed: None,
            pa_metadata: None,
            prepared_recorder: None,
            returned_recorder: None,
            recorder_stop: false,
            exchange_pending: false,
            closure,
        })
    }
    /// Off callback, whole-boundary quiescence. Recreates PA and resets FX. An
    /// active take remains under its original epoch and is marked incomplete.
    pub fn discontinuity(&mut self, epoch: u64, frame: u64) -> Result<()> {
        if self.source.0.is_none() || self.exchange_pending {
            return Err("source recovery requires returned ownership".into());
        }
        if (self.source.pa_v2.is_some() || self.source.inputs != 8) && epoch <= self.source.epoch {
            return Err("fresh module source epoch required".into());
        }
        if epoch == 0 || !frame.is_multiple_of(48) {
            return Err("graph timeline".into());
        }
        let pa = Dsp::load(&self.manifest.pa.library, "pa", 48).map_err(|e| e.to_string())?;
        // Controller-side recovery prepares first; a refusal preserves old state.
        let recovered_v2 = if let Some(storage) = &self.source.pa_storage {
            Some(
                crate::host::pa_v2::Pa::load(
                    &self.manifest.pa.library,
                    storage.configuration_json.as_bytes(),
                    epoch,
                    frame,
                )
                .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let recovered_preparation = recovered_v2.as_ref().map(|pa| pa.preparation());
        let recovered_observation = recovered_preparation
            .as_ref()
            .filter(|api| api.detached_eq_available())
            .map(|api| PaEqObservation::new(api, self.pa_program_buses()))
            .transpose()?;
        self.source.fx.reset();
        self.source.pa = pa;
        self.retire_pa_changes();
        self.source.pa_v2 = recovered_v2;
        self.pa_preparation = recovered_preparation;
        self.pa_observed = recovered_observation;
        self.source.pa_hold =
            self.source.pa_hold || self.source.pa_v2.is_some() || self.source.inputs != 8;
        if let Some(storage) = &mut self.source.pa_storage {
            storage.input.fill(0.);
            storage.output.fill(0.);
        }
        if let Some(t) = &mut self.take
            && (self.lifecycle.is_some() || self.source.recorder.is_some())
        {
            t.status.state = "finalizing".into();
            if let Some(worker) = &mut self.lifecycle {
                if worker.cancel_code == 0 {
                    worker.cancel_code = if self.source.recorder_cancel_code != 0 {
                        self.source.recorder_cancel_code
                    } else {
                        4
                    };
                }
            } else if let Some(mut r) = self.source.recorder.take() {
                r.fault(4);
                self.finish_worker(r)?;
            }
        }
        self.source.epoch = epoch;
        self.source.next = frame;
        self.source.source_fault = 0;
        self.source.recorder_cancel_code = 0;
        self.closure.store(0, std::sync::atomic::Ordering::Release);
        self.source.output.fill(0.);
        self.observe_pa();
        Ok(())
    }
    /// No allocation, locks, I/O or controller lifecycle calls. The same unprocessed
    /// eight-input block feeds recording and GP04; FOH alone feeds wet+dry→PA.
    pub fn process(
        &mut self,
        epoch: u64,
        frame: u64,
        raw: &[[f64; 8]; FRAMES],
        mixed: &[[f64; 4]; FRAMES],
    ) -> std::result::Result<(), ProcessError> {
        self.process_interleaved(
            epoch,
            frame,
            raw.as_flattened(),
            mixed.as_flattened(),
            4,
            None,
        )
    }
    pub fn process_interleaved(
        &mut self,
        epoch: u64,
        frame: u64,
        raw: &[f64],
        mixed: &[f64],
        buses: usize,
        external_wet: Option<&[f64]>,
    ) -> std::result::Result<(), ProcessError> {
        self.process_interleaved_brain(epoch, frame, raw, mixed, buses, external_wet, None)
    }
    pub fn meter_main_pre_pa(&self) -> &[f64; FRAMES * 2] {
        &self.source.sum
    }
    pub fn program_before_talkback(&self) -> &[f64; FRAMES * 2] {
        &self.source.program
    }
    #[allow(clippy::too_many_arguments)]
    pub fn process_interleaved_brain(
        &mut self,
        epoch: u64,
        frame: u64,
        raw: &[f64],
        mixed: &[f64],
        buses: usize,
        external_wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
    ) -> std::result::Result<(), ProcessError> {
        self.process_with_master(
            epoch,
            frame,
            raw,
            mixed,
            buses,
            external_wet,
            talkback,
            &[1.; 48],
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn process_with_master(
        &mut self,
        epoch: u64,
        frame: u64,
        raw: &[f64],
        mixed: &[f64],
        buses: usize,
        external_wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
        master: &[f64; 48],
    ) -> std::result::Result<(), ProcessError> {
        let result = self.source.process_with_master(
            epoch,
            frame,
            raw,
            mixed,
            buses,
            external_wet,
            talkback,
            master,
        );
        self.observe_pa();
        result
    }
    fn mark_source_fault(&mut self, code: u32) {
        let _ = self.closure.compare_exchange(
            0,
            code,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        );
        if let Some(source) = &mut self.source.0 {
            source.mark_source_fault(code);
        }
        self.observe_pa();
        if let Some(worker) = &mut self.lifecycle
            && worker.cancel_code == 0
        {
            worker.cancel_code = code;
        }
    }
    pub fn quiesce_source(&mut self) -> Result<()> {
        self.mark_source_fault(6);
        if self.source.0.is_none() {
            self.recorder_stop = true;
            if let Some(worker) = &mut self.lifecycle
                && worker.cancel_code == 0
            {
                worker.cancel_code = 6;
            }
            if let Some(t) = &mut self.take
                && t.status.state != "finalized"
            {
                t.status.state = "finalizing".into();
            }
            return Ok(());
        }
        self.mute_pa()?;
        if let Some(t) = &mut self.take {
            if let Some(worker) = &mut self.lifecycle {
                if worker.cancel_code == 0 {
                    worker.cancel_code = self.observed.recorder_cancel_code;
                }
            } else if let Some(mut recorder) = self.source.recorder.take() {
                t.status.state = "finalizing".into();
                recorder.fault(6);
                self.finish_worker(recorder)?;
            }
        }
        Ok(())
    }
    /// Initial convenience only: before processing, prepare stereo program
    /// buses. Runtime changes must use the retained boundary transaction below.
    pub fn configure_pa_v2(&mut self, json: &[u8]) -> Result<()> {
        if self.source.0.is_none() {
            return Err("initial PA configuration requires source ownership".into());
        }
        if self.source.processed != 0 || self.source.pa_v2.is_some() {
            return Err("initial PA configuration requires an unprocessed legacy graph".into());
        }
        let mut prepared = self.prepare_pa_change(json, vec![0, 1], 2)?;
        let result = self.commit_pa_change(&mut prepared, self.source.epoch, self.source.next);
        if result != 0 {
            return Err(format!("initial PA commit {result}"));
        }
        self.retire_pa_changes();
        Ok(())
    }
    /// OffRT preparation; validates real owner shape and allocates every new
    /// interleaved buffer before a structural change reaches the worker.
    pub fn prepare_pa_change(
        &self,
        json: &[u8],
        program_buses: Vec<usize>,
        total_buses: usize,
    ) -> Result<GraphPaPrepared> {
        if self.observed.pa_retirement_occupied {
            return Err("PA retirement reservation occupied".into());
        }
        if total_buses < 2
            || program_buses.is_empty()
            || program_buses.iter().any(|&b| b >= total_buses)
        {
            return Err("PA program bus reference".into());
        }
        let initial = if self.observed.pa_v2_status.is_none() {
            Some(
                crate::host::pa_v2::Pa::open(&self.manifest.pa.library)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let preparation = initial.as_ref().map(|pa| pa.preparation());
        let api = self
            .pa_preparation
            .as_ref()
            .or(preparation.as_ref())
            .ok_or("PA preparation unavailable")?;
        let owner = api.prepare(json).map_err(|e| e.to_string())?;
        let observation = api
            .detached_eq_available()
            .then(|| PaEqObservation::new(api, &program_buses))
            .transpose()?;
        let status = owner
            .status()
            .map_err(|e| format!("prepared PA status {e}"))?;
        if status.version != 2
            || status.size != 80
            || status.committed != 0
            || status.sample_rate != 48000
            || status.max_block < FRAMES as u32
            || status.input_channels as usize != program_buses.len()
            || status.output_channels == 0
        {
            return Err("prepared PA dimensions/rate/block mismatch".into());
        }
        let input_samples = program_buses
            .len()
            .checked_mul(FRAMES)
            .ok_or("PA input buffer overflow")?;
        let output_samples = (status.output_channels as usize)
            .checked_mul(FRAMES)
            .ok_or("PA output buffer overflow")?;
        let configuration_json = std::str::from_utf8(json)
            .map_err(|e| e.to_string())?
            .to_owned();
        let generation = self
            .pa_observed
            .as_ref()
            .map(|p| p.pa_status)
            .or(self.observed.pa_v2_status)
            .map(|status| status.map(|s| s.generation))
            .transpose()
            .map_err(|e| format!("observed PA status {e}"))?
            .unwrap_or(0);
        Ok(GraphPaPrepared {
            owner,
            initial,
            preparation,
            observation,
            metadata: Some(PaMetadata {
                configuration_json: configuration_json.clone(),
                program_buses: program_buses.clone(),
            }),
            storage: Some(PaStorage {
                input: vec![0.; input_samples],
                output: vec![0.; output_samples],
                configuration_json,
                program_buses,
                total_buses,
            }),
            expected_generation: generation,
            expected_epoch: self.observed.epoch,
            expected_library_sha256: self.manifest.pa.library_sha256.clone(),
        })
    }
    /// Actual boundary: no allocation/free/Arc change. Retires both old owner
    /// state and host buffers; the caller retains `prepared` off the RT path.
    pub fn commit_pa_change(
        &mut self,
        prepared: &mut GraphPaPrepared,
        epoch: u64,
        frame: u64,
    ) -> i32 {
        let result = self.source.commit_pa_change(prepared, epoch, frame);
        self.finish_pa_change(prepared, result);
        self.observe_pa();
        result
    }
    /// Controller completion after a globally ordered source commit. Replaced
    /// metadata/context owners stay in the prepared packet until caller disposal.
    /// Complete before issuing the next ModuleExchange: an already outstanding
    /// packet owns the previous PA context/indices and must not overwrite this
    /// completion. A future fused command packet must swap contexts source-side.
    pub(crate) fn finish_pa_change(&mut self, prepared: &mut GraphPaPrepared, result: i32) {
        if result != 0 {
            return;
        }
        // Keep replaced contexts/API ownership in the caller's prepared packet;
        // even a successful boundary commit performs no native destruction.
        if prepared.preparation.is_some() {
            std::mem::swap(&mut self.pa_preparation, &mut prepared.preparation);
        }
        std::mem::swap(&mut self.pa_observed, &mut prepared.observation);
        std::mem::swap(&mut self.pa_metadata, &mut prepared.metadata);
    }
    /// Bounded compatibility handoff. A detached caller transfers the same packet
    /// to ModuleSource::capture_pa_observation and returns it to the controller.
    fn observe_pa(&mut self) {
        let Some(source) = &self.source.0 else {
            return;
        };
        self.observed = source.observation();
        if let Some(observation) = &mut self.pa_observed {
            source.capture_pa_observation(observation);
        }
    }
    /// Controller-only retirement, including old host vectors/configuration.
    pub fn retire_pa_changes(&mut self) {
        let Some(source) = &mut self.source.0 else {
            return;
        };
        if let Some(pa) = &mut source.pa_v2 {
            pa.retire();
        }
        source.pa_retired_storage = None;
        self.observed.pa_retirement_occupied = false;
    }
    pub fn mute_pa(&mut self) -> Result<()> {
        let result = self
            .source
            .0
            .as_mut()
            .ok_or("PA mute requires prepared source command")?
            .mute_pa();
        self.observe_pa();
        if result == 0 {
            Ok(())
        } else {
            Err(format!("PA mute {result}"))
        }
    }
    pub fn master_eq_indices(&self) -> Result<[usize; 2]> {
        let map = self.pa_program_buses();
        let mut result = [0; 2];
        for (bus, index) in result.iter_mut().enumerate() {
            let mut matches = map.iter().enumerate().filter(|(_, b)| **b == bus);
            *index = matches.next().ok_or("master bus absent")?.0;
            if matches.next().is_some() {
                return Err("ambiguous master bus".into());
            }
        }
        Ok(result)
    }
    pub fn master_eq_source_recovery_required(&self) -> bool {
        self.observed.source_fault != 0
    }
    pub fn master_eq_status(&self) -> std::result::Result<crate::host::pa_v2::EqStatus, i32> {
        match &self.pa_observed {
            Some(observed) => observed.status(),
            None => self.observed.eq_status.ok_or(-5)?,
        }
    }
    pub fn master_eq_readback(&self) -> Result<String> {
        let Some(observed) = self.pa_observed.as_ref() else {
            return self
                .source
                .0
                .as_ref()
                .ok_or("legacy PA readback requires source ownership")?
                .pa_v2
                .as_ref()
                .ok_or("PA unavailable")?
                .eq_readback(self.master_eq_indices()?)
                .map_err(|e| e.to_string());
        };
        observed
            .status()
            .map_err(|e| format!("PA observation {e}"))?;
        observed.context.readback().map_err(|e| e.to_string())
    }
    pub fn prepare_master_eq(
        &mut self,
        json: &[u8],
        expected: crate::host::pa_v2::EqStatus,
        expected_map: &[usize],
        frame: u64,
    ) -> Result<GraphEqPrepared> {
        if expected_map != self.pa_program_buses() {
            return Err("stale PA bus map".into());
        }
        let indices = self.master_eq_indices()?;
        // Explicit source return, followed by controller destruction. Preparation
        // below uses only copied context and the retained API, never active DSP.
        let owner = if let Some(observation) = self.pa_observed.as_mut() {
            let api = self
                .pa_preparation
                .as_ref()
                .ok_or("PA preparation unavailable")?;
            // Compatibility callers may return a packet directly rather than
            // through finish_exchange. Release its already-returned allocation
            // before asking the source to fill the reserved slot again.
            if !observation.retirement.is_empty() {
                observation.retirement = api.eq_retirement_slot();
            }
            if let Some(source) = &mut self.source.0 {
                let code = source.return_eq_retirement(observation);
                if code != 0 {
                    return Err(format!("EQ retirement return {code}"));
                }
            }
            if !observation.retirement.is_empty() {
                observation.retirement = api.eq_retirement_slot();
            }
            observation
                .context
                .prepare(json, expected, frame)
                .map_err(|e| e.to_string())?
        } else {
            // Legacy synchronous ABI compatibility. This owner cannot qualify for
            // detached source execution until its context interface is available.
            let pa = self
                .source
                .0
                .as_mut()
                .ok_or("legacy PA preparation requires source ownership")?
                .pa_v2
                .as_mut()
                .ok_or("live EQ unavailable")?;
            pa.retire_eq();
            pa.prepare_eq(json, expected, frame)
                .map_err(|e| e.to_string())?
        };
        // The actual owner has already strictly validated this narrow patch.
        let patch: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        let edits = patch["inputs"].as_array().ok_or("EQ inputs")?;
        let actual: Vec<_> = edits
            .iter()
            .map(|e| e["input_index"].as_u64().map(|n| n as usize))
            .collect();
        if actual.len() != 2 || !indices.iter().all(|i| actual.contains(&Some(*i))) {
            return Err("master stereo identity".into());
        }
        let storage = self.pa_metadata.as_ref().ok_or("PA metadata unavailable")?;
        let mut metadata: serde_json::Value =
            serde_json::from_str(&storage.configuration_json).map_err(|e| e.to_string())?;
        for edit in edits {
            let index = edit["input_index"].as_u64().ok_or("EQ index")? as usize;
            for key in ["eq_enabled", "eq", "geq_enabled", "geq_db"] {
                metadata["inputs"][index][key] = edit[key].clone();
            }
        }
        let metadata = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;
        if metadata.len() > 48 * 1024 {
            return Err("retained PA metadata bound".into());
        }
        Ok(GraphEqPrepared {
            owner,
            accepted_metadata: metadata.clone(),
            metadata,
            expected_map: expected_map.to_vec(),
        })
    }
    pub fn commit_master_eq(&mut self, p: &mut GraphEqPrepared, epoch: u64, frame: u64) -> i32 {
        let code = self.source.commit_master_eq(p, epoch, frame);
        self.finish_master_eq(p, code);
        self.observe_pa();
        code
    }
    pub(crate) fn finish_master_eq(&mut self, p: &mut GraphEqPrepared, code: i32) {
        if code == 0
            && let Some(metadata) = &mut self.pa_metadata
        {
            std::mem::swap(&mut metadata.configuration_json, &mut p.accepted_metadata);
        }
    }

    pub fn pa_configuration_json(&self) -> Option<&str> {
        self.pa_metadata
            .as_ref()
            .map(|s| s.configuration_json.as_str())
    }
    pub fn pa_program_buses(&self) -> &[usize] {
        self.pa_metadata
            .as_ref()
            .map_or(&[], |s| s.program_buses.as_slice())
    }
    pub fn pa_v2_capabilities(&self) -> Option<crate::host::pa_v2::Capabilities> {
        self.observed.pa_v2_capabilities
    }
    pub fn pa_v2_status(&self) -> Option<std::result::Result<crate::host::pa_v2::Status, i32>> {
        self.pa_observed
            .as_ref()
            .map(|p| p.pa_status)
            .or(self.observed.pa_v2_status)
    }
    pub fn rearm_pa(&mut self) -> Result<()> {
        let result = self
            .source
            .0
            .as_mut()
            .ok_or("PA rearm requires prepared source command")?
            .rearm_pa();
        self.observe_pa();
        if result == 0 {
            Ok(())
        } else {
            Err(format!("PA rearm {result}"))
        }
    }
    pub fn output_interleaved(&self) -> &[f64] {
        self.source.output_interleaved()
    }
    pub fn output(&self) -> &[f64; FRAMES * 6] {
        &self.source.output
    }
    /// Accepts preparation only. One bounded worker creates the handle and
    /// observer. No physical endpoints and no client-chosen recording paths.
    pub fn start(&mut self, root: &Path, take_id: &str, operation_id: Counter) -> Result<()> {
        if !crate::show::id(take_id) || operation_id.0 == 0 {
            return Err("record identity".into());
        }
        if self.observed.source_fault != 0
            || self.closure.load(std::sync::atomic::Ordering::Acquire) != 0
        {
            return Err("source discontinuity requires explicit recreation".into());
        }
        if self.lifecycle.is_some()
            || self.exchange_pending
            || self.prepared_recorder.is_some()
            || self.returned_recorder.is_some()
            || self.observed.recorder_present
            || self
                .take
                .as_ref()
                .is_some_and(|t| t.status.state != "finalized")
        {
            return Err("recorder busy".into());
        }
        let path = root.join(take_id);
        let library = self.manifest.rec.library.clone();
        let epoch = self.observed.epoch;
        let channels = self.observed.inputs;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("gp05-rec-prepare".into())
            .spawn(move || {
                let prepared = (|| {
                    let mut r = Recorder::create_configured(&library, &path, 48, epoch, channels)
                        .map_err(|e| e.to_string())?;
                    let observer = r.observer().map_err(|e| e.to_string())?;
                    Ok((r, observer))
                })();
                let _ = sender.send(WorkerResult::Prepared(prepared));
            })
            .map_err(|e| e.to_string())?;
        let status = RecordingStatus {
            take_id: take_id.into(),
            operation_id,
            source_epoch: Counter(self.observed.epoch),
            first_source_frame: None,
            mapping: (0..self.observed.inputs)
                .map(|i| format!("input-{:02}/raw", i + 1))
                .collect(),
            state: "preparing".into(),
            outcome: "pending".into(),
            accepted_frames: Counter(0),
            written_frames: Counter(0),
            durable_frames: None,
            dropped_frames: Counter(0),
            overflow_blocks: Counter(0),
            gap_frames: Counter(0),
            invalid_blocks: Counter(0),
            clipped_samples: Counter(0),
            writer_fault: false,
            host_fault: 0,
        };
        self.recorder_stop = false;
        self.take = Some(Take {
            observer: None,
            status,
        });
        self.lifecycle = Some(Lifecycle {
            receiver,
            thread,
            cancel_code: 0,
        });
        self.last_poll = None;
        Ok(())
    }
    /// Quiesces pushes immediately at this control-side whole-frame boundary.
    /// Consuming finish/drain/sync happens exclusively on a bounded worker.
    pub fn stop(&mut self, take_id: &str, operation_id: Counter) -> Result<()> {
        let t = self.take.as_mut().ok_or("no take")?;
        if t.status.take_id != take_id || t.status.operation_id != operation_id {
            return Err("retired record operation".into());
        }
        if let Some(worker) = &mut self.lifecycle {
            if t.status.state != "preparing" {
                return Err("already finalizing".into());
            }
            if worker.cancel_code == 0 {
                worker.cancel_code = if self.observed.recorder_cancel_code != 0 {
                    self.observed.recorder_cancel_code
                } else {
                    5
                };
            }
            t.status.state = "finalizing".into();
            return Ok(());
        }
        if t.status.state == "finalizing" || t.status.state == "finalized" {
            return Err("already finalizing".into());
        }
        t.status.state = "finalizing".into();
        self.recorder_stop = true;
        if let Some(mut recorder) = self.prepared_recorder.take() {
            recorder.fault(5);
            return self.finish_worker(recorder);
        }
        self.service_attached_exchange()
    }
    fn finish_worker(&mut self, mut recorder: Recorder) -> Result<()> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let (give, take) = std::sync::mpsc::sync_channel::<Recorder>(1);
        // Spawn before handing over ownership: a spawn refusal must not consume
        // and synchronously finalize the recorder on the control pump.
        let thread = match std::thread::Builder::new()
            .name("gp05-rec-finish".into())
            .spawn(move || {
                if let Ok(r) = take.recv() {
                    let result = r.finish();
                    let _ = sender.send(WorkerResult::Finished(result));
                }
            }) {
            Ok(thread) => thread,
            Err(e) => {
                recorder.fault(10);
                if let Some(t) = &mut self.take {
                    self.returned_recorder = Some(recorder);
                    t.status.state = "finalizing".into();
                    t.status.host_fault = 10;
                }
                return Err(e.to_string());
            }
        };
        // Capacity one, empty queue, worker owns receiver until it obtains value.
        if let Err(e) = give.send(recorder) {
            let mut r = e.0;
            r.fault(10);
            if let Some(t) = &mut self.take {
                self.returned_recorder = Some(r);
                t.status.host_fault = 10;
            }
            let _ = thread.join();
            return Err("finalizer handoff failed".into());
        }
        self.lifecycle = Some(Lifecycle {
            receiver,
            thread,
            cancel_code: 0,
        });
        Ok(())
    }
    /// Nonblocking readiness/completion transfer, outside bounded processing.
    pub fn poll_lifecycle(&mut self) -> Result<()> {
        self.poll_lifecycle_result()?;
        self.service_attached_exchange()
    }
    fn poll_lifecycle_result(&mut self) -> Result<()> {
        if self.lifecycle.is_none()
            && let Some(recorder) = self.returned_recorder.take()
        {
            self.finish_worker(recorder)?;
        }
        if self.observed.recorder_cancel_code != 0
            && let Some(worker) = &mut self.lifecycle
            && worker.cancel_code == 0
        {
            worker.cancel_code = self.observed.recorder_cancel_code;
        }
        let Some(worker) = &self.lifecycle else {
            return Ok(());
        };
        if !worker.thread.is_finished() {
            return Ok(());
        }
        let result = worker.receiver.try_recv().map_err(|e| e.to_string());
        let worker = self.lifecycle.take().unwrap();
        let joined = worker.thread.join();
        let t = self.take.as_mut().ok_or("lost take")?;
        if joined.is_err() || result.is_err() {
            t.status.state = "finalized".into();
            t.status.outcome = "error".into();
            return Ok(());
        }
        match result.unwrap() {
            WorkerResult::Prepared(Ok((mut r, observer))) => {
                t.observer = Some(observer);
                if worker.cancel_code != 0 {
                    // Observer readiness describes the raw owner handle, not
                    // permission to activate this cancelled host operation.
                    t.status.state = "finalizing".into();
                    r.fault(worker.cancel_code);
                    self.finish_worker(r)?;
                } else {
                    // Host recording readiness is published only after the
                    // source accepts this handle at its own frame boundary.
                    self.prepared_recorder = Some(r);
                }
            }
            WorkerResult::Prepared(Err(_)) => {
                t.status.state = "finalized".into();
                t.status.outcome = "error".into();
            }
            WorkerResult::Finished(result) => {
                if let Some(observer) = &t.observer {
                    let progress = observer.snapshot().map_err(|e| e.to_string())?;
                    Self::update(t, progress);
                }
                if result < 0 {
                    t.status.state = "finalized".into();
                    t.status.outcome = "error".into();
                }
            }
        }
        Ok(())
    }
    fn update(t: &mut Take, p: RecorderProgress) {
        t.status.state = match p.state {
            1 => "recording",
            2 => "finalizing",
            _ => "finalized",
        }
        .into();
        t.status.outcome = match p.outcome {
            1 => "complete",
            2 => "incomplete",
            3 => "error",
            _ => "pending",
        }
        .into();
        t.status.accepted_frames = Counter(p.accepted_frames);
        t.status.written_frames = Counter(p.written_frames);
        t.status.durable_frames = if p.durable_frames == u64::MAX {
            None
        } else {
            Some(Counter(p.durable_frames))
        };
        t.status.dropped_frames = Counter(p.dropped_frames);
        t.status.overflow_blocks = Counter(p.overflow_blocks);
        t.status.gap_frames = Counter(p.gap_frames);
        t.status.invalid_blocks = Counter(p.invalid_blocks);
        t.status.clipped_samples = Counter(p.clipped_samples);
        t.status.writer_fault = p.writer_fault != 0;
        t.status.host_fault = p.host_fault;
    }
    /// Off callback <=10 Hz; finalization performs one independent terminal query.
    pub fn status(&mut self, show: &str, now: u64) -> Result<ModuleStatus> {
        if self
            .last_poll
            .is_none_or(|last| now.saturating_sub(last) >= 100)
        {
            if let Some(t) = &mut self.take
                && t.status.state != "finalized"
                && let Some(observer) = &t.observer
            {
                let p = observer.snapshot().map_err(|e| e.to_string())?;
                let finalizing = t.status.state == "finalizing";
                let preparing = t.status.state == "preparing";
                Self::update(t, p);
                if preparing {
                    t.status.state = "preparing".into();
                }
                // Owner atomics can lead the host handoff/join. Publish progress
                // while keeping host finalization pending until worker completion.
                if finalizing {
                    t.status.state = "finalizing".into();
                    t.status.outcome = "pending".into();
                }
            }
            self.last_poll = Some(now);
        }
        let mut status = ModuleStatus::unavailable(show, self.observed.epoch, self.observed.next);
        status.available = true;
        status.protection = "owner-sample-limiter-only".into();
        status.configuration = Some(Configuration {
            sample_rate: 48000,
            block_frames: 48,
            raw_inputs: (0..self.observed.inputs)
                .map(|i| format!("input-{:02}/raw", i + 1))
                .collect(),
            fx_mix: "dry-plus-fixed-wet".into(),
            pa_inputs: self.observed.pa_inputs as u32,
            pa_outputs: self.observed.pa_outputs as u32,
        });
        for (owner, a) in [
            ("rec", &self.manifest.rec),
            ("fx", &self.manifest.fx),
            ("pa", &self.manifest.pa),
        ] {
            status.libraries.insert(
                owner.into(),
                LibraryIdentity {
                    sha256: a.library_sha256.clone(),
                    state: if owner == "rec"
                        && !self.take.as_ref().is_some_and(|t| t.observer.is_some())
                    {
                        "verified"
                    } else {
                        "loaded"
                    }
                    .into(),
                },
            );
        }
        status.fx = Some(FxHealth {
            capabilities: self.fx_capabilities.clone(),
            status: self
                .observed
                .fx_status
                .clone()
                .map_err(|e| format!("FX status {e}"))?,
        });
        if let (Some(capabilities), Some(storage)) =
            (self.observed.pa_v2_capabilities, &self.pa_metadata)
        {
            status.version = 2;
            status.pa_v2 = Some(PaHealthV2 {
                capabilities,
                status: self
                    .pa_v2_status()
                    .ok_or("PA observation unavailable")?
                    .map_err(|e| format!("PA status {e}"))?,
                configuration_json: storage.configuration_json.clone(),
                program_buses: storage.program_buses.clone(),
            });
        } else {
            status.pa = Some(PaHealth {
                descriptor: self.pa_descriptor.clone(),
                status: self
                    .observed
                    .pa_status
                    .clone()
                    .map_err(|e| format!("PA status {e}"))?,
            });
        }
        if self.observed.inputs != 8 {
            status.version = 2;
        }
        status.source_fault = self.observed.source_fault;
        status.readiness = if status
            .pa
            .as_ref()
            .is_some_and(|p| p.status.fault_latched != 0)
            || status
                .pa_v2
                .as_ref()
                .is_some_and(|p| p.status.fault_latched != 0)
        {
            "faulted"
        } else if self.observed.source_fault != 0
            || status
                .fx
                .as_ref()
                .is_some_and(|f| f.status.last_process_result != 0)
        {
            "degraded"
        } else if self.observed.pa_hold
            || status.pa_v2.as_ref().is_some_and(|p| p.status.muted != 0)
        {
            "muted"
        } else {
            "ready"
        }
        .into();
        status.activity = if self.observed.processed == 0 {
            "idle"
        } else {
            "processed"
        }
        .into();
        status.recording = self.take.as_ref().map(|t| t.status.clone());
        Ok(status)
    }
}

impl Drop for ModuleGraph {
    fn drop(&mut self) {
        let _ = self.closure.compare_exchange(
            0,
            1,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        );
        // Explicit control-side shutdown joins only this owned lifecycle worker.
        // A late prepared handle is faulted/finalized by Recorder::drop, never
        // reactivated. All observers/libraries remain retained through cleanup.
        if let Some(worker) = self.lifecycle.take() {
            let _ = worker.thread.join();
            if let Ok(result) = worker.receiver.try_recv() {
                drop(result);
            }
        }
        if let Some(mut r) = self.source.0.as_mut().and_then(|s| s.recorder.take()) {
            r.fault(1);
            let _ = r.finish();
        }
    }
}

#[cfg(test)]
mod detached_pa_tests {
    use super::*;

    #[test]
    #[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
    fn actual_module_owned_recorder_exchange_and_closure() {
        use std::time::Duration;
        struct Directory(PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let path =
            std::env::temp_dir().join(format!("gp-h1-module-exchange-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let directory = Directory(path);
        let manifest =
            Manifest::load(Path::new(&std::env::var_os("GP05_MANIFEST").unwrap())).unwrap();
        let config = std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap();
        let mut graph = ModuleGraph::load(manifest, 1, 0).unwrap();
        graph.configure_pa_v2(&config).unwrap();
        graph.rearm_pa().unwrap();
        let mut source = graph.take_source().unwrap();
        graph.start(&directory.0, "owned-take", Counter(1)).unwrap();
        assert!((0..1000).any(|_| {
            graph.poll_lifecycle().unwrap();
            if graph.prepared_recorder.is_some() {
                true
            } else {
                std::thread::sleep(Duration::from_millis(1));
                false
            }
        }));
        assert_eq!(
            graph.status("test", 0).unwrap().recording.unwrap().state,
            "preparing"
        );
        let mut packet = graph.prepare_exchange().unwrap();
        assert!(graph.prepare_exchange().is_err());
        // A mixer command refused before source service must return this packet
        // without stranding the REC handle, PA context or exchange reservation.
        graph.cancel_exchange(packet).unwrap();
        assert!(graph.prepared_recorder.is_some());
        packet = graph.prepare_exchange().unwrap();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                crate::remote::assert_no_allocations(|| {
                    source.exchange(&mut packet);
                    for block in 0..32 {
                        source
                            .process_interleaved_brain(
                                1,
                                block * 48,
                                &[0.125; 384],
                                &[0.125; 192],
                                4,
                                None,
                                None,
                            )
                            .unwrap();
                    }
                    // A retained completion cannot be applied twice or overwritten.
                    source.exchange(&mut packet);
                    assert_eq!(packet.activated_frame, Some(0));
                    assert_eq!(packet.observation.next, 0);
                    source.observe_exchange(&mut packet);
                    assert_eq!(packet.observation.next, 1536);
                });
            });
            // Complete graph metadata and native detached EQ readback never
            // inspect the actively rendering source while the packet is away.
            assert_eq!(graph.pa_configuration_json().unwrap().as_bytes(), config);
            assert_eq!(graph.status("test", 1).unwrap().frame.0, 0);
            worker.join().unwrap();
        });
        graph.finish_exchange(packet).unwrap();
        assert_eq!(
            graph
                .status("test", 2)
                .unwrap()
                .recording
                .unwrap()
                .first_source_frame,
            Some(Counter(0))
        );
        let mut stale = graph.prepare_exchange().unwrap();
        stale.epoch = 2;
        stale.stop = true;
        crate::remote::assert_no_allocations(|| {
            source.exchange(&mut stale);
            assert!(source.recorder.is_some());
            assert!(source.recording);
        });
        graph.finish_exchange(stale).unwrap();
        graph.stop("owned-take", Counter(1)).unwrap();
        let mut packet = graph.prepare_exchange().unwrap();
        crate::remote::assert_no_allocations(|| {
            source.exchange(&mut packet);
            assert!(source.recorder.is_none());
            assert!(packet.retired_recorder.is_some());
            for block in 32..40 {
                source
                    .process_interleaved_brain(
                        1,
                        block * 48,
                        &[0.125; 384],
                        &[0.125; 192],
                        4,
                        None,
                        None,
                    )
                    .unwrap();
            }
        });
        graph.finish_exchange(packet).unwrap();
        assert!((0..1000).any(|_| {
            graph.poll_lifecycle().unwrap();
            if graph
                .status("test", 10000)
                .unwrap()
                .recording
                .unwrap()
                .state
                == "finalized"
            {
                true
            } else {
                std::thread::sleep(Duration::from_millis(1));
                false
            }
        }));
        let status = graph.status("test", 20000).unwrap().recording.unwrap();
        assert_eq!(status.outcome, "complete");
        assert_eq!(status.accepted_frames.0, 1536);
        assert_eq!(status.written_frames.0, 1536);
        assert_eq!(status.dropped_frames.0, 0);
        let wavs: Vec<_> = std::fs::read_dir(directory.0.join("owned-take"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
            .collect();
        assert_eq!(wavs.len(), 8);
        for path in wavs {
            let mut wav = hound::WavReader::open(path).unwrap();
            assert_eq!(wav.spec().bits_per_sample, 24);
            let samples: Vec<_> = wav.samples::<i32>().map(|sample| sample.unwrap()).collect();
            assert_eq!(samples, vec![1_048_576; 1536]);
        }
        // Structural preparation is controller-only even after source transfer;
        // both native graph retirement and host vectors return in one packet.
        assert_eq!(source.mute_pa(), 0);
        for block in 40..48 {
            source
                .process_interleaved_brain(
                    1,
                    block * 48,
                    &[0.125; 384],
                    &[0.125; 192],
                    4,
                    None,
                    None,
                )
                .unwrap();
        }
        let mut packet = graph.prepare_exchange().unwrap();
        crate::remote::assert_no_allocations(|| source.exchange(&mut packet));
        graph.finish_exchange(packet).unwrap();
        let mut change = graph.prepare_pa_change(&config, vec![0, 1], 4).unwrap();
        crate::remote::assert_no_allocations(|| {
            assert_eq!(source.commit_pa_change(&mut change, 2, 2304), -4);
            assert_eq!(source.commit_pa_change(&mut change, 1, 2304), 0);
            assert!(source.pa_retired_storage.is_some());
        });
        graph.finish_pa_change(&mut change, 0);
        drop(change);
        let mut packet = graph.prepare_exchange().unwrap();
        crate::remote::assert_no_allocations(|| {
            source.exchange(&mut packet);
            assert!(!packet.pa_retirement.as_ref().unwrap().is_empty());
            assert!(packet.retired_storage.is_some());
            assert!(source.pa_retired_storage.is_none());
        });
        graph.finish_exchange(packet).unwrap();
        assert_eq!(graph.pa_v2_status().unwrap().unwrap().next_frame, 2304);
        assert!(graph.prepare_pa_change(&config, vec![0, 1], 4).is_ok());
        // Controller loss reaches an owned source without another exchange.
        drop(graph);
        crate::remote::assert_no_allocations(|| {
            assert_eq!(
                source.process_interleaved_brain(
                    1,
                    2304,
                    &[0.125; 384],
                    &[0.125; 192],
                    4,
                    None,
                    None
                ),
                Err(ProcessError::Source)
            );
            assert!(source.output_interleaved().iter().all(|v| *v == 0.));
        });
    }

    #[test]
    #[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
    fn actual_module_numeric_observation_and_commit_retention() {
        let manifest =
            Manifest::load(Path::new(&std::env::var_os("GP05_MANIFEST").unwrap())).unwrap();
        let config = std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap();
        let mut graph = ModuleGraph::load(manifest, 1, 0).unwrap();
        graph.configure_pa_v2(&config).unwrap();
        graph.rearm_pa().unwrap();
        let before = graph.status("test", 0).unwrap();
        assert_eq!(before.frame.0, 0);
        for block in 0..32 {
            crate::remote::assert_no_allocations(|| {
                graph
                    .source
                    .process_interleaved_brain(
                        1,
                        block * 48,
                        &[0.125; 384],
                        &[0.125; 192],
                        4,
                        None,
                        None,
                    )
                    .unwrap();
                let observed = graph.source.observation();
                assert_eq!(observed.next, (block + 1) * 48);
                assert_eq!(observed.fx_status.as_ref().unwrap().last_process_result, 0);
                assert_eq!(
                    observed.pa_v2_status.unwrap().unwrap().next_frame,
                    observed.next
                );
            });
        }
        // Controller readback remains a coherent owned observation; it cannot
        // announce source progress until the observation is explicitly handed back.
        assert_eq!(graph.status("test", 1).unwrap().frame.0, 0);
        graph.observe_pa();
        assert_eq!(graph.status("test", 2).unwrap().frame.0, 1536);
        graph.mute_pa().unwrap();
        // Structural replacement waits for the existing owner mute ramp.
        for block in 32..40 {
            graph
                .process_interleaved_brain(
                    1,
                    block * 48,
                    &[0.125; 384],
                    &[0.125; 192],
                    4,
                    None,
                    None,
                )
                .unwrap();
        }
        let mut next = graph.prepare_pa_change(&config, vec![0, 1], 4).unwrap();
        crate::remote::assert_no_allocations(|| {
            assert_eq!(graph.source.commit_pa_change(&mut next, 2, 1920), -4);
            assert!(next.storage.is_some());
            assert_eq!(graph.source.commit_pa_change(&mut next, 1, 1920), 0);
            assert!(next.storage.is_none());
            assert!(graph.source.pa_retired_storage.is_some());
            // Refusal preserves both active and retired storage.
            assert_eq!(graph.source.commit_pa_change(&mut next, 1, 1920), -1);
            assert!(graph.source.pa_retired_storage.is_some());
        });
        graph.retire_pa_changes();
        drop(next);
    }

    #[test]
    #[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
    fn actual_module_detached_pa_packet_and_retirement() {
        let manifest =
            Manifest::load(Path::new(&std::env::var_os("GP05_MANIFEST").unwrap())).unwrap();
        let config = std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap();
        let mut graph = ModuleGraph::load(manifest, 1, 0).unwrap();
        graph.configure_pa_v2(&config).unwrap();
        graph.rearm_pa().unwrap();
        let mut reference = ModuleGraph::load(graph.manifest.clone(), 1, 0).unwrap();
        reference.configure_pa_v2(&config).unwrap();
        reference.rearm_pa().unwrap();
        let mut samples = Vec::with_capacity(32);
        for block in 0..32 {
            reference
                .process_interleaved_brain(
                    1,
                    block * 48,
                    &[0.125; 384],
                    &[0.125; 192],
                    4,
                    None,
                    None,
                )
                .unwrap();
            samples.push(reference.output_interleaved().to_vec());
        }
        assert!(samples.iter().flatten().any(|s| s.abs() > 0.0001));
        let mut observed = graph.pa_observed.take().unwrap();
        let expected = observed.status().unwrap();
        let readback = observed.context.readback().unwrap();
        let value: serde_json::Value = serde_json::from_str(&readback).unwrap();
        let mut patch = serde_json::json!({"version":1,"inputs":value["target"]});
        patch["inputs"][0]["eq"][0]["db"] = serde_json::json!(3.0);
        let patch = serde_json::to_vec(&patch).unwrap();
        // The production packet and library-only preparer remain on the
        // controller while the actual ModuleSource runs PA and local FX.
        std::thread::scope(|scope| {
            let source = &mut graph.source;
            let worker = scope.spawn(move || {
                for (block, expected_samples) in samples.iter().enumerate() {
                    crate::remote::assert_no_allocations(|| {
                        source
                            .process_interleaved_brain(
                                1,
                                block as u64 * 48,
                                &[0.125; 384],
                                &[0.125; 192],
                                4,
                                None,
                                None,
                            )
                            .unwrap();
                        assert_eq!(
                            source.pa_storage.as_ref().unwrap().output,
                            *expected_samples
                        );
                    });
                }
            });
            for _ in 0..4 {
                let token = observed.context.prepare(&patch, expected, 1536).unwrap();
                drop(token);
                let prepared = graph
                    .pa_preparation
                    .as_ref()
                    .unwrap()
                    .prepare(&config)
                    .unwrap();
                drop(prepared);
                assert_eq!(observed.context.readback().unwrap(), readback);
                assert_eq!(observed.status().unwrap().next_frame, 0);
            }
            worker.join().unwrap();
        });
        let mut token = observed.context.prepare(&patch, expected, 1536).unwrap();
        crate::remote::assert_no_allocations(|| {
            assert_eq!(
                graph
                    .source
                    .pa_v2
                    .as_mut()
                    .unwrap()
                    .commit_eq(&mut token, 1, 1536),
                0
            );
            for block in 32..37 {
                graph
                    .source
                    .process_interleaved_brain(1, block * 48, &[0.; 384], &[0.; 192], 4, None, None)
                    .unwrap();
            }
            assert_eq!(graph.source.return_eq_retirement(&mut observed), 0);
            assert_eq!(graph.source.return_eq_retirement(&mut observed), -3);
        });
        assert!(!observed.retirement.is_empty());
        assert_eq!(observed.status().unwrap().next_frame, 1776);
        assert_ne!(observed.context.readback().unwrap(), readback);
        // A directly returned occupied packet must not make the next controller
        // preparation ask the source to fill that occupied slot again.
        graph.pa_observed = Some(observed);
        let expected = graph.master_eq_status().unwrap();
        let mut second = graph
            .prepare_master_eq(&patch, expected, &[0, 1], 1776)
            .unwrap();
        assert!(graph.pa_observed.as_ref().unwrap().retirement.is_empty());
        crate::remote::assert_no_allocations(|| {
            assert_eq!(graph.commit_master_eq(&mut second, 1, 1776), 0);
            for block in 37..42 {
                graph
                    .source
                    .process_interleaved_brain(1, block * 48, &[0.; 384], &[0.; 192], 4, None, None)
                    .unwrap();
            }
        });
        // The production packet return destroys retirement controller-side and
        // leaves a reusable empty slot for the following source observation.
        graph.poll_lifecycle().unwrap();
        assert!(graph.pa_observed.as_ref().unwrap().retirement.is_empty());
        let expected = graph.master_eq_status().unwrap();
        let third = graph
            .prepare_master_eq(&patch, expected, &[0, 1], 2016)
            .unwrap();
        drop(third);
    }
}
