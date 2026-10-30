#[path = "fx_aux_relay.rs"]
mod fx_aux_relay;
#[path = "fx_relay.rs"]
mod fx_relay;
// The authenticated endpoint for the existing composed LocalAudio graph. It
// never starts a device or advances time from packet arrival. The source owner
// calls `process_source` with the device/fake-device epoch and first frame.
use super::*;
use crate::brain_audio::bridge::{Bridge, BridgeConfig, BridgeEpochs};
use crate::{
    local_audio::LocalAudio,
    show::Counter,
    transport::{JitterBuffer, Packet, Role, StreamSpec, WetGate},
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
struct DeviceChange {
    fingerprint: String,
    request: crate::control_model::Request,
    config: crate::brain_audio::device::DeviceConfig,
    ticket: u64,
    deadline: u64,
    sent: bool,
}
#[derive(Clone, Copy)]
struct MonitorRetirement {
    first_ms: u64,
    first_frame: u64,
}
struct BrainMediaState {
    context: AuthenticatedContext,
    descriptor: BrainMediaDescriptor,
    talkback: Bridge,
    bridge_epochs: BridgeEpochs,
    last_received: Option<u64>,
    outgoing: VecDeque<Vec<u8>>,
    monitor_retirement: Option<MonitorRetirement>,
}
impl BrainMediaState {
    fn epochs(&self) -> BridgeEpochs {
        self.bridge_epochs
    }
}
struct WetGroup {
    spec: StreamSpec,
    first: u64,
    buffer: JitterBuffer,
}

#[derive(Default, Serialize)]
pub struct HostMediaStats {
    pub accepted_talkback_packets: u64,
    pub rejected_talkback_packets: u64,
    pub emitted_monitor_packets: u64,
    pub dropped_monitor_packets: u64,
    pub accepted_wet_packets: u64,
    pub late_wet_packets: u64,
    pub rejected_wet_packets: u64,
    pub nonzero_received_wet_samples: u64,
    pub nonzero_rendered_wet_samples: u64,
    pub rendered_wet_energy: f64,
    pub first_wet_source_frame: Option<u64>,
    pub last_wet_source_frame: Option<u64>,
    pub first_wet_output_frame: Option<u64>,
    pub last_wet_output_frame: Option<u64>,
}

struct DeviceRead {
    at: u64,
    identity: Option<(u64, u64)>,
}

pub struct HostAuthority {
    fx: fx_relay::FxRelay,
    provider: LocalAudio,
    meter_reads: BTreeMap<u64, u64>,
    reads: BTreeMap<u64, ReadState>,
    structural_reads: BTreeMap<u64, u64>,
    fx_aux_reads: BTreeMap<u64, u64>,
    master_eq_reads: BTreeMap<u64, u64>,
    brain_reads: BTreeMap<u64, u64>,
    held_queries: BTreeMap<u64, u64>,
    paired_queries: BTreeMap<u64, u64>,
    brain_generations: BTreeMap<u64, u64>,
    device_reads: BTreeMap<u64, DeviceRead>,
    device_owner: Option<AuthenticatedContext>,
    device_observation: Option<BrainDeviceObservation>,
    device_observed_ms: Option<u64>,
    device_pending: Option<DeviceChange>,
    device_messages: VecDeque<(u64, Value)>,
    device_results: VecDeque<Value>,
    device_history: VecDeque<(String, Value)>,
    device_ticket: u64,
    brain: Option<BrainMediaState>,
    talkback: [f64; 48],
    descriptor: Option<MediaDescriptor>,
    media_owner: Option<u64>,
    wet_groups: Vec<WetGroup>,
    gates: [WetGate; 8],
    wet_channels: usize,
    aux: fx_aux_relay::AuxRelay,
    wet: [f64; 96],
    outgoing: VecDeque<Vec<u8>>,
    completions: VecDeque<Value>,
    analysis_indices: Vec<usize>,
    analysis: Vec<f64>,
    fx_send: Vec<f64>,
    capability_generation: u64,
    media_stats: HostMediaStats,
    accepted_wet_hash: Sha256,
    observed_source_epoch: u64,
}
// Account for the existing GP-REMOTE Reply wrapper as well as the additive
// envelope. Compute its small overhead through the real serializer; do not clone
// or serialize another topology merely to size the transport wrapper.
fn paired_transport_capacity(payload_bytes: usize, session: u64) -> Result<()> {
    let wrapper = serde_json::to_vec(&Response::Reply {
        session: Counter(session),
        payload: Value::Null,
    })
    .map_err(|e| e.to_string())?;
    let overhead = wrapper.len() - b"null".len();
    if payload_bytes
        .checked_add(overhead)
        .is_none_or(|n| n > crate::snapshot_pages::ASSEMBLY_BYTES)
    {
        return Err("paired transport assembly capacity".into());
    }
    Ok(())
}
impl HostAuthority {
    /// Both server read gates commit only after the whole paired envelope exists.
    fn finish_paired_readback(
        &mut self,
        context: &AuthenticatedContext,
        reply: crate::paired_readback::Reply,
        now_ms: u64,
    ) -> Result<Value> {
        use crate::paired_readback::{Reason, Reply};
        let mut reply = reply;
        // The entire envelope (including context) shares the existing 1 MiB cap.
        // A failed construction cannot admit either server read latch. The
        // authenticated query nonce was already consumed without caching a reply.
        let encoded = reply.encode().and_then(|bytes| {
            paired_transport_capacity(bytes.len(), context.session())?;
            Ok(bytes)
        });
        let bytes = match encoded {
            Ok(bytes) => bytes,
            Err(_) if reply.raw.is_some() => {
                reply = Reply::new(reply.context, Err(Reason::Capacity));
                reply.encode()?
            }
            Err(e) => return Err(e),
        };
        let value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if reply.raw.is_some() {
            self.reads
                .entry(context.session())
                .or_default()
                .admit_audio_snapshot();
            self.brain_reads.insert(context.session(), now_ms);
        }
        Ok(value)
    }

    pub fn new(provider: LocalAudio, capability_generation: u64) -> Result<Self> {
        if capability_generation == 0 {
            return Err("zero capability generation".into());
        }
        let observed_source_epoch = provider.source_epoch();
        Ok(Self {
            fx: fx_relay::FxRelay::default(),
            provider,
            meter_reads: BTreeMap::new(),
            reads: BTreeMap::new(),
            structural_reads: BTreeMap::new(),
            fx_aux_reads: BTreeMap::new(),
            master_eq_reads: BTreeMap::new(),
            brain_reads: BTreeMap::new(),
            held_queries: BTreeMap::new(),
            paired_queries: BTreeMap::new(),
            brain_generations: BTreeMap::new(),
            device_reads: BTreeMap::new(),
            device_owner: None,
            device_observation: None,
            device_observed_ms: None,
            device_pending: None,
            device_messages: VecDeque::with_capacity(8),
            device_results: VecDeque::with_capacity(16),
            device_history: VecDeque::with_capacity(64),
            device_ticket: 0,
            brain: None,
            talkback: [0.; 48],
            descriptor: None,
            media_owner: None,
            wet_groups: Vec::new(),
            gates: [WetGate::default(); 8],
            wet_channels: 2,
            aux: Default::default(),
            wet: [0.; 96],
            outgoing: VecDeque::with_capacity(256),
            completions: VecDeque::with_capacity(128),
            analysis_indices: Vec::new(),
            analysis: Vec::new(),
            fx_send: vec![0.; 96],
            capability_generation,
            media_stats: HostMediaStats::default(),
            accepted_wet_hash: Sha256::new(),
            observed_source_epoch,
        })
    }
    fn clear_media_history(&mut self) {
        self.fx_clear_media();
        self.clear_aux();
        self.brain = None;
        self.provider.close_brain_audio();
        self.descriptor = None;
        self.media_owner = None;
        self.wet_groups.clear();
        self.gates = [WetGate::default(); 8];
        self.wet.fill(0.);
        self.outgoing.clear();
    }
    fn queue_device_result(&mut self, value: Value) {
        if self.device_results.len() == 16 {
            self.device_results.pop_front();
        }
        self.device_results.push_back(value);
    }
    fn finish_device(&mut self, change: &DeviceChange, state: &str, reason: Option<String>) {
        let value = self.device_reply(change, state, reason);
        if self.device_history.len() == 64 {
            self.device_history.pop_front();
        }
        self.device_history
            .push_back((change.fingerprint.clone(), value.clone()));
        self.queue_device_result(value);
    }
    fn device_reply(&self, change: &DeviceChange, state: &str, reason: Option<String>) -> Value {
        serde_json::json!({"contract":"GP15-device","version":1,"state":state,"reason":reason,
            "ticket":change.ticket,"context":crate::mixer_control::RequestContext::request(&change.request),
            "revision":self.provider.brain_snapshot().revision,"stagebox_frame":self.provider.frame(),"observation":self.device_observation})
    }
    fn commit_device_intent(&mut self, now: u64) -> Result<()> {
        if self.device_pending.as_ref().is_some_and(|c| !c.sent)
            && self.provider.engine_mut().external_boundary().is_none()
        {
            let change = self.device_pending.take().unwrap();
            self.finish_device(
                &change,
                "failed_device",
                Some("configuration intent revoked".into()),
            );
        }
        let Some(change) = self.device_pending.as_ref() else {
            return Ok(());
        };
        if !change.sent
            && self.provider.engine_mut().external_boundary() == Some(self.provider.frame())
        {
            let valid = self
                .device_owner
                .as_ref()
                .is_some_and(|o| o.check_current().is_ok());
            let reply = self.provider.engine_mut().commit_external(now, || {
                if valid {
                    Ok(())
                } else {
                    Err("Brain duplex unavailable".into())
                }
            })?;
            let mut change = self.device_pending.take().unwrap();
            if reply.kind != "applied" {
                self.finish_device(&change, "failed_device", reply.body.reason);
                return Ok(());
            }
            self.provider.close_brain_audio();
            self.brain = None;
            change.sent = true;
            let owner = self.device_owner.as_ref().unwrap();
            self.device_messages.push_back((owner.session(),serde_json::json!({"contract":"GP15-device","version":1,"kind":"apply_configuration","ticket":change.ticket,"config":change.config})));
            self.queue_device_result(self.device_reply(&change, "accepted_intent", None));
            self.device_pending = Some(change);
        }
        if self
            .device_pending
            .as_ref()
            .is_some_and(|c| c.sent && (now >= c.deadline || self.device_owner.is_none()))
        {
            let change = self.device_pending.take().unwrap();
            self.finish_device(
                &change,
                "failed_device",
                Some("device acknowledgement deadline/session loss".into()),
            );
        }
        Ok(())
    }
    fn dispatch_device(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        now: u64,
    ) -> Result<Value> {
        if payload.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("Brain device version".into());
        }
        match payload.get("kind").and_then(Value::as_str) {
            Some("device_snapshot") => {
                self.device_reads.insert(
                    context.session(),
                    DeviceRead {
                        at: now,
                        identity: self
                            .device_observation
                            .as_ref()
                            .map(|o| (o.brain_epoch, o.brain_map)),
                    },
                );
                Ok(
                    serde_json::json!({"contract":"GP15-device","version":1,"state":"snapshot","observation":self.device_observation,
                    "observed_ms":self.device_observed_ms,"connected":self.device_owner.is_some(),"pending":self.device_pending.as_ref().map(|c|c.ticket),"bridge":self.brain_bridge_status().as_ref().map(brain_bridge_telemetry)}),
                )
            }
            Some("device_observation") => {
                context.require(&Permission::LocalOperatorMonitor)?;
                if self.device_owner.as_ref().is_some_and(|o| o != context) {
                    return Err("Brain device owner attached".into());
                }
                let observation: BrainDeviceObservation = serde_json::from_value(
                    payload
                        .get("observation")
                        .cloned()
                        .ok_or("device observation")?,
                )
                .map_err(|e| e.to_string())?;
                observation.validate()?;
                if let Some(ticket) = observation.ticket {
                    let change = self
                        .device_pending
                        .as_ref()
                        .ok_or("unexpected device ticket")?;
                    if !change.sent || ticket != change.ticket || now >= change.deadline {
                        return Err("stale device acknowledgement".into());
                    }
                    if observation.success == Some(true)
                        && (observation.config != change.config
                            || self.device_observation.as_ref().is_some_and(|o| {
                                observation.brain_epoch <= o.brain_epoch
                                    || observation.brain_map <= o.brain_map
                            }))
                    {
                        return Err(
                            "device configuration readback mismatch/fresh epoch required".into(),
                        );
                    }
                } else if self.device_observation.as_ref().is_some_and(|o| {
                    observation.brain_epoch < o.brain_epoch
                        || observation.brain_map < o.brain_map
                        || observation.brain_epoch == o.brain_epoch && observation.frame < o.frame
                }) {
                    return Err("stale device observation".into());
                }
                let replaced = self.device_observation.as_ref().is_some_and(|o| {
                    o.brain_epoch != observation.brain_epoch || o.brain_map != observation.brain_map
                });
                let device_closed = observation
                    .status
                    .get("fault")
                    .is_some_and(|f| !f.is_null())
                    || (observation.status.get("armed").and_then(Value::as_bool) == Some(false)
                        && self.device_observation.as_ref().is_some_and(|o| {
                            o.status.get("armed").and_then(Value::as_bool) == Some(true)
                        }));
                if replaced || device_closed {
                    self.provider.close_brain_audio();
                    self.brain = None;
                }
                self.device_owner = Some(context.clone());
                self.device_observed_ms = Some(now);
                self.device_observation = Some(observation.clone());
                if observation.ticket.is_some() {
                    let change = self.device_pending.take().unwrap();
                    self.finish_device(
                        &change,
                        if observation.success == Some(true) {
                            "applied_device"
                        } else {
                            "failed_device"
                        },
                        observation.error,
                    );
                }
                Ok(
                    serde_json::json!({"contract":"GP15-device","version":1,"state":"observed","brain_epoch":observation.brain_epoch,"brain_map":observation.brain_map}),
                )
            }
            Some("device_configure") => {
                context.require(&Permission::LocalOperatorMonitor)?;
                let fingerprint = format!("{:x}", Sha256::digest(super::encode(&payload)?));
                if let Some(change) = self
                    .device_pending
                    .as_ref()
                    .filter(|c| c.fingerprint == fingerprint)
                {
                    return Ok(self.device_reply(
                        change,
                        if change.sent {
                            "accepted_intent"
                        } else {
                            "pending"
                        },
                        None,
                    ));
                }
                if let Some((_, reply)) =
                    self.device_history.iter().find(|(f, _)| f == &fingerprint)
                {
                    return Ok(reply.clone());
                }
                if self.device_pending.is_some() || self.device_messages.len() >= 8 {
                    return Err("device configuration backpressure".into());
                }
                if self.device_owner.is_none() {
                    return Err("Brain device unavailable".into());
                }
                if !self
                    .device_reads
                    .get(&context.session())
                    .is_some_and(|read| {
                        now >= read.at
                            && now - read.at <= 250
                            && read.identity.is_some()
                            && read.identity
                                == self
                                    .device_observation
                                    .as_ref()
                                    .map(|o| (o.brain_epoch, o.brain_map))
                    })
                {
                    return Err("fresh device snapshot required".into());
                }
                let mut request_value = payload.clone();
                let object = request_value.as_object_mut().ok_or("device command")?;
                let config: crate::brain_audio::device::DeviceConfig =
                    serde_json::from_value(object.remove("config").ok_or("device config")?)
                        .map_err(|e| e.to_string())?;
                config.validate().map_err(|e| e.to_string())?;
                object.insert("contract".into(), Value::String("C-AUDIO".into()));
                object.insert("version".into(), serde_json::json!(2));
                object.insert("kind".into(), Value::String("renew".into()));
                object.insert("body".into(), serde_json::json!({}));
                let request =
                    crate::control_model::Request::decode(&super::encode(&request_value)?)?;
                if self.provider.engine_mut().writer_scope(&request, now)
                    != Some(crate::control_model::Scope::LocalOperatorMonitor)
                {
                    return Err("local operator lease required".into());
                }
                let (_, cached) = self.provider.engine_mut().begin_external(
                    &request,
                    &fingerprint,
                    crate::control_model::Scope::LocalOperatorMonitor,
                    now,
                )?;
                if let Some(reply) = cached {
                    return Ok(
                        serde_json::json!({"contract":"GP15-device","version":1,"state":"replayed","result":reply}),
                    );
                }
                self.device_ticket = self
                    .device_ticket
                    .checked_add(1)
                    .ok_or("device ticket exhausted")?;
                let change = DeviceChange {
                    fingerprint,
                    request,
                    config,
                    ticket: self.device_ticket,
                    deadline: now.saturating_add(2000),
                    sent: false,
                };
                let reply = self.device_reply(&change, "pending", None);
                self.device_pending = Some(change);
                Ok(reply)
            }
            _ => Err("Brain device command unsupported".into()),
        }
    }
    fn update_brain_readiness(&mut self, now: u64) {
        let snapshot = self.provider.brain_snapshot();
        let device = self.device_observation.as_ref().filter(|o| {
            self.device_observed_ms
                .is_some_and(|t| now >= t && now - t <= 250)
                && o.status.get("armed").and_then(Value::as_bool) == Some(true)
                && o.status.get("fault").is_none_or(Value::is_null)
        });
        let valid = self.brain.as_ref().filter(|b| {
            b.context.check_current().is_ok()
                && b.descriptor.stagebox_epoch == self.provider.source_epoch()
                && b.descriptor.stagebox_map == self.provider.topology().map_revision
                && device.is_some_and(|o| {
                    o.brain_epoch == b.descriptor.brain_epoch
                        && o.brain_map == b.descriptor.brain_map
                })
        });
        let talkback = valid.is_some_and(|b| {
            let status = b.talkback.status();
            b.descriptor.talkback
                && status.ready
                && status.armed
                && status.fault.is_none()
                && snapshot.held_generation.map(|g| g.0) == Some(b.descriptor.hold_generation)
                && self.provider.brain_media_authorized(now)
        });
        let monitor = valid.is_some_and(|b| {
            b.descriptor.monitor
                && b.descriptor.selection_generation == snapshot.selection_generation.0
        }) && snapshot.monitor_armed
            && !snapshot.monitor_mute
            && device
                .and_then(|o| o.status.get("monitor_bridge"))
                .is_some_and(|s| {
                    valid.is_some_and(|b| brain_monitor_bridge_current(s, &b.descriptor))
                });
        self.provider.set_brain_path_readiness(talkback, monitor);
    }
    fn prepare_brain_block(&mut self, frame: u64) {
        self.talkback.fill(0.);
        let snapshot = self.provider.brain_snapshot();
        let Some(brain) = &mut self.brain else {
            return;
        };
        if brain.context.check_current().is_err()
            || brain.descriptor.stagebox_epoch != self.provider.source_epoch()
            || brain.descriptor.stagebox_map != self.provider.topology().map_revision
        {
            self.brain = None;
            self.provider.close_brain_audio();
            return;
        }
        if !brain.descriptor.talkback {
            return;
        }
        if snapshot.held_generation.map(|v| v.0) != Some(brain.descriptor.hold_generation) {
            brain.talkback.invalidate();
            return;
        }
        let epochs = brain.epochs();
        let status = brain.talkback.status();
        if status.fault.is_some() {
            self.provider.close_brain_audio();
            return;
        }
        if !status.armed && status.ready {
            let _ = brain.talkback.arm(epochs);
        }
        if brain.talkback.status().armed
            && brain
                .talkback
                .render(epochs, frame, &mut self.talkback)
                .is_err()
        {
            self.provider.close_brain_audio();
        }
    }
    fn emit_brain_monitor(&mut self, frame: u64, now: u64) -> Result<()> {
        let snapshot = self.provider.brain_snapshot();
        let Some(brain) = self.brain.as_ref() else {
            return Ok(());
        };
        let retiring = brain.monitor_retirement.is_some()
            || snapshot.selection_generation.0 != brain.descriptor.selection_generation;
        // A silent tail is permitted only for a still-live duplex owner. It is
        // not a grace period for revoked permissions, stale device telemetry,
        // or replaced source/device identities. Disconnect clears device_owner.
        if retiring && brain.descriptor.monitor {
            let live = brain
                .context
                .require(&Permission::LocalOperatorMonitor)
                .is_ok()
                && self.device_owner.as_ref().is_some_and(|owner| {
                    owner == &brain.context
                        && owner.require(&Permission::LocalOperatorMonitor).is_ok()
                })
                && brain.descriptor.stagebox_epoch == self.provider.source_epoch()
                && brain.descriptor.stagebox_map == self.provider.topology().map_revision
                && self
                    .device_observed_ms
                    .is_some_and(|t| now >= t && now - t <= 250)
                && self.device_observation.as_ref().is_some_and(|o| {
                    o.brain_epoch == brain.descriptor.brain_epoch
                        && o.brain_map == brain.descriptor.brain_map
                        && o.status.get("armed").and_then(Value::as_bool) == Some(true)
                        && o.status.get("fault").is_none_or(Value::is_null)
                });
            let expired = brain.monitor_retirement.is_some_and(|r| {
                now < r.first_ms
                    || now - r.first_ms >= 250
                    || frame < r.first_frame
                    || frame - r.first_frame >= 12_000
            });
            if !live || expired {
                self.brain = None;
                self.provider.close_brain_audio();
                return Ok(());
            }
        }
        let brain = self.brain.as_mut().unwrap();
        if !brain.descriptor.monitor {
            brain.outgoing.clear();
            return Ok(());
        }
        if retiring && brain.monitor_retirement.is_none() {
            // Preserve source indices, headers and order: removing queued frames
            // would cause a genuine timeline gap at the receiving ASRC.
            for packet in &mut brain.outgoing {
                if let Err(error) = BrainPacket::silence_monitor(packet, &brain.descriptor) {
                    self.brain = None;
                    self.provider.close_brain_audio();
                    return Err(error);
                }
            }
            brain.monitor_retirement = Some(MonitorRetirement {
                first_ms: now,
                first_frame: frame,
            });
        }
        let packet = BrainPacket {
            descriptor: brain.descriptor.clone(),
            role: BrainMediaRole::Monitor,
            frame,
            samples: if retiring {
                vec![0.; 96]
            } else {
                self.provider.brain_monitor_output().to_vec()
            },
        }
        .encode()?;
        if brain.outgoing.len() == 64 {
            brain.outgoing.pop_front();
            self.media_stats.dropped_monitor_packets =
                self.media_stats.dropped_monitor_packets.saturating_add(1);
        }
        brain.outgoing.push_back(packet);
        self.media_stats.emitted_monitor_packets =
            self.media_stats.emitted_monitor_packets.saturating_add(1);
        Ok(())
    }
    pub fn brain_descriptor(&self) -> Option<&BrainMediaDescriptor> {
        self.brain.as_ref().map(|b| &b.descriptor)
    }
    pub fn brain_bridge_status(&self) -> Option<crate::brain_audio::bridge::BridgeStatus> {
        self.brain.as_ref().map(|b| b.talkback.status())
    }
    fn negotiate_brain(&mut self, context: &AuthenticatedContext, payload: Value) -> Result<Value> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Negotiation {
            contract: String,
            version: u32,
            kind: String,
            writer: String,
            descriptor: BrainMediaDescriptor,
        }
        let request: Negotiation = serde_json::from_value(payload).map_err(|e| e.to_string())?;
        if request.contract != "GP15-media"
            || request.version != 1
            || request.kind != "negotiate"
            || request.writer != context.writer()
        {
            return Err("Brain media envelope".into());
        }
        let descriptor = request.descriptor;
        descriptor.validate(context.session(), &self.identity())?;
        if descriptor.talkback {
            context.require(&Permission::TalkbackDestinations)?;
        }
        if descriptor.monitor {
            context.require(&Permission::LocalOperatorMonitor)?;
        }
        if let Some(old) = &self.brain {
            if old.context.session() != context.session() {
                return Err("Brain duplex media owner attached".into());
            }
            if descriptor == old.descriptor {
                return Ok(
                    serde_json::json!({"contract":"GP15-media","version":1,"state":"accepted","descriptor":descriptor}),
                );
            }
            if descriptor.generation <= old.descriptor.generation {
                return Err("Brain media generation stale".into());
            }
        }
        if self
            .brain_generations
            .get(&context.session())
            .is_some_and(|g| descriptor.generation <= *g)
        {
            return Err("retired Brain media generation".into());
        }
        if self.brain_generations.len() >= MAX_CONNECTIONS
            && !self.brain_generations.contains_key(&context.session())
        {
            return Err("Brain session generation capacity".into());
        }
        if self
            .device_owner
            .as_ref()
            .is_some_and(|owner| owner != context)
        {
            return Err("Brain media/device owner mismatch".into());
        }
        if self.device_observation.as_ref().is_some_and(|o| {
            descriptor.brain_epoch != o.brain_epoch || descriptor.brain_map != o.brain_map
        }) {
            return Err("Brain device epoch/map readback mismatch".into());
        }
        let snapshot = self.provider.brain_snapshot();
        if descriptor.monitor && descriptor.selection_generation != snapshot.selection_generation.0
        {
            return Err("Brain monitor selection stale".into());
        }
        if descriptor.talkback
            && descriptor.hold_generation != snapshot.held_generation.map_or(0, |v| v.0)
        {
            return Err("Brain talkback hold stale".into());
        }
        let epochs = BridgeEpochs {
            source: descriptor.brain_epoch,
            destination: descriptor.stagebox_epoch,
            route: descriptor.generation,
        };
        let preserve = self.brain.as_ref().is_some_and(|old| {
            old.descriptor.brain_epoch == descriptor.brain_epoch
                && old.descriptor.stagebox_epoch == descriptor.stagebox_epoch
                && old.descriptor.brain_map == descriptor.brain_map
                && old.descriptor.stagebox_map == descriptor.stagebox_map
                && old.descriptor.hold_generation == descriptor.hold_generation
                && old.descriptor.talkback == descriptor.talkback
                && old.talkback.status().fault.is_none()
        });
        let prepared = if preserve {
            None
        } else {
            Some(Bridge::prepare(BridgeConfig::voice(1), epochs).map_err(|e| e.to_string())?)
        };
        // Talkback hold changes do not change monitor wire identity. Retain its
        // queued source frames independently of talkback bridge replacement; a
        // gap here would correctly fault the already-running monitor ASRC.
        // All fallible validation/preparation above precedes taking old state.
        let mut old = self.brain.take();
        let mut monitor_retirement = None;
        let outgoing = if let Some(old) = old.as_mut().filter(|old| {
            old.context == *context
                && old.descriptor.monitor
                && descriptor.monitor
                && old.descriptor.session == descriptor.session
                && old.descriptor.stagebox_epoch == descriptor.stagebox_epoch
                && old.descriptor.brain_epoch == descriptor.brain_epoch
                && old.descriptor.stagebox_map == descriptor.stagebox_map
                && old.descriptor.brain_map == descriptor.brain_map
                && old.descriptor.selection_generation == descriptor.selection_generation
                && old.descriptor.sample_rate == descriptor.sample_rate
                && old.descriptor.frames == descriptor.frames
        }) {
            monitor_retirement = old.monitor_retirement;
            std::mem::take(&mut old.outgoing)
        } else {
            VecDeque::with_capacity(64)
        };
        let (talkback, last_received, bridge_epochs) = if let Some(talkback) = prepared {
            (talkback, None, epochs)
        } else {
            let old = old.unwrap();
            (old.talkback, old.last_received, old.bridge_epochs)
        };
        self.brain_generations
            .insert(context.session(), descriptor.generation);
        self.brain = Some(BrainMediaState {
            context: context.clone(),
            descriptor: descriptor.clone(),
            talkback,
            bridge_epochs,
            last_received,
            outgoing,
            monitor_retirement,
        });
        Ok(
            serde_json::json!({"contract":"GP15-media","version":1,"state":"accepted","descriptor":descriptor}),
        )
    }
    #[cfg(test)]
    pub(crate) fn test_admitted_wet(&self) -> &[f64; 96] {
        &self.wet
    }
    pub fn media_stats(&self) -> &HostMediaStats {
        &self.media_stats
    }
    pub fn accepted_wet_sha256(&self) -> String {
        format!("{:x}", self.accepted_wet_hash.clone().finalize())
    }
    pub fn provider(&self) -> &LocalAudio {
        &self.provider
    }
    pub fn provider_mut(&mut self) -> &mut LocalAudio {
        &mut self.provider
    }
    pub fn descriptor(&self) -> Option<&MediaDescriptor> {
        self.descriptor.as_ref()
    }
    /// Source worker entry, not a socket/TLS callback. The only timeline advance
    /// is inside the same production provider used by local/headless adapters.
    pub fn process_source(
        &mut self,
        now_ms: u64,
        epoch: u64,
        frame: u64,
        capture: &[f64],
        playback: &mut [f64],
    ) -> Result<()> {
        playback.fill(0.);
        if self.observed_source_epoch != self.provider.source_epoch() {
            self.clear_media_history();
            self.observed_source_epoch = self.provider.source_epoch();
        }
        if epoch != self.provider.source_epoch() || frame != self.provider.frame() {
            self.clear_media_history();
            self.provider.quiesce_source("source_discontinuity")?;
            return Err("source timeline".into());
        }
        if self.descriptor.as_ref().is_some_and(|d| {
            d.source_epoch.0 != epoch || d.map_generation.0 != self.provider.topology().map_revision
        }) {
            self.clear_media_history();
        }
        let mut incoming = [None; 384];
        for group in &mut self.wet_groups {
            if frame
                < group
                    .first
                    .saturating_add(u64::from(group.spec.delay_frames))
            {
                continue;
            }
            if group
                .buffer
                .next_frame()
                .checked_add(u64::from(group.spec.delay_frames))
                != Some(frame)
            {
                self.provider.quiesce_source("wet_source_timeline")?;
                return Err("wet source timeline".into());
            }
            let mut bytes = [0; crate::transport::MAX_DATAGRAM];
            if let Some(length) = group
                .buffer
                .pop(&mut bytes)
                .map_err(|e| format!("wet queue {e:?}"))?
            {
                let packet =
                    Packet::parse(&bytes[..length]).map_err(|e| format!("wet packet {e:?}"))?;
                for f in 0..48 {
                    for ch in 0..usize::from(group.spec.channels) {
                        incoming
                            [f * self.wet_channels + usize::from(group.spec.first_channel) + ch] =
                            Some(
                                packet
                                    .sample(f * usize::from(group.spec.channels) + ch)
                                    .map_err(|e| format!("wet sample {e:?}"))?,
                            );
                    }
                }
            }
        }
        self.wet.fill(0.);
        for f in 0..48 {
            for ch in 0..self.wet_channels {
                self.wet[f * 2 + ch % 2] +=
                    self.gates[ch].step(incoming[f * self.wet_channels + ch]);
            }
        }
        if self.wet_channels == 8 {
            for (i, sample) in self.wet.iter_mut().enumerate() {
                *sample *= self
                    .provider
                    .engine()
                    .output_safety_gain(frame + (i / 2) as u64);
            }
        }
        for sample in self.wet {
            self.media_stats.nonzero_rendered_wet_samples += u64::from(sample != 0.);
            self.media_stats.rendered_wet_energy += sample * sample;
        }
        // Always supply the explicit external wet path, including silence after
        // disconnect. This prevents fallback to another local FX instance.
        self.tick_fx(now_ms)?;
        self.tick_aux(now_ms)?;
        self.commit_device_intent(now_ms)?;
        self.prepare_brain_block(frame);
        self.provider.tick_with_capture_and_brain(
            now_ms,
            epoch,
            frame,
            capture,
            playback,
            Some(&self.wet),
            Some(&self.talkback),
        )?;
        self.emit_brain_monitor(frame, now_ms)?;
        self.update_brain_readiness(now_ms);
        for reply in self.provider.take_remote_completions() {
            if self.completions.len() == 128 {
                return Err("remote final queue capacity".into());
            }
            self.completions.push_back(reply);
        }
        if let Some(descriptor) = &self.descriptor {
            if descriptor.source_epoch.0 != epoch
                || descriptor.map_generation.0 != self.provider.topology().map_revision
            {
                self.descriptor = None;
                self.wet_groups.clear();
                self.outgoing.clear();
                return Ok(());
            }
            let capture_channels = self.provider.topology().capture_channels;
            for f in 0..48 {
                for (index, slot) in self.analysis_indices.iter().enumerate() {
                    self.analysis[f * self.analysis_indices.len() + index] =
                        capture[f * capture_channels + slot];
                }
            }
            if descriptor.channels(MediaRole::FxSend) == 8 {
                let aux = self.provider.engine_mut().mixer().fx_aux_samples();
                for f in 0..48 {
                    for (b, samples) in aux.iter().enumerate() {
                        self.fx_send[f * 8 + b * 2..f * 8 + b * 2 + 2]
                            .copy_from_slice(&samples[f * 2..f * 2 + 2]);
                    }
                }
            } else {
                self.fx_send.copy_from_slice(self.provider.brain_fx_send());
            }
            let mut packets = Vec::new();
            if !self.analysis_indices.is_empty() {
                packets.extend(encode_grouped(
                    descriptor,
                    MediaRole::Analysis,
                    frame,
                    &self.analysis,
                )?);
            }
            if descriptor.channels(MediaRole::FxSend) > 0 {
                packets.extend(encode_grouped(
                    descriptor,
                    MediaRole::FxSend,
                    frame,
                    &self.fx_send,
                )?);
            }
            for packet in packets {
                if self.outgoing.len() == 256 {
                    self.outgoing.pop_front();
                }
                self.outgoing.push_back(packet);
            }
        }
        Ok(())
    }
}
impl AuthorityEndpoint for HostAuthority {
    fn identity(&self) -> EngineIdentity {
        EngineIdentity {
            source_epoch: Counter(self.provider.source_epoch()),
            capability_generation: Counter(self.capability_generation),
            map_generation: Counter(self.provider.topology().map_revision),
        }
    }
    fn dispatch(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        now_ms: u64,
    ) -> Result<Option<Value>> {
        context.check_writer(&payload)?;
        if payload.get("contract").and_then(Value::as_str) == Some("GP-SPECTRUM") {
            if self
                .meter_reads
                .get(&context.session())
                .is_some_and(|t| now_ms.saturating_sub(*t) < 80)
            {
                return Err("spectrum observer poll bound".into());
            }
            let request: crate::spectrum::Request =
                serde_json::from_value(payload).map_err(|e| e.to_string())?;
            self.meter_reads.insert(context.session(), now_ms);
            return serde_json::to_value(self.provider.spectrum_snapshot(&request)?)
                .map(Some)
                .map_err(|e| e.to_string());
        }

        if payload.get("contract").and_then(Value::as_str)
            == Some(crate::remote::fx_aux_owner::CONTRACT)
        {
            return self.dispatch_aux_owner(context, payload, now_ms).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::fx_aux_wire::CONTRACT) {
            if !self.provider.fx_aux_enabled() {
                return self.dispatch_aux(context, payload, now_ms).map(Some);
            }
            let (_, mutation) =
                crate::fx_aux_wire::request(&payload, self.provider.engine_mut().wire_version())?;
            if mutation.is_some() {
                context.require(&Permission::FxConfiguration)?;
            }
            let fresh = self
                .fx_aux_reads
                .get(&context.session())
                .is_some_and(|t| now_ms.saturating_sub(*t) <= 1000);
            let read = mutation.is_none();
            let reply = self.provider.fx_aux_request(payload, now_ms, fresh, None)?;
            if read {
                self.fx_aux_reads.insert(context.session(), now_ms);
            }
            return Ok(Some(reply));
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::fx_wire::CONTRACT) {
            return self.dispatch_fx(context, payload, now_ms).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::meter_wire::CONTRACT) {
            if self
                .meter_reads
                .get(&context.session())
                .is_some_and(|t| now_ms.saturating_sub(*t) < 40)
            {
                return Err("meter observer poll bound".into());
            }
            let request = crate::meter_wire::Request::decode(&super::encode(&payload)?)?;
            self.meter_reads.insert(context.session(), now_ms);
            let reply = self
                .provider
                .meter_snapshot(&request, self.capability_generation)?;
            return serde_json::to_value(reply)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str)
            == Some(crate::lease_maintenance::CONTRACT)
        {
            use crate::lease_maintenance::{Reason, Reply, Request};
            let request = Request::decode(&super::encode(&payload)?)?;
            let result = (|| {
                if request.authenticated_session.0 != context.session()
                    || request.writer != context.writer
                    || request.epoch != self.identity().source_epoch
                    || request.show_id != self.provider.show_id()
                    || request.capability_generation != self.identity().capability_generation
                    || request.map_generation != self.identity().map_generation
                {
                    return Err(Reason::Identity);
                }
                if !request.supported_scope() {
                    return Err(Reason::Scope);
                }
                context
                    .require(&scope_permission(request.scope))
                    .map_err(|_| Reason::Permission)?;
                self.provider
                    .maintain_lease(&request, now_ms, self.capability_generation)
            })();
            // Maintenance is never a readback admission or a heartbeat witness.
            let bytes = Reply::new(request, result).encode()?;
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::paired_readback::CONTRACT)
        {
            use crate::paired_readback::{Reason, Reply, Request};
            let request = Request::decode(&super::encode(&payload)?)?;
            let result = (|| {
                if request.authenticated_session.0 != context.session()
                    || request.writer != context.writer
                    || request.epoch != self.identity().source_epoch
                    || request.show_id != self.provider.show_id()
                    || request.capability_generation != self.identity().capability_generation
                    || request.map_generation != self.identity().map_generation
                {
                    return Err(Reason::Identity);
                }
                if self
                    .paired_queries
                    .get(&context.session())
                    .is_some_and(|last| request.query_id.0 <= *last)
                {
                    return Err(Reason::QueryId);
                }
                if (!self.paired_queries.contains_key(&context.session())
                    && self.paired_queries.len() >= MAX_CONNECTIONS)
                    || (!self.reads.contains_key(&context.session())
                        && self.reads.len() >= MAX_CONNECTIONS)
                {
                    return Err(Reason::Capacity);
                }
                // A read nonce is single-use after authenticated attachment and
                // bounded namespace admission, even when observation later fails.
                // It is not a freshness latch and stores no response body.
                self.paired_queries
                    .insert(context.session(), request.query_id.0);
                self.provider
                    .paired_readback(&request, now_ms, self.capability_generation)
            })();
            return self
                .finish_paired_readback(context, Reply::new(request, result), now_ms)
                .map(Some);
        }
        if self.reads.len() >= MAX_CONNECTIONS && !self.reads.contains_key(&context.session()) {
            return Err("remote read-state capacity".into());
        }
        if payload.get("contract").and_then(Value::as_str) == Some(crate::held_proof::CONTRACT) {
            use crate::held_proof::{Reason, Reply, Request};
            let request = Request::decode(&super::encode(&payload)?)?;
            let result = (|| {
                if request.authenticated_session.0 != context.session()
                    || request.writer != context.writer
                    || request.capability_generation != self.identity().capability_generation
                    || request.map_generation != self.identity().map_generation
                {
                    return Err(Reason::Identity);
                }
                if request.scope != crate::control_model::Scope::TalkbackDestinations {
                    return Err(Reason::Scope);
                }
                context
                    .require(&Permission::TalkbackDestinations)
                    .map_err(|_| Reason::Permission)?;
                if self
                    .held_queries
                    .get(&context.session())
                    .is_some_and(|last| request.query_id.0 <= *last)
                {
                    return Err(Reason::QueryId);
                }
                if !self.held_queries.contains_key(&context.session())
                    && self.held_queries.len() >= MAX_CONNECTIONS
                {
                    return Err(Reason::Capacity);
                }
                let witness =
                    self.provider
                        .held_proof(&request, now_ms, self.capability_generation)?;
                Ok(witness)
            })();
            let accepted = result.is_ok();
            let query_id = request.query_id.0;
            let reply = Reply::new(request, result);
            reply.validate()?;
            let bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
            if bytes.len() > crate::held_proof::MAX_BYTES {
                return Err("held proof response capacity".into());
            }
            if accepted {
                self.held_queries.insert(context.session(), query_id);
                self.brain_reads.insert(context.session(), now_ms);
            }
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP15-device") {
            return self.dispatch_device(context, payload, now_ms).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP15-media") {
            return self.negotiate_brain(context, payload).map(Some);
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP15-brain") {
            let request = crate::brain_control::Request::decode(&super::encode(&payload)?)?;
            let snapshot = matches!(
                request.command,
                crate::brain_control::Command::BrainSnapshot {}
            );
            if !snapshot {
                context.require(&scope_permission(request.scope()))?;
                if self
                    .provider
                    .engine_mut()
                    .writer_scope(&request.authority_request(), now_ms)
                    != Some(request.scope())
                {
                    return Err("Brain scoped lease required".into());
                }
            }
            let fresh = self
                .brain_reads
                .get(&context.session())
                .is_some_and(|t| now_ms >= *t && now_ms - *t <= 250);
            let reply = self.provider.brain_request(request, now_ms, fresh, None)?;
            if snapshot && reply.snapshot.is_some() {
                self.brain_reads.insert(context.session(), now_ms);
            }
            return serde_json::to_value(reply)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        if payload.get("contract").and_then(Value::as_str) == Some("GP05-modules") {
            let request = crate::module_wire::ModuleRequest::decode(&super::encode(&payload)?)?;
            if request.version != self.provider.engine_mut().wire_version() {
                return Err("module version incompatible with admitted topology".into());
            }
            if !matches!(
                request.command,
                crate::module_wire::ModuleCommand::ModuleStatus {}
            ) {
                if !self
                    .reads
                    .get(&context.session())
                    .is_some_and(ReadState::audio_ready)
                {
                    return Err("remote snapshot required before recorder mutation".into());
                }
                context.require(&Permission::Foh)?;
                if self
                    .provider
                    .engine_mut()
                    .writer_scope(&request.authority_request(), now_ms)
                    != Some(crate::control_model::Scope::Foh)
                {
                    return Err("recorder FOH lease required".into());
                }
            }
            return self
                .provider
                .dispatch_module(request, now_ms, None)
                .map(Some);
        }
        if matches!(
            payload.get("contract").and_then(Value::as_str),
            Some("GP14-structure" | "GP18-master-eq")
        ) {
            let eq = payload["contract"] == crate::master_eq_wire::CONTRACT;
            let request = crate::structural_control::Request::decode(&super::encode(&payload)?)?;
            if let Some(scope) = request.scope() {
                context.require(&scope_permission(scope))?;
                if self
                    .provider
                    .engine_mut()
                    .writer_scope(&request.authority_request(), now_ms)
                    != Some(scope)
                {
                    return Err("structural scoped lease required".into());
                }
            }
            let fresh = (if eq {
                &self.master_eq_reads
            } else {
                &self.structural_reads
            })
            .get(&context.session())
            .is_some_and(|t| now_ms >= *t && now_ms - *t <= 250);
            let snapshot = matches!(
                request.command,
                crate::structural_control::Command::StructuralSnapshot {}
                    | crate::structural_control::Command::MasterEqSnapshot {}
            );
            let reply = self
                .provider
                .structural_request(request, now_ms, fresh, None)?;
            if snapshot {
                if eq && reply.master_eq.is_some() {
                    self.master_eq_reads.insert(context.session(), now_ms);
                } else if reply.snapshot.is_some() {
                    self.structural_reads.insert(context.session(), now_ms);
                }
            }
            return serde_json::to_value(reply)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        let state = self.reads.entry(context.session()).or_default();
        dispatch_engine(self.provider.engine_mut(), context, payload, now_ms, state).map(Some)
    }
    fn negotiate(
        &mut self,
        context: &AuthenticatedContext,
        descriptor: &MediaDescriptor,
    ) -> Result<bool> {
        if self
            .media_owner
            .is_some_and(|session| session != context.session())
        {
            return Err("Brain media owner already attached".into());
        }
        if self.fx_control_attached(context.session()) || self.aux_mode() {
            return Err("FX owner replacement requires fresh session".into());
        }
        descriptor.validate(context, &self.identity(), crate::transport::MAX_DATAGRAM)?;
        if self.provider.fx_aux_enabled()
            && descriptor
                .streams
                .iter()
                .any(|s| matches!(s.role, MediaRole::FxSend | MediaRole::WetReturn))
        {
            return Err("local four-aux owner excludes legacy remote FX media".into());
        }
        if descriptor.streams.iter().any(|s| s.frames != 48) {
            return Err("composed host currently advertises 48-frame media blocks".into());
        }
        let ordered_ids = |role| {
            let mut streams: Vec<_> = descriptor
                .streams
                .iter()
                .filter(|s| s.role == role)
                .collect();
            streams.sort_by_key(|s| s.first_channel);
            streams
                .into_iter()
                .flat_map(|s| s.channel_ids.iter().cloned())
                .collect::<Vec<_>>()
        };
        let analysis_ids = ordered_ids(MediaRole::Analysis);
        let mut indices = Vec::new();
        for id in &analysis_ids {
            indices.push(
                self.provider
                    .topology()
                    .inputs
                    .iter()
                    .find(|input| &input.id == id)
                    .ok_or("unknown admitted raw source")?
                    .capture_slot,
            );
        }
        let send_ids = ordered_ids(MediaRole::FxSend);
        let return_ids = ordered_ids(MediaRole::WetReturn);
        let four_aux = send_ids == crate::remote::fx_aux_owner::channel_ids();
        if four_aux {
            context.require(&Permission::FxConfiguration)?;
            if return_ids != send_ids {
                return Err("four FX aux return map must match sends".into());
            }
        } else {
            for ids in [&send_ids, &return_ids] {
                if !ids.is_empty() && *ids != ["foh-left", "foh-right"] {
                    return Err("explicit stereo or four-aux FX ports required".into());
                }
            }
        }
        let first = self.provider.frame();
        let mut wet_groups = Vec::new();
        for stream in descriptor
            .streams
            .iter()
            .filter(|s| s.role == MediaRole::WetReturn)
        {
            let spec = stream.spec(context.session())?;
            let buffer =
                JitterBuffer::new(spec, first, (first / u64::from(spec.frames)) as u32, 32)
                    .map_err(|e| format!("wet admission {e:?}"))?;
            wet_groups.push(WetGroup {
                spec,
                first,
                buffer,
            });
        }
        if four_aux {
            self.provider.enable_remote_fx_aux()?;
            let zeros = vec![0.; self.provider.topology().inputs.len()];
            for bus in 0..4 {
                self.provider.engine_mut().set_fx_sends(bus, &zeros)?;
            }
        }
        self.wet_channels = if four_aux { 8 } else { 2 };
        self.fx_send = vec![0.; 48 * self.wet_channels];
        self.analysis = vec![0.; 48 * indices.len()];
        self.analysis_indices = indices;
        self.wet_groups = wet_groups;
        self.gates = [WetGate::default(); 8];
        self.wet.fill(0.);
        self.outgoing.clear();
        self.observed_source_epoch = self.provider.source_epoch();
        self.descriptor = Some(descriptor.clone());
        self.media_owner = Some(context.session());
        Ok(true)
    }
    fn media(
        &mut self,
        context: &AuthenticatedContext,
        packet: Packet<'_>,
        _now_ms: u64,
    ) -> Result<()> {
        context.require(&Permission::Fx)?;
        if self.media_owner != Some(context.session()) || packet.spec().role != Role::WetReturn {
            return Err("wet owner/direction".into());
        }
        let group = self
            .wet_groups
            .iter_mut()
            .find(|g| g.spec == packet.spec())
            .ok_or("wet stream identity")?;
        // Deadline/loss statistics belong to the reused GPA1 queue. Drops are
        // expected recovery input, never a source failure or recorder stall.
        match group.buffer.insert_wet_at(packet, self.provider.frame()) {
            Ok(()) => {
                self.media_stats.accepted_wet_packets += 1;
                self.media_stats
                    .first_wet_source_frame
                    .get_or_insert(packet.source_frame());
                self.media_stats.last_wet_source_frame = Some(packet.source_frame());
                self.media_stats
                    .first_wet_output_frame
                    .get_or_insert(packet.output_frame());
                self.media_stats.last_wet_output_frame = Some(packet.output_frame());
                for index in 0..packet.spec().samples() {
                    self.media_stats.nonzero_received_wet_samples +=
                        u64::from(packet.sample(index).is_ok_and(|sample| sample != 0.));
                }
                self.accepted_wet_hash.update(packet.bytes());
            }
            Err(crate::transport::Error::Late) => self.media_stats.late_wet_packets += 1,
            Err(_) => self.media_stats.rejected_wet_packets += 1,
        }
        Ok(())
    }
    fn brain_media(
        &mut self,
        context: &AuthenticatedContext,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<()> {
        let result = (|| {
            context.require(&Permission::TalkbackDestinations)?;
            if !self.provider.brain_media_authorized(now_ms) {
                return Err("Brain talkback lease/deadman/safety closed".into());
            }
            let snapshot = self.provider.brain_snapshot();
            let brain = self.brain.as_mut().ok_or("Brain media not negotiated")?;
            if brain.context != *context
                || !brain.descriptor.talkback
                || snapshot.held_generation.map(|v| v.0) != Some(brain.descriptor.hold_generation)
            {
                return Err("Brain talkback owner/hold/direction".into());
            }
            let packet = BrainPacket::decode(bytes, &brain.descriptor, BrainMediaRole::Talkback)?;
            if brain.last_received.is_some_and(|f| packet.frame <= f) {
                return Err("Brain replay/reordered media".into());
            }
            if let Err(error) = brain
                .talkback
                .push(brain.epochs(), packet.frame, &packet.samples)
            {
                self.provider.close_brain_audio();
                return Err(error.to_string());
            }
            brain.last_received = Some(packet.frame);
            Ok(())
        })();
        if result.is_ok() {
            self.media_stats.accepted_talkback_packets =
                self.media_stats.accepted_talkback_packets.saturating_add(1);
        } else {
            self.media_stats.rejected_talkback_packets =
                self.media_stats.rejected_talkback_packets.saturating_add(1);
        }
        result
    }
    fn poll_brain_media(&mut self, context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        context.check_current()?;
        match &mut self.brain {
            Some(brain) if brain.context == *context => Ok(brain.outgoing.pop_front()),
            _ => Ok(None),
        }
    }
    fn disconnect(&mut self, context: &AuthenticatedContext) {
        self.fx_disconnect(context);
        self.disconnect_aux(context);
        self.brain_reads.remove(&context.session());
        self.held_queries.remove(&context.session());
        self.paired_queries.remove(&context.session());
        self.brain_generations.remove(&context.session());
        self.device_reads.remove(&context.session());
        if self
            .device_owner
            .as_ref()
            .is_some_and(|o| o.session() == context.session())
        {
            self.device_owner = None;
            self.device_observed_ms = None;
            self.provider.close_brain_audio();
            self.device_messages
                .retain(|(s, _)| *s != context.session());
        }
        if self
            .brain
            .as_ref()
            .is_some_and(|b| b.context.session() == context.session())
        {
            self.brain = None;
            self.provider.close_brain_audio();
        }
        self.provider.revoke_writer(context.writer());
        self.meter_reads.remove(&context.session());
        self.reads.remove(&context.session());
        self.structural_reads.remove(&context.session());
        self.fx_aux_reads.remove(&context.session());
        self.master_eq_reads.remove(&context.session());
        self.completions.retain(|r| {
            !(r["contract"] == crate::fx_aux_wire::CONTRACT
                && r["writer"].as_str() == Some(context.writer()))
                && r.get("context")
                    .and_then(|c| c.get("writer"))
                    .and_then(Value::as_str)
                    != Some(context.writer())
        });
        if self.media_owner == Some(context.session()) {
            self.media_owner = None;
            self.descriptor = None;
            self.wet_groups.clear();
            self.outgoing.clear();
        }
        // Keep WetGate history solely for its bounded five-ms fade to silence.
    }
    fn source_frame(&self) -> Option<u64> {
        Some(self.provider.frame())
    }
    fn poll_reply(
        &mut self,
        context: &AuthenticatedContext,
        _now_ms: u64,
    ) -> Result<Option<Value>> {
        if let Some(value) = self.poll_aux(context) {
            return Ok(Some(value));
        }
        if let Some(value) = self.poll_fx(context) {
            return Ok(Some(value));
        }
        if let Some(i) = self
            .device_messages
            .iter()
            .position(|(s, _)| *s == context.session())
        {
            return Ok(self.device_messages.remove(i).map(|(_, v)| v));
        }
        if let Some(index) = self.device_results.iter().position(|r| {
            r.get("context")
                .and_then(|c| c.get("writer"))
                .and_then(Value::as_str)
                == Some(context.writer())
        }) {
            return Ok(self.device_results.remove(index));
        }
        let index = self.completions.iter().position(|r| {
            (r["contract"] == crate::fx_aux_wire::CONTRACT
                && r["writer"].as_str() == Some(context.writer()))
                || r.get("context")
                    .and_then(|c| c.get("writer"))
                    .and_then(Value::as_str)
                    == Some(context.writer())
        });
        Ok(index.and_then(|index| self.completions.remove(index)))
    }
    fn poll_media(&mut self, context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        if self.media_owner == Some(context.session()) {
            Ok(self.outgoing.pop_front())
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod monitor_retirement_tests {
    use super::*;
    use crate::brain_audio::{
        bridge::{BridgeConfig, BridgeEpochs},
        device::{DeviceConfig, PortMap, SampleFormat},
    };
    use crate::control_model::{Command, Request, Scope};
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        host: HostAuthority,
        ctx: AuthenticatedContext,
        policy: PolicyStore,
        descriptor: BrainMediaDescriptor,
        path: std::path::PathBuf,
        lease: Counter,
        request: u64,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    impl Fixture {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("gp-retirement-{}-{label}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut provider = LocalAudio::bind_configured(
                &path,
                "host",
                "11111111-1111-4111-8111-111111111111",
                Counter(9),
                crate::topology::EngineTopology::reference_16_18(0, 4).unwrap(),
            )
            .unwrap();
            provider.rearm().unwrap();
            let policy = PolicyStore::new(vec![Peer {
                id: "duplex".into(),
                certificate_sha256: fingerprint(b"retirement"),
                permissions: [Permission::LocalOperatorMonitor].into_iter().collect(),
            }])
            .unwrap();
            let ctx = policy.authenticate(b"retirement", 12).unwrap();
            let grant = Request {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: "11111111-1111-4111-8111-111111111111".into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(ctx.writer().into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(Counter(0)),
                command: Command::Grant {
                    scope: Scope::LocalOperatorMonitor,
                },
            };
            let lease = provider
                .engine_mut()
                .handle(&grant, 0)
                .unwrap()
                .outcome
                .unwrap()
                .body
                .granted_lease
                .unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let descriptor = BrainMediaDescriptor {
                contract: "GP15-media".into(),
                version: 1,
                session: 12,
                stagebox_epoch: 9,
                brain_epoch: 77,
                stagebox_map: host.identity().map_generation.0,
                brain_map: 1,
                generation: 1,
                selection_generation: host.provider.brain_snapshot().selection_generation.0,
                hold_generation: 0,
                sample_rate: 48000,
                frames: 48,
                talkback: false,
                monitor: true,
            };
            host.negotiate_brain(&ctx, serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":ctx.writer(),"descriptor":descriptor})).unwrap();
            let port = |id: &str, slot| PortMap {
                id: id.into(),
                socket: id.into(),
                slot,
            };
            host.device_owner = Some(ctx.clone());
            host.device_observed_ms = Some(0);
            host.device_observation = Some(BrainDeviceObservation {
                brain_epoch: 77,
                brain_map: 1,
                frame: 0,
                config: DeviceConfig {
                    device_id: "fake".into(),
                    endpoint: "fake:brain".into(),
                    sample_rate: 48000,
                    capture_channels: 1,
                    playback_channels: 2,
                    format: SampleFormat::Float32Le,
                    period_frames: 48,
                    buffer_frames: 192,
                    microphone: port("mic", 0),
                    monitor: [port("left", 0), port("right", 1)],
                    socket_mapping_record: None,
                    shared_clock_record: None,
                    hardware_monitoring_record: None,
                },
                status: serde_json::json!({"armed":true,"fault":null}),
                ticket: None,
                success: None,
                error: None,
            });
            Self {
                host,
                ctx,
                policy,
                descriptor,
                path,
                lease,
                request: 2,
            }
        }
        fn tick(&mut self, now: u64) {
            let frame = self.host.provider.frame();
            let capture = vec![0.125; 48 * self.host.provider.topology().capture_channels];
            let mut playback = vec![0.; 48 * self.host.provider.topology().playback_channels];
            self.host
                .process_source(now, 9, frame, &capture, &mut playback)
                .unwrap();
        }
        fn select(&mut self, source: crate::brain_control::MonitorSource, now: u64) {
            let request = crate::brain_control::Request {
                contract: "GP15-brain".into(),
                version: 1,
                show_id: "11111111-1111-4111-8111-111111111111".into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(self.ctx.writer().into()),
                lease: Some(self.lease),
                request_id: Some(Counter(self.request)),
                expected_revision: Some(self.host.provider.brain_snapshot().revision),
                command: crate::brain_control::Command::MonitorSet {
                    source,
                    gain_cdb: 0,
                    mute: false,
                    dim: false,
                    armed: false,
                },
            };
            self.request += 1;
            let pending = self
                .host
                .provider
                .brain_request(request.clone(), now, true, None)
                .unwrap();
            assert_eq!(pending.state, "pending", "{pending:?}");
            assert!(pending.reason.is_none(), "{pending:?}");
            let boundary = pending
                .applied_frame
                .expect("declared effective boundary")
                .0;
            let current = self.host.provider.frame();
            assert!(boundary >= current && boundary <= current + 48 * 4);
            while self.host.provider.frame() <= boundary {
                self.tick(now);
            }
            let final_reply = self
                .host
                .provider
                .brain_request(request, now, true, None)
                .unwrap();
            assert_eq!(final_reply.state, "final", "{final_reply:?}");
            assert!(final_reply.reason.is_none(), "{final_reply:?}");
            assert_eq!(final_reply.applied_frame, Some(Counter(boundary)));
            assert_eq!(self.host.provider.brain_snapshot().source, source);
        }
        fn transition(&mut self) -> Vec<u8> {
            self.tick(0);
            let packet = BrainPacket {
                descriptor: self.descriptor.clone(),
                role: BrainMediaRole::Monitor,
                frame: 0,
                samples: vec![0.125; 96],
            }
            .encode()
            .unwrap();
            self.host.brain.as_mut().unwrap().outgoing[0] = packet.clone();
            self.select(crate::brain_control::MonitorSource::Main, 1);
            self.tick(1);
            assert!(
                self.host
                    .brain
                    .as_ref()
                    .unwrap()
                    .monitor_retirement
                    .is_some()
            );
            packet
        }
    }

    #[test]
    fn retirement_preserves_indices_and_runs_real_bridge_until_new_identity() {
        let mut f = Fixture::new("continuous");
        let old = f.transition();
        let epochs = BridgeEpochs {
            source: 9,
            destination: 77,
            route: f.descriptor.selection_generation,
        };
        let mut bridge = Bridge::prepare(BridgeConfig::voice(2), epochs).unwrap();
        let mut expected_frame = 0;
        let mut output_frame = 0;
        for now in 2..102 {
            while let Some(bytes) = f.host.poll_brain_media(&f.ctx).unwrap() {
                if expected_frame == 0 {
                    assert_eq!(&bytes[..80], &old[..80]);
                    assert_eq!(bytes.len(), old.len());
                }
                let packet =
                    BrainPacket::decode(&bytes, &f.descriptor, BrainMediaRole::Monitor).unwrap();
                assert_eq!(packet.frame, expected_frame);
                assert!(packet.samples.iter().all(|s| *s == 0. && s.is_finite()));
                bridge.push(epochs, packet.frame, &packet.samples).unwrap();
                expected_frame += 48;
            }
            if bridge.status().ready && !bridge.status().armed {
                bridge.arm(epochs).unwrap();
            }
            if bridge.status().armed {
                let mut samples = [0.; 96];
                bridge.render(epochs, output_frame, &mut samples).unwrap();
                assert!(samples.iter().all(|s| *s == 0.));
                output_frame += 48;
            }
            f.tick(now);
        }
        assert!(output_frame > 3000);
        assert_eq!(bridge.status().underruns, 0);
        assert_eq!(bridge.status().overflows, 0);
        let mut next = f.descriptor.clone();
        next.generation += 1;
        next.selection_generation = f.host.provider.brain_snapshot().selection_generation.0;
        f.host.negotiate_brain(&f.ctx,serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":f.ctx.writer(),"descriptor":next})).unwrap();
        assert!(f.host.brain.as_ref().unwrap().monitor_retirement.is_none());
        assert!(f.host.poll_brain_media(&f.ctx).unwrap().is_none());
        f.tick(102);
        let bytes = f.host.poll_brain_media(&f.ctx).unwrap().unwrap();
        BrainPacket::decode(&bytes, &next, BrainMediaRole::Monitor).unwrap();
        assert!(BrainPacket::decode(&bytes, &f.descriptor, BrainMediaRole::Monitor).is_err());
    }

    #[test]
    fn retirement_payload_rewrite_rejects_wrong_role_and_identity_without_mutation() {
        let f = Fixture::new("packet");
        let mut d = f.descriptor.clone();
        d.talkback = true;
        for role in [BrainMediaRole::Talkback, BrainMediaRole::Monitor] {
            let mut bytes = BrainPacket {
                descriptor: d.clone(),
                role,
                frame: 48,
                samples: vec![0.5; 48 * role.channels()],
            }
            .encode()
            .unwrap();
            if role == BrainMediaRole::Monitor {
                bytes[15] ^= 1;
            }
            let old = bytes.clone();
            assert!(BrainPacket::silence_monitor(&mut bytes, &d).is_err());
            assert_eq!(bytes, old);
        }
        let mut bytes = BrainPacket {
            descriptor: d.clone(),
            role: BrainMediaRole::Monitor,
            frame: 48,
            samples: vec![0.5; 96],
        }
        .encode()
        .unwrap();
        let old = bytes.clone();
        BrainPacket::silence_monitor(&mut bytes, &d).unwrap();
        assert_eq!(&bytes[..80], &old[..80]);
        assert_eq!(bytes.len(), old.len());
        assert!(
            BrainPacket::decode(&bytes, &d, BrainMediaRole::Monitor)
                .unwrap()
                .samples
                .iter()
                .all(|v| *v == 0. && v.is_finite())
        );
    }

    #[test]
    fn retirement_deadlines_do_not_extend_and_identity_loss_closes() {
        for case in 0..11 {
            let mut f = Fixture::new(&format!("expiry-{case}"));
            f.transition();
            let first = f.host.brain.as_ref().unwrap().monitor_retirement.unwrap();
            let queued = f.host.brain.as_ref().unwrap().outgoing.clone();
            let mut stale = f.descriptor.clone();
            stale.generation += 1;
            assert!(f.host.negotiate_brain(&f.ctx,serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":f.ctx.writer(),"descriptor":stale})).is_err());
            assert_eq!(f.host.brain.as_ref().unwrap().outgoing, queued);
            assert_eq!(
                f.host
                    .brain
                    .as_ref()
                    .unwrap()
                    .monitor_retirement
                    .unwrap()
                    .first_frame,
                first.first_frame
            );

            // An exact retransmission is idempotent, never a new retirement.
            f.host.negotiate_brain(&f.ctx,serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":f.ctx.writer(),"descriptor":f.descriptor})).unwrap();
            f.select(crate::brain_control::MonitorSource::None, 2);
            f.tick(2);
            assert_eq!(
                f.host
                    .brain
                    .as_ref()
                    .unwrap()
                    .monitor_retirement
                    .unwrap()
                    .first_ms,
                first.first_ms
            );
            let now = match case {
                0 => {
                    f.host.device_observed_ms = Some(251);
                    251
                }
                1 => {
                    let deadline = first.first_frame + 12_000;
                    while f.host.provider.frame() < deadline {
                        f.host.brain.as_mut().unwrap().outgoing.clear();
                        f.tick(2);
                        assert!(f.host.brain.is_some());
                    }
                    assert_eq!(f.host.provider.frame(), deadline);
                    2
                }
                2 => {
                    f.host.device_owner = None;
                    3
                }
                3 => {
                    f.host.device_observation.as_mut().unwrap().brain_epoch += 1;
                    3
                }
                4 => {
                    f.host.device_observation.as_mut().unwrap().status["fault"] =
                        serde_json::json!("xrun");
                    3
                }
                5 => {
                    f.host.device_observed_ms = None;
                    3
                }
                6 => {
                    f.host.device_observation.as_mut().unwrap().brain_map += 1;
                    3
                }
                7 => {
                    f.host.disconnect(&f.ctx);
                    3
                }
                8 => {
                    f.policy.replace(vec![]).unwrap();
                    3
                }
                9 => {
                    f.host.brain.as_mut().unwrap().descriptor.stagebox_map += 1;
                    3
                }
                _ => {
                    f.host.brain.as_mut().unwrap().descriptor.stagebox_epoch += 1;
                    3
                }
            };
            f.tick(now);
            assert!(f.host.brain.is_none(), "case {case}");
            assert_eq!(f.host.media_stats.dropped_monitor_packets, 0);
        }
    }
}

#[cfg(test)]
mod atomic_admission_tests {
    use super::*;
    use crate::{paired_readback as paired, topology::EngineTopology};
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn atomic_pair_capacity_includes_actual_remote_wrapper_at_exact_boundary() {
        for session in [1, u64::MAX] {
            let value = serde_json::to_vec(&Response::Reply {
                session: Counter(session),
                payload: Value::Null,
            })
            .unwrap();
            let overhead = value.len() - 4;
            let limit = crate::snapshot_pages::ASSEMBLY_BYTES - overhead;
            assert!(paired_transport_capacity(limit, session).is_ok());
            assert!(paired_transport_capacity(limit + 1, session).is_err());
            assert!(paired_transport_capacity(usize::MAX, session).is_err());
            let payload: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/atomic-control-v1/read-snapshot-48.json"
            ))
            .unwrap();
            let payload_bytes = serde_json::to_vec(&payload).unwrap().len();
            let wrapped = serde_json::to_vec(&Response::Reply {
                session: Counter(session),
                payload,
            })
            .unwrap();
            assert_eq!(wrapped.len(), payload_bytes + overhead);
        }
    }
    #[test]
    fn atomic_failed_pair_construction_changes_neither_read_latch() {
        let dir = std::env::temp_dir().join(format!("atomic-admission-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        let policy = PolicyStore::new(vec![Peer {
            id: "observer".into(),
            certificate_sha256: fingerprint(b"admission"),
            permissions: Default::default(),
        }])
        .unwrap();
        let context = policy.authenticate(b"admission", 101).unwrap();
        let local = LocalAudio::bind_configured(
            &dir,
            "audio",
            show,
            Counter(1),
            EngineTopology::software(48, 5, 0).unwrap(),
        )
        .unwrap();
        let mut host = HostAuthority::new(local, 1).unwrap();
        let request = paired::Request {
            contract: paired::CONTRACT.into(),
            version: 1,
            kind: "readback".into(),
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(1),
            authenticated_session: Counter(context.session()),
            writer: context.writer().into(),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            query_id: Counter(1),
        };
        let (raw, brain) = host.provider.paired_readback(&request, 10, 1).unwrap();
        let mut malformed = paired::Reply::new(request.clone(), Ok((raw.clone(), brain.clone())));
        malformed.brain.as_mut().unwrap().revision = Counter(u64::MAX);
        let refused = host
            .finish_paired_readback(&context, malformed, 10)
            .unwrap();
        assert_eq!(refused["reason"], "capacity");
        assert!(!host.reads.contains_key(&context.session()));
        assert!(!host.brain_reads.contains_key(&context.session()));
        assert!(!host.paired_queries.contains_key(&context.session()));
        let valid = paired::Reply::new(request.clone(), Ok((raw, brain)));
        assert_eq!(
            host.dispatch(&context, serde_json::to_value(&request).unwrap(), 10)
                .unwrap()
                .unwrap()["state"],
            "snapshot"
        );
        assert!(host.reads[&context.session()].audio_ready());
        assert_eq!(host.brain_reads[&context.session()], 10);
        let mut malformed = valid;
        malformed.raw.as_mut().unwrap().frame = Counter(96);
        assert_eq!(
            host.finish_paired_readback(&context, malformed, 99)
                .unwrap()["reason"],
            "capacity"
        );
        assert_eq!(host.brain_reads[&context.session()], 10);
        assert_eq!(host.paired_queries[&context.session()], 1);
        // Authenticated wrong attachment is checked before duplicate nonce admission.
        let mut wrong = request.clone();
        wrong.epoch = Counter(2);
        assert_eq!(
            host.dispatch(&context, serde_json::to_value(wrong).unwrap(), 11)
                .unwrap()
                .unwrap()["reason"],
            "identity"
        );
        for session in 102..102 + MAX_CONNECTIONS as u64 - 1 {
            let ctx = policy.authenticate(b"admission", session).unwrap();
            let mut r = request.clone();
            r.authenticated_session = Counter(session);
            r.writer = ctx.writer().into();
            assert_eq!(
                host.dispatch(&ctx, serde_json::to_value(r).unwrap(), 11)
                    .unwrap()
                    .unwrap()["state"],
                "snapshot"
            );
        }
        let ctx = policy.authenticate(b"admission", 200).unwrap();
        let mut r = request;
        r.authenticated_session = Counter(200);
        r.writer = ctx.writer().into();
        assert_eq!(
            host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 11)
                .unwrap()
                .unwrap()["reason"],
            "capacity"
        );
        host.disconnect(&context);
        assert_eq!(
            host.dispatch(&ctx, serde_json::to_value(r).unwrap(), 11)
                .unwrap()
                .unwrap()["state"],
            "snapshot"
        );
        drop(host);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[cfg(test)]
mod meter_reads_tests {
    use super::*;
    #[test]
    fn meter_reads_do_not_touch_paired_processing_leases_or_held_admission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("gp-meter-authority-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        {
            let provider = LocalAudio::bind(&dir, "meter.sock", show, Counter(1)).unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let policy = PolicyStore::new(vec![Peer {
                id: "reader".into(),
                certificate_sha256: fingerprint(b"meter"),
                permissions: Default::default(),
            }])
            .unwrap();
            let ctx = policy.authenticate(b"meter", 1).unwrap();
            let mut r = crate::meter_wire::Request::new(show, 1, 1, 1);
            let result = host
                .dispatch(&ctx, serde_json::to_value(&r).unwrap(), 0)
                .unwrap()
                .unwrap();
            assert_eq!(result["reason"], "missing_window");
            assert!(host.reads.is_empty());
            assert!(host.brain_reads.is_empty());
            assert!(host.structural_reads.is_empty());
            assert!(host.paired_queries.is_empty());
            assert_eq!(host.provider.engine_mut().revision(), Counter(0));
            r.query_id = Counter(2);
            assert!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 39)
                    .is_err()
            );
            r.version = 2;
            assert_eq!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 40)
                    .unwrap()
                    .unwrap()["reason"],
                "unsupported"
            );
            policy.replace(vec![]).unwrap();
            assert!(
                host.dispatch(&ctx, serde_json::to_value(&r).unwrap(), 80)
                    .is_err()
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
