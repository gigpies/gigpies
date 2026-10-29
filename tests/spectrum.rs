//! Actual provider transport and deterministic software capture; never physical audio.
use gigpies::{local_audio::LocalAudio, show::Counter, topology::EngineTopology};
use std::{
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
#[test]
fn actual_selected_spectrum_tones_levels_silence_and_reconnect() {
    let dir = std::env::temp_dir().join(format!("gp-spectrum-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let show = "11111111-1111-4111-8111-111111111111";
    let owner_dir = dir.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicUsize::new(0));
    let applied_level = Arc::new(AtomicUsize::new(usize::MAX));
    let transition_frame = Arc::new(AtomicU64::new(0));
    let (halt, gain) = (stop.clone(), level.clone());
    let (applied, transition) = (applied_level.clone(), transition_frame.clone());
    let child = std::thread::spawn(move || {
        let mut owner = LocalAudio::bind(&owner_dir, "audio.sock", show, Counter(1)).unwrap();
        let topology = EngineTopology::legacy();
        let mut pcm = vec![0.; 48 * topology.capture_channels];
        let mut out = vec![0.; 48 * topology.playback_channels];
        let start = Instant::now();
        let mut previous_level = usize::MAX;
        while !halt.load(Ordering::Relaxed) {
            let frame = owner.frame();
            let selected_level = gain.load(Ordering::Acquire);
            if selected_level != previous_level {
                transition.store(frame, Ordering::Relaxed);
                applied.store(selected_level, Ordering::Release);
                previous_level = selected_level;
            }
            let a = match selected_level {
                0 => 0.5,
                1 => 0.125,
                _ => 0.,
            };
            for f in 0..48 {
                for (i, p) in topology.inputs.iter().enumerate() {
                    pcm[f * topology.capture_channels + p.capture_slot] = a
                        * (std::f64::consts::TAU
                            * [8., 85., 853.][i % 3]
                            * (frame + f as u64) as f64
                            / 4096.)
                            .sin();
                }
            }
            owner
                .tick_with_capture(start.elapsed().as_millis() as u64, 1, frame, &pcm, &mut out)
                .unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    let mut query = 0;
    let mut costs = vec![];
    let mut bytes = 0;
    let mut next_request = Instant::now();
    let mut request = |stream: &mut UnixStream, input: usize| {
        // Respect the provider's 80 ms spectrum-query limit, including retries.
        std::thread::sleep(next_request.saturating_duration_since(Instant::now()));
        query += 1;
        let start = Instant::now();
        let r = serde_json::json!({"contract":"GP-SPECTRUM","version":1,"show_id":show,"epoch":1,"map":1,"query":query,"input":format!("input-{input:02}")});
        let b = serde_json::to_vec(&r).unwrap();
        stream.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
        stream.write_all(&b).unwrap();
        let mut head = [0; 4];
        stream.read_exact(&mut head).unwrap();
        let n = u32::from_be_bytes(head) as usize;
        assert!(n < 4096);
        let mut b = vec![0; n];
        stream.read_exact(&mut b).unwrap();
        costs.push(start.elapsed().as_micros());
        bytes += n + 4;
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        next_request = Instant::now() + Duration::from_millis(100);
        assert_eq!(v["query"], query);
        assert_eq!(v["tap"], "raw-pre-processing");
        v
    };
    let ready = Instant::now() + Duration::from_secs(2);
    while !dir.join("audio.sock").exists() {
        assert!(Instant::now() < ready);
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut s = UnixStream::connect(dir.join("audio.sock")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut fresh_snapshot = |stream: &mut UnixStream, input, first_frame| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let v = request(stream, input);
            if v["valid"] == true && v["first_frame"].as_u64().is_some_and(|f| f >= first_frame) {
                return v;
            }
            assert!(Instant::now() < deadline, "no fresh spectrum window: {v}");
        }
    };
    let change_level = |next| {
        level.store(next, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(5);
        while applied_level.load(Ordering::Acquire) != next {
            assert!(
                Instant::now() < deadline,
                "capture did not apply level {next}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        // Require a complete FFT window after the producer acknowledged the
        // change, rather than assuming wall-clock sleep advances source frames.
        transition_frame.load(Ordering::Relaxed)
    };
    for (input, bin) in [(1, 8f64), (2, 85.), (3, 853.)] {
        let v = fresh_snapshot(&mut s, input, 0);
        assert_eq!(v["valid"], true);
        let b = v["bins"].as_array().unwrap();
        let peak = b
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| v.as_i64().unwrap())
            .unwrap();
        assert_eq!(
            peak.0,
            ((bin * 48000. / 4096. / 20.).log10() / 3. * 64f64).floor() as usize
        );
        assert!((peak.1.as_i64().unwrap() + 6021).abs() < 4);
    }
    let v = fresh_snapshot(&mut s, 3, change_level(1));
    assert!(
        (v["bins"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .max()
            .unwrap()
            + 18062)
            .abs()
            < 4
    );
    let v = fresh_snapshot(&mut s, 3, change_level(2));
    assert!(
        v["bins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.as_i64() == Some(-96000))
    );
    drop(s);
    let mut s = UnixStream::connect(dir.join("audio.sock")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let v = request(&mut s, 3);
    assert_eq!(v["supported"], true);
    assert_eq!(v["valid"], true);
    stop.store(true, Ordering::Relaxed);
    child.join().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    costs.sort();
    eprintln!(
        "software spectrum transport: {} queries, median {} us, max {} us, {} reply bytes",
        costs.len(),
        costs[costs.len() / 2],
        costs.last().unwrap(),
        bytes
    );
}

#[test]
#[ignore = "opt-in analysis/transport cost evidence; see GP-PANEL"]
fn spectrum_costs() {
    let samples =
        std::array::from_fn(|i| 0.5 * (std::f64::consts::TAU * 85. * i as f64 / 4096.).sin());
    let mut times = Vec::new();
    for i in 0..1100 {
        let start = Instant::now();
        std::hint::black_box(gigpies::spectrum::analyze(
            std::hint::black_box(&samples),
            48000,
        ));
        if i >= 100 {
            times.push(start.elapsed().as_nanos());
        }
    }
    times.sort();
    eprintln!(
        "FFT+64 log bands / 1000 windows: median={} us p95={} us",
        times[500] / 1000,
        times[949] / 1000
    );
    let mut tap = gigpies::spectrum::Spectrum::new(48000);
    let r = gigpies::spectrum::Request {
        contract: "GP-SPECTRUM".into(),
        version: 1,
        show_id: "bench".into(),
        epoch: 1,
        map: 1,
        query: 1,
        input: "input-01".into(),
    };
    tap.snapshot(&r, 0, 0);
    let start = Instant::now();
    for i in 0..1000 {
        tap.offer(std::hint::black_box(&[0.5; 48]), 1, i * 48, 0, 1);
    }
    eprintln!(
        "bounded selected copy / 1000 blocks: {} us total",
        start.elapsed().as_micros()
    );
}

#[test]
fn v2_topology_taps_patch_slots_and_epoch_reset() {
    let dir = std::env::temp_dir().join(format!("gp-spectrum-v2-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let show = "11111111-1111-4111-8111-111111111111";
    let mut t = EngineTopology::software(17, 5, 0).unwrap();
    // Output identity is deliberately different from its playback-slot index.
    t.outputs[0].playback_slot = 6;
    t.outputs[6].playback_slot = 0;
    t.outputs[1].source = None;
    t.outputs[6].id = "z".repeat(128);
    t.outputs[6].physical_port = "lab\nport".into();
    let mut owner =
        LocalAudio::bind_configured(&dir, "audio.sock", show, Counter(1), t.clone()).unwrap();
    owner.rearm().unwrap();
    let mut pcm = vec![0.; 48 * t.capture_channels];
    let mut out = vec![0.; 48 * t.playback_channels];
    let mut request = gigpies::spectrum::Request {
        contract: "GP-SPECTRUM".into(),
        version: 2,
        show_id: show.into(),
        epoch: 1,
        map: 1,
        query: 1,
        input: "input-17".into(),
    };
    for (source, tap, expected) in [
        ("input-17", "raw-pre-processing", -6021),
        ("main:right", "main-pre-pa", -15031),
        ("output:output-01", "physical-output-post-patch", -15031),
        ("output:output-02", "physical-output-post-patch", -96000),
    ] {
        request.input = source.into();
        let initial = owner.spectrum_snapshot(&request).unwrap();
        assert!(!initial.valid);
        let sources = initial.sources.unwrap();
        assert_eq!(sources.len(), 26);
        let long = sources.iter().find(|s| s.id.len() == 135).unwrap();
        assert!(long.label.contains("lab\\nport"));
        for _ in 0..110 {
            let frame = owner.frame();
            for f in 0..48 {
                pcm[f * t.capture_channels + 16] =
                    0.5 * (std::f64::consts::TAU * 85. * (frame + f as u64) as f64 / 4096.).sin();
            }
            owner
                .tick_with_capture(frame / 48, 1, frame, &pcm, &mut out)
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        let observation = loop {
            let s = owner.spectrum_snapshot(&request).unwrap();
            if s.valid {
                break s;
            }
            assert!(Instant::now() < deadline, "source {source}");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(observation.tap, tap);
        let fft = observation.fft_bins.unwrap();
        assert_eq!(fft.len(), 2049);
        assert!((fft.iter().max().unwrap() - expected).abs() < 3, "{source}");
        assert_eq!(observation.end_frame - observation.first_frame, 4096);
    }
    owner.recover_source(Counter(2), 0).unwrap();
    request.epoch = 2;
    assert!(
        owner.spectrum_snapshot(&request).is_err(),
        "old map rejected after recovery"
    );
    request.map = owner.topology().map_revision;
    assert!(
        !owner.spectrum_snapshot(&request).unwrap().valid,
        "old epoch cannot become new data"
    );
    request.version = 3;
    assert!(owner.spectrum_snapshot(&request).is_err());
    drop(owner);
    std::fs::remove_dir_all(dir).unwrap();
}
