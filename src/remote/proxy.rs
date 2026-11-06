//! Bounded SPSC handoff between the network worker and the existing authority
//! owner. `AuthorityMailbox::service` runs outside render, before its next source
//! block. Neither network activity nor a stalled client can lock the callback.
use super::diagnostics::{self, Handle, Stage, Token};
use super::*;
use crate::transport::MediaDatagram;
use rtrb::{Consumer, Producer, RingBuffer};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

enum Job {
    Command(Value, Token),
    Negotiate(MediaDescriptor, Token),
}
enum Event {
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    BrainHandoff(SourceBrainMedia),
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    WetInput(SourceWetInput),
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    MediaOutput(SourceEgressOutput),
    Reply(Result<Value>, Token),
    Negotiated(Result<MediaDescriptor>, Token),
}
struct Shared {
    disconnected: Arc<AtomicBool>,
    retiring: AtomicBool,
    source_frame: AtomicU64,
    media_in_dropped: AtomicU64,
    brain_in_dropped: AtomicU64,
    media_out_dropped: AtomicU64,
    brain_out_dropped: AtomicU64,
    totals: MediaQueueDropTotals,
}

pub struct AuthorityProxy {
    trace: Handle,
    token: Token,
    context: AuthenticatedContext,
    identity: EngineIdentity,
    shared: Arc<Shared>,
    commands: Producer<Job>,
    events: Consumer<Event>,
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    media_output: Option<SourceEgressOutput>,
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    wet_input: Option<SourceWetInput>,
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    brain_handoff: Option<SourceBrainMedia>,
    media_in: Producer<MediaDatagram>,
    media_out: Consumer<Vec<u8>>,
    brain_in: Producer<MediaDatagram>,
    brain_out: Consumer<Vec<u8>>,
    replies: std::collections::VecDeque<(Result<Value>, Token)>,
    negotiations: std::collections::VecDeque<(Result<MediaDescriptor>, Token)>,
    refusals: std::collections::VecDeque<(String, Token)>,
}
pub struct AuthorityMailbox {
    trace: Handle,
    context: AuthenticatedContext,
    identity: EngineIdentity,
    shared: Arc<Shared>,
    commands: Consumer<Job>,
    events: Producer<Event>,
    media_in: Consumer<MediaDatagram>,
    media_out: Producer<Vec<u8>>,
    brain_in: Consumer<MediaDatagram>,
    brain_out: Producer<Vec<u8>>,
    retired: bool,
}
/// Per authenticated connection: 16 command slots, 128 completion slots and 256
/// independent media slots in each direction. Construction/all destruction is
/// outside render. Queue exhaustion retires control; media exhaustion drops wet.
pub fn authority_channel(
    context: &AuthenticatedContext,
    identity: EngineIdentity,
) -> Result<(AuthorityProxy, AuthorityMailbox)> {
    authority_channel_counted(context, identity, MediaQueueDropTotals::default())
}

