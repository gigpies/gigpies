//! Serialized native-f64 SHR FX palette owner. No DSP is duplicated here.
use libloading::Library;
use serde::{Deserialize, Serialize};
use std::{ffi::c_void, path::Path, sync::Arc};
type Result<T> = std::result::Result<T, String>;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub size: u32,
    pub algorithm: u32,
    pub bypass: u32,
    pub time_ms: f64,
    pub amount: f64,
    pub damping: f64,
    pub rate_hz: f64,
    pub gain: f64,
    pub seed: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Serialize)]
pub struct Capabilities {
    pub version: u32,
    pub size: u32,
    pub algorithm_mask: u32,
    pub sample_bits: u32,
    pub min_rate: u32,
    pub max_rate: u32,
    pub max_block: u32,
    pub pending_capacity: u32,
    pub retired_capacity: u32,
    pub transition_frames_per_second: u32,
    pub max_prepared_bytes: u32,
    pub hardware_budget_available: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Serialize)]
pub struct Parameter {
    pub version: u32,
    pub size: u32,
    pub algorithm: u32,
    pub parameter: u32,
    pub available: u32,
    pub unit: u32,
    pub minimum: f64,
    pub maximum: f64,
    pub default_value: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Serialize)]
pub struct Status {
    pub version: u32,
    pub size: u32,
    pub rate: u32,
    pub max_block: u32,
    pub revision: u64,
    pub accepted_request: u64,
    pub applied_request: u64,
    pub last_request: u64,
    pub request_state: u32,
    pub request_result: i32,
    pub last_result: i32,
    pub fault: u32,
    pub transition: u32,
    pub intentional_delay_frames: u32,
    pub current: Settings,
    pub target: Settings,
}
type Create = unsafe extern "C" fn(u32, u32, *const Settings) -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type Publish = unsafe extern "C" fn(*mut c_void, *mut c_void, u64, u64) -> i32;
type Process = unsafe extern "C" fn(*mut c_void, *const f64, *mut f64, u32) -> i32;
type Query = unsafe extern "C" fn(*mut c_void, *mut Status, u32, u32) -> i32;
/// One instance and at most one caller-owned token. Destruction is control-side.
pub struct Owner {
    handle: *mut c_void,
    token: *mut c_void,
    prepare: Create,
    cancel: Free,
    destroy: Free,
    publish: Publish,
    process: Process,
    query: Query,
    reset: unsafe extern "C" fn(*mut c_void) -> i32,
    retire: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    defaults: unsafe extern "C" fn(u32, *mut Settings, u32, u32) -> i32,
    parameter: unsafe extern "C" fn(u32, u32, *mut Parameter, u32, u32) -> i32,
    delay_time: Option<unsafe extern "C" fn(f64, u32, u32, *mut f64) -> i32>,
    pub capabilities: Capabilities,
    _library: Arc<Library>,
}
// SAFETY: the v2 ABI permits ownership transfer. Every instance call requires
// exclusive access; this type is deliberately not Sync. The library remains
// loaded for the entire instance lifetime.
unsafe impl Send for Owner {}

/// Caller-owned prepared storage. Transfer does not touch the instance. Drop is
/// allowed only on the controller/retirement worker, never in a render quantum.
pub(crate) struct PreparedToken {
    pointer: *mut c_void,
    cancel: Free,
    library: Arc<Library>,
}
// SAFETY: an unpublished or relinquished token has one owner and no concurrent
// instance access. The ABI explicitly permits preparation on another thread.
unsafe impl Send for PreparedToken {}
impl Drop for PreparedToken {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            unsafe { (self.cancel)(self.pointer) };
        }
    }
}

