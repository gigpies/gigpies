//! Explicit bounded authenticated synthetic or separately authorized physical runs.
//! Configuration, credentials and reports remain task-private. This is not a
//! service installer, load benchmark or hardware qualification.
pub(crate) mod analysis;
use super::*;
use crate::{
    control_model::{Command, Request as AudioRequest},
    local_audio::LocalAudio,
    show::Counter,
    topology::EngineTopology,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicalProviderConfig {
    pub device: String,
    pub topology: EngineTopology,
    pub acceptance: crate::host::duplex::DeviceAcceptance,
    pub period_frames: usize,
    pub buffer_frames: usize,
    pub epoch_file: PathBuf,
}
fn validate_provider_run(
    physical: Option<&PhysicalProviderConfig>,
    authorized: bool,
    duration_ms: u64,
    period_ms: u64,
    inputs: usize,
    monitors: usize,
    pa_outputs: usize,
) -> Result<()> {
    if physical.is_some() && !authorized {
        return Err("physical Stagebox requires explicit --activate-physical".into());
    }
    let maximum = if physical.is_some() {
        86_400_000
    } else {
        60_000
    };
    if !(1..=maximum).contains(&duration_ms) || !(1..=100).contains(&period_ms) {
        return Err("bounded provider duration/period required".into());
    }
    if let Some(p) = physical {
        if !p.device.starts_with("hw:") || p.device.len() <= 3 {
            return Err("physical provider requires an explicit raw hw endpoint".into());
        }
        if p.period_frames != 48 || !(96..=48_000).contains(&p.buffer_frames) {
            return Err(
                "physical provider deployment requires48-frame periods and96..48000-frame buffer"
                    .into(),
            );
        }
        if p.topology.inputs.len() != inputs
            || p.topology.monitors != monitors
            || p.topology.pa_outputs != pa_outputs
        {
            return Err("physical topology dimensions do not match assertions".into());
        }
        p.topology
            .validate(crate::topology::ResourceBudget::default())?;
        p.acceptance
            .validate(&p.device, &p.topology)
            .map_err(|e| e.to_string())?;
        if !p.epoch_file.is_absolute() {
            return Err("physical epoch file must be absolute".into());
        }
    }
    Ok(())
}
fn default_true() -> bool {
    true
}
fn validate_fx_run(duration_ms: u64, delay: u32, verify_synthetic: bool) -> Result<()> {
    let maximum = if verify_synthetic { 60_000 } else { 86_400_000 };
    if !(1..=maximum).contains(&duration_ms)
        || !(48..=1536).contains(&delay)
        || !delay.is_multiple_of(48)
    {
        return Err("bounded Brain duration/deadline required".into());
    }
    Ok(())
}
#[cfg(test)]
fn expected_analysis_pcm(verify_synthetic: bool, id: &str, frame: u64) -> Result<Option<i32>> {
    if !verify_synthetic {
        return Ok(None);
    }
    let (period, amplitude) = analysis_reference(id)?;
    Ok(Some(if (frame / period).is_multiple_of(2) {
        amplitude
    } else {
        -amplitude
    }))
}
fn analysis_reference(id: &str) -> Result<(u64, i32)> {
    let index = id
        .strip_prefix("input-")
        .ok_or("analysis source ID")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?
        .checked_sub(1)
        .ok_or("analysis source index")?;
    let period = index.checked_add(24).ok_or("analysis source period")?;
    let amplitude = i32::try_from(
        index
            .checked_add(1)
            .and_then(|v| v.checked_mul(128))
            .ok_or("analysis source amplitude")?,
    )
    .map_err(|e| e.to_string())?;
    Ok((period, amplitude))
}
// Optional software diagnostic, outside physical callbacks. Values are local to
// the controller dispatch thread; gp-source CPU/faults are not included.
fn thread_resources() -> Option<[u64; 5]> {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut usage) } != 0 {
        return None;
    }
    Some([
        ((usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) * 1_000_000
            + usage.ru_utime.tv_usec
            + usage.ru_stime.tv_usec) as u64,
        usage.ru_minflt as u64,
        usage.ru_majflt as u64,
        usage.ru_nvcsw as u64,
        usage.ru_nivcsw as u64,
    ])
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FxAuxConfig {
    pub library: PathBuf,
    pub sha256: String,
}
#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunConfig {
    Provider {
        bind: SocketAddr,
        certificate: PathBuf,
        key: PathBuf,
        ca: PathBuf,
        peers: Vec<Peer>,
        private_directory: PathBuf,
        manifest: PathBuf,
        pa_configuration: Option<PathBuf>,
        record_take: Option<String>,
        show_id: String,
        source_epoch: u64,
        inputs: usize,
        monitors: usize,
        pa_outputs: usize,
        reference_16_18: bool,
        duration_ms: u64,
        period_ms: u64,
        report: PathBuf,
        ready: Option<PathBuf>,
        #[serde(default)]
        physical_device: Option<Box<PhysicalProviderConfig>>,
        #[serde(default)]
        provider_timing: bool,
        #[serde(default)]
        media_timing: bool,
        #[serde(default)]
        fx_aux: Option<Box<FxAuxConfig>>,
        #[serde(default)]
        analysis_mapping: Option<crate::analysis_stream::Mapping>,
    },
    Brain {
        bind: SocketAddr,
        peer: SocketAddr,
        server_name: String,
        certificate: PathBuf,
        key: PathBuf,
        ca: PathBuf,
        peers: Vec<Peer>,
        show_id: String,
        fx_library: PathBuf,
        #[serde(default)]
        fx_control_sha256: Option<String>,
        #[serde(default)]
        fx_aux_sha256: Option<String>,
        #[serde(default = "default_true")]
        verify_synthetic_source: bool,
        #[serde(default)]
        media_timing: bool,
        duration_ms: u64,
        return_delay_frames: u32,
        report: PathBuf,
    },
}
/// Production source lifecycle; service never supplies capture quanta.
struct ProviderSource {
    status: super::SourceClockStatus,
}
impl ProviderSource {
    fn start(
        host: &mut HostAuthority,
        now_ms: u64,
        period_ms: u64,
        duration_ms: u64,
        duplex: Option<crate::host::duplex::Duplex>,
    ) -> Result<Self> {
        let status = if let Some(device) = duplex {
            host.start_device_source_clock(now_ms, duration_ms, device)?
        } else {
            host.start_source_clock(
                now_ms,
                Duration::from_millis(period_ms),
                (duration_ms / period_ms).max(1),
            )?
        };
        Ok(Self { status })
    }
    fn service(&self, host: &mut HostAuthority, now_ms: u64) -> Result<()> {
        host.service_source_clock(now_ms)
    }
    fn stopped(&self) -> bool {
        self.status.stopped()
    }
    fn finish(self, host: &mut HostAuthority) -> Result<super::SourceClockReport> {
        host.finish_source_clock(&self.status)
    }
}
#[cfg(test)]
#[path = "runner/source_tests.rs"]
mod source_tests;

