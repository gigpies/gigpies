//! Opt-in capacity evidence: worst-case core mixer render cost per 48-frame block
//! on the machine running it, across input × monitor topologies. Every strip has
//! gate, EQ, compressor and vocal assist engaged on a signal that drives them.
//! This measures the shared DSP core only (strips, sends, buses), not metering,
//! spectrum, PA or FX modules, transport or control work. Run explicitly:
//!
//! cargo +1.97.1 test --release --locked -j1 --test source_capacity -- --ignored --nocapture
use gigpies::channel_processing::{Config, Prepared as Strip};
use gigpies::mixer::{Mixer, Prepared};
use gigpies::topology::EngineTopology;
use std::time::Instant;

const BLOCK: usize = 48;
const PERIOD_US: f64 = 1000.; // 48 frames at 48 kHz
const WARMUP: usize = 500;
const MEASURED: usize = 10_000;

fn worst_case() -> Config {
    Config {
        gate: gigpies::gate::Config {
            bypass: false,
            detector_hz: 180,
            ..Default::default()
        },
        vocal_assist: gigpies::vocal_assist::Config {
            bypass: false,
            reference_mdb: -30000,
            ..Default::default()
        },
        eq_bypass: false,
        band1_gain_mdb: -3000,
        band2_gain_mdb: 4000,
        band3_gain_mdb: 6000,
        band4_gain_mdb: -6000,
        compressor_bypass: false,
        threshold_mdb: -30000,
        ratio_milli: 4000,
        ..Default::default()
    }
}

/// Microsecond quantiles of per-block render time.
fn measure(inputs: usize, monitors: usize) -> [f64; 4] {
    let topology = EngineTopology::software(inputs, monitors, 0).unwrap();
    let outputs = topology.playback_channels;
    let mut m = Mixer::from_topology(0, topology).unwrap();
    m.rearm().unwrap();
    let mut input = vec![0.; BLOCK * inputs];
    let mut output = vec![0.; BLOCK * outputs];
    let mut frame = 0u64;
    let fill = |input: &mut [f64], frame: &mut u64| {
        for f in 0..BLOCK {
            let t = (*frame + f as u64) as f64 / 48_000.;
            for c in 0..inputs {
                // About -12 dBFS, distinct per channel: gates open, compressors act.
                input[f * inputs + c] =
                    0.25 * (std::f64::consts::TAU * (110. + 37. * c as f64) * t).sin();
            }
        }
        *frame += BLOCK as u64;
    };
    for c in 0..inputs {
        let strip =
            Prepared::processing_for(inputs, monitors, c, Strip::new(worst_case()).unwrap())
                .unwrap();
        m.schedule(strip, c as u64 + 1).unwrap();
        let mut committed = false;
        for _ in 0..1000 {
            fill(&mut input, &mut frame);
            m.process_interleaved(&input, &mut output).unwrap();
            if m.take_completion().is_some() {
                committed = true;
                break;
            }
        }
        assert!(committed, "strip {c} did not commit");
        // The strip crossfade must settle before the next one can be scheduled.
        while !m.processing_ready() {
            fill(&mut input, &mut frame);
            m.process_interleaved(&input, &mut output).unwrap();
        }
    }
    for _ in 0..WARMUP {
        fill(&mut input, &mut frame);
        m.process_interleaved(&input, &mut output).unwrap();
    }
    let mut cost = Vec::with_capacity(MEASURED);
    for _ in 0..MEASURED {
        fill(&mut input, &mut frame);
        let start = Instant::now();
        m.process_interleaved(&input, &mut output).unwrap();
        cost.push(start.elapsed().as_secs_f64() * 1e6);
    }
    assert!(output.iter().all(|s| s.is_finite()));
    cost.sort_by(f64::total_cmp);
    let q = |p: f64| cost[((cost.len() - 1) as f64 * p).round() as usize];
    [q(0.5), q(0.99), q(0.999), cost[cost.len() - 1]]
}

#[test]
#[ignore = "opt-in capacity evidence; run in release on the target machine"]
fn worst_case_core_render_cost_per_block() {
    println!("inputs monitors  p50_us  p99_us  p99.9_us  max_us  p99_of_period");
    for (inputs, monitors) in [
        (8, 2),
        (16, 2),
        (16, 6),
        (16, 10),
        (16, 16),
        (24, 10),
        (32, 10),
        (32, 16),
        (48, 10),
        // Largest admitted monitor counts: the 1 MiB snapshot assembly budget
        // refuses 48 x 12 and 64 x 8 before provisioning.
        (48, 11),
        (64, 7),
    ] {
        let [p50, p99, p999, max] = measure(inputs, monitors);
        println!(
            "{inputs:>6} {monitors:>8} {p50:>7.1} {p99:>7.1} {p999:>9.1} {max:>7.1} {:>13.0}%",
            p99 / PERIOD_US * 100.
        );
    }
}
