use gigpies::automix::bass::{SIZE, estimate_note};
fn tone(hz: f64, scale: f64) -> Vec<[f64; 2]> {
    (0..SIZE)
        .map(|i| {
            let t = i as f64 / 44100.;
            let v = scale
                * ((std::f64::consts::TAU * hz * t).sin()
                    + 0.4 * (std::f64::consts::TAU * hz * 2. * t).sin());
            [v, -v]
        })
        .collect()
}
#[test]
fn notes_are_content_and_quiet_playing_remains_measurable() {
    for hz in [41.203, 55., 73.416, 110.] {
        for scale in [0.1, 0.001] {
            let n = estimate_note(&tone(hz, scale), 44100).unwrap();
            assert!((n.hz / hz - 1.).abs() < 0.01, "{n:?}");
            assert!(n.periodicity > 0.9);
        }
    }
}
#[test]
fn silence_noise_and_changing_notes_abstain() {
    assert!(estimate_note(&tone(55., 0.), 44100).is_none());
    let mut x = tone(55., 0.1);
    let y = tone(82.407, 0.1);
    x[SIZE / 2..].copy_from_slice(&y[SIZE / 2..]);
    assert!(estimate_note(&x, 44100).is_none());
    let mut seed = 123_u64;
    let noise = (0..SIZE)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let v = (seed >> 32) as f64 / u32::MAX as f64 - 0.5;
            [v, v]
        })
        .collect::<Vec<_>>();
    assert!(estimate_note(&noise, 44100).is_none());
}

