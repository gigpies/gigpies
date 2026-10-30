//! Explicit Linux-only synthetic engine. Opens only the requested private Unix socket.
#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = run() {
        eprintln!("gigpies-headless: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gigpies-headless requires Linux Unix peer credentials");
    std::process::exit(1);
}
#[cfg(target_os = "linux")]
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(target_os = "linux")]
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}
#[cfg(target_os = "linux")]
fn run() -> Result<(), String> {
    use gigpies::{local_audio::LocalAudio, show::Counter};
    let mut directory = None;
    let mut show = None;
    let mut epoch = None;
    let mut ticks = None;
    let mut analysis = false;
    let mut analysis_map = None;
    let mut measurement_owner = None;
    let mut fouraux = false;
    let mut modules: Option<String> = None;
    let mut fx_library = None;
    let mut fx_hash = None;
    let mut topology = None;
    let mut arm = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!(
                "Usage: gigpies-headless --directory ABS_PRIVATE_0700_DIR --show UUID --epoch POSITIVE_INTEGER [--ticks COUNT] [--synthetic-source fouraux] [--analysis | --analysis-map ABS_JSON] [--modules ABS_HASH_MANIFEST] [--fx-aux-library ABS_SO --fx-aux-sha256 SHA256] [--topology ABS_JSON] [--measurement-owner ABS_JSON] [--arm-synthetic]\nSynthetic configurable engine; defaults to explicit legacy eight-input/two-monitor compatibility. Expanded topology starts disarmed. Same-UID audio.sock only; no physical devices. New show/epoch identity must be supplied explicitly."
            );
            return Ok(());
        }
        if arg == "--arm-synthetic" {
            arm = true;
            continue;
        }
        if arg == "--analysis" {
            analysis = true;
            continue;
        }
        let value = args.next().ok_or("missing option value")?;
        match arg.as_str() {
            "--analysis-map" if analysis_map.is_none() => analysis_map = Some(value),
            "--measurement-owner" if measurement_owner.is_none() => measurement_owner = Some(value),
            "--topology" => topology = Some(value),
            "--modules" => modules = Some(value),
            "--fx-aux-library" => fx_library = Some(value),
            "--fx-aux-sha256" => fx_hash = Some(value),
            "--directory" => directory = Some(value),
            "--show" => show = Some(value),
            "--epoch" => epoch = Some(value.parse::<u64>().map_err(|e| e.to_string())?),
            "--synthetic-source" => {
                if value != "fouraux" {
                    return Err("unknown synthetic source".into());
                }
                fouraux = true;
            }
            "--ticks" => ticks = Some(value.parse::<u64>().map_err(|e| e.to_string())?),
            _ => return Err("unknown option".into()),
        }
    }
    let directory = directory.ok_or("explicit directory required")?;
    let show = show.ok_or("explicit show required")?;
    let epoch = epoch.ok_or("explicit new epoch required")?;
    if ticks.is_some_and(|n| n > 1_000_000) {
        return Err("ticks limit".into());
    }
    let analysis_mapping = if let Some(path) = analysis_map {
        use std::io::Read;
        if analysis || !std::path::Path::new(&path).is_absolute() {
            return Err("choose one analysis mode and an absolute mapping path".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 4096 {
            return Err("analysis mapping capacity".into());
        }
        Some(
            serde_json::from_slice::<gigpies::analysis_stream::Mapping>(&bytes)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let topology = if let Some(path) = topology {
        serde_json::from_slice::<gigpies::topology::EngineTopology>(
            &std::fs::read(path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
    } else {
        gigpies::topology::EngineTopology::legacy()
    };
    let capability_version = if topology.is_legacy() { 1 } else { 2 };
    let mut server = LocalAudio::bind_configured(
        std::path::Path::new(&directory),
        "audio.sock",
        &show,
        Counter(epoch),
        topology,
    )?;
    match (fx_library, fx_hash) {
        (Some(path), Some(hash)) => {
            #[cfg(feature = "hardware-host")]
            server.enable_fx_aux(std::path::Path::new(&path), &hash)?;
            #[cfg(not(feature = "hardware-host"))]
            {
                let _ = (path, hash);
                return Err("FX aux requires hardware-host feature".into());
            }
        }
        (None, None) => (),
        _ => return Err("FX aux library and hash required together".into()),
    }
    if arm {
        server.rearm()?;
    }
    if fouraux {
        server.enable_synthetic_fouraux()?;
    }
    if analysis {
        server.enable_analysis(std::path::Path::new(&directory))?;
    }
    if let Some(mapping) = analysis_mapping {
        server.enable_configured_analysis(std::path::Path::new(&directory), &mapping)?;
    }
    let module_enabled = modules.is_some();
    if let Some(manifest) = modules {
        #[cfg(feature = "hardware-host")]
        server.enable_modules(std::path::Path::new(&manifest))?;
        #[cfg(not(feature = "hardware-host"))]
        {
            let _ = manifest;
            return Err("--modules requires hardware-host build; no devices are opened".into());
        }
    }
    if let Some(path) = measurement_owner {
        use std::io::Read;
        if !std::path::Path::new(&path).is_absolute() {
            return Err("measurement startup path must be absolute".into());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 4096 {
            return Err("measurement startup capacity".into());
        }
        let config: gigpies::measurement_owner::Config =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        server.enable_software_measurement(config)?;
    }
    // SAFETY: handlers perform only an atomic store; installed in this explicit process.
    unsafe {
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
    }
    let path = if module_enabled {
        "GP05-modules:1 fixed owner graph; query module_status; physical unverified"
    } else {
        "offline-unprotected; modules unavailable"
    };
    eprintln!(
        "GP03-rendered:{capability_version} synthetic show={show} epoch={epoch} socket={directory}/audio.sock {path}; no physical output arm"
    );
    let started = std::time::Instant::now();
    let mut count = 0_u64;
    while !STOP.load(std::sync::atomic::Ordering::Relaxed)
        && ticks.is_none_or(|limit| count < limit)
    {
        let now = u64::try_from(started.elapsed().as_millis()).map_err(|_| "elapsed exhausted")?;
        server.tick(now)?;
        count = count.checked_add(1).ok_or("tick exhausted")?;
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    Ok(())
}
