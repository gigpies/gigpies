use super::diagnostics::{self, Handle, Stage, Token};
use super::*;
use crate::show::Counter;
use serde_json::Value;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

/// Implement this on the actual processing authority, not on GPC1's scalar
/// prototype. Dispatch must check payload-derived permissions, writer, leases,
/// revision and retry history before scheduling a real sample boundary.
pub trait AuthorityEndpoint {
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    fn take_brain_handoff(&mut self, _context: &AuthenticatedContext) -> Option<SourceBrainMedia> {
        None
    }

    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    fn take_wet_input(&mut self, _context: &AuthenticatedContext) -> Option<SourceWetInput> {
        None
    }

    /// Optional sole media consumer, transferred after authenticated negotiation.
    /// Its network drain must not require controller mailbox service.
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    fn take_media_output(&mut self, _context: &AuthenticatedContext) -> Option<SourceEgressOutput> {
        None
    }

    fn diagnostic_trace(&self) -> Handle {
        None
    }
    fn diagnostic_request(&mut self, _token: Token) {}
    fn diagnostic_reply(&mut self) -> Token {
        Token::default()
    }
    fn retiring(&self) -> bool {
        false
    }
    fn identity(&self) -> EngineIdentity;
    fn dispatch(
        &mut self,
        context: &AuthenticatedContext,
        payload: Value,
        now_ms: u64,
    ) -> Result<Option<Value>>;
    /// Validate actual tap/channel IDs, FX routes and permissions against the
    /// admitted engine topology. Transport-only shape validation is insufficient.
    fn negotiate(
        &mut self,
        context: &AuthenticatedContext,
        descriptor: &MediaDescriptor,
    ) -> Result<bool>;
    fn poll_negotiation(&mut self) -> Result<Option<MediaDescriptor>> {
        Ok(None)
    }
    fn poll_refusal(&mut self) -> Result<Option<String>> {
        Ok(None)
    }
    /// Packet already authenticated, negotiated and duplicate checked. Host must
    /// admit it to its independent bounded worker queue, never advance its clock.
    fn media(
        &mut self,
        context: &AuthenticatedContext,
        packet: crate::transport::Packet<'_>,
        now_ms: u64,
    ) -> Result<()>;
    fn brain_media(
        &mut self,
        _context: &AuthenticatedContext,
        _bytes: &[u8],
        _now_ms: u64,
    ) -> Result<()> {
        Err("Brain media unsupported".into())
    }
    fn poll_brain_media(&mut self, _context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
    fn disconnect(&mut self, context: &AuthenticatedContext);
    fn source_frame(&self) -> Option<u64> {
        None
    }
    fn poll_reply(
        &mut self,
        _context: &AuthenticatedContext,
        _now_ms: u64,
    ) -> Result<Option<Value>> {
        Ok(None)
    }
    fn poll_media(&mut self, _context: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

pub struct RemoteServer {
    endpoint: quinn::Endpoint,
    policy: PolicyStore,
    slots: Arc<Semaphore>,
}
impl RemoteServer {
    pub fn bind(
        address: SocketAddr,
        credentials: &Credentials,
        policy: PolicyStore,
    ) -> Result<Self> {
        Ok(Self {
            endpoint: server_endpoint(address, credentials)?,
            policy,
            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        })
    }
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.endpoint.local_addr().map_err(|e| e.to_string())
    }
    pub fn close(&self) {
        self.endpoint.close(0u8.into(), b"operator stop");
    }
    pub async fn accept(&self) -> Result<ServerSession> {
        let incoming = self.endpoint.accept().await.ok_or("endpoint closed")?;
        let permit = match self.slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                incoming.refuse();
                return Err("remote connection capacity".into());
            }
        };
        validate_private_address(incoming.remote_address())?;
        let connection = tokio::time::timeout(Duration::from_millis(IO_TIMEOUT_MS), incoming)
            .await
            .map_err(|_| "TLS handshake timeout")?
            .map_err(|e| e.to_string())?;
        let context = match authenticated(&connection, &self.policy) {
            Ok(context) => context,
            Err(error) => {
                connection.close(1u8.into(), b"peer authorization refused");
                return Err(error);
            }
        };
        Ok(ServerSession {
            connection,
            context,
            policy: self.policy.clone(),
            _permit: permit,
        })
    }
}
fn session_id(connection: &quinn::Connection) -> Result<u64> {
    let mut bytes = [0; 8];
    connection
        .export_keying_material(&mut bytes, b"gigpies-remote-session-v1", b"")
        .map_err(|_| "TLS session export")?;
    let session = u64::from_be_bytes(bytes);
    if session == 0 {
        return Err("zero TLS session; reconnect".into());
    }
    Ok(session)
}
fn authenticated(
    connection: &quinn::Connection,
    policy: &PolicyStore,
) -> Result<AuthenticatedContext> {
    let identity = connection
        .peer_identity()
        .ok_or("TLS peer certificate required")?;
    let certificates = identity
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .map_err(|_| "TLS peer identity type")?;
    let certificate = certificates
        .first()
        .ok_or("TLS peer certificate required")?;
    policy.authenticate(certificate.as_ref(), session_id(connection)?)
}

