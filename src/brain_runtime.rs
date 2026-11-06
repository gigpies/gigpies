//! Brain controller. The sole duplex service owner runs independently of control
//! awaits, network encoding and diagnostic serialization.
#[cfg(test)]
pub(crate) mod lifecycle_test;
mod media;
mod service;
use crate::{
    brain_audio::{
        bridge::{Bridge, BridgeConfig, BridgeEpochs},
        device::{AlsaDuplex, DeviceConfig, DeviceError, DuplexDevice, FakeDuplex},
        host::{BrainHost, BrainRender, LocalLevels},
    },
    brain_control::{Command, Request, Snapshot},
    device_epoch::Epoch,
    remote::*,
    show::Counter,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bind: SocketAddr,
    pub peer: SocketAddr,
    pub server_name: String,
    pub certificate: PathBuf,
    pub key: PathBuf,
    pub ca: PathBuf,
    pub peers: Vec<Peer>,
    pub show_id: String,
    pub device: DeviceConfig,
    #[serde(default)]
    pub restore_intent: bool,
    pub physical: bool,
    pub epoch_file: PathBuf,
    pub duration_ms: u64,
    pub fake_skew_ppm: i32,
    pub report: PathBuf,
}
enum Device {
    Fake(FakeDuplex),
    Physical(AlsaDuplex),
}
impl DuplexDevice for Device {
    fn capabilities(&self) -> &crate::brain_audio::device::DeviceCapabilities {
        match self {
            Self::Fake(d) => d.capabilities(),
            Self::Physical(d) => d.capabilities(),
        }
    }
    fn start(&mut self) -> std::result::Result<(), DeviceError> {
        match self {
            Self::Fake(d) => d.start(),
            Self::Physical(d) => d.start(),
        }
    }
    fn read_capture(&mut self, out: &mut [f64]) -> std::result::Result<usize, DeviceError> {
        match self {
            Self::Fake(d) => d.read_capture(out),
            Self::Physical(d) => d.read_capture(out),
        }
    }
    fn write_playback(&mut self, input: &[f64]) -> std::result::Result<usize, DeviceError> {
        match self {
            Self::Fake(d) => d.write_playback(input),
            Self::Physical(d) => d.write_playback(input),
        }
    }
    fn stop(&mut self) {
        match self {
            Self::Fake(d) => d.stop(),
            Self::Physical(d) => d.stop(),
        }
    }
}
fn device(config: &DeviceConfig, physical: bool) -> Result<Device> {
    if physical {
        AlsaDuplex::open_detailed(config)
            .map(Device::Physical)
            .map_err(|e| e.to_string())
    } else {
        FakeDuplex::prepare(config)
            .map(Device::Fake)
            .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Copy)]
