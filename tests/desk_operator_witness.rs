//! Explicit, bounded bridge for the separately built Desk operator acceptance driver.
//! This runs the real provider and owner libraries on generated PCM, never devices.
#![cfg(all(target_os = "linux", feature = "hardware-host"))]

use gigpies::{local_audio::LocalAudio, show::Counter, topology::EngineTopology};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufWriter, Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::mpsc::{SyncSender, sync_channel},
    thread::{JoinHandle, spawn},
    time::{Duration, Instant},
};

const SHOW: &str = "11111111-1111-4111-8111-111111111111";
const FRAMES: usize = 48;
const INPUTS: usize = 17;
const MONITORS: usize = 3;
const PA_OUTPUTS: usize = 6;
const MAX_CAPTURES: usize = 16;

fn hash_file(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 16384];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    gigpies::hash::hex(digest.finalize())
}
fn provenance(manifest: &Path, fixture: &Path) -> Value {
    let revision = match option_env!("GP_DESK_WITNESS_SOURCE") {
        Some(revision) => revision,
        None => panic!(
            "build this explicit witness with GP_DESK_WITNESS_SOURCE=<reviewed full Git revision>"
        ),
    };
    assert!(revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()));
    let module: Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    let mut libraries = serde_json::Map::new();
    for name in ["rec", "fx", "pa"] {
        let path = fs::canonicalize(module[name]["library"].as_str().unwrap()).unwrap();
        let hash = hash_file(&path);
        assert_eq!(Some(hash.as_str()), module[name]["library_sha256"].as_str());
        libraries.insert(name.into(), json!({"path":path,"sha256":hash}));
    }
    json!({"gigpies_revision":revision,
           "executable_sha256":hash_file(&std::env::current_exe().unwrap()),
           "owner_manifest_sha256":hash_file(manifest),"libraries":libraries,
           "initial_pa_fixture_sha256":hash_file(fixture)})
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRequest {
    id: String,
    blocks: usize,
}
impl CaptureRequest {
    fn validate(&self) {
        assert!(
            !self.id.is_empty() && self.id.len() <= 32,
            "capture ID bound"
        );
        assert!(
            self.id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "capture ID characters"
        );
        assert!((1..=32).contains(&self.blocks), "capture block bound");
    }
}
struct Capture {
    request: CaptureRequest,
    before: Value,
    blocks: Vec<Value>,
}

type Publication = (String, Value);

// One publication in flight and one queued. The driver requests the next capture
// only after observing the previous durable file. Never wait for storage in the
// provider loop; unexpected backlog is an explicit failed witness, not lost data.
fn start_publisher(
    mut write: impl FnMut(Publication) + Send + 'static,
) -> (SyncSender<Publication>, JoinHandle<usize>) {
    let (sender, receiver) = sync_channel(1);
    let worker = spawn(move || {
        let mut count = 0;
        for publication in receiver {
            write(publication);
            count += 1;
        }
        count
    });
    (sender, worker)
}

fn publish(directory: &Path, name: &str, value: &Value) {
    let final_path = directory.join(name);
    let temporary = directory.join(format!(".{name}.tmp"));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .expect("new private evidence file");
    let mut file = BufWriter::new(file);
    serde_json::to_writer(&mut file, value).unwrap();
    file.write_all(b"\n").unwrap();
    file.flush().unwrap();
    file.get_ref().sync_all().unwrap();
    // Publish without replacing an existing unique capture or following a symlink.
    fs::hard_link(&temporary, &final_path).expect("unique evidence destination");
    fs::remove_file(temporary).unwrap();
    fs::File::open(directory).unwrap().sync_all().unwrap();
}
fn state(provider: &mut LocalAudio) -> Value {
    let raw = provider.snapshot().unwrap();
    let processing = provider.processing_snapshot().unwrap();
    let sends = provider.engine_mut().sends_snapshot().unwrap();
    let structure = provider.structural_snapshot().unwrap();
    let master_eq = provider.master_eq_snapshot().unwrap();
    let exported_intent = provider.persisted_intent().unwrap();
    json!({"raw":raw,"processing":processing,"sends":sends,
           "structure":structure,"master_eq":master_eq,"exported_intent":exported_intent})
}

