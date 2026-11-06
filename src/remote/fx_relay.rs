//! Worker-side authenticated FX relay. Authorization intent is not owner application.
use super::*;
use crate::{
    control_model::{Request as AudioRequest, Scope},
    fx_wire::*,
};
#[derive(Default)]
pub(super) struct FxRelay {
    owner: Option<AuthenticatedContext>,
    observation: Option<Observation>,
    observed_at: u64,
    reads: BTreeMap<u64, (u64, Observation)>,
    pending: Option<Pending>,
    ticket: u64,
    messages: VecDeque<(u64, Value)>,
    results: VecDeque<(String, Value)>,
    deliveries: VecDeque<Value>,
}
struct Pending {
    request: AudioRequest,
    fingerprint: String,
    mutation: Mutation,
    ticket: u64,
    apply_frame: u64,
    deadline: u64,
    prepared: bool,
    permitted: bool,
    writer_fence: PolicyFence,
}
impl HostAuthority {
    fn fx_binding_current(&self, b: &Binding) -> bool {
        self.descriptor.as_ref().is_some_and(|d| {
            d.session == b.session
                && d.source_epoch == b.source_epoch
                && d.capability_generation == b.capability_generation
                && d.map_generation == b.map_generation
                && d.channels(MediaRole::FxSend) == 2
                && d.channels(MediaRole::WetReturn) == 2
                && [MediaRole::FxSend, MediaRole::WetReturn]
                    .iter()
                    .all(|role| {
                        d.streams
                            .iter()
                            .filter(|s| s.role == *role)
                            .flat_map(|s| s.channel_ids.iter())
                            .map(String::as_str)
                            .eq(["foh-left", "foh-right"])
                    })
        }) && self.media_owner == Some(b.session.0)
    }
    fn fx_reply(&self, p: &Pending, state: &str, reason: Option<&str>) -> Value {
        serde_json::json!({"contract":CONTRACT,"version":1,"state":state,"reason":reason,
            "ticket":Counter(p.ticket),"apply_frame":Counter(p.apply_frame),
            "context":crate::mixer_control::RequestContext::request(&p.request),
            "revision":self.provider.brain_snapshot().revision,"observation":self.fx.observation})
    }
    fn fx_result(&mut self, p: &Pending, state: &str, reason: Option<&str>) {
        let value = self.fx_reply(p, state, reason);
        if self.fx.results.len() == 64 {
            self.fx.results.pop_front();
        }
        if self.fx.deliveries.len() == 64 {
            self.fx.deliveries.pop_front();
        }
        self.fx.deliveries.push_back(value.clone());
        self.fx.results.push_back((p.fingerprint.clone(), value));
    }
    fn fx_send(&mut self, session: u64, command: OwnerCommand) {
        if self.fx.messages.len() == 16 {
            self.fx.messages.pop_front();
        }
        self.fx.messages.push_back((
            session,
            serde_json::json!({"contract":CONTRACT,"version":1,"command":command}),
        ));
    }
    pub(super) fn fx_control_attached(&self, session: u64) -> bool {
        self.fx
            .owner
            .as_ref()
            .is_some_and(|o| o.session() == session)
    }
    pub(super) fn fx_clear_media(&mut self) {
        if let Some(owner) = self.fx.owner.clone() {
            self.fx_disconnect(&owner);
        }
        self.fx.observation = None;
        self.fx.owner = None;
        self.fx.reads.clear();
    }
    pub(super) fn fx_disconnect(&mut self, context: &AuthenticatedContext) {
        self.fx.reads.remove(&context.session());
        if self
            .fx
            .owner
            .as_ref()
            .is_some_and(|o| o.session() == context.session())
        {
            self.fx.owner = None;
            self.fx.observation = None;
            self.fx.messages.retain(|(s, _)| *s != context.session());
        }
        if self.fx.pending.as_ref().is_some_and(|p| {
            p.request.writer.as_deref() == Some(context.writer())
                || p.mutation.binding.session.0 == context.session()
        }) {
            let p = self.fx.pending.take().unwrap();
            self.provider
                .engine_mut()
                .cancel_fx_external(&p.request, &p.fingerprint);
            if !p.permitted {
                self.fx_send(
                    p.mutation.binding.session.0,
                    OwnerCommand::Cancel {
                        ticket: Counter(p.ticket),
                    },
                );
            }
            self.fx_result(
                &p,
                if p.permitted { "unknown" } else { "cancelled" },
                Some("session lost"),
            );
        }
    }
    pub(super) fn tick_fx(&mut self, now: u64) -> Result<()> {
        let Some(p) = self.fx.pending.as_ref() else {
            return Ok(());
        };
        let valid = self.fx_binding_current(&p.mutation.binding)
            && self
                .fx
                .owner
                .as_ref()
                .is_some_and(|o| o.check_current().is_ok());
        let expired = now >= p.deadline || (!p.permitted && self.provider.frame() >= p.apply_frame);
        let reservation = self
            .provider
            .engine_mut()
            .external_matches(&p.request, &p.fingerprint);
        if !valid || expired || (!p.permitted && !reservation) {
            let p = self.fx.pending.take().unwrap();
            self.provider
                .engine_mut()
                .cancel_fx_external(&p.request, &p.fingerprint);
            if !p.permitted {
                self.fx_send(
                    p.mutation.binding.session.0,
                    OwnerCommand::Cancel {
                        ticket: Counter(p.ticket),
                    },
                );
            }
            self.fx_result(
                &p,
                if p.permitted { "unknown" } else { "cancelled" },
                Some("deadline, authority or owner lost"),
            );
            return Ok(());
        }
        if p.prepared && !p.permitted {
            let mut p = self.fx.pending.take().unwrap();
            let reply = if self.source_clock_status.is_some() {
                self.provider.clocked_remote_permission(
                    now,
                    &p.request,
                    &p.fingerprint,
                    p.apply_frame,
                    [
                        p.writer_fence.clone(),
                        self.fx.owner.as_ref().ok_or("FX owner lost")?.audio_fence(),
                    ],
                    None,
                )
            } else {
                let frame = self.provider.frame();
                self.provider.engine_mut().schedule_fx_external(
                    &p.request,
                    &p.fingerprint,
                    frame,
                )?;
                self.provider.engine_mut().commit_external(now, || Ok(()))
            };
            let reply = match reply {
                Ok(reply) => reply,
                Err(error) => {
                    self.provider
                        .engine_mut()
                        .cancel_fx_external(&p.request, &p.fingerprint);
                    self.fx_send(
                        p.mutation.binding.session.0,
                        OwnerCommand::Cancel {
                            ticket: Counter(p.ticket),
                        },
                    );
                    self.fx_result(&p, "cancelled", Some(&error));
                    return Ok(());
                }
            };
            if reply.kind != "applied" {
                self.fx_send(
                    p.mutation.binding.session.0,
                    OwnerCommand::Cancel {
                        ticket: Counter(p.ticket),
                    },
                );
                self.fx_result(&p, "cancelled", reply.body.reason.as_deref());
            } else {
                p.permitted = true;
                self.fx_send(
                    p.mutation.binding.session.0,
                    OwnerCommand::Permit {
                        ticket: Counter(p.ticket),
                        binding: p.mutation.binding.clone(),
                        apply_frame: Counter(p.apply_frame),
                    },
                );
                self.fx_result(&p, "permitted", None);
                self.fx.pending = Some(p);
            }
        }
        Ok(())
    }
    pub(super) fn dispatch_fx(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        now: u64,
    ) -> Result<Value> {
        if self.source_clock_status.is_some() {
            self.provider.service_source_clock(now)?;
        }
        if payload.get("kind").and_then(Value::as_str) == Some("fx_owner") {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Envelope {
                contract: String,
                version: u32,
                kind: String,
                writer: String,
                message: OwnerMessage,
            }
            let e: Envelope = serde_json::from_value(payload).map_err(|e| e.to_string())?;
            if e.contract != CONTRACT
                || e.version != 1
                || e.kind != "fx_owner"
                || e.writer != context.writer()
            {
                return Err("FX owner envelope".into());
            }
            context.require(&Permission::FxConfiguration)?;
            if self.media_owner != Some(context.session()) {
                return Err("FX actual media owner required".into());
            }
            match e.message {
                OwnerMessage::Observe { observation } => {
                    observation.validate()?;
                    if !self.fx_binding_current(&observation.binding)
                        || observation.next_source_frame.0 > self.provider.frame()
                    {
                        return Err("FX owner binding".into());
                    }
                    if self.fx.owner.as_ref().is_some_and(|o| o != context)
                        || self.fx.observation.as_ref().is_some_and(|o| {
                            o.binding != observation.binding
                                || observation.generation.0 < o.generation.0
                                || observation.next_source_frame.0 < o.next_source_frame.0
                        })
                    {
                        return Err("FX owner regression/replacement".into());
                    }
                    self.fx.owner = Some(context.clone());
                    self.fx.observation = Some(observation);
                    self.fx.observed_at = now;
                }
                OwnerMessage::Prepared {
                    ticket,
                    binding,
                    generation,
                    configuration_json,
                    panic_mask,
                } => {
                    let p = self.fx.pending.as_mut().ok_or("unexpected FX prepared")?;
                    if p.permitted
                        || p.ticket != ticket.0
                        || p.mutation.binding != binding
                        || p.mutation.expected_generation != generation
                        || p.mutation.configuration_json != configuration_json
                        || p.mutation.panic_mask != panic_mask
                        || now >= p.deadline
                    {
                        return Err("stale FX prepared".into());
                    }
                    p.prepared = true;
                }
                OwnerMessage::Completed {
                    ticket,
                    effective_source_frame,
                    panic_mask,
                    observation,
                } => {
                    observation.validate()?;
                    let p = self.fx.pending.as_ref().ok_or("unexpected FX completion")?;
                    let target_generation = if p.mutation.panic_mask.is_some() {
                        p.mutation.expected_generation.0
                    } else {
                        p.mutation
                            .expected_generation
                            .0
                            .checked_add(1)
                            .ok_or("FX generation exhausted")?
                    };
                    if !p.permitted
                        || p.ticket != ticket.0
                        || p.mutation.binding != observation.binding
                        || !self.fx_binding_current(&observation.binding)
                        || observation.next_source_frame.0 > self.provider.frame()
                        || observation.generation.0 != target_generation
                        || effective_source_frame.0 != p.apply_frame
                        || panic_mask != p.mutation.panic_mask
                        || observation.reset_count.0
                            != p.mutation
                                .expected_reset_count
                                .0
                                .saturating_add(u64::from(panic_mask.is_some()))
                        || self.fx.observation.as_ref().is_some_and(|o| {
                            observation.next_source_frame.0 < o.next_source_frame.0
                        })
                        || (p.mutation.panic_mask.is_none()
                            && observation.applied_source_frame.0 != p.apply_frame)
                        || p.mutation.configuration_json.as_ref().is_some_and(|c| {
                            Configuration::decode(c).ok()
                                != Configuration::decode(&observation.owner_json).ok()
                        })
                    {
                        return Err("FX applied readback mismatch".into());
                    }
                    self.fx.observation = Some(observation);
                    self.fx.observed_at = now;
                    let p = self.fx.pending.take().unwrap();
                    let settled = self.fx.observation.as_ref().is_some_and(|o| {
                        o.generation == o.settled_generation && o.remaining_frames == [0, 0]
                    });
                    self.fx_result(&p, if settled { "settled" } else { "applied" }, None);
                    if !settled {
                        self.fx.pending = Some(p);
                    }
                }
                OwnerMessage::Refused { ticket, reason } => {
                    if reason.len() > 256 {
                        return Err("FX reason capacity".into());
                    }
                    if self
                        .fx
                        .pending
                        .as_ref()
                        .is_none_or(|p| p.ticket != ticket.0)
                    {
                        return Err("FX refusal ticket".into());
                    }
                    let p = self.fx.pending.take().unwrap();
                    self.provider
                        .engine_mut()
                        .cancel_fx_external(&p.request, &p.fingerprint);
                    self.fx_result(&p, "refused", Some(&reason));
                }
            }
            return Ok(
                serde_json::json!({"contract":CONTRACT,"version":1,"state":"owner_observed"}),
            );
        }
        let (request, mutation) = request(&payload)?;
        if request.show_id != self.provider.show_id()
            || request.epoch.0 != self.provider.source_epoch()
        {
            return Err("FX source identity".into());
        }
        let Some(mutation) = mutation else {
            if let Some(o) = &self.fx.observation {
                self.fx.reads.insert(context.session(), (now, o.clone()));
            }
            return Ok(
                serde_json::json!({"contract":CONTRACT,"version":1,"state":"snapshot","show_id":self.provider.show_id(),"epoch":Counter(self.provider.source_epoch()),"revision":self.provider.brain_snapshot().revision,"frame":Counter(self.provider.frame()),"available":self.fx.owner.as_ref().is_some_and(|o|o.check_current().is_ok()) && self.fx.observation.as_ref().is_some_and(|o|self.fx_binding_current(&o.binding)) && now.saturating_sub(self.fx.observed_at)<=READ_MS,"observation":self.fx.observation,"pending":self.fx.pending.as_ref().map(|p|Counter(p.ticket))}),
            );
        };
        context.require(&Permission::FxConfiguration)?;
        let fingerprint = format!("{:x}", Sha256::digest(super::encode(&payload)?));
        if let Some((_, reply)) = self
            .fx
            .results
            .iter()
            .rev()
            .find(|(f, _)| f == &fingerprint)
        {
            return Ok(reply.clone());
        }
        if let Some(p) = &self.fx.pending {
            return if p.fingerprint == fingerprint {
                Ok(self.fx_reply(
                    p,
                    if p.permitted {
                        "permitted"
                    } else {
                        "preparing"
                    },
                    None,
                ))
            } else {
                Err("FX busy".into())
            };
        }
        if !self.fx_binding_current(&mutation.binding)
            || !self
                .fx
                .owner
                .as_ref()
                .is_some_and(|o| o.check_current().is_ok())
            || now.saturating_sub(self.fx.observed_at) > READ_MS
        {
            return Err("FX owner unavailable".into());
        }
        if !self
            .fx
            .reads
            .get(&context.session())
            .is_some_and(|(at, o)| {
                now >= *at
                    && now - *at <= READ_MS
                    && o.binding == mutation.binding
                    && o.generation == mutation.expected_generation
                    && o.reset_count == mutation.expected_reset_count
            })
            || !self.fx.observation.as_ref().is_some_and(|o| {
                o.binding == mutation.binding
                    && o.generation == mutation.expected_generation
                    && o.reset_count == mutation.expected_reset_count
                    && o.remaining_frames == [0, 0]
            })
        {
            return Err("fresh settled FX snapshot required".into());
        }
        if self.provider.engine_mut().writer_scope(&request, now) != Some(Scope::FxConfiguration) {
            return Err("FX scope lease required".into());
        }
        let apply_frame = if self.source_clock_status.is_some() {
            self.provider.remote_target_frame(mutation.lead_frames.0)?
        } else {
            self.provider
                .frame()
                .checked_add(mutation.lead_frames.0)
                .ok_or("FX frame exhausted")?
        };
        if self
            .fx
            .observation
            .as_ref()
            .is_some_and(|o| o.next_source_frame.0 >= apply_frame)
        {
            return Err("FX target already passed".into());
        }
        if !apply_frame.is_multiple_of(48) {
            return Err("FX source block alignment".into());
        }
        let ticket = self.fx.ticket.checked_add(1).ok_or("FX ticket exhausted")?;
        let deadline = now.checked_add(PREPARE_MS).ok_or("FX deadline exhausted")?;
        let (_, cached) = self.provider.engine_mut().begin_external(
            &request,
            &fingerprint,
            Scope::FxConfiguration,
            now,
        )?;
        if let Some(reply) = cached {
            return Ok(
                serde_json::json!({"contract":CONTRACT,"version":1,"state":"authorization_replayed","result":reply}),
            );
        }
        self.provider
            .engine_mut()
            .schedule_fx_external(&request, &fingerprint, apply_frame)?;
        let p = Pending {
            request,
            fingerprint,
            mutation,
            ticket,
            apply_frame,
            deadline,
            prepared: false,
            permitted: false,
            writer_fence: context.audio_fence(),
        };
        self.fx.ticket = ticket;
        self.fx_send(
            p.mutation.binding.session.0,
            OwnerCommand::Prepare {
                ticket: Counter(ticket),
                mutation: p.mutation.clone(),
                apply_frame: Counter(apply_frame),
            },
        );
        let reply = self.fx_reply(&p, "preparing", None);
        self.fx.pending = Some(p);
        Ok(reply)
    }
    pub(super) fn poll_fx(&mut self, context: &AuthenticatedContext) -> Option<Value> {
        if let Some(i) = self
            .fx
            .messages
            .iter()
            .position(|(s, _)| *s == context.session())
        {
            return self.fx.messages.remove(i).map(|(_, v)| v);
        }
        let i = self
            .fx
            .deliveries
            .iter()
            .position(|v| v["context"]["writer"].as_str() == Some(context.writer()))?;
        self.fx.deliveries.remove(i)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{control_model::Command, topology::EngineTopology};
    use std::os::unix::fs::PermissionsExt;
    const SHOW: &str = "11111111-1111-4111-8111-111111111111";
    struct Harness {
        host: HostAuthority,
        owner: AuthenticatedContext,
        writer: AuthenticatedContext,
        path: std::path::PathBuf,
        lease: Counter,
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    impl Harness {
        fn new(name: &str) -> Self {
            Self::with_clock(name, false)
        }
        fn with_clock(name: &str, clocked: bool) -> Self {
            let path = std::env::temp_dir().join(format!("gp21-{}-{name}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut provider = LocalAudio::bind_configured(
                &path,
                "host",
                SHOW,
                Counter(9),
                EngineTopology::reference_16_18(0, 4).unwrap(),
            )
            .unwrap();
            provider.rearm().unwrap();
            let policy = PolicyStore::new(vec![Peer {
                id: "fx-owner".into(),
                certificate_sha256: fingerprint(b"fx-owner"),
                permissions: [Permission::Fx, Permission::FxConfiguration]
                    .into_iter()
                    .collect(),
            }])
            .unwrap();
            let owner = policy.authenticate(b"fx-owner", 12).unwrap();
            let writer = policy.authenticate(b"fx-owner", 13).unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let ids = vec!["foh-left".into(), "foh-right".into()];
            let mut streams = grouped_streams(MediaRole::FxSend, &ids, 48, 0, 1, 1100).unwrap();
            streams.extend(grouped_streams(MediaRole::WetReturn, &ids, 48, 384, 2, 1100).unwrap());
            let identity = host.identity();
            let descriptor = MediaDescriptor {
                session: Counter(12),
                source_epoch: identity.source_epoch,
                capability_generation: identity.capability_generation,
                map_generation: identity.map_generation,
                sample_rate: 48000,
                streams,
            };
            host.negotiate(&owner, &descriptor).unwrap();
            let grant = AudioRequest {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(writer.writer().into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: Command::Grant {
                    scope: Scope::FxConfiguration,
                },
            };
            // Clocked startup retains the time charged by media preparation.
            // A time-zero lease may already be expired in a shared test process.
            let grant_now = if clocked {
                host.provider.source_control_time()
            } else {
                0
            };
            let lease = host
                .provider
                .engine_mut()
                .handle(&grant, grant_now)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .granted_lease
                .unwrap();
            Self {
                host,
                owner,
                writer,
                path,
                lease,
            }
        }
        fn observation(&self) -> Observation {
            Observation{binding:Binding{session:Counter(12),source_epoch:Counter(9),capability_generation:Counter(1),map_generation:self.host.identity().map_generation,owner_instance:Counter(1),library_sha256:"1".repeat(64),abi_version:2,config_size:104,status_size:168,capabilities_size:128,channels:["foh-left".into(),"foh-right".into()]},generation:Counter(0),settled_generation:Counter(0),applied_source_frame:Counter(0),settled_source_frame:Counter(0),next_source_frame:Counter(0),reset_count:Counter(0),remaining_frames:[0,0],owner_json:r#"{"channels":[{"delay_ms":20.0,"feedback":0.25,"damping":0.35,"wet_gain":0.5,"bypass":false},{"delay_ms":20.0,"feedback":0.25,"damping":0.35,"wet_gain":0.5,"bypass":false}]}"#.into()}
        }
        fn observe(&mut self, o: Observation, now: u64) {
            self.message(OwnerMessage::Observe { observation: o }, now)
                .unwrap();
        }
        fn message(&mut self, m: OwnerMessage, now: u64) -> Result<Value> {
            self.host.dispatch_fx(&self.owner,serde_json::json!({"contract":CONTRACT,"version":1,"kind":"fx_owner","writer":self.owner.writer(),"message":m}),now)
        }
        fn snapshot(&mut self, now: u64) -> Value {
            self.host.dispatch(&self.writer,serde_json::json!({"contract":CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_snapshot","body":{}}),now).unwrap().unwrap()
        }
        fn mutation(&self) -> Value {
            let o = self.host.fx.observation.as_ref().unwrap();
            serde_json::json!({"contract":CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":self.writer.writer(),"lease":self.lease,"request_id":"2","expected_revision":self.host.provider.brain_snapshot().revision,"kind":"fx_configure","body":Mutation{binding:o.binding.clone(),expected_generation:o.generation,expected_reset_count:o.reset_count,lead_frames:Counter(4800),configuration_json:Some(o.owner_json.replace("20.0","1.0")),panic_mask:None}})
        }
        fn block(&mut self, now: u64) {
            let f = self.host.provider.frame();
            let capture = vec![0.; 48 * self.host.provider.topology().capture_channels];
            let mut playback = vec![0.; 48 * self.host.provider.topology().playback_channels];
            self.host
                .process_source(now, 9, f, &capture, &mut playback)
                .unwrap();
        }
    }
    #[test]
    fn clocked_legacy_permission_survives_controller_stall() {
        let mut h = Harness::with_clock("clocked-permission", true);
        let status = h
            .host
            .start_source_clock(0, std::time::Duration::from_millis(2), 240)
            .unwrap();
        let o = h.observation();
        h.observe(o.clone(), 0);
        h.snapshot(0);
        let request = h.mutation();
        let reply = h.host.dispatch(&h.writer, request, 0).unwrap().unwrap();
        let ticket: Counter = serde_json::from_value(reply["ticket"].clone()).unwrap();
        let target = h.host.fx.pending.as_ref().unwrap().apply_frame;
        h.message(
            OwnerMessage::Prepared {
                ticket,
                binding: o.binding.clone(),
                generation: o.generation,
                configuration_json: Some(o.owner_json.replace("20.0", "1.0")),
                panic_mask: None,
            },
            1,
        )
        .unwrap();
        h.host.service_source_clock(1).unwrap();
        assert!(h.host.fx.pending.as_ref().unwrap().permitted);
        assert!(h.host.provider.frame() < target);
        assert!(
            h.host
                .fx
                .messages
                .iter()
                .any(|(_, v)| v["command"]["kind"] == "permit")
                || h.host
                    .fx
                    .messages
                    .iter()
                    .any(
                        |(_, v)| serde_json::from_value::<OwnerCommand>(v["command"].clone())
                            .is_ok_and(|c| matches!(c, OwnerCommand::Permit { .. }))
                    )
        );
        let before = status.blocks();
        let limit = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while status.blocks() * 48 <= target + 48 {
            assert!(std::time::Instant::now() < limit);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(status.blocks() > before);
        h.host.service_source_clock(300).unwrap();
        assert!(h.host.fx.pending.as_ref().unwrap().permitted);
        h.host.finish_source_clock(&status).unwrap();
    }
    #[test]
    fn gp21_relay_preparation_does_not_stall_dry_and_cancels_unpermitted_target() {
        let mut h = Harness::new("cancel");
        let o = h.observation();
        h.observe(o, 0);
        h.snapshot(0);
        let request = h.mutation();
        let reply = h
            .host
            .dispatch(&h.writer, request.clone(), 0)
            .unwrap()
            .unwrap();
        assert_eq!(reply["state"], "preparing");
        for i in 0..=100 {
            h.block(i);
        }
        assert_eq!(h.host.provider.frame(), 4848);
        assert!(h.host.fx.pending.is_none());
        assert!(h.host.provider.engine_mut().external_boundary().is_none());
        let replay = h.host.dispatch(&h.writer, request, 101).unwrap().unwrap();
        assert_eq!(replay["state"], "cancelled");
        h.observe(h.observation(), 102);
        h.snapshot(102);
        let mut reused = h.mutation();
        reused["body"]["lead_frames"] = serde_json::json!("4848");
        let r = h.host.dispatch(&h.writer, reused, 102).unwrap().unwrap();
        assert_eq!(r["state"], "authorization_replayed");
        assert_eq!(r["result"]["body"]["reason"], "reused_id");
        h.host.fx.results.clear();
        let r = h
            .host
            .dispatch(&h.writer, h.mutation(), 102)
            .unwrap()
            .unwrap();
        assert_eq!(r["result"]["kind"], "rejected");
    }
    #[test]
    fn gp21_relay_exact_binding_auth_and_lost_ack_unknown() {
        let mut h = Harness::new("unknown");
        let o = h.observation();
        h.observe(o.clone(), 0);
        let request = h.mutation();
        assert!(h.host.dispatch(&h.writer, request.clone(), 0).is_err());
        h.snapshot(0);
        let mut forged = request.clone();
        forged["writer"] = serde_json::json!(h.owner.writer());
        assert!(h.host.dispatch(&h.writer, forged, 0).is_err());
        let mut stale = request.clone();
        stale["body"]["binding"]["map_generation"] = serde_json::json!("999");
        assert!(h.host.dispatch(&h.writer, stale, 0).is_err());
        let reply = h
            .host
            .dispatch(&h.writer, request.clone(), 0)
            .unwrap()
            .unwrap();
        let ticket: Counter = serde_json::from_value(reply["ticket"].clone()).unwrap();
        let d = h.host.descriptor().unwrap().clone();
        assert!(h.host.negotiate(&h.owner, &d).is_err());
        h.message(
            OwnerMessage::Prepared {
                ticket,
                binding: o.binding.clone(),
                generation: Counter(0),
                configuration_json: Some(o.owner_json.replace("20.0", "1.0")),
                panic_mask: None,
            },
            1,
        )
        .unwrap();
        h.block(1);
        assert!(h.host.fx.pending.as_ref().unwrap().permitted);
        assert!(h.host.negotiate(&h.owner, &d).is_err());
        h.host.tick_fx(2000).unwrap();
        assert!(h.host.fx.pending.is_none());
        assert_eq!(
            h.host.dispatch(&h.writer, request, 2001).unwrap().unwrap()["state"],
            "unknown"
        );
        assert!(
            h.message(
                OwnerMessage::Prepared {
                    ticket,
                    binding: o.binding,
                    generation: Counter(0),
                    configuration_json: Some(o.owner_json.replace("20.0", "1.0")),
                    panic_mask: None,
                },
                2001
            )
            .is_err()
        );
    }
    #[test]
    fn gp21_relay_declines_wrong_scope_and_wrong_owner() {
        let mut h = Harness::new("scope");
        let o = h.observation();
        h.observe(o.clone(), 0);
        h.snapshot(0);
        let payload = serde_json::json!({"contract":CONTRACT,"version":1,"kind":"fx_owner","writer":h.writer.writer(),"message":OwnerMessage::Observe{observation:o}});
        assert!(h.host.dispatch(&h.writer, payload, 0).is_err());
        let certificate = b"media-only";
        let policy = PolicyStore::new(vec![Peer {
            id: "media-only".into(),
            certificate_sha256: fingerprint(certificate),
            permissions: [Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        let ctx = policy.authenticate(certificate, 14).unwrap();
        let mut request = h.mutation();
        request["writer"] = serde_json::json!(ctx.writer());
        assert!(h.host.dispatch(&ctx, request, 0).is_err());
    }
    #[test]
    #[ignore = "requires explicitly pinned actual SHR FX v2 library; optional corpus output"]
    fn gp21_actual_owner_relay_corpus() {
        let mut h = Harness::new("actual-corpus");
        let library = std::path::PathBuf::from(std::env::var_os("GP21_FX_LIBRARY").unwrap());
        let hash = format!("{:x}", Sha256::digest(std::fs::read(&library).unwrap()));
        let mut owner = crate::host::fx_v2::Owner::load(&library, 48)
            .unwrap()
            .unwrap();
        let mut observation = h.observation();
        observation.binding.library_sha256 = hash;
        let status = owner.status().unwrap();
        observation.owner_json = serde_json::to_string(&status.configuration()).unwrap();
        h.observe(observation.clone(), 0);
        let snapshot = h.snapshot(0);
        let request = h.mutation();
        let preparing = h
            .host
            .dispatch(&h.writer, request.clone(), 0)
            .unwrap()
            .unwrap();
        let ticket: Counter = serde_json::from_value(preparing["ticket"].clone()).unwrap();
        let apply_frame: Counter =
            serde_json::from_value(preparing["apply_frame"].clone()).unwrap();
        let (_, mutation) = crate::fx_wire::request(&request).unwrap();
        let mutation = mutation.unwrap();
        owner
            .prepare(
                &Configuration::decode(mutation.configuration_json.as_ref().unwrap()).unwrap(),
                0,
            )
            .unwrap();
        h.message(
            OwnerMessage::Prepared {
                ticket,
                binding: observation.binding.clone(),
                generation: Counter(0),
                configuration_json: mutation.configuration_json.clone(),
                panic_mask: None,
            },
            1,
        )
        .unwrap();
        h.block(1);
        let permitted = h.host.fx.results.back().unwrap().1.clone();
        assert_eq!(permitted["state"], "permitted");
        let input = [0.1; 96];
        let mut output = [0.; 96];
        for frame in (0..apply_frame.0).step_by(48) {
            assert_eq!(owner.process(&input, &mut output, frame), 0);
            if h.host.provider.frame() <= frame {
                h.block(1);
            }
        }
        assert_eq!(owner.commit(apply_frame.0), 0);
        owner.retire();
        let make_observation = |status: crate::host::fx_v2::Status| Observation {
            binding: observation.binding.clone(),
            generation: Counter(status.applied_generation),
            settled_generation: Counter(status.settled_generation),
            applied_source_frame: Counter(status.applied_source_frame),
            settled_source_frame: Counter(status.settled_source_frame),
            next_source_frame: Counter(status.next_source_frame),
            reset_count: Counter(status.reset_count),
            remaining_frames: status.remaining_frames,
            owner_json: serde_json::to_string(&status.configuration()).unwrap(),
        };
        let applied_observation = make_observation(owner.status().unwrap());
        h.message(
            OwnerMessage::Completed {
                ticket,
                effective_source_frame: apply_frame,
                panic_mask: None,
                observation: applied_observation,
            },
            2,
        )
        .unwrap();
        let applied = h.host.fx.results.back().unwrap().1.clone();
        assert_eq!(applied["state"], "applied");
        for frame in (apply_frame.0..apply_frame.0 + 960).step_by(48) {
            assert_eq!(owner.process(&input, &mut output, frame), 0);
            h.block(2);
        }
        let settled_observation = make_observation(owner.status().unwrap());
        h.message(
            OwnerMessage::Completed {
                ticket,
                effective_source_frame: apply_frame,
                panic_mask: None,
                observation: settled_observation,
            },
            3,
        )
        .unwrap();
        let settled = h.host.fx.results.back().unwrap().1.clone();
        assert_eq!(settled["state"], "settled");
        let final_snapshot = h.snapshot(3);
        assert_eq!(
            final_snapshot["observation"]["settled_source_frame"],
            "5760"
        );
        let corpus = serde_json::json!({"snapshot":snapshot,"request":request,"preparing":preparing,"permitted":permitted,"applied":applied,"settled":settled,"final_snapshot":final_snapshot});
        if let Some(path) = std::env::var_os("GP21_FIXTURE_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&corpus).unwrap()).unwrap();
        }
    }
}
