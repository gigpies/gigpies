//! Optional SHR FX source-timeline delay ABI. Library, handle and tokens share one owner.
use crate::fx_wire::{Channel, Configuration};
use libloading::Library;
use std::{ffi::c_void, path::Path, sync::Arc};
type Result<T> = std::result::Result<T, String>;
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawChannel {
    delay_ms: f64,
    feedback: f64,
    damping: f64,
    wet_gain: f64,
    bypass: u32,
    reserved: u32,
}
impl From<&Channel> for RawChannel {
    fn from(c: &Channel) -> Self {
        Self {
            delay_ms: c.delay_ms,
            feedback: c.feedback,
            damping: c.damping,
            wet_gain: c.wet_gain,
            bypass: u32::from(c.bypass),
            reserved: 0,
        }
    }
}
impl From<RawChannel> for Channel {
    fn from(c: RawChannel) -> Self {
        Self {
            delay_ms: c.delay_ms,
            feedback: c.feedback,
            damping: c.damping,
            wet_gain: c.wet_gain,
            bypass: c.bypass != 0,
        }
    }
}
#[repr(C)]
struct Config {
    version: u32,
    size: u32,
    expected_generation: u64,
    generation: u64,
    channel: [RawChannel; 2],
}
#[repr(C)]
#[derive(Default)]
struct Capabilities {
    version: u32,
    size: u32,
    identity: [u8; 32],
    min_rate: u32,
    max_rate: u32,
    min_block: u32,
    max_block: u32,
    channels: u32,
    sample_bits: u32,
    rack: u32,
    adapter_frames: u32,
    min_delay: f64,
    max_delay: f64,
    max_feedback: f64,
    max_damping: f64,
    max_wet: f64,
    transition_ms: u32,
    max_heads: u32,
    max_storage: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Status {
    pub version: u32,
    pub size: u32,
    pub sample_rate: u32,
    pub max_block_frames: u32,
    pub transition_frames: u32,
    pub transitioning_mask: u32,
    pub last_process_result: i32,
    pub reset_reason: u32,
    pub applied_generation: u64,
    pub settled_generation: u64,
    pub applied_source_frame: u64,
    pub settled_source_frame: u64,
    pub next_source_frame: u64,
    pub reset_count: u64,
    pub remaining_frames: [u32; 2],
    target: [RawChannel; 2],
}
impl Status {
    pub fn configuration(&self) -> Configuration {
        Configuration {
            channels: self.target.map(Channel::from),
        }
    }
}
type Prepare = unsafe extern "C" fn(*const Config, u32, u32, u32) -> *mut c_void;
type Commit = unsafe extern "C" fn(*mut c_void, *const c_void, u64) -> i32;
type Retire = unsafe extern "C" fn(*mut c_void);
type Process = unsafe extern "C" fn(*mut c_void, *const f64, *mut f64, u32, u64) -> i32;
type Panic = unsafe extern "C" fn(*mut c_void, u32) -> i32;
type Reset = unsafe extern "C" fn(*mut c_void, u64) -> i32;
type Query = unsafe extern "C" fn(*mut c_void, *mut Status, u32, u32) -> i32;
/// Token lives inside its owner; commit only borrows it. Cleanup runs off process.
pub struct Owner {
    handle: *mut c_void,
    token: *mut c_void,
    prepare: Prepare,
    commit: Commit,
    retire: Retire,
    process: Process,
    panic: Panic,
    reset: Reset,
    query: Query,
    destroy: Retire,
    max_block: usize,
    _library: Arc<Library>,
}
// Exclusive native handle transfer; no concurrent handle operations are allowed.
unsafe impl Send for Owner {}

/// Instance-free preparation pins the ABI independently of the render handle.
pub(crate) struct Preparer {
    prepare: Prepare,
    retire: Retire,
    library: Arc<Library>,
}
pub(crate) struct PreparedToken {
    raw: *mut c_void,
    retire: Retire,
    _library: Arc<Library>,
}
// The immutable token is exclusively transferred and only borrowed at commit.
unsafe impl Send for PreparedToken {}
impl Drop for PreparedToken {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { (self.retire)(self.raw) };
        }
    }
}
impl Preparer {
    pub(crate) fn prepare(&self, config: &Configuration, expected: u64) -> Result<PreparedToken> {
        let c = Config {
            version: 2,
            size: size_of::<Config>() as u32,
            expected_generation: expected,
            generation: expected.checked_add(1).ok_or("FX generation exhausted")?,
            channel: [(&config.channels[0]).into(), (&config.channels[1]).into()],
        };
        let raw = unsafe { (self.prepare)(&c, 48_000, 2, size_of::<Config>() as u32) };
        if raw.is_null() {
            return Err("FX preparation refused".into());
        }
        Ok(PreparedToken {
            raw,
            retire: self.retire,
            _library: self.library.clone(),
        })
    }
}
impl Owner {
    pub(crate) fn preparer(&self) -> Preparer {
        Preparer {
            prepare: self.prepare,
            retire: self.retire,
            library: self._library.clone(),
        }
    }
    pub(crate) fn commit_token(&mut self, token: &PreparedToken, frame: u64) -> i32 {
        if !Arc::ptr_eq(&self._library, &token._library) {
            return -1;
        }
        unsafe { (self.commit)(self.handle, token.raw, frame) }
    }

