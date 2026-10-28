use super::*;
use crate::automix::write_json;
use std::fs;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub baseline: Session,
    pub request: Request,
    pub frozen: Frozen,
}
impl State {
    pub fn validate(&self) -> Result<()> {
        self.baseline.validate()?;
        let f = &self.frozen;
        if f.version != VERSION
            || f.algorithm != ALGORITHM
            || f.baseline_id != identity(&self.baseline)?
            || f.request_id != identity(&self.request)?
            || f.id
                != identity(&(
                    VERSION,
                    ALGORITHM,
                    &f.baseline_id,
                    &f.request_id,
                    &f.inputs,
                    f.timeline_frames,
                ))?
        {
            return Err("invalid frozen ensemble state".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub channel: usize,
    pub role: crate::automix::config::Role,
    pub profile: Option<String>,
    pub applicability: String,
    pub changed: bool,
    pub reason: String,
}
pub fn decisions(state: &State) -> Vec<Decision> {
    state.baseline.channels.iter().enumerate().map(|(channel,c)| {
        let profile=state.request.profiles.iter().find(|p|p.channels.contains(&channel));
        let (applicability,reason)=match profile {
            None=>("observation_only","Bind an explicit family, performance, capture and numerical objective to enable correction.".into()),
            Some(p)=>if let Some(gap)=p.identity_gap(&state.request) {("missing_identity",gap)}
                else if p.tone.is_none() || !p.allow_static_eq {("disabled","No enabled tone action; preserve baseline settings.".into())}
                else if let Some(gap) = p.family.capture_gap(&p.capture) {("unsupported", gap.into())}
                else if p.family.supports_static() {("requires_training_validation", "Single-source static trials require explicit passages and protection evidence.".into())} else {("unsupported","This profile has observations but no validated correction family yet.".into())},
        };
        Decision{channel,role:c.role,profile:profile.map(|p|p.name.clone()),applicability:applicability.into(),changed:false,reason}
    }).collect()
}
fn write(out: &Path, state: &State, m: &measure::Measurement, mode: &str) -> Result<()> {
    fs::create_dir(out)?;
    write_json(&out.join("state.json"), state)?;
    write_json(&out.join("baseline.json"), &state.baseline)?;
    write_json(&out.join("settings.json"), &state.baseline)?;
    write_json(&out.join("observations.json"), m)?;
    let decisions = decisions(state);
    write_json(
        &out.join("report.json"),
        &serde_json::json!({"version":VERSION,"mode":mode,"frozen_id":state.frozen.id,"decisions":decisions,"changed":false,"all_targets_met":null,"listener_preferred":null,"finalized_export_checked":false}),
    )?;
    let mut report = format!(
        "Ensemble {mode}: unchanged baseline.\n\n{} channels, {} native frames at {} Hz.\n",
        state.baseline.channels.len(),
        m.frames,
        m.sample_rate
    );
    for d in &decisions {
        report.push_str(&format!(
            "\nChannel {} ({:?}): {} — {}\n",
            d.channel, d.role, d.applicability, d.reason
        ));
    }
    report.push_str(
        "\nChannel observations do not establish final mix, FX, export or listener acceptance.\n",
    );
    fs::write(out.join("report.txt"), report)?;
    Ok(())
}
pub fn analyze(s: Session, root: &Path, out: &Path, request: Request) -> Result<()> {
    if out.exists() {
        return Err("output already exists".into());
    }
    let frozen = freeze(&s, root, &request)?;
    let m = measure::measure(&s, root, &request, &frozen)?;
    write(
        out,
        &State {
            baseline: s,
            request,
            frozen,
        },
        &m,
        "analyze",
    )
}
pub fn prepare(state: State, root: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("output already exists".into());
    }
    state.validate()?;
    let m = measure::measure(&state.baseline, root, &state.request, &state.frozen)?;
    if state.request.hierarchy.is_none()
        && state
            .request
            .profiles
            .iter()
            .all(|p| p.tone.is_none() || !p.allow_static_eq)
    {
        return write(out, &state, &m, "prepare");
    }
    let mut outcome = super::correct::prepare(&state, root, &m)?;
    outcome.report["version"] = serde_json::json!(VERSION);
    outcome.report["frozen_id"] = serde_json::json!(state.frozen.id);
    write(out, &state, &m, "prepare")?;
    write_json(&out.join("candidate.json"), &outcome.candidate)?;
    if outcome.report["accepted"] == true {
        outcome.report["actual_dsp_evaluations"] = serde_json::json!(
            outcome.report["actual_dsp_evaluations"]
                .as_u64()
                .unwrap_or(0)
                + 2
        );
        let composition = super::composition::verify(&state, &outcome.candidate, root, out)?;
        if composition["passed"] != true {
            outcome.report["accepted"] = serde_json::json!(false);
            outcome.report["changed"] = serde_json::json!(false);
            outcome.report["all_targets_met"] = serde_json::json!(false);
            outcome.settings = state.baseline.clone();
        }
        outcome.report["composition"] = composition;
    }
    if outcome.report["accepted"] == true {
        outcome.report["actual_dsp_evaluations"] = serde_json::json!(
            outcome.report["actual_dsp_evaluations"]
                .as_u64()
                .unwrap_or(0)
                + 1
        );
        let policy = delivery_policy();
        crate::automix::run_policy(outcome.settings.clone(), root, &out.join("export"), policy)?;
        crate::automix::delivery::check(&out.join("export"), root)?;
        outcome.report["finalized_export_checked"] = serde_json::json!(true);
    }
    state.frozen.verify(&state.baseline, root, &state.request)?;
    fs::write(
        out.join("settings.json"),
        serde_json::to_vec_pretty(&outcome.settings)?,
    )?;

    let report = format!(
        "Ensemble preparation: changed={}, accepted={}, all_targets_met={}.\nSee report.json for controlled trials, abstentions and held-out vetoes.\nListener preference remains unqualified.\n",
        outcome.report["changed"], outcome.report["accepted"], outcome.report["all_targets_met"]
    );
    fs::write(out.join("report.txt"), report)?;
    fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&outcome.report)?,
    )?;
    Ok(())
}
pub fn reset(state: State, out: &Path) -> Result<()> {
    state.validate()?;
    fs::create_dir(out)?;
    write_json(&out.join("settings.json"), &state.baseline)?;
    write_json(&out.join("state.json"), &state)
}

pub(super) fn delivery_policy() -> crate::automix::delivery::Policy {
    crate::automix::delivery::Policy {
        delivery_gain: crate::automix::delivery::DeliveryGain::IndependentPeak { max_boost_db: 0. },
        ..Default::default()
    }
}
