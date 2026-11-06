//! C-AUDIO:1 encoded, pure authority harness. No DSP, device or rendered ACK.
use crate::show::{Counter, Result, decode, id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const LEASE_MS: u64 = 2000;
/// Bounded live authority/replay state: one writer per transport session budget.
/// This is a deployment resource limit, not a monitor/topology limit.
pub const MAX_LIVE_WRITERS: usize = 16;
pub const SESSION_HISTORY_CAPACITY: usize = 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Foh,
    Monitor1,
    Monitor2,
    Monitor(u16),
    PaConfiguration,
    FxConfiguration,
    OutputRoutes,
    LocalOperatorMonitor,
    TalkbackDestinations,
    TalkbackFoh,
}
impl Scope {
    pub fn monitor(n: usize) -> Self {
        match n {
            1 => Self::Monitor1,
            2 => Self::Monitor2,
            _ => Self::Monitor(n as u16),
        }
    }
    pub fn monitor_index(self) -> Option<usize> {
        match self {
            Self::Monitor1 => Some(0),
            Self::Monitor2 => Some(1),
            Self::Monitor(n) if n >= 3 => Some(n as usize - 1),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Manual,
    Assist,
    Auto,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "parameter", deny_unknown_fields, rename_all = "snake_case")]
pub enum Target {
    MasterGain {},
    Fader { input: String },
    Pan { input: String },
    Mute { input: String },
    Send { input: String, monitor: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub target: Target,
    pub value: Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoBound {
    pub target: Target,
    pub min: Value,
    pub max: Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    deny_unknown_fields,
    rename_all = "snake_case"
)]
pub enum Command {
    Snapshot {},
    Grant { scope: Scope },
    Renew {},
    Release {},
    Set { targets: Vec<Edit> },
    SetMode { mode: Mode, bounds: Vec<AutoBound> },
    Propose { targets: Vec<Edit> },
    PreviewRelease { targets: Vec<Target> },
    CancelPreview { token: String },
    ReleasePreview { token: String },
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
        // Flatten + deny_unknown_fields is not supported reliably by serde;
        // check the complete envelope keys before typed decoding as well.
        let raw: serde_json::Value = decode(bytes)?;
        let obj = raw.as_object().ok_or("envelope")?;
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
        if obj.len() != keys.len() || obj.keys().any(|k| !keys.contains(&k.as_str())) {
            return Err("envelope fields".into());
        }
        let r: Self = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        r.validate()?;
        Ok(r)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let b = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        Self::decode(&b)?;
        Ok(b)
    }
    fn validate(&self) -> Result<()> {
        if self.contract != "C-AUDIO" || ![1, 2].contains(&self.version) {
            return Err("version".into());
        }
        if !canonical_show(&self.show_id)
            || self.module != "audio"
            || self.epoch.0 == 0
            || self.writer.as_ref().is_some_and(|w| !id(w))
        {
            return Err("identity".into());
        }
        match self.command {
            Command::Snapshot {}
                if self.writer.is_none()
                    && self.lease.is_none()
                    && self.request_id.is_none()
                    && self.expected_revision.is_none() => {}
            Command::Grant { .. }
                if self.writer.is_some()
                    && self.lease.is_none()
                    && self.request_id.is_some_and(|n| n.0 > 0)
                    && self.expected_revision.is_some() => {}
            Command::Snapshot {} | Command::Grant { .. } => return Err("authority fields".into()),
            _ if self.writer.is_some()
                && self.lease.is_some_and(|n| n.0 > 0)
                && self.request_id.is_some_and(|n| n.0 > 0)
                && self.expected_revision.is_some() => {}
            _ => return Err("authority fields".into()),
        }
        match &self.command {
            Command::Set { targets } | Command::Propose { targets } => {
                for e in targets {
                    e.target.validate_identity()?;
                }
            }
            Command::SetMode { bounds, .. } => {
                for b in bounds {
                    b.target.validate_identity()?;
                }
            }
            Command::PreviewRelease { targets } => {
                for t in targets {
                    t.validate_identity()?;
                }
            }
            Command::CancelPreview { token } | Command::ReleasePreview { token } if !id(token) => {
                return Err("identity".into());
            }
            _ => (),
        }
        Ok(())
    }
}
fn canonical_show(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub target: Target,
    pub actual: Option<Value>,
    pub target_value: Value,
    pub proposal: Option<Value>,
    pub hold: Option<Value>,
    pub owner: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub token: String,
    pub revision: Counter,
    pub scope: Scope,
    pub remaining_ms: u32,
    pub ramp_frames: u32,
    pub destinations: Vec<Edit>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub show_id: String,
    pub epoch: Counter,
    pub revision: Counter,
    pub sequence: Counter,
    pub page: u8,
    pub page_count: u8,
    pub durability: String,
    pub validity: String,
    pub acquisition_frame: Option<Counter>,
    pub age_ms: Option<u32>,
    pub rendered_application: bool,
    pub release_commit: bool,
    pub session_history_capacity: u32,
    pub inputs: Vec<String>,
    pub monitors: Vec<String>,
    pub modes: Vec<(Scope, Mode)>,
    pub automation_bounds: Vec<AutoBound>,
    pub parameters: Vec<Parameter>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyBody {
    pub revision: Counter,
    pub reason: Option<String>,
    pub lease_remaining_ms: Option<u32>,
    pub granted_lease: Option<Counter>,
    pub scope: Option<Scope>,
    pub preview: Option<Preview>,
    pub snapshot: Option<Snapshot>,
    pub effective_frame: Option<Counter>,
    pub ramp_frames: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub contract: String,
    pub version: u32,
    pub show_id: String,
    pub module: String,
    pub epoch: Counter,
    pub writer: Option<String>,
    pub lease: Option<Counter>,
    pub request_id: Option<Counter>,
    pub expected_revision: Option<Counter>,
    pub kind: String,
    pub body: ReplyBody,
}
impl Reply {
    pub fn decode_versioned(bytes: &[u8]) -> Result<Self> {
        let r: Self = crate::show::decode_bounded(bytes, crate::snapshot_pages::ASSEMBLY_BYTES)?;
        r.validate()?;
        Ok(r)
    }
    /// Historical C-AUDIO1 decoder; expanded clients use decode_versioned.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let r: Self = decode(bytes)?;
        if r.version != 1 {
            return Err("legacy reply version".into());
        }
        r.validate()?;
        Ok(r)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let b = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if b.len() > 65536 {
            return Err("capacity".into());
        }
        Ok(b)
    }
}
type MaintenanceOutcome =
    std::result::Result<crate::lease_maintenance::Maintained, crate::lease_maintenance::Reason>;
#[derive(Debug, Clone)]
struct Session {
    scope: Scope,
    lease: Counter,
    expires: u64,
    high: u64,
    cache: VecDeque<(Request, Reply)>,
    maintenance_high: u64,
    maintenance_cache: VecDeque<(crate::lease_maintenance::Request, MaintenanceOutcome)>,
}
#[derive(Debug, Clone)]
struct StoredPreview {
    preview: Preview,
    writer: String,
    lease: Counter,
    expires: u64,
}
/// Injected monotonic milliseconds; all operations synchronous, at most one mutation
/// per writer. Pending DSP work is unavailable, rather than falsely acknowledged.
#[derive(Debug, Clone)]
pub struct Authority {
    show: String,
    epoch: Counter,
    revision: Counter,
    now: u64,
    next_lease: u64,
    next_preview: u64,
    sequence: u64,
    sessions: BTreeMap<String, Session>,
    retired: BTreeSet<String>,
    previews: BTreeMap<String, StoredPreview>,
    parameters: Vec<Parameter>,
    modes: BTreeMap<Scope, Mode>,
    automation_bounds: BTreeMap<Scope, Vec<AutoBound>>,
    rendered_release: bool,
    inputs: usize,
    monitors: usize,
    version: u32,
}
impl Target {
    fn validate_identity(&self) -> Result<()> {
        if !id(self.input()) {
            return Err("identity".into());
        }
        if let Self::Send { monitor, .. } = self
            && !id(monitor)
        {
            return Err("identity".into());
        }
        Ok(())
    }
    pub(crate) fn scope(&self) -> Result<Scope> {
        match self {
            Self::Send { monitor, .. } => match monitor.as_str() {
                "monitor-1" => Ok(Scope::Monitor1),
                "monitor-2" => Ok(Scope::Monitor2),
                _ => {
                    let n = monitor
                        .strip_prefix("monitor-")
                        .and_then(|s| s.parse::<u16>().ok())
                        .filter(|&n| n >= 3)
                        .ok_or("target")?;
                    if monitor != &format!("monitor-{n}") {
                        return Err("target".into());
                    }
                    Ok(Scope::Monitor(n))
                }
            },
            _ => Ok(Scope::Foh),
        }
    }
    fn input(&self) -> &str {
        match self {
            Self::MasterGain {} => "main",
            Self::Fader { input }
            | Self::Pan { input }
            | Self::Mute { input }
            | Self::Send { input, .. } => input,
        }
    }
    fn value_valid(&self, v: &Value) -> bool {
        match (self, v) {
            (Self::MasterGain {}, Value::Integer(n)) => (-60000..=6000).contains(n),
            (Self::Mute { .. }, Value::Boolean(_)) => true,
            (Self::Pan { .. }, Value::Integer(n)) => (-100..=100).contains(n),
            (Self::Fader { .. } | Self::Send { .. }, Value::Integer(n)) => {
                (-60000..=12000).contains(n) && n % 100 == 0
            }
            _ => false,
        }
    }
}
impl Authority {
    pub fn new(show_id: &str, epoch: Counter, revision: Counter) -> Result<Self> {
        Self::with_dimensions(show_id, epoch, revision, 8, 2)
    }
    pub fn with_dimensions(
        show_id: &str,
        epoch: Counter,
        revision: Counter,
        inputs: usize,
        monitors: usize,
    ) -> Result<Self> {
        crate::topology::EngineTopology::software(inputs, monitors, 0)?;
        if !canonical_show(show_id) || epoch.0 == 0 {
            return Err("identity".into());
        }
        let mut parameters = Vec::new();
        for i in 1..=inputs {
            let input = format!("input-{i:02}");
            let mut values = vec![
                (
                    Target::Fader {
                        input: input.clone(),
                    },
                    Value::Integer(-6000),
                ),
                (
                    Target::Pan {
                        input: input.clone(),
                    },
                    Value::Integer(0),
                ),
                (
                    Target::Mute {
                        input: input.clone(),
                    },
                    Value::Boolean(false),
                ),
            ];
            values.extend((1..=monitors).map(|n| {
                (
                    Target::Send {
                        input: input.clone(),
                        monitor: format!("monitor-{n}"),
                    },
                    Value::Integer(if n == 1 { 0 } else { -60000 }),
                )
            }));
            for (target, value) in values {
                parameters.push(Parameter {
                    target,
                    actual: None,
                    target_value: value,
                    proposal: None,
                    hold: None,
                    owner: None,
                });
            }
        }
        Ok(Self {
            show: show_id.into(),
            epoch,
            revision,
            now: 0,
            next_lease: 1,
            next_preview: 1,
            sequence: 0,
            sessions: BTreeMap::new(),
            retired: BTreeSet::new(),
            previews: BTreeMap::new(),
            parameters,
            automation_bounds: BTreeMap::new(),
            rendered_release: false,
            inputs,
            monitors,
            version: if inputs == 8 && monitors == 2 { 1 } else { 2 },
            modes: std::iter::once((Scope::Foh, Mode::Manual))
                .chain((1..=monitors).map(|n| (Scope::monitor(n), Mode::Manual)))
                .collect(),
        })
    }
    /// Only the owner calls this after authenticating the attachment and permission.
    /// Replay state lives in the existing bounded live session and follows clones.
    pub(crate) fn maintain_lease(
        &mut self,
        r: &crate::lease_maintenance::Request,
        now: u64,
        source_frame: u64,
        busy: bool,
    ) -> std::result::Result<crate::lease_maintenance::Maintained, crate::lease_maintenance::Reason>
    {
        use crate::lease_maintenance::{Maintained, Reason};
        if r.show_id != self.show || r.epoch != self.epoch {
            return Err(Reason::Identity);
        }
        if !r.supported_scope() {
            return Err(Reason::Scope);
        }
        if now < self.now {
            return Err(Reason::Clock);
        }
        // Observe authenticated owner time even on a terminal lease refusal;
        // a later regressed timestamp must not revive an expired cached success.
        self.now = now;
        let session = self.sessions.get_mut(&r.writer).ok_or(Reason::Lease)?;
        if session.lease != r.lease || session.expires <= now {
            return Err(Reason::Lease);
        }
        if session.scope != r.scope {
            return Err(Reason::Scope);
        }
        if let Some((old, result)) = session
            .maintenance_cache
            .iter()
            .find(|(old, _)| old.maintenance_id == r.maintenance_id)
        {
            return if old == r {
                result.clone()
            } else {
                Err(Reason::ReusedId)
            };
        }
        if r.maintenance_id.0 <= session.maintenance_high {
            return Err(Reason::ExpiredId);
        }
        let result = if busy {
            Err(Reason::Unavailable)
        } else if let Some(expiry) = now.checked_add(LEASE_MS) {
            session.expires = expiry;
            Ok(Maintained {
                revision: self.revision,
                source_frame: Counter(source_frame),
                lease_remaining_ms: LEASE_MS as u32,
            })
        } else {
            Err(Reason::Clock)
        };
        session.maintenance_high = r.maintenance_id.0;
        if session.maintenance_cache.len() == 64 {
            session.maintenance_cache.pop_front();
        }
        session
            .maintenance_cache
            .push_back((r.clone(), result.clone()));
        result
    }
    pub(crate) fn observe_control_time(&mut self, now: u64) -> Result<()> {
        if now < self.now {
            return Err("clock".into());
        }
        self.now = now;
        Ok(())
    }
    pub(crate) fn preserve_observations(&mut self, live: &Self) {
        self.now = self.now.max(live.now);
        self.sequence = self.sequence.max(live.sequence);
    }
    pub(crate) fn enable_rendered_release(&mut self) {
        self.rendered_release = true;
    }
    pub(crate) fn writer_live(&self, writer: &str, now: u64) -> bool {
        self.sessions.get(writer).is_some_and(|s| s.expires > now)
    }
    /// Read-only scoped witness; shares the live lease check and never extends it.
    pub(crate) fn live_lease_witness(
        &self,
        r: &Request,
        now: u64,
    ) -> std::result::Result<(Scope, u32), crate::held_proof::Reason> {
        if now < self.now {
            return Err(crate::held_proof::Reason::Clock);
        }
        let scope = self
            .live_scope(r, now)
            .ok_or(crate::held_proof::Reason::Lease)?;
        let session = self
            .sessions
            .get(r.writer.as_ref().ok_or(crate::held_proof::Reason::Lease)?)
            .ok_or(crate::held_proof::Reason::Lease)?;
        let remaining = session
            .expires
            .checked_sub(now)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| (1..=2000).contains(v))
            .ok_or(crate::held_proof::Reason::Lease)?;
        Ok((scope, remaining))
    }
    pub(crate) fn live_scope(&self, r: &Request, now: u64) -> Option<Scope> {
        let session = self.sessions.get(r.writer.as_ref()?)?;
        (now >= self.now && session.expires > now && r.lease == Some(session.lease))
            .then_some(session.scope)
    }
    pub(crate) fn cached(&self, r: &Request, now: u64) -> Option<Reply> {
        self.live_scope(r, now)?;
        self.sessions
            .get(r.writer.as_ref()?)?
            .cache
            .iter()
            .find(|(old, _)| old == r)
            .map(|(_, p)| p.clone())
    }
    pub(crate) fn install_rendered_reply(&mut self, r: &Request, p: &Reply) {
        if let Some(s) = r.writer.as_ref().and_then(|w| self.sessions.get_mut(w))
            && let Some((_, reply)) = s.cache.iter_mut().find(|(old, _)| old == r)
        {
            *reply = p.clone();
        }
    }
    pub fn revision(&self) -> Counter {
        self.revision
    }
    /// GP07 uses the same high-water/cache as C-AUDIO, with a distinct internal
    /// contract marker. This transaction never extends the writer's lease.
    pub(crate) fn processing_identity(&self, r: &Request, now: u64) -> Option<&'static str> {
        if r.show_id != self.show {
            Some("wrong_show")
        } else if r.epoch != self.epoch {
            Some("epoch")
        } else if now < self.now {
            Some("clock")
        } else if self.live_scope(r, now).is_none() {
            Some("lease")
        } else {
            None
        }
    }
    /// Consume a cancelled external reservation in the same bounded request
    /// history. Expired/retired leases cannot become writable again.
    pub(crate) fn cancel_external(&mut self, request: &Request, scope: Scope) {
        self.scoped_transaction(request, self.now, Some("configuration_failed"), scope);
    }
    pub(crate) fn processing_transaction(
        &mut self,
        r: &Request,
        now: u64,
        refusal: Option<&str>,
    ) -> Reply {
        self.scoped_transaction(r, now, refusal, Scope::Foh)
    }
    pub(crate) fn scoped_transaction(
        &mut self,
        r: &Request,
        now: u64,
        refusal: Option<&str>,
        scope: Scope,
    ) -> Reply {
        if let Some(reason) = self.processing_identity(r, now) {
            return self.reply(r, "rejected", Some(reason));
        }
        self.now = now;
        let writer = r.writer.as_ref().expect("validated extension");
        let session = &self.sessions[writer];
        if let Some((old, p)) = session
            .cache
            .iter()
            .find(|(old, _)| old.request_id == r.request_id)
        {
            return if old == r {
                p.clone()
            } else {
                self.reply(r, "rejected", Some("reused_id"))
            };
        }
        let number = r.request_id.expect("validated extension").0;
        if number <= session.high {
            return self.reply(r, "rejected", Some("expired_id"));
        }
        let reason = if session.scope != scope {
            Some("scope")
        } else if r.expected_revision != Some(self.revision) {
            Some("stale_revision")
        } else if self.revision.0 == u64::MAX {
            Some("capacity")
        } else {
            refusal
        };
        if reason.is_none() {
            self.revision.0 += 1;
        }
        let reply = self.reply(
            r,
            if reason.is_none() {
                "applied"
            } else {
                "rejected"
            },
            reason,
        );
        let session = self.sessions.get_mut(writer).expect("live session");
        session.high = number;
        session.cache.push_back((r.clone(), reply.clone()));
        if session.cache.len() > 64 {
            session.cache.pop_front();
        }
        reply
    }
    pub(crate) fn set_wire_version(&mut self, version: u32) {
        self.version = version;
    }
    pub fn version(&self) -> u32 {
        self.version
    }
    pub fn dimensions(&self) -> (usize, usize) {
        (self.inputs, self.monitors)
    }
    pub(crate) fn enable_master(&mut self) {
        if !self
            .parameters
            .iter()
            .any(|p| p.target == (Target::MasterGain {}))
        {
            self.parameters.push(Parameter {
                target: Target::MasterGain {},
                actual: None,
                target_value: Value::Integer(0),
                proposal: None,
                hold: None,
                owner: None,
            });
        }
    }
    pub(crate) fn has_master(&self) -> bool {
        self.parameters
            .iter()
            .any(|p| p.target == (Target::MasterGain {}))
    }
    pub(crate) fn restore_parameters(&mut self, edits: &[Edit]) -> Result<()> {
        let mut required = required_targets_for(self.inputs, self.monitors);
        if self.has_master() {
            required.insert(Target::MasterGain {});
        }
        if edits.len() != required.len()
            || edits
                .iter()
                .map(|e| e.target.clone())
                .collect::<BTreeSet<_>>()
                != required
            || edits.iter().any(|e| !e.target.value_valid(&e.value))
        {
            return Err("intent parameter inventory".into());
        }
        for p in &mut self.parameters {
            p.target_value = edits
                .iter()
                .find(|e| e.target == p.target)
                .unwrap()
                .value
                .clone();
            p.hold = None;
            p.owner = None;
            p.proposal = None;
        }
        Ok(())
    }
    pub fn revoke_all(&mut self) {
        let writers: Vec<_> = self.sessions.keys().cloned().collect();
        for w in writers {
            self.revoke_writer(&w);
        }
    }
    pub fn revoke_writer(&mut self, writer: &str) {
        if self.sessions.remove(writer).is_some() {
            self.retired.insert(writer.into());
        }
        self.previews.retain(|_, p| p.writer != writer);
    }
    pub fn state(&self) -> (&[Parameter], &BTreeMap<Scope, Mode>) {
        (&self.parameters, &self.modes)
    }
    pub(crate) fn reply(&self, r: &Request, kind: &str, reason: Option<&str>) -> Reply {
        Reply {
            contract: "C-AUDIO".into(),
            version: self.version,
            show_id: self.show.clone(),
            module: "audio".into(),
            epoch: self.epoch,
            writer: r.writer.clone(),
            lease: r.lease,
            request_id: r.request_id,
            expected_revision: r.expected_revision,
            kind: kind.into(),
            body: ReplyBody {
                revision: self.revision,
                reason: reason.map(str::to_owned),
                lease_remaining_ms: None,
                granted_lease: None,
                scope: None,
                preview: None,
                snapshot: None,
                effective_frame: None,
                ramp_frames: None,
            },
        }
    }
    pub fn handle_encoded(&mut self, bytes: &[u8], now_ms: u64) -> Result<Vec<u8>> {
        let r = Request::decode(bytes)?;
        self.handle(&r, now_ms).encode()
    }
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        let sequence = self.sequence.checked_add(1).ok_or("sequence exhausted")?;
        self.sequence = sequence;
        Ok(Snapshot {
            show_id: self.show.clone(),
            epoch: self.epoch,
            revision: self.revision,
            sequence: Counter(sequence),
            page: 0,
            page_count: 1,
            durability: "volatile".into(),
            validity: "unavailable".into(),
            acquisition_frame: None,
            age_ms: None,
            rendered_application: false,
            release_commit: false,
            session_history_capacity: SESSION_HISTORY_CAPACITY as u32,
            inputs: (1..=self.inputs).map(|i| format!("input-{i:02}")).collect(),
            monitors: (1..=self.monitors)
                .map(|n| format!("monitor-{n}"))
                .collect(),
            modes: self.modes.iter().map(|(s, m)| (*s, *m)).collect(),
            automation_bounds: self.automation_bounds.values().flatten().cloned().collect(),
            parameters: self.parameters.clone(),
        })
    }
    pub fn handle(&mut self, r: &Request, now_ms: u64) -> Reply {
        if r.validate().is_err() || r.version != self.version {
            return self.reply(r, "rejected", Some("version"));
        }
        if r.show_id != self.show {
            return self.reply(r, "rejected", Some("wrong_show"));
        }
        if r.epoch != self.epoch {
            return self.reply(r, "rejected", Some("epoch"));
        }
        if now_ms < self.now {
            return self.reply(r, "rejected", Some("clock"));
        }
        self.now = now_ms;
        if matches!(r.command, Command::Snapshot {}) {
            return match self.snapshot() {
                Ok(s) => {
                    let mut p = self.reply(r, "applied", None);
                    p.body.snapshot = Some(s);
                    p
                }
                Err(_) => self.reply(r, "rejected", Some("capacity")),
            };
        }
        if let Command::Grant { scope } = r.command
            && !self.modes.contains_key(&scope)
            && !(self.version == 2
                && matches!(
                    scope,
                    Scope::PaConfiguration
                        | Scope::FxConfiguration
                        | Scope::OutputRoutes
                        | Scope::LocalOperatorMonitor
                        | Scope::TalkbackDestinations
                        | Scope::TalkbackFoh
                ))
        {
            return self.reply(r, "rejected", Some("scope"));
        }
        let writer = r.writer.as_ref().expect("validated");
        if matches!(r.command, Command::Grant { .. }) && !self.sessions.contains_key(writer) {
            if self.retired.contains(writer) {
                return self.reply(r, "rejected", Some("lease"));
            }
            if r.request_id != Some(Counter(1)) {
                return self.reply(r, "rejected", Some("range"));
            }
            if self.sessions.len() + self.retired.len() >= SESSION_HISTORY_CAPACITY {
                return self.reply(r, "busy", Some("capacity"));
            }
            if r.expected_revision != Some(self.revision) {
                return self.reply(r, "conflict", Some("stale_revision"));
            }
            let Command::Grant { scope } = r.command else {
                unreachable!()
            };
            if self
                .sessions
                .values()
                .any(|s| s.expires > now_ms && s.scope == scope)
            {
                return self.reply(r, "busy", Some("scope"));
            }
            // Retire expired sessions so capacity measures simultaneous live writers.
            let expired: Vec<_> = self
                .sessions
                .iter()
                .filter(|(_, s)| s.expires <= now_ms)
                .map(|(w, _)| w.clone())
                .collect();
            if self.sessions.len() - expired.len() >= MAX_LIVE_WRITERS {
                return self.reply(r, "busy", Some("capacity"));
            }
            let Some(next) = self.next_lease.checked_add(1) else {
                return self.reply(r, "rejected", Some("capacity"));
            };
            let Some(expires) = now_ms.checked_add(LEASE_MS) else {
                return self.reply(r, "rejected", Some("capacity"));
            };
            for w in expired {
                self.sessions.remove(&w);
                self.previews.retain(|_, p| p.writer != w);
                self.retired.insert(w);
            }
            let lease = Counter(self.next_lease);
            self.next_lease = next;
            let mut p = self.reply(r, "applied", None);
            p.body.granted_lease = Some(lease);
            p.body.scope = Some(scope);
            p.body.lease_remaining_ms = Some(2000);
            self.sessions.insert(
                writer.clone(),
                Session {
                    scope,
                    lease,
                    expires,
                    high: r.request_id.unwrap().0,
                    cache: VecDeque::from([(r.clone(), p.clone())]),
                    maintenance_high: 0,
                    maintenance_cache: VecDeque::new(),
                },
            );
            return p;
        }
        let Some(session) = self.sessions.get(writer) else {
            return self.reply(r, "rejected", Some("lease"));
        };
        // Grant retries must present the original null lease but still identify the
        // exact live session. Every other command needs its engine-issued lease.
        if session.expires <= now_ms
            || (r.lease != Some(session.lease) && !matches!(r.command, Command::Grant { .. }))
        {
            return self.reply(r, "rejected", Some("lease"));
        }
        let number = r.request_id.unwrap().0;
        if let Some((old, p)) = session
            .cache
            .iter()
            .find(|(old, _)| old.request_id == r.request_id)
        {
            return if old == r {
                p.clone()
            } else {
                self.reply(r, "rejected", Some("reused_id"))
            };
        }
        if number <= session.high {
            return self.reply(r, "rejected", Some("expired_id"));
        }
        let scope = session.scope;
        let p = if r.expected_revision != Some(self.revision) {
            self.reply(r, "conflict", Some("stale_revision"))
        } else {
            self.execute(r, scope, now_ms)
        };
        if let Some(s) = self.sessions.get_mut(writer) {
            s.high = number;
            s.cache.push_back((r.clone(), p.clone()));
            if s.cache.len() > 64 {
                s.cache.pop_front();
            }
        }
        p
    }
    fn edits(&self, targets: &[Edit], scope: Scope) -> Result<()> {
        if targets.is_empty() || targets.len() > 64 {
            return Err("capacity".into());
        }
        let mut seen = BTreeSet::new();
        for e in targets {
            if !seen.insert(&e.target)
                || !self.parameters.iter().any(|p| p.target == e.target)
                || !id(e.target.input())
            {
                return Err("target".into());
            }
            if e.target.scope()? != scope {
                return Err("scope".into());
            }
            if !e.target.value_valid(&e.value) {
                return Err("range".into());
            }
        }
        Ok(())
    }
    fn execute(&mut self, r: &Request, scope: Scope, now: u64) -> Reply {
        if matches!(
            scope,
            Scope::PaConfiguration
                | Scope::FxConfiguration
                | Scope::OutputRoutes
                | Scope::LocalOperatorMonitor
                | Scope::TalkbackDestinations
                | Scope::TalkbackFoh
        ) && !matches!(r.command, Command::Renew {} | Command::Release {})
        {
            return self.reply(r, "rejected", Some("scope"));
        }
        let writer = r.writer.as_ref().unwrap();
        match &r.command {
            Command::Renew {} => {
                let Some(expires) = now.checked_add(LEASE_MS) else {
                    return self.reply(r, "rejected", Some("capacity"));
                };
                self.sessions.get_mut(writer).unwrap().expires = expires;
                let mut p = self.reply(r, "applied", None);
                p.body.lease_remaining_ms = Some(2000);
                p.body.scope = Some(scope);
                p
            }
            Command::Release {} => {
                self.sessions.remove(writer);
                self.retired.insert(writer.clone());
                self.previews.retain(|_, p| p.writer != *writer);
                self.reply(r, "applied", None)
            }
            Command::Set { targets } | Command::Propose { targets } => {
                if let Err(e) = self.edits(targets, scope) {
                    return self.reply(r, "rejected", Some(&e));
                }
                if matches!(r.command, Command::Propose { .. })
                    && self.modes[&scope] != Mode::Assist
                {
                    return self.reply(r, "rejected", Some("scope"));
                }
                let Some(rev) = self.revision.0.checked_add(1) else {
                    return self.reply(r, "rejected", Some("capacity"));
                };
                for e in targets {
                    let p = self
                        .parameters
                        .iter_mut()
                        .find(|p| p.target == e.target)
                        .unwrap();
                    if matches!(r.command, Command::Propose { .. }) {
                        p.proposal = Some(e.value.clone());
                    } else {
                        p.target_value = e.value.clone();
                        p.hold = Some(e.value.clone());
                        p.owner = Some(writer.clone());
                    }
                }
                self.revision = Counter(rev);
                let mut p = self.reply(r, "applied", None);
                p.body.ramp_frames = Some(240);
                p
            }
            Command::SetMode { mode, bounds } => {
                if let Err(e) = validate_bounds(bounds, self.inputs, self.monitors) {
                    return self.reply(r, "rejected", Some(&e));
                }
                if (*mode == Mode::Auto && bounds.is_empty())
                    || (*mode != Mode::Auto && !bounds.is_empty())
                    || bounds.iter().any(|b| b.target.scope() != Ok(scope))
                {
                    return self.reply(r, "rejected", Some("scope"));
                }

                let Some(rev) = self.revision.0.checked_add(1) else {
                    return self.reply(r, "rejected", Some("capacity"));
                };
                for p in &mut self.parameters {
                    if p.target.scope() == Ok(scope) {
                        p.hold = Some(p.target_value.clone());
                        p.owner = Some(writer.clone());
                    }
                }
                self.modes.insert(scope, *mode);
                self.automation_bounds.insert(scope, bounds.clone());
                self.revision = Counter(rev);
                self.reply(r, "applied", None)
            }
            Command::PreviewRelease { targets } => {
                let mut seen = BTreeSet::new();
                if targets.is_empty() || targets.len() > 64 {
                    return self.reply(r, "rejected", Some("capacity"));
                }
                let mut destinations = Vec::new();
                for t in targets {
                    if !seen.insert(t) {
                        return self.reply(r, "rejected", Some("target"));
                    }
                    if t.scope() != Ok(scope) {
                        return self.reply(r, "rejected", Some("scope"));
                    }
                    let Some(p) = self.parameters.iter().find(|p| p.target == *t) else {
                        return self.reply(r, "rejected", Some("target"));
                    };
                    let Some(value) = p.proposal.clone() else {
                        return self.reply(r, "rejected", Some("unavailable"));
                    };
                    if p.hold.is_none() || matches!(t, Target::Mute { .. }) {
                        return self.reply(r, "rejected", Some("unavailable"));
                    }
                    destinations.push(Edit {
                        target: t.clone(),
                        value,
                    });
                }
                let (Some(next), Some(expires)) =
                    (self.next_preview.checked_add(1), now.checked_add(LEASE_MS))
                else {
                    return self.reply(r, "rejected", Some("capacity"));
                };
                // One preview per writer bounds transient storage.
                self.previews.retain(|_, p| p.writer != *writer);
                let preview = Preview {
                    token: format!("preview-{}", self.next_preview),
                    revision: self.revision,
                    scope,
                    remaining_ms: 2000,
                    ramp_frames: 240,
                    destinations,
                };
                self.next_preview = next;
                self.previews.insert(
                    preview.token.clone(),
                    StoredPreview {
                        preview: preview.clone(),
                        writer: writer.clone(),
                        lease: r.lease.unwrap(),
                        expires,
                    },
                );
                let mut p = self.reply(r, "applied", None);
                p.body.preview = Some(preview);
                p
            }
            Command::CancelPreview { token } | Command::ReleasePreview { token } => {
                let Some(p) = self.previews.get(token) else {
                    return self.reply(r, "rejected", Some("target"));
                };
                if p.writer != *writer || Some(p.lease) != r.lease || p.preview.scope != scope {
                    return self.reply(r, "rejected", Some("scope"));
                }
                if p.expires <= now || p.preview.revision != self.revision {
                    return self.reply(r, "conflict", Some("stale_revision"));
                }
                if matches!(r.command, Command::ReleasePreview { .. }) {
                    if !self.rendered_release {
                        return self.reply(r, "rejected", Some("unavailable"));
                    }
                    let Some(rev) = self.revision.0.checked_add(1) else {
                        return self.reply(r, "rejected", Some("capacity"));
                    };
                    let destinations = p.preview.destinations.clone();
                    for e in destinations {
                        let parameter = self
                            .parameters
                            .iter_mut()
                            .find(|p| p.target == e.target)
                            .unwrap();
                        parameter.target_value = e.value;
                        parameter.hold = None;
                        parameter.owner = None;
                    }
                    self.revision = Counter(rev);
                }
                self.previews.remove(token);
                self.reply(r, "applied", None)
            }
            _ => self.reply(r, "rejected", Some("lease")),
        }
    }
}
/// Receipt-clock collector. An incomplete or inconsistent set is discarded.
#[derive(Default, Debug)]
pub struct SnapshotPages {
    started: Option<u64>,
    pages: BTreeMap<u8, Snapshot>,
}
impl SnapshotPages {
    pub fn push(&mut self, page: Snapshot, receipt_ms: u64) -> Result<Option<Vec<Snapshot>>> {
        let result = self.accept(page, receipt_ms);
        if result.is_err() || matches!(result, Ok(Some(_))) {
            self.started = None;
            self.pages.clear();
        }
        result
    }
    fn accept(&mut self, p: Snapshot, now: u64) -> Result<Option<Vec<Snapshot>>> {
        p.validate()?;
        if self.started.is_some_and(|t| now < t || now - t > 2000) {
            return Err("expired snapshot".into());
        }
        if let Some(old) = self.pages.values().next()
            && (old.show_id != p.show_id
                || old.epoch != p.epoch
                || old.revision != p.revision
                || old.sequence != p.sequence
                || old.page_count != p.page_count
                || old.inputs != p.inputs
                || old.monitors != p.monitors
                || old.modes != p.modes
                || old.automation_bounds != p.automation_bounds
                || old.durability != p.durability
                || old.validity != p.validity
                || old.acquisition_frame != p.acquisition_frame
                || old.age_ms != p.age_ms
                || old.rendered_application != p.rendered_application
                || old.release_commit != p.release_commit
                || old.session_history_capacity != p.session_history_capacity)
        {
            return Err("mixed snapshot".into());
        }
        if self.pages.contains_key(&p.page) {
            return Err("duplicate page".into());
        }
        self.started.get_or_insert(now);
        let count = p.page_count;
        self.pages.insert(p.page, p);
        if self.pages.len() == count as usize {
            validate_inventory(
                self.pages.values().flat_map(|p| p.parameters.iter()),
                self.pages.values().next().unwrap().inputs.len(),
                self.pages.values().next().unwrap().monitors.len(),
            )?;
            Ok(Some(self.pages.values().cloned().collect()))
        } else {
            Ok(None)
        }
    }
}
fn required_targets_for(inputs: usize, monitors: usize) -> BTreeSet<Target> {
    let mut targets = BTreeSet::new();
    for i in 1..=inputs {
        let input = format!("input-{i:02}");
        targets.insert(Target::Fader {
            input: input.clone(),
        });
        targets.insert(Target::Pan {
            input: input.clone(),
        });
        targets.insert(Target::Mute {
            input: input.clone(),
        });
        for n in 1..=monitors {
            let monitor = format!("monitor-{n}");
            targets.insert(Target::Send {
                input: input.clone(),
                monitor,
            });
        }
    }
    targets
}
fn validate_inventory<'a>(
    parameters: impl Iterator<Item = &'a Parameter>,
    inputs: usize,
    monitors: usize,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for p in parameters {
        if !seen.insert(p.target.clone()) {
            return Err("duplicate parameter".into());
        }
    }
    let mut required = required_targets_for(inputs, monitors);
    if seen.contains(&Target::MasterGain {}) {
        required.insert(Target::MasterGain {});
    }
    if seen != required {
        return Err("parameter inventory".into());
    }
    Ok(())
}
impl Snapshot {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let s: Self = decode(bytes)?;
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if !canonical_show(&self.show_id)
            || self.epoch.0 == 0
            || self.sequence.0 == 0
            || self.page_count == 0
            || self.page_count > 16
            || self.page >= self.page_count
        {
            return Err("snapshot identity".into());
        }
        if self.durability != "volatile"
            || self.validity != "unavailable"
            || self.acquisition_frame.is_some()
            || self.age_ms.is_some()
            || self.rendered_application
            || self.release_commit
            || self.session_history_capacity != SESSION_HISTORY_CAPACITY as u32
        {
            return Err("harness capabilities".into());
        }
        let inputs: BTreeSet<_> = self.inputs.iter().cloned().collect();
        let monitors: BTreeSet<_> = self.monitors.iter().cloned().collect();
        if self.inputs.is_empty()
            || self.inputs.len() > u16::MAX as usize
            || inputs
                != (1..=self.inputs.len())
                    .map(|i| format!("input-{i:02}"))
                    .collect()
            || monitors
                != (1..=self.monitors.len())
                    .map(|i| format!("monitor-{i}"))
                    .collect()
            || monitors.len() != self.monitors.len()
        {
            return Err("channel inventory".into());
        }
        let scopes: BTreeSet<_> = std::iter::once(Scope::Foh)
            .chain((1..=self.monitors.len()).map(Scope::monitor))
            .collect();
        if self.modes.len() != scopes.len()
            || self.modes.iter().map(|(s, _)| *s).collect::<BTreeSet<_>>() != scopes
        {
            return Err("modes".into());
        }
        validate_bounds(
            &self.automation_bounds,
            self.inputs.len(),
            self.monitors.len(),
        )?;
        for (scope, mode) in &self.modes {
            let has_bounds = self
                .automation_bounds
                .iter()
                .any(|b| b.target.scope() == Ok(*scope));
            if (*mode == Mode::Auto) != has_bounds {
                return Err("automation bounds".into());
            }
        }
        if self.parameters.len() > self.inputs.len() * (3 + self.monitors.len()) + 1 {
            return Err("capacity".into());
        }
        let mut allowed = required_targets_for(self.inputs.len(), self.monitors.len());
        allowed.insert(Target::MasterGain {});
        let mut seen = BTreeSet::new();
        for p in &self.parameters {
            if !allowed.contains(&p.target)
                || !seen.insert(&p.target)
                || p.actual.is_some()
                || !p.target.value_valid(&p.target_value)
                || p.hold.as_ref().is_some_and(|v| !p.target.value_valid(v))
                || p.proposal
                    .as_ref()
                    .is_some_and(|v| !p.target.value_valid(v))
                || p.owner.as_ref().is_some_and(|w| !id(w))
                || p.hold.is_some() != p.owner.is_some()
            {
                return Err("parameter".into());
            }
        }
        if self.page_count == 1 {
            validate_inventory(
                self.parameters.iter(),
                self.inputs.len(),
                self.monitors.len(),
            )?;
        }
        Ok(())
    }
}
impl Reply {
    pub fn validate(&self) -> Result<()> {
        if self.contract != "C-AUDIO"
            || ![1, 2].contains(&self.version)
            || self.module != "audio"
            || !canonical_show(&self.show_id)
            || self.epoch.0 == 0
        {
            return Err("reply identity".into());
        }
        if self.writer.as_ref().is_some_and(|w| !id(w))
            || self.lease.is_some_and(|n| n.0 == 0)
            || self.request_id.is_some_and(|n| n.0 == 0)
        {
            return Err("reply authority".into());
        }
        if (self.writer.is_some()
            && (self.request_id.is_none() || self.expected_revision.is_none()))
            || (self.writer.is_none()
                && (self.lease.is_some()
                    || self.request_id.is_some()
                    || self.expected_revision.is_some()))
        {
            return Err("reply authority".into());
        }
        let b = &self.body;
        if self.kind == "applied"
            && ((self.writer.is_none() && b.snapshot.is_none())
                || (self.writer.is_some() && self.lease.is_none() && b.granted_lease.is_none()))
        {
            return Err("reply authority".into());
        }
        if b.scope.is_some() != b.lease_remaining_ms.is_some()
            || (b.ramp_frames.is_some()
                && (b.preview.is_some() || b.snapshot.is_some() || b.scope.is_some()))
        {
            return Err("reply body".into());
        }
        let reasons = [
            "wrong_show",
            "version",
            "epoch",
            "lease",
            "scope",
            "target",
            "range",
            "stale_revision",
            "reused_id",
            "expired_id",
            "unavailable",
            "capacity",
            "clock",
            "configuration_failed",
        ];
        match self.kind.as_str() {
            "applied" if b.reason.is_none() => (),
            "rejected"
                if b.reason
                    .as_ref()
                    .is_some_and(|r| reasons.contains(&r.as_str())) => {}
            "conflict" if b.reason.as_deref() == Some("stale_revision") => (),
            "busy" if matches!(b.reason.as_deref(), Some("scope" | "capacity")) => (),
            _ => return Err("reply kind".into()),
        }
        if b.effective_frame.is_some()
            || b.ramp_frames.is_some_and(|n| n != 240)
            || b.lease_remaining_ms.is_some_and(|n| n > 2000)
            || b.granted_lease.is_some_and(|n| n.0 == 0)
        {
            return Err("reply timing".into());
        }
        if self.kind != "applied"
            && (b.snapshot.is_some()
                || b.preview.is_some()
                || b.granted_lease.is_some()
                || b.scope.is_some()
                || b.lease_remaining_ms.is_some()
                || b.ramp_frames.is_some())
        {
            return Err("refusal body".into());
        }
        if let Some(s) = &b.snapshot {
            s.validate()?;
            if self.version == 1 && (s.inputs.len() != 8 || s.monitors.len() != 2) {
                return Err("legacy snapshot inventory".into());
            }
            if s.show_id != self.show_id
                || s.epoch != self.epoch
                || s.revision != b.revision
                || self.writer.is_some()
                || self.lease.is_some()
                || self.request_id.is_some()
                || self.expected_revision.is_some()
                || b.preview.is_some()
                || b.scope.is_some()
                || b.granted_lease.is_some()
                || b.lease_remaining_ms.is_some()
                || b.ramp_frames.is_some()
            {
                return Err("snapshot envelope".into());
            }
        }
        if b.granted_lease.is_some()
            && (self.lease.is_some()
                || self.writer.is_none()
                || self.request_id.is_none()
                || self.expected_revision.is_none()
                || b.scope.is_none()
                || b.lease_remaining_ms.is_none()
                || b.preview.is_some()
                || b.ramp_frames.is_some())
        {
            return Err("grant body".into());
        }
        if let Some(p) = &b.preview {
            if !id(&p.token)
                || p.revision != b.revision
                || p.remaining_ms > 2000
                || p.ramp_frames != 240
                || p.destinations.is_empty()
                || p.destinations.len() > 64
                || self.writer.is_none()
                || self.lease.is_none()
                || self.request_id.is_none()
                || self.expected_revision.is_none()
                || b.granted_lease.is_some()
                || b.scope.is_some()
                || b.lease_remaining_ms.is_some()
                || b.ramp_frames.is_some()
            {
                return Err("preview".into());
            }
            let mut seen = BTreeSet::new();
            for e in &p.destinations {
                if (e.target != (Target::MasterGain {})
                    && crate::processing_wire::input_index(e.target.input()).is_err())
                    || (self.version == 1
                        && crate::processing_wire::input_index(e.target.input())
                            .is_ok_and(|i| i >= 8))
                    || !seen.insert(&e.target)
                    || e.target.scope() != Ok(p.scope)
                    || !e.target.value_valid(&e.value)
                    || matches!(e.target, Target::Mute { .. })
                {
                    return Err("preview destination".into());
                }
            }
        }
        Ok(())
    }
}
fn validate_bounds(bounds: &[AutoBound], inputs: usize, monitors: usize) -> Result<()> {
    if bounds.len() > 64 {
        return Err("capacity".into());
    }
    let allowed = required_targets_for(inputs, monitors);
    let mut seen = BTreeSet::new();
    for b in bounds {
        if !allowed.contains(&b.target)
            || !seen.insert(&b.target)
            || matches!(b.target, Target::Mute { .. })
        {
            return Err("target".into());
        }
        if !b.target.value_valid(&b.min)
            || !b.target.value_valid(&b.max)
            || !matches!((&b.min,&b.max),(Value::Integer(min),Value::Integer(max)) if min<=max)
        {
            return Err("range".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const SHOW: &str = "11111111-1111-4111-8111-111111111111";
    fn r(w: &str, command: Command, n: u64, rev: u64, lease: Option<Counter>) -> Request {
        Request {
            contract: "C-AUDIO".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some(w.into()),
            lease,
            request_id: Some(Counter(n)),
            expected_revision: Some(Counter(rev)),
            command,
        }
    }
    #[test]
    fn history_and_expired_previews_are_bounded_without_forgetting_writers() {
        let mut a = Authority::new(SHOW, Counter(9), Counter(0)).unwrap();
        for i in 0..SESSION_HISTORY_CAPACITY {
            let writer = format!("desk-{i}");
            let now = i as u64 * 2000;
            let rev = a.revision.0;
            let l = a
                .handle(
                    &r(&writer, Command::Grant { scope: Scope::Foh }, 1, rev, None),
                    now,
                )
                .body
                .granted_lease
                .unwrap();
            assert!(a.previews.is_empty());
            let t = Target::Fader {
                input: "input-01".into(),
            };
            let rev = a.revision.0;
            a.handle(
                &r(
                    &writer,
                    Command::SetMode {
                        mode: Mode::Assist,
                        bounds: vec![],
                    },
                    2,
                    rev,
                    Some(l),
                ),
                now,
            );
            let rev = a.revision.0;
            a.handle(
                &r(
                    &writer,
                    Command::Propose {
                        targets: vec![Edit {
                            target: t.clone(),
                            value: Value::Integer(-2000),
                        }],
                    },
                    3,
                    rev,
                    Some(l),
                ),
                now,
            );
            let rev = a.revision.0;
            assert!(
                a.handle(
                    &r(
                        &writer,
                        Command::PreviewRelease { targets: vec![t] },
                        4,
                        rev,
                        Some(l)
                    ),
                    now
                )
                .body
                .preview
                .is_some()
            );
            assert_eq!(a.previews.len(), 1);
            assert!(a.sessions.len() <= MAX_LIVE_WRITERS);
            assert_eq!(a.sessions.len() + a.retired.len(), i + 1);
        }
        let parameters = a.parameters.clone();
        let rev = a.revision;
        let p = a.handle(
            &r(
                "desk-new",
                Command::Grant { scope: Scope::Foh },
                1,
                rev.0,
                None,
            ),
            SESSION_HISTORY_CAPACITY as u64 * 2000,
        );
        assert_eq!(p.body.reason.as_deref(), Some("capacity"));
        assert_eq!(a.parameters, parameters);
        assert_eq!(a.revision, rev);
        assert_eq!(a.sessions.len() + a.retired.len(), 1024);
        assert_eq!(
            a.handle(
                &r(
                    "desk-0",
                    Command::Grant { scope: Scope::Foh },
                    1,
                    rev.0,
                    None
                ),
                SESSION_HISTORY_CAPACITY as u64 * 2000
            )
            .body
            .reason
            .as_deref(),
            Some("lease")
        );
    }
    #[test]
    fn internal_counter_exhaustion_never_wraps_or_changes_mix() {
        let mut a = Authority::new(SHOW, Counter(9), Counter(12)).unwrap();
        a.next_lease = u64::MAX;
        let state = a.parameters.clone();
        assert_eq!(
            a.handle(
                &r("desk", Command::Grant { scope: Scope::Foh }, 1, 12, None),
                0
            )
            .body
            .reason
            .as_deref(),
            Some("capacity")
        );
        assert!(a.sessions.is_empty());
        assert_eq!(a.parameters, state);
        a.sequence = u64::MAX;
        assert!(a.snapshot().is_err());
        assert_eq!(a.sequence, u64::MAX);
    }
}

#[cfg(test)]
mod atomic_maintenance_tests {
    use super::*;
    use crate::lease_maintenance::{Reason, Request as Maintenance};
    const SHOW: &str = "11111111-1111-4111-8111-111111111111";
    fn setup(now: u64) -> (Authority, Maintenance) {
        let mut a = Authority::with_dimensions(SHOW, Counter(1), Counter(0), 16, 5).unwrap();
        let r = Request {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some("atomic-writer".into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant {
                scope: Scope::TalkbackDestinations,
            },
        };
        let lease = a.handle(&r, now).body.granted_lease.unwrap();
        let m = Maintenance {
            contract: crate::lease_maintenance::CONTRACT.into(),
            version: 1,
            kind: "maintain".into(),
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            authenticated_session: Counter(1),
            writer: "atomic-writer".into(),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            maintenance_id: Counter(1),
            scope: Scope::TalkbackDestinations,
            lease,
        };
        (a, m)
    }
    #[test]
    fn concurrent_controller_grants_maintain_and_refuse_capacity_without_eviction() {
        let (_, template) = setup(0);
        // Start with the exact six-controller deployment, then fill independent scopes.
        let mut scopes = vec![
            Scope::Foh,
            Scope::LocalOperatorMonitor,
            Scope::TalkbackDestinations,
            Scope::TalkbackFoh,
            Scope::Monitor1,
            Scope::Monitor2,
        ];
        scopes.extend([Scope::PaConfiguration, Scope::OutputRoutes]);
        scopes.extend((3..=11).map(Scope::Monitor));
        let mut a = Authority::with_dimensions(SHOW, Counter(1), Counter(0), 16, 11).unwrap();
        let mut leases = Vec::new();
        for (i, scope) in scopes.iter().take(MAX_LIVE_WRITERS).enumerate() {
            let writer = format!("controller-{i}");
            let grant = Request {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(1),
                writer: Some(writer.clone()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(a.revision),
                command: Command::Grant { scope: *scope },
            };
            let lease = a
                .handle(&grant, 0)
                .body
                .granted_lease
                .expect("independent grant");
            leases.push(Maintenance {
                writer,
                lease,
                scope: *scope,
                ..template.clone()
            });
        }
        let revision = a.revision;
        let parameters = a.parameters.clone();
        let overflow = Request {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some("overflow".into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(revision),
            command: Command::Grant {
                scope: scopes[MAX_LIVE_WRITERS],
            },
        };
        let reply = a.handle(&overflow, 1);
        assert_eq!(reply.kind, "busy");
        assert_eq!(reply.body.reason.as_deref(), Some("capacity"));
        assert_eq!(a.sessions.len(), MAX_LIVE_WRITERS);
        assert_eq!(a.revision, revision);
        assert_eq!(a.parameters, parameters);
        for (id, now) in [(1, 1000), (2, 2500)] {
            for lease in &mut leases {
                lease.maintenance_id = Counter(id);
                a.maintain_lease(lease, now, 48 * id, false).unwrap();
            }
        }
        assert_eq!(a.sessions.len(), MAX_LIVE_WRITERS);
        // Expiry returns capacity; admission never evicts a live writer.
        assert!(a.handle(&overflow, 4500).body.granted_lease.is_some());
    }
    #[test]
    fn atomic_maintenance_overflow_replay_and_context_do_not_mutate_lease() {
        let (mut a, mut m) = setup(u64::MAX - LEASE_MS);
        assert_eq!(
            a.maintain_lease(&m, u64::MAX - 1, 48, false),
            Err(Reason::Clock)
        );
        assert_eq!(a.sessions[&m.writer].expires, u64::MAX);
        assert_eq!(a.sessions[&m.writer].maintenance_cache.len(), 1);
        assert_eq!(
            a.maintain_lease(&m, u64::MAX - 1, 96, true),
            Err(Reason::Clock)
        );
        m.capability_generation = Counter(2); // current owner admission is an outer prerequisite
        assert_eq!(
            a.maintain_lease(&m, u64::MAX - 1, 96, false),
            Err(Reason::ReusedId)
        );
        assert_eq!(
            a.maintain_lease(&m, u64::MAX, 96, false),
            Err(Reason::Lease)
        );
        assert_eq!(a.revision, Counter(0));
        let (mut a, m) = setup(0);
        let original = a.maintain_lease(&m, 10, 48, false).unwrap();
        let mut changed = m.clone();
        changed.capability_generation = Counter(2);
        assert_eq!(
            a.maintain_lease(&changed, 11, 48, false),
            Err(Reason::ReusedId)
        );
        changed = m.clone();
        changed.show_id = "22222222-2222-4222-8222-222222222222".into();
        assert_eq!(
            a.maintain_lease(&changed, 11, 48, false),
            Err(Reason::Identity)
        );
        changed = m.clone();
        changed.scope = Scope::TalkbackFoh;
        assert_eq!(
            a.maintain_lease(&changed, 11, 48, false),
            Err(Reason::Scope)
        );
        changed = m.clone();
        changed.lease = Counter(9);
        assert_eq!(
            a.maintain_lease(&changed, 11, 48, false),
            Err(Reason::Lease)
        );
        assert_eq!(a.sessions[&m.writer].maintenance_cache.len(), 1);
        assert_eq!(
            a.clone().maintain_lease(&m, 12, 96, true).unwrap(),
            original
        );
        assert_eq!(a.sessions[&m.writer].expires, 2010);
    }
    #[test]
    fn atomic_maintenance_unknown_writers_cannot_allocate_cache_or_advance_highwater() {
        let (mut a, mut m) = setup(0);
        for n in 0..100 {
            m.writer = format!("unknown-{n}");
            m.maintenance_id = Counter(u64::MAX);
            assert_eq!(a.maintain_lease(&m, 0, 0, false), Err(Reason::Lease));
        }
        assert_eq!(a.sessions.len(), 1);
        let s = &a.sessions["atomic-writer"];
        assert_eq!(s.maintenance_high, 0);
        assert!(s.maintenance_cache.is_empty());
        m.writer = "atomic-writer".into();
        assert!(a.maintain_lease(&m, 1, 0, false).is_ok());
        m.maintenance_id = Counter(1);
        assert_eq!(a.maintain_lease(&m, 2, 0, false), Err(Reason::ExpiredId));
    }
}
