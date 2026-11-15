//! Source-frame acoustic probes. No clock, device, allocation or control authority.
//! A/B share an onset; the same-carrier mode cross-checks frequency arrival bias.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub first_frame: u64,
    pub spacing_frames: u64,
    pub repeats: u32,
    pub same_carrier: bool,
    pub peak_dbfs: f64,
}
impl Probe {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.validate_limit(-42.)
    }
    pub(crate) fn validate_limit(&self, maximum: f64) -> Result<(), &'static str> {
        if !self.peak_dbfs.is_finite()
            || !(-60.0..=maximum).contains(&self.peak_dbfs)
            || !(1..=8).contains(&self.repeats)
            || !(48_000..=144_000).contains(&self.spacing_frames)
            || self.first_frame > 48_000 * 45
            || self
                .first_frame
                .checked_add(self.spacing_frames * u64::from(self.repeats))
                .is_none_or(|end| end > 48_000 * 60)
        {
            return Err("bounded quiet probe requires -60..-42 dBFS and <=60s timeline");
        }
        Ok(())
    }
    /// Fifty-ms Hann burst, exactly zero outside the finite excitation windows.
    /// Return dry reference A and pre-fader excitation B in logical input order.
    pub fn sample(&self, frame: u64) -> [f64; 2] {
        let Some(elapsed) = frame.checked_sub(self.first_frame) else {
            return [0.; 2];
        };
        if elapsed / self.spacing_frames >= u64::from(self.repeats) {
            return [0.; 2];
        }
        let n = elapsed % self.spacing_frames;
        if n >= 2400 {
            return [0.; 2];
        }
        let envelope = 0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / 2399.).cos();
        let amplitude = 10f64.powf(self.peak_dbfs / 20.) * envelope;
        let phase = std::f64::consts::TAU * n as f64 / 48.;
        // The production raw-input/recorder contract admits exact PCM24 values.
        // Quantize here, before both the dry and aux paths see the same source.
        [
            amplitude * phase.sin(),
            amplitude * (phase * if self.same_carrier { 1. } else { 2. }).sin(),
        ]
        .map(|sample| (sample * 8_388_608.).round() / 8_388_608.)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synchronous_finite_quiet_probes_and_same_probe_crosscheck() {
        let mut p = Probe {
            first_frame: 48000,
            spacing_frames: 96000,
            repeats: 3,
            same_carrier: false,
            peak_dbfs: -42.,
        };
        p.validate().unwrap();
        assert_eq!(p.sample(47999), [0.; 2]);
        assert_eq!(p.sample(50400), [0.; 2]);
        assert_eq!(p.sample(336000), [0.; 2]);
        for n in 0..2400 {
            let a = p.sample(48000 + n);
            assert_eq!(a, p.sample(144000 + n));
            assert!(
                a.iter()
                    .all(|s| s.abs() <= 10f64.powf(-42. / 20.) + 0.5 / 8_388_608.)
            );
            assert!(a.iter().all(|s| (s * 8_388_608.).fract() == 0.));
        }
        p.same_carrier = true;
        for n in 0..2400 {
            let s = p.sample(48000 + n);
            assert_eq!(s[0], s[1]);
        }
        p.spacing_frames = 0;
        assert!(p.validate().is_err());
        p.peak_dbfs = f64::NAN;
        assert!(p.validate().is_err());
    }
}
