//! Explicit bounded authenticated synthetic or separately authorized physical runs.
//! Configuration, credentials and reports remain task-private. This is not a
//! service installer, load benchmark or hardware qualification.
use super::*;
use crate::{
    control_model::{Command, Request as AudioRequest},
    local_audio::LocalAudio,
    show::Counter,
    topology::EngineTopology,
};
use serde::{Deserialize, Serialize};
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
fn expected_analysis_pcm(verify_synthetic: bool, id: &str, frame: u64) -> Result<Option<i32>> {
    if !verify_synthetic {
        return Ok(None);
    }
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
    Ok(Some(if (frame / period).is_multiple_of(2) {
        amplitude
    } else {
        -amplitude
    }))
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
        duration_ms: u64,
        return_delay_frames: u32,
        report: PathBuf,
    },
}
#[derive(Serialize)]
struct RevisionWindow {
    revision: Counter,
    first_source_frame: u64,
    last_source_frame: u64,
    blocks: u64,
    energy_per_playback_slot: Vec<f64>,
    first_block_samples: Vec<f64>,
}
// Diagnostic storage is capped independently of fake or physical run duration.
// Collection happens after the composed source worker, never inside Mixer/PA DSP.
const OUTPUT_WINDOW_FRAMES: u64 = 4_800;
const OUTPUT_WINDOW_LIMIT: usize = 1_024;
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
struct OutputState {
    source_epoch: Counter,
    revision: Counter,
    map_revision: Counter,
    clock_state: crate::clock_domain::ClockState,
    outputs_quiesced: bool,
    media_session: Option<Counter>,
    wet_return_negotiated: bool,
}
fn output_state(host: &mut HostAuthority) -> OutputState {
    let media_session = host.descriptor().map(|d| d.session);
    let wet_return_negotiated = host
        .descriptor()
        .is_some_and(|d| d.channels(MediaRole::WetReturn) > 0);
    let provider = host.provider_mut();
    let map_revision = Counter(provider.topology().map_revision);
    let engine = provider.engine_mut();
    OutputState {
        source_epoch: Counter(engine.clock_status().epoch),
        revision: engine.revision(),
        map_revision,
        clock_state: engine.clock_status().state,
        outputs_quiesced: engine.outputs_quiesced(),
        media_session,
        wet_return_negotiated,
    }
}
#[derive(Clone, Copy, Default)]
struct WetCounters {
    packets: u64,
    samples: u64,
    energy: f64,
}
impl WetCounters {
    fn read(host: &HostAuthority) -> Self {
        let stats = host.media_stats();
        Self {
            packets: stats.accepted_wet_packets,
            samples: stats.nonzero_rendered_wet_samples,
            energy: stats.rendered_wet_energy,
        }
    }
}
#[derive(Serialize)]
struct OutputWindow {
    #[serde(flatten)]
    state: OutputState,
    first_source_frame: u64,
    last_source_frame: u64,
    first_elapsed_ms: u64,
    last_elapsed_ms: u64,
    media_absent_since_source_frame: Option<u64>,
    blocks: u64,
    state_transition_blocks: u64,
    accepted_wet_packets_delta: u64,
    nonzero_rendered_wet_samples_delta: u64,
    rendered_wet_energy_delta: f64,
    energy_per_playback_slot: Vec<f64>,
    peak_per_playback_slot: Vec<f64>,
}
struct OutputBlock<'a> {
    frame: u64,
    elapsed_ms: u64,
    before: OutputState,
    after: OutputState,
    wet_before: WetCounters,
    wet_after: WetCounters,
    playback: &'a [f64],
    width: usize,
}
#[derive(Default)]
struct OutputWindows {
    windows: Vec<OutputWindow>,
    overflow_blocks: u64,
    previous_state: Option<OutputState>,
    absent_since_frame: Option<u64>,
}
impl OutputWindows {
    fn observe(&mut self, block: OutputBlock<'_>) {
        let state = block.after;
        if state.media_session.is_some() {
            self.absent_since_frame = None;
        } else if self.previous_state.is_none_or(|prior| {
            prior.media_session.is_some() || prior.source_epoch != state.source_epoch
        }) {
            self.absent_since_frame = Some(block.frame);
        }
        self.previous_state = Some(state);
        let transition = block.before != block.after;
        let new_window = self.windows.last().is_none_or(|last| {
            last.state != state
                || transition
                || last.state_transition_blocks > 0
                || block.frame != last.last_source_frame.saturating_add(1)
                || block.frame >= last.first_source_frame.saturating_add(OUTPUT_WINDOW_FRAMES)
        });
        if new_window {
            if self.windows.len() == OUTPUT_WINDOW_LIMIT {
                self.overflow_blocks += 1;
                return;
            }
            self.windows.push(OutputWindow {
                state,
                first_source_frame: block.frame,
                last_source_frame: block.frame,
                first_elapsed_ms: block.elapsed_ms,
                last_elapsed_ms: block.elapsed_ms,
                media_absent_since_source_frame: self.absent_since_frame,
                blocks: 0,
                state_transition_blocks: 0,
                accepted_wet_packets_delta: 0,
                nonzero_rendered_wet_samples_delta: 0,
                rendered_wet_energy_delta: 0.,
                energy_per_playback_slot: vec![0.; block.width],
                peak_per_playback_slot: vec![0.; block.width],
            });
        }
        let window = self.windows.last_mut().expect("window just admitted");
        window.last_source_frame = block.frame + 47;
        window.last_elapsed_ms = block.elapsed_ms;
        window.blocks += 1;
        window.state_transition_blocks += u64::from(transition);
        window.accepted_wet_packets_delta += block.wet_after.packets - block.wet_before.packets;
        window.nonzero_rendered_wet_samples_delta +=
            block.wet_after.samples - block.wet_before.samples;
        window.rendered_wet_energy_delta += block.wet_after.energy - block.wet_before.energy;
        for row in block.playback.chunks_exact(block.width) {
            for (channel, sample) in row.iter().enumerate() {
                window.energy_per_playback_slot[channel] += sample * sample;
                window.peak_per_playback_slot[channel] =
                    window.peak_per_playback_slot[channel].max(sample.abs());
            }
        }
    }
}

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
            if provider_timing && (physical_device.is_some() || duration_ms > 60_000) {
                return Err("provider timing requires finite <=60s software run".into());
            }
            let trace = provider_timing.then(super::diagnostics::Trace::new);
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
            let mut duplex = if let Some(p) = &physical_device {
                crate::host::duplex::validate_provider_map(provider.topology(), &p.topology)
                    .map_err(|e| e.to_string())?;
                let mut device = crate::host::duplex::Duplex::open(
                    &p.device,
                    p.topology.clone(),
                    &p.acceptance,
                    p.period_frames,
                    p.buffer_frames,
                )
                .map_err(|e| e.to_string())?;
                device.start().map_err(|e| e.to_string())?;
                Some(device)
            } else {
                None
            };
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
            let mut mailboxes = Vec::new();
            let mut networks = Vec::new();
            let mut timer = tokio::time::interval(if physical {
                Duration::from_micros(250)
            } else {
                Duration::from_millis(period_ms)
            });
            timer.set_missed_tick_behavior(if physical {
                tokio::time::MissedTickBehavior::Skip
            } else {
                tokio::time::MissedTickBehavior::Burst
            });
            let deadline = tokio::time::sleep(Duration::from_millis(duration_ms));
            tokio::pin!(deadline);
            let started = Instant::now();
            let mut capture = vec![0.; 48 * capture_channels];
            let mut playback = vec![0.; 48 * playback_channels];
            let mut peaks = vec![0f64; playback_channels];
            let mut sums = vec![0f64; playback_channels];
            let mut revision_windows: Vec<RevisionWindow> = Vec::new();
            let mut revision_windows_overflow = 0u64;
            let mut output_windows = OutputWindows::default();
            let mut brain_windows: Vec<Value> = Vec::new();
            let mut brain_windows_overflow = 0u64;
            let mut brain_window_identity = None;
            let mut blocks = 0u64;
            let mut sessions = 0u64;
            let mut control_faults = 0u64;
            let mut fault = None;
            loop {
                tokio::select! {
                    _=&mut deadline=>break,
                    accepted=accepted_rx.recv()=>{
                        let Some(session)=accepted else{fault=Some("accept worker stopped".to_string());break;};
                        let (mut proxy,mut mailbox)=authority_channel(session.context(),host.identity())?;
                        proxy.set_diagnostics(&mut mailbox,trace.clone());
                        mailboxes.push(mailbox);sessions+=1;
                        networks.retain(|task: &tokio::task::JoinHandle<_>| !task.is_finished());
                        networks.push(tokio::spawn(async move{session.run(&mut proxy).await}));
                    }
                    _=timer.tick()=>{
                        // A fake device keeps its own elapsed-time cadence. Catch up
                        // one bounded block per iteration; never hide skipped frames
                        // as oscillator drift or accumulate more than100ms of debt.
                        let due=started.elapsed().as_millis() as u64 / period_ms;
                        if !physical && due.saturating_sub(blocks)>100/period_ms.max(1) {fault=Some("synthetic source scheduling debt exceeded100ms".into());break;}
                        let diagnostic_begin=trace.as_ref().map(|t|t.now()).unwrap_or(0);
                        let mut diagnostic_dispatch=0;let mut diagnostic_render=0;
                        let diagnostic_debt=if trace.is_some(){started.elapsed().as_micros().saturating_sub(blocks as u128*period_ms as u128*1000) as u64}else{0};
                        let now=monotonic_ms();
                        let before_state=output_state(&mut host);
                        let wet_before=WetCounters::read(&host);
                        let frame=host.provider().frame();
                        if let Some(device) = &mut duplex {
                            let result=device.pump_authority_with(&mut host,now,|owner,fresh_now| {
                                for mailbox in &mut mailboxes {if mailbox.service(owner,fresh_now).is_err(){control_faults+=1;}}
                                Ok(())
                            });
                            if let Err(error)=result {fault=Some(error.to_string());break;}
                            capture.copy_from_slice(device.last_capture());
                            playback.copy_from_slice(device.last_playback());
                        } else {
                            let dispatch_begin=trace.as_ref().map(|t|t.now()).unwrap_or(0);
                            for mailbox in &mut mailboxes {if mailbox.service(&mut host,now).is_err(){control_faults+=1;}}
                            diagnostic_dispatch=trace.as_ref().map(|t|t.now().saturating_sub(dispatch_begin)).unwrap_or(0);
                            capture.fill(0.);
                            for f in 0..48 {for (index,input) in host.provider().topology().inputs.iter().enumerate(){
                                let polarity=if ((frame+f as u64)/(24+index as u64)).is_multiple_of(2) {1.}else{-1.};
                                capture[f*capture_channels+input.capture_slot]=polarity*((index+1)*128) as f64/8_388_608.;
                            }}
                            let render_begin=trace.as_ref().map(|t|t.now()).unwrap_or(0);
                            if let Err(error)=host.process_source(now,source_epoch,frame,&capture,&mut playback){fault=Some(error);break;}
                            diagnostic_render=trace.as_ref().map(|t|t.now().saturating_sub(render_begin)).unwrap_or(0);
                        }
                        let report_begin=trace.as_ref().map(|t|t.now()).unwrap_or(0);
                        mailboxes.retain(|m|!m.retired());
                        output_windows.observe(OutputBlock{frame,elapsed_ms:started.elapsed().as_millis() as u64,before:before_state,after:output_state(&mut host),wet_before,wet_after:WetCounters::read(&host),playback:&playback,width:playback_channels});
                        for row in playback.chunks_exact(playback_channels){for(channel,sample)in row.iter().enumerate(){peaks[channel]=peaks[channel].max(sample.abs());sums[channel]+=sample*sample;}}
                        let revision=host.provider_mut().engine_mut().revision();
                        let brain=host.provider().brain_snapshot();
                        let brain_identity=(revision.0,brain.selection_generation.0,brain.held_generation,frame/4800);
                        if brain_window_identity!=Some(brain_identity){
                            if brain_windows.len()<2048{brain_windows.push(json!({"elapsed_ms":started.elapsed().as_millis() as u64,"source_frame":frame,"snapshot":brain,"bridge":host.brain_bridge_status(),"playback":playback,"monitor":host.provider().brain_monitor_output(),"fx_send":host.provider().brain_fx_send()}));}else{brain_windows_overflow+=1;}
                            brain_window_identity=Some(brain_identity);
                        }
                        if revision_windows.last().is_none_or(|w|w.revision!=revision) {
                            if revision_windows.len()<64 {
                                revision_windows.push(RevisionWindow{revision,first_source_frame:frame,last_source_frame:frame+47,blocks:0,energy_per_playback_slot:vec![0.;playback_channels],first_block_samples:playback.clone()});
                            } else {revision_windows_overflow+=1;}
                        }
                        if let Some(window)=revision_windows.last_mut().filter(|w|w.revision==revision) {
                            window.last_source_frame=frame+47;window.blocks+=1;
                            for row in playback.chunks_exact(playback_channels){for(channel,sample)in row.iter().enumerate(){window.energy_per_playback_slot[channel]+=sample*sample;}}
                        }
                        if let Some(trace)=&trace {trace.record(super::diagnostics::Token{session:trace.now().saturating_sub(report_begin),ordinal:frame,request:diagnostic_debt,kind:diagnostic_dispatch},super::diagnostics::Stage::SourceTick,diagnostic_render,trace.now().saturating_sub(diagnostic_begin));}
                        blocks+=1;
                    }
                }
            }
            if physical {
                if let Err(error) = host
                    .provider_mut()
                    .quiesce_source("bounded_physical_source_end")
                {
                    fault.get_or_insert(error);
                }
                host.provider_mut().close_brain_audio();
                drop(duplex.take());
            }
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
            let snapshot = host.provider_mut().snapshot()?;
            let structure = host.provider_mut().structural_snapshot()?;
            let mut evidence = json!({"mode":"provider","software_only":!physical,"physical_qualified":false,"real_time_qualified":false,"inputs":inputs,"capture_channels":capture_channels,"playback_channels":playback_channels,"source_epoch":source_epoch,"processed_frames":blocks*48,"blocks":blocks,"elapsed_ms":started.elapsed().as_millis(),"synthetic_period_ms":(!physical).then_some(period_ms),"clock_driver":if physical {"physical_capture"}else{"synthetic_elapsed_clock"},"sessions":sessions,"control_faults":control_faults,"revision_windows":revision_windows,"brain_windows":brain_windows,"brain_windows_overflow":brain_windows_overflow,"revision_windows_overflow":revision_windows_overflow,"output_window_frames":OUTPUT_WINDOW_FRAMES,"output_window_limit":OUTPUT_WINDOW_LIMIT,"output_windows":output_windows.windows,"output_windows_overflow_blocks":output_windows.overflow_blocks,"media":host.media_stats(),"accepted_wet_arrival_sha256":host.accepted_wet_sha256(),"fault":fault,"peak_per_playback_slot":peaks,"energy_per_playback_slot":sums,"final_snapshot":snapshot,"final_structure":structure,"modules":modules,"record_take":record_take,"recording_root":private_directory.join("takes")});
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
            duration_ms,
            return_delay_frames,
            report,
        } => {
            validate_fx_run(duration_ms, return_delay_frames, verify_synthetic_source)?;
            let credentials = Credentials::load_der(&certificate, &key, &ca)?;
            let endpoint = client_endpoint(bind, &credentials)?;
            let policy = PolicyStore::new(peers)?;
            let mut client = RemoteClient::connect(endpoint, peer, &server_name, policy).await?;
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
            let mut streams = grouped_streams(MediaRole::Analysis, &ids, 48, 0, 1, max)?;
            let first = streams.len() as u32 + 1;
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
            let controls = if aux_controls {
                true
            } else if let Some(hash) = &fx_control_sha256 {
                worker.enable_fx_control(&fx_library, hash)?
            } else {
                false
            };
            let (mut control_rx, control_task) = if controls {
                let (rx, task) = client.take_fx_replies()?;
                (rx, Some(task))
            } else {
                let (_, rx) = tokio::sync::mpsc::channel(1);
                (rx, None)
            };
            let mut observation_tick = tokio::time::interval(Duration::from_millis(50));
            let started = Instant::now();
            let deadline = tokio::time::sleep(Duration::from_millis(duration_ms));
            tokio::pin!(deadline);
            let mut first_source = None;
            let mut last_source = None;
            let mut analysis_hash = Sha256::new();
            let mut mismatches = 0u64;
            let mut channel_samples: std::collections::BTreeMap<String, u64> =
                ids.iter().cloned().map(|id| (id, 0)).collect();
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
                        Ok(Some(bytes))=>{
                            let packet=crate::transport::Packet::parse(&bytes).map_err(|e|format!("analysis evidence {e:?}"))?;
                            first_source.get_or_insert(packet.source_frame());last_source=Some(packet.source_frame());
                            let stream=descriptor.streams.iter().find(|s|s.stream==packet.spec().stream).ok_or("analysis evidence stream")?;
                            for (channel,id) in stream.channel_ids.iter().enumerate() {
                                for f in 0..usize::from(packet.spec().frames) {
                                    let actual=packet.pcm(f*usize::from(packet.spec().channels)+channel).map_err(|e|format!("analysis PCM {e:?}"))?;
                                    if let Some(expected)=expected_analysis_pcm(verify_synthetic_source,id,packet.source_frame()+f as u64)? && actual!=expected {mismatches+=1;}
                                    *channel_samples.get_mut(id).ok_or("analysis channel evidence")?+=1;
                                }
                            }
                            analysis_hash.update(&bytes);
                        },
                        Ok(None)=>(),Err(error)=>{fault=Some(error);break;}
                    }
                }
            }
            if let Some(task) = control_task {
                task.abort();
            }
            client.close();
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
            let evidence = json!({"mode":"brain","software_only":verify_synthetic_source,"physical_io_owned":false,"physical_qualified":false,"descriptor":descriptor,"elapsed_ms":started.elapsed().as_millis(),"first_analysis_source_frame":first_source,"last_analysis_source_frame":last_source,"analysis_packets":worker.stats.analysis_packets,"synthetic_reference_checked":verify_synthetic_source,"analysis_sample_mismatches":if verify_synthetic_source {Some(mismatches)}else{None},"analysis_samples_per_channel":channel_samples,"analysis_arrival_sha256":format!("{:x}",analysis_hash.finalize()),"returned_wet_sha256":worker.returned_sha256(),"nonzero_returned_wet_samples":worker.nonzero_return_samples(),"fx_send_packets":worker.stats.send_packets,"actual_owner_fx_blocks":worker.stats.processed_blocks,"wet_packets":worker.stats.returned_packets,"incomplete_blocks":worker.stats.incomplete_blocks,"stale_groups":worker.stats.stale_groups,"fx_owner_resets":worker.owner_resets(),"fx_intentional_delay_frames":if controls {None}else{Some(worker.intentional_delay_frames())},"fx_controls_available":controls,"four_aux_controls":aux_controls,"fx_aux_owner_observation":if aux_controls {worker.poll_aux_observation().ok()}else{None},"fx_owner_observation":worker.fx_observation().ok(),"fault":fault});
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
mod output_window_tests {
    use super::*;

