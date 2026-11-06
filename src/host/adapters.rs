//! Independent owner libraries. Symbols are versioned; handles never cross owners.
//! Loading and destruction occur outside the render section. Trusted local libraries
//! are executable code; the CLI requires explicit paths and never searches siblings.
use libloading::Library;
use std::{
    ffi::{CString, c_void},
    path::Path,
    sync::Arc,
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Process = unsafe extern "C" fn(*mut c_void, *const f64, *mut f64, u32) -> i32;
type Destroy = unsafe extern "C" fn(*mut c_void);
pub struct Dsp {
    handle: *mut c_void,
    process: Process,
    destroy: Destroy,
    reset: Option<unsafe extern "C" fn(*mut c_void)>,
    fx_status: Option<unsafe extern "C" fn(*mut c_void, *mut FxStatus, u32, u32) -> i32>,
    pa_status: Option<unsafe extern "C" fn(*mut c_void, *mut PaStatus, u32, u32) -> i32>,
    pub delay: u32,
    outputs: usize,
    max_block: usize,
    _library: Library,
}
// SAFETY: the accepted PA v1 and FX v1 ABIs create exclusively owned handles
// on the controller for use by one audio worker. Their retained DSP has no
// thread-affine state; all process/reset/query/destroy calls remain serialized
// by ownership. Library stays loaded through handle destruction. This permits
// transfer, not concurrent access (Dsp deliberately does not implement Sync).
unsafe impl Send for Dsp {}
impl Dsp {
    /// Loads an explicitly selected trusted owner library. All ABI signatures must
    /// match the owner's v1 contract; callers cannot supply untrusted plugin code.
    pub fn load(path: &Path, owner: &str, block: u32) -> Result<Self> {
        if !matches!(owner, "pa" | "fx") {
            return Err("unknown DSP owner".into());
        }
        // SAFETY: versioned symbols below have the documented matching owner ABI.
        unsafe {
            let library = Library::new(path)?;
            let create = *library.get::<unsafe extern "C" fn(u32, u32) -> *mut c_void>(
                format!("shr_{owner}_v1_create").as_bytes(),
            )?;
            let process = *library.get::<Process>(format!("shr_{owner}_v1_process").as_bytes())?;
            let destroy = *library.get::<Destroy>(format!("shr_{owner}_v1_destroy").as_bytes())?;
            let reset = if owner == "fx" {
                Some(*library.get::<unsafe extern "C" fn(*mut c_void)>(b"shr_fx_v1_reset")?)
            } else {
                None
            };
            let delay_fn =
                if owner == "fx" {
                    Some(*library.get::<unsafe extern "C" fn(*mut c_void) -> u32>(
                        b"shr_fx_v1_delay_frames",
                    )?)
                } else {
                    None
                };
            // Resolve status functions before handing the owner to the source.
            let fx_status = if owner == "fx" {
                Some(
                    *library
                        .get::<unsafe extern "C" fn(*mut c_void, *mut FxStatus, u32, u32) -> i32>(
                            b"shr_fx_v1_status",
                        )?,
                )
            } else {
                None
            };
            let pa_status = if owner == "pa" {
                Some(
                    *library
                        .get::<unsafe extern "C" fn(*mut c_void, *mut PaStatus, u32, u32) -> i32>(
                            b"shr_pa_v1_status",
                        )?,
                )
            } else {
                None
            };
            let handle = create(48000, block);
            if handle.is_null() {
                return Err("DSP create refused configuration".into());
            }
            let delay = delay_fn.map_or(0, |f| f(handle));
            Ok(Self {
                handle,
                process,
                destroy,
                reset,
                fx_status,
                pa_status,
                delay,
                outputs: if owner == "pa" { 6 } else { 2 },
                max_block: block as usize,
                _library: library,
            })
        }
    }
    pub fn process(&mut self, input: &[f64], output: &mut [f64], outputs: usize) -> bool {
        self.process_result(input, output, outputs) == 0
    }
    /// Exact owner result; shape refusal never calls the owner.
    pub fn process_result(&mut self, input: &[f64], output: &mut [f64], outputs: usize) -> i32 {
        if input.is_empty()
            || !input.len().is_multiple_of(2)
            || output.len() != input.len() / 2 * outputs
            || outputs != self.outputs
            || input.len() / 2 > self.max_block
        {
            return -1;
        }
        // SAFETY: live exclusive handle, complete buffers; owner checks max frames.
        unsafe {
            (self.process)(
                self.handle,
                input.as_ptr(),
                output.as_mut_ptr(),
                (input.len() / 2) as u32,
            )
        }
    }
    pub fn reset(&mut self) {
        if let Some(reset) = self.reset {
            unsafe {
                reset(self.handle);
            }
        }
    }
}
impl Drop for Dsp {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.handle);
        }
    }
}

