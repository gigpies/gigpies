use gigpies::{mixer::Mixer, topology::EngineTopology};
#[test]
fn four_auxes_are_independent_post_fader_and_monitor_count_is_unchanged() {
    let mut m = Mixer::from_topology(0, EngineTopology::software(4, 3, 0).unwrap()).unwrap();
    m.enable_fx_aux().unwrap();
    for i in 0..4 {
        let mut sends = [0.; 4];
        sends[i] = 1.;
        m.set_fx_sends(i, &sends).unwrap();
    }
    let mut output = [0.; 48 * 5];
    let source: [f64; 48 * 4] = std::array::from_fn(|i| (i % 4 + 1) as f64 * 0.01);
    for _ in 0..6 {
        m.process_interleaved(&source, &mut output).unwrap();
    }
    let gain = 10_f64.powf(-6. / 20.) * std::f64::consts::FRAC_1_SQRT_2;
    for (bus, samples) in m.fx_aux_samples().iter().enumerate() {
        for s in samples {
            assert!((s - (bus + 1) as f64 * 0.01 * gain).abs() < 1e-12);
        }
    }
    assert_eq!(m.monitor_count(), 3);
    let before = m.fx_sends(0).unwrap();
    assert!(m.set_fx_sends(0, &[f64::NAN; 4]).is_err());
    assert_eq!(m.fx_sends(0).unwrap(), before);
    assert!(m.set_fx_sends(4, &[0.; 4]).is_err());
    m.quiesce();
    m.process_interleaved(&source, &mut output).unwrap();
    assert!(output.iter().all(|s| *s == 0.));
    assert!(m.fx_aux_samples().iter().flatten().all(|s| *s == 0.));
}
#[cfg(feature = "hardware-host")]
#[test]
#[ignore = "actual SHR FX library: GP_FX_LIBRARY absolute path; no hardware"]
fn actual_palette_four_owners_wet_output_and_lifecycle() {
    use gigpies::host::fx_palette::Owner;
    let path = std::path::PathBuf::from(std::env::var("GP_FX_LIBRARY").expect("GP_FX_LIBRARY"));
    let mut owners: Vec<_> = (0..4).map(|_| Owner::load(&path).unwrap()).collect();
    let mut outs = [[0.; 96]; 4];
    for (i, o) in owners.iter_mut().enumerate() {
        let mut c = o.defaults([1, 6, 8, 10][i]).unwrap();
        c.time_ms = if matches!(i, 1 | 3) { 0. } else { 2. };
        o.prepare(&c).unwrap();
        let revision = o.status().unwrap().revision;
        o.publish(1, revision).unwrap();
    }
    for _ in 0..24 {
        for (o, out) in owners.iter_mut().zip(&mut outs) {
            assert_eq!(o.process(&[0.; 96], out), 0);
            o.reclaim();
        }
    }
    assert!(
        owners
            .iter_mut()
            .all(|o| o.status().unwrap().applied_request == 1)
    );
    let mut energy = [0.; 4];
    for q in 0..128 {
        for (i, (o, out)) in owners.iter_mut().zip(&mut outs).enumerate() {
            let mut input = [0.; 96];
            if q == 0 {
                input[0] = 0.2 * (i + 1) as f64;
            }
            assert_eq!(o.process(&input, out), 0);
            assert!(out.iter().all(|x| x.is_finite()));
            energy[i] += out.iter().map(|v| v * v).sum::<f64>();
        }
    }
    assert!(
        energy.iter().all(|e| *e > 1e-12),
        "actual wet energy {energy:?}"
    );
    let prior = owners[1].status().unwrap();
    let mut bad = owners[0].defaults(1).unwrap();
    bad.amount = 10.;
    assert!(owners[0].prepare(&bad).is_err());
    assert_eq!(owners[0].status().unwrap().accepted_request, 1);
    let mut bypass = owners[1].status().unwrap().target;
    bypass.bypass = 1;
    owners[1].prepare(&bypass).unwrap();
    owners[1].publish(2, prior.revision).unwrap();
    owners[1].process(&[0.; 96], &mut outs[1]);
    assert!(
        outs[1].iter().any(|s| *s != 0.),
        "plate bypass must drain existing tail"
    );
    owners[1].reclaim();
    owners[1].reset().unwrap();
    owners[1].process(&[0.; 96], &mut outs[1]);
    assert!(outs[1].iter().all(|s| *s == 0.));
    assert_eq!(owners[2].status().unwrap().applied_request, 1);
    let mut invalid = [0.; 96];
    invalid[0] = f64::NAN;
    assert_ne!(owners[0].process(&invalid, &mut outs[0]), 0);
    assert_eq!(outs[0], [0.; 96]);
    assert_ne!(owners[0].status().unwrap().fault, 0);
    owners[0].reset().unwrap();
    assert_eq!(owners[0].status().unwrap().fault, 0);
    eprintln!("four actual owners wet energies {energy:?}");
}

#[test]
fn source_recovery_retains_aux_send_intent_without_rearming_or_changing_saved_core_schema() {
    use gigpies::{mixer_control::OfflineEngine, show::Counter};
    let mut e = OfflineEngine::new(
        "11111111-1111-4111-8111-111111111111",
        Counter(1),
        Counter(0),
        0,
    )
    .unwrap();
    e.enable_fx_aux().unwrap();
    let mut sends = [0.; 8];
    sends[3] = 0.75;
    e.set_fx_sends(2, &sends).unwrap();
    let saved = e.persisted_intent().unwrap();
    e.quiesce("source lost");
    e.recover(Counter(2), 0).unwrap();
    assert!(e.outputs_quiesced());
    assert_eq!(e.mixer().fx_sends(2).unwrap(), sends);
    let fresh = OfflineEngine::restore_intent(&saved, Counter(3), 0).unwrap();
    assert!(
        fresh.mixer().fx_sends(2).is_none(),
        "aux state is runtime-owned, not silently added to the core saved schema"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn malformed_aux_client_cannot_stop_source_processing() {
    use std::{
        io::Write,
        os::unix::{fs::PermissionsExt, net::UnixStream},
    };
    let path = std::env::temp_dir().join(format!("fx-aux-malformed-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    let mut p = gigpies::local_audio::LocalAudio::bind_configured(
        &path,
        "host",
        "11111111-1111-4111-8111-111111111111",
        gigpies::show::Counter(9),
        EngineTopology::software(4, 2, 0).unwrap(),
    )
    .unwrap();
    for(now,value)in [serde_json::json!({"contract":"GP-FX-aux","version":999}),serde_json::json!({"contract":"GP-FX-aux","version":1,"show_id":"11111111-1111-4111-8111-111111111111","module":"audio","epoch":"8","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_aux_snapshot","body":{}})].into_iter().enumerate(){let mut c=UnixStream::connect(path.join("host")).unwrap();let bytes=serde_json::to_vec(&value).unwrap();c.write_all(&(bytes.len()as u32).to_be_bytes()).unwrap();c.write_all(&bytes).unwrap();p.tick(now as u64).unwrap();}
    assert_eq!(p.frame(), 96);
}