/// All sessions in one runner share this ledger, including retired connections.
pub fn authority_channel_counted(
    context: &AuthenticatedContext,
    identity: EngineIdentity,
    totals: MediaQueueDropTotals,
) -> Result<(AuthorityProxy, AuthorityMailbox)> {
    identity.validate()?;
    let shared = Arc::new(Shared {
        disconnected: context.disconnect_signal(),
        retiring: AtomicBool::new(false),
        source_frame: AtomicU64::new(u64::MAX),
        media_in_dropped: AtomicU64::new(0),
        brain_in_dropped: AtomicU64::new(0),
        media_out_dropped: AtomicU64::new(0),
        brain_out_dropped: AtomicU64::new(0),
        totals,
    });
    let (commands_tx, commands_rx) = RingBuffer::new(16);
    let (events_tx, events_rx) = RingBuffer::new(128);
    let (media_in_tx, media_in_rx) = RingBuffer::new(256);
    let (media_out_tx, media_out_rx) = RingBuffer::new(256);
    let (brain_in_tx, brain_in_rx) = RingBuffer::new(64);
    let (brain_out_tx, brain_out_rx) = RingBuffer::new(64);
    Ok((
        AuthorityProxy {
            trace: None,
            token: Token::default(),
            context: context.clone(),
            identity: identity.clone(),
            shared: shared.clone(),
            commands: commands_tx,
            events: events_rx,
            #[cfg(all(target_os = "linux", feature = "hardware-host"))]
            media_output: None,
            #[cfg(all(target_os = "linux", feature = "hardware-host"))]
            wet_input: None,
            #[cfg(all(target_os = "linux", feature = "hardware-host"))]
            brain_handoff: None,
            media_in: media_in_tx,
            media_out: media_out_rx,
            brain_in: brain_in_tx,
            brain_out: brain_out_rx,
            replies: std::collections::VecDeque::with_capacity(128),
            negotiations: std::collections::VecDeque::with_capacity(1),
            refusals: std::collections::VecDeque::with_capacity(32),
        },
        AuthorityMailbox {
            trace: None,
            context: context.clone(),
            identity,
            shared,
            commands: commands_rx,
            events: events_tx,
            media_in: media_in_rx,
            media_out: media_out_tx,
            brain_in: brain_in_rx,
            brain_out: brain_out_tx,
            retired: false,
        },
    ))
}
/// Lifetime drops at the proxy boundary, independent of opt-in diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct MediaQueueDrops {
    pub media_in: u64,
    pub brain_in: u64,
    pub media_out: u64,
    pub brain_out: u64,
}
/// Fixed-size process-lifetime counters. No per-session retained records and no
/// final-sample race with a retired mailbox: producers add directly to the ledger.
#[derive(Clone, Default)]
pub struct MediaQueueDropTotals(Arc<[AtomicU64; 4]>);
impl MediaQueueDropTotals {
    pub fn snapshot(&self) -> MediaQueueDrops {
        MediaQueueDrops {
            media_in: self.0[0].load(Ordering::Relaxed),
            brain_in: self.0[1].load(Ordering::Relaxed),
            media_out: self.0[2].load(Ordering::Relaxed),
            brain_out: self.0[3].load(Ordering::Relaxed),
        }
    }
}
impl Shared {
    fn media_queue_drops(&self) -> MediaQueueDrops {
        MediaQueueDrops {
            media_in: self.media_in_dropped.load(Ordering::Relaxed),
            brain_in: self.brain_in_dropped.load(Ordering::Relaxed),
            media_out: self.media_out_dropped.load(Ordering::Relaxed),
            brain_out: self.brain_out_dropped.load(Ordering::Relaxed),
        }
    }
}
impl AuthorityProxy {
    pub fn media_queue_drops(&self) -> MediaQueueDrops {
        self.shared.media_queue_drops()
    }

    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(crate) fn set_diagnostics(&mut self, mailbox: &mut AuthorityMailbox, trace: Handle) {
        self.trace = trace.clone();
        mailbox.trace = trace;
    }
    fn check(&self, context: &AuthenticatedContext) -> Result<()> {
        if context != &self.context
            || self.shared.disconnected.load(Ordering::Acquire)
            || self.shared.retiring.load(Ordering::Acquire)
        {
            return Err("authority session retired".into());
        }
        Ok(())
    }
    fn drain(&mut self) -> Result<()> {
        if self.shared.disconnected.load(Ordering::Acquire) {
            return Err("authority session retired".into());
        }
        for _ in 0..128 {
            let Ok(event) = self.events.pop() else {
                break;
            };
            match event {
                #[cfg(all(target_os = "linux", feature = "hardware-host"))]
                Event::BrainHandoff(handoff) => self.brain_handoff = Some(handoff),
                #[cfg(all(target_os = "linux", feature = "hardware-host"))]
                Event::WetInput(input) => self.wet_input = Some(input),
                #[cfg(all(target_os = "linux", feature = "hardware-host"))]
                Event::MediaOutput(output) => self.media_output = Some(output),
                Event::Reply(Err(reason), token) if self.refusals.len() < 32 => {
                    self.refusals.push_back((reason, token))
                }
                Event::Reply(Ok(reply), token) if self.replies.len() < 128 => {
                    self.replies.push_back((Ok(reply), token))
                }
                Event::Negotiated(reply, token) if self.negotiations.is_empty() => {
                    self.negotiations.push_back((reply, token))
                }
                _ => {
                    self.shared.disconnected.store(true, Ordering::Release);
                    return Err("authority proxy result capacity".into());
                }
            }
        }
        Ok(())
    }
}
impl Drop for AuthorityProxy {
    fn drop(&mut self) {
        self.shared.disconnected.store(true, Ordering::Release);
    }
}
impl AuthorityEndpoint for AuthorityProxy {
    fn diagnostic_trace(&self) -> Handle {
        self.trace.clone()
    }
    fn diagnostic_request(&mut self, token: Token) {
        self.token = token;
    }
    fn diagnostic_reply(&mut self) -> Token {
        self.token
    }
    fn retiring(&self) -> bool {
        self.shared.retiring.load(Ordering::Acquire)
    }
    fn identity(&self) -> EngineIdentity {
        self.identity.clone()
    }
    fn dispatch(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        _now_ms: u64,
    ) -> Result<Option<Value>> {
        self.check(context)?;
        self.commands
            .push(Job::Command(payload, self.token))
            .map_err(|_| "authority command queue full")?;
        diagnostics::record(&self.trace, self.token, Stage::ProxyEnqueued, 0, 0);
        Ok(None)
    }
    fn negotiate(
        &mut self,
        context: &AuthenticatedContext,
        descriptor: &MediaDescriptor,
    ) -> Result<bool> {
        self.check(context)?;
        self.commands
            .push(Job::Negotiate(descriptor.clone(), self.token))
            .map_err(|_| "authority command queue full")?;
        Ok(false)
    }
    fn poll_refusal(&mut self) -> Result<Option<String>> {
        self.drain()?;
        Ok(self.refusals.pop_front().map(|(reply, token)| {
            self.token = token;
            diagnostics::record(&self.trace, token, Stage::EventDequeued, 0, 0);
            reply
        }))
    }
    fn poll_negotiation(&mut self) -> Result<Option<MediaDescriptor>> {
        self.drain()?;
        self.negotiations
            .pop_front()
            .map(|(reply, token)| {
                self.token = token;
                diagnostics::record(&self.trace, token, Stage::EventDequeued, 0, 0);
                reply
            })
            .transpose()
    }
    fn media(
        &mut self,
        context: &AuthenticatedContext,
        packet: crate::transport::Packet<'_>,
        _now_ms: u64,
    ) -> Result<()> {
        self.check(context)?;
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if let Some(input) = &mut self.wet_input {
            return input.submit(context, packet);
        }
        // Drop on bounded media pressure. Missing wet follows the host's fade;
        // it never stalls source progress or consumes REC capacity.
        if self
            .media_in
            .push(MediaDatagram::copy_from(packet.bytes()).map_err(|_| "media packet capacity")?)
            .is_err()
        {
            self.shared.media_in_dropped.fetch_add(1, Ordering::Relaxed);
            self.shared.totals.0[0].fetch_add(1, Ordering::Relaxed);
            diagnostics::media_record(
                &self.trace,
                Stage::MediaQueueFull,
                packet,
                self.source_frame().unwrap_or(0),
            );
        }
        Ok(())
    }
    fn brain_media(
        &mut self,
        context: &AuthenticatedContext,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<()> {
        self.check(context)?;
        self.drain()?;
        #[cfg(not(all(target_os = "linux", feature = "hardware-host")))]
        let _ = now_ms;
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if let Some(handoff) = &mut self.brain_handoff {
            return handoff.submit(context, bytes, now_ms);
        }
        context.require(&Permission::TalkbackDestinations)?;
        if bytes.len() > 464 {
            return Err("Brain media capacity".into());
        }
        if self
            .brain_in
            .push(MediaDatagram::copy_from(bytes).map_err(|_| "Brain media capacity")?)
            .is_err()
        {
            self.shared.brain_in_dropped.fetch_add(1, Ordering::Relaxed);
            self.shared.totals.0[1].fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
    fn poll_brain_media(&mut self, context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        // A map replacement can retire the mailbox while the network task is
        // draining its final control reply. Do not abort that writer merely
        // because this same tick reaches its media drain afterwards.
        if context == &self.context && self.shared.retiring.load(Ordering::Acquire) {
            return Ok(None);
        }
        self.check(context)?;
        self.drain()?;
        if let Ok(bytes) = self.brain_out.pop() {
            return Ok(Some(bytes));
        }
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if let Some(handoff) = &mut self.brain_handoff {
            return handoff.poll(context);
        }
        Ok(None)
    }
    fn disconnect(&mut self, _context: &AuthenticatedContext) {
        self.shared.disconnected.store(true, Ordering::Release);
    }
    fn source_frame(&self) -> Option<u64> {
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if let Some(frame) = self
            .wet_input
            .as_ref()
            .and_then(SourceWetInput::source_frame)
        {
            return Some(frame);
        }
        match self.shared.source_frame.load(Ordering::Acquire) {
            u64::MAX => None,
            frame => Some(frame),
        }
    }
    fn poll_reply(
        &mut self,
        _context: &AuthenticatedContext,
        _now_ms: u64,
    ) -> Result<Option<Value>> {
        self.drain()?;
        self.replies
            .pop_front()
            .map(|(reply, token)| {
                self.token = token;
                diagnostics::record(&self.trace, token, Stage::EventDequeued, 0, 0);
                reply
            })
            .transpose()
    }
    fn poll_media(&mut self, _context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        if self.shared.disconnected.load(Ordering::Acquire)
            || self.shared.retiring.load(Ordering::Acquire)
        {
            return Ok(None);
        }
        // An event-lane stall can delay endpoint transfer after compatibility
        // packets were queued. Drain that earlier prefix before the new consumer.
        if let Ok(bytes) = self.media_out.pop() {
            return Ok(Some(bytes));
        }
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if let Some(output) = &mut self.media_output {
            return Ok(output.poll());
        }
        Ok(None)
    }
}
impl AuthorityMailbox {
    pub fn media_queue_drops(&self) -> MediaQueueDrops {
        self.shared.media_queue_drops()
    }
    pub fn retired(&self) -> bool {
        self.retired
    }
    fn retire<A: AuthorityEndpoint>(&mut self, authority: &mut A) {
        self.shared.disconnected.store(true, Ordering::Release);
        if !self.retired {
            authority.disconnect(&self.context);
            self.retired = true;
        }
        while self.commands.pop().is_ok() {}
        while self.media_in.pop().is_ok() {}
        while self.brain_in.pop().is_ok() {}
    }
    /// Owner must call this outside render before processing its next block,
    /// including after a network worker exits. Retirement cannot be lost behind
    /// a full queue. The owner's monotonic clock is used for all actual leases.
    pub fn service<A: AuthorityEndpoint>(&mut self, authority: &mut A, now_ms: u64) -> Result<()> {
        if self.retired {
            return Ok(());
        }
        if self.shared.disconnected.load(Ordering::Acquire) || self.context.check_current().is_err()
        {
            self.retire(authority);
            return Ok(());
        }
        if authority.identity() != self.identity {
            // Drain the applied structural result before invalidating its map.
            for _ in 0..128 {
                let Some(reply) = authority.poll_reply(&self.context, now_ms)? else {
                    break;
                };
                let token = if self.trace.is_some() {
                    Token::payload(self.context.session, 0, &reply)
                } else {
                    Token::default()
                };
                if self.events.push(Event::Reply(Ok(reply), token)).is_err() {
                    self.retire(authority);
                    return Err("generation completion capacity".into());
                }
            }
            authority.disconnect(&self.context);
            self.retired = true;
            while self.commands.pop().is_ok() {}
            while self.media_in.pop().is_ok() {}
            while self.brain_in.pop().is_ok() {}
            self.shared.retiring.store(true, Ordering::Release);
            return Ok(());
        }
        self.shared.source_frame.store(
            authority.source_frame().unwrap_or(u64::MAX),
            Ordering::Release,
        );
        let result = self.service_inner(authority, now_ms);
        if result.is_err() {
            self.retire(authority);
        }
        result
    }
    fn service_inner<A: AuthorityEndpoint>(
        &mut self,
        authority: &mut A,
        now_ms: u64,
    ) -> Result<()> {
        for _ in 0..16 {
            let Ok(job) = self.commands.pop() else {
                break;
            };
            // Observe disconnect between queued intents as well as at entry.
            if self.shared.disconnected.load(Ordering::Acquire) {
                self.retire(authority);
                return Ok(());
            }
            let token = match &job {
                Job::Command(_, token) | Job::Negotiate(_, token) => *token,
            };
            diagnostics::record(&self.trace, token, Stage::DispatchBegin, 0, 0);
            let event = match job {
                Job::Command(payload, token) => {
                    match authority.dispatch(&self.context, payload, now_ms) {
                        Ok(Some(reply)) => Event::Reply(Ok(reply), token),
                        Err(reason) => Event::Reply(Err(reason), token),
                        Ok(None) => {
                            // The bounded owner retains the pending operation;
                            // poll_reply below delivers its eventual completion.
                            diagnostics::record(&self.trace, token, Stage::DispatchEnd, 0, 0);
                            continue;
                        }
                    }
                }
                Job::Negotiate(descriptor, token) => Event::Negotiated(
                    authority
                        .negotiate(&self.context, &descriptor)
                        .and_then(|ready| {
                            if ready {
                                Ok(descriptor)
                            } else {
                                Err("nested deferred negotiation unsupported".into())
                            }
                        }),
                    token,
                ),
            };
            diagnostics::record(&self.trace, token, Stage::DispatchEnd, 0, 0);
            self.events
                .push(event)
                .map_err(|_| "authority result queue full")?;
            diagnostics::record(&self.trace, token, Stage::EventEnqueued, 0, 0);
        }
        for _ in 0..32 {
            let Some(reply) = authority.poll_reply(&self.context, now_ms)? else {
                break;
            };
            let token = if self.trace.is_some() {
                Token::payload(self.context.session, 0, &reply)
            } else {
                Token::default()
            };
            self.events
                .push(Event::Reply(Ok(reply), token))
                .map_err(|_| "authority completion queue full")?;
        }
        for _ in 0..64 {
            let Ok(bytes) = self.media_in.pop() else {
                break;
            };
            let packet =
                crate::transport::Packet::parse(bytes.bytes()).map_err(|_| "host media packet")?;
            if packet.spec().session != self.context.session {
                return Err("host media session".into());
            }
            diagnostics::media_record(
                &self.trace,
                Stage::MediaAdmission,
                packet,
                authority.source_frame().unwrap_or(0),
            );
            if authority
                .source_frame()
                .is_some_and(|frame| packet.output_frame() <= frame)
            {
                diagnostics::media_record(
                    &self.trace,
                    Stage::MediaRefused,
                    packet,
                    authority.source_frame().unwrap_or(0),
                );
                continue;
            }
            authority.media(&self.context, packet, now_ms)?;
        }
        // Preserve the queued compatibility prefix before transferring ingress.
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if self.media_in.is_empty()
            && self.events.slots() > 0
            && let Some(input) = authority.take_wet_input(&self.context)
        {
            self.events
                .push(Event::WetInput(input))
                .map_err(|_| "wet input handoff capacity")?;
        }
        for _ in 0..64 {
            let Ok(bytes) = self.brain_in.pop() else {
                break;
            };
            let _ = authority.brain_media(&self.context, bytes.bytes(), now_ms);
        }
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if self.brain_in.is_empty()
            && self.events.slots() > 0
            && let Some(handoff) = authority.take_brain_handoff(&self.context)
        {
            self.events
                .push(Event::BrainHandoff(handoff))
                .map_err(|_| "Brain handoff capacity")?;
        }
        for _ in 0..64 {
            let Some(bytes) = authority.poll_brain_media(&self.context)? else {
                break;
            };
            if self.brain_out.push(bytes).is_err() {
                self.shared
                    .brain_out_dropped
                    .fetch_add(1, Ordering::Relaxed);
                self.shared.totals.0[3].fetch_add(1, Ordering::Relaxed);
            }
        }
        #[cfg(all(target_os = "linux", feature = "hardware-host"))]
        if self.events.slots() > 0
            && let Some(output) = authority.take_media_output(&self.context)
        {
            self.events
                .push(Event::MediaOutput(output))
                .map_err(|_| "media output handoff capacity")?;
        }
        for _ in 0..64 {
            let Some(bytes) = authority.poll_media(&self.context)? else {
                break;
            };
            if self.media_out.push(bytes).is_err() {
                self.shared
                    .media_out_dropped
                    .fetch_add(1, Ordering::Relaxed);
                self.shared.totals.0[2].fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use crate::show::Counter;
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    #[test]
    fn transferred_media_consumer_preserves_already_queued_prefix() {
        let policy = PolicyStore::new(vec![Peer {
            id: "media".into(),
            certificate_sha256: fingerprint(b"media"),
            permissions: [Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"media", 91).unwrap();
        let identity = EngineIdentity {
            source_epoch: Counter(1),
            capability_generation: Counter(1),
            map_generation: Counter(1),
        };
        let (mut proxy, mut mailbox) = authority_channel(&context, identity).unwrap();
        let mut descriptor = MediaDescriptor {
            session: Counter(91),
            source_epoch: Counter(1),
            capability_generation: Counter(1),
            map_generation: Counter(1),
            sample_rate: 48000,
            streams: grouped_streams(
                MediaRole::FxSend,
                &["foh-left".into(), "foh-right".into()],
                48,
                0,
                1,
                1100,
            )
            .unwrap(),
        };
        descriptor.streams.extend(
            grouped_streams(
                MediaRole::WetReturn,
                &["foh-left".into(), "foh-right".into()],
                48,
                768,
                2,
                1100,
            )
            .unwrap(),
        );
        let mut encoder =
            super::super::source_egress::SourceEgress::prepare(descriptor, 0, 96).unwrap();
        assert!(encoder.offer(48, &[], &[0.25; 96]));
        encoder.wait_encoded();
        mailbox.media_out.push(vec![1, 2, 3]).unwrap();
        mailbox
            .events
            .push(Event::MediaOutput(encoder.take_output().unwrap()))
            .ok()
            .unwrap();
        proxy.drain().unwrap();
        assert_eq!(proxy.poll_media(&context).unwrap(), Some(vec![1, 2, 3]));
        let bytes = proxy.poll_media(&context).unwrap().unwrap();
        assert_eq!(
            crate::transport::Packet::parse(&bytes)
                .unwrap()
                .source_frame(),
            48
        );
        assert!(proxy.poll_media(&context).unwrap().is_none());
        encoder.finish();
        assert!(proxy.poll_media(&context).unwrap().is_none());
    }

    #[test]
    fn terminal_disconnect_reaches_render_fence_without_mailbox_service() {
        let policy = PolicyStore::new(vec![Peer {
            id: "closure".into(),
            certificate_sha256: fingerprint(b"closure"),
            permissions: [Permission::Fx].into_iter().collect(),
        }])
        .unwrap();
        for drop_proxy in [false, true] {
            let context = policy.authenticate(b"closure", 92).unwrap();
            #[cfg(feature = "hardware-host")]
            let fence = context.audio_fence();
            let identity = EngineIdentity {
                source_epoch: Counter(1),
                capability_generation: Counter(1),
                map_generation: Counter(1),
            };
            let (mut proxy, mailbox) = authority_channel(&context, identity).unwrap();
            for _ in 0..16 {
                proxy.dispatch(&context, serde_json::json!({}), 0).unwrap();
            }
            assert!(proxy.dispatch(&context, serde_json::json!({}), 0).is_err());
            if drop_proxy {
                drop(proxy);
            } else {
                proxy.disconnect(&context);
            }
            // No mailbox service, authority callback or observation drain.
            assert!(!mailbox.retired());
            #[cfg(feature = "hardware-host")]
            crate::remote::assert_no_allocations(|| assert!(!fence.valid()));
            assert!(context.disconnect_signal().load(Ordering::Acquire));
            // Audio closure must not discard an already acknowledged reliable
            // result while the connection drains. Protocol retirement is separate.
            assert!(context.check_current().is_ok());
            assert!(policy.check(&context).is_ok());
            let fresh = policy.authenticate(b"closure", 93).unwrap();
            assert!(fresh.check_current().is_ok());
            #[cfg(feature = "hardware-host")]
            assert!(fresh.audio_fence().valid());
        }
    }
    #[test]
    fn ingress_overflow_is_counted_without_diagnostics_or_payload_destruction() {
        let policy = PolicyStore::new(vec![Peer {
            id: "test".into(),
            certificate_sha256: fingerprint(b"test"),
            permissions: [Permission::TalkbackDestinations].into_iter().collect(),
        }])
        .unwrap();
        let context = policy.authenticate(b"test", 91).unwrap();
        let identity = EngineIdentity {
            source_epoch: Counter(1),
            capability_generation: Counter(1),
            map_generation: Counter(1),
        };
        let (mut proxy, mut mailbox) = authority_channel(&context, identity).unwrap();
        let spec = crate::transport::StreamSpec {
            session: 91,
            stream: 1,
            first_channel: 0,
            channels: 1,
            frames: 48,
            encoding: crate::transport::Encoding::Float32,
            role: crate::transport::Role::WetReturn,
            delay_frames: 768,
        };
        let mut bytes = [0; crate::transport::MAX_DATAGRAM];
        let n = spec.encode_float(0, 0, &[0.; 48], &mut bytes).unwrap();
        let packet = crate::transport::Packet::parse(&bytes[..n]).unwrap();
        for _ in 0..257 {
            proxy.media(&context, packet, 0).unwrap();
        }
        for _ in 0..65 {
            proxy.brain_media(&context, &[0; 272], 0).unwrap();
        }
        assert_eq!(
            proxy.media_queue_drops(),
            MediaQueueDrops {
                media_in: 1,
                brain_in: 1,
                ..Default::default()
            }
        );
        assert_eq!(mailbox.media_queue_drops(), proxy.media_queue_drops());
        assert_eq!(mailbox.media_in.pop().unwrap().bytes(), packet.bytes());
        assert_eq!(mailbox.brain_in.pop().unwrap().bytes(), &[0; 272]);
        proxy.disconnect(&context);
        assert!(proxy.media(&context, packet, 0).is_err());
        assert!(proxy.brain_media(&context, &[0; 272], 0).is_err());
    }
    #[test]
    fn lifetime_totals_survive_retirement_without_final_sample_race() {
        let policy = PolicyStore::new(vec![Peer {
            id: "test".into(),
            certificate_sha256: fingerprint(b"test"),
            permissions: [Permission::TalkbackDestinations].into_iter().collect(),
        }])
        .unwrap();
        let totals = MediaQueueDropTotals::default();
        for session in 1..=2 {
            let context = policy.authenticate(b"test", session).unwrap();
            let identity = EngineIdentity {
                source_epoch: Counter(1),
                capability_generation: Counter(1),
                map_generation: Counter(1),
            };
            let (mut proxy, mailbox) =
                authority_channel_counted(&context, identity, totals.clone()).unwrap();
            for _ in 0..65 {
                proxy.brain_media(&context, &[0; 272], 0).unwrap();
            }
            // Network ingress can still be completing when the owner retires its
            // mailbox. The shared ledger outlives either endpoint.
            drop(mailbox);
            proxy.brain_media(&context, &[0; 272], 0).unwrap();
            assert_eq!(proxy.media_queue_drops().brain_in, 2);
            drop(proxy);
            assert_eq!(totals.snapshot().brain_in, session * 2);
        }
        assert_eq!(
            totals.snapshot(),
            MediaQueueDrops {
                brain_in: 4,
                ..Default::default()
            }
        );
    }
    #[test]
    fn outgoing_overflow_survives_session_destruction() {
        struct Source(EngineIdentity);
        impl AuthorityEndpoint for Source {
            fn identity(&self) -> EngineIdentity {
                self.0.clone()
            }
            fn dispatch(
                &mut self,
                _: &AuthenticatedContext,
                _: Value,
                _: u64,
            ) -> Result<Option<Value>> {
                Ok(None)
            }
            fn negotiate(&mut self, _: &AuthenticatedContext, _: &MediaDescriptor) -> Result<bool> {
                Ok(true)
            }
            fn media(
                &mut self,
                _: &AuthenticatedContext,
                _: crate::transport::Packet<'_>,
                _: u64,
            ) -> Result<()> {
                Ok(())
            }
            fn disconnect(&mut self, _: &AuthenticatedContext) {}
            fn poll_media(&mut self, _: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
                Ok(Some(vec![0]))
            }
            fn poll_brain_media(&mut self, _: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
                Ok(Some(vec![0]))
            }
        }
        let policy = PolicyStore::new(vec![Peer {
            id: "test".into(),
            certificate_sha256: fingerprint(b"test"),
            permissions: Default::default(),
        }])
        .unwrap();
        let totals = MediaQueueDropTotals::default();
        for session in 1..=2 {
            let context = policy.authenticate(b"test", session).unwrap();
            let identity = EngineIdentity {
                source_epoch: Counter(1),
                capability_generation: Counter(1),
                map_generation: Counter(1),
            };
            let (proxy, mut mailbox) =
                authority_channel_counted(&context, identity.clone(), totals.clone()).unwrap();
            let mut source = Source(identity);
            for now in 0..5 {
                mailbox.service(&mut source, now).unwrap();
            }
            assert_eq!(
                mailbox.media_queue_drops(),
                MediaQueueDrops {
                    media_out: 64,
                    brain_out: 256,
                    ..Default::default()
                }
            );
            drop(proxy);
            mailbox.service(&mut source, 5).unwrap();
            assert!(mailbox.retired());
            drop(mailbox);
        }
        assert_eq!(
            totals.snapshot(),
            MediaQueueDrops {
                media_out: 128,
                brain_out: 512,
                ..Default::default()
            }
        );
    }
    #[test]
    fn retirement_during_media_drain_preserves_structural_completion() {
        let policy = PolicyStore::new(vec![Peer {
            id: "test".into(),
            certificate_sha256: fingerprint(b"test"),
            permissions: std::collections::BTreeSet::new(),
        }])
        .unwrap();
        let context = policy.authenticate(b"test", 91).unwrap();
        let identity = EngineIdentity {
            source_epoch: Counter(1),
            capability_generation: Counter(1),
            map_generation: Counter(1),
        };
        let (mut proxy, mut mailbox) = authority_channel(&context, identity).unwrap();
        assert!(!proxy.retiring());
        let completion = serde_json::json!({"state":"final"});
        mailbox
            .events
            .push(Event::Reply(Ok(completion.clone()), Token::default()))
            .ok()
            .unwrap();
        mailbox.shared.retiring.store(true, Ordering::Release);
        assert_eq!(proxy.poll_reply(&context, 0).unwrap(), Some(completion));
        assert!(proxy.poll_brain_media(&context).unwrap().is_none());
        assert!(proxy.retiring());
        assert!(proxy.dispatch(&context, serde_json::json!({}), 0).is_err());
    }
    #[test]
    fn event_tokens_survive_interleaved_queries_refusal_and_deferred_final() {
        let policy = PolicyStore::new(vec![Peer {
            id: "test".into(),
            certificate_sha256: fingerprint(b"test"),
            permissions: std::collections::BTreeSet::new(),
        }])
        .unwrap();
        let context = policy.authenticate(b"test", 91).unwrap();
        let identity = EngineIdentity {
            source_epoch: Counter(1),
            capability_generation: Counter(1),
            map_generation: Counter(1),
        };
        let (mut proxy, mut mailbox) = authority_channel(&context, identity).unwrap();
        let trace = Some(diagnostics::Trace::test_trace());
        proxy.trace = trace.clone();
        mailbox.trace = trace;
        let first = Token {
            session: 91,
            ordinal: 4,
            request: 10,
            kind: 1,
        };
        let second = Token {
            session: 91,
            ordinal: 5,
            request: 11,
            kind: 2,
        };
        proxy.diagnostic_request(first);
        proxy.dispatch(&context, serde_json::json!({}), 0).unwrap();
        proxy.diagnostic_request(second);
        proxy.dispatch(&context, serde_json::json!({}), 0).unwrap();
        assert!(matches!(mailbox.commands.pop().unwrap(),Job::Command(_,token) if token==first));
        assert!(matches!(mailbox.commands.pop().unwrap(),Job::Command(_,token) if token==second));
        let final_reply = serde_json::json!({"context":{"request_id":"10"},"state":"final"});
        let completion = Token::payload(91, 0, &final_reply);
        mailbox
            .events
            .push(Event::Reply(Ok(serde_json::json!({"query":1})), first))
            .ok()
            .unwrap();
        mailbox
            .events
            .push(Event::Reply(Ok(final_reply), completion))
            .ok()
            .unwrap();
        mailbox
            .events
            .push(Event::Reply(Err("refused".into()), second))
            .ok()
            .unwrap();
        assert!(proxy.poll_reply(&context, 0).unwrap().is_some());
        assert_eq!(proxy.diagnostic_reply(), first);
        assert!(proxy.poll_reply(&context, 0).unwrap().is_some());
        assert_eq!(proxy.diagnostic_reply(), completion);
        assert_eq!(completion.ordinal, 0);
        assert_eq!(proxy.poll_refusal().unwrap().as_deref(), Some("refused"));
        assert_eq!(proxy.diagnostic_reply(), second);
    }
}
