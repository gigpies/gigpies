//! GP03 control-side offline pump. Authority/JSON/heap work stays outside Mixer::process.
//! This is not a realtime multi-thread host integration.
use crate::processing_wire::{
    Channel, ProcessingCommand, ProcessingReply, ProcessingRequest, ProcessingSnapshot,
};
use crate::sends_wire::{SendsCommand, SendsReply, SendsRequest, SendsSnapshot};
use crate::{
    control_model::{Authority, Command, Edit, Reply, Request, Scope, Snapshot},
    mixer::{INPUTS, Mixer, Prepared},
    show::{Counter, Result},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub input: String,
    /// Unit: linear amplitude gain * 1e9 (rounded), order fader, pan L/R,
    /// shared mute, send1/2. Not millidecibels.
    pub current_nanogain: Vec<u64>,
    pub ramp_target_nanogain: Vec<u64>,
    pub held_nanogain: Vec<Option<u64>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderedSnapshot {
    pub capability: String,
    pub capability_version: u32,
    pub rendered_application: bool,
    pub release_commit: bool,
    pub frame: Counter,
    pub faulted: bool,
    pub protection: String,
    pub meters: Option<Vec<i32>>,
    /// Unchanged GP02 target metadata. Its unavailable actual values are deliberate.
    pub authority: Snapshot,
    pub coefficients: Vec<Observation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<crate::topology::EngineTopology>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<crate::clock_domain::ClockDomain>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<EngineResources>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineResources {
    pub admission: crate::topology::Admission,
    pub render_budget: crate::topology::ResourceBudget,
    pub snapshot_assembly_bytes: usize,
    pub live_writer_capacity: usize,
    pub reply_history_per_writer: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContext {
    pub show_id: String,
    pub module: String,
    pub epoch: Counter,
    pub writer: Option<String>,
    pub lease: Option<Counter>,
    pub request_id: Option<Counter>,
    pub expected_revision: Option<Counter>,
}
impl RequestContext {
    pub(crate) fn request(r: &Request) -> Self {
        Self {
            show_id: r.show_id.clone(),
            module: r.module.clone(),
            epoch: r.epoch,
            writer: r.writer.clone(),
            lease: r.lease,
            request_id: r.request_id,
            expected_revision: r.expected_revision,
        }
    }
    fn reply(r: &Reply) -> Self {
        Self {
            show_id: r.show_id.clone(),
            module: r.module.clone(),
            epoch: r.epoch,
            writer: r.writer.clone(),
            lease: r.lease,
            request_id: r.request_id,
            expected_revision: r.expected_revision,
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        let command = if self.writer.is_none() {
            Command::Snapshot {}
        } else if self.lease.is_none() {
            Command::Grant { scope: Scope::Foh }
        } else {
            Command::Renew {}
        };
        Request {
            contract: "C-AUDIO".into(),
            version: 1,
            show_id: self.show_id.clone(),
            module: self.module.clone(),
            epoch: self.epoch,
            writer: self.writer.clone(),
            lease: self.lease,
            request_id: self.request_id,
            expected_revision: self.expected_revision,
            command,
        }
        .encode()?;
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderedReply {
    pub context: RequestContext,
    pub capability: String,
    pub capability_version: u32,
    pub state: String,
    pub ticket: Option<Counter>,
    pub effective_frame: Option<Counter>,
    pub ramp_frames: Option<u32>,
    /// Absent while pending. Applied only after authority + DSP commit.
    pub outcome: Option<Reply>,
    pub snapshot: Option<RenderedSnapshot>,
}
impl RenderedSnapshot {
    pub fn validate(&self) -> Result<()> {
        self.authority.validate()?;
        if self.capability != "GP03-rendered"
            || ![1, 2].contains(&self.capability_version)
            || !self.rendered_application
            || !self.release_commit
            || self.protection != "offline-unprotected"
            || self.meters.is_some()
            || self.coefficients.len() != self.authority.inputs.len()
            || (self.capability_version == 1
                && (self.authority.inputs.len() != 8 || self.authority.monitors.len() != 2))
        {
            return Err("rendered snapshot capability".into());
        }
        if self.capability_version == 1
            && (self.topology.is_some() || self.clock.is_some() || self.resources.is_some())
        {
            return Err("legacy capabilities".into());
        }
        if self.capability_version == 2 {
            let topology = self.topology.as_ref().ok_or("topology missing")?;
            let resources = self.resources.as_ref().ok_or("resources missing")?;
            if resources.render_budget != crate::topology::ResourceBudget::default()
                || resources.admission != topology.validate(resources.render_budget)?
                || resources.snapshot_assembly_bytes != crate::snapshot_pages::ASSEMBLY_BYTES
                || resources.live_writer_capacity != 4
                || resources.reply_history_per_writer != 64
            {
                return Err("resource capabilities".into());
            }
            topology.validate(crate::topology::ResourceBudget::default())?;
            if topology.inputs.len() != self.authority.inputs.len()
                || topology.monitors != self.authority.monitors.len()
                || self.clock.as_ref().is_none_or(|c| {
                    c.epoch != self.authority.epoch.0
                        || c.sample_rate != 48000
                        || c.next_frame != self.frame.0
                })
            {
                return Err("topology/clock coherence".into());
            }
        }
        for (i, observation) in self.coefficients.iter().enumerate() {
            if observation.input != format!("input-{:02}", i + 1) {
                return Err("rendered inventory".into());
            }
            let width = 4 + self.authority.monitors.len();
            if observation.current_nanogain.len() != width
                || observation.ramp_target_nanogain.len() != width
                || observation.held_nanogain.len() != width
            {
                return Err("coefficient shape".into());
            }
            for j in 0..width {
                let max = if j == 0 || j >= 4 {
                    3_981_071_706
                } else {
                    1_000_000_000
                };
                if observation.current_nanogain[j] > max
                    || observation.ramp_target_nanogain[j] > max
                    || observation.held_nanogain[j].is_some_and(|v| v > max)
                {
                    return Err("rendered coefficient".into());
                }
            }
        }
        Ok(())
    }
}
impl RenderedReply {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        Self::decode_assembled(bytes)
    }
    pub fn decode_assembled(bytes: &[u8]) -> Result<Self> {
        let reply: Self =
            crate::show::decode_bounded(bytes, crate::snapshot_pages::ASSEMBLY_BYTES)?;
        reply.validate()?;
        Ok(reply)
    }
    pub fn validate(&self) -> Result<()> {
        if self.capability != "GP03-rendered" || ![1, 2].contains(&self.capability_version) {
            return Err("rendered capability".into());
        }
        self.context.validate()?;
        match self.state.as_str() {
            "backpressure"
                if self.ticket.is_none()
                    && self.effective_frame.is_none()
                    && self.ramp_frames.is_none()
                    && self.outcome.is_none()
                    && self.snapshot.is_none() => {}
            "pending"
                if self.context.lease.is_some()
                    && self.ticket.is_some_and(|v| v.0 > 0)
                    && self.effective_frame.is_some_and(|v| v.0 % 48 == 0)
                    && self.ramp_frames == Some(240)
                    && self.outcome.is_none()
                    && self.snapshot.is_none() => {}
            "final" => {
                let outcome = self.outcome.as_ref().ok_or("final outcome")?;
                if self.context != RequestContext::reply(outcome) {
                    return Err("reply correlation".into());
                }
                outcome.validate()?;
                if outcome.version != self.capability_version {
                    return Err("version coherence".into());
                }
                if self.ticket.is_some() {
                    if self.ticket.is_some_and(|v| v.0 == 0)
                        || self.effective_frame.is_none_or(|v| v.0 % 48 != 0)
                        || self.ramp_frames != Some(240)
                        || outcome.kind != "applied"
                        || self.snapshot.is_none()
                    {
                        return Err("rendered timing".into());
                    }
                } else if self.effective_frame.is_some() || self.ramp_frames.is_some() {
                    return Err("rendered timing".into());
                }
                if let Some(snapshot) = &self.snapshot {
                    snapshot.validate()?;
                    if snapshot.capability_version != self.capability_version {
                        return Err("version coherence".into());
                    }
                    if outcome.kind != "applied"
                        || snapshot.authority.revision != outcome.body.revision
                        || snapshot.authority.show_id != outcome.show_id
                        || snapshot.authority.epoch != outcome.epoch
                        || self
                            .effective_frame
                            .is_some_and(|f| f.0 >= snapshot.frame.0)
                    {
                        return Err("rendered snapshot binding".into());
                    }
                    if let Some(inner) = &outcome.body.snapshot
                        && inner != &snapshot.authority
                    {
                        return Err("snapshot coherence".into());
                    }
                }
            }
            _ => return Err("rendered reply state".into()),
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("capacity".into());
        }
        Ok(bytes)
    }
}
#[derive(Debug)]
struct Pending {
    request: Request,
    staged: Authority,
    reply: Reply,
    ticket: u64,
    frame: u64,
    scope: Scope,
}
#[derive(Debug)]
struct ProcessingPending {
    request: ProcessingRequest,
    ticket: u64,
    frame: u64,
}
#[derive(Debug)]
struct SendsPending {
    request: SendsRequest,
    ticket: u64,
    frame: u64,
}
#[derive(Debug)]
pub struct OfflineEngine {
    authority: Authority,
    mixer: Mixer,
    pending: Option<Pending>,
    next_ticket: u64,
    outcomes: BTreeMap<String, VecDeque<(Request, RenderedReply)>>,
    holds: Vec<Vec<Option<u64>>>,
    sends_pending: Option<SendsPending>,
    sends_outcomes: BTreeMap<String, VecDeque<(SendsRequest, SendsReply)>>,
    sends_completions: Vec<SendsReply>,
    processing_pending: Option<ProcessingPending>,
    processing_outcomes: BTreeMap<String, VecDeque<(ProcessingRequest, ProcessingReply)>>,
    processing_completions: Vec<ProcessingReply>,
    external_pending: Option<(Request, Scope, u64)>,
    clock: crate::clock_domain::ClockDomain,
    module_ids: BTreeMap<String, VecDeque<(Request, String, Option<String>)>>,
}
impl OfflineEngine {
    pub fn new(show: &str, epoch: Counter, revision: Counter, frame: u64) -> Result<Self> {
        Self::with_topology(
            show,
            epoch,
            revision,
            frame,
            crate::topology::EngineTopology::legacy(),
        )
    }
    pub fn with_topology(
        show: &str,
        epoch: Counter,
        revision: Counter,
        frame: u64,
        topology: crate::topology::EngineTopology,
    ) -> Result<Self> {
        topology.validate(crate::topology::ResourceBudget::default())?;
        let inputs = topology.inputs.len();
        let monitors = topology.monitors;
        let mut authority = Authority::with_dimensions(show, epoch, revision, inputs, monitors)?;
        authority.set_wire_version(if topology.is_legacy() { 1 } else { 2 });
        authority.enable_rendered_release();
        Ok(Self {
            external_pending: None,
            clock: {
                let mut c = crate::clock_domain::ClockDomain::new(epoch.0, frame);
                if topology.is_legacy() {
                    c.rearm().map_err(String::from)?;
                }
                c
            },
            authority,
            mixer: {
                let legacy = topology.is_legacy();
                let mut m = Mixer::from_topology(frame, topology)?;
                if !legacy {
                    m.quiesce();
                }
                m
            },
            pending: None,
            next_ticket: 1,
            outcomes: BTreeMap::new(),
            holds: vec![vec![None; 4 + monitors]; inputs],
            module_ids: BTreeMap::new(),
            sends_pending: None,
            sends_outcomes: BTreeMap::new(),
            sends_completions: Vec::new(),
            processing_pending: None,
            processing_outcomes: BTreeMap::new(),
            processing_completions: Vec::new(),
        })
    }
    fn send_id_conflict(&self, r: &Request) -> bool {
        self.sends_pending
            .as_ref()
            .is_some_and(|p| p.request.writer == r.writer && p.request.request_id == r.request_id)
    }
    /// Reserve one shared authority transaction. Fingerprint must bind the exact
    /// contract and complete body. The caller retains prepared owned state offRT.
    pub fn begin_external(
        &mut self,
        r: &Request,
        fingerprint: &str,
        scope: Scope,
        now: u64,
    ) -> Result<(u64, Option<Reply>)> {
        r.encode()?;
        if r.version != self.wire_version()
            || !matches!(
                scope,
                Scope::PaConfiguration
                    | Scope::FxConfiguration
                    | Scope::OutputRoutes
                    | Scope::LocalOperatorMonitor
                    | Scope::TalkbackDestinations
                    | Scope::TalkbackFoh
            )
            || fingerprint.is_empty()
            || fingerprint.len() > 128
        {
            return Err("external contract".into());
        }
        let mut history = r.clone();
        history.contract = format!("external:{fingerprint}");
        if let Some(reason) = self.authority.processing_identity(&history, now) {
            return Err(reason.into());
        }
        if self.send_id_conflict(r) {
            return Err("reused_id".into());
        }
        if let Some(reply) = self.authority.cached(&history, now) {
            return Ok((0, Some(reply)));
        }
        if let Some((old, old_scope, frame)) = &self.external_pending {
            return if old == &history && *old_scope == scope {
                Ok((*frame, None))
            } else {
                Err("backpressure".into())
            };
        }
        if self.pending.is_some()
            || self.processing_pending.is_some()
            || self.sends_pending.is_some()
        {
            return Err("backpressure".into());
        }
        let checked = self
            .authority
            .clone()
            .scoped_transaction(&history, now, None, scope);
        if checked.kind != "applied" {
            return Ok((
                0,
                Some(
                    self.authority
                        .scoped_transaction(&history, now, None, scope),
                ),
            ));
        }
        let frame = self.mixer.next_boundary()?;
        self.external_pending = Some((history, scope, frame));
        Ok((frame, None))
    }
    /// Only FX preparation may hold a reservation beyond the next boundary.
    /// It never stalls dry audio while a remote owner prepares off rendering.
    pub fn schedule_fx_external(
        &mut self,
        request: &Request,
        fingerprint: &str,
        frame: u64,
    ) -> Result<()> {
        if !self.external_matches(request, fingerprint)
            || frame < self.frame()
            || frame
                .checked_sub(self.frame())
                .is_none_or(|lead| lead > 48_000)
            || !frame.is_multiple_of(48)
        {
            return Err("FX reservation boundary".into());
        }
        let pending = self.external_pending.as_mut().ok_or("external missing")?;
        if pending.1 != Scope::FxConfiguration {
            return Err("FX reservation scope".into());
        }
        pending.2 = frame;
        Ok(())
    }
    pub fn cancel_fx_external(&mut self, request: &Request, fingerprint: &str) {
        if self.external_matches(request, fingerprint)
            && self
                .external_pending
                .as_ref()
                .is_some_and(|p| p.1 == Scope::FxConfiguration)
        {
            let (history, scope, _) = self.external_pending.take().unwrap();
            self.authority.cancel_external(&history, scope);
        }
    }
    pub fn external_matches(&self, request: &Request, fingerprint: &str) -> bool {
        let mut history = request.clone();
        history.contract = format!("external:{fingerprint}");
        self.external_pending
            .as_ref()
            .is_some_and(|(pending, _, _)| pending == &history)
    }
    pub fn external_boundary(&self) -> Option<u64> {
        self.external_pending.as_ref().map(|p| p.2)
    }
    /// Invoke before the boundary sample. Validation precedes the supplied atomic
    /// prepared commit; revision changes once only after successful application.
    pub fn commit_external(
        &mut self,
        now: u64,
        apply: impl FnOnce() -> Result<()>,
    ) -> Result<Reply> {
        self.commit_external_with_engine(now, |_| apply())
    }
    /// Trusted composition callback may only apply its already prepared engine or
    /// module mutation; it must leave authority/epoch/session state untouched.
    pub fn commit_external_with_engine(
        &mut self,
        now: u64,
        apply: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Reply> {
        let (request, scope, frame) = self.external_pending.as_ref().ok_or("external missing")?;
        if self.frame() != *frame {
            return Err("external boundary".into());
        }
        let mut staged = self.authority.clone();
        let checked = staged.scoped_transaction(request, now, None, *scope);
        let (request, scope, _) = self.external_pending.take().unwrap();
        if checked.kind != "applied" {
            self.authority = staged;
            return Ok(checked);
        }
        if apply(self).is_err() {
            return Ok(self.authority.scoped_transaction(
                &request,
                now,
                Some("configuration_failed"),
                scope,
            ));
        }
        self.authority = staged;
        Ok(checked)
    }
    pub fn admit_source(&mut self, epoch: u64, frame: u64, frames: usize) -> Result<()> {
        if self.clock.state == crate::clock_domain::ClockState::Disarmed {
            if epoch != self.clock.epoch || frame != self.frame() {
                self.quiesce("source_discontinuity");
                return Err("source timeline".into());
            }
            self.clock.next_frame = frame.checked_add(frames as u64).ok_or("clock exhausted")?;
            return Ok(());
        }
        if let Err(e) = self.clock.admit(epoch, frame, frames) {
            self.quiesce(e);
            return Err(e.into());
        }
        Ok(())
    }
    pub fn clock_status(&self) -> &crate::clock_domain::ClockDomain {
        &self.clock
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn mark_verified_mapping(&mut self) -> Result<()> {
        if self.topology().mapping_evidence != "operator-verified" {
            return Err("mapping evidence is not operator verified".into());
        }
        self.clock.mapping_verified = true;
        Ok(())
    }
    pub fn topology(&self) -> &crate::topology::EngineTopology {
        self.mixer.topology()
    }
    pub fn enable_fx_aux(&mut self) -> Result<()> {
        self.mixer.enable_fx_aux()
    }
    pub fn set_fx_sends(&mut self, bus: usize, levels: &[f64]) -> Result<()> {
        self.mixer.set_fx_sends(bus, levels)
    }
    pub fn set_operator_tap(&mut self, source: crate::brain_control::MonitorSource) {
        self.mixer.set_operator_tap(source);
    }
    pub fn operator_tap(&self) -> &[f64; 96] {
        self.mixer.operator_tap()
    }
    pub fn output_safety_gain(&self, frame: u64) -> f64 {
        self.mixer.output_safety_gain(frame)
    }
    pub fn maintain_lease(
        &mut self,
        r: &crate::lease_maintenance::Request,
        now: u64,
    ) -> std::result::Result<crate::lease_maintenance::Maintained, crate::lease_maintenance::Reason>
    {
        if r.validate().is_err() {
            return Err(crate::lease_maintenance::Reason::Identity);
        }
        if !self.frame().is_multiple_of(48) {
            return Err(crate::lease_maintenance::Reason::Clock);
        }
        let busy = self.pending.is_some()
            || self.processing_pending.is_some()
            || self.sends_pending.is_some()
            || self.external_pending.is_some();
        self.authority.maintain_lease(r, now, self.frame(), busy)
    }
    pub(crate) fn observe_atomic_read_time(&mut self, now: u64) -> Result<()> {
        self.authority.observe_control_time(now)
    }
    pub fn live_lease_witness(
        &self,
        r: &Request,
        now: u64,
    ) -> std::result::Result<(Scope, u32), crate::held_proof::Reason> {
        self.authority.live_lease_witness(r, now)
    }
    pub fn writer_scope(&self, r: &Request, now: u64) -> Option<Scope> {
        self.authority.live_scope(r, now)
    }
    pub fn quiesce(&mut self, reason: &str) {
        self.clock.quiesce(reason);
        self.mixer.quiesce();
        self.pending = None;
        self.processing_pending = None;
        self.sends_pending = None;
        self.sends_completions.clear();
        self.external_pending = None;
        self.authority.revoke_all();
    }
    pub fn recover(&mut self, epoch: Counter, frame: u64) -> Result<()> {
        let mut clock = self.clock.clone();
        clock.recover(epoch.0, frame).map_err(String::from)?;
        let mut intent = self.persisted_intent()?;
        intent.topology.map_revision = intent
            .topology
            .map_revision
            .checked_add(1)
            .ok_or("map exhausted")?;
        let mut next = Self::restore_intent(&intent, epoch, frame)?;
        next.mixer.inherit_fx_send_intent(&self.mixer)?;
        next.clock = clock;
        *self = next;
        Ok(())
    }
    pub fn persisted_intent(&mut self) -> Result<EngineIntent> {
        let snapshot = self.authority.snapshot()?;
        Ok(EngineIntent {
            version: 2,
            show_id: snapshot.show_id,
            send_taps: Some(
                self.mixer
                    .send_observations()
                    .into_iter()
                    .map(|r| r.into_iter().map(|s| s.1).collect())
                    .collect(),
            ),
            topology: self.topology().clone(),
            parameters: snapshot
                .parameters
                .into_iter()
                .map(|p| Edit {
                    target: p.target,
                    value: p.target_value,
                })
                .collect(),
            processing: self
                .mixer
                .processing_observations()
                .into_iter()
                .map(|o| o.1)
                .collect(),
        })
    }
    pub fn restore_intent_for(
        intent: &EngineIntent,
        expected: &crate::topology::EngineTopology,
        epoch: Counter,
        frame: u64,
    ) -> Result<Self> {
        if &intent.topology != expected {
            return Err("persisted topology/capability mismatch; explicit remap required".into());
        }
        Self::restore_intent(intent, epoch, frame)
    }
    pub fn restore_intent(intent: &EngineIntent, epoch: Counter, frame: u64) -> Result<Self> {
        if ![1, 2].contains(&intent.version)
            || intent.processing.len() != intent.topology.inputs.len()
            || (intent.version == 1 && intent.send_taps.is_some())
            || (intent.version == 2 && intent.send_taps.is_none())
        {
            return Err("intent version/shape".into());
        }
        intent
            .topology
            .validate(crate::topology::ResourceBudget::default())?;
        let mut e = Self::with_topology(
            &intent.show_id,
            epoch,
            Counter(0),
            frame,
            intent.topology.clone(),
        )?;
        e.mixer
            .restore_intent(&intent.parameters, &intent.processing)?;
        e.authority.restore_parameters(&intent.parameters)?;
        if let Some(taps) = &intent.send_taps {
            if e.topology().is_legacy()
                && taps
                    .iter()
                    .flatten()
                    .any(|t| *t != crate::sends_wire::Tap::RawPostMute)
            {
                return Err("legacy monitor taps".into());
            }
            e.mixer.restore_sends(taps)?;
        }
        e.clock.state = crate::clock_domain::ClockState::Disarmed;
        e.mixer.quiesce();
        Ok(e)
    }
    pub fn mute_outputs(&mut self) -> Result<()> {
        self.mixer.mute_outputs().map_err(|e| format!("{e:?}"))
    }
    pub fn outputs_quiesced(&self) -> bool {
        self.mixer.outputs_quiesced()
    }
    pub fn rearm_outputs(&mut self) -> Result<()> {
        if self.mixer.faulted()
            || self
                .frame()
                .checked_add(crate::mixer::RAMP_FRAMES)
                .is_none()
        {
            return Err("fault/clock recovery required".into());
        }
        if self.clock.state == crate::clock_domain::ClockState::Disarmed {
            self.clock.rearm().map_err(String::from)?;
        }
        if self.clock.state != crate::clock_domain::ClockState::Running {
            return Err("clock recovery required".into());
        }
        self.mixer.rearm().map_err(|e| format!("{e:?}"))
    }
    pub fn prepare_output_patch(
        &self,
        outputs: Vec<crate::topology::OutputPort>,
    ) -> Result<crate::mixer::PreparedOutputPatch> {
        self.mixer.prepare_output_patch(outputs)
    }
    pub fn apply_output_patch(
        &mut self,
        prepared: &mut crate::mixer::PreparedOutputPatch,
    ) -> Result<()> {
        self.mixer
            .apply_output_patch(prepared)
            .map_err(String::from)
    }
    pub fn replace_output_patch(
        &mut self,
        outputs: Vec<crate::topology::OutputPort>,
    ) -> Result<()> {
        self.mixer.replace_output_patch(outputs)
    }
    pub fn rearm(&mut self) -> Result<()> {
        if self.mixer.faulted()
            || self
                .frame()
                .checked_add(crate::mixer::RAMP_FRAMES)
                .is_none()
        {
            return Err("fault/clock recovery required".into());
        }
        self.clock.rearm().map_err(String::from)?;
        self.mixer.rearm().map_err(|e| format!("{e:?}"))
    }
    pub(crate) fn pending_sends_ticket_for(&self, writer: &str) -> Option<u64> {
        self.sends_pending
            .as_ref()
            .filter(|p| p.request.writer.as_deref() == Some(writer))
            .map(|p| p.ticket)
    }
    pub fn revoke_writer(&mut self, writer: &str) {
        self.authority.revoke_writer(writer);
        if self
            .external_pending
            .as_ref()
            .is_some_and(|p| p.0.writer.as_deref() == Some(writer))
        {
            self.external_pending = None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.request.writer.as_deref() == Some(writer))
        {
            let p = self.pending.take().unwrap();
            self.mixer.cancel(p.ticket);
        }
        if self
            .processing_pending
            .as_ref()
            .is_some_and(|p| p.request.writer.as_deref() == Some(writer))
        {
            let p = self.processing_pending.take().unwrap();
            self.mixer.cancel(p.ticket);
        }
        self.outcomes.remove(writer);
        if self
            .sends_pending
            .as_ref()
            .is_some_and(|p| p.request.writer.as_deref() == Some(writer))
        {
            let p = self.sends_pending.take().unwrap();
            self.mixer.cancel(p.ticket);
        }
        self.sends_outcomes.remove(writer);
        self.sends_completions
            .retain(|p| p.context.writer.as_deref() != Some(writer));
        self.processing_outcomes.remove(writer);
        self.module_ids.remove(writer);
    }
    pub fn processing_version(&self) -> u32 {
        if self.topology().is_legacy() { 2 } else { 4 }
    }
    pub fn wire_version(&self) -> u32 {
        self.authority.version()
    }
    pub fn frame(&self) -> u64 {
        self.mixer.frame()
    }
    pub fn revision(&self) -> Counter {
        self.authority.revision()
    }
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }
    fn wrapped(reply: Reply) -> RenderedReply {
        RenderedReply {
            context: RequestContext::reply(&reply),
            capability: "GP03-rendered".into(),
            capability_version: reply.version,
            state: "final".into(),
            ticket: None,
            effective_frame: None,
            ramp_frames: None,
            outcome: Some(reply),
            snapshot: None,
        }
    }
    fn backpressure(r: &Request) -> RenderedReply {
        RenderedReply {
            context: RequestContext::request(r),
            capability: "GP03-rendered".into(),
            capability_version: r.version,
            state: "backpressure".into(),
            ticket: None,
            effective_frame: None,
            ramp_frames: None,
            outcome: None,
            snapshot: None,
        }
    }
    fn remember(&mut self, r: Request, reply: RenderedReply) {
        let cache = self.outcomes.entry(r.writer.clone().unwrap()).or_default();
        cache.push_back((r, reply));
        if cache.len() > 64 {
            cache.pop_front();
        }
    }
    fn pending_reply(p: &Pending) -> RenderedReply {
        RenderedReply {
            context: RequestContext::request(&p.request),
            capability: "GP03-rendered".into(),
            capability_version: p.request.version,
            state: "pending".into(),
            ticket: Some(Counter(p.ticket)),
            effective_frame: Some(Counter(p.frame)),
            ramp_frames: Some(240),
            outcome: None,
            snapshot: None,
        }
    }
    pub fn snapshot(&mut self) -> Result<RenderedSnapshot> {
        let current = self.mixer.coefficients();
        let target = self.mixer.targets();
        Ok(RenderedSnapshot {
            capability: "GP03-rendered".into(),
            capability_version: self.authority.version(),
            rendered_application: true,
            release_commit: true,
            frame: Counter(self.frame()),
            faulted: self.mixer.faulted(),
            protection: "offline-unprotected".into(),
            meters: None,
            resources: if self.authority.version() == 2 {
                Some(EngineResources {
                    admission: self
                        .topology()
                        .validate(crate::topology::ResourceBudget::default())?,
                    render_budget: crate::topology::ResourceBudget::default(),
                    snapshot_assembly_bytes: crate::snapshot_pages::ASSEMBLY_BYTES,
                    live_writer_capacity: 4,
                    reply_history_per_writer: 64,
                })
            } else {
                None
            },
            topology: (self.authority.version() == 2).then(|| self.topology().clone()),
            clock: (self.authority.version() == 2).then(|| {
                let mut c = self.clock.clone();
                c.next_frame = self.frame();
                c
            }),
            authority: self.authority.snapshot()?,
            coefficients: (0..self.mixer.input_count())
                .map(|i| Observation {
                    input: format!("input-{:02}", i + 1),
                    current_nanogain: current[i].iter().copied().map(nano).collect(),
                    ramp_target_nanogain: target[i].iter().copied().map(nano).collect(),
                    held_nanogain: self.holds[i].clone(),
                })
                .collect(),
        })
    }
    pub fn processing_snapshot(&mut self) -> Result<ProcessingSnapshot> {
        let a = self.authority.snapshot()?;
        Ok(ProcessingSnapshot {
            show_id: a.show_id,
            epoch: a.epoch,
            revision: a.revision,
            sequence: a.sequence,
            frame: Counter(self.frame()),
            sample_rate: 48000,
            foh_tap: crate::processing_wire::FOH_TAP.into(),
            monitor_tap: if self.topology().is_legacy() {
                crate::processing_wire::MONITOR_TAP
            } else {
                crate::processing_wire::PER_SEND_TAP
            }
            .into(),
            faulted: self.mixer.faulted(),
            channels: self
                .mixer
                .processing_observations()
                .into_iter()
                .enumerate()
                .map(|(i, (current, target, remaining, gr))| Channel {
                    input: format!("input-{:02}", i + 1),
                    current,
                    target,
                    transition_remaining_frames: remaining,
                    ready: remaining == 0,
                    gain_reduction_mdb: gr,
                })
                .collect(),
        })
    }
    fn remember_processing(&mut self, r: ProcessingRequest, p: ProcessingReply) {
        let cache = self
            .processing_outcomes
            .entry(r.writer.clone().expect("mutation"))
            .or_default();
        cache.push_back((r, p));
        if cache.len() > 64 {
            cache.pop_front();
        }
    }
    fn processing_pending_reply(&self, p: &ProcessingPending) -> ProcessingReply {
        let mut r = ProcessingReply::new(&p.request, "pending", None, self.revision());
        r.ticket = Some(Counter(p.ticket));
        r.effective_frame = Some(Counter(p.frame));
        r.ramp_frames = Some(240);
        r
    }
    /// Complete atomic processing admission; all preparation remains off render.
    pub fn handle_processing(
        &mut self,
        r: &ProcessingRequest,
        now: u64,
    ) -> Result<ProcessingReply> {
        self.handle_processing_with_freshness(r, now, true)
    }
    pub(crate) fn handle_processing_with_freshness(
        &mut self,
        r: &ProcessingRequest,
        now: u64,
        fresh: bool,
    ) -> Result<ProcessingReply> {
        r.validate()?;
        if r.version != self.processing_version() {
            return Ok(ProcessingReply::new(
                r,
                "final",
                Some("unsupported_version".into()),
                self.revision(),
            ));
        }
        if let ProcessingCommand::ProcessingSet { input, .. } = &r.command
            && crate::processing_wire::input_index(input)? >= self.mixer.input_count()
        {
            return Ok(ProcessingReply::new(
                r,
                "final",
                Some("target".into()),
                self.revision(),
            ));
        }
        if matches!(r.command, ProcessingCommand::ProcessingSnapshot {}) {
            let reply = self.authority.handle(&r.authority_request(), now);
            let mut p = ProcessingReply::new(r, "final", reply.body.reason, self.revision());
            if reply.kind == "applied" {
                p.snapshot = Some(self.processing_snapshot()?);
            }
            return Ok(p);
        }
        let history = r.history_request();
        if let Some(reason) = self.authority.processing_identity(&history, now) {
            return Ok(ProcessingReply::new(
                r,
                "final",
                Some(reason.into()),
                self.revision(),
            ));
        }
        self.authority.observe_control_time(now)?;
        self.processing_outcomes
            .retain(|w, _| self.authority.writer_live(w, now));
        if self.authority.cached(&history, now).is_some() {
            if let Some((old, p)) = self
                .processing_outcomes
                .get(r.writer.as_ref().unwrap())
                .and_then(|v| v.iter().find(|(old, _)| old.request_id == r.request_id))
            {
                return Ok(if old == r {
                    p.clone()
                } else {
                    ProcessingReply::new(r, "final", Some("reused_id".into()), self.revision())
                });
            }
            return Ok(ProcessingReply::new(
                r,
                "final",
                Some("expired_id".into()),
                self.revision(),
            ));
        }
        if let Some(p) = &self.processing_pending {
            if p.request == *r {
                return Ok(self.processing_pending_reply(p));
            }
            if p.request.writer == r.writer && p.request.request_id == r.request_id {
                return Ok(ProcessingReply::new(
                    r,
                    "final",
                    Some("reused_id".into()),
                    self.revision(),
                ));
            }
        }
        if let Some(p) = &self.pending
            && p.request.writer == r.writer
            && p.request.request_id == r.request_id
        {
            return Ok(ProcessingReply::new(
                r,
                "final",
                Some("reused_id".into()),
                self.revision(),
            ));
        }
        if self
            .sends_pending
            .as_ref()
            .is_some_and(|p| p.request.writer == r.writer && p.request.request_id == r.request_id)
        {
            return Ok(ProcessingReply::new(
                r,
                "final",
                Some("reused_id".into()),
                self.revision(),
            ));
        }
        // Probe shared high-water/cache before pressure; do not consume new IDs.
        let mut check = self.authority.clone();
        let refusal = if !fresh {
            Some("stale_snapshot")
        } else {
            self.mixer.faulted().then_some("faulted")
        };
        let checked = check.processing_transaction(&history, now, refusal);
        if matches!(
            checked.body.reason.as_deref(),
            Some("reused_id" | "expired_id")
        ) {
            return Ok(ProcessingReply::new(
                r,
                "final",
                checked.body.reason,
                self.revision(),
            ));
        }
        if self.external_pending.is_some()
            || self.pending.is_some()
            || self.processing_pending.is_some()
            || self.sends_pending.is_some()
            || !self.mixer.processing_ready()
            || !self.processing_completions.is_empty()
        {
            return Ok(ProcessingReply::new(
                r,
                "backpressure",
                None,
                self.revision(),
            ));
        }
        if checked.kind != "applied" {
            let reply = self
                .authority
                .processing_transaction(&history, now, refusal);
            let p = ProcessingReply::new(r, "final", reply.body.reason, self.revision());
            if self.authority.cached(&history, now).is_some() {
                self.remember_processing(r.clone(), p.clone());
            }
            return Ok(p);
        }
        let ProcessingCommand::ProcessingSet { input, config } = r.command.clone() else {
            unreachable!()
        };
        let prepared = Prepared::processing_for(
            self.mixer.input_count(),
            self.mixer.monitor_count(),
            crate::processing_wire::input_index(&input)?,
            crate::channel_processing::Prepared::new(config)?,
        )?;
        let Some(next) = self.next_ticket.checked_add(1) else {
            return Ok(ProcessingReply::new(
                r,
                "backpressure",
                None,
                self.revision(),
            ));
        };
        let Ok(frame) = self.mixer.schedule(prepared, self.next_ticket) else {
            return Ok(ProcessingReply::new(
                r,
                "backpressure",
                None,
                self.revision(),
            ));
        };
        let pending = ProcessingPending {
            request: r.clone(),
            ticket: self.next_ticket,
            frame,
        };
        self.next_ticket = next;
        let reply = self.processing_pending_reply(&pending);
        self.processing_pending = Some(pending);
        Ok(reply)
    }
    pub fn take_processing_completions(&mut self) -> Vec<ProcessingReply> {
        std::mem::take(&mut self.processing_completions)
    }
    pub fn sends_snapshot(&mut self) -> Result<SendsSnapshot> {
        let a = self.authority.snapshot()?;
        Ok(SendsSnapshot {
            show_id: a.show_id,
            epoch: a.epoch,
            revision: a.revision,
            sequence: a.sequence,
            frame: Counter(self.frame()),
            sample_rate: 48000,
            supported_modes: crate::sends_wire::MODES.to_vec(),
            tap_positions: crate::sends_wire::POSITIONS
                .iter()
                .map(|s| (*s).into())
                .collect(),
            monitors: (1..=self.mixer.monitor_count())
                .map(|m| format!("monitor-{m}"))
                .collect(),
            faulted: self.mixer.faulted(),
            channels: self
                .mixer
                .send_observations()
                .into_iter()
                .enumerate()
                .map(|(i, row)| crate::sends_wire::Channel {
                    input: format!("input-{:02}", i + 1),
                    sends: row
                        .into_iter()
                        .enumerate()
                        .map(
                            |(m, (current, target, remaining))| crate::sends_wire::Send {
                                monitor: format!("monitor-{}", m + 1),
                                current,
                                target,
                                transition_remaining_frames: remaining,
                                ready: remaining == 0,
                            },
                        )
                        .collect(),
                })
                .collect(),
        })
    }
    fn remember_sends(&mut self, r: SendsRequest, p: SendsReply) {
        let cache = self
            .sends_outcomes
            .entry(r.writer.clone().expect("mutation"))
            .or_default();
        cache.push_back((r, p));
        if cache.len() > 64 {
            cache.pop_front();
        }
    }
    fn sends_pending_reply(&self, p: &SendsPending) -> SendsReply {
        let mut r = SendsReply::new(&p.request, "pending", None, self.revision());
        r.ticket = Some(Counter(p.ticket));
        r.effective_frame = Some(Counter(p.frame));
        r.ramp_frames = Some(240);
        r
    }
    /// Complete atomic processing admission; all preparation remains off render.
    pub fn handle_sends(&mut self, r: &SendsRequest, now: u64) -> Result<SendsReply> {
        self.handle_sends_with_freshness(r, now, true)
    }
    pub(crate) fn handle_sends_with_freshness(
        &mut self,
        r: &SendsRequest,
        now: u64,
        fresh: bool,
    ) -> Result<SendsReply> {
        r.validate()?;
        if self.topology().is_legacy() {
            return Ok(SendsReply::new(
                r,
                "final",
                Some("unsupported_topology".into()),
                self.revision(),
            ));
        }
        if let SendsCommand::SendTapSet { input, monitor, .. } = &r.command
            && (crate::processing_wire::input_index(input)? >= self.mixer.input_count()
                || crate::sends_wire::monitor_index(monitor)? >= self.mixer.monitor_count())
        {
            return Ok(SendsReply::new(
                r,
                "final",
                Some("target".into()),
                self.revision(),
            ));
        }
        if matches!(r.command, SendsCommand::SendsSnapshot {}) {
            let reply = self.authority.handle(&r.authority_request(), now);
            let mut p = SendsReply::new(r, "final", reply.body.reason, self.revision());
            if reply.kind == "applied" {
                p.snapshot = Some(self.sends_snapshot()?);
            }
            return Ok(p);
        }
        let history = r.history_request();
        if let Some(reason) = self.authority.processing_identity(&history, now) {
            return Ok(SendsReply::new(
                r,
                "final",
                Some(reason.into()),
                self.revision(),
            ));
        }
        self.authority.observe_control_time(now)?;
        self.sends_outcomes
            .retain(|w, _| self.authority.writer_live(w, now));
        if self.authority.cached(&history, now).is_some() {
            if let Some((old, p)) = self
                .sends_outcomes
                .get(r.writer.as_ref().unwrap())
                .and_then(|v| v.iter().find(|(old, _)| old.request_id == r.request_id))
            {
                return Ok(if old == r {
                    p.clone()
                } else {
                    SendsReply::new(r, "final", Some("reused_id".into()), self.revision())
                });
            }
            return Ok(SendsReply::new(
                r,
                "final",
                Some("expired_id".into()),
                self.revision(),
            ));
        }
        if let Some(p) = &self.sends_pending {
            if p.request == *r {
                return Ok(self.sends_pending_reply(p));
            }
            if p.request.writer == r.writer && p.request.request_id == r.request_id {
                return Ok(SendsReply::new(
                    r,
                    "final",
                    Some("reused_id".into()),
                    self.revision(),
                ));
            }
        }
        if let Some(p) = &self.pending
            && p.request.writer == r.writer
            && p.request.request_id == r.request_id
        {
            return Ok(SendsReply::new(
                r,
                "final",
                Some("reused_id".into()),
                self.revision(),
            ));
        }
        if self
            .processing_pending
            .as_ref()
            .is_some_and(|p| p.request.writer == r.writer && p.request.request_id == r.request_id)
            || self
                .external_pending
                .as_ref()
                .is_some_and(|p| p.0.writer == r.writer && p.0.request_id == r.request_id)
        {
            return Ok(SendsReply::new(
                r,
                "final",
                Some("reused_id".into()),
                self.revision(),
            ));
        }
        // Probe shared high-water/cache before pressure; do not consume new IDs.
        let mut check = self.authority.clone();
        let refusal = if !fresh {
            Some("stale_snapshot")
        } else {
            self.mixer.faulted().then_some("faulted")
        };
        let checked = check.scoped_transaction(&history, now, refusal, r.scope()?);
        if matches!(
            checked.body.reason.as_deref(),
            Some("reused_id" | "expired_id")
        ) {
            return Ok(SendsReply::new(
                r,
                "final",
                checked.body.reason,
                self.revision(),
            ));
        }
        if self.external_pending.is_some()
            || self.pending.is_some()
            || self.sends_pending.is_some()
            || self.processing_pending.is_some()
            || !self.mixer.sends_ready()
            || !self.sends_completions.is_empty()
        {
            return Ok(SendsReply::new(r, "backpressure", None, self.revision()));
        }
        if checked.kind != "applied" {
            let reply = self
                .authority
                .scoped_transaction(&history, now, refusal, r.scope()?);
            let p = SendsReply::new(r, "final", reply.body.reason, self.revision());
            if self.authority.cached(&history, now).is_some() {
                self.remember_sends(r.clone(), p.clone());
            }
            return Ok(p);
        }
        let SendsCommand::SendTapSet {
            input,
            monitor,
            tap,
        } = &r.command
        else {
            unreachable!()
        };
        let prepared = Prepared::send_for(
            self.mixer.input_count(),
            self.mixer.monitor_count(),
            crate::processing_wire::input_index(input)?,
            crate::sends_wire::monitor_index(monitor)?,
            *tap,
        )?;
        let Some(next) = self.next_ticket.checked_add(1) else {
            return Ok(SendsReply::new(r, "backpressure", None, self.revision()));
        };
        let Ok(frame) = self.mixer.schedule(prepared, self.next_ticket) else {
            return Ok(SendsReply::new(r, "backpressure", None, self.revision()));
        };
        let pending = SendsPending {
            request: r.clone(),
            ticket: self.next_ticket,
            frame,
        };
        self.next_ticket = next;
        let reply = self.sends_pending_reply(&pending);
        self.sends_pending = Some(pending);
        Ok(reply)
    }
    pub fn take_sends_completions(&mut self) -> Vec<SendsReply> {
        std::mem::take(&mut self.sends_completions)
    }
    /// GP05 admission shares the authority's session high-water with GP03.
    /// Fingerprints include contract/kind/body. A private Renew reserves the ID;
    /// GP03 is fenced from replaying that internal reservation below. No graph
    /// mutation/revision occurs, and lifecycle completion remains provider-owned.
    pub fn authorize_module(
        &mut self,
        r: &Request,
        fingerprint: &str,
        now: u64,
    ) -> Result<(bool, Option<String>)> {
        r.encode()?;
        let snapshot = self.authority.snapshot()?;
        if r.show_id != snapshot.show_id
            || r.epoch != snapshot.epoch
            || self.authority.live_scope(r, now) != Some(Scope::Foh)
            || !matches!(r.command, Command::Renew {})
        {
            return Err("module authority identity/lease/scope".into());
        }
        self.module_ids
            .retain(|w, _| self.authority.writer_live(w, now));
        let writer = r.writer.as_ref().ok_or("writer")?;
        if let Some((old, hash, reason)) = self
            .module_ids
            .get(writer)
            .and_then(|v| v.iter().find(|(old, _, _)| old.request_id == r.request_id))
        {
            if self.authority.cached(r, now).is_none() {
                return Err("expired_id".into());
            }
            return if old == r && hash == fingerprint {
                Ok((true, reason.clone()))
            } else {
                Err("reused_id".into())
            };
        }
        // An earlier GP03 Renew with the same identity must not be interpreted
        // as a module admission. Other collisions are rejected by Authority.
        if self.authority.cached(r, now).is_some() {
            return Err("reused_id".into());
        }
        if self.send_id_conflict(r) {
            return Err("reused_id".into());
        }
        if self.external_pending.is_some() {
            return Err("backpressure".into());
        }
        if let Some(p) = &self.processing_pending
            && p.request.writer == r.writer
            && p.request.request_id == r.request_id
        {
            return Err("reused_id".into());
        }
        if self.pending.is_some()
            || self.processing_pending.is_some()
            || self.sends_pending.is_some()
        {
            return Err("backpressure".into());
        }
        let reply = self.authority.handle(r, now);
        let reason = if reply.kind == "applied" {
            None
        } else {
            Some(reply.body.reason.unwrap_or(reply.kind))
        };
        // Refusals consuming the shared ID (such as stale_revision) retain the
        // original GP05 fingerprint/outcome, just like successful admissions.
        if self.authority.cached(r, now).is_none() {
            return Err(reason.unwrap_or_else(|| "admission".into()));
        }
        let cache = self.module_ids.entry(writer.clone()).or_default();
        cache.push_back((r.clone(), fingerprint.into(), reason.clone()));
        if cache.len() > 64 {
            cache.pop_front();
        }
        Ok((false, reason))
    }
    pub fn handle_encoded(&mut self, bytes: &[u8], now_ms: u64) -> Result<Vec<u8>> {
        self.handle(&Request::decode(bytes)?, now_ms)?.encode()
    }
    pub fn handle(&mut self, r: &Request, now: u64) -> Result<RenderedReply> {
        // Decode validation also protects callers using the typed public entry.
        r.encode()?;
        if self.external_pending.is_some() && !matches!(r.command, Command::Snapshot {}) {
            return Ok(Self::backpressure(r));
        }
        self.module_ids
            .retain(|w, _| self.authority.writer_live(w, now));
        if self.authority.live_scope(r, now).is_some()
            && self
                .module_ids
                .get(r.writer.as_deref().unwrap_or(""))
                .is_some_and(|v| v.iter().any(|(old, _, _)| old.request_id == r.request_id))
        {
            return Ok(Self::wrapped(self.authority.reply(
                r,
                "rejected",
                Some("reused_id"),
            )));
        }
        if self.authority.observe_control_time(now).is_err() {
            return Ok(Self::wrapped(self.authority.reply(
                r,
                "rejected",
                Some("clock"),
            )));
        }
        self.outcomes
            .retain(|writer, _| self.authority.writer_live(writer, now));
        if let Some(cached) = self.authority.cached(r, now) {
            if let Some((_, reply)) = self
                .outcomes
                .get(r.writer.as_ref().unwrap())
                .and_then(|cache| cache.iter().find(|(old, _)| old == r))
            {
                return Ok(reply.clone());
            }
            return Ok(Self::wrapped(cached));
        }
        if let Some(p) = &self.processing_pending
            && !matches!(r.command, Command::Snapshot {})
        {
            let mut check = self.authority.clone();
            let checked = check.handle(r, now);
            if matches!(
                checked.body.reason.as_deref(),
                Some("wrong_show" | "epoch" | "lease" | "clock" | "expired_id" | "reused_id")
            ) {
                return Ok(Self::wrapped(checked));
            }
            if p.request.writer == r.writer && p.request.request_id == r.request_id {
                return Ok(Self::wrapped(self.authority.reply(
                    r,
                    "rejected",
                    Some("reused_id"),
                )));
            }
            return Ok(Self::backpressure(r));
        }
        if let Some(p) = &self.sends_pending
            && !matches!(r.command, Command::Snapshot {})
        {
            let mut check = self.authority.clone();
            let checked = check.handle(r, now);
            if matches!(
                checked.body.reason.as_deref(),
                Some("wrong_show" | "epoch" | "lease" | "clock" | "expired_id" | "reused_id")
            ) {
                return Ok(Self::wrapped(checked));
            }
            if p.request.writer == r.writer && p.request.request_id == r.request_id {
                return Ok(Self::wrapped(self.authority.reply(
                    r,
                    "rejected",
                    Some("reused_id"),
                )));
            }
            return Ok(Self::backpressure(r));
        }
        if let Some(p) = &self.pending {
            // Identity and current lease validity precede pending retry identity.
            // A retry after expiry must not reveal a pending ticket or become a
            // misleading reused-ID refusal.
            if !matches!(r.command, Command::Snapshot {} | Command::Grant { .. })
                && self.authority.live_scope(r, now).is_none()
            {
                let mut check = self.authority.clone();
                return Ok(Self::wrapped(check.handle(r, now)));
            }
            if p.request == *r && self.authority.live_scope(r, now).is_some() {
                return Ok(Self::pending_reply(p));
            }
            if p.request.writer == r.writer
                && p.request.request_id == r.request_id
                && p.request.lease == r.lease
                && p.request.show_id == r.show_id
                && p.request.epoch == r.epoch
            {
                return Ok(Self::wrapped(self.authority.reply(
                    r,
                    "rejected",
                    Some("reused_id"),
                )));
            }
            // No metadata mutation may overwrite a staged authority. Read snapshots
            // observe only the committed authority/graph, never the staged state.
            if !matches!(r.command, Command::Snapshot {}) {
                let mut check = self.authority.clone();
                let reply = check.handle(r, now);
                if matches!(
                    reply.body.reason.as_deref(),
                    Some("wrong_show" | "epoch" | "lease" | "clock" | "expired_id" | "reused_id")
                ) {
                    return Ok(Self::wrapped(reply));
                }
                return Ok(Self::backpressure(r));
            }
        }
        if matches!(r.command, Command::Snapshot {}) {
            let reply = self.authority.handle(r, now);
            let mut wrapped = Self::wrapped(reply);
            if wrapped
                .outcome
                .as_ref()
                .is_some_and(|p| p.kind == "applied")
            {
                let snapshot = self.snapshot()?;
                wrapped.outcome.as_mut().unwrap().body.snapshot = Some(snapshot.authority.clone());
                wrapped.snapshot = Some(snapshot);
            }
            return Ok(wrapped);
        }
        let graph = matches!(
            r.command,
            Command::Set { .. } | Command::SetMode { .. } | Command::ReleasePreview { .. }
        );
        if !graph {
            return Ok(Self::wrapped(self.authority.handle(r, now)));
        }
        let mut staged = self.authority.clone();
        let reply = staged.handle(r, now);
        if reply.kind != "applied" {
            return Ok(Self::wrapped(self.authority.handle(r, now)));
        }
        // Cached retry can be applied without advancing revision. Never reschedule.
        if staged.revision() == self.authority.revision() {
            return Ok(Self::wrapped(reply));
        }
        let scope = self.authority.live_scope(r, now).ok_or("lease")?;
        let prepared = match &r.command {
            Command::Set { targets } => Prepared::edits_for(
                self.mixer.input_count(),
                self.mixer.monitor_count(),
                targets,
            )?,
            Command::SetMode { .. } => {
                Prepared::freeze_for(self.mixer.input_count(), self.mixer.monitor_count(), scope)
            }
            Command::ReleasePreview { .. } => {
                let (old, _) = self.authority.state();
                let (new, _) = staged.state();
                let edits: Vec<_> = new
                    .iter()
                    .zip(old)
                    .filter(|(n, o)| n.hold.is_none() && o.hold.is_some())
                    .map(|(n, _)| Edit {
                        target: n.target.clone(),
                        value: n.target_value.clone(),
                    })
                    .collect();
                Prepared::edits_for(self.mixer.input_count(), self.mixer.monitor_count(), &edits)?
            }
            _ => unreachable!(),
        };
        let Some(next) = self.next_ticket.checked_add(1) else {
            return Ok(Self::backpressure(r));
        };
        let frame = match self.mixer.schedule(prepared, self.next_ticket) {
            Ok(frame) => frame,
            Err(_) => return Ok(Self::backpressure(r)),
        };
        let p = Pending {
            request: r.clone(),
            staged,
            reply,
            ticket: self.next_ticket,
            frame,
            scope,
        };
        self.next_ticket = next;
        let reply = Self::pending_reply(&p);
        self.pending = Some(p);
        Ok(reply)
    }
    /// Offline control pump: split at pending boundary, validate lease before its
    /// sample, call fixed renderer, then commit metadata outside process. Injected
    /// now_ms is monotonic authority time, not the audio frame counter.
    pub fn process(
        &mut self,
        input: &[[f64; INPUTS]],
        output: &mut [[f64; 4]],
        now_ms: u64,
    ) -> Result<Vec<RenderedReply>> {
        if !self.mixer.topology().is_legacy() {
            return Err("legacy shape".into());
        }
        self.process_interleaved(input.as_flattened(), output.as_flattened_mut(), now_ms)
    }
    pub fn process_interleaved(
        &mut self,
        input: &[f64],
        output: &mut [f64],
        now_ms: u64,
    ) -> Result<Vec<RenderedReply>> {
        let ni = self.mixer.input_count();
        let no = self.mixer.monitor_count() + 2;
        if !input.len().is_multiple_of(ni) || output.len() != input.len() / ni * no {
            return Err("buffer length".into());
        }
        self.authority.observe_control_time(now_ms)?;
        let end = self
            .frame()
            .checked_add((input.len() / ni) as u64)
            .ok_or("clock exhausted")?;
        if self
            .external_boundary()
            .is_some_and(|boundary| end > boundary)
        {
            return Err("external boundary requires commit before render".into());
        }
        let mut replies = Vec::new();
        if let Some(split) = self
            .processing_pending
            .as_ref()
            .filter(|p| p.frame < end)
            .map(|p| (p.frame - self.frame()) as usize)
        {
            self.mixer
                .process_interleaved(&input[..split * ni], &mut output[..split * no])
                .map_err(|e| format!("{e:?}"))?;
            let p = self.processing_pending.take().expect("pending");
            let mut staged = self.authority.clone();
            let checked = staged.processing_transaction(
                &p.request.history_request(),
                now_ms,
                self.mixer.faulted().then_some("faulted"),
            );
            if checked.kind != "applied" {
                self.mixer.cancel(p.ticket);
                self.authority = staged;
                let reply =
                    ProcessingReply::new(&p.request, "final", checked.body.reason, self.revision());
                self.remember_processing(p.request, reply.clone());
                self.processing_completions.push(reply);
                self.mixer
                    .process_interleaved(&input[split * ni..], &mut output[split * no..])
                    .map_err(|e| format!("{e:?}"))?;
            } else {
                self.mixer
                    .process_interleaved(
                        &input[split * ni..(split + 1) * ni],
                        &mut output[split * no..(split + 1) * no],
                    )
                    .map_err(|e| format!("{e:?}"))?;
                let done = self.mixer.take_completion().ok_or("completion missing")?;
                if done.ticket != p.ticket || done.frame != p.frame {
                    return Err("completion identity".into());
                }
                self.authority = staged;
                let mut reply = ProcessingReply::new(&p.request, "final", None, self.revision());
                reply.ticket = Some(Counter(p.ticket));
                reply.effective_frame = Some(Counter(p.frame));
                reply.ramp_frames = Some(240);
                reply.snapshot = Some(self.processing_snapshot()?);
                self.remember_processing(p.request, reply.clone());
                self.processing_completions.push(reply);
                self.mixer
                    .process_interleaved(
                        &input[(split + 1) * ni..],
                        &mut output[(split + 1) * no..],
                    )
                    .map_err(|e| format!("{e:?}"))?;
            }
            self.clock.next_frame = self.frame();
            return Ok(replies);
        }
        if let Some(split) = self
            .sends_pending
            .as_ref()
            .filter(|p| p.frame < end)
            .map(|p| (p.frame - self.frame()) as usize)
        {
            self.mixer
                .process_interleaved(&input[..split * ni], &mut output[..split * no])
                .map_err(|e| format!("{e:?}"))?;
            let p = self.sends_pending.take().expect("pending");
            let mut staged = self.authority.clone();
            let checked = staged.scoped_transaction(
                &p.request.history_request(),
                now_ms,
                self.mixer.faulted().then_some("faulted"),
                p.request.scope()?,
            );
            if checked.kind != "applied" {
                self.mixer.cancel(p.ticket);
                self.authority = staged;
                let reply =
                    SendsReply::new(&p.request, "final", checked.body.reason, self.revision());
                self.remember_sends(p.request, reply.clone());
                self.sends_completions.push(reply);
                self.mixer
                    .process_interleaved(&input[split * ni..], &mut output[split * no..])
                    .map_err(|e| format!("{e:?}"))?;
            } else {
                self.mixer
                    .process_interleaved(
                        &input[split * ni..(split + 1) * ni],
                        &mut output[split * no..(split + 1) * no],
                    )
                    .map_err(|e| format!("{e:?}"))?;
                let done = self.mixer.take_completion().ok_or("completion missing")?;
                if done.ticket != p.ticket || done.frame != p.frame {
                    return Err("completion identity".into());
                }
                self.authority = staged;
                let mut reply = SendsReply::new(&p.request, "final", None, self.revision());
                reply.ticket = Some(Counter(p.ticket));
                reply.effective_frame = Some(Counter(p.frame));
                reply.ramp_frames = Some(240);
                reply.snapshot = Some(self.sends_snapshot()?);
                self.remember_sends(p.request, reply.clone());
                self.sends_completions.push(reply);
                self.mixer
                    .process_interleaved(
                        &input[(split + 1) * ni..],
                        &mut output[(split + 1) * no..],
                    )
                    .map_err(|e| format!("{e:?}"))?;
            }
            self.clock.next_frame = self.frame();
            return Ok(replies);
        }
        let split = self
            .pending
            .as_ref()
            .filter(|p| p.frame < end)
            .map(|p| (p.frame - self.frame()) as usize);
        if let Some(split) = split {
            self.mixer
                .process_interleaved(&input[..split * ni], &mut output[..split * no])
                .map_err(|e| format!("{e:?}"))?;
            if let Some(p) = &mut self.pending {
                // Re-run authority validation at commit, before any DSP application.
                // No mutating requests were admitted while pending, so only time and
                // observation sequence can differ. This validates preview expiry too.
                let mut current = self.authority.clone();
                let reply = current.handle(&p.request, now_ms);
                if reply.kind != "applied" {
                    let p = self.pending.take().unwrap();
                    self.mixer.cancel(p.ticket);
                    self.authority = current;
                    let final_reply = Self::wrapped(reply);
                    self.remember(p.request, final_reply.clone());
                    replies.push(final_reply);
                } else {
                    p.staged = current;
                    p.reply = reply;
                }
            }
            // Render exactly the boundary sample, then drain primitive completion.
            self.mixer
                .process_interleaved(
                    &input[split * ni..(split + 1) * ni],
                    &mut output[split * no..(split + 1) * no],
                )
                .map_err(|e| format!("{e:?}"))?;
            if let Some(done) = self.mixer.take_completion() {
                let mut p = self.pending.take().ok_or("completion without pending")?;
                if done.ticket != p.ticket || done.frame != p.frame {
                    return Err("completion identity".into());
                }
                self.update_holds(&p);
                p.reply.body.ramp_frames = None;
                p.staged.install_rendered_reply(&p.request, &p.reply);
                p.staged.preserve_observations(&self.authority);
                self.authority = p.staged;
                let mut reply = Self::wrapped(p.reply);
                reply.ticket = Some(Counter(p.ticket));
                reply.effective_frame = Some(Counter(p.frame));
                reply.ramp_frames = Some(240);
                reply.snapshot = Some(self.snapshot()?);
                self.remember(p.request, reply.clone());
                replies.push(reply);
            }
            self.mixer
                .process_interleaved(&input[(split + 1) * ni..], &mut output[(split + 1) * no..])
                .map_err(|e| format!("{e:?}"))?;
        } else {
            self.mixer
                .process_interleaved(input, output)
                .map_err(|e| format!("{e:?}"))?;
        }
        self.clock.next_frame = self.frame();
        Ok(replies)
    }
    fn update_holds(&mut self, p: &Pending) {
        let (new, _) = p.staged.state();
        let c = self.mixer.coefficients();
        let targets = self.mixer.targets();
        for (i, row) in self.holds.iter_mut().enumerate() {
            if matches!(p.request.command, Command::SetMode { .. }) {
                let indices: &[usize] = match p.scope {
                    Scope::Foh => &[0, 1, 2, 3],
                    Scope::Monitor1 => &[4],
                    Scope::Monitor2 => &[5],
                    Scope::Monitor(n) => &[usize::from(n) + 3],
                    _ => &[],
                };
                for &j in indices {
                    row[j] = Some(nano(c[i][j]));
                }
            } else {
                for parameter in new.iter().filter(|v| match &v.target {
                    crate::control_model::Target::Fader { input }
                    | crate::control_model::Target::Pan { input }
                    | crate::control_model::Target::Mute { input }
                    | crate::control_model::Target::Send { input, .. } => {
                        input == &format!("input-{:02}", i + 1)
                    }
                }) {
                    let indices: &[usize] = match &parameter.target {
                        crate::control_model::Target::Fader { .. } => &[0],
                        crate::control_model::Target::Pan { .. } => &[1, 2],
                        crate::control_model::Target::Mute { .. } => &[3],
                        crate::control_model::Target::Send { monitor, .. }
                            if monitor == "monitor-1" =>
                        {
                            &[4]
                        }
                        crate::control_model::Target::Send { monitor, .. } => &[monitor
                            .strip_prefix("monitor-")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap()
                            + 3],
                    };
                    for &j in indices {
                        if parameter.hold.is_none() {
                            row[j] = None;
                        } else if row[j].is_none()
                            || matches!(&p.request.command,Command::Set {targets} if targets.iter().any(|e|e.target==parameter.target))
                        {
                            row[j] = Some(nano(targets[i][j]));
                        }
                    }
                }
            }
        }
    }
}
fn nano(gain: f64) -> u64 {
    (gain * 1_000_000_000.0).round() as u64
}

/// Show intent only: leases, pending requests, mode grants and armed state are
/// deliberately unrepresentable. Restoring always requires explicit rearm.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineIntent {
    pub version: u32,
    pub show_id: String,
    pub topology: crate::topology::EngineTopology,
    pub parameters: Vec<Edit>,
    pub processing: Vec<crate::channel_processing::Config>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_taps: Option<Vec<Vec<crate::sends_wire::Tap>>>,
}

impl<'de> Deserialize<'de> for EngineIntent {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("intent object"))?;
        let old = ["version", "show_id", "topology", "parameters", "processing"];
        let version = value["version"]
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("intent version"))?;
        let count = match version {
            1 => 5,
            2 => 6,
            _ => return Err(serde::de::Error::custom("intent version")),
        };
        if object.len() != count
            || object
                .keys()
                .any(|k| !old.contains(&k.as_str()) && !(version == 2 && k == "send_taps"))
        {
            return Err(serde::de::Error::custom("intent fields"));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            version: u32,
            show_id: String,
            topology: crate::topology::EngineTopology,
            parameters: Vec<Edit>,
            processing: Vec<crate::channel_processing::Config>,
            send_taps: Option<Vec<Vec<crate::sends_wire::Tap>>>,
        }
        let fields: Fields = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            version: fields.version,
            show_id: fields.show_id,
            topology: fields.topology,
            parameters: fields.parameters,
            processing: fields.processing,
            send_taps: fields.send_taps,
        })
    }
}

impl EngineIntent {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let intent: Self =
            crate::show::decode_bounded(bytes, crate::snapshot_pages::ASSEMBLY_BYTES)?;
        // Reuse the actual off-render preparation and complete inventory validation.
        OfflineEngine::restore_intent(&intent, Counter(1), 0)?;
        Ok(intent)
    }
    pub fn save(&self, directory: &std::path::Path, name: &str) -> Result<()> {
        OfflineEngine::restore_intent(self, Counter(1), 0)?;
        crate::show::persist_bounded(directory, name, self, crate::snapshot_pages::ASSEMBLY_BYTES)
    }
}
