//! Opt-in, bounded numeric controller diagnostics. Never used inside sample loops.
//! Timestamps are same-process elapsed wall time, not thread CPU or network latency.
#[cfg(any(test, all(target_os = "linux", feature = "hardware-host")))]
use serde::Serialize;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

#[cfg(any(test, all(target_os = "linux", feature = "hardware-host")))]
pub(crate) const SOURCE_CAPACITY: usize = 60_001;
// Four simultaneous Desk scopes plus owner traffic in a bounded 60s trial.
pub(crate) const CONTROL_CAPACITY: usize = 4 * 79_264;
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Token {
    pub session: u64,
    pub ordinal: u64,
    pub request: u64,
    pub kind: u64,
}
impl Token {
    pub fn payload(session: u64, ordinal: u64, value: &Value) -> Self {
        let id = value
            .get("request_id")
            .or_else(|| value.get("context").and_then(|v| v.get("request_id")))
            .or_else(|| value.pointer("/outcome/body/context/request_id"));
        let request = id
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .or_else(|| v.as_u64())
            })
            .unwrap_or(0);
        let kind = match value.get("contract").and_then(Value::as_str) {
            Some("GP15-brain") => 1,
            Some("GP15-device") => 2,
            Some("GP14-mixer") => 3,
            Some("GP14-structure") => 4,
            Some("GP07-processing") => 5,
            Some("GP18-sends") => 8,
            Some("C-AUDIO") => 6,
            Some("GP15-media") => 7,
            _ => 255,
        };
        let operation = match value
            .get("kind")
            .or_else(|| value.get("command").and_then(|v| v.get("kind")))
            .and_then(Value::as_str)
        {
            Some("snapshot") => 1,
            Some("heartbeat") => 2,
            Some("hold") => 3,
            Some("release") => 4,
            Some("grant") => 5,
            Some("renew") => 6,
            Some("query") => 7,
            _ => 255,
        };
        let kind = kind * 256 + operation;
        Self {
            session,
            ordinal,
            request,
            kind,
        }
    }
}
#[repr(u64)]
#[derive(Clone, Copy)]
pub(crate) enum Stage {
    FirstByte = 1,
    Framed = 2,
    Decoded = 3,
    RequestDequeued = 4,
    ProxyEnqueued = 5,
    DispatchBegin = 6,
    DispatchEnd = 7,
    EventEnqueued = 8,
    EventDequeued = 9,
    ReplyEnqueued = 10,
    ReplyDequeued = 11,
    SerializeBegin = 12,
    SerializeEnd = 13,
    PagesEnd = 14,
    WritePoll = 15,
    WritePending = 16,
    WriteEnd = 17,
    QuinnPath = 18,
    QuinnTraffic = 19,
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    SourceTick = 20,
    QuinnMaxData = 21,
    QuinnMaxStreamData = 22,
    QuinnDataBlocked = 23,
    QuinnStreamDataBlocked = 24,
    QuinnUdp = 25,
    QuinnBoundary = 26,
    ResponseIdentity = 27,
    MediaSend = 28,
    MediaReceive = 29,
    MediaAdmission = 30,
    #[cfg(feature = "hardware-host")]
    BrainReceive = 31,
    #[cfg(feature = "hardware-host")]
    BrainProcessBegin = 32,
    #[cfg(feature = "hardware-host")]
    BrainProcessEnd = 33,
    #[cfg(feature = "hardware-host")]
    BrainSend = 34,
    MediaRefused = 35,
    MediaQueueFull = 36,
    #[cfg(feature = "hardware-host")]
    SourceDispatchResources = 38,
}
struct Slot {
    words: [AtomicU64; 8],
}
const _: () = assert!(std::mem::size_of::<Slot>() == 64);
#[cfg(any(test, all(target_os = "linux", feature = "hardware-host")))]
#[derive(Serialize)]
pub(crate) struct Report {
    pub schema: u32,
    pub clock: &'static str,
    pub allocated_bytes: usize,
    pub media_attempts: u64,
    pub media_overflow: u64,
    pub control_attempts: u64,
    pub source_attempts: u64,
    pub control_overflow: u64,
    pub source_overflow: u64,
    pub incomplete: u64,
    pub active_tasks: u64,
    pub append_elapsed_us: u64,
    pub overhead_scope: &'static str,
    pub request_kinds: &'static str,
    pub columns: [&'static str; 8],
    pub stages: &'static str,
    pub records: Vec<[u64; 8]>,
}
pub struct Trace {
    origin: Instant,
    slots: Box<[Slot]>,
    media_slots: Box<[Slot]>,
    media: AtomicU64,
    control: AtomicU64,
    source: AtomicU64,
    active: AtomicU64,
    overhead: AtomicU64,
}
pub type Handle = Option<Arc<Trace>>;
impl Trace {
    #[cfg(test)]
    pub(crate) fn test_trace() -> Arc<Self> {
        Self::with_capacity(128)
    }
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(crate) fn new() -> Arc<Self> {
        Self::with_capacity(CONTROL_CAPACITY + SOURCE_CAPACITY)
    }
    #[cfg(any(test, all(target_os = "linux", feature = "hardware-host")))]
    fn with_capacity(capacity: usize) -> Arc<Self> {
        let trace = Arc::new(Self {
            origin: Instant::now(),
            slots: (0..capacity)
                .map(|_| Slot {
                    words: std::array::from_fn(|_| AtomicU64::new(0)),
                })
                .collect(),
            media_slots: Box::new([]),
            media: AtomicU64::new(0),
            control: AtomicU64::new(0),
            source: AtomicU64::new(0),
            active: AtomicU64::new(0),
            overhead: AtomicU64::new(0),
        });
        // calloc/zero initialization can leave pages backed by the shared zero
        // page. Touch storage before timing, rather than fault on first append.
        for slot in &trace.slots {
            // Still exclusively owned during construction; volatile forces a page write.
            unsafe { std::ptr::write_volatile(slot.words[0].as_ptr(), 0) };
        }
        trace
    }
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    pub(crate) fn with_media() -> Arc<Self> {
        let mut trace = Self::new();
        Arc::get_mut(&mut trace).unwrap().media_slots = (0..480_000)
            .map(|_| Slot {
                words: std::array::from_fn(|_| AtomicU64::new(0)),
            })
            .collect();
        for slot in &trace.media_slots {
            // Still exclusively owned during construction; volatile forces a page write.
            unsafe { std::ptr::write_volatile(slot.words[0].as_ptr(), 0) };
        }
        trace
    }
    pub fn active_tasks(&self) -> u64 {
        self.active.load(Ordering::Acquire)
    }
    pub fn task(self: &Arc<Self>) -> TaskGuard {
        self.active.fetch_add(1, Ordering::Relaxed);
        TaskGuard(self.clone())
    }
    pub fn now(&self) -> u64 {
        self.origin.elapsed().as_micros().min(u64::MAX as u128) as u64
    }
    pub(crate) fn record(&self, token: Token, stage: Stage, a: u64, b: u64) {
        // Diagnostics are admitted only for <=60s software runs. Even one event/ns
        // stays below 2^36; u64 fetch_add cannot wrap in the admitted lifetime.
        let begin = self.now();
        let media = stage as u64 >= 28;
        if media && self.media_slots.is_empty() {
            return;
        }
        let source = stage as u64 == 20;
        let (index, end) = if media {
            (
                self.media.fetch_add(1, Ordering::Relaxed),
                self.media_slots.len() as u64,
            )
        } else if source {
            (
                CONTROL_CAPACITY as u64 + self.source.fetch_add(1, Ordering::Relaxed),
                self.slots.len() as u64,
            )
        } else {
            (
                self.control.fetch_add(1, Ordering::Relaxed),
                CONTROL_CAPACITY.min(self.slots.len()) as u64,
            )
        };
        if index >= end {
            return;
        }
        let slot = if media {
            &self.media_slots[index as usize]
        } else {
            &self.slots[index as usize]
        };
        let words = [
            0,
            self.now(),
            token.session,
            token.ordinal,
            token.request,
            token.kind,
            a,
            b,
        ];
        for (dst, value) in slot.words.iter().zip(words).skip(1) {
            dst.store(value, Ordering::Relaxed);
        }
        slot.words[0].store(stage as u64, Ordering::Release);
        self.overhead
            .fetch_add(self.now().saturating_sub(begin), Ordering::Relaxed);
    }
    #[cfg(any(test, all(target_os = "linux", feature = "hardware-host")))]
    pub(crate) fn report(&self) -> Report {
        let control = self.control.load(Ordering::Relaxed);
        let source = self.source.load(Ordering::Relaxed);
        let mut records = Vec::new();
        let mut incomplete = 0;
        for (index, slot) in self.slots.iter().enumerate() {
            let reserved = if index < CONTROL_CAPACITY {
                (index as u64) < control
            } else {
                ((index - CONTROL_CAPACITY) as u64) < source
            };
            if !reserved {
                continue;
            }
            let stage = slot.words[0].load(Ordering::Acquire);
            if stage == 0 {
                incomplete += 1;
                continue;
            }
            let mut row = std::array::from_fn(|i| slot.words[i].load(Ordering::Relaxed));
            row[0] = stage;
            records.push(row);
        }
        let media = self.media.load(Ordering::Relaxed);
        for slot in self.media_slots.iter().take(media as usize) {
            let stage = slot.words[0].load(Ordering::Acquire);
            if stage == 0 {
                incomplete += 1;
                continue;
            }
            records.push(std::array::from_fn(|i| {
                slot.words[i].load(Ordering::Relaxed)
            }));
        }
        Report {
            media_attempts: media,
            media_overflow: media.saturating_sub(self.media_slots.len() as u64),
            schema: 1,
            clock: "process-local monotonic microseconds; stage elapsed includes scheduling/preemption; not CPU time",
            allocated_bytes: (self.slots.len() + self.media_slots.len()) * 64,
            control_attempts: control,
            source_attempts: source,
            control_overflow: control.saturating_sub(CONTROL_CAPACITY.min(self.slots.len()) as u64),
            source_overflow: source.saturating_sub(SOURCE_CAPACITY as u64),
            incomplete,
            active_tasks: self.active_tasks(),
            append_elapsed_us: self.overhead.load(Ordering::Relaxed),
            overhead_scope: "sum successful append elapsed microseconds only; excludes failed-capacity attempts, token extraction, stats reads and timestamp call overhead; per-event truncation applies",
            request_kinds: "contract*256+operation; contracts 1:GP15-brain 2:GP15-device 3:GP14-mixer 4:GP14-structure 5:GP07-processing 6:C-AUDIO 7:GP15-media 8:GP18-sends 255:other; operations 1:snapshot 2:heartbeat 3:hold 4:release 5:grant 6:renew 7:query 255:other; ordinal0:explicit uncorrelated/deferred completion; request_id0:unavailable",
            columns: [
                "stage",
                "time_us",
                "session",
                "ordinal",
                "request_id_or_zero",
                "kind",
                "a",
                "b",
            ],
            stages: "1:first_byte 2:framed(bytes) 3:decoded 4:request_dequeued 5:proxy_enqueued 6:dispatch_begin 7:dispatch_end 8:event_enqueued 9:event_dequeued 10:reply_enqueued 11:reply_dequeued 12:serialize_begin 13:serialize_end(bytes) 14:pages_end(pages,bytes) 15:write_first_poll 16:write_first_pending 17:write_end(outcome,pending_polls) 18:quinn_path(rtt_us,lost_packets) 19:quinn_traffic(sent_packets,cwnd) 20:source_tick(report_us in session,frame in ordinal,debt_us in request,dispatch_us in kind,render_us,total_tick_us) 21:max_data(rx,tx) 22:max_stream_data(rx,tx) 23:data_blocked(rx,tx) 24:stream_data_blocked(rx,tx) 25:udp_datagrams(rx,tx) 26:quinn_boundary(0:before/1:after,0); connection-wide aggregates cannot identify a stream stall cause; 27:response_identity(revision_or_u64MAX,frame_or_u64MAX); media stages use session,source_frame,output_frame,stream,a=reserved_zero,b=observed_source_or_zero: 28:quinn_send_call 29:quinn_dequeue 30:provider_admission 31:brain_dequeue 32:brain_process_begin 33:brain_process_end 34:brain_send_call 35:media_refused 36:media_queue_full; 37:source_render_resources(frame in ordinal,CPU_us in request,minor_faults in session,voluntary_switches in kind,involuntary_switches in a,major_faults in b), historical inline owner; 38:source_dispatch_resources(same fields, controller thread only; excludes gp-source worker); first channel group only except refusals; send call is not wire departure and dequeue is not socket arrival",
            records,
        }
    }
}
pub(crate) fn record(trace: &Handle, token: Token, stage: Stage, a: u64, b: u64) {
    if let Some(trace) = trace {
        trace.record(token, stage, a, b);
    }
}

pub struct TaskGuard(Arc<Trace>);
impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Release);
    }
}
pub(crate) struct AbortChildren(pub Option<[tokio::task::AbortHandle; 2]>);
impl Drop for AbortChildren {
    fn drop(&mut self) {
        if let Some(handles) = &self.0 {
            for handle in handles {
                handle.abort();
            }
        }
    }
}

