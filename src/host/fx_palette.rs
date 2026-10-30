//! Serialized native-f64 SHR FX palette owner. No DSP is duplicated here.
use libloading::Library;
use serde::{Deserialize, Serialize};
use std::{ffi::c_void, path::Path};
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
    _library: Library,
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
            _library: library,
        };
        let settings = owner.defaults(1)?;
        owner.handle = unsafe { create(48000, 48, &settings) };
        if owner.handle.is_null() {
            return Err("FX palette create".into());
        }
        owner.status()?;
        Ok(owner)
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
        let mut s = Status::default();
        if unsafe { (self.query)(self.handle, &mut s, 2, 200) } != 0
            || s.version != 2
            || s.size != 200
            || s.rate != 48000
            || s.max_block != 48
        {
            return Err("FX palette status ABI".into());
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
        let result = unsafe { (self.publish)(self.handle, self.token, request, revision) };
        if result != 0 {
            return Err(format!("FX publish refused {result}"));
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
        if unsafe { (self.reset)(self.handle) } != 0 {
            return Err("FX reset failed".into());
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
