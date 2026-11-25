//! GP-AUTO main-vocal assist through the real engine authority and renderer.
use gigpies::{
    channel_processing::Config,
    control_model::{Command, Request, Scope},
    mixer_control::{EngineIntent, OfflineEngine},
    processing_wire::{ProcessingCommand, ProcessingReply, ProcessingRequest, ProcessingSnapshot},
    show::Counter,
    topology::EngineTopology,
    vocal_assist::{self, Origin, State},
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
const INPUTS: usize = 16;
const OUTPUTS: usize = 5;
const EPOCH: Counter = Counter(3);

fn audio(command: Command, id: u64, rev: u64, lease: Option<Counter>) -> Request {
    Request {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: EPOCH,
        writer: Some("desk-1".into()),
        lease,
        request_id: Some(Counter(id)),
        expected_revision: Some(Counter(rev)),
        command,
    }
}
fn read(version: u32) -> ProcessingRequest {
    ProcessingRequest {
        contract: "GP07-processing".into(),
        version,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: EPOCH,
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: ProcessingCommand::ProcessingSnapshot {},
    }
}
/// `id`/`rev` count processing writes; engine() already used one ID and revision
/// to open the input-01 fader.
fn set(input: &str, config: Config, id: u64, rev: u64) -> ProcessingRequest {
    ProcessingRequest {
        writer: Some("desk-1".into()),
        lease: Some(Counter(1)),
        request_id: Some(Counter(id + 1)),
        expected_revision: Some(Counter(rev + 1)),
        command: ProcessingCommand::ProcessingSet {
            input: input.into(),
            config,
        },
        ..read(6)
    }
}
fn engine() -> OfflineEngine {
    let mut e = OfflineEngine::with_topology(
        SHOW,
        EPOCH,
        Counter(0),
        0,
        EngineTopology::software(INPUTS, 3, 0).unwrap(),
    )
    .unwrap();
    let lease = e
        .handle(&audio(Command::Grant { scope: Scope::Foh }, 1, 0, None), 0)
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease;
    assert_eq!(lease, Some(Counter(1)));
    let fader = audio(
        Command::Set {
            targets: vec![gigpies::control_model::Edit {
                target: gigpies::control_model::Target::Fader {
                    input: "input-01".into(),
                },
                value: gigpies::control_model::Value::Integer(0),
            }],
        },
        2,
        0,
        lease,
    );
    e.handle(&fader, 0).unwrap();
    // Software topologies start with quiesced outputs; Main is observed here.
    e.rearm_outputs().unwrap();
    let mut out = vec![0.; 480 * OUTPUTS];
    e.process_interleaved(&vec![0.; 480 * INPUTS], &mut out, 0)
        .unwrap();
    assert_eq!(e.revision(), Counter(1));
    e
}
fn assist(reference_mdb: i32) -> vocal_assist::Config {
    vocal_assist::Config {
        bypass: false,
        reference_mdb,
        origin: Origin::Soundcheck,
        evidence_windows: 300,
        ..Default::default()
    }
}
/// Render `frames` (a multiple of 48) with a 220 Hz sine of `amplitude` on input 1.
/// Returns Main left.
fn pump(e: &mut OfflineEngine, frames: usize, offset: usize, amplitude: f64, now: u64) -> Vec<f64> {
    let mut input = vec![0.; frames * INPUTS];
    for f in 0..frames {
        input[f * INPUTS] =
            amplitude * (std::f64::consts::TAU * 220. * (offset + f) as f64 / 48000.).sin();
    }
    let mut out = vec![0.; frames * OUTPUTS];
    for (i, o) in input
        .chunks(480 * INPUTS)
        .zip(out.chunks_mut(480 * OUTPUTS))
    {
        e.process_interleaved(i, o, now).unwrap();
    }
    out.chunks(OUTPUTS).map(|f| f[0]).collect()
}
fn snapshot(e: &mut OfflineEngine, now: u64) -> ProcessingSnapshot {
    let reply = e.handle_processing(&read(6), now).unwrap();
    let decoded = ProcessingReply::decode(&reply.encode().unwrap()).unwrap();
    decoded.snapshot.unwrap()
}
fn apply(e: &mut OfflineEngine, request: &ProcessingRequest, now: u64) {
    assert_eq!(e.handle_processing(request, now).unwrap().state, "pending");
    pump(e, 480, 0, 0., now);
    let done = e.take_processing_completions().pop().unwrap();
    assert!(done.reason.is_none(), "{:?}", done.reason);
}
fn peak(samples: &[f64]) -> f64 {
    samples.iter().fold(0., |m, x| m.max(x.abs()))
}

#[test]
fn v6_readback_reduces_only_peaks_and_recovers_automatically() {
    let mut e = engine();
    let at_level = 0.1 * 2f64.sqrt(); // −20 dBFS RMS
    let config = Config {
        vocal_assist: assist(-20000),
        ..Config::default()
    };
    apply(&mut e, &set("input-01", config, 2, 0), 1);
    let s = snapshot(&mut e, 2);
    let c = &s.channels[0];
    assert_eq!(c.target.vocal_assist, config.vocal_assist);
    assert_eq!(c.vocal_assist.unwrap().state, State::Idle);
    assert!(s.channels[1].vocal_assist.is_none());

    // At the soundchecked level: untouched, identical to a bypassed reference engine.
    let mut reference = engine();
    let kept = pump(&mut e, 48000, 0, at_level, 3);
    let plain = pump(&mut reference, 48000, 0, at_level, 3);
    assert_eq!(kept, plain);
    assert_eq!(
        snapshot(&mut e, 4).channels[0]
            .vocal_assist
            .unwrap()
            .reduction_mdb,
        0
    );

    // +9 dB peak: Main follows the bounded 6 dB cut.
    let loud = pump(&mut e, 24000, 48000, at_level * 10f64.powf(9. / 20.), 5);
    let loud_plain = pump(
        &mut reference,
        24000,
        48000,
        at_level * 10f64.powf(9. / 20.),
        5,
    );
    let ratio = 20. * (peak(&loud[19200..]) / peak(&loud_plain[19200..])).log10();
    assert!((-6.1..=-5.0).contains(&ratio), "Main reduction {ratio} dB");
    let o = snapshot(&mut e, 6).channels[0].vocal_assist.unwrap();
    assert_eq!(o.state, State::Reducing);
    assert!((5000..=6000).contains(&o.reduction_mdb), "{o:?}");
    assert_eq!(o.onsets, Counter(1));

    // Back at level: recovers to exact unity without any operator action.
    pump(&mut e, 48000 * 6, 72000, at_level, 7);
    let o = snapshot(&mut e, 8).channels[0].vocal_assist.unwrap();
    assert_eq!((o.state, o.reduction_mdb), (State::Idle, 0));
    assert!(o.deepest_mdb >= 5000);
}

#[test]
fn successor_version_is_required_for_assist_readback_and_writes() {
    let mut e = engine();
    let config = Config {
        vocal_assist: assist(-20000),
        ..Config::default()
    };
    let mut v5 = set("input-01", config, 2, 0);
    v5.version = 5;
    assert!(v5.encode().is_err());
    let mut bytes = serde_json::to_value(set("input-01", config, 2, 0)).unwrap();
    bytes["version"] = 5.into();
    assert!(ProcessingRequest::decode(&serde_json::to_vec(&bytes).unwrap()).is_err());
    // Before configuration v5 readers still work; afterwards they are refused rather
    // than shown a snapshot that silently omits the assist.
    assert!(e.handle_processing(&read(5), 1).unwrap().reason.is_none());
    apply(&mut e, &set("input-01", config, 2, 0), 1);
    assert_eq!(
        e.handle_processing(&read(5), 2).unwrap().reason.as_deref(),
        Some("unsupported_version")
    );
    assert!(e.handle_processing(&read(6), 2).unwrap().reason.is_none());
}

#[test]
fn lease_loss_refuses_edits_and_engine_keeps_applied_assist() {
    let mut e = engine();
    let config = Config {
        vocal_assist: assist(-30000),
        ..Config::default()
    };
    apply(&mut e, &set("input-01", config, 2, 0), 1);
    pump(&mut e, 9600, 0, 0.5, 2);
    assert_eq!(
        snapshot(&mut e, 3).channels[0].vocal_assist.unwrap().state,
        State::Reducing
    );
    // The FOH lease expires without renewal: the surface is gone.
    let late = 60_000;
    let disable = Config {
        vocal_assist: vocal_assist::Config {
            bypass: true,
            ..config.vocal_assist
        },
        ..config
    };
    let reply = e
        .handle_processing(&set("input-01", disable, 3, 1), late)
        .unwrap();
    assert_eq!(reply.state, "final");
    assert_eq!(reply.reason.as_deref(), Some("lease"));
    pump(&mut e, 9600, 9600, 0.5, late);
    let s = snapshot(&mut e, late);
    assert_eq!(s.revision, Counter(2));
    assert_eq!(s.channels[0].target.vocal_assist, config.vocal_assist);
    assert_eq!(s.channels[0].vocal_assist.unwrap().state, State::Reducing);
}

#[test]
fn operator_edits_mid_reduction_never_step_or_reset_the_assist() {
    let mut e = engine();
    let config = Config {
        vocal_assist: assist(-30000),
        ..Config::default()
    };
    apply(&mut e, &set("input-01", config, 2, 0), 1);
    pump(&mut e, 9600, 0, 0.5, 2);
    let before = snapshot(&mut e, 3).channels[0].vocal_assist.unwrap();
    assert_eq!(before.state, State::Reducing);
    // An unrelated EQ edit crossfades the strip; the assist history continues.
    let eq = Config {
        eq_bypass: false,
        band2_gain_mdb: 1000,
        ..config
    };
    assert_eq!(
        e.handle_processing(&set("input-01", eq, 3, 1), 4)
            .unwrap()
            .state,
        "pending"
    );
    let main = pump(&mut e, 960, 9600, 0.5, 4);
    let steps = main
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0., f64::max);
    assert!(steps < 0.05, "largest sample step {steps}");
    e.take_processing_completions();
    let during = snapshot(&mut e, 5).channels[0].vocal_assist.unwrap();
    assert_eq!(during.onsets, before.onsets);
    assert_eq!(during.state, State::Reducing);
    // The operator disables the assist mid-reduction: release, not a jump.
    let off = Config {
        vocal_assist: vocal_assist::Config {
            bypass: true,
            ..eq.vocal_assist
        },
        ..eq
    };
    apply(&mut e, &set("input-01", off, 4, 2), 6);
    let o = snapshot(&mut e, 7).channels[0].vocal_assist.unwrap();
    assert_eq!(o.state, State::Recovering);
    pump(&mut e, 48000 * 6, 20000, 0.5, 8);
    let o = snapshot(&mut e, 9).channels[0].vocal_assist.unwrap();
    assert_eq!((o.state, o.reduction_mdb), (State::Bypassed, 0));
}