fn save(path: &Path, value: &Value) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
pub async fn run_config(config: RunConfig) -> Result<Value> {
    run_config_authorized(config, false).await
}
pub async fn run_config_authorized(config: RunConfig, physical_authorized: bool) -> Result<Value> {
    match config {
        RunConfig::Provider {
            bind,
            certificate,
            key,
            ca,
            peers,
            private_directory,
            manifest,
            pa_configuration,
            record_take,
            show_id,
            source_epoch,
            inputs,
            monitors,
            pa_outputs,
            reference_16_18,
            duration_ms,
            period_ms,
            report,
            ready,
            physical_device,
            provider_timing,
            media_timing,
            fx_aux,
            analysis_mapping,
        } => {
            validate_provider_run(
                physical_device.as_deref(),
                physical_authorized,
                duration_ms,
                period_ms,
                inputs,
                monitors,
                pa_outputs,
            )?;
            if physical_device.is_some() && analysis_mapping.is_some() {
                return Err("configured analysis requires software source until physical acquisition age is propagated".into());
            }
            if (provider_timing || media_timing)
                && (physical_device.is_some() || duration_ms > 60_000)
            {
                return Err("provider timing requires finite <=60s software run".into());
            }
            let trace = if media_timing {
                Some(super::diagnostics::Trace::with_media())
            } else {
                provider_timing.then(super::diagnostics::Trace::new)
            };
            let physical = physical_device.is_some();
            let physical_epoch = physical_device
                .as_ref()
                .map(|p| crate::device_epoch::Epoch::open_at_least(&p.epoch_file, source_epoch))
                .transpose()?;
            let source_epoch = physical_epoch.as_ref().map_or(source_epoch, |e| e.value);
            let credentials = Credentials::load_der(&certificate, &key, &ca)?;
            let policy = PolicyStore::new(peers)?;
            let server = RemoteServer::bind(bind, &credentials, policy)?;
            let address = server.local_addr()?;
            let topology = if let Some(p) = &physical_device {
                p.topology.clone()
            } else if reference_16_18 {
                if inputs != 16 {
                    return Err("reference analog profile requires16 inputs".into());
                }
                EngineTopology::reference_16_18(pa_outputs, monitors)?
            } else {
                EngineTopology::software(inputs, monitors, pa_outputs)?
            };
            let mut provider = LocalAudio::bind_configured(
                &private_directory,
                "remote-provider.sock",
                &show_id,
                Counter(source_epoch),
                topology,
            )?;
            provider.enable_modules(&manifest)?;
            if let Some(fx) = fx_aux {
                provider.enable_fx_aux(&fx.library, &fx.sha256)?;
            }
            if let Some(mapping) = analysis_mapping {
                provider.enable_configured_analysis(&private_directory, &mapping)?;
            }
            if let Some(path) = pa_configuration {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                if bytes.len() > 48 * 1024 {
                    return Err("PA configuration file capacity".into());
                }
                provider.configure_pa(&bytes)?;
            }
            if let Some(take) = &record_take {
                start_recording(&mut provider, &show_id, take, monotonic_ms())?;
            }
            let duplex = if let Some(p) = &physical_device {
                crate::host::duplex::validate_provider_map(provider.topology(), &p.topology)
                    .map_err(|e| e.to_string())?;
                let device = crate::host::duplex::Duplex::open(
                    &p.device,
                    p.topology.clone(),
                    &p.acceptance,
                    p.period_frames,
                    p.buffer_frames,
                )
                .map_err(|e| e.to_string())?;
                Some(device)
            } else {
                None
            };
            if physical {
                provider.engine_mut().mark_verified_mapping()?;
            }
            let mut host = HostAuthority::new(provider, 1)?;
            let capture_channels = host.provider().topology().capture_channels;
            let playback_channels = host.provider().topology().playback_channels;
            let (accepted_tx, mut accepted_rx) = tokio::sync::mpsc::channel(4);
            let acceptor = tokio::spawn(async move {
                loop {
                    match server.accept().await {
                        Ok(session) => {
                            if accepted_tx.send(session).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => tokio::task::yield_now().await,
                    }
                }
            });
            if let Some(path) = ready {
                save(
                    &path,
                    &json!({"state":"ready","bind":address.to_string(),"source_epoch":source_epoch,"inputs":inputs,"capture_channels":capture_channels,"playback_channels":playback_channels,"outputs_initially_muted":true}),
                )?;
            }
            let media_queue_drops = super::proxy::MediaQueueDropTotals::default();
            let mut mailboxes = Vec::new();
            let mut networks = Vec::new();
            let mut timer = tokio::time::interval(Duration::from_millis(1));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let deadline = tokio::time::sleep(Duration::from_millis(duration_ms));
            tokio::pin!(deadline);
            let started = Instant::now();
            let source =
                ProviderSource::start(&mut host, monotonic_ms(), period_ms, duration_ms, duplex)?;
            let resources_ready = crate::brain_runtime::process_resources().ok();
            let mut sessions = 0u64;
            let mut control_faults = 0u64;
            let mut fault = None;
            loop {
                tokio::select! {
                    _=&mut deadline=>break,
                    accepted=accepted_rx.recv()=>{
                        let Some(session)=accepted else{fault=Some("accept worker stopped".to_string());break;};
                        let (mut proxy,mut mailbox)=super::proxy::authority_channel_counted(session.context(),host.identity(),media_queue_drops.clone())?;
                        proxy.set_diagnostics(&mut mailbox,trace.clone());
                        mailboxes.push(mailbox);sessions+=1;
                        networks.retain(|task: &tokio::task::JoinHandle<_>| !task.is_finished());
                        networks.push(tokio::spawn(async move{session.run(&mut proxy).await}));
                    }
                    _=timer.tick()=>{
                        if source.stopped() { break; }
                        let now = monotonic_ms();
                        let resources_begin = media_timing.then(thread_resources).flatten();
                        let diagnostic_begin = trace.as_ref().map(|t| t.now()).unwrap_or(0);
                        if let Err(error) = source.service(&mut host, now) { fault=Some(error); break; }
                        for mailbox in &mut mailboxes {
                            if mailbox.service(&mut host, now).is_err() { control_faults += 1; }
                        }
                        mailboxes.retain(|m| !m.retired());
                        if let (Some(before), Some(after), Some(trace)) = (resources_begin, media_timing.then(thread_resources).flatten(), &trace) {
                            let delta: [u64;5] = std::array::from_fn(|i| after[i].saturating_sub(before[i]));
                            trace.record(super::diagnostics::Token { session: delta[1], ordinal: host.provider().frame(), request: delta[0], kind: delta[3] }, super::diagnostics::Stage::SourceDispatchResources, delta[4], delta[2]);
                        }
                        if let Some(trace) = &trace {
                            trace.record(super::diagnostics::Token { session: 0, ordinal: host.provider().frame(), request: 0, kind: trace.now().saturating_sub(diagnostic_begin) }, super::diagnostics::Stage::SourceTick, 0, trace.now().saturating_sub(diagnostic_begin));
                        }
                    }
                }
            }
            let resources_before_stop = crate::brain_runtime::process_resources().ok();
            let prepared_media_storage = host.prepared_media_storage()?;
            let source_report = source.finish(&mut host)?;
            if let Some(error) = source_report.fault {
                fault.get_or_insert(error.to_string());
            }
            let blocks = source_report.blocks;
            acceptor.abort();
            let _ = acceptor.await;
            for network in &networks {
                network.abort();
            }
            for network in networks {
                let _ = network.await;
            }
            if let Some(trace) = &trace {
                let _ = tokio::time::timeout(Duration::from_millis(100), async {
                    while trace.active_tasks() != 0 {
                        tokio::task::yield_now().await;
                    }
                })
                .await;
            }
            for mailbox in &mut mailboxes {
                let _ = mailbox.service(&mut host, monotonic_ms());
            }
            let modules = if let Some(take) = &record_take {
                if let Err(error) = host.provider_mut().quiesce_source("bounded_source_end") {
                    fault.get_or_insert_with(|| {
                        format!("recording shutdown quiesce failed: {error}")
                    });
                }
                observe_recording_shutdown(
                    take,
                    Duration::from_secs(5),
                    || host.provider_mut().module_status(monotonic_ms()),
                    Instant::now,
                    tokio::time::sleep,
                    &mut fault,
                )
                .await
            } else {
                host.provider_mut().module_status(monotonic_ms())?
            };
            host.finish_source_ingress();
            host.finish_source_egress();
            let snapshot = host.provider_mut().snapshot()?;
            let structure = host.provider_mut().structural_snapshot()?;
            let mut evidence = json!({"mode":"provider","software_only":!physical,"physical_qualified":false,"real_time_qualified":false,"inputs":inputs,"capture_channels":capture_channels,"playback_channels":playback_channels,"source_epoch":source_epoch,"processed_frames":blocks*48,"blocks":blocks,"elapsed_ms":started.elapsed().as_millis(),"synthetic_period_ms":(!physical).then_some(period_ms),"source_execution":"dedicated_worker_autonomous","resource_observation":"controller_service_only","clock_driver":if physical {"physical_capture"}else{"synthetic_elapsed_clock"},"sessions":sessions,"control_faults":control_faults,"media_queue_drops":media_queue_drops.snapshot(),"source_report":source_report,"sample_window_schema":"source_clock_v2","revision_windows":null,"brain_windows":null,"media":host.media_stats(),"source_egress":host.source_egress_report(),"accepted_wet_arrival_sha256":host.accepted_wet_sha256(),"fault":fault,"peak_per_playback_slot":source_report.peaks,"energy_per_playback_slot":source_report.energy,"final_snapshot":snapshot,"final_structure":structure,"modules":modules,"record_take":record_take,"recording_root":private_directory.join("takes")});
            evidence["prepared_media_storage"] = prepared_media_storage;
            evidence["process_resources_ready"] = json!(resources_ready);
            evidence["process_resources_before_stop"] = json!(resources_before_stop);
            if let Some(trace) = &trace {
                evidence["provider_timing"] =
                    serde_json::to_value(trace.report()).map_err(|e| e.to_string())?;
            }
            save(&report, &evidence)?;
            Ok(evidence)
        }
        RunConfig::Brain {
            bind,
            peer,
            server_name,
            certificate,
            key,
            ca,
            peers,
            show_id,
            fx_library,
            fx_control_sha256,
            fx_aux_sha256,
            verify_synthetic_source,
            media_timing,
            duration_ms,
            return_delay_frames,
            report,
        } => {
            validate_fx_run(duration_ms, return_delay_frames, verify_synthetic_source)?;
            if media_timing && (!verify_synthetic_source || duration_ms > 60_000) {
                return Err("media timing requires finite <=60s synthetic run".into());
            }
            let trace = media_timing.then(super::diagnostics::Trace::with_media);
            let credentials = Credentials::load_der(&certificate, &key, &ca)?;
            let endpoint = client_endpoint(bind, &credentials)?;
            let policy = PolicyStore::new(peers)?;
            let mut client =
                RemoteClient::connect(endpoint, peer, &server_name, policy.clone()).await?;
            let request = AudioRequest {
                contract: "C-AUDIO".into(),
                version: 2,
                show_id,
                module: "audio".into(),
                epoch: client.identity().source_epoch,
                writer: None,
                lease: None,
                request_id: None,
                expected_revision: None,
                command: Command::Snapshot {},
            };
            client
                .send_command(serde_json::to_value(request).map_err(|e| e.to_string())?)
                .await?;
            let Response::Reply { payload, .. } = client.receive().await? else {
                return Err("Brain requires actual engine snapshot".into());
            };
            let ids: Vec<String> =
                serde_json::from_value(payload["snapshot"]["authority"]["inputs"].clone())
                    .map_err(|e| e.to_string())?;
            let max = client.max_datagram();
            let mut streams = Vec::new();
            let first = 1;
            if fx_aux_sha256.is_some() && fx_control_sha256.is_some() {
                return Err("select four auxes or legacy stereo FX control".into());
            }
            let fx_ids = if fx_aux_sha256.is_some() {
                super::fx_aux_owner::channel_ids()
            } else {
                vec!["foh-left".into(), "foh-right".into()]
            };
            streams.extend(grouped_streams(
                MediaRole::FxSend,
                &fx_ids,
                48,
                0,
                first,
                max,
            )?);
            streams.extend(grouped_streams(
                MediaRole::WetReturn,
                &fx_ids,
                48,
                return_delay_frames,
                streams.len() as u32 + 1,
                max,
            )?);
            let identity = client.identity().clone();
            let descriptor = MediaDescriptor {
                session: Counter(client.session()),
                source_epoch: identity.source_epoch,
                capability_generation: identity.capability_generation,
                map_generation: identity.map_generation,
                sample_rate: 48000,
                streams,
            };
            client.negotiate(descriptor.clone()).await?;
            if !matches!(client.receive().await?, Response::MediaAccepted { .. }) {
                return Err("Brain media negotiation refused".into());
            }
            let aux_controls = fx_aux_sha256.is_some();
            let mut worker = if let Some(hash) = &fx_aux_sha256 {
                BrainWorker::prepare_aux(client.media_channel()?, &fx_library, hash, ids.len())?
            } else {
                BrainWorker::prepare(client.media_channel()?, &fx_library)?
            };
            worker.set_diagnostics(trace.clone());
            let controls = if aux_controls {
                true
            } else if let Some(hash) = &fx_control_sha256 {
                worker.enable_fx_control(&fx_library, hash)?
            } else {
                false
            };
            let execution_deadline = Instant::now() + Duration::from_millis(duration_ms);
            if aux_controls {
                worker.start_aux_execution(execution_deadline)?;
            } else {
                worker.start_fx_execution(execution_deadline)?;
            }
            let mut analysis = analysis::Worker::start(
                bind,
                peer,
                server_name,
                credentials,
                policy,
                identity,
                ids.clone(),
                verify_synthetic_source,
                execution_deadline,
            )?;
            let resources_ready = crate::brain_runtime::process_resources().ok();
            let (mut control_rx, control_task) = if controls {
                let (rx, task) = client.take_fx_replies()?;
                (rx, Some(task))
            } else {
                let (_, rx) = tokio::sync::mpsc::channel(1);
                (rx, None)
            };
            let mut observation_tick = tokio::time::interval(Duration::from_millis(50));
            let started = Instant::now();
            // Keep the connection alive for the last return's existing budget.
            // Otherwise close can retire the provider mailbox after dequeue but
            // before source-owner admission. Reserve this inside the run limit;
            // it does not extend media deadlines or acknowledge delivery.
            let shutdown_hold_ms = u64::from(return_delay_frames)
                .div_ceil(48)
                .min(duration_ms / 2);
            let finish_at = tokio::time::Instant::from_std(execution_deadline);
            let deadline =
                tokio::time::sleep_until(finish_at - Duration::from_millis(shutdown_hold_ms));
            tokio::pin!(deadline);
            let mut fault = None;
            loop {
                tokio::select! {
                    _=&mut deadline=>break,
                    response=control_rx.recv(),if controls=>{
                        match response {
                            Some(Ok(Response::Reply{payload,..})) if payload.get("contract").and_then(serde_json::Value::as_str)==Some(super::fx_aux_owner::CONTRACT)=>{
                                if let Some(command)=payload.get("command") {
                                    let command=serde_json::from_value(command.clone()).map_err(|e|e.to_string())?;
                                    if let Some(message)=worker.aux_command(command)? {client.send_command(serde_json::json!({"contract":super::fx_aux_owner::CONTRACT,"version":1,"kind":"fx_aux_owner","writer":client.writer(),"message":message})).await?;}
                                }
                            },
                            Some(Ok(Response::Reply{payload,..})) if payload.get("contract").and_then(serde_json::Value::as_str)==Some(crate::fx_wire::CONTRACT)=>{
                                if let Some(command)=payload.get("command") {
                                    let command=serde_json::from_value(command.clone()).map_err(|e|e.to_string())?;
                                    match worker.fx_command(command) {
                                        Ok(Some(message))=>client.send_command(serde_json::json!({"contract":crate::fx_wire::CONTRACT,"version":1,"kind":"fx_owner","writer":client.writer(),"message":message})).await?,
                                        Ok(None)=>(),Err(error)=>{fault=Some(error);break;}
                                    }
                                }
                            },
                            Some(Ok(Response::Reply{..}))=>(),
                            Some(Ok(Response::Refused{reason,..}))|Some(Err(reason))=>{fault=Some(reason);break;},
                            _=>{fault=Some("FX control channel closed".into());break;}
                        }
                    },
                    _=observation_tick.tick(),if controls=>{
                        if aux_controls {
                            if let Some(message)=worker.poll_aux_observation()? {client.send_command(serde_json::json!({"contract":super::fx_aux_owner::CONTRACT,"version":1,"kind":"fx_aux_owner","writer":client.writer(),"message":message})).await?;}
                        } else if let Some(message)=worker.poll_fx_observation()? {
                            client.send_command(serde_json::json!({"contract":crate::fx_wire::CONTRACT,"version":1,"kind":"fx_owner","writer":client.writer(),"message":message})).await?;
                        }
                    },
                    result=worker.step()=>match result {
                        Ok(Some(_))=>{fault=Some("analysis entered FX connection".into());break;},
                        Ok(None)=>(),Err(error)=>{fault=Some(error);break;}
                    }
                }
            }
            if fault.is_none() {
                tokio::time::sleep_until(finish_at).await;
            }
            if let Some(task) = control_task {
                task.abort();
            }
            let resources_before_stop = crate::brain_runtime::process_resources().ok();
            worker.finish_execution();
            client.close();
            let analysis_report = analysis.finish()?;
            let first_source = analysis_report.first_source;
            let last_source = analysis_report.last_source;
            let mismatches = analysis_report.mismatches;
            let channel_samples = &analysis_report.channel_samples;
            if let Some(error) = &analysis_report.fault {
                fault = Some(error.clone());
            }
            if mismatches != 0 {
                fault = Some("raw analysis sample mismatch".into());
            }
            if channel_samples.values().any(|count| *count == 0) {
                fault = Some("not every negotiated analysis channel was observed".into());
            }
            if worker.stats.processed_blocks == 0 {
                fault = Some("no actual owner FX blocks observed".into());
            }
            if verify_synthetic_source && worker.nonzero_return_samples() == 0 {
                fault = Some("functional acceptance observed no nonzero owner wet result; source may remain muted".into());
            }
            let execution_report = worker.execution_report();
            if let Some(report) = &execution_report
                && let Some(code) = report["terminal_fault"].as_u64().filter(|code| *code != 0)
            {
                fault = Some(format!(
                    "legacy FX execution fault {code}, native code {}",
                    report["terminal_native_code"]
                ));
            }
            let mut evidence = json!({"mode":"brain","software_only":verify_synthetic_source,"physical_io_owned":false,"physical_qualified":false,"descriptor":descriptor,"elapsed_ms":started.elapsed().as_millis(),"first_analysis_source_frame":first_source,"last_analysis_source_frame":last_source,"analysis_packets":analysis_report.packets,"synthetic_reference_checked":verify_synthetic_source,"analysis_sample_mismatches":if verify_synthetic_source {Some(mismatches)}else{None},"analysis_samples_per_channel":channel_samples,"analysis_arrival_sha256":analysis_report.sha256,"returned_wet_sha256":worker.returned_sha256(),"nonzero_returned_wet_samples":worker.nonzero_return_samples(),"fx_send_packets":worker.stats.send_packets,"actual_owner_fx_blocks":worker.stats.processed_blocks,"wet_packets":worker.stats.returned_packets,"incomplete_blocks":worker.stats.incomplete_blocks,"stale_groups":worker.stats.stale_groups,"fx_owner_resets":worker.owner_resets(),"fx_intentional_delay_frames":if controls {None}else{Some(worker.intentional_delay_frames())},"fx_controls_available":controls,"four_aux_controls":aux_controls,"fx_aux_owner_observation":if aux_controls {worker.poll_aux_observation().ok()}else{None},"fx_owner_observation":worker.fx_observation().ok(),"fault":fault});
            evidence["analysis_execution"] =
                serde_json::to_value(&analysis_report).map_err(|e| e.to_string())?;
            if let Some(trace) = &trace {
                evidence["timing"] =
                    serde_json::to_value(trace.report()).map_err(|e| e.to_string())?;
            }
            evidence["shutdown_hold_ms"] = json!(shutdown_hold_ms);
            evidence["fx_execution"] = json!(execution_report);
            evidence["process_resources_ready"] = json!(resources_ready);
            evidence["process_resources_before_stop"] = json!(resources_before_stop);
            evidence["process_resources_stopped"] =
                json!(crate::brain_runtime::process_resources().ok());
            save(&report, &evidence)?;
            Ok(evidence)
        }
    }
}

/// Observe only: the owner worker retains drain, sync and publication duties.
/// The injected monotonic clock/sleep let fast tests exercise the production loop.
/// This deadline does not bound report I/O, owner Drop or process lifetime.
async fn observe_recording_shutdown<F: std::future::Future<Output = ()>>(
    take: &str,
    budget: Duration,
    mut poll: impl FnMut() -> Result<Value>,
    mut now: impl FnMut() -> Instant,
    mut sleep: impl FnMut(Duration) -> F,
    fault: &mut Option<String>,
) -> Value {
    let deadline = now() + budget;
    let mut status = Value::Null;
    loop {
        if now() >= deadline {
            fault
                .get_or_insert_with(|| format!("recording shutdown observation timed out: {take}"));
            return status;
        }
        match poll() {
            Ok(observed) => status = observed,
            Err(error) => {
                fault.get_or_insert_with(|| {
                    format!("recording shutdown observation failed: {error}")
                });
                return status;
            }
        }
        // Never accept a terminal observation made after scheduling/poll overshoot.
        let observed_at = now();
        if observed_at >= deadline {
            fault
                .get_or_insert_with(|| format!("recording shutdown observation timed out: {take}"));
            return status;
        }
        // Deserialize the actual owner schema: absent/malformed fields, unknown
        // states/outcomes and a different take cannot prove terminal completion.
        if let Ok(recording) = serde_json::from_value::<crate::module_graph::RecordingStatus>(
            status["recording"].clone(),
        ) && recording.take_id == take
            && recording.state == "finalized"
            && matches!(
                recording.outcome.as_str(),
                "complete" | "incomplete" | "error"
            )
        {
            let expected = !recording.writer_fault
                && matches!(
                    (recording.outcome.as_str(), recording.host_fault),
                    ("complete", 0) | ("incomplete", 6)
                );
            if !expected {
                fault.get_or_insert_with(|| {
                    format!(
                        "recording shutdown failed: {take}, outcome={}, writer_fault={}, host_fault={}",
                        recording.outcome, recording.writer_fault, recording.host_fault
                    )
                });
            }
            return status;
        }
        sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(now()))).await;
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::cell::Cell;

    fn recording(state: &str, outcome: &str, host_fault: u32) -> Value {
        // Frozen owner-schema fixture, including actual counter encodings.
        let mut status: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/gp05/v1/status-finalized.json"
        ))
        .unwrap();
        status["recording"]["take_id"] = json!("take");
        status["recording"]["state"] = json!(state);
        status["recording"]["outcome"] = json!(outcome);
        status["recording"]["host_fault"] = json!(host_fault);
        status
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_deadline_retains_evidence_and_fails_report() {
        let mut pending = recording("finalizing", "pending", 6);
        pending["recording"]["durable_frames"] = Value::Null;
        let start = Instant::now();
        let clock = Cell::new(start);
        let mut fault = None;
        let observed = observe_recording_shutdown(
            "take",
            Duration::from_millis(17),
            || Ok(pending.clone()),
            || clock.get(),
            |duration| {
                clock.set(clock.get() + duration);
                std::future::ready(())
            },
            &mut fault,
        )
        .await;
        assert_eq!(observed, pending);
        assert!(fault.unwrap().contains("timed out"));
        assert_eq!(clock.get() - start, Duration::from_millis(17));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn delayed_terminal_transfer_beyond_old_allowance_is_observed() {
        let pending = recording("finalizing", "pending", 6);
        let terminal = recording("finalized", "incomplete", 6);
        let start = Instant::now();
        let clock = Cell::new(start);
        let mut fault = None;
        let observed = observe_recording_shutdown(
            "take",
            Duration::from_secs(5),
            || {
                Ok(if clock.get() - start < Duration::from_millis(1250) {
                    pending.clone()
                } else {
                    terminal.clone()
                })
            },
            || clock.get(),
            |duration| {
                clock.set(clock.get() + duration);
                std::future::ready(())
            },
            &mut fault,
        )
        .await;
        assert_eq!(observed, terminal);
        assert_eq!(clock.get() - start, Duration::from_millis(1250));
        assert_eq!(fault, None, "expected source-end incomplete stays allowed");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_positive_requested_take_terminal_evidence_can_succeed() {
        let terminal = recording("finalized", "complete", 0);
        let mut cases = vec![
            recording("preparing", "pending", 0),
            recording("recording", "pending", 0),
            recording("finalized", "pending", 0),
            recording("unknown", "complete", 0),
            recording("finalized", "unknown", 0),
            json!({}),
            json!({"recording":null}),
        ];
        let mut wrong_take = terminal.clone();
        wrong_take["recording"]["take_id"] = json!("other-take");
        cases.push(wrong_take);
        let mut malformed = terminal.clone();
        malformed["recording"]["writer_fault"] = json!("false");
        cases.push(malformed);
        let mut missing = terminal.clone();
        missing["recording"]
            .as_object_mut()
            .unwrap()
            .remove("outcome");
        cases.push(missing);
        for pending in cases {
            let clock = Cell::new(Instant::now());
            let mut fault = None;
            let observed = observe_recording_shutdown(
                "take",
                Duration::from_millis(7),
                || Ok(pending.clone()),
                || clock.get(),
                |duration| {
                    clock.set(clock.get() + duration);
                    std::future::ready(())
                },
                &mut fault,
            )
            .await;
            assert_eq!(observed, pending);
            assert!(fault.unwrap().contains("timed out"));
        }
        let mut fault = None;
        let observed = observe_recording_shutdown(
            "take",
            Duration::from_secs(5),
            || Ok(terminal.clone()),
            Instant::now,
            |_| async { panic!("terminal status must not sleep") },
            &mut fault,
        )
        .await;
        assert_eq!(observed, terminal);
        assert_eq!(fault, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn terminal_error_writer_failure_and_unexpected_host_fault_fail() {
        let mut writer_failure = recording("finalized", "incomplete", 6);
        writer_failure["recording"]["writer_fault"] = json!(true);
        for terminal in [
            recording("finalized", "error", 0),
            writer_failure,
            recording("finalized", "incomplete", 10),
        ] {
            for prior in [None, Some("earlier runtime fault".to_string())] {
                let mut fault = prior.clone();
                let observed = observe_recording_shutdown(
                    "take",
                    Duration::from_secs(5),
                    || Ok(terminal.clone()),
                    Instant::now,
                    |_| async { panic!("terminal status must not sleep") },
                    &mut fault,
                )
                .await;
                assert_eq!(observed, terminal);
                if let Some(prior) = prior {
                    assert_eq!(fault.as_deref(), Some(prior.as_str()));
                } else {
                    assert!(fault.unwrap().contains("shutdown failed"));
                }
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn poll_error_retains_last_status_and_prior_fault() {
        let pending = recording("preparing", "pending", 0);
        for prior in [None, Some("earlier runtime fault".to_string())] {
            let clock = Cell::new(Instant::now());
            let mut calls = 0;
            let mut fault = prior.clone();
            let observed = observe_recording_shutdown(
                "take",
                Duration::from_millis(17),
                || {
                    calls += 1;
                    if calls == 1 {
                        Ok(pending.clone())
                    } else {
                        Err("observer unavailable".into())
                    }
                },
                || clock.get(),
                |duration| {
                    clock.set(clock.get() + duration);
                    std::future::ready(())
                },
                &mut fault,
            )
            .await;
            assert_eq!(observed, pending);
            assert_eq!(calls, 2);
            if let Some(prior) = prior {
                assert_eq!(fault.as_deref(), Some(prior.as_str()));
            } else {
                assert!(fault.unwrap().contains("observer unavailable"));
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scheduler_overshoot_does_not_extend_deadline_or_erase_prior_fault() {
        let pending = recording("finalizing", "pending", 6);
        let clock = Cell::new(Instant::now());
        let mut calls = 0;
        let mut fault = Some("earlier runtime fault".to_string());
        let observed = observe_recording_shutdown(
            "take",
            Duration::from_millis(17),
            || {
                calls += 1;
                Ok(pending.clone())
            },
            || clock.get(),
            |_| {
                clock.set(clock.get() + Duration::from_secs(1));
                std::future::ready(())
            },
            &mut fault,
        )
        .await;
        assert_eq!(observed, pending);
        assert_eq!(calls, 1, "no extra poll after the absolute deadline");
        assert_eq!(fault.as_deref(), Some("earlier runtime fault"));
    }
}

/// Explicit configured-run recorder request through the same live authority and
/// owner lifecycle. Permission ends immediately; the take continues independently.
pub(crate) fn start_recording(
    provider: &mut LocalAudio,
    show_id: &str,
    take: &str,
    now: u64,
) -> Result<()> {
    let epoch = Counter(provider.source_epoch());
    let version = provider.engine_mut().wire_version();
    let revision = provider.engine_mut().revision();
    let mut grant = AudioRequest {
        contract: "C-AUDIO".into(),
        version,
        show_id: show_id.into(),
        module: "audio".into(),
        epoch,
        writer: Some("bounded-recorder".into()),
        lease: None,
        request_id: Some(Counter(1)),
        expected_revision: Some(revision),
        command: Command::Grant {
            scope: crate::control_model::Scope::Foh,
        },
    };
    let reply = provider.engine_mut().handle(&grant, now)?;
    let lease = reply
        .outcome
        .and_then(|r| r.body.granted_lease)
        .ok_or("recorder grant refused")?;
    let request = crate::module_wire::ModuleRequest {
        contract: "GP05-modules".into(),
        version,
        show_id: show_id.into(),
        module: "audio".into(),
        epoch,
        writer: grant.writer.clone(),
        lease: Some(lease),
        request_id: Some(Counter(2)),
        expected_revision: Some(revision),
        command: crate::module_wire::ModuleCommand::RecordStart {
            take_id: take.into(),
            operation_id: Counter(1),
        },
    };
    let accepted = provider.dispatch_module(request, now, None)?;
    if accepted["state"].as_str() == Some("rejected") {
        return Err(format!("recorder start refused: {}", accepted["reason"]));
    }
    grant.lease = Some(lease);
    grant.request_id = Some(Counter(3));
    grant.command = Command::Release {};
    let release = provider.engine_mut().handle(&grant, now)?;
    if release.outcome.is_none_or(|r| r.kind != "applied") {
        return Err("recorder release refused".into());
    }
    Ok(())
}

#[cfg(test)]
mod provider_admission_tests {
    use super::*;

    fn physical_fixture() -> PhysicalProviderConfig {
        let mut topology = EngineTopology::software(16, 5, 0).unwrap();
        topology.mapping_evidence = "operator-verified".into();
        let hash = crate::hash::hex(Sha256::digest(serde_json::to_vec(&topology).unwrap()));
        PhysicalProviderConfig {
            device: "hw:observed,0".into(),
            topology,
            acceptance: crate::host::duplex::DeviceAcceptance {
                device: "hw:observed,0".into(),
                topology_sha256: hash,
                native_significant_bits: 24,
                socket_mapping_record: "test observed mapping".into(),
                single_clock_setup_record: "test observed shared clock".into(),
            },
            period_frames: 48,
            buffer_frames: 192,
            epoch_file: "/nonexistent/private/epoch.json".into(),
        }
    }
    #[test]
    fn physical_provider_limits_and_exact_topology_are_pure_admission() {
        let p = physical_fixture();
        assert!(
            validate_provider_run(Some(&p), false, 1, 1, 16, 5, 0)
                .unwrap_err()
                .contains("--activate-physical")
        );
        assert!(validate_provider_run(Some(&p), true, 86_400_000, 1, 16, 5, 0).is_ok());
        assert!(validate_provider_run(Some(&p), true, 86_400_001, 1, 16, 5, 0).is_err());
        assert!(validate_provider_run(None, true, 60_001, 1, 16, 5, 0).is_err());
        assert!(validate_provider_run(None, false, 60_000, 1, 16, 5, 0).is_ok());
        assert!(validate_provider_run(Some(&p), true, 1, 1, 32, 5, 0).is_err());
        let mut invalid = p.clone();
        invalid.period_frames = 96;
        assert!(validate_provider_run(Some(&invalid), true, 1, 1, 16, 5, 0).is_err());
        invalid = p.clone();
        invalid.buffer_frames = 48_001;
        assert!(validate_provider_run(Some(&invalid), true, 1, 1, 16, 5, 0).is_err());
        invalid = p;
        invalid.topology.mapping_evidence = "synthetic".into();
        assert!(validate_provider_run(Some(&invalid), true, 1, 1, 16, 5, 0).is_err());
    }
    fn provider_config_json() -> Value {
        json!({"mode":"provider","bind":"127.0.0.1:39150","certificate":"/nonexistent/certificate","key":"/nonexistent/key","ca":"/nonexistent/ca","peers":[],"private_directory":"/nonexistent/private","manifest":"/nonexistent/manifest","pa_configuration":null,"record_take":null,"show_id":"11111111-1111-4111-8111-111111111111","source_epoch":9,"inputs":16,"monitors":5,"pa_outputs":0,"reference_16_18":false,"duration_ms":1,"period_ms":1,"report":"/nonexistent/report","ready":null})
    }
    #[tokio::test(flavor = "current_thread")]
    async fn legacy_provider_config_defaults_fake_and_physical_denies_before_io() {
        let original = provider_config_json();
        let legacy: RunConfig = serde_json::from_value(original.clone()).unwrap();
        assert!(matches!(
            legacy,
            RunConfig::Provider {
                physical_device: None,
                ..
            }
        ));
        let mut physical = original;
        let p = physical_fixture();
        physical["physical_device"] = json!({"device":p.device,"topology":p.topology,"acceptance":p.acceptance,"period_frames":48,"buffer_frames":192,"epoch_file":p.epoch_file});
        let config: RunConfig = serde_json::from_value(physical).unwrap();
        assert!(
            run_config(config)
                .await
                .unwrap_err()
                .contains("--activate-physical"),
            "must deny before nonexistent credentials, private epoch ledger, listener or PCM are touched"
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn configured_physical_analysis_refuses_before_any_io_even_when_authorized() {
        let mut value = provider_config_json();
        let p = physical_fixture();
        value["physical_device"] = json!({"device":p.device,"topology":p.topology,"acceptance":p.acceptance,"period_frames":48,"buffer_frames":192,"epoch_file":p.epoch_file});
        value["analysis_mapping"] =
            json!({"version":1,"inputs":["input-01","input-02","input-03","input-04"]});
        let config: RunConfig = serde_json::from_value(value).unwrap();
        assert!(
            run_config_authorized(config, true)
                .await
                .unwrap_err()
                .contains("physical acquisition age")
        );
    }
    #[test]
    fn source_agnostic_fx_accepts_arbitrary_ids_and_silence_without_synthetic_claim() {
        assert_eq!(
            expected_analysis_pcm(false, "observed-microphone-a", 0).unwrap(),
            None
        );
        assert_eq!(
            expected_analysis_pcm(false, "observed-microphone-a", u64::MAX).unwrap(),
            None
        );
        assert_eq!(
            expected_analysis_pcm(true, "input-01", 0).unwrap(),
            Some(128)
        );
        assert_eq!(
            expected_analysis_pcm(true, "input-01", 24).unwrap(),
            Some(-128)
        );
        assert!(expected_analysis_pcm(true, "observed-microphone-a", 0).is_err());
        assert!(validate_fx_run(60_000, 384, true).is_ok());
        assert!(validate_fx_run(60_001, 384, true).is_err());
        assert!(validate_fx_run(86_400_000, 384, false).is_ok());
        assert!(validate_fx_run(86_400_001, 384, false).is_err());
        let config:RunConfig=serde_json::from_value(json!({"mode":"brain","bind":"127.0.0.1:0","peer":"127.0.0.1:39150","server_name":"test","certificate":"c","key":"k","ca":"ca","peers":[],"show_id":"test","fx_library":"fx","duration_ms":1,"return_delay_frames":384,"report":"r"})).unwrap();
        assert!(matches!(
            config,
            RunConfig::Brain {
                verify_synthetic_source: true,
                ..
            }
        ));
    }
}