    fn state(session: Option<u64>) -> OutputState {
        OutputState {
            source_epoch: Counter(14),
            revision: Counter(8),
            map_revision: Counter(3),
            clock_state: crate::clock_domain::ClockState::Running,
            outputs_quiesced: false,
            media_session: session.map(Counter),
            wet_return_negotiated: session.is_some(),
        }
    }

    #[test]
    fn actual_sample_windows_separate_session_loss_and_settled_dry_output() {
        let mut recorder = OutputWindows::default();
        let samples = [2.; 96];
        let mut prior = state(Some(1));
        let mut wet = WetCounters::default();
        for block in 0..225_u64 {
            let after = state(if block < 10 {
                Some(1)
            } else if block < 220 {
                None
            } else {
                Some(2)
            });
            let before_wet = wet;
            if !(16..220).contains(&block) {
                wet.samples += 96;
                wet.energy += 0.25;
            }
            if after.media_session.is_some() {
                wet.packets += 1;
            }
            recorder.observe(OutputBlock {
                frame: block * 48,
                elapsed_ms: block,
                before: prior,
                after,
                wet_before: before_wet,
                wet_after: wet,
                playback: &samples,
                width: 2,
            });
            prior = after;
        }
        assert_eq!(recorder.overflow_blocks, 0);
        assert_eq!(recorder.windows.iter().map(|w| w.blocks).sum::<u64>(), 225);
        let dry = recorder
            .windows
            .iter()
            .find(|w| {
                w.state.media_session.is_none()
                    && w.first_source_frame > w.media_absent_since_source_frame.unwrap() + 240
                    && w.blocks == 100
                    && w.nonzero_rendered_wet_samples_delta == 0
            })
            .unwrap();
        assert_eq!(dry.rendered_wet_energy_delta, 0.);
        assert_eq!(dry.state_transition_blocks, 0);
        assert_eq!(dry.energy_per_playback_slot, vec![4. * 48. * 100.; 2]);
        assert_eq!(dry.peak_per_playback_slot, vec![2.; 2]);
        assert_eq!(dry.last_source_frame - dry.first_source_frame + 1, 4_800);
        assert!(
            recorder
                .windows
                .iter()
                .any(|w| w.state.media_session == Some(Counter(2)))
        );
        assert_eq!(
            recorder
                .windows
                .iter()
                .map(|w| w.state_transition_blocks)
                .sum::<u64>(),
            2
        );
        let value = serde_json::to_value(dry).unwrap();
        assert_eq!(value["media_session"], Value::Null);
        assert_eq!(value["revision"], "8");
        assert_eq!(value["outputs_quiesced"], false);
    }

    #[test]
    fn diagnostic_capacity_is_explicit_and_never_relabels_dropped_samples() {
        let mut recorder = OutputWindows::default();
        let samples = [0.; 48];
        for block in 0..OUTPUT_WINDOW_LIMIT as u64 + 3 {
            let mut current = state(None);
            current.revision = Counter(block);
            recorder.observe(OutputBlock {
                frame: block * 48,
                elapsed_ms: block,
                before: current,
                after: current,
                wet_before: WetCounters::default(),
                wet_after: WetCounters::default(),
                playback: &samples,
                width: 1,
            });
        }
        assert_eq!(recorder.windows.len(), OUTPUT_WINDOW_LIMIT);
        assert_eq!(recorder.overflow_blocks, 3);
        assert_eq!(
            recorder.windows.iter().map(|w| w.blocks).sum::<u64>(),
            OUTPUT_WINDOW_LIMIT as u64
        );
    }
    fn physical_fixture() -> PhysicalProviderConfig {
        let mut topology = EngineTopology::software(16, 5, 0).unwrap();
        topology.mapping_evidence = "operator-verified".into();
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&topology).unwrap())
        );
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
