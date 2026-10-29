//! Explicit performance-group hierarchy before nonlinear master processing.
use super::*;
use crate::automix::{
    config::Role,
    dsp::{Loudness, Strip, gain},
    effects::Rack,
    render::route,
};
use serde_json::{Value, json};

#[derive(Debug, Serialize)]
pub struct Levels {
    pub members: Vec<Vec<usize>>,
    pub identities: Vec<String>,
    pub direct_lufs: Vec<Option<f64>>,
    pub direct_and_dedicated_returns_lufs: Vec<Option<f64>>,
    pub section_mean_power: Vec<Vec<f64>>,
    pub sections: Vec<[f64; 2]>,
    pub section_lufs: Vec<Vec<Option<f64>>>,
}
pub fn measure(s: &Session, root: &Path, r: &Request, grouped: bool, end: u64) -> Result<Levels> {
    measure_selected(s, root, r, grouped, end, false)
}
pub fn measure_training(
    s: &Session,
    root: &Path,
    r: &Request,
    grouped: bool,
    end: u64,
) -> Result<Levels> {
    measure_selected(s, root, r, grouped, end, true)
}
fn measure_selected(
    s: &Session,
    root: &Path,
    r: &Request,
    grouped: bool,
    end: u64,
    training_only: bool,
) -> Result<Levels> {
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut identities = Vec::new();
    for i in 0..s.channels.len() {
        let id = r
            .captures
            .iter()
            .find(|c| c.channel == i)
            .and_then(|c| c.performance.clone())
            .ok_or("hierarchy needs every performance identity")?;
        if let Some(g) = identities.iter().position(|x| x == &id) {
            if !grouped {
                return Err("duplicate performance: explicitly enable grouped_hierarchy to change hierarchy semantics".into());
            }
            members[g].push(i);
        } else {
            identities.push(id);
            members.push(vec![i]);
        }
    }
    let which = (0..s.channels.len())
        .map(|i| members.iter().position(|g| g.contains(&i)).unwrap())
        .collect::<Vec<_>>();
    let mut bus_groups = Vec::new();
    if let Some(fx) = &s.effects {
        for bus in &fx.buses {
            let groups = bus
                .sends
                .iter()
                .map(|send| which[send.channel])
                .collect::<std::collections::BTreeSet<_>>();
            if groups.len() != 1 {
                return Err("shared or unbound FX return cannot be attributed to one instrument; dedicate sends or omit hierarchy".into());
            }
            bus_groups.push(*groups.first().unwrap());
        }
    }
    let mut observation_request = r.clone();
    observation_request.dynamic_trials.clear();
    let f = freeze(s, root, &observation_request)?;
    let mut sources = s
        .channels
        .iter()
        .zip(&f.inputs)
        .map(|(c, id)| {
            let mut src = Source::open(&root.join(&c.file), s.sample_rate)?;
            src.offset = id.offset;
            Ok(src)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut strips = s
        .channels
        .iter()
        .map(|c| Strip::new(c, s.sample_rate))
        .collect::<Vec<_>>();
    let mut rack = s
        .effects
        .as_ref()
        .map(|fx| Rack::new(fx, s.channels.len(), s.sample_rate));
    let mut direct = (0..members.len())
        .map(|_| Loudness::new(s.sample_rate))
        .collect::<Vec<_>>();
    let mut combined = (0..members.len())
        .map(|_| Loudness::new(s.sample_rate))
        .collect::<Vec<_>>();
    let sections = r
        .training
        .iter()
        .chain(&r.held_out)
        .copied()
        .collect::<Vec<_>>();
    let mut section_meters = sections
        .iter()
        .map(|_| {
            (0..members.len())
                .map(|_| Loudness::new(s.sample_rate))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut direct_sections = sections
        .iter()
        .map(|_| {
            (0..members.len())
                .map(|_| Loudness::new(s.sample_rate))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut energy = vec![vec![0.; members.len()]; sections.len()];
    let mut counts = vec![0u64; sections.len()];
    let mut sums = vec![[0.; 2]; members.len()];
    for t in 0..end.min(f.timeline_frames) {
        sums.fill([0.; 2]);
        for (i, src) in sources.iter_mut().enumerate() {
            let mut x = strips[i].tick(src.next(t)?);
            if let Some(fx) = &mut rack {
                x = fx.excite(i, x);
            }
            let x = route(
                x,
                src.channels,
                s.channels[i].pan,
                gain(s.channels[i].fader_db),
            );
            for (sum, v) in sums[which[i]].iter_mut().zip(x) {
                *sum += v;
            }
            if let Some(fx) = &mut rack {
                fx.send(i, x);
            }
        }
        for (meter, x) in direct.iter_mut().zip(&sums) {
            meter.add(*x);
        }
        for (j, span) in sections.iter().enumerate() {
            let included = t as f64 / s.sample_rate as f64 >= span[0]
                && (t + 1) as f64 / s.sample_rate as f64 <= span[1] + 1e-9;
            for (m, x) in direct_sections[j].iter_mut().zip(&sums) {
                m.add_selected(*x, included);
            }
        }
        if let Some(fx) = &mut rack {
            fx.returns();
            for (bus, &g) in bus_groups.iter().enumerate() {
                for (sum, value) in sums[g].iter_mut().zip(fx.return_outputs[bus]) {
                    *sum += value;
                }
            }
        }
        for (meter, x) in combined.iter_mut().zip(&sums) {
            meter.add(*x);
        }
        for (j, span) in sections.iter().enumerate() {
            let included = t as f64 / s.sample_rate as f64 >= span[0]
                && (t + 1) as f64 / s.sample_rate as f64 <= span[1] + 1e-9;
            for (meter, x) in section_meters[j].iter_mut().zip(&sums) {
                meter.add_selected(*x, included);
            }
            if included {
                counts[j] += 1;
                for (p, x) in energy[j].iter_mut().zip(&sums) {
                    *p += (x[0] * x[0] + x[1] * x[1]) * 0.5;
                }
            }
        }
    }
    for (row, n) in energy.iter_mut().zip(counts) {
        for p in row {
            *p /= n.max(1) as f64;
        }
    }
    let selected_levels = |meters: &[Vec<Loudness>]| {
        (0..members.len())
            .map(|g| {
                let parts = meters
                    .iter()
                    .take(r.training.len())
                    .map(|row| &row[g])
                    .collect::<Vec<_>>();
                Loudness::integrated_parts(&parts)
            })
            .collect::<Vec<_>>()
    };
    let direct_lufs = if training_only {
        selected_levels(&direct_sections)
    } else {
        direct.iter().map(Loudness::integrated).collect()
    };
    let combined_lufs = if training_only {
        selected_levels(&section_meters)
    } else {
        combined.iter().map(Loudness::integrated).collect()
    };
    Ok(Levels {
        members,
        identities,
        direct_lufs,
        direct_and_dedicated_returns_lufs: combined_lufs,
        section_mean_power: energy,
        section_lufs: section_meters
            .iter()
            .map(|row| row.iter().map(Loudness::integrated).collect())
            .collect(),
        sections,
    })
}
fn anchors(s: &Session, m: &Levels) -> Result<(usize, usize)> {
    let find = |kick| {
        m.members
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                g.iter().any(|&i| {
                    if kick {
                        matches!(s.channels[i].role, Role::Kick)
                    } else {
                        matches!(s.channels[i].role, Role::BassDi)
                    }
                })
            })
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let k = find(true);
    let b = find(false);
    if k.len() != 1 || b.len() != 1 || k[0] == b[0] {
        return Err("resolve one distinct kick and bass performance".into());
    }
    // An anchor cannot absorb an unrelated role to satisfy the hierarchy.
    for (g, kick) in [(k[0], true), (b[0], false)] {
        if m.members[g].iter().any(|&i| {
            if kick {
                !matches!(s.channels[i].role, Role::Kick)
            } else {
                !matches!(s.channels[i].role, Role::BassDi | Role::BassAmp)
            }
        }) {
            return Err("mixed-role anchor group".into());
        }
    }
    Ok((k[0], b[0]))
}
pub fn propose(s: &Session, m: &Levels, p: &crate::automix::hierarchy::Policy) -> Result<Vec<f64>> {
    let (k, b) = anchors(s, m)?;
    let levels = &m.direct_and_dedicated_returns_lufs;
    let target = levels[k].ok_or("missing group kick LUFS")? - p.kick_to_bass_lu;
    if levels[b].is_none() {
        return Err("missing group bass LUFS".into());
    }
    let mut deltas = vec![0.; s.channels.len()];
    for (g, group) in m.members.iter().enumerate() {
        let level = levels[g].ok_or("missing group loudness")?;
        let desired = if g == k {
            0.
        } else if g == b {
            target - level
        } else {
            (target - p.bass_to_others_lu - level).min(0.)
        };
        let lo = group
            .iter()
            .map(|&i| -80. - s.channels[i].fader_db)
            .fold(-p.max_fader_change_db, f64::max);
        let hi = group
            .iter()
            .map(|&i| 12. - s.channels[i].fader_db)
            .fold(p.max_fader_change_db, f64::min);
        for &i in group {
            deltas[i] = desired.clamp(lo, hi);
        }
    }
    Ok(deltas)
}
pub fn compliance(s: &Session, m: &Levels, p: &crate::automix::hierarchy::Policy) -> Result<Value> {
    let (k, b) = anchors(s, m)?;
    let levels = &m.direct_and_dedicated_returns_lufs;
    let ok = levels[k].zip(levels[b]).is_some_and(|(kl, bl)| {
        (kl - bl - p.kick_to_bass_lu).abs() <= 0.1
            && levels.iter().enumerate().all(|(i, l)| {
                i == k || i == b || l.is_some_and(|l| l <= bl - p.bass_to_others_lu + 0.1)
            })
    });
    let sections=m.section_mean_power.iter().zip(&m.sections).zip(&m.section_lufs).map(|((row,span),lu)| {
        let compliant=lu[k].zip(lu[b]).map(|(kl,bl)|(kl-bl-p.kick_to_bass_lu).abs()<=0.1 && lu.iter().enumerate().all(|(i,l)|i==k || i==b || l.is_some_and(|l|l<=bl-p.bass_to_others_lu+0.1)));
        let delta=|a:usize,b:usize|(row[a]>1e-20 && row[b]>1e-20).then(||10.*(row[a]/row[b]).log10());
        json!({"span":span,"lufs":lu,"lufs_targets_met":compliant,"kick_minus_bass_energy_db":delta(k,b),"other_minus_bass_energy_db":(0..row.len()).filter(|&i|i!=k && i!=b).map(|i|delta(i,b)).collect::<Vec<_>>(),"metric":"common-window mean energy, not LUFS; silence is missing evidence"})
    }).collect::<Vec<_>>();
    let all_sections_targets_met = !sections.is_empty()
        && sections
            .iter()
            .all(|section| section["lufs_targets_met"] == true);
    Ok(
        json!({"whole_source_targets_met":ok,"all_sections_targets_met":all_sections_targets_met,"measurement":m,"sections":sections,"scope":"declared coherent performances after channel DSP/excitation/routing plus dedicated returns, before nonlinear master; shared-return attribution abstains"}),
    )
}
