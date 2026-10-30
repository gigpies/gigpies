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
fn send_prepared_apply_endpoint_fault_quiescence_and_shape_refusal_have_no_heap_events() {
    use gigpies::{mixer::Prepared, sends_wire::Tap, topology::EngineTopology};
    for path in 0..5 {
        let mut m = Mixer::from_topology(0, EngineTopology::software(17, 3, 0).unwrap()).unwrap();
        m.enable_fx_aux().unwrap();
        m.schedule(
            Prepared::send_for(17, 3, 16, 2, Tap::ProcessedPostFader).unwrap(),
            1,
        )
        .unwrap();
        let input = vec![if path == 2 { f64::NAN } else { 0.5 }; 400 * 17];
        let mut output = vec![0.; 400 * 5];
        EVENTS.with(|n| n.set(0));
        ACTIVE.with(|a| a.set(true));
        for bus in 0..4 {
            m.set_fx_sends(bus, &[0.25; 17]).unwrap();
        }
        let result = match path {
            0 | 2 => m.process_interleaved(&input, &mut output),
            1 => m.process_interleaved(&input, &mut []),
            3 => {
                m.quiesce();
                m.process_interleaved(&input, &mut output)
            }
            _ => {
                m.process_interleaved(&input, &mut output).unwrap();
                m.process_interleaved(&input, &mut output)
            }
        };
        ACTIVE.with(|a| a.set(false));
        assert_eq!(EVENTS.with(Cell::get), 0);
        assert_eq!(result.is_err(), path == 1);
        m.take_completion();
    }
}
