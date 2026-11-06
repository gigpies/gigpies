//! Additive, scoped read-only witness for held talkback. Not a UI snapshot.
use crate::{
    brain_control::MonitorSource,
    control_model::Scope,
    show::{Counter, Result},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const CONTRACT: &str = "GP15-held-proof";
pub const MAX_BYTES: usize = 8192;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub contract: String,
    pub version: u32,
    pub kind: String,
    pub query_id: Counter,
    pub show_id: String,
    pub module: String,
    pub epoch: Counter,
    pub authenticated_session: Counter,
    pub writer: String,
    pub capability_generation: Counter,
    pub map_generation: Counter,
    pub scope: Scope,
    pub lease: Counter,
    pub expected_config_digest: String,
}
impl Request {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let r: Self = crate::show::decode_bounded(bytes, MAX_BYTES)?;
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<()> {
        if self.contract != CONTRACT
            || self.version != 1
            || self.kind != "held_proof"
            || self.module != "audio"
            || self.query_id.0 == 0
            || self.authenticated_session.0 == 0
            || self.capability_generation.0 == 0
            || self.map_generation.0 == 0
            || self.lease.0 == 0
            || !valid_digest(&self.expected_config_digest)
        {
            return Err("held proof envelope".into());
        }
        if !crate::show::id(&self.writer) {
            return Err("held proof writer".into());
        }
        let mut identity = self.authority_request();
        identity.writer = None;
        identity.lease = None;
        identity.encode()?;
        Ok(())
    }
    pub fn authority_request(&self) -> crate::control_model::Request {
        crate::control_model::Request {
            contract: "C-AUDIO".into(),
            version: 1,
            show_id: self.show_id.clone(),
            module: self.module.clone(),
            epoch: self.epoch,
            writer: Some(self.writer.clone()),
            lease: Some(self.lease),
            request_id: None,
            expected_revision: None,
            command: crate::control_model::Command::Snapshot {},
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Identity,
    Permission,
    Scope,
    Lease,
    Clock,
    QueryId,
    ConfigChanged,
    Capacity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brain {
    pub selection_generation: Counter,
    pub hold_generation_counter: Counter,
    pub held_generation: Option<Counter>,
    pub hold_deadline_ms: Option<Counter>,
    pub source: MonitorSource,
    pub monitor_armed: bool,
    pub monitor_mute: bool,
    pub monitor_dim: bool,
    pub talkback_mute: bool,
    pub talkback_foh: bool,
    pub monitor_path_ready: bool,
    pub talkback_path_ready: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub revision: Counter,
    pub source_frame: Counter,
    pub config_digest: String,
    pub lease_remaining_ms: u32,
    pub brain: Brain,
    pub foh_authorized: bool,
    pub media_authorized: bool,
    pub heartbeat_ms: u64,
    pub deadman_ms: u64,
    pub fade_frames: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub contract: String,
    pub version: u32,
    pub state: String,
    pub reason: Option<Reason>,
    pub context: Request,
    pub witness: Option<Witness>,
}
impl Reply {
    pub fn new(context: Request, result: std::result::Result<Witness, Reason>) -> Self {
        let (witness, reason, state) = match result {
            Ok(w) => (Some(w), None, "proof"),
            Err(r) => (None, Some(r), "refused"),
        };
        Self {
            contract: CONTRACT.into(),
            version: 1,
            state: state.into(),
            reason,
            context,
            witness,
        }
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = crate::show::decode_bounded(bytes, MAX_BYTES)?;
        fn exact(value: &serde_json::Value, keys: &[&str]) -> Result<()> {
            let object = value.as_object().ok_or("held proof object")?;
            if object.len() != keys.len() || object.keys().any(|k| !keys.contains(&k.as_str())) {
                return Err("held proof exact fields".into());
            }
            Ok(())
        }
        exact(
            &value,
            &[
                "contract", "version", "state", "reason", "context", "witness",
            ],
        )?;
        if !value["witness"].is_null() {
            exact(
                &value["witness"],
                &[
                    "revision",
                    "source_frame",
                    "config_digest",
                    "lease_remaining_ms",
                    "brain",
                    "foh_authorized",
                    "media_authorized",
                    "heartbeat_ms",
                    "deadman_ms",
                    "fade_frames",
                ],
            )?;
            exact(
                &value["witness"]["brain"],
                &[
                    "selection_generation",
                    "hold_generation_counter",
                    "held_generation",
                    "hold_deadline_ms",
                    "source",
                    "monitor_armed",
                    "monitor_mute",
                    "monitor_dim",
                    "talkback_mute",
                    "talkback_foh",
                    "monitor_path_ready",
                    "talkback_path_ready",
                ],
            )?;
        }
        let r: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<()> {
        self.context.validate()?;
        if self.contract != CONTRACT || self.version != 1 {
            return Err("held proof reply identity".into());
        }
        match (&self.witness, self.reason, self.state.as_str()) {
            (None, Some(_), "refused") => Ok(()),
            (Some(w), None, "proof")
                if self.context.scope == Scope::TalkbackDestinations
                    && w.config_digest == self.context.expected_config_digest
                    && valid_digest(&w.config_digest)
                    && (1..=2000).contains(&w.lease_remaining_ms)
                    && w.heartbeat_ms == 50
                    && w.deadman_ms == 150
                    && w.fade_frames == 240
                    && w.brain.held_generation.is_some() == w.brain.hold_deadline_ms.is_some()
                    && w.brain
                        .held_generation
                        .is_none_or(|g| g.0 > 0 && g.0 <= w.brain.hold_generation_counter.0)
                    && w.brain.hold_deadline_ms.is_none_or(|d| d.0 > 0)
                    && (!w.media_authorized
                        || (w.brain.held_generation.is_some()
                            && !w.brain.talkback_mute
                            && (!w.brain.talkback_foh || w.foh_authorized)))
                    && (!w.brain.talkback_path_ready || w.media_authorized)
                    && (!w.brain.monitor_path_ready
                        || (w.brain.monitor_armed
                            && !w.brain.monitor_mute
                            && w.brain.source != MonitorSource::None))
                    && w.brain.selection_generation.0 > 0
                    && w.source_frame.0.is_multiple_of(48) =>
            {
                Ok(())
            }
            _ => Err("held proof reply shape".into()),
        }
    }
}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Sorted-set hash prepared off render. Persisted intents use the existing
/// topology u16 monitor-index domain; the narrower command cap is separate.
pub fn destination_hash(destinations: &[usize]) -> Result<[u8; 32]> {
    if destinations.len() > usize::from(u16::MAX) {
        return Err("held proof destinations".into());
    }
    #[cfg(test)]
    HASH_CALLS.with(|n| n.set(n.get() + 1));
    let mut sorted = destinations.to_vec();
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0] == w[1]) {
        return Err("held proof destinations".into());
    }
    let mut h = Sha256::new();
    h.update(b"GP15-HELD-PROOF/TB-DESTINATIONS/v1\0");
    h.update((sorted.len() as u32).to_be_bytes());
    for index in sorted {
        h.update(
            u32::try_from(index)
                .map_err(|_| "held proof destination index")?
                .to_be_bytes(),
        );
    }
    Ok(h.finalize().into())
}
/// Canonical common bytes and scalar fields; no matrix/destination traversal.
pub struct Configuration<'a> {
    pub show_id: &'a str,
    pub epoch: u64,
    pub capability: u64,
    pub map: u64,
    pub dimensions: [u32; 6],
    pub destinations: [u8; 32],
    pub gain_cdb: i32,
    pub mute: bool,
    pub foh: bool,
}
pub fn config_digest(c: Configuration<'_>) -> String {
    let mut h = Sha256::new();
    h.update(b"GP15-HELD-PROOF/TB-CONFIG/v1\0");
    h.update((c.show_id.len() as u32).to_be_bytes());
    h.update(c.show_id.as_bytes());
    for n in [c.epoch, c.capability, c.map] {
        h.update(n.to_be_bytes());
    }
    for n in c.dimensions {
        h.update(n.to_be_bytes());
    }
    h.update(c.destinations);
    h.update(c.gain_cdb.to_be_bytes());
    h.update([u8::from(c.mute), u8::from(c.foh)]);
    crate::hash::hex(h.finalize())
}

#[cfg(test)]
thread_local! {pub(crate) static HASH_CALLS:std::cell::Cell<u64>=const{std::cell::Cell::new(0)};}
