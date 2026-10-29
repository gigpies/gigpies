//! Training-only bounded trials followed by one frozen-bundle held-out veto.
use super::{
    measure::{Measurement, Window},
    profile::{Family, Profile},
    *,
};
use crate::automix::{
    config::{EqBand, EqKind},
    dsp::Biquad,
};
use serde_json::{Value, json};

pub(super) struct Outcome {
    pub candidate: Session,
    pub settings: Session,
    pub report: Value,
}
fn inside(start: u64, frames: usize, rate: u32, spans: &[[f64; 2]]) -> bool {
    spans.iter().any(|s| {
        start as f64 / rate as f64 >= s[0]
            && (start + frames as u64) as f64 / rate as f64 <= s[1] + 1e-9
    })
}
fn power(m: &Measurement, w: &Window, stage: usize, band: [f64; 2]) -> Option<f64> {
    if band[1] > m.sample_rate as f64 * 0.5 {
        return None;
    }
    let mut total = 0.;
    let mut count = 0;
    for (edge, value) in m.band_edges_hz.windows(2).zip(&w.stages[stage].band_power) {
        if edge[0] >= band[0] && edge[1] <= band[1] {
            total += (*value)?;
            count += 1;
        }
    }
    (count > 0 && total > 1e-20).then_some(total)
}
fn active(w: &Window, p: &Profile) -> bool {
    w.stages[0].mean_power > 10_f64.powf(p.activity_floor_dbfs / 10.)
}
fn score(m: &Measurement, p: &Profile, spans: &[[f64; 2]], mask: &Measurement) -> Option<f64> {
    spans
        .iter()
        .map(|span| passage_score(m, p, &[*span], mask))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .reduce(f64::max)
}
fn score_channel(m: &Measurement, p: &Profile, channel: usize) -> bool {
    m.profile_groups
        .iter()
        .find(|(name, _)| name == &p.name)
        .map_or_else(|| p.channels.contains(&channel), |(_, id)| channel == *id)
}
fn protected_channel(m: &Measurement, p: &Profile, channel: usize) -> bool {
    p.channels.contains(&channel) || score_channel(m, p, channel)
}
fn passage_score(
    m: &Measurement,
    p: &Profile,
    spans: &[[f64; 2]],
    mask: &Measurement,
) -> Option<f64> {
    let t = p.tone.as_ref()?;
    let mut ratios = mask
        .windows
        .iter()
        .enumerate()
        .filter(|(_, w)| {
            score_channel(mask, p, w.channel)
                && active(w, p)
                && inside(w.start, w.frames, mask.sample_rate, spans)
                && (p.family != Family::Bass
                    || w.note_hz.is_some_and(|hz| {
                        p.supported_register_hz
                            .is_some_and(|r| hz >= r[0] && hz <= r[1])
                    }))
        })
        .filter_map(|(index, baseline)| {
            let w = m.windows.get(index)?;
            if w.start != baseline.start
                || w.frames != baseline.frames
                || w.channel != baseline.channel
            {
                return None;
            }
            Some(
                10. * (power(m, w, 3, t.numerator_hz)? / power(m, w, 3, t.denominator_hz)?).log10(),
            )
        })
        .collect::<Vec<_>>();
    if ratios.len() < p.minimum_active_windows {
        return None;
    }
    ratios.sort_by(f64::total_cmp);
    let value = ratios[ratios.len() / 2];
    Some((t.target_db[0] - value).max(0.) + (value - t.target_db[1]).max(0.))
}

