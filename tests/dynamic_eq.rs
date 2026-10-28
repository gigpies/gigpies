use gigpies::automix::{
    config,
    dsp::Strip,
    dynamic_eq::{Config, Processor},
};
use std::f64::consts::TAU;
fn config(hz: f64) -> Config {
    Config {
        version: 1,
        hz,
        q: 1.,
        depth_db: 6.,
        threshold_dbfs: -30.,
        attack_ms: 2.,
        release_ms: 40.,
    }
}
#[test]
fn exact_bypass_reset_partition_and_stereo_linking() {
    let mut c = config(6000.);
    c.depth_db = 0.;
    let mut bypass = Processor::new(&c, 48000);
    for n in 0..10000 {
        let x = [(n as f64).sin() * 0.1, -0.03];
        assert_eq!(bypass.tick(x), x);
    }
    c.depth_db = 6.;
    let input = (0..48000)
        .map(|n| {
            let x = 0.1 * (TAU * 6000. * n as f64 / 48000.).sin();
            [x, 0.3 * x]
        })
        .collect::<Vec<_>>();
    let mut p = Processor::new(&c, 48000);
    let expected = input.iter().map(|&x| p.tick(x)).collect::<Vec<_>>();
    for size in [1, 17, 127, 1024] {
        p.reset();
        let mut actual = Vec::new();
        for block in input.chunks(size) {
            for &x in block {
                actual.push(p.tick(x));
            }
        }
        assert_eq!(actual, expected);
    }
    for x in &expected {
        assert!((x[1] - 0.3 * x[0]).abs() < 1e-14);
    }
    p.tick([f64::NAN, 1.]);
    let mut fresh = Processor::new(&c, 48000);
    for &x in &input[..100] {
        assert_eq!(p.tick(x), fresh.tick(x));
    }
}
#[test]
fn selective_bursts_preserve_quiet_content_and_release_without_overshoot() {
    for hz in [60., 1000., 6000.] {
        let c = config(hz);
        c.validate(48000).unwrap();
        let mut p = Processor::new(&c, 48000);
        let mut active = [0.; 2];
        let mut quiet = [0.; 2];
        let mut previous = 0.;
        let mut max_step = 0_f64;
        for n in 0..144000 {
            let t = n as f64 / 48000.;
            let amplitude = if (0.5..1.).contains(&t) { 0.2 } else { 0.001 };
            let x = amplitude * (TAU * hz * t).sin();
            let y = p.tick([x, x])[0];
            if (0.7..0.95).contains(&t) {
                active[0] += x * x;
                active[1] += y * y;
            }
            if t > 2. {
                quiet[0] += x * x;
                quiet[1] += y * y;
            }
            max_step = max_step.max((p.amount() - previous).abs());
            previous = p.amount();
        }
        let attenuation = 10. * (active[1] / active[0]).log10();
        assert!(
            attenuation < -4. && attenuation > -6.1,
            "{hz}: {attenuation}"
        );
        assert!((10. * (quiet[1] / quiet[0]).log10()).abs() < 0.01);
        assert!(max_step < 0.011);
        assert!(p.amount() < 1e-8);
    }
}
#[test]
fn strip_uses_same_processor_and_old_json_defaults_remain_unchanged() {
    let mut s = config::example();
    let channel = &mut s.channels[0];
    channel.eq.clear();
    channel.hpf_hz = 0.;
    channel.compressor.ratio = 1.;
    channel.compressor.makeup_db = 0.;
    let legacy = serde_json::to_value(&channel).unwrap();
    assert!(legacy.get("dynamic_eq").is_none());
    let restored: config::Channel = serde_json::from_value(legacy).unwrap();
    assert!(restored.dynamic_eq.is_none());
    let c = config(6000.);
    channel.dynamic_eq = Some(c.clone());
    let mut strip = Strip::new(channel, 48000);
    let mut p = Processor::new(&c, 48000);
    for n in 0..48000 {
        let x = [0.1 * (TAU * 6000. * n as f64 / 48000.).sin(); 2];
        assert_eq!(strip.tick(x), p.tick(x));
    }
}
#[test]
fn invalid_controls_are_rejected() {
    let mut c = config(6000.);
    for depth in [-1., 6.1, f64::NAN, f64::INFINITY] {
        c.depth_db = depth;
        assert!(c.validate(48000).is_err());
    }
    c = config(6000.);
    assert!(c.validate(8000).is_err());
}

#[test]
fn stable_cut_has_no_additional_harmonics_and_native_rates_stay_finite() {
    use gigpies::automix::{
        config::{EqBand, EqKind},
        dsp::Biquad,
    };
    for rate in [8000, 44101, 48000, 192000] {
        let hz = rate as f64 * 0.125;
        let mut c = config(hz);
        c.threshold_dbfs = -60.;
        c.validate(rate).unwrap();
        let mut dynamic = Processor::new(&c, rate);
        let mut linear = Biquad::equalizer(
            &EqBand {
                kind: EqKind::Bell,
                hz,
                q: c.q,
                db: -c.depth_db,
            },
            rate,
        );
        let mut error = 0.;
        let mut energy = 0.;
        for n in 0..rate * 2 {
            let x = 0.2 * (TAU * hz * n as f64 / rate as f64).sin();
            let y = dynamic.tick([x, x])[0];
            let reference = linear.tick(x);
            assert!(y.is_finite());
            if n > rate {
                error += (y - reference).powi(2);
                energy += reference.powi(2);
            }
        }
        assert!(error / energy < 1e-20);
    }
}

#[test]
fn brief_high_band_events_reduce_without_ducking_low_consonant_body() {
    let mut c = config(6000.);
    c.threshold_dbfs = -45.;
    let mut p = Processor::new(&c, 48000);
    let mut high = [0.; 2];
    let mut body = [0.; 2];
    // A 50 ms high-band burst, with separate low-band recovery evidence.
    for n in 0..48000 {
        let t = n as f64 / 48000.;
        let x = if (0.2..0.25).contains(&t) {
            0.1 * (TAU * 6000. * t).sin()
        } else {
            0.01 * (TAU * 220. * t).sin()
        };
        let y = p.tick([x, x])[0];
        if (0.215..0.245).contains(&t) {
            high[0] += x * x;
            high[1] += y * y;
        }
        if t > 0.6 {
            body[0] += x * x;
            body[1] += y * y;
        }
    }
    assert!(10. * (high[1] / high[0]).log10() < -4.);
    assert!((10. * (body[1] / body[0]).log10()).abs() < 0.01);
}