use gigpies::automix::{
    bass::{self, Policy},
    config::{self, Role},
    unity,
};
struct Fixture {
    root: std::path::PathBuf,
    s: config::Session,
    p: Policy,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn fixture() -> Fixture {
    fixture_with_low_balance(false)
}
fn fixture_with_low_balance(sub_heavy: bool) -> Fixture {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("gigpies-bass-{}-{id}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut s = config::example();
    s.channels.truncate(2);
    s.sample_rate = 32000;
    for (i, c) in s.channels.iter_mut().enumerate() {
        c.file = format!("{i}.wav").into();
        c.role = if i == 0 { Role::BassDi } else { Role::Kick };
        c.eq.clear();
        c.compressor.ratio = 2.;
        c.compressor.threshold_db = -20.;
        c.compressor.makeup_db = 0.;
        c.compressor.attack_ms = 15.;
        c.compressor.release_ms = 200.;
    }
    unity::prepare(&mut s).unwrap();
    for ch in 0..2 {
        let mut w = hound::WavWriter::create(
            root.join(format!("{ch}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 32000,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for k in 0..32000 * 24 {
            let t = k as f64 / 32000.;
            let hz = [41.203, 55., 82.407][(t / 4.) as usize % 3];
            let phase = std::f64::consts::TAU * hz * t;
            // Last second of each phrase includes a kick-only pause.
            let env = if t % 4. > 3. {
                0.
            } else if t % 4. < 1. {
                0.15
            } else {
                1.
            };
            let kick = 0.3 * (-(t % 0.5) * 35.).exp() * (std::f64::consts::TAU * 60. * t).sin();
            let x = if ch == 0 {
                if sub_heavy {
                    // Body/definition already balanced, but fundamental dwarfs both.
                    env * (0.25 * phase.sin()
                        + 0.05 * (phase * 3.).sin()
                        + 0.02 * (phase * 10.).sin())
                } else {
                    env * (0.1 * phase.sin()
                        + 0.1 * (phase * 3.).sin()
                        + 0.004 * (phase * 10.).sin())
                        + kick * 0.0001
                }
            } else {
                kick
            };
            w.write_sample((x * 8388607.) as i32).unwrap();
        }
        w.finalize().unwrap();
    }
    let p = Policy {
        bass: 0,
        bass_file: "0.wav".into(),
        kick: 1,
        kick_file: "1.wav".into(),
        definition_body_db: [-12., -4.],
        definition_low_db: None,
        max_eq_db: 4.,
        max_output_rise_db: 3.,
        max_note_spread_db: 2.,
        max_added_reduction_db: 1.,
        max_fader_db: 1.5,
        recorded_source: true,
    };
    Fixture { root, s, p }
}
#[test]
fn actual_dsp_guards_split_independence_and_routing_contract() {
    let f = fixture();
    let m = bass::measure(&f.s, &f.root, &f.p, 0., 48.).unwrap();
    let q = bass::propose(&m.windows, 32000, &f.p, 8);
    assert!(
        q.diagnosis.training_notes >= 3,
        "{}",
        q.diagnosis.confidence
    );
    assert!(q.chosen.is_some());
    assert!(
        q.candidates
            .iter()
            .any(|c| c.eq.iter().any(|e| e.hz == 55.) && !c.rejection.is_empty())
    );
    let mut held_changed = m.windows.clone();
    for w in &mut held_changed {
        if w.held_out {
            w.bass_spectrum.fill(1.);
        }
    }
    assert_eq!(
        bass::propose(&held_changed, 32000, &f.p, 8).chosen,
        q.chosen
    );
    let mut silent = m.windows.clone();
    for w in &mut silent {
        w.note = None;
        w.stages[0].rms_dbfs = -240.;
    }
    assert!(bass::propose(&silent, 32000, &f.p, 8).chosen.is_none());
    assert!(bass::propose(&m.windows, 32000, &f.p, 0).chosen.is_none());
    let mut s = f.s.clone();
    s.channels[0]
        .eq
        .extend(q.candidates[q.chosen.unwrap()].eq.clone());
    let after = bass::measure(&s, &f.root, &f.p, 0., 48.).unwrap();
    let valid = bass::validate(&m, &after, &q, 32000, &f.p);
    // Synthetic source is deliberately sparse: acceptance is not assumed from proposal.
    assert!(valid.definition_body_after[0].unwrap() > valid.definition_body_before[0].unwrap());
    for (a, b) in m.windows.iter().zip(&after.windows) {
        assert_eq!(a.stages[6].rms_dbfs, b.stages[6].rms_dbfs);
    }
    let mut contact = m;
    contact.windows[0].pcm_contact_fraction = 0.002;
    assert!(
        !bass::validate(&contact, &after, &q, 32000, &f.p)
            .failures
            .iter()
            .any(|r| r.contains("repeated PCM"))
    );
    contact.windows[1].pcm_contact_fraction = 0.002;
    contact.windows[2].pcm_contact_fraction = 0.002;
    assert!(
        bass::validate(&contact, &after, &q, 32000, &f.p)
            .failures
            .iter()
            .any(|r| r.contains("repeated PCM"))
    );
    for w in &mut contact.windows {
        w.pcm_contact_fraction = 0.;
    }
    let m = contact;
    let mut bad = after;
    bad.envelopes[20].max_reduction_db = 30.;
    assert!(
        bass::validate(&m, &bad, &q, 32000, &f.p)
            .failures
            .iter()
            .any(|r| r.contains("transient"))
    );
    bad.windows[4].stages[4].rms_dbfs += 12.;
    assert!(!bass::validate(&m, &bad, &q, 32000, &f.p).accepted);
    let mut low_conf = q;
    low_conf.diagnosis.held_out_notes = 0;
    assert!(
        bass::validate(&m, &bad, &low_conf, 32000, &f.p)
            .failures
            .iter()
            .any(|r| r.contains("coverage"))
    );
    let mut excessive = f.p.clone();
    excessive.max_eq_db = 12.;
    assert!(excessive.validate(&s).is_err());
    let mut changed = f.s.clone();
    changed.channels[0].role = Role::BassAmp;
    assert!(f.p.validate(&changed).is_err());
    changed = f.s.clone();
    changed.groups[0].trim_db = 1.;
    assert!(f.p.validate(&changed).is_err());
    assert!(bass::measure(&f.s, &f.root, &f.p, 1., 601.).is_err());
    let out = f.root.join("report");
    bass::analyze(f.s.clone(), &f.root, &out, f.p.clone(), 0., 4.).unwrap();
    assert!(bass::analyze(f.s.clone(), &f.root, &out, f.p.clone(), 0., 4.).is_err());
}

#[test]
fn below_25_energy_is_visible_and_silence_has_no_ratio() {
    let rate = 32768;
    let mut spectrum = vec![0.; SIZE / 2 + 1];
    spectrum[8] = 4.; // 16 Hz
    spectrum[25] = 3.; // 50 Hz
    spectrum[100] = 2.; // 200 Hz
    spectrum[400] = 1.; // 800 Hz
    let e = bass::low_energy(&spectrum, rate);
    assert!((e.below_100_fraction.unwrap() - 0.7).abs() < 1e-12);
    assert!((e.below_25_dbfs - 10. * 4_f64.log10()).abs() < 1e-10);
    assert!((e.from_25_to_100_dbfs - 10. * 3_f64.log10()).abs() < 1e-10);
    assert!((e.definition_low_db.unwrap() + 10. * 7_f64.log10()).abs() < 1e-10);
    spectrum.fill(0.);
    let e = bass::low_energy(&spectrum, rate);
    assert!(e.definition_low_db.is_none() && e.below_100_fraction.is_none());
}
#[test]
fn sub_heavy_bass_can_trigger_without_a_body_fault_and_keeps_note_guards() {
    let mut f = fixture_with_low_balance(true);
    // Isolate spectral balance from compression for a controlled known fault.
    f.s.channels[0].compressor.ratio = 1.;
    f.s.channels[0].hpf_hz = 0.;
    f.s.master_hpf_hz = 0.;
    f.p.definition_body_db = [-12., 0.];
    let m = bass::measure(&f.s, &f.root, &f.p, 0., 24.).unwrap();
    let old = bass::propose(&m.windows, 32000, &f.p, 8);
    assert!(
        old.chosen.is_none(),
        "body-only intent should not invent a low target"
    );
    f.p.definition_low_db = Some([-18., -6.]);
    let q = bass::propose(&m.windows, 32000, &f.p, 8);
    assert!(q.diagnosis.fraction_below_intent.unwrap() < 0.7);
    assert!(q.diagnosis.fraction_below_low_intent.unwrap() >= 0.7);
    assert!(q.diagnosis.training_below_100_fraction.unwrap() > 0.8);
    assert!(q.chosen.is_some());
    assert_eq!(q.candidates.len(), 99);
    assert!(q.candidates.iter().any(|c| {
        c.eq.iter()
            .any(|e| matches!(e.kind, config::EqKind::LowShelf) && e.db < 0.)
    }));
    assert!(
        q.candidates
            .iter()
            .any(|c| c.rejection.iter().any(|r| r.contains("fundamental")))
    );
    let mut changed = m.windows.clone();
    for w in &mut changed {
        if w.held_out {
            w.bass_spectrum.fill(1.);
        }
    }
    assert_eq!(bass::propose(&changed, 32000, &f.p, 8).chosen, q.chosen);
    let mut s = f.s.clone();
    s.channels[0]
        .eq
        .extend(q.candidates[q.chosen.unwrap()].eq.clone());
    let after = bass::measure(&s, &f.root, &f.p, 0., 24.).unwrap();
    let o = bass::validate(&m, &after, &q, 32000, &f.p);
    assert!(o.accepted, "{:?}", o.failures);
    for split in 0..2 {
        assert!(
            o.definition_low_after[split].unwrap() > o.definition_low_before[split].unwrap() + 0.25
        );
    }
    // A master-filtered result must not hide increased pre-compressor sub load.
    let mut bad = after;
    for w in &mut bad.windows {
        w.stages[1].low_energy.from_25_to_100_dbfs += 6.;
    }
    let o = bass::validate(&m, &bad, &q, 32000, &f.p);
    assert!(
        o.failures
            .iter()
            .any(|r| r.contains("pre-compressor low energy"))
    );
    f.p.definition_low_db = Some([f64::NAN, 0.]);
    assert!(f.p.validate(&f.s).is_err());
}
#[test]
fn legacy_policy_does_not_acquire_an_unrequested_low_target() {
    let f = fixture();
    let mut value = serde_json::to_value(&f.p).unwrap();
    value.as_object_mut().unwrap().remove("definition_low_db");
    let restored: Policy = serde_json::from_value(value).unwrap();
    assert!(restored.definition_low_db.is_none());
}