pub struct ServerSession {
    connection: quinn::Connection,
    context: AuthenticatedContext,
    policy: PolicyStore,
    _permit: OwnedSemaphorePermit,
}
impl ServerSession {
    pub fn context(&self) -> &AuthenticatedContext {
        &self.context
    }
    /// All child tasks are bounded and aborted/joined before returning. A slow
    /// network client can close this session but cannot block render/REC.
    pub async fn run<A: AuthorityEndpoint>(self, authority: &mut A) -> Result<()> {
        let result = self.run_inner(authority).await;
        authority.disconnect(&self.context);
        self.connection
            .close(0u8.into(), b"session ended; reacquire required");
        result
    }
    async fn run_inner<A: AuthorityEndpoint>(&self, authority: &mut A) -> Result<()> {
        let identity = authority.identity();
        identity.validate()?;
        self.policy.check(&self.context)?;
        let (mut send, mut recv) = tokio::time::timeout(
            Duration::from_millis(IO_TIMEOUT_MS),
            self.connection.accept_bi(),
        )
        .await
        .map_err(|_| "control stream timeout")?
        .map_err(|e| e.to_string())?;
        match read_frame::<Request>(&mut recv).await? {
            Request::Open { contract, version } if contract == "GP-REMOTE" && version == 1 => (),
            _ => return Err("remote protocol version refused".into()),
        }
        let max_datagram = self
            .connection
            .max_datagram_size()
            .ok_or("QUIC datagrams required")?
            .min(crate::transport::MAX_DATAGRAM);
        write_frame(
            &mut send,
            &Response::Hello {
                contract: "GP-REMOTE".into(),
                version: 1,
                session: Counter(self.context.session),
                writer: self.context.writer.clone(),
                peer_id: self.context.peer_id.clone(),
                policy_generation: Counter(self.context.policy_generation),
                identity: identity.clone(),
                permissions: self.context.permissions.clone(),
                max_datagram: max_datagram as u32,
            },
        )
        .await?;
        let trace = authority.diagnostic_trace();
        let reader_trace = trace.clone();
        let writer_trace = trace.clone();
        let trace_session = self.context.session;
        let writer_connection = self.connection.clone();
        let (requests_tx, mut requests_rx) = mpsc::channel(8);
        let (replies_tx, mut replies_rx) = mpsc::channel::<(Response, Token)>(32);
        let reader_guard = trace.as_ref().map(|t| t.task());
        let writer_guard = trace.as_ref().map(|t| t.task());
        let reader = tokio::spawn(async move {
            let _guard = reader_guard;
            let mut ordinal = 0u64;
            loop {
                if reader_trace.is_some() {
                    ordinal += 1;
                }
                let mut token = Token {
                    session: trace_session,
                    ordinal,
                    ..Token::default()
                };
                let request =
                    super::wire::read_frame_diagnostic::<Request>(&mut recv, &reader_trace, token)
                        .await;
                if reader_trace.is_some()
                    && let Ok(Request::Command { payload, .. }) = &request
                {
                    token = Token::payload(trace_session, ordinal, payload);
                }
                let failed = request.is_err();
                if requests_tx.try_send((request, token)).is_err() || failed {
                    break;
                }
            }
        });
        let (writer_error_tx, mut writer_error_rx) = mpsc::channel(1);
        let mut writer = tokio::spawn(async move {
            let _guard = writer_guard;
            while let Some((reply, token)) = replies_rx.recv().await {
                diagnostics::record(&writer_trace, token, Stage::ReplyDequeued, 0, 0);
                diagnostics::response_identity(&writer_trace, token, &reply);
                diagnostics::quinn_stats(&writer_trace, token, &writer_connection, false);
                let result =
                    super::wire::write_response_diagnostic(&mut send, &reply, &writer_trace, token)
                        .await;
                diagnostics::quinn_stats(&writer_trace, token, &writer_connection, true);
                if let Err(error) = result {
                    let _ = writer_error_tx.try_send(error);
                    break;
                }
            }
            let _ = send.finish();
            let _ =
                tokio::time::timeout(Duration::from_millis(IO_TIMEOUT_MS), send.stopped()).await;
        });
        let _diagnostic_children = diagnostics::AbortChildren(
            trace
                .as_ref()
                .map(|_| [reader.abort_handle(), writer.abort_handle()]),
        );
        let result = async {
            let mut media = MediaRegistry::new();
            let mut pending_negotiation = None;
            let mut ticker = tokio::time::interval(Duration::from_millis(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                self.policy.check(&self.context)?;
                if authority.retiring() {
                    for _ in 0..128 {
                        let Some(payload) = authority.poll_reply(&self.context, monotonic_ms())? else { break; };
                        let token=authority.diagnostic_reply();
                        diagnostics::record(&trace,token,Stage::ReplyEnqueued,0,0);
                        tokio::time::timeout(Duration::from_millis(IO_TIMEOUT_MS), replies_tx.send((Response::Reply { session: Counter(self.context.session), payload },token)))
                            .await.map_err(|_| "generation completion timeout")?.map_err(|_| "generation completion writer")?;
                    }
                    return Ok(());
                }
                if authority.identity() != identity { return Err("engine epoch/capabilities/map changed; reconnect required".into()); }
                let now = monotonic_ms();
                tokio::select! {
                    request = requests_rx.recv() => {
                        let (request,token) = request.ok_or("control reader closed")?;
                        let request=request?;
                        diagnostics::record(&trace,token,Stage::RequestDequeued,0,0);
                        authority.diagnostic_request(token);
                        self.policy.check(&self.context)?;
                        let response = match request {
                            Request::Command { session, capability_generation, payload } => {
                                if session.0 != self.context.session || capability_generation != identity.capability_generation {
                                    return Err("stale command session/capabilities".into());
                                }
                                self.context.check_writer(&payload)?;
                                match authority.dispatch(&self.context, payload, now) {
                                    Ok(payload) => payload.map(|payload| Response::Reply { session, payload }),
                                    Err(reason) => Some(Response::Refused { session, reason }),
                                }
                            }
                            Request::Negotiate { session, capability_generation, descriptor } => {
                                if session.0 != self.context.session || capability_generation != identity.capability_generation {
                                    return Err("stale media session/capabilities".into());
                                }
                                let result = descriptor.validate(&self.context, &identity, max_datagram)
                                    .and_then(|()| {
                                        if pending_negotiation.is_some() { return Err("media negotiation pending".into()); }
                                        media.check_install(&descriptor)?;
                                        authority.negotiate(&self.context, &descriptor)
                                    });
                                match result {
                                    Ok(true) => {
                                        media.install(descriptor.clone(), &self.context, &identity, max_datagram)?;
                                        Some(Response::MediaAccepted { session, descriptor })
                                    }
                                    Ok(false) => { pending_negotiation = Some(descriptor); None }
                                    Err(reason) => Some(Response::Refused { session, reason }),
                                }
                            }
                            Request::Open { .. } => return Err("repeated remote open".into()),
                        };
                        if let Some(response) = response { diagnostics::record(&trace,token,Stage::ReplyEnqueued,0,0); replies_tx.try_send((response,token)).map_err(|_| "slow client reply capacity")?; }
                    }
                    bytes = self.connection.read_datagram() => {
                        let bytes = bytes.map_err(|e| e.to_string())?;
                        self.policy.check(&self.context)?;
                        if bytes.starts_with(b"GBA1") {
                            let _ = authority.brain_media(&self.context, &bytes, now);
                        } else {
                            if trace.is_some() && let Ok(packet) = crate::transport::Packet::parse(&bytes) {
                                diagnostics::media_record(&trace, Stage::MediaReceive, packet, authority.source_frame().unwrap_or(0));
                            }
                            match media.receive(&bytes, MediaSide::ProcessingNode, authority.source_frame()) {
                                Ok(packet) => authority.media(&self.context, packet, now)?,
                                Err(_) => if trace.is_some() && let Ok(packet) = crate::transport::Packet::parse(&bytes) {
                                    diagnostics::media_record(&trace, Stage::MediaRefused, packet, authority.source_frame().unwrap_or(0));
                                },
                            }
                        }
                    }
                    error = writer_error_rx.recv() => return Err(error.unwrap_or_else(|| "control writer closed".into())),
                    _ = ticker.tick() => {
                        match authority.poll_negotiation() {
                            Ok(Some(descriptor)) => {
                                if pending_negotiation.take().as_ref() != Some(&descriptor) { return Err("unexpected media negotiation completion".into()); }
                                media.install(descriptor.clone(), &self.context, &identity, max_datagram)?;
                                let token=authority.diagnostic_reply();diagnostics::record(&trace,token,Stage::ReplyEnqueued,0,0);
                                replies_tx.try_send((Response::MediaAccepted { session: Counter(self.context.session), descriptor },token))
                                    .map_err(|_| "slow client reply capacity")?;
                            }
                            Ok(None) => (),
                            Err(reason) => {
                                if pending_negotiation.take().is_none() { return Err(reason); }
                                let token=authority.diagnostic_reply();diagnostics::record(&trace,token,Stage::ReplyEnqueued,0,0);
                                replies_tx.try_send((Response::Refused { session: Counter(self.context.session), reason },token))
                                    .map_err(|_| "slow client reply capacity")?;
                            }
                        }
                        for _ in 0..32 {
                            let Some(reason) = authority.poll_refusal()? else { break; };
                            let token=authority.diagnostic_reply();diagnostics::record(&trace,token,Stage::ReplyEnqueued,0,0);
                            replies_tx.try_send((Response::Refused { session: Counter(self.context.session), reason },token))
                                .map_err(|_| "slow client reply capacity")?;
                        }
                        // Bounded batches; independent host queues own overrun policy.
                        for _ in 0..32 {
                            let Some(payload) = authority.poll_reply(&self.context, now)? else { break; };
                            let mut token=authority.diagnostic_reply();
                            if trace.is_some() && token.ordinal==0 {token=Token::payload(self.context.session,0,&payload);}
                            diagnostics::record(&trace,token,Stage::ReplyEnqueued,0,0);
                            replies_tx.try_send((Response::Reply { session: Counter(self.context.session), payload },token))
                                .map_err(|_| "slow client reply capacity")?;
                        }
                        for _ in 0..64 {
                            let Some(bytes) = authority.poll_brain_media(&self.context)? else { break; };
                            if bytes.len() > max_datagram { return Err("Brain datagram capacity".into()); }
                            self.connection.send_datagram(bytes.into()).map_err(|e| e.to_string())?;
                        }
                        for _ in 0..64 {
                            let Some(bytes) = authority.poll_media(&self.context)? else { break; };
                            media.validate_outgoing(&bytes, MediaSide::ProcessingNode)?;
                            if trace.is_some() && let Ok(packet) = crate::transport::Packet::parse(&bytes) {
                                diagnostics::media_record(&trace, Stage::MediaSend, packet, authority.source_frame().unwrap_or(0));
                                if packet.spec().first_channel == 0 && packet.source_frame().is_multiple_of(4800) { diagnostics::quinn_stats(&trace, Token { ordinal: packet.source_frame(), ..Token::default() }, &self.connection, false); }
                            }
                            if bytes.len() > max_datagram { return Err("media datagram capacity".into()); }
                            self.connection.send_datagram(bytes.into()).map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
        }.await;
        reader.abort();
        let _ = reader.await;
        drop(replies_tx);
        if result.is_ok() {
            if tokio::time::timeout(Duration::from_millis(IO_TIMEOUT_MS + 100), &mut writer)
                .await
                .is_err()
            {
                writer.abort();
                let _ = writer.await;
            }
        } else {
            writer.abort();
            let _ = writer.await;
        }
        result
    }
}

/// A client must explicitly create a fresh connection. There is no automatic
/// reconnect/replay, grant resurrection, output rearm or saved command queue.
/// Dropping a worker aborts its independent reader without joining or leaking a
/// task waiting for a first control byte. No receiver exists on legacy media.
pub struct FxReplyReader(
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicBool>,
);
impl FxReplyReader {
    pub fn abort(&self) {
        self.1.store(true, std::sync::atomic::Ordering::Release);
        self.0.abort();
    }
}
impl Drop for FxReplyReader {
    fn drop(&mut self) {
        self.abort();
    }
}
pub struct RemoteClient {
    _endpoint: quinn::Endpoint,
    connection: quinn::Connection,
    send: quinn::SendStream,
    recv: Option<quinn::RecvStream>,
    context: AuthenticatedContext,
    policy: PolicyStore,
    hello: Response,
    identity: EngineIdentity,
    media: MediaRegistry,
    media_generation: Arc<std::sync::atomic::AtomicU64>,
    media_taken: bool,
    brain_taken: bool,
    brain_last_received: Option<(u64, u64)>,
    brain_generation: Arc<std::sync::atomic::AtomicU64>,
}
impl RemoteClient {
    pub async fn connect(
        endpoint: quinn::Endpoint,
        address: SocketAddr,
        server_name: &str,
        policy: PolicyStore,
    ) -> Result<Self> {
        validate_private_address(address)?;
        let connection = tokio::time::timeout(
            Duration::from_millis(IO_TIMEOUT_MS),
            endpoint
                .connect(address, server_name)
                .map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|_| "TLS handshake timeout")?
        .map_err(|e| e.to_string())?;
        let context = match authenticated(&connection, &policy) {
            Ok(context) => context,
            Err(error) => {
                connection.close(1u8.into(), b"peer authorization refused");
                return Err(error);
            }
        };
        let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
        write_frame(
            &mut send,
            &Request::Open {
                contract: "GP-REMOTE".into(),
                version: 1,
            },
        )
        .await?;
        let hello = read_frame::<Response>(&mut recv).await?;
        let identity = match &hello {
            Response::Hello {
                contract,
                version: 1,
                session,
                writer,
                identity,
                max_datagram,
                ..
            } if contract == "GP-REMOTE"
                && session.0 == context.session
                && crate::show::id(writer)
                && *max_datagram > crate::transport::HEADER_BYTES as u32
                && *max_datagram as usize <= connection.max_datagram_size().unwrap_or(0) =>
            {
                identity.validate()?;
                identity.clone()
            }
            _ => {
                connection.close(1u8.into(), b"invalid remote hello");
                return Err("remote hello refused".into());
            }
        };
        Ok(Self {
            _endpoint: endpoint,
            connection,
            send,
            recv: Some(recv),
            context,
            policy,
            hello,
            identity,
            media: MediaRegistry::new(),
            media_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            media_taken: false,
            brain_taken: false,
            brain_last_received: None,
            brain_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }
    pub fn hello(&self) -> &Response {
        &self.hello
    }
    pub fn identity(&self) -> &EngineIdentity {
        &self.identity
    }
    pub fn session(&self) -> u64 {
        self.context.session
    }
    pub fn writer(&self) -> &str {
        match &self.hello {
            Response::Hello { writer, .. } => writer,
            _ => unreachable!(),
        }
    }
    pub fn max_datagram(&self) -> usize {
        match &self.hello {
            Response::Hello { max_datagram, .. } => *max_datagram as usize,
            _ => unreachable!(),
        }
    }
    pub fn close(&self) {
        self.context.close_audio();
        self.connection.close(0u8.into(), b"operator disconnect");
    }
    fn check(&self) -> Result<()> {
        if self.connection.close_reason().is_some() {
            self.context.close_audio();
            return Err("remote connection closed".into());
        }
        if let Err(error) = self.policy.check(&self.context) {
            self.connection.close(1u8.into(), b"peer policy revoked");
            return Err(error);
        }
        Ok(())
    }
    /// Move the connection's one Brain datagram reader into the media worker.
    /// The reliable control stream remains owned by this client.
    pub fn take_brain_media(&mut self) -> Result<BrainMediaChannel> {
        self.check()?;
        if self.brain_taken || self.media_taken || self.media.descriptor().is_some() {
            return Err("connection media reader already assigned".into());
        }
        self.brain_taken = true;
        Ok(BrainMediaChannel {
            connection: self.connection.clone(),
            context: self.context.clone(),
            policy: self.policy.clone(),
            identity: self.identity.clone(),
            generation: self.brain_generation.clone(),
            max_datagram: self.max_datagram(),
            last_received: None,
        })
    }
    pub async fn report_brain_device(&mut self, observation: BrainDeviceObservation) -> Result<()> {
        observation.validate()?;
        self.send_command(serde_json::json!({"contract":"GP15-device","version":1,"kind":"device_observation","writer":self.writer(),"observation":observation})).await
    }
    pub async fn negotiate_brain_media(&mut self, descriptor: BrainMediaDescriptor) -> Result<()> {
        descriptor.validate(self.session(), &self.identity)?;
        self.send_command(serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":self.writer(),"descriptor":descriptor})).await
    }
    /// A dedicated duplex connection has one datagram reader, independent of FX.
    pub fn send_brain_media(&self, packet: &BrainPacket) -> Result<()> {
        self.check()?;
        packet.descriptor.validate(self.session(), &self.identity)?;
        if packet.role != BrainMediaRole::Talkback {
            return Err("Brain send direction".into());
        }
        let bytes = packet.encode()?;
        if bytes.len() > self.max_datagram() {
            return Err("Brain datagram capacity".into());
        }
        self.connection
            .send_datagram(bytes.into())
            .map_err(|e| e.to_string())
    }
    pub async fn receive_brain_media(
        &mut self,
        descriptor: &BrainMediaDescriptor,
    ) -> Result<BrainPacket> {
        self.check()?;
        if self.brain_taken || self.media_taken {
            return Err("connection media reader already assigned".into());
        }
        descriptor.validate(self.session(), &self.identity)?;
        if descriptor.generation
            != self
                .brain_generation
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("Brain media negotiation not accepted/current".into());
        }
        let bytes = self.connection.read_datagram().await.map_err(|e| {
            self.context.close_audio();
            e.to_string()
        })?;
        self.check()?;
        let packet = BrainPacket::decode(&bytes, descriptor, BrainMediaRole::Monitor)?;
        if self
            .brain_last_received
            .is_some_and(|(g, f)| g == descriptor.selection_generation && packet.frame <= f)
        {
            return Err("Brain replay/reordered media".into());
        }
        self.brain_last_received = Some((descriptor.selection_generation, packet.frame));
        Ok(packet)
    }
    pub async fn send_command(&mut self, payload: Value) -> Result<()> {
        self.check()?;
        write_frame(
            &mut self.send,
            &Request::Command {
                session: Counter(self.context.session),
                capability_generation: self.identity.capability_generation,
                payload,
            },
        )
        .await
    }
    pub async fn receive(&mut self) -> Result<Response> {
        // Drain the final reliable boundary result even if QUIC has subsequently
        // closed after acknowledging it. New sends/media still require a live connection.
        self.policy.check(&self.context)?;
        let response: Response = read_response(self.recv.as_mut().ok_or("control receiver moved")?)
            .await
            .inspect_err(|_| {
                self.context.close_audio();
            })?;
        self.policy.check(&self.context)?;
        let session = match &response {
            Response::Reply { session, .. }
            | Response::Refused { session, .. }
            | Response::MediaAccepted { session, .. } => *session,
            _ => return Err("unexpected remote hello".into()),
        };
        if session.0 != self.session() {
            return Err("remote response session".into());
        }
        if let Response::Reply { payload, .. } = &response
            && payload.get("contract").and_then(Value::as_str) == Some("GP15-media")
            && payload.get("state").and_then(Value::as_str) == Some("accepted")
        {
            let descriptor: BrainMediaDescriptor = serde_json::from_value(
                payload
                    .get("descriptor")
                    .cloned()
                    .ok_or("Brain accepted descriptor missing")?,
            )
            .map_err(|e| e.to_string())?;
            descriptor.validate(self.session(), &self.identity)?;
            self.brain_generation
                .store(descriptor.generation, std::sync::atomic::Ordering::Release);
        }
        if let Response::MediaAccepted { descriptor, .. } = &response {
            // These permissions describe the local peer's grant ceiling from the
            // authenticated server, not the local policy for server commands.
            self.media.install_received(
                descriptor.clone(),
                self.session(),
                &self.identity,
                self.max_datagram(),
            )?;
            self.media_generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            self.media_taken = false;
        }
        Ok(response)
    }
    /// Move an already-negotiated FX control reader to one bounded task. Reading
    /// complete frames is never cancelled by the media select loop.
    pub fn take_fx_replies(
        &mut self,
    ) -> Result<(tokio::sync::mpsc::Receiver<Result<Response>>, FxReplyReader)> {
        if !self.media_taken {
            return Err("FX media must be admitted first".into());
        }
        let mut recv = self.recv.take().ok_or("control reader already moved")?;
        let context = self.context.clone();
        let closed = context.disconnect_signal();
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let task = tokio::spawn(async move {
            loop {
                let result = match context.check_current() {
                    Err(e) => Err(e),
                    Ok(()) => read_response(&mut recv).await.and_then(|r| {
                        context.check_current()?;
                        match &r {
                            Response::Reply { session, .. } | Response::Refused { session, .. }
                                if session.0 == context.session() =>
                            {
                                Ok(r)
                            }
                            _ => Err("FX control response identity".into()),
                        }
                    }),
                };
                let failed = result.is_err();
                if failed {
                    context.close_audio();
                }
                if tx.send(result).await.is_err() || failed {
                    context.close_audio();
                    break;
                }
            }
        });
        Ok((rx, FxReplyReader(task, closed)))
    }
    pub async fn negotiate(&mut self, descriptor: MediaDescriptor) -> Result<()> {
        self.check()?;
        if self.brain_taken {
            return Err("dedicated Brain duplex connection".into());
        }
        write_frame(
            &mut self.send,
            &Request::Negotiate {
                session: Counter(self.context.session),
                capability_generation: self.identity.capability_generation,
                descriptor,
            },
        )
        .await
    }
    /// Move media consumption to an independent worker after negotiation. A new
    /// negotiated map invalidates every previous channel; control stays separate.
    pub fn media_channel(&mut self) -> Result<MediaChannel> {
        self.check()?;
        if self.media_taken || self.brain_taken {
            return Err("media channel already owned".into());
        }
        let descriptor = self
            .media
            .descriptor()
            .ok_or("media not negotiated")?
            .clone();
        let mut registry = MediaRegistry::new();
        registry.install_received(
            descriptor,
            self.session(),
            &self.identity,
            self.max_datagram(),
        )?;
        self.media_taken = true;
        Ok(MediaChannel {
            connection: self.connection.clone(),
            context: self.context.clone(),
            policy: self.policy.clone(),
            registry,
            generation: self
                .media_generation
                .load(std::sync::atomic::Ordering::Acquire),
            current_generation: self.media_generation.clone(),
            max_datagram: self.max_datagram(),
        })
    }
    pub fn send_media(&self, bytes: Vec<u8>) -> Result<()> {
        self.check()?;
        self.media.validate_outgoing(&bytes, MediaSide::Brain)?;
        if bytes.len() > self.max_datagram() {
            return Err("media datagram capacity".into());
        }
        self.connection
            .send_datagram(bytes.into())
            .map_err(|e| e.to_string())
    }
    pub async fn receive_media(&mut self) -> Result<Vec<u8>> {
        self.check()?;
        if self.media_taken || self.brain_taken {
            return Err("media owned by independent channel".into());
        }
        let bytes = self.connection.read_datagram().await.map_err(|e| {
            self.context.close_audio();
            e.to_string()
        })?;
        self.check()?;
        self.media.receive(&bytes, MediaSide::Brain, None)?;
        Ok(bytes.to_vec())
    }
}
impl Drop for RemoteClient {
    fn drop(&mut self) {
        self.close();
    }
}

/// Authenticated media worker handle. It performs no rendering and owns no clock.
pub struct MediaChannel {
    connection: quinn::Connection,
    context: AuthenticatedContext,
    policy: PolicyStore,
    registry: MediaRegistry,
    generation: u64,
    current_generation: Arc<std::sync::atomic::AtomicU64>,
    max_datagram: usize,
}
impl MediaChannel {
    #[cfg(feature = "hardware-host")]
    pub(crate) fn audio_fence(&self) -> super::policy::PolicyFence {
        self.context.audio_fence()
    }

    #[cfg(feature = "hardware-host")]
    pub(crate) fn diagnostic_stats(&self, trace: &Handle, source_frame: u64) {
        diagnostics::quinn_stats(
            trace,
            Token {
                session: self.context.session,
                ordinal: source_frame,
                ..Token::default()
            },
            &self.connection,
            false,
        );
    }
    fn check(&self) -> Result<()> {
        if self.connection.close_reason().is_some() {
            self.context.close_audio();
            return Err("media channel closed".into());
        }
        if self.generation
            != self
                .current_generation
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("media channel stale or closed".into());
        }
        if let Err(error) = self.policy.check(&self.context) {
            self.connection.close(1u8.into(), b"peer policy revoked");
            return Err(error);
        }
        Ok(())
    }
    #[cfg(feature = "hardware-host")]
    pub(crate) fn check_current(&self) -> Result<()> {
        self.check()
    }
    pub fn descriptor(&self) -> &MediaDescriptor {
        self.registry.descriptor().expect("admitted descriptor")
    }
    pub fn send(&self, bytes: Vec<u8>) -> Result<()> {
        self.check()?;
        self.registry.validate_outgoing(&bytes, MediaSide::Brain)?;
        if bytes.len() > self.max_datagram {
            return Err("media datagram capacity".into());
        }
        self.connection
            .send_datagram(bytes.into())
            .map_err(|e| e.to_string())
    }
    pub async fn receive(&mut self) -> Result<Vec<u8>> {
        self.check()?;
        loop {
            let bytes = self.connection.read_datagram().await.map_err(|e| {
                self.context.close_audio();
                e.to_string()
            })?;
            self.check()?;
            if self
                .registry
                .receive(&bytes, MediaSide::Brain, None)
                .is_ok()
            {
                return Ok(bytes.to_vec());
            }
            tokio::task::yield_now().await;
        }
    }
}

/// Independent authenticated duplex datagram handle. This is not Clone: exactly
/// one receiver consumes its connection, while RemoteClient retains control.
pub struct BrainMediaChannel {
    connection: quinn::Connection,
    context: AuthenticatedContext,
    policy: PolicyStore,
    identity: EngineIdentity,
    generation: Arc<std::sync::atomic::AtomicU64>,
    max_datagram: usize,
    last_received: Option<(u64, u64)>,
}
impl BrainMediaChannel {
    /// Scheduling hint only. Every send/receive still performs full policy and
    /// descriptor admission; this does not grant media authority.
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(crate) fn accepted_generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    fn check(&self, descriptor: &BrainMediaDescriptor) -> Result<()> {
        if self.connection.close_reason().is_some() {
            self.context.close_audio();
            return Err("Brain media connection closed".into());
        }
        self.policy.check(&self.context)?;
        descriptor.validate(self.context.session(), &self.identity)?;
        if descriptor.generation != self.generation.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Brain media negotiation not accepted/current".into());
        }
        Ok(())
    }
    pub fn send(&self, packet: &BrainPacket) -> Result<()> {
        self.check(&packet.descriptor)?;
        if packet.role != BrainMediaRole::Talkback {
            return Err("Brain media send direction".into());
        }
        let bytes = packet.encode()?;
        if bytes.len() > self.max_datagram {
            return Err("Brain datagram capacity".into());
        }
        self.connection
            .send_datagram(bytes.into())
            .map_err(|e| e.to_string())
    }
    pub async fn receive(&mut self, descriptor: &BrainMediaDescriptor) -> Result<BrainPacket> {
        self.check(descriptor)?;
        loop {
            let bytes = self.connection.read_datagram().await.map_err(|e| {
                self.context.close_audio();
                e.to_string()
            })?;
            self.check(descriptor)?;
            if let Ok(packet) = BrainPacket::decode(&bytes, descriptor, BrainMediaRole::Monitor)
                && !self.last_received.is_some_and(|(generation, frame)| {
                    generation == descriptor.selection_generation && packet.frame <= frame
                })
            {
                self.last_received = Some((descriptor.selection_generation, packet.frame));
                return Ok(packet);
            }
            tokio::task::yield_now().await;
        }
    }
}
