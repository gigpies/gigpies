//! Measurement control glue. All owner work runs on the startup worker, never render.
use super::*;
use crate::{
    measurement_capture::{Action, State},
    measurement_wire::{self as wire, Basis, Command as M, Reply, Snapshot},
};
pub(super) struct Pending {
    pub request: wire::Request,
    pub action: Action,
    pub basis: Option<Basis>,
    pub owner: Option<u64>,
}
impl LocalAudio {
    /// Explicit offline-only integration; physical runners do not expose this option.
    pub fn enable_software_measurement(
        &mut self,
        config: crate::measurement_owner::Config,
    ) -> Result<()> {
        if self.frame() != 0
            || self.render_graph.is_some()
            || self.source_executor.is_some()
            || self.measurement.is_some()
            || self.topology().is_legacy()
        {
            return Err("measurement startup requires configured software source".into());
        }
        if self.topology().measurement_slots.is_empty() {
            return Err("measurement requires explicit reserved capture slot".into());
        }
        let mut state = State::new(config)?;
        self.source.measurement = Some(state.detach_capture()?);
        self.measurement = Some(state);
        Ok(())
    }
    pub fn measurement_current_basis(&self) -> Result<Basis> {
        if self.measurement.is_none() {
            return Err("measurement unavailable: explicit software startup required".into());
        }
        #[cfg(feature = "hardware-host")]
        {
            let g = self.modules.as_ref().ok_or("PA modules unavailable")?;
            let s = g
                .pa_v2_status()
                .ok_or("PA v2 unavailable")?
                .map_err(|_| "PA status unavailable")?;
            if s.version != 2
                || s.size != 80
                || s.epoch != self.epoch.0
                || s.generation == 0
                || s.committed != 1
                || s.fault_latched != 0
            {
                return Err("PA owner identity or fault".into());
            }
            let configuration = g
                .pa_configuration_json()
                .ok_or("PA configuration unavailable")?;
            if configuration.len() > 48 * 1024 {
                return Err("PA configuration capacity".into());
            }
            Ok(Basis {
                source_epoch: self.epoch,
                map_revision: Counter(self.topology().map_revision),
                graph_generation: Counter(s.generation),
                clock_domain: "software-common-source".into(),
                configuration_json: configuration.into(),
                program_buses: g.pa_program_buses().to_vec(),
            })
        }
        #[cfg(not(feature = "hardware-host"))]
        {
            Err("PA owner unavailable in this build".into())
        }
    }
    fn measurement_basis_valid(&self, b: &Basis) -> bool {
        if b.source_epoch != self.epoch
            || b.map_revision.0 != self.topology().map_revision
            || !matches!(
                self.engine.clock_status().state,
                crate::clock_domain::ClockState::Running
                    | crate::clock_domain::ClockState::Disarmed
            )
        {
            return false;
        }
        #[cfg(feature = "hardware-host")]
        {
            self.modules.as_ref().is_some_and(|g| {
                g.pa_configuration_json() == Some(b.configuration_json.as_str())
                    && g.pa_program_buses() == b.program_buses
                    && g.pa_v2_status().is_some_and(|s| {
                        s.is_ok_and(|s| {
                            s.epoch == b.source_epoch.0
                                && s.generation == b.graph_generation.0
                                && s.committed == 1
                                && s.fault_latched == 0
                        })
                    })
            })
        }
        #[cfg(not(feature = "hardware-host"))]
        {
            false
        }
    }
    /// Explicit software witness may select which already-generated virtual mic path to inject.
    pub fn measurement_requested_output(&self) -> Option<usize> {
        self.measurement_pending
            .as_ref()
            .and_then(|p| {
                if let M::CaptureStart { output_index, .. } = p.request.command {
                    Some(output_index)
                } else {
                    None
                }
            })
            .or_else(|| {
                self.measurement
                    .as_ref()
                    .and_then(State::progress)
                    .map(|p| p.output_index)
            })
    }
    pub fn measurement_snapshot(&self) -> Snapshot {
        let basis = self.measurement_current_basis();
        Snapshot {
            available: basis.is_ok(),
            software_only: true,
            reason: basis.as_ref().err().cloned(),
            capture_progress: self.measurement.as_ref().and_then(State::progress),
            active_id: self
                .measurement
                .as_ref()
                .and_then(|s| s.active_id().map(str::to_owned)),
            results: self.measurement.as_ref().map_or_else(Vec::new, |s| {
                s.records.iter().map(|r| r.summary.clone()).collect()
            }),
            current_basis: basis.ok(),
            reserved_mic_slots: self.topology().measurement_slots.clone(),
            reference_inputs: self
                .topology()
                .inputs
                .iter()
                .map(|p| p.id.clone())
                .collect(),
        }
    }
    pub fn measurement_request(
        &mut self,
        r: wire::Request,
        now: u64,
        fresh: bool,
        owner: Option<u64>,
    ) -> Result<Reply> {
        r.validate()?;
        if r.show_id != self.show || r.epoch != self.epoch {
            return Err("measurement attachment identity".into());
        }
        if let M::MeasurementSnapshot {} = r.command {
            let mut p = Reply::new(&r, "snapshot", None, self.engine.revision());
            p.snapshot = Some(self.measurement_snapshot());
            return Ok(p);
        }
        if let M::MeasurementResult { id } = &r.command {
            let mut p = Reply::new(&r, "result", None, self.engine.revision());
            p.result = self
                .measurement
                .as_ref()
                .and_then(|s| s.records.iter().find(|p| p.summary.id == *id).cloned());
            if p.result.is_none() {
                p.reason = Some("measurement result missing".into())
            }
            return Ok(p);
        }
        if self.engine.external_boundary().is_none() {
            self.measurement_pending = None;
        }
        let (frame, cached) = match self.engine.begin_external(
            &r.authority_request(),
            &r.fingerprint()?,
            crate::control_model::Scope::PaConfiguration,
            now,
        ) {
            Ok(v) => v,
            Err(e) => return Ok(Reply::new(&r, "final", Some(e), self.engine.revision())),
        };
        if let Some(cached) = cached {
            return Ok(self
                .measurement_cache
                .iter()
                .find(|(old, _)| old == &r)
                .map(|(_, p)| p.clone())
                .unwrap_or_else(|| {
                    Reply::new(
                        &r,
                        "final",
                        Some(cached.body.reason.unwrap_or("expired_outcome".into())),
                        cached.body.revision,
                    )
                }));
        }
        if let Some(p) = &self.measurement_pending {
            if p.request != r {
                return Err("measurement pending ownership".into());
            }
            let mut reply = Reply::new(&r, "pending", None, self.engine.revision());
            reply.effective_frame = Some(Counter(frame));
            return Ok(reply);
        }
        let basis = self.measurement_current_basis();
        let prepared = (|| {
            if !fresh {
                return Err("fresh_measurement_snapshot_required".into());
            }
            let b = basis.as_ref().map_err(Clone::clone)?;
            let state = self.measurement.as_ref().ok_or("measurement unavailable")?;
            state.authorize_records(&r.command, &r.authority_request())?;
            state.prepare(&r.command, b, self.topology(), frame, now)
        })();
        self.measurement_pending = Some(Pending {
            request: r.clone(),
            action: prepared.unwrap_or_else(Action::Failed),
            basis: basis.ok(),
            owner,
        });
        let mut reply = Reply::new(&r, "pending", None, self.engine.revision());
        reply.effective_frame = Some(Counter(frame));
        Ok(reply)
    }
    pub(super) fn commit_measurement(&mut self, now: u64) -> Result<()> {
        if self.engine.external_boundary() != Some(self.frame())
            || self.measurement_pending.is_none()
        {
            return Ok(());
        }
        let p = self
            .measurement_pending
            .take()
            .ok_or("measurement pending")?;
        let r = p.request;
        if !self
            .engine
            .external_matches(&r.authority_request(), &r.fingerprint()?)
        {
            return Err("measurement pending identity".into());
        }
        let valid = p
            .basis
            .as_ref()
            .is_some_and(|b| self.measurement_basis_valid(b));
        let mut failure = None;
        let frame = self.frame();
        let state = &mut self.measurement;
        let result = self.engine.commit_external(now, || {
            let result = if let Action::Failed(e) = p.action {
                Err(e)
            } else if !valid {
                Err("measurement basis changed".into())
            } else {
                state.as_mut().ok_or("measurement unavailable")?.apply(
                    p.action,
                    r.authority_request(),
                    p.basis.ok_or("measurement basis missing")?,
                )
            };
            if let Err(e) = &result {
                failure = Some(e.clone())
            }
            result
        })?;
        let mut reply = Reply::new(
            &r,
            "final",
            failure.or(result.body.reason),
            result.body.revision,
        );
        if result.kind == "applied" {
            reply.effective_frame = Some(Counter(frame));
        }
        self.measurement_cache.push_back((r, reply.clone()));
        if self.measurement_cache.len() > 256 {
            self.measurement_cache.pop_front();
        }
        if let Some(id) = p.owner {
            if let Some(c) = self.clients.iter_mut().find(|c| c.id == id) {
                c.queue_module(&reply, now)?;
            }
        } else if self.remote_completions.len() < 64 {
            self.remote_completions
                .push_back(serde_json::to_value(reply).map_err(|e| e.to_string())?);
        }
        Ok(())
    }
    pub(super) fn invalidate_pending_measurement(&mut self, reason: &str) {
        if let Some(pending) = self.measurement_pending.take()
            && let Some(state) = &mut self.measurement
            && let Some(basis) = pending.basis
        {
            state.invalidate_prepared(
                pending.action,
                pending.request.authority_request(),
                basis,
                reason,
            );
        }
    }
    pub(super) fn validate_measurement(&mut self, now: u64) {
        let invalid: Vec<_> = self.measurement.as_ref().map_or_else(Vec::new, |s| {
            s.guards()
                .filter_map(|(id, auth, b)| {
                    if !self.measurement_basis_valid(b) {
                        Some((id.to_owned(), "measurement basis changed"))
                    } else if self.engine.writer_scope(auth, now)
                        != Some(crate::control_model::Scope::PaConfiguration)
                    {
                        Some((id.to_owned(), "measurement authority expired"))
                    } else {
                        None
                    }
                })
                .collect()
        });
        if let Some(s) = &mut self.measurement {
            for (id, reason) in invalid {
                s.invalidate(&id, reason)
            }
        }
    }
    pub(super) fn poll_measurement(&mut self, now: u64) {
        self.validate_measurement(now);
        if let Some(s) = &mut self.measurement {
            s.poll(now);
        }
    }
}
