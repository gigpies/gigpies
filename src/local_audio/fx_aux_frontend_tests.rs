//! Same-process synthetic source with the actual Desk Frontend in an owned child.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
#[test]
#[ignore = "GP_FX_LIBRARY and GP_FX_DESK_TEST_EXE; bounded local software-only Frontend/sample acceptance"]
fn actual_frontend_returns_local_samples() {
    let library = std::path::PathBuf::from(std::env::var_os("GP_FX_LIBRARY").unwrap());
    let hash = crate::hash::hex(Sha256::digest(std::fs::read(&library).unwrap()));
    let dir = std::env::temp_dir().join(format!("gigpies-desk-returns-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct Cleanup {
        dir: std::path::PathBuf,
        child: Option<std::process::Child>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(c) = &mut self.child {
                let _ = c.kill();
                let _ = c.wait();
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    let mut cleanup = Cleanup {
        dir: dir.clone(),
        child: None,
    };
    let show = "11111111-1111-4111-8111-111111111111";
    let mut p = LocalAudio::bind_configured(
        &dir,
        "audio.sock",
        show,
        Counter(9),
        crate::topology::EngineTopology::software(4, 2, 0).unwrap(),
    )
    .unwrap();
    p.enable_fx_aux(&library, &hash).unwrap();
    p.rearm().unwrap();
    let config = dir.join("desk.json");
    std::fs::write(
        &config,
        serde_json::to_vec(
            &json!({"show":show,"endpoint":dir.join("audio.sock"),"prepare_sends":true}),
        )
        .unwrap(),
    )
    .unwrap();
    let capture: [f64; 192] = std::array::from_fn(|i| if i % 4 == 0 { 0.125 } else { 0. });
    let mut output = [0.; 192];
    let dry = 0.125 * 10_f64.powf(-6. / 20.) * std::f64::consts::FRAC_1_SQRT_2;
    for _ in 0..32 {
        p.tick_with_capture(0, 9, p.frame(), &capture, &mut output)
            .unwrap();
    }
    for f in output.chunks_exact(4) {
        assert!((f[0] - dry).abs() < 1e-12 && (f[1] - dry).abs() < 1e-12);
        assert!((f[2] - 0.125).abs() < 1e-12 && (f[3] - 0.000125).abs() < 1e-12);
    }
    cleanup.child = Some(
        std::process::Command::new(std::env::var_os("GP_FX_DESK_TEST_EXE").unwrap())
            .args([
                "--ignored",
                "--exact",
                "actual_provider_return_controls",
                "--nocapture",
            ])
            .env("GP_FX_DESK_CONFIG", config)
            .env_remove("GP_FX_DESK_FIXTURE")
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "bounded Desk episode"
        );
        p.tick_with_capture(
            start.elapsed().as_millis() as u64,
            9,
            p.frame(),
            &capture,
            &mut output,
        )
        .unwrap();
        if let Some(status) = cleanup.child.as_mut().unwrap().try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..32 {
        p.tick_with_capture(
            start.elapsed().as_millis() as u64,
            9,
            p.frame(),
            &capture,
            &mut output,
        )
        .unwrap();
    }
    let wet = p
        .graph_command([0.; 384], |graph, wet| {
            *wet = graph.aux.as_ref().unwrap().wet;
        })
        .unwrap();
    let mut energy = 0.;
    for f in 0..48 {
        let left = (0..4).map(|b| wet[f * 8 + b * 2]).sum::<f64>();
        let right = (0..4).map(|b| wet[f * 8 + b * 2 + 1]).sum::<f64>();
        assert!((output[f * 4] - dry - left * 0.95).abs() < 1e-12);
        assert!((output[f * 4 + 1] - dry - right * 0.95).abs() < 1e-12);
        assert!((output[f * 4 + 2] - 0.125 - (left + right) * 0.025).abs() < 1e-12);
        assert!((output[f * 4 + 3] - 0.000125 - (left + right) * 0.05).abs() < 1e-12);
        energy += left * left + right * right;
    }
    assert!(energy > 0., "actual wet samples required");
    eprintln!(
        "Desk local applied routes: 192 independent Main/monitor sample comparisons; wet energy {energy}"
    );
}

#[test]
#[ignore = "GP_FX_LIBRARY and GP_FX_DESK_TEST_EXE; bounded local software-only Frontend/sample acceptance"]
fn actual_frontend_saved_fx_local_samples() {
    let library = std::path::PathBuf::from(std::env::var_os("GP_FX_LIBRARY").unwrap());
    let hash = crate::hash::hex(Sha256::digest(std::fs::read(&library).unwrap()));
    let dir = std::env::temp_dir().join(format!("gigpies-desk-saved-fx-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct Cleanup {
        dir: std::path::PathBuf,
        child: Option<std::process::Child>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(c) = &mut self.child {
                let _ = c.kill();
                let _ = c.wait();
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    let mut cleanup = Cleanup {
        dir: dir.clone(),
        child: None,
    };
    let show = "11111111-1111-4111-8111-111111111111";
    let mut p = LocalAudio::bind_configured(
        &dir,
        "audio.sock",
        show,
        Counter(9),
        crate::topology::EngineTopology::software(4, 2, 0).unwrap(),
    )
    .unwrap();
    p.enable_fx_aux(&library, &hash).unwrap();
    let core_before = serde_json::to_value(p.persisted_intent().unwrap()).unwrap();
    let config = dir.join("desk.json");
    std::fs::write(
        &config,
        serde_json::to_vec(
            &json!({"show":show,"endpoint":dir.join("audio.sock"),"saved_directory":dir}),
        )
        .unwrap(),
    )
    .unwrap();
    let capture: [f64; 192] = std::array::from_fn(|i| if i % 4 == 0 { 0.125 } else { 0. });
    let mut output = [0.; 192];
    cleanup.child = Some(
        std::process::Command::new(std::env::var_os("GP_FX_DESK_TEST_EXE").unwrap())
            .args([
                "--ignored",
                "--exact",
                "actual_provider_saved_fx_controls",
                "--nocapture",
            ])
            .env("GP_FX_DESK_CONFIG", config)
            .env_remove("GP_FX_DESK_FIXTURE")
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "bounded Desk episode"
        );
        p.tick_with_capture(
            start.elapsed().as_millis() as u64,
            9,
            p.frame(),
            &capture,
            &mut output,
        )
        .unwrap();
        assert!(output.iter().all(|s| *s == 0.));
        if let Some(status) = cleanup.child.as_mut().unwrap().try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(output.iter().all(|s| *s == 0.));
    assert_eq!(
        serde_json::to_value(p.persisted_intent().unwrap()).unwrap(),
        core_before
    );
    assert_eq!(
        p.engine().clock_status().state,
        crate::clock_domain::ClockState::Disarmed
    );
    eprintln!(
        "saved FX local: {} silent output samples; core/composed intent unchanged",
        p.frame() * 4
    );
}
