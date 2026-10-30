//! Offline foundations for GigPies. No live audio or hardware is opened.
pub mod inventory;

/// Intended live processing rate; source recordings retain their original rate.
pub const LIVE_SAMPLE_RATE_HZ: u32 = 48_000;

pub mod automix;

/// Synthetic network audio and control contracts; no hardware is opened.
pub mod transport;

/// Bounded source-frame host contracts; opening hardware is explicitly opt-in.
pub mod host;

/// Pure C-ROLE:1 intended assignments; native ownership is a later adapter.
pub mod roles;
/// Portable C-SHOW:1 metadata and private checkpoints, outside callbacks.
pub mod show;

/// C-AUDIO:1 pure offline authority harness and bounded codec.
pub mod control_model;

/// Fixed-storage eight-input offline mixer; no host/PA binding.
pub mod mixer;
/// Control-side deterministic offline authority/render pump.
pub mod mixer_control;

/// Explicit opt-in same-UID private Unix binding for the offline engine.
#[cfg(target_os = "linux")]
pub mod local_audio;

/// Bounded named raw analysis subscriptions.
pub mod analysis_stream;

/// Explicit trusted owner-library synthetic graph; opens no hardware endpoints.
#[cfg(feature = "hardware-host")]
pub mod module_graph;

/// Separately discriminated read-only module health and scoped recorder envelope.
pub mod module_wire;

/// Versioned fixed-storage channel EQ and dynamics.
pub mod channel_processing;
/// Strict GP07 processing extension.
pub mod processing_wire;

pub mod topology;

pub mod clock_domain;

pub mod remote;

pub mod snapshot_pages;

/// Explicit PA/physical patch controls using the same authority and boundary.
pub mod structural_control;

/// Brain local duplex I/O and bounded bridges between independent device clocks.
pub mod brain_audio;
/// Scoped operator monitoring and held talkback control.
pub mod brain_control;
/// Explicit fake/physical Brain process; never auto-started.
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
pub mod brain_runtime;

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
mod device_epoch;

/// Correlated, scoped held-talkback authority readback.
pub mod held_proof;

/// Atomic scoped lease maintenance and committed paired observation.
pub mod lease_maintenance;
pub mod paired_readback;

/// GP18 per-destination monitor tap control.
pub mod sends_wire;

pub mod master_eq_wire;

#[cfg(target_os = "linux")]
pub mod measurement_owner;
pub mod measurement_wire;

pub mod fx_wire;
#[cfg(target_os = "linux")]
pub mod measurement_capture;
pub mod metering;

pub mod meter_wire;

/// Bounded selected-channel spectral observation.
pub mod spectrum;

pub mod fx_aux_wire;
