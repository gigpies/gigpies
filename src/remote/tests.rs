use super::*;
use crate::{
    control_model::{Command, Edit, Request as AudioRequest, Scope, Target, Value as AudioValue},
    mixer_control::OfflineEngine,
    show::Counter,
    topology::EngineTopology,
};
use rustls::pki_types::PrivatePkcs8KeyDer;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
const SHOW: &str = "11111111-1111-4111-8111-111111111111";
fn peer(certificate: &[u8], permissions: &[Permission]) -> Peer {
    Peer {
        id: "brain".into(),
        certificate_sha256: fingerprint(certificate),
        permissions: permissions.iter().cloned().collect(),
    }
}
fn context(permissions: &[Permission]) -> AuthenticatedContext {
    PolicyStore::new(vec![peer(b"unit-certificate", permissions)])
        .unwrap()
        .authenticate(b"unit-certificate", 91)
        .unwrap()
}
fn identity() -> EngineIdentity {
    EngineIdentity {
        source_epoch: Counter(9),
        capability_generation: Counter(1),
        map_generation: Counter(1),
    }
}
fn descriptor(context: &AuthenticatedContext, channels: usize) -> MediaDescriptor {
    let mut streams = grouped_streams(
        MediaRole::Analysis,
        &(1..=channels)
            .map(|n| format!("input-{n:02}"))
            .collect::<Vec<_>>(),
        48,
        0,
        1,
        1100,
    )
    .unwrap();
    let fx = vec!["foh-left".into(), "foh-right".into()];
    streams.extend(grouped_streams(MediaRole::FxSend, &fx, 48, 0, 40, 1100).unwrap());
    streams.extend(grouped_streams(MediaRole::WetReturn, &fx, 48, 384, 50, 1100).unwrap());
    MediaDescriptor {
        session: Counter(context.session()),
        source_epoch: Counter(9),
        capability_generation: Counter(1),
        map_generation: Counter(1),
        sample_rate: 48000,
        streams,
    }
}
#[test]
fn policy_revocation_identity_and_private_bind() {
    let original = peer(b"cert", &[Permission::Foh]);
    let store = PolicyStore::new(vec![original.clone()]).unwrap();
    assert!(store.authenticate(b"wrong", 1).is_err());
    let ctx = store.authenticate(b"cert", 7).unwrap();
    assert!(ctx.require(&Permission::Foh).is_ok());
    assert!(ctx.require(&Permission::PaConfiguration).is_err());
    assert!(
        ctx.check_writer(&json!({"kind":"set", "writer":"spoof"}))
            .is_err()
    );
    assert!(
        ctx.check_writer(&json!({"kind":"processing_snapshot", "writer":null}))
            .is_ok()
    );
    store.replace(vec![original]).unwrap();
    assert!(store.check(&ctx).is_err());
    for address in ["0.0.0.0:9999", "8.8.8.8:9999", "[::]:9999"] {
        assert!(validate_private_address(address.parse().unwrap()).is_err());
    }
    for address in [
        "127.0.0.1:0",
        "192.168.234.221:9999",
        "[::1]:0",
        "[fd00::1]:9999",
    ] {
        assert!(validate_private_address(address.parse().unwrap()).is_ok());
    }
}
#[test]
fn grouping_authentication_replay_epoch_deadline_and_exact_pcm() {
    let ctx = context(&[Permission::Analysis, Permission::Fx]);
    for channels in [16, 32, 48, 53] {
        let d = descriptor(&ctx, channels);
        d.validate(&ctx, &identity(), 1100).unwrap();
        let samples: Vec<f64> = (0..48 * channels).map(|i| i as f64 / 8_388_608.).collect();
        let packets = encode_grouped(&d, MediaRole::Analysis, 96, &samples).unwrap();
        let mut registry = MediaRegistry::new();
        registry
            .install(d.clone(), &ctx, &identity(), 1100)
            .unwrap();
        let mut recovered = vec![0.; samples.len()];
        for bytes in &packets {
            assert!(bytes.len() <= 1100);
            let p = registry.receive(bytes, MediaSide::Brain, None).unwrap();
            for f in 0..48 {
                for c in 0..usize::from(p.spec().channels) {
                    recovered[f * channels + usize::from(p.spec().first_channel) + c] =
                        p.sample(f * usize::from(p.spec().channels) + c).unwrap();
                }
            }
            assert!(registry.receive(bytes, MediaSide::Brain, None).is_err());
        }
        assert_eq!(samples, recovered);
        let wet = encode_grouped(&d, MediaRole::WetReturn, 96, &[0.25; 96])
            .unwrap()
            .remove(0);
        assert!(registry.receive(&wet, MediaSide::Brain, None).is_err());
        assert!(
            registry
                .receive(&wet, MediaSide::ProcessingNode, Some(481))
                .is_err()
        );
        assert!(
            registry
                .receive(&wet, MediaSide::ProcessingNode, Some(480))
                .is_ok()
        );
        let mut wrong = d.clone();
        wrong.source_epoch = Counter(10);
        assert!(registry.install(wrong, &ctx, &identity(), 1100).is_err());
        let mut wrong = d.clone();
        wrong.streams[0].channel_ids[0] = "another".into();
        assert!(registry.install(wrong, &ctx, &identity(), 1100).is_err());
    }
}
fn audio(
    ctx: &AuthenticatedContext,
    command: Command,
    request_id: u64,
    revision: u64,
    lease: Option<Counter>,
) -> Value {
    serde_json::to_value(AudioRequest {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: Some(ctx.writer().into()),
        lease,
        request_id: Some(Counter(request_id)),
        expected_revision: Some(Counter(revision)),
        command,
    })
    .unwrap()
}
fn snapshot_request() -> Value {
    serde_json::to_value(AudioRequest {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(9),
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: Command::Snapshot {},
    })
    .unwrap()
}
fn engine() -> EngineAuthority {
    let engine = OfflineEngine::with_topology(
        SHOW,
        Counter(9),
        Counter(12),
        0,
        EngineTopology::software(48, 4, 8).unwrap(),
    )
    .unwrap();
    EngineAuthority::new(engine, 1).unwrap()
}
#[test]
fn real_authority_permission_ceiling_and_proxy_disconnect_cancels_pending() {
    let ctx = context(&[Permission::Foh]);
    let mut owner = engine();
    owner.engine_mut().rearm().unwrap();
    owner.dispatch_command(&ctx, snapshot_request(), 0).unwrap();
    let denied = audio(
        &ctx,
        Command::Grant {
            scope: Scope::PaConfiguration,
        },
        1,
        12,
        None,
    );
    assert!(owner.dispatch_command(&ctx, denied, 0).is_err());
    let grant = owner
        .dispatch_command(
            &ctx,
            audio(&ctx, Command::Grant { scope: Scope::Foh }, 1, 12, None),
            0,
        )
        .unwrap();
    let lease = grant["outcome"]["body"]["granted_lease"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let revision = grant["outcome"]["body"]["revision"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let (mut proxy, mut mailbox) = authority_channel(&ctx, owner.identity()).unwrap();
    let request = audio(
        &ctx,
        Command::Set {
            targets: vec![Edit {
                target: Target::Fader {
                    input: "input-48".into(),
                },
                value: AudioValue::Integer(-3000),
            }],
        },
        2,
        revision,
        Some(Counter(lease)),
    );
    assert!(proxy.dispatch(&ctx, request, 1).unwrap().is_none());
    mailbox.service(&mut owner, 1).unwrap();
    let pending = proxy.poll_reply(&ctx, 1).unwrap().unwrap();
    assert_eq!(pending["state"], "pending");
    proxy.disconnect(&ctx);
    mailbox.service(&mut owner, 2).unwrap();
    assert!(mailbox.retired());
    let mut output = vec![0.; 48 * 6];
    owner
        .process_interleaved(&vec![0.; 48 * 48], &mut output, 3)
        .unwrap();
    assert_eq!(
        owner.engine().mixer().targets()[47][0],
        10_f64.powf(-6. / 20.)
    );
}
fn credentials() -> (Credentials, Credentials) {
    use rcgen::{
        BasicConstraints, CertificateParams, CertifiedIssuer, ExtendedKeyUsagePurpose, IsCa,
        KeyPair, KeyUsagePurpose,
    };
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
    let issue = |name: &str| {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![name.into()]).unwrap();
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        let certificate = params.signed_by(&key, &ca).unwrap();
        Credentials {
            certificate_chain: vec![certificate.der().clone()],
            private_key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            trust_roots: vec![ca.der().clone()],
        }
    };
    (issue("stagebox.test"), issue("brain.test"))
}
#[tokio::test(flavor = "current_thread")]
async fn mutual_tls_actual_high_channel_authority_media_and_revocation() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (server_credentials, client_credentials) = credentials();
        let server_policy = PolicyStore::new(vec![peer(
            client_credentials.certificate_chain[0].as_ref(),
            &[Permission::Foh, Permission::Analysis, Permission::Fx],
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
            server_policy.clone(),
        )
        .unwrap();
        let address = server.local_addr().unwrap();
        let client_endpoint =
            client_endpoint("127.0.0.1:0".parse().unwrap(), &client_credentials).unwrap();
        let peak = Arc::new(AtomicU64::new(0));
        let retired = Arc::new(AtomicBool::new(false));
        let wet_received = Arc::new(AtomicU64::new(0));
        let wet_owner = wet_received.clone();
        let peak_owner = peak.clone();
        let retired_owner = retired.clone();
        let server_task = tokio::spawn(async move {
            let session = server.accept().await.unwrap();
            let mut owner = engine();
            owner.engine_mut().rearm().unwrap();
            let (mut proxy, mut mailbox) =
                authority_channel(session.context(), owner.identity()).unwrap();
            let network = tokio::spawn(async move { session.run(&mut proxy).await });
            let start = std::time::Instant::now();
            let mut timer = tokio::time::interval(Duration::from_millis(5));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut input = vec![0.; 48 * 48];
            for f in 0..48 {
                input[f * 48 + 47] = 0.25;
            }
            let mut output = vec![0.; 48 * 6];
            loop {
                timer.tick().await;
                mailbox
                    .service(&mut owner, start.elapsed().as_millis() as u64)
                    .unwrap();
                if mailbox.retired() {
                    retired_owner.store(true, Ordering::Release);
                    break;
                }
                while owner.take_media().is_some() {
                    wet_owner.fetch_add(1, Ordering::AcqRel);
                }
                let frame = owner.engine().frame();
                owner
                    .process_interleaved(&input, &mut output, start.elapsed().as_millis() as u64)
                    .unwrap();
                if let Some(descriptor) = owner.descriptor().cloned() {
                    for packet in
                        encode_grouped(&descriptor, MediaRole::Analysis, frame, &input).unwrap()
                    {
                        owner.queue_media(packet).unwrap();
                    }
                    let fx: Vec<_> = output
                        .chunks_exact(6)
                        .flat_map(|row| row[..2].iter().copied())
                        .collect();
                    for packet in
                        encode_grouped(&descriptor, MediaRole::FxSend, frame, &fx).unwrap()
                    {
                        owner.queue_media(packet).unwrap();
                    }
                }
                peak_owner.store(output[output.len() - 6].to_bits(), Ordering::Release);
            }
            let _ = network.await;
        });
        let mut client =
            RemoteClient::connect(client_endpoint, address, "stagebox.test", client_policy)
                .await
                .unwrap();
        client.send_command(snapshot_request()).await.unwrap();
        assert!(matches!(
            client.receive().await.unwrap(),
            Response::Reply { .. }
        ));
        let ctx = context(&[Permission::Foh]);
        let request = |command, id, revision, lease| {
            let mut v = audio(&ctx, command, id, revision, lease);
            v["writer"] = json!(client.writer());
            v
        };
        client
            .send_command(request(Command::Grant { scope: Scope::Foh }, 1, 12, None))
            .await
            .unwrap();
        let Response::Reply { payload: grant, .. } = client.receive().await.unwrap() else {
            panic!("grant reply")
        };
        let lease = grant["outcome"]["body"]["granted_lease"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let revision = grant["outcome"]["body"]["revision"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let mut set = audio(
            &ctx,
            Command::Set {
                targets: vec![Edit {
                    target: Target::Fader {
                        input: "input-48".into(),
                    },
                    value: AudioValue::Integer(0),
                }],
            },
            2,
            revision,
            Some(Counter(lease)),
        );
        set["writer"] = json!(client.writer());
        client.send_command(set).await.unwrap();
        let Response::Reply {
            payload: pending, ..
        } = client.receive().await.unwrap()
        else {
            panic!("pending reply")
        };
        assert_eq!(pending["state"], "pending");
        let Response::Reply {
            payload: final_reply,
            ..
        } = client.receive().await.unwrap()
        else {
            panic!("final reply")
        };
        assert_eq!(final_reply["state"], "final");
        assert!(
            final_reply["effective_frame"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
                > 0
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        let actual = f64::from_bits(peak.load(Ordering::Acquire));
        assert!(
            (actual - 0.25 / 2_f64.sqrt()).abs() < 1e-12,
            "actual high-channel output {actual}"
        );
        let snapshot = serde_json::to_value(AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: None,
            lease: None,
            request_id: None,
            expected_revision: None,
            command: Command::Snapshot {},
        })
        .unwrap();
        client.send_command(snapshot).await.unwrap();
        let Response::Reply {
            payload: snapshot, ..
        } = client.receive().await.unwrap()
        else {
            panic!("snapshot reply")
        };
        assert_eq!(
            snapshot["snapshot"]["authority"]["inputs"]
                .as_array()
                .unwrap()
                .len(),
            48
        );
        let mut media_context = context(&[Permission::Analysis, Permission::Fx]);
        media_context.session = client.session();
        let negotiated = descriptor(&media_context, 48);
        client.negotiate(negotiated.clone()).await.unwrap();
        assert!(matches!(
            client.receive().await.unwrap(),
            Response::MediaAccepted { .. }
        ));
        let mut media = client.media_channel().unwrap();
        let source_frame = loop {
            let bytes = media.receive().await.unwrap();
            let packet = crate::transport::Packet::parse(&bytes).unwrap();
            if packet.spec().role == crate::transport::Role::FxSend {
                break packet.source_frame();
            }
        };
        let wet = encode_grouped(
            &negotiated,
            MediaRole::WetReturn,
            source_frame,
            &[0.125; 96],
        )
        .unwrap();
        for packet in wet {
            media.send(packet).unwrap();
        }
        for _ in 0..100 {
            if wet_received.load(Ordering::Acquire) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(wet_received.load(Ordering::Acquire) > 0);
        server_policy.replace(vec![]).unwrap();
        assert!(client.receive().await.is_err());
        server_task.await.unwrap();
        assert!(retired.load(Ordering::Acquire));
    })
    .await
    .unwrap();
}
#[test]
fn strict_wire_and_peer_bound_authority() {
    assert!(
        decode::<Request>(br#"{"kind":"open","contract":"GP-REMOTE","version":1,"version":1}"#)
            .is_err()
    );
    assert!(
        decode::<Request>(br#"{"kind":"open","contract":"GP-REMOTE","version":1,"extra":true}"#)
            .is_err()
    );
    let ctx = context(&[Permission::Foh]);
    let mut owner = engine();
    let mut request = audio(&ctx, Command::Grant { scope: Scope::Foh }, 1, 12, None);
    request["writer"] = json!("spoofed");
    assert!(owner.dispatch_command(&ctx, request, 0).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn valid_ca_without_pairing_and_wrong_ca_are_refused() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (server_credentials, permitted_client) = credentials();
        let policy = PolicyStore::new(vec![peer(
            permitted_client.certificate_chain[0].as_ref(),
            &[Permission::Foh],
        )])
        .unwrap();
        let server =
            RemoteServer::bind("127.0.0.1:0".parse().unwrap(), &server_credentials, policy)
                .unwrap();
        // This leaf is signed by the trusted CA and valid for client auth, but
        // only the other individual leaf was paired. CA trust grants no writes.
        let endpoint =
            client_endpoint("127.0.0.1:0".parse().unwrap(), &server_credentials).unwrap();
        let client_policy = PolicyStore::new(vec![peer(
            server_credentials.certificate_chain[0].as_ref(),
            &[],
        )])
        .unwrap();
        let (accepted, connected) = tokio::join!(
            server.accept(),
            RemoteClient::connect(
                endpoint,
                server.local_addr().unwrap(),
                "stagebox.test",
                client_policy
            )
        );
        assert!(matches!(accepted, Err(error) if error.contains("not paired")));
        assert!(connected.is_err());
        let (_, foreign) = credentials();
        let endpoint = client_endpoint("127.0.0.1:0".parse().unwrap(), &foreign).unwrap();
        let policy = PolicyStore::new(vec![peer(
            server_credentials.certificate_chain[0].as_ref(),
            &[],
        )])
        .unwrap();
        let (accepted, connected) = tokio::join!(
            server.accept(),
            RemoteClient::connect(
                endpoint,
                server.local_addr().unwrap(),
                "stagebox.test",
                policy
            )
        );
        assert!(accepted.is_err());
        assert!(connected.is_err());
    })
    .await
    .unwrap();
}

#[test]
fn proxy_capacity_snapshot_gate_and_generation_final_are_bounded() {
    let ctx = context(&[Permission::Foh]);
    let mut owner = engine();
    assert!(
        owner
            .dispatch_command(
                &ctx,
                audio(&ctx, Command::Grant { scope: Scope::Foh }, 1, 12, None),
                0
            )
            .is_err()
    );
    let (mut proxy, mut mailbox) = authority_channel(&ctx, owner.identity()).unwrap();
    for _ in 0..16 {
        assert!(
            proxy
                .dispatch(&ctx, snapshot_request(), 0)
                .unwrap()
                .is_none()
        );
    }
    assert!(proxy.dispatch(&ctx, snapshot_request(), 0).is_err());
    mailbox.service(&mut owner, 0).unwrap();
    // Already-applied replies remain drainable as a generation retires. New
    // queued intent is rejected and cannot run against the replacement epoch.
    owner.engine_mut().recover(Counter(10), 0).unwrap();
    mailbox.service(&mut owner, 1).unwrap();
    assert!(mailbox.retired());
    assert!(proxy.retiring());
    assert!(proxy.dispatch(&ctx, snapshot_request(), 1).is_err());
    let mut count = 0;
    while proxy.poll_reply(&ctx, 1).unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 16);
}

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
struct PrivateTestDirectory(std::path::PathBuf);
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
impl PrivateTestDirectory {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "gigpies-remote-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
impl Drop for PrivateTestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[test]
fn composed_host_noncontiguous_analysis_deadlines_loss_and_epoch() {
    let directory = PrivateTestDirectory::new();
    let mut topology = EngineTopology::reference_16_18(0, 4).unwrap();
    topology.outputs[0].source = Some(crate::topology::OutputSource::Main { channel: 0 });
    let mut provider = crate::local_audio::LocalAudio::bind_configured(
        &directory.0,
        "host.sock",
        SHOW,
        Counter(9),
        topology.clone(),
    )
    .unwrap();
    provider.rearm().unwrap();
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let ctx = context(&[Permission::Analysis, Permission::Fx]);
    let descriptor = descriptor(&ctx, 16);
    assert!(host.negotiate(&ctx, &descriptor).unwrap());
    let wet = encode_grouped(&descriptor, MediaRole::WetReturn, 0, &[0.25; 96])
        .unwrap()
        .remove(0);
    host.media(&ctx, crate::transport::Packet::parse(&wet).unwrap(), 0)
        .unwrap();
    host.media(&ctx, crate::transport::Packet::parse(&wet).unwrap(), 0)
        .unwrap();
    assert_eq!(host.media_stats().accepted_wet_packets, 1);
    assert_eq!(host.media_stats().rejected_wet_packets, 1);
    let mut capture = vec![0.; 48 * topology.capture_channels];
    for frame in 0..48 {
        for (index, input) in topology.inputs.iter().enumerate() {
            capture[frame * topology.capture_channels + input.capture_slot] =
                (index + 1) as f64 / 8_388_608.;
        }
    }
    let mut playback = vec![0.; 48 * topology.playback_channels];
    for block in 0..9 {
        host.process_source(block, 9, block * 48, &capture, &mut playback)
            .unwrap();
    }
    assert!(host.media_stats().nonzero_rendered_wet_samples > 0);
    assert!(playback.iter().any(|sample| *sample > 0.));
    let mut seen = std::collections::BTreeSet::new();
    while let Some(bytes) = host.poll_media(&ctx).unwrap() {
        let packet = crate::transport::Packet::parse(&bytes).unwrap();
        if packet.spec().role == crate::transport::Role::Analysis {
            for channel in 0..usize::from(packet.spec().channels) {
                let index = usize::from(packet.spec().first_channel) + channel;
                assert_eq!(packet.pcm(channel).unwrap(), (index + 1) as i32);
                seen.insert(index);
            }
        }
    }
    assert_eq!(seen.len(), 16);
    host.disconnect(&ctx);
    assert!(
        host.media(&ctx, crate::transport::Packet::parse(&wet).unwrap(), 9)
            .is_err()
    );
    for block in 9..16 {
        host.process_source(block, 9, block * 48, &capture, &mut playback)
            .unwrap();
        assert!(playback.iter().any(|sample| *sample > 0.));
    }
    assert_eq!(host.provider().frame(), 16 * 48);
    host.provider_mut().recover_source(Counter(10), 0).unwrap();
    assert!(host.negotiate(&ctx, &descriptor).is_err());
    let mut renewed = descriptor.clone();
    renewed.source_epoch = Counter(10);
    renewed.map_generation = host.identity().map_generation;
    assert!(host.negotiate(&ctx, &renewed).unwrap());
    host.provider_mut().rearm().unwrap();
    host.process_source(18, 10, 0, &capture, &mut playback)
        .unwrap();
    assert!(host.poll_media(&ctx).unwrap().is_some());
    assert_eq!(
        host.provider_mut().engine_mut().clock_status().state,
        crate::clock_domain::ClockState::Running
    );
}

#[test]
fn revoked_policy_cancels_already_queued_authority_command() {
    let policy = PolicyStore::new(vec![peer(b"queued", &[Permission::Foh])]).unwrap();
    let ctx = policy.authenticate(b"queued", 991).unwrap();
    let mut owner = engine();
    owner.dispatch_command(&ctx, snapshot_request(), 0).unwrap();
    let (mut proxy, mut mailbox) = authority_channel(&ctx, owner.identity()).unwrap();
    proxy
        .dispatch(
            &ctx,
            audio(&ctx, Command::Grant { scope: Scope::Foh }, 1, 12, None),
            0,
        )
        .unwrap();
    policy.replace(vec![]).unwrap();
    mailbox.service(&mut owner, 0).unwrap();
    assert!(mailbox.retired());
    assert!(ctx.require(&Permission::Foh).is_err());
    assert!(proxy.poll_reply(&ctx, 0).is_err());
}

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[test]
#[ignore = "explicit accepted owner libraries; private synthetic REC evidence"]
fn actual_host_recording_survives_brain_loss_and_finalizes_clock_fault() {
    let directory = PrivateTestDirectory::new();
    let topology = EngineTopology::software(48, 4, 8).unwrap();
    let mut provider = crate::local_audio::LocalAudio::bind_configured(
        &directory.0,
        "record.sock",
        SHOW,
        Counter(9),
        topology.clone(),
    )
    .unwrap();
    provider
        .enable_modules(std::path::Path::new(
            &std::env::var("GP05_MANIFEST").unwrap(),
        ))
        .unwrap();
    super::runner::start_recording(&mut provider, SHOW, "remote-loss", 0).unwrap();
    provider.rearm().unwrap();
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let ctx = context(&[Permission::Analysis, Permission::Fx]);
    host.negotiate(&ctx, &descriptor(&ctx, 48)).unwrap();
    let capture = vec![1. / 8388608.; 48 * topology.capture_channels];
    let mut output = vec![0.; 48 * topology.playback_channels];
    for block in 0..100u64 {
        if block == 20 {
            host.disconnect(&ctx);
        }
        host.process_source(block, 9, block * 48, &capture, &mut output)
            .unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    let before = host.provider_mut().module_status(100).unwrap();
    assert_eq!(before["recording"]["state"], "recording", "{before}");
    assert_eq!(host.provider().frame(), 4800);
    host.provider_mut()
        .quiesce_source("test_clock_fault")
        .unwrap();
    let mut final_status = Value::Null;
    for _ in 0..200 {
        final_status = host.provider_mut().module_status(101).unwrap();
        if final_status["recording"]["outcome"] == "incomplete" {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        final_status["recording"]["outcome"], "incomplete",
        "{final_status}"
    );
    assert_eq!(host.provider().frame(), 4800);
}

#[test]
fn unsupported_processing_body_receives_typed_version_refusal() {
    let ctx = context(&[Permission::Foh]);
    let mut owner = engine();
    let payload = json!({"contract":"GP07-processing","version":2,"show_id":SHOW,"module":"audio","epoch":"9","writer":ctx.writer(),"lease":"1","request_id":"1","expected_revision":"12","kind":"processing_set","body":{"legacy_shape":"not a v3 configuration"}});
    let reply = owner.dispatch_command(&ctx, payload, 0).unwrap();
    assert_eq!(reply["contract"], "GP07-processing");
    assert_eq!(reply["version"], 2);
    assert_eq!(reply["reason"], "unsupported_version");
    assert_eq!(reply["state"], "final");
}

#[test]
#[ignore = "private consumer fixture from actual dynamic authority and production page encoder"]
fn write_actual_remote48_response_pages() {
    use sha2::{Digest, Sha256};
    let root = std::path::PathBuf::from(std::env::var("GP14_REMOTE_FIXTURES").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    let ctx = context(&[Permission::Foh]);
    let mut owner = engine();
    let payload = owner.dispatch_command(&ctx, snapshot_request(), 0).unwrap();
    let response = Response::Reply {
        session: Counter(ctx.session()),
        payload,
    };
    let whole = serde_json::to_vec(&response).unwrap();
    assert!(whole.len() > MAX_FRAME);
    let pages = crate::snapshot_pages::encode(whole.clone()).unwrap();
    let mut framed = Vec::new();
    for page in &pages {
        framed.extend_from_slice(&(page.len() as u32).to_be_bytes());
        framed.extend_from_slice(page);
    }
    std::fs::write(root.join("response-whole.json"), &whole).unwrap();
    std::fs::write(root.join("response-framed.bin"), &framed).unwrap();
    let manifest = json!({"source":"actual EngineAuthority48 snapshot + typed Response::Reply + production snapshot_pages::encode","session":ctx.session().to_string(),"page_count":pages.len(),"whole_bytes":whole.len(),"framed_bytes":framed.len(),"whole_sha256":format!("{:x}",Sha256::digest(&whole)),"framed_sha256":format!("{:x}",Sha256::digest(&framed))});
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn scoped_controllers_workers_and_reconnect_share_bounded_tls_admission() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (server_credentials, client_credentials) = credentials();
        let policy = PolicyStore::new(vec![peer(
            client_credentials.certificate_chain[0].as_ref(),
            &[
                Permission::LocalOperatorMonitor,
                Permission::TalkbackDestinations,
                Permission::TalkbackFoh,
                Permission::Fx,
                Permission::Analysis,
            ],
        )])
        .unwrap();
        let server =
            RemoteServer::bind("127.0.0.1:0".parse().unwrap(), &server_credentials, policy)
                .unwrap();
        let endpoint =
            client_endpoint("127.0.0.1:0".parse().unwrap(), &client_credentials).unwrap();
        let address = server.local_addr().unwrap();
        let mut sessions = Vec::new();
        let mut connections = Vec::new();
        // Concrete reviewed deployment budget, including the five simultaneously
        // required Brain/Desk/FX roles that the historical four-slot cap rejected.
        for _ in 0..16 {
            let connecting = endpoint.connect(address, "stagebox.test").unwrap();
            let (accepted, connected) = tokio::join!(server.accept(), connecting);
            sessions.push(accepted.unwrap());
            connections.push(connected.unwrap());
        }
        let identities: std::collections::BTreeSet<_> =
            sessions.iter().map(|s| s.context().session()).collect();
        assert_eq!(
            identities.len(),
            16,
            "each role has an independent authenticated session"
        );
        let connecting = endpoint.connect(address, "stagebox.test").unwrap();
        let (refused, connected) = tokio::join!(server.accept(), connecting);
        assert!(matches!(refused, Err(e) if e == "remote connection capacity"));
        assert!(connected.is_err());
        assert!(
            connections.iter().all(|c| c.close_reason().is_none()),
            "capacity refusal must not evict existing workers/controllers"
        );
        let retired = sessions.pop().unwrap().context().session();
        connections
            .pop()
            .unwrap()
            .close(0u8.into(), b"explicit reconnect");
        let connecting = endpoint.connect(address, "stagebox.test").unwrap();
        let (accepted, connected) = tokio::join!(server.accept(), connecting);
        let replacement = accepted.unwrap();
        assert_ne!(replacement.context().session(), retired);
        assert!(!identities.contains(&replacement.context().session()));
        let replacement_connection = connected.unwrap();
        assert!(sessions.iter().all(|s| s.context().check_current().is_ok()));
        replacement_connection.close(0u8.into(), b"test complete");
        endpoint.close(0u8.into(), b"test complete");
        server.close();
    })
    .await
    .unwrap();
}

fn sends_read() -> Value {
    json!({"contract":"GP18-sends","version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"sends_snapshot","body":{}})
}
fn sends_set(writer: &str, lease: &str, id: u64, revision: u64) -> Value {
    json!({"contract":"GP18-sends","version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":writer,"lease":lease,"request_id":id.to_string(),"expected_revision":revision.to_string(),"kind":"send_tap_set","body":{"input":"input-48","monitor":"monitor-3","tap":"processed_post_fader"}})
}
#[test]
fn sends_authenticated_scope_freshness_completions_reconnect_and_revocation() {
    let ctx = context(&[Permission::Monitor(3)]);
    let mut owner = engine();
    owner.dispatch_command(&ctx, snapshot_request(), 0).unwrap();
    let grant = owner
        .dispatch_command(
            &ctx,
            audio(
                &ctx,
                Command::Grant {
                    scope: Scope::Monitor(3),
                },
                1,
                12,
                None,
            ),
            0,
        )
        .unwrap();
    let lease = grant["outcome"]["body"]["granted_lease"].as_str().unwrap();
    let stale = owner
        .dispatch_command(&ctx, sends_set(ctx.writer(), lease, 2, 12), 1)
        .unwrap();
    assert_eq!(stale["reason"], "stale_snapshot");
    let read = owner.dispatch_command(&ctx, sends_read(), 2).unwrap();
    assert!(read["snapshot"].is_object());
    let mut forged = sends_read();
    forged["writer"] = json!("forged-writer");
    assert!(owner.dispatch_command(&ctx, forged, 2).is_err());
    let pending = owner
        .dispatch_command(&ctx, sends_set(ctx.writer(), lease, 3, 12), 3)
        .unwrap();
    assert_eq!(pending["state"], "pending");
    let mut output = vec![0.; 96 * 6];
    owner
        .process_interleaved(&vec![0.; 96 * 48], &mut output, 4)
        .unwrap();
    let final_reply = owner.poll_reply(&ctx, 4).unwrap().unwrap();
    assert_eq!(final_reply["ticket"], pending["ticket"]);
    assert_eq!(final_reply["revision"], "13");
    assert_eq!(
        owner
            .dispatch_command(&ctx, sends_set(ctx.writer(), lease, 3, 12), 5)
            .unwrap(),
        final_reply
    );
    let mut wrong = sends_set(ctx.writer(), lease, 4, 13);
    wrong["body"]["monitor"] = json!("monitor-2");
    assert!(owner.dispatch_command(&ctx, wrong, 5).is_err());
    owner
        .process_interleaved(&vec![0.; 240 * 48], &mut vec![0.; 240 * 6], 6)
        .unwrap();
    assert_eq!(
        owner
            .dispatch_command(&ctx, sends_set(ctx.writer(), lease, 4, 13), 253)
            .unwrap()["reason"],
        "stale_snapshot"
    );
    owner.dispatch_command(&ctx, sends_read(), 254).unwrap();
    let mut queued = sends_set(ctx.writer(), lease, 5, 13);
    queued["body"]["tap"] = json!("raw_post_mute");
    assert_eq!(
        owner.dispatch_command(&ctx, queued, 255).unwrap()["state"],
        "pending"
    );
    owner.disconnect(&ctx);
    owner
        .process_interleaved(&vec![0.; 96 * 48], &mut vec![0.; 96 * 6], 256)
        .unwrap();
    assert_eq!(owner.engine().revision(), Counter(13));
    assert_eq!(
        owner.engine().mixer().send_observations()[47][2].1,
        crate::sends_wire::Tap::ProcessedPostFader
    );
    assert!(
        owner
            .dispatch_command(&ctx, sends_set(ctx.writer(), lease, 5, 13), 257)
            .is_err()
    );
    let foh = context(&[Permission::Foh]);
    assert!(
        owner
            .dispatch_command(&foh, sends_set(foh.writer(), lease, 6, 13), 258)
            .is_err()
    );
}
#[tokio::test(flavor = "current_thread")]
async fn sends_actual_mutual_tls_provider_boundary_and_policy_revocation() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (server_credentials, client_credentials) = credentials();
        let server_policy = PolicyStore::new(vec![peer(
            client_credentials.certificate_chain[0].as_ref(),
            &[Permission::Monitor(3)],
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
            server_policy.clone(),
        )
        .unwrap();
        let address = server.local_addr().unwrap();
        let endpoint =
            client_endpoint("127.0.0.1:0".parse().unwrap(), &client_credentials).unwrap();
        let retired = Arc::new(AtomicBool::new(false));
        let flag = retired.clone();
        let server_task = tokio::spawn(async move {
            let session = server.accept().await.unwrap();
            let mut owner = engine();
            let (mut proxy, mut mailbox) =
                authority_channel(session.context(), owner.identity()).unwrap();
            let network = tokio::spawn(async move { session.run(&mut proxy).await });
            let start = std::time::Instant::now();
            let mut timer = tokio::time::interval(Duration::from_millis(5));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let input = vec![0.25; 48 * 48];
            let mut output = vec![0.; 48 * 6];
            loop {
                timer.tick().await;
                let now = start.elapsed().as_millis() as u64;
                mailbox.service(&mut owner, now).unwrap();
                if mailbox.retired() {
                    flag.store(true, Ordering::Release);
                    break;
                }
                owner.process_interleaved(&input, &mut output, now).unwrap();
            }
            let _ = network.await;
            assert_eq!(owner.engine().revision(), Counter(13));
            assert_eq!(
                owner.engine().mixer().send_observations()[47][2].1,
                crate::sends_wire::Tap::ProcessedPostFader
            );
        });
        let mut client = RemoteClient::connect(endpoint, address, "stagebox.test", client_policy)
            .await
            .unwrap();
        client.send_command(snapshot_request()).await.unwrap();
        client.receive().await.unwrap();
        let ctx = context(&[Permission::Monitor(3)]);
        let mut grant = audio(
            &ctx,
            Command::Grant {
                scope: Scope::Monitor(3),
            },
            1,
            12,
            None,
        );
        grant["writer"] = json!(client.writer());
        client.send_command(grant).await.unwrap();
        let Response::Reply { payload: grant, .. } = client.receive().await.unwrap() else {
            panic!("grant")
        };
        let lease = grant["outcome"]["body"]["granted_lease"].as_str().unwrap();
        client.send_command(sends_read()).await.unwrap();
        let Response::Reply { payload: read, .. } = client.receive().await.unwrap() else {
            panic!("read")
        };
        assert_eq!(read["snapshot"]["channels"].as_array().unwrap().len(), 48);
        client
            .send_command(sends_set(client.writer(), lease, 2, 12))
            .await
            .unwrap();
        let Response::Reply {
            payload: pending, ..
        } = client.receive().await.unwrap()
        else {
            panic!("pending")
        };
        assert_eq!(pending["state"], "pending");
        let Response::Reply {
            payload: final_reply,
            ..
        } = client.receive().await.unwrap()
        else {
            panic!("final")
        };
        assert_eq!(final_reply["revision"], "13");
        assert_eq!(pending["ticket"], final_reply["ticket"]);
        server_policy.replace(vec![]).unwrap();
        server_task.await.unwrap();
        assert!(retired.load(Ordering::Acquire));
    })
    .await
    .expect("bounded authenticated sends test");
}

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[path = "gp21_tests.rs"]
mod gp21_tests;

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[tokio::test(flavor = "current_thread")]
async fn mutual_tls_meter_reads_actual_host_and_revocation_without_control_admission() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let directory = PrivateTestDirectory::new();
        let topology = EngineTopology::software(48, 7, 0).unwrap();
        let mut provider = crate::local_audio::LocalAudio::bind_configured(
            &directory.0,
            "meter.sock",
            SHOW,
            Counter(9),
            topology.clone(),
        )
        .unwrap();
        provider.rearm().unwrap();
        let capture = vec![0.5; 48 * topology.capture_channels];
        let mut playback = vec![0.; 48 * topology.playback_channels];
        for block in 0..40 {
            provider
                .tick_with_capture(block, 9, block * 48, &capture, &mut playback)
                .unwrap();
        }
        let mut host = HostAuthority::new(provider, 1).unwrap();
        let (server_credentials, client_credentials) = credentials();
        let policy = PolicyStore::new(vec![peer(
            client_credentials.certificate_chain[0].as_ref(),
            &[],
        )])
        .unwrap();
        let paired = PolicyStore::new(vec![peer(
            server_credentials.certificate_chain[0].as_ref(),
            &[],
        )])
        .unwrap();
        let server = RemoteServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            &server_credentials,
            policy.clone(),
        )
        .unwrap();
        let address = server.local_addr().unwrap();
        let endpoint =
            client_endpoint("127.0.0.1:0".parse().unwrap(), &client_credentials).unwrap();
        let server_task = async move {
            let session = server.accept().await.unwrap();
            let _ = session.run(&mut host).await;
            assert_eq!(host.provider_mut().engine_mut().revision(), Counter(0));
        };
        let client_task = async move {
            let mut client = RemoteClient::connect(endpoint, address, "stagebox.test", paired)
                .await
                .unwrap();
            let request = crate::meter_wire::Request::new(SHOW, 9, 1, 1);
            client
                .send_command(serde_json::to_value(&request).unwrap())
                .await
                .unwrap();
            let Response::Reply { payload, .. } = client.receive().await.unwrap() else {
                panic!("actual meter reply required")
            };
            let s = crate::meter_wire::Snapshot::decode(&serde_json::to_vec(&payload).unwrap())
                .unwrap();
            assert!(s.valid);
            assert_eq!(s.taps[94].peak_millidbfs, Some(-6021));
            assert_eq!(s.taps.len(), 105);
            // No render ticks run while this authenticated control session stays
            // alive. Another read must retain the original acquisition age/window.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut repeat = request.clone();
            repeat.query_id = Counter(2);
            client
                .send_command(serde_json::to_value(&repeat).unwrap())
                .await
                .unwrap();
            let Response::Reply { payload, .. } = client.receive().await.unwrap() else {
                panic!("stopped source still has bounded observational readback")
            };
            let stale = crate::meter_wire::Snapshot::decode(&serde_json::to_vec(&payload).unwrap())
                .unwrap();
            assert_eq!(stale.sequence, s.sequence);
            assert_eq!(stale.first_frame, s.first_frame);
            assert_eq!(stale.taps, s.taps);
            assert!(stale.acquisition_age_ms.unwrap().0 > 250);
            policy.replace(vec![]).unwrap();
            let mut request = request;
            request.query_id = Counter(3);
            let _ = client
                .send_command(serde_json::to_value(&request).unwrap())
                .await;
            assert!(!matches!(
                client.receive().await,
                Ok(Response::Reply { .. })
            ));
            client.close();
        };
        tokio::join!(server_task, client_task);
    })
    .await
    .unwrap();
}

/// Cross-repository acceptance uses the prebuilt Desk integration-test executable.
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[tokio::test(flavor = "current_thread")]
#[ignore = "actual Desk consumer; set GP_DESK_SPECTRUM_TEST to its tests/spectrum executable"]
async fn actual_remote_spectrum_desk() {
    use std::{os::unix::fs::PermissionsExt, time::Instant};
    let directory = PrivateTestDirectory::new();
    let (server_credentials, client_credentials) = credentials();
    for (name, data) in [
        (
            "client.der",
            client_credentials.certificate_chain[0].as_ref(),
        ),
        ("client.key", client_credentials.private_key.secret_der()),
        ("ca.der", client_credentials.trust_roots[0].as_ref()),
    ] {
        let path = directory.0.join(name);
        std::fs::write(&path, data).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let policy = PolicyStore::new(vec![peer(
        client_credentials.certificate_chain[0].as_ref(),
        &[Permission::Foh],
    )])
    .unwrap();
    let server =
        RemoteServer::bind("127.0.0.1:0".parse().unwrap(), &server_credentials, policy).unwrap();
    let config = json!({"bind":"127.0.0.1:0", "server":server.local_addr().unwrap().to_string(), "server_name":"stagebox.test", "certificate":directory.0.join("client.der"), "private_key":directory.0.join("client.key"), "ca":directory.0.join("ca.der"), "server_certificate_sha256":fingerprint(server_credentials.certificate_chain[0].as_ref()), "peer_id":"brain"});
    let path = directory.0.join("desk.json");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let mut topology = EngineTopology::reference_16_18(0, 2).unwrap();
    topology.outputs[0].source = Some(crate::topology::OutputSource::Main { channel: 0 });
    let mut provider = crate::local_audio::LocalAudio::bind_configured(
        &directory.0,
        "spectrum.sock",
        SHOW,
        Counter(1),
        topology,
    )
    .unwrap();
    provider.rearm().unwrap();
    let t = provider.topology().clone();
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let mut capture = vec![0.; 48 * t.capture_channels];
    let mut playback = vec![0.; 48 * t.playback_channels];
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let acceptor = tokio::spawn(async move {
        while let Ok(session) = server.accept().await {
            if tx.send(session).await.is_err() {
                break;
            }
        }
    });
    let mut mailboxes = Vec::new();
    let mut networks = Vec::new();
    let mut child = std::process::Command::new(
        std::env::var("GP_DESK_SPECTRUM_TEST").expect("prebuilt Desk spectrum test"),
    )
    .args([
        "--exact",
        "actual_remote_provider_spectrum",
        "--ignored",
        "--nocapture",
    ])
    .env("GP_SPECTRUM_REMOTE_CONFIG", path)
    .spawn()
    .unwrap();
    let started = Instant::now();
    let mut timer = tokio::time::interval(Duration::from_millis(1));
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(15) {
            child.kill().unwrap();
            panic!("bounded remote spectrum acceptance timed out");
        }
        tokio::select! {
            Some(session)=rx.recv()=>{
                let (mut proxy,mailbox)=authority_channel(session.context(),host.identity()).unwrap();
                mailboxes.push(mailbox);
                networks.push(tokio::spawn(async move { session.run(&mut proxy).await }));
            }
            _=timer.tick()=>{
                let now=monotonic_ms();
                for mailbox in &mut mailboxes { let _=mailbox.service(&mut host,now); }
                mailboxes.retain(|m| !m.retired());
                let frame=host.provider().frame();
                for f in 0..48 { capture[f*t.capture_channels]=0.5*(std::f64::consts::TAU*85.*(frame+f as u64) as f64/4096.).sin(); }
                host.process_source(now,1,frame,&capture,&mut playback).unwrap();
            }
        }
    };
    acceptor.abort();
    for task in networks {
        task.abort();
    }
    assert!(
        status.success(),
        "actual Desk remote spectrum failed: {status}"
    );
    assert_eq!(host.provider_mut().engine_mut().revision(), Counter(0));
}

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[path = "fx_aux_tls_tests.rs"]
mod fx_aux_tls_tests;

#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[test]
fn four_aux_media_admission_requires_exact_map_and_control_permission() {
    let directory = PrivateTestDirectory::new();
    let provider = crate::local_audio::LocalAudio::bind_configured(
        &directory.0,
        "host",
        SHOW,
        Counter(9),
        EngineTopology::software(4, 2, 0).unwrap(),
    )
    .unwrap();
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let c = context(&[Permission::Fx, Permission::FxConfiguration]);
    let ids = super::fx_aux_owner::channel_ids();
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
    let i = host.identity();
    let d = MediaDescriptor {
        session: Counter(c.session()),
        source_epoch: i.source_epoch,
        capability_generation: i.capability_generation,
        map_generation: i.map_generation,
        sample_rate: 48000,
        streams,
    };
    assert!(host.negotiate(&context(&[Permission::Fx]), &d).is_err());
    let mut bad = d.clone();
    bad.streams
        .last_mut()
        .unwrap()
        .channel_ids
        .last_mut()
        .unwrap()
        .push_str("-wrong");
    assert!(host.negotiate(&c, &bad).is_err());
    assert!(host.descriptor().is_none());
    assert!(host.negotiate(&c, &d).unwrap());
    assert_eq!(host.descriptor().unwrap().channels(MediaRole::WetReturn), 8);
    assert_eq!(host.provider().engine().mixer().monitor_count(), 2);
    assert_eq!(
        host.provider().engine().mixer().fx_sends(3),
        Some(vec![0.; 4])
    );
    assert!(host.negotiate(&c, &d).is_err());
}

#[test]
fn aux_snapshot_is_read_only_but_aux_edits_keep_authenticated_writer() {
    let c = context(&[]);
    let mut v = json!({"kind":"fx_aux_snapshot","writer":null});
    assert!(c.check_writer(&v).is_ok());
    v["writer"] = json!(c.writer());
    assert!(c.check_writer(&v).is_err());
    v["kind"] = json!("fx_aux_configure");
    assert!(c.check_writer(&v).is_ok());
    v["writer"] = Value::Null;
    assert!(c.check_writer(&v).is_err());
}
