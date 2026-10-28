//! Final production-render observations. Never selects against held-out evidence.
use super::*;
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protection {
    pub max_peak_rise_db: f64,
    pub max_mix_loss_db: f64,
    pub max_mono_loss_db: f64,
}
impl Default for Protection {
    fn default() -> Self {
        Self {
            max_peak_rise_db: 1.,
            max_mix_loss_db: 6.,
            max_mono_loss_db: 1.,
        }
    }
}
impl Protection {
    pub fn validate(&self) -> Result<()> {
        if [
            self.max_peak_rise_db,
            self.max_mix_loss_db,
            self.max_mono_loss_db,
        ]
        .iter()
        .any(|v| !v.is_finite() || !(0. ..=12.).contains(v))
        {
            return Err("invalid composition protection budget".into());
        }
        Ok(())
    }
}
pub(super) fn verify(
    state: &workflow::State,
    candidate: &Session,
    root: &Path,
    out: &Path,
) -> Result<Value> {
    let seconds = state.frozen.timeline_frames as f64 / candidate.sample_rate as f64;
    let mut observed = Vec::new();
    for (name, s) in [
        ("baseline-composition", &state.baseline),
        ("candidate-composition", candidate),
    ] {
        let dir = out.join(name);
        crate::automix::render::observe_policy(
            s.clone(),
            root,
            &dir,
            seconds,
            workflow::delivery_policy(),
        )?;
        observed.push(serde_json::from_reader::<_, Value>(std::fs::File::open(
            dir.join("stages.json"),
        )?)?);
    }
    let [a, b] = [&observed[0], &observed[1]];
    let mut failures = Vec::<String>::new();
    let budget = &state.request.composition;
    for i in 0..4 {
        let x = &a["stages"][i];
        let y = &b["stages"][i];
        let rise = y["peak_dbfs"].as_f64().ok_or("missing composition peak")?
            - x["peak_dbfs"].as_f64().ok_or("missing baseline peak")?;
        let change = y["rms_dbfs"].as_f64().ok_or("missing composition energy")?
            - x["rms_dbfs"].as_f64().ok_or("missing baseline energy")?;
        if rise > budget.max_peak_rise_db + 1e-6 {
            failures.push(format!("stage {i}: peak-rise budget exceeded"));
        }
        if change < -budget.max_mix_loss_db - 1e-6 {
            failures.push(format!("stage {i}: mix-energy loss budget exceeded"));
        }
    }
    let mono_change = b["mono_minus_stereo_db"]
        .as_f64()
        .ok_or("missing mono evidence")?
        - a["mono_minus_stereo_db"]
            .as_f64()
            .ok_or("missing mono baseline")?;
    if mono_change < -budget.max_mono_loss_db - 1e-6 {
        failures.push("final mono compatibility regressed".into());
    }
    // Check every common 100 ms production stage window, including recovery/tails.
    let read = |name: &str| -> Result<Vec<Vec<String>>> {
        Ok(
            std::fs::read_to_string(out.join(name).join("stage-windows.csv"))?
                .lines()
                .skip(1)
                .filter(|line| line.contains(",stage_rms_peak,"))
                .map(|line| line.split(',').map(String::from).collect())
                .collect::<Vec<_>>(),
        )
    };
    let before = read("baseline-composition")?;
    let after = read("candidate-composition")?;
    if before.len() != after.len() {
        return Err("composition window coverage changed".into());
    }
    for (x, y) in before.iter().zip(&after) {
        if x[..3] != y[..3] {
            return Err("composition window alignment changed".into());
        }
        let energy = y[3].parse::<f64>()?.max(-100.) - x[3].parse::<f64>()?.max(-100.);
        let peak = y[4].parse::<f64>()?.max(-100.) - x[4].parse::<f64>()?.max(-100.);
        if peak > budget.max_peak_rise_db + 1e-6 || energy < -budget.max_mix_loss_db - 1e-6 {
            failures.push(format!(
                "frame {} stage {}: composition window protection failed",
                x[0], x[2]
            ));
        }
    }
    Ok(
        json!({"passed":failures.is_empty(),"failures":failures,"baseline":a,"candidate":b,"windows_checked":before.len(),"window_floor_dbfs":-100.,"fixed_fx_unchanged":identity(&state.baseline.effects)?==identity(&candidate.effects)?,"scope":"actual production direct, direct+returns, master HPF, FX master and final limiter; common windows include tails; no nonlinear per-source attribution"}),
    )
}
