//! Authenticated four-aux control relay; authority permits are not DSP completion.
use super::*;
use crate::{
    control_model::{Request, Scope},
    fx_aux_wire as wire,
    remote::fx_aux_owner as owner,
};
use serde_json::json;
#[derive(Default)]
pub(super) struct AuxRelay {
    owner: Option<AuthenticatedContext>,
    observation: Option<owner::Observation>,
    observed_at: u64,
    reads: BTreeMap<u64, u64>,
    pending: Option<Pending>,
    ticket: u64,
    messages: VecDeque<(u64, Value)>,
    deliveries: VecDeque<(u64, Value)>,
}
struct Pending {
    request: Request,
    writer_session: u64,
    fingerprint: String,
    mutation: wire::Mutation,
    ticket: u64,
    frame: u64,
    deadline: u64,
    prepared: bool,
    permitted: bool,
    source_applied: bool,
    brain_applied: bool,
}
impl HostAuthority {
    pub(super) fn aux_mode(&self) -> bool {
        self.descriptor.as_ref().is_some_and(|d| {
            d.channels(MediaRole::FxSend) == 8 && d.channels(MediaRole::WetReturn) == 8
        })
    }
    fn aux_current(&self, o: &owner::Observation) -> bool {
        self.aux_mode()
            && self.descriptor.as_ref() == Some(&o.descriptor)
            && self.media_owner == Some(o.descriptor.session.0)
    }
    fn aux_reply(r: &Request, state: &str, reason: Option<String>, frame: Option<u64>) -> Value {
        json!({"contract":wire::CONTRACT,"version":1,"show_id":r.show_id,"module":"audio","epoch":r.epoch,"writer":r.writer,"lease":r.lease,"request_id":r.request_id,"state":state,"reason":reason,"effective_frame":frame.map(|f|f.to_string())})
    }
    fn aux_message(&mut self, session: u64, c: owner::Command) {
        if self.aux.messages.len() == 16 {
            self.aux.messages.pop_front();
        }
        self.aux.messages.push_back((
            session,
            json!({"contract":owner::CONTRACT,"version":1,"command":c}),
        ));
    }
    fn aux_finish(&mut self, p: Pending, state: &str, reason: Option<String>) {
        let v = Self::aux_reply(
            &p.request,
            state,
            reason,
            (state == "accepted").then_some(p.frame),
        );
        if self.aux.deliveries.len() == 64 {
            self.aux.deliveries.pop_front();
        }
        self.aux.deliveries.push_back((p.writer_session, v));
    }
    fn aux_snapshot(&self, r: &Request, now: u64) -> Result<Value> {
        let mut v = Self::aux_reply(r, "snapshot", None, None);
        v["revision"] = json!(self.provider.brain_snapshot().revision);
        v["available"] = json!(false);
        if let Some(o) = &self.aux.observation
            && self.aux_current(o)
            && self
                .aux
                .owner
                .as_ref()
                .is_some_and(|c| c.check_current().is_ok())
            && now.saturating_sub(self.aux.observed_at) <= 1000
        {
            v["available"] = json!(true);
            v["pending"] = json!(self.aux.pending.as_ref().map(|p| p.mutation.bus));
            v["library_sha256"] = json!(o.library_sha256);
            v["owner_binding"] = json!(owner::binding(&o.descriptor, &o.library_sha256)?);
            v["map_revision"] = json!(self.provider.topology().map_revision.to_string());
            v["frame"] = json!(self.provider.frame().to_string());
            v["inputs"] = serde_json::to_value(&self.provider.topology().inputs)
                .map_err(|e| e.to_string())?;
            v["buses"] = json!(o.buses);
            v["palette_json"] = json!(o.palette_json);
            v["capabilities_json"] = json!(o.capabilities_json);
            for bus in 0..4 {
                let mut config: Value = serde_json::from_str(&o.buses[bus].configuration_json)
                    .map_err(|e| e.to_string())?;
                config["sends"] = json!(
                    self.provider
                        .engine()
                        .mixer()
                        .fx_sends(bus)
                        .ok_or("FX aux source sends unavailable")?
                );
                v["buses"][bus]["configuration_json"] =
                    json!(serde_json::to_string(&config).map_err(|e| e.to_string())?);
            }
            // A remote DSP can settle before the source-side send ramp has completed.
            if let Some(p) = &self.aux.pending
                && self.provider.frame() < p.frame.saturating_add(240)
            {
                v["buses"][p.mutation.bus]["applied_generation"] =
                    json!(p.mutation.expected_generation);
            }
        }
        Ok(v)
    }
    fn validate_aux_observation(&self, o: &owner::Observation) -> Result<()> {
        if !self.aux_current(o)
            || o.next_frame.0 > self.provider.frame()
            || o.buses.len() != 4
            || o.library_sha256.len() != 64
            || !o
                .library_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || o.palette_json.len() > 32768
            || o.capabilities_json.len() > 4096
        {
            return Err("FX aux owner observation identity/bounds".into());
        }
        for (i, b) in o.buses.iter().enumerate() {
            if b.id != format!("fx-{}", i + 1)
                || b.applied_generation.0 > b.generation.0
                || b.configuration_json.len() > 32768
                || b.owner_json.len() > 4096
            {
                return Err("FX aux owner bus identity".into());
            }
            let c: owner::Configuration =
                serde_json::from_str(&b.configuration_json).map_err(|e| e.to_string())?;
            if c.sends.len() != self.provider.topology().inputs.len()
                || c.sends
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.).contains(v))
            {
                return Err("FX aux owner sends".into());
            }
        }
        if let Some(old) = &self.aux.observation
            && (old.descriptor != o.descriptor
                || old.library_sha256 != o.library_sha256
                || old.next_frame.0 > o.next_frame.0
                || old
                    .buses
                    .iter()
                    .zip(&o.buses)
                    .any(|(a, b)| a.generation.0 > b.generation.0))
        {
            return Err("FX aux owner replacement/regression".into());
        }
        Ok(())
    }
    pub(super) fn dispatch_aux_owner(
        &mut self,
        context: &AuthenticatedContext,
        value: Value,
        now: u64,
    ) -> Result<Value> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Envelope {
            contract: String,
            version: u32,
            kind: String,
            writer: String,
            message: owner::Message,
        }
        let e: Envelope = serde_json::from_value(value).map_err(|e| e.to_string())?;
        context.require(&Permission::FxConfiguration)?;
        if e.contract != owner::CONTRACT
            || e.version != 1
            || e.kind != "fx_aux_owner"
            || e.writer != context.writer()
            || self.media_owner != Some(context.session())
            || !self.aux_mode()
        {
            return Err("FX aux actual media owner required".into());
        }
        let reply_ticket = match &e.message {
            owner::Message::Prepared { ticket, .. }
            | owner::Message::Completed { ticket, .. }
            | owner::Message::Refused { ticket, .. } => Some(ticket.0),
            owner::Message::Observe { .. } => None,
        };
        // Ordered replies may arrive after a writer disconnect or cancellation.
        // Ignore only already-issued, no-longer-current tickets; never revive them.
        if reply_ticket.is_some_and(|t| {
            t <= self.aux.ticket && self.aux.pending.as_ref().is_none_or(|p| p.ticket != t)
        }) {
            return Ok(json!({"contract":owner::CONTRACT,"version":1,"state":"owner_observed"}));
        }
        match e.message {
            owner::Message::Observe { observation } => {
                self.validate_aux_observation(&observation)?;
                self.aux.owner = Some(context.clone());
                self.aux.observation = Some(observation);
                self.aux.observed_at = now;
            }
            owner::Message::Prepared {
                ticket,
                mutation,
                apply_frame,
            } => {
                let p = self
                    .aux
                    .pending
                    .as_mut()
                    .ok_or("FX aux unexpected prepared")?;
                if p.permitted
                    || p.ticket != ticket.0
                    || p.frame != apply_frame.0
                    || serde_json::to_value(&p.mutation).unwrap()
                        != serde_json::to_value(&mutation).unwrap()
                    || now >= p.deadline
                {
                    return Err("FX aux prepared mismatch".into());
                }
                p.prepared = true;
                let response = Self::aux_reply(&p.request, "prepared", None, None);
                let session = p.writer_session;
                if self.aux.deliveries.len() == 64 {
                    self.aux.deliveries.pop_front();
                }
                self.aux.deliveries.push_back((session, response));
            }
            owner::Message::Completed {
                ticket,
                apply_frame,
                observation,
            } => {
                self.validate_aux_observation(&observation)?;
                let p = self
                    .aux
                    .pending
                    .as_ref()
                    .ok_or("FX aux unexpected completion")?;
                let b = &observation.buses[p.mutation.bus];
                if !p.permitted
                    || !p.source_applied
                    || p.ticket != ticket.0
                    || p.frame != apply_frame.0
                    || b.generation.0 != p.mutation.expected_generation.0 + 1
                    || observation.next_frame.0 <= p.frame
                {
                    return Err("FX aux applied boundary mismatch".into());
                }
                let actual: owner::Configuration =
                    serde_json::from_str(&b.configuration_json).map_err(|e| e.to_string())?;
                if let Some(c) = &p.mutation.configuration_json {
                    let target: Value = serde_json::from_str(c).map_err(|e| e.to_string())?;
                    if serde_json::to_value(&actual).map_err(|e| e.to_string())? != target {
                        return Err("FX aux applied target mismatch".into());
                    }
                }
                if !p.mutation.panic
                    && self
                        .provider
                        .engine_mut()
                        .mixer()
                        .fx_sends(p.mutation.bus)
                        .as_ref()
                        != Some(&actual.sends)
                {
                    return Err("FX aux source send readback mismatch".into());
                }
                let settled = b.generation == b.applied_generation;
                self.aux.observation = Some(observation);
                self.aux.observed_at = now;
                self.aux.pending.as_mut().unwrap().brain_applied = settled;
            }
            owner::Message::Refused { ticket, reason } => {
                let p = self.aux.pending.take().ok_or("FX aux unexpected refusal")?;
                if p.ticket != ticket.0 || reason.len() > 256 {
                    self.aux.pending = Some(p);
                    return Err("FX aux refusal identity".into());
                }
                self.provider
                    .engine_mut()
                    .cancel_fx_external(&p.request, &p.fingerprint);
                let state = if p.permitted { "unknown" } else { "rejected" };
                self.aux_finish(p, state, Some(reason));
            }
        }
        Ok(json!({"contract":owner::CONTRACT,"version":1,"state":"owner_observed"}))
    }
    pub(super) fn dispatch_aux(
        &mut self,
        context: &AuthenticatedContext,
        value: Value,
        now: u64,
    ) -> Result<Value> {
        let (r, m) = wire::request(&value, self.provider.engine_mut().wire_version())?;
        if r.show_id != self.provider.show_id() || r.epoch.0 != self.provider.source_epoch() {
            return Err("FX aux attachment".into());
        }
        let Some(m) = m else {
            self.aux.reads.insert(context.session(), now);
            return self.aux_snapshot(&r, now);
        };
        context.require(&Permission::FxConfiguration)?;
        if r.writer.as_deref() != Some(context.writer()) {
            return Err("FX aux authenticated writer mismatch".into());
        }
        let result = (|| -> Result<()> {
            if self.aux.pending.is_some()
                || self
                    .aux
                    .reads
                    .get(&context.session())
                    .is_none_or(|t| now.saturating_sub(*t) > 1000)
            {
                return Err("FX aux fresh read required/busy".into());
            }
            let s = self.aux_snapshot(&r, now)?;
            if s["available"] != true {
                return Err("FX aux Brain owner unavailable".into());
            }
            let o = self.aux.observation.as_ref().unwrap();
            let b = &o.buses[m.bus];
            if m.owner_binding.as_deref()
                != Some(owner::binding(&o.descriptor, &o.library_sha256)?.as_str())
                || m.library_sha256 != o.library_sha256
                || m.map_revision != o.descriptor.map_generation
                || m.expected_generation.0 == u64::MAX
                || m.expected_generation != b.generation
                || (!m.panic && b.generation != b.applied_generation)
            {
                return Err("FX aux stale/busy basis".into());
            }
            if let Some(c) = &m.configuration_json {
                let c: owner::Configuration = serde_json::from_str(c).map_err(|e| e.to_string())?;
                if c.sends.len() != self.provider.topology().inputs.len()
                    || c.sends
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..=1.).contains(v))
                {
                    return Err("FX aux sends".into());
                }
            }
            let descriptor = o.descriptor.clone();
            let frame = self
                .provider
                .frame()
                .checked_add(4800)
                .ok_or("FX aux frame exhausted")?;
            let ticket = self
                .aux
                .ticket
                .checked_add(1)
                .ok_or("FX aux ticket exhausted")?;
            let fingerprint = format!("{:x}", Sha256::digest(super::encode(&value)?));
            let (_, cached) = self.provider.engine_mut().begin_external(
                &r,
                &fingerprint,
                Scope::FxConfiguration,
                now,
            )?;
            if cached.is_some() {
                return Err("FX aux request replay; refresh".into());
            }
            self.provider
                .engine_mut()
                .schedule_fx_external(&r, &fingerprint, frame)?;
            self.aux_message(
                descriptor.session.0,
                owner::Command::Prepare {
                    ticket: Counter(ticket),
                    descriptor,
                    mutation: m.clone(),
                    apply_frame: Counter(frame),
                },
            );
            self.aux.ticket = ticket;
            self.aux.pending = Some(Pending {
                request: r.clone(),
                writer_session: context.session(),
                fingerprint,
                mutation: m,
                ticket,
                frame,
                deadline: now.saturating_add(1500),
                prepared: false,
                permitted: false,
                source_applied: false,
                brain_applied: false,
            });
            Ok(())
        })();
        Ok(match result {
            Ok(()) => Self::aux_reply(&r, "preparing", None, None),
            Err(e) => Self::aux_reply(&r, "rejected", Some(e), None),
        })
    }
    pub(super) fn tick_aux(&mut self, now: u64) -> Result<()> {
        let Some(mut p) = self.aux.pending.take() else {
            return Ok(());
        };
        let frame = self.provider.frame();
        let valid = self
            .aux
            .observation
            .as_ref()
            .is_some_and(|o| self.aux_current(o))
            && self
                .aux
                .owner
                .as_ref()
                .is_some_and(|o| o.check_current().is_ok());
        if !valid
            || (!p.permitted
                && !self
                    .provider
                    .engine_mut()
                    .external_matches(&p.request, &p.fingerprint))
            || now >= p.deadline
            || (!p.permitted && frame >= p.frame)
            || (!p.source_applied && frame > p.frame)
        {
            self.provider
                .engine_mut()
                .cancel_fx_external(&p.request, &p.fingerprint);
            if !p.permitted
                && let Some(d) = self.descriptor.as_ref()
            {
                self.aux_message(
                    d.session.0,
                    owner::Command::Cancel {
                        ticket: Counter(p.ticket),
                    },
                );
            }
            let state = if p.permitted { "unknown" } else { "rejected" };
            self.aux_finish(
                p,
                state,
                Some("FX aux owner/deadline/source boundary lost".into()),
            );
            return Ok(());
        }
        if p.prepared && !p.permitted {
            self.provider
                .engine_mut()
                .schedule_fx_external(&p.request, &p.fingerprint, frame)?;
            let reply = self.provider.engine_mut().commit_external(now, || Ok(()))?;
            if reply.kind != "applied" {
                if let Some(d) = self.descriptor.as_ref() {
                    self.aux_message(
                        d.session.0,
                        owner::Command::Cancel {
                            ticket: Counter(p.ticket),
                        },
                    );
                }
                self.aux_finish(p, "rejected", reply.body.reason);
                return Ok(());
            }
            p.permitted = true;
            let d = self.descriptor.clone().ok_or("FX aux descriptor lost")?;
            self.aux_message(
                d.session.0,
                owner::Command::Permit {
                    ticket: Counter(p.ticket),
                    descriptor: d,
                    apply_frame: Counter(p.frame),
                },
            );
        }
        if p.permitted && !p.source_applied && frame == p.frame {
            if let Some(c) = &p.mutation.configuration_json {
                let c: owner::Configuration = serde_json::from_str(c).map_err(|e| e.to_string())?;
                self.provider
                    .engine_mut()
                    .set_fx_sends(p.mutation.bus, &c.sends)?;
            }
            p.source_applied = true;
        }
        if p.brain_applied && frame >= p.frame.saturating_add(240) {
            self.aux_finish(p, "accepted", None);
        } else {
            self.aux.pending = Some(p);
        }
        Ok(())
    }
    pub(super) fn disconnect_aux(&mut self, c: &AuthenticatedContext) {
        self.aux.reads.remove(&c.session());
        if self.aux.pending.as_ref().is_some_and(|p| {
            p.writer_session == c.session()
                || self
                    .aux
                    .owner
                    .as_ref()
                    .is_some_and(|o| o.session() == c.session())
        }) {
            let p = self.aux.pending.take().unwrap();
            self.provider
                .engine_mut()
                .cancel_fx_external(&p.request, &p.fingerprint);
            if !p.permitted
                && let Some(d) = self.descriptor.as_ref()
            {
                self.aux_message(
                    d.session.0,
                    owner::Command::Cancel {
                        ticket: Counter(p.ticket),
                    },
                );
            }
            let state = if p.permitted { "unknown" } else { "rejected" };
            self.aux_finish(p, state, Some("FX aux connection lost".into()));
        }
        if self
            .aux
            .owner
            .as_ref()
            .is_some_and(|o| o.session() == c.session())
        {
            self.aux.owner = None;
            self.aux.observation = None;
            self.aux.messages.clear();
        }
    }
    pub(super) fn clear_aux(&mut self) {
        if let Some(c) = self.aux.owner.clone() {
            self.disconnect_aux(&c);
        }
        self.aux.observation = None;
        self.aux.reads.clear();
    }
    pub(super) fn poll_aux(&mut self, c: &AuthenticatedContext) -> Option<Value> {
        if let Some(i) = self
            .aux
            .messages
            .iter()
            .position(|(s, _)| *s == c.session())
        {
            return self.aux.messages.remove(i).map(|(_, v)| v);
        }
        let i = self
            .aux
            .deliveries
            .iter()
            .position(|(s, _)| *s == c.session())?;
        self.aux.deliveries.remove(i).map(|(_, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{control_model::Command as AudioCommand, topology::EngineTopology};
    use std::os::unix::fs::PermissionsExt;
    const SHOW: &str = "11111111-1111-4111-8111-111111111111";
    struct Harness {
        host: HostAuthority,
        brain: owner::BrainAux,
        receiver: GroupedReceiver,
        descriptor: MediaDescriptor,
        owner: AuthenticatedContext,
        writer: AuthenticatedContext,
        lease: Counter,
        now: u64,
        path: std::path::PathBuf,
        energy: [f64; 4],
        last: [f64; 192],
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    impl Harness {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("fx-aux-remote-{}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut provider = LocalAudio::bind_configured(
                &path,
                "host",
                SHOW,
                Counter(9),
                EngineTopology::software(4, 2, 0).unwrap(),
            )
            .unwrap();
            provider.rearm().unwrap();
            let policy = PolicyStore::new(vec![Peer {
                id: "aux-owner".into(),
                certificate_sha256: fingerprint(b"aux-owner"),
                permissions: [Permission::Fx, Permission::FxConfiguration]
                    .into_iter()
                    .collect(),
            }])
            .unwrap();
            let owner = policy.authenticate(b"aux-owner", 12).unwrap();
            let writer = policy.authenticate(b"aux-owner", 13).unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let ids = owner::channel_ids();
            let mut streams = grouped_streams(MediaRole::FxSend, &ids, 48, 0, 1, 1100).unwrap();
            streams.extend(
                grouped_streams(
                    MediaRole::WetReturn,
                    &ids,
                    48,
                    384,
                    streams.len() as u32 + 1,
                    1100,
                )
                .unwrap(),
            );
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
            let library = std::path::PathBuf::from(std::env::var_os("GP_FX_LIBRARY").unwrap());
            let hash = format!("{:x}", Sha256::digest(std::fs::read(&library).unwrap()));
            let brain = owner::BrainAux::load(descriptor.clone(), &library, &hash, 4).unwrap();
            let receiver = GroupedReceiver::new(descriptor.clone(), MediaRole::FxSend).unwrap();
            let request = Request {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(writer.writer().into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: AudioCommand::Grant {
                    scope: Scope::FxConfiguration,
                },
            };
            let lease = host
                .provider
                .engine_mut()
                .handle(&request, 0)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .granted_lease
                .unwrap();
            Self {
                host,
                brain,
                receiver,
                descriptor,
                owner,
                writer,
                lease,
                now: 0,
                path,
                energy: [0.; 4],
                last: [0.; 192],
            }
        }
        fn message(&mut self, m: owner::Message) {
            self.host.dispatch_aux_owner(&self.owner,json!({"contract":owner::CONTRACT,"version":1,"kind":"fx_aux_owner","writer":self.owner.writer(),"message":m}),self.now).unwrap();
        }
        fn block(&mut self) {
            while let Some(v) = self.host.poll_aux(&self.owner) {
                let c = serde_json::from_value(v["command"].clone()).unwrap();
                if let Some(m) = self.brain.command(c).unwrap() {
                    self.message(m);
                }
            }
            let frame = self.host.provider.frame();
            let capture: [f64; 192] =
                std::array::from_fn(|i| ((i % 4 + 1) * 128) as f64 / 8388608.);
            self.host
                .process_source(self.now, 9, frame, &capture, &mut self.last)
                .unwrap();
            self.now += 1;
            while let Some(bytes) = self.host.outgoing.pop_front() {
                let packet = Packet::parse(&bytes).unwrap();
                if let Some((frame, input)) = self.receiver.push(packet).unwrap() {
                    let mut wet = [0.; 384];
                    self.brain.process(frame, &input, &mut wet).unwrap();
                    for f in 0..48 {
                        for b in 0..4 {
                            self.energy[b] +=
                                wet[f * 8 + b * 2].powi(2) + wet[f * 8 + b * 2 + 1].powi(2);
                        }
                    }
                    for bytes in
                        encode_grouped(&self.descriptor, MediaRole::WetReturn, frame, &wet).unwrap()
                    {
                        self.host
                            .media(&self.owner, Packet::parse(&bytes).unwrap(), self.now)
                            .unwrap();
                    }
                }
            }
            if let Some(m) = self.brain.poll().unwrap() {
                self.message(m);
            }
        }
        fn snapshot(&mut self) -> Value {
            self.host.dispatch_aux(&self.writer,json!({"contract":wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_aux_snapshot","body":{}}),self.now).unwrap()
        }
        fn edit(&mut self, bus: usize, id: u64) -> Value {
            let s = self.snapshot();
            let mut c: owner::Configuration =
                serde_json::from_str(s["buses"][bus]["configuration_json"].as_str().unwrap())
                    .unwrap();
            c.settings.time_ms = 2.;
            c.settings.amount = 0.;
            c.settings.damping = 0.;
            c.settings.gain = 1.;
            c.sends[bus] = 1.;
            json!({"contract":wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":self.writer.writer(),"lease":self.lease,"request_id":id.to_string(),"expected_revision":s["revision"],"kind":"fx_aux_configure","body":{"bus":bus,"map_revision":s["map_revision"],"library_sha256":s["library_sha256"],"owner_binding":s["owner_binding"],"expected_generation":s["buses"][bus]["generation"],"configuration_json":serde_json::to_string(&c).unwrap(),"panic":false}})
        }
    }
    #[test]
    #[ignore = "GP_FX_LIBRARY actual palette, source/Brain grouped transport and wet sample acceptance"]
    fn actual_four_aux_source_brain_roundtrip_and_loss() {
        let mut h = Harness::new();
        for _ in 0..10 {
            h.block();
        }
        let monitors = [h.last[2], h.last[3]];
        for bus in 0..4 {
            let r = h.edit(bus, bus as u64 + 2);
            let reply = h.host.dispatch_aux(&h.writer, r, h.now).unwrap();
            assert_eq!(reply["state"], "preparing");
            let mut accepted = false;
            for _ in 0..135 {
                h.block();
                while let Some(v) = h.host.poll_aux(&h.writer) {
                    assert!(
                        ["prepared", "accepted"].contains(&v["state"].as_str().unwrap_or("")),
                        "{v}"
                    );
                    accepted |= v["state"] == "accepted";
                }
            }
            assert!(accepted);
        }
        for _ in 0..20 {
            h.block();
        }
        assert!(h.energy.iter().all(|e| *e > 0.), "{:?}", h.energy);
        let dry = h.host.provider.brain_fx_send().to_vec();
        for f in 0..48 {
            for ch in 0..2 {
                assert!(
                    (h.last[f * 4 + ch] - 2. * dry[f * 2 + ch]).abs() < 1e-9,
                    "wet sum frame{f} ch{ch}: {} vs {}",
                    h.last[f * 4 + ch],
                    2. * dry[f * 2 + ch]
                );
            }
            assert_eq!(h.last[f * 4 + 2], monitors[0]);
            assert_eq!(h.last[f * 4 + 3], monitors[1]);
        }
        let stale = h.edit(0, 8);
        let mut bad = stale.clone();
        bad["body"]["expected_generation"] = json!("0");
        assert_eq!(
            h.host.dispatch_aux(&h.writer, bad, h.now).unwrap()["state"],
            "rejected"
        );
        let before = h.snapshot();
        let mut spoof = stale;
        spoof["writer"] = json!(h.owner.writer());
        assert!(h.host.dispatch_aux(&h.writer, spoof, h.now).is_err());
        assert_eq!(before["buses"], h.snapshot()["buses"]);
        let sounding = h.snapshot()["buses"].clone();
        let request = h.edit(0, 10);
        assert_eq!(
            h.host.dispatch_aux(&h.writer, request, h.now).unwrap()["state"],
            "preparing"
        );
        h.host.disconnect(&h.writer);
        // The queued Prepare can still return Prepared after cancellation.
        for _ in 0..8 {
            h.block();
        }
        assert_eq!(h.snapshot()["buses"], sounding);
        assert!(h.host.media_owner.is_some());
        h.host.disconnect(&h.owner);
        for _ in 0..8 {
            let frame = h.host.provider.frame();
            let capture: [f64; 192] =
                std::array::from_fn(|i| ((i % 4 + 1) * 128) as f64 / 8388608.);
            h.host
                .process_source(h.now, 9, frame, &capture, &mut h.last)
                .unwrap();
            h.now += 1;
        }
        for f in 0..48 {
            assert_eq!(
                &h.last[f * 4..f * 4 + 2],
                &h.host.provider.brain_fx_send()[f * 2..f * 2 + 2]
            );
        }
        assert_eq!(h.snapshot()["available"], false);
    }
}
