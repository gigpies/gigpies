//! Bounded SPSC handoff between the network worker and the existing authority
//! owner. `AuthorityMailbox::service` runs outside render, before its next source
//! block. Neither network activity nor a stalled client can lock the callback.
use super::diagnostics::{self, Handle, Stage, Token};
use super::*;
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
    Reply(Result<Value>, Token),
    Negotiated(Result<MediaDescriptor>, Token),
}
struct Shared {
    disconnected: AtomicBool,
    retiring: AtomicBool,
    source_frame: AtomicU64,
}

pub struct AuthorityProxy {
    trace: Handle,
    token: Token,
    context: AuthenticatedContext,
    identity: EngineIdentity,
    shared: Arc<Shared>,
    commands: Producer<Job>,
    events: Consumer<Event>,
    media_in: Producer<Vec<u8>>,
    media_out: Consumer<Vec<u8>>,
    brain_in: Producer<Vec<u8>>,
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
    media_in: Consumer<Vec<u8>>,
    media_out: Producer<Vec<u8>>,
    brain_in: Consumer<Vec<u8>>,
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
    identity.validate()?;
    let shared = Arc::new(Shared {
        disconnected: AtomicBool::new(false),
        retiring: AtomicBool::new(false),
        source_frame: AtomicU64::new(u64::MAX),
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
impl AuthorityProxy {
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
        // Drop on bounded media pressure. Missing wet follows the host's fade;
        // it never stalls source progress or consumes REC capacity.
        if self.media_in.push(packet.bytes().to_vec()).is_err() {
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
        _now_ms: u64,
    ) -> Result<()> {
        self.check(context)?;
        context.require(&Permission::TalkbackDestinations)?;
        if bytes.len() > 464 {
            return Err("Brain media capacity".into());
        }
        let _ = self.brain_in.push(bytes.to_vec());
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
        Ok(self.brain_out.pop().ok())
    }
    fn disconnect(&mut self, _context: &AuthenticatedContext) {
        self.shared.disconnected.store(true, Ordering::Release);
    }
    fn source_frame(&self) -> Option<u64> {
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
        Ok(self.media_out.pop().ok())
    }
}
impl AuthorityMailbox {
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
                crate::transport::Packet::parse(&bytes).map_err(|_| "host media packet")?;
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
                .is_some_and(|frame| packet.output_frame() < frame)
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
        for _ in 0..64 {
            let Ok(bytes) = self.brain_in.pop() else {
                break;
            };
            let _ = authority.brain_media(&self.context, &bytes, now_ms);
        }
        for _ in 0..64 {
            let Some(bytes) = authority.poll_brain_media(&self.context)? else {
                break;
            };
            let _ = self.brain_out.push(bytes);
        }
        for _ in 0..64 {
            let Some(bytes) = authority.poll_media(&self.context)? else {
                break;
            };
            let _ = self.media_out.push(bytes);
        }
        Ok(())
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use crate::show::Counter;
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
