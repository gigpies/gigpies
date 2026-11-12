//! Bounded actual-library provider for the Desk setup episode. Never opens PCM.
#![cfg(all(target_os = "linux", feature = "hardware-host"))]
use gigpies::{local_audio::LocalAudio, show::Counter, topology::EngineTopology};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[test]
#[ignore = "GP_SETUP_DIRECTORY, GP_SETUP_MANIFEST, GP_SETUP_PA_CONFIG, GP_SETUP_FX_LIBRARY, GP_SETUP_FX_HASH; software only, max 180 seconds"]
fn serve_audio_setup() {
    let path = PathBuf::from(std::env::var_os("GP_SETUP_DIRECTORY").unwrap());
    let epoch = std::env::var("GP_SETUP_EPOCH")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let mut provider = LocalAudio::bind_configured(
        &path,
        "audio.sock",
        "11111111-1111-4111-8111-111111111111",
        Counter(epoch),
        EngineTopology::software(2, 2, 6).unwrap(),
    )
    .unwrap();
    provider
        .enable_modules(&PathBuf::from(
            std::env::var_os("GP_SETUP_MANIFEST").unwrap(),
        ))
        .unwrap();
    provider
        .configure_pa(&std::fs::read(std::env::var_os("GP_SETUP_PA_CONFIG").unwrap()).unwrap())
        .unwrap();
    provider
        .enable_fx_aux(
            &PathBuf::from(std::env::var_os("GP_SETUP_FX_LIBRARY").unwrap()),
            &std::env::var("GP_SETUP_FX_HASH").unwrap(),
        )
        .unwrap();
    let start = Instant::now();
    let mut silent = 0usize;
    while start.elapsed() < Duration::from_secs(180) && !path.join("stop").exists() {
        let capture = [0.125; 96];
        let mut output = [0.; 480];
        provider
            .tick_with_capture(
                start.elapsed().as_millis() as u64,
                epoch,
                provider.frame(),
                &capture,
                &mut output,
            )
            .unwrap();
        assert!(output.iter().all(|s| *s == 0.), "disarmed output activated");
        silent += output.len();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(silent > 0);
    std::fs::write(
        path.join("result.json"),
        serde_json::json!({"silent_samples":silent,"physical_devices":false}).to_string(),
    )
    .unwrap();
}
