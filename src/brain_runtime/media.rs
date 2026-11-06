//! Non-audio network execution. The service never awaits this worker, and this
//! worker never awaits controller writes. Authentication/replay stay in the
//! existing BrainMediaChannel; the worker adds no alternate admission path.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
struct Route {
    descriptor: Option<BrainMediaDescriptor>,
    hold: u64,
    monitor: bool,
    expires: Instant,
}
enum Update {
    Route(Route),
    Attach(service::MediaLanes),
}
#[derive(Default, Serialize)]
pub(super) struct Report {
    pub captured_packets: u64,
    pub monitor_packets: u64,
    pub rejected_monitor_packets: u64,
    pub stale_capture_blocks: u64,
    pub send_refused: u64,
    pub captured_pre_wire_sha256: String,
    pub received_monitor_sha256: String,
}
pub(super) struct MediaWorker {
    updates: tokio::sync::mpsc::Sender<Update>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<Report>>>,
}
impl MediaWorker {
    pub fn start(mut channel: BrainMediaChannel, mut lanes: service::MediaLanes) -> Result<Self> {
        let (updates, mut rx) = tokio::sync::mpsc::channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new().name("gp-brain-media".into()).spawn(move || -> Result<Report> {
            let runtime=match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime)=>runtime,
                Err(error)=>{ let error=error.to_string(); let _=ready_tx.send(Err(error.clone())); return Err(error); }
            };
            Ok(runtime.block_on(async move {
            let mut route = Route { descriptor: None, hold: 0, monitor: false, expires: Instant::now() };
            let mut report = Report::default();
            let mut pending_capture: Option<Captured> = None;
            let mut capture_hash = Sha256::new();
            let mut monitor_hash = Sha256::new();
            let mut timer = tokio::time::interval(Duration::from_micros(250));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let _=ready_tx.send(Ok(()));
            while !stopping.load(Ordering::Acquire) {
                tokio::select! {
                    update = rx.recv() => match update {
                        Some(Update::Route(next)) => route = next,
                        Some(Update::Attach(next)) => { lanes=next; route.descriptor=None; route.hold=0; route.monitor=false; },
                        None => break,
                    },
                    _ = timer.tick() => {
                        for _ in 0..4 {
                            let Some(block) = pending_capture.take().or_else(||lanes.captured.pop().ok()) else { break };
                            if Instant::now()<route.expires && route.descriptor.as_ref().is_none_or(|d|channel.accepted_generation()!=d.generation) {
                                pending_capture=Some(block);
                                break;
                            }
                            if let Some(d) = &route.descriptor && Instant::now() < route.expires && route.hold != 0 && block.hold_generation == route.hold && d.hold_generation == route.hold && block.epoch == d.brain_epoch {
                                if channel.send(&BrainPacket { descriptor:d.clone(), role:BrainMediaRole::Talkback, frame:block.frame, samples:block.samples.to_vec() }).is_ok() {
                                    for sample in block.samples { capture_hash.update(sample.to_le_bytes()); }
                                    report.captured_packets+=1;
                                } else if channel.accepted_generation()!=d.generation {
                                    pending_capture=Some(block);
                                    break;
                                } else { report.send_refused+=1; }
                            } else { report.stale_capture_blocks+=1; }
                        }
                    },
                    packet = async { if let Some(d) = &route.descriptor { channel.receive(d).await } else { std::future::pending().await } }, if lanes.monitor.slots() > 0 => {
                        match packet {
                            Ok(packet) if route.monitor && Instant::now()<route.expires && packet.samples.len()==96 => {
                                for sample in &packet.samples { monitor_hash.update(sample.to_le_bytes()); }
                                let mut samples=[0.;96]; samples.copy_from_slice(&packet.samples);
                                let epochs=BridgeEpochs { source:packet.descriptor.stagebox_epoch, destination:packet.descriptor.brain_epoch, route:packet.descriptor.selection_generation };
                                if lanes.monitor.push(service::Monitor { epochs, frame:packet.frame, samples }).is_err() { lanes.dropped.fetch_add(1,Ordering::Relaxed); }
                                report.monitor_packets+=1;
                            },
                            Ok(_) => { report.rejected_monitor_packets+=1; },
                            Err(_) => {
                                report.rejected_monitor_packets+=1;
                                // An in-flight negotiation can change the accepted
                                // generation before its route update reaches us.
                                // Wait for that update rather than spin on check().
                                route.descriptor=None;
                            }
                        }
                    }
                }
            }
            report.stale_capture_blocks+=u64::from(pending_capture.is_some());
            for _ in 0..4 { if lanes.captured.pop().is_ok() { report.stale_capture_blocks+=1; } else { break; } }
            report.captured_pre_wire_sha256=crate::hash::hex(capture_hash.finalize());
            report.received_monitor_sha256=crate::hash::hex(monitor_hash.finalize());
            report
        }))
        }).map_err(|e|e.to_string())?;
        if let Err(error) = ready_rx
            .recv()
            .map_err(|e| e.to_string())
            .and_then(|ready| ready)
        {
            let _ = thread.join();
            return Err(error);
        }
        Ok(Self {
            updates,
            stop,
            thread: Some(thread),
        })
    }
    pub fn route(
        &self,
        descriptor: Option<&BrainMediaDescriptor>,
        snapshot: Option<&Snapshot>,
    ) -> Result<()> {
        let hold = snapshot.and_then(|s| s.held_generation).map_or(0, |g| g.0);
        let monitor = descriptor.zip(snapshot).is_some_and(|(d, s)| {
            s.monitor_armed && monitor_selection_current(Some(s.selection_generation.0), d)
        });
        self.updates
            .try_send(Update::Route(Route {
                descriptor: descriptor.cloned(),
                hold,
                monitor,
                expires: Instant::now()
                    + Duration::from_millis(
                        snapshot.map_or(0, |s| s.deadman_ms.min(crate::brain_control::DEADMAN_MS)),
                    ),
            }))
            .map_err(|_| "Brain media control queue full/closed".into())
    }
    pub fn attach(&self, lanes: service::MediaLanes) -> Result<()> {
        self.updates
            .try_send(Update::Attach(lanes))
            .map_err(|_| "Brain media replacement queue full/closed".into())
    }
    pub fn finish(&mut self) -> Result<Report> {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .ok_or("Brain media already joined")?
            .join()
            .map_err(|_| "Brain media worker panicked".to_owned())?
    }
}
impl Drop for MediaWorker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::show::Counter;
    use std::sync::atomic::AtomicU64;
    #[test]
    fn monitor_receive_pressure_preserves_existing_four_block_capacity() {
        fn schedule(reserve: bool) -> (usize, Vec<u64>) {
            let (mut incoming, mut service) = rtrb::RingBuffer::new(4);
            let (mut next, mut drops) = (0, 0);
            let mut delivered = Vec::new();
            while next < 12 {
                // Network is runnable for an eight-packet burst while service
                // is withheld. Full-lane admission must leave bytes in transport.
                for _ in 0..8 {
                    if next == 12 || (reserve && incoming.slots() == 0) {
                        break;
                    }
                    if incoming.push(next * 48).is_err() {
                        drops += 1;
                    }
                    next += 1;
                }
                while let Ok(frame) = service.pop() {
                    delivered.push(frame);
                }
            }
            (drops, delivered)
        }
        let old = schedule(false);
        assert_eq!(old.0, 4);
        assert_eq!(old.1[4], 384);
        let fixed = schedule(true);
        assert_eq!(fixed.0, 0);
        assert_eq!(fixed.1, (0..12).map(|n| n * 48).collect::<Vec<_>>());
    }
    // The remote fixture supplies authenticated paced monitor packets; processing
    // and device service are the production MediaWorker/BrainHost/Bridge owners.
    struct Source {
        descriptor: Option<BrainMediaDescriptor>,
        frame: u64,
        due: Instant,
        received: Arc<AtomicU64>,
        gaps: Arc<AtomicU64>,
        last_capture: Option<u64>,
    }
    impl AuthorityEndpoint for Source {
        fn identity(&self) -> EngineIdentity {
            EngineIdentity {
                source_epoch: Counter(9),
                capability_generation: Counter(1),
                map_generation: Counter(1),
            }
        }
        fn dispatch(
            &mut self,
            context: &AuthenticatedContext,
            payload: Value,
            _: u64,
        ) -> Result<Option<Value>> {
            context.check_writer(&payload)?;
            context.require(&Permission::LocalOperatorMonitor)?;
            if payload["contract"] != "GP15-media" {
                return Err("fixture only admits media negotiation".into());
            }
            let descriptor: BrainMediaDescriptor =
                serde_json::from_value(payload["descriptor"].clone()).map_err(|e| e.to_string())?;
            descriptor.validate(context.session(), &self.identity())?;
            self.descriptor = Some(descriptor.clone());
            self.due = Instant::now();
            Ok(Some(
                json!({"contract":"GP15-media","version":1,"state":"accepted","descriptor":descriptor}),
            ))
        }
        fn negotiate(&mut self, _: &AuthenticatedContext, _: &MediaDescriptor) -> Result<bool> {
            Err("not GPA1".into())
        }
        fn media(
            &mut self,
            _: &AuthenticatedContext,
            _: crate::transport::Packet<'_>,
            _: u64,
        ) -> Result<()> {
            Err("not GPA1".into())
        }
        fn brain_media(&mut self, _: &AuthenticatedContext, bytes: &[u8], _: u64) -> Result<()> {
            let packet = BrainPacket::decode(
                bytes,
                self.descriptor.as_ref().ok_or("descriptor")?,
                BrainMediaRole::Talkback,
            )?;
            if self
                .last_capture
                .is_some_and(|last| packet.frame != last + 48)
            {
                self.gaps.fetch_add(1, Ordering::Relaxed);
            }
            self.last_capture = Some(packet.frame);
            self.received.fetch_add(1, Ordering::Release);
            Ok(())
        }
        fn poll_brain_media(&mut self, _: &AuthenticatedContext) -> Result<Option<Vec<u8>>> {
            let Some(d) = &self.descriptor else {
                return Ok(None);
            };
            if Instant::now() < self.due {
                return Ok(None);
            }
            self.due += Duration::from_millis(1);
            let frame = self.frame;
            self.frame += 48;
            BrainPacket {
                descriptor: d.clone(),
                role: BrainMediaRole::Monitor,
                frame,
                samples: vec![0.125; 96],
            }
            .encode()
            .map(Some)
        }
        fn disconnect(&mut self, _: &AuthenticatedContext) {}
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn negotiation_handoff_retains_the_actual_queued_capture_block() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let (server_credentials, client_credentials) = crate::remote::test_credentials();
            let peer = |certificate: &[u8]| Peer {
                id: "test".into(),
                certificate_sha256: fingerprint(certificate),
                permissions: [
                    Permission::LocalOperatorMonitor,
                    Permission::TalkbackDestinations,
                ]
                .into_iter()
                .collect(),
            };
            let server = RemoteServer::bind(
                "127.0.0.1:0".parse().unwrap(),
                &server_credentials,
                PolicyStore::new(vec![peer(client_credentials.certificate_chain[0].as_ref())])
                    .unwrap(),
            )
            .unwrap();
            let address = server.local_addr().unwrap();
            let received = Arc::new(AtomicU64::new(0));
            let gaps = Arc::new(AtomicU64::new(0));
            let remote_received = received.clone();
            let remote_gaps = gaps.clone();
            let server_task = tokio::spawn(async move {
                let session = server.accept().await.unwrap();
                let mut source = Source {
                    descriptor: None,
                    frame: 0,
                    due: Instant::now(),
                    received: remote_received,
                    gaps: remote_gaps,
                    last_capture: None,
                };
                let result = session.run(&mut source).await;
                server.close();
                result
            });
            let mut client = RemoteClient::connect(
                client_endpoint("127.0.0.1:0".parse().unwrap(), &client_credentials).unwrap(),
                address,
                "stagebox.test",
                PolicyStore::new(vec![peer(server_credentials.certificate_chain[0].as_ref())])
                    .unwrap(),
            )
            .await
            .unwrap();
            let mut descriptor = BrainMediaDescriptor {
                contract: "GP15-media".into(),
                version: 1,
                session: client.session(),
                stagebox_epoch: 9,
                brain_epoch: 2,
                stagebox_map: 1,
                brain_map: 2,
                generation: 1,
                selection_generation: 3,
                hold_generation: 11,
                sample_rate: 48000,
                frames: 48,
                talkback: true,
                monitor: true,
            };
            client
                .negotiate_brain_media(descriptor.clone())
                .await
                .unwrap();
            client.receive().await.unwrap();
            let (mut capture, captured) = rtrb::RingBuffer::new(4);
            let (monitor, _monitor_rx) = rtrb::RingBuffer::new(4);
            let mut worker = MediaWorker::start(
                client.take_brain_media().unwrap(),
                service::MediaLanes {
                    captured,
                    monitor,
                    dropped: Arc::new(AtomicU64::new(0)),
                },
            )
            .unwrap();
            let publish = |worker: &MediaWorker, descriptor: &BrainMediaDescriptor| {
                worker
                    .updates
                    .try_send(Update::Route(Route {
                        descriptor: Some(descriptor.clone()),
                        hold: 11,
                        monitor: false,
                        expires: Instant::now() + Duration::from_secs(2),
                    }))
                    .unwrap()
            };
            publish(&worker, &descriptor);
            capture
                .push(Captured {
                    epoch: 2,
                    frame: 0,
                    hold_generation: 11,
                    samples: [0.125; 48],
                })
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while received.load(Ordering::Acquire) < 1 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            descriptor.generation = 2;
            descriptor.selection_generation = 4;
            client
                .negotiate_brain_media(descriptor.clone())
                .await
                .unwrap();
            client.receive().await.unwrap();
            // Accepted control generation changes before delivery to the media
            // worker. The held-role identity is unchanged across this handoff.
            capture
                .push(Captured {
                    epoch: 2,
                    frame: 48,
                    hold_generation: 11,
                    samples: [0.25; 48],
                })
                .unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert_eq!(received.load(Ordering::Acquire), 1);
            publish(&worker, &descriptor);
            tokio::time::timeout(Duration::from_secs(2), async {
                while received.load(Ordering::Acquire) < 2 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            let report = worker.finish().unwrap();
            assert_eq!(report.captured_packets, 2);
            assert_eq!(report.stale_capture_blocks, 0);
            assert_eq!(report.send_refused, 0);
            assert_eq!(gaps.load(Ordering::Acquire), 0);
            client.close();
            let _ = server_task.await.unwrap();
        })
        .await
        .unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticated_media_and_actual_duplex_keep_running_while_controller_stalls() {
        tokio::time::timeout(Duration::from_secs(10), async {
            let (server_credentials, client_credentials) = crate::remote::test_credentials();
            let peer = |certificate: &[u8]| Peer {
                id: "test".into(),
                certificate_sha256: fingerprint(certificate),
                permissions: [
                    Permission::LocalOperatorMonitor,
                    Permission::TalkbackDestinations,
                ]
                .into_iter()
                .collect(),
            };
            let server = RemoteServer::bind(
                "127.0.0.1:0".parse().unwrap(),
                &server_credentials,
                PolicyStore::new(vec![peer(client_credentials.certificate_chain[0].as_ref())])
                    .unwrap(),
            )
            .unwrap();
            let address = server.local_addr().unwrap();
            let server_task = tokio::spawn(async move {
                let session = server.accept().await.unwrap();
                let mut source = Source {
                    descriptor: None,
                    frame: 0,
                    due: Instant::now(),
                    received: Arc::new(AtomicU64::new(0)),
                    gaps: Arc::new(AtomicU64::new(0)),
                    last_capture: None,
                };
                let result = session.run(&mut source).await;
                server.close();
                result
            });
            let mut client = RemoteClient::connect(
                client_endpoint("127.0.0.1:0".parse().unwrap(), &client_credentials).unwrap(),
                address,
                "stagebox.test",
                PolicyStore::new(vec![peer(server_credentials.certificate_chain[0].as_ref())])
                    .unwrap(),
            )
            .await
            .unwrap();
            let descriptor = BrainMediaDescriptor {
                contract: "GP15-media".into(),
                version: 1,
                session: client.session(),
                stagebox_epoch: 9,
                brain_epoch: 2,
                stagebox_map: 1,
                brain_map: 2,
                generation: 1,
                selection_generation: 3,
                hold_generation: 11,
                sample_rate: 48000,
                frames: 48,
                talkback: true,
                monitor: true,
            };
            client
                .negotiate_brain_media(descriptor.clone())
                .await
                .unwrap();
            assert!(matches!(
                client.receive().await.unwrap(),
                Response::Reply { .. }
            ));
            let mut service = service::Service::start(
                service::tests::config(),
                false,
                2,
                9,
                0,
                Instant::now() + Duration::from_millis(500),
            )
            .unwrap();
            let mut worker =
                MediaWorker::start(client.take_brain_media().unwrap(), service.take_media())
                    .unwrap();
            let epochs = BridgeEpochs {
                source: 9,
                destination: 2,
                route: 3,
            };
            let expires = Instant::now() + Duration::from_millis(150);
            worker
                .updates
                .try_send(Update::Route(Route {
                    descriptor: Some(descriptor),
                    hold: 11,
                    monitor: true,
                    expires,
                }))
                .unwrap();
            service.publish(service::State {
                closure: 0,
                revision: 1,
                epochs,
                hold: 11,
                monitor: true,
                levels: LocalLevels {
                    microphone_muted: false,
                    monitor_muted: false,
                    ..LocalLevels::default()
                },
                expires,
            });
            // This thread does no control reads, status polling, packet handling,
            // diagnostic draining or async yielding during the stalled interval.
            std::thread::sleep(Duration::from_millis(80));
            service.finish().unwrap();
            let report = worker.finish().unwrap();
            assert!(service.latest.host.capture_frame >= 480);
            assert!(report.captured_packets >= 10);
            assert!(report.monitor_packets >= 10);
            assert!(service.latest.playback_dropped > 0);
            client.close();
            let _ = server_task.await.unwrap();
        })
        .await
        .unwrap();
    }
}
