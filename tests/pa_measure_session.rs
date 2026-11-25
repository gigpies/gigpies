//! GP-PA-MEASURE step 1: frozen capture/excitation contract corpus.
//! `write_fixture_v1` (ignored) regenerates it from owner code; `fixture_v1` checks it.
//! SHR PA keeps byte-identical copies of the record and excitation samples.
use gigpies::{
    measurement_session::{
        Basis, Calibration, Event, Excitation, LEVEL_DEFAULT_MDBFS, Mic, Path, Plan, PriorRoute,
        Session,
    },
    show::Counter,
};
use std::path::PathBuf;

const DIR: &str = "tests/fixtures/gp-pa-measure/v1";
/// Sample indexes covering both fades, the body and the start of the silent tail.
const INDEXES: [usize; 10] = [0, 1, 128, 255, 256, 1000, 12288, 24000, 24575, 24576];

fn plan_repeated() -> Plan {
    Plan {
        contract: "GP-PA-MEASURE-CAPTURE".into(),
        version: 1,
        id: "sweep-foh-left".into(),
        position_id: "foh-left".into(),
        basis: Basis {
            show_id: "00000000-0000-0000-0000-000000000001".into(),
            source_epoch: Counter(3),
            map_revision: Counter(1),
            graph_generation: Counter(2),
            configuration_revision: Counter(7),
        },
        path: Path { output_index: 1 },
        mic: Mic {
            capture_slot: 16,
            calibration: Some(Calibration {
                provenance: "measurement-mic-1".into(),
                sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            }),
        },
        excitation: Excitation {
            kind: "log_sweep".into(),
            start_hz: 40,
            end_hz: 16000,
            sweep_samples: 24576,
            total_samples: 32768,
            level_mdbfs: LEVEL_DEFAULT_MDBFS,
            repeats: 4,
        },
        confirm_audible: true,
    }
}

fn plan_single() -> Plan {
    let mut p = plan_repeated();
    p.id = "sweep-sub".into();
    p.path.output_index = 0;
    p.mic.calibration = None;
    p.excitation = Excitation {
        kind: "log_sweep".into(),
        start_hz: 20,
        end_hz: 2000,
        sweep_samples: 32768,
        total_samples: 65536,
        level_mdbfs: -36000,
        repeats: 1,
    };
    p
}

fn captured(plan: Plan) -> Session {
    let mut s = Session::new(
        plan,
        PriorRoute {
            configuration_revision: Counter(7),
            output_muted: vec![false, true],
        },
    )
    .unwrap();
    s.on(Event::Start { first_frame: 96000 }).unwrap();
    let total = s.plan.excitation.session_samples() as u64;
    let mut at = 96000;
    while at < 96000 + total {
        s.on(Event::Frames {
            first_frame: at,
            frames: 480,
            mic_peak: 0.25,
            dropped: 0,
        })
        .unwrap();
        at += 480;
    }
    s.on(Event::Restored { ok: true }).unwrap();
    s
}

fn json(v: &impl serde::Serialize) -> Vec<u8> {
    let mut b = serde_json::to_vec_pretty(v).unwrap();
    b.push(b'\n');
    b
}

fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    let repeated = plan_repeated();
    let text = String::from_utf8(json(&repeated)).unwrap();
    let e = &repeated.excitation;
    let samples = serde_json::json!({
        "spec": e.owner_spec(),
        "indexes": INDEXES,
        "values": INDEXES.iter().map(|&i| e.sample(i)).collect::<Vec<_>>(),
    });
    vec![
        ("plan-repeated.json", json(&repeated)),
        ("plan-single.json", json(&plan_single())),
        (
            "reject-float-level.json",
            text.replace("-42000", "-42000.0").into_bytes(),
        ),
        (
            "reject-too-loud.json",
            text.replace("-42000", "-29999").into_bytes(),
        ),
        (
            "reject-unconfirmed.json",
            text.replace("\"confirm_audible\": true", "\"confirm_audible\": false")
                .into_bytes(),
        ),
        (
            "reject-extra-field.json",
            text.replace("\"repeats\": 4", "\"repeats\": 4,\n    \"gain_db\": 0")
                .into_bytes(),
        ),
        (
            "record-captured.json",
            json(&captured(plan_repeated()).record().unwrap()),
        ),
        (
            "record-single.json",
            json(&captured(plan_single()).record().unwrap()),
        ),
        ("excitation-samples.json", json(&samples)),
    ]
}

#[test]
#[ignore = "regenerates the frozen corpus; review the diff and README hashes"]
fn write_fixture_v1() {
    let dir = PathBuf::from(DIR);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in corpus() {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

#[test]
fn fixture_v1() {
    for (name, bytes) in corpus() {
        let frozen = std::fs::read(PathBuf::from(DIR).join(name)).unwrap();
        assert_eq!(frozen, bytes, "{name} drifted from owner code");
        if name.starts_with("plan-") {
            Plan::decode(&frozen).unwrap();
        } else if name.starts_with("reject-") {
            assert!(Plan::decode(&frozen).is_err(), "{name}");
        }
    }
    // README hashes match the files.
    let readme = std::fs::read_to_string(PathBuf::from(DIR).join("README.md")).unwrap();
    for (name, bytes) in corpus() {
        use sha2::{Digest, Sha256};
        let hash = gigpies::hash::hex(Sha256::digest(&bytes));
        assert!(
            readme.contains(&format!("| {name} | {hash} |")),
            "{name} hash missing from README"
        );
    }
}
