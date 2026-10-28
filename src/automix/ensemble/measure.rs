//! Feature extraction through production channel DSP with continuous history.
use super::*;
use crate::automix::{
    dsp::{Loudness, Strip, gain},
    render::route,
};

pub const FFT_FRAMES: usize = crate::automix::bass::SIZE;
pub const STAGES: [&str; 5] = [
    "raw",
    "post_filter_pre_compressor",
    "compressed_before_makeup",
    "compressed_after_makeup",
    "routed_before_excitation_fx_master",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stage {
    pub fft_size: usize,
    pub bin_hz: f64,
    pub window_square_sum: f64,
    pub dc_bin_power: f64,
    pub nyquist_power: f64,
    pub fundamental_power: Option<f64>,
    pub mean_power: f64,
    pub peak: f64,
    pub mono_power: f64,
    pub dc_mean: [f64; 2],
    pub spectral_power: f64,
    pub band_power: Vec<Option<f64>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Competition {
    pub rest_band_power: Vec<Option<f64>>,
    pub overlap_fraction: Option<f64>,
    pub signed_cross_power: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Window {
    pub competition: Competition,
    pub start: u64,
    pub frames: usize,
    pub channel: usize,
    pub note_hz: Option<f64>,
    pub stages: Vec<Stage>,
    pub max_reduction_db: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventWindow {
    pub start: u64,
    pub frames: u64,
    pub channel: usize,
    pub raw_power: f64,
    pub processed_power: f64,
    pub raw_peak: f64,
    pub processed_peak: f64,
    pub max_reduction_db: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub integrated_lufs: Option<f64>,
    pub momentary_max_lufs: Option<f64>,
    pub short_term_max_lufs: Option<f64>,
    pub integer_full_scale_samples: u64,
    pub float_headroom_samples: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measurement {
    pub profile_groups: Vec<(String, usize)>,
    pub sample_rate: u32,
    pub frames: u64,
    pub stage_names: Vec<String>,
    pub band_edges_hz: Vec<f64>,
    pub channels: Vec<ChannelSummary>,
    pub windows: Vec<Window>,
    /// Contiguous native-frame ~10 ms energy windows; these are not LUFS.
    pub events: Vec<EventWindow>,
    pub limitations: Vec<String>,
}
fn stage(samples: &[[f64; 2]], rate: u32, edges: &[f64], note: Option<f64>) -> Result<Stage> {
    let spectra = (0..2)
        .map(|side| {
            spectrum::measure(
                &samples.iter().map(|x| x[side]).collect::<Vec<_>>(),
                rate,
                edges,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let n = samples.len() as f64;
    let fundamental_power = note
        .map(|hz| {
            (0..2)
                .map(|side| {
                    spectrum::measure(
                        &samples.iter().map(|x| x[side]).collect::<Vec<_>>(),
                        rate,
                        &[0., hz * 0.8, hz * 1.2, rate as f64 / 2.],
                    )
                    .map(|s| s.bands[1].power.unwrap_or(0.))
                })
                .collect::<Result<Vec<_>>>()
                .map(|p| (p[0] + p[1]) * 0.5)
        })
        .transpose()?;
    Ok(Stage {
        fundamental_power,
        fft_size: spectra[0].fft_size,
        bin_hz: spectra[0].bin_hz,
        window_square_sum: spectra[0].window_square_sum,
        dc_bin_power: (spectra[0].dc_bin_power + spectra[1].dc_bin_power) * 0.5,
        nyquist_power: (spectra[0].nyquist_power + spectra[1].nyquist_power) * 0.5,
        mean_power: samples
            .iter()
            .map(|x| (x[0] * x[0] + x[1] * x[1]) * 0.5)
            .sum::<f64>()
            / n,
        peak: samples
            .iter()
            .flatten()
            .copied()
            .map(f64::abs)
            .fold(0., f64::max),
        mono_power: samples
            .iter()
            .map(|x| ((x[0] + x[1]) * 0.5).powi(2))
            .sum::<f64>()
            / n,
        dc_mean: [spectra[0].dc_mean, spectra[1].dc_mean],
        spectral_power: (spectra[0].total_power + spectra[1].total_power) * 0.5,
        band_power: spectra[0]
            .bands
            .iter()
            .zip(&spectra[1].bands)
            .map(|(a, b)| a.power.zip(b.power).map(|(a, b)| (a + b) * 0.5))
            .collect(),
    })
}
pub fn measure(
    s: &Session,
    root: &Path,
    request: &Request,
    frozen: &Frozen,
) -> Result<Measurement> {
    measure_to(s, root, request, frozen, None)
}
pub(super) fn measure_to(
    s: &Session,
    root: &Path,
    request: &Request,
    frozen: &Frozen,
    until: Option<u64>,
) -> Result<Measurement> {
    let end = until
        .unwrap_or(frozen.timeline_frames)
        .min(frozen.timeline_frames);
    frozen.verify(s, root, request)?;
    let mut edges = vec![
        0.,
        25.,
        100.,
        120.,
        350.,
        400.,
        1000.,
        1600.,
        3200.,
        5000.,
        8000.,
        12000.,
        s.sample_rate as f64 / 2.,
    ];
    for p in &request.profiles {
        if let Some(register) = p.supported_register_hz {
            edges.extend(register);
        }
        if let Some(t) = &p.tone {
            edges.extend(t.numerator_hz);
            edges.extend(t.denominator_hz);
        }
    }
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    if edges.len() > 128 {
        return Err("too many distinct requested spectral boundaries".into());
    }
    let mut sources = s
        .channels
        .iter()
        .zip(&frozen.inputs)
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
    let mut loud = s
        .channels
        .iter()
        .map(|_| Loudness::new(s.sample_rate))
        .collect::<Vec<_>>();
    let count = s.channels.len();
    let mut buffers = (0..count)
        .map(|_| {
            (0..5)
                .map(|_| Vec::with_capacity(FFT_FRAMES))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut reduction = vec![0_f64; count];
    let mut events = (0..count)
        .map(|channel| EventWindow {
            start: 0,
            frames: 0,
            channel,
            raw_power: 0.,
            processed_power: 0.,
            raw_peak: 0.,
            processed_peak: 0.,
            max_reduction_db: 0.,
        })
        .collect::<Vec<_>>();
    let mut group_events = request
        .profiles
        .iter()
        .enumerate()
        .filter(|(_, p)| p.channels.len() > 1)
        .map(|(i, _)| EventWindow {
            start: 0,
            frames: 0,
            channel: count + i,
            raw_power: 0.,
            processed_power: 0.,
            raw_peak: 0.,
            processed_peak: 0.,
            max_reduction_db: 0.,
        })
        .collect::<Vec<_>>();
    let mut result=Measurement{profile_groups: request.profiles.iter().enumerate().filter(|(_,p)| p.channels.len()>1).map(|(i,p)|(p.name.clone(), count+i)).collect(),sample_rate:s.sample_rate,frames:frozen.timeline_frames,stage_names:STAGES.iter().map(|s|(*s).into()).collect(),band_edges_hz:edges.clone(),channels:Vec::new(),windows:Vec::new(),events:Vec::new(),limitations:vec!["Channel stages precede excitation, FX returns and master processing. Routed-source LUFS is not an additive contribution through nonlinear master DSP.".into(),"Spectra use symmetric Hann windows and zero-padded partial tails; weighted power is distinct from all-sample mean power. Feature windows do not diagnose musical faults.".into()]};
    let mut phase = 0_u64;
    let mut start = 0;
    for frame in 0..end {
        phase += 100;
        let event_end = phase >= u64::from(s.sample_rate) || frame + 1 == end;
        if phase >= u64::from(s.sample_rate) {
            phase -= u64::from(s.sample_rate);
        }
        for i in 0..count {
            let raw = sources[i].next(frame)?;
            let (eq, post) = strips[i].tick_with_eq_tap(raw);
            let pre = post.map(|v| v / gain(s.channels[i].compressor.makeup_db));
            let routed = route(
                post,
                sources[i].channels,
                s.channels[i].pan,
                gain(s.channels[i].fader_db),
            );
            loud[i].add(routed);
            for (buffer, value) in buffers[i].iter_mut().zip([raw, eq, pre, post, routed]) {
                buffer.push(value);
            }
            reduction[i] = reduction[i].max(strips[i].reduction_db());
            let e = &mut events[i];
            e.frames += 1;
            e.raw_power += (raw[0] * raw[0] + raw[1] * raw[1]) * 0.5;
            e.processed_power += (post[0] * post[0] + post[1] * post[1]) * 0.5;
            e.raw_peak = e.raw_peak.max(raw[0].abs().max(raw[1].abs()));
            e.processed_peak = e.processed_peak.max(post[0].abs().max(post[1].abs()));
            e.max_reduction_db = e.max_reduction_db.max(strips[i].reduction_db());
            if event_end {
                e.raw_power /= e.frames as f64;
                e.processed_power /= e.frames as f64;
                result.events.push(e.clone());
                *e = EventWindow {
                    start: frame + 1,
                    frames: 0,
                    channel: i,
                    raw_power: 0.,
                    processed_power: 0.,
                    raw_peak: 0.,
                    processed_peak: 0.,
                    max_reduction_db: 0.,
                };
            }
        }
        for (e, (_, p)) in group_events.iter_mut().zip(
            request
                .profiles
                .iter()
                .enumerate()
                .filter(|(_, p)| p.channels.len() > 1),
        ) {
            let raw = std::array::from_fn::<_, 2, _>(|side| {
                p.channels
                    .iter()
                    .map(|&c| buffers[c][0].last().unwrap()[side])
                    .sum::<f64>()
            });
            let post = std::array::from_fn::<_, 2, _>(|side| {
                p.channels
                    .iter()
                    .map(|&c| buffers[c][3].last().unwrap()[side])
                    .sum::<f64>()
            });
            e.frames += 1;
            e.raw_power += (raw[0] * raw[0] + raw[1] * raw[1]) * 0.5;
            e.processed_power += (post[0] * post[0] + post[1] * post[1]) * 0.5;
            e.raw_peak = e.raw_peak.max(raw[0].abs().max(raw[1].abs()));
            e.processed_peak = e.processed_peak.max(post[0].abs().max(post[1].abs()));
            if event_end {
                e.raw_power /= e.frames as f64;
                e.processed_power /= e.frames as f64;
                result.events.push(e.clone());
                *e = EventWindow {
                    start: frame + 1,
                    frames: 0,
                    channel: e.channel,
                    raw_power: 0.,
                    processed_power: 0.,
                    raw_peak: 0.,
                    processed_peak: 0.,
                    max_reduction_db: 0.,
                };
            }
        }
        if buffers[0][0].len() == FFT_FRAMES || frame + 1 == end {
            let sum = (0..buffers[0][0].len())
                .map(|frame| {
                    std::array::from_fn(|side| {
                        buffers
                            .iter()
                            .map(|channel| channel[4][frame][side])
                            .sum::<f64>()
                    })
                })
                .collect::<Vec<[f64; 2]>>();
            for i in 0..count {
                let rest = sum
                    .iter()
                    .zip(&buffers[i][4])
                    .map(|(sum, source)| [sum[0] - source[0], sum[1] - source[1]])
                    .collect::<Vec<_>>();
                let rest_stage = stage(&rest, s.sample_rate, &edges, None)?;
                let source_stage = stage(&buffers[i][4], s.sample_rate, &edges, None)?;
                let overlap = source_stage
                    .band_power
                    .iter()
                    .zip(&rest_stage.band_power)
                    .filter_map(|(s, r)| s.zip(*r).map(|(s, r)| s.min(r)))
                    .sum::<f64>();
                let competition = Competition {
                    rest_band_power: rest_stage.band_power,
                    overlap_fraction: (source_stage.spectral_power > 1e-20)
                        .then_some((overlap / source_stage.spectral_power).clamp(0., 1.)),
                    signed_cross_power: buffers[i][4]
                        .iter()
                        .zip(&rest)
                        .map(|(a, b)| a[0] * b[0] + a[1] * b[1])
                        .sum::<f64>()
                        / rest.len() as f64,
                };
                let note_hz = if matches!(
                    s.channels[i].role,
                    crate::automix::config::Role::BassDi | crate::automix::config::Role::BassAmp
                ) {
                    crate::automix::bass::estimate_note(&buffers[i][0], s.sample_rate).map(|n| n.hz)
                } else {
                    None
                };
                result.windows.push(Window {
                    competition,
                    note_hz,
                    start,
                    frames: buffers[i][0].len(),
                    channel: i,
                    stages: buffers[i]
                        .iter()
                        .map(|b| stage(b, s.sample_rate, &edges, note_hz))
                        .collect::<Result<Vec<_>>>()?,
                    max_reduction_db: reduction[i],
                });
            }
            for (index, p) in request
                .profiles
                .iter()
                .enumerate()
                .filter(|(_, p)| p.channels.len() > 1)
            {
                let grouped = (0..5)
                    .map(|stage_index| {
                        (0..buffers[0][0].len())
                            .map(|frame_index| {
                                std::array::from_fn(|side| {
                                    p.channels
                                        .iter()
                                        .map(|&c| buffers[c][stage_index][frame_index][side])
                                        .sum()
                                })
                            })
                            .collect::<Vec<[f64; 2]>>()
                    })
                    .collect::<Vec<_>>();
                let rest = sum
                    .iter()
                    .zip(&grouped[4])
                    .map(|(a, b)| [a[0] - b[0], a[1] - b[1]])
                    .collect::<Vec<_>>();
                let rest_stage = stage(&rest, s.sample_rate, &edges, None)?;
                let note = if p.family == profile::Family::Bass {
                    crate::automix::bass::estimate_note(&grouped[0], s.sample_rate).map(|n| n.hz)
                } else {
                    None
                };
                let stages = grouped
                    .iter()
                    .map(|x| stage(x, s.sample_rate, &edges, note))
                    .collect::<Result<Vec<_>>>()?;
                let overlap = stages[4]
                    .band_power
                    .iter()
                    .zip(&rest_stage.band_power)
                    .filter_map(|(a, b)| a.zip(*b).map(|(a, b)| a.min(b)))
                    .sum::<f64>();
                result.windows.push(Window {
                    start,
                    frames: grouped[0].len(),
                    channel: count + index,
                    note_hz: note,
                    competition: Competition {
                        rest_band_power: rest_stage.band_power,
                        overlap_fraction: (stages[4].spectral_power > 1e-20)
                            .then_some((overlap / stages[4].spectral_power).clamp(0., 1.)),
                        signed_cross_power: grouped[4]
                            .iter()
                            .zip(&rest)
                            .map(|(a, b)| a[0] * b[0] + a[1] * b[1])
                            .sum::<f64>()
                            / rest.len() as f64,
                    },
                    stages,
                    max_reduction_db: p.channels.iter().map(|&c| reduction[c]).fold(0., f64::max),
                });
            }
            for i in 0..count {
                for b in &mut buffers[i] {
                    b.clear();
                }
                reduction[i] = 0.;
            }
            start = frame + 1;
        }
    }
    result.channels = sources
        .iter()
        .zip(loud)
        .map(|(src, m)| ChannelSummary {
            integrated_lufs: m.integrated(),
            momentary_max_lufs: m.momentary_max(),
            short_term_max_lufs: m.short_term_max(),
            integer_full_scale_samples: if src.is_float() {
                0
            } else {
                src.full_scale_samples()
            },
            float_headroom_samples: if src.is_float() {
                src.full_scale_samples()
            } else {
                0
            },
        })
        .collect();
    frozen.verify(s, root, request)?;
    Ok(result)
}