pub(crate) fn quinn_stats(
    trace: &Handle,
    token: Token,
    connection: &quinn::Connection,
    after: bool,
) {
    if trace.is_none() {
        return;
    }
    let stats = connection.stats();
    record(trace, token, Stage::QuinnBoundary, u64::from(after), 0);
    record(
        trace,
        token,
        Stage::QuinnPath,
        stats.path.rtt.as_micros() as u64,
        stats.path.lost_packets,
    );
    record(
        trace,
        token,
        Stage::QuinnTraffic,
        stats.path.sent_packets,
        stats.path.cwnd,
    );
    record(
        trace,
        token,
        Stage::QuinnMaxData,
        stats.frame_rx.max_data,
        stats.frame_tx.max_data,
    );
    record(
        trace,
        token,
        Stage::QuinnMaxStreamData,
        stats.frame_rx.max_stream_data,
        stats.frame_tx.max_stream_data,
    );
    record(
        trace,
        token,
        Stage::QuinnDataBlocked,
        stats.frame_rx.data_blocked,
        stats.frame_tx.data_blocked,
    );
    record(
        trace,
        token,
        Stage::QuinnStreamDataBlocked,
        stats.frame_rx.stream_data_blocked,
        stats.frame_tx.stream_data_blocked,
    );
    record(
        trace,
        token,
        Stage::QuinnUdp,
        stats.udp_rx.datagrams,
        stats.udp_tx.datagrams,
    );
}

