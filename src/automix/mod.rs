//! Offline, causal soundcheck and frozen-settings stereo rendering.
pub mod config;
pub mod delivery;
pub mod dsp;
mod identity;
mod observation;
mod render;
pub mod true_peak;
pub use render::{compare, observe, observe_policy, run, run_policy, write_json};

pub mod ambience;
pub mod effects;
mod exciter;
mod fx_engines;

pub mod analysis;

pub mod unity;

pub mod preservation;

pub mod balance;

/// Explicit kick > bass > other-source loudness policy.
pub mod hierarchy;

/// Intent-conditioned offline guitar tone preparation.
pub mod tone;

/// Frozen spectral matching, artistic maps and offline musician review.
pub mod matching;

/// Explicit source profiles and bounded processing rules.
pub mod expert;

/// Independent produced-reference measurements; never selects mix settings.
pub mod reference;

/// DI bass evidence, note guards and bounded correction.
pub mod bass;

/// Kick/snare processing evidence and explicit rhythmic balance.
pub mod drums;

/// Conditional snare spill evidence and protected static trials.
pub mod bleed;

/// Frozen multi-reference identifiability diagnostics; never changes audio.
pub mod bleed_reference;

/// Continuous DSP history must not carry a held-out passage into later training.
/// Call alongside the owning policy's span validation; this checks ordering only.
fn training_precedes_held_out(training: &[[f64; 2]], held_out: &[[f64; 2]]) -> bool {
    !training.is_empty()
        && !held_out.is_empty()
        && training
            .iter()
            .chain(held_out)
            .flatten()
            .all(|v| v.is_finite())
        && training
            .iter()
            .map(|s| s[1])
            .fold(f64::NEG_INFINITY, f64::max)
            <= held_out.iter().map(|s| s[0]).fold(f64::INFINITY, f64::min)
}

/// Frozen whole-band observation and preparation contracts.
pub mod ensemble;

pub mod dynamic_eq;