#[test]
fn retried_request_returns_its_outcome_without_reapplying() {
    let mut e = engine();
    let config = Config {
        vocal_assist: assist(-20000),
        ..Config::default()
    };
    let r = set("input-01", config, 2, 0);
    apply(&mut e, &r, 1);
    // A reconnecting surface resending the same identity cannot apply twice.
    let again = e.handle_processing(&r, 2).unwrap();
    assert!(again.reason.is_none());
    assert_eq!(e.revision(), Counter(2));
}

#[test]
fn persisted_intent_restores_settings_but_never_a_live_reduction() {
    let mut e = engine();
    let config = Config {
        vocal_assist: assist(-30000),
        ..Config::default()
    };
    apply(&mut e, &set("input-01", config, 2, 0), 1);
    pump(&mut e, 9600, 0, 0.5, 2);
    let intent = e.persisted_intent().unwrap();
    assert_eq!(intent.version, 3);
    let bytes = serde_json::to_vec(&intent).unwrap();
    let decoded = EngineIntent::decode(&bytes).unwrap();
    let mut restored = OfflineEngine::restore_intent(&decoded, Counter(4), 0).unwrap();
    let mut out = vec![0.; 480 * OUTPUTS];
    restored
        .process_interleaved(&vec![0.; 480 * INPUTS], &mut out, 0)
        .unwrap();
    let reply = restored.handle_processing(
        &ProcessingRequest {
            epoch: Counter(4),
            ..read(6)
        },
        1,
    );
    let s = reply.unwrap().snapshot.unwrap();
    assert_eq!(s.channels[0].target.vocal_assist, config.vocal_assist);
    let o = s.channels[0].vocal_assist.unwrap();
    assert_eq!(
        (o.state, o.reduction_mdb, o.onsets),
        (State::Idle, 0, Counter(0))
    );
    // An older intent version cannot carry assist settings.
    let mut old = intent.clone();
    old.version = 2;
    assert!(OfflineEngine::restore_intent(&old, Counter(5), 0).is_err());
}

