//! Owned mixer execution state. No protocol, authority or controller lifetime is
//! borrowed by rendering. ModuleGraph and routing are not yet part of this owner.
use crate::mixer::{Mixer, Prepared, ProcessError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Independent terminal closure and cancellation do not require controller
/// admission or free observation capacity. Fresh owners get fresh signals.
#[derive(Clone, Debug)]
pub(crate) struct SourceClosure {
    closed: Arc<AtomicBool>,
    cancelled: Arc<AtomicU64>,
}
impl SourceClosure {
    fn new() -> Self {
        Self {
            closed: Arc::new(AtomicBool::new(false)),
            cancelled: Arc::new(AtomicU64::new(0)),
        }
    }
    pub(super) fn identity(&self) -> usize {
        Arc::as_ptr(&self.closed) as usize
    }
    pub(super) fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.closed, &other.closed)
    }
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
    pub(super) fn reopen(&self) {
        self.closed.store(false, Ordering::Release);
    }
    pub(super) fn cancel(&self, ticket: u64) {
        self.cancelled.store(ticket, Ordering::Release);
    }
}

/// Source-issued permission to prepare for one exact future source boundary.
/// This is not a render fence: an absent or late command never stops the source.
#[derive(Clone, Debug)]
pub(crate) struct SourceAdmission {
    pub(super) owner: SourceClosure,
    pub(super) epoch: u64,
    pub(super) ticket: u64,
    pub(super) frame: u64,
}
impl SourceAdmission {
    pub fn frame(&self) -> u64 {
        self.frame
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn ticket(&self) -> u64 {
        self.ticket
    }
}

/// Reusable controller-allocated completion storage. The source only swaps
/// numeric observations and retired ownership; destruction belongs to controller.
#[derive(Debug)]
pub(crate) struct SourceCompletion {
    pub(super) owner: SourceClosure,
    pub(super) epoch: u64,
    pub(super) receipt: Option<RenderReceipt>,
    pub(super) observation: crate::mixer::SourceObservation,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RenderPlan {
    pub frame: u64,
    pub revision: u64,
    pub frames: usize,
    pub commit: Option<(u64, u64, bool, bool)>,
    // Never cross an external or not-yet-validated transaction boundary.
    pub fence: Option<u64>,
}
/// Controller-owned validation capability. DSP checks its independent closure
/// and exact timeline before applying any already prepared external operation.
#[derive(Debug)]
pub(crate) struct ExternalPermit {
    pub(super) revision: u64,
    owner: SourceClosure,
    epoch: u64,
    frame: u64,
    ticket: u64,
    permitted: bool,
    pub(super) admission: Option<SourceAdmission>,
    pub(crate) expires: Option<std::time::Instant>,
    #[cfg(feature = "hardware-host")]
    pub(crate) authority_fence: Option<crate::remote::PolicyFence>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExternalReceipt {
    pub(super) owner_id: usize,
    pub(super) epoch: u64,
    pub(super) frame: u64,
    pub(super) ticket: u64,
    pub code: i32,
    pub(super) output: Option<crate::mixer::OutputObservation>,
}
impl ExternalPermit {
    pub(super) fn new(
        owner: SourceClosure,
        epoch: u64,
        frame: u64,
        ticket: u64,
        permitted: bool,
    ) -> Self {
        Self {
            revision: 0,
            owner,
            epoch,
            frame,
            ticket,
            permitted,
            admission: None,
            expires: None,
            #[cfg(feature = "hardware-host")]
            authority_fence: None,
        }
    }
    pub(super) fn permitted(&self) -> bool {
        self.permitted
    }
    pub(crate) fn receipt(&self, code: i32) -> ExternalReceipt {
        ExternalReceipt {
            owner_id: self.owner.identity(),
            epoch: self.epoch,
            frame: self.frame,
            ticket: self.ticket,
            code,
            output: None,
        }
    }
}

/// One globally ordered prepared mixer operation and its validated numeric plan.
/// On refusal this packet keeps its prepared storage; its holder returns it to
/// the controller rather than destroying it on the source.
#[derive(Debug)]
pub(crate) struct SourceCommand {
    pub(super) owner: SourceClosure,
    pub(super) epoch: u64,
    pub(super) prepared: Option<(Prepared, u64, u64)>,
    pub(super) plan: RenderPlan,
    pub(super) operator_tap: Option<crate::brain_control::MonitorSource>,
    pub(super) fx_sends: crate::mixer::PreparedFxSends,
    pub(super) admission: Option<SourceAdmission>,
    pub(super) installed: bool,
    pub(crate) expires: Option<std::time::Instant>,
    #[cfg(feature = "hardware-host")]
    pub(crate) authority_fence: Option<crate::remote::PolicyFence>,
}

#[derive(Debug)]
pub(super) struct RenderReceipt {
    pub fault_before_commit: bool,
    pub cancelled_before_commit: bool,
    pub result: Result<(), ProcessError>,
    pub retirement: crate::mixer::SourceRetirement,
}
#[derive(Debug)]
pub(crate) struct SourceState {
    pub(super) applied_revision: u64,
    delayed_revision: u64,
    #[cfg(feature = "hardware-host")]
    pub(crate) remote_fx: Option<RemoteFxSends>,
    #[cfg(feature = "hardware-host")]
    pub(crate) remote_fx_applied: Option<(u64, u64)>,
    pub mixer: Mixer,
    epoch: u64,
    closure: SourceClosure,
    running: bool,
    last_external_ticket: u64,
    admission_ticket: u64,
    admission_frame: u64,
    delayed_commit: Option<(u64, u64, bool, bool)>,
    expires: Option<std::time::Instant>,
    #[cfg(feature = "hardware-host")]
    authority_fence: Option<crate::remote::PolicyFence>,
    plan: Option<RenderPlan>,
    receipt: Option<RenderReceipt>,
    fence: Option<u64>,
    pub observation: crate::mixer::SourceObservation,
}
impl SourceState {
    pub fn new(epoch: u64, mixer: Mixer) -> Self {
        Self {
            applied_revision: 0,
            delayed_revision: 0,
            #[cfg(feature = "hardware-host")]
            remote_fx: None,
            #[cfg(feature = "hardware-host")]
            remote_fx_applied: None,
            observation: crate::mixer::SourceObservation::new(&mixer),
            mixer,
            epoch,
            closure: SourceClosure::new(),
            running: false,
            last_external_ticket: 0,
            admission_ticket: 0,
            admission_frame: 0,
            delayed_commit: None,
            expires: None,
            #[cfg(feature = "hardware-host")]
            authority_fence: None,
            plan: None,
            receipt: None,
            fence: None,
        }
    }
    /// Issue a bounded future preparation window without freezing source time.
    /// A newer admission invalidates an older uninstalled packet. The executor
    /// reserves completion/retirement capacity before requesting this capability.
    pub fn reserve_admission(&mut self, frame: u64) -> Result<SourceAdmission, ProcessError> {
        let current = self.mixer.frame();
        if self.closure.closed()
            || !frame.is_multiple_of(48)
            || frame.checked_sub(current).is_none_or(|lead| lead > 48_000)
        {
            return Err(ProcessError::Faulted);
        }
        self.admission_ticket = self
            .admission_ticket
            .checked_add(1)
            .ok_or(ProcessError::ClockExhausted)?;
        self.admission_frame = frame;
        Ok(SourceAdmission {
            owner: self.closure.clone(),
            epoch: self.epoch,
            ticket: self.admission_ticket,
            frame,
        })
    }
    fn accepts_admission(&self, admission: &SourceAdmission) -> bool {
        self.closure.same_owner(&admission.owner)
            && admission.epoch == self.epoch
            && admission.ticket == self.admission_ticket
            && admission.frame == self.admission_frame
            && admission.frame == self.mixer.frame()
            && !self.closure.closed()
    }
    /// Run only an already prepared bounded numeric operation at its reserved
    /// boundary. The caller retains the action for controller-side retirement.
    pub fn commit_external(
        &mut self,
        permit: &ExternalPermit,
        apply: impl FnOnce(&mut Mixer) -> i32,
    ) -> ExternalReceipt {
        let identity = self.closure.same_owner(&permit.owner)
            && permit.epoch == self.epoch
            && permit
                .admission
                .as_ref()
                .is_none_or(|a| self.accepts_admission(a));
        let busy = self.busy();
        let expired = permit
            .expires
            .is_some_and(|d| std::time::Instant::now() >= d);
        #[cfg(feature = "hardware-host")]
        let expired = expired || permit.authority_fence.as_ref().is_some_and(|f| !f.valid());
        let code = if expired {
            -3
        } else if !identity {
            -2
        } else if permit.ticket <= self.last_external_ticket {
            -4
        } else {
            self.last_external_ticket = permit.ticket;
            if busy {
                -5
            } else if !permit.permitted {
                -1
            } else if permit.frame != self.mixer.frame() {
                -2
            } else if self.closure.closed.load(Ordering::Acquire)
                || self.closure.cancelled.load(Ordering::Acquire) == permit.ticket
            {
                -3
            } else {
                apply(&mut self.mixer)
            }
        };
        if identity
            && !busy
            && permit.frame == self.mixer.frame()
            && code != -4
            && self.fence == Some(permit.frame)
        {
            self.fence = None;
        }
        if code == 0 {
            self.applied_revision = permit.revision;
        }
        let mut receipt = permit.receipt(code);
        if identity {
            receipt.output = Some(self.mixer.output_observation());
        }
        receipt
    }
    pub fn install_command(&mut self, command: &mut SourceCommand) -> Result<(), ProcessError> {
        if self.busy()
            || command.installed
            || !self.closure.same_owner(&command.owner)
            || command.epoch != self.epoch
            || command.plan.frame != self.mixer.frame()
            || command
                .admission
                .as_ref()
                .is_some_and(|a| !self.accepts_admission(a))
            || !self.mixer.accepts_fx_sends(&command.fx_sends)
            || self.closure.closed.load(Ordering::Acquire)
        {
            return Err(ProcessError::Faulted);
        }
        if let Some((_, _, boundary)) = &command.prepared {
            let next = self
                .mixer
                .frame()
                .checked_div(48)
                .and_then(|n| n.checked_add(1))
                .and_then(|n| n.checked_mul(48));
            if next != Some(*boundary) {
                return Err(ProcessError::Faulted);
            }
        }
        if let Some((prepared, ticket, boundary)) = command.prepared.take()
            && let Err((_, prepared)) = self.mixer.schedule_retained(prepared, ticket)
        {
            command.prepared = Some((prepared, ticket, boundary));
            return Err(ProcessError::Faulted);
        }
        if !self.mixer.apply_fx_sends(&mut command.fx_sends) {
            return Err(ProcessError::Faulted);
        }
        if let Some(tap) = command.operator_tap.take() {
            self.mixer.set_operator_tap(tap);
        }
        #[cfg(feature = "hardware-host")]
        std::mem::swap(&mut self.authority_fence, &mut command.authority_fence);
        self.expires = command.expires;
        self.admit(command.plan);
        command.installed = true;
        Ok(())
    }
    /// Complete a retained command after terminal closure without requiring
    /// another capture block. Prepared/native retirement still returns in receipt.
    #[cfg(feature = "hardware-host")]
    pub fn finish_closed_command(&mut self) {
        if self.receipt.is_some() {
            return;
        }
        self.mixer.quiesce();
        self.plan = None;
        self.delayed_commit = None;
        self.expires = None;
        self.mixer.observe_into(&mut self.observation);
        self.receipt = Some(RenderReceipt {
            fault_before_commit: true,
            cancelled_before_commit: true,
            result: Ok(()),
            retirement: self.mixer.take_source_retirement(),
        });
    }
    pub fn closure(&self) -> SourceClosure {
        self.closure.clone()
    }
    #[cfg(any(feature = "hardware-host", test))]
    pub(crate) fn applied_revision(&self) -> u64 {
        self.applied_revision
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn publish_completion(&mut self, packet: &mut SourceCompletion) -> bool {
        if packet.receipt.is_some()
            || self.receipt.is_none()
            || packet.epoch != self.epoch
            || !self.closure.same_owner(&packet.owner)
        {
            return false;
        }
        packet.receipt = self.receipt.take();
        std::mem::swap(&mut packet.observation, &mut self.observation);
        true
    }
    #[cfg(feature = "hardware-host")]
    pub fn observe_latest(&self, packet: &mut SourceCompletion) -> bool {
        if packet.receipt.is_some()
            || packet.epoch != self.epoch
            || !self.closure.same_owner(&packet.owner)
            || self.busy()
        {
            return false;
        }
        self.mixer.observe_into(&mut packet.observation);
        true
    }
    pub fn busy(&self) -> bool {
        self.plan.is_some() || self.receipt.is_some() || self.delayed_commit.is_some()
    }
    /// Controller admission must not mutate DSP behind an unobserved plan.
    pub fn schedule(&mut self, prepared: Prepared, ticket: u64) -> crate::show::Result<u64> {
        if self.busy() {
            return Err("capacity".into());
        }
        self.mixer.schedule(prepared, ticket)
    }
    pub(super) fn admit(&mut self, plan: RenderPlan) {
        assert!(!self.busy());
        self.fence = plan.fence;
        self.plan = Some(plan);
    }
    pub(super) fn take_receipt(&mut self) -> Option<RenderReceipt> {
        self.receipt.take()
    }
    /// Subsequent unchanged quanta need no controller preparation or completion
    /// poll. An occupied receipt/retirement slot blocks admission, not rendering.
    /// The controller must still validate any future pending mutation at its fence.
    pub fn render(
        &mut self,
        epoch: u64,
        input: &[f64],
        output: &mut [f64],
    ) -> Result<(), ProcessError> {
        #[cfg(feature = "hardware-host")]
        self.advance_remote_fx();
        if self.closure.closed.load(Ordering::Acquire) {
            self.mixer.quiesce();
        }
        if self.plan.is_none() && !self.running {
            output.fill(0.);
            return Err(ProcessError::Faulted);
        }
        let ni = self.mixer.input_count();
        let no = self.mixer.monitor_count() + 2;
        let frame = self.mixer.frame();
        let frames = input.len() / ni;
        let plan = self.plan.take();
        let revision = plan.map_or(self.delayed_revision, |p| p.revision);
        let mut commit = plan.and_then(|p| p.commit).or(self.delayed_commit.take());
        // Admitted preparation may validate the next edge before the first
        // quantum. Retain that numeric decision until its actual boundary;
        // ordinary 48-frame rendering never waits for the controller.
        if commit.is_some_and(|c| c.1 == frame.saturating_add(frames as u64)) {
            self.delayed_commit = commit.take();
            self.delayed_revision = revision;
        }
        let mut fault_before_commit = false;
        let mut cancelled_before_commit = false;
        let result = (|| {
            if epoch != self.epoch
                || !input.len().is_multiple_of(ni)
                || output.len() != frames * no
                || plan.is_some_and(|p| p.frame != frame || p.frames != frames)
                || frame
                    .checked_add(frames as u64)
                    .is_none_or(|end| self.fence.is_some_and(|f| end > f))
            {
                return Err(ProcessError::BufferLength);
            }
            if let Some((ticket, boundary, permitted, reject_fault)) = commit {
                let Some(split) = boundary
                    .checked_sub(frame)
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n < frames)
                else {
                    return Err(ProcessError::BufferLength);
                };
                self.mixer
                    .process_interleaved(&input[..split * ni], &mut output[..split * no])?;
                fault_before_commit = reject_fault && self.mixer.faulted();
                cancelled_before_commit = self.closure.cancelled.load(Ordering::Acquire) == ticket
                    || self
                        .expires
                        .is_some_and(|deadline| std::time::Instant::now() >= deadline);
                #[cfg(feature = "hardware-host")]
                {
                    cancelled_before_commit |=
                        self.authority_fence.as_ref().is_some_and(|f| !f.valid());
                }
                self.expires = None;
                if (!permitted || fault_before_commit || cancelled_before_commit)
                    && !self.mixer.cancel_for_render(ticket)
                {
                    return Err(ProcessError::Faulted);
                }
                self.mixer
                    .process_interleaved(&input[split * ni..], &mut output[split * no..])
            } else {
                self.mixer.process_interleaved(input, output)
            }
        })();
        if result.is_ok()
            && commit.is_some_and(|c| c.2)
            && !fault_before_commit
            && !cancelled_before_commit
        {
            self.applied_revision = revision;
        }
        // Never overwrite an unobserved mutation outcome during ordinary rendering.
        self.running = result.is_ok();
        self.mixer.observe_into(&mut self.observation);
        if self.receipt.is_none()
            && (plan.is_some() || commit.is_some() || result.is_err())
            && (self.delayed_commit.is_none() || result.is_err())
        {
            self.receipt = Some(RenderReceipt {
                fault_before_commit,
                cancelled_before_commit,
                result,
                retirement: self.mixer.take_source_retirement(),
            });
        }
        if result.is_err() {
            output.fill(0.);
        }
        result
    }
}

/// One retained future source send mutation. Its allocated storage is never
/// dropped by rendering, including cancellation and expired preparation.
#[cfg(feature = "hardware-host")]
#[derive(Debug)]
pub(crate) struct RemoteFxSends {
    pub ticket: u64,
    pub frame: u64,
    pub sends: crate::mixer::PreparedFxSends,
    pub fences: [crate::remote::PolicyFence; 2],
    pub expires: std::time::Instant,
    pub cancelled: Arc<AtomicBool>,
    pub result: Option<bool>,
}
#[cfg(feature = "hardware-host")]
impl SourceState {
    fn advance_remote_fx(&mut self) {
        let Some(p) = &mut self.remote_fx else { return };
        if p.result.is_some() {
            return;
        }
        let frame = self.mixer.frame();
        if self.closure.closed()
            || p.cancelled.load(Ordering::Acquire)
            || p.fences.iter().any(|f| !f.valid())
            || std::time::Instant::now() >= p.expires
            || frame > p.frame
        {
            p.result = Some(false);
        } else if frame == p.frame {
            let applied = self.mixer.apply_fx_sends(&mut p.sends);
            p.result = Some(applied);
            if applied {
                self.remote_fx_applied = Some((p.ticket, frame));
            }
        }
    }
}

#[cfg(all(test, feature = "hardware-host"))]
mod remote_send_tests {
    use super::*;
    use crate::{
        mixer_control::OfflineEngine,
        remote::{Peer, Permission, PolicyStore},
        show::Counter,
    };
    use sha2::{Digest, Sha256};
    #[test]
    fn clocked_remote_sends_refuse_cancel_expiry_and_closed_sessions_without_retirement() {
        for mode in 0..5 {
            let mut engine = OfflineEngine::new(
                "11111111-1111-4111-8111-111111111111",
                Counter(9),
                Counter(0),
                0,
            )
            .unwrap();
            engine.enable_fx_aux().unwrap();
            let mut source = engine.take_source().unwrap();
            let policy = PolicyStore::new(vec![Peer {
                id: "remote-send".into(),
                certificate_sha256: crate::hash::hex(Sha256::digest(b"remote-send")),
                permissions: [Permission::FxConfiguration].into_iter().collect(),
            }])
            .unwrap();
            let writer = policy.authenticate(b"remote-send", 1).unwrap();
            let owner = policy.authenticate(b"remote-send", 2).unwrap();
            let cancelled = Arc::new(AtomicBool::new(mode == 1));
            let mut sends = crate::mixer::PreparedFxSends::default();
            sends.set(8, 0, &[0.75; 8]).unwrap();
            sends
                .set_returns(
                    2,
                    0,
                    &crate::fx_aux_wire::Returns {
                        main: 0.,
                        monitors: vec![0.25, 0.75],
                    },
                )
                .unwrap();
            source.remote_fx = Some(RemoteFxSends {
                ticket: 7,
                frame: 0,
                sends,
                fences: [writer.audio_fence(), owner.audio_fence()],
                expires: std::time::Instant::now()
                    + std::time::Duration::from_secs(if mode == 2 { 0 } else { 10 }),
                cancelled,
                result: None,
            });
            if mode == 3 {
                writer.close_audio();
            }
            if mode == 4 {
                owner.close_audio();
            }
            crate::remote::assert_no_allocations(|| {
                source.advance_remote_fx();
                for _ in 0..16 {
                    source.advance_remote_fx();
                }
            });
            assert_eq!(source.remote_fx.as_ref().unwrap().result, Some(mode == 0));
            assert_eq!(
                source.mixer.fx_sends(0).unwrap(),
                vec![if mode == 0 { 0.75 } else { 0. }; 8]
            );
            assert_eq!(
                source.mixer.fx_returns(0).unwrap(),
                if mode == 0 {
                    crate::fx_aux_wire::Returns {
                        main: 0.,
                        monitors: vec![0.25, 0.75],
                    }
                } else {
                    crate::fx_aux_wire::Returns::silent_monitors(2)
                }
            );
            // The slot and allocating prepared payload survive both outcomes
            // until controller code takes them; repeated ticks cannot reapply.
            let retired = source.remote_fx.take().unwrap();
            assert_eq!(retired.ticket, 7);
        }
    }
}
