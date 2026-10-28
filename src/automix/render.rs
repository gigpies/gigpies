use super::{
    config::{OutputMode, Session},
    dsp::*,
};
use crate::inventory::Result;
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let f = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(f);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.flush()?;
    Ok(())
}
fn time_reference(path: &Path) -> Result<Option<u64>> {
    let mut f = File::open(path)?;
    let length = f.metadata()?.len();
    let mut header = [0; 12];
    f.read_exact(&mut header)?;
    let mut result = None;
    while f.stream_position()? + 8 <= length {
        let mut h = [0; 8];
        f.read_exact(&mut h)?;
        let n = u32::from_le_bytes(h[4..8].try_into()?) as u64;
        let start = f.stream_position()?;
        if start + n > length {
            return Err("truncated RIFF chunk".into());
        }
        if &h[..4] == b"bext" {
            if n < 346 {
                return Err("short Broadcast WAV metadata".into());
            }
            f.seek(SeekFrom::Start(start + 338))?;
            let mut t = [0; 8];
            f.read_exact(&mut t)?;
            result = Some(u64::from_le_bytes(t));
        }
        f.seek(SeekFrom::Start(start + n + (n % 2)))?;
    }
    Ok(result)
}
pub(super) struct Source {
    reader: hound::WavReader<BufReader<File>>,
    pub(super) channels: usize,
    scale: f64,
    float: bool,
    pub(super) frames: u64,
    position: u64,
    pub(super) offset: u64,
    pub(super) reference: Option<u64>,
    meter: Meter,
}
impl Source {
    pub(super) fn open(path: &Path, rate: u32) -> Result<Self> {
        let reader = hound::WavReader::open(path)?;
        let s = reader.spec();
        if s.sample_rate != rate
            || !(1..=2).contains(&s.channels)
            || reader.len() % u32::from(s.channels) != 0
            || !(s.sample_format == hound::SampleFormat::Float && s.bits_per_sample == 32
                || s.sample_format == hound::SampleFormat::Int
                    && [16, 24, 32].contains(&s.bits_per_sample))
        {
            return Err(format!("unsupported/mismatched WAV: {}", path.display()).into());
        }
        let frames = reader.duration() as u64;
        Ok(Self {
            reader,
            channels: s.channels as usize,
            scale: 2_f64.powi(i32::from(s.bits_per_sample) - 1),
            float: s.sample_format == hound::SampleFormat::Float,
            frames,
            position: 0,
            offset: 0,
            reference: time_reference(path)?,
            meter: Meter::default(),
        })
    }
    pub(super) fn is_float(&self) -> bool {
        self.float
    }
    pub(super) fn full_scale_samples(&self) -> u64 {
        self.meter.clipped_samples
    }
    pub(super) fn next(&mut self, t: u64) -> Result<[f64; 2]> {
        if t < self.offset || self.position >= self.frames {
            return Ok([0.; 2]);
        }
        let mut x = [0.; 2];
        for v in x.iter_mut().take(self.channels) {
            *v = if self.float {
                self.reader
                    .samples::<f32>()
                    .next()
                    .ok_or("truncated sample payload")?? as f64
            } else {
                self.reader
                    .samples::<i32>()
                    .next()
                    .ok_or("truncated sample payload")?? as f64
                    / self.scale
            };
            if !v.is_finite() || v.abs() > 64. {
                return Err("nonfinite or implausibly large input sample".into());
            }
            let clips = self.meter.clipped_samples;
            self.meter.add(*v);
            let threshold = if self.float {
                1.0
            } else {
                1.0 - 1.0 / self.scale
            };
            self.meter.clipped_samples = clips + u64::from(v.abs() >= threshold);
        }
        if self.channels == 1 {
            x[1] = x[0];
        }
        self.position += 1;
        Ok(x)
    }
}
pub(super) fn route(x: [f64; 2], channels: usize, pan: f64, fader: f64) -> [f64; 2] {
    if channels == 1 {
        let angle = (pan + 1.) * std::f64::consts::FRAC_PI_4;
        [x[0] * angle.cos() * fader, x[0] * angle.sin() * fader]
    } else {
        [
            x[0] * (1. - pan.max(0.)).sqrt() * fader,
            x[1] * (1. + pan.min(0.)).sqrt() * fader,
        ]
    }
}
fn spec(rate: u32, bits: u16) -> hound::WavSpec {
    hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: bits,
        sample_format: if bits == 32 {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    }
}
pub(super) fn export(input: &Path, output: &Path, scale: f64) -> Result<()> {
    let mut reader = hound::WavReader::open(input)?;
    let mut writer = hound::WavWriter::create(output, spec(reader.spec().sample_rate, 24))?;
    for s in reader.samples::<f32>() {
        let v = s? as f64 * scale;
        if !v.is_finite() || v.abs() >= 1. {
            return Err("export would clip".into());
        }
        writer.write_sample((v * 8388608.).round() as i32)?;
    }
    writer.finalize()?;
    Ok(())
}
/// `finish_seconds`: Some = rehearsal from neutral, then freeze; None = prepared show.
/// Output must be a new directory. Source files are only opened read-only.
pub fn run(session: Session, root: &Path, out: &Path, finish_seconds: Option<f64>) -> Result<()> {
    run_inner(session, root, out, finish_seconds, None, None)
}
/// Explicit policy-aware render; preserves the frozen session schema and identities.
pub fn run_policy(
    session: Session,
    root: &Path,
    out: &Path,
    policy: super::delivery::Policy,
) -> Result<()> {
    policy.validate()?;
    run_inner(session, root, out, None, Some(&policy), None)
}
/// Production DSP observation without writing listening audio. State starts at zero.
pub fn observe(session: Session, root: &Path, out: &Path, until_seconds: f64) -> Result<()> {
    if !until_seconds.is_finite() || until_seconds <= 0. {
        return Err("invalid observation end".into());
    }
    run_inner(session, root, out, None, None, Some(until_seconds))
}
/// Observe the exact processing selected by an explicit delivery policy, without PCM export.
pub fn observe_policy(
    session: Session,
    root: &Path,
    out: &Path,
    until_seconds: f64,
    policy: super::delivery::Policy,
) -> Result<()> {
    policy.validate()?;
    if !until_seconds.is_finite() || until_seconds <= 0. {
        return Err("invalid observation end".into());
    }
    run_inner(session, root, out, None, Some(&policy), Some(until_seconds))
}
fn run_inner(
    mut session: Session,
    root: &Path,
    out: &Path,
    finish_seconds: Option<f64>,
    policy: Option<&super::delivery::Policy>,
    observe_until: Option<f64>,
) -> Result<()> {
    session.validate()?;
    if let Some(t) = finish_seconds {
        if !t.is_finite() || t <= 0. {
            return Err("soundcheck duration must be positive and finite".into());
        }
    } else if !session.prepared {
        return Err("render requires prepared settings from soundcheck".into());
    }
    let mut sources: Vec<_> = session
        .channels
        .iter()
        .map(|ch| Source::open(&root.join(&ch.file), session.sample_rate))
        .collect::<Result<_>>()?;
    let references: Vec<_> = sources.iter().filter_map(|s| s.reference).collect();
    if !references.is_empty() && references.len() != sources.len() {
        return Err("mixed missing/present BWF time references: explicitly prepare a consistent timeline first".into());
    }
    let origin = references.iter().min().copied().unwrap_or(0);
    for s in &mut sources {
        s.offset = s.reference.unwrap_or(0) - origin;
    }
    let mut total = 0;
    for s in &sources {
        total = total.max(s.offset.checked_add(s.frames).ok_or("timeline overflow")?);
    }
    let source_frames = total;
    let tail_frames = session
        .effects
        .as_ref()
        .map(|fx| (fx.tail_seconds * session.sample_rate as f64).round() as u64)
        .unwrap_or(0);
    total += tail_frames;
    if source_frames == 0 || total > u64::from(session.sample_rate) * 86400 {
        return Err("empty or >24 hour timeline".into());
    }
    let total = observe_until.map_or(total, |v| {
        total.min((v * session.sample_rate as f64).round() as u64)
    });
    let freeze = finish_seconds
        .map(|s| (s * session.sample_rate as f64).round() as u64)
        .unwrap_or(0);
    if finish_seconds.is_some() && (freeze == 0 || freeze > source_frames) {
        return Err(
            "soundcheck finish must be at least one frame and within the source timeline".into(),
        );
    }
    let identities = if policy.is_some() || observe_until.is_some() {
        Some(super::identity::source_identities(&session, root)?)
    } else {
        None
    };
    // Validation and source opening precede output creation; never overwrite a prior experiment.
    std::fs::create_dir(out)?;
    write_json(&out.join("initial-settings.json"), &session)?;
    let mut history = BufWriter::new(File::create(out.join("gain-history.csv"))?);
    writeln!(
        history,
        "start_frame,end_frame,group,reference_rms_dbfs,group_peak_dbfs,active,active_seconds,trim_start_db,trim_end_db,next_trim_db,frozen"
    )?;
    let mut channel_history = BufWriter::new(File::create(out.join("channel-history.csv"))?);
    writeln!(
        channel_history,
        "end_frame,channel,input_rms_dbfs,input_peak_dbfs,post_strip_rms_dbfs,post_strip_peak_dbfs,max_compressor_reduction_db"
    )?;
    let mut master_history = BufWriter::new(File::create(out.join("master-history.csv"))?);
    writeln!(
        master_history,
        "end_frame,output_peak_dbfs,max_limiter_reduction_db,affected_frames,maximizer_max_reduction_db,maximizer_affected_frames"
    )?;
    let groups: Vec<usize> = session
        .channels
        .iter()
        .map(|ch| {
            session
                .groups
                .iter()
                .position(|g| g.name == ch.group)
                .unwrap()
        })
        .collect();
    let mut calibrators: Vec<_> = session
        .groups
        .iter()
        .map(|_| Calibrator::new(&session.calibration))
        .collect();
    let mut current: Vec<_> = session
        .groups
        .iter()
        .map(|g| {
            if finish_seconds.is_some() {
                session.calibration.initial_trim_db
            } else {
                g.trim_db
            }
        })
        .collect();
    let mut strips: Vec<_> = session
        .channels
        .iter()
        .map(|ch| Strip::new(ch, session.sample_rate))
        .collect();
    let effective_limiter = policy
        .map(|p| p.final_sample_limiter.clone())
        .unwrap_or_else(|| {
            if session.output_mode == OutputMode::Matched {
                super::delivery::FinalLimiter::Enabled {
                    threshold_dbfs: session.ceiling_db,
                    release_ms: session.limiter_release_ms,
                }
            } else {
                super::delivery::FinalLimiter::Disabled
            }
        });
    let (threshold, release) = match effective_limiter {
        super::delivery::FinalLimiter::Enabled {
            threshold_dbfs,
            release_ms,
        } => (threshold_dbfs, release_ms),
        super::delivery::FinalLimiter::Disabled => (session.ceiling_db, session.limiter_release_ms),
    };
    let mut limiter = Limiter::new(threshold, release, session.sample_rate);
    let mut observer = if policy.is_some() || observe_until.is_some() {
        Some(super::observation::Observer::new(&session, out)?)
    } else {
        None
    };
    let mut master_hp = (session.master_hpf_hz > 0.).then(|| {
        [Biquad::highpass(
            session.master_hpf_hz,
            std::f64::consts::FRAC_1_SQRT_2,
            session.sample_rate,
        ); 2]
    });
    let mut rack = session
        .effects
        .as_ref()
        .map(|fx| super::effects::Rack::new(fx, sources.len(), session.sample_rate));
    let mut dry = if observe_until.is_none() {
        Some(hound::WavWriter::create(
            out.join("bypass-bus.tmp.wav"),
            spec(session.sample_rate, 32),
        )?)
    } else {
        None
    };
    let mut wet = if observe_until.is_none() {
        Some(hound::WavWriter::create(
            out.join("processed-bus.tmp.wav"),
            spec(session.sample_rate, 32),
        )?)
    } else {
        None
    };
    let mut dry_meter = Meter::default();
    let mut wet_meter = Meter::default();
    let mut loud_dry = Loudness::new(session.sample_rate);
    let mut loud_wet = Loudness::new(session.sample_rate);
    let mut after = vec![Meter::default(); sources.len()];
    let faders: Vec<_> = session
        .channels
        .iter()
        .map(|ch| gain(ch.fader_db))
        .collect();
    let master = gain(session.master_db);
    let neutral = gain(session.calibration.initial_trim_db);
    let mut start = 0;
    while start < total {
        let mut end = (start + session.block_frames as u64).min(total);
        if start < freeze {
            end = end.min(freeze);
        }
        let adapting = finish_seconds.is_some() && start < freeze;
        let targets: Vec<_> = calibrators
            .iter()
            .enumerate()
            .map(|(i, c)| if adapting { c.trim_db } else { current[i] })
            .collect();
        let previous = current.clone();
        let mut input = vec![Meter::default(); sources.len()];
        let mut processed = vec![Meter::default(); sources.len()];
        let mut block_wet = Meter::default();
        let mut gains = vec![0.; current.len()];
        for t in start..end {
            let fraction = (t - start + 1) as f64 / (end - start) as f64;
            for (i, value) in gains.iter_mut().enumerate() {
                *value = gain(current[i] + (targets[i] - current[i]) * fraction);
            }
            let mut a = [0.; 2];
            let mut b = [0.; 2];
            for i in 0..sources.len() {
                let x = sources[i].next(t)?;
                for v in x.iter().take(sources[i].channels) {
                    input[i].add(*v);
                }
                let (eq, y) = strips[i].tick_with_eq_tap(x.map(|v| v * gains[groups[i]]));
                let y = if let Some(fx) = &mut rack {
                    fx.excite(i, y)
                } else {
                    y
                };
                for v in y.iter().take(sources[i].channels) {
                    processed[i].add(*v);
                    after[i].add(*v);
                }
                let ch = &session.channels[i];
                let raw = route(x, sources[i].channels, ch.pan, faders[i] * neutral);
                let cooked = route(y, sources[i].channels, ch.pan, faders[i]);
                if let Some(o) = &mut observer {
                    o.channel(
                        i,
                        [
                            x,
                            eq,
                            eq.map(|v| v * gain(-strips[i].reduction_db())),
                            cooked.map(|v| v * master),
                        ],
                        sources[i].channels,
                        strips[i].reduction_db(),
                    );
                }
                if let Some(fx) = &mut rack {
                    fx.send(i, cooked);
                }
                for j in 0..2 {
                    a[j] += raw[j] * master;
                    b[j] += cooked[j] * master;
                }
            }
            let direct = b;
            if let Some(fx) = &mut rack {
                let wet = fx.returns();
                for j in 0..2 {
                    b[j] += wet[j] * master;
                }
            }
            let combined = b;
            if let Some(filters) = &mut master_hp {
                for j in 0..2 {
                    b[j] = filters[j].tick(b[j]);
                }
            }
            let filtered = b;
            if let Some(fx) = &mut rack {
                b = fx.master(b);
            }
            if matches!(
                effective_limiter,
                super::delivery::FinalLimiter::Enabled { .. }
            ) {
                b = limiter.tick(b);
            }
            if a.iter().chain(b.iter()).any(|v| !v.is_finite()) {
                return Err("nonfinite DSP output".into());
            }
            if let Some(o) = &mut observer {
                o.neutral(a)?;
                o.frame(
                    t,
                    rack.as_ref()
                        .map(|fx| fx.return_outputs.as_slice())
                        .unwrap_or(&[]),
                    master,
                    [direct, combined, filtered, b],
                    rack.as_ref().map_or(0., |fx| fx.reduction_db()),
                    limiter.reduction_db(),
                )?;
            }
            for j in 0..2 {
                if let Some(w) = &mut dry {
                    w.write_sample(a[j] as f32)?;
                }
                if let Some(w) = &mut wet {
                    w.write_sample(b[j] as f32)?;
                }
                dry_meter.add(a[j]);
                wet_meter.add(b[j]);
                block_wet.add(b[j]);
            }
            loud_dry.add(a);
            loud_wet.add(b);
        }
        current = targets;
        for (i, g) in session.groups.iter().enumerate() {
            let peak = input
                .iter()
                .enumerate()
                .filter(|(j, _)| groups[*j] == i)
                .map(|(_, m)| m.peak)
                .fold(0., f64::max);
            if adapting {
                calibrators[i].observe(
                    input[g.reference].rms(),
                    peak,
                    (end - start) as f64 / session.sample_rate as f64,
                );
            }
            let c = &calibrators[i];
            writeln!(
                history,
                "{start},{end},{},{:.4},{:.4},{},{:.4},{:.4},{:.4},{:.4},{}",
                g.name,
                db(input[g.reference].rms()),
                db(peak),
                c.active,
                c.active_seconds,
                previous[i],
                current[i],
                if adapting { c.trim_db } else { current[i] },
                !adapting
            )?;
        }
        for i in 0..sources.len() {
            writeln!(
                channel_history,
                "{end},{i},{:.4},{:.4},{:.4},{:.4},{:.4}",
                db(input[i].rms()),
                db(input[i].peak),
                db(processed[i].rms()),
                db(processed[i].peak),
                strips[i].max_reduction
            )?;
        }
        writeln!(
            master_history,
            "{end},{:.4},{:.4},{},{:.4},{}",
            db(block_wet.peak),
            limiter.max_reduction,
            limiter.affected_frames,
            rack.as_ref().map(|fx| fx.max_reduction()).unwrap_or(0.),
            rack.as_ref().map(|fx| fx.affected_frames()).unwrap_or(0)
        )?;
        start = end;
    }
    history.flush()?;
    channel_history.flush()?;
    master_history.flush()?;
    if let Some(w) = dry {
        w.finalize()?;
    }
    if let Some(w) = wet {
        w.finalize()?;
    }
    if let Some(o) = observer {
        o.finish(out, &session)?;
    }
    // Final trim equals the last applied value, not the un-applied last block decision.
    for (g, trim) in session.groups.iter_mut().zip(&current) {
        g.trim_db = *trim;
    }
    session.prepared = true;
    write_json(&out.join("prepared.json"), &session)?;
    let ld = loud_dry.integrated();
    let lw = loud_wet.integrated();
    let export_db = (session.ceiling_db - db(dry_meter.peak.max(wet_meter.peak))).min(0.);
    // Finalize each unmatched output from its own peak. Neither reference bus
    // participates in the other output's gain decision. Silence stays silent.
    let independent_gain = |peak: f64| {
        if peak > 0. {
            session.ceiling_db - db(peak)
        } else {
            0.
        }
    };
    let (dry_export, wet_export) = if session.output_mode == OutputMode::Unmatched {
        (
            independent_gain(dry_meter.peak),
            independent_gain(wet_meter.peak),
        )
    } else {
        (export_db, export_db)
    };
    let target = match (ld, lw) {
        (Some(d), Some(w)) if session.output_mode == OutputMode::Matched => Some(
            session
                .effects
                .as_ref()
                .map(|fx| fx.listening_target_lufs)
                .unwrap_or(-23.)
                .min(d + session.ceiling_db - db(dry_meter.peak))
                .min(w + session.ceiling_db - db(wet_meter.peak)),
        ),
        _ => None,
    };
    let dry_match = target.zip(ld).map(|(t, l)| t - l).unwrap_or(export_db);
    let wet_match = target.zip(lw).map(|(t, l)| t - l).unwrap_or(export_db);
    if observe_until.is_none() && policy.is_none() {
        for (stem, matched_db, output_db) in [
            ("bypass", dry_match, dry_export),
            ("processed", wet_match, wet_export),
        ] {
            let input = out.join(format!("{stem}-bus.tmp.wav"));
            export(&input, &out.join(format!("{stem}.wav")), gain(output_db))?;
            if session.output_mode == OutputMode::Matched {
                export(
                    &input,
                    &out.join(format!("{stem}-matched.wav")),
                    gain(matched_db),
                )?;
                std::fs::remove_file(input)?;
            } else {
                std::fs::rename(input, out.join(format!("{stem}-unity-float.wav")))?;
            }
        }
    } else if policy.is_some() && observe_until.is_none() {
        for stem in ["bypass", "processed"] {
            std::fs::rename(
                out.join(format!("{stem}-bus.tmp.wav")),
                out.join(format!("{stem}-unity-float.wav")),
            )?;
        }
    }
    let channels:Vec<_>=sources.iter().enumerate().map(|(i,s)|serde_json::json!({"file":session.channels[i].file,"role":session.channels[i].role,"bwf_time_reference":s.reference,"offset_frames":s.offset,"source_frames":s.frames,"tail_padding_frames":(source_frames+tail_frames)-s.offset-s.frames,"input":s.meter.report(),"post_strip":after[i].report(),"max_compressor_reduction_db":strips[i].max_reduction})).collect();
    let fx_report = rack.as_ref().map(|fx| serde_json::json!({
        "send_reference_meters": fx.reference_meters.iter().map(|m|m.report()).collect::<Vec<_>>(),
        "return_meters_before_master": fx.return_meters.iter().map(|m|m.report()).collect::<Vec<_>>(),
        "exciter_residual": fx.exciter_meter.report(),
        "maximizer_max_reduction_db": fx.max_reduction(), "maximizer_affected_frames": fx.affected_frames(),
        "source_frames":source_frames,"tail_frames":tail_frames
    }));
    let mut report = serde_json::json!({"output_mode":session.output_mode,"master_hpf_hz":session.master_hpf_hz,"mode":if finish_seconds.is_some(){"causal_soundcheck_then_freeze"}else{"frozen_show"},"sample_rate":session.sample_rate,"frames":total,"duration_seconds":total as f64/session.sample_rate as f64,"freeze_frame":finish_seconds.map(|_|freeze),"timeline_origin":origin,"channels":channels,
        "bypass_bus":dry_meter.report(),"processed_bus":wet_meter.report(),"bypass_lufs":ld,"processed_lufs":lw,"loudness_meter_stage":"before_export_gain","common_export_gain_db":(session.output_mode == OutputMode::Matched).then_some(export_db),"bypass_export_gain_db":dry_export,"processed_export_gain_db":wet_export,
        "bypass_export_peak_dbfs":db(dry_meter.peak)+dry_export,"processed_export_peak_dbfs":db(wet_meter.peak)+wet_export,
        "matched_target_lufs":target,"bypass_matching_gain_db":(session.output_mode == OutputMode::Matched).then_some(dry_match),"processed_matching_gain_db":(session.output_mode == OutputMode::Matched).then_some(wet_match),
        "master_max_reduction_db":limiter.max_reduction,"master_affected_frames":limiter.affected_frames,"true_peak":false,"effects":fx_report,
        "calibration_active_seconds":calibrators.iter().map(|c|c.active_seconds).collect::<Vec<_>>()});
    if policy.is_some() || observe_until.is_some() {
        report["effective_final_sample_limiter"] = serde_json::to_value(&effective_limiter)?;
        report["render_contract"] = serde_json::json!(if policy.is_some() {
            "delivery-policy-v1"
        } else {
            "observation-only-v1"
        });
        for key in [
            "common_export_gain_db",
            "bypass_export_gain_db",
            "processed_export_gain_db",
            "bypass_export_peak_dbfs",
            "processed_export_peak_dbfs",
            "matched_target_lufs",
            "bypass_matching_gain_db",
            "processed_matching_gain_db",
        ] {
            report[key] = serde_json::Value::Null;
        }
    }
    write_json(&out.join("measurements.json"), &report)?;
    if let Some(identities) = &identities {
        if super::identity::source_identities(&session, root)? != *identities {
            return Err("sources changed during render/observation".into());
        }
        if observe_until.is_some() {
            write_json(&out.join("sources.json"), identities)?;
        }
    }
    if observe_until.is_some() {
        std::fs::write(
            out.join("report.txt"),
            "Production DSP observation only; no audio export. See stages.json, stage-windows.csv and measurements.json.\n",
        )?;
        return Ok(());
    }
    if let Some(p) = policy {
        std::fs::write(
            out.join("report.txt"),
            "Policy-aware production DSP; see effective limiter in measurements.json and delivery.json for final PCM. Completion requires delivery-ready.json.\n",
        )?;
        return super::delivery::finish(&session, root, out, p, identities.as_ref().unwrap());
    }
    let comparison_note = if session.output_mode == OutputMode::Unmatched {
        "No loudness matching. Unity float buses are preserved, including any values above full scale. Each PCM output is finalized independently to its own configured sample peak; neither output determines the other's gain."
    } else {
        "Matching uses one static gain per file, outside the automixer, with sample-peak headroom."
    };
    std::fs::write(
        out.join("report.txt"),
        format!(
            "GigPies offline automixer\nMode: {}\nFrames: {total}; rate: {} Hz. No resampling or independent trimming.\nBypass: neutral trim + same pan/faders/master; no EQ/compression/calibration/limiter.\nProcessed: input trim -> HPF -> bell EQ -> linked compressor + explicit makeup -> optional exciter -> fader/pan + optional post-fader FX returns -> master -> optional master HPF -> optional master EQ/maximizer -> linked sample-peak limiter (matched mode only).\nStatic export gains: bypass {dry_export:.3} dB, processed {wet_export:.3} dB (outside causal engine).\nIntegrated K-weighted loudness BEFORE export gain: bypass {ld:?}, processed {lw:?}; matched target {target:?} LUFS. Measure finalized PCM separately for finished-file loudness.\n{comparison_note} Silence/short programmes may have no gated loudness.\nMaster maximum reduction: {:.3} dB; {} affected frames. No true-peak claim.\nFull-scale input samples are reported, not repaired. No automatic polarity or timing correction.\nSee prepared.json, measurements.json and the three history CSVs. See docs/AUTOMIX.md for preset rationale and limitations.\nListening remains required; no playback or hardware verification performed.\n",
            if finish_seconds.is_some() {
                "causal rehearsal then frozen"
            } else {
                "frozen prepared show"
            },
            session.sample_rate,
            limiter.max_reduction,
            limiter.affected_frames
        ),
    )?;
    println!("Wrote {} frames to {}", total, out.display());
    Ok(())
}

