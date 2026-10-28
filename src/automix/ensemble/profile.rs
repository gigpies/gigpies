use super::*;
use crate::automix::config::Role;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Kick,
    Bass,
    Snare,
    Tom,
    Overheads,
    DrumRoom,
    ElectricGuitar,
    AcousticGuitar,
    LeadVocal,
    BackingVocal,
    VocalRoom,
    HiHat,
    Piano,
    Keys,
    Synth,
    Strings,
    Winds,
    PrintedEffects,
}
impl Family {
    /// Static correction is limited to explicitly declared, isolated captures.
    /// Printed effects have no separable dry-source preservation contract.
    pub fn supports_static(self) -> bool {
        !matches!(self, Self::PrintedEffects)
    }

    pub fn capture_gap(self, capture: &str) -> Option<&'static str> {
        if !self.supports_static() {
            Some("printed effects require dry-source identity and printed-processing evidence")
        } else if matches!(self, Self::Overheads | Self::DrumRoom | Self::VocalRoom)
            && capture != "isolated_ensemble_capture"
        {
            Some(
                "mixed microphone: declare isolated_ensemble_capture only after reviewing bleed and intended decay; energy cannot identify unwanted bleed",
            )
        } else {
            None
        }
    }
    pub fn matches(self, role: Role) -> bool {
        match self {
            Self::Kick => matches!(role, Role::Kick),
            Self::Bass => matches!(role, Role::BassDi | Role::BassAmp),
            Self::Snare => matches!(role, Role::Snare),
            Self::Tom => matches!(role, Role::Tom),
            Self::Overheads => matches!(role, Role::Overheads),
            Self::DrumRoom => matches!(role, Role::DrumRoom),
            Self::ElectricGuitar => matches!(role, Role::RhythmGuitar | Role::LeadGuitar),
            Self::AcousticGuitar => matches!(role, Role::AcousticGuitar),
            Self::LeadVocal => matches!(role, Role::LeadVocal),
            Self::BackingVocal => matches!(role, Role::BackingVocal),
            Self::VocalRoom => matches!(role, Role::VocalRoom),
            _ => matches!(role, Role::Other),
        }
    }
}
/// An explicit range in a measured band-power ratio, never a role preset.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tone {
    pub numerator_hz: [f64; 2],
    pub denominator_hz: [f64; 2],
    pub target_db: [f64; 2],
    pub control_hz: f64,
    pub control_q: f64,
    pub max_cut_db: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub channels: Vec<usize>,
    pub family: Family,
    pub capture: String,
    pub supported_register_hz: Option<[f64; 2]>,
    pub tone: Option<Tone>,
    pub allow_static_eq: bool,
    pub max_output_rise_db: f64,
    pub max_protected_loss_db: f64,
    pub minimum_active_windows: usize,
    pub activity_floor_dbfs: f64,
}
impl Profile {
    pub fn validate(&self, s: &Session, request: &Request) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.len() > 120
            || self.channels.is_empty()
            || self.channels.len() > s.channels.len()
            || self.capture.trim().is_empty()
            || self.capture.len() > 512
            || self.minimum_active_windows == 0
            || self.minimum_active_windows > 10000
            || !valid(self.activity_floor_dbfs, -100., -20.)
            || !valid(self.max_output_rise_db, 0., 6.)
            || !valid(self.max_protected_loss_db, 0., 6.)
        {
            return Err("invalid ensemble profile/budgets".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut performance = None;
        for &i in &self.channels {
            if i >= s.channels.len() || !seen.insert(i) || !self.family.matches(s.channels[i].role)
            {
                return Err("profile channel/family mismatch".into());
            }
            let declared = request.captures.iter().find(|c| c.channel == i);
            // Missing identity is an actionable abstention, not inferred applicability.
            if let Some(id) = declared.and_then(|c| c.performance.as_ref()) {
                if performance.is_some_and(|p| p != id) {
                    return Err("group includes different declared performances".into());
                }
                performance = Some(id);
            }
        }
        let band = |b: [f64; 2]| {
            b[0].is_finite() && b[1].is_finite() && b[0] >= 0. && b[1] > b[0] && b[1] <= 96000.
        };
        if self.supported_register_hz.is_some_and(|b| !band(b)) {
            return Err("invalid supported register".into());
        }
        if let Some(t) = &self.tone
            && (!band(t.numerator_hz)
                || !band(t.denominator_hz)
                || !valid(t.target_db[0], -60., 60.)
                || !valid(t.target_db[1], t.target_db[0], 60.)
                || !valid(t.control_hz, 10., s.sample_rate as f64 * 0.45)
                || !valid(t.control_q, 0.3, 2.)
                || !valid(t.max_cut_db, 0., 6.))
        {
            return Err("invalid explicit tone objective/control".into());
        }
        Ok(())
    }
    pub fn identity_gap(&self, request: &Request) -> Option<String> {
        for &i in &self.channels {
            let c = request.captures.iter().find(|c| c.channel == i);
            if c.is_none_or(|c| {
                c.performance.is_none() || c.capture.as_deref() != Some(self.capture.as_str())
            }) {
                return Some(format!(
                    "channel {i}: declare performance and matching capture before correction"
                ));
            }
        }
        None
    }
}
fn valid(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}
