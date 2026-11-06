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
            let admission = topology.validate(resources.render_budget)?;
            // Historical GP07 v2/v4 producers used 672 bytes per strip. Read their
            // immutable evidence; runtime restore always admits the current state size.
            let mut legacy_admission = admission.clone();
            legacy_admission.estimated_bytes -= topology.inputs.len()
                * (std::mem::size_of::<crate::channel_processing::Strip>() - 672)
                * 2;
            if resources.render_budget != crate::topology::ResourceBudget::default()
                || (resources.admission != admission && resources.admission != legacy_admission)
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
/// Controller-owned validation. Neither authority nor protocol objects are
/// accessible through the owned source render state.
#[derive(Debug)]
struct ValidatedRender {
    staged: Authority,
    reply: Reply,
    now_ms: u64,
}
#[derive(Debug)]
struct ValidatedExternal {
    staged: Authority,
    reply: Reply,
    request: Request,
    scope: Scope,
    now_ms: u64,
    ticket: u64,
    epoch: u64,
    frame: u64,
}
pub(crate) mod source;
use source::{RenderPlan, SourceState};
#[derive(Debug)]
pub struct OfflineEngine {
    authority: Authority,
    validated_render: Option<ValidatedRender>,
    validated_external: Option<ValidatedExternal>,
    source: Option<SourceState>,
    source_topology: crate::topology::EngineTopology,
    source_closure: source::SourceClosure,
    pending_operator_tap: Option<crate::brain_control::MonitorSource>,
    pending_fx_sends: crate::mixer::PreparedFxSends,
    pending_source_prepared: Option<(Prepared, u64, u64)>,
    source_command: Option<source::SourceCommand>,
    detached_in_flight: bool,
    source_admission: Option<source::SourceAdmission>,
    observed: crate::mixer::SourceObservation,
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
        let legacy = topology.is_legacy();
        let mut source = SourceState::new(epoch.0, {
            let legacy = topology.is_legacy();
            let mut m = Mixer::from_topology(frame, topology)?;
            if !legacy {
                m.quiesce();
            }
            m
        });
        source.applied_revision = revision.0;
        Ok(Self {
            validated_render: None,
            validated_external: None,
            external_pending: None,
            clock: {
                let mut c = crate::clock_domain::ClockDomain::new(epoch.0, frame);
                if legacy {
                    c.rearm().map_err(String::from)?;
                }
                c
            },
            authority,
            observed: source.observation.clone(),
            source_topology: source.mixer.topology().clone(),
            source_closure: source.closure(),
            pending_operator_tap: None,
            pending_fx_sends: Default::default(),
            pending_source_prepared: None,
            source_command: None,
            detached_in_flight: false,
            source_admission: None,
            source: Some(source),
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
        if self.source.as_ref().is_some_and(SourceState::busy)
            || self.detached_in_flight
            || self.source_command.is_some()
            || self.pending_source_prepared.is_some()
            || self.validated_external.is_some()
            || self.pending.is_some()
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
        let frame = match self.admission_frame() {
            Some(frame) => frame,
            None => self
                .frame()
                .checked_div(48)
                .and_then(|n| n.checked_add(1))
                .and_then(|n| n.checked_mul(48))
                .ok_or("clock exhausted")?,
        };
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
        if self.validated_external.is_some()
            || !self.external_matches(request, fingerprint)
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
            if let Some(validated) = &mut self.validated_external {
                self.source_closure.cancel(validated.ticket);
                return;
            }
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
    #[cfg(feature = "hardware-host")]
    pub(crate) fn abort_external_preparation(&mut self) -> Result<()> {
        if self.validated_external.is_some() {
            return Err("external source outcome pending".into());
        }
        if let Some((request, scope, _)) = self.external_pending.take() {
            self.authority.cancel_external(&request, scope);
        }
        Ok(())
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
        let permit = self.prepare_external(now)?;
        let code = if permit.permitted() && apply(self).is_err() {
            1
        } else {
            0
        };
        self.finish_external(permit.receipt(code))
    }
    /// Validate once on the controller while preserving the global reservation.
    /// Only the returned numeric permit crosses into source execution.
    pub(crate) fn prepare_external(&mut self, now: u64) -> Result<source::ExternalPermit> {
        if self.validated_external.is_some() || self.validated_render.is_some() {
            return Err("external completion pending".into());
        }
        let (request, scope, frame) = self.external_pending.as_ref().ok_or("external missing")?;
        if self.preparation_frame() != *frame {
            return Err("external boundary".into());
        }
        let ticket = self.next_ticket;
        let next = ticket.checked_add(1).ok_or("ticket exhausted")?;
        let mut staged = self.authority.clone();
        let reply = staged.scoped_transaction(request, now, None, *scope);
        let mut permit = source::ExternalPermit::new(
            self.source_closure.clone(),
            self.clock.epoch,
            *frame,
            ticket,
            reply.kind == "applied",
        );
        permit.revision = reply.body.revision.0;
        permit.admission = self.source_admission.clone();
        // Delayed source application rechecks local lease expiry independently.
        if self.source_admission.is_some() {
            permit.expires = Some(
                std::time::Instant::now()
                    + std::time::Duration::from_millis(
                        self.authority
                            .live_lease_witness(request, now)
                            .map_or(0, |(_, remaining)| u64::from(remaining)),
                    ),
            );
        }
        self.validated_external = Some(ValidatedExternal {
            staged,
            reply,
            request: request.clone(),
            scope: *scope,
            now_ms: now,
            ticket,
            epoch: self.clock.epoch,
            frame: *frame,
        });
        self.next_ticket = next;
        Ok(permit)
    }
    /// Install authority only after the exact numeric source outcome returns.
    pub(crate) fn finish_external(&mut self, receipt: source::ExternalReceipt) -> Result<Reply> {
        let validated = self
            .validated_external
            .as_ref()
            .ok_or("external completion missing")?;
        if receipt.ticket != validated.ticket
            || receipt.epoch != validated.epoch
            || receipt.frame != validated.frame
            || self.source_closure.identity() != receipt.owner_id
        {
            return Err("external completion identity".into());
        }
        if let Some(output) = receipt.output {
            self.observed.output = output;
        }
        let mut validated = self.validated_external.take().unwrap();
        self.external_pending = None;
        if validated.reply.kind != "applied" {
            validated.staged.preserve_observations(&self.authority);
            self.authority = validated.staged;
            return Ok(validated.reply);
        }
        if receipt.code != 0 {
            return Ok(self.authority.scoped_transaction(
                &validated.request,
                validated.now_ms,
                Some("configuration_failed"),
                validated.scope,
            ));
        }
        validated.staged.preserve_observations(&self.authority);
        self.authority = validated.staged;
        // Compatibility callers may commit an external operation while the
        // source is still locally owned, before autonomous startup. Preserve
        // that completed identity on transfer; detached owners update it at the
        // numeric boundary and must never be overwritten from controller polls.
        if let Some(source) = &mut self.source {
            source.applied_revision = validated.reply.body.revision.0;
        }
        Ok(validated.reply)
    }
    /// Readiness acknowledgment after a successful source-side output rearm.
    /// Terminal closure is never reopened by a delayed completion.
    pub(crate) fn acknowledge_output_rearm(&mut self) -> Result<()> {
        if self.clock.state == crate::clock_domain::ClockState::Disarmed {
            self.clock.rearm().map_err(String::from)?;
        }
        if self.clock.state != crate::clock_domain::ClockState::Running {
            return Err("clock recovery required".into());
        }
        Ok(())
    }
    pub(crate) fn acknowledge_startup_rearm(
        &mut self,
        output: crate::mixer::OutputObservation,
    ) -> Result<()> {
        if self.frame() != 0 || self.source_closure.closed() {
            return Err("startup rearm identity".into());
        }
        self.acknowledge_output_rearm()?;
        self.observed.output = output;
        Ok(())
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
        self.source
            .as_ref()
            .map_or(&self.source_topology, |s| s.mixer.topology())
    }
    pub fn enable_fx_aux(&mut self) -> Result<()> {
        if let Some(source) = &mut self.source {
            source.mixer.enable_fx_aux()
        } else if self.observed.fx_aux_enabled {
            Ok(())
        } else {
            self.pending_fx_sends.enable(&self.source_topology)
        }
    }
    pub fn set_fx_sends(&mut self, bus: usize, levels: &[f64]) -> Result<()> {
        if let Some(source) = &mut self.source {
            source.mixer.set_fx_sends(bus, levels)
        } else {
            self.enable_fx_aux()?;
            self.pending_fx_sends
                .set(self.source_topology.inputs.len(), bus, levels)
        }
    }
    /// Controller readback of applied source send targets. Detached pending
    /// commands cannot appear here before their coherent source observation.
    pub fn fx_sends(&self, bus: usize) -> Option<Vec<f64>> {
        if let Some(source) = &self.source {
            source.mixer.fx_sends(bus)
        } else {
            (bus < 4 && self.observed.fx_aux_enabled)
                .then(|| self.observed.fx_sends.iter().map(|row| row[bus]).collect())
        }
    }
    pub fn set_operator_tap(&mut self, source: crate::brain_control::MonitorSource) {
        if let Some(active) = &mut self.source {
            active.mixer.set_operator_tap(source);
        } else {
            self.pending_operator_tap = Some(source);
        }
    }
    pub fn operator_tap(&self) -> &[f64; 96] {
        self.source
            .as_ref()
            .expect("operator samples belong to source owner")
            .mixer
            .operator_tap()
    }
    pub fn output_safety_gain(&self, frame: u64) -> f64 {
        // Independent terminal closure overrides the last observed envelope.
        // It does not invent a new completed-source quiescence observation.
        if self.source_closure.closed() {
            return 0.;
        }
        self.source.as_ref().map_or_else(
            || self.observed.output.gain_at(frame),
            |s| s.mixer.output_safety_gain(frame),
        )
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
        self.source_admission = None;
        self.validated_external = None;
        self.validated_render = None;
        self.pending_source_prepared = None;
        self.source_command = None;
        self.source_closure.close();
        if let Some(source) = &mut self.source {
            source.mixer.quiesce();
        }
        self.pending = None;
        self.processing_pending = None;
        self.sends_pending = None;
        self.sends_completions.clear();
        self.external_pending = None;
        self.authority.revoke_all();
    }
    pub(crate) fn acknowledge_source_quiescence(
        &mut self,
        output: crate::mixer::OutputObservation,
    ) {
        if self.source_closure.closed() {
            self.observed.output = output;
        }
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
        next.source.as_mut().unwrap().mixer.inherit_fx_send_intent(
            &self
                .source
                .as_ref()
                .ok_or("source must return before recovery")?
                .mixer,
        )?;
        next.clock = clock;
        *self = next;
        Ok(())
    }
    pub fn persisted_intent(&mut self) -> Result<EngineIntent> {
        let snapshot = self.authority.snapshot()?;
        Ok(EngineIntent {
            version: if self
                .observed
                .processing
                .iter()
                .any(|o| !o.1.gate.is_default())
            {
                3
            } else {
                2
            },
            show_id: snapshot.show_id,
            send_taps: Some(
                self.observed
                    .sends
                    .iter()
                    .map(|r| r.iter().map(|s| s.1).collect())
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
            processing: self.observed.processing.iter().map(|o| o.1).collect(),
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
        if ![1, 2, 3].contains(&intent.version)
            || intent.processing.len() != intent.topology.inputs.len()
            || (intent.version == 1 && intent.send_taps.is_some())
            || (intent.version >= 2 && intent.send_taps.is_none())
            || (intent.version < 3 && intent.processing.iter().any(|c| !c.gate.is_default()))
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
        e.source
            .as_mut()
            .unwrap()
            .mixer
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
            e.source.as_mut().unwrap().mixer.restore_sends(taps)?;
        }
        e.clock.state = crate::clock_domain::ClockState::Disarmed;
        e.source.as_mut().unwrap().mixer.quiesce();
        e.source
            .as_mut()
            .unwrap()
            .mixer
            .observe_into(&mut e.observed);
        Ok(e)
    }
    pub fn mute_outputs(&mut self) -> Result<()> {
        self.source
            .as_mut()
            .ok_or("source detached")?
            .mixer
            .mute_outputs()
            .map_err(|e| format!("{e:?}"))
    }
    pub fn outputs_quiesced(&self) -> bool {
        self.source.as_ref().map_or_else(
            || self.observed.output.quiesced_at(self.observed.frame),
            |s| s.mixer.outputs_quiesced(),
        )
    }
    pub fn rearm_outputs(&mut self) -> Result<()> {
        if self.source_faulted()
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
        self.source
            .as_mut()
            .ok_or("source detached")?
            .mixer
            .rearm()
            .map_err(|e| format!("{e:?}"))?;
        self.source_closure.reopen();
        Ok(())
    }
    pub fn prepare_output_patch(
        &self,
        outputs: Vec<crate::topology::OutputPort>,
    ) -> Result<crate::mixer::PreparedOutputPatch> {
        crate::mixer::PreparedOutputPatch::for_topology(self.topology(), outputs)
    }
    /// Publish only the returned, successfully applied patch after its external
    /// receipt has been accepted. No active source access is required.
    pub(crate) fn acknowledge_output_patch(
        &mut self,
        prepared: &crate::mixer::PreparedOutputPatch,
    ) -> Result<()> {
        let topology = prepared
            .applied_topology()
            .ok_or("output patch not applied")?;
        if topology.map_revision
            != self
                .source_topology
                .map_revision
                .checked_add(1)
                .ok_or("map exhausted")?
        {
            return Err("output patch publication order".into());
        }
        self.source_topology.clone_from(topology);
        Ok(())
    }
    pub fn apply_output_patch(
        &mut self,
        prepared: &mut crate::mixer::PreparedOutputPatch,
    ) -> Result<()> {
        self.source
            .as_mut()
            .ok_or("source detached")?
            .mixer
            .apply_output_patch(prepared)
            .map_err(String::from)
    }
    pub fn replace_output_patch(
        &mut self,
        outputs: Vec<crate::topology::OutputPort>,
    ) -> Result<()> {
        self.source
            .as_mut()
            .ok_or("source detached")?
            .mixer
            .replace_output_patch(outputs)
    }
    pub fn rearm(&mut self) -> Result<()> {
        if self.source_faulted()
            || self
                .frame()
                .checked_add(crate::mixer::RAMP_FRAMES)
                .is_none()
        {
            return Err("fault/clock recovery required".into());
        }
        self.clock.rearm().map_err(String::from)?;
        self.source
            .as_mut()
            .ok_or("source detached")?
            .mixer
            .rearm()
            .map_err(|e| format!("{e:?}"))?;
        self.source_closure.reopen();
        Ok(())
    }
    pub(crate) fn pending_sends_ticket_for(&self, writer: &str) -> Option<u64> {
        self.sends_pending
            .as_ref()
            .filter(|p| p.request.writer.as_deref() == Some(writer))
            .map(|p| p.ticket)
    }
    /// Returns true when the source may already have applied the operation.
    /// Keep its authority record until its actual outcome returns; cancellation
    /// is only a source-boundary request, never proof that DSP did not commit.
    fn revoke_source_ticket(&mut self, writer: &str, ticket: u64) -> bool {
        self.source_closure.cancel(ticket);
        let transferred = self.source.is_none()
            && self
                .pending_source_prepared
                .as_ref()
                .is_none_or(|(_, pending, _)| *pending != ticket);
        if transferred {
            if let Some(validated) = &mut self.validated_render {
                validated.staged.revoke_writer(writer);
            }
            return true;
        }
        self.validated_render = None;
        if self
            .pending_source_prepared
            .as_ref()
            .is_some_and(|(_, pending, _)| *pending == ticket)
        {
            self.pending_source_prepared = None;
        }
        if let Some(source) = &mut self.source {
            source.mixer.cancel(ticket);
        }
        false
    }
    pub fn revoke_writer(&mut self, writer: &str) {
        self.authority.revoke_writer(writer);
        if let Some(validated) = &mut self.validated_render {
            validated.staged.revoke_writer(writer);
        }
        if let Some(validated) = &mut self.validated_external {
            validated.staged.revoke_writer(writer);
            if validated.request.writer.as_deref() == Some(writer) {
                self.source_closure.cancel(validated.ticket);
            }
        }
        if self
            .external_pending
            .as_ref()
            .is_some_and(|p| p.0.writer.as_deref() == Some(writer))
            && self.validated_external.is_none()
        {
            self.external_pending = None;
        }
        if let Some(ticket) = self
            .pending
            .as_ref()
            .filter(|p| p.request.writer.as_deref() == Some(writer))
            .map(|p| p.ticket)
            && !self.revoke_source_ticket(writer, ticket)
        {
            self.pending = None;
        }
        if let Some(ticket) = self
            .processing_pending
            .as_ref()
            .filter(|p| p.request.writer.as_deref() == Some(writer))
            .map(|p| p.ticket)
            && !self.revoke_source_ticket(writer, ticket)
        {
            self.processing_pending = None;
        }
        self.outcomes.remove(writer);
        if let Some(ticket) = self
            .sends_pending
            .as_ref()
            .filter(|p| p.request.writer.as_deref() == Some(writer))
            .map(|p| p.ticket)
            && !self.revoke_source_ticket(writer, ticket)
        {
            self.sends_pending = None;
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
    fn schedule_source(&mut self, prepared: Prepared, ticket: u64) -> Result<u64> {
        if let Some(source) = &mut self.source {
            return source.schedule(prepared, ticket);
        }
        if self.pending_source_prepared.is_some()
            || self.source_command.is_some()
            || self.validated_render.is_some()
            || self.detached_in_flight
        {
            return Err("capacity".into());
        }
        let frame = self
            .preparation_frame()
            .checked_div(48)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| n.checked_mul(48))
            .ok_or("clock exhausted")?;
        frame
            .checked_add(crate::mixer::RAMP_FRAMES)
            .ok_or("clock exhausted")?;
        self.pending_source_prepared = Some((prepared, ticket, frame));
        Ok(frame)
    }
    fn source_faulted(&self) -> bool {
        self.source
            .as_ref()
            .map_or(self.observed.faulted, |s| s.mixer.faulted())
    }
    pub fn frame(&self) -> u64 {
        self.source
            .as_ref()
            .map_or(self.observed.frame, |s| s.mixer.frame())
    }
    pub fn revision(&self) -> Counter {
        self.authority.revision()
    }
    pub fn mixer(&self) -> &Mixer {
        &self
            .source
            .as_ref()
            .expect("active mixer belongs to source owner")
            .mixer
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
        let current = &self.observed.current;
        let target = &self.observed.targets;
        Ok(RenderedSnapshot {
            capability: "GP03-rendered".into(),
            capability_version: self.authority.version(),
            rendered_application: true,
            release_commit: true,
            frame: Counter(self.observed.frame),
            faulted: self.observed.faulted,
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
                c.next_frame = self.observed.frame;
                c
            }),
            authority: self.authority.snapshot()?,
            coefficients: (0..self.observed.current.len())
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
            frame: Counter(self.observed.frame),
            sample_rate: 48000,
            foh_tap: crate::processing_wire::FOH_TAP.into(),
            monitor_tap: if self.topology().is_legacy() {
                crate::processing_wire::MONITOR_TAP
            } else {
                crate::processing_wire::PER_SEND_TAP
            }
            .into(),
            faulted: self.observed.faulted,
            channels: self
                .observed
                .processing
                .iter()
                .copied()
                .enumerate()
                .map(|(i, (current, target, remaining, gr))| Channel {
                    input: format!("input-{:02}", i + 1),
                    current,
                    target,
                    transition_remaining_frames: remaining,
                    ready: remaining == 0,
                    gain_reduction_mdb: gr,
                    gate: self.observed.gates[i].map(|g| g.readback()),
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
        if (r.version != self.processing_version()
            && !(self.processing_version() == 4 && r.version == 5))
            || (r.version < 5
                && self
                    .observed
                    .processing
                    .iter()
                    .any(|o| !o.0.gate.is_default() || !o.1.gate.is_default()))
        {
            return Ok(ProcessingReply::new(
                r,
                "final",
                Some("unsupported_version".into()),
                self.revision(),
            ));
        }
        if let ProcessingCommand::ProcessingSet { input, .. } = &r.command
            && crate::processing_wire::input_index(input)? >= self.topology().inputs.len()
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
            self.source_faulted().then_some("faulted")
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
            || self.observed.processing.iter().any(|p| p.2 != 0)
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
            self.topology().inputs.len(),
            self.topology().monitors,
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
        let Ok(frame) = self.schedule_source(prepared, self.next_ticket) else {
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
            frame: Counter(self.observed.frame),
            sample_rate: 48000,
            supported_modes: crate::sends_wire::MODES.to_vec(),
            tap_positions: crate::sends_wire::POSITIONS
                .iter()
                .map(|s| (*s).into())
                .collect(),
            monitors: (1..=self.topology().monitors)
                .map(|m| format!("monitor-{m}"))
                .collect(),
            faulted: self.observed.faulted,
            channels: self
                .observed
                .sends
                .iter()
                .enumerate()
                .map(|(i, row)| crate::sends_wire::Channel {
                    input: format!("input-{:02}", i + 1),
                    sends: row
                        .iter()
                        .copied()
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
            && (crate::processing_wire::input_index(input)? >= self.topology().inputs.len()
                || crate::sends_wire::monitor_index(monitor)? >= self.topology().monitors)
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
            self.source_faulted().then_some("faulted")
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
            || self.observed.sends.iter().flatten().any(|s| s.2 != 0)
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
            self.topology().inputs.len(),
            self.topology().monitors,
            crate::processing_wire::input_index(input)?,
            crate::sends_wire::monitor_index(monitor)?,
            *tap,
        )?;
        let Some(next) = self.next_ticket.checked_add(1) else {
            return Ok(SendsReply::new(r, "backpressure", None, self.revision()));
        };
        let Ok(frame) = self.schedule_source(prepared, self.next_ticket) else {
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
                self.topology().inputs.len(),
                self.topology().monitors,
                targets,
            )?,
            Command::SetMode { .. } => Prepared::freeze_for(
                self.topology().inputs.len(),
                self.topology().monitors,
                scope,
            ),
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
                Prepared::edits_for(
                    self.topology().inputs.len(),
                    self.topology().monitors,
                    &edits,
                )?
            }
            _ => unreachable!(),
        };
        let Some(next) = self.next_ticket.checked_add(1) else {
            return Ok(Self::backpressure(r));
        };
        let frame = match self.schedule_source(prepared, self.next_ticket) {
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
        if !self.topology().is_legacy() {
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
        let ni = self.topology().inputs.len();
        let no = self.topology().monitors + 2;
        if !input.len().is_multiple_of(ni) || output.len() != input.len() / ni * no {
            return Err("buffer length".into());
        }
        let frames = input.len() / ni;
        let end = self
            .frame()
            .checked_add(frames as u64)
            .ok_or("clock exhausted")?;
        if self.validated_external.is_none()
            && self
                .external_boundary()
                .is_some_and(|boundary| end > boundary)
        {
            return Err("external boundary requires commit before render".into());
        }
        // Preserve the historical offline API's boundary-sample snapshot. The
        // production source caller uses a complete prepared quantum and observes
        // it after PA/REC; it does not interleave this compatibility pump with DSP.
        let boundary = self
            .processing_pending
            .as_ref()
            .map(|p| p.frame)
            .or_else(|| self.sends_pending.as_ref().map(|p| p.frame))
            .or_else(|| self.pending.as_ref().map(|p| p.frame));
        let first = boundary
            .filter(|f| *f < end)
            .map_or(frames, |f| (f - self.frame()) as usize + 1);
        self.prepare_render(first, now_ms)?;
        let _ = self.render_prepared(&input[..first * ni], &mut output[..first * no]);
        let replies = self.finish_render()?;
        if first < frames {
            self.prepare_render(frames - first, now_ms)?;
            let _ = self.render_prepared(&input[first * ni..], &mut output[first * no..]);
            debug_assert!(self.validated_render.is_none());
            self.finish_render()?;
        }
        Ok(replies)
    }

    /// Enter a source-issued preparation window. Public frame/readback continues
    /// to report the last observed source state, never this future reservation.
    pub(crate) fn begin_source_admission(
        &mut self,
        admission: source::SourceAdmission,
    ) -> Result<()> {
        if self.source.is_some()
            || self.detached_in_flight
            || self.source_command.is_some()
            || self.validated_render.is_some()
            || self.validated_external.is_some()
            || self.source_admission.is_some()
            || admission.epoch != self.clock.epoch
            || !self.source_closure.same_owner(&admission.owner)
            || self.source_closure.closed()
            || admission.frame < self.frame()
        {
            return Err("source admission ownership/state".into());
        }
        self.source_admission = Some(admission);
        Ok(())
    }
    pub(crate) fn admission_frame(&self) -> Option<u64> {
        self.source_admission
            .as_ref()
            .map(source::SourceAdmission::frame)
    }
    pub(crate) fn end_source_admission(&mut self) {
        self.source_admission = None;
    }
    fn preparation_frame(&self) -> u64 {
        self.admission_frame().unwrap_or_else(|| self.frame())
    }

    /// Controller phase: validate the one globally reserved transaction and
    /// admit a numeric plan to owned DSP state. No metadata is published yet.
    pub(crate) fn prepare_render(&mut self, frames: usize, now_ms: u64) -> Result<()> {
        if self.source.as_ref().is_some_and(SourceState::busy)
            || self.source_command.is_some()
            || self.validated_render.is_some()
            || self.detached_in_flight
        {
            return Err("render completion not observed".into());
        }
        self.authority.observe_control_time(now_ms)?;
        let frame = self.preparation_frame();
        let end = frame.checked_add(frames as u64).ok_or("clock exhausted")?;
        if self.validated_external.is_none()
            && self
                .external_boundary()
                .is_some_and(|boundary| end > boundary)
        {
            return Err("external boundary requires commit before render".into());
        }
        if !self.topology().is_legacy() && frames > self.topology().max_block_frames {
            return Err("buffer length".into());
        }
        let validation_end = if self
            .source_admission
            .as_ref()
            .is_some_and(|a| a.frame > self.frame())
        {
            end.checked_add(1).ok_or("clock exhausted")?
        } else {
            end
        };
        let mut commit = None;
        let fault = self.source_faulted().then_some("faulted");
        if let Some(p) = self
            .processing_pending
            .as_ref()
            .filter(|p| p.frame < validation_end)
        {
            let mut staged = self.authority.clone();
            let reply = staged.processing_transaction(&p.request.history_request(), now_ms, fault);
            commit = Some((p.ticket, p.frame, reply.kind == "applied", true));
            self.validated_render = Some(ValidatedRender {
                staged,
                reply,
                now_ms,
            });
        } else if let Some(p) = self
            .sends_pending
            .as_ref()
            .filter(|p| p.frame < validation_end)
        {
            let mut staged = self.authority.clone();
            let reply = staged.scoped_transaction(
                &p.request.history_request(),
                now_ms,
                fault,
                p.request.scope()?,
            );
            commit = Some((p.ticket, p.frame, reply.kind == "applied", true));
            self.validated_render = Some(ValidatedRender {
                staged,
                reply,
                now_ms,
            });
        } else if let Some(p) = self.pending.as_ref().filter(|p| p.frame < validation_end) {
            let mut staged = self.authority.clone();
            let reply = staged.handle(&p.request, now_ms);
            commit = Some((p.ticket, p.frame, reply.kind == "applied", false));
            self.validated_render = Some(ValidatedRender {
                staged,
                reply,
                now_ms,
            });
        }
        let fence = self
            .external_boundary()
            .filter(|_| self.validated_external.is_none())
            .into_iter()
            .chain(
                self.processing_pending
                    .as_ref()
                    .map(|p| p.frame)
                    .or_else(|| self.sends_pending.as_ref().map(|p| p.frame))
                    .or_else(|| self.pending.as_ref().map(|p| p.frame))
                    .filter(|_| commit.is_none()),
            )
            .min();
        let plan = RenderPlan {
            revision: self
                .validated_render
                .as_ref()
                .map_or(self.revision().0, |validated| {
                    validated.reply.body.revision.0
                }),
            frame,
            frames,
            commit,
            fence,
        };
        if let Some(source) = &mut self.source {
            source.admit(plan);
        } else {
            self.detached_in_flight = true;
            self.source_command = Some(source::SourceCommand {
                owner: self.source_closure.clone(),
                epoch: self.clock.epoch,
                prepared: self.pending_source_prepared.take(),
                plan,
                operator_tap: self.pending_operator_tap.take(),
                fx_sends: std::mem::take(&mut self.pending_fx_sends),
                admission: self.source_admission.clone(),
                installed: false,
                expires: None,
                #[cfg(feature = "hardware-host")]
                authority_fence: None,
            });
        }
        Ok(())
    }

    #[cfg(feature = "hardware-host")]
    pub(crate) fn queued_render_replies(&self) -> usize {
        self.processing_completions.len() + self.sends_completions.len()
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn pending_lease_remaining(&self, now: u64) -> Option<u32> {
        let request = self
            .pending
            .as_ref()
            .map(|p| p.request.clone())
            .or_else(|| {
                self.processing_pending
                    .as_ref()
                    .map(|p| p.request.history_request())
            })
            .or_else(|| {
                self.sends_pending
                    .as_ref()
                    .map(|p| p.request.history_request())
            });
        request.and_then(|r| {
            self.authority
                .live_lease_witness(&r, now)
                .ok()
                .map(|(_, ms)| ms)
        })
    }
    pub(crate) fn take_source_command(&mut self) -> Option<source::SourceCommand> {
        self.source_command.take()
    }

    /// Finish a returned packet which was refused before any source mutation.
    /// Refusal is protocol history, not a source fault or a published revision.
    pub(crate) fn reject_source_command(
        &mut self,
        command: &source::SourceCommand,
    ) -> Result<Vec<RenderedReply>> {
        if command.installed
            || !self.detached_in_flight
            || self.source.is_some()
            || command.epoch != self.clock.epoch
            || !self.source_closure.same_owner(&command.owner)
        {
            return Err("source refusal ownership/state".into());
        }
        self.reject_uninstalled_preparation()
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn abort_source_preparation(&mut self) -> Result<Vec<RenderedReply>> {
        if self.source.is_some() || self.detached_in_flight || self.source_command.is_some() {
            return Err("source preparation already published".into());
        }
        self.reject_uninstalled_preparation()
    }
    fn reject_uninstalled_preparation(&mut self) -> Result<Vec<RenderedReply>> {
        self.detached_in_flight = false;
        self.validated_render = None;
        self.pending_source_prepared = None;
        let mut replies = Vec::new();
        if let Some(p) = self.pending.take() {
            let reply = Self::wrapped(self.authority.reply(
                &p.request,
                "rejected",
                Some("source_admission"),
            ));
            self.remember(p.request, reply.clone());
            replies.push(reply);
        }
        if let Some(p) = self.processing_pending.take() {
            let reply = ProcessingReply::new(
                &p.request,
                "final",
                Some("source_admission".into()),
                self.revision(),
            );
            self.remember_processing(p.request, reply.clone());
            self.processing_completions.push(reply);
        }
        if let Some(p) = self.sends_pending.take() {
            let reply = SendsReply::new(
                &p.request,
                "final",
                Some("source_admission".into()),
                self.revision(),
            );
            self.remember_sends(p.request, reply.clone());
            self.sends_completions.push(reply);
        }
        Ok(replies)
    }

    /// Transfer the sole DSP owner. Controller readback remains at its last
    /// coherent observation; synchronous mutation entry points refuse detachment.
    pub(crate) fn take_source(&mut self) -> Result<SourceState> {
        let source = self.source.take().ok_or("source already transferred")?;
        self.source_topology.clone_from(source.mixer.topology());
        self.detached_in_flight = source.busy();
        Ok(source)
    }
    pub(crate) fn restore_source(&mut self, mut source: SourceState) -> Result<()> {
        if self.source.is_some()
            || source.epoch() != self.clock.epoch
            || !self.source_closure.same_owner(&source.closure())
        {
            return Err("source ownership/epoch mismatch".into());
        }
        self.source_topology.clone_from(source.mixer.topology());
        if let Some(tap) = self.pending_operator_tap.take() {
            source.mixer.set_operator_tap(tap);
        }
        if !source.mixer.apply_fx_sends(&mut self.pending_fx_sends) {
            return Err("FX aux deferred command refused".into());
        }
        self.pending_fx_sends = Default::default();
        self.source = Some(source);
        self.detached_in_flight = false;
        Ok(())
    }
    pub(crate) fn source_completion(&self) -> source::SourceCompletion {
        source::SourceCompletion {
            owner: self.source_closure.clone(),
            epoch: self.clock.epoch,
            receipt: None,
            observation: self.observed.clone(),
        }
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn finish_source_observation(
        &mut self,
        packet: &mut source::SourceCompletion,
    ) -> Result<()> {
        if self.source.is_some()
            || packet.epoch != self.clock.epoch
            || !self.source_closure.same_owner(&packet.owner)
            || packet.receipt.is_some()
            || self.detached_in_flight
        {
            return Err("source observation ownership/state".into());
        }
        std::mem::swap(&mut self.observed, &mut packet.observation);
        self.clock.next_frame = self.observed.frame;
        Ok(())
    }
    pub(crate) fn finish_source_completion(
        &mut self,
        packet: &mut source::SourceCompletion,
    ) -> Result<Vec<RenderedReply>> {
        if self.source.is_some()
            || packet.epoch != self.clock.epoch
            || !self.source_closure.same_owner(&packet.owner)
        {
            return Err("source completion ownership/epoch".into());
        }
        let receipt = packet.receipt.take().ok_or("render receipt missing")?;
        self.detached_in_flight = false;
        std::mem::swap(&mut self.observed, &mut packet.observation);
        self.finish_receipt(receipt)
    }
    /// Bounded source phase; the owned state retains its receipt until observed.
    pub(crate) fn render_prepared(
        &mut self,
        input: &[f64],
        output: &mut [f64],
    ) -> std::result::Result<(), crate::mixer::ProcessError> {
        let Some(source) = &mut self.source else {
            output.fill(0.);
            return Err(crate::mixer::ProcessError::Faulted);
        };
        source.render(self.clock.epoch, input, output)
    }

    /// Controller phase: observe the numeric result, destroy retired storage,
    /// install applied authority and create coherent end-of-quantum readback.
    pub(crate) fn finish_render(&mut self) -> Result<Vec<RenderedReply>> {
        let source = self.source.as_mut().ok_or("source detached")?;
        let receipt = source.take_receipt().ok_or("render receipt missing")?;
        std::mem::swap(&mut self.observed, &mut source.observation);
        self.finish_receipt(receipt)
    }

    fn finish_receipt(&mut self, receipt: source::RenderReceipt) -> Result<Vec<RenderedReply>> {
        self.clock.next_frame = self.observed.frame;
        if let Err(error) = receipt.result {
            self.validated_render = None;
            return Err(format!("{error:?}"));
        }
        let mut replies = Vec::new();
        let Some(mut validated) = self.validated_render.take() else {
            return Ok(replies);
        };
        let done = receipt.retirement.completion; // owned storage drops on this controller
        if let Some(p) = self.processing_pending.take() {
            if receipt.fault_before_commit || receipt.cancelled_before_commit {
                validated.staged = self.authority.clone();
                validated.reply = validated.staged.processing_transaction(
                    &p.request.history_request(),
                    validated.now_ms,
                    Some(if receipt.cancelled_before_commit {
                        "lease"
                    } else {
                        "faulted"
                    }),
                );
            }
            let applied = validated.reply.kind == "applied";
            if applied
                && done
                    .as_ref()
                    .is_none_or(|d| d.ticket != p.ticket || d.frame != p.frame)
            {
                return Err("completion identity".into());
            }
            self.authority = validated.staged;
            let mut reply = ProcessingReply::new(
                &p.request,
                "final",
                validated.reply.body.reason,
                self.revision(),
            );
            if applied {
                reply.ticket = Some(Counter(p.ticket));
                reply.effective_frame = Some(Counter(p.frame));
                reply.ramp_frames = Some(240);
                reply.snapshot = Some(self.processing_snapshot()?);
            }
            self.remember_processing(p.request, reply.clone());
            self.processing_completions.push(reply);
        } else if let Some(p) = self.sends_pending.take() {
            if receipt.fault_before_commit || receipt.cancelled_before_commit {
                validated.staged = self.authority.clone();
                validated.reply = validated.staged.scoped_transaction(
                    &p.request.history_request(),
                    validated.now_ms,
                    Some(if receipt.cancelled_before_commit {
                        "lease"
                    } else {
                        "faulted"
                    }),
                    p.request.scope()?,
                );
            }
            let applied = validated.reply.kind == "applied";
            if applied
                && done
                    .as_ref()
                    .is_none_or(|d| d.ticket != p.ticket || d.frame != p.frame)
            {
                return Err("completion identity".into());
            }
            self.authority = validated.staged;
            let mut reply = SendsReply::new(
                &p.request,
                "final",
                validated.reply.body.reason,
                self.revision(),
            );
            if applied {
                reply.ticket = Some(Counter(p.ticket));
                reply.effective_frame = Some(Counter(p.frame));
                reply.ramp_frames = Some(240);
                reply.snapshot = Some(self.sends_snapshot()?);
            }
            self.remember_sends(p.request, reply.clone());
            self.sends_completions.push(reply);
        } else {
            let mut p = self.pending.take().ok_or("completion without pending")?;
            p.staged = validated.staged;
            p.reply = validated.reply;
            if receipt.cancelled_before_commit {
                p.staged = self.authority.clone();
                p.reply = p.staged.reply(&p.request, "rejected", Some("lease"));
            }
            if p.reply.kind != "applied" {
                self.authority = p.staged;
                let reply = Self::wrapped(p.reply);
                self.remember(p.request, reply.clone());
                replies.push(reply);
            } else {
                if done
                    .as_ref()
                    .is_none_or(|d| d.ticket != p.ticket || d.frame != p.frame)
                {
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
        }
        Ok(replies)
    }
    fn update_holds(&mut self, p: &Pending) {
        let (new, _) = p.staged.state();
        let c = &self.observed.current;
        let targets = &self.observed.targets;
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
            2 | 3 => 6,
            _ => return Err(serde::de::Error::custom("intent version")),
        };
        if object.len() != count
            || object
                .keys()
                .any(|k| !old.contains(&k.as_str()) && !(version >= 2 && k == "send_taps"))
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

#[cfg(test)]
mod render_boundary_tests {
    use super::*;

    fn unprepared_processing() -> (OfflineEngine, ProcessingRequest) {
        let mut engine = OfflineEngine::new(
            "11111111-1111-4111-8111-111111111111",
            Counter(9),
            Counter(0),
            0,
        )
        .unwrap();
        let request = Request {
            contract: "C-AUDIO".into(),
            version: 1,
            show_id: "11111111-1111-4111-8111-111111111111".into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some("desk".into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant { scope: Scope::Foh },
        };
        let lease = engine
            .handle(&request, 0)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease;
        let request = ProcessingRequest {
            contract: "GP07-processing".into(),
            version: 2,
            show_id: request.show_id,
            module: request.module,
            epoch: request.epoch,
            writer: request.writer,
            lease,
            request_id: Some(Counter(2)),
            expected_revision: Some(Counter(0)),
            command: ProcessingCommand::ProcessingSet {
                input: "input-01".into(),
                config: crate::channel_processing::Config {
                    eq_bypass: false,
                    band2_gain_mdb: 6000,
                    ..Default::default()
                },
            },
        };
        (engine, request)
    }
    fn prepared_processing() -> (OfflineEngine, ProcessingRequest) {
        let (mut engine, request) = unprepared_processing();
        assert_eq!(
            engine.handle_processing(&request, 1).unwrap().state,
            "pending"
        );
        (engine, request)
    }

    #[test]
    fn admitted_future_processing_keeps_observed_time_and_runs_without_second_pump() {
        let (mut engine, request) = unprepared_processing();
        let mut packet = engine.source_completion();
        let mut source = engine.take_source().unwrap();
        let mut output = [0.; 48 * 4];
        engine.prepare_render(48, 1).unwrap();
        let mut bootstrap = engine.take_source_command().unwrap();
        source.install_command(&mut bootstrap).unwrap();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        assert!(source.publish_completion(&mut packet));
        engine.finish_source_completion(&mut packet).unwrap();

        let admission = source.reserve_admission(144).unwrap();
        assert_eq!(
            (admission.epoch(), admission.frame(), admission.ticket()),
            (9, 144, 1)
        );
        engine.begin_source_admission(admission).unwrap();
        assert_eq!(engine.frame(), 48);
        assert_eq!(engine.snapshot().unwrap().frame, Counter(48));
        assert_eq!(
            engine.handle_processing(&request, 2).unwrap().state,
            "pending"
        );
        assert_eq!(engine.processing_pending.as_ref().unwrap().frame, 192);
        engine.prepare_render(48, 2).unwrap();
        let mut command = engine.take_source_command().unwrap();
        engine.end_source_admission();
        crate::remote::assert_no_allocations(|| {
            for _ in 0..2 {
                source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            }
            assert!(!source.publish_completion(&mut packet));
            source.install_command(&mut command).unwrap();
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            assert_eq!(source.mixer.frame(), 192);
            assert!(!source.publish_completion(&mut packet));
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            assert!(source.publish_completion(&mut packet));
        });
        assert_eq!(engine.frame(), 48);
        engine.finish_source_completion(&mut packet).unwrap();
        assert_eq!(engine.frame(), 240);
        assert_eq!(engine.revision(), Counter(1));
        assert_eq!(
            engine.take_processing_completions()[0].effective_frame,
            Some(Counter(192))
        );
        assert!(engine.reject_source_command(&command).is_err());
    }

    #[test]
    fn clocked_lease_expiry_cancels_installed_future_mutation_without_publication() {
        let (mut engine, request) = unprepared_processing();
        let mut packet = engine.source_completion();
        let mut source = engine.take_source().unwrap();
        let mut output = [0.; 48 * 4];
        engine.prepare_render(48, 1).unwrap();
        let mut bootstrap = engine.take_source_command().unwrap();
        source.install_command(&mut bootstrap).unwrap();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        source.publish_completion(&mut packet);
        engine.finish_source_completion(&mut packet).unwrap();
        let admission = source.reserve_admission(96).unwrap();
        engine.begin_source_admission(admission).unwrap();
        assert_eq!(
            engine.handle_processing(&request, 2).unwrap().state,
            "pending"
        );
        engine.prepare_render(48, 2).unwrap();
        let mut command = engine.take_source_command().unwrap();
        engine.end_source_admission();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        // Exercise the numeric commit's own deadline check, independently of
        // the executor's pre-install deadline gate.
        command.expires = Some(std::time::Instant::now());
        source.install_command(&mut command).unwrap();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        assert!(source.publish_completion(&mut packet));
        engine.finish_source_completion(&mut packet).unwrap();
        assert_eq!(engine.revision(), Counter(0));
        assert_eq!(
            engine.take_processing_completions()[0].reason.as_deref(),
            Some("lease")
        );
        assert_eq!(source.mixer.frame(), 192);
    }

    #[test]
    #[cfg(feature = "hardware-host")]
    fn clocked_session_closure_cancels_delayed_commit_without_controller_service() {
        let (mut engine, request) = unprepared_processing();
        let mut packet = engine.source_completion();
        let mut source = engine.take_source().unwrap();
        let mut output = [0.; 48 * 4];
        engine.prepare_render(48, 1).unwrap();
        let mut bootstrap = engine.take_source_command().unwrap();
        source.install_command(&mut bootstrap).unwrap();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        source.publish_completion(&mut packet);
        engine.finish_source_completion(&mut packet).unwrap();
        let admission = source.reserve_admission(96).unwrap();
        engine.begin_source_admission(admission).unwrap();
        assert_eq!(
            engine.handle_processing(&request, 2).unwrap().state,
            "pending"
        );
        engine.prepare_render(48, 2).unwrap();
        let mut command = engine.take_source_command().unwrap();
        engine.end_source_admission();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        use sha2::{Digest, Sha256};
        let policy = crate::remote::PolicyStore::new(vec![crate::remote::Peer {
            id: "clock-peer".into(),
            certificate_sha256: format!("{:x}", Sha256::digest(b"clock-peer")),
            permissions: [crate::remote::Permission::Foh].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"clock-peer", 1).unwrap();
        command.authority_fence = Some(context.audio_fence());
        source.install_command(&mut command).unwrap();
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        context.close_audio();
        crate::remote::assert_no_allocations(|| {
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        });
        assert!(source.publish_completion(&mut packet));
        engine.finish_source_completion(&mut packet).unwrap();
        assert_eq!(engine.revision(), Counter(0));
        assert_eq!(
            engine.take_processing_completions()[0].reason.as_deref(),
            Some("lease")
        );
        assert_eq!(source.mixer.frame(), 192);
    }

    #[test]
    fn expired_or_superseded_admission_returns_storage_without_stopping_source() {
        for superseded in [false, true] {
            let (mut engine, request) = unprepared_processing();
            let mut packet = engine.source_completion();
            let mut source = engine.take_source().unwrap();
            let mut output = [0.; 48 * 4];
            engine.prepare_render(48, 1).unwrap();
            let mut bootstrap = engine.take_source_command().unwrap();
            source.install_command(&mut bootstrap).unwrap();
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            assert!(source.publish_completion(&mut packet));
            engine.finish_source_completion(&mut packet).unwrap();
            let admission = source.reserve_admission(96).unwrap();
            engine.begin_source_admission(admission).unwrap();
            engine.handle_processing(&request, 2).unwrap();
            engine.prepare_render(48, 2).unwrap();
            let mut command = engine.take_source_command().unwrap();
            engine.end_source_admission();
            if superseded {
                source.reserve_admission(96).unwrap();
            }
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            if !superseded {
                source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            }
            crate::remote::assert_no_allocations(|| {
                assert!(source.install_command(&mut command).is_err());
                for _ in 0..4 {
                    source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
                }
            });
            assert!(command.prepared.is_some());
            engine.reject_source_command(&command).unwrap();
            assert_eq!(engine.revision(), Counter(0));
            assert_eq!(
                engine.take_processing_completions()[0].reason.as_deref(),
                Some("source_admission")
            );
            assert!(!source.mixer.faulted());
            assert!(engine.processing_pending.is_none());
        }
    }

    #[test]
    fn admission_binds_owner_epoch_and_terminal_closure() {
        let (mut engine, _) = unprepared_processing();
        let mut source = engine.take_source().unwrap();
        let admission = source.reserve_admission(48).unwrap();
        let (mut other, _) = unprepared_processing();
        let _other_source = other.take_source().unwrap();
        assert!(other.begin_source_admission(admission.clone()).is_err());
        source.closure().close();
        assert!(engine.begin_source_admission(admission).is_err());
        assert!(source.reserve_admission(48).is_err());
    }

    #[test]
    fn detached_output_observation_and_patch_publication_do_not_borrow_source() {
        let (mut engine, _) = unprepared_processing();
        let mut completion = engine.source_completion();
        let mut source = engine.take_source().unwrap();
        source.mixer.mute_outputs().unwrap();
        engine.prepare_render(48, 1).unwrap();
        let mut command = engine.take_source_command().unwrap();
        source.install_command(&mut command).unwrap();
        let mut output = [0.; 48 * 4];
        for _ in 0..5 {
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        }
        assert!(source.publish_completion(&mut completion));
        engine.finish_source_completion(&mut completion).unwrap();
        assert!(engine.outputs_quiesced());
        assert_eq!(engine.output_safety_gain(engine.frame()), 0.);
        let revision = engine.topology().map_revision;
        let mut patch = engine
            .prepare_output_patch(engine.topology().outputs.clone())
            .unwrap();
        assert!(engine.acknowledge_output_patch(&patch).is_err());
        crate::remote::assert_no_allocations(|| {
            source.mixer.apply_output_patch(&mut patch).unwrap()
        });
        engine.acknowledge_output_patch(&patch).unwrap();
        assert_eq!(engine.topology().map_revision, revision + 1);
        assert!(engine.acknowledge_output_patch(&patch).is_err());
        assert_eq!(engine.topology(), source.mixer.topology());
    }

    #[test]
    fn detached_operator_and_fx_commands_retain_storage_until_controller_return() {
        let (mut engine, _) = unprepared_processing();
        let mut completion = engine.source_completion();
        let mut source = engine.take_source().unwrap();
        engine.set_operator_tap(crate::brain_control::MonitorSource::Pfl { input: 0 });
        engine.enable_fx_aux().unwrap();
        engine.set_fx_sends(2, &[0.5; 8]).unwrap();
        engine.prepare_render(48, 1).unwrap();
        let mut command = engine.take_source_command().unwrap();
        let mut output = [0.; 48 * 4];
        crate::remote::assert_no_allocations(|| {
            source.install_command(&mut command).unwrap();
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            assert!(source.publish_completion(&mut completion));
        });
        assert!(source.mixer.operator_tap().iter().any(|v| *v != 0.));
        assert_eq!(source.mixer.fx_sends(2).unwrap(), vec![0.5; 8]);
        engine.finish_source_completion(&mut completion).unwrap();
        assert!(engine.observed.fx_aux_enabled);
        drop(command); // allocating payloads are retired only on the controller.
        engine.set_fx_sends(2, &[0.25; 8]).unwrap();
        engine.prepare_render(48, 2).unwrap();
        let mut command = engine.take_source_command().unwrap();
        crate::remote::assert_no_allocations(|| source.install_command(&mut command).unwrap());
        assert_eq!(source.mixer.fx_sends(2).unwrap(), vec![0.25; 8]);
        assert_eq!(engine.fx_sends(2).unwrap(), vec![0.5; 8]);
        crate::remote::assert_no_allocations(|| {
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            assert!(source.publish_completion(&mut completion));
        });
        engine.finish_source_completion(&mut completion).unwrap();
        assert_eq!(engine.fx_sends(2).unwrap(), vec![0.25; 8]);
        engine.quiesce("detached_source_close");
        assert_eq!(engine.output_safety_gain(engine.frame()), 0.);
    }

    #[test]
    fn detached_revocation_before_and_after_processing_commit_uses_actual_receipt() {
        for revoke_before in [true, false] {
            let (mut engine, request) = unprepared_processing();
            let mut packet = engine.source_completion();
            let mut source = engine.take_source().unwrap();
            assert_eq!(
                engine.handle_processing(&request, 1).unwrap().state,
                "pending"
            );
            engine.prepare_render(96, 2).unwrap();
            let mut command = engine.take_source_command().unwrap();
            source.install_command(&mut command).unwrap();
            if revoke_before {
                engine.revoke_writer("desk");
            }
            let mut output = [0.; 96 * 4];
            crate::remote::assert_no_allocations(|| {
                source.render(9, &[0.25; 96 * 8], &mut output).unwrap();
                assert!(source.publish_completion(&mut packet));
            });
            if !revoke_before {
                engine.revoke_writer("desk");
            }
            assert_eq!(engine.revision(), Counter(0));
            engine.finish_source_completion(&mut packet).unwrap();
            let replies = engine.take_processing_completions();
            assert_eq!(replies.len(), 1);
            assert_eq!(
                replies[0].effective_frame,
                if revoke_before {
                    None
                } else {
                    Some(Counter(48))
                }
            );
            assert_eq!(replies[0].reason.is_some(), revoke_before);
            assert_eq!(engine.revision(), Counter(u64::from(!revoke_before)));
            assert_eq!(
                engine.observed.processing[0].1.band2_gain_mdb,
                if revoke_before { 0 } else { 6000 }
            );
            assert!(!engine.authority.writer_live("desk", 3));
            assert!(engine.processing_pending.is_none());
        }
    }

    #[test]
    fn detached_revocation_before_and_after_mixer_commit_preserves_dsp_authority_agreement() {
        for revoke_before in [true, false] {
            let (mut engine, processing) = unprepared_processing();
            let mut request = processing.history_request();
            request.contract = "C-AUDIO".into();
            request.version = engine.wire_version();
            request.command = Command::Set {
                targets: vec![Edit {
                    target: crate::control_model::Target::Fader {
                        input: "input-01".into(),
                    },
                    value: crate::control_model::Value::Integer(0),
                }],
            };
            let before = engine.observed.targets[0][0];
            assert_eq!(before, 10_f64.powf(-6.0 / 20.0));
            let mut packet = engine.source_completion();
            let mut source = engine.take_source().unwrap();
            assert_eq!(engine.handle(&request, 1).unwrap().state, "pending");
            engine.prepare_render(96, 2).unwrap();
            let mut command = engine.take_source_command().unwrap();
            source.install_command(&mut command).unwrap();
            if revoke_before {
                engine.revoke_writer("desk");
            }
            let mut output = [0.; 96 * 4];
            crate::remote::assert_no_allocations(|| {
                source.render(9, &[0.25; 96 * 8], &mut output).unwrap();
                assert!(source.publish_completion(&mut packet));
            });
            if !revoke_before {
                engine.revoke_writer("desk");
            }
            let replies = engine.finish_source_completion(&mut packet).unwrap();
            assert_eq!(replies.len(), 1);
            assert_eq!(
                replies[0].effective_frame,
                if revoke_before {
                    None
                } else {
                    Some(Counter(48))
                }
            );
            assert_eq!(engine.revision(), Counter(u64::from(!revoke_before)));
            assert_eq!(
                engine.observed.targets[0][0],
                if revoke_before { before } else { 1.0 }
            );
            assert!(!engine.authority.writer_live("desk", 3));
            assert!(engine.pending.is_none());
        }
    }

    #[test]
    fn synchronous_external_revision_transfers_only_after_success() {
        for fail in [false, true] {
            let mut engine = OfflineEngine::with_topology(
                "11111111-1111-4111-8111-111111111111",
                Counter(9),
                Counter(0),
                0,
                crate::topology::EngineTopology::software(8, 2, 0).unwrap(),
            )
            .unwrap();
            let mut request = Request {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: "11111111-1111-4111-8111-111111111111".into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some("desk".into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: Command::Grant {
                    scope: Scope::OutputRoutes,
                },
            };
            request.lease = engine
                .handle(&request, 0)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .granted_lease;
            request.request_id = Some(Counter(2));
            request.command = Command::Renew {};
            assert_eq!(
                engine
                    .begin_external(&request, "source-evidence", Scope::OutputRoutes, 1)
                    .unwrap()
                    .0,
                48
            );
            engine
                .process_interleaved(&[0.25; 48 * 8], &mut [0.; 48 * 4], 1)
                .unwrap();
            let reply = engine
                .commit_external(2, || {
                    if fail {
                        Err("refused preparation".into())
                    } else {
                        Ok(())
                    }
                })
                .unwrap();
            assert_eq!(reply.kind == "applied", !fail);
            let source = engine.take_source().unwrap();
            assert_eq!(source.applied_revision(), u64::from(!fail));
            assert_eq!(source.applied_revision(), engine.revision().0);
        }
        let mut engine = OfflineEngine::new(
            "11111111-1111-4111-8111-111111111111",
            Counter(9),
            Counter(17),
            0,
        )
        .unwrap();
        assert_eq!(engine.take_source().unwrap().applied_revision(), 17);
    }

    #[test]
    fn detached_external_exact_boundary_retirement_order_and_revocation() {
        for refusal in 0..3 {
            let cancel = refusal == 1;
            let expired = refusal == 2;
            let refused = cancel || expired;
            let show = "11111111-1111-4111-8111-111111111111";
            let mut engine = OfflineEngine::with_topology(
                show,
                Counter(9),
                Counter(0),
                0,
                crate::topology::EngineTopology::software(8, 2, 0).unwrap(),
            )
            .unwrap();
            let mut request = Request {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: show.into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some("desk".into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: Command::Grant {
                    scope: Scope::OutputRoutes,
                },
            };
            request.lease = engine
                .handle(&request, 0)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .granted_lease;
            request.request_id = Some(Counter(2));
            request.command = Command::Renew {};
            let mut packet = engine.source_completion();
            let mut source = engine.take_source().unwrap();
            assert_eq!(
                engine
                    .begin_external(&request, "detached-test", Scope::OutputRoutes, 1)
                    .unwrap()
                    .0,
                48
            );
            engine.prepare_render(48, 1).unwrap();
            let mut command = engine.take_source_command().unwrap();
            let mut output = [0.; 48 * 4];
            source.install_command(&mut command).unwrap();
            source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            assert!(source.publish_completion(&mut packet));
            engine.finish_source_completion(&mut packet).unwrap();
            let mut permit = engine.prepare_external(2).unwrap();
            if expired {
                permit.expires = Some(std::time::Instant::now());
            }
            if cancel {
                engine.revoke_writer("desk");
            }
            let mut applications = 0;
            let mut receipt = None;
            crate::remote::assert_no_allocations(|| {
                receipt = Some(source.commit_external(&permit, |mixer| {
                    applications += 1;
                    mixer.mute_outputs().unwrap();
                    0
                }));
            });
            let receipt = receipt.unwrap();
            assert_eq!(applications, usize::from(!refused));
            assert_eq!(receipt.code, if refused { -3 } else { 0 });
            assert_eq!(engine.revision(), Counter(0));
            crate::remote::assert_no_allocations(|| {
                assert_eq!(
                    source
                        .commit_external(&permit, |_| panic!("permit replay"))
                        .code,
                    if expired { -3 } else { -4 }
                );
                // Completed/refused external boundary no longer stops dry frames
                // while the controller retains the unapplied authority receipt.
                source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
            });
            let reply = engine.finish_external(receipt).unwrap();
            assert_eq!(reply.kind == "applied", !refused);
            assert_eq!(engine.revision(), Counter(u64::from(!refused)));
            assert!(engine.finish_external(receipt).is_err());
        }
    }

    #[test]
    fn detached_prepared_command_observation_and_terminal_closure() {
        let (mut engine, request) = unprepared_processing();
        let mut packet = engine.source_completion();
        let mut source = engine.take_source().unwrap();
        assert_eq!(
            engine.handle_processing(&request, 1).unwrap().state,
            "pending"
        );
        engine.prepare_render(96, 2).unwrap();
        let mut command = engine.take_source_command().unwrap();
        let mut output = [0.; 96 * 4];
        crate::remote::assert_no_allocations(|| {
            source.install_command(&mut command).unwrap();
            source.render(9, &[0.25; 96 * 8], &mut output).unwrap();
            assert!(source.publish_completion(&mut packet));
        });
        engine.finish_source_completion(&mut packet).unwrap();
        assert_eq!(engine.revision(), Counter(1));
        assert_eq!(engine.frame(), 96);
        assert_eq!(
            engine.take_processing_completions()[0].effective_frame,
            Some(Counter(48))
        );
        // Exporting the completion must not make unchanged rendering depend on
        // another controller command, even while the observer remains stalled.
        crate::remote::assert_no_allocations(|| {
            for _ in 0..8 {
                source.render(9, &[0.25; 96 * 8], &mut output).unwrap();
            }
        });
        engine.quiesce("detached closure");
        crate::remote::assert_no_allocations(|| {
            source.render(9, &[0.25; 96 * 8], &mut output).unwrap();
        });
        assert!(output.iter().all(|v| *v == 0.));
        assert_eq!(engine.frame(), 96); // no invented progress before observation
    }

    #[test]
    fn detached_stale_command_retains_prepared_storage() {
        let (mut engine, request) = unprepared_processing();
        engine.prepare_render(48, 1).unwrap();
        let mut source = engine.take_source().unwrap();
        let mut packet = engine.source_completion();
        let mut output = [0.; 48 * 4];
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        assert!(source.publish_completion(&mut packet));
        engine.finish_source_completion(&mut packet).unwrap();
        assert_eq!(
            engine.handle_processing(&request, 2).unwrap().state,
            "pending"
        );
        engine.prepare_render(96, 3).unwrap();
        let mut command = engine.take_source_command().unwrap();
        // Source progress races the controller's observed frame. Refuse without
        // destroying or silently moving the accepted boundary.
        source.render(9, &[0.25; 48 * 8], &mut output).unwrap();
        crate::remote::assert_no_allocations(|| {
            assert!(source.install_command(&mut command).is_err());
        });
        assert!(command.prepared.is_some());
        assert_eq!(engine.revision(), Counter(0));
    }

    #[test]
    fn source_commit_and_expiry_have_no_allocation_or_retirement() {
        for expired in [false, true] {
            let (mut engine, request) = prepared_processing();
            let input = [0.25; 96 * 8];
            let mut output = [0.; 96 * 4];
            engine
                .prepare_render(96, if expired { 2001 } else { 2 })
                .unwrap();
            crate::remote::assert_no_allocations(|| {
                engine.render_prepared(&input, &mut output).unwrap()
            });
            // Authority advances only after observing the actual source result;
            // the occupied retirement slot cannot be reused during that delay.
            assert_eq!(engine.revision(), Counter(0));
            let unobserved = engine.processing_snapshot().unwrap();
            assert_eq!(unobserved.frame, Counter(0));
            assert_eq!(
                unobserved.channels[0].target,
                crate::channel_processing::Config::default()
            );
            assert!(engine.prepare_render(48, 2002).is_err());
            engine.finish_render().unwrap();
            let replies = engine.take_processing_completions();
            assert_eq!(replies.len(), 1);
            if expired {
                assert!(replies[0].reason.is_some());
                assert_eq!(engine.revision(), Counter(0));
            } else {
                assert_eq!(engine.revision(), Counter(1));
                assert_eq!(replies[0].effective_frame, Some(Counter(48)));
                assert_eq!(replies[0].snapshot.as_ref().unwrap().frame, Counter(96));
                assert_eq!(engine.handle_processing(&request, 3).unwrap(), replies[0]);
            }
        }
    }

    #[test]
    fn owned_mixer_continues_with_unobserved_completion_and_retirement() {
        let (mut engine, _) = prepared_processing();
        engine.prepare_render(96, 2).unwrap();
        let epoch = engine.clock.epoch;
        let (mut reference, _) = prepared_processing();
        let mut expected = Vec::new();
        for _ in 0..32 {
            let mut output = [0.; 96 * 4];
            reference.prepare_render(96, 2).unwrap();
            reference
                .render_prepared(&[0.25; 96 * 8], &mut output)
                .unwrap();
            reference.finish_render().unwrap();
            expected.push(output);
        }
        // Transfer the actual production mixer state, with no borrowed engine.
        let source = engine.take_source().unwrap();
        let source = std::thread::spawn(move || {
            let mut source = source;
            let mut output = [0.; 96 * 4];
            crate::remote::assert_no_allocations(|| {
                for expected in &expected {
                    source.render(epoch, &[0.25; 96 * 8], &mut output).unwrap();
                    assert_eq!(&output, expected);
                }
            });
            assert_eq!(source.mixer.frame(), 32 * 96);
            assert!(source.busy());
            // Unconsumed retirement refuses another mutation while audio advanced.
            let next = Prepared::freeze_for(8, 2, Scope::Foh);
            assert!(source.schedule(next, 99).is_err());
            source
        })
        .join()
        .unwrap();
        engine.restore_source(source).unwrap();
        assert_eq!(engine.revision(), Counter(0));
        engine.finish_render().unwrap();
        let replies = engine.take_processing_completions();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].effective_frame, Some(Counter(48)));
        assert_eq!(
            replies[0].snapshot.as_ref().unwrap().frame,
            Counter(32 * 96)
        );
        assert_eq!(engine.revision(), Counter(1));
    }

    #[test]
    fn unobserved_unchanged_quantum_refuses_new_mixer_admission() {
        let mut engine = OfflineEngine::new(
            "11111111-1111-4111-8111-111111111111",
            Counter(9),
            Counter(0),
            0,
        )
        .unwrap();
        engine.prepare_render(48, 0).unwrap();
        let mut output = [0.; 48 * 4];
        engine
            .render_prepared(&[0.25; 48 * 8], &mut output)
            .unwrap();
        assert!(
            engine
                .source
                .as_mut()
                .unwrap()
                .schedule(Prepared::freeze_for(8, 2, Scope::Foh), 1)
                .is_err()
        );
        crate::remote::assert_no_allocations(|| {
            engine
                .render_prepared(&[0.25; 48 * 8], &mut output)
                .unwrap();
        });
        assert_eq!(engine.frame(), 96);
        engine.finish_render().unwrap();
        assert_eq!(
            engine
                .source
                .as_mut()
                .unwrap()
                .schedule(Prepared::freeze_for(8, 2, Scope::Foh), 1)
                .unwrap(),
            144
        );
    }

    #[test]
    fn owned_mixer_stops_at_unvalidated_boundary_and_rejects_stale_epoch() {
        let (mut engine, _) = prepared_processing();
        engine.prepare_render(48, 2).unwrap();
        let mut output = [0.; 48 * 4];
        engine
            .render_prepared(&[0.25; 48 * 8], &mut output)
            .unwrap();
        crate::remote::assert_no_allocations(|| {
            assert!(
                engine
                    .source
                    .as_mut()
                    .unwrap()
                    .render(engine.clock.epoch + 1, &[0.; 48 * 8], &mut output)
                    .is_err()
            );
            assert!(engine.render_prepared(&[0.; 48 * 8], &mut output).is_err());
        });
        assert_eq!(engine.frame(), 48);
        assert!(output.iter().all(|v| *v == 0.));
        engine.finish_render().unwrap();
        assert!(engine.take_processing_completions().is_empty());
        engine.prepare_render(48, 3).unwrap();
        engine
            .render_prepared(&[0.25; 48 * 8], &mut output)
            .unwrap();
        engine.finish_render().unwrap();
        assert_eq!(
            engine.take_processing_completions()[0].effective_frame,
            Some(Counter(48))
        );
    }

    #[test]
    fn source_fault_before_boundary_refuses_prevalidated_processing() {
        let (mut engine, _) = prepared_processing();
        let mut input = [0.25; 96 * 8];
        input[0] = f64::NAN;
        let mut output = [0.; 96 * 4];
        engine.prepare_render(96, 2).unwrap();
        crate::remote::assert_no_allocations(|| {
            engine.render_prepared(&input, &mut output).unwrap()
        });
        engine.finish_render().unwrap();
        let replies = engine.take_processing_completions();
        assert_eq!(replies[0].reason.as_deref(), Some("faulted"));
        assert_eq!(engine.revision(), Counter(0));
        assert!(output.iter().all(|v| *v == 0.));
    }

    #[test]
    fn invalid_source_shape_does_not_apply_prepared_transaction() {
        let (mut engine, _) = prepared_processing();
        let mut output = [0.; 96 * 4];
        engine.prepare_render(96, 2).unwrap();
        crate::remote::assert_no_allocations(|| {
            assert_eq!(
                engine.render_prepared(&[0.; 8], &mut output),
                Err(crate::mixer::ProcessError::BufferLength)
            );
        });
        assert!(engine.finish_render().is_err());
        assert_eq!(engine.frame(), 0);
        assert_eq!(engine.revision(), Counter(0));
        // The original prepared state remains available for a correctly shaped
        // quantum; a malformed buffer is not an applied or discarded command.
        engine.prepare_render(96, 3).unwrap();
        crate::remote::assert_no_allocations(|| {
            engine
                .render_prepared(&[0.25; 96 * 8], &mut output)
                .unwrap()
        });
        engine.finish_render().unwrap();
        assert_eq!(engine.revision(), Counter(1));
    }
}
