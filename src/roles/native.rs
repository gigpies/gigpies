//! Injected descriptor adapter and private process-held role leases. No enumeration.
use super::{Binding, Registry};
use crate::show::{Counter, Result, decode, id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayDescriptor {
    pub connector: String,
    pub edid: String,
    pub operator_label: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerDescriptor {
    pub identity: String,
    pub serial: Option<String>,
    pub profile: String,
    pub operator_label: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub displays: Vec<DisplayDescriptor>,
    pub controllers: Vec<ControllerDescriptor>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acquire {
    pub format: String,
    pub version: u32,
    pub expected_generation: Counter,
    pub binding: Binding,
    pub inventory: Inventory,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub format: String,
    pub version: u32,
    pub generation: Counter,
    pub binding: Binding,
}

fn label(s: &Option<String>) -> bool {
    s.as_ref()
        .is_none_or(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
}
impl Inventory {
    pub fn verify(&self, b: &Binding) -> Result<()> {
        if self.displays.is_empty()
            || self.displays.len() > 16
            || self.controllers.is_empty()
            || self.controllers.len() > 16
        {
            return Err("inventory_capacity".into());
        }
        if self
            .displays
            .iter()
            .any(|d| !id(&d.connector) || !id(&d.edid) || !label(&d.operator_label))
            || self.controllers.iter().any(|c| {
                !id(&c.identity)
                    || !id(&c.profile)
                    || !label(&c.operator_label)
                    || c.serial.as_ref().is_some_and(|s| !id(s))
            })
        {
            return Err("descriptor_identity".into());
        }
        let ds: Vec<_> = self
            .displays
            .iter()
            .filter(|d| d.connector == b.display_connector && d.edid == b.display_edid)
            .collect();
        let cs: Vec<_> = self
            .controllers
            .iter()
            .filter(|c| c.identity == b.controller && c.profile == b.profile)
            .collect();
        if ds.len() != 1
            || cs.len() != 1
            || self
                .displays
                .iter()
                .filter(|d| d.connector == b.display_connector)
                .count()
                != 1
            || self
                .controllers
                .iter()
                .filter(|c| c.identity == b.controller)
                .count()
                != 1
        {
            return Err("identity".into());
        }
        let d = ds[0];
        let c = cs[0];
        let ambiguous_display = self.displays.iter().filter(|x| x.edid == d.edid).count() > 1;
        let ambiguous_controller = c.serial.is_none()
            || self
                .controllers
                .iter()
                .filter(|x| x.serial == c.serial)
                .count()
                > 1;
        if (ambiguous_display
            && (b.operator_label.is_none()
                || d.operator_label != b.operator_label
                || self
                    .displays
                    .iter()
                    .filter(|x| x.edid == d.edid && x.operator_label == b.operator_label)
                    .count()
                    != 1))
            || (ambiguous_controller
                && (b.operator_label.is_none()
                    || c.operator_label != b.operator_label
                    || self
                        .controllers
                        .iter()
                        .filter(|x| x.operator_label == b.operator_label)
                        .count()
                        != 1))
        {
            return Err("ambiguous_identity".into());
        }
        Ok(())
    }
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn private(f: &File, directory: bool) -> Result<()> {
    let m = f.metadata().map_err(err)?;
    // SAFETY: geteuid has no pointer arguments.
    let uid = unsafe { libc::geteuid() };
    if m.uid() != uid
        || m.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
        || if directory {
            !m.is_dir()
        } else {
            !m.is_file() || m.nlink() != 1
        }
    {
        return Err("private_permissions".into());
    }
    Ok(())
}
fn open_dir(path: &Path) -> Result<File> {
    if !path.is_absolute() {
        return Err("absolute_directory_required".into());
    }
    let mut f = File::open("/").map_err(err)?;
    for part in path.components() {
        match part {
            Component::RootDir => (),
            Component::Normal(s) => {
                let s = std::ffi::CString::new(s.as_encoded_bytes()).map_err(err)?;
                // SAFETY: valid C string, live directory fd; successful fd is owned below.
                let fd = unsafe {
                    libc::openat(
                        f.as_raw_fd(),
                        s.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(err(std::io::Error::last_os_error()));
                }
                // SAFETY: openat returned a fresh owned fd.
                f = unsafe { File::from_raw_fd(fd) };
            }
            _ => return Err("directory_components".into()),
        }
    }
    private(&f, true)?;
    Ok(f)
}
fn open_file(dir: &File, name: &str, create: bool) -> Result<File> {
    let name = std::ffi::CString::new(name).map_err(err)?;
    let flags = libc::O_RDWR
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | libc::O_NONBLOCK
        | if create { libc::O_CREAT } else { 0 };
    // SAFETY: bounded internal filename, owned directory fd.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    // SAFETY: fresh fd belongs to this File.
    let f = unsafe { File::from_raw_fd(fd) };
    private(&f, false)?;
    Ok(f)
}
fn lock(f: &File) -> Result<()> {
    // SAFETY: live fd; nonblocking advisory lock.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("occupied".into());
    }
    Ok(())
}
fn registry(dir: &File, initialize: bool) -> Result<Registry> {
    let mut f = match open_file(dir, "registry.json", false) {
        Ok(f) => f,
        Err(e) if initialize => {
            let name = c"registry.json";
            // SAFETY: stat relative to pinned directory, initialized output.
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            let rc = unsafe {
                libc::fstatat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) {
                return Err(e);
            }
            return Ok(Registry {
                format: "gigpies-roles".into(),
                version: 1,
                generation: Counter(0),
                bindings: vec![],
            });
        }
        Err(e) => return Err(e),
    };
    if f.metadata().map_err(err)?.len() > 65536 {
        return Err("registry_capacity".into());
    }
    let mut data = Vec::new();
    (&mut f).take(65537).read_to_end(&mut data).map_err(err)?;
    Registry::decode(&data)
}
fn save(dir: &File, r: &Registry) -> Result<()> {
    r.validate()?;
    let bytes = serde_json::to_vec(r).map_err(err)?;
    if bytes.len() > 65536 {
        return Err("registry_capacity".into());
    }
    let name = format!("pending-{}", std::process::id());
    let cname = std::ffi::CString::new(name.clone()).map_err(err)?;
    // SAFETY: private directory, exclusive new internal filename.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            cname.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    // SAFETY: newly created fd.
    let mut f = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        f.write_all(&bytes).map_err(err)?;
        f.sync_all().map_err(err)?;
        let dest = c"registry.json";
        // SAFETY: both filenames relative to pinned private directory.
        if unsafe {
            libc::renameat(
                dir.as_raw_fd(),
                cname.as_ptr(),
                dir.as_raw_fd(),
                dest.as_ptr(),
            )
        } != 0
        {
            return Err(err(std::io::Error::last_os_error()));
        }
        dir.sync_all().map_err(err)
    })();
    if result.is_err() {
        // SAFETY: deletes only own exclusive scratch file.
        unsafe { libc::unlinkat(dir.as_raw_fd(), cname.as_ptr(), 0) };
    }
    result
}
fn inode(f: &File) -> Result<(u64, u64)> {
    let m = f.metadata().map_err(err)?;
    Ok((m.dev(), m.ino()))
}
fn check_inode(dir: &File, name: &str, expected: (u64, u64)) -> Result<()> {
    if inode(&open_file(dir, name, false)?)? != expected {
        return Err("lock_inode_changed".into());
    }
    Ok(())
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LockMap {
    locks: std::collections::BTreeMap<String, (u64, u64)>,
    descriptors: std::collections::BTreeMap<String, String>,
}
fn lock_map(dir: &File) -> Result<Option<LockMap>> {
    let mut f = match open_file(dir, "initialized", false) {
        Ok(f) => f,
        Err(e) => {
            let name = c"initialized";
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: pinned directory and initialized stat output.
            if unsafe {
                libc::fstatat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
            {
                return Ok(None);
            }
            return Err(e);
        }
    };
    let mut bytes = Vec::new();
    (&mut f).take(65537).read_to_end(&mut bytes).map_err(err)?;
    let map: LockMap = decode(&bytes)?;
    if map.locks.is_empty() || map.locks.len() > 65 || map.descriptors.len() > 64 {
        return Err("lock_map_capacity".into());
    }
    for (name, identity) in &map.locks {
        if name != "registry.lock"
            && !(name.starts_with("device-") && name.ends_with(".lock") && name.len() == 76)
        {
            return Err("lock_map_name".into());
        }
        check_inode(dir, name, *identity)?;
    }
    Ok(Some(map))
}
fn native_marker(dir: &File) -> Result<bool> {
    let mut f = match open_file(dir, "native.created", false) {
        Ok(f) => f,
        Err(e) => {
            let name = c"native.created";
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: fixed name, pinned directory, valid stat storage.
            if unsafe {
                libc::fstatat(
                    dir.as_raw_fd(),
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
            {
                return Ok(false);
            }
            return Err(e);
        }
    };
    let mut bytes = Vec::new();
    (&mut f).take(65).read_to_end(&mut bytes).map_err(err)?;
    if bytes != b"gigpies-role-native:1\n" {
        return Err("native_marker_corrupt".into());
    }
    Ok(true)
}
fn create_native_marker(dir: &File) -> Result<()> {
    // SAFETY: exclusive fixed marker creation in pinned private directory.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            c"native.created".as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    // SAFETY: fresh owned fd.
    let mut f = unsafe { File::from_raw_fd(fd) };
    f.write_all(b"gigpies-role-native:1\n").map_err(err)?;
    f.sync_all().map_err(err)?;
    dir.sync_all().map_err(err)
}
fn save_map(dir: &File, map: &LockMap) -> Result<()> {
    let name = c"initialized.pending";
    // SAFETY: fixed exclusive private scratch file; leftovers fail closed.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    // SAFETY: fresh owned fd.
    let mut f = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        f.write_all(&serde_json::to_vec(map).map_err(err)?)
            .map_err(err)?;
        f.sync_all().map_err(err)?;
        // SAFETY: fixed filenames in pinned directory.
        if unsafe {
            libc::renameat(
                dir.as_raw_fd(),
                name.as_ptr(),
                dir.as_raw_fd(),
                c"initialized".as_ptr(),
            )
        } != 0
        {
            return Err(err(std::io::Error::last_os_error()));
        }
        dir.sync_all().map_err(err)
    })();
    if result.is_err() {
        // SAFETY: own exclusive scratch file only.
        unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
    }
    result
}
/// Holds two device locks. Drop/crash releases locks but retains intended assignment.
pub struct Lease {
    dir: File,
    devices: Vec<(String, File)>,
    mutex_identity: (u64, u64),
    grant: Grant,
}
impl Lease {
    pub fn acquire(path: &Path, request: &Acquire) -> Result<Self> {
        if request.format != "gigpies-role-acquire" || request.version != 1 {
            return Err("version".into());
        }
        request.inventory.verify(&request.binding)?;
        let dir = open_dir(path)?;
        let mutex = open_file(&dir, "registry.lock", true)?;
        lock(&mutex)?;
        let established = native_marker(&dir)?;
        let previous_map = lock_map(&dir)?;
        if established != previous_map.is_some() {
            return Err("native_metadata_lost_or_interrupted".into());
        }
        let initialized = previous_map.is_some();
        let mut map = previous_map.unwrap_or_default();
        map.locks
            .entry("registry.lock".into())
            .or_insert(inode(&mutex)?);
        let mut r = registry(&dir, !initialized)?;
        if r.generation != request.expected_generation {
            return Err("stale_generation".into());
        }
        let b = &request.binding;
        let reclaim = r.bindings.iter().any(|x| x.role == b.role);
        if let Some(existing) = r.bindings.iter().find(|x| x.role == b.role) {
            if existing != b {
                return Err("identity_profile_mismatch".into());
            }
            r.generation = Counter(
                r.generation
                    .0
                    .checked_add(1)
                    .ok_or("generation_exhausted")?,
            );
        } else {
            r.claim(request.expected_generation, b.clone())?;
        }
        let d = request
            .inventory
            .displays
            .iter()
            .find(|d| d.connector == b.display_connector)
            .ok_or("identity")?;
        let c = request
            .inventory
            .controllers
            .iter()
            .find(|c| c.identity == b.controller)
            .ok_or("identity")?;
        for (key, bytes) in [
            (
                format!("display:{}", b.display_connector),
                serde_json::to_vec(d).map_err(err)?,
            ),
            (
                format!("controller:{}", b.controller),
                serde_json::to_vec(c).map_err(err)?,
            ),
        ] {
            let fingerprint = crate::hash::hex(Sha256::digest(bytes));
            if reclaim
                && initialized
                && map
                    .descriptors
                    .get(&key)
                    .is_none_or(|old| old != &fingerprint)
            {
                return Err("descriptor_changed".into());
            }
            map.descriptors.insert(key, fingerprint);
        }
        let device_names: Vec<_> = [
            format!("display:{}", b.display_connector),
            format!("controller:{}", b.controller),
        ]
        .iter()
        .map(|identity| {
            format!(
                "device-{}.lock",
                crate::hash::hex(Sha256::digest(identity.as_bytes()))
            )
        })
        .collect();
        if map.locks.len()
            + device_names
                .iter()
                .filter(|name| !map.locks.contains_key(*name))
                .count()
            > 65
            || map.descriptors.len() > 64
        {
            return Err("lock_map_capacity".into());
        }
        let mut devices = Vec::new();
        for name in device_names {
            let f = open_file(&dir, &name, true)?;
            lock(&f)?;
            devices.push((name, f));
        }
        for (name, f) in &devices {
            map.locks.insert(name.clone(), inode(f)?);
        }
        if !established {
            create_native_marker(&dir)?;
        }
        save_map(&dir, &map)?;
        save(&dir, &r)?;
        let grant = Grant {
            format: "gigpies-role-lease".into(),
            version: 1,
            generation: r.generation,
            binding: b.clone(),
        };
        Ok(Self {
            dir,
            devices,
            mutex_identity: inode(&mutex)?,
            grant,
        })
    }
    pub fn grant(&self) -> &Grant {
        &self.grant
    }
    pub fn verify(&self, generation: Counter, binding: &Binding) -> Result<()> {
        if generation != self.grant.generation || binding != &self.grant.binding {
            return Err("stale_lease".into());
        }
        check_inode(&self.dir, "registry.lock", self.mutex_identity)?;
        for (name, f) in &self.devices {
            check_inode(&self.dir, name, inode(f)?)?;
        }
        if !native_marker(&self.dir)? {
            return Err("native_metadata_lost".into());
        }
        lock_map(&self.dir)?.ok_or("missing_lock_map")?;
        let mutex = open_file(&self.dir, "registry.lock", false)?;
        lock(&mutex)?;
        let r = registry(&self.dir, false)?;
        // Another role's update may advance global generation; this lease's generation remains fixed.
        if !r.bindings.contains(binding) {
            return Err("lost_assignment".into());
        }
        Ok(())
    }
    pub fn registry_generation(&self) -> Result<Counter> {
        check_inode(&self.dir, "registry.lock", self.mutex_identity)?;
        for (name, f) in &self.devices {
            check_inode(&self.dir, name, inode(f)?)?;
        }
        if !native_marker(&self.dir)? {
            return Err("native_metadata_lost".into());
        }
        lock_map(&self.dir)?.ok_or("missing_lock_map")?;
        let mutex = open_file(&self.dir, "registry.lock", false)?;
        lock(&mutex)?;
        Ok(registry(&self.dir, false)?.generation)
    }
    pub fn release(self, generation: Counter, binding: &Binding) -> Result<()> {
        self.verify(generation, binding)?;
        Ok(())
    }
    pub fn forget(self, expected_registry_generation: Counter) -> Result<Counter> {
        check_inode(&self.dir, "registry.lock", self.mutex_identity)?;
        for (name, f) in &self.devices {
            check_inode(&self.dir, name, inode(f)?)?;
        }
        if !native_marker(&self.dir)? {
            return Err("native_metadata_lost".into());
        }
        lock_map(&self.dir)?.ok_or("missing_lock_map")?;
        let mutex = open_file(&self.dir, "registry.lock", false)?;
        lock(&mutex)?;
        let mut r = registry(&self.dir, false)?;
        if !r.bindings.contains(&self.grant.binding) {
            return Err("identity".into());
        }
        let generation = r.release(
            expected_registry_generation,
            self.grant.binding.role,
            &self.grant.binding.controller,
        )?;
        save(&self.dir, &r)?;
        Ok(generation)
    }
}
/// Strict bounded request parser shared by the provider CLI, not consumer code.
pub fn decode_acquire(bytes: &[u8]) -> Result<Acquire> {
    decode(bytes)
}

pub fn decode_protocol<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    decode(bytes)
}
