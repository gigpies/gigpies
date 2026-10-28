//! Versioned, immutable inputs and full-band measurement for offline ensemble work.
//! No source correction is authorized by a role label or an energy measurement.
use super::{
    config::Session,
    identity::{SourceIdentity, identity, source_identities},
    render::Source,
};
use crate::inventory::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
pub mod composition;
mod correct;
pub mod dynamic;
pub mod group_levels;
pub mod listening;
pub mod measure;
pub mod profile;
pub mod spectrum;
pub mod workflow;

pub const VERSION: u32 = 2;
pub const ALGORITHM: &str = "ensemble-observation-2";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub max_static_trials: usize,
    pub max_channels: usize,
    pub max_seconds: f64,
    pub max_sample_visits: u64,
    pub max_source_bytes: u64,
    pub max_feature_records: u64,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            max_static_trials: 3,
            max_channels: 64,
            max_seconds: 1800.,
            max_sample_visits: 5_529_600_000,
            max_source_bytes: 24_000_000_000,
            max_feature_records: 2_000_000,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CaptureIdentity {
    pub channel: usize,
    /// Caller-declared performance identity, never inferred from role or filename.
    pub performance: Option<String>,
    pub capture: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    pub dynamic_trials: Vec<dynamic::Trial>,
    #[serde(default)]
    pub grouped_hierarchy: bool,
    #[serde(default)]
    pub composition: composition::Protection,
    pub version: u32,
    pub budget: Budget,
    pub captures: Vec<CaptureIdentity>,
    #[serde(default)]
    pub profiles: Vec<profile::Profile>,
    #[serde(default)]
    pub training: Vec<[f64; 2]>,
    #[serde(default)]
    pub held_out: Vec<[f64; 2]>,
    #[serde(default)]
    pub hierarchy: Option<super::hierarchy::Policy>,
}
impl Default for Request {
    fn default() -> Self {
        Self {
            dynamic_trials: Vec::new(),
            grouped_hierarchy: false,
            composition: composition::Protection::default(),
            version: VERSION,
            budget: Budget::default(),
            captures: Vec::new(),
            profiles: Vec::new(),
            training: Vec::new(),
            held_out: Vec::new(),
            hierarchy: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub source: SourceIdentity,
    pub channels: usize,
    pub frames: u64,
    pub bwf_reference: Option<u64>,
    pub offset: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frozen {
    pub version: u32,
    pub algorithm: String,
    pub baseline_id: String,
    pub request_id: String,
    pub id: String,
    pub sample_rate: u32,
    pub timeline_frames: u64,
    pub sample_visits: u64,
    pub inputs: Vec<Input>,
}
impl Frozen {
    pub fn verify(&self, s: &Session, root: &Path, request: &Request) -> Result<()> {
        let now = freeze(s, root, request)?;
        if serde_json::to_value(self)? != serde_json::to_value(now)? {
            return Err("stale ensemble baseline, source, request or algorithm".into());
        }
        Ok(())
    }
}
/// Admit the complete native timeline before hashing/processing it. BWF gaps count
/// against duration and CPU budgets. No resampling or implicit identity grouping.
pub fn freeze(s: &Session, root: &Path, request: &Request) -> Result<Frozen> {
    s.validate()?;
    if !s.prepared
        || s.calibration.initial_trim_db != 0.
        || s.groups.iter().any(|g| g.trim_db != 0.)
    {
        return Err(
            "ensemble observation requires prepared settings and explicit unity trims".into(),
        );
    }
    request.composition.validate()?;
    let b = &request.budget;
    if !(1..=3).contains(&b.max_static_trials)
        || request.version != VERSION
        || b.max_channels == 0
        || b.max_channels > 64
        || !b.max_seconds.is_finite()
        || !(0.001..=86400.).contains(&b.max_seconds)
        || s.channels.len() > b.max_channels
        || b.max_sample_visits == 0
        || b.max_source_bytes == 0
    {
        return Err("invalid or exceeded ensemble analysis budget/version".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &request.captures {
        if c.channel >= s.channels.len()
            || !seen.insert(c.channel)
            || [&c.performance, &c.capture].iter().any(|v| {
                v.as_ref()
                    .is_some_and(|s| s.trim().is_empty() || s.len() > 512)
            })
        {
            return Err("invalid/duplicate declared capture identity".into());
        }
    }
    if b.max_feature_records == 0 || b.max_feature_records > 8_000_000 {
        return Err("feature record budget must be 1..8000000".into());
    }
    if !request.training.is_empty() || !request.held_out.is_empty() {
        super::matching::validate_spans(&request.training)?;
        super::matching::validate_spans(&request.held_out)?;
        if !super::training_precedes_held_out(&request.training, &request.held_out) {
            return Err("training must precede held-out on the continuous timeline".into());
        }
    }
    if let Some(policy) = &request.hierarchy {
        policy.validate()?;
    }
    let mut profiled = std::collections::BTreeSet::new();
    let mut profile_names = std::collections::BTreeSet::new();
    for profile in &request.profiles {
        if !profile_names.insert(&profile.name) {
            return Err("duplicate ensemble profile name".into());
        }
        profile.validate(s, request)?;
        for &channel in &profile.channels {
            if !profiled.insert(channel) {
                return Err("channel belongs to multiple profiles".into());
            }
        }
    }
    let mut dynamic_profiles = std::collections::BTreeSet::new();
    for trial in &request.dynamic_trials {
        if !dynamic_profiles.insert(&trial.profile) {
            return Err("duplicate dynamic trial".into());
        }
        trial.validate(s, request)?;
    }
    let mut total_bytes = 0_u64;
    for c in &s.channels {
        total_bytes = total_bytes
            .checked_add(std::fs::metadata(root.join(&c.file))?.len())
            .ok_or("source byte overflow")?;
        if total_bytes > b.max_source_bytes {
            return Err("source byte budget exceeded".into());
        }
    }
    let sources = s
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
    for src in &sources {
        end = end.max(
            (src.reference.unwrap_or(0) - origin)
                .checked_add(src.frames)
                .ok_or("timeline overflow")?,
        );
    }
    if end == 0 {
        return Err("empty ensemble timeline".into());
    }
    let tail = s.effects.as_ref().map_or(0, |fx| {
        (fx.tail_seconds * s.sample_rate as f64).round() as u64
    });
    end = end.checked_add(tail).ok_or("FX tail timeline overflow")?;
    let visits = end
        .checked_mul(s.channels.len() as u64)
        .ok_or("sample visit overflow")?;
    if end as f64 / s.sample_rate as f64 > b.max_seconds || visits > b.max_sample_visits {
        return Err("native timeline exceeds ensemble analysis budget".into());
    }
    let records = end
        .div_ceil(measure::FFT_FRAMES as u64)
        .checked_mul(6)
        .and_then(|n| n.checked_add((end * 100).div_ceil(u64::from(s.sample_rate))))
        .and_then(|n| {
            n.checked_mul(
                (s.channels.len()
                    + request
                        .profiles
                        .iter()
                        .filter(|p| p.channels.len() > 1)
                        .count()) as u64,
            )
        })
        .ok_or("feature budget overflow")?;
    if records > b.max_feature_records {
        return Err("feature record budget exceeded".into());
    }
    if request
        .training
        .iter()
        .chain(&request.held_out)
        .any(|span| span[1] > end as f64 / s.sample_rate as f64)
    {
        return Err("declared passage extends beyond native timeline".into());
    }
    if request
        .dynamic_trials
        .iter()
        .flat_map(|t| t.problem_spans.iter().chain(&t.protected_spans))
        .any(|span| span[1] > end as f64 / s.sample_rate as f64)
    {
        return Err("dynamic passage beyond timeline".into());
    }
    let identities = source_identities(s, root)?;
    let inputs = sources
        .iter()
        .zip(identities)
        .map(|(src, source)| Input {
            source,
            channels: src.channels,
            frames: src.frames,
            bwf_reference: src.reference,
            offset: src.reference.unwrap_or(0) - origin,
        })
        .collect::<Vec<_>>();
    let baseline_id = identity(s)?;
    let request_id = identity(request)?;
    let id = identity(&(VERSION, ALGORITHM, &baseline_id, &request_id, &inputs, end))?;
    Ok(Frozen {
        version: VERSION,
        algorithm: ALGORITHM.into(),
        baseline_id,
        request_id,
        id,
        sample_rate: s.sample_rate,
        timeline_frames: end,
        sample_visits: visits,
        inputs,
    })
}
