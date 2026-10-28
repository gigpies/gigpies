//! Explicit reviewed event intent; energy alone is not a sibilance classification.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    IsolatedSibilance,
    TransientResonance,
    LowLoad,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trial {
    pub profile: String,
    pub evidence: Evidence,
    pub processor: crate::automix::dynamic_eq::Config,
    pub problem_spans: Vec<[f64; 2]>,
    pub protected_spans: Vec<[f64; 2]>,
    pub max_protected_loss_db: f64,
}
impl Trial {
    pub fn validate(&self, s: &Session, r: &Request) -> Result<()> {
        self.processor.validate(s.sample_rate)?;
        let p = r
            .profiles
            .iter()
            .find(|p| p.name == self.profile)
            .ok_or("dynamic trial needs a profile")?;
        if p.channels.len() != 1
            || s.channels[p.channels[0]].dynamic_eq.is_some()
            || p.identity_gap(r).is_some()
            || p.tone.is_none()
            || !p.allow_static_eq
            || !self.max_protected_loss_db.is_finite()
            || !(0. ..=1.).contains(&self.max_protected_loss_db)
        {
            return Err("dynamic trial requires one identified baseline source, enabled tone intent, no existing dynamic band, and 0..1 dB quiet budget".into());
        }
        if matches!(self.evidence, Evidence::IsolatedSibilance)
            && (!matches!(
                p.family,
                profile::Family::LeadVocal | profile::Family::BackingVocal
            ) || p.capture != "isolated_vocal"
                || self.processor.hz < 3000.)
        {
            return Err(
                "de-essing requires reviewed isolated vocal capture and high-band intent".into(),
            );
        }
        if matches!(
            p.family,
            profile::Family::Overheads
                | profile::Family::DrumRoom
                | profile::Family::VocalRoom
                | profile::Family::PrintedEffects
        ) {
            return Err("mixed/printed source event classification unsupported".into());
        }
        if matches!(self.evidence, Evidence::LowLoad) && self.processor.hz > 120. {
            return Err("low-load evidence requires a detector at or below 120 Hz".into());
        }
        let tone = p.tone.as_ref().unwrap();
        if self.processor.depth_db > tone.max_cut_db
            || self.processor.hz != tone.control_hz
            || self.processor.q != tone.control_q
        {
            return Err(
                "dynamic trial must share the static control band and cumulative depth budget"
                    .into(),
            );
        }
        crate::automix::matching::validate_spans(&self.problem_spans)?;
        crate::automix::matching::validate_spans(&self.protected_spans)?;
        if self.problem_spans.is_empty()
            || self.protected_spans.is_empty()
            || self.problem_spans.iter().any(|a| {
                self.protected_spans
                    .iter()
                    .any(|b| a[0] < b[1] && b[0] < a[1])
            })
        {
            return Err(
                "dynamic event and protection passages must be nonempty and disjoint".into(),
            );
        }
        for split in [&r.training, &r.held_out] {
            if intersect(split, &self.problem_spans).is_empty()
                || intersect(split, &self.protected_spans).is_empty()
            {
                return Err("both splits require problem and protected passages".into());
            }
        }
        Ok(())
    }
}
pub fn intersect(a: &[[f64; 2]], b: &[[f64; 2]]) -> Vec<[f64; 2]> {
    a.iter()
        .flat_map(|a| {
            b.iter().filter_map(|b| {
                let x = [a[0].max(b[0]), a[1].min(b[1])];
                (x[0] < x[1]).then_some(x)
            })
        })
        .collect()
}
