//! Serialized native-f64 SHR FX palette owner. No DSP is duplicated here.
use libloading::Library;
use serde::Serialize;
use std::{ffi::c_void, path::Path, sync::Arc};
type Result<T> = std::result::Result<T, String>;
pub use crate::fx_aux_wire::Settings;
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
    /// Preflight the audited v2 publication conditions without publishing a target.
    /// The caller holds all four owners exclusively until the complete bundle is
    /// published. Retired storage transfers into a reserved control-owned wrapper.
    pub(crate) fn preflight_token(
        &mut self,
        token: Option<&PreparedToken>,
        revision: u64,
        retired: &mut PreparedToken,
    ) -> std::result::Result<Option<u64>, i32> {
        let status = self.status_bounded().map_err(|_| -1)?;
        if status.revision != revision {
            return Err(-4);
        }
        if status.transition != 0
            || status.fault != 0
            || status.current != status.target
            || matches!(status.request_state, 1 | 2)
        {
            return Err(-2);
        }
        if !retired.pointer.is_null() || !Arc::ptr_eq(&self._library, &retired.library) {
            return Err(-2);
        }
        if let Some(token) = token {
            if token.pointer.is_null() || !Arc::ptr_eq(&self._library, &token.library) {
                return Err(-1);
            }
            let request = status.accepted_request.checked_add(1).ok_or(-5)?;
            self.take_retired(retired);
            Ok(Some(request))
        } else {
            Ok(None)
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

/// Fixed four-owner publication storage; all allocation and destruction is on
/// the controller. Only the hash-audited bundle path may construct this packet.
pub(crate) struct PreparedBundle {
    tokens: [Option<PreparedToken>; 4],
    retired: [PreparedToken; 4],
    revisions: [u64; 4],
    pub(crate) generations: [u64; 4],
    pub(crate) settings: [Settings; 4],
}
impl PreparedBundle {
    pub(crate) fn prepare(
        preparers: [&Preparer; 4],
        status: [Status; 4],
        generations: [u64; 4],
        configs: &[crate::fx_aux_wire::Configuration; 4],
    ) -> Result<Self> {
        let mut tokens = std::array::from_fn(|_| None);
        for i in 0..4 {
            if status[i].transition != 0
                || status[i].fault != 0
                || matches!(status[i].request_state, 1 | 2)
                || generations[i] == u64::MAX
            {
                return Err("FX restore requires four settled owners".into());
            }
            let token = preparers[i].prepare(&configs[i].settings)?;
            if configs[i].settings != status[i].current {
                tokens[i] = Some(token);
            }
        }
        Ok(Self {
            tokens,
            retired: std::array::from_fn(|i| preparers[i].empty_token()),
            revisions: status.map(|s| s.revision),
            generations,
            settings: configs.each_ref().map(|c| c.settings),
        })
    }
    pub(crate) fn publish(
        &mut self,
        owners: [&mut Owner; 4],
    ) -> std::result::Result<[Option<u64>; 4], i32> {
        let mut requests = [None; 4];
        for i in 0..4 {
            requests[i] = owners[i].preflight_token(
                self.tokens[i].as_ref(),
                self.revisions[i],
                &mut self.retired[i],
            )?;
        }
        // For the audited artifact all refusal predicates have been discharged.
        // No process/reset/query from another execution context can interleave.
        for i in 0..4 {
            if let Some(token) = &mut self.tokens[i]
                && let Err(code) =
                    owners[i].publish_token(token, requests[i].unwrap(), self.revisions[i])
            {
                return Err(-10000 + code); // unexpected partial application is UNKNOWN
            }
        }
        Ok(requests)
    }
}

#[cfg(test)]
mod bundle_tests {
    use super::*;
    #[test]
    #[ignore = "GP_FX_LIBRARY approved actual owner; no device or network"]
    fn actual_bundle_refuses_before_any_publication_and_applies_all() {
        let path = std::path::PathBuf::from(std::env::var_os("GP_FX_LIBRARY").unwrap());
        let mut owners: [Owner; 4] = std::array::from_fn(|_| Owner::load(&path).unwrap());
        let preparers = owners.each_ref().map(Owner::preparer);
        let initial = owners.each_mut().map(|o| o.status().unwrap());
        let configs = std::array::from_fn(|i| {
            let mut settings = initial[i].current;
            settings.time_ms = 30. + i as f64;
            crate::fx_aux_wire::Configuration {
                settings,
                sends: vec![0.; 4],
                returns: Some(crate::fx_aux_wire::Returns::silent_monitors(2)),
                tempo: None,
            }
        });
        let mut bundle =
            PreparedBundle::prepare(preparers.each_ref(), initial, [0; 4], &configs).unwrap();
        let mut other = initial[3].current;
        other.time_ms = 50.;
        owners[3].prepare(&other).unwrap();
        owners[3].publish(1, 0).unwrap();
        let mut result = None;
        crate::remote::assert_no_allocations(|| {
            result = Some(bundle.publish(owners.each_mut()));
        });
        assert!(result.unwrap().is_err());
        for o in &mut owners[..3] {
            assert_eq!(o.status().unwrap().accepted_request, 0);
        }
        for _ in 0..21 {
            for o in &mut owners {
                o.process(&[0.; 96], &mut [0.; 96]);
            }
        }
        // Deliberately leave retirement occupied: preflight must transfer it.
        let status = owners.each_mut().map(|o| o.status().unwrap());
        let mut bundle =
            PreparedBundle::prepare(preparers.each_ref(), status, [0; 4], &configs).unwrap();
        let mut result = None;
        crate::remote::assert_no_allocations(|| {
            result = Some(bundle.publish(owners.each_mut()));
        });
        let requests = result.unwrap().unwrap();
        for _ in 0..21 {
            for o in &mut owners {
                o.process(&[0.; 96], &mut [0.; 96]);
            }
        }
        for i in 0..4 {
            let s = owners[i].status().unwrap();
            assert_eq!(s.current, configs[i].settings);
            assert_eq!(Some(s.applied_request), requests[i]);
        }
    }
}
