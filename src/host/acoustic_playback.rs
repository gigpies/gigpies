//! Explicit single-speaker PCM16 adapter; the playback device paces source frames.
//! No microphone device is opened. Driver calls sit outside graph rendering.
use super::acoustic_probe::Probe;
use crate::topology::{EngineTopology, OutputSource};
use alsa::{
    Direction, ValueOr,
    pcm::{Access, Format, HwParams, PCM},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Explicit operator evidence for this output map; never inferred from USB ID.
    #[serde(default)]
    pub mapping_confirmation: Option<String>,
    /// Explicit workaround; ordinary isolation/level policy remains unchanged.
    #[serde(default)]
    pub pilot_bench: bool,
    /// Optional fixed mono PCM16 broadband source, loaded before opening PCM.
    #[serde(default)]
    pub excitation: Option<PathBuf>,
    pub device: String,
    pub software_only: bool,
    pub expected_usb_id: String,
    pub channel: usize,
    pub probe: Probe,
    pub references: PathBuf,
    pub epoch_file: PathBuf,
}
impl Config {
    fn mapping_confirmed(&self) -> bool {
        self.mapping_confirmation
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty() && v.len() <= 512)
    }
    pub fn validate(&self, duration_ms: u64) -> Result<(), String> {
        if !self.software_only && !self.mapping_confirmed() {
            return Err(
                "physical acoustic playback requires explicit operator output-map confirmation"
                    .into(),
            );
        }
        if self.pilot_bench {
            self.probe.validate_limit(-30.)?;
        } else {
            self.probe.validate()?;
        }
        if self.device != "hw:AUDIO,0"
            || self.expected_usb_id != "0d8c:4147"
            || self.channel > 1
            || !(1000..=if self.pilot_bench { 60_000 } else { 20_000 }).contains(&duration_ms)
            || (self.pilot_bench && (self.channel != 1 || self.probe.first_frame < 192_000))
            || (self.excitation.is_some() && !self.pilot_bench)
            || !self.references.is_absolute()
            || !self.epoch_file.is_absolute()
            || self.probe.first_frame
                + self.probe.spacing_frames * u64::from(self.probe.repeats - 1)
                + 48_000
                > duration_ms * 48
        {
            return Err("acoustic bench endpoint/channel/duration/reference bounds".into());
        }
        Ok(())
    }
    pub fn buffer_frames(&self) -> usize {
        // The explicit slow-USB bench favors margin over minimum playback latency.
        if self.pilot_bench { 4800 } else { 960 }
    }
    pub fn topology(&self) -> Result<EngineTopology, String> {
        if !self.software_only && !self.mapping_confirmed() {
            return Err("unconfirmed acoustic output map".into());
        }
        let mut t = EngineTopology::software(2, 0, 2)?;
        t.identity = "acoustic-playback-generated-inputs".into();
        t.mapping_evidence = if self.software_only {
            "synthetic"
        } else {
            "operator-verified"
        }
        .into();
        t.playback_channels = 2;
        t.outputs.truncate(2);
        for (i, out) in t.outputs.iter_mut().enumerate() {
            if !self.software_only {
                out.physical_port = format!("usb-amp-output-{}", i + 1);
            }
            out.source = if i == self.channel {
                Some(OutputSource::Pa { index: 0 })
            } else {
                None
            };
        }
        t.validate(crate::topology::ResourceBudget::default())?;
        Ok(t)
    }
}
pub struct Playback {
    pcm: Option<PCM>,
    excitation: Option<Vec<f64>>,
    pilot_frame: u64,
    next_tick: Instant,
    config: Config,
    topology: EngineTopology,
    bytes: [u8; 192],
    offset: usize,
    pending: bool,
    deadline: Instant,
    stopped: bool,
    reference_file: std::fs::File,
    references: Vec<f32>,
    used: usize,
}
impl Playback {
    pub fn open(config: Config, duration_ms: u64) -> Result<Self, String> {
        config.validate(duration_ms)?;
        let topology = config.topology()?;
        let excitation = if let Some(path) = &config.excitation {
            let mut r = hound::WavReader::open(path).map_err(|e| e.to_string())?;
            let spec = r.spec();
            if spec.channels != 1
                || spec.sample_rate != 48000
                || spec.bits_per_sample != 16
                || spec.sample_format != hound::SampleFormat::Int
                || !(48000..=144000).contains(&r.duration())
                || config.probe.first_frame
                    + config.probe.spacing_frames * u64::from(config.probe.repeats - 1)
                    + u64::from(r.duration())
                    + 48000
                    > duration_ms * 48
                || u64::from(r.duration()) >= config.probe.spacing_frames
            {
                return Err("bench excitation requires bounded mono PCM16/48k and a gap".into());
            }
            let samples: Vec<f64> = r
                .samples::<i16>()
                .map(|v| v.map(|v| f64::from(v) / 32768.))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            if samples.iter().any(|v| v.abs() > 10f64.powf(-24. / 20.)) {
                return Err("bench excitation exceeds reviewed -24 dBFS ceiling".into());
            }
            Some(samples)
        } else {
            None
        };
        if !config.software_only
            && std::fs::read_to_string("/proc/asound/AUDIO/usbid")
                .map_err(|e| e.to_string())?
                .trim()
                != config.expected_usb_id
        {
            return Err("USB identity mismatch".into());
        }
        let reference_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&config.references)
            .map_err(|e| e.to_string())?;
        let references = vec![0.; duration_ms as usize * 48 * 20];
        let result = if config.software_only {
            None
        } else {
            Some(
                (|| -> Result<_, Box<dyn std::error::Error>> {
                    let pcm = PCM::new(&config.device, Direction::Playback, true)?;
                    let p = HwParams::any(&pcm)?;
                    p.set_access(Access::RWInterleaved)?;
                    p.set_rate_resample(false)?;
                    p.set_format(Format::S16LE)?;
                    p.set_channels(2)?;
                    p.set_rate(48000, ValueOr::Nearest)?;
                    p.set_period_size(48, ValueOr::Nearest)?;
                    p.set_buffer_size(config.buffer_frames() as i64)?;
                    pcm.hw_params(&p)?;
                    let actual = pcm.hw_params_current()?;
                    if actual.get_rate()? != 48000
                        || actual.get_channels()? != 2
                        || actual.get_format()? != Format::S16LE
                        || actual.get_period_size()? != 48
                        || actual.get_buffer_size()? != config.buffer_frames() as i64
                    {
                        return Err("PCM negotiation changed explicit bench shape".into());
                    }
                    let sw = pcm.sw_params_current()?;
                    sw.set_start_threshold(sw.get_boundary()?)?;
                    sw.set_avail_min(48)?;
                    pcm.sw_params(&sw)?;
                    drop(sw);
                    drop(actual);
                    drop(p);
                    pcm.prepare()?;
                    Ok(pcm)
                })()
                .map_err(|e| e.to_string())?,
            )
        };
        Ok(Self {
            pcm: result,
            excitation,
            pilot_frame: 0,
            next_tick: Instant::now(),
            config,
            topology,
            bytes: [0; 192],
            offset: 0,
            pending: false,
            deadline: Instant::now(),
            stopped: false,
            reference_file,
            references,
            used: 0,
        })
    }
    pub fn map_valid(&self, t: &EngineTopology) -> bool {
        super::duplex::same_device_map(t, &self.topology) && t.pa_outputs == 2
    }
    pub fn start(&mut self) -> Result<(), &'static str> {
        if self.stopped {
            return Err("acoustic adapter latched stopped");
        }
        // Pilot starts with prefill and continues independently of graph mute and
        // gaps. The graph must still produce exact zero on the inactive channel.
        let frames = self.config.buffer_frames();
        let mut prefill = [0u8; 4800 * 4];
        if self.config.pilot_bench {
            for (i, frame) in prefill[..frames * 4].chunks_exact_mut(4).enumerate() {
                frame[..2].copy_from_slice(&pilot(i as u64).to_le_bytes());
            }
            self.pilot_frame = frames as u64;
        }
        let Some(pcm) = &self.pcm else {
            self.next_tick = Instant::now();
            self.deadline = self.next_tick + Duration::from_millis(100);
            return Ok(());
        };
        if pcm
            .io_bytes()
            .writei(&prefill[..frames * 4])
            .map_err(|_| "PCM prefill")?
            != frames
        {
            return Err("partial PCM prefill");
        }
        pcm.start().map_err(|_| "PCM start")?;
        self.deadline = Instant::now() + Duration::from_millis(100);
        Ok(())
    }
    pub fn ready(&mut self, now: Instant) -> Result<bool, &'static str> {
        if self.stopped || now >= self.deadline {
            return Err("acoustic playback deadline");
        }
        let Some(pcm) = &self.pcm else {
            return Ok(now >= self.next_tick);
        };
        let available = pcm.avail_update().map_err(|_| "acoustic playback xrun")?;
        Ok(available >= 48)
    }
    pub fn measuring(&self, frame: u64) -> bool {
        frame >= self.config.probe.first_frame
    }
    pub fn fill(&self, frame: u64, capture: &mut [f64]) {
        for (i, pair) in capture.chunks_exact_mut(2).enumerate() {
            if let Some(source) = &self.excitation {
                let p = &self.config.probe;
                let sample = (frame + i as u64)
                    .checked_sub(p.first_frame)
                    .filter(|n| n / p.spacing_frames < u64::from(p.repeats))
                    .and_then(|n| source.get((n % p.spacing_frames) as usize))
                    .copied()
                    .unwrap_or(0.);
                pair.copy_from_slice(&[sample, 0.]);
            } else {
                pair.copy_from_slice(&self.config.probe.sample(frame + i as u64));
            }
        }
    }
    pub fn queue(&mut self, playback: &[f64]) -> Result<(), &'static str> {
        if self.pending || playback.len() != 96 {
            return Err("acoustic playback shape/pending");
        }
        for (i, &s) in playback.iter().enumerate() {
            if !s.is_finite()
                || s.abs()
                    > 10f64.powf(if self.config.pilot_bench {
                        -24. / 20.
                    } else {
                        -30. / 20.
                    })
                || (i % 2 != self.config.channel && s != 0.)
            {
                return Err("acoustic output peak/isolation refusal");
            }
            self.bytes[i * 2..i * 2 + 2]
                .copy_from_slice(&((s * 32768.).round() as i16).to_le_bytes());
        }
        if self.config.pilot_bench {
            for (i, frame) in self.bytes.chunks_exact_mut(4).enumerate() {
                frame[..2].copy_from_slice(&pilot(self.pilot_frame + i as u64).to_le_bytes());
            }
            self.pilot_frame += 48;
        }
        self.offset = 0;
        self.pending = true;
        Ok(())
    }
    pub fn flush(&mut self, now: Instant) -> Result<bool, &'static str> {
        if self.stopped || now >= self.deadline {
            return Err("acoustic stale playback");
        }
        let Some(pcm) = &self.pcm else {
            self.pending = false;
            self.next_tick += Duration::from_millis(1);
            self.deadline = now + Duration::from_millis(100);
            return Ok(true);
        };
        match pcm.io_bytes().writei(&self.bytes[self.offset * 4..]) {
            Ok(n) if n <= 48 - self.offset => self.offset += n,
            Err(e) if matches!(e.errno(), libc::EAGAIN | libc::EINTR) => return Ok(false),
            _ => return Err("acoustic write/xrun"),
        }
        if self.offset == 48 {
            self.pending = false;
            self.deadline = now + Duration::from_millis(100);
            Ok(true)
        } else {
            Ok(false)
        }
    }
    pub fn observe(
        &mut self,
        capture: &[f64],
        sends: &[[f64; 96]],
        wet: &[f64],
        output: &[f64],
    ) -> Result<(), &'static str> {
        if capture.len() != 96
            || sends.len() != 4
            || wet.len() != 384
            || output.len() != 96
            || self.used + 48 * 20 > self.references.len()
        {
            return Err("acoustic reference shape/capacity");
        }
        for f in 0..48 {
            let row = &mut self.references[self.used..self.used + 20];
            row[0] = capture[f * 2] as f32;
            row[1] = capture[f * 2 + 1] as f32;
            for b in 0..4 {
                for c in 0..2 {
                    row[2 + b * 2 + c] = sends[b][f * 2 + c] as f32;
                    row[10 + b * 2 + c] = wet[f * 8 + b * 2 + c] as f32;
                }
            }
            row[18] = if self.config.pilot_bench {
                f32::from(i16::from_le_bytes([
                    self.bytes[f * 4],
                    self.bytes[f * 4 + 1],
                ])) / 32768.
            } else {
                output[f * 2] as f32
            };
            row[19] = output[f * 2 + 1] as f32;
            self.used += 20;
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        self.stopped = true;
        if let Some(pcm) = self.pcm.take() {
            let _ = pcm.drop();
        }
    }
    /// Controller only, after the source owner is stopped. Never write evidence in render.
    pub fn finish(&mut self) -> Result<(), String> {
        self.stop();
        let spec = hound::WavSpec {
            channels: 20,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer =
            hound::WavWriter::new(std::io::BufWriter::new(&mut self.reference_file), spec)
                .map_err(|e| e.to_string())?;
        for &s in &self.references[..self.used] {
            writer.write_sample(s).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

// Fixed -80dBFS / 440Hz, PCM16 quantization is part of the bench provenance.
fn pilot(frame: u64) -> i16 {
    (32768. * 0.0001 * (std::f64::consts::TAU * 440. * (frame % 1200) as f64 / 48000.).sin())
        .round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acoustic_software_adapter_bounds_isolation_deadline_and_no_allocations() {
        let path = std::env::temp_dir().join(format!("gp-acoustic-{}.wav", std::process::id()));
        let c = Config {
            mapping_confirmation: None,
            pilot_bench: false,
            excitation: None,
            device: "hw:AUDIO,0".into(),
            software_only: true,
            expected_usb_id: "0d8c:4147".into(),
            channel: 1,
            probe: Probe {
                first_frame: 48000,
                spacing_frames: 48000,
                repeats: 1,
                same_carrier: false,
                peak_dbfs: -42.,
            },
            references: path.clone(),
            epoch_file: std::env::temp_dir().join("unused-acoustic-epoch"),
        };
        assert_eq!(c.buffer_frames(), 960);
        assert!(c.validate(1000).is_err());
        let mut physical = c.clone();
        physical.software_only = false;
        assert!(physical.validate(3000).is_err());
        assert!(physical.topology().is_err());
        physical.mapping_confirmation =
            Some("Operator confirms selected right USB amp output; retained route evidence".into());
        physical.validate(3000).unwrap();
        let topology = physical.topology().unwrap();
        assert_eq!(topology.mapping_evidence, "operator-verified");
        let mut engine = crate::mixer_control::OfflineEngine::with_topology(
            "11111111-1111-4111-8111-111111111111",
            crate::show::Counter(1),
            crate::show::Counter(0),
            0,
            topology,
        )
        .unwrap();
        engine.mark_verified_mapping().unwrap();
        assert!(engine.clock_status().mapping_verified);
        drop(engine);
        let t = c.topology().unwrap();
        assert_eq!(t.outputs[0].source, None);
        assert_eq!(t.outputs[1].source, Some(OutputSource::Pa { index: 0 }));
        let mut d = Playback::open(c.clone(), 3000).unwrap();
        d.start().unwrap();
        let now = Instant::now();
        crate::remote::assert_no_allocations(|| {
            assert!(d.ready(now).unwrap());
            let mut capture = [0.; 96];
            d.fill(48000, &mut capture);
            let mut output = [0.; 96];
            output[1] = 0.001;
            d.observe(&capture, &[[0.; 96]; 4], &[0.; 384], &output)
                .unwrap();
            d.queue(&output).unwrap();
            assert!(d.queue(&output).is_err());
            assert!(d.flush(now).unwrap());
            output[0] = 0.001;
            assert!(d.queue(&output).is_err());
            output[0] = 0.;
            output[1] = 0.1;
            assert!(d.queue(&output).is_err());
            assert!(d.ready(now + Duration::from_secs(1)).is_err());
        });
        d.finish().unwrap();
        assert!(d.start().is_err());
        drop(d);
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), 48);
        assert_eq!(reader.spec().channels, 20);
        drop(reader);
        std::fs::remove_file(&path).unwrap();
        let mut bench = c;
        bench.pilot_bench = true;
        assert_eq!(bench.buffer_frames(), 4800);
        assert!(bench.validate(6000).is_err()); // four second warmup mandatory
        bench.probe.first_frame = 192000;
        bench.validate(6000).unwrap();
        let mut d = Playback::open(bench, 6000).unwrap();
        d.start().unwrap();
        let now = Instant::now();
        crate::remote::assert_no_allocations(|| {
            let mut output = [0.; 96];
            output[1] = 0.05;
            d.queue(&output).unwrap();
            let first = d.bytes;
            assert!(first.chunks_exact(4).any(|v| v[0] != 0 || v[1] != 0));
            assert!(d.flush(now).unwrap());
            d.queue(&output).unwrap();
            assert_ne!(first, d.bytes); // phase continues through block boundaries
            assert!(d.flush(now).unwrap());
            output[0] = 0.00001;
            assert!(d.queue(&output).is_err());
            output[0] = 0.;
            output[1] = 0.064;
            assert!(d.queue(&output).is_err());
        });
        d.stop();
        assert!(d.ready(now).is_err());
        let mut c = d.config.clone();
        drop(d);
        std::fs::remove_file(&path).unwrap();
        let mono = path.with_extension("mono.wav");
        c.excitation = Some(mono.clone());
        c.probe.spacing_frames = 144000;
        let write = |sample: i16| {
            let mut w = hound::WavWriter::create(
                &mono,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 48000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for _ in 0..48000 {
                w.write_sample(sample).unwrap();
            }
            w.finalize().unwrap();
        };
        write(3000);
        assert!(Playback::open(c.clone(), 8000).is_err());
        assert!(!path.exists()); // validation happens before reference/device open
        write(327);
        let d = Playback::open(c, 8000).unwrap();
        let mut capture = [0.; 96];
        d.fill(192000, &mut capture);
        assert_eq!(capture[0], 327. / 32768.);
        assert_eq!(capture[1], 0.);
        d.fill(240000, &mut capture);
        assert_eq!(capture, [0.; 96]);
        drop(d);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(mono).unwrap();
    }
}
