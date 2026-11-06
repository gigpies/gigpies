use gigpies::{
    fx_aux_wire::{Configuration, Returns, Settings, Tempo},
    fx_saved::Saved,
    topology::EngineTopology,
};
fn saved(n: usize, m: usize) -> Saved {
    let config = Configuration {
        settings: Settings {
            version: 2,
            size: 64,
            algorithm: 1,
            bypass: 0,
            time_ms: 20.,
            amount: 0.25,
            damping: 0.35,
            rate_hz: 0.,
            gain: 0.5,
            seed: 0,
            reserved: 0,
        },
        sends: vec![0.; n],
        returns: Some(Returns::silent_monitors(m)),
        tempo: None,
    };
    Saved {
        format: "gigpies-fx-intent".into(),
        version: 1,
        show_id: "11111111-1111-4111-8111-111111111111".into(),
        library_sha256: gigpies::fx_aux_wire::BUNDLE_LIBRARY.into(),
        inputs: EngineTopology::software(n, m, 0).unwrap().inputs,
        monitors: (1..=m).map(|i| format!("monitor-{i}")).collect(),
        configurations: std::array::from_fn(|_| serde_json::to_string(&config).unwrap()),
    }
}
#[test]
fn strict_saved_payload_and_identity_replacement() {
    for (n, m) in [(1, 0), (4, 3), (17, 5), (32, 1)] {
        let mut s = saved(n, m);
        s.inputs[0].physical_port = format!("Stage Left {}", "x".repeat(80));
        let mut c: Configuration = serde_json::from_str(&s.configurations[0]).unwrap();
        c.tempo = Some(Tempo {
            bpm: 121.,
            denominator: 8,
            modifier: 1,
        });
        c.settings.time_ms = 240000. / 121. / 8. * 1.5;
        c.settings.bypass = 1;
        c.settings.seed = 42;
        c.sends.fill(0.125);
        c.returns = Some(Returns {
            main: 0.75,
            monitors: vec![0.25; m],
        });
        s.configurations[0] = serde_json::to_string(&c).unwrap();
        let bytes = serde_json::to_vec(&s).unwrap();
        assert_eq!(Saved::decode(&bytes).unwrap(), s);
        assert!(
            s.compatible(&s.show_id, &s.library_sha256, &s.inputs, m)
                .is_ok()
        );
        let mut inputs = s.inputs.clone();
        inputs[0].physical_port = "replacement".into();
        assert!(
            s.compatible(&s.show_id, &s.library_sha256, &inputs, m)
                .is_err()
        );
        assert!(
            s.compatible(&s.show_id, &s.library_sha256, &s.inputs, m + 1)
                .is_err()
        );
        for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(Saved::decode(&bytes[..cut]).is_err());
        }
        assert_eq!(s.manifest_module().unwrap().id, "fx-aux");
        let mut v = serde_json::to_value(&s).unwrap();
        v["armed"] = true.into();
        assert!(Saved::decode(&serde_json::to_vec(&v).unwrap()).is_err());
        v.as_object_mut().unwrap().remove("armed");
        v["version"] = 2.into();
        assert!(Saved::decode(&serde_json::to_vec(&v).unwrap()).is_err());
        let text = String::from_utf8(bytes).unwrap().replacen(
            "\"version\":1",
            "\"version\":1,\"version\":1",
            1,
        );
        assert!(Saved::decode(text.as_bytes()).is_err());
    }
    assert!(Saved::decode(br#"{"version":1,"engine":{}}"#).is_err());
    assert!(Saved::decode(&vec![b' '; gigpies::fx_saved::MAX_BYTES + 1]).is_err());
}
#[test]
fn complete_configuration_no_defaults_no_runtime_fields() {
    let mut mismatch = saved(4, 2);
    let mut c: Configuration = serde_json::from_str(&mismatch.configurations[3]).unwrap();
    c.tempo = Some(Tempo {
        bpm: 120.,
        denominator: 4,
        modifier: 0,
    });
    mismatch.configurations[3] = serde_json::to_string(&c).unwrap();
    assert!(
        mismatch.validate().is_err(),
        "fourth aux tempo/time mismatch"
    );
    for field in ["settings", "tempo", "returns", "sends"] {
        let mut s = saved(4, 2);
        let mut c: serde_json::Value = serde_json::from_str(&s.configurations[3]).unwrap();
        c.as_object_mut().unwrap().remove(field);
        s.configurations[3] = c.to_string();
        assert!(s.validate().is_err(), "{field}");
    }
    for port in [String::new(), "x".repeat(129)] {
        let mut s = saved(4, 2);
        s.inputs[0].physical_port = port;
        assert!(s.validate().is_err());
    }
    let mut s = saved(4, 2);
    s.inputs[1] = s.inputs[0].clone();
    assert!(s.validate().is_err());
    let mut s = saved(4, 2);
    s.configurations[2] =
        s.configurations[2].replacen("\"tempo\":null", "\"tempo\":null,\"tempo\":null", 1);
    assert!(s.validate().is_err());
    let mut s = saved(4, 2);
    let mut c: serde_json::Value = serde_json::from_str(&s.configurations[1]).unwrap();
    c["lease"] = "1".into();
    s.configurations[1] = c.to_string();
    assert!(s.validate().is_err());
}