type Push = unsafe extern "C" fn(*mut c_void, u64, *const f64, u32) -> i32;
type Finish = unsafe extern "C" fn(*mut c_void) -> i32;
pub struct Recorder {
    handle: *mut c_void,
    push: Push,
    finish: Finish,
    fault: unsafe extern "C" fn(*mut c_void, u32),
    channels: usize,
    max_block: usize,
    _library: Arc<Library>,
}
impl Recorder {
    pub fn create(path: &Path, directory: &Path, block: u32, epoch: u64) -> Result<Self> {
        Self::create_configured(path, directory, block, epoch, 8)
    }
    /// Prepare the unchanged owner ABI for the admitted raw source count.
    /// The owner's current 64-track storage bound is explicit; it is not a mixer cap.
    pub fn create_configured(
        path: &Path,
        directory: &Path,
        block: u32,
        epoch: u64,
        channels: usize,
    ) -> Result<Self> {
        if !(1..=64).contains(&channels) {
            return Err("SHR REC v1 admission: channels must be 1..64".into());
        }
        if !(1..=8192).contains(&block) || epoch == 0 {
            return Err("SHR REC v1 admission: block1..8192 and nonzero epoch required".into());
        }
        let queue_bytes = channels
            .checked_mul(block as usize)
            .and_then(|n| n.checked_mul(256))
            .and_then(|n| n.checked_mul(8))
            .ok_or("SHR REC queue size overflow")?;
        if queue_bytes > 64 * 1024 * 1024 {
            return Err(
                "SHR REC v1 admission:256-block queue exceeds64MiB; reduce block size explicitly"
                    .into(),
            );
        }
        let name = CString::new(directory.as_os_str().as_encoded_bytes())?;
        unsafe {
            let library = Library::new(path)?;
            let create = *library.get::<unsafe extern "C" fn(
                *const std::ffi::c_char,
                u32,
                u32,
                u32,
                u32,
                u64,
            ) -> *mut c_void>(b"shr_rec_v1_create")?;
            let push = *library.get::<Push>(b"shr_rec_v1_push")?;
            let finish = *library.get::<Finish>(b"shr_rec_v1_finish")?;
            let fault =
                *library.get::<unsafe extern "C" fn(*mut c_void, u32)>(b"shr_rec_v1_mark_fault")?;
            let source_bits = *library.get::<unsafe extern "C" fn(*mut c_void, u32) -> i32>(
                b"shr_rec_v1_set_source_bits",
            )?;
            let handle = create(name.as_ptr(), 48000, channels as u32, block, 256, epoch);
            if handle.is_null() {
                return Err("recorder refused new take".into());
            }
            if source_bits(handle, 24) != 0 {
                fault(handle, 3);
                finish(handle);
                return Err("recorder source precision metadata rejected".into());
            }
            Ok(Self {
                handle,
                push,
                finish,
                fault,
                channels,
                max_block: block as usize,
                _library: Arc::new(library),
            })
        }
    }
    pub fn push(&mut self, frame: u64, samples: &[f64]) -> i32 {
        if samples.is_empty()
            || !samples.len().is_multiple_of(self.channels)
            || samples.len() / self.channels > self.max_block
        {
            return -1;
        }
        unsafe {
            (self.push)(
                self.handle,
                frame,
                samples.as_ptr(),
                (samples.len() / self.channels) as u32,
            )
        }
    }
    pub fn fault(&mut self, code: u32) {
        unsafe {
            (self.fault)(self.handle, code);
        }
    }
    pub fn finish(mut self) -> i32 {
        let result = unsafe { (self.finish)(self.handle) };
        self.handle = std::ptr::null_mut();
        result
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            self.fault(1);
            unsafe {
                (self.finish)(self.handle);
            }
        }
    }
}