/// Producer corpus for consumers: GP07-processing:6 readback with an active assist,
/// a soundcheck request and its GigPies document.
fn producer_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let mut e = engine();
    let config = Config {
        vocal_assist: assist(-30000),
        ..Config::default()
    };
    apply(&mut e, &set("input-01", config, 2, 0), 1);
    pump(&mut e, 9600, 0, 0.5, 2);
    let snapshot = e.handle_processing(&read(6), 3).unwrap().encode().unwrap();
    let windows: Vec<vocal_assist::Window> = (0..240u64)
        .map(|i| {
            let silent = i % 12 == 0;
            let rms = if silent {
                -120000
            } else {
                -32000 + (i % 40) as i32 * 200
            };
            vocal_assist::Window {
                sequence: Counter(100 + i * 2),
                rms_mdbfs: Some(rms),
                sample_peak_mdbfs: Some(if silent { -120000 } else { rms + 10000 }),
                silent,
                clip_count: 0,
                age_ms: 30 + (i % 5) as u32 * 20,
            }
        })
        .collect();
    let request = vocal_assist::Request {
        version: 1,
        show_id: SHOW.into(),
        epoch: EPOCH,
        input: "input-01".into(),
        tap: "input-01:processed".into(),
        windows,
    };
    let document = request.prepare();
    assert_eq!(document.proposal.reason, vocal_assist::Reason::Prepared);
    let mut clipped = request.clone();
    clipped.windows[50].clip_count = 2;
    vec![
        ("snapshot.json", snapshot),
        (
            "soundcheck-request.json",
            serde_json::to_vec(&request).unwrap(),
        ),
        (
            "soundcheck-document.json",
            serde_json::to_vec(&document).unwrap(),
        ),
        (
            "soundcheck-clipped-document.json",
            serde_json::to_vec(&clipped.prepare()).unwrap(),
        ),
    ]
}
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/vocal-assist/v1"
);