/// Instance-free preparation uses copied settings and pins the same library.
#[derive(Clone)]
pub(crate) struct Preparer {
    prepare: Create,
    cancel: Free,
    library: Arc<Library>,
    delay_time: Option<unsafe extern "C" fn(f64, u32, u32, *mut f64) -> i32>,
    defaults: unsafe extern "C" fn(u32, *mut Settings, u32, u32) -> i32,
    parameter: unsafe extern "C" fn(u32, u32, *mut Parameter, u32, u32) -> i32,
    pub(crate) capabilities: Capabilities,
}
impl Preparer {
    pub(crate) fn defaults(&self, algorithm: u32) -> Result<Settings> {
        if algorithm >= 32 || self.capabilities.algorithm_mask & (1 << algorithm) == 0 {
            return Err("FX algorithm unavailable".into());
        }
        let mut s = Settings::default();
        if unsafe { (self.defaults)(algorithm, &mut s, 2, 64) } != 0
            || s.version != 2
            || s.size != 64
            || s.algorithm != algorithm
        {
            return Err("FX defaults ABI".into());
        }
        Ok(s)
    }
    pub(crate) fn parameters(&self, algorithm: u32) -> Result<Vec<Parameter>> {
        self.defaults(algorithm)?;
        (1..=6)
            .map(|id| {
                let mut p = Parameter::default();
                if unsafe { (self.parameter)(algorithm, id, &mut p, 2, 48) } != 0
                    || p.version != 2
                    || p.size != 48
                    || p.algorithm != algorithm
                    || p.parameter != id
                    || !p.minimum.is_finite()
                    || !p.maximum.is_finite()
                    || p.minimum > p.maximum
                    || !p.default_value.is_finite()
                {
                    return Err("FX parameter ABI".into());
                }
                Ok(p)
            })
            .collect()
    }
    pub(crate) fn empty_token(&self) -> PreparedToken {
        PreparedToken {
            pointer: std::ptr::null_mut(),
            cancel: self.cancel,
            library: self.library.clone(),
        }
    }

    pub(crate) fn delay_time(&self, bpm: f64, denominator: u32, modifier: u32) -> Result<f64> {
        let resolve = self
            .delay_time
            .ok_or("provider tempo divisions unavailable")?;
        let mut ms = 0.;
        if unsafe { resolve(bpm, denominator, modifier, &mut ms) } != 0 {
            return Err("delay division outside provider time/BPM bounds".into());
        }
        Ok(ms)
    }

