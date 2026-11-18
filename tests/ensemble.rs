use gigpies::automix::{
    config,
    dsp::Loudness,
    ensemble::{self, Request, spectrum},
    unity,
};
use std::f64::consts::TAU;

#[test]
fn spectral_accounting_matches_independent_dft_and_parseval() {
    for size in [1_usize, 2, 3, 17, 64, 255] {
        let x = (0..size)
            .map(|i| {
                0.13 + 0.2 * (TAU * 3.0 * i as f64 / size as f64).cos()
                    + if i % 2 == 0 { 0.1 } else { -0.1 }
            })
            .collect::<Vec<_>>();
        let s = spectrum::measure(&x, 8001, &[0., 25., 100., 1000., 4000.5, 12000.]).unwrap();
        let windowed = x
            .iter()
            .enumerate()
            .map(|(i, x)| {
                x * if size < 3 {
                    1.
                } else {
                    0.5 - 0.5 * (TAU * i as f64 / (size - 1) as f64).cos()
                }
            })
            .collect::<Vec<_>>();
        let truth = windowed.iter().map(|x| x * x).sum::<f64>() / s.window_square_sum;
        assert!((truth - s.total_power).abs() < 1e-12);
        let mut dft = 0.;
        for k in 0..s.fft_size {
            let re = windowed
                .iter()
                .enumerate()
                .map(|(i, x)| x * (TAU * k as f64 * i as f64 / s.fft_size as f64).cos())
                .sum::<f64>();
            let im = windowed
                .iter()
                .enumerate()
                .map(|(i, x)| x * (TAU * k as f64 * i as f64 / s.fft_size as f64).sin())
                .sum::<f64>();
            dft += (re * re + im * im) / (s.fft_size as f64 * s.window_square_sum);
        }
        assert!((dft - s.total_power).abs() < 1e-12);
        assert!((s.bands.iter().filter_map(|b| b.power).sum::<f64>() - truth).abs() < 1e-12);
        assert!(!s.bands.last().unwrap().supported);
        assert_eq!(s.bands.last().unwrap().power, None);
    }
}
#[test]
fn dc_nyquist_silence_and_sub_are_not_lost() {
    for rate in [8000, 44101, 48000, 192000] {
        for kind in 0..4 {
            let x = (0..8192)
                .map(|i| match kind {
                    0 => 0.,
                    1 => 0.2,
                    2 => {
                        if i % 2 == 0 {
                            0.2
                        } else {
                            -0.2
                        }
                    }
                    _ => 0.2 * (TAU * 10. * i as f64 / rate as f64).sin(),
                })
                .collect::<Vec<_>>();
            let s = spectrum::measure(&x, rate, &[0., 25., rate as f64 / 2.]).unwrap();
            assert!(
                (s.bands.iter().map(|b| b.power.unwrap()).sum::<f64>() - s.total_power).abs()
                    < 1e-10
            );
            if kind == 0 {
                assert_eq!(s.total_power, 0.);
            }
            if kind == 1 {
                assert!((s.dc_mean - 0.2).abs() < 1e-12);
                assert!((s.total_power - 0.04).abs() < 1e-12);
            }
            if kind == 2 {
                assert!(s.nyquist_power > 0.02);
                assert!((s.total_power - 0.04).abs() < 1e-12);
            }
        }
    }
}
#[test]
fn loudness_native_windows_do_not_drift_or_accept_incomplete_blocks() {
    for rate in [8000_u32, 8001, 44101, 48000] {
        let mut m = Loudness::new(rate);
        let first = (u64::from(rate) * 4).div_ceil(10);
        for t in 0..first - 1 {
            m.add([0.1 * (TAU * 997. * t as f64 / rate as f64).sin(), 0.]);
        }
        assert_eq!(m.integrated(), None);
        assert_eq!(m.momentary_max(), None);
        m.add([0.1, 0.]);
        assert!(m.integrated().is_some());
        assert!(m.momentary_max().is_some());
        for t in first..u64::from(rate) * 3 - 1 {
            m.add([0.1 * (TAU * 997. * t as f64 / rate as f64).sin(), 0.]);
        }
        assert_eq!(m.short_term_max(), None);
        m.add([0.1, 0.]);
        assert!(m.short_term_max().is_some());
        if rate > 40000 {
            assert!((m.integrated().unwrap() + 23.01).abs() < 0.08);
        }
    }
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("ensemble-{name}-{}-{id}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch directory {}: {error}", path.display()),
            }
        }
    }
    fn wav(&self, file: &str) {
        let mut w = hound::WavWriter::create(
            self.0.join(file),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..480 {
            w.write_sample((0.1 * (TAU * 997. * i as f64 / 48000.).sin()) as f32)
                .unwrap();
        }
        w.finalize().unwrap();
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn same_name_scratch_directories_have_independent_lifetimes() {
    let first = Scratch::new("shared-label");
    let second = Scratch::new("shared-label");
    assert_ne!(first.0, second.0);
    drop(first);
    assert!(second.0.is_dir());
}

fn fixture(dir: &Scratch, count: usize) -> config::Session {
    let mut s = config::example();
    s.sample_rate = 48000;
    s.channels = vec![s.channels[0].clone(); count];
    s.groups.truncate(1);
    s.groups[0].reference = 0;
    for (i, c) in s.channels.iter_mut().enumerate() {
        c.file = format!("{i}.wav").into();
        c.group = s.groups[0].name.clone();
        dir.wav(&format!("{i}.wav"));
    }
    unity::prepare(&mut s).unwrap();
    s
}
#[test]
fn frozen_identity_budgets_and_configured_capacity() {
    for count in [16, 32, 48] {
        let dir = Scratch::new(&format!("identity-{count}"));
        let s = fixture(&dir, count);
        let mut request = Request::default();
        let frozen = ensemble::freeze(&s, &dir.0, &request).unwrap();
        frozen.verify(&s, &dir.0, &request).unwrap();
        assert_eq!(frozen.sample_visits, 480 * count as u64);
        request.budget.max_sample_visits = frozen.sample_visits - 1;
        assert!(ensemble::freeze(&s, &dir.0, &request).is_err());
        request = Request::default();
        request.budget.max_source_bytes = 1;
        assert!(ensemble::freeze(&s, &dir.0, &request).is_err());
        request = Request::default();
        request.captures.push(ensemble::CaptureIdentity {
            channel: 0,
            performance: Some("bass-performance".into()),
            capture: Some("DI".into()),
        });
        assert!(frozen.verify(&s, &dir.0, &request).is_err());
        let mut changed = s.clone();
        changed.channels[0].fader_db -= 1.;
        assert!(
            frozen
                .verify(&changed, &dir.0, &Request::default())
                .is_err()
        );
        let mut bytes = std::fs::read(dir.0.join("0.wav")).unwrap();
        let n = bytes.len();
        bytes[n - 1] ^= 1;
        std::fs::write(dir.0.join("0.wav"), bytes).unwrap();
        assert!(frozen.verify(&s, &dir.0, &Request::default()).is_err());
    }
}

fn bwf(path: &std::path::Path, reference: u64) {
    let bytes = std::fs::read(path).unwrap();
    let mut data = bytes[..12].to_vec();
    data.extend(b"bext");
    data.extend(346_u32.to_le_bytes());
    let mut meta = vec![0; 346];
    meta[338..346].copy_from_slice(&reference.to_le_bytes());
    data.extend(meta);
    data.extend(&bytes[12..]);
    let size = (data.len() - 8) as u32;
    data[4..8].copy_from_slice(&size.to_le_bytes());
    std::fs::write(path, data).unwrap();
}
#[test]
fn offsets_are_frozen_and_sparse_timeline_work_is_bounded() {
    let dir = Scratch::new("bwf");
    let s = fixture(&dir, 2);
    let p = Request::default();
    bwf(&dir.0.join("0.wav"), 1000);
    assert!(ensemble::freeze(&s, &dir.0, &p).is_err());
    bwf(&dir.0.join("1.wav"), 1200);
    let f = ensemble::freeze(&s, &dir.0, &p).unwrap();
    assert_eq!(f.timeline_frames, 680);
    assert_eq!(f.inputs[1].offset, 200);
    let mut p = p;
    p.budget.max_sample_visits = 1359;
    assert!(ensemble::freeze(&s, &dir.0, &p).is_err());
}

#[test]
fn shared_workflow_routes_every_role_and_preserves_no_policy() {
    use gigpies::automix::{
        config::Role,
        ensemble::workflow::{self, State},
    };
    let dir = Scratch::new("workflow");
    let mut s = fixture(&dir, 14);
    let roles = [
        Role::Kick,
        Role::BassDi,
        Role::BassAmp,
        Role::Snare,
        Role::Tom,
        Role::Overheads,
        Role::DrumRoom,
        Role::RhythmGuitar,
        Role::LeadGuitar,
        Role::AcousticGuitar,
        Role::LeadVocal,
        Role::BackingVocal,
        Role::VocalRoom,
        Role::Other,
    ];
    for (c, role) in s.channels.iter_mut().zip(roles) {
        c.role = role;
    }
    let p = Request::default();
    let out = dir.0.join("analysis");
    workflow::analyze(s.clone(), &dir.0, &out, p).unwrap();
    let state: State =
        serde_json::from_reader(std::fs::File::open(out.join("state.json")).unwrap()).unwrap();
    let decisions = workflow::decisions(&state);
    assert_eq!(decisions.len(), 14);
    assert!(
        decisions
            .iter()
            .all(|d| !d.changed && d.applicability == "observation_only")
    );
    workflow::prepare(state.clone(), &dir.0, &dir.0.join("one")).unwrap();
    workflow::prepare(state.clone(), &dir.0, &dir.0.join("two")).unwrap();
    assert_eq!(
        std::fs::read(dir.0.join("one/settings.json")).unwrap(),
        std::fs::read(dir.0.join("two/settings.json")).unwrap()
    );
    assert_eq!(
        std::fs::read(dir.0.join("one/observations.json")).unwrap(),
        std::fs::read(dir.0.join("two/observations.json")).unwrap()
    );
    workflow::reset(state.clone(), &dir.0.join("reset")).unwrap();
    let restored: config::Session =
        serde_json::from_reader(std::fs::File::open(dir.0.join("reset/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        serde_json::to_value(s).unwrap(),
        serde_json::to_value(restored).unwrap()
    );
    let mut stale = state.clone();
    stale.request.budget.max_seconds -= 1.;
    assert!(workflow::prepare(stale, &dir.0, &dir.0.join("stale")).is_err());
    assert!(!dir.0.join("stale").exists());
    assert!(workflow::prepare(state, &dir.0, &out).is_err());
}
#[test]
fn explicit_profiles_abstain_and_invalid_splits_are_rejected() {
    use gigpies::automix::ensemble::{
        profile::{Family, Profile, Tone},
        workflow::{self, State},
    };
    let dir = Scratch::new("profile");
    let s = fixture(&dir, 1);
    let mut p = Request::default();
    p.profiles.push(Profile {
        name: "explicit-kick".into(),
        channels: vec![0],
        family: Family::Kick,
        capture: "recorded_track".into(),
        supported_register_hz: None,
        tone: Some(Tone {
            numerator_hz: [0., 100.],
            denominator_hz: [400., 1600.],
            target_db: [-3., 3.],
            control_hz: 60.,
            control_q: 0.7,
            max_cut_db: 3.,
        }),
        allow_static_eq: true,
        max_output_rise_db: 1.,
        max_protected_loss_db: 1.,
        minimum_active_windows: 2,
        activity_floor_dbfs: -60.,
    });
    let f = ensemble::freeze(&s, &dir.0, &p).unwrap();
    let state = State {
        baseline: s.clone(),
        request: p.clone(),
        frozen: f,
    };
    assert_eq!(
        workflow::decisions(&state)[0].applicability,
        "missing_identity"
    );
    p.captures.push(ensemble::CaptureIdentity {
        channel: 0,
        performance: Some("kick".into()),
        capture: Some("recorded_track".into()),
    });
    let f = ensemble::freeze(&s, &dir.0, &p).unwrap();
    let state = State {
        baseline: s.clone(),
        request: p.clone(),
        frozen: f,
    };
    assert_eq!(
        workflow::decisions(&state)[0].applicability,
        "requires_training_validation"
    );
    p.training = vec![[0.005, 0.009]];
    p.held_out = vec![[0., 0.004]];
    assert!(ensemble::freeze(&s, &dir.0, &p).is_err());
    p.training = vec![[0., 0.004]];
    p.held_out = vec![[0.005, 0.02]];
    assert!(ensemble::freeze(&s, &dir.0, &p).is_err());
    use gigpies::automix::config::Role;
    for family in [
        Family::HiHat,
        Family::Piano,
        Family::Keys,
        Family::Synth,
        Family::Strings,
        Family::Winds,
        Family::PrintedEffects,
    ] {
        assert!(family.matches(Role::Other));
        assert!(!family.matches(Role::Kick));
    }
}
#[test]
fn stage_observations_match_analytic_routing_and_linked_processing() {
    let dir = Scratch::new("stages");
    let mut s = fixture(&dir, 1);
    let c = &mut s.channels[0];
    c.hpf_hz = 0.;
    c.eq.clear();
    c.compressor.ratio = 1.;
    c.compressor.makeup_db = 6.;
    c.fader_db = -6.;
    c.pan = -1.;
    let p = Request::default();
    let f = ensemble::freeze(&s, &dir.0, &p).unwrap();
    let m = ensemble::measure::measure(&s, &dir.0, &p, &f).unwrap();
    let w = &m.windows[0];
    assert_eq!(w.frames, 480);
    let stages = &w.stages;
    assert!((stages[0].mean_power - stages[1].mean_power).abs() < 1e-14);
    assert!((stages[1].mean_power - stages[2].mean_power).abs() < 1e-14);
    assert!((stages[3].mean_power / stages[2].mean_power - 10_f64.powf(0.6)).abs() < 1e-10);
    assert!((stages[4].mean_power / stages[0].mean_power - 0.5).abs() < 1e-10);
    assert!((stages[4].mono_power / stages[4].mean_power - 0.5).abs() < 1e-10);
    assert_eq!(m.channels[0].integrated_lufs, None);
    assert_eq!(m.events.iter().map(|e| e.frames).sum::<u64>(), 480);
}
#[test]
fn shared_cli_exposes_analysis_prepare_reset_and_rejects_overwrite() {
    let dir = Scratch::new("cli");
    let s = fixture(&dir, 1);
    std::fs::write(dir.0.join("baseline.json"), serde_json::to_vec(&s).unwrap()).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_gigpies"))
            .current_dir(&dir.0)
            .args(args)
            .status()
            .unwrap()
            .success()
    };
    assert!(run(&["ensemble-request", "request.json"]));
    assert!(run(&[
        "ensemble-analyze",
        "baseline.json",
        ".",
        "analysis",
        "request.json"
    ]));
    assert!(run(&[
        "ensemble-prepare",
        "analysis/state.json",
        ".",
        "candidate"
    ]));
    assert!(run(&["ensemble-reset", "analysis/state.json", "restored"]));
    assert!(!run(&["ensemble-request", "request.json"]));
}

fn band_case(name: &str, clean: bool, heldout_clean: bool) -> (Scratch, config::Session, Request) {
    use gigpies::automix::{
        config::Role,
        ensemble::profile::{Family, Profile, Tone},
    };
    let dir = Scratch::new(name);
    let mut s = fixture(&dir, 3);
    for (i, c) in s.channels.iter_mut().enumerate() {
        c.role = [Role::Kick, Role::BassDi, Role::Snare][i];
        c.hpf_hz = 0.;
        c.eq.clear();
        c.compressor.ratio = 1.;
        c.compressor.makeup_db = 0.;
        c.fader_db = -6.;
        c.pan = 0.;
    }
    for i in 0..3 {
        let mut w = hound::WavWriter::create(
            dir.0.join(format!("{i}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for n in 0..96000 {
            let t = n as f64 / 48000.;
            let ghost = if (t * 4.) as usize % 4 == 3 { 0.25 } else { 1. };
            let env = (-((t % 0.25) * 14.)).exp() * ghost;
            let x = match i {
                0 => {
                    let high = if clean || heldout_clean && t >= 1. {
                        0.0035
                    } else {
                        0.02
                    };
                    env * (0.05 * (TAU * 60. * t).sin() + high * (TAU * 1000. * t).sin())
                }
                1 => {
                    0.02 * ((TAU * 55. * t).sin()
                        + 0.4 * (TAU * 110. * t).sin()
                        + 0.2 * (TAU * 220. * t).sin())
                }
                _ => env * 0.02 * ((TAU * 180. * t).sin() + 0.3 * (TAU * 2500. * t).sin()),
            };
            w.write_sample(x as f32).unwrap();
        }
        w.finalize().unwrap();
    }
    let mut r = Request {
        training: vec![[0., 1.]],
        held_out: vec![[1., 2.]],
        ..Request::default()
    };
    r.captures.push(ensemble::CaptureIdentity {
        channel: 0,
        performance: Some("kick-performance".into()),
        capture: Some("recorded_track".into()),
    });
    r.profiles.push(Profile {
        name: "kick-attack-intent".into(),
        channels: vec![0],
        family: Family::Kick,
        capture: "recorded_track".into(),
        supported_register_hz: Some([50., 70.]),
        tone: Some(Tone {
            numerator_hz: [800., 1600.],
            denominator_hz: [25., 120.],
            target_db: [-24., -12.],
            control_hz: 1000.,
            control_q: 0.7,
            max_cut_db: 6.,
        }),
        allow_static_eq: true,
        max_output_rise_db: 0.5,
        max_protected_loss_db: 3.,
        minimum_active_windows: 2,
        activity_floor_dbfs: -70.,
    });
    (dir, s, r)
}
#[test]
fn training_selects_one_bundle_and_heldout_can_only_veto() {
    use gigpies::automix::ensemble::workflow::{self, State};
    for (name, clean, heldout_clean) in [
        ("fault", false, false),
        ("clean", true, false),
        ("veto", false, true),
    ] {
        let (dir, s, r) = band_case(name, clean, heldout_clean);
        let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
        let state = State {
            baseline: s.clone(),
            request: r,
            frozen,
        };
        workflow::prepare(state.clone(), &dir.0, &dir.0.join("result")).unwrap();
        let report: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
                .unwrap();
        let settings: config::Session = serde_json::from_reader(
            std::fs::File::open(dir.0.join("result/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["held_out_used_for_selection"], false);
        if clean {
            assert_eq!(report["changed"], false);
            assert_eq!(
                serde_json::to_value(&settings).unwrap(),
                serde_json::to_value(&s).unwrap()
            );
        } else if heldout_clean {
            assert_eq!(report["accepted"], false, "{report}");
            assert_eq!(
                serde_json::to_value(&settings).unwrap(),
                serde_json::to_value(&s).unwrap()
            );
        } else {
            assert_eq!(report["accepted"], true, "{report}");
            assert_eq!(report["changed"], true, "{report}");
            assert_eq!(report["finalized_export_checked"], true);
            assert_eq!(settings.channels[0].eq.len(), 1);
            assert!(settings.channels[1].eq.is_empty());
            assert!(settings.channels[2].eq.is_empty());
            ensemble::listening::prepare(
                state.clone(),
                &dir.0,
                &dir.0.join("result"),
                &dir.0.join("listen"),
            )
            .unwrap();
            let review: serde_json::Value = serde_json::from_reader(
                std::fs::File::open(dir.0.join("listen/review.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(review["preference"], serde_json::Value::Null);
            assert_eq!(review["playback"], false);
            assert_eq!(review["gain_changes"], false);
            assert_eq!(review["clips"].as_array().unwrap().len(), 2);
            assert!(
                ensemble::listening::prepare(
                    state.clone(),
                    &dir.0,
                    &dir.0.join("result"),
                    &dir.0.join("listen")
                )
                .is_err()
            );
        }
        assert!(report["actual_dsp_evaluations"].as_u64().unwrap() <= 10);
    }
}

#[test]
fn bass_notes_snare_ghosts_and_training_hierarchy_share_one_bundle() {
    use gigpies::automix::ensemble::{
        profile::{Family, Tone},
        workflow::{self, State},
    };
    let (dir, mut s, mut r) = band_case("bass-snare", true, false);
    s.channels[1].fader_db = -14.;
    let template = r.profiles[0].clone();
    r.profiles.clear();
    for i in 1..3 {
        let mut w = hound::WavWriter::create(
            dir.0.join(format!("{i}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for n in 0..96000 {
            let t = n as f64 / 48000.;
            let env =
                (-((t % 0.25) * 14.)).exp() * if (t * 4.) as usize % 4 == 3 { 0.25 } else { 1. };
            let x = if i == 1 {
                0.04 * (TAU * 55. * t).sin()
                    + 0.02 * (TAU * 110. * t).sin()
                    + 0.02 * (TAU * 440. * t).sin()
            } else {
                env * (0.05 * (TAU * 180. * t).sin() + 0.02 * (TAU * 30. * t).sin())
            };
            w.write_sample(x as f32).unwrap();
        }
        w.finalize().unwrap();
        r.captures.push(ensemble::CaptureIdentity {
            channel: i,
            performance: Some(format!("performance-{i}")),
            capture: Some("recorded_track".into()),
        });
        let mut p = template.clone();
        p.channels = vec![i];
        p.name = format!("profile-{i}");
        if i == 1 {
            p.family = Family::Bass;
            p.supported_register_hz = Some([50., 60.]);
            p.tone = Some(Tone {
                numerator_hz: [0., 100.],
                denominator_hz: [400., 1600.],
                target_db: [0., 4.],
                control_hz: 55.,
                control_q: 0.7,
                max_cut_db: 3.,
            });
        } else {
            p.family = Family::Snare;
            p.supported_register_hz = Some([150., 220.]);
            p.tone = Some(Tone {
                numerator_hz: [0., 100.],
                denominator_hz: [120., 350.],
                target_db: [-24., -12.],
                control_hz: 30.,
                control_q: 0.7,
                max_cut_db: 6.,
            });
        }
        r.profiles.push(p);
    }
    r.hierarchy = Some(gigpies::automix::hierarchy::Policy::default());
    let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
    let state = State {
        baseline: s.clone(),
        request: r,
        frozen,
    };
    workflow::prepare(state, &dir.0, &dir.0.join("result")).unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["accepted"], true, "{report}");
    assert_eq!(report["selected"].as_array().unwrap().len(), 2, "{report}");
    assert_eq!(
        report["hierarchy"]["whole_source_targets_met"], true,
        "{report}"
    );
    assert!(report["hierarchy"]["training_proposal"].is_object());
    assert!(report["hierarchy"]["level_only"].is_object());
    assert_eq!(report["finalized_export_checked"], true);
}

#[test]
fn guitar_fades_and_vocal_quiet_phrases_have_protected_static_trials() {
    use gigpies::automix::{
        config::Role,
        ensemble::{
            profile::{Family, Profile, Tone},
            workflow::{self, State},
        },
    };
    for clean in [false, true] {
        let dir = Scratch::new(if clean {
            "phrases-clean"
        } else {
            "phrases-fault"
        });
        let mut s = fixture(&dir, 2);
        let mut r = Request {
            training: vec![[0., 1.]],
            held_out: vec![[1., 2.]],
            ..Request::default()
        };
        for (i, c) in s.channels.iter_mut().enumerate() {
            c.role = if i == 0 {
                Role::RhythmGuitar
            } else {
                Role::LeadVocal
            };
            c.hpf_hz = 0.;
            c.eq.clear();
            c.compressor.ratio = 1.;
            c.compressor.makeup_db = 0.;
            c.fader_db = -6.;
            c.pan = if i == 0 { -0.4 } else { 0.4 };
            let mut w = hound::WavWriter::create(
                dir.0.join(format!("{i}.wav")),
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 48000,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .unwrap();
            for n in 0..96000 {
                let t = n as f64 / 48000.;
                let phase = t % 0.5;
                let quiet = if ((t * 2.) as usize).is_multiple_of(2) {
                    1.
                } else {
                    0.25
                };
                let env = (-phase * 5.).exp() * quiet;
                let hz = if i == 0 { 110. } else { 220. };
                let consonant = if i == 1 && phase < 0.025 {
                    0.01 * (TAU * 5000. * t).sin()
                } else {
                    0.
                };
                let low = if clean { 0.0035 } else { 0.02 };
                let x = env
                    * (0.05 * (TAU * hz * t).sin()
                        + 0.015 * (TAU * hz * 2. * t).sin()
                        + 0.01 * (TAU * hz * 4. * t).sin()
                        + low * (TAU * 40. * t).sin()
                        + consonant);
                w.write_sample(x as f32).unwrap();
            }
            w.finalize().unwrap();
            r.captures.push(ensemble::CaptureIdentity {
                channel: i,
                performance: Some(format!("phrase-source-{i}")),
                capture: Some("recorded_track".into()),
            });
            r.profiles.push(Profile {
                name: format!("phrase-profile-{i}"),
                channels: vec![i],
                family: if i == 0 {
                    Family::ElectricGuitar
                } else {
                    Family::LeadVocal
                },
                capture: "recorded_track".into(),
                supported_register_hz: Some([100., 500.]),
                tone: Some(Tone {
                    numerator_hz: [0., 100.],
                    denominator_hz: [100., 1000.],
                    target_db: [-24., -12.],
                    control_hz: 40.,
                    control_q: 0.7,
                    max_cut_db: 6.,
                }),
                allow_static_eq: true,
                max_output_rise_db: 0.5,
                max_protected_loss_db: 3.,
                minimum_active_windows: 2,
                activity_floor_dbfs: -70.,
            });
        }
        let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
        workflow::prepare(
            State {
                baseline: s.clone(),
                request: r,
                frozen,
            },
            &dir.0,
            &dir.0.join("result"),
        )
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
                .unwrap();
        assert_eq!(report["accepted"], true, "{report}");
        assert_eq!(report["changed"], !clean, "{report}");
        if !clean {
            assert_eq!(report["selected"].as_array().unwrap().len(), 2, "{report}");
        }
    }
}
#[test]
fn simultaneous_signed_interaction_exposes_cancellation() {
    let dir = Scratch::new("competition");
    let mut s = fixture(&dir, 2);
    for (i, c) in s.channels.iter_mut().enumerate() {
        c.hpf_hz = 0.;
        c.eq.clear();
        c.compressor.ratio = 1.;
        c.compressor.makeup_db = 0.;
        c.fader_db = 0.;
        c.pan = 0.;
        let mut w = hound::WavWriter::create(
            dir.0.join(format!("{i}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for n in 0..16384 {
            w.write_sample(
                (0.1 * (TAU * 997. * n as f64 / 48000.).sin() * if i == 0 { 1. } else { -1. })
                    as f32,
            )
            .unwrap();
        }
        w.finalize().unwrap();
    }
    let r = Request::default();
    let f = ensemble::freeze(&s, &dir.0, &r).unwrap();
    let m = ensemble::measure::measure(&s, &dir.0, &r, &f).unwrap();
    let w = &m.windows[0];
    assert!((w.competition.overlap_fraction.unwrap() - 1.).abs() < 1e-12);
    assert!((2. * w.stages[4].mean_power + w.competition.signed_cross_power).abs() < 1e-12);
}

#[test]
fn grouped_capture_paths_abstain_without_independent_eq_moves() {
    use gigpies::automix::{
        config::Role,
        ensemble::{
            profile::Family,
            workflow::{self, State},
        },
    };
    let (dir, mut s, mut r) = band_case("group-abstain", false, false);
    s.channels[0].role = Role::RhythmGuitar;
    s.channels[2].role = Role::RhythmGuitar;
    r.profiles[0].family = Family::ElectricGuitar;
    r.captures.push(ensemble::CaptureIdentity {
        channel: 2,
        performance: Some("kick-performance".into()),
        capture: Some("recorded_track".into()),
    });
    let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
    workflow::prepare(
        State {
            baseline: s.clone(),
            request: r,
            frozen,
        },
        &dir.0,
        &dir.0.join("result"),
    )
    .unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["changed"], false);
    assert_eq!(report["all_targets_met"], false);
    assert_eq!(report["selected"], serde_json::json!([]));
    assert!(
        report["abstentions"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("coherent group")
    );
    let after: config::Session =
        serde_json::from_reader(std::fs::File::open(dir.0.join("result/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        serde_json::to_value(s).unwrap(),
        serde_json::to_value(after).unwrap()
    );
}

#[test]
fn remaining_families_require_identity_and_preserve_quiet_decay() {
    remaining_family_matrix(false);
}
#[test]
#[ignore = "Full E4 register/envelope matrix; recorded research evidence, run explicitly when changing family support"]
fn all_remaining_family_register_envelope_matrix() {
    remaining_family_matrix(true);
}
fn remaining_family_matrix(all: bool) {
    use gigpies::automix::{
        config::Role,
        ensemble::{
            profile::Family,
            workflow::{self, State},
        },
    };
    for (family, role) in [
        (Family::Tom, Role::Tom),
        (Family::Overheads, Role::Overheads),
        (Family::DrumRoom, Role::DrumRoom),
        (Family::VocalRoom, Role::VocalRoom),
        (Family::AcousticGuitar, Role::AcousticGuitar),
        (Family::BackingVocal, Role::BackingVocal),
        (Family::HiHat, Role::Other),
        (Family::Piano, Role::Other),
        (Family::Keys, Role::Other),
        (Family::Synth, Role::Other),
        (Family::Strings, Role::Other),
        (Family::Winds, Role::Other),
    ] {
        if !all
            && !matches!(
                family,
                Family::Tom | Family::AcousticGuitar | Family::BackingVocal | Family::Overheads
            )
        {
            continue;
        }
        for clean in [false, true] {
            let (dir, mut s, mut r) = band_case("families", clean, false);
            s.channels.truncate(1);
            s.channels[0].role = role;
            // Different body/register and envelope cases, not role-name aliases.
            let (body, decay, overtone) = match family {
                Family::Tom => (180., 12., 360.),
                Family::Overheads | Family::HiHat => (3500., 7., 7000.),
                Family::DrumRoom | Family::VocalRoom => (250., 2., 500.),
                Family::AcousticGuitar => (196., 5., 392.),
                Family::BackingVocal => (330., 3., 660.),
                Family::Piano => (220., 8., 440.),
                Family::Keys => (440., 1., 880.),
                Family::Synth => (220., 0.5, 440.),
                Family::Strings => (294., 1.5, 588.),
                Family::Winds => (392., 2.5, 784.),
                _ => unreachable!(),
            };
            let mut wav = hound::WavWriter::create(
                dir.0.join("0.wav"),
                hound::WavSpec {
                    channels: 2,
                    sample_rate: 48000,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .unwrap();
            for n in 0..96000 {
                let t = n as f64 / 48000.;
                let envelope = (-(t % 0.25) * decay).exp()
                    * if (t * 4.) as usize % 4 == 3 { 0.25 } else { 1. };
                let signal = envelope
                    * (0.05 * (TAU * body * t).sin()
                        + 0.01 * (TAU * overtone * t).sin()
                        + (if clean { 0.0035 } else { 0.02 }) * (TAU * 40. * t).sin());
                wav.write_sample(signal as f32).unwrap();
                wav.write_sample((signal * 0.7) as f32).unwrap();
            }
            wav.finalize().unwrap();
            r.profiles[0].supported_register_hz = Some([body * 0.9, body * 1.1]);
            let tone = r.profiles[0].tone.as_mut().unwrap();
            tone.numerator_hz = [0., 100.];
            tone.denominator_hz = [body * 0.9, overtone * 1.1];
            tone.control_hz = 40.;
            r.profiles[0].family = family;
            r.profiles[0].capture = "isolated_ensemble_capture".into();
            r.captures[0].capture = Some("isolated_ensemble_capture".into());
            let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
            let state = State {
                baseline: s,
                request: r,
                frozen,
            };
            assert_eq!(
                workflow::decisions(&state)[0].applicability,
                "requires_training_validation"
            );
            workflow::prepare(state, &dir.0, &dir.0.join("result")).unwrap();
            let report: serde_json::Value = serde_json::from_reader(
                std::fs::File::open(dir.0.join("result/report.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(report["accepted"], true, "{family:?}: {report}");
            assert_eq!(report["changed"], !clean, "{family:?}: {report}");
        }
    }
}

#[test]
fn mixed_microphone_ambiguity_and_printed_processing_abstain() {
    use gigpies::automix::{
        config::Role,
        ensemble::{
            profile::Family,
            workflow::{self, State},
        },
    };
    for (family, role) in [
        (Family::Overheads, Role::Overheads),
        (Family::DrumRoom, Role::DrumRoom),
        (Family::VocalRoom, Role::VocalRoom),
        (Family::PrintedEffects, Role::Other),
    ] {
        let (dir, mut s, mut r) = band_case("ambiguity", false, false);
        s.channels[0].role = role;
        r.profiles[0].family = family;
        let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
        workflow::prepare(
            State {
                baseline: s,
                request: r,
                frozen,
            },
            &dir.0,
            &dir.0.join("result"),
        )
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
                .unwrap();
        assert_eq!(report["changed"], false);
        assert_eq!(report["all_targets_met"], false);
        assert!(!report["abstentions"].as_array().unwrap().is_empty());
    }
}

#[test]
fn coherent_group_is_corrected_together_and_cancellation_abstains() {
    use gigpies::automix::{
        config::Role,
        ensemble::workflow::{self, State},
    };
    for cancel in [false, true] {
        let (dir, mut s, mut r) = band_case("coherent", false, false);
        s.channels[2].role = Role::Kick;
        if !cancel {
            std::fs::copy(dir.0.join("0.wav"), dir.0.join("2.wav")).unwrap();
        } else {
            let mut reader = hound::WavReader::open(dir.0.join("0.wav")).unwrap();
            let mut writer = hound::WavWriter::create(dir.0.join("2.wav"), reader.spec()).unwrap();
            for sample in reader.samples::<f32>() {
                writer.write_sample(-sample.unwrap()).unwrap();
            }
            writer.finalize().unwrap();
        }
        r.profiles[0].channels = vec![0, 2];
        r.captures.push(ensemble::CaptureIdentity {
            channel: 2,
            performance: Some("kick-performance".into()),
            capture: Some("recorded_track".into()),
        });
        let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
        workflow::prepare(
            State {
                baseline: s.clone(),
                request: r,
                frozen,
            },
            &dir.0,
            &dir.0.join("result"),
        )
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
                .unwrap();
        let settings: config::Session = serde_json::from_reader(
            std::fs::File::open(dir.0.join("result/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["changed"], !cancel, "{report}");
        assert_eq!(
            serde_json::to_value(&settings.channels[0].eq).unwrap(),
            serde_json::to_value(&settings.channels[2].eq).unwrap()
        );
        if cancel {
            assert_eq!(report["all_targets_met"], false);
        }
    }
}

#[test]
fn fixed_fx_is_verified_and_shared_returns_refuse_hierarchy_attribution() {
    use gigpies::automix::{
        effects,
        ensemble::workflow::{self, State},
    };
    let (dir, mut s, mut r) = band_case("fixed-fx", false, false);
    effects::add_pass(&mut s).unwrap();
    let fx = s.effects.as_mut().unwrap();
    fx.exciter_amount = 0.;
    fx.tail_seconds = 0.2;
    for bus in &mut fx.buses {
        bus.return_db = -40.;
    }
    let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
    workflow::prepare(
        State {
            baseline: s.clone(),
            request: r.clone(),
            frozen,
        },
        &dir.0,
        &dir.0.join("result"),
    )
    .unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
            .unwrap();
    // Abruptly truncated input creates a new EQ release tail: the final
    // composition guard must veto it even though channel tone checks passed.
    assert_eq!(report["accepted"], false, "{report}");
    assert_eq!(report["composition"]["fixed_fx_unchanged"], true);
    assert_eq!(report["finalized_export_checked"], false);
    for i in 0..3 {
        let path = dir.0.join(format!("{i}.wav"));
        let mut reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        let mut samples = reader
            .samples::<f32>()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let len = samples.len();
        for (n, x) in samples.iter_mut().enumerate().skip(len - 4800) {
            *x *= ((len - 1 - n) as f32 / 4800.).powi(2);
        }
        drop(reader);
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for x in samples {
            writer.write_sample(x).unwrap();
        }
        writer.finalize().unwrap();
    }
    let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
    workflow::prepare(
        State {
            baseline: s.clone(),
            request: r.clone(),
            frozen,
        },
        &dir.0,
        &dir.0.join("smooth"),
    )
    .unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(dir.0.join("smooth/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["accepted"], true, "{report}");
    assert_eq!(report["finalized_export_checked"], true);
    for i in 1..3 {
        r.captures.push(ensemble::CaptureIdentity {
            channel: i,
            performance: Some(format!("source-{i}")),
            capture: Some("recorded_track".into()),
        });
    }
    let bus = s.effects.as_mut().unwrap().buses.first_mut().unwrap();
    bus.sends = vec![
        effects::Send {
            channel: 0,
            db: -10.,
        },
        effects::Send {
            channel: 1,
            db: -10.,
        },
    ];
    assert!(
        ensemble::group_levels::measure(&s, &dir.0, &r, true, 48000)
            .unwrap_err()
            .to_string()
            .contains("shared")
    );
}

#[test]
fn reviewed_dynamic_events_beat_static_cuts_without_quiet_phrase_loss() {
    use gigpies::automix::{
        config::Role,
        dynamic_eq,
        ensemble::{
            dynamic::{Evidence, Trial},
            profile::{Family, Profile, Tone},
            workflow::{self, State},
        },
    };
    for (hz, family, role, evidence, capture) in [
        (
            6000.,
            Family::LeadVocal,
            Role::LeadVocal,
            Evidence::IsolatedSibilance,
            "isolated_vocal",
        ),
        (
            60.,
            Family::Kick,
            Role::Kick,
            Evidence::LowLoad,
            "recorded_track",
        ),
    ] {
        let dir = Scratch::new("dynamic-events");
        let mut s = fixture(&dir, 1);
        let c = &mut s.channels[0];
        c.role = role;
        c.hpf_hz = 0.;
        c.eq.clear();
        c.compressor.ratio = 1.;
        c.compressor.makeup_db = 0.;
        c.fader_db = -6.;
        let mut w = hound::WavWriter::create(
            dir.0.join("0.wav"),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for n in 0..192000 {
            let t = n as f64 / 48000.;
            let phase = t % 2.;
            let amp = if phase < 0.7 {
                1.
            } else if phase < 0.9 {
                0.01 + 0.99 * 0.5 * (1. + (std::f64::consts::PI * (phase - 0.7) / 0.2).cos())
            } else {
                0.01
            };
            let x = amp * (0.05 * (TAU * 220. * t).sin() + 0.02 * (TAU * hz * t).sin());
            w.write_sample(x as f32).unwrap();
            w.write_sample((0.7 * x) as f32).unwrap();
        }
        w.finalize().unwrap();
        let mut r = Request {
            training: vec![[0., 2.]],
            held_out: vec![[2., 4.]],
            ..Default::default()
        };
        r.captures.push(ensemble::CaptureIdentity {
            channel: 0,
            performance: Some("isolated-performance".into()),
            capture: Some(capture.into()),
        });
        r.profiles.push(Profile {
            name: "reviewed-events".into(),
            channels: vec![0],
            family,
            capture: capture.into(),
            supported_register_hz: Some([180., 260.]),
            tone: Some(Tone {
                numerator_hz: if hz > 1000. {
                    [4000., 8000.]
                } else {
                    [25., 100.]
                },
                denominator_hz: [180., 350.],
                target_db: [-30., -12.],
                control_hz: hz,
                control_q: 1.,
                max_cut_db: 6.,
            }),
            allow_static_eq: true,
            max_output_rise_db: 1.,
            max_protected_loss_db: 3.,
            minimum_active_windows: 1,
            activity_floor_dbfs: -90.,
        });
        r.dynamic_trials.push(Trial {
            profile: "reviewed-events".into(),
            evidence,
            processor: dynamic_eq::Config {
                version: 1,
                hz,
                q: 1.,
                depth_db: 6.,
                threshold_dbfs: -45.,
                attack_ms: 2.,
                release_ms: 40.,
            },
            problem_spans: vec![[0., 0.8], [2., 2.8]],
            protected_spans: vec![[1.3, 2.], [3.3, 4.]],
            max_protected_loss_db: 0.1,
        });
        let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
        workflow::prepare(
            State {
                baseline: s,
                request: r,
                frozen,
            },
            &dir.0,
            &dir.0.join("result"),
        )
        .unwrap();
        let report: serde_json::Value =
            serde_json::from_reader(std::fs::File::open(dir.0.join("result/report.json")).unwrap())
                .unwrap();
        assert_eq!(report["accepted"], true, "{report}");
        assert!(report["selected"][0]["dynamic_eq"].is_object(), "{report}");
        assert_eq!(report["finalized_export_checked"], true);
    }
}

#[test]
fn explicit_group_hierarchy_uses_coherent_lufs_and_reports_section_conflicts() {
    use gigpies::automix::{config::Role, ensemble::group_levels};
    let (dir, mut s, mut r) = band_case("group-levels", true, false);
    s.channels[2].role = Role::Kick;
    std::fs::copy(dir.0.join("0.wav"), dir.0.join("2.wav")).unwrap();
    r.captures.push(ensemble::CaptureIdentity {
        channel: 1,
        performance: Some("bass".into()),
        capture: Some("recorded_track".into()),
    });
    r.captures.push(ensemble::CaptureIdentity {
        channel: 2,
        performance: Some("kick-performance".into()),
        capture: Some("recorded_track".into()),
    });
    let mut reader = hound::WavReader::open(dir.0.join("1.wav")).unwrap();
    let spec = reader.spec();
    let samples = reader
        .samples::<f32>()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    drop(reader);
    let mut writer = hound::WavWriter::create(dir.0.join("1.wav"), spec).unwrap();
    for (i, x) in samples.into_iter().enumerate() {
        writer
            .write_sample(x * if i >= 48000 { 4. } else { 1. })
            .unwrap();
    }
    writer.finalize().unwrap();
    assert!(group_levels::measure(&s, &dir.0, &r, false, 96000).is_err());
    let m = group_levels::measure(&s, &dir.0, &r, true, 96000).unwrap();
    assert_eq!(m.members, vec![vec![0, 2], vec![1]]);
    let policy = gigpies::automix::hierarchy::Policy {
        max_fader_change_db: 12.,
        ..Default::default()
    };
    let deltas = group_levels::propose(&s, &m, &policy).unwrap();
    assert_eq!(deltas[0], deltas[2]);
    for (c, d) in s.channels.iter_mut().zip(deltas) {
        c.fader_db += d;
    }
    let m = group_levels::measure(&s, &dir.0, &r, true, 96000).unwrap();
    let result = group_levels::compliance(&s, &m, &policy).unwrap();
    assert_eq!(result["whole_source_targets_met"], true, "{result}");
    assert_eq!(result["all_sections_targets_met"], false, "{result}");
    assert!(
        result["sections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["lufs_targets_met"] == false)
    );
    assert!(m.section_lufs.iter().flatten().all(Option::is_some));
    let mut missing = m;
    missing.section_lufs[0][0] = None;
    let result = group_levels::compliance(&s, &missing, &policy).unwrap();
    assert_eq!(result["all_sections_targets_met"], false);
    assert!(result["sections"][0]["lufs_targets_met"].is_null());

    // Repeat the same changing arrangement into a later holdout. Its aggregate
    // hierarchy is attainable, but neither averaging nor technical acceptance
    // may hide the individual section conflicts in the preparation report.
    for i in 0..3 {
        let path = dir.0.join(format!("{i}.wav"));
        let mut reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        let samples = reader
            .samples::<f32>()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        drop(reader);
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &x in samples.iter().chain(&samples) {
            writer.write_sample(x).unwrap();
        }
        writer.finalize().unwrap();
    }
    r.training = vec![[0., 1.], [1., 2.]];
    r.held_out = vec![[2., 3.], [3., 4.]];
    r.profiles.clear();
    r.grouped_hierarchy = true;
    r.hierarchy = Some(policy);
    let frozen = ensemble::freeze(&s, &dir.0, &r).unwrap();
    let state = ensemble::workflow::State {
        baseline: s,
        request: r,
        frozen,
    };
    let out = dir.0.join("section-conflict");
    ensemble::workflow::prepare(state, &dir.0, &out).unwrap();
    let report: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["accepted"], true, "{report}");
    assert_eq!(
        report["hierarchy"]["whole_source_targets_met"], true,
        "{report}"
    );
    assert_eq!(report["all_targets_met"], false, "{report}");
}

#[test]
fn hierarchy_training_excludes_undeclared_prefix_and_passage_gaps() {
    use gigpies::automix::ensemble::group_levels;
    let (dir, s, mut r) = band_case("training-passages", false, false);
    for channel in 1..s.channels.len() {
        r.captures.push(ensemble::CaptureIdentity {
            channel,
            performance: Some(format!("independent-{channel}")),
            capture: Some("recorded_track".into()),
        });
    }
    r.training = vec![[1., 1.5]];
    r.held_out = vec![[1.5, 2.]];
    let m = group_levels::measure_training(&s, &dir.0, &r, false, 72000).unwrap();
    assert_eq!(m.direct_lufs, m.section_lufs[0]);
    assert_eq!(m.direct_and_dedicated_returns_lufs, m.section_lufs[0]);
    let mut first = Loudness::new(48000);
    let mut second = Loudness::new(48000);
    for i in 0..48000 {
        let x = [0.1 * (TAU * 1000. * i as f64 / 48000.).sin(); 2];
        first.add_selected(x, i < 9600);
        second.add_selected(x, (24000..33600).contains(&i));
    }
    // Two 200 ms passages cannot manufacture one 400 ms loudness block.
    assert_eq!(Loudness::integrated_parts(&[&first, &second]), None);
}

#[test]
fn duplicate_profile_names_cannot_alias_group_or_dynamic_evidence() {
    use gigpies::automix::ensemble::profile::Family;
    let (dir, s, mut r) = band_case("duplicate-names", false, false);
    let mut second = r.profiles[0].clone();
    second.channels = vec![2];
    second.family = Family::Snare;
    r.profiles.push(second);
    assert!(
        ensemble::freeze(&s, &dir.0, &r)
            .unwrap_err()
            .to_string()
            .contains("duplicate ensemble profile")
    );
}

#[test]
fn composition_observation_uses_the_export_limiter_policy_exactly() {
    use gigpies::automix::{self, delivery};
    let dir = Scratch::new("policy-observation");
    let mut s = fixture(&dir, 1);
    s.channels[0].hpf_hz = 0.;
    s.channels[0].eq.clear();
    s.channels[0].compressor.ratio = 1.;
    s.channels[0].compressor.makeup_db = 0.;
    s.channels[0].fader_db = 0.;
    s.master_db = 12.;
    s.master_hpf_hz = 0.;
    s.ceiling_db = -24.;
    s.output_mode = config::OutputMode::Matched;
    let policy = delivery::Policy {
        delivery_gain: delivery::DeliveryGain::IndependentPeak { max_boost_db: 0. },
        ..Default::default()
    };
    automix::observe_policy(
        s.clone(),
        &dir.0,
        &dir.0.join("observed"),
        0.01,
        policy.clone(),
    )
    .unwrap();
    automix::run_policy(s.clone(), &dir.0, &dir.0.join("exported"), policy).unwrap();
    automix::observe(s, &dir.0, &dir.0.join("legacy"), 0.01).unwrap();
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_reader(std::fs::File::open(dir.0.join(name).join("stages.json")).unwrap())
            .unwrap()
    };
    assert_eq!(read("observed")["stages"], read("exported")["stages"]);
    assert_ne!(read("legacy")["stages"][3], read("observed")["stages"][3]);
    assert!(!dir.0.join("observed/processed.wav").exists());
}

#[test]
fn measurement_only_groups_allow_mixed_families_and_preserve_coherent_power() {
    use ensemble::ObservationGroup;
    let dir = Scratch::new("observation-partition");
    let mut s = fixture(&dir, 3);
    s.channels[0].role = config::Role::LeadVocal;
    s.channels[1].role = config::Role::BassDi;
    s.channels[2].role = config::Role::RhythmGuitar;
    // Identical strips/waveforms but distinct musical identities.
    let mut r = Request::default();
    let legacy = serde_json::to_value(&r).unwrap();
    assert!(legacy.get("observation_groups").is_none());
    r.observation_groups.push(ObservationGroup {
        name: "accompaniment".into(),
        channels: vec![1, 2],
    });
    let f = ensemble::freeze(&s, &dir.0, &r).unwrap();
    let m = ensemble::measure::measure(&s, &dir.0, &r, &f).unwrap();
    assert_eq!(m.observation_groups, vec![("accompaniment".into(), 3)]);
    assert!(m.profile_groups.is_empty());
    let group = m.windows.iter().find(|w| w.channel == 3).unwrap();
    let one = m.windows.iter().find(|w| w.channel == 1).unwrap();
    let lead = m.windows.iter().find(|w| w.channel == 0).unwrap();
    for stage in 0..5 {
        assert!((group.stages[stage].mean_power - 4. * one.stages[stage].mean_power).abs() < 1e-12);
    }
    for (rest, voice) in group
        .competition
        .rest_band_power
        .iter()
        .zip(&lead.stages[4].band_power)
    {
        assert!((rest.unwrap() - voice.unwrap()).abs() < 1e-12);
    }
    let original = r.clone();
    r.observation_groups[0].channels.push(1);
    assert!(ensemble::freeze(&s, &dir.0, &r).is_err());
    r = original.clone();
    r.observation_groups[0].channels.push(3);
    assert!(ensemble::freeze(&s, &dir.0, &r).is_err());
    r = original.clone();
    r.observation_groups.push(ObservationGroup {
        name: "overlap".into(),
        channels: vec![0, 1],
    });
    assert!(ensemble::freeze(&s, &dir.0, &r).is_err());
    r = original;
    r.budget.max_feature_records = 7 * 3; // channels alone fit; extra group must count.
    assert!(ensemble::freeze(&s, &dir.0, &r).is_err());
}

#[test]
fn vocal_decision_cli_consumes_production_measurement_groups() {
    vocal_production_case(false, false, 0);
}

#[test]
fn vocal_fx_decision_cli_replays_excitation_shared_returns_and_tails() {
    vocal_production_case(true, false, 0);
}

#[test]
fn vocal_isolated_activity_replays_production_fx_without_reviewed_spans() {
    vocal_production_case(true, true, 0);
}

#[test]
fn vocal_spectral_contrast_replays_production_fx_with_frozen_templates() {
    vocal_production_case(true, false, 1);
}

#[test]
fn vocal_spectral_states_replays_production_fx_with_multiple_bleed_states() {
    vocal_production_case(true, false, 2);
}

#[test]
fn vocal_spectral_states_refuses_direct_spectrum_reproduced_by_bleed_mixture() {
    vocal_production_case(true, false, 3);
}

#[test]
fn vocal_direct_state_protection_replays_two_supported_timbres_with_fx() {
    vocal_production_case(true, false, 4);
}

fn vocal_production_case(with_fx: bool, isolated: bool, contrast_mode: u8) {
    use ensemble::ObservationGroup;
    let window = ensemble::measure::FFT_FRAMES;
    let states = contrast_mode >= 2;
    let mixture = contrast_mode == 3;
    let direct_states = contrast_mode == 4;
    let contrast = contrast_mode != 0;
    let calibration_windows = if direct_states {
        20
    } else if states {
        15
    } else {
        10
    };
    let direct_windows = if direct_states { 10 } else { 5 };
    let split_windows = calibration_windows + 5;
    let source_frames = if contrast {
        (2 * calibration_windows + 6) * window
    } else {
        192000
    };
    let split = if contrast {
        split_windows * window
    } else {
        98304
    };
    let dir = Scratch::new("vocal-cli-production");
    let mut s = fixture(&dir, 4);
    for (channel, c) in s.channels.iter().enumerate() {
        let mut w = hound::WavWriter::create(
            dir.0.join(&c.file),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        let mut noise = 42_u64;
        for frame in 0..source_frames {
            noise = noise.wrapping_mul(6364136223846793005).wrapping_add(1);
            let sample = if contrast
                && channel == 0
                && ((direct_windows * window..calibration_windows * window).contains(&frame)
                    || (split..(split_windows + calibration_windows - direct_windows) * window)
                        .contains(&frame))
            {
                // Distinct synthetic bleed class; no claim that a tone is real bleed.
                let second = states
                    && (((direct_windows + 5) * window..calibration_windows * window)
                        .contains(&frame)
                        || ((split_windows + 5) * window..(split_windows + 10) * window)
                            .contains(&frame));
                let hz = if second { 700. } else { 250. };
                (0.01 * (std::f64::consts::TAU * hz * frame as f64 / 48000.).sin()) as f32
            } else if mixture && channel == 0 {
                // The sum has the same raw spectrum whether called direct or
                // two bleed sources. Pairwise separation cannot establish origin.
                let phase = std::f64::consts::TAU * frame as f64 / 48000.;
                (0.01 * ((250. * phase).sin() + (700. * phase).sin()) / std::f64::consts::SQRT_2)
                    as f32
            } else if isolated && channel == 0 && frame % 98304 < 16384 {
                0. // Solo backing entries in both splits; FX history stays continuous.
            } else {
                let broadband = ((noise >> 32) as u32 as f64 / u32::MAX as f64 - 0.5) * 0.1;
                // A second reviewed raw shape, with energy outside the objective
                // ranges. Both still go through the complete production DSP/FX.
                let second_direct = direct_states
                    && channel == 0
                    && ((5 * window..10 * window).contains(&frame)
                        || (22 * window..25 * window).contains(&frame)
                        || (40 * window..45 * window).contains(&frame));
                (broadband
                    + if second_direct {
                        0.02 * (std::f64::consts::TAU * 8000. * frame as f64 / 48000.).sin()
                    } else {
                        0.
                    }) as f32
            };
            w.write_sample(sample).unwrap();
        }
        w.finalize().unwrap();
    }
    s.channels[0].role = config::Role::LeadVocal;
    s.channels[1].role = config::Role::BackingVocal;
    s.channels[2].role = config::Role::BassDi;
    s.channels[3].role = config::Role::RhythmGuitar;
    for (c, gain) in s
        .channels
        .iter_mut()
        .zip([-8., -3., -6.020599913, -6.020599913])
    {
        c.fader_db = gain;
    }
    // Identical nonlinear upstream compression is replayed with continuous history.
    for c in &mut s.channels {
        c.compressor.threshold_db = -40.;
        c.compressor.ratio = 4.;
    }
    let mut r = Request::default();
    r.observation_groups.push(ObservationGroup {
        name: "band".into(),
        channels: vec![2, 3],
    });
    if with_fx {
        gigpies::automix::effects::add_pass(&mut s).unwrap();
        let fx = s.effects.as_mut().unwrap();
        // One shared bus per distinct production engine; duplicate presets add
        // no coverage to this normal-suite regression.
        fx.buses.retain(|b| {
            matches!(
                b.name.as_str(),
                "vocal_hall" | "vocal_chorus" | "vocal_delay"
            )
        });
        // Shared buses: all four sources feed each existing production engine.
        for bus in &mut fx.buses {
            bus.sends = (0..4)
                .map(|channel| gigpies::automix::effects::Send { channel, db: -18. })
                .collect();
            if let gigpies::automix::effects::Effect::Reverb(ref mut rv) = bus.effect {
                rv.predelay_ms = 40.;
            }
        }
        r.vocal_fx_attribution = true;
    }
    let analysis = dir.0.join("analysis");
    ensemble::workflow::analyze(s, &dir.0, &analysis, r).unwrap();
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(analysis.join("state.json")).unwrap()).unwrap();
    let mut capture = serde_json::json!({
        "frozen_id": state["frozen"]["id"], "training_end_frame": split,
        "provenance": ["inherited", "inherited", "preset", "preset"],
        "groups": {
            "lead": {"channels":[0],"observation":0,"performances":[{"singer":"lead", "channels":[0], "capture":"direct", "confidence":0.9,"spans":[[0,source_frames]]}]},
            "backing": {"channels":[1],"observation":1,"performances":[{"singer":"back", "channels":[1], "capture":"direct", "confidence":0.9,"spans":[[0,source_frames]]}]},
            "accompaniment": {"channels":[2,3],"observation":"band"}
        }
    });
    if isolated {
        for name in ["lead", "backing"] {
            let p = capture["groups"][name]["performances"][0]
                .as_object_mut()
                .unwrap();
            p.remove("spans");
            p.insert("activity".into(), serde_json::json!({
                "mode": "isolated_voice", "attestation": "voice_only_no_bleed",
                "review_basis": "Controlled synthetic isolated input; energy eligibility, not voice classification"
            }));
        }
    }
    if contrast {
        let p = capture["groups"]["lead"]["performances"][0]
            .as_object_mut()
            .unwrap();
        p.remove("spans");
        p.insert(
            "activity".into(),
            serde_json::json!({
                "mode": "spectral_contrast", "calibration_end_frame": 10 * window,
                "assumption": "stable_capture_spectral_separation",
                "review_basis": "Generated broadband direct class and tonal negative class",
                "examples": [
                    {"span": [0,5 * window], "label": "direct"},
                    {"span": [5 * window,10 * window], "label": "bleed"},
                    {"span": [15 * window,20 * window], "label": "bleed"},
                    {"span": [20 * window,25 * window], "label": "direct"}
                ]
            }),
        );
    }
    if states {
        let direct_state = if mixture { "two-tone" } else { "broadband" };
        capture["groups"]["lead"]["performances"][0]["activity"] = serde_json::json!({
            "mode": "spectral_states", "calibration_end_frame": calibration_windows * window,
            "assumption": "stable_capture_spectral_states",
            "review_basis": if mixture {
                "Generated positive sum of two tonal negative states; indistinguishable-origin refusal mechanics only"
            } else {
                "Generated broadband direct and two distinct tonal negative states; mechanics only"
            },
            "examples": [
                {"span": [0,5 * window], "label": "direct", "state": direct_state},
                {"span": [5 * window,10 * window], "label": "bleed", "state": "low"},
                {"span": [10 * window,15 * window], "label": "bleed", "state": "mid"},
                {"span": [20 * window,25 * window], "label": "bleed", "state": "low"},
                {"span": [25 * window,30 * window], "label": "bleed", "state": "mid"},
                {"span": [30 * window,35 * window], "label": "direct", "state": direct_state}
            ]
        });
    }
    if direct_states {
        capture["groups"]["lead"]["performances"][0]["activity"]["examples"] = serde_json::json!([
            {"span": [0,5 * window], "label": "direct", "state": "broadband"},
            {"span": [5 * window,10 * window], "label": "direct", "state": "upper-tone"},
            {"span": [10 * window,15 * window], "label": "bleed", "state": "low"},
            {"span": [15 * window,20 * window], "label": "bleed", "state": "mid"},
            {"span": [25 * window,30 * window], "label": "bleed", "state": "low"},
            {"span": [30 * window,35 * window], "label": "bleed", "state": "mid"},
            {"span": [35 * window,40 * window], "label": "direct", "state": "broadband"},
            {"span": [40 * window,45 * window], "label": "direct", "state": "upper-tone"}
        ]);
    }
    let path = dir.0.join("capture.json");
    std::fs::write(&path, serde_json::to_vec(&capture).unwrap()).unwrap();
    let output = dir.0.join("candidate");
    let status = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/vocal_balance.py"
        ))
        .arg(&analysis)
        .arg(&path)
        .arg(&output)
        .arg("--source")
        .arg(&dir.0)
        .arg("--analyzer")
        .arg(env!("CARGO_BIN_EXE_gigpies"))
        .status()
        .unwrap();
    assert!(status.success());
    let decision: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("decision.json")).unwrap()).unwrap();
    if mixture {
        assert_eq!(decision["status"], "refused", "{decision}");
        assert_eq!(decision["accepted"], false);
        assert!(decision["reasons"].as_array().unwrap().iter().any(|r| {
            r.as_str()
                .unwrap()
                .contains("bleed-power mixture enclosure")
        }));
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings, state["baseline"]);
        return;
    }
    assert_eq!(decision["status"], "accepted", "{decision}");
    assert!(decision["decisions"][0]["delta_db"].as_f64().unwrap() > 0.);
    assert!(!decision["metrics_resulting"].is_null());
    assert_eq!(
        decision["context_policy"]["version"],
        "vocal-lineup-protection-v1"
    );
    for split in ["training", "held_out"] {
        assert!(
            !decision["metrics_resulting"][split]["context_losses"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }
    assert!(decision["prediction_error_db"].as_f64().unwrap() < 0.01);
    if direct_states {
        assert_eq!(
            decision["state_context_policy"]["version"],
            "vocal-state-protection-v1"
        );
        for split in ["training", "held_out"] {
            let losses = decision["metrics_resulting"][split]["state_losses"]
                .as_object()
                .unwrap();
            assert_eq!(losses.len(), 2);
            for (key, actual) in losses {
                let predicted = &decision["remeasured_prediction"][split]["state_losses"][key];
                assert_eq!(actual["frames"], predicted["frames"]);
                assert!(
                    (actual["loss"].as_f64().unwrap() - predicted["loss"].as_f64().unwrap()).abs()
                        < 1e-6
                );
            }
        }
    }
    if contrast {
        let classifier = &decision["activity_evidence"][0]["spectral_contrast"];
        if states {
            assert_eq!(
                classifier["bleed_mixture_exclusion"]["policy"]["version"],
                "bleed-power-mixture-exclusion-v1"
            );
        }
        assert_eq!(
            classifier["policy"]["version"],
            if states {
                "capture-spectral-states-v1"
            } else {
                "capture-spectral-contrast-v1"
            }
        );
        for coverage in classifier["held_out"].as_object().unwrap().values() {
            assert_eq!(coverage["errors"], 0);
            assert!(coverage["windows"].as_u64().unwrap() >= 5);
        }
        assert_eq!(
            classifier["templates"].as_object().unwrap().len(),
            if direct_states {
                4
            } else if states {
                3
            } else {
                2
            }
        );
        assert!(
            decision["activity_evidence"][0]["counts"]["contrast_direct"]
                .as_u64()
                .unwrap()
                > 5
        );
    }
    if isolated {
        for split in ["training", "held_out"] {
            assert_eq!(
                decision["metrics_resulting"][split]["context_losses"]
                    .as_object()
                    .unwrap()
                    .len(),
                2
            );
        }
        assert_eq!(
            decision["activity_policy"]["version"],
            "vocal-eligibility-v1"
        );
        assert!(
            decision["activity_evidence"][0]["counts"]["isolated_energy"]
                .as_u64()
                .unwrap()
                > 5
        );
        assert!(decision["evidence"]["groups"]["lead"]["performances"][0]["spans"].is_null());
    }
    if with_fx {
        assert_eq!(decision["resulting_attribution"]["tail_complete"], true);
        assert_eq!(
            decision["resulting_attribution"]["checked_frames"],
            source_frames + 192000
        );
        assert_eq!(decision["composition"]["fixed_fx_unchanged"], true);
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["effects"], state["baseline"]["effects"]);
    }
}

#[test]
fn vocal_remeasurement_rejects_unrelated_controls_and_stale_sources() {
    let dir = Scratch::new("vocal-replay-bindings");
    let s = fixture(&dir, 3);
    let request = Request::default();
    let state = ensemble::workflow::State {
        frozen: ensemble::freeze(&s, &dir.0, &request).unwrap(),
        baseline: s.clone(),
        request,
    };
    let out = dir.0.join("replay");
    let mut candidate = s.clone();
    candidate.channels[0].pan = 0.5;
    assert!(ensemble::workflow::remeasure(state.clone(), candidate, &dir.0, &out).is_err());
    assert!(!out.exists());
    let mut bytes = std::fs::read(dir.0.join("0.wav")).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(dir.0.join("0.wav"), bytes).unwrap();
    assert!(ensemble::workflow::remeasure(state, s, &dir.0, &out).is_err());
    assert!(!out.exists());
}

#[test]
fn vocal_fx_attribution_bounds_tail_and_preserves_dry_stages() {
    let dir = Scratch::new("vocal-fx-contract");
    let mut s = fixture(&dir, 3);
    let dry = Request::default();
    let f = ensemble::freeze(&s, &dir.0, &dry).unwrap();
    let a = ensemble::measure::measure(&s, &dir.0, &dry, &f).unwrap();
    let mut request = Request {
        vocal_fx_attribution: true,
        ..Default::default()
    };
    let f = ensemble::freeze(&s, &dir.0, &request).unwrap();
    let b = ensemble::measure::measure(&s, &dir.0, &request, &f).unwrap();
    assert_eq!(
        serde_json::to_value(&a.windows).unwrap(),
        serde_json::to_value(&b.windows).unwrap()
    );
    gigpies::automix::effects::add_pass(&mut s).unwrap();
    s.effects.as_mut().unwrap().tail_seconds = 0.;
    let f = ensemble::freeze(&s, &dir.0, &request).unwrap();
    let m = ensemble::measure::measure(&s, &dir.0, &request, &f).unwrap();
    assert!(!m.attribution.unwrap().tail_complete);
    request.budget.max_sample_visits = f.sample_visits;
    assert!(
        ensemble::freeze(&s, &dir.0, &request)
            .unwrap_err()
            .to_string()
            .contains("FX attribution")
    );
}

#[test]
fn vocal_fx_sum_matches_full_context_production_bus_with_cancellation() {
    use gigpies::automix::{effects, observe};
    let dir = Scratch::new("vocal-fx-full-context");
    let mut s = fixture(&dir, 3);
    // Opposite inputs plus a distinct third source exercise cancellation, not
    // just positive coherent copies. Excitation has even-order harmonics.
    for (i, c) in s.channels.iter_mut().enumerate() {
        let mut w = hound::WavWriter::create(
            dir.0.join(&c.file),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..9600 {
            let hz = if i == 2 { 2300. } else { 997. };
            let sign = if i == 1 { -1. } else { 1. };
            w.write_sample((sign * 0.4 * (TAU * hz * frame as f64 / 48000.).sin()) as f32)
                .unwrap();
        }
        w.finalize().unwrap();
        c.fader_db = 0.;
        c.pan = 0.;
        c.compressor.ratio = 1.;
    }
    effects::add_pass(&mut s).unwrap();
    let fx = s.effects.as_mut().unwrap();
    fx.buses.retain(|b| {
        matches!(
            b.name.as_str(),
            "vocal_hall" | "vocal_chorus" | "vocal_delay"
        )
    });
    fx.exciter_amount = 1.;
    fx.exciter.drive = 0.5;
    fx.exciter.tune_hz = 600.;
    for b in &mut fx.buses {
        b.sends = (0..3)
            .map(|channel| effects::Send { channel, db: -6. })
            .collect();
    }
    let request = Request {
        vocal_fx_attribution: true,
        observation_pairs: vec![[0, 1], [0, 2], [1, 2]],
        observation_groups: vec![ensemble::ObservationGroup {
            name: "all".into(),
            channels: vec![0, 1, 2],
        }],
        ..Default::default()
    };
    let f = ensemble::freeze(&s, &dir.0, &request).unwrap();
    let m = ensemble::measure::measure(&s, &dir.0, &request, &f).unwrap();
    let a = m.attribution.as_ref().unwrap();
    assert!(a.tail_complete, "{a:?}");
    assert!(a.max_tolerance_fraction <= 1.);
    let power = m
        .windows
        .iter()
        .filter(|w| w.channel == 3)
        .map(|w| w.stages[4].mean_power * w.frames as f64)
        .sum::<f64>()
        / f.timeline_frames as f64;
    let out = dir.0.join("production");
    observe(s.clone(), &dir.0, &out, f.timeline_frames as f64 / 48000.).unwrap();
    let stages: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("stages.json")).unwrap()).unwrap();
    let actual = stages["stages"][1]["rms_dbfs"].as_f64().unwrap();
    assert!(
        (10. * power.log10() + s.master_db - actual).abs() < 0.001,
        "{actual} vs {power}"
    );
    // Independent send attenuation across the three production FX engines.
    // Direct taps stay fixed; each source return scales, including tail history.
    let mut reduced = s.clone();
    let deltas = [-3., -6., 0.];
    for bus in &mut reduced.effects.as_mut().unwrap().buses {
        for send in &mut bus.sends {
            send.db += deltas[send.channel];
        }
    }
    let f_reduced = ensemble::freeze(&reduced, &dir.0, &request).unwrap();
    let attenuated = ensemble::measure::measure(&reduced, &dir.0, &request, &f_reduced).unwrap();
    for (old, new) in m
        .windows
        .iter()
        .zip(&attenuated.windows)
        .filter(|(w, _)| w.channel < 3)
    {
        let a = old.vocal_components.as_ref().unwrap();
        let b = new.vocal_components.as_ref().unwrap();
        let gain = 10_f64.powf(deltas[old.channel] / 20.);
        for band in 0..a.direct.band_power.len() {
            let direct = a.direct.band_power[band].unwrap();
            let wet = a.wet.band_power[band].unwrap();
            let total = old.stages[4].band_power[band].unwrap();
            let cross = total - direct - wet;
            assert_eq!(a.direct.band_power[band], b.direct.band_power[band]);
            let expected = (direct + gain * gain * wet + gain * cross).max(0.);
            let actual = new.stages[4].band_power[band].unwrap();
            if expected.max(actual) > 1e-12 {
                assert!((10. * (expected / actual).log10()).abs() < 0.01);
            }
        }
        assert_eq!(
            old.vocal_component_pairs.len(),
            new.vocal_component_pairs.len()
        );
        for ((other, components), (_, actual)) in old
            .vocal_component_pairs
            .iter()
            .zip(&new.vocal_component_pairs)
        {
            let peer = m
                .windows
                .iter()
                .find(|w| w.start == old.start && w.channel == *other)
                .unwrap();
            let peer = peer.vocal_components.as_ref().unwrap();
            let peer_gain = 10_f64.powf(deltas[*other] / 20.);
            for (index, (left, right, ga, gb)) in [
                (&a.direct, &peer.direct, 1., 1.),
                (&a.direct, &peer.wet, 1., peer_gain),
                (&a.wet, &peer.direct, gain, 1.),
                (&a.wet, &peer.wet, gain, peer_gain),
            ]
            .into_iter()
            .enumerate()
            {
                for band in 0..left.band_power.len() {
                    let x = left.band_power[band].unwrap();
                    let y = right.band_power[band].unwrap();
                    let cross = components[index][band].unwrap() - x - y;
                    let expected = (ga * ga * x + gb * gb * y + ga * gb * cross).max(0.);
                    let measured = actual[index][band].unwrap();
                    if expected.max(measured) > 1e-12 {
                        assert!((10. * (expected / measured).log10()).abs() < 0.01);
                    }
                }
            }
        }
    }
    // Nonlinear excitation is visible even when the opposite fundamentals cancel.
    s.effects.as_mut().unwrap().exciter_amount = 0.;
    let f = ensemble::freeze(&s, &dir.0, &request).unwrap();
    let dry_exciter = ensemble::measure::measure(&s, &dir.0, &request, &f).unwrap();
    assert!(
        m.windows
            .iter()
            .zip(&dry_exciter.windows)
            .any(|(a, b)| a.channel == 3
                && (a.stages[4].mean_power - b.stages[4].mean_power).abs() > 1e-9)
    );
}

#[test]
fn vocal_fx_final_master_tail_can_veto_unchanged_settings() {
    let dir = Scratch::new("vocal-master-tail");
    let mut s = fixture(&dir, 1);
    s.channels[0].hpf_hz = 0.;
    s.channels[0].eq.clear();
    s.channels[0].compressor.ratio = 1.;
    s.channels[0].compressor.makeup_db = 0.;
    s.channels[0].fader_db = 0.;
    s.master_hpf_hz = 20.;
    gigpies::automix::effects::add_pass(&mut s).unwrap();
    let fx = s.effects.as_mut().unwrap();
    fx.buses.clear();
    fx.exciter_amount = 0.;
    fx.tail_seconds = 0.01;
    let request = Request {
        vocal_fx_attribution: true,
        ..Default::default()
    };
    let state = ensemble::workflow::State {
        frozen: ensemble::freeze(&s, &dir.0, &request).unwrap(),
        baseline: s.clone(),
        request,
    };
    let out = dir.0.join("replay");
    ensemble::workflow::remeasure(state, s, &dir.0, &out).unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("composition.json")).unwrap()).unwrap();
    assert_eq!(report["passed"], false, "{report}");
    assert!(
        report["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("master tail"))
    );
    assert_eq!(report["final_tails"][0]["frames"], 480);
    assert!(report["final_tails"][0]["peak"].as_f64().unwrap() > 1e-5);
}

#[test]
fn vocal_pair_measurements_preserve_signed_power_and_charge_budget() {
    let dir = Scratch::new("vocal-pairs");
    let mut s = fixture(&dir, 3);
    for c in &mut s.channels {
        c.hpf_hz = 0.;
        c.eq.clear();
        c.compressor.ratio = 1.;
    }
    let mut writer = hound::WavWriter::create(
        dir.0.join("1.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for i in 0..480 {
        writer
            .write_sample((-0.1 * (TAU * 997. * i as f64 / 48000.).sin()) as f32)
            .unwrap();
    }
    writer.finalize().unwrap();
    let mut request = Request {
        observation_pairs: vec![[0, 1], [0, 2]],
        ..Default::default()
    };
    let frozen = ensemble::freeze(&s, &dir.0, &request).unwrap();
    let measured = ensemble::measure::measure(&s, &dir.0, &request, &frozen).unwrap();
    let pair = &measured.windows[0].pair_sum_band_power;
    assert_eq!(pair.len(), 2);
    assert!(pair[0].1.iter().all(|p| p.unwrap().abs() < 1e-20));
    for (sum, source) in pair[1]
        .1
        .iter()
        .zip(&measured.windows[0].stages[4].band_power)
    {
        assert!((sum.unwrap() - 4. * source.unwrap()).abs() < 1e-12);
    }
    request.observation_pairs.push([1, 0]);
    assert!(ensemble::freeze(&s, &dir.0, &request).is_err());
    request.observation_pairs = vec![[0, 1], [0, 1]];
    assert!(ensemble::freeze(&s, &dir.0, &request).is_err());
    request.observation_pairs.clear();
    // 3 sources * (6 spectral records + one 10ms event) = 21.
    request.budget.max_feature_records = 21;
    assert!(ensemble::freeze(&s, &dir.0, &request).is_ok());
    request.observation_pairs.push([0, 1]);
    assert!(ensemble::freeze(&s, &dir.0, &request).is_err());
}

#[test]
fn vocal_independent_singers_and_linked_captures_replay_with_fx() {
    vocal_singer_replay(false);
}

#[test]
fn vocal_contextual_singers_without_solo_anchors_replay_with_fx() {
    vocal_singer_replay(true);
}

fn vocal_singer_replay(contextual: bool) {
    let dir = Scratch::new("vocal-singer-replay");
    let mut s = fixture(&dir, 5);
    let window = ensemble::measure::FFT_FRAMES;
    let split_windows = if contextual { 5 } else { 15 };
    let frames = 2 * split_windows * window;
    for (index, c) in s.channels.iter_mut().enumerate() {
        c.hpf_hz = 0.;
        c.eq.clear();
        c.compressor.ratio = 1.;
        c.compressor.makeup_db = 0.;
        c.role = if index == 0 {
            config::Role::LeadVocal
        } else if index < 4 {
            config::Role::BackingVocal
        } else {
            config::Role::RhythmGuitar
        };
        c.fader_db = [-3., -8., -1., -8., 0.][index];
        let mut w = hound::WavWriter::create(
            dir.0.join(&c.file),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        let mut noise = if index == 3 { 43 } else { 42 + index as u64 };
        for frame in 0..frames {
            noise = noise.wrapping_mul(6364136223846793005).wrapping_add(1);
            let context = (frame / window % 15) / 5;
            let active = contextual
                || match index {
                    1 | 3 => context != 1,
                    2 => context != 0,
                    _ => true,
                };
            let x = if active {
                ((noise >> 32) as u32 as f64 / u32::MAX as f64 - 0.5) * 0.02
            } else {
                0.
            };
            // A second capture with signed cancellation, not a second singer.
            w.write_sample((x * if index == 3 { -0.25 } else { 1. }) as f32)
                .unwrap();
        }
        w.finalize().unwrap();
    }
    gigpies::automix::effects::add_pass(&mut s).unwrap();
    let fx = s.effects.as_mut().unwrap();
    fx.buses.retain(|b| b.name == "vocal_delay");
    for bus in &mut fx.buses {
        bus.sends = (0..5)
            .map(|channel| gigpies::automix::effects::Send { channel, db: -24. })
            .collect();
    }
    let mut request = Request {
        vocal_fx_attribution: true,
        observation_groups: vec![ensemble::ObservationGroup {
            name: "backs".into(),
            channels: vec![1, 2, 3],
        }],
        observation_pairs: (0..4)
            .flat_map(|i| ((i + 1)..4).map(move |j| [i, j]))
            .collect(),
        ..Default::default()
    };
    // Explicit synthetic composition budget; this test does not relax defaults.
    request.composition.max_peak_rise_db = 3.;
    let analysis = dir.0.join("analysis");
    ensemble::workflow::analyze(s, &dir.0, &analysis, request).unwrap();
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(analysis.join("state.json")).unwrap()).unwrap();
    let performance = |singer: &str, channels: Vec<usize>, capture: &str| {
        serde_json::json!({
            "singer":singer,"channels":channels,"capture":capture,"confidence":0.9,"spans":[[0,frames]]
        })
    };
    let capture = serde_json::json!({"decision_mode":if contextual {"contextual"} else {"solo_anchored"},"frozen_id":state["frozen"]["id"],
        "training_end_frame":split_windows*window,"provenance":["inherited","preset","preset","inherited","preset"],
        "groups":{
            "lead":{"channels":[0],"observation":0,"performances":[performance("lead",vec![0],"direct")]},
            "backing":{"channels":[1,2,3],"observation":"backs","performances":[
                performance("a",vec![1],"direct"), performance("b",vec![2],"direct"), performance("a",vec![3],"double")]},
            "accompaniment":{"channels":[4],"observation":4}}});
    let path = dir.0.join("capture.json");
    std::fs::write(&path, serde_json::to_vec(&capture).unwrap()).unwrap();
    let output = dir.0.join("decision");
    let status = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/vocal_balance.py"
        ))
        .arg(&analysis)
        .arg(path)
        .arg(&output)
        .arg("--source")
        .arg(&dir.0)
        .arg("--analyzer")
        .arg(env!("CARGO_BIN_EXE_gigpies"))
        .status()
        .unwrap();
    assert!(status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("decision.json")).unwrap()).unwrap();
    assert_eq!(report["accepted"], true, "{report}");
    assert!(report["prediction_error_db"].as_f64().unwrap() < 0.01);
    let changes = report["decisions"].as_array().unwrap();
    let delta = |channel| {
        changes.iter().find(|d| d["channel"] == channel).unwrap()["delta_db"]
            .as_f64()
            .unwrap()
    };
    assert!(delta(1) > 0.);
    assert!(delta(2) < 0.);
    assert_eq!(delta(1), delta(3));
    if contextual {
        assert_eq!(
            report["singer_policy"]["version"],
            "contextual-vocal-contribution-v1"
        );
        assert!(
            report["windows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|w| w["solo"][1] == false && w["solo"][2] == false)
        );
        assert!(report["metrics_resulting"]["training"]["singers"].is_array());
    }
}

#[test]
fn vocal_wetness_send_decision_replays_direct_returns_and_shared_bus() {
    replay_wetness_decision(false, false);
}

#[test]
fn individual_wet_send_replays_with_locked_backing_peer() {
    replay_wetness_decision(true, false);
}

#[test]
fn separate_bus_wetness_replays_both_returns() {
    replay_wetness_decision(false, true);
}

fn replay_wetness_decision(individual: bool, separate_bus: bool) {
    use gigpies::automix::effects::{self, Effect, Send};
    let dir = Scratch::new("vocal-wetness");
    let mut s = fixture(&dir, if individual { 4 } else { 3 });
    for (i, c) in s.channels.iter_mut().enumerate() {
        let mut w = hound::WavWriter::create(
            dir.0.join(&c.file),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        let mut random = 17_u32;
        for _ in 0..192000 {
            random = random.wrapping_mul(1664525).wrapping_add(1013904223);
            w.write_sample(((random as f64 / u32::MAX as f64 - 0.5) * 0.1) as f32)
                .unwrap();
        }
        w.finalize().unwrap();
        c.hpf_hz = 0.;
        c.eq.clear();
        c.compressor.ratio = 1.;
        c.compressor.makeup_db = 0.;
        c.fader_db = if individual && i == 2 { 6. } else { 0. };
        c.pan = 0.;
        c.role = match i {
            0 => config::Role::LeadVocal,
            1 => config::Role::BackingVocal,
            2 if individual => config::Role::BackingVocal,
            _ => config::Role::RhythmGuitar,
        };
    }
    effects::add_pass(&mut s).unwrap();
    let fx = s.effects.as_mut().unwrap();
    fx.exciter_amount = 0.;
    fx.tail_seconds = 0.5;
    fx.buses.retain(|b| b.name == "vocal_delay");
    assert_eq!(fx.buses.len(), 1);
    let bus = &mut fx.buses[0];
    bus.return_db = 0.;
    bus.hpf_hz = 20.;
    bus.lowpass_hz = 20000.;
    bus.sends = (0..s.channels.len())
        .map(|channel| Send {
            channel,
            db: if channel == s.channels.len() - 1 {
                -30.
            } else {
                0.
            },
        })
        .collect();
    if let Effect::Delay(delay) = &mut bus.effect {
        delay.left_ms = 20.;
        delay.right_ms = 20.;
        delay.feedback = 0.;
        delay.damping = 0.;
    } else {
        panic!("delay fixture");
    }
    if separate_bus {
        let mut quiet = bus.clone();
        quiet.name = "quiet_echo".into();
        quiet.return_db = -42.;
        fx.buses.push(quiet);
    }
    let request = Request {
        vocal_fx_attribution: true,
        observation_groups: if individual {
            vec![ensemble::ObservationGroup {
                name: "backs".into(),
                channels: vec![1, 2],
            }]
        } else {
            vec![]
        },
        observation_pairs: if individual {
            vec![[0, 1], [1, 2], [0, 2]]
        } else {
            vec![[0, 1]]
        },
        ..Default::default()
    };
    let analysis = dir.0.join("analysis");
    ensemble::workflow::analyze(s.clone(), &dir.0, &analysis, request).unwrap();
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(analysis.join("state.json")).unwrap()).unwrap();
    let mut capture = serde_json::json!({
        "frozen_id": state["frozen"]["id"], "training_end_frame": 98304,
        "decision_mode": "wetness", "provenance": ["inherited","inherited","preset"],
        "wetness_provenance": ["inherited","inherited","preset"],
        "groups": {
            "lead": {"channels":[0],"observation":0,"performances":[{"singer":"lead","channels":[0],"capture":"direct","confidence":0.9,"spans":[[0,192000]]}]},
            "backing": {"channels":[1],"observation":1,"performances":[{"singer":"back","channels":[1],"capture":"direct","confidence":0.9,"spans":[[0,192000]]}]},
            "accompaniment": {"channels":[2],"observation":2}
        }
    });
    if separate_bus {
        capture["decision_mode"] = serde_json::json!("bus_wetness");
    }
    if individual {
        capture["provenance"] =
            serde_json::json!(["inherited", "inherited", "inherited", "preset"]);
        capture["wetness_provenance"] = serde_json::json!([
            "operator_override",
            "operator_override",
            "inherited",
            "preset"
        ]);
        capture["groups"]["accompaniment"] = serde_json::json!({"channels":[3],"observation":3});
        capture["groups"]["backing"] = serde_json::json!({"channels":[1,2],"observation":"backs","performances":[
            {"singer":"back","channels":[1],"capture":"direct","confidence":0.9,"spans":[[0,192000]]},
            {"singer":"second","channels":[2],"capture":"direct","confidence":0.9,"spans":[[0,192000]]}]});
    }
    let capture_path = dir.0.join("capture.json");
    std::fs::write(&capture_path, serde_json::to_vec(&capture).unwrap()).unwrap();
    let out = dir.0.join("decision");
    let output = std::process::Command::new("python3")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .args(["scripts/vocal_balance.py"])
        .arg(&analysis)
        .arg(&capture_path)
        .arg(&out)
        .arg("--source")
        .arg(&dir.0)
        .arg("--analyzer")
        .arg(env!("CARGO_BIN_EXE_gigpies"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decision: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("decision.json")).unwrap()).unwrap();
    assert_eq!(decision["accepted"], true, "{decision}");
    if separate_bus {
        assert_eq!(decision["bus_action"]["bus"], "vocal_delay");
        assert_eq!(decision["bus_trials"].as_array().unwrap().len(), 5);
        let candidate: config::Session =
            serde_json::from_slice(&std::fs::read(out.join("settings.json")).unwrap()).unwrap();
        let candidate_fx = candidate.effects.unwrap();
        let original = s.effects.unwrap();
        assert_eq!(
            serde_json::to_value(&candidate_fx.buses[1]).unwrap(),
            serde_json::to_value(&original.buses[1]).unwrap()
        );
        assert_eq!(candidate_fx.buses[0].sends[2].db, -30.);
        assert!(candidate_fx.buses[0].sends[0].db < 0.);
        return;
    }
    if individual {
        assert_eq!(
            decision["wet_individual"]["channels"],
            serde_json::json!([2]),
            "{decision}"
        );
        assert!(decision["wet_individual"]["delta"].as_f64().unwrap() < 0.);
    } else {
        assert_eq!(decision["wet_deltas_db"], serde_json::json!([-6., -6.]));
    }
    assert!(decision["prediction_error_db"].as_f64().unwrap() < 0.01);
    let candidate: config::Session =
        serde_json::from_slice(&std::fs::read(out.join("settings.json")).unwrap()).unwrap();
    assert_eq!(candidate.channels[0].fader_db, s.channels[0].fader_db);
    assert_eq!(
        candidate.effects.unwrap().buses[0].sends.last().unwrap().db,
        -30.
    );
}

#[test]
fn vocal_send_replay_rejects_topology_boost_and_unattributed_changes() {
    let dir = Scratch::new("vocal-send-boundaries");
    let mut s = fixture(&dir, 3);
    s.channels[0].role = config::Role::LeadVocal;
    gigpies::automix::effects::add_pass(&mut s).unwrap();
    for attributed in [false, true] {
        let request = Request {
            vocal_fx_attribution: attributed,
            ..Default::default()
        };
        let state = ensemble::workflow::State {
            frozen: ensemble::freeze(&s, &dir.0, &request).unwrap(),
            baseline: s.clone(),
            request,
        };
        for fault in [
            "boost", "cut", "return", "topology", "engine", "exciter", "tail",
        ] {
            let mut candidate = s.clone();
            let fx = candidate.effects.as_mut().unwrap();
            let bus = fx.buses.iter_mut().find(|b| !b.sends.is_empty()).unwrap();
            match fault {
                "boost" => bus.sends[0].db += 0.5,
                "cut" => bus.sends[0].db -= 6.5,
                "return" => bus.return_db -= 1.,
                "topology" => bus.sends.clear(),
                "engine" => bus.hpf_hz += 1.,
                "exciter" => fx.exciter_amount *= 0.5,
                "tail" => fx.tail_seconds += 1.,
                _ => unreachable!(),
            }
            let out = dir.0.join("replay");
            assert!(
                ensemble::workflow::remeasure(state.clone(), candidate, &dir.0, &out).is_err(),
                "{attributed} {fault}"
            );
            assert!(!out.exists());
        }
        if !attributed {
            let mut candidate = s.clone();
            candidate
                .effects
                .as_mut()
                .unwrap()
                .buses
                .iter_mut()
                .find(|b| !b.sends.is_empty())
                .unwrap()
                .sends[0]
                .db -= 1.;
            assert!(
                ensemble::workflow::remeasure(state, candidate, &dir.0, &dir.0.join("replay"))
                    .is_err()
            );
        }
    }
}
