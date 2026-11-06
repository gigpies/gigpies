//! Producer-executed preview interoperability amendment; original corpus is retained.
use gigpies::{
    control_model::{Command, Edit, Mode, Request, Scope, Target, Value},
    mixer_control::{OfflineEngine, RenderedReply},
    show::Counter,
};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::path::Path;
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
fn target() -> Target {
    Target::Fader {
        input: "input-01".into(),
    }
}
struct Flow {
    engine: OfflineEngine,
    lease: Option<Counter>,
    next_id: u64,
    events: Vec<Json>,
}
impl Flow {
    fn new() -> Self {
        Self {
            engine: OfflineEngine::new(SHOW, Counter(9), Counter(0), 0).unwrap(),
            lease: None,
            next_id: 1,
            events: vec![],
        }
    }
    fn command(&mut self, label: &str, command: Command, now: u64) -> RenderedReply {
        let request = Request {
            contract: "C-AUDIO".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some("desk-preview-corpus".into()),
            lease: self.lease,
            request_id: Some(Counter(self.next_id)),
            expected_revision: Some(self.engine.revision()),
            command,
        };
        self.next_id += 1;
        let encoded = request.encode().unwrap();
        let response = self.engine.handle_encoded(&encoded, now).unwrap();
        let decoded = RenderedReply::decode(&response).unwrap();
        self.events.push(json!({"label":label,"now_ms":now,"request":serde_json::from_slice::<Json>(&encoded).unwrap(),"response":serde_json::from_slice::<Json>(&response).unwrap()}));
        decoded
    }
    fn pump(&mut self, label: &str, now: u64) -> Vec<RenderedReply> {
        let input = [[0.125, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; 48];
        let mut output = [[0.0; 4]; 48];
        let replies = self.engine.process(&input, &mut output, now).unwrap();
        for reply in &replies {
            RenderedReply::decode(&reply.encode().unwrap()).unwrap();
        }
        self.events.push(json!({"label":label,"now_ms":now,"frames":48,"completions":replies,"snapshot":self.engine.snapshot().unwrap(),"first_sample":output[0],"last_sample":output[47]}));
        replies
    }
    fn setup(&mut self) {
        let grant = self.command("grant", Command::Grant { scope: Scope::Foh }, 0);
        self.lease = grant.outcome.unwrap().body.granted_lease;
        assert!(self.lease.is_some());
        assert_eq!(
            self.command(
                "assist_pending",
                Command::SetMode {
                    mode: Mode::Assist,
                    bounds: vec![]
                },
                0
            )
            .state,
            "pending"
        );
        self.pump("assist_tick_before_boundary", 0);
        assert_eq!(
            self.pump("assist_tick_committed", 0)[0]
                .outcome
                .as_ref()
                .unwrap()
                .kind,
            "applied"
        );
        assert_eq!(
            self.command(
                "proposal",
                Command::Propose {
                    targets: vec![Edit {
                        target: target(),
                        value: Value::Integer(-3000)
                    }]
                },
                100
            )
            .outcome
            .unwrap()
            .kind,
            "applied"
        );
    }
    fn preview(&mut self, label: &str, now: u64) -> String {
        self.command(
            label,
            Command::PreviewRelease {
                targets: vec![target()],
            },
            now,
        )
        .outcome
        .unwrap()
        .body
        .preview
        .unwrap()
        .token
    }
}
fn execute_preview() -> Json {
    let mut cancel = Flow::new();
    cancel.setup();
    let token = cancel.preview("preview_to_cancel", 100);
    let held = cancel.engine.mixer().coefficients();
    assert_eq!(
        cancel
            .command(
                "cancel",
                Command::CancelPreview {
                    token: token.clone()
                },
                101
            )
            .outcome
            .unwrap()
            .kind,
        "applied"
    );
    assert_eq!(
        cancel
            .command(
                "canceled_token_refused",
                Command::ReleasePreview { token },
                102
            )
            .outcome
            .unwrap()
            .kind,
        "rejected"
    );
    assert_eq!(cancel.engine.mixer().coefficients(), held);
    let token = cancel.preview("preview_to_commit", 103);
    let renew = cancel
        .command("renew", Command::Renew {}, 104)
        .outcome
        .unwrap();
    assert_eq!(renew.body.scope, Some(Scope::Foh));
    assert_eq!(renew.body.lease_remaining_ms, Some(2000));
    assert!(renew.body.granted_lease.is_none());
    assert_eq!(
        cancel
            .command("release_pending", Command::ReleasePreview { token }, 105)
            .state,
        "pending"
    );
    cancel.pump("release_tick_before_boundary", 105);
    let committed = cancel.pump("release_tick_committed", 105);
    assert_eq!(committed[0].outcome.as_ref().unwrap().kind, "applied");
    assert!(
        cancel
            .engine
            .snapshot()
            .unwrap()
            .authority
            .parameters
            .iter()
            .find(|p| p.target == target())
            .unwrap()
            .hold
            .is_none()
    );
    for i in 0..5 {
        cancel.pump(&format!("release_ramp_tick_{i}"), 106);
    }
    assert_eq!(
        cancel.engine.mixer().coefficients(),
        cancel.engine.mixer().targets()
    );

    let mut expiry = Flow::new();
    expiry.setup();
    let token = expiry.preview("preview_to_expire", 100);
    expiry.command("renew_before_expiry", Command::Renew {}, 1000);
    let held = expiry.engine.mixer().targets();
    assert_eq!(
        expiry
            .command(
                "release_just_before_expiry",
                Command::ReleasePreview { token },
                2099
            )
            .state,
        "pending"
    );
    expiry.pump("expiry_tick_before_boundary", 2101);
    let refused = expiry.pump("expiry_tick_refused", 2101);
    assert_eq!(refused[0].outcome.as_ref().unwrap().kind, "conflict");
    assert_eq!(expiry.engine.mixer().targets(), held);
    assert!(
        expiry
            .engine
            .snapshot()
            .unwrap()
            .authority
            .parameters
            .iter()
            .find(|p| p.target == target())
            .unwrap()
            .hold
            .is_some()
    );
    json!({"capability":"GP03-rendered:1","amendment":"preview-flow:1","cancel_and_commit":cancel.events,"expiry_with_live_lease":expiry.events})
}
#[test]
fn replay_actual_preview_flow_corpus() {
    let fixture: Json = serde_json::from_slice(
        &std::fs::read("tests/fixtures/gp03/preview-v1/preview-flow.json").unwrap(),
    )
    .unwrap();
    assert_eq!(execute_preview(), fixture);
}
#[test]
#[ignore = "one-time preview amendment generation; run explicitly when its evidence changes"]
fn regenerate_gp03_preview_corpus() {
    let dir = Path::new("tests/fixtures/gp03/preview-v1");
    std::fs::create_dir_all(dir).unwrap();
    let bytes = serde_json::to_vec_pretty(&execute_preview()).unwrap();
    std::fs::write(dir.join("preview-flow.json"), &bytes).unwrap();
    let source: serde_json::Map<String, Json> = [
        "src/control_model.rs",
        "src/mixer.rs",
        "src/mixer_control.rs",
        "tests/gp03_preview_corpus.rs",
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
    let manifest = json!({"producer":"actual OfflineEngine::handle_encoded/process; each pump48frames synthetic input01=0.125","capability":"GP03-rendered:1","amendment":"preview-flow:1","baseline":"99eb0f08050a9a86039af9f1ba2a3541649ca7b6","contract_sha256":"54a887f4a300d049d3ba67077a2e0446f7a92c57c41cef9d3b7da13735a092ac","source_sha256":source,"fixture_sha256":gigpies::hash::hex(Sha256::digest(bytes)),"retained_original_fixture_sha256":"f41286ddabb915f6a18231e0de6e163b2926d44b926414c5e31dceb5755c97e1"});
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}
