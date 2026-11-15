use gigpies::{
    analysis_stream::{Descriptor, Mapping, PACKET_BYTES, Tap},
    topology::EngineTopology,
    transport::Packet,
};

fn mapping() -> Mapping {
    Mapping {
        version: 1,
        sources: vec![],
        inputs: ["input-17", "input-03", "input-09", "input-01"]
            .map(String::from)
            .to_vec(),
    }
}

#[test]
fn configured_mapping_is_admitted_against_real_inventory_and_preserves_v1() {
    let topology = EngineTopology::software(17, 3, 0).unwrap();
    assert_eq!(
        serde_json::to_value(&topology).unwrap(),
        serde_json::from_str::<serde_json::Value>(include_str!("fixtures/gp04/v2/topology.json"))
            .unwrap()
    );
    let d = mapping().descriptor(&topology, 9, 0).unwrap();
    assert_eq!(d.version, 2);
    assert_eq!(d.map_revision.0, topology.map_revision);
    assert_eq!(
        serde_json::to_value(&d).unwrap(),
        serde_json::from_str::<serde_json::Value>(include_str!(
            "fixtures/gp04/v2/configured-descriptor.json"
        ))
        .unwrap()
    );
    assert_ne!(d.attach_request(), Descriptor::new(9, 0).attach_request());
    assert!(
        mapping()
            .descriptor(&EngineTopology::legacy(), 9, 0)
            .is_err()
    );
    let mut legacy = d.clone();
    legacy.version = 1;
    legacy.subscription = "lux.aux.v1".into();
    assert!(legacy.validate().is_err());
    for invalid in [
        "input-001",
        "input-0",
        "input-65536",
        "input-+1",
        "input-1 ",
        "input-18",
        "input-03",
    ] {
        let mut m = mapping();
        m.inputs[0] = invalid.into();
        assert!(m.descriptor(&topology, 9, 0).is_err(), "{invalid}");
    }
    assert!(
        serde_json::from_str::<Mapping>(
            r#"{"version":1,"version":1,"inputs":["input-01","input-02","input-03","input-04"]}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<Mapping>(
            r#"{"version":1,"inputs":["input-01","input-02","input-03","input-04"],"other":true}"#
        )
        .is_err()
    );
}

#[test]
fn configured_tap_proves_every_selected_raw_sample_across_supported_sizes() {
    for count in [17, 32, 48] {
        let topology = EngineTopology::software(count, 3, 0).unwrap();
        let d = mapping().descriptor(&topology, 9, 0).unwrap();
        let (mut tap, mut receiver) = Tap::new(&d).unwrap();
        for block in 0..10 {
            let raw: Vec<i32> = (0..count * 48)
                .map(|i| ((block * 48 + i / count) * 100 + i % count) as i32 - 30000)
                .collect();
            tap.offer_interleaved(block as u64 * 48, &raw, count, block as u64)
                .unwrap();
        }
        let window = receiver.pop().unwrap();
        window.validate(&d, 0, 0).unwrap();
        for bytes in window.packets.chunks_exact(PACKET_BYTES) {
            let packet = Packet::parse(bytes).unwrap();
            for frame in 0..48 {
                for (channel, index) in [16, 2, 8, 0].iter().enumerate() {
                    let expected =
                        ((packet.source_frame() as usize + frame) * 100 + index) as i32 - 30000;
                    assert_eq!(packet.pcm(frame * 4 + channel).unwrap(), expected);
                }
            }
        }
    }
}

#[test]
fn seven_named_feeds_preserve_exact_pcm_and_reject_over_budget_or_duplicate_maps() {
    let topology = EngineTopology::software(16, 3, 0).unwrap();
    let mut mapping = Mapping {
        version: 2,
        inputs: (1..=7).map(|n| format!("input-{n:02}")).collect(),
        sources: [
            "kick", "bass", "guitar-1", "guitar-2", "vocal", "synth", "backs",
        ]
        .map(String::from)
        .to_vec(),
    };
    let d = mapping.descriptor(&topology, 9, 0).unwrap();
    assert_eq!(d.version, 3);
    let (mut tap, mut queue) = Tap::new(&d).unwrap();
    for frame in (0..480).step_by(48) {
        let raw: Vec<_> = (0..48)
            .flat_map(|f| (0..16).map(move |c| (frame + f) * 100 + c))
            .collect();
        tap.offer_interleaved(frame as u64, &raw, 16, 1000).unwrap();
    }
    let w = queue.pop().unwrap();
    w.validate(&d, 0, 0).unwrap();
    let wire = w.wire(0, d.map_revision.0, d.calibration_revision.0);
    let mut cursor = gigpies::analysis_stream::WindowCursor::new(d.clone()).unwrap();
    cursor.accept(&wire, 1000).unwrap();
    for (n, bytes) in w.packets.chunks_exact(d.spec().packet_bytes()).enumerate() {
        let packet = Packet::parse(bytes).unwrap();
        for frame in 0..48 {
            for c in 0..7 {
                assert_eq!(
                    packet.pcm(frame * 7 + c).unwrap(),
                    ((n * 48 + frame) * 100 + c) as i32
                );
            }
        }
    }
    mapping.inputs[6] = mapping.inputs[0].clone();
    assert!(mapping.descriptor(&topology, 9, 0).is_err());
    mapping.inputs = (1..=9).map(|n| format!("input-{n:02}")).collect();
    mapping.sources.extend(["extra-8".into(), "extra-9".into()]);
    assert!(mapping.descriptor(&topology, 9, 0).is_err());
}
