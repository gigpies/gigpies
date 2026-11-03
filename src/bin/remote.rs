//! Explicit bounded authenticated provider/Brain; physical PCM requires a CLI gate.
// The source future stays on the calling thread. QUIC/control tasks must keep
// polling while that owner renders, services commands, or catches up source ticks.
fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
}
fn main() {
    let result = runtime()
        .map_err(|e| e.to_string())
        .and_then(|rt| rt.block_on(run()));
    if let Err(error) = result {
        eprintln!("gigpies-remote: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let physical_authorized = args.len() == 3 && args[2] == "--activate-physical";
    if (args.len() != 2 && !physical_authorized) || args[0] != "--config" {
        return Err(
            "usage: gigpies-remote --config PRIVATE.json [--activate-physical] (explicit provider or brain mode)".into(),
        );
    }
    let path = std::path::Path::new(&args[1]);
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 65_536 {
        return Err("config file capacity/type".into());
    }
    let config = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let report = gigpies::remote::run_config_authorized(config, physical_authorized).await?;
    println!(
        "{}",
        serde_json::json!({"mode":report["mode"],"fault":report["fault"],"software_only":report["software_only"]})
    );
    if !report["fault"].is_null() {
        return Err("bounded run reported a fault; retain its report".into());
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn network_tasks_progress_while_source_owner_is_busy() {
    let rt = runtime().unwrap();
    rt.block_on(async {
        let source_thread = std::thread::current().id();
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        let network = tokio::spawn(async move {
            tx.send(std::thread::current().id()).unwrap();
        });
        // A current-thread runtime cannot poll the network task until this owner
        // yields. This models the source's synchronous render/control interval.
        let network_thread = rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
        assert_ne!(source_thread, network_thread);
        network.await.unwrap();
    });
}