    pub(crate) fn prepare(&self, settings: &Settings) -> Result<PreparedToken> {
        let pointer = unsafe { (self.prepare)(48000, 48, settings) };
        if pointer.is_null() {
            return Err("FX settings refused".into());
        }
        Ok(PreparedToken {
            pointer,
            cancel: self.cancel,
            library: self.library.clone(),
        })
    }
}
impl Owner {
    pub fn load(path: &Path) -> Result<Self> {
        let library = unsafe { Library::new(path) }.map_err(|e| e.to_string())?;
        macro_rules! symbol {
            ($name:literal,$t:ty) => {
                *unsafe { library.get::<$t>(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| e.to_string())?
            };
        }
        let caps = symbol!(
            "shr_fx_v2_capabilities",
            unsafe extern "C" fn(*mut Capabilities, u32, u32) -> i32
        );
        let mut capabilities = Capabilities::default();
        if unsafe { caps(&mut capabilities, 2, 48) } != 0
            || capabilities.version != 2
            || capabilities.size != 48
            || capabilities.sample_bits != 64
            || capabilities.min_rate > 48000
            || capabilities.max_rate < 48000
            || capabilities.max_block < 48
            || capabilities.pending_capacity != 1
            || capabilities.retired_capacity != 1
            || capabilities.algorithm_mask & 2 == 0
            || capabilities.max_prepared_bytes > 8 * 1024 * 1024
        {
            return Err("FX palette ABI/capacity".into());
        }
        let create = symbol!("shr_fx_v2_create", Create);
        let mut owner = Self {
            handle: std::ptr::null_mut(),
            token: std::ptr::null_mut(),
            capabilities,
            prepare: symbol!("shr_fx_v2_prepare", Create),
            cancel: symbol!("shr_fx_v2_cancel", Free),
            destroy: symbol!("shr_fx_v2_destroy", Free),
            publish: symbol!("shr_fx_v2_publish", Publish),
            process: symbol!("shr_fx_v2_process", Process),
            query: symbol!("shr_fx_v2_status", Query),
            reset: symbol!("shr_fx_v2_reset", unsafe extern "C" fn(*mut c_void) -> i32),
            retire: symbol!(
                "shr_fx_v2_retire",
                unsafe extern "C" fn(*mut c_void) -> *mut c_void
            ),
            defaults: symbol!(
                "shr_fx_v2_defaults",
                unsafe extern "C" fn(u32, *mut Settings, u32, u32) -> i32
            ),
            parameter: symbol!(
                "shr_fx_v2_parameter",
                unsafe extern "C" fn(u32, u32, *mut Parameter, u32, u32) -> i32
            ),
            delay_time: unsafe {
                library.get::<unsafe extern "C" fn(f64, u32, u32, *mut f64) -> i32>(
                    b"shr_fx_v2_delay_time\0",
                )
            }
            .ok()
            .map(|s| *s),
            _library: Arc::new(library),
        };
        let settings = owner.defaults(1)?;
        owner.handle = unsafe { create(48000, 48, &settings) };
        if owner.handle.is_null() {
            return Err("FX palette create".into());
        }
        owner.status()?;
        Ok(owner)
    }
    pub(crate) fn preparer(&self) -> Preparer {
        Preparer {
            prepare: self.prepare,
            cancel: self.cancel,
            library: self._library.clone(),
            delay_time: self.delay_time,
            defaults: self.defaults,
            parameter: self.parameter,
            capabilities: self.capabilities,
        }
    }
    /// Publish external storage; the caller retains its wrapper for off-thread
    /// destruction even on success. No Arc decrement occurs in this method.
    pub(crate) fn publish_token(
        &mut self,
        token: &mut PreparedToken,
        request: u64,
        revision: u64,
    ) -> std::result::Result<(), i32> {
        if token.pointer.is_null() || !Arc::ptr_eq(&self._library, &token.library) {
            return Err(-1);
        }
        let result = unsafe { (self.publish)(self.handle, token.pointer, request, revision) };
        if result != 0 {
            return Err(result);
        }
        token.pointer = std::ptr::null_mut();
        Ok(())
    }
    /// Poll at most one relinquished object. Caller must reserve a retirement
    /// queue slot first and move the returned token there without dropping it.
    pub(crate) fn retired_token(&mut self) -> Option<PreparedToken> {
        let pointer = unsafe { (self.retire)(self.handle) };
        (!pointer.is_null()).then(|| PreparedToken {
            pointer,
            cancel: self.cancel,
            library: self._library.clone(),
        })
    }
    /// Transfer one relinquished object into a preallocated wrapper. The
    /// wrapper must be empty and originate from this library. Never frees.
    pub(crate) fn take_retired(&mut self, into: &mut PreparedToken) -> bool {
        if !into.pointer.is_null() || !Arc::ptr_eq(&self._library, &into.library) {
            return false;
        }
        into.pointer = unsafe { (self.retire)(self.handle) };
        !into.pointer.is_null()
    }
    pub fn defaults(&self, algorithm: u32) -> Result<Settings> {
        if algorithm >= 32 || self.capabilities.algorithm_mask & (1 << algorithm) == 0 {
            return Err("FX algorithm unavailable".into());
        }
        let mut s = Settings::default();
        if unsafe { (self.defaults)(algorithm, &mut s, 2, 64) } != 0
            || s.version != 2
            || s.size != 64
            || s.algorithm != algorithm
        {
            return Err("FX defaults ABI".into());
        }
        Ok(s)
    }
    pub fn parameters(&self, algorithm: u32) -> Result<Vec<Parameter>> {
        self.defaults(algorithm)?;
        (1..=6)
            .map(|id| {
                let mut p = Parameter::default();
                if unsafe { (self.parameter)(algorithm, id, &mut p, 2, 48) } != 0
                    || p.version != 2
                    || p.size != 48
                    || p.algorithm != algorithm
                    || p.parameter != id
                    || !p.minimum.is_finite()
                    || !p.maximum.is_finite()
                    || p.minimum > p.maximum
                    || !p.default_value.is_finite()
                {
                    return Err("FX parameter ABI".into());
                }
                Ok(p)
            })
            .collect()
    }
    pub fn delay_time(&self, bpm: f64, denominator: u32, modifier: u32) -> Result<f64> {
        let resolve = self
            .delay_time
            .ok_or("provider tempo divisions unavailable")?;
        let mut ms = 0.;
        if unsafe { resolve(bpm, denominator, modifier, &mut ms) } != 0 {
            return Err("delay division outside provider time/BPM bounds".into());
        }
        Ok(ms)
    }
    pub fn status(&mut self) -> Result<Status> {
        self.status_bounded().map_err(str::to_owned)
    }
    pub(crate) fn status_bounded(&mut self) -> std::result::Result<Status, &'static str> {
        let mut s = Status::default();
        if unsafe { (self.query)(self.handle, &mut s, 2, 200) } != 0
            || s.version != 2
            || s.size != 200
            || s.rate != 48000
            || s.max_block != 48
        {
            return Err("FX palette status ABI");
        }
        Ok(s)
    }
    pub fn prepare(&mut self, s: &Settings) -> Result<()> {
        if !self.token.is_null() {
            return Err("FX token busy".into());
        }
        self.token = unsafe { (self.prepare)(48000, 48, s) };
        if self.token.is_null() {
            Err("FX settings refused".into())
        } else {
            Ok(())
        }
    }
    pub fn publish(&mut self, request: u64, revision: u64) -> Result<()> {
        if self.token.is_null() {
            return Err("FX token absent".into());
        }
        self.publish_bounded(request, revision)
            .map_err(|code| format!("FX publish refused {code}"))
    }
    /// Numeric errors preserve allocation-free failure as well as success.
    pub(crate) fn publish_bounded(
        &mut self,
        request: u64,
        revision: u64,
    ) -> std::result::Result<(), i32> {
        if self.token.is_null() {
            return Err(-1);
        }
        let result = unsafe { (self.publish)(self.handle, self.token, request, revision) };
        if result != 0 {
            return Err(result);
        }
        self.token = std::ptr::null_mut();
        Ok(())
    }
    pub fn cancel(&mut self) {
        if !self.token.is_null() {
            unsafe { (self.cancel)(self.token) };
            self.token = std::ptr::null_mut();
        }
    }
    pub fn reclaim(&mut self) {
        let token = unsafe { (self.retire)(self.handle) };
        if !token.is_null() {
            unsafe { (self.cancel)(token) };
        }
    }
    pub fn reset(&mut self) -> Result<()> {
        self.reset_bounded().map_err(str::to_owned)
    }
    pub(crate) fn reset_bounded(&mut self) -> std::result::Result<(), &'static str> {
        if unsafe { (self.reset)(self.handle) } != 0 {
            return Err("FX reset failed");
        }
        Ok(())
    }
    /// Fixed 48-frame stereo source quantum; failed spans are always muted.
    pub fn process(&mut self, input: &[f64; 96], output: &mut [f64; 96]) -> i32 {
        let result =
            unsafe { (self.process)(self.handle, input.as_ptr(), output.as_mut_ptr(), 48) };
        if result != 0 {
            output.fill(0.);
        }
        result
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.cancel();
        if !self.handle.is_null() {
            unsafe { (self.destroy)(self.handle) };
        }
    }
}
