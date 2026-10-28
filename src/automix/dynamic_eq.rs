//! Bounded linked frequency-selective attenuation in the production offline strip.
//! A fixed cut filter is crossfaded with dry signal by a smoothed band detector.
//! No coefficient modulation, lookahead, makeup, allocation, locks or I/O in tick.
use super::{
    config::{EqBand, EqKind},
    dsp::{Biquad, db},
};
use crate::inventory::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub hz: f64,
    pub q: f64,
    pub depth_db: f64,
    pub threshold_dbfs: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
}
impl Config {
    pub fn validate(&self, rate: u32) -> Result<()> {
        let in_range = |v: f64, a: f64, b: f64| v.is_finite() && (a..=b).contains(&v);
        if self.version != 1
            || !in_range(self.hz, 20., rate as f64 * 0.4)
            || !in_range(self.q, 0.3, 4.)
            || !in_range(self.depth_db, 0., 6.)
            || !in_range(self.threshold_dbfs, -80., 0.)
            || !in_range(self.attack_ms, 1., 100.)
            || !in_range(self.release_ms, 20., 1000.)
        {
            return Err("invalid dynamic EQ v1 settings".into());
        }
        Ok(())
    }
}
pub struct Processor {
    cut: [Biquad; 2],
    detector: [Biquad; 2],
    initial_cut: Biquad,
    initial_detector: Biquad,
    attack: f64,
    release: f64,
    threshold: f64,
    amount: f64,
    envelope: f64,
    bypass: bool,
}
impl Processor {
    pub fn new(c: &Config, rate: u32) -> Self {
        let cut = Biquad::equalizer(
            &EqBand {
                kind: EqKind::Bell,
                hz: c.hz,
                q: c.q,
                db: -c.depth_db,
            },
            rate,
        );
        let w = std::f64::consts::TAU * c.hz / rate as f64;
        let alpha = w.sin() / (2. * c.q);
        let detector = Biquad::new([alpha, 0., -alpha], [1. + alpha, -2. * w.cos(), 1. - alpha]);
        Self {
            cut: [cut; 2],
            detector: [detector; 2],
            initial_cut: cut,
            initial_detector: detector,
            attack: (-1. / (c.attack_ms * 0.001 * rate as f64)).exp(),
            release: (-1. / (c.release_ms * 0.001 * rate as f64)).exp(),
            threshold: c.threshold_dbfs,
            amount: 0.,
            envelope: 0.,
            bypass: c.depth_db == 0.,
        }
    }
    pub fn reset(&mut self) {
        self.cut = [self.initial_cut; 2];
        self.detector = [self.initial_detector; 2];
        self.amount = 0.;
        self.envelope = 0.;
    }
    pub fn amount(&self) -> f64 {
        self.amount
    }
    pub fn tick(&mut self, x: [f64; 2]) -> [f64; 2] {
        if x.iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
            self.reset();
            return [0.; 2];
        }
        if self.bypass {
            return x;
        }
        let detected = std::array::from_fn::<_, 2, _>(|i| self.detector[i].tick(x[i]));
        let peak = detected[0].abs().max(detected[1].abs());
        let c = if peak > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope = peak + c * (self.envelope - peak);
        let target = ((db(self.envelope) - self.threshold) / 6.).clamp(0., 1.);
        let c = if target > self.amount {
            self.attack
        } else {
            self.release
        };
        self.amount = (target + c * (self.amount - target)).clamp(0., 1.);
        std::array::from_fn(|i| {
            let wet = self.cut[i].tick(x[i]);
            x[i] + self.amount * (wet - x[i])
        })
    }
}
