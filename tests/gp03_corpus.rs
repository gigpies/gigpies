//! Small deterministic producer-executed corpus; regeneration is explicit only.
use gigpies::{
    control_model::{Command, Edit, Request, Scope, Target, Value},
    mixer_control::OfflineEngine,
    show::Counter,
};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::path::Path;
fn execute() -> Json {
    let show = "11111111-1111-4111-8111-111111111111";
    let mut engine = OfflineEngine::new(show, Counter(9), Counter(12), 48000).unwrap();
    let mut request = Request {
        contract: "C-AUDIO".into(),
        version: 1,
        show_id: show.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: Some("desk-corpus".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(12)),
        command: Command::Grant { scope: Scope::Foh },
    };
    let grant = engine
        .handle_encoded(&request.encode().unwrap(), 0)
        .unwrap();
    let granted: Json = serde_json::from_slice(&grant).unwrap();
    request.lease = Some(Counter(
        granted["outcome"]["body"]["granted_lease"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    ));
    request.request_id = Some(Counter(41));
    request.command = Command::Set {
        targets: vec![Edit {
            target: Target::Fader {
                input: "input-01".into(),
            },
            value: Value::Integer(-3000),
        }],
    };
    let initial = engine.snapshot().unwrap();
    let command = request.encode().unwrap();
    let pending = engine.handle_encoded(&command, 0).unwrap();
    let mut output = [[0.0; 4]; 289];
    let input = [[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; 289];
    let final_reply = engine.process(&input, &mut output, 1).unwrap();
    let final_snapshot = engine.snapshot().unwrap();
    let retry = engine.handle_encoded(&command, 1).unwrap();
    json!({"grant_request":Request {lease:None,request_id:Some(Counter(1)),command:Command::Grant {scope:Scope::Foh},..request.clone()},"grant_response":granted,"request":serde_json::from_slice::<Json>(&command).unwrap(),"pending":serde_json::from_slice::<Json>(&pending).unwrap(),"initial":initial,"committed":final_reply,"final":final_snapshot,"retry":serde_json::from_slice::<Json>(&retry).unwrap(),"known_samples":[{"frame":"48000","output":output[0]},{"frame":"48048","output":output[48]},{"frame":"48288","output":output[288]}]})
}
#[test]
fn replay_actual_producer_corpus() {
    let fixture: Json =
        serde_json::from_slice(&std::fs::read("tests/fixtures/gp03/v1/e03-rendered.json").unwrap())
            .unwrap();
    assert_eq!(execute(), fixture);
}
#[test]
#[ignore = "one-time producer evidence generation; run explicitly when GP03 corpus changes"]
fn regenerate_gp03_corpus() {
    let dir = Path::new("tests/fixtures/gp03/v1");
    std::fs::create_dir_all(dir).unwrap();
    let bytes = serde_json::to_vec_pretty(&execute()).unwrap();
    std::fs::write(dir.join("e03-rendered.json"), &bytes).unwrap();
    let source: serde_json::Map<String, Json> = [
        "src/control_model.rs",
        "src/mixer.rs",
        "src/mixer_control.rs",
        "tests/gp03_corpus.rs",
    ]
    .into_iter()
    .map(|p| {
        (
            p.to_owned(),
            json!(gigpies::hash::hex(Sha256::digest(
                std::fs::read(p).unwrap()
            ))),
        )
    })
    .collect();
    let manifest = json!({"producer":"actual OfflineEngine::handle_encoded/process execution","capability":"GP03-rendered:1","baseline":"99eb0f08050a9a86039af9f1ba2a3541649ca7b6","contract_sha256":"54a887f4a300d049d3ba67077a2e0446f7a92c57c41cef9d3b7da13735a092ac","source_sha256":source,"fixture_sha256":gigpies::hash::hex(Sha256::digest(bytes))});
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}
