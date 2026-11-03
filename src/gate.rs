//! Prepared, zero-lookahead mono gate. No allocation or coefficient calculation in tick.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bypass: bool,
    pub threshold_mdb: i32,
    pub attack_us: u32,
    pub hold_ms: u32,
    pub release_ms: u32,
    pub range_mdb: u32,
    pub hysteresis_mdb: u32,
    /// Zero disables detector filtering; otherwise a detector-only bandpass.
    pub detector_hz: u32,
    pub detector_q_milli: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            bypass: true,
            threshold_mdb: -36000,
            attack_us: 100,
            hold_ms: 100,
            release_ms: 300,
            range_mdb: 24000,
            hysteresis_mdb: 6000,
            detector_hz: 0,
            detector_q_milli: 700,
        }
    }
}
impl Config {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn validate(self) -> Result<(), String> {
        if !(-90000..=0).contains(&self.threshold_mdb)
            || !(100..=200000).contains(&self.attack_us)
            || self.hold_ms > 2000
            || !(1..=5000).contains(&self.release_ms)
            || self.range_mdb > 90000
            || self.hysteresis_mdb > 24000
            || (self.detector_hz != 0 && !(20..=16000).contains(&self.detector_hz))
            || !(100..=10000).contains(&self.detector_q_milli)
        {
            return Err("gate range".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Prepared {
    pub config: Config,
    open: f64,
    close: f64,
    floor: f64,
    attack: u32,
    hold: u32,
    release: u32,
    b: [f64; 3],
    a: [f64; 2],
}
impl Prepared {
    pub(crate) fn detector_coefficients(&self) -> ([f64; 3], [f64; 2]) {
        (self.b, self.a)
    }
    pub fn new(config: Config, rate: u32) -> Result<Self, String> {
        config.validate()?;
        if !(8000..=192000).contains(&rate) || config.detector_hz as f64 >= rate as f64 * 0.45 {
            return Err("gate sample rate/detector".into());
        }
        let w = std::f64::consts::TAU * config.detector_hz as f64 / rate as f64;
        let alpha = w.sin() / (2. * config.detector_q_milli as f64 / 1000.);
        let norm = 1. / (1. + alpha);
        Ok(Self {
            config,
            open: 10f64.powf(config.threshold_mdb as f64 / 20000.),
            close: 10f64
                .powf((config.threshold_mdb as f64 - config.hysteresis_mdb as f64) / 20000.),
            floor: 10f64.powf(-(config.range_mdb as f64) / 20000.),
            attack: ((config.attack_us as u64 * rate as u64).div_ceil(1_000_000)) as u32,
            hold: ((config.hold_ms as u64 * rate as u64).div_ceil(1000)) as u32,
            release: ((config.release_ms as u64 * rate as u64).div_ceil(1000)) as u32,
            b: [alpha * norm, 0., -alpha * norm],
            a: [-2. * w.cos() * norm, (1. - alpha) * norm],
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Bypassed,
    Opening,
    Open,
    Holding,
    Closing,
    Closed,
}
#[derive(Debug, Clone, Copy)]
pub struct Gate {
    prepared: Prepared,
    pub state: State,
    gain: f64,
    start: f64,
    destination: f64,
    elapsed: u32,
    duration: u32,
    below: u32,
    z: [f64; 2],
}
impl Gate {
    pub fn new(prepared: Prepared) -> Self {
        Self {
            prepared,
            state: State::Bypassed,
            gain: 1.,
            start: 1.,
            destination: 1.,
            elapsed: 0,
            duration: 0,
            below: 0,
            z: [0.; 2],
        }
    }
    /// Retain gain and detector history across edits; source epoch reset uses new().
    pub fn apply(&mut self, prepared: Prepared) {
        if self.prepared.config == prepared.config {
            return;
        }
        if self.prepared.config.detector_hz != prepared.config.detector_hz
            || self.prepared.config.detector_q_milli != prepared.config.detector_q_milli
        {
            self.z = [0.; 2];
        }
        self.prepared = prepared;
        self.below = 0;
        self.ramp(1., prepared.attack, State::Opening);
    }
    fn ramp(&mut self, destination: f64, duration: u32, state: State) {
        self.start = self.gain;
        self.destination = destination;
        self.elapsed = 0;
        self.duration = duration.max(1);
        self.state = state;
    }
    pub fn linear_gain(&self) -> f64 {
        self.gain
    }
    /// Causal offline/block adapter; identical state and sample path to the live strip.
    pub fn process_block(&mut self, samples: &mut [f64]) {
        for sample in samples {
            *sample = self.tick(*sample);
        }
    }
    pub fn tick(&mut self, input: f64) -> f64 {
        if !input.is_finite() {
            self.z = [0.; 2];
            return 0.;
        }
        let p = self.prepared;
        if p.config.bypass && self.gain == 1. {
            self.state = State::Bypassed;
            return input;
        }
        let detector = if p.config.detector_hz == 0 {
            input.abs()
        } else {
            let y = p.b[0] * input + self.z[0];
            self.z[0] = p.b[1] * input - p.a[0] * y + self.z[1];
            self.z[1] = p.b[2] * input - p.a[1] * y;
            for z in &mut self.z {
                if z.abs() < 1e-30 {
                    *z = 0.;
                }
            }
            y.abs()
        };
        if p.config.bypass || detector >= p.open {
            self.below = 0;
            if !matches!(self.state, State::Opening | State::Open) {
                self.ramp(1., p.attack, State::Opening);
            }
        } else if !matches!(self.state, State::Closing | State::Closed) {
            if detector >= p.close {
                self.below = 0;
            } else {
                self.below = self.below.saturating_add(1);
            }
            if self.below >= p.hold.max(1) && self.state != State::Opening {
                self.ramp(p.floor, p.release, State::Closing);
            } else if self.state != State::Opening {
                self.state = State::Holding;
            }
        }
        if matches!(self.state, State::Opening | State::Closing) {
            self.elapsed = self.elapsed.saturating_add(1).min(self.duration);
            let t = self.elapsed as f64 / self.duration as f64;
            let smooth = t * t * (3. - 2. * t);
            self.gain = self.start + (self.destination - self.start) * smooth;
            if self.elapsed == self.duration {
                self.gain = self.destination;
                self.state = if self.state == State::Opening {
                    State::Open
                } else {
                    State::Closed
                };
            }
        }
        input * self.gain
    }
}

/// Independently labelled soundcheck hits/bleed, measured upstream of the gate.
/// Calibration and held-out evaluation must be separate. Never learns during performance.
#[derive(Debug)]
pub struct Evidence<'a> {
    pub hits_db: &'a [f64],
    pub bleed_db: &'a [f64],
    pub decay_ms: &'a [f64],
    pub evaluation_hits_db: &'a [f64],
    pub evaluation_bleed_db: &'a [f64],
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Prepared,
    InsufficientEvidence,
    InvalidEvidence,
    Overlap,
    EvaluationOverlap,
    CausalEvaluationFailed,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Measurements {
    pub quiet_hit_db: f64,
    pub bleed_p95_db: f64,
    pub decay_p90_ms: f64,
    pub evaluation_hits: usize,
    pub evaluation_missed_hits: usize,
    pub evaluation_bleed_windows: usize,
    pub evaluation_false_openings: usize,
}
#[derive(Debug, Serialize)]
pub struct Proposal {
    pub config: Config,
    pub confidence: f64,
    pub measurements: Option<Measurements>,
    pub reason: Reason,
}
pub fn prepare_soundcheck(e: Evidence<'_>) -> Proposal {
    let reject = |reason| Proposal {
        config: Config::default(),
        confidence: 0.,
        measurements: None,
        reason,
    };
    let arrays = [
        e.hits_db,
        e.bleed_db,
        e.decay_ms,
        e.evaluation_hits_db,
        e.evaluation_bleed_db,
    ];
    if arrays.iter().any(|a| a.len() < 8 || a.len() > 4096) {
        return reject(Reason::InsufficientEvidence);
    }
    if arrays.iter().any(|a| a.iter().any(|v| !v.is_finite()))
        || e.decay_ms.iter().any(|v| *v <= 0. || *v > 5000.)
    {
        return reject(Reason::InvalidEvidence);
    }
    fn quantile(a: &[f64], q: f64) -> f64 {
        let mut a = a.to_vec();
        a.sort_by(f64::total_cmp);
        a[((a.len() - 1) as f64 * q).round() as usize]
    }
    let quiet = quantile(e.hits_db, 0.1);
    let bleed = quantile(e.bleed_db, 0.95);
    let separation = quiet - bleed;
    let threshold = quiet - 6.;
    let measurements = Measurements {
        quiet_hit_db: quiet,
        bleed_p95_db: bleed,
        decay_p90_ms: quantile(e.decay_ms, 0.9),
        evaluation_hits: e.evaluation_hits_db.len(),
        evaluation_missed_hits: e
            .evaluation_hits_db
            .iter()
            .filter(|v| **v < threshold)
            .count(),
        evaluation_bleed_windows: e.evaluation_bleed_db.len(),
        evaluation_false_openings: e
            .evaluation_bleed_db
            .iter()
            .filter(|v| **v >= threshold)
            .count(),
    };
    let measured_reject = |reason| Proposal {
        measurements: Some(measurements),
        ..reject(reason)
    };
    if separation < 12. {
        return measured_reject(Reason::Overlap);
    }
    if e.evaluation_hits_db.iter().any(|v| *v < threshold + 3.)
        || e.evaluation_bleed_db.iter().any(|v| *v > threshold - 3.)
    {
        return measured_reject(Reason::EvaluationOverlap);
    }
    let config = Config {
        bypass: false,
        threshold_mdb: (threshold * 1000.).round() as i32,
        hold_ms: quantile(e.decay_ms, 0.9).ceil().clamp(20., 2000.) as u32,
        release_ms: (quantile(e.decay_ms, 0.9) * 0.5).ceil().clamp(50., 2500.) as u32,
        ..Config::default()
    };
    if config.validate().is_err() {
        return reject(Reason::InvalidEvidence);
    }
    Proposal {
        config,
        confidence: ((separation - 6.) / 18.).clamp(0., 1.),
        reason: Reason::Prepared,
        measurements: Some(measurements),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_retrigger_and_history() {
        let c = Config {
            bypass: false,
            hold_ms: 1,
            release_ms: 10,
            ..Config::default()
        };
        let p = Prepared::new(c, 48000).unwrap();
        let mut g = Gate::new(p);
        for _ in 0..48 {
            g.tick(0.);
        }
        assert_eq!(g.state, State::Closing);
        for _ in 0..200 {
            g.tick(0.);
        }
        let gain = g.linear_gain();
        g.apply(p);
        assert_eq!(g.linear_gain(), gain);
        g.tick(0.1);
        assert!(g.linear_gain() > gain);
        assert!(g.linear_gain() < 1.);
        for _ in 0..5 {
            g.tick(0.1);
        }
        assert_eq!(g.linear_gain(), 1.);
        for _ in 0..528 {
            g.tick(0.);
        }
        assert_eq!(g.state, State::Closed);
        assert!((g.linear_gain() - p.floor).abs() < 1e-12);
    }
    #[test]
    fn quiet_rolls_and_bypass() {
        let c = Config {
            bypass: false,
            ..Config::default()
        };
        let mut g = Gate::new(Prepared::new(c, 48000).unwrap());
        for i in 0..48000 {
            g.tick(if i % 2400 < 20 { 0.02 } else { 0.001 });
        }
        assert_eq!(g.linear_gain(), 1.);
        g.apply(Prepared::new(Config::default(), 48000).unwrap());
        for _ in 0..10 {
            assert_eq!(g.tick(0.2), 0.2);
        }
        assert_eq!(g.tick(f64::NAN), 0.);
    }
    #[test]
    fn evidence_requires_separation_and_holdout() {
        let hits = [-20.; 8];
        let bleed = [-40.; 8];
        let decay = [300.; 8];
        let proposal = prepare_soundcheck(Evidence {
            hits_db: &hits,
            bleed_db: &bleed,
            decay_ms: &decay,
            evaluation_hits_db: &hits,
            evaluation_bleed_db: &bleed,
        });
        assert_eq!(proposal.reason, Reason::Prepared);
        assert_eq!(proposal.config.hold_ms, 300);
        let proposal = prepare_soundcheck(Evidence {
            hits_db: &hits,
            bleed_db: &hits,
            decay_ms: &decay,
            evaluation_hits_db: &hits,
            evaluation_bleed_db: &bleed,
        });
        assert_eq!(proposal.reason, Reason::Overlap);
        assert!(proposal.config.bypass);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub state: State,
    pub attenuation_mdb: u32,
}
impl Gate {
    /// Control/readback path only: logarithm never runs in tick().
    pub fn observation(&self) -> Observation {
        Observation {
            state: self.state,
            attenuation_mdb: (-20000. * self.gain.max(1e-12).log10())
                .round()
                .clamp(0., 90000.) as u32,
        }
    }
}
