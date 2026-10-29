use gigpies::automix::{self, balance, bass, config, drums, preservation, unity};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    s: config::Session,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "gigpies-preservation-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    let mut s = config::example();
    s.channels.truncate(4);
    s.sample_rate = 8000;
    for (i, c) in s.channels.iter_mut().enumerate() {
        c.file = format!("{i}.wav").into();
        c.role = [
            config::Role::Kick,
            config::Role::Snare,
            config::Role::BassDi,
            config::Role::Other,
        ][i];
        c.eq.clear();
        let mut w = hound::WavWriter::create(
            root.join(&c.file),
            hound::WavSpec {
                channels: if i == 3 { 2 } else { 1 },
                sample_rate: 8000,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for k in 0..8000 * 3 {
            let t = k as f64 / 8000.;
            let x = 0.15
                * (std::f64::consts::TAU * [63., 170., 31., 93.][i] * t).sin()
                * (-(t % 0.5) * 8.).exp();
            w.write_sample((x * 8388608.) as i32).unwrap();
            if i == 3 {
                w.write_sample((-0.7 * x * 8388608.) as i32).unwrap();
            }
        }
        w.finalize().unwrap();
    }
    unity::prepare(&mut s).unwrap();
    for (i, c) in s.channels.iter_mut().enumerate() {
        c.fader_db = -(i as f64);
        c.pan = i as f64 / 10.;
    }
    Fixture { root, s }
}
#[test]
fn source_can_be_final_with_exact_samples_stereo_and_balance() {
    let mut f = fixture();
    // Reopening an ensemble candidate must also bypass its dynamic processing.
    f.s.channels[0].dynamic_eq = Some(automix::dynamic_eq::Config {
        version: 1,
        hz: 63.,
        q: 1.,
        depth_db: 6.,
        threshold_dbfs: -60.,
        attack_ms: 1.,
        release_ms: 80.,
    });
    let originals: Vec<_> =
        f.s.channels
            .iter()
            .map(|c| std::fs::read(f.root.join(&c.file)).unwrap())
            .collect();
    let s = preservation::source_settings(&f.s).unwrap();
    assert!(s.channels.iter().all(|c| c.dynamic_eq.is_none()));
    assert!(matches!(s.channels[3].role, config::Role::Other));
    for (a, b) in s.channels.iter().zip(&f.s.channels) {
        assert_eq!(a.pan, b.pan);
        assert_eq!(a.fader_db, b.fader_db);
        assert_eq!(a.file, b.file);
    }
    let out = f.root.join("selection");
    preservation::prepare(f.s.clone(), &out).unwrap();
    assert!(preservation::prepare(f.s.clone(), &out).is_err());
    automix::run(s.clone(), &f.root, &f.root.join("render"), None).unwrap();
    for (a, b) in [
        ("bypass.wav", "processed.wav"),
        ("bypass-unity-float.wav", "processed-unity-float.wav"),
    ] {
        assert_eq!(
            std::fs::read(f.root.join("render").join(a)).unwrap(),
            std::fs::read(f.root.join("render").join(b)).unwrap()
        );
    }
    let m = balance::measure(&s, &f.root, &balance::Policy::for_session(&s)).unwrap();
    let source = balance::measure_source(&s, &f.root, &balance::Policy::for_session(&s)).unwrap();
    for (a, b) in m.windows.iter().zip(&source.windows) {
        assert_eq!(a.master_dbfs, b.master_dbfs);
    }
    for (c, bytes) in s.channels.iter().zip(originals) {
        assert_eq!(std::fs::read(f.root.join(&c.file)).unwrap(), bytes);
    }
    let r: serde_json::Value = serde_json::from_reader(
        std::fs::File::open(f.root.join("render/measurements.json")).unwrap(),
    )
    .unwrap();
    assert!((r["processed_export_peak_dbfs"].as_f64().unwrap() + 0.01).abs() < 1e-8);
    assert_eq!(r["master_affected_frames"], 0);
}
#[test]
fn analysis_uses_configured_master_filter_including_bypass() {
    let f = fixture();
    let b = bass::Policy {
        bass: 2,
        bass_file: "2.wav".into(),
        kick: 0,
        kick_file: "0.wav".into(),
        definition_body_db: [-12., -4.],
        definition_low_db: None,
        max_eq_db: 4.,
        max_output_rise_db: 3.,
        max_note_spread_db: 2.,
        max_added_reduction_db: 1.,
        max_fader_db: 0.,
        recorded_source: true,
    };
    let d = drums::Policy {
        kick: 0,
        kick_file: "0.wav".into(),
        snare: 1,
        snare_file: "1.wav".into(),
        bass: 2,
        bass_file: "2.wav".into(),
        neutral_basis: "unknown".into(),
        current_emphasis_db: None,
        desired_emphasis_db: [0.; 2],
    };
    for hz in [0., 40., 65.] {
        let mut s = preservation::source_settings(&f.s).unwrap();
        s.master_hpf_hz = hz;
        let out = f.root.join(format!("hp-{hz}"));
        automix::run(s.clone(), &f.root, &out, None).unwrap();
        let bm = bass::measure(&s, &f.root, &b, 0., 3.).unwrap();
        let dm = drums::measure(&s, &f.root, &d, 0., 3.).unwrap();
        let mut r = hound::WavReader::open(out.join("processed-unity-float.wav")).unwrap();
        let samples: Vec<_> = r.samples::<f32>().map(Result::unwrap).collect();
        for window in &bm.windows {
            let start = (window.seconds * 8000.).round() as usize * 2;
            let xs = &samples[start..start + 2 * bass::SIZE];
            let rms =
                (xs.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / xs.len() as f64).sqrt();
            assert!((automix::dsp::db(rms) - window.stages[10].rms_dbfs).abs() < 1e-6);
        }
        for (i, frame) in dm.frames.iter().enumerate() {
            let a = i * 80 * 2;
            let xs = &samples[a..a + 80 * 2];
            let rms =
                (xs.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / xs.len() as f64).sqrt();
            assert!(
                (automix::dsp::db(rms) - frame.rms[14]).abs() < 1e-6,
                "HPF {hz}"
            );
        }
    }
}
#[test]
fn optional_filters_do_not_weaken_routing_trim_or_export_contracts() {
    let f = fixture();
    for field in ["trim", "amp", "export", "nonfinite"] {
        let mut s = f.s.clone();
        match field {
            "trim" => s.groups[0].trim_db = 1.,
            "amp" => s.channels[2].role = config::Role::BassAmp,
            "export" => s.ceiling_db = -1.,
            _ => s.channels[0].hpf_hz = f64::NAN,
        }
        assert!(preservation::prepare(s, &f.root.join(field)).is_err());
        assert!(!f.root.join(field).exists());
    }
}