struct Captured {
    epoch: u64,
    frame: u64,
    hold_generation: u64,
    samples: [f64; 48],
}
struct Renderer {
    captured: rtrb::Producer<Captured>,
    monitor: Bridge,
    epochs: BridgeEpochs,
    fault: Option<DeviceError>,
    dropped: u64,
    hold_generation: u64,
}
fn monitor_selection_current(selection: Option<u64>, descriptor: &BrainMediaDescriptor) -> bool {
    selection == Some(descriptor.selection_generation)
}
fn accepted_capture_hold(hold: Option<u64>, descriptor: &BrainMediaDescriptor) -> u64 {
    if hold == Some(descriptor.hold_generation) {
        descriptor.hold_generation
    } else {
        0
    }
}
impl BrainRender for Renderer {
    fn capture(&mut self, epoch: u64, first: u64, mono: &[f64]) {
        if self.hold_generation == 0 {
            return;
        }
        for (n, chunk) in mono.chunks_exact(48).enumerate() {
            let mut samples = [0.; 48];
            samples.copy_from_slice(chunk);
            if self
                .captured
                .push(Captured {
                    epoch,
                    frame: first + n as u64 * 48,
                    hold_generation: self.hold_generation,
                    samples,
                })
                .is_err()
            {
                self.dropped += 1;
            }
        }
    }
    fn playback(&mut self, epoch: u64, first: u64, stereo: &mut [f64]) {
        if epoch != self.epochs.destination {
            stereo.fill(0.);
            return;
        }
        let _ = self.monitor.render(self.epochs, first, stereo);
    }
    fn fault(&mut self, _: u64, error: DeviceError) {
        self.fault = Some(error);
        self.monitor.invalidate();
    }
}
fn query(show: &str, epoch: Counter) -> Result<Value> {
    serde_json::to_value(Request {
        contract: "GP15-brain".into(),
        version: 1,
        show_id: show.into(),
        module: "audio".into(),
        epoch,
        writer: None,
        lease: None,
        request_id: None,
        expected_revision: None,
        command: Command::BrainSnapshot {},
    })
    .map_err(|e| e.to_string())
}
fn observation(
    service: &service::Service,
    config: &DeviceConfig,
    ticket: Option<u64>,
    success: Option<bool>,
    error: Option<String>,
) -> BrainDeviceObservation {
    let status = service.latest.host.with_description(
        service.capabilities.clone(),
        service.capabilities.physical && config.physically_verified(),
    );
    let mut telemetry = brain_device_telemetry(
        &status,
        &service.latest.bridge,
        service.latest.capture_dropped,
    );
    telemetry["prepared_storage"] = json!(service.prepared_storage);
    telemetry["prepared_storage_excludes"] = json!([
        "ASRC coefficients/internal library storage",
        "ALSA/QUIC/TLS",
        "ring and shared metadata",
        "device descriptions",
        "thread stacks",
        "allocator overhead"
    ]);
    telemetry["service_fault"] = json!(service.latest.service_fault);
    telemetry["observation_age_us"] = json!(
        service
            .latest
            .observed_at
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64
    );
    telemetry["service_ticks"] = json!(service.latest.ticks);
    telemetry["applied_revision"] = json!(service.latest.applied_revision);
    telemetry["closure_generation"] =
        json!({"capture":service.latest.closure as u32,"monitor":service.latest.closure >> 32});
    telemetry["state_queue_dropped"] = json!(service.state_dropped);
    telemetry["state_refused"] = json!(service.latest.state_refused);
    telemetry["monitor_queue_dropped"] = json!(
        service
            .monitor_dropped
            .load(std::sync::atomic::Ordering::Relaxed)
    );
    telemetry["monitor_rejected"] = json!(service.latest.monitor_rejected);
    telemetry["playback_evidence_dropped"] = json!(service.latest.playback_dropped);
    telemetry["status_queue_dropped"] = json!(service.latest.status_dropped);
    BrainDeviceObservation {
        brain_epoch: status.readback.epoch,
        brain_map: status.readback.configuration_generation,
        frame: status.capture_frame,
        config: config.clone(),
        status: telemetry,
        ticket,
        success,
        error,
    }
}
fn publish_state(
    service: &mut service::Service,
    snapshot: &Snapshot,
    descriptor: Option<&BrainMediaDescriptor>,
) {
    let Some(d) = descriptor else { return };
    let hold = accepted_capture_hold(Some(snapshot.held_generation.map_or(0, |g| g.0)), d);
    let monitor = snapshot.monitor_armed
        && monitor_selection_current(Some(snapshot.selection_generation.0), d);
    service.publish(service::State {
        closure: 0,
        revision: snapshot.revision.0,
        epochs: BridgeEpochs {
            source: d.stagebox_epoch,
            destination: d.brain_epoch,
            route: d.selection_generation,
        },
        hold,
        monitor,
        levels: LocalLevels {
            microphone_gain_db: 0.,
            microphone_muted: hold == 0 || snapshot.talkback_mute,
            monitor_gain_db: f64::from(snapshot.monitor_gain_cdb) / 100.,
            monitor_muted: snapshot.monitor_mute || !monitor,
            monitor_dim: snapshot.monitor_dim,
        },
        expires: Instant::now()
            + Duration::from_millis(snapshot.deadman_ms.min(crate::brain_control::DEADMAN_MS)),
    });
}
/// A device epoch must first be acknowledged by the authority, then observed
/// closed, before a later explicit control revision may make it audible.
#[derive(Default)]
struct ArmFence {
    acknowledged: bool,
    closed_revision: Option<u64>,
}
impl ArmFence {
    fn observe(&mut self, revision: u64, active: bool) -> bool {
        if !self.acknowledged {
            return false;
        }
        if !active {
            self.closed_revision = Some(revision);
            return true;
        }
        self.closed_revision.is_some_and(|closed| revision > closed)
    }
}
#[derive(Default, Serialize)]
struct Evidence {
    captured_packets: u64,
    monitor_packets: u64,
    rejected_monitor_packets: u64,
    playback_frames: u64,
    playback_energy: f64,
    playback_peak: f64,
    first_playback: Vec<f64>,
    negotiations: u64,
    configuration_changes: u64,
    playback_windows: Vec<Value>,
    playback_windows_overflow: u64,
}

