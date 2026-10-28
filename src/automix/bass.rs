//! Offline DI/kick evidence and intent-conditioned correction. No hardware access.
use super::{
    analysis::fft,
    balance::validate_baseline,
    config::{EqBand, EqKind, Role, Session},
    dsp::{Biquad, Strip, db, gain},
    render::{Source, route, write_json},
};
use crate::inventory::Result;
use serde::{Deserialize, Serialize};
use std::{
    f64::consts::FRAC_1_SQRT_2,
    path::{Path, PathBuf},
};

pub const SIZE: usize = 16384;
pub const BANDS: [(f64, f64); 9] = [
    (25., 40.),
    (40., 65.),
    (65., 100.),
    (100., 200.),
    (200., 400.),
    (400., 800.),
    (800., 1600.),
    (1600., 3200.),
    (3200., 8000.),
];
pub const STAGES: [&str; 11] = [
    "bass_raw",
    "bass_eq",
    "bass_compressed_with_makeup",
    "bass_routed",
    "bass_after_master_hpf",
    "kick_raw",
    "kick_after_master_hpf",
    "guitars_after_master_hpf",
    "vocals_after_master_hpf",
    "mix_before_master_hpf",
    "mix_after_master_hpf",
];
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub bass: usize,
    pub bass_file: PathBuf,
    pub kick: usize,
    pub kick_file: PathBuf,
    /// Explicit intent: definition (400–1600) relative to body (100–400).
    pub definition_body_db: [f64; 2],
    /// Optional independent intent: 400–1600 Hz definition / all energy below 100 Hz.
    /// Omission preserves historical policies; no universal bass spectrum is inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_low_db: Option<[f64; 2]>,
    pub max_eq_db: f64,
    pub max_output_rise_db: f64,
    pub max_note_spread_db: f64,
    pub max_added_reduction_db: f64,
    pub max_fader_db: f64,
    pub recorded_source: bool,
}
impl Policy {
    pub fn validate(&self, s: &Session) -> Result<()> {
        validate_baseline(s)?;
        if s.effects.is_some()
            || self.bass == self.kick
            || !s
                .channels
                .get(self.bass)
                .is_some_and(|c| matches!(c.role, Role::BassDi) && c.file == self.bass_file)
            || !s
                .channels
                .get(self.kick)
                .is_some_and(|c| matches!(c.role, Role::Kick) && c.file == self.kick_file)
            || !self
                .definition_body_db
                .iter()
                .all(|v| v.is_finite() && (-40. ..=20.).contains(v))
            || self.definition_body_db[0] >= self.definition_body_db[1]
            || self.definition_low_db.is_some_and(|r| {
                r.iter()
                    .any(|v| !v.is_finite() || !(-60. ..=20.).contains(v))
                    || r[0] >= r[1]
            })
            || ![
                (self.max_eq_db, 6.),
                (self.max_output_rise_db, 6.),
                (self.max_note_spread_db, 3.),
                (self.max_added_reduction_db, 2.),
                (self.max_fader_db, 3.),
            ]
            .iter()
            .all(|(x, cap)| x.is_finite() && (0. ..=*cap).contains(x))
        {
            return Err("bass policy requires verified DI/kick identity, no FX, finite bounded intent and budgets".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Note {
    pub hz: f64,
    pub midi: i32,
    pub cents: f64,
    pub periodicity: f64,
    pub harmonic_fraction: f64,
}
fn power_db(v: f64) -> f64 {
    db(v.max(0.).sqrt())
}
pub(super) fn spectrum(x: &[[f64; 2]]) -> Vec<f64> {
    let n = x.len();
    let mut p = vec![0.; n / 2 + 1];
    for ch in 0..2 {
        let mut a = x
            .iter()
            .enumerate()
            .map(|(i, v)| {
                [
                    v[ch] * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos()),
                    0.,
                ]
            })
            .collect::<Vec<_>>();
        fft(&mut a);
        for (k, v) in p.iter_mut().enumerate() {
            // One-sided power doubles only bins with a negative-frequency partner.
            // DC and Nyquist must not be doubled, especially in the sub-energy report.
            let weight = if k == 0 || k == n / 2 { 1. } else { 2. };
            *v += (a[k][0].powi(2) + a[k][1].powi(2)) * (weight / (n * n) as f64 / 0.375) / 2.;
        }
    }
    p
}
pub(super) fn band(p: &[f64], rate: u32, lo: f64, hi: f64) -> f64 {
    p.iter()
        .enumerate()
        .filter(|(k, _)| {
            let hz = *k as f64 * rate as f64 / SIZE as f64;
            hz >= lo && hz < hi
        })
        .map(|(_, v)| v)
        .sum()
}
/// Periodicity alone can mistake a harmonic/bleed for a note. Require an observed
/// fundamental, harmonic concentration, and proximity to equal-tempered pitch.
/// Unknown/polyphonic/transition windows retain spectra but do not vote on note EQ.
pub fn estimate_note(x: &[[f64; 2]], rate: u32) -> Option<Note> {
    if x.len() != SIZE
        || !(8000..=192000).contains(&rate)
        || x.iter().flatten().any(|v| !v.is_finite())
    {
        return None;
    }
    let p = spectrum(x);
    let step = (rate / 6000).max(1) as usize;
    let y = x
        .chunks(step)
        .map(|c| c.iter().map(|v| v[0]).sum::<f64>() / c.len() as f64)
        .collect::<Vec<_>>();
    let r = rate as f64 / step as f64;
    let lo = (r / 160.).floor() as usize;
    let hi = (r / 30.).ceil() as usize;
    if y.len() < hi * 3 || power_db(y.iter().map(|v| v * v).sum::<f64>() / y.len() as f64) < -65. {
        return None;
    }
    let mut ac = vec![0.; hi + 2];
    for (lag, v) in ac
        .iter_mut()
        .enumerate()
        .take(hi + 2)
        .skip(lo.saturating_sub(1))
    {
        let (mut xy, mut xx, mut yy) = (0., 0., 0.);
        for i in lag..y.len() {
            xy += y[i] * y[i - lag];
            xx += y[i] * y[i];
            yy += y[i - lag] * y[i - lag];
        }
        *v = xy / (xx * yy).sqrt().max(1e-30);
    }
    let best = (lo..=hi).map(|i| ac[i]).fold(0_f64, f64::max);
    let lag = (lo..=hi).find(|&i| {
        ac[i] >= 0.80 && ac[i] >= best - 0.04 && ac[i] > ac[i - 1] && ac[i] >= ac[i + 1]
    })?;
    let offset = 0.5 * (ac[lag - 1] - ac[lag + 1]) / (ac[lag - 1] - 2. * ac[lag] + ac[lag + 1]);
    let hz = r / (lag as f64 + offset.clamp(-0.5, 0.5));
    let pitch = 69. + 12. * (hz / 440.).log2();
    let midi = pitch.round() as i32;
    let cents = (pitch - midi as f64) * 100.;
    let total = band(&p, rate, 25., 3200.);
    let width = (rate as f64 / SIZE as f64 * 1.5).max(hz * 0.04);
    let fundamental = band(&p, rate, hz - width, hz + width);
    let harmonics = (1..=10)
        .map(|h| band(&p, rate, hz * h as f64 - width, hz * h as f64 + width))
        .sum::<f64>()
        / total.max(1e-30);
    if cents.abs() > 40. || fundamental / total.max(1e-30) < 0.03 || harmonics < 0.65 {
        return None;
    }
    Some(Note {
        hz,
        midi,
        cents,
        periodicity: ac[lag],
        harmonic_fraction: harmonics,
    })
}
/// Unweighted spectral power: no claim that a strong fundamental is unwanted rumble.
#[derive(Clone, Serialize)]
pub struct LowEnergy {
    pub below_25_dbfs: f64,
    pub from_25_to_100_dbfs: f64,
    pub below_100_fraction: Option<f64>,
    pub definition_low_db: Option<f64>,
}
pub fn low_energy(sp: &[f64], rate: u32) -> LowEnergy {
    let total = sp.iter().sum::<f64>();
    let low = band(sp, rate, 0., 100.);
    let definition = band(sp, rate, 400., 1600.);
    LowEnergy {
        below_25_dbfs: power_db(band(sp, rate, 0., 25.)),
        from_25_to_100_dbfs: power_db(band(sp, rate, 25., 100.)),
        below_100_fraction: (total > 1e-20).then_some(low / total.max(1e-30)),
        definition_low_db: (low > 1e-20 && definition > 1e-20)
            .then_some(power_db(definition) - power_db(low)),
    }
}
#[derive(Clone, Serialize)]
pub struct Stage {
    pub rms_dbfs: f64,
    pub peak_dbfs: f64,
    pub bands_dbfs: [f64; 9],
    pub low_energy: LowEnergy,
    pub fundamental_dbfs: Option<f64>,
}
#[derive(Clone, Serialize)]
pub struct Window {
    pub seconds: f64,
    pub held_out: bool,
    pub within_section: bool,
    pub note: Option<Note>,
    pub stages: Vec<Stage>,
    pub mean_reduction_db: f64,
    pub max_reduction_db: f64,
    pub pcm_contact_fraction: f64,
    #[serde(skip)]
    pub bass_spectrum: Vec<f64>,
}
#[derive(Serialize)]
pub struct Envelope {
    pub seconds: f64,
    pub raw_dbfs: f64,
    pub kick_raw_dbfs: f64,
    pub post_dbfs: f64,
    pub mean_reduction_db: f64,
    pub max_reduction_db: f64,
}
#[derive(Serialize)]
pub struct Measurement {
    pub stages: [&'static str; 11],
    pub bands: [(f64, f64); 9],
    pub windows: Vec<Window>,
    pub envelopes: Vec<Envelope>,
}
/// Stream continuously from the common origin, including filter and compressor
/// history before the requested pilot; no cropped-source startup artifact.
pub fn measure(
    s: &Session,
    root: &Path,
    p: &Policy,
    start_seconds: f64,
    end_seconds: f64,
) -> Result<Measurement> {
    p.validate(s)?;
    if !start_seconds.is_finite()
        || !end_seconds.is_finite()
        || start_seconds < 0.
        || end_seconds <= start_seconds
        || end_seconds > 600.
    {
        return Err("measurement span must be within 0..600 seconds".into());
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
    for src in &mut sources {
        src.offset = src.reference.unwrap_or(0) - origin;
    }
    let total = sources
        .iter()
        .map(|x| x.offset.checked_add(x.frames).ok_or("timeline overflow"))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    let end = total.min((end_seconds * s.sample_rate as f64) as u64);
    let mut strips = s
        .channels
        .iter()
        .map(|c| Strip::new(c, s.sample_rate))
        .collect::<Vec<_>>();
    let mut eq_channel = s.channels[p.bass].clone();
    eq_channel.compressor.ratio = 1.;
    eq_channel.compressor.makeup_db = 0.;
    let mut eq = Strip::new(&eq_channel, s.sample_rate);
    let mut hp = vec![[Biquad::highpass(s.master_hpf_hz, FRAC_1_SQRT_2, s.sample_rate); 2]; 5];
    let mut windows = vec![];
    let mut envelopes = vec![];
    let mut blocks = vec![vec![[0.; 2]; SIZE]; 11];
    let (mut gr, mut max_gr, mut contacts) = (0., 0_f64, 0_u64);
    let hop = (s.sample_rate / 100) as u64;
    let mut env = [0_f64; 5];
    let mut count = 0;
    for t in 0..end {
        let j = t as usize % SIZE;
        let mut sample = [[0.; 2]; 11];
        for (i, (src, strip)) in sources.iter_mut().zip(&mut strips).enumerate() {
            let before = src.full_scale_samples();
            let raw = src.next(t)?;
            let post = strip.tick(raw);
            let routed = route(
                post,
                src.channels,
                s.channels[i].pan,
                gain(s.channels[i].fader_db),
            );
            if i == p.bass {
                sample[0] = raw;
                sample[1] = eq.tick(raw);
                sample[2] = post;
                sample[3] = routed;
                sample[4] = routed;
                let r = strip.reduction_db();
                gr += r;
                max_gr = max_gr.max(r);
                env[3] += r;
                env[4] = env[4].max(r);
                if !src.is_float() {
                    contacts += src.full_scale_samples() - before;
                }
            }
            if i == p.kick {
                sample[5] = raw;
                sample[6] = routed;
            }
            for ch in 0..2 {
                sample[9][ch] += routed[ch] * gain(s.master_db);
                if matches!(
                    s.channels[i].role,
                    Role::RhythmGuitar | Role::LeadGuitar | Role::AcousticGuitar
                ) {
                    sample[7][ch] += routed[ch];
                }
                if matches!(
                    s.channels[i].role,
                    Role::LeadVocal | Role::BackingVocal | Role::VocalRoom
                ) {
                    sample[8][ch] += routed[ch];
                }
            }
        }
        sample[10] = sample[9];
        for (slot, stage) in [4, 6, 7, 8, 10].iter().enumerate() {
            for ch in 0..2 {
                sample[*stage][ch] = hp[slot][ch].tick(sample[*stage][ch]);
            }
        }
        for k in 0..11 {
            blocks[k][j] = sample[k];
        }
        for (k, stage) in [0, 5, 2].iter().enumerate() {
            env[k] += (sample[*stage][0].powi(2) + sample[*stage][1].powi(2)) / 2.;
        }
        count += 1;
        if count == hop {
            let seconds = (t + 1 - hop) as f64 / s.sample_rate as f64;
            if seconds >= start_seconds {
                envelopes.push(Envelope {
                    seconds,
                    raw_dbfs: power_db(env[0] / hop as f64),
                    kick_raw_dbfs: power_db(env[1] / hop as f64),
                    post_dbfs: power_db(env[2] / hop as f64),
                    mean_reduction_db: env[3] / hop as f64,
                    max_reduction_db: env[4],
                });
            }
            env = [0.; 5];
            count = 0;
        }
        if j == SIZE - 1 {
            let seconds = (t + 1 - SIZE as u64) as f64 / s.sample_rate as f64;
            if seconds >= start_seconds {
                let spectra = blocks.iter().map(|b| spectrum(b)).collect::<Vec<_>>();
                let note = estimate_note(&blocks[0], s.sample_rate);
                let stages = blocks
                    .iter()
                    .zip(&spectra)
                    .map(|(b, sp)| Stage {
                        low_energy: low_energy(sp, s.sample_rate),
                        fundamental_dbfs: note.as_ref().map(|n| fundamental(sp, s.sample_rate, n)),
                        rms_dbfs: power_db(
                            b.iter().flat_map(|x| x.iter()).map(|v| v * v).sum::<f64>()
                                / (2 * SIZE) as f64,
                        ),
                        peak_dbfs: db(b
                            .iter()
                            .flat_map(|x| x.iter())
                            .map(|v| v.abs())
                            .fold(0_f64, f64::max)),
                        bands_dbfs: BANDS.map(|(lo, hi)| power_db(band(sp, s.sample_rate, lo, hi))),
                    })
                    .collect();
                windows.push(Window {
                    seconds,
                    held_out: (seconds / 12.).floor() as u64 % 2 == 1,
                    within_section: (seconds / 12.).floor()
                        == ((t as f64 / s.sample_rate as f64) / 12.).floor(),
                    note,
                    stages,
                    mean_reduction_db: gr / SIZE as f64,
                    max_reduction_db: max_gr,
                    pcm_contact_fraction: contacts as f64
                        / (SIZE * sources[p.bass].channels) as f64,
                    bass_spectrum: spectra[4].clone(),
                });
            }
            gr = 0.;
            max_gr = 0.;
            contacts = 0;
        }
    }
    Ok(Measurement {
        stages: STAGES,
        bands: BANDS,
        windows,
        envelopes,
    })
}
pub fn analyze(s: Session, root: &Path, out: &Path, p: Policy, start: f64, end: f64) -> Result<()> {
    p.validate(&s)?;
    if out.exists() {
        return Err("output already exists".into());
    }
    let m = measure(&s, root, &p, start, end)?;
    std::fs::create_dir(out)?;
    write_json(&out.join("settings.json"), &s)?;
    write_json(&out.join("policy.json"), &p)?;
    write_json(&out.join("measurement.json"), &m)?;
    write_json(
        &out.join("diagnosis.json"),
        &diagnose(&m.windows, s.sample_rate, &p),
    )
}

fn quantile(values: impl Iterator<Item = f64>, q: f64) -> Option<f64> {
    let mut v = values.filter(|x| x.is_finite()).collect::<Vec<_>>();
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        None
    } else {
        Some(v[((v.len() - 1) as f64 * q).round() as usize])
    }
}
fn definition(sp: &[f64], rate: u32) -> f64 {
    power_db(band(sp, rate, 400., 1600.)) - power_db(band(sp, rate, 100., 400.))
}
fn definition_low(sp: &[f64], rate: u32) -> f64 {
    power_db(band(sp, rate, 400., 1600.)) - power_db(band(sp, rate, 0., 100.))
}
fn tone_cost(sp: &[f64], rate: u32, p: &Policy) -> f64 {
    violation(definition(sp, rate), p.definition_body_db).powi(2)
        + p.definition_low_db
            .map_or(0., |r| violation(definition_low(sp, rate), r).powi(2))
}
fn tone_regresses(before: &[f64], after: &[f64], rate: u32, p: &Policy) -> bool {
    violation(definition(after, rate), p.definition_body_db)
        > violation(definition(before, rate), p.definition_body_db) + 1.
        || p.definition_low_db.is_some_and(|r| {
            violation(definition_low(after, rate), r)
                > violation(definition_low(before, rate), r) + 1.
        })
}
fn violation(x: f64, range: [f64; 2]) -> f64 {
    (range[0] - x).max(x - range[1]).max(0.)
}
fn fundamental(sp: &[f64], rate: u32, n: &Note) -> f64 {
    let width = (rate as f64 / SIZE as f64 * 1.5).max(n.hz * 0.04);
    power_db(band(sp, rate, n.hz - width, n.hz + width))
}
#[derive(Serialize)]
pub struct Diagnosis {
    pub activity_floor_dbfs: f64,
    pub eligible: Vec<bool>,
    pub training_windows: usize,
    pub held_out_windows: usize,
    pub training_notes: usize,
    pub held_out_notes: usize,
    pub training_definition_body_db: Option<f64>,
    pub training_definition_low_db: Option<f64>,
    pub training_below_100_fraction: Option<f64>,
    pub fraction_below_low_intent: Option<f64>,
    pub fraction_below_intent: Option<f64>,
    pub confidence: String,
    pub interpretation: Vec<String>,
}
pub fn diagnose(w: &[Window], rate: u32, p: &Policy) -> Diagnosis {
    let floor = quantile(
        w.iter()
            .filter(|x| x.within_section && !x.held_out)
            .map(|x| x.stages[0].rms_dbfs),
        0.95,
    )
    .unwrap_or(-120.)
        - 30.;
    let floor = floor.max(-65.);
    let mask = w
        .iter()
        .map(|x| x.within_section && x.stages[0].rms_dbfs > floor && x.note.is_some())
        .collect::<Vec<_>>();
    let sets = [false, true].map(|held| {
        w.iter()
            .zip(&mask)
            .filter(|(x, m)| **m && x.held_out == held)
            .map(|(x, _)| x)
            .collect::<Vec<_>>()
    });
    let counts = sets.each_ref().map(|v| {
        let mut notes = std::collections::BTreeMap::new();
        for x in v {
            *notes.entry(x.note.as_ref().unwrap().midi).or_insert(0) += 1;
        }
        notes.values().filter(|&&n| n >= 3).count()
    });
    let median = quantile(
        sets[0].iter().map(|x| definition(&x.bass_spectrum, rate)),
        0.5,
    );
    let fraction = (!sets[0].is_empty()).then(|| {
        sets[0]
            .iter()
            .filter(|x| definition(&x.bass_spectrum, rate) < p.definition_body_db[0])
            .count() as f64
            / sets[0].len() as f64
    });
    let low_fraction = p.definition_low_db.and_then(|r| {
        (!sets[0].is_empty()).then(|| {
            sets[0]
                .iter()
                .filter(|x| definition_low(&x.bass_spectrum, rate) < r[0])
                .count() as f64
                / sets[0].len() as f64
        })
    });
    let confidence = if sets[0].len() >= 12 && counts[0] >= 3 {
        "adequate training note coverage; heuristic, not probability"
    } else {
        "insufficient training note coverage; abstain"
    }
    .to_string();
    Diagnosis{activity_floor_dbfs:floor,eligible:mask.clone(),training_windows:sets[0].len(),held_out_windows:sets[1].len(),training_notes:counts[0],held_out_notes:counts[1],training_definition_body_db:median,
        training_definition_low_db:quantile(sets[0].iter().map(|x| definition_low(&x.bass_spectrum, rate)), 0.5),
        training_below_100_fraction:quantile(sets[0].iter().filter_map(|x| low_energy(&x.bass_spectrum, rate).below_100_fraction), 0.5),
        fraction_below_low_intent:low_fraction, fraction_below_intent:fraction,confidence,interpretation:vec![
        "Band imbalance is relative to explicit intent; fundamentals are musical content, never notch detections.".into(),
        "Below-100 Hz includes below-25 Hz energy and musical fundamentals. Strong low-band share alone never authorizes cuts; the independent definition/low target must be explicit.".into(),
        "Guitar/vocal band overlap is a masking risk, not a perceptual masking measurement or proof of audibility.".into(),
        "DI identity does not establish pickup, pedal, preamp or amp routing; spectrum does not establish a capture defect.".into(),
        "Pitch-changing, low-confidence and pause windows cannot vote for spectral correction; they retain level and dynamics guards.".into()]}
}
#[derive(Clone, Serialize)]
pub struct Candidate {
    pub eq: Vec<EqBand>,
    pub predicted_cost: Option<f64>,
    pub rejection: Vec<String>,
}
#[derive(Serialize)]
pub struct Proposal {
    pub diagnosis: Diagnosis,
    pub candidates: Vec<Candidate>,
    pub chosen: Option<usize>,
    pub source_advice: Vec<String>,
    pub repeat_measurement_requested: bool,
}
fn bell(hz: f64, db: f64, q: f64) -> EqBand {
    EqBand {
        kind: EqKind::Bell,
        hz,
        db,
        q,
    }
}
fn note_guard(
    w: &[Window],
    after: &[Vec<f64>],
    mask: &[bool],
    rate: u32,
    p: &Policy,
    held: bool,
) -> Vec<String> {
    let mut notes: std::collections::BTreeMap<i32, Vec<f64>> = Default::default();
    let mut failures = vec![];
    for ((x, sp), &yes) in w.iter().zip(after).zip(mask) {
        if !yes || x.held_out != held {
            continue;
        }
        let n = x.note.as_ref().unwrap();
        notes
            .entry(n.midi)
            .or_default()
            .push(fundamental(sp, rate, n) - fundamental(&x.bass_spectrum, rate, n));
    }
    let deltas = notes
        .values()
        .filter(|v| v.len() >= 3)
        .map(|v| quantile(v.iter().copied(), 0.5).unwrap())
        .collect::<Vec<_>>();
    if deltas.iter().any(|&d| !(-1. ..=3.).contains(&d)) {
        failures.push("a repeated note loses >1 dB fundamental support or gains >3 dB".into());
    }
    if let (Some(lo), Some(hi)) = (
        quantile(deltas.iter().copied(), 0.),
        quantile(deltas.iter().copied(), 1.),
    ) && hi - lo > p.max_note_spread_db
    {
        failures.push("correction disproportionately favors a note".into());
    }
    failures
}
pub fn propose(w: &[Window], rate: u32, p: &Policy, available_eq: usize) -> Proposal {
    let d = diagnose(w, rate, p);
    let mut q = Proposal {
        diagnosis: d,
        candidates: vec![],
        chosen: None,
        source_advice: vec![],
        repeat_measurement_requested: false,
    };
    if q.diagnosis.training_windows < 12
        || q.diagnosis.training_notes < 3
        || (q.diagnosis.fraction_below_intent.unwrap_or(0.) < 0.7
            && q.diagnosis.fraction_below_low_intent.unwrap_or(0.) < 0.7)
    {
        return q;
    }
    // Historical 54-member grid. No frequencies are fitted to spectral peaks/notes.
    let mut grid = vec![vec![]];
    for mid in [700., 1000.] {
        for boost in [2., 4.] {
            for body in [0., -2., -4.] {
                let mut e = vec![bell(mid, boost, 0.7)];
                if body != 0. {
                    e.push(bell(180., body, 0.7));
                }
                grid.push(e.clone());
                for low in [45., 55., 65.] {
                    let mut v = e.clone();
                    v.push(bell(low, 4., 3.));
                    grid.push(v);
                }
            }
        }
    }
    for low in [45., 55., 65.] {
        grid.push(vec![bell(low, 4., 3.)]);
    }
    // Broad low-shelf relief is assessed independently of narrow sub emphasis.
    for mid in [700., 1000.] {
        grid.push(vec![
            bell(mid, 4., 0.7),
            EqBand {
                kind: EqKind::LowShelf,
                hz: 35.5,
                q: FRAC_1_SQRT_2,
                db: 4.,
            },
        ]);
    }
    // Only observed low-intent deficiency enables low relief. Fixed broad shelves,
    // never notches fitted to a played fundamental; note support guards still apply.
    let low_deficient = q.diagnosis.fraction_below_low_intent.unwrap_or(0.) >= 0.7;
    if low_deficient {
        for hz in [35., 60., 90.] {
            for cut in [-1., -2., -4.] {
                let shelf = EqBand {
                    kind: EqKind::LowShelf,
                    hz,
                    q: FRAC_1_SQRT_2,
                    db: cut,
                };
                grid.push(vec![shelf.clone()]);
                for mid in [700., 1000.] {
                    for boost in [2., 4.] {
                        grid.push(vec![shelf.clone(), bell(mid, boost, 0.7)]);
                    }
                }
            }
        }
    }
    let baseline_cost = quantile(
        w.iter()
            .zip(&q.diagnosis.eligible)
            .filter(|(x, m)| **m && !x.held_out)
            .map(|(x, _)| tone_cost(&x.bass_spectrum, rate, p)),
        0.5,
    )
    .unwrap();
    let mut best = baseline_cost;
    for eq in grid {
        let mut c = Candidate {
            eq,
            predicted_cost: None,
            rejection: vec![],
        };
        if c.eq.len() > available_eq || c.eq.iter().any(|e| e.db.abs() > p.max_eq_db) {
            c.rejection
                .push("EQ capacity or explicit gain budget".into());
            q.candidates.push(c);
            continue;
        }
        let filters =
            c.eq.iter()
                .map(|e| Biquad::equalizer(e, rate))
                .collect::<Vec<_>>();
        let response = (0..=SIZE / 2)
            .map(|k| {
                filters
                    .iter()
                    .map(|f| f.power_response(k as f64 * rate as f64 / SIZE as f64, rate))
                    .product::<f64>()
            })
            .collect::<Vec<_>>();
        // Only training spectra enter prediction/search. Held-out stays unmodified.
        let predicted = w
            .iter()
            .map(|x| {
                x.bass_spectrum
                    .iter()
                    .zip(&response)
                    .map(|(a, b)| a * if x.held_out { 1. } else { *b })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        c.rejection.extend(note_guard(
            w,
            &predicted,
            &q.diagnosis.eligible,
            rate,
            p,
            false,
        ));
        let cost = quantile(
            w.iter()
                .zip(&predicted)
                .zip(&q.diagnosis.eligible)
                .filter(|((x, _), m)| **m && !x.held_out)
                .map(|((_, sp), _)| tone_cost(sp, rate, p)),
            0.5,
        )
        .unwrap()
            + c.eq.iter().map(|e| 0.025 * e.db * e.db).sum::<f64>();
        c.predicted_cost = Some(cost);
        for ((x, sp), &yes) in w.iter().zip(&predicted).zip(&q.diagnosis.eligible) {
            if yes
                && !x.held_out
                && (power_db(sp.iter().sum()) - power_db(x.bass_spectrum.iter().sum())
                    > p.max_output_rise_db
                    || tone_regresses(&x.bass_spectrum, sp, rate, p)
                    || (low_deficient
                        && power_db(band(sp, rate, 0., 100.))
                            > power_db(band(&x.bass_spectrum, rate, 0., 100.)) + 0.1))
            {
                c.rejection
                    .push("training output or tone regression".into());
                break;
            }
        }
        if c.rejection.is_empty() && cost < best - 0.01 {
            best = cost;
            q.chosen = Some(q.candidates.len());
        }
        q.candidates.push(c);
    }
    if let Some(i) = q.chosen
        && q.candidates[i].eq.iter().any(|e| e.db.abs() >= 4.)
    {
        q.source_advice=vec!["Substantial correction remains a source-review signal, not a source-defect diagnosis.".into(),"For a new soundcheck, verify where the DI is tapped. If pickup/tone or a pre-DI pedal affects that tap, compare one modest change with the same quiet, strong and sustained notes; include bass-only, kick-only and overlapping phrases.".into(),"Do not prescribe an amp control without establishing that it affects this DI. Repeat measurement before stacking corrections; this historical recording can only receive a bounded software trial.".into()];
        q.repeat_measurement_requested = true;
    }
    q
}
#[derive(Serialize)]
pub struct Outcome {
    pub accepted: bool,
    pub failures: Vec<String>,
    pub definition_body_before: [Option<f64>; 2],
    pub definition_body_after: [Option<f64>; 2],
    pub definition_low_before: [Option<f64>; 2],
    pub definition_low_after: [Option<f64>; 2],
    pub median_targets_met: bool,
    pub max_added_reduction_db: f64,
    pub max_mix_peak_rise_db: f64,
    pub pcm_contact_windows: usize,
    pub source_limitations: Vec<String>,
    pub listener_preference: Option<String>,
}
pub fn validate(
    before: &Measurement,
    after: &Measurement,
    q: &Proposal,
    rate: u32,
    p: &Policy,
) -> Outcome {
    let mut o = Outcome {
        accepted: false,
        failures: vec![],
        definition_body_before: [None; 2],
        definition_body_after: [None; 2],
        definition_low_before: [None; 2],
        definition_low_after: [None; 2],
        median_targets_met: true,
        max_added_reduction_db: 0.,
        max_mix_peak_rise_db: 0.,
        pcm_contact_windows: before
            .windows
            .iter()
            .filter(|w| w.pcm_contact_fraction >= super::expert::PCM_CONTACT_FRACTION)
            .count(),
        source_limitations: vec![],
        listener_preference: None,
    };
    if before.windows.len() != after.windows.len()
        || before.envelopes.len() != after.envelopes.len()
        || before
            .windows
            .iter()
            .zip(&after.windows)
            .any(|(a, b)| a.seconds != b.seconds)
    {
        o.median_targets_met = false;
        o.failures.push("timeline mismatch".into());
        return o;
    }
    if q.diagnosis.held_out_windows < 8 || q.diagnosis.held_out_notes < 2 {
        o.failures
            .push("insufficient held-out note coverage".into());
    }
    let spectra = after
        .windows
        .iter()
        .map(|x| x.bass_spectrum.clone())
        .collect::<Vec<_>>();
    for (split, held) in [false, true].iter().enumerate() {
        o.failures.extend(note_guard(
            &before.windows,
            &spectra,
            &q.diagnosis.eligible,
            rate,
            p,
            *held,
        ));
        let mask = &q.diagnosis.eligible;
        let a = quantile(
            before
                .windows
                .iter()
                .zip(mask)
                .filter(|(x, m)| **m && x.held_out == *held)
                .map(|(x, _)| definition(&x.bass_spectrum, rate)),
            0.5,
        );
        let b = quantile(
            after
                .windows
                .iter()
                .zip(mask)
                .filter(|(x, m)| **m && x.held_out == *held)
                .map(|(x, _)| definition(&x.bass_spectrum, rate)),
            0.5,
        );
        o.definition_body_before[split] = a;
        o.definition_body_after[split] = b;
        let low_median = |m: &Measurement| {
            quantile(
                m.windows
                    .iter()
                    .zip(mask)
                    .filter(|(x, yes)| **yes && x.held_out == *held)
                    .map(|(x, _)| definition_low(&x.bass_spectrum, rate)),
                0.5,
            )
        };
        let (la, lb) = (low_median(before), low_median(after));
        o.definition_low_before[split] = la;
        o.definition_low_after[split] = lb;
        o.median_targets_met &= b.is_some_and(|b| violation(b, p.definition_body_db) <= 0.1)
            && p.definition_low_db
                .is_none_or(|r| lb.is_some_and(|b| violation(b, r) <= 0.1));
        let low_improvement = p.definition_low_db.map_or(Some(0.), |r| {
            la.zip(lb).map(|(a, b)| violation(a, r) - violation(b, r))
        });
        if !a.zip(b).zip(low_improvement).is_some_and(|((a, b), low)| {
            let body = violation(a, p.definition_body_db) - violation(b, p.definition_body_db);
            body >= -0.1 && low >= -0.1 && body + low >= 0.25
        }) {
            o.failures
                .push(format!("split {split}: inadequate tone improvement"));
        }
    }
    if o.pcm_contact_windows > 0 {
        o.source_limitations.push("PCM full-scale contact remains in the source. Inspect the recording/input path and repeat capture if possible; EQ does not recover clipped samples. Isolated events retain every output, crest and compressor guard.".into());
    }
    if o.pcm_contact_windows >= super::expert::PCM_CONTACT_WINDOWS {
        o.failures
            .push("repeated PCM full-scale contact: review source".into());
    }
    for (i, (a, b)) in before.windows.iter().zip(&after.windows).enumerate() {
        if a.stages[0].rms_dbfs > -65. {
            let rise = b.stages[4].rms_dbfs - a.stages[4].rms_dbfs;
            let crest_loss = (a.stages[4].peak_dbfs - a.stages[4].rms_dbfs)
                - (b.stages[4].peak_dbfs - b.stages[4].rms_dbfs);
            if rise > p.max_output_rise_db + 0.05 || crest_loss > 1.5 {
                o.failures
                    .push(format!("window {i}: output rise or crest loss"));
            }
            if a.stages[0].rms_dbfs <= q.diagnosis.activity_floor_dbfs && rise > 1. {
                o.failures
                    .push(format!("window {i}: pause/noise amplification"));
            }
            if q.diagnosis.fraction_below_low_intent.unwrap_or(0.) >= 0.7
                && b.stages[1].low_energy.below_100_fraction.is_some()
                && low_power(&b.stages[1]) > low_power(&a.stages[1]) + 0.1
            {
                o.failures.push(format!(
                    "window {i}: added pre-compressor low energy despite low-intent deficiency"
                ));
            }
            if q.diagnosis.eligible[i]
                && tone_regresses(&a.bass_spectrum, &b.bass_spectrum, rate, p)
            {
                o.failures.push(format!("window {i}: tone regression"));
            }
        }
    }
    for (a, b) in before.envelopes.iter().zip(&after.envelopes) {
        o.max_added_reduction_db = o
            .max_added_reduction_db
            .max(b.max_reduction_db - a.max_reduction_db);
    }
    if o.max_added_reduction_db > p.max_added_reduction_db {
        o.failures
            .push("10 ms compressor transient action exceeds budget".into());
    }
    let peaks = [before, after].map(|m| {
        m.windows
            .iter()
            .map(|w| w.stages[10].peak_dbfs)
            .fold(-240_f64, f64::max)
    });
    o.max_mix_peak_rise_db = peaks[1] - peaks[0];
    if o.max_mix_peak_rise_db > 1. {
        o.failures
            .push("mix peak consumes >1 dB additional export headroom".into());
    }
    o.accepted = o.failures.is_empty();
    o.median_targets_met &= o.accepted;
    o
}
fn low_power(stage: &Stage) -> f64 {
    power_db(
        gain(stage.low_energy.below_25_dbfs).powi(2)
            + gain(stage.low_energy.from_25_to_100_dbfs).powi(2),
    )
}
/// Search once from training; held-out only accepts/rejects, never retunes.
pub fn run(s: Session, root: &Path, out: &Path, p: Policy, start: f64, end: f64) -> Result<()> {
    p.validate(&s)?;
    if out.exists() {
        return Err("output already exists".into());
    }
    let before = measure(&s, root, &p, start, end)?;
    let q = propose(
        &before.windows,
        s.sample_rate,
        &p,
        8 - s.channels[p.bass].eq.len(),
    );
    std::fs::create_dir(out)?;
    write_json(&out.join("policy.json"), &p)?;
    write_json(&out.join("before-settings.json"), &s)?;
    write_json(&out.join("baseline.json"), &before)?;
    write_json(&out.join("proposal.json"), &q)?;
    let mut applied = s.clone();
    if let Some(i) = q.chosen {
        let mut candidate = s.clone();
        candidate.channels[p.bass]
            .eq
            .extend(q.candidates[i].eq.clone());
        candidate.validate()?;
        write_json(&out.join("candidate-settings.json"), &candidate)?;
        if p.recorded_source || !q.repeat_measurement_requested {
            let after = measure(&candidate, root, &p, start, end)?;
            let o = validate(&before, &after, &q, s.sample_rate, &p);
            write_json(&out.join("candidate.json"), &after)?;
            write_json(&out.join("outcome.json"), &o)?;
            if o.accepted {
                applied = candidate;
            }
        }
    }
    // Fader-only proposal is logged separately and never silently combined with EQ.
    let mask = &q.diagnosis.eligible;
    let overlap = quantile(
        before
            .windows
            .iter()
            .zip(mask)
            .filter(|(x, m)| **m && !x.held_out)
            .map(|(x, _)| {
                let b = &x.stages[4].bands_dbfs;
                let g = &x.stages[7].bands_dbfs;
                power_db(gain(b[5]).powi(2) + gain(b[6]).powi(2))
                    - power_db(gain(g[5]).powi(2) + gain(g[6]).powi(2))
            }),
        0.5,
    );
    let fader = if q.diagnosis.training_windows >= 12 && overlap.is_some_and(|v| v < -12.) {
        p.max_fader_db.min(1.5)
    } else {
        0.
    };
    let mut fader_s = s.clone();
    fader_s.channels[p.bass].fader_db += fader;
    write_json(&out.join("fader-only-settings.json"), &fader_s)?;
    write_json(
        &out.join("fader-proposal.json"),
        &serde_json::json!({"delta_db":fader,"training_bass_minus_guitars_definition_db":overlap,"applied":false,"reason":"A fader changes every bass band equally; assess separately through actual DSP. It cannot repair missing source harmonics.","listener_preference":null}),
    )?;
    if fader > 0. {
        let f = measure(&fader_s, root, &p, start, end)?;
        let peak = |m: &Measurement| {
            m.windows
                .iter()
                .map(|x| x.stages[10].peak_dbfs)
                .fold(-240_f64, f64::max)
        };
        let peak_rise = peak(&f) - peak(&before);
        let gr_change = before
            .envelopes
            .iter()
            .zip(&f.envelopes)
            .map(|(a, b)| (b.max_reduction_db - a.max_reduction_db).abs())
            .fold(0_f64, f64::max);
        write_json(&out.join("fader-measurement.json"), &f)?;
        write_json(
            &out.join("fader-outcome.json"),
            &serde_json::json!({"applied":false,"delta_db":fader,"mix_peak_rise_db":peak_rise,"max_compressor_change_db":gr_change,"technical_budgets_pass":peak_rise<=1. && gr_change<1e-9,"all_bass_bands_rise_db":fader,"tone_ratio_change_db":0.,"reason":"Raises low-band kick competition as well as definition; separate listening option, not selected correction.","listener_preference":null}),
        )?;
    }
    write_json(&out.join("settings.json"), &applied)
}

#[cfg(test)]
mod spectral_tests {
    use super::*;
    #[test]
    fn dc_sub_and_nyquist_power_are_not_double_counted() {
        for hz in [0., 16., 50., 16384.] {
            let x = (0..SIZE)
                .map(|i| {
                    let value = 0.1 * (std::f64::consts::TAU * hz * i as f64 / 32768.).cos();
                    [value, value]
                })
                .collect::<Vec<_>>();
            let sp = spectrum(&x);
            let expected = if hz == 0. || hz == 16384. {
                0.01
            } else {
                0.005
            };
            assert!((sp.iter().sum::<f64>() - expected).abs() < 1e-10);
            let low = low_energy(&sp, 32768);
            if hz < 25. {
                assert!((gain(low.below_25_dbfs).powi(2) - expected).abs() < 1e-10);
            }
        }
    }
}
