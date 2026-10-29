//! GP20 measurement control: review-only PA owner integration, no output command.
use crate::{
    control_model::{Command as AudioCommand, Request as AudioRequest},
    mixer_control::RequestContext,
    show::{Counter, Result},
};
use serde::{Deserialize, Serialize};
pub const CONTRACT: &str = "GP20-measurement";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub max_arrival_samples: usize,
    pub band_hz: [u32; 2],
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_arrival_samples: 2048,
            band_hz: [100, 10000],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Command {
    MeasurementSnapshot {},
    MeasurementResult {
        id: String,
    },
    CaptureStart {
        id: String,
        reference_input: String,
        mic_capture_slot: usize,
        output_index: usize,
        position_id: String,
        samples: usize,
        options: Options,
    },
    CaptureCancel {
        id: String,
    },
    MeasurementOfflineEq {
        id: String,
        evidence_id: String,
    },
    MeasurementEq {
        id: String,
        captures: Vec<String>,
        target: String,
    },
    MeasurementOverlap {
        id: String,
        pairs: Vec<[String; 2]>,
    },
    MeasurementPropose {
        id: String,
        pairs: Vec<[String; 2]>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub contract: String,
    pub version: u32,
    pub show_id: String,
    pub module: String,
    pub epoch: Counter,
    pub writer: Option<String>,
    pub lease: Option<Counter>,
    pub request_id: Option<Counter>,
    pub expected_revision: Option<Counter>,
    #[serde(flatten)]
    pub command: Command,
}
pub fn identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic())
}
impl Request {
    pub fn read(&self) -> bool {
        matches!(
            self.command,
            Command::MeasurementSnapshot {} | Command::MeasurementResult { .. }
        )
    }
    pub fn authority_request(&self) -> AudioRequest {
        AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: self.show_id.clone(),
            module: self.module.clone(),
            epoch: self.epoch,
            writer: self.writer.clone(),
            lease: self.lease,
            request_id: self.request_id,
            expected_revision: self.expected_revision,
            command: if self.read() {
                AudioCommand::Snapshot {}
            } else {
                AudioCommand::Renew {}
            },
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.contract != CONTRACT || !matches!(self.version, 1 | 2) {
            return Err("measurement contract".into());
        }
        if matches!(
            self.command,
            Command::MeasurementOverlap { .. }
                | Command::MeasurementEq { .. }
                | Command::MeasurementOfflineEq { .. }
        ) && self.version != 2
        {
            return Err("measurement operation requires GP20 v2".into());
        }
        self.authority_request().encode()?;
        match &self.command {
            Command::MeasurementSnapshot {} => (),
            Command::MeasurementResult { id } | Command::CaptureCancel { id } => {
                if !identifier(id) {
                    return Err("measurement ID".into());
                }
            }
            Command::CaptureStart {
                id,
                reference_input,
                position_id,
                samples,
                options,
                ..
            } => {
                if !identifier(id)
                    || !identifier(reference_input)
                    || !identifier(position_id)
                    || !(32768..=65536).contains(samples)
                    || !(1..=2048).contains(&options.max_arrival_samples)
                    || options.band_hz[0] < 20
                    || options.band_hz[1] > 20000
                    || options.band_hz[1] <= options.band_hz[0]
                {
                    return Err("capture bounds".into());
                }
            }
            Command::MeasurementOfflineEq { id, evidence_id } => {
                if !identifier(id) || !identifier(evidence_id) {
                    return Err("offline evidence ID".into());
                }
            }
            Command::MeasurementEq {
                id,
                captures,
                target,
            } => {
                if !identifier(id)
                    || captures.is_empty()
                    || captures.len() > 4
                    || captures.iter().any(|id| !identifier(id))
                    || !matches!(
                        target.as_str(),
                        "flat" | "bass_shaped" | "damped_high_frequency"
                    )
                {
                    return Err("EQ proposal bounds".into());
                }
            }
            Command::MeasurementPropose { id, pairs }
            | Command::MeasurementOverlap { id, pairs } => {
                if !identifier(id)
                    || pairs.is_empty()
                    || pairs.len() > 8
                    || pairs.iter().flatten().any(|s| !identifier(s))
                {
                    return Err("proposal bounds".into());
                }
            }
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value = crate::show::decode(bytes)?;
        let o = v.as_object().ok_or("measurement envelope")?;
        let keys = [
            "contract",
            "version",
            "show_id",
            "module",
            "epoch",
            "writer",
            "lease",
            "request_id",
            "expected_revision",
            "kind",
            "body",
        ];
        if o.len() != keys.len() || o.keys().any(|k| !keys.contains(&k.as_str())) {
            return Err("measurement envelope fields".into());
        }
        let r: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        r.validate()?;
        Ok(r)
    }
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        self.validate()?;
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|e| e.to_string())?)
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Basis {
    pub source_epoch: Counter,
    pub map_revision: Counter,
    pub graph_generation: Counter,
    pub clock_domain: String,
    pub configuration_json: String,
    pub program_buses: Vec<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub summary: Summary,
    pub basis: Basis,
    pub owner_result_json: Option<String>,
    pub candidate_configuration_json: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub id: String,
    pub received_samples: usize,
    pub requested_samples: usize,
    pub output_index: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub available: bool,
    pub software_only: bool,
    pub reason: Option<String>,
    pub active_id: Option<String>,
    pub capture_progress: Option<Progress>,
    pub results: Vec<Summary>,
    pub current_basis: Option<Basis>,
    pub reserved_mic_slots: Vec<usize>,
    pub reference_inputs: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub contract: String,
    pub version: u32,
    pub context: RequestContext,
    pub state: String,
    pub reason: Option<String>,
    pub revision: Counter,
    pub effective_frame: Option<Counter>,
    pub snapshot: Option<Snapshot>,
    pub result: Option<Record>,
}
impl Reply {
    pub fn new(r: &Request, state: &str, reason: Option<String>, revision: Counter) -> Self {
        Self {
            contract: CONTRACT.into(),
            version: 1,
            context: RequestContext::request(&r.authority_request()),
            state: state.into(),
            reason,
            revision,
            effective_frame: None,
            snapshot: None,
            result: None,
        }
    }
}