#[test]
#[ignore = "external Desk driver; requires GP_DESK_WITNESS_DIR, GP_EQ_MANIFEST and GP_PA_V2_FIXTURES; generated PCM only; max180s"]
fn serve_desk_operator_witness() {
    let directory = PathBuf::from(
        std::env::var_os("GP_DESK_WITNESS_DIR").expect("explicit private run directory"),
    );
    assert!(directory.is_absolute());
    let metadata = fs::symlink_metadata(&directory).unwrap();
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    // SAFETY: reading the effective uid has no side effects.
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert!(
        fs::read_dir(&directory).unwrap().next().is_none(),
        "new empty private run directory required"
    );
    let manifest =
        PathBuf::from(std::env::var_os("GP_EQ_MANIFEST").expect("trusted owner manifest"));
    let fixtures =
        PathBuf::from(std::env::var_os("GP_PA_V2_FIXTURES").expect("owner fixture directory"));
    assert!(manifest.is_absolute() && fixtures.is_absolute());
    let proof = provenance(&manifest, &fixtures.join("stereo3way.json"));
    let topology = EngineTopology::software(INPUTS, MONITORS, PA_OUTPUTS).unwrap();
    let outputs = topology.playback_channels;
    let mut provider =
        LocalAudio::bind_configured(&directory, "audio.sock", SHOW, Counter(1), topology.clone())
            .unwrap();
    provider.enable_modules(&manifest).unwrap();
    provider
        .configure_pa(&fs::read(fixtures.join("stereo3way.json")).unwrap())
        .unwrap();
    assert!(provider.structural_snapshot().unwrap().outputs_quiesced);
    assert!(provider.master_eq_snapshot().unwrap().live_available);
    let pattern: Vec<f64> = (0..FRAMES)
        .map(|i| {
            let sample = 0.01 * (std::f64::consts::TAU * i as f64 / FRAMES as f64).sin();
            (sample * 8_388_608.).round() / 8_388_608.
        })
        .collect();
    let mut input = vec![0.; FRAMES * INPUTS];
    for (frame, sample) in pattern.iter().enumerate() {
        input[frame * INPUTS] = *sample;
    }
    let mut output = vec![0.; FRAMES * outputs];
    publish(
        &directory,
        "ready.json",
        &json!({
            "schema_version":1,"show_id":SHOW,"epoch":"1","socket":"audio.sock",
            "topology":topology,"input_zero_pattern":pattern,"other_inputs":"exact zero",
            "frame_block":FRAMES,"max_seconds":180,"max_captures":MAX_CAPTURES,
            "hardware_opened":false,"initial":state(&mut provider),"provenance":proof,
            "clock":"synthetic source frames; wall monotonic control age; no deadline evidence"
        }),
    );
    let start = Instant::now();
    let publication_directory = directory.clone();
    let (publications, publisher) = start_publisher(move |(name, value)| {
        publish(&publication_directory, &name, &value);
    });
    let mut active: Option<Capture> = None;
    let mut completed = BTreeMap::<String, CaptureRequest>::new();
    let mut blocks = 0_u64;
    let mut stopped = false;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        while start.elapsed() < Duration::from_secs(180) {
            assert!(!publisher.is_finished(), "evidence publisher failed");
            if directory.join("stop").exists() {
                assert!(active.is_none(), "stop during incomplete capture");
                stopped = true;
                break;
            }
            if active.is_none() {
                let request_path = directory.join("capture-request.json");
                if request_path.exists() {
                    let meta = fs::symlink_metadata(&request_path).unwrap();
                    assert!(meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 1024);
                    let request: CaptureRequest =
                        serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap();
                    request.validate();
                    if let Some(previous) = completed.get(&request.id) {
                        assert_eq!(previous, &request, "capture ID reused with different body");
                    } else {
                        assert!(completed.len() < MAX_CAPTURES, "capture count bound");
                        let before = state(&mut provider);
                        active = Some(Capture {
                            blocks: Vec::with_capacity(request.blocks),
                            before,
                            request,
                        });
                    }
                }
            }
            let now = u64::try_from(start.elapsed().as_millis()).unwrap();
            let epoch = provider.source_epoch();
            let frame = provider.frame();
            let revision_before = provider.engine_mut().revision();
            provider
                .tick_with_capture(now, epoch, frame, &input, &mut output)
                .unwrap();
            assert!(
                output.iter().all(|v| v.is_finite()),
                "finite synthetic output"
            );
            assert!(
                output
                    .chunks_exact(outputs)
                    .all(|row| row[2 + MONITORS..].iter().all(|v| v.abs() <= 1.)),
                "PA protection bound"
            );
            blocks += 1;
            if let Some(capture) = &mut active {
                capture.blocks.push(json!({
                "first_frame":frame.to_string(),"epoch":epoch.to_string(),"control_now_ms":now,
                "revision_before":revision_before,"revision_after":provider.engine_mut().revision(),
                "input_zero":pattern,"playback_interleaved":output
            }));
                if capture.blocks.len() == capture.request.blocks {
                    let capture = active.as_ref().unwrap();
                    let after = state(&mut provider);
                    publications
                        .try_send((
                            format!("capture-{}.json", capture.request.id),
                            json!({
                                "schema_version":1,"id":capture.request.id,"channels":outputs,
                                "before":capture.before,"blocks":capture.blocks,"after":after,
                                "hardware_opened":false
                            }),
                        ))
                        .expect("bounded evidence publisher unavailable/backlogged");
                    let capture = active.take().unwrap();
                    completed.insert(capture.request.id.clone(), capture.request);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            stopped,
            "bounded witness expired before explicit driver stop"
        );
        assert!(!completed.is_empty(), "driver completed no sample witness");
    }));
    drop(publications);
    let publication_result = publisher.join();
    let published = publication_result.as_ref().copied().unwrap_or(0);
    let outcome = outcome.and(publication_result.map(|_| ()));
    if let Err(payload) = outcome {
        let reason = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "non-string witness panic".into());
        let partial = active
            .as_ref()
            .map(|c| json!({"id":c.request.id,"before":c.before,"blocks":c.blocks}));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            publish(
                &directory,
                "failure.json",
                &json!({"reason":reason,"partial_capture":partial,
                "last_playback":output,"provider_frame":provider.frame().to_string(),
                "completed":completed.len(),"hardware_opened":false}),
            )
        }));
        std::panic::resume_unwind(payload);
    }
    publish(
        &directory,
        "summary.json",
        &json!({
            "schema_version":1,"stopped_by_driver":stopped,"captured":completed.len(),
            "rendered_blocks":blocks,"published":published,
            "final":state(&mut provider),"hardware_opened":false
        }),
    );
    assert!(
        stopped,
        "bounded witness expired before explicit driver stop"
    );
    assert!(!completed.is_empty(), "driver completed no sample witness");
    assert_eq!(
        published,
        completed.len(),
        "every accepted capture is durable"
    );
}