/// Compare existing stereo renders with static loudness gains and common tail padding.
/// No dynamics processing, time shifting or source replacement is performed.
pub fn compare(old: &Path, new: &Path, out: &Path) -> Result<()> {
    let rate = hound::WavReader::open(old)?.spec().sample_rate;
    let mut sources = [Source::open(old, rate)?, Source::open(new, rate)?];
    if sources.iter().any(|s| s.channels != 2) {
        return Err("comparison requires stereo WAVs".into());
    }
    let frames = sources.iter().map(|s| s.frames).max().unwrap();
    let mut meters = [Meter::default(), Meter::default()];
    let mut loud = [Loudness::new(rate), Loudness::new(rate)];
    for t in 0..frames {
        for i in 0..2 {
            let x = sources[i].next(t)?;
            for v in x {
                meters[i].add(v);
            }
            loud[i].add(x);
        }
    }
    let levels = [loud[0].integrated(), loud[1].integrated()];
    let target = match levels {
        [Some(a), Some(b)] => Some(
            (-23_f64)
                .min(a - 2. - db(meters[0].peak))
                .min(b - 2. - db(meters[1].peak)),
        ),
        _ => None,
    };
    let gains = std::array::from_fn::<_, 2, _>(|i| {
        target
            .zip(levels[i])
            .map(|(t, l)| t - l)
            .unwrap_or((-2. - db(meters[i].peak)).min(0.))
    });
    std::fs::create_dir(out)?;
    let mut final_metrics = Vec::new();
    for (i, path) in [old, new].into_iter().enumerate() {
        let mut src = Source::open(path, rate)?;
        let mut writer = hound::WavWriter::create(
            out.join(if i == 0 {
                "previous-matched.wav"
            } else {
                "new-matched.wav"
            }),
            spec(rate, 24),
        )?;
        let mut meter = Meter::default();
        let mut loud = Loudness::new(rate);
        for t in 0..frames {
            let x = src.next(t)?.map(|v| v * gain(gains[i]));
            let mut quantized = [0.; 2];
            for c in 0..2 {
                if !x[c].is_finite() || x[c].abs() >= 1. {
                    return Err("comparison export would clip".into());
                }
                let v = (x[c] * 8388608.).round() as i32;
                writer.write_sample(v)?;
                quantized[c] = v as f64 / 8388608.;
                meter.add(quantized[c]);
            }
            loud.add(quantized);
        }
        writer.finalize()?;
        final_metrics.push(serde_json::json!({"source":path,"source_frames":src.frames,"padding_frames":frames-src.frames,"gain_db":gains[i],"output":meter.report(),"output_lufs":loud.integrated()}));
    }
    write_json(
        &out.join("comparison.json"),
        &serde_json::json!({"sample_rate":rate,"frames":frames,"target_lufs":target,"files":final_metrics}),
    )?;
    println!("Wrote matched comparison to {}", out.display());
    Ok(())
}
