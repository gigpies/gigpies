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
        let p = std::env::temp_dir().join(format!("ensemble-{name}-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
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