#[test]
fn witness_storage_backpressure_is_bounded_and_failure_is_observable() {
    let (entered, active) = sync_channel(0);
    let (release, released) = sync_channel(0);
    let (sender, worker) = start_publisher(move |_| {
        entered.send(()).unwrap();
        released.recv().unwrap();
    });
    sender.try_send(("first".into(), json!(1))).unwrap();
    active.recv().unwrap();
    sender.try_send(("second".into(), json!(2))).unwrap();
    assert!(matches!(
        sender.try_send(("third".into(), json!(3))),
        Err(std::sync::mpsc::TrySendError::Full(_))
    ));
    release.send(()).unwrap();
    active.recv().unwrap();
    drop(sender);
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), 2);

    let (sender, worker) = start_publisher(|_| panic!("injected storage failure"));
    sender.try_send(("failed".into(), Value::Null)).unwrap();
    assert!(worker.join().is_err());
    assert!(matches!(
        sender.try_send(("later".into(), Value::Null)),
        Err(std::sync::mpsc::TrySendError::Disconnected(_))
    ));
}

#[test]
fn witness_publication_drains_exact_data_without_overwriting() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("gp-witness-{}-{unique}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let target = directory.clone();
    let (sender, worker) = start_publisher(move |(name, value)| publish(&target, &name, &value));
    let expected = json!({"samples":[0.0,-0.125,0.25],"epoch":"1","frame":"48"});
    sender
        .try_send(("capture.json".into(), expected.clone()))
        .unwrap();
    drop(sender);
    assert_eq!(worker.join().unwrap(), 1);
    let actual: Value =
        serde_json::from_slice(&fs::read(directory.join("capture.json")).unwrap()).unwrap();
    assert_eq!(actual, expected);
    assert!(!directory.join(".capture.json.tmp").exists());
    assert!(
        std::panic::catch_unwind(|| publish(&directory, "capture.json", &Value::Null)).is_err()
    );
    let actual: Value =
        serde_json::from_slice(&fs::read(directory.join("capture.json")).unwrap()).unwrap();
    assert_eq!(actual, expected);
    fs::remove_dir_all(directory).unwrap();
}