fn validate_run_bounds(
    duration_ms: u64,
    fake_skew_ppm: i32,
    physical: bool,
    physical_authorized: bool,
) -> Result<()> {
    let maximum_ms = if physical { 86_400_000 } else { 60_000 };
    if !(1..=maximum_ms).contains(&duration_ms) || !(-2000..=2000).contains(&fake_skew_ppm) {
        return Err("bounded duration/skew required".into());
    }
    if physical && !physical_authorized {
        return Err("physical duplex requires explicit --activate-physical".into());
    }
    Ok(())
}

/// Controller-side measurement of this process, distinct from prepared layouts.
pub(crate) fn process_resources() -> Result<Value> {
    let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    let field = |name: &str| -> Option<u64> {
        status.lines().find_map(|line| {
            line.strip_prefix(name)
                .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        })
    };
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut threads = Vec::new();
    let mut unavailable_threads = 0;
    for entry in std::fs::read_dir("/proc/self/task").map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let result = (|| -> Option<Value> {
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            let end = stat.rfind(')')?;
            let start = stat.find('(')?;
            let fields: Vec<_> = stat[end + 1..].split_whitespace().collect();
            let number = |index: usize| fields.get(index)?.parse::<u64>().ok();
            let scheduling = std::fs::read_to_string(entry.path().join("schedstat"))
                .ok()
                .and_then(|s| {
                    s.split_whitespace()
                        .take(3)
                        .map(str::parse::<u64>)
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .ok()
                });
            Some(
                json!({"tid":entry.file_name().to_str()?.parse::<u64>().ok()?,
                "name":&stat[start+1..end],"start_ticks":number(19)?,
                "minor_faults":number(7)?,"major_faults":number(9)?,
                "user_cpu_ticks":number(11)?,"system_cpu_ticks":number(12)?,
                "schedstat_run_ns_wait_ns_slices":scheduling}),
            )
        })();
        if let Some(thread) = result {
            threads.push(thread);
        } else {
            unavailable_threads += 1;
        }
    }
    threads.sort_by_key(|thread| thread["tid"].as_u64());
    // These controller snapshots include existing worker threads without adding
    // syscalls to a render quantum. A thread can retire during the sample.
    Ok(
        json!({"thread_samples":threads,"unavailable_threads":unavailable_threads,"cpu_ticks_per_second":unsafe { libc::sysconf(libc::_SC_CLK_TCK) },"resident_bytes":field("VmRSS:").and_then(|v|v.checked_mul(1024)),"peak_resident_bytes":field("VmHWM:").and_then(|v|v.checked_mul(1024)),"threads":field("Threads:"),"user_cpu_us":usage.ru_utime.tv_sec as u64*1_000_000+usage.ru_utime.tv_usec as u64,"system_cpu_us":usage.ru_stime.tv_sec as u64*1_000_000+usage.ru_stime.tv_usec as u64,"minor_faults":usage.ru_minflt,"major_faults":usage.ru_majflt}),
    )
}

