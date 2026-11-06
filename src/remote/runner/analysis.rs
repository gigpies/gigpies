//! Read-only analysis connection and runtime. No FX samples or controller
//! commands enter this worker; its finite report contains counters and a digest.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default, serde::Serialize)]
pub(crate) struct Report {
    pub descriptor: Option<MediaDescriptor>,
    pub first_source: Option<u64>,
    pub last_source: Option<u64>,
    pub packets: u64,
    pub mismatches: u64,
    pub channel_samples: std::collections::BTreeMap<String, u64>,
    pub sha256: String,
    pub fault: Option<String>,
}
pub(crate) struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<Report>>>,
}
impl Worker {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        mut bind: SocketAddr,
        address: SocketAddr,
        server_name: String,
        credentials: Credentials,
        policy: PolicyStore,
        identity: EngineIdentity,
        ids: Vec<String>,
        verify: bool,
        deadline: Instant,
    ) -> Result<Self> {
        bind.set_port(0);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new().name("gp-analysis-media".into()).spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
                .map_err(|e| e.to_string())?;
            runtime.block_on(async move {
                let endpoint = client_endpoint(bind, &credentials)?;
                let mut client = RemoteClient::connect(endpoint, address, &server_name, policy).await?;
                if client.identity() != &identity { return Err("analysis source changed during attachment".into()); }
                let descriptor = MediaDescriptor { session: Counter(client.session()),
                    source_epoch: identity.source_epoch, capability_generation: identity.capability_generation,
                    map_generation: identity.map_generation, sample_rate: 48000,
                    streams: grouped_streams(MediaRole::Analysis, &ids, 48, 0, 1, client.max_datagram())? };
                client.negotiate(descriptor.clone()).await?;
                if !matches!(client.receive().await?, Response::MediaAccepted { .. }) {
                    return Err("analysis negotiation refused".into());
                }
                let mut channel = client.media_channel()?;
                let references: Vec<_> = ids.iter().map(|id| verify.then(|| analysis_reference(id)).transpose()).collect::<Result<_>>()?;
                let mut report = Report { descriptor: Some(descriptor.clone()),
                    channel_samples: ids.iter().cloned().map(|id| (id, 0)).collect(), ..Report::default() };
                let mut hash = Sha256::new();
                let mut timer = tokio::time::interval(Duration::from_millis(1));
                timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
                    tokio::select! {
                        _ = timer.tick() => (),
                        received = channel.receive() => {
                            let bytes = match received { Ok(bytes) => bytes, Err(error) => { report.fault = Some(error); break; } };
                            let packet = crate::transport::Packet::parse(&bytes).map_err(|e| format!("analysis evidence {e:?}"))?;
                            let stream = descriptor.streams.iter().find(|s| s.stream == packet.spec().stream).ok_or("analysis evidence stream")?;
                            report.first_source.get_or_insert(packet.source_frame());
                            report.last_source = Some(packet.source_frame());
                            report.packets += 1;
                            for (channel, id) in stream.channel_ids.iter().enumerate() {
                                let reference = references[stream.first_channel as usize + channel];
                                for f in 0..usize::from(packet.spec().frames) {
                                    let actual = packet.pcm(f * usize::from(packet.spec().channels) + channel).map_err(|e| format!("analysis PCM {e:?}"))?;
                                    if let Some((period, amplitude)) = reference {
                                        let expected = if ((packet.source_frame() + f as u64) / period).is_multiple_of(2) { amplitude } else { -amplitude };
                                        report.mismatches += u64::from(actual != expected);
                                    }
                                }
                                *report.channel_samples.get_mut(id).ok_or("analysis channel evidence")? += u64::from(packet.spec().frames);
                            }
                            hash.update(&bytes);
                        }
                    }
                }
                client.close();
                report.sha256 = format!("{:x}", hash.finalize());
                Ok(report)
            })
        }).map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
    pub fn finish(&mut self) -> Result<Report> {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .ok_or("analysis worker already joined")?
            .join()
            .map_err(|_| "analysis worker panicked".to_string())?
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.finish();
        }
    }
}
