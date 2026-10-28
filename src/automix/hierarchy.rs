//! Explicit offline source LUFS hierarchy; not an auditory masking model.
use super::{
    balance::validate_baseline,
    config::{Role, Session},
    dsp::{Loudness, Strip, gain},
    render::{Source, route},
    write_json,
};
use crate::inventory::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub kick_to_bass_lu: f64,
    pub bass_to_others_lu: f64,
    pub max_fader_change_db: f64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            kick_to_bass_lu: 2.,
            bass_to_others_lu: 1.,
            max_fader_change_db: 6.,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<()> {
        if ![self.kick_to_bass_lu, self.bass_to_others_lu]
            .iter()
            .all(|x| x.is_finite() && (0.1..=12.).contains(x))
            || !self.max_fader_change_db.is_finite()
            || !(0. ..=12.).contains(&self.max_fader_change_db)
        {
            return Err(
                "hierarchy requires positive finite gaps and a 0..12 dB change budget".into(),
            );
        }
        Ok(())
    }
}
fn anchors(s: &Session) -> Result<(usize, usize)> {
    let indices = |kick| {
        s.channels
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                if kick {
                    matches!(c.role, Role::Kick)
                } else {
                    matches!(c.role, Role::BassDi)
                }
            })
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let (k, b) = (indices(true), indices(false));
    if k.len() != 1 || b.len() != 1 {
        return Err("hierarchy requires exactly one kick and one DI bass; combine or resolve microphone roles explicitly".into());
    }
    Ok((k[0], b[0]))
}
#[derive(Debug, Serialize)]
pub struct Proposal {
    pub deltas_db: Vec<f64>,
    pub targets_lufs: Vec<Option<f64>>,
    pub reason: String,
}
/// Keep kick fixed; move bass to its offset, and only lower other sources above
/// their ceiling. A missing anchor abstains. Bounds never silently redefine intent.
pub fn propose(s: &Session, levels: &[Option<f64>], p: &Policy) -> Result<Proposal> {
    s.validate()?;
    p.validate()?;
    let (kick, bass) = anchors(s)?;
    if levels.len() != s.channels.len() || levels.iter().flatten().any(|x| !x.is_finite()) {
        return Err("invalid source loudness measurements".into());
    }
    let mut result = Proposal {
        deltas_db: vec![0.; levels.len()],
        targets_lufs: vec![None; levels.len()],
        reason: "Bounded proposal; actual remeasurement determines target compliance.".into(),
    };
    let (Some(k), Some(_)) = (levels[kick], levels[bass]) else {
        result.reason = "No change: kick or bass lacks gated loudness evidence.".into();
        return Ok(result);
    };
    let bass_target = k - p.kick_to_bass_lu;
    for (i, level) in levels.iter().enumerate() {
        let Some(level) = level else { continue };
        let target = if i == kick {
            k
        } else if i == bass {
            bass_target
        } else {
            level.min(bass_target - p.bass_to_others_lu)
        };
        result.targets_lufs[i] = Some(target);
        result.deltas_db[i] = (target - level)
            .clamp(-p.max_fader_change_db, p.max_fader_change_db)
            .clamp(-80. - s.channels[i].fader_db, 12. - s.channels[i].fader_db);
    }
    Ok(result)
}
/// Measure production channel DSP and stereo routing before master/export.
/// No FX is allowed until its excitation and returns have an explicit policy.
pub fn measure(s: &Session, root: &Path) -> Result<Vec<Option<f64>>> {
    validate_baseline(s)?;
    if s.effects.is_some() {
        return Err("source LUFS hierarchy currently requires no FX configuration".into());
    }
    let mut sources = s
        .channels
        .iter()
        .map(|c| Source::open(&root.join(&c.file), s.sample_rate))
        .collect::<Result<Vec<_>>>()?;
    let refs = sources
        .iter()
        .filter_map(|x| x.reference)
        .collect::<Vec<_>>();
    if !refs.is_empty() && refs.len() != sources.len() {
        return Err("mixed BWF references".into());
    }
    let origin = refs.iter().min().copied().unwrap_or(0);
    let mut end = 0;
    for src in &mut sources {
        src.offset = src.reference.unwrap_or(0) - origin;
        end = end.max(
            src.offset
                .checked_add(src.frames)
                .ok_or("timeline overflow")?,
        );
    }
    let mut strips = s
        .channels
        .iter()
        .map(|c| Strip::new(c, s.sample_rate))
        .collect::<Vec<_>>();
    let mut meters = s
        .channels
        .iter()
        .map(|_| Loudness::new(s.sample_rate))
        .collect::<Vec<_>>();
    for t in 0..end {
        for (i, src) in sources.iter_mut().enumerate() {
            let x = strips[i].tick(src.next(t)?);
            meters[i].add(route(
                x,
                src.channels,
                s.channels[i].pan,
                gain(s.channels[i].fader_db),
            ));
        }
    }
    Ok(meters.iter().map(Loudness::integrated).collect())
}
pub fn run(s: Session, root: &Path, out: &Path, p: Policy) -> Result<()> {
    p.validate()?;
    anchors(&s)?;
    if out.exists() {
        return Err("output already exists".into());
    }
    let before = measure(&s, root)?;
    let proposal = propose(&s, &before, &p)?;
    let mut candidate = s.clone();
    for (c, delta) in candidate.channels.iter_mut().zip(&proposal.deltas_db) {
        c.fader_db += delta;
    }
    candidate.validate()?;
    let after = measure(&candidate, root)?;
    let (kick, bass) = anchors(&s)?;
    let targets_met = match (after[kick], after[bass]) {
        (Some(k), Some(b)) => {
            (k - b - p.kick_to_bass_lu).abs() <= 0.1
                && after.iter().enumerate().all(|(i, x)| {
                    i == kick || i == bass || x.is_some_and(|x| x <= b - p.bass_to_others_lu + 0.1)
                })
        }
        _ => false,
    };
    std::fs::create_dir(out)?;
    write_json(&out.join("policy.json"), &p)?;
    write_json(&out.join("baseline.json"), &s)?;
    write_json(&out.join("settings.json"), &candidate)?;
    write_json(
        &out.join("report.json"),
        &serde_json::json!({
            "before_lufs": before, "after_lufs": after, "proposal": proposal,
            "policy_targets_met": targets_met, "listener_preferred": null,
            "scope": "Independently gated whole-recording source LUFS after channel DSP/fader/pan, before master/export. Individual sources, not summed ensemble. No masking, transient dominance, section-wise hierarchy or bass-tone guarantee. Missing loudness is unresolved, never zero.",
            "no_playback": true
        }),
    )?;
    write_json(
        &out.join("status.json"),
        &serde_json::json!({"technical_checks_passed": true, "policy_targets_met": targets_met}),
    )
}