/// `physical_authorized` comes only from the explicit command-line activation
/// switch. The default fake mode cannot turn into physical mode through Desk.
pub async fn run(mut config: Config, physical_authorized: bool) -> Result<Value> {
    validate_run_bounds(
        config.duration_ms,
        config.fake_skew_ppm,
        config.physical,
        physical_authorized,
    )?;
    if config.restore_intent {
        let path = config
            .epoch_file
            .parent()
            .ok_or("intent parent")?
            .join("brain-device-intent.json");
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > 65_536 {
            return Err("intent type/capacity".into());
        }
        let saved: DeviceConfig =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if saved.device_id != config.device.device_id || saved.endpoint != config.device.endpoint {
            return Err("saved intent device mismatch; explicit remap required".into());
        }
        config.device = saved;
    }
    config.device.validate().map_err(|e| e.to_string())?;
    let mut epoch = Epoch::open(&config.epoch_file)?;
    let credentials = Credentials::load_der(&config.certificate, &config.key, &config.ca)?;
    let mut client = RemoteClient::connect(
        client_endpoint(config.bind, &credentials)?,
        config.peer,
        &config.server_name,
        PolicyStore::new(config.peers)?,
    )
    .await?;
    let channel = client.take_brain_media()?;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(config.duration_ms);
    let mut service = service::Service::start(
        config.device.clone(),
        config.physical,
        epoch.value,
        client.identity().source_epoch.0,
        config.fake_skew_ppm,
        deadline,
    )?;
    let mut media = media::MediaWorker::start(channel, service.take_media())?;
    let resources_ready = process_resources()?;
    let mut descriptor: Option<BrainMediaDescriptor> = None;
    let mut proposed: Option<BrainMediaDescriptor> = None;
    let mut snapshot: Option<Snapshot> = None;
    let mut arm_fence = ArmFence::default();
    let mut generation = 1u64;

    let mut evidence = Evidence::default();
    let mut playback_hash = Sha256::new();
    let mut last_window: Option<(u64, u64, u64)> = None;
    let mut fault = None;
    let mut timer = tokio::time::interval(Duration::from_micros(250));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut controls = tokio::time::interval(Duration::from_millis(25));
    controls.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    client
        .report_brain_device(observation(&service, &config.device, None, None, None))
        .await?;
    client
        .send_command(query(&config.show_id, client.identity().source_epoch)?)
        .await?;
    let execution: Result<()> = async {
    loop {
        if started.elapsed() >= Duration::from_millis(config.duration_ms) {
            break;
        }
        tokio::select! {
            _=timer.tick()=>{
                service.poll();
                if let Some(error) = service.latest.service_fault.or(service.latest.host.fault) { fault=Some(error.to_string()); break; }
                for _ in 0..4 {
                    let Ok(block) = service.playback.pop() else { break };
                    let mut reconstructed = vec![0.; block.frames * block.width];
                    for (frame, row) in reconstructed.chunks_exact_mut(block.width).enumerate() {
                        row[block.slots[0]]=block.samples[frame*2];
                        row[block.slots[1]]=block.samples[frame*2+1];
                    }
                    let samples = reconstructed.as_slice();
                    evidence.playback_frames += block.frames as u64;
                    for sample in samples { playback_hash.update(sample.to_le_bytes()); evidence.playback_energy += sample*sample; evidence.playback_peak=evidence.playback_peak.max(sample.abs()); }
                    let state=(service.latest.bridge.epochs.route, block.revision, block.frame/4800);
                    if last_window!=Some(state) {
                        if evidence.playback_windows.len()<2048 { evidence.playback_windows.push(json!({"elapsed_ms":started.elapsed().as_millis() as u64,"epoch":service.latest.host.readback.epoch,"first_frame":block.frame,"service_revision_at_drain":block.revision,"controller_snapshot":snapshot,"bridge":service.latest.bridge,"samples":samples})); } else { evidence.playback_windows_overflow+=1; }
                        last_window=Some(state);
                    }
                    if evidence.first_playback.is_empty() && samples.iter().any(|v| *v!=0.) { evidence.first_playback=samples.to_vec(); }
                }

            }
            _=controls.tick()=>{
                client.send_command(query(&config.show_id,client.identity().source_epoch)?).await?;
                client.report_brain_device(observation(&service,&config.device,None,None,None)).await?;
            }
            reply=client.receive()=>{
                let payload=match reply{Ok(Response::Reply{payload,..})=>payload,Ok(Response::Refused{reason,..})=>{fault=Some(reason);break;},Ok(_)=>continue,Err(e)=>{fault=Some(e);break;}};
                match payload.get("contract").and_then(Value::as_str){
                    Some("GP15-brain")=>{
                        let reply:crate::brain_control::Reply=serde_json::from_value(payload).map_err(|e|e.to_string())?;
                        if let Some(s)=reply.snapshot{
                            if !arm_fence.observe(s.revision.0,s.monitor_armed||s.held_generation.is_some()){continue;}
                            let hold=s.held_generation.map_or(0,|v|v.0);
                            let changed=descriptor.as_ref().is_none_or(|d|d.selection_generation!=s.selection_generation.0||d.hold_generation!=hold||d.brain_epoch!=service.latest.host.readback.epoch);
                            if let Some(old)=&snapshot {
                                let capture=old.held_generation!=s.held_generation || (!old.talkback_mute && s.talkback_mute);
                                let monitor=old.selection_generation!=s.selection_generation || (old.monitor_armed && !s.monitor_armed) || (!old.monitor_mute && s.monitor_mute);
                                if capture || monitor { service.close_roles(capture,monitor)?; }
                            }
                            media.route(descriptor.as_ref(), Some(&s))?;
                            if changed&&proposed.is_none(){
                                generation=generation.checked_add(1).ok_or("media generation exhausted")?;
                                let next=BrainMediaDescriptor{contract:"GP15-media".into(),version:1,session:client.session(),stagebox_epoch:client.identity().source_epoch.0,brain_epoch:service.latest.host.readback.epoch,stagebox_map:client.identity().map_generation.0,brain_map:service.latest.host.readback.configuration_generation,generation,selection_generation:s.selection_generation.0,hold_generation:hold,sample_rate:48_000,frames:48,talkback:true,monitor:true};
                                client.negotiate_brain_media(next.clone()).await?;proposed=Some(next);
                            }
                            publish_state(&mut service, &s, descriptor.as_ref());
                            snapshot=Some(s);
                        }
                    }
                    Some("GP15-media") if payload["state"]=="accepted"=>{
                        let d:BrainMediaDescriptor=serde_json::from_value(payload["descriptor"].clone()).map_err(|e|e.to_string())?;
                        if proposed.as_ref()!=Some(&d){fault=Some("uncorrelated media acceptance".into());break;}
                        proposed=None;
                        if d.brain_epoch!=service.latest.host.readback.epoch {descriptor=None;continue;}
                        descriptor=Some(d);evidence.negotiations+=1;
                        media.route(descriptor.as_ref(), snapshot.as_ref())?;
                        if let Some(s) = &snapshot { publish_state(&mut service, s, descriptor.as_ref()); }
                    }
                    Some("GP15-device") if payload["state"]=="observed"=>{
                        if payload["brain_epoch"].as_u64()==Some(service.latest.host.readback.epoch)&&payload["brain_map"].as_u64()==Some(service.latest.host.readback.configuration_generation){arm_fence.acknowledged=true;}
                    }
                    Some("GP15-device") if payload["kind"]=="apply_configuration"=>{
                        let ticket=payload["ticket"].as_u64().ok_or("device ticket")?;
                        let next:DeviceConfig=serde_json::from_value(payload["config"].clone()).map_err(|e|e.to_string())?;
                        service.close()?;
                        service.finish()?;
                        arm_fence=ArmFence::default();snapshot=None;descriptor=None;proposed=None;
                        let fresh=epoch.advance()?;
                        match service::Service::start(next.clone(), config.physical, fresh, client.identity().source_epoch.0, config.fake_skew_ppm, deadline) {
                            Ok(next_service)=>{
                                service=next_service; config.device=next;
                                media.attach(service.take_media())?;
                                crate::show::persist(config.epoch_file.parent().ok_or("intent parent")?,"brain-device-intent.json",&config.device)?;
                                evidence.configuration_changes+=1;
                                client.report_brain_device(observation(&service,&config.device,Some(ticket),Some(true),None)).await?;
                            }
                            Err(e)=>{client.report_brain_device(observation(&service,&config.device,Some(ticket),Some(false),Some(e.clone()))).await?;fault=Some(e);break;}
                        }
                    }
                    _=>(),
                }
            }
        }
    }
        Ok(())
    }.await;
    if let Err(error) = execution {
        fault.get_or_insert(error);
    }
    let resources_before_stop = process_resources()?;
    service.finish()?;
    let media_report = media.finish()?;
    evidence.captured_packets = media_report.captured_packets;
    evidence.monitor_packets = media_report.monitor_packets;
    evidence.rejected_monitor_packets = media_report.rejected_monitor_packets;
    let final_observation = observation(&service, &config.device, None, None, fault.clone());
    let _ = client.report_brain_device(final_observation.clone()).await;
    client.close();
    let resources_stopped = process_resources()?;
    let report = json!({"resources_ready":resources_ready,"resources_before_stop":resources_before_stop,"resources_stopped":resources_stopped,"mode":"brain_duplex","software_only":!config.physical,"physical_qualified":false,"fault":fault,"evidence":evidence,"captured_pre_wire_sha256":media_report.captured_pre_wire_sha256,"received_monitor_sha256":media_report.received_monitor_sha256,"media_worker":media_report,"submitted_playback_sha256":crate::hash::hex(playback_hash.finalize()),"final_observation":final_observation,"elapsed_ms":started.elapsed().as_millis(),"fake_skew_ppm":config.fake_skew_ppm});
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&config.report)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn finite_run_limits_require_separate_physical_activation() {
        assert!(validate_run_bounds(60_000, 1000, false, false).is_ok());
        assert!(validate_run_bounds(60_001, 0, false, true).is_err());
        assert!(validate_run_bounds(86_400_000, 0, true, true).is_ok());
        assert!(validate_run_bounds(86_400_001, 0, true, true).is_err());
        assert!(validate_run_bounds(60_001, 0, true, false).is_err());
        assert!(validate_run_bounds(1, 0, true, false).is_err());
        assert!(validate_run_bounds(0, 0, false, false).is_err());
        assert!(validate_run_bounds(0, 0, true, true).is_err());
        assert!(validate_run_bounds(1, 2001, false, false).is_err());
    }

    #[test]
    fn durable_epochs_exclude_competing_owner_and_refuse_corrupt_recovery() {
        let directory = std::env::temp_dir().join(format!("gp15-epoch-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("epoch.json");
        let mut first = Epoch::open(&path).unwrap();
        assert_eq!(first.value, 1);
        assert!(Epoch::open(&path).is_err());
        assert_eq!(first.advance().unwrap(), 2);
        drop(first);
        assert_eq!(Epoch::open(&path).unwrap().value, 3);
        std::fs::write(&path, []).unwrap();
        assert!(Epoch::open(&path).is_err());
        std::fs::write(&path, b"broken").unwrap();
        assert!(Epoch::open(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn replacement_requires_ack_closed_readback_and_new_arm_revision() {
        let mut fence = ArmFence::default();
        assert!(!fence.observe(10, true));
        assert!(!fence.observe(11, false));
        fence.acknowledged = true;
        assert!(!fence.observe(12, true));
        assert!(fence.observe(13, false));
        assert!(!fence.observe(12, true));
        assert!(!fence.observe(13, true));
        assert!(fence.observe(14, true));
        fence = ArmFence::default();
        assert!(!fence.observe(14, true));
    }

    #[test]
    fn captured_blocks_keep_the_hold_that_owned_their_samples() {
        let epochs = BridgeEpochs {
            source: 1,
            destination: 2,
            route: 3,
        };
        let (tx, mut rx) = rtrb::RingBuffer::new(4);
        let mut renderer = Renderer {
            captured: tx,
            monitor: Bridge::prepare(BridgeConfig::voice(2), epochs).unwrap(),
            epochs,
            fault: None,
            dropped: 0,
            hold_generation: 11,
        };
        renderer.capture(2, 0, &[0.25; 48]);
        renderer.hold_generation = 0;
        renderer.capture(2, 48, &[0.5; 48]);
        renderer.hold_generation = 12;
        renderer.capture(2, 96, &[0.75; 48]);
        let old = rx.pop().unwrap();
        let new = rx.pop().unwrap();
        assert_eq!(
            (old.hold_generation, old.frame, old.samples[0]),
            (11, 0, 0.25)
        );
        assert_eq!(
            (new.hold_generation, new.frame, new.samples[0]),
            (12, 96, 0.75)
        );
        assert!(rx.pop().is_err());
    }
    fn route_descriptor(selection: u64, hold: u64) -> BrainMediaDescriptor {
        BrainMediaDescriptor {
            contract: "GP15-media".into(),
            version: 1,
            session: 1,
            stagebox_epoch: 1,
            brain_epoch: 2,
            stagebox_map: 1,
            brain_map: 1,
            generation: 9,
            selection_generation: selection,
            hold_generation: hold,
            sample_rate: 48_000,
            frames: 48,
            talkback: true,
            monitor: true,
        }
    }
    #[test]
    fn pending_hold_proposal_cannot_restore_a_superseded_monitor_selection() {
        let pending = route_descriptor(3, 12);
        // The selection changed while this hold-only proposal was in flight.
        assert!(!monitor_selection_current(Some(4), &pending));
        assert_eq!(accepted_capture_hold(Some(12), &pending), 12);
        // A subsequent release also closes capture, regardless of stale ACK.
        assert_eq!(accepted_capture_hold(Some(0), &pending), 0);
        assert!(!monitor_selection_current(None, &pending));
        let current = route_descriptor(4, 12);
        assert!(monitor_selection_current(Some(4), &current));
    }
    #[test]
    fn captured_hold_packets_remain_valid_across_monitor_selection_generations() {
        let epochs = BridgeEpochs {
            source: 1,
            destination: 2,
            route: 3,
        };
        let (tx, mut rx) = rtrb::RingBuffer::new(4);
        let mut renderer = Renderer {
            captured: tx,
            monitor: Bridge::prepare(BridgeConfig::voice(2), epochs).unwrap(),
            epochs,
            fault: None,
            dropped: 0,
            hold_generation: 11,
        };
        renderer.capture(2, 0, &[0.25; 48]);
        // Monitor route identity does not alter the held-role wire identity.
        renderer.monitor.invalidate();
        renderer.capture(2, 48, &[0.5; 48]);
        let old = route_descriptor(3, 11);
        let next = route_descriptor(4, 11);
        assert!(!monitor_selection_current(Some(4), &old));
        for (frame, sample) in [(0, 0.25), (48, 0.5)] {
            let block = rx.pop().unwrap();
            assert_eq!((block.frame, block.hold_generation), (frame, 11));
            let bytes = BrainPacket {
                descriptor: old.clone(),
                role: BrainMediaRole::Talkback,
                frame: block.frame,
                samples: block.samples.to_vec(),
            }
            .encode()
            .unwrap();
            let decoded = BrainPacket::decode(&bytes, &next, BrainMediaRole::Talkback).unwrap();
            assert_eq!(decoded.samples, vec![sample; 48]);
        }
        assert!(rx.pop().is_err());
    }
}
