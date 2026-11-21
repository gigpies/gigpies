#![cfg(target_os = "linux")]
use gigpies::{
    analysis_stream::{Descriptor, WindowCursor, local::monotonic_ms},
    local_audio::LocalAudio,
    show::Counter,
};
use std::{
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    process::{Child, Command, Stdio},
    time::Duration,
};
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "gp04-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
// Drain only a bounded tail, and always reap on failed assertions or reads.
struct Process {
    child: Child,
    stderr: Option<std::thread::JoinHandle<Vec<u8>>>,
}
impl Process {
    fn new(mut child: Child) -> Self {
        let mut pipe = child.stderr.take().unwrap();
        let stderr = std::thread::spawn(move || {
            let mut tail = Vec::new();
            let mut bytes = [0; 1024];
            while let Ok(n) = pipe.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&bytes[..n]);
                if tail.len() > 8192 {
                    tail.drain(..tail.len() - 8192);
                }
            }
            tail
        });
        Self {
            child,
            stderr: Some(stderr),
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.stderr.take() {
            eprintln!(
                "headless stderr: {}",
                String::from_utf8_lossy(&reader.join().unwrap())
            );
        }
    }
}
fn attach(path: &std::path::Path) -> UnixStream {
    subscribe(UnixStream::connect(path).unwrap())
}
fn subscribe(mut s: UnixStream) -> UnixStream {
    s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let b = br#"{"subscription":"lux.aux.v1","version":1}"#;
    s.write_all(&(b.len() as u32).to_be_bytes()).unwrap();
    s.write_all(b).unwrap();
    s
}
fn frame(s: &mut UnixStream, stage: &str) -> Vec<u8> {
    let mut n = [0; 4];
    s.read_exact(&mut n)
        .unwrap_or_else(|e| panic!("{stage}: length prefix: {e}"));
    let n = u32::from_be_bytes(n) as usize;
    assert!(n <= gigpies::analysis_stream::MAX_WIRE_BYTES);
    let mut bytes = vec![0; n];
    s.read_exact(&mut bytes)
        .unwrap_or_else(|e| panic!("{stage}: body {n} bytes: {e}"));
    bytes
}
#[test]
fn real_process_delivers_actual_signed_pcm_and_survives_reader_loss() {
    let dir = Directory::new();
    let mut child = Process::new(
        Command::new(env!("CARGO_BIN_EXE_gigpies-headless"))
            .args([
                "--directory",
                dir.0.to_str().unwrap(),
                "--show",
                "11111111-1111-4111-8111-111111111111",
                "--epoch",
                "9",
                "--synthetic-source",
                "fouraux",
                "--analysis",
                "--ticks",
                "2000",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let socket = dir.0.join("analysis.sock");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let stream = loop {
        match UnixStream::connect(&socket) {
            Ok(stream) => break stream,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) && std::time::Instant::now() < deadline =>
            {
                assert!(
                    child.child.try_wait().unwrap().is_none(),
                    "headless exited during startup: {e}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("initial analysis connection: {e}"),
        }
    };
    let mut s = subscribe(stream);
    let d: Descriptor = serde_json::from_slice(&frame(&mut s, "initial descriptor")).unwrap();
    d.validate().unwrap();
    let mut cursor = WindowCursor::new(d).unwrap();
    let bytes = frame(&mut s, "initial window");
    let w = cursor.accept(&bytes, monotonic_ms().unwrap()).unwrap();
    let p = gigpies::transport::Packet::parse(&w.packets[..624]).unwrap();
    let raw = gigpies::analysis_stream::synthetic_inputs(p.source_frame());
    assert_eq!(p.pcm(0).unwrap(), raw[0][0]);
    assert_ne!(p.pcm(1).unwrap(), 0);
    // Stalled consumer cannot stop another attachment or the real engine child.
    let _stalled = attach(&socket);
    drop(s);
    std::thread::sleep(Duration::from_millis(150));
    assert!(child.child.try_wait().unwrap().is_none());
    let mut fresh = attach(&socket);
    let d: Descriptor =
        serde_json::from_slice(&frame(&mut fresh, "reattached descriptor")).unwrap();
    let bytes = frame(&mut fresh, "reattached window");
    WindowCursor::new(d)
        .unwrap()
        .accept(&bytes, monotonic_ms().unwrap())
        .unwrap();
    drop(child);
}
#[test]
fn transport_stop_preserves_engine_and_analysis_does_not_select_source() {
    let dir = Directory::new();
    let mut audio = LocalAudio::bind(
        &dir.0,
        "audio.sock",
        "11111111-1111-4111-8111-111111111111",
        Counter(9),
    )
    .unwrap();
    assert!(audio.enable_analysis(&dir.0).is_err());
    audio.enable_synthetic_fouraux().unwrap();
    audio.enable_analysis(&dir.0).unwrap();
    audio.tick(0).unwrap();
    audio.stop_analysis();
    for _ in 0..100 {
        if !audio.analysis_alive() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!audio.analysis_alive());
    for i in 1..100 {
        audio.tick(i).unwrap();
    }
    assert_eq!(audio.frame(), 4800);
    assert!(audio.snapshot().is_ok());
}

#[test]
fn configured_capture_uses_logical_mapping_and_closes_on_patch_change() {
    use gigpies::{analysis_stream::Mapping, topology::EngineTopology};
    for channels in [4, 7] {
        let dir = Directory::new();
        let mut topology = EngineTopology::software(17, 3, 0).unwrap();
        for (i, port) in topology.inputs.iter_mut().enumerate() {
            port.capture_slot = 16 - i;
        }
        let mut audio = LocalAudio::bind_configured(
            &dir.0,
            "audio.sock",
            "11111111-1111-4111-8111-111111111111",
            Counter(9),
            topology.clone(),
        )
        .unwrap();
        let mapping = Mapping {
            version: if channels == 4 { 1 } else { 2 },
            sources: if channels == 4 {
                vec![]
            } else {
                [
                    "kick", "bass", "guitar-1", "guitar-2", "vocal", "synth", "backs",
                ]
                .map(String::from)
                .to_vec()
            },
            inputs: [
                "input-17", "input-03", "input-09", "input-01", "input-02", "input-04", "input-05",
            ][..channels]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        audio.enable_configured_analysis(&dir.0, &mapping).unwrap();
        let mut wrong_version = attach(&dir.0.join("analysis.sock"));
        let mut prefix = [0; 4];
        assert!(wrong_version.read_exact(&mut prefix).is_err());
        let mut stream = UnixStream::connect(dir.0.join("analysis.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let request = mapping.descriptor(&topology, 9, 0).unwrap();
        stream
            .write_all(&(request.attach_request().len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(request.attach_request()).unwrap();
        let descriptor: Descriptor =
            serde_json::from_slice(&frame(&mut stream, "configured descriptor")).unwrap();
        assert_eq!(descriptor, request);
        let mut playback = vec![0.; topology.playback_channels * 48];
        for block in 0..10 {
            let mut capture = vec![0.; 17 * 48];
            for f in 0..48 {
                for (i, p) in topology.inputs.iter().enumerate() {
                    capture[f * 17 + p.capture_slot] =
                        ((block * 48 + f) * 100 + i) as f64 / 8_388_608.;
                }
            }
            audio
                .tick_with_capture(block as u64, 9, block as u64 * 48, &capture, &mut playback)
                .unwrap();
        }
        let window = frame(&mut stream, "configured raw samples");
        assert_eq!(&window[..4], b"GAW1");
        for bytes in window[40..].chunks_exact(descriptor.spec().packet_bytes()) {
            let packet = gigpies::transport::Packet::parse(bytes).unwrap();
            for f in 0..48 {
                for (channel, index) in [16, 2, 8, 0, 1, 3, 4][..channels].iter().enumerate() {
                    assert_eq!(
                        packet.pcm(f * channels + channel).unwrap(),
                        ((packet.source_frame() as usize + f) * 100 + index) as i32
                    );
                }
            }
        }
        assert_eq!(audio.frame(), 480);
        assert!(audio.enable_configured_analysis(&dir.0, &mapping).is_err());
        let mut outputs = topology.outputs;
        outputs[0].source = None;
        audio.engine_mut().replace_output_patch(outputs).unwrap();
        audio
            .tick_with_capture(10, 9, 480, &vec![0.; 17 * 48], &mut playback)
            .unwrap();
        assert!(!audio.analysis_alive());
        assert_eq!(
            audio.frame(),
            528,
            "analysis loss cannot stop mixer processing"
        );
    }
}

#[test]
fn configured_analysis_refuses_unrepresentable_pcm_without_stuck_publisher() {
    use gigpies::{analysis_stream::Mapping, topology::EngineTopology};
    for value in [-1., 8_388_607. / 8_388_608., 1., f64::NAN, f64::INFINITY] {
        let dir = Directory::new();
        let topology = EngineTopology::software(4, 1, 0).unwrap();
        let mut audio = LocalAudio::bind_configured(
            &dir.0,
            "audio.sock",
            "11111111-1111-4111-8111-111111111111",
            Counter(9),
            topology.clone(),
        )
        .unwrap();
        let mapping = Mapping {
            version: 1,
            sources: vec![],
            inputs: ["input-01", "input-02", "input-03", "input-04"]
                .map(String::from)
                .to_vec(),
        };
        audio.enable_configured_analysis(&dir.0, &mapping).unwrap();
        let result = audio.tick_with_capture(
            0,
            9,
            0,
            &vec![value; 4 * 48],
            &mut vec![0.; topology.playback_channels * 48],
        );
        if value.is_finite() && value < 1. {
            result.unwrap();
            assert!(audio.analysis_alive());
        } else {
            assert!(!audio.analysis_alive());
        }
    }
}

#[test]
fn legacy_publisher_refuses_configured_request_and_fresh_epoch_requires_reattach() {
    use gigpies::{analysis_stream::Mapping, topology::EngineTopology};
    let dir = Directory::new();
    let topology = EngineTopology::software(17, 3, 0).unwrap();
    let mapping = Mapping {
        version: 1,
        sources: vec![],
        inputs: ["input-17", "input-03", "input-09", "input-01"]
            .map(String::from)
            .to_vec(),
    };
    let mut audio = LocalAudio::bind_configured(
        &dir.0,
        "audio.sock",
        "11111111-1111-4111-8111-111111111111",
        Counter(9),
        topology.clone(),
    )
    .unwrap();
    audio.enable_synthetic_fouraux().unwrap();
    audio.enable_analysis(&dir.0).unwrap();
    let mut stream = UnixStream::connect(dir.0.join("analysis.sock")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let descriptor = mapping.descriptor(&topology, 9, 0).unwrap();
    stream
        .write_all(&(descriptor.attach_request().len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(descriptor.attach_request()).unwrap();
    assert!(stream.read_exact(&mut [0; 4]).is_err());
    audio.quiesce_source("test_quiescence").unwrap();
    assert!(!audio.analysis_alive());
    audio.recover_source(Counter(10), 0).unwrap();
    assert!(!audio.analysis_alive());
    audio.enable_configured_analysis(&dir.0, &mapping).unwrap();
    let mut new = UnixStream::connect(dir.0.join("analysis.sock")).unwrap();
    new.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    new.write_all(&(descriptor.attach_request().len() as u32).to_be_bytes())
        .unwrap();
    new.write_all(descriptor.attach_request()).unwrap();
    let actual: Descriptor =
        serde_json::from_slice(&frame(&mut new, "new epoch descriptor")).unwrap();
    assert_eq!(actual.source_epoch.0, 10);
    assert_eq!(actual.inputs, mapping.inputs);
    assert!(audio.engine_mut().outputs_quiesced());
}
