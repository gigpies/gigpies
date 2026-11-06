//! Configurable SHR PA owner ABI. No speaker DSP is implemented in this host.
//! Prepare and retire on the controller; process and commit have exclusive ownership.
use super::adapters::Result;
use libloading::Library;
use std::{ffi::c_void, path::Path, sync::Arc};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub version: u32,
    pub size: u32,
    pub sample_rate: u32,
    pub max_block: u32,
    pub input_channels: u32,
    pub output_channels: u32,
    pub muted: u32,
    pub quiesced: u32,
    pub fault_latched: u32,
    pub committed: u32,
    pub reserved0: u32,
    pub reserved1: u32,
    pub epoch: u64,
    pub next_frame: u64,
    pub generation: u64,
    pub applied_frame: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub version: u32,
    pub size: u32,
    pub min_rate: u32,
    pub max_rate: u32,
    pub min_block: u32,
    pub max_block: u32,
    pub max_ports: u32,
    pub max_nodes: u32,
    pub max_operations: u32,
    pub max_delay_samples: u32,
    pub max_json_bytes: u32,
    pub sample_format: u32,
    pub measurement_available: u32,
    pub true_peak_available: u32,
    pub acoustic_protection_available: u32,
    pub physical_io_owned: u32,
}
const _: () = assert!(std::mem::size_of::<Status>() == 80);
const _: () = assert!(std::mem::size_of::<Capabilities>() == 64);
type Handle = *mut c_void;
type Prepare = unsafe extern "C" fn(*const u8, u32, u32) -> Handle;
type Validate = unsafe extern "C" fn(*const u8, u32, *mut u8, u32) -> i32;
type Apply = unsafe extern "C" fn(*mut Handle, Handle, *mut Handle, u64, u64) -> i32;
type Process = unsafe extern "C" fn(Handle, *const f64, *mut f64, u32, u32, u32, u64, u64) -> i32;
type Query = unsafe extern "C" fn(Handle, *mut Status, u32, u32) -> i32;
type Destroy = unsafe extern "C" fn(Handle);
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqStatus {
    pub version: u32,
    pub size: u32,
    pub eligible: u32,
    pub retirement_occupied: u32,
    pub graph_generation: u64,
    pub eq_generation: u64,
    pub epoch: u64,
    pub next_frame: u64,
    pub remaining: u64,
    pub instance: u64,
}
#[repr(C)]
#[derive(Default)]
struct EqCapabilities {
    version: u32,
    size: u32,
    status_size: u32,
    max_json_bytes: u32,
    sections_per_input: u32,
    selected_inputs: u32,
    extra_work_units: u32,
    reserved: u32,
}
struct EqContextApi {
    create: unsafe extern "C" fn() -> Handle,
    destroy: Destroy,
    capture: unsafe extern "C" fn(Handle, Handle, u32, u32) -> i32,
    status: unsafe extern "C" fn(Handle, *mut EqStatus) -> i32,
    prepare: unsafe extern "C" fn(Handle, *const u8, u32, u64) -> Handle,
    readback: unsafe extern "C" fn(Handle, *mut u8, u32) -> i32,
}
impl EqContextApi {
    unsafe fn load(l: &Library) -> Option<Self> {
        unsafe {
            Some(Self {
                create: *l.get(b"shr_pa_eq_context_v1_create").ok()?,
                destroy: *l.get(b"shr_pa_eq_context_v1_destroy").ok()?,
                capture: *l.get(b"shr_pa_eq_context_v1_capture").ok()?,
                status: *l.get(b"shr_pa_eq_context_v1_status").ok()?,
                prepare: *l.get(b"shr_pa_eq_context_v1_prepare").ok()?,
                readback: *l.get(b"shr_pa_eq_context_v1_readback").ok()?,
            })
        }
    }
}
/// Independently owned copied EQ state; never contains a borrowed active handle.
/// Allocate/drop on the controller; transfer exclusively for source capture.
pub struct EqContext {
    handle: Handle,
    api: Arc<Api>,
}
// SAFETY: the context owns only copied fixed-size native state. All accesses are
// serialized by ownership; the Arc keeps its code loaded through destruction.
unsafe impl Send for EqContext {}
impl EqContext {
    pub fn status(&self) -> std::result::Result<EqStatus, i32> {
        let api = self
            .api
            .eq
            .as_ref()
            .and_then(|e| e.context.as_ref())
            .ok_or(-5)?;
        let mut status = EqStatus::default();
        let code = unsafe { (api.status)(self.handle, &mut status) };
        if code == 0 { Ok(status) } else { Err(code) }
    }
    pub fn prepare(&self, json: &[u8], expected: EqStatus, frame: u64) -> Result<PreparedEq> {
        if json.is_empty() || json.len() > 32768 {
            return Err("EQ JSON bound".into());
        }
        let current = self
            .status()
            .map_err(|e| format!("EQ context status {e}"))?;
        if current.instance != expected.instance
            || current.graph_generation != expected.graph_generation
            || current.eq_generation != expected.eq_generation
            || current.epoch != expected.epoch
        {
            return Err("stale EQ identity".into());
        }
        let api = self
            .api
            .eq
            .as_ref()
            .and_then(|e| e.context.as_ref())
            .ok_or("detached EQ unavailable")?;
        let handle = unsafe { (api.prepare)(self.handle, json.as_ptr(), json.len() as u32, frame) };
        if handle.is_null() {
            return Err("EQ owner preparation refused".into());
        }
        Ok(PreparedEq {
            handle,
            api: Arc::clone(&self.api),
        })
    }
    pub fn readback(&self) -> Result<String> {
        let api = self
            .api
            .eq
            .as_ref()
            .and_then(|e| e.context.as_ref())
            .ok_or("detached EQ unavailable")?;
        let mut bytes = vec![0; 65536];
        let code = unsafe { (api.readback)(self.handle, bytes.as_mut_ptr(), bytes.len() as u32) };
        if code <= 0 {
            return Err(format!("EQ context readback {code}").into());
        }
        bytes.truncate(code as usize);
        String::from_utf8(bytes).map_err(Into::into)
    }
}
impl Drop for EqContext {
    fn drop(&mut self) {
        if let Some(api) = self.api.eq.as_ref().and_then(|e| e.context.as_ref()) {
            unsafe { (api.destroy)(self.handle) };
        }
    }
}
struct EqApi {
    context: Option<EqContextApi>,
    status: unsafe extern "C" fn(Handle, *mut EqStatus, u32, u32) -> i32,
    prepare: unsafe extern "C" fn(Handle, *const u8, u32, u32, u64, u64, u64, u64) -> Handle,
    apply: unsafe extern "C" fn(Handle, *mut Handle, u64, u64) -> i32,
    retire: unsafe extern "C" fn(Handle, *mut Handle) -> i32,
    destroy: Destroy,
    readback: unsafe extern "C" fn(Handle, u32, u32, *mut u8, u32) -> i32,
}
impl EqApi {
    unsafe fn load(l: &Library) -> Option<Self> {
        // Missing, partial and incompatible extensions all retain legacy loading.
        unsafe {
            let caps = *l
                .get::<unsafe extern "C" fn(*mut EqCapabilities, u32, u32) -> i32>(
                    b"shr_pa_eq_v1_capabilities",
                )
                .ok()?;
            let mut c = EqCapabilities::default();
            if caps(&mut c, 1, 32) != 0
                || c.version != 1
                || c.size != 32
                || c.status_size != 64
                || c.max_json_bytes != 32768
                || c.sections_per_input != 39
                || c.selected_inputs != 2
                || c.extra_work_units != 80
                || c.reserved != 0
            {
                return None;
            }
            Some(Self {
                context: EqContextApi::load(l),
                status: *l.get(b"shr_pa_eq_v1_status").ok()?,
                prepare: *l.get(b"shr_pa_eq_v1_prepare").ok()?,
                apply: *l.get(b"shr_pa_eq_v1_apply").ok()?,
                retire: *l.get(b"shr_pa_eq_v1_retire").ok()?,
                destroy: *l.get(b"shr_pa_eq_v1_destroy").ok()?,
                readback: *l.get(b"shr_pa_eq_v1_readback").ok()?,
            })
        }
    }
}
pub struct PreparedEq {
    handle: Handle,
    api: Arc<Api>,
}
// SAFETY: exclusively owned native token; no thread-affine data or active borrow.
unsafe impl Send for PreparedEq {}
impl PreparedEq {
    pub fn is_empty(&self) -> bool {
        self.handle.is_null()
    }
}
impl Drop for PreparedEq {
    fn drop(&mut self) {
        if let Some(eq) = &self.api.eq {
            unsafe {
                (eq.destroy)(self.handle);
            }
        }
    }
}
struct Api {
    eq: Option<EqApi>,
    prepare: Prepare,
    validate: Validate,
    apply: Apply,
    process: Process,
    status: Query,
    destroy: Destroy,
    mute: unsafe extern "C" fn(Handle) -> i32,
    rearm: unsafe extern "C" fn(Handle, u64, u64) -> i32,
    capabilities: Capabilities,
    _library: Library,
}
/// A prepared owner allocation. Drop only outside rendering, including rejected edits.
pub struct Prepared {
    handle: Handle,
    api: Arc<Api>,
}
// SAFETY: exclusively owned graph state, with library lifetime retained.
unsafe impl Send for Prepared {}
impl Prepared {
    pub fn is_empty(&self) -> bool {
        self.handle.is_null()
    }
    /// Exclusive read of the fresh owner shape, before host buffer admission.
    pub fn status(&self) -> std::result::Result<Status, i32> {
        if self.handle.is_null() {
            return Err(-1);
        }
        let mut status = Status::default();
        let result = unsafe { (self.api.status)(self.handle, &mut status, 2, 80) };
        if result == 0 { Ok(status) } else { Err(result) }
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the pointer or it was consumed and is null.
        unsafe { (self.api.destroy)(self.handle) };
    }
}
/// Controller-owned library API. Preparing a new graph never reads the source
/// graph, and tokens retain the exact API identity required by boundary commit.
pub struct PaPreparation {
    api: Arc<Api>,
}
impl PaPreparation {
    pub fn detached_eq_available(&self) -> bool {
        self.api.eq.as_ref().is_some_and(|e| e.context.is_some())
    }
    /// Allocation, parsing and coefficient design occur only here in the owner.
    pub fn prepare(&self, json: &[u8]) -> Result<Prepared> {
        if json.is_empty() || json.len() > self.api.capabilities.max_json_bytes as usize {
            return Err("SHR PA v2 JSON resource bound".into());
        }
        let mut reason = [0u8; 512];
        // SAFETY: disjoint live input/error storage; owner copies configuration.
        unsafe {
            let result = (self.api.validate)(
                json.as_ptr(),
                json.len() as u32,
                reason.as_mut_ptr(),
                reason.len() as u32,
            );
            if result != 0 {
                let len = reason.iter().position(|v| *v == 0).unwrap_or(reason.len());
                return Err(format!(
                    "SHR PA v2 preparation refused: {}",
                    String::from_utf8_lossy(&reason[..len])
                )
                .into());
            }
            let handle = (self.api.prepare)(json.as_ptr(), json.len() as u32, 2);
            if handle.is_null() {
                return Err("SHR PA v2 preparation failed".into());
            }
            Ok(Prepared {
                handle,
                api: Arc::clone(&self.api),
            })
        }
    }
    pub fn create_eq_context(&self) -> Result<EqContext> {
        let api = self
            .api
            .eq
            .as_ref()
            .and_then(|e| e.context.as_ref())
            .ok_or("detached EQ unavailable")?;
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("EQ context allocation refused".into());
        }
        Ok(EqContext {
            handle,
            api: Arc::clone(&self.api),
        })
    }
    pub fn retirement_slot(&self) -> Prepared {
        Prepared {
            handle: std::ptr::null_mut(),
            api: Arc::clone(&self.api),
        }
    }
    pub fn eq_retirement_slot(&self) -> PreparedEq {
        PreparedEq {
            handle: std::ptr::null_mut(),
            api: Arc::clone(&self.api),
        }
    }
}
/// One active handle and one reserved retired slot. A full slot rejects the next
/// commit until the controller calls retire(); no render-side destruction occurs.
pub struct Pa {
    active: Handle,
    retired: Handle,
    api: Arc<Api>,
    inputs: usize,
    outputs: usize,
    max_block: usize,
}
// SAFETY: PA ABI permits exclusive ownership transfer. Methods serialize native
// access; this does not permit concurrent prepare/query/process on one handle.
unsafe impl Send for Pa {}
impl Pa {
    /// Load an explicitly hash-verified trusted owner library off the render path.
    pub fn load(path: &Path, json: &[u8], epoch: u64, frame: u64) -> Result<Self> {
        let mut pa = Self::open(path)?;
        let mut prepared = pa.prepare(json)?;
        let result = pa.commit(&mut prepared, epoch, frame);
        if result != 0 {
            return Err(format!("SHR PA v2 initial commit refused: {result}").into());
        }
        Ok(pa)
    }
    /// Load the ABI without an active graph. Initial prepared ownership is
    /// committed later at the actual render boundary, not preparation time.
    pub fn open(path: &Path) -> Result<Self> {
        // SAFETY: caller selects trusted library; all names/layouts match SHR PA v2.
        let api = unsafe {
            let library = Library::new(path)?;
            let mut capabilities = Capabilities::default();
            let query = *library.get::<unsafe extern "C" fn(*mut Capabilities, u32, u32) -> i32>(
                b"shr_pa_v2_capabilities",
            )?;
            if query(&mut capabilities, 2, 64) != 0
                || capabilities.version != 2
                || capabilities.size != 64
                || capabilities.sample_format != 1
            {
                return Err("SHR PA v2 capabilities mismatch".into());
            }
            Arc::new(Api {
                eq: EqApi::load(&library),
                prepare: *library.get::<Prepare>(b"shr_pa_v2_prepare")?,
                validate: *library.get::<Validate>(b"shr_pa_v2_validate")?,
                apply: *library.get::<Apply>(b"shr_pa_v2_apply")?,
                process: *library.get::<Process>(b"shr_pa_v2_process")?,
                status: *library.get::<Query>(b"shr_pa_v2_status")?,
                destroy: *library.get::<Destroy>(b"shr_pa_v2_destroy")?,
                mute: *library.get(b"shr_pa_v2_mute")?,
                rearm: *library.get(b"shr_pa_v2_rearm")?,
                capabilities,
                _library: library,
            })
        };
        Ok(Self {
            active: std::ptr::null_mut(),
            retired: std::ptr::null_mut(),
            api,
            inputs: 0,
            outputs: 0,
            max_block: 0,
        })
    }
    pub fn eq_status(&self) -> std::result::Result<EqStatus, i32> {
        let eq = self.api.eq.as_ref().ok_or(-5)?;
        let mut s = EqStatus::default();
        let code = unsafe { (eq.status)(self.active, &mut s, 1, 64) };
        if code != 0 {
            return Err(code);
        }
        if s.version != 1 || s.size != 64 || s.eligible > 1 || s.retirement_occupied > 1 {
            return Err(-5);
        }
        Ok(s)
    }
    pub fn detached_eq_available(&self) -> bool {
        self.api.eq.as_ref().is_some_and(|e| e.context.is_some())
    }
    pub fn create_eq_context(&self) -> Result<EqContext> {
        let api = self
            .api
            .eq
            .as_ref()
            .and_then(|e| e.context.as_ref())
            .ok_or("detached EQ unavailable")?;
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("EQ context allocation refused".into());
        }
        Ok(EqContext {
            handle,
            api: Arc::clone(&self.api),
        })
    }
    /// Source-only bounded copy into already allocated exclusively owned storage.
    pub fn capture_eq_context(&self, context: &mut EqContext, indices: [usize; 2]) -> i32 {
        if !Arc::ptr_eq(&self.api, &context.api) || indices.iter().any(|i| *i > u32::MAX as usize) {
            return -1;
        }
        let Some(api) = self.api.eq.as_ref().and_then(|e| e.context.as_ref()) else {
            return -5;
        };
        unsafe {
            (api.capture)(
                self.active,
                context.handle,
                indices[0] as u32,
                indices[1] as u32,
            )
        }
    }
    pub fn prepare_eq(&self, json: &[u8], expected: EqStatus, frame: u64) -> Result<PreparedEq> {
        if json.is_empty() || json.len() > 32768 {
            return Err("EQ JSON bound".into());
        }
        if self.detached_eq_available() {
            let patch: serde_json::Value = serde_json::from_slice(json)?;
            let inputs = patch["inputs"].as_array().ok_or("EQ inputs")?;
            if inputs.len() != 2 {
                return Err("EQ inputs".into());
            }
            let first = inputs[0]["input_index"].as_u64().ok_or("EQ input index")?;
            let second = inputs[1]["input_index"].as_u64().ok_or("EQ input index")?;
            let indices = [usize::try_from(first)?, usize::try_from(second)?];
            let mut context = self.create_eq_context()?;
            let code = self.capture_eq_context(&mut context, indices);
            if code != 0 {
                return Err(format!("EQ context capture {code}").into());
            }
            return context.prepare(json, expected, frame);
        }
        let eq = self.api.eq.as_ref().ok_or("live EQ unavailable")?;
        let current = self.eq_status().map_err(|e| format!("EQ status {e}"))?;
        if current.instance != expected.instance
            || current.graph_generation != expected.graph_generation
            || current.eq_generation != expected.eq_generation
            || current.epoch != expected.epoch
        {
            return Err("stale EQ identity".into());
        }
        let handle = unsafe {
            (eq.prepare)(
                self.active,
                json.as_ptr(),
                json.len() as u32,
                1,
                expected.graph_generation,
                expected.eq_generation,
                expected.epoch,
                frame,
            )
        };
        if handle.is_null() {
            return Err("EQ owner preparation refused".into());
        }
        Ok(PreparedEq {
            handle,
            api: Arc::clone(&self.api),
        })
    }
    /// Narrow boundary operation; preserves buffers and PA hold/arm state.
    pub fn commit_eq(&mut self, p: &mut PreparedEq, epoch: u64, frame: u64) -> i32 {
        if !Arc::ptr_eq(&self.api, &p.api) || p.handle.is_null() {
            return -1;
        }
        let Some(eq) = &self.api.eq else {
            return -5;
        };
        unsafe { (eq.apply)(self.active, &mut p.handle, epoch, frame) }
    }
    /// Allocate the host ownership slot before source execution.
    pub fn eq_retirement_slot(&self) -> PreparedEq {
        PreparedEq {
            handle: std::ptr::null_mut(),
            api: Arc::clone(&self.api),
        }
    }
    /// Bounded transfer only. Caller reserves a controller retirement lane before
    /// calling; occupied slots are refused and never destroyed here.
    pub fn take_retired_eq(&mut self, into: &mut PreparedEq) -> i32 {
        if !into.handle.is_null() || !Arc::ptr_eq(&self.api, &into.api) {
            return -3;
        }
        let Some(eq) = &self.api.eq else {
            return -5;
        };
        unsafe { (eq.retire)(self.active, &mut into.handle) }
    }
    /// Synchronous compatibility disposal; controller-only.
    pub fn retire_eq(&mut self) {
        let mut slot = self.eq_retirement_slot();
        let _ = self.take_retired_eq(&mut slot);
    }
    pub fn eq_readback(&self, indices: [usize; 2]) -> Result<String> {
        if self.detached_eq_available() {
            let mut context = self.create_eq_context()?;
            let code = self.capture_eq_context(&mut context, indices);
            if code != 0 {
                return Err(format!("EQ context capture {code}").into());
            }
            return context.readback();
        }
        let eq = self.api.eq.as_ref().ok_or("live EQ unavailable")?;
        let mut bytes = vec![0; 65536];
        let code = unsafe {
            (eq.readback)(
                self.active,
                indices[0] as u32,
                indices[1] as u32,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
            )
        };
        if code <= 0 {
            return Err(format!("EQ readback {code}").into());
        }
        bytes.truncate(code as usize);
        String::from_utf8(bytes).map_err(Into::into)
    }
    pub fn capabilities(&self) -> Capabilities {
        self.api.capabilities
    }
    pub fn input_channels(&self) -> usize {
        self.inputs
    }
    pub fn output_channels(&self) -> usize {
        self.outputs
    }
    /// Controller-only API ownership; no active graph is borrowed by preparation.
    pub fn preparation(&self) -> PaPreparation {
        PaPreparation {
            api: Arc::clone(&self.api),
        }
    }
    pub fn prepare(&self, json: &[u8]) -> Result<Prepared> {
        self.preparation().prepare(json)
    }
    /// Boundary-only ownership swap. On refusal prepared and active remain owned.
    /// No allocation, deallocation or refcount change; retire() is controller-only.
    pub fn commit(&mut self, prepared: &mut Prepared, epoch: u64, frame: u64) -> i32 {
        if prepared.handle.is_null() || !Arc::ptr_eq(&self.api, &prepared.api) {
            return -1;
        }
        if !self.retired.is_null() {
            return -3;
        }
        // Query the fresh state before swapping, so shape is known without fallible
        // work after ownership has transferred.
        let mut status = Status::default();
        unsafe {
            let result = (self.api.status)(prepared.handle, &mut status, 2, 80);
            if result != 0 {
                return result;
            }
            let result = (self.api.apply)(
                &mut self.active,
                prepared.handle,
                &mut self.retired,
                epoch,
                frame,
            );
            if result == 0 {
                prepared.handle = std::ptr::null_mut();
                self.inputs = status.input_channels as usize;
                self.outputs = status.output_channels as usize;
                self.max_block = status.max_block as usize;
            }
            result
        }
    }
    /// Bounded exclusive ownership transfer into controller-prepared storage.
    /// Neither native destruction nor an Arc operation occurs on this path.
    pub fn take_retired(&mut self, into: &mut Prepared) -> i32 {
        if !Arc::ptr_eq(&self.api, &into.api) {
            return -1;
        }
        if !into.handle.is_null() {
            return -3;
        }
        std::mem::swap(&mut self.retired, &mut into.handle);
        0
    }
    /// Controller only, after exclusive render use has returned.
    pub fn retire(&mut self) {
        unsafe { (self.api.destroy)(self.retired) };
        self.retired = std::ptr::null_mut();
    }
    pub fn status(&self) -> std::result::Result<Status, i32> {
        let mut status = Status::default();
        let result = unsafe { (self.api.status)(self.active, &mut status, 2, 80) };
        if result == 0 { Ok(status) } else { Err(result) }
    }
    pub fn mute(&mut self) -> i32 {
        unsafe { (self.api.mute)(self.active) }
    }
    pub fn rearm(&mut self, epoch: u64, frame: u64) -> i32 {
        unsafe { (self.api.rearm)(self.active, epoch, frame) }
    }
    pub fn process(&mut self, input: &[f64], output: &mut [f64], epoch: u64, frame: u64) -> i32 {
        if self.inputs == 0 || input.is_empty() || !input.len().is_multiple_of(self.inputs) {
            return -1;
        }
        let frames = input.len() / self.inputs;
        if frames > self.max_block || frames.checked_mul(self.outputs) != Some(output.len()) {
            return -1;
        }
        // SAFETY: exclusive active state, prepared exact shapes and disjoint Rust slices.
        unsafe {
            (self.api.process)(
                self.active,
                input.as_ptr(),
                output.as_mut_ptr(),
                self.inputs as u32,
                self.outputs as u32,
                frames as u32,
                epoch,
                frame,
            )
        }
    }
}
impl Drop for Pa {
    fn drop(&mut self) {
        self.retire();
        unsafe { (self.api.destroy)(self.active) };
    }
}

