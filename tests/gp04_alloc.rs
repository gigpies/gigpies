use gigpies::analysis_stream::{Descriptor, Tap, synthetic_inputs};
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
fn raw_tap_full_queue_and_errors_do_not_allocate_or_retire() {
    let (mut tap, _q) = Tap::new(&Descriptor::new(9, 0)).unwrap();
    let raw = synthetic_inputs(0);
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    for i in 0..100 {
        assert!(tap.offer(i * 48, &raw, 0).is_ok());
    }
    let error = tap.offer(0, &raw, 0);
    ACTIVE.with(|a| a.set(false));
    assert!(error.is_err());
    assert_eq!(tap.dropped_windows, 8);
    assert_eq!(EVENTS.with(Cell::get), 0);
}

#[test]
fn configured_tap_retains_allocation_free_bounded_publication() {
    let topology = gigpies::topology::EngineTopology::software(17, 3, 0).unwrap();
    let mapping = gigpies::analysis_stream::Mapping {
        version: 2,
        sources: [
            "kick", "bass", "guitar-1", "guitar-2", "vocal", "synth", "backs",
        ]
        .map(String::from)
        .to_vec(),
        inputs: [
            "input-17", "input-03", "input-09", "input-01", "input-02", "input-04", "input-05",
        ]
        .map(String::from)
        .to_vec(),
    };
    let descriptor = mapping.descriptor(&topology, 9, 0).unwrap();
    let (mut tap, _q) = Tap::new(&descriptor).unwrap();
    let raw = vec![42; 17 * 48];
    EVENTS.with(|n| n.set(0));
    ACTIVE.with(|a| a.set(true));
    for i in 0..100 {
        assert!(tap.offer_interleaved(i * 48, &raw, 17, i).is_ok());
    }
    let error = tap.offer_interleaved(0, &raw, 17, 100);
    ACTIVE.with(|a| a.set(false));
    assert!(error.is_err());
    assert_eq!(EVENTS.with(Cell::get), 0);
}
