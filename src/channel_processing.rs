//! GP07 fixed-storage mono FOH processing, prepared outside the renderer.
use crate::automix::{
    config::{Compressor, EqBand, EqKind},
    dsp::{Biquad, compression_db, db, gain},
};
use crate::show::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing_if = "crate::gate::Config::is_default")]
    pub gate: crate::gate::Config,
    pub eq_bypass: bool,
    pub band1_hz: i32,
    pub band1_gain_mdb: i32,
    pub band1_q_milli: i32,
    pub band1_bypass: bool,
    pub band2_hz: i32,
    pub band2_gain_mdb: i32,
    pub band2_q_milli: i32,
    pub band2_bypass: bool,
    pub band3_hz: i32,
    pub band3_gain_mdb: i32,
    pub band3_q_milli: i32,
    pub band3_bypass: bool,
    pub band4_hz: i32,
    pub band4_gain_mdb: i32,
    pub band4_q_milli: i32,
    pub band4_bypass: bool,
    pub compressor_bypass: bool,
    pub threshold_mdb: i32,
    pub ratio_milli: i32,
    pub knee_mdb: i32,
    pub attack_us: i32,
    pub release_ms: i32,
    pub makeup_mdb: i32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            gate: crate::gate::Config::default(),
            eq_bypass: true,
            band1_hz: 120,
            band1_gain_mdb: 0,
            band1_q_milli: 1000,
            band1_bypass: false,
            band2_hz: 500,
            band2_gain_mdb: 0,
            band2_q_milli: 1000,
            band2_bypass: false,
            band3_hz: 2000,
            band3_gain_mdb: 0,
            band3_q_milli: 1000,
            band3_bypass: false,
            band4_hz: 8000,
            band4_gain_mdb: 0,
            band4_q_milli: 1000,
            band4_bypass: false,
            compressor_bypass: true,
            threshold_mdb: -18000,
            ratio_milli: 1000,
            knee_mdb: 6000,
            attack_us: 10000,
            release_ms: 100,
            makeup_mdb: 0,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        self.gate.validate()?;
        for (v, lo, hi, step) in [
            (self.band1_hz, 20, 20000, 1),
            (self.band1_gain_mdb, -12000, 12000, 100),
            (self.band1_q_milli, 100, 10000, 100),
            (self.band2_hz, 20, 20000, 1),
            (self.band2_gain_mdb, -12000, 12000, 100),
            (self.band2_q_milli, 100, 10000, 100),
            (self.band3_hz, 20, 20000, 1),
            (self.band3_gain_mdb, -12000, 12000, 100),
            (self.band3_q_milli, 100, 10000, 100),
            (self.band4_hz, 20, 20000, 1),
            (self.band4_gain_mdb, -12000, 12000, 100),
            (self.band4_q_milli, 100, 10000, 100),
            (self.threshold_mdb, -60000, 0, 100),
            (self.ratio_milli, 1000, 20000, 100),
            (self.knee_mdb, 0, 18000, 100),
            (self.attack_us, 100, 200000, 100),
            (self.release_ms, 10, 2000, 1),
            (self.makeup_mdb, -12000, 12000, 100),
        ] {
            if !(lo..=hi).contains(&v) || v % step != 0 {
                return Err("range".into());
            }
        }
        Ok(())
    }
}
/// Four prepared filters and detector coefficients. No heap ownership.
#[derive(Debug, Clone, Copy)]
pub struct Prepared {
    pub(crate) config: Config,
    filters: [Biquad; 4],
    gate: crate::gate::Prepared,
    attack: f64,
    release: f64,
}
impl Prepared {
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let filters = [
            (
                config.band1_hz,
                config.band1_gain_mdb,
                config.band1_q_milli,
                config.band1_bypass,
            ),
            (
                config.band2_hz,
                config.band2_gain_mdb,
                config.band2_q_milli,
                config.band2_bypass,
            ),
            (
                config.band3_hz,
                config.band3_gain_mdb,
                config.band3_q_milli,
                config.band3_bypass,
            ),
            (
                config.band4_hz,
                config.band4_gain_mdb,
                config.band4_q_milli,
                config.band4_bypass,
            ),
        ]
        .map(|(hz, g, q, bypass)| {
            if bypass || g == 0 {
                Biquad::new([1., 0., 0.], [1., 0., 0.])
            } else {
                Biquad::equalizer(
                    &EqBand {
                        kind: EqKind::Bell,
                        hz: hz as f64,
                        q: q as f64 / 1000.,
                        db: g as f64 / 1000.,
                    },
                    48000,
                )
            }
        });
        Ok(Self {
            config,
            gate: crate::gate::Prepared::new(config.gate, 48000)?,
            filters,
            attack: (-1. / (config.attack_us as f64 * 0.000001 * 48000.)).exp(),
            release: (-1. / (config.release_ms as f64 * 0.001 * 48000.)).exp(),
        })
    }
}
#[derive(Debug, Clone, Copy)]
struct Branch {
    prepared: Prepared,
    reduction: f64,
}
impl Branch {
    fn new(prepared: Prepared) -> Self {
        Self {
            prepared,
            reduction: 0.,
        }
    }
    fn equalize(&mut self, mut x: f64) -> f64 {
        let p = &mut self.prepared;
        if !p.config.eq_bypass {
            for filter in &mut p.filters {
                x = filter.tick(x);
            }
        }
        x
    }
    fn compress(&mut self, mut x: f64) -> f64 {
        let p = &self.prepared;
        if !p.config.compressor_bypass {
            let c = p.config;
            let target = compression_db(
                db(x.abs()),
                &Compressor {
                    threshold_db: c.threshold_mdb as f64 / 1000.,
                    ratio: c.ratio_milli as f64 / 1000.,
                    knee_db: c.knee_mdb as f64 / 1000.,
                    attack_ms: 0.,
                    release_ms: 0.,
                    makeup_db: 0.,
                },
            );
            let coefficient = if target < self.reduction {
                p.attack
            } else {
                p.release
            };
            self.reduction = target + coefficient * (self.reduction - target);
            x *= gain(self.reduction + c.makeup_mdb as f64 / 1000.);
        }
        x
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct Strip {
    old: Branch,
    target: Branch,
    gate: crate::gate::Gate,
    begin: u64,
    end: u64,
}
impl Default for Strip {
    fn default() -> Self {
        let branch = Branch::new(Prepared::new(Config::default()).expect("valid default"));
        Self {
            old: branch,
            target: branch,
            gate: crate::gate::Gate::new(branch.prepared.gate),
            begin: 0,
            end: 0,
        }
    }
}
impl Strip {
    pub(crate) fn gate_observation(&self, frame: u64) -> Option<crate::gate::Observation> {
        (frame >= self.end && !self.target.prepared.config.gate.is_default())
            .then(|| self.gate.observation())
    }
    pub(crate) fn apply(&mut self, prepared: Prepared, frame: u64) {
        if self.target.prepared.config == prepared.config {
            return;
        }
        self.old = self.target;
        self.target = Branch::new(prepared);
        self.gate.apply(prepared.gate);
        self.target.reduction = self.old.reduction;
        self.begin = frame;
        self.end = frame + 240;
    }
    pub(crate) fn tick(&mut self, x: f64, frame: u64) -> f64 {
        self.tick_inner(x, frame, true)
    }
    fn tick_inner(&mut self, x: f64, frame: u64, gate_enabled: bool) -> f64 {
        let target = self.target.equalize(x);
        if frame >= self.end {
            if !target.is_finite() {
                return f64::NAN;
            }
            let gated = if gate_enabled {
                self.gate.tick(target)
            } else {
                target
            };
            return self.target.compress(gated);
        }
        let old = self.old.equalize(x);
        let t = (frame - self.begin) as f64 / 240.;
        if !old.is_finite() || !target.is_finite() {
            return f64::NAN;
        }
        // One detector/history for the actual blended EQ signal, shared gain before
        // both compressors. No duplicate gate work or history reset on unrelated edits.
        let detector = old + (target - old) * t;
        let gain = if gate_enabled {
            self.gate.tick(detector);
            self.gate.linear_gain()
        } else {
            1.
        };
        let old = self.old.compress(old * gain);
        let target = self.target.compress(target * gain);
        if !old.is_finite() || !target.is_finite() {
            return f64::NAN;
        }
        if t == 0. {
            old
        } else {
            old + (target - old) * t
        }
    }
    pub(crate) fn observation(
        &self,
        frame: u64,
        fault: bool,
    ) -> (Config, Config, u32, Option<i32>) {
        let remaining = self.end.saturating_sub(frame) as u32;
        let config = self.target.prepared.config;
        let current = if remaining == 0 {
            config
        } else {
            self.old.prepared.config
        };
        let reduction = if remaining == 0 && !fault && !config.compressor_bypass {
            Some((-self.target.reduction * 1000.).round().clamp(0., 240000.) as i32)
        } else {
            None
        };
        (current, config, remaining, reduction)
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    #[test]
    fn unrelated_edits_preserve_open_gate_and_raw_input() {
        let mut strip = Strip::default();
        let config = Config {
            gate: crate::gate::Config {
                bypass: false,
                hold_ms: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        strip.apply(Prepared::new(config).unwrap(), 0);
        for frame in 0..1000 {
            strip.tick(0.1, frame);
        }
        let before = strip.gate.linear_gain();
        strip.apply(
            Prepared::new(Config {
                band2_hz: 600,
                ..config
            })
            .unwrap(),
            1000,
        );
        assert_eq!(strip.gate.linear_gain(), before);
        for frame in 1000..1240 {
            assert!((strip.tick(0.1, frame) - 0.1).abs() < 1e-12);
        }
    }
    #[test]
    #[ignore = "opt-in CPU benchmark; cargo test --release --lib gate_benchmark -- --ignored --nocapture"]
    fn gate_benchmark() {
        use std::{hint::black_box, time::Instant};
        println!(
            "gate_bytes={} strip_bytes={}",
            std::mem::size_of::<crate::gate::Gate>(),
            std::mem::size_of::<Strip>()
        );
        for count in [16, 24, 32, 48] {
            for frames in [48, 128, 256] {
                for mode in [
                    "baseline",
                    "bypass",
                    "basic",
                    "tom",
                    "baseline_crossfade",
                    "basic_crossfade",
                    "tom_crossfade",
                ] {
                    let config = Config {
                        gate: crate::gate::Config {
                            bypass: mode == "baseline"
                                || mode == "bypass"
                                || mode == "baseline_crossfade",
                            hold_ms: 1,
                            detector_hz: if mode.starts_with("tom") { 180 } else { 0 },
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    let p = Prepared::new(config).unwrap();
                    let q = Prepared::new(Config {
                        band2_hz: 600,
                        ..config
                    })
                    .unwrap();
                    let mut strips = vec![Strip::default(); count];
                    for s in &mut strips {
                        s.apply(p, 0);
                    }
                    let mut times = Vec::with_capacity(500);
                    let mut frame = 0;
                    for block in 0..520 {
                        let start = Instant::now();
                        if mode.ends_with("crossfade") {
                            for s in &mut strips {
                                s.apply(if block % 2 == 0 { p } else { q }, frame);
                            }
                        }
                        for n in 0..frames {
                            let x = if (frame + n) % 12000 < 48 {
                                0.2
                            } else {
                                0.0001
                            };
                            for s in &mut strips {
                                black_box(s.tick_inner(
                                    black_box(x),
                                    frame + n,
                                    !mode.starts_with("baseline"),
                                ));
                            }
                        }
                        let us = start.elapsed().as_secs_f64() * 1e6;
                        if block >= 20 {
                            times.push(us);
                        }
                        frame += frames;
                    }
                    times.sort_by(f64::total_cmp);
                    println!(
                        "channels={count} frames={frames} mode={mode} median_us={:.3} p99_us={:.3} max_us={:.3} deadline_us={:.3}",
                        times[250],
                        times[495],
                        times[499],
                        frames as f64 / 48000. * 1e6
                    );
                }
            }
        }
    }
}
