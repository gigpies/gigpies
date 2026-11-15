use gigpies::analysis_stream::*;
use gigpies::transport::Packet;
#[test]
fn exact_e09_window_and_mapping() {
    let d = Descriptor::new(9, 48000);
    let (mut tap, mut input) = Tap::new(&d).unwrap();
    for i in 0..10 {
        let frame = 48000 + i * 48;
        tap.offer(frame, &synthetic_inputs(frame), 0).unwrap();
    }
    let w = input.pop().unwrap();
    w.validate(&d, 48000, 0).unwrap();
    for (i, bytes) in w.packets.chunks_exact(PACKET_BYTES).enumerate() {
        let p = Packet::parse(bytes).unwrap();
        let raw = synthetic_inputs(48000 + i as u64 * 48);
        for (f, values) in raw.iter().enumerate() {
            for (c, &value) in values[..4].iter().enumerate() {
                assert_eq!(p.pcm(f * 4 + c).unwrap(), value);
            }
        }
    }
    let wire = w.wire(3, 1, 2);
    assert_eq!(&wire[..4], b"GAW1");
    assert_eq!(u64::from_be_bytes(wire[4..12].try_into().unwrap()), 3);
    assert!(w.validate(&d, 48048, 0).is_err());
    assert!(w.validate(&d, 48000, 1).is_err());
    let mut changed = d.clone();
    changed.source_epoch.0 = 10;
    assert!(w.validate(&changed, 48000, 0).is_err());
    let mut reversed = w;
    reversed.packets[..PACKET_BYTES * 2].rotate_left(PACKET_BYTES);
    assert!(reversed.validate(&d, 48000, 0).is_err());
}
#[test]
fn descriptor_refuses_rate_and_implicit_sources() {
    let mut d = Descriptor::new(9, 48000);
    d.sample_rate = 44100;
    assert!(Tap::new(&d).is_err());
    d.sample_rate = 48000;
    d.sources.swap(0, 1);
    assert!(Tap::new(&d).is_err());
    d.sources.swap(0, 1);
    d.inputs[1] = d.inputs[0].clone();
    assert!(Tap::new(&d).is_err());
}
#[test]
fn staging_drops_whole_windows_and_never_waits() {
    let d = Descriptor::new(9, 48000);
    let (mut tap, mut input) = Tap::new(&d).unwrap();
    for i in 0..40 {
        let f = 48000 + i * 48;
        tap.offer(f, &synthetic_inputs(f), 0).unwrap();
    }
    assert_eq!(tap.dropped_windows, 2);
    input.pop().unwrap().validate(&d, 48000, 0).unwrap();
    input.pop().unwrap().validate(&d, 48480, 10).unwrap();
    assert!(input.pop().is_err());
    assert!(tap.offer(0, &synthetic_inputs(0), 0).is_err());
    for i in 40..50 {
        let f = 48000 + i * 48;
        tap.offer(f, &synthetic_inputs(f), 0).unwrap();
    }
    input.pop().unwrap().validate(&d, 49920, 40).unwrap();
}
#[test]
fn explicit_alternate_mapping() {
    let mut d = Descriptor::new(9, 0);
    d.inputs = ["input-08", "input-06", "input-04", "input-02"]
        .map(String::from)
        .to_vec();
    let (mut tap, mut input) = Tap::new(&d).unwrap();
    for i in 0..10 {
        tap.offer(i * 48, &synthetic_inputs(i * 48), 0).unwrap();
    }
    let w = input.pop().unwrap();
    let p = Packet::parse(&w.packets[..PACKET_BYTES]).unwrap();
    assert_eq!(p.pcm(0).unwrap(), synthetic_inputs(0)[0][7]);
}
#[test]
fn attachment_rejects_changes_gaps_and_stale_buffered_audio() {
    let d = Descriptor::new(9, 48000);
    let (mut tap, mut q) = Tap::new(&d).unwrap();
    for i in 0..20 {
        let f = 48000 + i * 48;
        tap.offer(f, &synthetic_inputs(f), 500).unwrap();
    }
    let a = q.pop().unwrap().wire(0, 1, 2);
    let b = q.pop().unwrap().wire(0, 1, 2);
    let mut cursor = WindowCursor::new(d.clone()).unwrap();
    cursor.accept(&a, 550).unwrap();
    cursor.accept(&b, 550).unwrap();
    assert!(cursor.accept(&a, 550).is_err());
    for field in [12, 20, 28, 36] {
        let mut bad = a.clone();
        bad[field] ^= 1;
        assert!(
            WindowCursor::new(d.clone())
                .unwrap()
                .accept(&bad, 550)
                .is_err()
        );
    }
    assert!(
        WindowCursor::new(d.clone())
            .unwrap()
            .accept(&a, 601)
            .is_err()
    );
    assert!(
        WindowCursor::new(d.clone())
            .unwrap()
            .accept(&a, 499)
            .is_err()
    );
    let mut cursor = WindowCursor::new(d).unwrap();
    cursor.accept(&a, 500).unwrap();
    let mut lost = b.clone();
    lost[11] = 1;
    assert!(cursor.accept(&lost, 500).is_err());
    assert!(cursor.accept(&b, 500).is_err());
}
#[test]
fn exact_published_e09_corpus() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/gp04/v1/e09.json")).unwrap();
    let d: Descriptor = serde_json::from_value(corpus["descriptor"].clone()).unwrap();
    let decode = |s: &str| {
        s.as_bytes()
            .chunks_exact(2)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    };
    for case in corpus["wire_cases"].as_array().unwrap() {
        let mut cursor = WindowCursor::new(d.clone()).unwrap();
        if let Some(prior) = case["prior_hex"].as_str() {
            cursor.accept(&decode(prior), 1000).unwrap();
        }
        let result = cursor.accept(
            &decode(case["hex"].as_str().unwrap()),
            case["now_ms"].as_u64().unwrap(),
        );
        assert_eq!(
            result.is_ok(),
            case["accept"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}
#[test]
fn first_packet_age_survives_long_partial_window_pause() {
    let d = Descriptor::new(9, 0);
    let (mut tap, mut input) = Tap::new(&d).unwrap();
    tap.offer(0, &synthetic_inputs(0), 100).unwrap();
    for i in 1..10 {
        tap.offer(i * 48, &synthetic_inputs(i * 48), 500).unwrap();
    }
    let w = input.pop().unwrap();
    assert_eq!(w.produced_mono_ms, 100);
    assert!(
        WindowCursor::new(d)
            .unwrap()
            .accept(&w.wire(0, 1, 2), 500)
            .is_err()
    );
    assert!(tap.offer(480, &synthetic_inputs(480), 499).is_err());
}
#[test]
fn receipt_and_capture_clock_regressions_are_terminal() {
    let d = Descriptor::new(9, 0);
    let (mut tap, mut q) = Tap::new(&d).unwrap();
    for i in 0..20 {
        tap.offer(i * 48, &synthetic_inputs(i * 48), 500 + i)
            .unwrap();
    }
    let a = q.pop().unwrap().wire(0, 1, 2);
    let mut b = q.pop().unwrap().wire(0, 1, 2);
    let mut cursor = WindowCursor::new(d.clone()).unwrap();
    cursor.accept(&a, 550).unwrap();
    assert!(cursor.accept(&b, 549).is_err());
    b[28..36].copy_from_slice(&499u64.to_be_bytes());
    let mut cursor = WindowCursor::new(d).unwrap();
    cursor.accept(&a, 550).unwrap();
    assert!(cursor.accept(&b, 550).is_err());
}
