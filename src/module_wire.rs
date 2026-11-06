//! Additive GP05 query/record envelope, independently discriminated from GP03.
//! It reuses the existing authority context; no writable PA/FX controls exist.
use crate::{
    control_model::{Command, Request},
    show::{Counter, Result},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    deny_unknown_fields,
    rename_all = "snake_case"
)]
pub enum ModuleCommand {
    ModuleStatus {},
    RecordStart {
        take_id: String,
        operation_id: Counter,
    },
    RecordStop {
        take_id: String,
        operation_id: Counter,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleRequest {
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
    pub command: ModuleCommand,
}
impl ModuleRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value = crate::show::decode(bytes)?;
        let object = v.as_object().ok_or("envelope")?;
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
        if object.len() != keys.len() || object.keys().any(|k| !keys.contains(&k.as_str())) {
            return Err("module envelope fields".into());
        }
        let r: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<()> {
        if self.contract != "GP05-modules" || ![1, 2].contains(&self.version) {
            return Err("module version".into());
        }
        self.authority_request().encode()?;
        if let ModuleCommand::RecordStart {
            take_id,
            operation_id,
        }
        | ModuleCommand::RecordStop {
            take_id,
            operation_id,
        } = &self.command
            && (!crate::show::id(take_id) || operation_id.0 == 0)
        {
            return Err("record identity".into());
        }
        Ok(())
    }
    pub fn authority_request(&self) -> Request {
        Request {
            contract: "C-AUDIO".into(),
            version: self.version,
            show_id: self.show_id.clone(),
            module: self.module.clone(),
            epoch: self.epoch,
            writer: self.writer.clone(),
            lease: self.lease,
            request_id: self.request_id,
            expected_revision: self.expected_revision,
            command: if matches!(self.command, ModuleCommand::ModuleStatus {}) {
                Command::Snapshot {}
            } else {
                Command::Renew {}
            },
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        Ok(crate::hash::hex(Sha256::digest(self.encode()?)))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleReply {
    pub contract: String,
    pub version: u32,
    pub show_id: String,
    pub module: String,
    pub epoch: Counter,
    pub writer: Option<String>,
    pub lease: Option<Counter>,
    pub request_id: Option<Counter>,
    pub take_id: String,
    pub operation_id: Counter,
    pub state: String,
    pub reason: Option<String>,
}
impl ModuleReply {
    pub fn new(r: &ModuleRequest, state: &str, reason: Option<String>) -> Self {
        let (take_id, operation_id) = match &r.command {
            ModuleCommand::RecordStart {
                take_id,
                operation_id,
            }
            | ModuleCommand::RecordStop {
                take_id,
                operation_id,
            } => (take_id.clone(), *operation_id),
            _ => (String::new(), Counter(0)),
        };
        Self {
            contract: r.contract.clone(),
            version: r.version,
            show_id: r.show_id.clone(),
            module: r.module.clone(),
            epoch: r.epoch,
            writer: r.writer.clone(),
            lease: r.lease,
            request_id: r.request_id,
            take_id,
            operation_id,
            state: state.into(),
            reason,
        }
    }
}
pub fn unavailable_status(show: &str, epoch: u64, frame: u64) -> serde_json::Value {
    serde_json::json!({"contract":"GP05-modules","version":1,"show_id":show,"module":"audio","epoch":epoch.to_string(),"frame":frame.to_string(),"available":false,"physical":"unverified","protection":"offline-unprotected","readiness":"unavailable","activity":"idle","source_fault":0,"configuration":null,"libraries":{},"fx":null,"pa":null,"recording":null})
}