#[test]
fn producer_fixture_v1() {
    for (name, bytes) in producer_fixtures() {
        let path = format!("{FIXTURES}/{name}");
        assert_eq!(
            std::fs::read(&path).unwrap_or_default(),
            bytes,
            "{path} differs; regenerate with --ignored write_producer_fixture_v1 and review"
        );
    }
}
#[test]
#[ignore = "writes the reviewed producer corpus"]
fn write_producer_fixture_v1() {
    std::fs::create_dir_all(FIXTURES).unwrap();
    for (name, bytes) in producer_fixtures() {
        std::fs::write(format!("{FIXTURES}/{name}"), bytes).unwrap();
    }
}

/// Opt-in recorded check, statistics only (no PCM written):
/// `GP_VOCAL_STEMS="/abs/lead1.wav@0:/abs/lead2.wav@60" cargo test --release --test vocal_assist recorded -- --ignored --nocapture`.
/// Thirty seconds of singing from `@start` seconds stand in for the soundcheck; the
/// assist then runs over the complete stem with the derived settings.
#[test]
#[ignore = "needs local private recordings; set GP_VOCAL_STEMS"]
fn recorded_lead_vocal_soundcheck_and_assist() {
    let stems = std::env::var("GP_VOCAL_STEMS").expect("GP_VOCAL_STEMS");
    for stem in stems.split(':') {
        let (path, start) = stem.rsplit_once('@').unwrap_or((stem, "0"));
        let start: usize = start.parse().unwrap();
        let mut reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        let scale = match spec.sample_format {
            hound::SampleFormat::Float => 1.,
            hound::SampleFormat::Int => 2f64.powi(spec.bits_per_sample as i32 - 1),
        };
        let raw: Vec<f64> = match spec.sample_format {
            hound::SampleFormat::Float => {
                reader.samples::<f32>().map(|s| s.unwrap() as f64).collect()
            }
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f64 / scale)
                .collect(),
        };
        let channels = spec.channels as usize;
        let mono: Vec<f64> = raw.chunks(channels).map(|c| c[0]).collect();
        let rate = spec.sample_rate as usize;
        let window = rate / 50;
        let mdb = |x: f64| ((20000. * x.max(1e-6).log10()).round() as i32).clamp(-120000, 0);
        let windows: Vec<vocal_assist::Window> = mono
            [(start * rate).min(mono.len())..((start + 30) * rate).min(mono.len())]
            .chunks_exact(window)
            .enumerate()
            .filter(|(i, _)| i % 2 == 0) // the Desk observer samples at most 25 Hz
            .map(|(i, w)| {
                let peak = w.iter().fold(0., |m: f64, x| m.max(x.abs()));
                let rms = (w.iter().map(|x| x * x).sum::<f64>() / w.len() as f64).sqrt();
                vocal_assist::Window {
                    sequence: Counter(i as u64 + 1),
                    rms_mdbfs: Some(mdb(rms)),
                    sample_peak_mdbfs: Some(mdb(peak)),
                    silent: peak == 0.,
                    clip_count: w.iter().filter(|x| x.abs() >= 1.).count() as u32,
                    age_ms: 40,
                }
            })
            .collect();
        let proposal = vocal_assist::prepare_soundcheck(&windows);
        println!(
            "{path} @{start}s: {:?} {:?}",
            proposal.reason, proposal.measurements
        );
        if proposal.reason != vocal_assist::Reason::Prepared {
            continue;
        }
        let mut a = vocal_assist::Assist::new(
            vocal_assist::Prepared::new(proposal.config, spec.sample_rate).unwrap(),
        );
        let (mut reducing, mut cuts) = (0usize, Vec::new());
        for (n, x) in mono.iter().enumerate() {
            a.tick(*x);
            if a.state == State::Reducing {
                reducing += 1;
            }
            if n % window == 0 {
                cuts.push(-20. * a.linear_gain().log10());
            }
        }
        cuts.sort_by(f64::total_cmp);
        let o = a.observation();
        println!(
            "  keep {:.1} dBFS, threshold {:.1}: reducing {:.1}% of samples, cut p50 {:.2} / p95 {:.2} / p99 {:.2} / max {:.2} dB, {} events over {:.0} s",
            proposal.config.reference_mdb as f64 / 1000.,
            proposal.config.threshold_mdb() as f64 / 1000.,
            100. * reducing as f64 / mono.len() as f64,
            cuts[cuts.len() / 2],
            cuts[cuts.len() * 95 / 100],
            cuts[cuts.len() * 99 / 100],
            o.deepest_mdb as f64 / 1000.,
            o.onsets.0,
            mono.len() as f64 / rate as f64
        );
        assert!(o.deepest_mdb <= proposal.config.max_cut_mdb);
    }
}
