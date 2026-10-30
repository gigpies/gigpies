//! Opt-in authenticated command and media transport. All methods are worker-side;
//! none belong in an audio callback. TLS identity is an authorization ceiling,
//! never a replacement for the processing authority's leases and revisions.
mod authority;
#[cfg(feature = "hardware-host")]
mod brain;
mod brain_audio;
mod diagnostics;
#[cfg(feature = "hardware-host")]
pub mod fx_aux_owner;
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
mod host;
mod media;
mod policy;
mod proxy;
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
mod runner;
mod session;
mod tls;
mod wire;

pub use authority::*;
#[cfg(feature = "hardware-host")]
pub use brain::*;
pub use brain_audio::*;
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
pub use host::*;
pub use media::*;
pub use policy::*;
pub use proxy::*;
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
pub use runner::*;
pub use session::*;
pub use tls::*;
pub use wire::*;

pub type Result<T> = std::result::Result<T, String>;
pub const ALPN: &[u8] = b"gigpies-remote/1";
pub const MAX_FRAME: usize = 65_536;
pub const MAX_PEERS: usize = 64;
// Finite deployment budget: six independently scoped controllers, duplex + FX
// workers, and eight observer/reconnect slots. Not an engine channel/product cap.
pub const MAX_CONNECTIONS: usize = 16;
pub const IO_TIMEOUT_MS: u64 = 2_000;

#[cfg(test)]
mod tests;

/// Shared process-local lease clock. It is independent of source audio frames;
/// every controller endpoint on this host uses the same monotonic origin.
pub fn monotonic_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(all(test, target_os = "linux", feature = "hardware-host"))]
mod held_proof_tests;

#[cfg(all(test, target_os = "linux", feature = "hardware-host"))]
mod atomic_control_tests;

#[cfg(all(test, target_os = "linux", feature = "hardware-host"))]
mod master_eq_tests;
