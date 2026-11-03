//! Control-side measured-summary exchange. No runtime learning or audio I/O.
use crate::gate::{self, Evidence, Proposal};
use serde::{Deserialize, Serialize};
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurements {
    pub version: u8,
    pub input: String,
    pub calibration_id: String,
    pub evaluation_id: String,
    pub hits_db: Vec<f64>,
    pub bleed_db: Vec<f64>,
    pub decay_ms: Vec<f64>,
    pub evaluation_hits_db: Vec<f64>,
    pub evaluation_bleed_db: Vec<f64>,
}
#[derive(Debug, Serialize)]
pub struct PreparedDocument {
    pub version: u8,
    pub input: String,
    pub calibration_id: String,
    pub evaluation_id: String,
    pub proposal: Proposal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causal_evaluation: Option<CausalEvaluation>,
}
impl Measurements {
    pub fn prepare(self) -> Result<PreparedDocument, String> {
        if self.version != 1
            || !self.input.starts_with("input-")
            || self.input.len() > 64
            || self.calibration_id.is_empty()
            || self.evaluation_id.is_empty()
            || self.calibration_id == self.evaluation_id
            || self.calibration_id.len() > 128
            || self.evaluation_id.len() > 128
        {
            return Err("gate preparation identity/split".into());
        }
        let proposal = gate::prepare_soundcheck(Evidence {
            hits_db: &self.hits_db,
            bleed_db: &self.bleed_db,
            decay_ms: &self.decay_ms,
            evaluation_hits_db: &self.evaluation_hits_db,
            evaluation_bleed_db: &self.evaluation_bleed_db,
        });
        Ok(PreparedDocument {
            version: 1,
            input: self.input,
            calibration_id: self.calibration_id,
            evaluation_id: self.evaluation_id,
            proposal,
            causal_evaluation: None,
        })
    }
}
pub fn prepare_file(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> crate::inventory::Result<()> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(source)?
        .take(512 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 512 * 1024 {
        return Err("gate measurement file capacity".into());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let document = if value["version"] == 2 {
        serde_json::from_value::<CaptureFiles>(value)?.prepare()?
    } else {
        serde_json::from_value::<Measurements>(value)?.prepare()?
    };
    crate::automix::write_json(destination, &document)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identified_frozen_proposal_and_refusal() {
        let document = |same: bool| Measurements {
            version: 1,
            input: "input-03".into(),
            calibration_id: "soundcheck/train".into(),
            evaluation_id: if same {
                "soundcheck/train"
            } else {
                "soundcheck/eval"
            }
            .into(),
            hits_db: vec![-20.; 8],
            bleed_db: vec![-40.; 8],
            decay_ms: vec![400.; 8],
            evaluation_hits_db: vec![-22.; 8],
            evaluation_bleed_db: vec![-40.; 8],
        };
        assert!(document(true).prepare().is_err());
        let p = document(false).prepare().unwrap();
        assert_eq!(p.proposal.reason, gate::Reason::Prepared);
        assert_eq!(p.proposal.config.hold_ms, 400);
        let mut bad = document(false);
        bad.evaluation_hits_db[0] = -30.;
        let p = bad.prepare().unwrap();
        assert!(p.proposal.config.bypass);
        assert_eq!(p.proposal.reason, gate::Reason::EvaluationOverlap);
    }
}

/// Bounded control-side capture of a cued soundcheck segment. The cue establishes
/// source identity; waveform amplitude alone cannot establish which drum played.
/// No raw samples are retained. Each candidate uses the same detector coefficients
/// as the runtime gate, with fixed 10 ms peak windows (maximum 40.96 seconds).
pub struct Capture {
    rate: u32,
    frames: usize,
    detectors: [Detector; 9],
    peaks: [f64; 9],
    windows: Vec<[f64; 9]>,
}
const BANDS: [u32; 9] = [0, 60, 90, 135, 200, 300, 450, 675, 1000];
#[derive(Clone, Copy)]
struct Detector {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl Capture {
    pub fn new(rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&rate) || !rate.is_multiple_of(100) {
            return Err("capture rate must have integral 10 ms windows".into());
        }
        Ok(Self {
            rate,
            frames: 0,
            peaks: [0.; 9],
            windows: Vec::with_capacity(4096),
            detectors: std::array::from_fn(|i| {
                let p = gate::Prepared::new(
                    gate::Config {
                        detector_hz: BANDS[i],
                        ..Default::default()
                    },
                    rate,
                )
                .expect("validated capture rate/bands");
                let (b, a) = p.detector_coefficients();
                Detector { b, a, z: [0.; 2] }
            }),
        })
    }
    /// All-or-nothing admission of each block, independent of producer partitioning.
    pub fn push(&mut self, samples: &[f64]) -> Result<(), String> {
        let capacity = 4096 * (self.rate as usize / 100);
        if samples.len() > capacity.saturating_sub(self.frames)
            || samples.iter().any(|x| !x.is_finite() || x.abs() > 1.)
        {
            return Err("capture capacity/nonfinite/out-of-range PCM".into());
        }
        for &x in samples {
            for (i, d) in self.detectors.iter_mut().enumerate() {
                let y = if i == 0 {
                    x
                } else {
                    let y = d.b[0] * x + d.z[0];
                    d.z[0] = d.b[1] * x - d.a[0] * y + d.z[1];
                    d.z[1] = d.b[2] * x - d.a[1] * y;
                    y
                };
                self.peaks[i] = self.peaks[i].max(y.abs());
            }
            self.frames += 1;
            if self.frames.is_multiple_of(self.rate as usize / 100) {
                self.windows.push(self.peaks);
                self.peaks = [0.; 9];
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<Captured, String> {
        if self.frames == 0 || !self.frames.is_multiple_of(self.rate as usize / 100) {
            return Err("capture requires complete 10 ms windows".into());
        }
        Ok(Captured {
            windows: self.windows,
        })
    }
}
pub struct Captured {
    windows: Vec<[f64; 9]>,
}
fn db(x: f64) -> f64 {
    20. * x.max(1e-12).log10()
}
fn percentile(mut values: Vec<f64>, q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * q).round() as usize]
}
impl Captured {
    /// An independently cued isolated hit, including its full decay. Automatic
    /// peak and -20 dB decay measurement; reject truncated/inaudible evidence.
    fn hit(&self, band: usize) -> Option<(f64, f64)> {
        let (peak_index, peak) = self
            .windows
            .iter()
            .enumerate()
            .map(|(i, w)| (i, w[band]))
            .max_by(|a, b| a.1.total_cmp(&b.1))?;
        if peak < 1e-5 {
            return None;
        }
        let last = self.windows.iter().rposition(|w| w[band] >= peak * 0.1)?;
        if last + 1 == self.windows.len() {
            return None;
        }
        Some((
            db(peak),
            ((last.saturating_sub(peak_index) + 1) * 10) as f64,
        ))
    }
    fn bleed(&self, band: usize) -> f64 {
        // Maximum, not p95: a single independently observed bleed transient matters.
        db(self.windows.iter().map(|w| w[band]).fold(0., f64::max))
    }
}
/// Cued raw capture is required: the operator records isolated target hits and
/// non-target bleed separately. Classification of arbitrary mixtures is ambiguous
/// and is deliberately not inferred. Band selection uses calibration only.
type SelectedBand = (f64, usize, Vec<(f64, f64)>, Vec<f64>);

pub fn prepare_captures(
    hits: &[Captured],
    bleed: &[Captured],
    evaluation_hits: &[Captured],
    evaluation_bleed: &[Captured],
) -> Proposal {
    let reject = || {
        gate::prepare_soundcheck(Evidence {
            hits_db: &[],
            bleed_db: &[],
            decay_ms: &[],
            evaluation_hits_db: &[],
            evaluation_bleed_db: &[],
        })
    };
    if [
        hits.len(),
        bleed.len(),
        evaluation_hits.len(),
        evaluation_bleed.len(),
    ]
    .iter()
    .any(|n| !(8..=4096).contains(n))
    {
        return reject();
    }
    let mut selected: Option<SelectedBand> = None;
    for band in 0..BANDS.len() {
        let Some(h): Option<Vec<_>> = hits.iter().map(|c| c.hit(band)).collect() else {
            continue;
        };
        let b: Vec<_> = bleed.iter().map(|c| c.bleed(band)).collect();
        let separation =
            percentile(h.iter().map(|h| h.0).collect(), 0.1) - percentile(b.clone(), 0.95);
        if selected
            .as_ref()
            .is_none_or(|(best, ..)| separation > *best)
        {
            selected = Some((separation, band, h, b));
        }
    }
    let Some((_, band, h, b)) = selected else {
        return reject();
    };
    let Some(e): Option<Vec<_>> = evaluation_hits.iter().map(|c| c.hit(band)).collect() else {
        return reject();
    };
    let mut proposal = gate::prepare_soundcheck(Evidence {
        hits_db: &h.iter().map(|h| h.0).collect::<Vec<_>>(),
        bleed_db: &b,
        decay_ms: &h.iter().map(|h| h.1).collect::<Vec<_>>(),
        evaluation_hits_db: &e.iter().map(|h| h.0).collect::<Vec<_>>(),
        evaluation_bleed_db: &evaluation_bleed
            .iter()
            .map(|c| c.bleed(band))
            .collect::<Vec<_>>(),
    });
    if !proposal.config.bypass {
        proposal.config.detector_hz = BANDS[band];
    }
    proposal
}

/// Explicit local mono WAV cues; no device, network or media-writing capability.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFiles {
    pub version: u8,
    pub input: String,
    pub calibration_id: String,
    pub evaluation_id: String,
    pub hits: Vec<std::path::PathBuf>,
    pub bleed: Vec<std::path::PathBuf>,
    pub evaluation_hits: Vec<std::path::PathBuf>,
    pub evaluation_bleed: Vec<std::path::PathBuf>,
}
impl CaptureFiles {
    pub fn prepare(self) -> Result<PreparedDocument, String> {
        if self.version != 2 {
            return Err("capture version".into());
        }
        // Reuse the strict output identity check, not the summary decision.
        let mut document = Measurements {
            version: 1,
            input: self.input,
            calibration_id: self.calibration_id,
            evaluation_id: self.evaluation_id,
            hits_db: vec![],
            bleed_db: vec![],
            decay_ms: vec![],
            evaluation_hits_db: vec![],
            evaluation_bleed_db: vec![],
        }
        .prepare()?;
        let mut identities = std::collections::BTreeSet::new();
        let mut fingerprints = std::collections::BTreeMap::new();
        let mut groups = Vec::new();
        for paths in [
            &self.hits,
            &self.bleed,
            &self.evaluation_hits,
            &self.evaluation_bleed,
        ] {
            if !(8..=32).contains(&paths.len()) {
                return Err("8..32 independent WAV cues per group required".into());
            }
            let mut captures = Vec::new();
            for path in paths {
                if !path.is_absolute() {
                    return Err("absolute capture paths required".into());
                }
                let path = path.canonicalize().map_err(|e| e.to_string())?;
                let mut reader = hound::WavReader::open(&path).map_err(|e| e.to_string())?;
                let spec = reader.spec();
                if spec.channels != 1
                    || spec.sample_rate != 48000
                    || spec.sample_format != hound::SampleFormat::Int
                    || ![16, 24, 32].contains(&spec.bits_per_sample)
                    || reader.len() > 4096 * 480
                {
                    return Err("capture requires bounded mono 48 kHz PCM16/24/32".into());
                }
                let mut capture = Capture::new(48000)?;
                let mut hash = sha2::Sha256::default();
                use sha2::Digest;
                let mut block = [0.; 480];
                let mut used = 0;
                for sample in reader.samples::<i32>() {
                    let sample = sample.map_err(|e| e.to_string())?;
                    let x = sample as f64 / 2f64.powi(i32::from(spec.bits_per_sample) - 1);
                    hash.update(x.to_le_bytes());
                    block[used] = x;
                    used += 1;
                    if used == block.len() {
                        capture.push(&block)?;
                        used = 0;
                    }
                }
                if used != 0 {
                    return Err("capture must end on a 10 ms window".into());
                }
                let digest = hash.finalize().to_vec();
                fingerprints.insert(path, digest.clone());
                if !identities.insert(digest) {
                    return Err(
                        "duplicate PCM evidence: calibration/evaluation cues must be independent"
                            .into(),
                    );
                }
                captures.push(capture.finish()?);
            }
            groups.push(captures);
        }
        document.proposal = prepare_captures(&groups[0], &groups[1], &groups[2], &groups[3]);
        if !document.proposal.config.bypass {
            let evidence = evaluate_files(
                document.proposal.config,
                &self.evaluation_hits,
                &self.evaluation_bleed,
                &fingerprints,
            )?;
            if evidence.missed_hits != 0 || evidence.false_opening_passages != 0 {
                document.proposal.config = gate::Config::default();
                document.proposal.confidence = 0.;
                document.proposal.reason = gate::Reason::CausalEvaluationFailed;
            }
            document.causal_evaluation = Some(evidence);
        }
        Ok(document)
    }
}

#[derive(Debug, Serialize)]
pub struct CausalEvaluation {
    pub hits: usize,
    pub missed_hits: usize,
    pub bleed_passages: usize,
    pub false_opening_passages: usize,
    pub worst_hit_energy_change_db: f64,
}
fn evaluate_files(
    config: gate::Config,
    hits: &[std::path::PathBuf],
    bleed: &[std::path::PathBuf],
    fingerprints: &std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
) -> Result<CausalEvaluation, String> {
    let mut result = CausalEvaluation {
        hits: hits.len(),
        missed_hits: 0,
        bleed_passages: bleed.len(),
        false_opening_passages: 0,
        worst_hit_energy_change_db: 0.,
    };
    for (is_hit, paths) in [(true, hits), (false, bleed)] {
        for path in paths {
            let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
            let spec = reader.spec();
            // Revalidate the second read: evidence may have changed since capture.
            if spec.channels != 1
                || spec.sample_rate != 48000
                || spec.sample_format != hound::SampleFormat::Int
                || ![16, 24, 32].contains(&spec.bits_per_sample)
                || reader.len() > 4096 * 480
            {
                return Err("evaluation PCM changed".into());
            }
            let mut core = gate::Gate::new(gate::Prepared::new(config, 48000)?);
            for _ in 0..(config.hold_ms + config.release_ms + 1) * 48 {
                core.tick(0.);
            }
            let floor = 10f64.powf(-(config.range_mdb as f64) / 20000.);
            let (mut original, mut processed, mut opened) = (0., 0., false);
            use sha2::Digest;
            let mut hash = sha2::Sha256::default();
            for sample in reader.samples::<i32>() {
                let x = sample.map_err(|e| e.to_string())? as f64
                    / 2f64.powi(i32::from(spec.bits_per_sample) - 1);
                hash.update(x.to_le_bytes());
                let y = core.tick(x);
                original += x * x;
                processed += y * y;
                opened |= core.linear_gain() > floor + 1e-9;
            }
            if fingerprints.get(&path.canonicalize().map_err(|e| e.to_string())?)
                != Some(&hash.finalize().to_vec())
            {
                return Err("evaluation PCM changed after calibration".into());
            }
            if is_hit {
                let change = 10. * (processed.max(1e-30) / original.max(1e-30)).log10();
                result.worst_hit_energy_change_db = result.worst_hit_energy_change_db.min(change);
                result.missed_hits += usize::from(original < 1e-12 || change < -1.);
            } else {
                result.false_opening_passages += usize::from(opened);
            }
        }
    }
    Ok(result)
}
