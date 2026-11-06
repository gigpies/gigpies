//! Independently versioned Brain duplex media. Frames belong to the role's
//! producer clock; they are never compared across device domains.
use super::{EngineIdentity, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrainMediaDescriptor {
    pub contract: String,
    pub version: u32,
    pub session: u64,
    pub stagebox_epoch: u64,
    pub brain_epoch: u64,
    pub stagebox_map: u64,
    pub brain_map: u64,
    pub generation: u64,
    pub selection_generation: u64,
    pub hold_generation: u64,
    pub sample_rate: u32,
    pub frames: u16,
    pub talkback: bool,
    pub monitor: bool,
}
impl BrainMediaDescriptor {
    pub fn validate(&self, session: u64, identity: &EngineIdentity) -> Result<()> {
        if self.contract != "GP15-media"
            || self.version != 1
            || self.session != session
            || self.stagebox_epoch != identity.source_epoch.0
            || self.stagebox_map != identity.map_generation.0
            || self.session == 0
            || self.stagebox_epoch == 0
            || self.brain_epoch == 0
            || self.brain_map == 0
            || self.generation == 0
            || self.sample_rate != 48000
            || self.frames != 48
            || (!self.talkback && !self.monitor)
        {
            return Err("Brain media version/identity/shape".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainMediaRole {
    Talkback,
    Monitor,
}
impl BrainMediaRole {
    pub fn generation(self, descriptor: &BrainMediaDescriptor) -> u64 {
        match self {
            Self::Talkback => descriptor.hold_generation,
            Self::Monitor => descriptor.selection_generation,
        }
    }
    pub fn channels(self) -> usize {
        if self == Self::Talkback { 1 } else { 2 }
    }
}
#[derive(Clone, Debug)]
pub struct BrainPacket {
    pub descriptor: BrainMediaDescriptor,
    pub role: BrainMediaRole,
    pub frame: u64,
    pub samples: Vec<f64>,
}
impl BrainPacket {
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.descriptor.validate(
            self.descriptor.session,
            &EngineIdentity {
                source_epoch: crate::show::Counter(self.descriptor.stagebox_epoch),
                capability_generation: crate::show::Counter(1),
                map_generation: crate::show::Counter(self.descriptor.stagebox_map),
            },
        )?;
        if self.samples.len() != 48 * self.role.channels()
            || !self.frame.is_multiple_of(48)
            || self.frame.checked_add(48).is_none()
            || self
                .samples
                .iter()
                .any(|s| !s.is_finite() || s.abs() > f32::MAX as f64)
            || (self.role == BrainMediaRole::Talkback && !self.descriptor.talkback)
            || (self.role == BrainMediaRole::Monitor && !self.descriptor.monitor)
        {
            return Err("Brain packet shape".into());
        }
        let mut bytes = Vec::with_capacity(80 + self.samples.len() * 4);
        bytes.extend_from_slice(b"GBA1");
        bytes.extend_from_slice(&[
            if self.role == BrainMediaRole::Talkback {
                1
            } else {
                2
            },
            0,
            0,
            0,
        ]);
        for value in [
            self.descriptor.session,
            self.descriptor.stagebox_epoch,
            self.descriptor.brain_epoch,
            self.descriptor.stagebox_map,
            self.descriptor.brain_map,
            self.role.generation(&self.descriptor),
            if self.role == BrainMediaRole::Monitor {
                self.descriptor.selection_generation
            } else {
                0
            },
            if self.role == BrainMediaRole::Talkback {
                self.descriptor.hold_generation
            } else {
                0
            },
            self.frame,
        ] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        for value in &self.samples {
            bytes.extend_from_slice(&(*value as f32).to_be_bytes());
        }
        Ok(bytes)
    }
    /// Rewrite only an authenticated monitor packet's sample payload. Validation
    /// precedes mutation; source indices and all other header bytes are retained.
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(super) fn silence_monitor(
        bytes: &mut [u8],
        descriptor: &BrainMediaDescriptor,
    ) -> Result<()> {
        Self::decode(bytes, descriptor, BrainMediaRole::Monitor)?;
        bytes[80..].fill(0);
        Ok(())
    }
    pub fn decode(
        bytes: &[u8],
        descriptor: &BrainMediaDescriptor,
        role: BrainMediaRole,
    ) -> Result<Self> {
        if bytes.len() != 80 + 48 * role.channels() * 4
            || &bytes[..4] != b"GBA1"
            || bytes[4]
                != if role == BrainMediaRole::Talkback {
                    1
                } else {
                    2
                }
            || bytes[5..8] != [0, 0, 0]
        {
            return Err("Brain packet header/shape".into());
        }
        let mut values = [0u64; 9];
        for (value, chunk) in values.iter_mut().zip(bytes[8..80].chunks_exact(8)) {
            *value = u64::from_be_bytes(chunk.try_into().map_err(|_| "Brain header")?);
        }
        if values[..8]
            != [
                descriptor.session,
                descriptor.stagebox_epoch,
                descriptor.brain_epoch,
                descriptor.stagebox_map,
                descriptor.brain_map,
                role.generation(descriptor),
                if role == BrainMediaRole::Monitor {
                    descriptor.selection_generation
                } else {
                    0
                },
                if role == BrainMediaRole::Talkback {
                    descriptor.hold_generation
                } else {
                    0
                },
            ]
        {
            return Err("Brain packet stale identity".into());
        }
        let samples = bytes[80..]
            .chunks_exact(4)
            .map(|v| f32::from_be_bytes(v.try_into().unwrap()) as f64)
            .collect();
        let packet = Self {
            descriptor: descriptor.clone(),
            role,
            frame: values[8],
            samples,
        };
        packet.encode()?;
        Ok(packet)
    }
}

/// Report produced by the single duplex owner. Status is bounded telemetry from
/// the production device host, never a permission or proof of physical lock.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrainDeviceObservation {
    pub brain_epoch: u64,
    pub brain_map: u64,
    pub frame: u64,
    pub config: crate::brain_audio::device::DeviceConfig,
    pub status: serde_json::Value,
    pub ticket: Option<u64>,
    pub success: Option<bool>,
    pub error: Option<String>,
}
impl BrainDeviceObservation {
    pub fn validate(&self) -> Result<()> {
        self.config.validate().map_err(|e| e.to_string())?;
        let _: serde_json::Value =
            crate::show::decode(&serde_json::to_vec(&self.status).map_err(|e| e.to_string())?)?;
        if self.brain_epoch == 0
            || self.brain_map == 0
            || !self.status.is_object()
            || self.error.as_ref().is_some_and(|s| s.len() > 1024)
            || self.ticket.is_some() != self.success.is_some()
            || self.success == Some(true) && self.error.is_some()
            || serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 32768
        {
            return Err("Brain device observation shape".into());
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux", feature = "hardware-host"))]
mod tests {
    use super::*;
    use crate::remote::*;
    use crate::show::Counter;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn duplex_negotiation_isolated_from_fx_and_requires_its_own_permissions() {
        let path = std::env::temp_dir().join(format!("gp-brain-remote-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let run = || {
            let provider = crate::local_audio::LocalAudio::bind_configured(
                &path,
                "host",
                "11111111-1111-4111-8111-111111111111",
                Counter(9),
                crate::topology::EngineTopology::reference_16_18(0, 4).unwrap(),
            )
            .unwrap();
            let mut host = HostAuthority::new(provider, 1).unwrap();
            let store = PolicyStore::new(vec![
                Peer {
                    id: "fx".into(),
                    certificate_sha256: fingerprint(b"fx"),
                    permissions: [Permission::Fx].into_iter().collect(),
                },
                Peer {
                    id: "duplex".into(),
                    certificate_sha256: fingerprint(b"duplex"),
                    permissions: [
                        Permission::TalkbackDestinations,
                        Permission::LocalOperatorMonitor,
                    ]
                    .into_iter()
                    .collect(),
                },
            ])
            .unwrap();
            let fx = store.authenticate(b"fx", 11).unwrap();
            let duplex = store.authenticate(b"duplex", 12).unwrap();
            let identity = host.identity();
            let mut legacy = MediaDescriptor {
                session: Counter(11),
                source_epoch: identity.source_epoch,
                capability_generation: identity.capability_generation,
                map_generation: identity.map_generation,
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
            legacy.streams.extend(
                grouped_streams(
                    MediaRole::WetReturn,
                    &["foh-left".into(), "foh-right".into()],
                    48,
                    384,
                    2,
                    1100,
                )
                .unwrap(),
            );
            host.negotiate(&fx, &legacy).unwrap();
            let mut descriptor = BrainMediaDescriptor {
                contract: "GP15-media".into(),
                version: 1,
                session: 11,
                stagebox_epoch: 9,
                brain_epoch: 77,
                stagebox_map: identity.map_generation.0,
                brain_map: 1,
                generation: 1,
                selection_generation: 1,
                hold_generation: 0,
                sample_rate: 48000,
                frames: 48,
                talkback: false,
                monitor: true,
            };
            let request = |ctx: &AuthenticatedContext, d: &BrainMediaDescriptor| serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":ctx.writer(),"descriptor":d});
            assert!(host.dispatch(&fx, request(&fx, &descriptor), 0).is_err());
            descriptor.session = 12;
            assert!(
                host.dispatch(&duplex, request(&duplex, &descriptor), 0)
                    .unwrap()
                    .is_some()
            );
            assert_eq!(host.descriptor(), Some(&legacy));
            let competitor = store.authenticate(b"duplex", 13).unwrap();
            let mut competing = descriptor.clone();
            competing.session = 13;
            assert!(
                host.dispatch(&competitor, request(&competitor, &competing), 0)
                    .is_err()
            );
            host.disconnect(&duplex);
            assert_eq!(host.descriptor(), Some(&legacy));
            assert!(host.brain_descriptor().is_none());
        };
        run();
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(all(test, target_os = "linux", feature = "hardware-host"))]
mod device_fixture_tests {
    use super::*;
    use crate::{
        brain_audio::{
            bridge::{Bridge, BridgeConfig, BridgeEpochs},
            device::{DeviceConfig, FakeDuplex, PortMap, SampleFormat},
            host::BrainHost,
        },
        control_model::{Command, Request as AudioRequest, Scope},
        remote::*,
        show::Counter,
    };
    use serde_json::{Value, json};
    use std::os::unix::fs::PermissionsExt;
    fn fixture(name: &str, value: &Value) {
        let bytes = serde_json::to_vec_pretty(value).unwrap();
        let decoded: Value = crate::remote::decode(&bytes).unwrap();
        assert_eq!(&decoded, value);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/brain-device-v1")
            .join(format!("{name}.json"));
        if std::env::var_os("UPDATE_BRAIN_DEVICE_FIXTURES").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        }
        let existing: Value = serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}; regenerate UPDATE_BRAIN_DEVICE_FIXTURES=1",
                path.display()
            )
        }))
        .unwrap();
        assert_eq!(existing, *value, "fixture {name}");
    }
    fn config() -> DeviceConfig {
        DeviceConfig {
            device_id: "fake-brain-duplex".into(),
            endpoint: "fake:brain".into(),
            sample_rate: 48000,
            capture_channels: 1,
            playback_channels: 2,
            format: SampleFormat::Float32Le,
            period_frames: 48,
            buffer_frames: 192,
            microphone: PortMap {
                id: "talkback-mic".into(),
                socket: "mic-observed-model".into(),
                slot: 0,
            },
            monitor: [
                PortMap {
                    id: "operator-left".into(),
                    socket: "left-model".into(),
                    slot: 0,
                },
                PortMap {
                    id: "operator-right".into(),
                    socket: "right-model".into(),
                    slot: 1,
                },
            ],
            socket_mapping_record: None,
            shared_clock_record: None,
            hardware_monitoring_record: None,
        }
    }
    fn observe(config: DeviceConfig, epoch: u64, map: u64) -> BrainDeviceObservation {
        let host = BrainHost::prepare(
            FakeDuplex::prepare(&config).unwrap(),
            config.clone(),
            epoch,
            map,
        )
        .unwrap();
        let bridge = Bridge::prepare(
            BridgeConfig::voice(2),
            BridgeEpochs {
                source: 9,
                destination: epoch,
                route: 1,
            },
        )
        .unwrap();
        BrainDeviceObservation {
            brain_epoch: epoch,
            brain_map: map,
            frame: 0,
            config,
            status: brain_device_telemetry(&host.status(), &bridge.status(), 0),
            ticket: None,
            success: None,
            error: None,
        }
    }
    #[test]
    fn production_device_telemetry_includes_exact_unsigned_capture_queue_counter() {
        let config = config();
        let host =
            BrainHost::prepare(FakeDuplex::prepare(&config).unwrap(), config, 77, 1).unwrap();
        let bridge = Bridge::prepare(
            BridgeConfig::voice(2),
            BridgeEpochs {
                source: 9,
                destination: 77,
                route: 1,
            },
        )
        .unwrap();
        for count in [0, 1, u64::MAX] {
            let status = brain_device_telemetry(&host.status(), &bridge.status(), count);
            let bytes = crate::remote::encode(&status).unwrap();
            let decoded: Value = crate::remote::decode(&bytes).unwrap();
            assert_eq!(decoded["capture_queue_dropped"].as_u64(), Some(count));
            assert_eq!(decoded, status);
        }
        assert_eq!(
            observe(self::config(), 77, 1).status["capture_queue_dropped"].as_u64(),
            Some(0),
            "producer fixtures must use the same telemetry shape as the runtime"
        );
    }
    #[test]
    fn device_restart_invalidates_fresh_snapshot_without_shared_revision_change() {
        let path =
            std::env::temp_dir().join(format!("gp-brain-device-restart-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        let provider = crate::local_audio::LocalAudio::bind_configured(
            &path,
            "host",
            show,
            Counter(9),
            crate::topology::EngineTopology::reference_16_18(0, 4).unwrap(),
        )
        .unwrap();
        let mut host = HostAuthority::new(provider, 1).unwrap();
        let policy = PolicyStore::new(vec![
            Peer {
                id: "desk".into(),
                certificate_sha256: fingerprint(b"restart-desk"),
                permissions: [Permission::LocalOperatorMonitor].into_iter().collect(),
            },
            Peer {
                id: "duplex".into(),
                certificate_sha256: fingerprint(b"restart-duplex"),
                permissions: [Permission::LocalOperatorMonitor].into_iter().collect(),
            },
        ])
        .unwrap();
        let desk = policy.authenticate(b"restart-desk", 12).unwrap();
        let duplex = policy.authenticate(b"restart-duplex", 13).unwrap();
        let grant = AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some(desk.writer().into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant {
                scope: Scope::LocalOperatorMonitor,
            },
        };
        let lease = host
            .provider_mut()
            .engine_mut()
            .handle(&grant, 0)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap();
        let snapshot =
            || json!({"contract":"GP15-device","version":1,"kind":"device_snapshot","writer":null});
        let report = |context: &AuthenticatedContext, o: BrainDeviceObservation| json!({"contract":"GP15-device","version":1,"kind":"device_observation","writer":context.writer(),"observation":o});
        host.dispatch(&duplex, report(&duplex, observe(config(), 77, 1)), 0)
            .unwrap();
        host.dispatch(&desk, snapshot(), 1).unwrap();
        host.disconnect(&duplex);
        let replacement = policy.authenticate(b"restart-duplex", 14).unwrap();
        host.dispatch(
            &replacement,
            report(&replacement, observe(config(), 78, 2)),
            2,
        )
        .unwrap();
        let request = || json!({"contract":"GP15-device","version":1,"show_id":show,"module":"audio","epoch":Counter(9),"writer":desk.writer(),"lease":lease,"request_id":Counter(2),"expected_revision":Counter(0),"kind":"device_configure","body":{},"config":config()});
        assert!(
            host.dispatch(&desk, request(), 3)
                .unwrap_err()
                .contains("fresh device snapshot")
        );
        host.dispatch(&desk, snapshot(), 3).unwrap();
        assert_eq!(
            host.dispatch(&desk, request(), 4).unwrap().unwrap()["state"],
            "pending",
            "same revision/request succeeds only after reading replacement identity"
        );
        drop(host);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn actual_device_dispatch_producer_fixtures() {
        let path =
            std::env::temp_dir().join(format!("gp-brain-device-fixture-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        let topology = crate::topology::EngineTopology::reference_16_18(0, 4).unwrap();
        let provider = crate::local_audio::LocalAudio::bind_configured(
            &path,
            "host",
            show,
            Counter(9),
            topology.clone(),
        )
        .unwrap();
        let mut host = HostAuthority::new(provider, 1).unwrap();
        let policy = PolicyStore::new(vec![
            Peer {
                id: "desk".into(),
                certificate_sha256: fingerprint(b"fixture-desk"),
                permissions: [Permission::LocalOperatorMonitor].into_iter().collect(),
            },
            Peer {
                id: "duplex".into(),
                certificate_sha256: fingerprint(b"fixture-duplex"),
                permissions: [
                    Permission::LocalOperatorMonitor,
                    Permission::TalkbackDestinations,
                ]
                .into_iter()
                .collect(),
            },
        ])
        .unwrap();
        let desk = policy.authenticate(b"fixture-desk", 12).unwrap();
        let duplex = policy.authenticate(b"fixture-duplex", 13).unwrap();
        let snapshot =
            || json!({"contract":"GP15-device","version":1,"kind":"device_snapshot","writer":null});
        fixture(
            "snapshot-unavailable",
            &host.dispatch(&desk, snapshot(), 0).unwrap().unwrap(),
        );
        let mut observation = observe(config(), 77, 1);
        let report = |o: &BrainDeviceObservation| json!({"contract":"GP15-device","version":1,"kind":"device_observation","writer":duplex.writer(),"observation":o});
        host.dispatch(&duplex, report(&observation), 0).unwrap();
        fixture(
            "snapshot-unarmed",
            &host.dispatch(&desk, snapshot(), 0).unwrap().unwrap(),
        );
        let audio = AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: None,
            lease: None,
            request_id: None,
            expected_revision: None,
            command: Command::Snapshot {},
        };
        host.dispatch(&desk, serde_json::to_value(&audio).unwrap(), 0)
            .unwrap();
        let mut grant = audio.clone();
        grant.writer = Some(desk.writer().into());
        grant.request_id = Some(Counter(1));
        grant.expected_revision = Some(Counter(0));
        grant.command = Command::Grant {
            scope: Scope::LocalOperatorMonitor,
        };
        let granted = host
            .dispatch(&desk, serde_json::to_value(grant).unwrap(), 0)
            .unwrap()
            .unwrap();
        let lease: Counter =
            serde_json::from_value(granted["outcome"]["body"]["granted_lease"].clone()).unwrap();
        let mut next = config();
        next.monitor[0].slot = 1;
        next.monitor[1].slot = 0;
        let request = |id: u64, revision: u64, c: &DeviceConfig| json!({"contract":"GP15-device","version":1,"show_id":show,"module":"audio","epoch":Counter(9),"writer":desk.writer(),"lease":lease,"request_id":Counter(id),"expected_revision":Counter(revision),"kind":"device_configure","body":{},"config":c});
        fixture("configure-request", &request(2, 0, &next));
        let pending = host
            .dispatch(&desk, request(2, 0, &next), 1)
            .unwrap()
            .unwrap();
        fixture("configure-pending", &pending);
        assert_eq!(
            host.dispatch(&desk, request(2, 0, &next), 1)
                .unwrap()
                .unwrap(),
            pending
        );
        let capture = vec![0.; 48 * topology.capture_channels];
        let mut playback = vec![0.; 48 * topology.playback_channels];
        host.process_source(1, 9, 0, &capture, &mut playback)
            .unwrap();
        host.process_source(2, 9, 48, &capture, &mut playback)
            .unwrap();
        fixture(
            "configure-accepted-intent",
            &host.poll_reply(&desk, 2).unwrap().unwrap(),
        );
        let apply = host.poll_reply(&duplex, 2).unwrap().unwrap();
        fixture("apply-configuration", &apply);
        observation = observe(next.clone(), 78, 2);
        observation.ticket = apply["ticket"].as_u64();
        observation.success = Some(true);
        host.dispatch(&duplex, report(&observation), 3).unwrap();
        let applied = host.poll_reply(&desk, 3).unwrap().unwrap();
        fixture("configure-applied-device", &applied);
        assert_eq!(
            host.dispatch(&desk, request(2, 0, &next), 3)
                .unwrap()
                .unwrap(),
            applied
        );
        fixture(
            "snapshot-configured",
            &host.dispatch(&desk, snapshot(), 3).unwrap().unwrap(),
        );
        let mut unsupported = next.clone();
        unsupported.capture_channels = 2;
        host.dispatch(&desk, request(3, 1, &unsupported), 3)
            .unwrap();
        host.process_source(3, 9, 96, &capture, &mut playback)
            .unwrap();
        host.process_source(4, 9, 144, &capture, &mut playback)
            .unwrap();
        host.poll_reply(&desk, 4).unwrap();
        let apply = host.poll_reply(&duplex, 4).unwrap().unwrap();
        let failed = BrainHost::prepare(FakeDuplex::prepare(&next).unwrap(), unsupported, 79, 3)
            .err()
            .unwrap();
        observation.ticket = apply["ticket"].as_u64();
        observation.success = Some(false);
        observation.error = Some(failed.to_string());
        host.dispatch(&duplex, report(&observation), 5).unwrap();
        fixture(
            "configure-failed-device",
            &host.poll_reply(&desk, 5).unwrap().unwrap(),
        );
        drop(host);
        std::fs::remove_dir_all(path).unwrap();
    }
}

/// Integer wire units preserve the strict control codec: ratio is parts per
/// billion of unity; signed skew is parts per billion of source speed.
pub fn brain_bridge_telemetry(
    status: &crate::brain_audio::bridge::BridgeStatus,
) -> serde_json::Value {
    let mut value = serde_json::to_value(status).expect("bounded bridge telemetry");
    let object = value.as_object_mut().expect("bridge telemetry object");
    object.remove("ratio_output_per_input");
    object.remove("estimated_source_skew_ppm");
    object.remove("queue_latency_nominal_ms");
    object.remove("filter_latency_nominal_ms");
    object.remove("physical_mapping_uncertainty_frames");
    object.insert(
        "queue_latency_nominal_us".into(),
        serde_json::json!((status.queue_latency_nominal_ms * 1000.).round() as u64),
    );
    object.insert(
        "filter_latency_nominal_us".into(),
        serde_json::json!((status.filter_latency_nominal_ms * 1000.).round() as u64),
    );
    object.insert(
        "physical_mapping_uncertainty_milliframes".into(),
        serde_json::json!(
            status
                .physical_mapping_uncertainty_frames
                .map(|v| (v * 1000.).round() as u64)
        ),
    );
    object.insert(
        "ratio_ppb".into(),
        serde_json::json!((status.ratio_output_per_input * 1_000_000_000.).round() as i64),
    );
    object.insert(
        "skew_ppb".into(),
        serde_json::json!((status.estimated_source_skew_ppm * 1000.).round() as i64),
    );
    value
}
/// Meter units are one billionth of normalized full scale. These are digital
/// observations, not sound-pressure units or evidence of physical clock lock.
pub fn brain_device_telemetry(
    status: &crate::brain_audio::host::DeviceStatus,
    bridge: &crate::brain_audio::bridge::BridgeStatus,
    capture_queue_dropped: u64,
) -> serde_json::Value {
    let mut value = serde_json::to_value(status).expect("bounded device telemetry");
    let object = value.as_object_mut().expect("device telemetry object");
    for (name, peak) in [
        ("captured_peak", status.captured_peak),
        ("outgoing_peak", status.outgoing_peak),
        ("playback_peak", status.playback_peak),
    ] {
        object.remove(name);
        object.insert(
            format!("{name}_nano"),
            serde_json::json!((peak * 1_000_000_000.).round() as u64),
        );
    }
    object.insert("monitor_bridge".into(), brain_bridge_telemetry(bridge));
    object.insert(
        "capture_queue_dropped".into(),
        serde_json::json!(capture_queue_dropped),
    );
    value
}

/// Readiness is meaningful only for this exact independently clocked route.
#[cfg(any(test, all(target_os = "linux", feature = "hardware-host")))]
pub(crate) fn brain_monitor_bridge_current(
    status: &serde_json::Value,
    descriptor: &BrainMediaDescriptor,
) -> bool {
    status.get("armed").and_then(serde_json::Value::as_bool) == Some(true)
        && status.get("ready").and_then(serde_json::Value::as_bool) == Some(true)
        && status.get("fault").is_none_or(serde_json::Value::is_null)
        && status.get("epochs").is_some_and(|e| {
            e.get("source").and_then(serde_json::Value::as_u64) == Some(descriptor.stagebox_epoch)
                && e.get("destination").and_then(serde_json::Value::as_u64)
                    == Some(descriptor.brain_epoch)
                && e.get("route").and_then(serde_json::Value::as_u64)
                    == Some(descriptor.selection_generation)
        })
}
#[cfg(test)]
mod readiness_tests {
    use super::*;
    #[test]
    fn monitor_readiness_rejects_fresh_observation_of_an_old_route() {
        let descriptor = BrainMediaDescriptor {
            contract: "GP15-media".into(),
            version: 1,
            session: 1,
            stagebox_epoch: 2,
            brain_epoch: 3,
            stagebox_map: 4,
            brain_map: 5,
            generation: 6,
            selection_generation: 7,
            hold_generation: 8,
            sample_rate: 48000,
            frames: 48,
            talkback: true,
            monitor: true,
        };
        let valid = serde_json::json!({"armed":true,"ready":true,"fault":null,"epochs":{"source":2,"destination":3,"route":7}});
        assert!(brain_monitor_bridge_current(&valid, &descriptor));
        for key in ["source", "destination", "route"] {
            let mut stale = valid.clone();
            stale["epochs"][key] = serde_json::json!(99);
            assert!(!brain_monitor_bridge_current(&stale, &descriptor));
        }
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove("epochs");
        assert!(!brain_monitor_bridge_current(&missing, &descriptor));
        let mut fault = valid;
        fault["fault"] = serde_json::json!("starved");
        assert!(!brain_monitor_bridge_current(&fault, &descriptor));
    }
}

#[cfg(all(test, target_os = "linux", feature = "hardware-host"))]
mod deadline_admission_tests {
    use super::*;
    use crate::{
        brain_control::{Command as BrainCommand, Request as BrainRequest},
        control_model::{Command, Request as AudioRequest, Scope},
        remote::*,
        show::Counter,
    };
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn expired_hold_rejects_media_before_the_next_source_render() {
        let path = std::env::temp_dir().join(format!("gp-brain-deadline-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        let topology = crate::topology::EngineTopology::reference_16_18(0, 4).unwrap();
        let mut provider = crate::local_audio::LocalAudio::bind_configured(
            &path,
            "host",
            show,
            Counter(9),
            topology.clone(),
        )
        .unwrap();
        provider.rearm().unwrap();
        let policy = PolicyStore::new(vec![Peer {
            id: "duplex".into(),
            certificate_sha256: fingerprint(b"deadline-duplex"),
            permissions: [Permission::TalkbackDestinations].into_iter().collect(),
        }])
        .unwrap();
        let ctx = policy.authenticate(b"deadline-duplex", 12).unwrap();
        let grant = AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some(ctx.writer().into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant {
                scope: Scope::TalkbackDestinations,
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
        let capture = vec![0.; 48 * topology.capture_channels];
        let mut playback = vec![0.; 48 * topology.playback_channels];
        for (id, revision, now, command) in [
            (
                2,
                0,
                1,
                BrainCommand::TalkbackSet {
                    monitors: vec![0],
                    gain_cdb: 0,
                    mute: false,
                },
            ),
            (
                3,
                1,
                3,
                BrainCommand::Hold {
                    generation: Counter(1),
                },
            ),
        ] {
            let request = BrainRequest {
                contract: "GP15-brain".into(),
                version: 1,
                show_id: show.into(),
                module: "audio".into(),
                epoch: Counter(9),
                writer: Some(ctx.writer().into()),
                lease: Some(lease),
                request_id: Some(Counter(id)),
                expected_revision: Some(Counter(revision)),
                command,
            };
            host.provider_mut()
                .brain_request(request, now, true, None)
                .unwrap();
            for delta in 0..2 {
                let frame = host.provider().frame();
                host.process_source(now + delta, 9, frame, &capture, &mut playback)
                    .unwrap();
            }
        }
        assert_eq!(
            host.provider().brain_snapshot().held_generation,
            Some(Counter(1))
        );
        let descriptor = BrainMediaDescriptor {
            contract: "GP15-media".into(),
            version: 1,
            session: 12,
            stagebox_epoch: 9,
            brain_epoch: 77,
            stagebox_map: host.identity().map_generation.0,
            brain_map: 1,
            generation: 1,
            selection_generation: 1,
            hold_generation: 1,
            sample_rate: 48000,
            frames: 48,
            talkback: true,
            monitor: false,
        };
        host.dispatch(&ctx,serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":ctx.writer(),"descriptor":descriptor}),5).unwrap();
        let packet = BrainPacket {
            descriptor,
            role: BrainMediaRole::Talkback,
            frame: 480_000,
            samples: vec![0.125; 48],
        }
        .encode()
        .unwrap();
        assert!(
            host.brain_media(&ctx, &packet, 200)
                .unwrap_err()
                .contains("deadman")
        );
        assert_eq!(
            host.provider().brain_snapshot().held_generation,
            Some(Counter(1)),
            "no render occurred to clear expired stored state"
        );
        assert_eq!(host.media_stats().accepted_talkback_packets, 0);
        assert_eq!(host.media_stats().rejected_talkback_packets, 1);
        assert_eq!(host.brain_bridge_status().unwrap().occupancy_frames, 0);
        drop(host);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[cfg(all(test, target_os = "linux", feature = "hardware-host"))]
mod monitor_renegotiation_tests {
    use super::*;
    use crate::{
        brain_control::{Command as BrainCommand, Request as BrainRequest},
        control_model::{Command, Request as AudioRequest, Scope},
        remote::*,
        show::Counter,
    };
    use std::os::unix::fs::PermissionsExt;

    fn negotiate(
        host: &mut HostAuthority,
        ctx: &AuthenticatedContext,
        d: &BrainMediaDescriptor,
        now: u64,
    ) -> crate::remote::Result<Option<serde_json::Value>> {
        host.dispatch(ctx, serde_json::json!({"contract":"GP15-media","version":1,"kind":"negotiate","writer":ctx.writer(),"descriptor":d}), now)
    }
    fn render(host: &mut HostAuthority, d: &BrainMediaDescriptor, now: u64) -> Vec<u8> {
        let frame = host.provider().frame();
        let capture = vec![0.125; 48 * host.provider().topology().capture_channels];
        let mut playback = vec![0.; 48 * host.provider().topology().playback_channels];
        host.process_source(now, d.stagebox_epoch, frame, &capture, &mut playback)
            .unwrap();
        BrainPacket {
            descriptor: d.clone(),
            role: BrainMediaRole::Monitor,
            frame,
            samples: host.provider().brain_monitor_output().to_vec(),
        }
        .encode()
        .unwrap()
    }
    #[test]
    fn queued_monitor_frames_survive_hold_and_release_but_not_identity_changes() {
        let path =
            std::env::temp_dir().join(format!("gp-monitor-renegotiate-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let show = "11111111-1111-4111-8111-111111111111";
        let mut provider = crate::local_audio::LocalAudio::bind_configured(
            &path,
            "host",
            show,
            Counter(9),
            crate::topology::EngineTopology::reference_16_18(0, 4).unwrap(),
        )
        .unwrap();
        provider.rearm().unwrap();
        let policy = PolicyStore::new(vec![Peer {
            id: "duplex".into(),
            certificate_sha256: fingerprint(b"monitor-renegotiate"),
            permissions: [
                Permission::TalkbackDestinations,
                Permission::LocalOperatorMonitor,
            ]
            .into_iter()
            .collect(),
        }])
        .unwrap();
        let ctx = policy.authenticate(b"monitor-renegotiate", 12).unwrap();
        let grant = AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: show.into(),
            module: "audio".into(),
            epoch: Counter(9),
            writer: Some(ctx.writer().into()),
            lease: None,
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(0)),
            command: Command::Grant {
                scope: Scope::TalkbackDestinations,
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
        let mut d = BrainMediaDescriptor {
            contract: "GP15-media".into(),
            version: 1,
            session: 12,
            stagebox_epoch: 9,
            brain_epoch: 77,
            stagebox_map: host.identity().map_generation.0,
            brain_map: 1,
            generation: 1,
            selection_generation: 1,
            hold_generation: 0,
            sample_rate: 48000,
            frames: 48,
            talkback: true,
            monitor: true,
        };
        negotiate(&mut host, &ctx, &d, 0).unwrap();
        let mut expected = vec![render(&mut host, &d, 0)];
        for (id, revision, now, command) in [
            (
                2,
                0,
                1,
                BrainCommand::TalkbackSet {
                    monitors: vec![0],
                    gain_cdb: 0,
                    mute: false,
                },
            ),
            (
                3,
                1,
                3,
                BrainCommand::Hold {
                    generation: Counter(1),
                },
            ),
        ] {
            host.provider_mut()
                .brain_request(
                    BrainRequest {
                        contract: "GP15-brain".into(),
                        version: 1,
                        show_id: show.into(),
                        module: "audio".into(),
                        epoch: Counter(9),
                        writer: Some(ctx.writer().into()),
                        lease: Some(lease),
                        request_id: Some(Counter(id)),
                        expected_revision: Some(Counter(revision)),
                        command,
                    },
                    now,
                    true,
                    None,
                )
                .unwrap();
            for delta in 0..2 {
                expected.push(render(&mut host, &d, now + delta));
            }
        }
        assert_eq!(
            host.provider().brain_snapshot().held_generation,
            Some(Counter(1))
        );
        let old_unheld = d.clone();
        d.generation += 1;
        d.hold_generation = 1;
        negotiate(&mut host, &ctx, &d, 5).unwrap();
        expected.push(render(&mut host, &d, 5));
        // Old implementation returned only the newest frame here: every queued
        // pre-renegotiation frame was discarded. Byte equality also proves no
        // rewriting, duplicated samples, or incorrect ordering was introduced.
        for bytes in expected.drain(..) {
            assert_eq!(host.poll_brain_media(&ctx).unwrap(), Some(bytes));
        }
        assert!(host.poll_brain_media(&ctx).unwrap().is_none());
        let packet = |descriptor: &BrainMediaDescriptor, frame| {
            BrainPacket {
                descriptor: descriptor.clone(),
                role: BrainMediaRole::Talkback,
                frame,
                samples: vec![0.125; 48],
            }
            .encode()
            .unwrap()
        };
        assert!(
            host.brain_media(&ctx, &packet(&old_unheld, 480_000), 5)
                .is_err()
        );
        host.brain_media(&ctx, &packet(&d, 480_000), 5).unwrap();
        host.test_drain_media_ingress(5);
        assert_eq!(host.media_stats().accepted_talkback_packets, 1);
        let old_held = d.clone();
        expected.push(render(&mut host, &d, 6));
        host.provider_mut()
            .brain_request(
                BrainRequest {
                    contract: "GP15-brain".into(),
                    version: 1,
                    show_id: show.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some(ctx.writer().into()),
                    lease: Some(lease),
                    request_id: Some(Counter(4)),
                    expected_revision: Some(Counter(2)),
                    command: BrainCommand::Release {
                        generation: Counter(1),
                    },
                },
                7,
                true,
                None,
            )
            .unwrap();
        for now in 7..9 {
            expected.push(render(&mut host, &d, now));
        }
        assert!(host.provider().brain_snapshot().held_generation.is_none());
        d.generation += 1;
        d.hold_generation = 0;
        negotiate(&mut host, &ctx, &d, 9).unwrap();
        expected.push(render(&mut host, &d, 9));
        for bytes in expected {
            assert_eq!(host.poll_brain_media(&ctx).unwrap(), Some(bytes));
        }
        assert!(host.poll_brain_media(&ctx).unwrap().is_none());
        assert!(
            host.brain_media(&ctx, &packet(&old_held, 480_048), 9)
                .is_err()
        );
        assert_eq!(host.media_stats().accepted_talkback_packets, 1);

        // A refused identity change must not consume an already queued packet.
        let queued = render(&mut host, &d, 10);
        let mut invalid = d.clone();
        invalid.generation += 1;
        invalid.stagebox_map += 1;
        assert!(negotiate(&mut host, &ctx, &invalid, 10).is_err());
        assert_eq!(host.poll_brain_media(&ctx).unwrap(), Some(queued));
        for change in 0..3 {
            let _ = render(&mut host, &d, 11 + change);
            d.generation += 1;
            match change {
                0 => d.brain_epoch += 1,
                1 => d.brain_map += 1,
                _ => d.monitor = false,
            }
            negotiate(&mut host, &ctx, &d, 11 + change).unwrap();
            assert!(
                host.poll_brain_media(&ctx).unwrap().is_none(),
                "new device epoch/map or disabled monitor must discard old frames"
            );
        }
        d.monitor = true;
        d.generation += 1;
        negotiate(&mut host, &ctx, &d, 14).unwrap();
        let _ = render(&mut host, &d, 14);
        // Each granted writer retains one scope; monitor control therefore uses
        // its own authenticated session rather than regranting the TB writer.
        let monitor_ctx = policy.authenticate(b"monitor-renegotiate", 13).unwrap();
        let grant_monitor = AudioRequest {
            command: Command::Grant {
                scope: Scope::LocalOperatorMonitor,
            },
            writer: Some(monitor_ctx.writer().into()),
            request_id: Some(Counter(1)),
            expected_revision: Some(Counter(3)),
            ..grant.clone()
        };
        let monitor_lease = host
            .provider_mut()
            .engine_mut()
            .handle(&grant_monitor, 15)
            .unwrap()
            .outcome
            .unwrap()
            .body
            .granted_lease
            .unwrap();
        host.provider_mut()
            .brain_request(
                BrainRequest {
                    contract: "GP15-brain".into(),
                    version: 1,
                    show_id: show.into(),
                    module: "audio".into(),
                    epoch: Counter(9),
                    writer: Some(monitor_ctx.writer().into()),
                    lease: Some(monitor_lease),
                    request_id: Some(Counter(2)),
                    expected_revision: Some(Counter(3)),
                    command: BrainCommand::MonitorSet {
                        source: crate::brain_control::MonitorSource::Main,
                        gain_cdb: 0,
                        mute: false,
                        dim: false,
                        armed: false,
                    },
                },
                15,
                true,
                None,
            )
            .unwrap();
        for now in 15..17 {
            let _ = render(&mut host, &d, now);
        }
        let selection = host.provider().brain_snapshot().selection_generation.0;
        assert_ne!(selection, d.selection_generation);
        d.selection_generation = selection;
        d.generation += 1;
        negotiate(&mut host, &ctx, &d, 17).unwrap();
        assert!(
            host.poll_brain_media(&ctx).unwrap().is_none(),
            "source selection must not replay old-source queued packets"
        );
        let fresh = render(&mut host, &d, 17);
        assert_eq!(host.poll_brain_media(&ctx).unwrap(), Some(fresh));
        drop(host);
        std::fs::remove_dir_all(path).unwrap();
    }
}
