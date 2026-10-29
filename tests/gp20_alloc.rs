use gigpies::{
    measurement_capture::State,
    measurement_owner::Config,
    measurement_wire::{Basis, Command, Options},
    show::Counter,
    topology::EngineTopology,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
thread_local! {static ACTIVE:Cell<bool>=const {Cell::new(false)};static EVENTS:Cell<usize>=const {Cell::new(0)};}
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

#[test]
fn reserved_mic_borrowed_offer_never_allocates_or_retires() {
    use sha2::{Digest, Sha256};
    let executable = std::fs::canonicalize("/usr/bin/true").unwrap();
    let hash = format!("{:x}", Sha256::digest(std::fs::read(&executable).unwrap()));
    let mut state = State::new(Config {
        version: 1,
        mode: "software-only".into(),
        owner_executable: executable,
        owner_sha256: hash,
        offline_evidence: vec![],
    })
    .unwrap();
    let mut topology = EngineTopology::software(16, 2, 2).unwrap();
    topology.capture_channels = 17;
    topology.measurement_slots = vec![16];
    let basis = Basis {
        source_epoch: Counter(1),
        map_revision: Counter(1),
        graph_generation: Counter(1),
        clock_domain: "software-common-source".into(),
        configuration_json: r#"{"outputs":[{"source":{"input":0}}]}"#.into(),
        program_buses: vec![0, 1],
    };
    let action = state
        .prepare(
            &Command::CaptureStart {
                id: "a".into(),
                reference_input: "input-01".into(),
                mic_capture_slot: 16,
                output_index: 0,
                position_id: "p".into(),
                samples: 32768,
                options: Options::default(),
            },
            &basis,
            &topology,
            0,
            0,
        )
        .unwrap();
    let auth = gigpies::control_model::Request {
        contract: "C-AUDIO".into(),
        version: 2,
        show_id: "11111111-1111-4111-8111-111111111111".into(),
        module: "audio".into(),
        epoch: Counter(1),
        writer: Some("test".into()),
        lease: Some(Counter(1)),
        request_id: Some(Counter(1)),
        expected_revision: Some(Counter(0)),
        command: gigpies::control_model::Command::Renew {},
    };
    state.apply(action, auth, basis).unwrap();
    let raw = vec![0.01; 48 * 17];
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    for block in 0..684 {
        state.offer(1, 1, block * 48, &raw, true);
    }
    state.offer(2, 2, 0, &raw, false);
    ACTIVE.with(|a| a.set(false));
    assert_eq!(EVENTS.with(Cell::get), 0);
    assert_eq!(state.progress().unwrap().received_samples, 32768);
}