#[cfg(test)]
mod detached_tests {
    use super::*;
    #[test]
    #[ignore = "actual PA library and stereo4way fixture: GP_PA_LIBRARY, GP_PA_CONFIG"]
    fn actual_detached_preparation_during_render_and_transferred_retirement() {
        let path = std::env::var_os("GP_PA_LIBRARY").unwrap();
        let config = std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap();
        let mut pa = Pa::load(Path::new(&path), &config, 1, 0).unwrap();
        assert!(pa.detached_eq_available());
        let mut context = pa.create_eq_context().unwrap();
        let mut retirement = pa.eq_retirement_slot();
        crate::remote::assert_no_allocations(|| {
            assert_eq!(pa.capture_eq_context(&mut context, [0, 1]), 0);
        });
        let expected = context.status().unwrap();
        let readback = context.readback().unwrap();
        let value: serde_json::Value = serde_json::from_str(&readback).unwrap();
        let mut patch = serde_json::json!({"version":1,"inputs":value["target"]});
        patch["inputs"][0]["eq"][0]["db"] = serde_json::json!(3.0);
        let patch = serde_json::to_vec(&patch).unwrap();
        // Exercise the production synchronous caller's optional-interface route
        // as well as the explicitly detached API used across ownership transfer.
        assert_eq!(pa.eq_readback([0, 1]).unwrap(), readback);
        let production_token = pa.prepare_eq(&patch, expected, 48000).unwrap();
        drop(production_token);
        let count = pa.output_channels();
        let mut output = vec![0.; count * 48];
        let worker = std::thread::spawn(move || {
            for block in 0..1000 {
                crate::remote::assert_no_allocations(|| {
                    assert_eq!(pa.process(&[0.; 96], &mut output, 1, block * 48), 0);
                });
            }
            pa
        });
        for _ in 0..20 {
            let _prepared = context.prepare(&patch, expected, 48000).unwrap();
            assert_eq!(context.readback().unwrap(), readback);
        }
        let mut token = context.prepare(&patch, expected, 48000).unwrap();
        let mut stale = context.prepare(&patch, expected, 48240).unwrap();
        let mut pa = worker.join().unwrap();
        let mut output = vec![0.; count * 48];
        crate::remote::assert_no_allocations(|| {
            assert_eq!(pa.commit_eq(&mut token, 1, 48000), 0);
            for block in 1000..1005 {
                assert_eq!(pa.process(&[0.; 96], &mut output, 1, block * 48), 0);
            }
            assert_eq!(pa.take_retired_eq(&mut retirement), 0);
            assert_eq!(pa.take_retired_eq(&mut retirement), -3);
            assert_eq!(pa.commit_eq(&mut stale, 1, 48240), -4);
        });
        assert_eq!(context.readback().unwrap(), readback);
        // Controller owns and destroys the retired native allocation after join.
        drop(retirement);
    }
}