// Exact additive fixed-width layouts from accepted owner headers. No DSP is
// implemented here. These queries are control-side and serialized with processing.
#[repr(C)]
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FxCapabilities {
    pub version: u32,
    pub size: u32,
    pub identity: [i8; 32],
    pub min_sample_rate: u32,
    pub max_sample_rate: u32,
    pub min_block_frames: u32,
    pub max_block_frames: u32,
    pub channels: u32,
    pub sample_bits: u32,
    pub reset_supported: u32,
    pub writable_parameters: u32,
    pub rack_available: u32,
    pub adapter_buffer_frames: u32,
    pub delay_ms: f64,
    pub feedback: f64,
    pub damping: f64,
    pub wet_gain: f64,
}
#[repr(C)]
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FxStatus {
    pub version: u32,
    pub size: u32,
    pub sample_rate: u32,
    pub max_block_frames: u32,
    pub intentional_delay_frames: u32,
    pub adapter_buffer_frames: u32,
    pub last_process_result: i32,
    pub reset_reason: u32,
    pub reset_count: u64,
}
#[repr(C)]
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaDescriptor {
    pub version: u32,
    pub size: u32,
    pub input_channels: u32,
    pub logical_outputs: u32,
    pub active_output_mask: u32,
    pub silent_output_mask: u32,
    pub physical_io_owned: u32,
    pub sample_format: u32,
    pub fixed_preset: u32,
    pub limiter_kind: u32,
    pub limiter_threshold_millidbfs: i32,
    pub limiter_linked: u32,
    pub limiter_release_ms: u32,
    pub startup_ramp_ms: u32,
    pub fixed_delay_frames: u32,
    pub min_rate: u32,
    pub max_rate: u32,
    pub min_block: u32,
    pub max_block: u32,
    pub unavailable_capabilities: u32,
}
#[repr(C)]
#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaStatus {
    pub version: u32,
    pub size: u32,
    pub sample_rate: u32,
    pub max_block: u32,
    pub fault_latched: u32,
    pub recreate_required: u32,
}
impl Dsp {
    fn descriptor<T: Default>(&self, name: &[u8]) -> Result<T> {
        let mut out = T::default();
        // SAFETY: names are selected internally with matching accepted repr(C)
        // layouts. Output is live, aligned and disjoint from the exclusive owner.
        unsafe {
            let query = self
                ._library
                .get::<unsafe extern "C" fn(*mut T, u32, u32) -> i32>(name)?;
            let rc = query(&mut out, 1, std::mem::size_of::<T>() as u32);
            if rc != 0 {
                return Err(format!("owner descriptor refused: {rc}").into());
            }
        }
        Ok(out)
    }
    /// Fixed-width source observation. Symbol lookup and error formatting stay
    /// outside the source quantum; exclusive ownership serializes native access.
    pub(crate) fn fx_status_numeric(&self) -> std::result::Result<FxStatus, i32> {
        let query = self.fx_status.ok_or(-1)?;
        let mut out = FxStatus::default();
        // SAFETY: cached accepted ABI, live exclusive owner and disjoint output.
        let rc = unsafe {
            query(
                self.handle,
                &mut out,
                1,
                std::mem::size_of::<FxStatus>() as u32,
            )
        };
        if rc == 0 { Ok(out) } else { Err(rc) }
    }
    pub(crate) fn pa_status_numeric(&self) -> std::result::Result<PaStatus, i32> {
        let query = self.pa_status.ok_or(-1)?;
        let mut out = PaStatus::default();
        // SAFETY: cached accepted ABI, live exclusive owner and disjoint output.
        let rc = unsafe {
            query(
                self.handle,
                &mut out,
                1,
                std::mem::size_of::<PaStatus>() as u32,
            )
        };
        if rc == 0 { Ok(out) } else { Err(rc) }
    }
    pub fn fx_capabilities(&self) -> Result<FxCapabilities> {
        if self.outputs != 2 {
            return Err("wrong owner query".into());
        }
        self.descriptor(b"shr_fx_v1_capabilities")
    }
    pub fn fx_status(&self) -> Result<FxStatus> {
        if self.outputs != 2 {
            return Err("wrong owner query".into());
        }
        self.fx_status_numeric()
            .map_err(|rc| format!("owner status refused: {rc}").into())
    }
    pub fn pa_descriptor(&self) -> Result<PaDescriptor> {
        if self.outputs != 6 {
            return Err("wrong owner query".into());
        }
        self.descriptor(b"shr_pa_v1_descriptor")
    }
    pub fn pa_status(&self) -> Result<PaStatus> {
        if self.outputs != 6 {
            return Err("wrong owner query".into());
        }
        self.pa_status_numeric()
            .map_err(|rc| format!("owner status refused: {rc}").into())
    }
}
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct RecorderProgress {
    pub version: u32,
    pub state: u32,
    pub outcome: u32,
    pub writer_fault: u32,
    pub host_fault: u32,
    pub reserved: u32,
    pub epoch: u64,
    pub accepted_frames: u64,
    pub written_frames: u64,
    pub durable_frames: u64,
    pub dropped_frames: u64,
    pub overflow_blocks: u64,
    pub gap_frames: u64,
    pub invalid_blocks: u64,
    pub clipped_samples: u64,
}
pub struct RecorderObserver {
    handle: *mut c_void,
    snapshot: unsafe extern "C" fn(*const c_void, *mut RecorderProgress, u32) -> i32,
    release: Destroy,
    _library: Arc<Library>,
}
impl Recorder {
    /// Producer is quiesced by exclusive borrow; creation is off callback.
    pub fn observer(&mut self) -> Result<RecorderObserver> {
        unsafe {
            let create = self
                ._library
                .get::<unsafe extern "C" fn(*mut c_void) -> *mut c_void>(
                    b"shr_rec_v2_observer_create",
                )?;
            let snapshot = *self._library.get::<unsafe extern "C" fn(
                *const c_void,
                *mut RecorderProgress,
                u32,
            ) -> i32>(b"shr_rec_v2_observer_snapshot")?;
            let release = *self
                ._library
                .get::<Destroy>(b"shr_rec_v2_observer_release")?;
            let handle = create(self.handle);
            if handle.is_null() {
                return Err("observer creation refused".into());
            }
            Ok(RecorderObserver {
                handle,
                snapshot,
                release,
                _library: Arc::clone(&self._library),
            })
        }
    }
}
impl RecorderObserver {
    /// Call off callback, at most 10 Hz. Library retained through terminal query.
    pub fn snapshot(&self) -> Result<RecorderProgress> {
        let mut out = RecorderProgress::default();
        let rc = unsafe {
            (self.snapshot)(
                self.handle,
                &mut out,
                std::mem::size_of::<RecorderProgress>() as u32,
            )
        };
        if rc != 0 || out.version != 2 {
            return Err(format!("observer query refused: {rc}").into());
        }
        Ok(out)
    }
}
impl Drop for RecorderObserver {
    fn drop(&mut self) {
        unsafe { (self.release)(self.handle) };
    }
}

// Owner ABI requires exclusive use, not thread affinity. These handles are moved
// only after producer quiescence; no process/query/release races are permitted.
unsafe impl Send for Recorder {}
// Independent observer atomics support a moved, retained handle; release remains
// exclusive to its owning control thread and never races snapshot.
unsafe impl Send for RecorderObserver {}