fn db_ratio(after: f64, before: f64) -> f64 {
    10. * (after.max(1e-30) / before.max(1e-30)).log10()
}
/// Every protected window and event must pass. No aggregate can cancel a failure.
fn guards(
    before: &Measurement,
    after: &Measurement,
    p: &Profile,
    spans: &[[f64; 2]],
) -> Vec<String> {
    let mut failures = Vec::new();
    for (index, a) in before.windows.iter().enumerate() {
        if !protected_channel(before, p, a.channel)
            || !active(a, p)
            || !inside(a.start, a.frames, before.sample_rate, spans)
        {
            continue;
        }
        let Some(b) = after
            .windows
            .get(index)
            .filter(|b| b.channel == a.channel && b.start == a.start && b.frames == a.frames)
        else {
            failures.push("missing matching protected window".into());
            break;
        };
        if db_ratio(b.stages[3].peak.powi(2), a.stages[3].peak.powi(2))
            > p.max_output_rise_db + 1e-6
        {
            failures.push("protected peak rise".into());
        }
        if let Some(t) = &p.tone {
            if let (Some(x), Some(y)) = (
                power(before, a, 3, t.denominator_hz),
                power(after, b, 3, t.denominator_hz),
            ) {
                if db_ratio(y, x) < -p.max_protected_loss_db - 1e-6 {
                    failures.push("protected denominator-band loss".into());
                }
            } else {
                failures.push("missing protected spectral evidence".into());
            }
        }
        if let Some(register) = p.supported_register_hz {
            match (power(before, a, 3, register), power(after, b, 3, register)) {
                (Some(x), Some(y)) if db_ratio(y, x) >= -p.max_protected_loss_db - 1e-6 => {}
                _ => failures.push("missing or attenuated protected register".into()),
            }
        }
        if p.family == Family::Bass
            && let (Some(x), Some(y)) =
                (a.stages[3].fundamental_power, b.stages[3].fundamental_power)
            && db_ratio(y, x) < -p.max_protected_loss_db - 1e-6
        {
            failures.push("protected bass fundamental loss".into());
        }
    }
    for (index, a) in before.events.iter().enumerate() {
        if !protected_channel(before, p, a.channel)
            || a.raw_power <= 10_f64.powf(p.activity_floor_dbfs / 10.)
            || !inside(a.start, a.frames as usize, before.sample_rate, spans)
        {
            continue;
        }
        let Some(b) = after
            .events
            .get(index)
            .filter(|b| b.channel == a.channel && b.start == a.start && b.frames == a.frames)
        else {
            failures.push("missing matching protected event".into());
            break;
        };
        let change = db_ratio(b.processed_power, a.processed_power);
        let peak = db_ratio(b.processed_peak.powi(2), a.processed_peak.powi(2));
        if change < -p.max_protected_loss_db - 1e-6 || peak < -p.max_protected_loss_db - 1e-6 {
            failures.push("quiet/attack/sustain event loss".into());
        }
        if change > p.max_output_rise_db + 1e-6 || peak > p.max_output_rise_db + 1e-6 {
            failures.push("event energy/peak rise".into());
        }
    }
    failures.sort();
    failures.dedup();
    failures
}
fn quiet_guards(
    before: &Measurement,
    after: &Measurement,
    p: &Profile,
    d: &super::dynamic::Trial,
    split: &[[f64; 2]],
) -> Vec<String> {
    let spans = super::dynamic::intersect(split, &d.protected_spans);
    let mut failures = Vec::new();
    let mut count = 0;
    for (a, b) in before.events.iter().zip(&after.events) {
        if p.channels.contains(&a.channel)
            && inside(a.start, a.frames as usize, before.sample_rate, &spans)
            && a.raw_power > 10_f64.powf(p.activity_floor_dbfs / 10.)
        {
            count += 1;
            if db_ratio(b.processed_power, a.processed_power) < -d.max_protected_loss_db - 1e-6 {
                failures.push(format!("{}: protected event passage loss", p.name));
            }
        }
    }
    if count == 0 {
        failures.push(format!("{}: missing protected event evidence", p.name));
    }
    failures.sort();
    failures.dedup();
    failures
}
fn reduction(m: &Measurement, p: &Profile, spans: &[[f64; 2]]) -> Option<f64> {
    let values = m
        .events
        .iter()
        .filter(|e| {
            p.channels.contains(&e.channel)
                && inside(e.start, e.frames as usize, m.sample_rate, spans)
        })
        .map(|e| e.max_reduction_db)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}
