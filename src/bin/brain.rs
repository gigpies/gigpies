//! Explicit local-duplex Brain host. Physical opening requires an extra switch.
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("gigpies-brain: {error}");
        std::process::exit(1);
    }
}
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
async fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(args.len() == 2 || args.len() == 3)
        || args[0] != "--config"
        || (args.len() == 3 && args[2] != "--activate-physical")
    {
        return Err("usage: gigpies-brain --config PRIVATE.json [--activate-physical]".into());
    }
    let metadata = std::fs::metadata(&args[1]).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 65_536 {
        return Err("configuration type/capacity".into());
    }
    let bytes = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("configuration capacity".into());
    }
    let config = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let report = gigpies::brain_runtime::run(config, args.len() == 3).await?;
    println!(
        "{}",
        serde_json::json!({"mode":report["mode"],"software_only":report["software_only"],"fault":report["fault"]})
    );
    if !report["fault"].is_null() {
        return Err("Brain stopped with retained fault evidence".into());
    }
    Ok(())
}
#[cfg(not(all(target_os = "linux", feature = "hardware-host")))]
fn main() {
    eprintln!("gigpies-brain requires Linux and --features hardware-host");
    std::process::exit(1);
}
