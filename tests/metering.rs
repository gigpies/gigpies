use gigpies::{
    meter_wire::{Request, Snapshot},
    metering::{self, Accumulator},
    topology::{EngineTopology, ResourceBudget},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
thread_local! {static ACTIVE:Cell<bool>=const{Cell::new(false)};static EVENTS:Cell<usize>=const{Cell::new(0)};}
struct Guard;
unsafe impl GlobalAlloc for Guard {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ACTIVE.with(|a| {
            if a.get() {
                EVENTS.with(|n| n.set(n.get() + 1))
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        ACTIVE.with(|a| {
            if a.get() {
                EVENTS.with(|n| n.set(n.get() + 1))
            }
        });
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOC: Guard = Guard;
const SHOW: &str = "00000000-0000-0000-0000-000000000001";
fn window(inputs: usize, monitors: usize, signal: impl Fn(usize, usize) -> f64) -> Snapshot {
    let topology = EngineTopology::software(inputs, monitors, 0).unwrap();
    let (mut tap, mut latest) = metering::prepare(inputs, monitors, 48000).unwrap();
    let raw: Vec<f64> = (0..960 * inputs)
        .map(|i| signal(i / inputs, i % inputs))
        .collect();
    let processed = raw.clone();
    let valid = vec![true; 960];
    let main = vec![0.; 1920];
    let buses = vec![0.; 960 * (monitors + 2)];
    tap.offer(0, 0, &raw, &processed, &valid, &main, &buses, true);
    latest.snapshot(
        &Request::new(SHOW, 1, 1, 1),
        &topology,
        1,
        1,
        20,
        tap.losses,
        false,
    )
}
#[test]
fn arithmetic_silence_dc_impulse_sine_floor_clip_nonfinite_extreme() {
    for (signal, peak, rms, silent) in [
        (0., -120000, -120000, true),
        (0.5, -6021, -6021, false),
        (1., 0, 0, false),
        (2., 6021, 6021, false),
        (1e-9, -120000, -120000, false),
    ] {
        let s = window(1, 0, |_, _| signal);
        s.validate().unwrap();
        let t = &s.taps[0];
        assert_eq!(t.peak_millidbfs, Some(peak));
        assert_eq!(t.rms_millidbfs, Some(rms));
        assert_eq!(t.silent, silent);
        assert_eq!(t.clip_count.0, if signal >= 1. { 960 } else { 0 });
        assert_eq!(t.over_range, signal > 1.);
    }
    let s = window(1, 0, |f, _| {
        0.5 * (std::f64::consts::TAU * f as f64 / 48.).sin()
    });
    assert_eq!(s.taps[0].rms_millidbfs, Some(-9031));
    let s = window(1, 0, |f, _| if f == 959 { 0.5 } else { 0. });
    assert_eq!(s.taps[0].peak_millidbfs, Some(-6021));
    assert_eq!(
        s.taps[0].rms_millidbfs,
        Some((-6020.599913279624 - 10000. * 960_f64.log10()).round() as i32)
    );
    let s = window(1, 0, |f, _| if f == 99 { f64::NAN } else { 0.5 });
    s.validate().unwrap();
    assert!(!s.taps[0].valid);
    assert_eq!(s.taps[0].invalid_count.0, 1);
    assert_eq!(s.taps[0].peak_millidbfs, None);
    let s = window(1, 0, |_, _| f64::MAX);
    s.validate().unwrap();
    assert_eq!(s.taps[0].rms_millidbfs, s.taps[0].peak_millidbfs);
    let mut a = Accumulator::default();
    a.add(0., false);
    assert_eq!(
        a.record("x".into(), 1).reason.as_deref(),
        Some("graph_fault")
    );
}
#[test]
fn render_handoff_is_heap_free_even_saturated_disconnected_and_reset() {
    let (mut tap, latest) = metering::prepare(48, 7, 48000).unwrap();
    let raw = vec![0.5; 48 * 48];
    let main = [0.; 96];
    let buses = vec![0.; 48 * 9];
    let valid = [true; 48];
    // Dropping the worker does not change the source or render work.
    drop(latest);
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|n| n.set(true));
    for i in 0..100 {
        tap.offer(i * 48, i, &raw, &raw, &valid, &main, &buses, true);
    }
    tap.reset();
    ACTIVE.with(|n| n.set(false));
    assert_eq!(EVENTS.with(Cell::get), 0);
    assert_eq!(tap.losses, 3);
}
#[test]
fn partial_gap_and_reset_discard_without_identity_splice() {
    let t = EngineTopology::software(1, 0, 0).unwrap();
    let (mut tap, mut latest) = metering::prepare(1, 0, 48000).unwrap();
    let r = Request::new(SHOW, 1, 1, 1);
    tap.offer(
        0,
        0,
        &[0.5; 48],
        &[0.5; 48],
        &[true; 48],
        &[0.; 96],
        &[0.; 96],
        true,
    );
    assert!(!latest.snapshot(&r, &t, 1, 1, 1, 0, false).valid);
    tap.offer(
        960,
        1,
        &[0.5; 960],
        &[0.5; 960],
        &[true; 960],
        &[0.; 1920],
        &[0.; 1920],
        true,
    );
    let s = latest.snapshot(&r, &t, 1, 1, 21, 0, false);
    assert_eq!(s.first_frame.unwrap().0, 960);
    assert_eq!(s.end_frame.unwrap().0, 1920);
    tap.reset();
    latest.clear();
    assert!(!latest.snapshot(&r, &t, 1, 1, 22, 0, false).valid);
    assert!(metering::prepare(1, 0, 44101).is_none());
    assert!(metering::prepare(2048, 0, 48000).is_none());
    let t = EngineTopology::software(48, 7, 0).unwrap();
    let audio = t.validate(ResourceBudget::default()).unwrap();
    let low = ResourceBudget {
        bytes: audio.estimated_bytes,
        sample_operations: ResourceBudget::default().sample_operations,
    };
    assert!(t.validate(low).is_ok());
    assert!(t.meter_admission(low).is_err());
}
#[test]
fn strict_wire_and_frozen_provider_corpus() {
    let request = Request::new(SHOW, 1, 1, 1);
    let bytes = serde_json::to_vec(&request).unwrap();
    Request::decode(&bytes).unwrap();
    let mut v = serde_json::to_value(&request).unwrap();
    v["writer"] = "bad".into();
    assert!(Request::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    let s = window(48, 5, |_, c| if c == 47 { 0.5 } else { 0. });
    Snapshot::decode(&serde_json::to_vec(&s).unwrap()).unwrap();
    let mut v = serde_json::to_value(s).unwrap();
    v["end_frame"] = "961".into();
    assert!(Snapshot::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    for entry in std::fs::read_dir("tests/fixtures/gp-meter/v1").unwrap() {
        let p = entry.unwrap().path();
        let n = p.file_name().unwrap().to_str().unwrap();
        if n.starts_with("valid-") || n.starts_with("unavailable-") {
            Snapshot::decode(&std::fs::read(&p).unwrap()).unwrap();
        } else if n.starts_with("reject-") {
            assert!(
                Snapshot::decode(&std::fs::read(&p).unwrap()).is_err(),
                "{n}"
            );
        }
    }
}
#[test]
#[ignore = "explicit producer corpus export; no automatic fixture changes"]
fn freeze_provider_corpus() {
    let out = std::path::PathBuf::from(std::env::var("GP_METER_FIXTURE_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let write = |name: &str, v: serde_json::Value| {
        std::fs::write(
            out.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&v).unwrap(),
        )
        .unwrap()
    };
    for (name, s) in [
        ("valid-dc", window(16, 3, |_, _| 0.5)),
        ("valid-silence", window(17, 1, |_, _| 0.)),
        (
            "valid-sine",
            window(32, 5, |f, _| {
                0.5 * (std::f64::consts::TAU * f as f64 / 48.).sin()
            }),
        ),
        (
            "valid-invalid",
            window(
                48,
                7,
                |f, c| if f == 99 && c == 47 { f64::NAN } else { 0.5 },
            ),
        ),
        ("valid-over-range", window(16, 0, |_, _| 2.)),
    ] {
        write(name, serde_json::to_value(s).unwrap());
    }
    let base = serde_json::to_value(window(16, 3, |_, _| 0.5)).unwrap();
    for reason in [
        "missing_window",
        "unsupported",
        "identity",
        "capacity",
        "quiesced",
    ] {
        let mut v = base.clone();
        v["valid"] = false.into();
        v["reason"] = reason.into();
        v["taps"] = serde_json::json!([]);
        for k in ["first_frame", "end_frame", "acquisition_age_ms"] {
            v[k] = serde_json::Value::Null;
        }
        write(&format!("unavailable-{reason}"), v);
    }
    for (name, key, value) in [
        ("end-frame", "end_frame", serde_json::json!("959")),
        ("bad-counter", "sequence", serde_json::json!(1)),
        ("extra", "extra", serde_json::json!(true)),
        ("map-zero", "map_generation", serde_json::json!("0")),
        ("rate", "sample_rate", serde_json::json!(44101)),
    ] {
        let mut v = base.clone();
        v[key] = value;
        write(&format!("reject-{name}"), v);
    }
    let mut v = base.clone();
    v["taps"][0]["rms_millidbfs"] = 0.into();
    write("reject-rms", v);
    let mut v = base.clone();
    v["taps"][1]["id"] = v["taps"][0]["id"].clone();
    write("reject-duplicate-tap", v);
    let mut v = base.clone();
    v.as_object_mut().unwrap().remove("reason");
    write("reject-missing-null", v);
    let mut v = base.clone();
    v["sequence"] = "2".into();
    v["first_frame"] = "960".into();
    v["end_frame"] = "1920".into();
    write("valid-next-window", v);
    let mut v = base.clone();
    v["acquisition_age_ms"] = "251".into();
    write("valid-stale", v);
    write(
        "request",
        serde_json::to_value(Request::new(SHOW, 1, 1, 1)).unwrap(),
    );
}
#[cfg(target_os = "linux")]
#[test]
fn real_local_audio_produces_independent_all_input_bus_levels() {
    use gigpies::{local_audio::LocalAudio, show::Counter};
    use std::os::unix::fs::PermissionsExt;
    for (inputs, monitors) in [(16, 3), (17, 1), (32, 5), (48, 7)] {
        let dir = std::env::temp_dir().join(format!("gp-meter-{}-{inputs}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        {
            let mut topology = EngineTopology::software(inputs, monitors, 0).unwrap();
            topology.inputs.reverse(); // admission refuses noncanonical logical IDs, preserve order but change physical map
            topology.inputs.reverse();
            for p in &mut topology.inputs {
                p.capture_slot = inputs - 1 - (p.id[6..].parse::<usize>().unwrap() - 1);
            }
            let mut provider =
                LocalAudio::bind_configured(&dir, "audio.sock", SHOW, Counter(1), topology.clone())
                    .unwrap();
            provider.rearm().unwrap();
            let mut output = vec![0.; 48 * topology.playback_channels];
            let capture: Vec<f64> = (0..48 * topology.capture_channels)
                .map(|i| {
                    if i % topology.capture_channels < inputs {
                        0.5
                    } else {
                        0.
                    }
                })
                .collect();
            for i in 0..40 {
                provider
                    .tick_with_capture(i, 1, i * 48, &capture, &mut output)
                    .unwrap();
            }
            let s = provider
                .meter_snapshot(&Request::new(SHOW, 1, 1, 1), 1)
                .unwrap();
            s.validate().unwrap();
            assert_eq!(s.first_frame.unwrap().0, 960);
            assert_eq!(s.end_frame.unwrap().0, 1920);
            let db = |v: f64| (20000. * v.log10()).round() as i32;
            for t in &s.taps[..inputs * 2] {
                assert_eq!(t.peak_millidbfs, Some(-6021));
                assert_eq!(t.rms_millidbfs, Some(-6021));
            }
            let main =
                inputs as f64 * 0.5 * 10f64.powf(-6. / 20.) * std::f64::consts::FRAC_1_SQRT_2;
            for t in &s.taps[inputs * 2..inputs * 2 + 2] {
                assert_eq!(t.peak_millidbfs, Some(db(main)));
                assert_eq!(t.clip_count.0, 960);
            }
            for m in 0..monitors {
                let amplitude = inputs as f64 * 0.5 * if m == 0 { 1. } else { 0.001 };
                assert_eq!(
                    s.taps[inputs * 2 + 2 + m].peak_millidbfs,
                    Some(db(amplitude))
                );
            }
            let old = provider
                .meter_snapshot(&Request::new(SHOW, 1, 1, 2), 1)
                .unwrap();
            assert_eq!(old.sequence, s.sequence);
            provider.quiesce_source("test").unwrap();
            assert!(
                !provider
                    .meter_snapshot(&Request::new(SHOW, 1, 1, 3), 1)
                    .unwrap()
                    .valid
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
#[cfg(target_os = "linux")]
#[test]
fn private_unix_read_and_bad_identity_cannot_stop_audio_or_admit_a_writer() {
    use gigpies::{local_audio::LocalAudio, show::Counter};
    use std::{
        io::{Read, Write},
        os::unix::{fs::PermissionsExt, net::UnixStream},
    };
    let dir = std::env::temp_dir().join(format!("gp-meter-unix-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    {
        let mut provider = LocalAudio::bind(&dir, "audio.sock", SHOW, Counter(1)).unwrap();
        for i in 0..20 {
            provider.tick(i).unwrap();
        }
        let mut client = UnixStream::connect(dir.join("audio.sock")).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .unwrap();
        let r = Request::new(SHOW, 1, 1, 1);
        let b = serde_json::to_vec(&r).unwrap();
        client.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
        client.write_all(&b).unwrap();
        provider.tick(20).unwrap();
        let mut prefix = [0; 4];
        client.read_exact(&mut prefix).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(prefix) as usize];
        client.read_exact(&mut bytes).unwrap();
        let s = Snapshot::decode(&bytes).unwrap();
        assert!(s.valid);
        assert_eq!(s.first_frame.unwrap().0, 0);
        assert_eq!(provider.engine_mut().revision(), Counter(0));
        let mut bad = UnixStream::connect(dir.join("audio.sock")).unwrap();
        let r = Request::new("00000000-0000-0000-0000-000000000002", 1, 1, 1);
        let b = serde_json::to_vec(&r).unwrap();
        bad.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
        bad.write_all(&b).unwrap();
        provider.tick(21).unwrap();
        assert_eq!(provider.frame(), 1056);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "explicit producer topology export to private GP_METER_TOPOLOGY_DIR"]
fn export_source_topologies() {
    let root = std::path::PathBuf::from(std::env::var("GP_METER_TOPOLOGY_DIR").unwrap());
    std::fs::create_dir_all(&root).unwrap();
    for (inputs, monitors) in [(16, 3), (17, 1), (32, 5), (48, 7), (17, 13)] {
        let mut t = EngineTopology::software(inputs, monitors, 0).unwrap();
        for p in &mut t.inputs {
            p.capture_slot = inputs - 1 - (p.id[6..].parse::<usize>().unwrap() - 1);
        }
        t.outputs.reverse();
        t.validate(ResourceBudget::default()).unwrap();
        std::fs::write(
            root.join(format!("{inputs}-{monitors}.json")),
            serde_json::to_vec_pretty(&t).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn selected_spectrum_handoff_has_no_callback_heap_work() {
    let mut tap = gigpies::spectrum::Spectrum::new(48000);
    let r = gigpies::spectrum::Request {
        contract: "GP-SPECTRUM".into(),
        version: 1,
        show_id: SHOW.into(),
        epoch: 1,
        map: 1,
        query: 1,
        input: "input-01".into(),
    };
    tap.snapshot(&r, 0, 0);
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    for i in 0..1000 {
        tap.offer(&[0.5; 48], 1, i * 48, i, 1);
    }
    ACTIVE.with(|a| a.set(false));
    assert_eq!(EVENTS.with(Cell::get), 0);
}

/// GP-METER P4: the main pre-PA tap with actual hash-verified owner PA/FX/REC
/// libraries loaded. The oracle is an independently loaded FX instance and an
/// independent slot-zero mixer, not another meter or a decoder round trip.
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
#[test]
#[ignore = "explicit hash-verified PA/FX/REC libraries (GP05_MANIFEST) and owner PA fixtures (GP_PA_V2_FIXTURES)"]
fn actual_owner_main_pre_pa_and_monitor_taps_match_independent_references() {
    use gigpies::{
        host::{adapters::Dsp, pa_v2::Pa},
        local_audio::LocalAudio,
        mixer::Mixer,
        module_graph::Manifest,
        show::Counter,
    };
    use std::os::unix::fs::PermissionsExt;
    let manifest_path = std::path::PathBuf::from(std::env::var("GP05_MANIFEST").unwrap());
    let manifest = Manifest::load(&manifest_path).unwrap();
    let fixtures = std::path::PathBuf::from(std::env::var("GP_PA_V2_FIXTURES").unwrap());
    let pa_json = std::fs::read(fixtures.join("stereo3way.json")).unwrap();
    let db = |v: f64| {
        if v == 0. {
            -120000
        } else {
            ((20000. * v.log10()).round() as i32).max(-120000)
        }
    };
    for (inputs, monitors) in [(16, 3), (17, 1), (32, 5), (48, 7)] {
        let dir =
            std::env::temp_dir().join(format!("gp-meter-owner-{}-{inputs}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        {
            let topology = EngineTopology::software(inputs, monitors, 6).unwrap();
            let mut provider =
                LocalAudio::bind_configured(&dir, "audio.sock", SHOW, Counter(1), topology.clone())
                    .unwrap();
            provider.enable_modules(&manifest_path).unwrap();
            provider.configure_pa(&pa_json).unwrap();
            provider.rearm().unwrap();
            let mut mixer = Mixer::from_topology(0, topology.clone()).unwrap();
            mixer.rearm().unwrap();
            let mut fx = Dsp::load(&manifest.fx.library, "fx", 48).unwrap();
            let mut pa = Pa::load(&manifest.pa.library, &pa_json, 1, 0).unwrap();
            assert_eq!(pa.rearm(1, 0), 0);
            let buses = monitors + 2;
            let mut capture = vec![0.; 48 * topology.capture_channels];
            let mut playback = vec![0.; 48 * topology.playback_channels];
            let mut raw = vec![0.; 48 * inputs];
            let mut dry = vec![0.; 48 * buses];
            let mut foh = [0.; 96];
            let mut wet = [0.; 96];
            let mut pa_out = [0.; 288];
            let mut main_reference = Vec::new();
            let mut bus_reference = Vec::new();
            let mut pa_matched = 0usize;
            for block in 0..60u64 {
                for f in 0..48 {
                    let frame = block * 48 + f as u64;
                    for i in 0..inputs {
                        let hz = 110. * (1 + i % 7) as f64;
                        let level = 0.05 + 0.01 * (i % 5) as f64;
                        let s = level * (std::f64::consts::TAU * hz * frame as f64 / 48000.).sin();
                        // REC accepts exact signed PCM24; both references use these values.
                        let q = (s * 8_388_608.).round() / 8_388_608.;
                        raw[f * inputs + i] = q;
                        capture[f * topology.capture_channels + topology.inputs[i].capture_slot] =
                            q;
                    }
                }
                provider
                    .tick_with_capture(block, 1, block * 48, &capture, &mut playback)
                    .unwrap();
                mixer.process_interleaved(&raw, &mut dry).unwrap();
                for f in 0..48 {
                    foh[f * 2] = dry[f * buses];
                    foh[f * 2 + 1] = dry[f * buses + 1];
                }
                assert_eq!(fx.process_result(&foh, &mut wet, 2), 0);
                let main: Vec<f64> = foh.iter().zip(&wet).map(|(d, w)| d + w).collect();
                assert_eq!(pa.process(&main, &mut pa_out, 1, block * 48), 0);
                for f in 0..48 {
                    let played =
                        &playback[f * topology.playback_channels..][..topology.playback_channels];
                    if played[buses..buses + 6] == pa_out[f * 6..f * 6 + 6] {
                        pa_matched += 1;
                    }
                }
                main_reference.extend(main);
                bus_reference.extend_from_slice(&dry);
            }
            assert_eq!(
                pa_matched,
                60 * 48,
                "actual PA output equals independent PA(dry+wet)"
            );
            let s = provider
                .meter_snapshot(&Request::new(SHOW, 1, 1, 1), 1)
                .unwrap();
            s.validate().unwrap();
            assert!(s.valid, "{:?}", s.reason);
            let (first, end) = (
                s.first_frame.unwrap().0 as usize,
                s.end_frame.unwrap().0 as usize,
            );
            assert_eq!(end - first, 960);
            let stats = |samples: &mut dyn Iterator<Item = f64>| {
                let (mut peak, mut sum, mut n, mut clips) = (0f64, 0f64, 0usize, 0u64);
                for v in samples {
                    peak = peak.max(v.abs());
                    sum += v * v;
                    n += 1;
                    clips += (v.abs() >= 1.) as u64;
                }
                (db(peak), db((sum / n as f64).sqrt()), clips)
            };
            for (ch, id) in ["main-l", "main-r"].into_iter().enumerate() {
                let tap = s.taps.iter().find(|t| t.id == id).unwrap();
                let (peak, rms, clips) =
                    stats(&mut (first..end).map(|f| main_reference[f * 2 + ch]));
                assert!(tap.valid, "{id}");
                assert!(
                    (tap.peak_millidbfs.unwrap() - peak).abs() <= 1,
                    "{id} peak {tap:?} {peak}"
                );
                assert!(
                    (tap.rms_millidbfs.unwrap() - rms).abs() <= 1,
                    "{id} rms {tap:?} {rms}"
                );
                assert_eq!(tap.clip_count.0, clips, "{id}");
            }
            let dry_main: f64 = (first..end)
                .map(|f| main_reference[f * 2] - bus_reference[f * buses])
                .map(f64::abs)
                .fold(0., f64::max);
            assert!(
                dry_main > 1e-4,
                "actual FX wet contributes to the measured main"
            );
            for m in 0..monitors {
                let tap = &s.taps[inputs * 2 + 2 + m];
                let (peak, rms, clips) =
                    stats(&mut (first..end).map(|f| bus_reference[f * buses + 2 + m]));
                assert!(
                    (tap.peak_millidbfs.unwrap() - peak).abs() <= 1,
                    "monitor {m} {tap:?}"
                );
                assert!(
                    (tap.rms_millidbfs.unwrap() - rms).abs() <= 1,
                    "monitor {m} {tap:?}"
                );
                assert_eq!(tap.clip_count.0, clips);
            }
            provider.quiesce_source("test").unwrap();
            assert!(
                !provider
                    .meter_snapshot(&Request::new(SHOW, 1, 1, 2), 1)
                    .unwrap()
                    .valid
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// GP-METER P2 fault matrix: identity changes, half-written frames and repeated
/// reads can neither refresh an old window nor stall audio or touch control state.
#[cfg(target_os = "linux")]
#[test]
fn epoch_recovery_slow_reader_and_repeated_reads_never_refresh_or_block() {
    use gigpies::{local_audio::LocalAudio, show::Counter};
    use std::{
        io::{Read, Write},
        os::unix::{fs::PermissionsExt, net::UnixStream},
    };
    let dir = std::env::temp_dir().join(format!("gp-meter-faults-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    {
        let topology = EngineTopology::software(4, 2, 0).unwrap();
        let mut provider =
            LocalAudio::bind_configured(&dir, "audio.sock", SHOW, Counter(1), topology.clone())
                .unwrap();
        provider.rearm().unwrap();
        let capture = vec![0.25; 48 * topology.capture_channels];
        let mut output = vec![0.; 48 * topology.playback_channels];
        for i in 0..40 {
            provider
                .tick_with_capture(i, 1, i * 48, &capture, &mut output)
                .unwrap();
        }
        let revision = provider.engine_mut().revision();
        let first = provider
            .meter_snapshot(&Request::new(SHOW, 1, 1, 1), 1)
            .unwrap();
        assert!(first.valid);
        // Repeated reads return the same immutable window, never a fresher one.
        for q in 2..6 {
            let again = provider
                .meter_snapshot(&Request::new(SHOW, 1, 1, q), 1)
                .unwrap();
            assert_eq!(
                (again.sequence, again.first_frame, again.end_frame),
                (first.sequence, first.first_frame, first.end_frame)
            );
        }
        // A slow reader that sends only part of a frame header must not stall
        // rendering or another reader's reply.
        let mut slow = UnixStream::connect(dir.join("audio.sock")).unwrap();
        slow.write_all(&[0, 0]).unwrap();
        let mut reader = UnixStream::connect(dir.join("audio.sock")).unwrap();
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let b = serde_json::to_vec(&Request::new(SHOW, 1, 1, 7)).unwrap();
        reader.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
        reader.write_all(&b).unwrap();
        let before = provider.frame();
        for i in 40..44 {
            provider
                .tick_with_capture(i, 1, i * 48, &capture, &mut output)
                .unwrap();
        }
        assert_eq!(
            provider.frame(),
            before + 4 * 48,
            "slow reader cannot block audio"
        );
        let mut prefix = [0; 4];
        reader.read_exact(&mut prefix).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(prefix) as usize];
        reader.read_exact(&mut bytes).unwrap();
        assert!(Snapshot::decode(&bytes).unwrap().valid);
        assert_eq!(
            provider.engine_mut().revision(),
            revision,
            "reads never mutate"
        );
        // Epoch recovery clears every cached window; nothing from epoch 1 survives.
        provider.recover_source(Counter(2), 44 * 48).unwrap();
        // Recovery also advances the map identity; the old identity is refused.
        let map = provider.topology().map_revision;
        assert!(map > 1);
        let stale = provider
            .meter_snapshot(&Request::new(SHOW, 2, 1, 8), 1)
            .unwrap();
        assert_eq!(stale.reason.as_deref(), Some("identity"));
        let cleared = provider
            .meter_snapshot(&Request::new(SHOW, 2, map, 8), 1)
            .unwrap();
        assert_eq!(cleared.reason.as_deref(), Some("missing_window"));
        assert!(
            cleared.first_frame.is_none()
                && cleared.taps.iter().all(|t| t.peak_millidbfs.is_none())
        );
        provider.rearm().unwrap();
        for i in 44..90 {
            provider
                .tick_with_capture(i, 2, i * 48, &capture, &mut output)
                .unwrap();
        }
        let fresh = provider
            .meter_snapshot(&Request::new(SHOW, 2, map, 9), 1)
            .unwrap();
        assert!(fresh.valid, "{:?}", fresh.reason);
        assert_eq!(fresh.source_epoch, Counter(2));
        assert!(
            fresh.first_frame.unwrap().0 >= 44 * 48,
            "no window spans the epoch change"
        );
        drop(slow);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
