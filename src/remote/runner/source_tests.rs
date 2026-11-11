//! Device-free checks of the same source lifecycle used by the production runner.
use super::*;
use crate::remote::aux_execution::ExecutionLanes;
use std::os::unix::fs::PermissionsExt;

const SHOW: &str = "11111111-1111-4111-8111-111111111111";

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn await_blocks(source: &ProviderSource, blocks: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while source.status.blocks() < blocks {
        assert!(!source.stopped(), "source stopped before requested block");
        assert!(Instant::now() < deadline, "source progression timeout");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn runner_frame_zero_startup_uses_time_after_control_preparation() {
    let directory =
        Directory(std::env::temp_dir().join(format!("gp-h1-startup-clock-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut provider = LocalAudio::bind_configured(
        &directory.0,
        "clock.sock",
        SHOW,
        Counter(9),
        EngineTopology::software(8, 2, 0).unwrap(),
    )
    .unwrap();
    provider.rearm().unwrap();
    provider
        .engine_mut()
        .handle(
            &AudioRequest {
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
            },
            1234,
        )
        .unwrap();
    assert_eq!(provider.frame(), 0);
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let source = ProviderSource::start(&mut host, 1234, 2, 100, None).unwrap();
    await_blocks(&source, 4);
    source.status.close();
    let report = source.finish(&mut host).unwrap();
    assert_eq!(report.fault, None);
    assert!(report.blocks >= 4);
}

#[test]
fn runner_autonomous_epoch_reconstruction_fences_old_handles_and_requires_rearm() {
    let directory = Directory(
        std::env::temp_dir().join(format!("gp-h1-runner-recovery-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut provider = LocalAudio::bind_configured(
        &directory.0,
        "recovery.sock",
        SHOW,
        Counter(9),
        EngineTopology::software(8, 2, 0).unwrap(),
    )
    .unwrap();
    provider.rearm().unwrap();
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let started = Instant::now();
    let current_time = || started.elapsed().as_millis() as u64;
    let source = ProviderSource::start(&mut host, 0, 2, 600, None).unwrap();
    await_blocks(&source, 4);
    assert!(
        host.recover_source_clock(&source.status, Counter(10), 1, current_time())
            .is_err()
    );
    assert!(!source.stopped());
    let recovery = host
        .recover_source_clock(&source.status, Counter(10), 0, current_time())
        .unwrap();
    assert_eq!(recovery.previous.epoch, 9);
    assert!(recovery.previous.energy.iter().any(|energy| *energy > 0.));
    let replacement = ProviderSource {
        status: recovery.replacement.unwrap(),
    };
    source.status.close();
    assert!(host.finish_source_clock(&source.status).is_err());
    assert!(
        host.recover_source_clock(&source.status, Counter(11), 0, current_time())
            .is_err()
    );
    // Source reconstruction and worker progress consume real elapsed time.
    // Cross the former fixed 20 ms timestamp before copying authority readback.
    std::thread::sleep(Duration::from_millis(25));
    await_blocks(&replacement, 4);
    replacement.service(&mut host, current_time()).unwrap();
    assert_eq!(host.identity().source_epoch, Counter(10));
    assert!(host.descriptor().is_none());
    assert_eq!(
        host.provider().engine().clock_status().state,
        crate::clock_domain::ClockState::Disarmed
    );
    let revision = host.provider().engine().revision();
    let lease = host
        .provider_mut()
        .engine_mut()
        .handle(
            &AudioRequest {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(10),
                writer: Some("recovery".into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(revision),
                command: Command::Grant {
                    scope: crate::control_model::Scope::PaConfiguration,
                },
            },
            current_time(),
        )
        .unwrap()
        .outcome
        .unwrap()
        .body
        .granted_lease
        .unwrap();
    let rearm = crate::structural_control::Request {
        contract: "GP14-structure".into(),
        version: 1,
        show_id: SHOW.into(),
        module: "audio".into(),
        epoch: Counter(10),
        writer: Some("recovery".into()),
        lease: Some(lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(revision),
        command: crate::structural_control::Command::OutputRearm {},
    };
    let now = current_time();
    host.provider_mut()
        .clocked_control(now, None, |provider| {
            serde_json::to_value(provider.structural_request(rearm, now, true, None)?)
                .map_err(|error| error.to_string())
        })
        .unwrap();
    assert_eq!(
        host.provider().engine().clock_status().state,
        crate::clock_domain::ClockState::Running
    );
    let started = Instant::now();
    while !replacement.stopped() {
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(1));
    }
    let report = replacement.finish(&mut host).unwrap();
    assert_eq!(report.epoch, 10);
    assert!(report.blocks > 4);
    assert!(report.energy.iter().any(|energy| *energy > 0.));
    assert!(
        report.output_windows[0]
            .energy
            .iter()
            .all(|energy| *energy == 0.)
    );
    assert_eq!(report.fault, None);
    assert!(report.blocks + recovery.previous.blocks <= 300);
}

#[test]
fn runner_clocked_media_negotiation_uses_live_source_frames_and_accounts_wet() {
    let directory =
        Directory(std::env::temp_dir().join(format!("gp-h1-runner-media-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut provider = LocalAudio::bind_configured(
        &directory.0,
        "media.sock",
        SHOW,
        Counter(9),
        EngineTopology::software(8, 2, 0).unwrap(),
    )
    .unwrap();
    provider.rearm().unwrap();
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let certificate = b"runner-media-unit-certificate";
    let policy = PolicyStore::new(vec![Peer {
        id: "runner-media".into(),
        certificate_sha256: fingerprint(certificate),
        permissions: [Permission::Fx, Permission::Analysis].into_iter().collect(),
    }])
    .unwrap();
    let context = policy.authenticate(certificate, 91).unwrap();
    let stereo = ["foh-left".to_string(), "foh-right".to_string()];
    let mut streams = grouped_streams(MediaRole::FxSend, &stereo, 48, 0, 40, 1100).unwrap();
    streams.extend(grouped_streams(MediaRole::WetReturn, &stereo, 48, 384, 50, 1100).unwrap());
    streams.extend(
        grouped_streams(MediaRole::Analysis, &["input-01".into()], 48, 0, 1, 1100).unwrap(),
    );
    let descriptor = MediaDescriptor {
        session: Counter(context.session()),
        source_epoch: Counter(9),
        capability_generation: Counter(1),
        map_generation: Counter(1),
        sample_rate: 48_000,
        streams,
    };
    let source = ProviderSource::start(&mut host, 0, 5, 2000, None).unwrap();
    await_blocks(&source, 4);
    let before = source.status.blocks() * 48;
    assert!(host.negotiate(&context, &descriptor).unwrap());
    assert_eq!(host.descriptor(), Some(&descriptor));
    let deadline = Instant::now() + Duration::from_secs(2);
    let first = loop {
        if let Some(bytes) = host.poll_media(&context).unwrap() {
            let packet = crate::transport::Packet::parse(&bytes).unwrap();
            if packet.spec().role == crate::transport::Role::FxSend {
                break packet.source_frame();
            }
        }
        assert!(Instant::now() < deadline, "negotiated egress timeout");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        first >= before + 480,
        "negotiation must start at its future boundary"
    );
    for bytes in encode_grouped(&descriptor, MediaRole::WetReturn, first, &[0.25; 96]).unwrap() {
        host.media(
            &context,
            crate::transport::Packet::parse(&bytes).unwrap(),
            0,
        )
        .unwrap();
    }
    await_blocks(&source, first / 48 + 12);
    source.service(&mut host, 200).unwrap();
    let active_samples = host.media_stats().nonzero_rendered_wet_samples;
    let active_energy = host.media_stats().rendered_wet_energy;
    assert!(
        active_samples > 0,
        "live service must observe rendered wet counters"
    );
    assert!(active_energy > 0.);
    source.status.close();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !source.stopped() {
        assert!(Instant::now() < deadline, "source closure timeout");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(host.negotiate(&context, &descriptor).is_err());
    assert_eq!(host.descriptor(), Some(&descriptor));
    let report = source.finish(&mut host).unwrap();
    assert_eq!(report.fault, None);
    assert_eq!(
        host.media_stats().nonzero_rendered_wet_samples,
        report.nonzero_wet_samples
    );
    assert!(
        (host.media_stats().rendered_wet_energy - report.rendered_wet_energy).abs()
            <= f64::EPSILON * report.rendered_wet_energy.max(1.) * 4.
    );
    assert!(report.nonzero_wet_samples >= active_samples);
    assert!(report.rendered_wet_energy >= active_energy);
    host.finish_source_ingress();
    host.finish_source_egress();
}

/// The transport harness exchanges the production encoded packets and bounded
/// FX lanes, without a socket or a synthetic substitute for either native owner.
/// It deliberately makes no network timing or wet-deadline acceptance claim.
struct LegacyLifecycle {
    execution: crate::remote::fx_execution::Execution,
    lanes: crate::remote::fx_execution::MediaLanes,
    owner: AuthenticatedContext,
    descriptor: MediaDescriptor,
    lease: Counter,
    last_frame: Option<u64>,
    wet_energy: f64,
    completed: Option<crate::fx_wire::OwnerMessage>,
}

impl LegacyLifecycle {
    fn prepare(host: &mut HostAuthority, manifest: &Value, started: &Instant) -> Self {
        let now = started.elapsed().as_millis() as u64;
        let certificate = b"actual-runner-legacy-fx";
        let policy = PolicyStore::new(vec![Peer {
            id: "actual-runner-legacy-fx".into(),
            certificate_sha256: fingerprint(certificate),
            permissions: [Permission::Fx, Permission::FxConfiguration]
                .into_iter()
                .collect(),
        }])
        .unwrap();
        let owner = policy.authenticate(certificate, 73).unwrap();
        let stereo = ["foh-left".into(), "foh-right".into()];
        let mut streams = grouped_streams(MediaRole::FxSend, &stereo, 48, 0, 40, 1100).unwrap();
        streams.extend(grouped_streams(MediaRole::WetReturn, &stereo, 48, 384, 50, 1100).unwrap());
        let identity = host.identity();
        let descriptor = MediaDescriptor {
            session: Counter(owner.session()),
            source_epoch: identity.source_epoch,
            capability_generation: identity.capability_generation,
            map_generation: identity.map_generation,
            sample_rate: 48_000,
            streams,
        };
        assert!(host.negotiate(&owner, &descriptor).unwrap());
        let library = Path::new(manifest["fx"]["library"].as_str().unwrap());
        let mut fx = crate::host::brain_fx::BrainFx::prepare(library, 2, 48, 9).unwrap();
        assert!(fx.enable_configured(library).unwrap());
        let binding = crate::fx_wire::Binding {
            session: descriptor.session,
            source_epoch: descriptor.source_epoch,
            capability_generation: descriptor.capability_generation,
            map_generation: descriptor.map_generation,
            owner_instance: Counter(1),
            library_sha256: manifest["fx"]["library_sha256"].as_str().unwrap().into(),
            abi_version: 2,
            config_size: 104,
            status_size: 168,
            capabilities_size: 128,
            channels: ["foh-left".into(), "foh-right".into()],
        };
        let mut execution = crate::remote::fx_execution::Execution::from_owner(
            fx,
            Some(binding),
            9,
            *started + Duration::from_secs(5),
            Some(owner.audio_fence()),
        )
        .unwrap();
        let lanes = execution.take_media();
        let revision = host.provider().engine().revision();
        let lease = host
            .provider_mut()
            .engine_mut()
            .handle(
                &AudioRequest {
                    contract: "C-AUDIO".into(),
                    version: 2,
                    show_id: SHOW.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some(owner.writer().into()),
                    lease: None,
                    request_id: Some(Counter(1)),
                    expected_revision: Some(revision),
                    command: Command::Grant {
                        scope: crate::control_model::Scope::FxConfiguration,
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
        Self {
            execution,
            lanes,
            owner,
            descriptor,
            lease,
            last_frame: None,
            wet_energy: 0.,
            completed: None,
        }
    }

    fn message(
        &self,
        host: &mut HostAuthority,
        message: crate::fx_wire::OwnerMessage,
        now: u64,
    ) -> Result<Value> {
        host.dispatch(
            &self.owner,
            json!({
                "contract":crate::fx_wire::CONTRACT,"version":1,"kind":"fx_owner",
                "writer":self.owner.writer(),"message":message,
            }),
            now,
        )?
        .ok_or_else(|| "FX owner reply absent".into())
    }

    fn process_next(&mut self, host: &mut HostAuthority) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(2);
        let bytes = loop {
            if let Some(bytes) = host.poll_media(&self.owner).unwrap() {
                break bytes;
            }
            assert!(Instant::now() < deadline, "actual source FX egress timeout");
            std::thread::sleep(Duration::from_micros(100));
        };
        let packet = crate::transport::Packet::parse(&bytes).unwrap();
        assert_eq!(packet.spec().role, crate::transport::Role::FxSend);
        let frame = packet.source_frame();
        if let Some(previous) = self.last_frame {
            assert_eq!(frame, previous + 48, "actual source egress continuity");
        }
        let mut samples = [0.; 384];
        for (index, sample) in samples[..96].iter_mut().enumerate() {
            *sample = packet.sample(index).unwrap();
        }
        assert!(self.lanes.submit(frame, &samples[..96]));
        let wet = loop {
            if let Some(block) = self.lanes.output() {
                break block;
            }
            assert!(Instant::now() < deadline, "actual legacy FX output timeout");
            std::thread::sleep(Duration::from_micros(100));
        };
        assert_eq!(wet.frame, frame);
        self.wet_energy += wet.samples[..96]
            .iter()
            .map(|sample| sample * sample)
            .sum::<f64>();
        // Preserve source identity through the actual Stagebox ingress. A late
        // return may be refused by its normal deadline; never enlarge that limit.
        for bytes in encode_grouped(
            &self.descriptor,
            MediaRole::WetReturn,
            frame,
            &wet.samples[..96],
        )
        .unwrap()
        {
            host.media(
                &self.owner,
                crate::transport::Packet::parse(&bytes).unwrap(),
                0,
            )
            .unwrap();
        }
        self.lanes.recycle(wet);
        self.last_frame = Some(frame);
        frame
    }

    fn configure_and_run(&mut self, host: &mut HostAuthority, started: &Instant) {
        use crate::fx_wire::{OwnerCommand, OwnerMessage};
        self.process_next(host);
        let deadline = Instant::now() + Duration::from_secs(2);
        let observation = loop {
            let observation = self.execution.observation().unwrap();
            if observation.next_source_frame.0 == self.last_frame.unwrap() + 48 {
                break observation;
            }
            assert!(
                Instant::now() < deadline,
                "legacy initial observation timeout"
            );
            std::thread::sleep(Duration::from_micros(100));
        };
        let now = started.elapsed().as_millis() as u64;
        self.message(
            host,
            OwnerMessage::Observe {
                observation: observation.clone(),
            },
            now,
        )
        .unwrap();
        host.dispatch(
            &self.owner,
            json!({"contract":crate::fx_wire::CONTRACT,"version":1,
            "show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,
            "request_id":null,"expected_revision":null,"kind":"fx_snapshot","body":{}}),
            now,
        )
        .unwrap();
        let mut configuration =
            crate::fx_wire::Configuration::decode(&observation.owner_json).unwrap();
        for channel in &mut configuration.channels {
            channel.delay_ms = 1.;
        }
        let mutation = crate::fx_wire::Mutation {
            binding: observation.binding.clone(),
            expected_generation: observation.generation,
            expected_reset_count: observation.reset_count,
            lead_frames: Counter(4800),
            configuration_json: Some(serde_json::to_string(&configuration).unwrap()),
            panic_mask: None,
        };
        let request = json!({"contract":crate::fx_wire::CONTRACT,"version":1,"show_id":SHOW,
            "module":"audio","epoch":"9","writer":self.owner.writer(),"lease":self.lease,
            "request_id":"2","expected_revision":host.provider().engine().revision(),
            "kind":"fx_configure","body":mutation});
        let reply = host.dispatch(&self.owner, request, now).unwrap().unwrap();
        assert_eq!(reply["state"], "preparing");
        let target: Counter = serde_json::from_value(reply["apply_frame"].clone()).unwrap();
        let command = host
            .poll_reply(&self.owner, started.elapsed().as_millis() as u64)
            .unwrap()
            .unwrap();
        let command: OwnerCommand = serde_json::from_value(command["command"].clone()).unwrap();
        let prepared = self.execution.command(command).unwrap().unwrap();
        assert!(
            matches!(prepared, OwnerMessage::Prepared { .. }),
            "{prepared:?}"
        );
        self.message(host, prepared, started.elapsed().as_millis() as u64)
            .unwrap();
        host.service_source_clock(started.elapsed().as_millis() as u64)
            .unwrap();
        let command = host
            .poll_reply(&self.owner, started.elapsed().as_millis() as u64)
            .unwrap()
            .unwrap();
        let command: OwnerCommand = serde_json::from_value(command["command"].clone()).unwrap();
        assert!(matches!(command, OwnerCommand::Permit { .. }));
        self.execution.command(command).unwrap();
        let observed = host.provider().frame();
        // Only packet exchange occurs here: no source controller service, owner
        // observation, preparation or retirement through application and settling.
        while self.process_next(host) < target.0 + 4800 {}
        assert_eq!(host.provider().frame(), observed);
        let deadline = Instant::now() + Duration::from_secs(2);
        let message = loop {
            let message = self.execution.poll().unwrap().unwrap();
            match &message {
                OwnerMessage::Completed { observation, .. }
                    if observation.remaining_frames == [0, 0]
                        && observation.generation == observation.settled_generation =>
                {
                    break message;
                }
                OwnerMessage::Refused { .. } => panic!("actual owner refused: {message:?}"),
                _ => (),
            }
            assert!(
                Instant::now() < deadline,
                "retained settled FX completion timeout: {message:?}"
            );
            std::thread::sleep(Duration::from_micros(100));
        };
        let OwnerMessage::Completed {
            effective_source_frame,
            observation,
            ..
        } = &message
        else {
            panic!("actual owner did not retain completion: {message:?}");
        };
        assert_eq!(*effective_source_frame, target);
        assert_eq!(observation.applied_source_frame, target);
        assert_eq!(observation.remaining_frames, [0, 0]);
        assert_eq!(observation.generation, observation.settled_generation);
        self.message(host, message.clone(), started.elapsed().as_millis() as u64)
            .unwrap();
        assert!(
            self.wet_energy > 0.,
            "actual remote FX produced no wet signal"
        );
        host.service_source_clock(started.elapsed().as_millis() as u64)
            .unwrap();
        assert!(host.media_stats().nonzero_rendered_wet_samples > 0);
        assert!(host.media_stats().rendered_wet_energy > 0.);
        self.completed = Some(message);
    }

    fn close_and_check(&mut self, host: &mut HostAuthority, started: &Instant) {
        let completed = self.completed.take().unwrap();
        assert!(
            self.message(host, completed, started.elapsed().as_millis() as u64)
                .is_err()
        );
        self.owner.close_audio();
        let frame = self.last_frame.unwrap() + 48;
        assert!(self.lanes.submit(frame, &[0.25; 96]));
        let deadline = Instant::now() + Duration::from_secs(2);
        let wet = loop {
            if let Some(block) = self.lanes.output() {
                break block;
            }
            assert!(
                Instant::now() < deadline,
                "legacy independent closure timeout"
            );
            std::thread::sleep(Duration::from_micros(100));
        };
        assert!(wet.samples[..96].iter().all(|sample| *sample == 0.));
        self.lanes.recycle(wet);
        self.execution.finish();
    }
}

#[test]
#[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
fn actual_runner_source_stalls_saturation_recording_and_epoch_rearm() {
    actual_runner_source_lifecycle(false);
}

#[test]
#[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
fn actual_runner_source_legacy_remote_pa_rec_reconstruction() {
    actual_runner_source_lifecycle(true);
}

fn attach_actual_duplex(
    host: &mut HostAuthority,
    now: u64,
    epoch: u64,
) -> crate::brain_runtime::lifecycle_test::DuplexProbe {
    use crate::remote::{AuthorityEndpoint, BrainMediaDescriptor, Peer, Permission, PolicyStore};
    use sha2::{Digest, Sha256};
    let policy = PolicyStore::new(vec![Peer {
        id: "combined-duplex".into(),
        certificate_sha256: crate::hash::hex(Sha256::digest(b"combined-duplex")),
        permissions: [
            Permission::TalkbackDestinations,
            Permission::LocalOperatorMonitor,
        ]
        .into_iter()
        .collect(),
    }])
    .unwrap();
    let context = policy
        .authenticate(b"combined-duplex", epoch + 100)
        .unwrap();
    let revision = host.provider().engine().revision();
    let lease = host
        .provider_mut()
        .engine_mut()
        .handle(
            &AudioRequest {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(epoch),
                writer: Some(context.writer().into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(revision),
                command: Command::Grant {
                    scope: crate::control_model::Scope::TalkbackDestinations,
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
    let hold = host.provider().brain_snapshot().hold_generation_counter.0 + 1;
    for (id, command) in [
        (
            2,
            crate::brain_control::Command::TalkbackSet {
                monitors: vec![0],
                gain_cdb: -1200,
                mute: false,
            },
        ),
        (
            3,
            crate::brain_control::Command::Hold {
                generation: Counter(hold),
            },
        ),
    ] {
        let request = crate::brain_control::Request {
            contract: "GP15-brain".into(),
            version: 1,
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(epoch),
            writer: Some(context.writer().into()),
            lease: Some(lease),
            request_id: Some(Counter(id)),
            expected_revision: Some(host.provider().engine().revision()),
            command,
        };
        let reply = host
            .provider_mut()
            .clocked_control(now, Some(context.audio_fence()), |provider| {
                serde_json::to_value(provider.brain_request(request, now, true, None)?)
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(reply["reason"].is_null(), "{reply}");
    }
    let descriptor = BrainMediaDescriptor {
        contract: "GP15-media".into(),
        version: 1,
        session: context.session(),
        stagebox_epoch: epoch,
        brain_epoch: epoch + 70,
        stagebox_map: host.identity().map_generation.0,
        brain_map: 1,
        generation: 1,
        selection_generation: host.provider().brain_snapshot().selection_generation.0,
        hold_generation: hold,
        sample_rate: 48000,
        frames: 48,
        talkback: true,
        monitor: true,
    };
    host.dispatch(
        &context,
        json!({"contract":"GP15-media","version":1,"kind":"negotiate",
        "writer":context.writer(),"descriptor":descriptor}),
        now,
    )
    .unwrap();
    let (proxy, mut mailbox) = crate::remote::authority_channel(&context, host.identity()).unwrap();
    mailbox.service(host, now).unwrap();
    let mut probe = crate::brain_runtime::lifecycle_test::DuplexProbe::start(
        proxy,
        context.clone(),
        descriptor,
        now,
    );
    host.dispatch(
        &context,
        json!({"contract":"GP15-device","version":1,"kind":"device_observation",
        "writer":context.writer(),"observation":probe.observation()}),
        now,
    )
    .unwrap();
    host.service_source_clock(now).unwrap();
    probe
}

fn actual_runner_source_lifecycle(legacy_remote: bool) {
    let directory = Directory(std::env::temp_dir().join(format!(
        "gp-h1-runner-source-{}-{legacy_remote}",
        std::process::id()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut provider = LocalAudio::bind_configured(
        &directory.0,
        "runner.sock",
        SHOW,
        Counter(9),
        EngineTopology::software(8, 2, 8).unwrap(),
    )
    .unwrap();
    let manifest_path = std::env::var_os("GP05_MANIFEST").unwrap();
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    provider.enable_modules(Path::new(&manifest_path)).unwrap();
    provider
        .configure_pa(&std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap())
        .unwrap();
    if !legacy_remote {
        provider
            .enable_fx_aux(
                Path::new(manifest["fx"]["library"].as_str().unwrap()),
                manifest["fx"]["library_sha256"].as_str().unwrap(),
            )
            .unwrap();
    }
    start_recording(&mut provider, SHOW, "runner-take", 0).unwrap();
    let aux_lease = (!legacy_remote).then(|| {
        let aux_grant = AudioRequest {
            contract: "C-AUDIO".into(),
            version: provider.engine().wire_version(),
            show_id: SHOW.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some("runner-aux".into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(provider.engine().revision()),
            command: Command::Grant {
                scope: crate::control_model::Scope::FxConfiguration,
            },
        };
        provider
            .engine_mut()
            .handle(&aux_grant, 0)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap()
    });
    provider.rearm().unwrap();
    let mut host = HostAuthority::new(provider, 1).unwrap();
    let started = Instant::now();
    let source = ProviderSource::start(&mut host, 0, 2, 5000, None).unwrap();
    let old_status = source.status.clone();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let now = started.elapsed().as_millis() as u64;
        source.service(&mut host, now).unwrap();
        let state = host.provider_mut().module_status(now).unwrap();
        if state["recording"]["state"] == "recording" {
            break;
        }
        assert!(Instant::now() < deadline, "REC startup timeout: {state}");
        assert!(!source.stopped(), "source stopped during REC startup");
        std::thread::sleep(Duration::from_millis(1));
    }

    if let Some(aux_lease) = aux_lease {
        // Prepare a real local aux owner while the runner's source is advancing.
        let now = started.elapsed().as_millis() as u64;
        let read = json!({"contract":crate::fx_aux_wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_aux_snapshot","body":{}});
        let snapshot = host
            .provider_mut()
            .fx_aux_request(read, now, false, None)
            .unwrap();
        let mut config: Value =
            serde_json::from_str(snapshot["buses"][0]["configuration_json"].as_str().unwrap())
                .unwrap();
        config["sends"][0] = json!(1.);
        config["settings"]["time_ms"] = json!(2.);
        config["settings"]["amount"] = json!(0.);
        config["settings"]["damping"] = json!(0.);
        config["settings"]["gain"] = json!(1.);
        let edit = json!({"contract":crate::fx_aux_wire::CONTRACT,"version":1,"show_id":SHOW,"module":"audio","epoch":"9","writer":"runner-aux","lease":aux_lease,"request_id":"2","expected_revision":snapshot["revision"],"kind":"fx_aux_configure","body":{"bus":0,"map_revision":"1","library_sha256":manifest["fx"]["library_sha256"],"expected_generation":"0","configuration_json":serde_json::to_string(&config).unwrap(),"panic":false}});
        let reply = host
            .provider_mut()
            .clocked_control(now, None, |provider| {
                provider.fx_aux_request(edit, now, true, None)
            })
            .unwrap();
        assert_eq!(reply["state"], "prepared");
    }

    let mut duplex = attach_actual_duplex(&mut host, started.elapsed().as_millis() as u64, 9);
    // Both occupied return lanes leave the source owner free to capture, run
    // PA/local FX and push raw REC. The controller does no service in this span.
    let first = source.status.blocks();
    let observed = host.provider().frame();
    host.with_saturated_source_returns(false, || await_blocks(&source, first + 32));
    use std::sync::atomic::Ordering;
    assert!(duplex.captured.load(Ordering::Acquire) >= 16);
    assert!(duplex.monitored.load(Ordering::Acquire) >= 16);
    duplex.close_capture();
    let capture_closed = duplex.captured.load(Ordering::Acquire);
    let monitor_before = duplex.monitored.load(Ordering::Acquire);
    host.with_saturated_source_returns(true, || await_blocks(&source, first + 64));
    assert_eq!(host.provider().frame(), observed);
    assert!(duplex.monitored.load(Ordering::Acquire) > monitor_before);
    assert!(duplex.captured.load(Ordering::Acquire) <= capture_closed + 4);
    let (capture_frame, playback_pressure, status_pressure) = duplex.finish();
    assert!(capture_frame >= 32 * 48);
    assert!(playback_pressure > 0 && status_pressure > 0);
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        host.service_source_clock(started.elapsed().as_millis() as u64)
            .unwrap();
        let stats = host.media_stats();
        if stats.accepted_talkback_packets + stats.rejected_talkback_packets
            == duplex.attempted.load(Ordering::Acquire)
        {
            assert!(stats.accepted_talkback_packets > 0);
            break;
        }
        assert!(
            Instant::now() < until,
            "talkback receipts did not reconcile"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // The real legacy remote owner receives source-encoded blocks, executes a
    // prepare/permit transition without control polling, and retains completion.
    let mut remote = legacy_remote.then(|| {
        let mut remote = LegacyLifecycle::prepare(&mut host, &manifest, &started);
        remote.configure_and_run(&mut host, &started);
        remote
    });
    let recovery = host
        .recover_source_clock(
            &source.status,
            Counter(10),
            0,
            started.elapsed().as_millis() as u64,
        )
        .unwrap();
    let report = recovery.previous;
    let replacement = ProviderSource {
        status: recovery.replacement.unwrap(),
    };
    assert!(host.descriptor().is_none());
    old_status.close();
    assert!(host.finish_source_clock(&old_status).is_err());
    await_blocks(&replacement, 4);
    assert_eq!(
        host.provider().engine().clock_status().state,
        crate::clock_domain::ClockState::Disarmed
    );
    // Session closure independently reaches the remote owner even though neither
    // controller polls its observation lane. Epoch replacement rejects old replies.
    if let Some(remote) = &mut remote {
        remote.close_and_check(&mut host, &started);
    }

    assert_eq!(report.fault, None);
    assert_eq!(report.first_frame, 0);
    assert_eq!(report.next_frame, report.blocks * 48);
    assert!(report.blocks >= first + 64);
    if legacy_remote {
        assert!(report.nonzero_wet_samples > 0);
        assert!(report.rendered_wet_energy > 0.);
    }
    assert!(report.energy.iter().any(|energy| *energy > 0.));

    let deadline = Instant::now() + Duration::from_secs(5);
    let final_status = loop {
        let state = host
            .provider_mut()
            .module_status(started.elapsed().as_millis() as u64)
            .unwrap();
        if state["recording"]["outcome"] == "incomplete" {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "REC finalization timeout: {state}"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(final_status["recording"]["dropped_frames"], "0");
    let mut paths: Vec<_> = std::fs::read_dir(directory.0.join("takes/runner-take"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
        .collect();
    assert_eq!(paths.len(), 8);
    paths.sort();
    for (channel, path) in paths.into_iter().enumerate() {
        let mut wav = hound::WavReader::open(path).unwrap();
        assert_eq!(wav.spec().bits_per_sample, 24);
        let samples: Vec<_> = wav
            .samples::<i32>()
            .map(std::result::Result::unwrap)
            .collect();
        assert!(samples.len() >= 64 * 48);
        let wanted: Vec<i32> = (report.next_frame - 64 * 48..report.next_frame)
            .map(|frame| {
                let polarity = if (frame / (24 + channel as u64)).is_multiple_of(2) {
                    1
                } else {
                    -1
                };
                polarity * (channel as i32 + 1) * 128
            })
            .collect();
        assert_eq!(&samples[samples.len() - wanted.len()..], wanted.as_slice());
    }

    // Rearm is a fresh authority transaction in the replacement epoch. Recovery
    // must preserve the original absolute run budget rather than starting a new run.
    let now = started.elapsed().as_millis() as u64;
    let revision = host.provider().engine().revision();
    let lease = host
        .provider_mut()
        .engine_mut()
        .handle(
            &AudioRequest {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id: SHOW.into(),
                module: "audio".into(),
                epoch: Counter(10),
                writer: Some("runner-recovery".into()),
                lease: None,
                request_id: Some(Counter(1)),
                expected_revision: Some(revision),
                command: Command::Grant {
                    scope: crate::control_model::Scope::PaConfiguration,
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
        epoch: Counter(10),
        writer: Some("runner-recovery".into()),
        lease: Some(lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(revision),
        command: crate::structural_control::Command::OutputRearm {},
    };
    let reply = host
        .provider_mut()
        .clocked_control(now, None, |provider| {
            serde_json::to_value(provider.structural_request(request, now, true, None)?)
                .map_err(|error| error.to_string())
        })
        .unwrap();
    assert_eq!(
        host.provider().engine().clock_status().state,
        crate::clock_domain::ClockState::Running,
        "{reply}"
    );
    let mut fresh_duplex =
        attach_actual_duplex(&mut host, started.elapsed().as_millis() as u64, 10);
    // A retained old device closure cannot close the new device/source epochs.
    duplex.close_capture();
    await_blocks(&replacement, replacement.status.blocks() + 16);
    assert!(fresh_duplex.captured.load(Ordering::Acquire) >= 8);
    assert!(fresh_duplex.monitored.load(Ordering::Acquire) >= 8);
    fresh_duplex.finish();
    replacement.status.close();
    let replacement = replacement.finish(&mut host).unwrap();
    assert_eq!(replacement.epoch, 10);
    assert!(replacement.blocks >= 20);
    assert_eq!(replacement.next_frame, replacement.blocks * 48);
    assert_eq!(replacement.fault, None);
    assert!(
        replacement.output_windows[0]
            .energy
            .iter()
            .all(|energy| *energy == 0.)
    );
    assert!(replacement.energy.iter().any(|energy| *energy > 0.));
    assert!(replacement.blocks + report.blocks <= 2500);
}