fn measure_trial(s: &Session, root: &Path, r: &Request, end: Option<u64>) -> Result<Measurement> {
    let mut observation_request = r.clone();
    observation_request.dynamic_trials.clear();
    let f = freeze(s, root, &observation_request)?;
    measure::measure_to(s, root, &observation_request, &f, end)
}
fn eligible(p: &Profile, state: &workflow::State, base: &Measurement) -> Option<String> {
    if let Some(gap) = p.identity_gap(&state.request) {
        return Some(gap);
    }
    if let Some(gap) = p.family.capture_gap(&p.capture) {
        return Some(gap.into());
    }
    let c = p.channels[0];
    if matches!(
        state.baseline.channels[c].role,
        crate::automix::config::Role::BassAmp
    ) && p.channels.len() == 1
    {
        return Some("bass amp paths require declared coherent DI/amp group validation".into());
    }
    let performance = state
        .request
        .captures
        .iter()
        .find(|x| x.channel == c)
        .and_then(|x| x.performance.as_ref());
    if state
        .request
        .captures
        .iter()
        .any(|x| !p.channels.contains(&x.channel) && x.performance.as_ref() == performance)
    {
        return Some("related microphone paths require coherent group validation".into());
    }

    if p.tone.is_none() || !p.allow_static_eq {
        return Some("static tone correction disabled".into());
    }
    if p.channels
        .iter()
        .any(|&c| state.baseline.channels[c].eq.len() >= 8)
    {
        return Some("EQ capacity exhausted; existing controls preserved".into());
    }
    if p.family == Family::Bass {
        let count = base
            .windows
            .iter()
            .filter(|w| {
                w.channel == c
                    && active(w, p)
                    && w.note_hz.is_some_and(|hz| {
                        p.supported_register_hz
                            .is_some_and(|r| hz >= r[0] && hz <= r[1])
                    })
                    && inside(w.start, w.frames, base.sample_rate, &state.request.training)
            })
            .count();
        if count < p.minimum_active_windows {
            return Some("insufficient bass note support in the declared register".into());
        }
    }
    None
}
pub(super) fn prepare(state: &workflow::State, root: &Path, base: &Measurement) -> Result<Outcome> {
    let started = std::time::Instant::now();
    let s = &state.baseline;
    let r = &state.request;
    if r.training.is_empty() || r.held_out.is_empty() {
        return Ok(Outcome {
            candidate: s.clone(),
            settings: s.clone(),
            report: json!({"changed":false,"all_targets_met":null,"reason":"Explicit training-before-held-out passages required for correction; baseline preserved."}),
        });
    }
    let training_end =
        (r.training.iter().map(|s| s[1]).fold(0., f64::max) * s.sample_rate as f64).ceil() as u64;
    let mut bundle = s.clone();
    let mut trials = Vec::new();
    let mut selected = Vec::new();
    let mut abstentions = Vec::new();
    let mut evaluations = 0;
    for p in &r.profiles {
        if let Some(reason) = eligible(p, state, base) {
            abstentions.push(json!({"profile":p.name,"reason":reason}));
            continue;
        }
        let dynamic = r.dynamic_trials.iter().find(|d| d.profile == p.name);
        let training_spans = dynamic.map_or_else(
            || r.training.clone(),
            |d| super::dynamic::intersect(&r.training, &d.problem_spans),
        );
        let Some(before) = score(base, p, &training_spans, base) else {
            abstentions.push(
                json!({"profile":p.name,"reason":"insufficient supported active training spectra"}),
            );
            continue;
        };
        if before <= 0.05 {
            abstentions.push(
                json!({"profile":p.name,"reason":"declared tone range already met; no change"}),
            );
            continue;
        }
        let t = p.tone.as_ref().unwrap();
        let channel = p.channels[0];
        let mut best = before;
        let mut choice = None;
        let trial_count = r
            .budget
            .max_static_trials
            .min(if dynamic.is_some() { 2 } else { 3 });
        for number in 1..=trial_count {
            let fraction = number as f64 / trial_count as f64;
            let eq = EqBand {
                kind: EqKind::Bell,
                hz: t.control_hz,
                q: t.control_q,
                db: -t.max_cut_db * fraction,
            };
            let filter = Biquad::equalizer(&eq, s.sample_rate);
            if p.family == Family::Bass
                && base
                    .windows
                    .iter()
                    .filter(|w| {
                        w.channel == channel
                            && inside(w.start, w.frames, s.sample_rate, &r.training)
                    })
                    .filter_map(|w| w.note_hz)
                    .any(|hz| {
                        10. * filter.power_response(hz, s.sample_rate).max(1e-30).log10()
                            < -p.max_protected_loss_db
                    })
            {
                continue;
            }
            let mut candidate = s.clone();
            for &c in &p.channels {
                candidate.channels[c].eq.push(eq.clone());
            }
            let m = measure_trial(&candidate, root, r, Some(training_end))?;
            evaluations += 1;
            let mut failures = guards(base, &m, p, &r.training);
            if let Some(d) = dynamic {
                failures.extend(quiet_guards(base, &m, p, d, &r.training));
            }
            let error = score(&m, p, &training_spans, base);
            trials.push(json!({"profile":p.name,"kind":"eq_only","eq_db":eq.db,"mean_event_max_reduction_db":reduction(&m,p,&r.training),"baseline_mean_event_max_reduction_db":reduction(base,p,&r.training),"training_error_db":error,"protection_failures":failures}));
            if failures.is_empty() && error.is_some_and(|error| error + 0.05 < best) {
                best = error.unwrap();
                choice = Some(eq);
            }
        }
        // Controlled compressor-only intervention is evidence, not an enabled action.
        let mut relief = s.clone();
        relief.channels[channel].compressor.threshold_db =
            (relief.channels[channel].compressor.threshold_db + 3.).min(0.);
        let m = measure_trial(&relief, root, r, Some(training_end))?;
        evaluations += 1;
        trials.push(json!({"profile":p.name,"kind":"compressor_only_comparison_not_applied","threshold_raise_db":relief.channels[channel].compressor.threshold_db-s.channels[channel].compressor.threshold_db,"mean_event_max_reduction_db":reduction(&m,p,&r.training),"baseline_mean_event_max_reduction_db":reduction(base,p,&r.training),"training_error_db":score(&m,p,&r.training,base),"protection_failures":guards(base,&m,p,&r.training),"interpretation":"Controlled intervention only; a changed ratio does not prove sub-driven compression."}));
        let mut dynamic_choice = None;
        if let Some(d) = dynamic {
            let mut candidate = s.clone();
            candidate.channels[channel].dynamic_eq = Some(d.processor.clone());
            let m = measure_trial(&candidate, root, r, Some(training_end))?;
            evaluations += 1;
            let mut failures = guards(base, &m, p, &r.training);
            failures.extend(quiet_guards(base, &m, p, d, &r.training));
            let error = score(&m, p, &training_spans, base);
            trials.push(json!({"profile":p.name,"kind":"reviewed_dynamic_event","evidence":d.evidence,"training_error_db":error,"best_static_error_db":best,"protection_failures":failures}));
            if failures.is_empty() && error.is_some_and(|e| e + 0.05 < best) {
                best = error.unwrap();
                dynamic_choice = Some(d.processor.clone());
            }
        }
        if let Some(processor) = dynamic_choice {
            bundle.channels[channel].dynamic_eq = Some(processor.clone());
            selected.push(json!({"profile":p.name,"channel":channel,"dynamic_eq":processor,"training_error_before_db":before,"training_error_after_db":best}));
        } else if let Some(eq) = choice {
            for &c in &p.channels {
                bundle.channels[c].eq.push(eq.clone());
            }
            selected.push(json!({"profile":p.name,"channel":channel,"channels":p.channels,"eq":eq,"training_error_before_db":before,"training_error_after_db":best}));
        } else {
            abstentions.push(json!({"profile":p.name,"reason":"No static trial improves intent within every protection budget; event-specific evidence is required before time-selective correction."}));
        }
    }
    let mut hierarchy_report = Value::Null;
    if let Some(policy) = &r.hierarchy {
        if r.grouped_hierarchy || s.effects.is_some() {
            evaluations += 1;
            let proposal = (|| -> Result<_> {
                let levels = super::group_levels::measure_training(
                    &bundle,
                    root,
                    r,
                    r.grouped_hierarchy,
                    training_end,
                )?;
                let deltas = super::group_levels::propose(&bundle, &levels, policy)?;
                Ok((levels, deltas))
            })();
            match proposal {
                Ok((levels, deltas)) => {
                    for (c, delta) in bundle.channels.iter_mut().zip(&deltas) {
                        c.fader_db += delta;
                    }
                    hierarchy_report = json!({"training_groups":levels,"deltas_db":deltas});
                }
                Err(e) => hierarchy_report = json!({"abstention":e.to_string()}),
            }
        } else {
            evaluations += 1;
            let levels =
                super::group_levels::measure_training(&bundle, root, r, false, training_end)
                    .map(|m| m.direct_lufs);
            let identities = (0..s.channels.len())
                .map(|i| {
                    r.captures
                        .iter()
                        .find(|c| c.channel == i)
                        .and_then(|c| c.performance.as_ref())
                })
                .collect::<Vec<_>>();
            let unique = identities
                .iter()
                .flatten()
                .collect::<std::collections::BTreeSet<_>>();
            let proposal = if s.effects.is_some() {
                Err("fixed FX hierarchy requires ensemble composition validation".into())
            } else if identities.iter().any(|id| id.is_none()) || unique.len() != s.channels.len() {
                Err("hierarchy requires explicit independent performance identities; grouped mapping remains unresolved".into())
            } else {
                levels
                    .and_then(|levels| crate::automix::hierarchy::propose(&bundle, &levels, policy))
            };
            match proposal {
                Ok(proposal) => {
                    let baseline_training =
                        super::group_levels::measure_training(s, root, r, false, training_end)?;
                    evaluations += 1;
                    let level_only = crate::automix::hierarchy::propose(
                        s,
                        &baseline_training.direct_lufs,
                        policy,
                    )?;
                    for (c, delta) in bundle.channels.iter_mut().zip(&proposal.deltas_db) {
                        c.fader_db += delta;
                    }
                    hierarchy_report = json!({"training_proposal":proposal,"level_only":level_only,"scope":"training-derived individual routed-source hierarchy; final whole-source and held-out compliance are separate"});
                }
                Err(e) => hierarchy_report = json!({"abstention":e.to_string()}),
            }
        }
    }
    // The bundle is frozen here. Held-out evidence can only veto it, never retune it.
    let bundle_id = identity(&bundle)?;
    let final_measure = measure_trial(&bundle, root, r, None)?;
    evaluations += 1;
    let mut failures = Vec::new();
    for p in &r.profiles {
        failures.extend(
            guards(base, &final_measure, p, &r.training)
                .into_iter()
                .map(|f| format!("{}: assembled training {f}", p.name)),
        );
        if score(&final_measure, p, &r.training, base)
            .zip(score(base, p, &r.training, base))
            .is_some_and(|(after, before)| after > before + 0.1)
        {
            failures.push(format!(
                "{}: assembled training objective regressed",
                p.name
            ));
        }
    }
    if base
        .channels
        .iter()
        .any(|c| c.integer_full_scale_samples > 0)
    {
        failures.push("input PCM full-scale contact: source review required; processing cannot repair clipping".into());
    }
    let mut targets = abstentions.iter().all(|a| {
        a["reason"].as_str().is_some_and(|reason| {
            reason == "declared tone range already met; no change"
                || reason == "static tone correction disabled"
        })
    });
    for p in &r.profiles {
        if p.tone.is_none() || !p.allow_static_eq {
            continue;
        }
        failures.extend(
            guards(base, &final_measure, p, &r.held_out)
                .into_iter()
                .map(|f| format!("{}: {f}", p.name)),
        );
        let d = r.dynamic_trials.iter().find(|d| d.profile == p.name);
        let spans = d.map_or_else(
            || r.held_out.clone(),
            |d| super::dynamic::intersect(&r.held_out, &d.problem_spans),
        );
        if let Some(d) = d {
            failures.extend(quiet_guards(base, &final_measure, p, d, &r.held_out));
        }
        let before = score(base, p, &spans, base);
        let after = score(&final_measure, p, &spans, base);
        targets &= after.is_some_and(|e| e <= 0.1);
        if after.is_none() || before.zip(after).is_some_and(|(a, b)| b > a + 0.1) {
            failures.push(format!(
                "{}: missing or regressed held-out objective",
                p.name
            ));
        }
    }
    if let Some(policy) = &r.hierarchy {
        evaluations += 1;
        let sections = super::group_levels::measure(
            &bundle,
            root,
            r,
            r.grouped_hierarchy,
            state.frozen.timeline_frames,
        )
        .and_then(|m| super::group_levels::compliance(&bundle, &m, policy));
        match sections {
            Ok(value) => {
                // A whole-song average cannot certify the declared passages.
                // Missing section evidence is also an unmet objective.
                targets &= value["all_sections_targets_met"] == true;
                if r.grouped_hierarchy || s.effects.is_some() {
                    targets &= value["whole_source_targets_met"] == true;
                }
                hierarchy_report["group_and_section_validation"] = value;
            }
            Err(e) => {
                targets = false;
                hierarchy_report["group_and_section_gap"] = json!(e.to_string());
            }
        }
        if r.grouped_hierarchy || s.effects.is_some() {
            hierarchy_report["whole_source_targets_met"] =
                hierarchy_report["group_and_section_validation"]["whole_source_targets_met"]
                    .clone();
            targets &= hierarchy_report.get("abstention").is_none();
        } else {
            use crate::automix::config::Role;
            let kick = s.channels.iter().position(|c| matches!(c.role, Role::Kick));
            let bass = s
                .channels
                .iter()
                .position(|c| matches!(c.role, Role::BassDi));
            let compliant = kick
                .zip(bass)
                .and_then(|(k, b)| {
                    final_measure.channels[k]
                        .integrated_lufs
                        .zip(final_measure.channels[b].integrated_lufs)
                        .map(|(kl, bl)| {
                            (kl - bl - policy.kick_to_bass_lu).abs() <= 0.1
                                && final_measure.channels.iter().enumerate().all(|(i, c)| {
                                    i == k
                                        || i == b
                                        || c.integrated_lufs.is_some_and(|l| {
                                            l <= bl - policy.bass_to_others_lu + 0.1
                                        })
                                })
                        })
                })
                .unwrap_or(false);
            targets &= compliant && hierarchy_report.get("abstention").is_none();
            hierarchy_report["whole_source_targets_met"] = json!(compliant);
            hierarchy_report["after_lufs"] = json!(
                final_measure
                    .channels
                    .iter()
                    .map(|c| c.integrated_lufs)
                    .collect::<Vec<_>>()
            );
        }
    }
    let accepted = failures.is_empty();
    let changed = accepted && identity(s)? != bundle_id;
    Ok(Outcome {
        candidate: bundle.clone(),
        settings: if accepted { bundle } else { s.clone() },
        report: json!({"changed":changed,"accepted":accepted,"all_targets_met":accepted && targets,"bundle_id":bundle_id,"trials":trials,"selected":selected,"abstentions":abstentions,"held_out_failures":failures,"hierarchy":hierarchy_report,"actual_dsp_evaluations":evaluations+1,"baseline_measurement_included":true,"elapsed_seconds":started.elapsed().as_secs_f64(),"search_budget":{"spectral_proposals_per_instrument":r.budget.max_static_trials,"dsp_finalists_per_instrument":4,"ensemble_refinement_rounds":0},"held_out_used_for_selection":false,"finalized_export_checked":false,"listener_preferred":null}),
    })
}
