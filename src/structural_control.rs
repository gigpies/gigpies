//! Typed structural controls on the existing authority and sample boundary.
//! PA JSON is an opaque versioned owner document, validated by the actual library.
use crate::{
    control_model::{Command as AudioCommand, Request as AudioRequest, Scope},
    mixer_control::RequestContext,
    show::{Counter, Result},
    topology::OutputPort,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Command {
    StructuralSnapshot {},
    SetupSnapshot {},
    SetupHold {},
    SetupRelease {},
    MasterEqSnapshot {},
    MasterEqSet {
        patch_json: String,
        program_buses: Vec<usize>,
        owner_instance: Counter,
        graph_generation: Counter,
        eq_generation: Counter,
        map_revision: Counter,
    },
    PaSet {
        configuration_json: String,
        program_buses: Vec<usize>,
    },
    OutputPatch {
        outputs: Vec<OutputPort>,
    },
    OutputMute {},
    OutputRearm {},
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
impl Request {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = crate::show::decode(bytes)?;
        let object = value.as_object().ok_or("structural envelope")?;
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
            return Err("structural envelope fields".into());
        }
        let request: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        request.validate()?;
        Ok(request)
    }
    pub fn validate(&self) -> Result<()> {
        let eq = matches!(
            self.command,
            Command::MasterEqSnapshot {} | Command::MasterEqSet { .. }
        );
        if self.version != 1
            || (eq && self.contract != crate::master_eq_wire::CONTRACT)
            || (!eq && self.contract != "GP14-structure")
        {
            return Err("structural version".into());
        }
        self.authority_request().encode()?;
        match &self.command {
            Command::MasterEqSet {
                patch_json,
                program_buses,
                owner_instance,
                graph_generation,
                ..
            } if patch_json.is_empty()
                || patch_json.len() > 32768
                || program_buses.is_empty()
                || program_buses.len() > 4096
                || owner_instance.0 == 0
                || graph_generation.0 == 0 =>
            {
                Err("live EQ request bound/identity".into())
            }
            Command::PaSet {
                configuration_json,
                program_buses,
            } if configuration_json.is_empty()
                || configuration_json.len() > 48 * 1024
                || program_buses.is_empty()
                || program_buses.len() > 4096 =>
            {
                Err("PA request resource bound".into())
            }
            Command::OutputPatch { outputs } if outputs.is_empty() || outputs.len() > 4096 => {
                Err("output patch descriptor resource bound".into())
            }
            _ => Ok(()),
        }
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
            command: if matches!(
                self.command,
                Command::StructuralSnapshot {}
                    | Command::SetupSnapshot {}
                    | Command::MasterEqSnapshot {}
            ) {
                AudioCommand::Snapshot {}
            } else {
                AudioCommand::Renew {}
            },
        }
    }
    pub fn scope(&self) -> Option<Scope> {
        match self.command {
            Command::StructuralSnapshot {}
            | Command::SetupSnapshot {}
            | Command::MasterEqSnapshot {} => None,
            Command::OutputPatch { .. } => Some(Scope::OutputRoutes),
            _ => Some(Scope::PaConfiguration),
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("structural frame capacity".into());
        }
        Ok(bytes)
    }
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        Ok(crate::hash::hex(Sha256::digest(self.encode()?)))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub show_id: String,
    pub epoch: Counter,
    pub revision: Counter,
    pub frame: Counter,
    pub topology: crate::topology::EngineTopology,
    pub clock: crate::clock_domain::ClockDomain,
    pub outputs_quiesced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_hold: Option<bool>,
    pub pa_configuration_json: Option<String>,
    pub pa_program_buses: Vec<usize>,
    /// Actual owner status; null means the successor module is unavailable.
    pub pa_status: Option<serde_json::Value>,
    pub pa_capabilities: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub contract: String,
    pub version: u32,
    pub context: RequestContext,
    pub state: String,
    pub reason: Option<String>,
    pub effective_frame: Option<Counter>,
    pub revision: Counter,
    pub snapshot: Option<Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_eq: Option<crate::master_eq_wire::Snapshot>,
}
impl Reply {
    pub fn new(
        request: &Request,
        state: &str,
        reason: Option<String>,
        frame: Option<u64>,
        revision: Counter,
        snapshot: Option<Snapshot>,
    ) -> Self {
        Self {
            contract: request.contract.clone(),
            version: 1,
            context: RequestContext::request(&request.authority_request()),
            state: state.into(),
            reason,
            effective_frame: frame.map(Counter),
            revision,
            snapshot,
            master_eq: None,
        }
    }
}

/// Persistent intent deliberately omits epochs, leases, output arm state, queues
/// and credentials. Restoring uses a fresh durable authority epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub version: u32,
    pub engine: crate::mixer_control::EngineIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brain: Option<crate::brain_control::Intent>,
    pub pa_configuration_json: Option<String>,
    pub pa_program_buses: Vec<usize>,
}
impl Intent {
    pub fn save(&self, directory: &std::path::Path, name: &str) -> Result<()> {
        self.validate()?;
        crate::show::persist_bounded(directory, name, self, 1024 * 1024)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("composed intent capacity".into());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: Self = crate::show::decode_bounded(bytes, 1024 * 1024)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err("composed intent version".into());
        }
        crate::mixer_control::OfflineEngine::restore_intent(&self.engine, Counter(1), 0)?;
        if let Some(brain) = &self.brain {
            brain.validate(&self.engine.topology)?;
        }
        match &self.pa_configuration_json {
            None if self.pa_program_buses.is_empty() => Ok(()),
            Some(json)
                if !json.is_empty()
                    && json.len() <= 48 * 1024
                    && !self.pa_program_buses.is_empty()
                    && self.pa_program_buses.len() <= 4096
                    && self
                        .pa_program_buses
                        .iter()
                        .all(|b| *b < self.engine.topology.monitors + 2) =>
            {
                Ok(())
            }
            _ => Err("composed PA intent shape".into()),
        }
    }
}
