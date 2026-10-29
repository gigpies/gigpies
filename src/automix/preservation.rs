//! Explicit offline source preservation. This is a selectable baseline, not an
//! "already processed" detector or a claim that the recording has no defects.
use super::{balance::validate_baseline, config::Session, write_json};
use crate::inventory::Result;
use std::path::Path;

/// Recover the supplied session's SOURCE path without changing musical routing.
/// Callers must supply the established initial faders/pan to reproduce SOURCE.
/// Existing frozen mixes are never implicitly reset by analysis or rendering.
pub fn source_settings(s: &Session) -> Result<Session> {
    validate_baseline(s)?;
    if s.master_db != 0. {
        return Err("source preservation requires unity master gain".into());
    }
    let mut source = s.clone();
    source.effects = None;
    source.master_hpf_hz = 0.;
    for ch in &mut source.channels {
        ch.hpf_hz = 0.;
        ch.eq.clear();
        ch.dynamic_eq = None;
        ch.compressor.ratio = 1.;
        ch.compressor.makeup_db = 0.;
    }
    source.validate()?;
    Ok(source)
}

/// An explicit choice to prepare the unchanged source sum as FINAL. No source
/// level calibration, role preset, loss compensation, or artistic fader move.
pub fn prepare(s: Session, out: &Path) -> Result<()> {
    let source = source_settings(&s)?;
    std::fs::create_dir(out)?;
    write_json(&out.join("before-settings.json"), &s)?;
    write_json(&out.join("settings.json"), &source)?;
    write_json(
        &out.join("decision.json"),
        &serde_json::json!({
            "selection": "preserve_source",
            "basis": "Explicit source-preservation request; no corrective or artistic intervention selected",
            "source_quality": "ungraded; processing history cannot be recovered from the waveform",
            "observed_problem": null,
            "correction_confidence": null,
            "expected_benefit": "Retain supplied envelopes, tone and ensemble balance through the established routing",
            "tradeoff": "Any recorded defects and printed effects remain; no source repair or hardware protection is claimed",
            "baseline_can_win": true,
            "routing_provenance": "Caller must pin initial SOURCE settings; later faders do not redefine SOURCE",
            "protection": "Schema, finite samples, BWF timing, linked stereo and independent sample-peak export remain enforced by production rendering",
            "listener_accepted": null
        }),
    )
}
