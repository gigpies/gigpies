//! Local production handoff harness for combined owner lifecycle acceptance.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(crate) struct DuplexProbe {
    service: service::Service,
    stop: Arc<AtomicBool>,
    task: Option<std::thread::JoinHandle<()>>,
    pub attempted: Arc<AtomicU64>,
    pub captured: Arc<AtomicU64>,
    pub monitored: Arc<AtomicU64>,
}
impl DuplexProbe {
    pub fn start(
        mut proxy: AuthorityProxy,
        context: AuthenticatedContext,
        descriptor: BrainMediaDescriptor,
        now: u64,
    ) -> Self {
        let started = Instant::now();
        let mut service = service::Service::start(
            service::tests::config(),
            false,
            descriptor.brain_epoch,
            descriptor.stagebox_epoch,
            0,
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap();
        let mut lanes = service.take_media();
        let epochs = BridgeEpochs {
            source: descriptor.stagebox_epoch,
            destination: descriptor.brain_epoch,
            route: descriptor.selection_generation,
        };
        service.publish(service::State {
            closure: 0,
            revision: 1,
            epochs,
            hold: descriptor.hold_generation,
            monitor: true,
            levels: LocalLevels {
                microphone_muted: false,
                monitor_muted: false,
                ..LocalLevels::default()
            },
            expires: Instant::now() + Duration::from_millis(crate::brain_control::DEADMAN_MS),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let attempted = Arc::new(AtomicU64::new(0));
        let attempts = attempted.clone();
        let captured = Arc::new(AtomicU64::new(0));
        let monitored = Arc::new(AtomicU64::new(0));
        let (stopping, captures, monitors) = (stop.clone(), captured.clone(), monitored.clone());
        let task = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                for _ in 0..4 {
                    let Ok(block) = lanes.captured.pop() else {
                        break;
                    };
                    let bytes = BrainPacket {
                        descriptor: descriptor.clone(),
                        role: BrainMediaRole::Talkback,
                        frame: block.frame,
                        samples: block.samples.to_vec(),
                    }
                    .encode()
                    .unwrap();
                    attempts.fetch_add(1, Ordering::Release);
                    if proxy
                        .brain_media(&context, &bytes, now + started.elapsed().as_millis() as u64)
                        .is_ok()
                    {
                        captures.fetch_add(1, Ordering::Release);
                    }
                }
                for _ in 0..4 {
                    let Some(bytes) = proxy.poll_brain_media(&context).unwrap() else {
                        break;
                    };
                    let packet =
                        BrainPacket::decode(&bytes, &descriptor, BrainMediaRole::Monitor).unwrap();
                    if lanes
                        .monitor
                        .push(service::Monitor {
                            epochs,
                            frame: packet.frame,
                            samples: packet.samples.try_into().unwrap(),
                        })
                        .is_ok()
                    {
                        monitors.fetch_add(1, Ordering::Release);
                    }
                }
                std::thread::sleep(Duration::from_micros(250));
            }
        });
        Self {
            service,
            stop,
            task: Some(task),
            attempted,
            captured,
            monitored,
        }
    }
    pub fn observation(&mut self) -> BrainDeviceObservation {
        self.service.poll();
        super::observation(&self.service, &service::tests::config(), None, None, None)
    }
    pub fn close_capture(&mut self) {
        self.service.close_roles(true, false).unwrap();
    }
    pub fn finish(&mut self) -> (u64, u64, u64) {
        self.service.finish().unwrap();
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            task.join().unwrap();
        }
        (
            self.service.latest.host.capture_frame,
            self.service.latest.playback_dropped,
            self.service.latest.status_dropped,
        )
    }
}
impl Drop for DuplexProbe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

/// Authenticated counterpart: uses the production network worker and device
/// service, with playback evidence consumed only by the test controller.
pub(crate) struct AuthenticatedDuplexProbe {
    service: service::Service,
    media: media::MediaWorker,
    pub nonzero_playback: u64,
    pub playback_frames: u64,
    evidence_stop: Arc<AtomicBool>,
    evidence: Option<std::thread::JoinHandle<(u64, u64)>>,
    nonzero: Arc<AtomicU64>,
}
impl AuthenticatedDuplexProbe {
    pub fn start(
        channel: BrainMediaChannel,
        descriptor: &BrainMediaDescriptor,
        snapshot: &Snapshot,
        deadline: Instant,
    ) -> Self {
        let mut service = service::Service::start(
            service::tests::config(),
            false,
            descriptor.brain_epoch,
            descriptor.stagebox_epoch,
            0,
            deadline,
        )
        .unwrap();
        let media = media::MediaWorker::start(channel, service.take_media()).unwrap();
        media.route(Some(descriptor), Some(snapshot)).unwrap();
        service.publish(service::State {
            closure: 0,
            revision: 1,
            epochs: BridgeEpochs {
                source: descriptor.stagebox_epoch,
                destination: descriptor.brain_epoch,
                route: descriptor.selection_generation,
            },
            hold: descriptor.hold_generation,
            monitor: snapshot.monitor_armed,
            levels: LocalLevels {
                microphone_muted: false,
                monitor_muted: false,
                ..LocalLevels::default()
            },
            expires: Instant::now() + Duration::from_millis(crate::brain_control::DEADMAN_MS),
        });
        let (_, empty) = rtrb::RingBuffer::new(1);
        let mut playback = std::mem::replace(&mut service.playback, empty);
        let evidence_stop = Arc::new(AtomicBool::new(false));
        let stopping = evidence_stop.clone();
        let nonzero = Arc::new(AtomicU64::new(0));
        let progress = nonzero.clone();
        let evidence = std::thread::spawn(move || {
            let (mut frames, mut samples) = (0, 0);
            loop {
                while let Ok(block) = playback.pop() {
                    frames += block.frames as u64;
                    samples += block.samples[..block.frames * 2]
                        .iter()
                        .filter(|s| **s != 0.)
                        .count() as u64;
                }
                progress.store(samples, Ordering::Release);
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(Duration::from_micros(250));
            }
            (frames, samples)
        });
        Self {
            service,
            media,
            nonzero_playback: 0,
            playback_frames: 0,
            evidence_stop,
            evidence: Some(evidence),
            nonzero,
        }
    }
    pub fn observation(&mut self) -> BrainDeviceObservation {
        self.service.poll();
        super::observation(&self.service, &service::tests::config(), None, None, None)
    }
    pub fn poll(&mut self) {
        self.service.poll();
        self.nonzero_playback = self.nonzero.load(Ordering::Acquire);
    }
    pub fn close_capture(&mut self) {
        self.service.close_roles(true, false).unwrap();
    }
    pub fn finish(&mut self) -> serde_json::Value {
        self.service.finish().unwrap();
        self.evidence_stop.store(true, Ordering::Release);
        let (frames, samples) = self.evidence.take().unwrap().join().unwrap();
        self.playback_frames = frames;
        self.nonzero_playback = samples;
        let report = self.media.finish().unwrap();
        serde_json::json!({ "media": report,
            "capture_frame": self.service.latest.host.capture_frame,
            "nonzero_playback": self.nonzero_playback,
            "playback_frames": self.playback_frames,
            "playback_dropped": self.service.latest.playback_dropped,
            "service_fault": self.service.latest.service_fault.map(|e| format!("{e:?}")),
        })
    }
}

impl Drop for AuthenticatedDuplexProbe {
    fn drop(&mut self) {
        let _ = self.service.finish();
        self.evidence_stop.store(true, Ordering::Release);
        if let Some(evidence) = self.evidence.take() {
            let _ = evidence.join();
        }
    }
}