    pub fn load(path: &Path, max_block: usize) -> Result<Option<Self>> {
        if max_block == 0 || max_block > 8192 {
            return Err("FX block capacity".into());
        }
        let library = unsafe { Library::new(path) }.map_err(|e| e.to_string())?;
        // This dedicated namespace is distinct from the published generic v2 ABI,
        // whose signatures/layouts are incompatible. Never probe or call those
        // symbols as a delay owner. Missing ANY delay symbol retains v1 media.
        macro_rules! sym {
            ($name:literal,$ty:ty) => {
                match unsafe { library.get::<$ty>(concat!($name, "\0").as_bytes()) } {
                    Ok(s) => *s,
                    Err(_) => return Ok(None),
                }
            };
        }
        let create = sym!(
            "shr_fx_delay_v2_create",
            unsafe extern "C" fn(u32, u32) -> *mut c_void
        );
        let prepare = sym!("shr_fx_delay_v2_prepare", Prepare);
        let commit = sym!("shr_fx_delay_v2_commit", Commit);
        let retire = sym!("shr_fx_delay_v2_retire", Retire);
        let process = sym!("shr_fx_delay_v2_process", Process);
        let panic = sym!("shr_fx_delay_v2_panic", Panic);
        let reset = sym!("shr_fx_delay_v2_reset", Reset);
        let query = sym!("shr_fx_delay_v2_status", Query);
        let destroy = sym!("shr_fx_delay_v2_destroy", Retire);
        let caps = sym!(
            "shr_fx_delay_v2_capabilities",
            unsafe extern "C" fn(*mut Capabilities, u32, u32) -> i32
        );
        let mut c = Capabilities::default();
        if unsafe { caps(&mut c, 2, size_of::<Capabilities>() as u32) } != 0
            || c.version != 2
            || c.size as usize != size_of::<Capabilities>()
            || c.identity.split(|b| *b == 0).next() != Some(b"fx-a/prepared-delay-v2".as_slice())
            || c.channels != 2
            || c.sample_bits != 64
            || c.rack != 0
            || c.adapter_frames != 0
            || c.min_delay != 1.
            || c.max_delay != 500.
            || c.max_feedback != 0.85
            || c.max_damping != 0.99
            || c.max_wet != 1.
            || c.transition_ms != 20
            || c.max_heads != 2
            || c.min_rate > 48_000
            || c.max_rate < 48_000
            || c.min_block > max_block as u32
            || c.max_block < max_block as u32
        {
            return Ok(None);
        }
        let handle = unsafe { create(48_000, max_block as u32) };
        if handle.is_null() {
            return Ok(None);
        }
        let mut owner = Self {
            handle,
            token: std::ptr::null_mut(),
            prepare,
            commit,
            retire,
            process,
            panic,
            reset,
            query,
            destroy,
            max_block,
            _library: Arc::new(library),
        };
        if owner.status().is_err() {
            return Ok(None);
        }
        Ok(Some(owner))
    }
    pub fn status(&mut self) -> Result<Status> {
        self.status_bounded().map_err(str::to_owned)
    }
    pub(crate) fn status_bounded(&mut self) -> std::result::Result<Status, &'static str> {
        let mut s = Status::default();
        if unsafe { (self.query)(self.handle, &mut s, 2, size_of::<Status>() as u32) } != 0
            || s.version != 2
            || s.size as usize != size_of::<Status>()
            || s.sample_rate != 48_000
            || s.max_block_frames as usize != self.max_block
        {
            return Err("FX owner status");
        }
        Ok(s)
    }
    pub fn prepare(&mut self, config: &Configuration, expected: u64) -> Result<()> {
        if !self.token.is_null() {
            return Err("FX token busy".into());
        }
        let c = Config {
            version: 2,
            size: size_of::<Config>() as u32,
            expected_generation: expected,
            generation: expected.checked_add(1).ok_or("FX generation exhausted")?,
            channel: [(&config.channels[0]).into(), (&config.channels[1]).into()],
        };
        self.token = unsafe { (self.prepare)(&c, 48_000, 2, size_of::<Config>() as u32) };
        if self.token.is_null() {
            Err("FX preparation refused".into())
        } else {
            Ok(())
        }
    }
    pub fn retire(&mut self) {
        if !self.token.is_null() {
            unsafe { (self.retire)(self.token) };
            self.token = std::ptr::null_mut();
        }
    }
    pub fn commit(&mut self, frame: u64) -> i32 {
        if self.token.is_null() {
            return -1;
        }
        unsafe { (self.commit)(self.handle, self.token, frame) }
    }
    pub fn panic(&mut self, mask: u32) -> i32 {
        unsafe { (self.panic)(self.handle, mask) }
    }
    pub fn reset(&mut self, frame: u64) -> i32 {
        unsafe { (self.reset)(self.handle, frame) }
    }
    pub fn process(&mut self, input: &[f64], output: &mut [f64], frame: u64) -> i32 {
        if input.len() != output.len()
            || !input.len().is_multiple_of(2)
            || input.len() / 2 > self.max_block
        {
            return -2;
        }
        unsafe {
            (self.process)(
                self.handle,
                input.as_ptr(),
                output.as_mut_ptr(),
                (input.len() / 2) as u32,
                frame,
            )
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.retire();
        unsafe { (self.destroy)(self.handle) }
    }
}
