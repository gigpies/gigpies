//! Strict GP07-processing:2 envelope, independent of legacy wire bytes.
use crate::{
    channel_processing::Config,
    control_model::{Command, Request},
    mixer_control::RequestContext,
    show::{Counter, Result},
};
use serde::{Deserialize, Serialize};
pub const CONTRACT: &str = "GP07-processing";
pub const FOH_TAP: &str = "foh-post-eq-dynamics-v2";
pub const PER_SEND_TAP: &str = "per-send-gp18-v1";
pub const MONITOR_TAP: &str = "raw-post-mute-v1";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    deny_unknown_fields,
    rename_all = "snake_case"
)]
pub enum ProcessingCommand {
    ProcessingSnapshot {},
    ProcessingSet { input: String, config: Config },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessingRequest {
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
    pub command: ProcessingCommand,
}
fn keys(v: &serde_json::Value, expected: &[&str]) -> Result<()> {
    let o = v.as_object().ok_or("object")?;
    if o.len() != expected.len() || o.keys().any(|k| !expected.contains(&k.as_str())) {
        return Err("fields".into());
    }
    Ok(())
}
impl ProcessingRequest {
    pub fn authority_request(&self) -> Request {
        Request {
            contract: "C-AUDIO".into(),
            version: if self.version >= 3 { 2 } else { 1 },
            show_id: self.show_id.clone(),
            module: self.module.clone(),
            epoch: self.epoch,
            writer: self.writer.clone(),
            lease: self.lease,
            request_id: self.request_id,
            expected_revision: self.expected_revision,
            command: if matches!(self.command, ProcessingCommand::ProcessingSnapshot {}) {
                Command::Snapshot {}
            } else {
                Command::Renew {}
            },
        }
    }
    pub(crate) fn history_request(&self) -> Request {
        let mut r = self.authority_request();
        r.contract = CONTRACT.into();
        r.version = self.version;
        r
    }
    pub fn context(&self) -> RequestContext {
        RequestContext::request(&self.authority_request())
    }
    pub fn validate(&self) -> Result<()> {
        if self.contract != CONTRACT || ![2, 3, 4, 5].contains(&self.version) {
            return Err("version".into());
        }
        self.authority_request().encode()?;
        if let ProcessingCommand::ProcessingSet { input, config } = &self.command {
            if self.version == 2 && input_index(input)? >= 8 {
                return Err("target".into());
            }
            input_index(input)?;
            config.validate()?;
            if self.version < 5 && !config.gate.is_default() {
                return Err("gate requires processing v5".into());
            }
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value = crate::show::decode(bytes)?;
        keys(
            &v,
            &[
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
            ],
        )?;
        if v["contract"] != CONTRACT || !matches!(v["version"].as_u64(), Some(2..=5)) {
            return Err("version".into());
        }
        if v["version"].as_u64().is_some_and(|v| v < 5) && v["body"]["config"].get("gate").is_some()
        {
            return Err("gate requires processing v5".into());
        }
        let r: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        r.validate()?;
        Ok(r)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }
}
/// Validated outer request for a version this provider does not implement.
/// The opaque body is never interpreted as a supported Config or admitted to authority.
#[derive(Debug, Clone)]
pub struct UnsupportedRequest {
    version: u32,
    context: RequestContext,
}
impl UnsupportedRequest {
    pub fn decode(bytes: &[u8]) -> Result<Option<Self>> {
        Self::decode_for(bytes, 2)
    }
    pub fn decode_for(bytes: &[u8], supported_version: u32) -> Result<Option<Self>> {
        if ![2, 3, 4, 5].contains(&supported_version) {
            return Err("provider version".into());
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Outer {
            contract: String,
            version: u32,
            show_id: String,
            module: String,
            epoch: Counter,
            writer: Option<String>,
            lease: Option<Counter>,
            request_id: Option<Counter>,
            expected_revision: Option<Counter>,
            kind: String,
            body: serde_json::Value,
        }
        let v: serde_json::Value = crate::show::decode(bytes)?;
        keys(
            &v,
            &[
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
            ],
        )?;
        let o: Outer = serde_json::from_value(v).map_err(|e| e.to_string())?;
        if o.contract != CONTRACT {
            return Err("contract".into());
        }
        let command = match o.kind.as_str() {
            "processing_snapshot" => {
                keys(&o.body, &[])?;
                Command::Snapshot {}
            }
            "processing_set" if o.body.is_object() => Command::Renew {},
            _ => return Err("kind/body".into()),
        };
        let authority = Request {
            contract: "C-AUDIO".into(),
            version: 1,
            show_id: o.show_id,
            module: o.module,
            epoch: o.epoch,
            writer: o.writer,
            lease: o.lease,
            request_id: o.request_id,
            expected_revision: o.expected_revision,
            command,
        };
        authority.encode()?;
        if o.version == supported_version || (supported_version == 4 && o.version == 5) {
            return Ok(None);
        }
        Ok(Some(Self {
            version: o.version,
            context: RequestContext::request(&authority),
        }))
    }
    /// Dedicated serializer: supported-v2 reply validation cannot emit legacy snapshots.
    pub fn refusal(&self, revision: Counter) -> serde_json::Value {
        serde_json::json!({"contract": CONTRACT, "version": self.version,
            "context": self.context, "state": "final", "reason": "unsupported_version",
            "ticket": null, "effective_frame": null, "ramp_frames": null,
            "revision": revision, "snapshot": null})
    }
}
pub fn input_index(input: &str) -> Result<usize> {
    let n = input
        .strip_prefix("input-")
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|&n| n > 0)
        .ok_or("target")?;
    if input != format!("input-{n:02}") {
        return Err("target".into());
    }
    Ok(usize::from(n) - 1)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub input: String,
    pub current: Config,
    pub target: Config,
    pub transition_remaining_frames: u32,
    pub ready: bool,
    pub gain_reduction_mdb: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<crate::gate::Observation>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingSnapshot {
    pub show_id: String,
    pub epoch: Counter,
    pub revision: Counter,
    pub sequence: Counter,
    pub frame: Counter,
    pub sample_rate: u32,
    pub foh_tap: String,
    pub monitor_tap: String,
    pub faulted: bool,
    pub channels: Vec<Channel>,
}
impl ProcessingSnapshot {
    pub fn validate(&self) -> Result<()> {
        Request {
            contract: "C-AUDIO".into(),
            version: 1,
            show_id: self.show_id.clone(),
            module: "audio".into(),
            epoch: self.epoch,
            writer: None,
            lease: None,
            request_id: None,
            expected_revision: None,
            command: Command::Snapshot {},
        }
        .encode()?;
        if self.sequence.0 == 0
            || self.sample_rate != 48000
            || self.foh_tap != FOH_TAP
            || ![MONITOR_TAP, PER_SEND_TAP].contains(&self.monitor_tap.as_str())
            || self.channels.is_empty()
            || self.channels.len() > u16::MAX as usize
        {
            return Err("processing snapshot".into());
        }
        for (i, c) in self.channels.iter().enumerate() {
            if input_index(&c.input)? != i {
                return Err("inventory".into());
            }
            if c.gate.is_some() != (c.ready && !self.faulted && !c.target.gate.is_default())
                || c.gate.is_some_and(|g| g.attenuation_mdb > 90000)
                || (self.faulted && c.gate.is_some())
            {
                return Err("gate observation".into());
            }
            c.current.validate()?;
            c.target.validate()?;
            if c.transition_remaining_frames > 240
                || c.ready != (c.transition_remaining_frames == 0)
                || (c.ready && c.current != c.target)
            {
                return Err("transition".into());
            }
            let valid = c.ready && !c.target.compressor_bypass && !self.faulted;
            if valid != c.gain_reduction_mdb.is_some()
                || c.gain_reduction_mdb
                    .is_some_and(|v| !(0..=240000).contains(&v))
            {
                return Err("gain reduction".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessingReply {
    pub contract: String,
    pub version: u32,
    pub context: RequestContext,
    pub state: String,
    pub reason: Option<String>,
    pub ticket: Option<Counter>,
    pub effective_frame: Option<Counter>,
    pub ramp_frames: Option<u32>,
    pub revision: Counter,
    pub snapshot: Option<ProcessingSnapshot>,
}
impl ProcessingReply {
    pub(crate) fn new(
        r: &ProcessingRequest,
        state: &str,
        reason: Option<String>,
        revision: Counter,
    ) -> Self {
        Self {
            contract: CONTRACT.into(),
            version: r.version,
            context: r.context(),
            state: state.into(),
            reason,
            ticket: None,
            effective_frame: None,
            ramp_frames: None,
            revision,
            snapshot: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.contract != CONTRACT || ![2, 3, 4, 5].contains(&self.version) {
            return Err("version".into());
        }
        self.context.validate()?;
        let mutation = self.context.writer.is_some()
            && self.context.lease.is_some()
            && self.context.request_id.is_some()
            && self.context.expected_revision.is_some();
        let read = self.context.writer.is_none()
            && self.context.lease.is_none()
            && self.context.request_id.is_none()
            && self.context.expected_revision.is_none();
        if !mutation && !read {
            return Err("processing context".into());
        }
        let no_timing =
            self.ticket.is_none() && self.effective_frame.is_none() && self.ramp_frames.is_none();
        let timing = self.ticket.is_some_and(|t| t.0 > 0)
            && self
                .effective_frame
                .is_some_and(|f| f.0 > 0 && f.0 % 48 == 0)
            && self.ramp_frames == Some(240);
        if self.reason.as_ref().is_some_and(|s| {
            s.is_empty() || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        }) {
            return Err("reason".into());
        }
        match self.state.as_str() {
            "pending"
                if mutation
                    && timing
                    && self.reason.is_none()
                    && self.snapshot.is_none()
                    && self.context.expected_revision == Some(self.revision) => {}
            "backpressure"
                if mutation && no_timing && self.reason.is_none() && self.snapshot.is_none() => {}
            "final" if self.reason.is_some() && no_timing && self.snapshot.is_none() => (),
            "final" if self.reason.is_none() => {
                let s = self.snapshot.as_ref().ok_or("snapshot")?;
                s.validate()?;
                if self.version < 5
                    && s.channels.iter().any(|c| {
                        !c.current.gate.is_default()
                            || !c.target.gate.is_default()
                            || c.gate.is_some()
                    })
                {
                    return Err("gate requires processing v5".into());
                }
                if s.monitor_tap
                    != if self.version >= 4 {
                        PER_SEND_TAP
                    } else {
                        MONITOR_TAP
                    }
                {
                    return Err("monitor tap version".into());
                }
                if self.version == 2 && s.channels.len() != 8 {
                    return Err("legacy inventory".into());
                }
                if s.show_id != self.context.show_id
                    || s.epoch != self.context.epoch
                    || s.revision != self.revision
                {
                    return Err("snapshot identity".into());
                }
                if mutation {
                    if !timing
                        || self
                            .context
                            .expected_revision
                            .and_then(|v| v.0.checked_add(1))
                            != Some(self.revision.0)
                        || self.effective_frame.is_none_or(|f| f.0 >= s.frame.0)
                    {
                        return Err("commit timing".into());
                    }
                } else if self.context.writer.is_some() || !no_timing {
                    return Err("snapshot context".into());
                }
            }
            _ => return Err("reply state".into()),
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::decode_assembled(bytes)
    }
    pub fn decode_assembled(bytes: &[u8]) -> Result<Self> {
        let v: serde_json::Value =
            crate::show::decode_bounded(bytes, crate::snapshot_pages::ASSEMBLY_BYTES)?;
        keys(
            &v,
            &[
                "contract",
                "version",
                "context",
                "state",
                "reason",
                "ticket",
                "effective_frame",
                "ramp_frames",
                "revision",
                "snapshot",
            ],
        )?;
        keys(
            &v["context"],
            &[
                "show_id",
                "module",
                "epoch",
                "writer",
                "lease",
                "request_id",
                "expected_revision",
            ],
        )?;
        if !v["snapshot"].is_null() {
            let s = &v["snapshot"];
            keys(
                s,
                &[
                    "show_id",
                    "epoch",
                    "revision",
                    "sequence",
                    "frame",
                    "sample_rate",
                    "foh_tap",
                    "monitor_tap",
                    "faulted",
                    "channels",
                ],
            )?;
            for c in s["channels"].as_array().ok_or("channels")? {
                let mut fields = vec![
                    "input",
                    "current",
                    "target",
                    "transition_remaining_frames",
                    "ready",
                    "gain_reduction_mdb",
                ];
                if c.get("gate").is_some() {
                    fields.push("gate");
                }
                keys(c, &fields)?;
            }
        }
        let r: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        r.validate()?;
        Ok(r)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let b = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if b.len()
            > if self.version >= 4 {
                crate::snapshot_pages::ASSEMBLY_BYTES
            } else {
                65536
            }
        {
            return Err("capacity".into());
        }
        Ok(b)
    }
}
