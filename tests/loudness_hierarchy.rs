use gigpies::automix::{
    config::{self, Role},
    hierarchy::{self, Policy},
    unity,
};
fn session() -> config::Session {
    let mut s = config::example();
    s.channels.truncate(4);
    for (i, c) in s.channels.iter_mut().enumerate() {
        c.role = [
            Role::Kick,
            Role::BassDi,
            Role::LeadVocal,
            Role::RhythmGuitar,
        ][i];
        c.file = format!("{i}.wav").into();
        c.fader_db = 0.;
        c.pan = 0.;
        c.hpf_hz = 0.;
        c.eq.clear();
        c.compressor.ratio = 1.;
        c.compressor.makeup_db = 0.;
    }
    unity::prepare(&mut s).unwrap();
    s
}
#[test]
fn hierarchy_preserves_quieter_sources_and_limits_changes() {
    let s = session();
    let p = Policy::default();
    let a = hierarchy::propose(&s, &[Some(-20.), Some(-20.), Some(-20.), Some(-30.)], &p).unwrap();
    assert_eq!(a.deltas_db, [0., -2., -3., 0.]);
    let b = hierarchy::propose(&s, &[Some(-10.), Some(-10.), Some(-10.), Some(-20.)], &p).unwrap();
    assert_eq!(a.deltas_db, b.deltas_db);
    let a = hierarchy::propose(&s, &[Some(-20.), Some(-35.), Some(-10.), None], &p).unwrap();
    assert_eq!(a.deltas_db, [0., 6., -6., 0.]);
    assert!(a.targets_lufs[3].is_none());
    for levels in [
        [None, Some(-20.), Some(-20.), None],
        [Some(-20.), None, Some(-20.), None],
    ] {
        assert_eq!(
            hierarchy::propose(&s, &levels, &p).unwrap().deltas_db,
            [0.; 4]
        );
    }
    let mut p = p;
    p.bass_to_others_lu = f64::NAN;
    assert!(hierarchy::propose(&s, &[Some(-20.); 4], &p).is_err());
    let mut s = s;
    s.channels[3].role = Role::Kick;
    assert!(hierarchy::propose(&s, &[Some(-20.); 4], &Policy::default()).is_err());
}
#[test]
fn production_remeasurement_reports_compliance_and_unresolved_sources() {
    let root = std::env::temp_dir().join(format!("gigpies-hierarchy-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let s = session();
    for i in 0..4 {
        let mut w = hound::WavWriter::create(
            root.join(format!("{i}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: s.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        // Equal sources isolate routing/fader behavior; differing spectra tested below.
        for n in 0..s.sample_rate * 2 {
            w.write_sample(
                (0.1 * (std::f64::consts::TAU * 997. * n as f64 / s.sample_rate as f64).sin())
                    as f32,
            )
            .unwrap();
        }
        w.finalize().unwrap();
    }
    let out = root.join("proposal");
    hierarchy::run(s.clone(), &root, &out, Policy::default()).unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["policy_targets_met"], true);
    let levels = report["after_lufs"].as_array().unwrap();
    let level = |i: usize| levels[i].as_f64().unwrap();
    assert!((level(0) - level(1) - 2.).abs() < 0.01);
    assert!((level(1) - level(2) - 1.).abs() < 0.01);
    assert!(hierarchy::run(s.clone(), &root, &out, Policy::default()).is_err());
    let constrained = root.join("constrained");
    hierarchy::run(
        s.clone(),
        &root,
        &constrained,
        Policy {
            max_fader_change_db: 0.5,
            ..Policy::default()
        },
    )
    .unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(constrained.join("report.json")).unwrap())
            .unwrap();
    assert_eq!(report["policy_targets_met"], false);
    let mut w = hound::WavWriter::create(
        root.join("3.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: s.sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for _ in 0..s.sample_rate * 2 {
        w.write_sample(0_f32).unwrap();
    }
    w.finalize().unwrap();
    hierarchy::run(s, &root, &root.join("silent"), Policy::default()).unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(root.join("silent/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["policy_targets_met"], false);
    assert!(report["after_lufs"][3].is_null());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn k_weighting_distinguishes_sub_energy_from_midrange() {
    use gigpies::automix::dsp::Loudness;
    let level = |hz: f64| {
        let mut meter = Loudness::new(48000);
        for n in 0..96000 {
            let x = 0.1 * (std::f64::consts::TAU * hz * n as f64 / 48000.).sin();
            meter.add([x, 0.]);
        }
        meter.integrated().unwrap()
    };
    assert!(level(997.) > level(30.) + 6.);
}