pub(crate) fn response_identity(trace: &Handle, token: Token, response: &super::Response) {
    if trace.is_none() {
        return;
    }
    if let super::Response::Reply { payload, .. } = response {
        fn number(value: Option<&Value>) -> Option<u64> {
            value.and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| v.as_u64())
            })
        }
        let snapshot = payload.get("snapshot").unwrap_or(payload);
        let revision = number(payload.get("revision"))
            .or_else(|| number(snapshot.get("revision")))
            .or_else(|| number(snapshot.pointer("/authority/revision")));
        let frame = number(snapshot.get("frame")).or_else(|| number(payload.get("applied_frame")));
        record(
            trace,
            token,
            Stage::ResponseIdentity,
            revision.unwrap_or(u64::MAX),
            frame.unwrap_or(u64::MAX),
        );
    }
}

/// Normal events trace the first group; refusals trace all non-analysis groups.
/// Session and source frame join both nodes; the descriptor supplies the epoch.
/// QUIC dequeue is application observation, never a kernel socket arrival time.
pub(crate) fn media_record(
    trace: &Handle,
    stage: Stage,
    packet: crate::transport::Packet<'_>,
    observed: u64,
) {
    let Some(trace) = trace else {
        return;
    };
    if trace.media_slots.is_empty()
        || (packet.spec().first_channel != 0
            && !matches!(stage, Stage::MediaRefused | Stage::MediaQueueFull))
        || packet.spec().role == crate::transport::Role::Analysis
    {
        return;
    }
    trace.record(
        Token {
            session: packet.spec().session,
            ordinal: packet.source_frame(),
            request: packet.output_frame(),
            kind: u64::from(packet.spec().stream),
        },
        stage,
        0,
        observed,
    );
}

