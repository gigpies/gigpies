use gigpies::mixer::Mixer;
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
fn public_process_has_no_heap_allocation_or_retirement() {
    let mut m = Mixer::default();
    let config = gigpies::channel_processing::Config {
        gate: gigpies::gate::Config {
            bypass: false,
            detector_hz: 180,
            ..Default::default()
        },
        eq_bypass: false,
        band1_gain_mdb: -3000,
        band2_gain_mdb: 6000,
        band3_gain_mdb: 12000,
        band4_gain_mdb: -12000,
        compressor_bypass: false,
        ratio_milli: 4000,
        ..Default::default()
    };
    let prepared = gigpies::mixer::Prepared::processing(
        0,
        gigpies::channel_processing::Prepared::new(config).unwrap(),
    )
    .unwrap();
    m.schedule(prepared, 1).unwrap();
    let input = [[0.5; 8]; 1024];
    let mut output = [[0.0; 4]; 1024];
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    let result = m.process(&input, &mut output);
    ACTIVE.with(|a| a.set(false));
    assert!(result.is_ok());
    assert_eq!(EVENTS.with(Cell::get), 0);
}

#[test]
fn process_error_and_fault_paths_do_not_allocate_or_deallocate() {
    for kind in 0..3 {
        let mut m = if kind == 1 {
            Mixer::new(u64::MAX)
        } else {
            Mixer::default()
        };
        let input = [[f64::NAN; 8]; 1];
        let mut output = [[0.0; 4]; 1];
        EVENTS.with(|n| n.set(0));
        ACTIVE.with(|a| a.set(true));
        let result = if kind == 0 {
            m.process(&input, &mut [])
        } else {
            m.process(&input, &mut output)
        };
        ACTIVE.with(|a| a.set(false));
        assert_eq!(EVENTS.with(Cell::get), 0);
        assert_eq!(result.is_err(), kind != 2);
    }
}

#[test]
fn configurable_profiles_have_no_render_allocation_or_retirement() {
    for n in [16, 17, 32, 48] {
        let mut m = Mixer::from_topology(
            0,
            gigpies::topology::EngineTopology::software(n, 5, 0).unwrap(),
        )
        .unwrap();
        let config = gigpies::channel_processing::Config {
            gate: gigpies::gate::Config {
                bypass: false,
                detector_hz: 180,
                ..Default::default()
            },
            eq_bypass: false,
            band4_gain_mdb: 6000,
            compressor_bypass: false,
            ratio_milli: 4000,
            ..Default::default()
        };
        m.schedule(
            gigpies::mixer::Prepared::processing_for(
                n,
                5,
                n - 1,
                gigpies::channel_processing::Prepared::new(config).unwrap(),
            )
            .unwrap(),
            1,
        )
        .unwrap();
        let input = vec![0.5; 1024 * n];
        let mut output = vec![0.; 1024 * 7];
        EVENTS.with(|n| n.set(0));
        ACTIVE.with(|a| a.set(true));
        let result = m.process_interleaved(&input, &mut output);
        ACTIVE.with(|a| a.set(false));
        assert!(result.is_ok());
        assert_eq!(EVENTS.with(Cell::get), 0);
        m.take_completion();
    }
}

#[test]
fn expanded_raw_analysis_queue_is_bounded_and_allocation_free() {
    use gigpies::analysis_stream::{RawDescriptor, RawTap};
    let descriptor = RawDescriptor::new(
        14,
        0,
        1,
        (1..=48).map(|i| format!("input-{i:02}")).collect(),
    )
    .unwrap();
    let (mut tap, _consumer) = RawTap::new(&descriptor, 48).unwrap();
    let raw = vec![123; 48 * 48];
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    let a = tap.offer(14, 0, &raw, 48);
    let b = tap.offer(14, 48, &raw, 48);
    let c = tap.offer(14, 96, &raw, 48);
    ACTIVE.with(|a| a.set(false));
    assert!(a.is_ok() && b.is_ok() && c.is_ok());
    assert_eq!(tap.dropped_blocks, 1);
    assert_eq!(EVENTS.with(Cell::get), 0);
}

#[test]
fn output_patch_swap_and_pending_fault_retirement_do_not_allocate() {
    let topology = gigpies::topology::EngineTopology::software(16, 3, 0).unwrap();
    let mut mixer = Mixer::from_topology(0, topology.clone()).unwrap();
    let mut outputs = topology.outputs.clone();
    outputs[0].source = None;
    let mut prepared = mixer.prepare_output_patch(outputs).unwrap();
    mixer.quiesce();
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    let applied = mixer.apply_output_patch(&mut prepared);
    ACTIVE.with(|a| a.set(false));
    assert!(applied.is_ok());
    assert_eq!(EVENTS.with(Cell::get), 0);
    mixer.rearm().unwrap();
    mixer
        .schedule(
            gigpies::mixer::Prepared::edits_for(
                16,
                3,
                &[gigpies::control_model::Edit {
                    target: gigpies::control_model::Target::Mute {
                        input: "input-16".into(),
                    },
                    value: gigpies::control_model::Value::Boolean(true),
                }],
            )
            .unwrap(),
            1,
        )
        .unwrap();
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    mixer.quiesce();
    ACTIVE.with(|a| a.set(false));
    assert_eq!(EVENTS.with(Cell::get), 0);
    mixer.take_completion();
}
