//! Disjoint one-sided FFT power; no overlapping-filter energy accounting.
use crate::{automix::analysis::fft, inventory::Result};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spectrum {
    pub frames: usize,
    pub fft_size: usize,
    pub bin_hz: f64,
    pub window: String,
    pub window_square_sum: f64,
    /// Unwindowed mean, distinct from the Hann-windowed DC-bin power.
    pub dc_mean: f64,
    pub dc_bin_power: f64,
    pub nyquist_power: f64,
    pub total_power: f64,
    pub bands: Vec<Band>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Band {
    pub hz: [f64; 2],
    /// False for a partially or wholly unsupported band; power is then absent.
    pub supported: bool,
    pub power: Option<f64>,
}
/// Full or partial windows use a symmetric Hann over the actual samples and
/// zero-pad to a radix-2 transform. One/two-frame tails use a rectangular window.
/// Parseval total equals sum((x*w)^2)/sum(w^2), not unwindowed RMS.
pub fn measure(x: &[f64], rate: u32, edges: &[f64]) -> Result<Spectrum> {
    if x.is_empty()
        || x.len() > 65536
        || !(8000..=192000).contains(&rate)
        || x.iter().any(|x| !x.is_finite() || x.abs() > 1e6)
        || edges.len() < 2
        || edges.len() > 128
        || edges[0] != 0.
        || edges.iter().any(|v| !v.is_finite() || *v < 0.)
        || edges.windows(2).any(|v| v[1] <= v[0])
    {
        return Err("invalid spectral window/rate/disjoint band edges".into());
    }
    let n = x.len().next_power_of_two().max(2);
    let mut data = vec![[0.; 2]; n];
    let mut norm = 0.;
    for (i, &v) in x.iter().enumerate() {
        let w = if x.len() < 3 {
            1.
        } else {
            0.5 - 0.5 * (TAU * i as f64 / (x.len() - 1) as f64).cos()
        };
        norm += w * w;
        data[i][0] = v * w;
    }
    fft(&mut data);
    let powers = data[..=n / 2]
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let factor = if i == 0 || i == n / 2 { 1. } else { 2. };
            factor * (v[0] * v[0] + v[1] * v[1]) / (n as f64 * norm)
        })
        .collect::<Vec<_>>();
    let bin_hz = rate as f64 / n as f64;
    let nyquist = rate as f64 / 2.;
    let bands = edges
        .windows(2)
        .map(|e| {
            let supported = e[1] <= nyquist;
            let power = supported.then(|| {
                powers
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        let hz = *i as f64 * bin_hz;
                        hz >= e[0] && (hz < e[1] || hz == nyquist && e[1] == nyquist)
                    })
                    .map(|(_, p)| p)
                    .sum()
            });
            Band {
                hz: [e[0], e[1]],
                supported,
                power,
            }
        })
        .collect();
    Ok(Spectrum {
        frames: x.len(),
        fft_size: n,
        bin_hz,
        window: if x.len() < 3 {
            "rectangular-tail"
        } else {
            "symmetric-hann"
        }
        .into(),
        window_square_sum: norm,
        dc_mean: x.iter().sum::<f64>() / x.len() as f64,
        dc_bin_power: powers[0],
        nyquist_power: powers[n / 2],
        total_power: powers.iter().sum(),
        bands,
    })
}