#[cfg(test)]
#[test]
fn media_trace_is_opt_in_bounded_and_preserves_source_identity() {
    let mut trace = Trace::test_trace();
    let token = Token {
        session: 7,
        ordinal: 633360,
        request: 634128,
        kind: 9,
    };
    trace.record(token, Stage::MediaAdmission, 0, 634128);
    assert_eq!(trace.report().media_attempts, 0);
    Arc::get_mut(&mut trace).unwrap().media_slots = vec![Slot {
        words: std::array::from_fn(|_| AtomicU64::new(0)),
    }]
    .into_boxed_slice();
    trace.record(token, Stage::MediaAdmission, 0, 634128);
    trace.record(token, Stage::MediaAdmission, 0, 634176);
    let report = trace.report();
    assert_eq!(report.media_attempts, 2);
    assert_eq!(report.media_overflow, 1);
    assert_eq!(report.records.len(), 1);
    assert_eq!(&report.records[0][2..], &[7, 633360, 634128, 9, 0, 634128]);
    assert_eq!(report.incomplete, 0);
}

#[cfg(test)]
pub(crate) fn assert_no_allocations(action: impl FnOnce()) {
    tests::assert_no_allocations(action);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    thread_local! {static WATCH:Cell<bool>=const{Cell::new(false)};static CHANGES:Cell<u64>=const{Cell::new(0)};}
    struct TestAllocator;
    // Test-only forwarding allocator; production tracing contains no unsafe code.
    unsafe impl GlobalAlloc for TestAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            WATCH.with(|w| {
                if w.get() {
                    CHANGES.with(|n| n.set(n.get() + 1))
                }
            });
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
            WATCH.with(|w| {
                if w.get() {
                    CHANGES.with(|n| n.set(n.get() + 1))
                }
            });
            unsafe { System.dealloc(p, layout) }
        }
        unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            WATCH.with(|w| {
                if w.get() {
                    CHANGES.with(|n| n.set(n.get() + 1))
                }
            });
            unsafe { System.realloc(p, layout, size) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: TestAllocator = TestAllocator;
    pub(super) fn assert_no_allocations(action: impl FnOnce()) {
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                WATCH.with(|w| w.set(self.0));
            }
        }
        // Source packets can contain smaller guarded kernels. An inner guard
        // must neither erase the outer count nor disable its remaining scope.
        let before = CHANGES.with(Cell::get);
        let reset = Reset(WATCH.with(|w| w.replace(true)));
        action();
        drop(reset);
        assert_eq!(
            CHANGES.with(Cell::get).saturating_sub(before),
            0,
            "processing allocated or destroyed storage"
        );
    }
    #[test]
    fn nested_render_guards_keep_outer_allocation_detection_enabled() {
        assert!(
            std::panic::catch_unwind(|| {
                assert_no_allocations(|| {
                    assert_no_allocations(|| {});
                    let storage = Box::new(std::hint::black_box(42u64));
                    std::hint::black_box(&storage);
                    drop(storage);
                });
            })
            .is_err()
        );
        assert_no_allocations(|| assert_no_allocations(|| {}));
    }
    #[test]
    fn diagnostic_append_and_overflow_allocate_and_free_nothing() {
        let trace = Trace::test_trace();
        super::assert_no_allocations(|| {
            for ordinal in 1..4096 {
                trace.record(
                    Token {
                        ordinal,
                        ..Token::default()
                    },
                    Stage::Decoded,
                    0,
                    0,
                );
            }
        });
    }

    #[test]
    fn fixed_capacity_and_explicit_overflow() {
        let t = Trace::with_capacity(2);
        for n in 1..=4 {
            t.record(
                Token {
                    ordinal: n,
                    ..Token::default()
                },
                Stage::Decoded,
                0,
                0,
            );
        }
        let r = t.report();
        assert_eq!(r.records.len(), 2);
        assert_eq!(r.control_overflow, 2);
        assert_eq!(r.incomplete, 0);
        assert_eq!(r.records[1][3], 2);
    }
    #[test]
    fn incomplete_reserved_slots_and_task_lifetimes_are_explicit() {
        let trace = Trace::test_trace();
        trace.control.store(1, Ordering::Relaxed);
        assert_eq!(trace.report().incomplete, 1);
        let guard = trace.task();
        assert_eq!(trace.active_tasks(), 1);
        drop(guard);
        assert_eq!(trace.active_tasks(), 0);
        let pointer = trace.slots.as_ptr();
        let capacity = trace.slots.len();
        for n in 0..200 {
            trace.record(
                Token {
                    ordinal: n,
                    ..Token::default()
                },
                Stage::Decoded,
                0,
                0,
            );
        }
        assert_eq!(pointer, trace.slots.as_ptr());
        assert_eq!(capacity, trace.slots.len());
    }
    #[cfg(all(target_os = "linux", feature = "hardware-host"))]
    #[test]
    fn source_overflow_cannot_consume_control_partition() {
        let trace = Trace::new();
        trace
            .source
            .store(SOURCE_CAPACITY as u64, Ordering::Relaxed);
        trace.record(Token::default(), Stage::SourceTick, 0, 0);
        trace.record(
            Token {
                ordinal: 7,
                ..Token::default()
            },
            Stage::Decoded,
            0,
            0,
        );
        let report = trace.report();
        assert_eq!(report.source_overflow, 1);
        assert_eq!(report.control_overflow, 0);
        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0][3], 7);
    }
    #[test]
    fn deferred_completion_never_borrows_query_ordinal() {
        let v = serde_json::json!({"contract":"GP15-brain","context":{"request_id":"42"}});
        let t = Token::payload(7, 0, &v);
        assert_eq!(t.ordinal, 0);
        assert_eq!(t.request, 42);
        assert_eq!(Token::payload(7, 8, &v).ordinal, 8);
    }
}
