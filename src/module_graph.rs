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
    Ok(format!("{:x}", hasher.finalize()))
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
    recorder: Option<Recorder>,
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
struct PaStorage {
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
}
pub struct ModuleGraph {
    manifest: Manifest,
    fx: Dsp,
    pa: Dsp,
    take: Option<Take>,
    lifecycle: Option<Lifecycle>,
    processed: u64,
    source_fault: u32,
    epoch: u64,
    next: u64,
    wet: [f64; FRAMES * 2],
    sum: [f64; FRAMES * 2],
    program: [f64; FRAMES * 2],
    raw: Vec<f64>,
    output: [f64; FRAMES * 6],
    last_poll: Option<u64>,
    inputs: usize,
    pa_v2: Option<crate::host::pa_v2::Pa>,
    pa_storage: Option<PaStorage>,
    pa_retired_storage: Option<PaStorage>,
    pa_hold: bool,
}
impl ModuleGraph {
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
        Ok(Self {
            manifest,
            fx,
            pa,
            take: None,
            lifecycle: None,
            processed: 0,
            source_fault: 0,
            epoch,
            next: frame,
            wet: [0.; FRAMES * 2],
            sum: [0.; FRAMES * 2],
            program: [0.; FRAMES * 2],
            raw: vec![0.; FRAMES * inputs],
            output: [0.; FRAMES * 6],
            last_poll: None,
            inputs,
            pa_v2: None,
            pa_storage: None,
            pa_retired_storage: None,
            pa_hold: false,
        })
    }
    /// Off callback, whole-boundary quiescence. Recreates PA and resets FX. An
    /// active take remains under its original epoch and is marked incomplete.
    pub fn discontinuity(&mut self, epoch: u64, frame: u64) -> Result<()> {
        if (self.pa_v2.is_some() || self.inputs != 8) && epoch <= self.epoch {
            return Err("fresh module source epoch required".into());
        }
        if epoch == 0 || !frame.is_multiple_of(48) {
            return Err("graph timeline".into());
        }
        let pa = Dsp::load(&self.manifest.pa.library, "pa", 48).map_err(|e| e.to_string())?;
        // Controller-side recovery prepares first; a refusal preserves old state.
        let recovered_v2 = if let Some(storage) = &self.pa_storage {
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
        self.fx.reset();
        self.pa = pa;
        self.retire_pa_changes();
        self.pa_v2 = recovered_v2;
        self.pa_hold = self.pa_hold || self.pa_v2.is_some() || self.inputs != 8;
        if let Some(storage) = &mut self.pa_storage {
            storage.input.fill(0.);
            storage.output.fill(0.);
        }
        if let Some(t) = &mut self.take
            && (self.lifecycle.is_some() || t.recorder.is_some())
        {
            t.status.state = "finalizing".into();
            if let Some(worker) = &mut self.lifecycle {
                if worker.cancel_code == 0 {
                    worker.cancel_code = 4;
                }
            } else if let Some(mut r) = t.recorder.take() {
                r.fault(4);
                self.finish_worker(r)?;
            }
        }
        self.epoch = epoch;
        self.next = frame;
        self.source_fault = 0;
        self.output.fill(0.);
        Ok(())
    }
    /// No allocation, locks, I/O or lifecycle/query calls. The same unprocessed
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
        &self.sum
    }
    pub fn program_before_talkback(&self) -> &[f64; FRAMES * 2] {
        &self.program
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
        self.output.fill(0.);
        if let Some(storage) = &mut self.pa_storage {
            storage.output.fill(0.);
        }
        if raw.len() != FRAMES * self.inputs
            || buses < 2
            || buses.checked_mul(FRAMES) != Some(mixed.len())
            || external_wet.is_some_and(|w| w.len() != FRAMES * 2)
            || talkback.is_some_and(|v| v.len() != mixed.len() || v.iter().any(|x| !x.is_finite()))
            || self
                .pa_storage
                .as_ref()
                .is_some_and(|s| buses < s.total_buses)
        {
            self.mark_source_fault(7);
            return Err(ProcessError::Source);
        }
        if epoch != self.epoch || frame != self.next || frame.checked_add(48).is_none() {
            self.mark_source_fault(6);
            return Err(ProcessError::Timeline);
        }
        if raw.iter().any(|v| {
            !v.is_finite()
                || *v < -1.
                || *v > 8_388_607. / 8_388_608.
                || (v * 8_388_608.).fract() != 0.
        }) {
            self.mark_source_fault(7);
            return Err(ProcessError::Source);
        }
        for i in 0..FRAMES {
            self.raw[i * self.inputs..(i + 1) * self.inputs]
                .copy_from_slice(&raw[i * self.inputs..(i + 1) * self.inputs]);
            self.sum[i * 2..i * 2 + 2].copy_from_slice(&mixed[i * buses..i * buses + 2]);
        }
        if let Some(t) = &mut self.take
            && t.status.state == "recording"
            && let Some(r) = &mut t.recorder
        {
            let _ = r.push(frame, &self.raw);
        }
        let fx = if let Some(wet) = external_wet {
            self.wet.copy_from_slice(wet);
            0
        } else {
            self.fx.process_result(&self.sum, &mut self.wet, 2)
        };
        if fx != 0 {
            self.wet.fill(0.);
        }
        for i in 0..FRAMES * 2 {
            self.sum[i] += self.wet[i];
        }
        self.program.copy_from_slice(&self.sum);
        if let Some(tb) = talkback {
            for f in 0..FRAMES {
                self.sum[f * 2] += tb[f * buses];
                self.sum[f * 2 + 1] += tb[f * buses + 1];
            }
        }
        let pa = if let (Some(pa), Some(storage)) = (&mut self.pa_v2, &mut self.pa_storage) {
            let count = storage.program_buses.len();
            for i in 0..FRAMES {
                for (channel, &bus) in storage.program_buses.iter().enumerate() {
                    // Main buses include the actual wet return before owner
                    // summing/crossover/protection; monitor buses remain explicit.
                    storage.input[i * count + channel] = if bus < 2 {
                        self.sum[i * 2 + bus]
                    } else {
                        mixed[i * buses + bus] + talkback.map_or(0., |tb| tb[i * buses + bus])
                    };
                }
            }
            pa.process(&storage.input, &mut storage.output, epoch, frame)
        } else {
            let result = self.pa.process_result(&self.sum, &mut self.output, 6);
            if self.pa_hold {
                self.output.fill(0.);
            }
            result
        };
        if pa != 0 {
            self.output.fill(0.);
            if let Some(storage) = &mut self.pa_storage {
                storage.output.fill(0.);
            }
        }
        self.next += 48;
        self.processed = self.processed.saturating_add(1);
        if pa != 0 {
            Err(ProcessError::Pa(pa))
        } else if fx != 0 {
            Err(ProcessError::Fx(fx))
        } else {
            Ok(())
        }
    }
    fn mark_source_fault(&mut self, code: u32) {
        if self.source_fault == 0 {
            self.source_fault = code;
        }
        if let Some(t) = &mut self.take
            && let Some(r) = &mut t.recorder
        {
            r.fault(code);
        }
        if let Some(worker) = &mut self.lifecycle
            && worker.cancel_code == 0
        {
            worker.cancel_code = code;
        }
    }
    pub fn quiesce_source(&mut self) -> Result<()> {
        self.mark_source_fault(6);
        self.mute_pa()?;
        if let Some(t) = &mut self.take {
            if let Some(worker) = &mut self.lifecycle {
                worker.cancel_code = 6;
            } else if let Some(mut recorder) = t.recorder.take() {
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
        if self.processed != 0 || self.pa_v2.is_some() {
            return Err("initial PA configuration requires an unprocessed legacy graph".into());
        }
        let mut prepared = self.prepare_pa_change(json, vec![0, 1], 2)?;
        let result = self.commit_pa_change(&mut prepared, self.epoch, self.next);
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
        if self.pa_retired_storage.is_some() {
            return Err("PA retirement reservation occupied".into());
        }
        if total_buses < 2
            || program_buses.is_empty()
            || program_buses.iter().any(|&b| b >= total_buses)
        {
            return Err("PA program bus reference".into());
        }
        let initial = if self.pa_v2.is_none() {
            Some(
                crate::host::pa_v2::Pa::open(&self.manifest.pa.library)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let pa = self
            .pa_v2
            .as_ref()
            .or(initial.as_ref())
            .ok_or("PA unavailable")?;
        let owner = pa.prepare(json).map_err(|e| e.to_string())?;
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
            .pa_v2
            .as_ref()
            .map(|p| p.status().map(|s| s.generation))
            .transpose()
            .map_err(|e| format!("active PA status {e}"))?
            .unwrap_or(0);
        Ok(GraphPaPrepared {
            owner,
            initial,
            storage: Some(PaStorage {
                input: vec![0.; input_samples],
                output: vec![0.; output_samples],
                configuration_json,
                program_buses,
                total_buses,
            }),
            expected_generation: generation,
            expected_epoch: self.epoch,
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
        if prepared.storage.is_none()
            || prepared.expected_library_sha256 != self.manifest.pa.library_sha256
        {
            return -1;
        }
        if self.pa_retired_storage.is_some() {
            return -3;
        }
        if epoch != self.epoch || epoch != prepared.expected_epoch || frame != self.next {
            return -4;
        }
        if self.source_fault != 0 {
            return -2;
        }
        if let Some(pa) = &self.pa_v2 {
            let status = match pa.status() {
                Ok(s) => s,
                Err(e) => return e,
            };
            if status.generation != prepared.expected_generation {
                return -4;
            }
        } else if self.processed != 0 && !self.pa_hold {
            return -3;
        }
        let result = if let Some(pa) = &mut self.pa_v2 {
            pa.commit(&mut prepared.owner, epoch, frame)
        } else if let Some(pa) = &mut prepared.initial {
            pa.commit(&mut prepared.owner, epoch, frame)
        } else {
            return -1;
        };
        if result != 0 {
            return result;
        }
        if self.pa_v2.is_none() {
            self.pa_v2 = prepared.initial.take();
        }
        self.pa_retired_storage = self.pa_storage.take();
        self.pa_storage = prepared.storage.take();
        self.pa_hold = true;
        0
    }
    /// Controller-only retirement, including old host vectors/configuration.
    pub fn retire_pa_changes(&mut self) {
        if let Some(pa) = &mut self.pa_v2 {
            pa.retire();
        }
        self.pa_retired_storage = None;
    }
    pub fn mute_pa(&mut self) -> Result<()> {
        if let Some(pa) = &mut self.pa_v2 {
            let result = pa.mute();
            if result != 0 {
                return Err(format!("PA mute {result}"));
            }
        }
        self.pa_hold = true;
        Ok(())
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
        self.source_fault != 0
    }
    pub fn master_eq_status(&self) -> std::result::Result<crate::host::pa_v2::EqStatus, i32> {
        self.pa_v2.as_ref().ok_or(-5)?.eq_status()
    }
    pub fn master_eq_readback(&self) -> Result<String> {
        self.pa_v2
            .as_ref()
            .ok_or("PA unavailable")?
            .eq_readback(self.master_eq_indices()?)
            .map_err(|e| e.to_string())
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
        let pa = self.pa_v2.as_mut().ok_or("live EQ unavailable")?;
        pa.retire_eq(); // controller-only, explicit retirement before admission
        let owner = pa
            .prepare_eq(json, expected, frame)
            .map_err(|e| e.to_string())?;
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
        let storage = self.pa_storage.as_ref().ok_or("PA metadata unavailable")?;
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
            metadata,
            expected_map: expected_map.to_vec(),
        })
    }
    pub fn commit_master_eq(&mut self, p: &mut GraphEqPrepared, epoch: u64, frame: u64) -> i32 {
        if self.source_fault != 0 {
            return -2;
        }
        if epoch != self.epoch || frame != self.next || p.expected_map != self.pa_program_buses() {
            return -4;
        }
        let Some(storage) = self.pa_storage.as_mut() else {
            return -5;
        };
        let Some(pa) = self.pa_v2.as_mut() else {
            return -5;
        };
        let code = pa.commit_eq(&mut p.owner, epoch, frame);
        if code == 0 {
            std::mem::swap(&mut storage.configuration_json, &mut p.metadata);
        }
        code
    }
    pub fn pa_configuration_json(&self) -> Option<&str> {
        self.pa_storage
            .as_ref()
            .map(|s| s.configuration_json.as_str())
    }
    pub fn pa_program_buses(&self) -> &[usize] {
        self.pa_storage
            .as_ref()
            .map_or(&[], |s| s.program_buses.as_slice())
    }
    pub fn pa_v2_capabilities(&self) -> Option<crate::host::pa_v2::Capabilities> {
        self.pa_v2.as_ref().map(|p| p.capabilities())
    }
    pub fn pa_v2_status(&self) -> Option<std::result::Result<crate::host::pa_v2::Status, i32>> {
        self.pa_v2.as_ref().map(|p| p.status())
    }
    pub fn rearm_pa(&mut self) -> Result<()> {
        if self.source_fault != 0 {
            return Err("source fault requires fresh epoch before PA rearm".into());
        }
        if let Some(pa) = &mut self.pa_v2 {
            let r = pa.rearm(self.epoch, self.next);
            if r != 0 {
                return Err(format!("PA rearm {r}"));
            }
        }
        self.pa_hold = false;
        Ok(())
    }
    pub fn output_interleaved(&self) -> &[f64] {
        self.pa_storage
            .as_ref()
            .map_or(&self.output, |s| s.output.as_slice())
    }
    pub fn output(&self) -> &[f64; FRAMES * 6] {
        &self.output
    }
    /// Accepts preparation only. One bounded worker creates the handle and
    /// observer. No physical endpoints and no client-chosen recording paths.
    pub fn start(&mut self, root: &Path, take_id: &str, operation_id: Counter) -> Result<()> {
        if !crate::show::id(take_id) || operation_id.0 == 0 {
            return Err("record identity".into());
        }
        if self.source_fault != 0 {
            return Err("source discontinuity requires explicit recreation".into());
        }
        if self.lifecycle.is_some() || self.take.as_ref().is_some_and(|t| t.recorder.is_some()) {
            return Err("recorder busy".into());
        }
        let path = root.join(take_id);
        let library = self.manifest.rec.library.clone();
        let epoch = self.epoch;
        let channels = self.inputs;
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
            source_epoch: Counter(self.epoch),
            first_source_frame: None,
            mapping: (0..self.inputs)
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
        self.take = Some(Take {
            recorder: None,
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
                worker.cancel_code = 5;
            }
            t.status.state = "finalizing".into();
            return Ok(());
        }
        let r = t.recorder.take().ok_or("already finalized")?;
        t.status.state = "finalizing".into();
        self.finish_worker(r)
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
                    t.recorder = Some(recorder);
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
                t.recorder = Some(r);
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
                    t.status.first_source_frame = Some(Counter(self.next));
                    t.status.state = "recording".into();
                    t.recorder = Some(r);
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
                Self::update(t, p);
                // Owner atomics can lead the host handoff/join. Publish progress
                // while keeping host finalization pending until worker completion.
                if finalizing {
                    t.status.state = "finalizing".into();
                    t.status.outcome = "pending".into();
                }
            }
            self.last_poll = Some(now);
        }
        let mut status = ModuleStatus::unavailable(show, self.epoch, self.next);
        status.available = true;
        status.protection = "owner-sample-limiter-only".into();
        status.configuration = Some(Configuration {
            sample_rate: 48000,
            block_frames: 48,
            raw_inputs: (0..self.inputs)
                .map(|i| format!("input-{:02}/raw", i + 1))
                .collect(),
            fx_mix: "dry-plus-fixed-wet".into(),
            pa_inputs: self.pa_v2.as_ref().map_or(2, |p| p.input_channels() as u32),
            pa_outputs: self
                .pa_v2
                .as_ref()
                .map_or(6, |p| p.output_channels() as u32),
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
            capabilities: self.fx.fx_capabilities().map_err(|e| e.to_string())?,
            status: self.fx.fx_status().map_err(|e| e.to_string())?,
        });
        if let (Some(pa), Some(storage)) = (&self.pa_v2, &self.pa_storage) {
            status.version = 2;
            status.pa_v2 = Some(PaHealthV2 {
                capabilities: pa.capabilities(),
                status: pa.status().map_err(|e| format!("PA status {e}"))?,
                configuration_json: storage.configuration_json.clone(),
                program_buses: storage.program_buses.clone(),
            });
        } else {
            status.pa = Some(PaHealth {
                descriptor: self.pa.pa_descriptor().map_err(|e| e.to_string())?,
                status: self.pa.pa_status().map_err(|e| e.to_string())?,
            });
        }
        if self.inputs != 8 {
            status.version = 2;
        }
        status.source_fault = self.source_fault;
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
        } else if self.source_fault != 0
            || status
                .fx
                .as_ref()
                .is_some_and(|f| f.status.last_process_result != 0)
        {
            "degraded"
        } else if self.pa_hold || status.pa_v2.as_ref().is_some_and(|p| p.status.muted != 0) {
            "muted"
        } else {
            "ready"
        }
        .into();
        status.activity = if self.processed == 0 {
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
        // Explicit control-side shutdown joins only this owned lifecycle worker.
        // A late prepared handle is faulted/finalized by Recorder::drop, never
        // reactivated. All observers/libraries remain retained through cleanup.
        if let Some(worker) = self.lifecycle.take() {
            let _ = worker.thread.join();
            if let Ok(result) = worker.receiver.try_recv() {
                drop(result);
            }
        }
        if let Some(t) = &mut self.take
            && let Some(mut r) = t.recorder.take()
        {
            r.fault(1);
            let _ = r.finish();
        }
    }
}
