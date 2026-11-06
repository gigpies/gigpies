//! Actual owners across authenticated loopback and autonomous source recovery.
//! Software lifecycle evidence only; neither hardware nor network timing acceptance.
use super::*;
use crate::fx_wire::{Mutation, OwnerCommand, OwnerMessage};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    time::Instant,
};

async fn ready(worker: &mut BrainWorker) -> crate::fx_wire::Observation {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let observation = worker.fx_observation().unwrap();
        if observation.next_source_frame.0 > 48 {
            return observation;
        }
        assert!(Instant::now() < until, "FX source initialization timeout");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

// Drive the same reliable owner envelopes as the production Brain runner. Native
// media/execution remain autonomous when this controller task is not serviced.
struct Relay {
    stop: Rc<Cell<bool>>,
    commands: Rc<RefCell<Vec<OwnerCommand>>>,
    messages: Rc<RefCell<Vec<OwnerMessage>>>,
    observation: Rc<RefCell<crate::fx_wire::Observation>>,
    task: tokio::task::JoinHandle<(RemoteClient, BrainWorker)>,
}
impl Relay {
    fn start(mut client: RemoteClient, mut worker: BrainWorker) -> Self {
        let stop = Rc::new(Cell::new(false));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let messages = Rc::new(RefCell::new(Vec::new()));
        let observation = Rc::new(RefCell::new(worker.fx_observation().unwrap()));
        let (mut controls, reader) = client.take_fx_replies().unwrap();
        let (stopping, received, sent, observed) = (
            stop.clone(),
            commands.clone(),
            messages.clone(),
            observation.clone(),
        );
        let task = tokio::task::spawn_local(async move {
            let mut timer = tokio::time::interval(Duration::from_millis(2));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_observation = Instant::now() - Duration::from_millis(50);
            while !stopping.get() {
                let message = tokio::select! {
                    response = controls.recv() => {
                        if stopping.get() { break; }
                        match response.unwrap().unwrap() {
                        Response::Reply { payload, .. } => {
                            if let Some(command) = payload.get("command") {
                                let command: OwnerCommand = serde_json::from_value(command.clone()).unwrap();
                                received.borrow_mut().push(command.clone());
                                worker.fx_command(command).unwrap()
                            } else { None }
                        }
                        response => panic!("Brain relay: {response:?}"),
                        }
                    },
                    _ = timer.tick() => {
                        if stopping.get() { break; }
                        worker.poll_fx_observation().unwrap()
                    },
                };
                *observed.borrow_mut() = worker.fx_observation().unwrap();
                if let Some(message) = message {
                    if matches!(message, OwnerMessage::Observe { .. }) {
                        // Match the production runner's observation publication cadence.
                        if last_observation.elapsed() < Duration::from_millis(50) {
                            continue;
                        }
                        last_observation = Instant::now();
                    }
                    if !matches!(message, OwnerMessage::Observe { .. }) {
                        sent.borrow_mut().push(message.clone());
                    }
                    if let Err(error) = client.send_command(owner_envelope(&client, message)).await
                    {
                        assert!(stopping.get(), "Brain relay send: {error}");
                        break;
                    }
                }
            }
            client.close();
            reader.abort();
            worker.finish_execution();
            if let Some(message) = worker.poll_fx_observation().unwrap() {
                sent.borrow_mut().push(message);
            }
            (client, worker)
        });
        Self {
            stop,
            commands,
            messages,
            observation,
            task,
        }
    }
    async fn catch_up(&self, source: &super::super::SourceClockStatus) {
        let until = Instant::now() + Duration::from_secs(2);
        let mut reset = self.observation.borrow().reset_count;
        let mut stable_since = Instant::now();
        let mut frame = source.blocks() * 48;
        loop {
            // Compare the asynchronously sampled owner to the source position
            // captured before this polling interval, not a newer live position.
            let observation = self.observation.borrow().clone();
            if observation.reset_count != reset {
                reset = observation.reset_count;
                stable_since = Instant::now();
            }
            // A first live-frame observation can still be followed by a queued
            // startup gap. Require a bounded continuity window before review;
            // never retry a mutation after refusal.
            if frame > 0
                && observation.next_source_frame.0 >= frame
                && stable_since.elapsed() >= Duration::from_millis(128)
            {
                return;
            }
            assert!(
                Instant::now() < until,
                "FX relay failed to catch up to source: frame={frame}, observation={observation:?}"
            );
            frame = source.blocks() * 48;
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}
fn owner_envelope(client: &RemoteClient, message: OwnerMessage) -> Value {
    json!({"contract":"GP21-fx","version":1,"kind":"fx_owner","writer":client.writer(),"message":message})
}
async fn connect_desk(
    address: std::net::SocketAddr,
    credentials: &Credentials,
    policy: PolicyStore,
) -> RemoteClient {
    RemoteClient::connect(
        client_endpoint("127.0.0.1:0".parse().unwrap(), credentials).unwrap(),
        address,
        "stagebox.test",
        policy,
    )
    .await
    .unwrap()
}
async fn reply(client: &mut RemoteClient) -> Value {
    match tokio::time::timeout(Duration::from_secs(3), client.receive())
        .await
        .unwrap()
        .unwrap()
    {
        Response::Reply { payload, .. } => payload,
        response => panic!("Desk relay: {response:?}"),
    }
}
async fn snapshot(desk: &mut RemoteClient) -> Value {
    desk.send_command(json!({"contract":"GP21-fx","version":1,"show_id":SHOW,"module":"audio","epoch":desk.identity().source_epoch,"writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_snapshot","body":{}})).await.unwrap();
    reply(desk).await
}
async fn configure(
    desk: &mut RemoteClient,
    relay: &Relay,
    source: &super::super::SourceClockStatus,
) -> (Value, Value) {
    let until = Instant::now() + Duration::from_secs(2);
    let observed = loop {
        let value = snapshot(desk).await;
        if value["available"] == true {
            break value;
        }
        assert!(Instant::now() < until, "fresh owner observation: {value}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    let observation: crate::fx_wire::Observation =
        serde_json::from_value(observed["observation"].clone()).unwrap();
    let mut request = json!({"contract":"GP21-fx","version":1,"show_id":SHOW,"module":"audio","epoch":desk.identity().source_epoch,"writer":desk.writer(),"lease":"999999","request_id":"2","expected_revision":observed["revision"],"kind":"fx_configure","body":Mutation {
        binding: observation.binding, expected_generation: observation.generation,
        expected_reset_count: observation.reset_count, lead_frames: Counter(4800),
        configuration_json: Some(observation.owner_json), panic_mask: None,
    }});
    // Authentication and a fresh read alone must not authorize a mutation.
    desk.send_command(request.clone()).await.unwrap();
    let refused = desk.receive().await.unwrap();
    assert!(
        matches!(&refused, Response::Refused { reason, .. } if reason == "FX scope lease required"),
        "{refused:?}"
    );
    let mut audio_read = snapshot_request();
    audio_read["epoch"] = json!(desk.identity().source_epoch);
    desk.send_command(audio_read).await.unwrap();
    let authority = reply(desk).await;
    assert_eq!(
        authority["snapshot"]["authority"]["revision"],
        observed["revision"]
    );
    desk.send_command(json!({"contract":"C-AUDIO","version":2,"show_id":SHOW,"module":"audio","epoch":desk.identity().source_epoch,"writer":desk.writer(),"lease":null,"request_id":"1","expected_revision":observed["revision"],"kind":"grant","body":{"scope":"fx_configuration"}})).await.unwrap();
    let grant = reply(desk).await;
    let lease = grant["outcome"]["body"]["granted_lease"].clone();
    assert!(lease.is_string(), "{grant}");
    relay.catch_up(source).await;
    let fresh = snapshot(desk).await;
    let reviewed: crate::fx_wire::Observation =
        serde_json::from_value(fresh["observation"].clone()).unwrap();
    request["body"] = json!(Mutation {
        binding: reviewed.binding,
        expected_generation: reviewed.generation,
        expected_reset_count: reviewed.reset_count,
        lead_frames: Counter(4800),
        configuration_json: Some(reviewed.owner_json),
        panic_mask: None,
    });
    request["lease"] = lease;
    request["expected_revision"] = fresh["revision"].clone();
    desk.send_command(request.clone()).await.unwrap();
    let response = desk.receive().await.unwrap();
    let Response::Reply {
        payload: preparing, ..
    } = response
    else {
        panic!("reviewed request refused: {response:?}; snapshot={fresh}; request={request}");
    };
    assert_eq!(preparing["state"], "preparing", "{preparing}");
    (request, preparing)
}

async fn attach(
    address: std::net::SocketAddr,
    credentials: &Credentials,
    policy: PolicyStore,
    library: &Path,
    hash: &str,
    deadline: Instant,
) -> (RemoteClient, BrainWorker) {
    let endpoint = client_endpoint("127.0.0.1:0".parse().unwrap(), credentials).unwrap();
    let mut client = RemoteClient::connect(endpoint, address, "stagebox.test", policy)
        .await
        .unwrap();
    let mut media = descriptor(&context(&[Permission::Fx]), 8);
    media.session = Counter(client.session());
    media.source_epoch = client.identity().source_epoch;
    media.capability_generation = client.identity().capability_generation;
    media.map_generation = client.identity().map_generation;
    media
        .streams
        .retain(|stream| stream.role != MediaRole::Analysis);
    for stream in &mut media.streams {
        if stream.role == MediaRole::WetReturn {
            stream.delay_frames = 768;
        }
    }
    client.negotiate(media).await.unwrap();
    assert!(matches!(
        client.receive().await.unwrap(),
        Response::MediaAccepted { .. }
    ));
    let media = client.media_channel().unwrap();
    let library = library.to_path_buf();
    let hash = hash.to_owned();
    // The two production processes do not share a controller. Loading/hashing
    // the Brain library must not block this fixture's Stagebox service loop.
    let worker = tokio::task::spawn_blocking(move || {
        let mut worker = BrainWorker::prepare(media, &library).unwrap();
        assert!(worker.enable_fx_control(&library, &hash).unwrap());
        worker.start_fx_execution(deadline).unwrap();
        worker
    })
    .await
    .unwrap();
    assert!(
        worker.execution_report().is_some(),
        "must use dedicated production execution"
    );
    (client, worker)
}

async fn brain_read(client: &mut RemoteClient) -> Value {
    client
        .send_command(json!({"contract":"GP15-brain","version":1,"show_id":SHOW,
        "module":"audio","epoch":client.identity().source_epoch,"writer":null,
        "lease":null,"request_id":null,"expected_revision":null,
        "kind":"brain_snapshot","body":{}}))
        .await
        .unwrap();
    reply(client).await
}
async fn brain_change(
    client: &mut RemoteClient,
    id: u64,
    lease: Value,
    command: crate::brain_control::Command,
) -> crate::brain_control::Snapshot {
    let read = brain_read(client).await;
    let mut request = serde_json::to_value(command).unwrap();
    let object = request.as_object_mut().unwrap();
    object.extend(
        json!({"contract":"GP15-brain","version":1,"show_id":SHOW,
        "module":"audio","epoch":client.identity().source_epoch,"writer":client.writer(),
        "lease":lease,"request_id":Counter(id),"expected_revision":read["revision"]})
        .as_object()
        .unwrap()
        .clone(),
    );
    client.send_command(request).await.unwrap();
    let mut result = reply(client).await;
    if result["state"] == "pending" {
        result = reply(client).await;
    }
    assert!(result["reason"].is_null(), "Brain change: {result}");
    assert_eq!(result["state"], "final", "{result}");
    serde_json::from_value(result["snapshot"].clone()).unwrap()
}
async fn brain_grant(client: &mut RemoteClient, id: u64, scope: &str) -> Value {
    let mut read = snapshot_request();
    read["epoch"] = json!(client.identity().source_epoch);
    client.send_command(read).await.unwrap();
    let read = reply(client).await;
    client.send_command(json!({"contract":"C-AUDIO","version":2,"show_id":SHOW,
        "module":"audio","epoch":client.identity().source_epoch,"writer":client.writer(),
        "lease":null,"request_id":Counter(id),"expected_revision":read["snapshot"]["authority"]["revision"],
        "kind":"grant","body":{"scope":scope}})).await.unwrap();
    let result = reply(client).await;
    let lease = result["outcome"]["body"]["granted_lease"].clone();
    assert!(lease.is_string(), "{result}");
    lease
}
async fn attach_duplex(
    address: std::net::SocketAddr,
    credentials: &Credentials,
    policy: PolicyStore,
    deadline: Instant,
) -> (
    RemoteClient,
    RemoteClient,
    crate::brain_runtime::lifecycle_test::AuthenticatedDuplexProbe,
) {
    use crate::brain_control::{Command as B, MonitorSource};
    let mut monitor_client = connect_desk(address, credentials, policy.clone()).await;
    let mut client = connect_desk(address, credentials, policy).await;
    let lease = brain_grant(&mut monitor_client, 1, "local_operator_monitor").await;
    for (id, armed) in [(2, false), (3, true)] {
        brain_change(
            &mut monitor_client,
            id,
            lease.clone(),
            B::MonitorSet {
                source: MonitorSource::Pfl { input: 7 },
                gain_cdb: 0,
                mute: false,
                dim: false,
                armed,
            },
        )
        .await;
    }
    let lease = brain_grant(&mut client, 1, "talkback_destinations").await;
    let snapshot = brain_change(
        &mut client,
        2,
        lease.clone(),
        B::TalkbackSet {
            monitors: vec![0],
            gain_cdb: -1200,
            mute: false,
        },
    )
    .await;
    let snapshot = brain_change(
        &mut client,
        3,
        lease,
        B::Hold {
            generation: Counter(snapshot.hold_generation_counter.0 + 1),
        },
    )
    .await;
    let descriptor = BrainMediaDescriptor {
        contract: "GP15-media".into(),
        version: 1,
        session: client.session(),
        stagebox_epoch: client.identity().source_epoch.0,
        brain_epoch: client.identity().source_epoch.0 + 70,
        stagebox_map: client.identity().map_generation.0,
        brain_map: client.identity().source_epoch.0 + 70,
        generation: 1,
        selection_generation: snapshot.selection_generation.0,
        hold_generation: snapshot.held_generation.unwrap().0,
        sample_rate: 48000,
        frames: 48,
        talkback: true,
        monitor: true,
    };
    client
        .negotiate_brain_media(descriptor.clone())
        .await
        .unwrap();
    let result = reply(&mut client).await;
    assert!(result["reason"].is_null(), "{result}");
    let probe = crate::brain_runtime::lifecycle_test::AuthenticatedDuplexProbe::start(
        client.take_brain_media().unwrap(),
        &descriptor,
        &snapshot,
        deadline,
    );
    (client, monitor_client, probe)
}

async fn authenticated_rearm(client: &mut RemoteClient) {
    let lease = brain_grant(client, 1, "pa_configuration").await;
    let mut request = json!({"contract":"GP14-structure","version":1,"show_id":SHOW,
        "module":"audio","epoch":client.identity().source_epoch,"writer":null,
        "lease":null,"request_id":null,"expected_revision":null,
        "kind":"structural_snapshot","body":{}});
    client.send_command(request.clone()).await.unwrap();
    let snapshot = reply(client).await;
    assert!(snapshot["reason"].is_null(), "{snapshot}");
    request["writer"] = json!(client.writer());
    request["lease"] = lease;
    request["request_id"] = json!("2");
    request["expected_revision"] = snapshot["revision"].clone();
    request["kind"] = json!("output_rearm");
    client.send_command(request).await.unwrap();
    let mut result = reply(client).await;
    if result["state"] == "pending" {
        result = reply(client).await;
    }
    assert_eq!(result["state"], "final", "{result}");
    assert!(result["reason"].is_null(), "{result}");
}

async fn nonzero_duplex(
    client: &mut RemoteClient,
    probe: &mut crate::brain_runtime::lifecycle_test::AuthenticatedDuplexProbe,
) {
    let until = Instant::now() + Duration::from_millis(100);
    while probe.nonzero_playback == 0 {
        probe.poll();
        client
            .send_command(json!({"contract":"GP15-device","version":1,
            "kind":"device_observation","writer":client.writer(),
            "observation":probe.observation()}))
            .await
            .unwrap();
        let observed = reply(client).await;
        assert!(observed["reason"].is_null(), "{observed}");
        probe.poll();
        assert!(
            probe.nonzero_playback > 0 || Instant::now() < until,
            "selected monitor remained silent: {:?}",
            probe.observation()
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

fn rearm(host: &mut HostAuthority, now: u64) {
    let revision = host.provider().engine().revision();
    let epoch = host.identity().source_epoch;
    let lease = host
        .provider_mut()
        .engine_mut()
        .handle(
            &AudioRequest {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch,
                writer: Some("recovery".into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(revision),
                command: Command::Grant {
                    scope: Scope::PaConfiguration,
                },
            },
            now,
        )
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap();
    let request = crate::structural_control::Request {
        contract: "GP14-structure".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch,
        writer: Some("recovery".into()),
        lease: Some(lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(revision),
        command: crate::structural_control::Command::OutputRearm {},
    };
    host.provider_mut()
        .clocked_control(now, None, |provider| {
            serde_json::to_value(provider.structural_request(request, now, true, None)?)
                .map_err(|error| error.to_string())
        })
        .unwrap();
    assert_eq!(
        host.provider().engine().clock_status().state,
        crate::clock_domain::ClockState::Running
    );
}

fn prepare_host(
    directory: &Path,
    combined: bool,
) -> (
    Rc<RefCell<HostAuthority>>,
    super::super::SourceClockStatus,
    crate::module_graph::Manifest,
) {
    let manifest_path = std::env::var_os("GP05_MANIFEST").unwrap();
    let manifest = crate::module_graph::Manifest::load(Path::new(&manifest_path)).unwrap();
    let mut provider = crate::local_audio::LocalAudio::bind_configured(
        directory,
        "host",
        SHOW,
        Counter(9),
        EngineTopology::software(8, 2, 8).unwrap(),
    )
    .unwrap();
    provider.enable_modules(Path::new(&manifest_path)).unwrap();
    provider
        .configure_pa(&std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap())
        .unwrap();
    provider.rearm().unwrap();
    super::super::runner::start_recording(&mut provider, SHOW, "tls-recovery", monotonic_ms())
        .unwrap();
    let mut authority = HostAuthority::new(provider, 1).unwrap();
    // Deliberately paced fixture; the unchanged 768-frame admission bound
    // is not being qualified as a real-time network margin here.
    let source = authority
        .start_source_clock(
            monotonic_ms(),
            Duration::from_millis(if combined { 1 } else { 2 }),
            if combined { 12000 } else { 6000 },
        )
        .unwrap();
    let host = Rc::new(RefCell::new(authority));
    (host, source, manifest)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "GP05_MANIFEST and GP_PA_CONFIG actual owners; bounded authenticated local loopback"]
async fn actual_tls_fx_pending_close_and_replacement_after_source_recovery() {
    tls_recovery(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "GP05_MANIFEST and GP_PA_CONFIG actual owners; bounded combined authenticated local loopback"]
async fn actual_tls_combined_duplex_fx_rec_recovery() {
    tls_recovery(true).await;
}
async fn tls_recovery(combined: bool) {
    tokio::task::LocalSet::new()
        .run_until(Box::pin(async {
            tokio::time::timeout(
                Duration::from_secs(20),
                Box::pin(async {
                    let directory = Rc::new(PrivateTestDirectory::new());
                    let (host, source, manifest) = prepare_host(&directory.0, combined);
                    let (server_credentials, client_credentials) = credentials();
                    let server_policy = PolicyStore::new(vec![peer(
                        client_credentials.certificate_chain[0].as_ref(),
                        &[Permission::Fx, Permission::FxConfiguration, Permission::TalkbackDestinations, Permission::LocalOperatorMonitor, Permission::Analysis, Permission::PaConfiguration],
                    )])
                    .unwrap();
                    let client_policy = PolicyStore::new(vec![peer(
                        server_credentials.certificate_chain[0].as_ref(),
                        &[],
                    )])
                    .unwrap();
                    let server = RemoteServer::bind(
                        "127.0.0.1:0".parse().unwrap(),
                        &server_credentials,
                        server_policy,
                    )
                    .unwrap();
                    let address = server.local_addr().unwrap();
                    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
                    let accepting_host = host.clone();
                    let acceptor = tokio::task::spawn_local(async move {
                        loop {
                            let session = server.accept().await.unwrap();
                            let identity = accepting_host.borrow().identity();
                            let (mut proxy, mailbox) =
                                authority_channel(session.context(), identity).unwrap();
                            sender.send(mailbox).await.unwrap();
                            tokio::spawn(async move {
                                let _ = session.run(&mut proxy).await;
                            });
                        }
                    });
                    let stalled = Rc::new(Cell::new(false));
                    let stop = Rc::new(Cell::new(false));
                    let service_host = host.clone();
                    let service_stalled = stalled.clone();
                    let service_stop = stop.clone();
                    let retained_directory = directory.clone();
                    let service = tokio::task::spawn_local(async move {
                        let _directory = retained_directory;
                        let mut mailboxes = Vec::new();
                        while !service_stop.get() {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                            if service_stalled.get() {
                                continue;
                            }
                            while let Ok(mailbox) = receiver.try_recv() {
                                mailboxes.push(mailbox);
                            }
                            let mut host = service_host.borrow_mut();
                            host.service_source_clock(monotonic_ms()).unwrap();
                            for mailbox in &mut mailboxes {
                                mailbox.service(&mut *host, monotonic_ms()).unwrap();
                            }
                        }
                    });
                    let recording_until = Instant::now() + Duration::from_secs(3);
                    loop {
                        let status = host
                            .borrow_mut()
                            .provider_mut()
                            .module_status(monotonic_ms())
                            .unwrap();
                        if status["recording"]["state"] == "recording" {
                            break;
                        }
                        assert!(Instant::now() < recording_until, "REC startup: {status}");
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    let deadline = Instant::now() + Duration::from_secs(8);
                    let (brain, mut worker) = attach(
                        address, &client_credentials, client_policy.clone(),
                        &manifest.fx.library, &manifest.fx.library_sha256, deadline,
                    ).await;
                    ready(&mut worker).await;
                    let mut analysis = if combined {
                        let credentials = Credentials { certificate_chain: client_credentials.certificate_chain.clone(),
                            private_key: client_credentials.private_key.clone_key(), trust_roots: client_credentials.trust_roots.clone() };
                        let ids = host.borrow().provider().topology().inputs.iter().map(|i| i.id.clone()).collect();
                        Some(super::super::runner::analysis::Worker::start("127.0.0.1:0".parse().unwrap(),
                            address, "stagebox.test".into(), credentials, client_policy.clone(), host.borrow().identity(), ids, true, deadline).unwrap())
                    } else { None };
                    let mut duplex = if combined {
                        Some(attach_duplex(address, &client_credentials, client_policy.clone(), deadline).await)
                    } else { None };
                    if let Some((client, _, probe)) = &mut duplex {
                        nonzero_duplex(client, probe).await;
                    }
                    let old_device = duplex.as_mut().map(|(_, _, probe)| probe.observation());
                    let prepared = host.borrow_mut().prepared_media_storage().unwrap();
                    assert!(prepared["known_total_bytes"].as_u64().unwrap() > 256 * 1232 * 2);
                    let resource_before = crate::brain_runtime::process_resources().unwrap();
                    let pressure_started = Instant::now();
                    stalled.set(true);
                    let wet_before = host.borrow_mut().test_pending_accepted_wet_receipts();
                    let before = source.blocks();
                    let native_before = worker.fx_observation().unwrap().next_source_frame.0;
                    let observed_frame = host.borrow().provider().frame();
                    for completions in [false, true] {
                        host.borrow().with_saturated_source_returns(completions, || {
                            let target = source.blocks() + 32;
                            let until = Instant::now() + Duration::from_secs(2);
                            while source.blocks() < target {
                                assert!(Instant::now() < until, "saturated source stalled");
                                std::thread::sleep(Duration::from_millis(1));
                            }
                        });
                    }
                    assert!(source.blocks() >= before + 64);
                    assert_eq!(host.borrow().provider().frame(), observed_frame);
                    assert!(worker.fx_observation().unwrap().next_source_frame.0 >= native_before + 32 * 48,
                        "native media stopped behind controller saturation");
                    let wet_after = host.borrow_mut().test_pending_accepted_wet_receipts();
                    if wet_after < wet_before + 48 {
                        worker.test_poll_media_report();
                        let kinds = host.borrow_mut().test_pending_wet_receipt_kinds();
                        eprintln!("wet receipt kinds accepted/late/refused={kinds:?}; source egress={}; resources={}",
                            host.borrow().source_egress_report(), crate::brain_runtime::process_resources().unwrap());
                    }
                    assert!(wet_after >= wet_before + 48,
                        "authenticated wet admission stopped behind controller service: accepted before={wet_before}, after={wet_after}, blocks before={before}, after={}, native before={native_before}, after={:?}, execution={:?}, media_stats={:?}", source.blocks(), worker.fx_observation(), worker.execution_report(), serde_json::to_value(host.borrow().media_stats()).unwrap());
                    let resource_after = crate::brain_runtime::process_resources().unwrap();
                    eprintln!("local resource interval: wall_us={}, before={}, after={}, prepared={}",
                        pressure_started.elapsed().as_micros(), resource_before, resource_after, prepared);
                    stalled.set(false);
                    if let Some((client, monitor_client, probe)) = &mut duplex {
                        let report = probe.finish();
                        assert!(report["nonzero_playback"].as_u64().unwrap() > 0, "{report}");
                        assert!(report["media"]["captured_packets"].as_u64().unwrap() > 0, "{report}");
                        assert!(report["media"]["monitor_packets"].as_u64().unwrap() > 0, "{report}");
                        let submitted = report["media"]["captured_packets"].as_u64().unwrap();
                        let until = Instant::now() + Duration::from_secs(1);
                        loop {
                            let (accepted, refused) = { let host = host.borrow(); let stats = host.media_stats(); (stats.accepted_talkback_packets, stats.rejected_talkback_packets) };
                            let accounted = accepted + refused;
                            assert!(accounted <= submitted, "duplicate talkback receipts: {report}");
                            if accounted == submitted { break; }
                            assert!(Instant::now() < until, "talkback receipt deficit: submitted={submitted}, accepted={}, refused={}; {report}", accepted, refused);
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                        client.close();
                        monitor_client.close();
                    }
                    if let Some(analysis) = &mut analysis {
                        let report = analysis.finish().unwrap();
                        assert!(report.fault.is_none(), "{:?}", report.fault);
                        assert!(report.packets > 0);
                        assert_eq!(report.mismatches, 0);
                        assert!(report.channel_samples.values().all(|n| *n > 0));
                    }
                    let relay = Relay::start(brain, worker);
                    relay.catch_up(&source).await;
                    let mut desk = connect_desk(address, &client_credentials, client_policy.clone()).await;
                    let (old_request, preparing) = configure(&mut desk, &relay, &source).await;
                    let permitted = reply(&mut desk).await;
                    if permitted["state"] != "permitted" {
                        relay.stop.set(true);
                        let (_, worker) = relay.task.await.unwrap();
                        panic!("{permitted}; execution={:?}; media={:?}", worker.execution_report(), worker.stats);
                    }
                    assert_eq!(permitted["ticket"], preparing["ticket"]);
                    let until = Instant::now() + Duration::from_secs(2);
                    while !relay.commands.borrow().iter().any(|command| matches!(command, OwnerCommand::Permit { .. })) {
                        assert!(Instant::now() < until, "permit delivery timeout");
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    let old_binding = relay.observation.borrow().binding.clone();
                    let old_prepared = relay.messages.borrow().iter().find(|message| matches!(message, OwnerMessage::Prepared { .. })).unwrap().clone();
                    let apply_frame: Counter = serde_json::from_value(preparing["apply_frame"].clone()).unwrap();
                    assert!(relay.observation.borrow().next_source_frame < apply_frame);
                    // Close a permitted future transition before its source frame.
                    let recovery = host
                        .borrow_mut()
                        .recover_source_clock(&source, Counter(10), 0, monotonic_ms())
                        .unwrap();
                    let replacement = recovery.replacement.unwrap();
                    source.close();
                    assert!(host.borrow_mut().finish_source_clock(&source).is_err());
                    relay.stop.set(true);
                    let messages = relay.messages.clone();
                    let (brain, mut worker) = relay.task.await.unwrap();
                    assert!(messages.borrow().iter().any(|message| matches!(message,
                        OwnerMessage::Refused { ticket, reason } if *ticket == Counter(1) && reason == "FX owner closed"
                    )), "{:?}", messages.borrow());
                    assert!(!messages.borrow().iter().any(|message| matches!(message, OwnerMessage::Completed { .. })));
                    let lost = reply(&mut desk).await;
                    assert_eq!(lost["state"], "unknown", "{lost}");
                    assert_eq!(lost["ticket"], preparing["ticket"]);
                    desk.close();
                    assert_eq!(worker.fx_observation().unwrap().generation, Counter(0));
                    let execution = worker.execution_report().unwrap();
                    assert_eq!(execution["terminal_fault"], 0, "{execution}");
                    assert!(worker.stats.processed_blocks > 0);
                    assert!(worker.nonzero_return_samples() > 0);
                    assert_eq!(recovery.previous.fault, None);
                    assert!(recovery.previous.nonzero_wet_samples > 0);
                    assert!(recovery.previous.energy.iter().any(|value| *value > 0.));
                    assert_eq!(
                        host.borrow().provider().engine().clock_status().state,
                        crate::clock_domain::ClockState::Disarmed
                    );

                    let (mut new_brain, mut new_worker) = attach(
                        address, &client_credentials, client_policy.clone(),
                        &manifest.fx.library, &manifest.fx.library_sha256, deadline,
                    ).await;
                    assert_eq!(new_brain.identity().source_epoch, Counter(10));
                    assert_ne!(new_brain.session(), brain.session());
                    let new_observation = ready(&mut new_worker).await;
                    assert_ne!(new_observation.binding, old_binding);
                    // Retain the native stale-binding check as well as testing the
                    // authenticated relay below. Only this negative injection
                    // bypasses the relay; successful mutations all enter via Desk.
                    assert!(matches!(new_worker.fx_command(OwnerCommand::Prepare {
                        ticket: Counter(1),
                        mutation: serde_json::from_value(old_request["body"].clone()).unwrap(),
                        apply_frame: Counter(new_observation.next_source_frame.0 + 4800),
                    }).unwrap(), Some(OwnerMessage::Refused { .. })));
                    // A delayed old Prepared reply cannot attach to a fresh owner.
                    new_brain.send_command(owner_envelope(&new_brain, old_prepared)).await.unwrap();
                    assert!(matches!(new_brain.receive().await.unwrap(), Response::Refused { reason, .. } if reason == "unexpected FX prepared"));
                    assert_eq!(new_worker.fx_observation().unwrap().generation, Counter(0));
                    assert_eq!(host.borrow().provider().engine().clock_status().state, crate::clock_domain::ClockState::Disarmed);
                    let mut new_desk = connect_desk(address, &client_credentials, client_policy.clone()).await;
                    // A fresh authenticated writer cannot replay a previous epoch request.
                    let mut stale = old_request;
                    stale["writer"] = json!(new_desk.writer());
                    new_desk.send_command(stale).await.unwrap();
                    assert!(matches!(new_desk.receive().await.unwrap(), Response::Refused { reason, .. } if reason == "FX source identity"));
                    if combined {
                        let mut rearming = connect_desk(address, &client_credentials, client_policy.clone()).await;
                        authenticated_rearm(&mut rearming).await;
                        rearming.close();
                        assert_eq!(host.borrow().provider().engine().clock_status().state, crate::clock_domain::ClockState::Running);
                        let (mut fresh, monitor, mut probe) = attach_duplex(address, &client_credentials, client_policy.clone(), deadline).await;
                        assert_eq!(fresh.identity().source_epoch, Counter(10));
                        assert_eq!(probe.observation().brain_epoch, 80);
                        nonzero_duplex(&mut fresh, &mut probe).await;
                        fresh.send_command(json!({"contract":"GP15-device","version":1,
                            "kind":"device_observation","writer":fresh.writer(),
                            "observation":old_device.as_ref().unwrap()})).await.unwrap();
                        assert!(matches!(fresh.receive().await.unwrap(), Response::Refused { reason, .. }
                            if reason == "stale device observation"));
                        probe.close_capture();
                        probe.poll();
                        let played = probe.nonzero_playback;
                        let until = Instant::now() + Duration::from_millis(20);
                        while probe.nonzero_playback <= played {
                            assert!(Instant::now() < until, "capture closure stopped selected monitor");
                            tokio::time::sleep(Duration::from_millis(1)).await;
                            probe.poll();
                        }
                        let report = probe.finish();
                        assert!(report["nonzero_playback"].as_u64().unwrap() > 0, "{report}");
                        assert!(report["media"]["captured_packets"].as_u64().unwrap() > 0, "{report}");
                        fresh.close();
                        monitor.close();
                    } else {
                        rearm(&mut host.borrow_mut(), monotonic_ms());
                    }
                    let new_relay = Relay::start(new_brain, new_worker);
                    new_relay.catch_up(&replacement).await;
                    let (_, fresh_preparing) = configure(&mut new_desk, &new_relay, &replacement).await;
                    let fresh_permitted = reply(&mut new_desk).await;
                    if fresh_permitted["state"] != "permitted" {
                        new_relay.stop.set(true);
                        let (_, worker) = new_relay.task.await.unwrap();
                        panic!("{fresh_permitted}; execution={:?}; media={:?}", worker.execution_report(), worker.stats);
                    }
                    let settled = loop {
                        let result = reply(&mut new_desk).await;
                        assert!(["applied", "settled"].contains(&result["state"].as_str().unwrap_or("")), "{result}");
                        assert_eq!(result["ticket"], fresh_preparing["ticket"]);
                        assert_eq!(result["observation"]["applied_source_frame"], fresh_preparing["apply_frame"]);
                        if result["state"] == "settled" { break result; }
                    };
                    assert_eq!(settled["observation"]["generation"], "1");
                    assert_eq!(settled["observation"]["settled_generation"], "1");
                    assert_eq!(settled["observation"]["remaining_frames"], json!([0,0]));
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    new_desk.close();
                    new_relay.stop.set(true);
                    let (_, new_worker) = new_relay.task.await.unwrap();
                    assert!(new_worker.nonzero_return_samples() > 0);
                    assert_eq!(new_worker.execution_report().unwrap()["terminal_fault"], 0);
                    stop.set(true);
                    service.await.unwrap();
                    acceptor.abort();
                    let _ = acceptor.await;
                    let report = host.borrow_mut().finish_source_clock(&replacement).unwrap();
                    assert_eq!(report.fault, None);
                    assert_eq!(report.epoch, 10);
                    assert!(
                        report.output_windows[0]
                            .energy
                            .iter()
                            .all(|value| *value == 0.)
                    );
                    assert!(report.energy.iter().any(|value| *value > 0.));
                    assert!(report.nonzero_wet_samples > 0);
                    let until = Instant::now() + Duration::from_secs(3);
                    loop {
                        let status = host
                            .borrow_mut()
                            .provider_mut()
                            .module_status(monotonic_ms())
                            .unwrap();
                        if status["recording"]["outcome"] == "incomplete" {
                            assert_eq!(status["recording"]["dropped_frames"], "0");
                            break;
                        }
                        assert!(Instant::now() < until, "REC finalization: {status}");
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    let mut stems: Vec<_> =
                        std::fs::read_dir(directory.0.join("takes/tls-recovery"))
                            .unwrap()
                            .map(|entry| entry.unwrap().path())
                            .filter(|path| path.extension().is_some_and(|value| value == "wav"))
                            .collect();
                    stems.sort();
                    assert_eq!(stems.len(), 8);
                    for (channel, stem) in stems.iter().enumerate() {
                        let mut wav = hound::WavReader::open(stem).unwrap();
                        let samples: Vec<i32> =
                            wav.samples().map(std::result::Result::unwrap).collect();
                        let end = recovery.previous.next_frame;
                        assert!(samples.len() >= 64 * 48);
                        let expected: Vec<i32> = (end - 64 * 48..end)
                            .map(|frame| {
                                let sign = if (frame / (24 + channel as u64)).is_multiple_of(2) {
                                    1
                                } else {
                                    -1
                                };
                                sign * (channel as i32 + 1) * 128
                            })
                            .collect();
                        assert_eq!(&samples[samples.len() - expected.len()..], expected);
                    }
                }),
            )
            .await
            .expect("bounded authenticated source recovery");
        }))
        .await;
}
