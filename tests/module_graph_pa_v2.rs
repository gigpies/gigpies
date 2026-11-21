#![cfg(feature = "hardware-host")]
//! Actual independently built owner acceptance, opt-in because libraries are
//! trusted external artifacts. Run with GP05_MANIFEST and GP14_PA_FIXTURES.
use gigpies::{
    host::pa_v2::Pa,
    module_graph::{Manifest, ModuleGraph},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    path::{Path, PathBuf},
};
thread_local! {static GUARD:Cell<Option<usize>>=const {Cell::new(None)};}
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        GUARD.with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1));
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        GUARD.with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1));
            }
        });
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn setup() -> (Manifest, PathBuf) {
    (
        Manifest::load(Path::new(
            &std::env::var("GP05_MANIFEST").expect("explicit owner manifest"),
        ))
        .unwrap(),
        PathBuf::from(
            std::env::var("GP14_PA_FIXTURES").expect("explicit validated owner fixture directory"),
        ),
    )
}
fn read(root: &Path, name: &str) -> Vec<u8> {
    std::fs::read(root.join(format!("{name}.json"))).unwrap()
}
#[test]
#[ignore = "explicit trusted owner library/header/manifest and provider fixture directory required"]
fn real_matrix_program_buses_wet_summing_retirement_and_recovery() {
    let (manifest, fixtures) = setup();
    let json = read(&fixtures, "matrix4x8");
    let next_json = read(&fixtures, "stereo3way");
    for map in [vec![0, 1, 2, 3], vec![3, 0, 4, 1]] {
        let mut graph = ModuleGraph::load_configured(manifest.clone(), 7, 0, 16).unwrap();
        assert!(graph.prepare_pa_change(&json, vec![0, 1, 2, 6], 6).is_err());
        assert!(graph.prepare_pa_change(&json, vec![0, 1], 6).is_err());
        let mut prepared = graph.prepare_pa_change(&json, map.clone(), 6).unwrap();
        assert_eq!(prepared.output_channels(), 8);
        assert_eq!(graph.commit_pa_change(&mut prepared, 7, 0), 0);
        let mut reference = Pa::load(&manifest.pa.library, &json, 7, 0).unwrap();
        let raw = vec![0.; 16 * 48];
        let mut mix = vec![0.; 6 * 48];
        let mut wet = vec![0.; 2 * 48];
        let mut inputs = vec![0.; 4 * 48];
        let mut expected = vec![0.; 8 * 48];
        for block in 0..6 {
            let frame = block * 48;
            graph
                .process_interleaved(7, frame, &raw, &mix, 6, Some(&wet))
                .unwrap();
            assert_eq!(reference.process(&inputs, &mut expected, 7, frame), 0);
            assert!(graph.output_interleaved().iter().all(|x| *x == 0.));
        }
        graph.rearm_pa().unwrap();
        assert_eq!(reference.rearm(7, 288), 0);
        for block in 6..30 {
            for i in 0..48 {
                for bus in 0..6 {
                    mix[i * 6 + bus] = 0.01
                        * (bus + 1) as f64
                        * ((block * 48 + i + bus * 37) as f64 * 0.017).sin();
                }
                wet[i * 2] = 0.025;
                wet[i * 2 + 1] = -0.0125;
                for (channel, &bus) in map.iter().enumerate() {
                    inputs[i * 4 + channel] =
                        mix[i * 6 + bus] + if bus < 2 { wet[i * 2 + bus] } else { 0. };
                }
            }
            graph
                .process_interleaved(7, block as u64 * 48, &raw, &mix, 6, Some(&wet))
                .unwrap();
            assert_eq!(
                reference.process(&inputs, &mut expected, 7, block as u64 * 48),
                0
            );
            assert_eq!(graph.output_interleaved(), expected);
        }
        let status = graph
            .status("11111111-1111-4111-8111-111111111111", 0)
            .unwrap();
        assert_eq!(status.version, 2);
        assert!(status.pa.is_none());
        assert_eq!(status.pa_v2.unwrap().program_buses, map);
        let mut next = graph.prepare_pa_change(&next_json, vec![0, 1], 6).unwrap();
        let mut stale = graph.prepare_pa_change(&next_json, vec![0, 1], 6).unwrap();
        assert_eq!(graph.commit_pa_change(&mut next, 7, 1440), -3);
        assert_eq!(graph.output_interleaved().len(), 384);
        graph.mute_pa().unwrap();
        for block in 30..36 {
            graph
                .process_interleaved(7, block * 48, &raw, &mix, 6, Some(&wet))
                .unwrap();
        }
        assert_eq!(graph.pa_v2_status().unwrap().unwrap().quiesced, 1);
        assert_eq!(graph.commit_pa_change(&mut next, 7, 1728), 0);
        assert_eq!(graph.commit_pa_change(&mut stale, 7, 1728), -3);
        graph.retire_pa_changes();
        assert_eq!(graph.commit_pa_change(&mut stale, 7, 1728), -4);
        assert_eq!(graph.pa_configuration_json().unwrap().as_bytes(), next_json);
        assert_eq!(graph.pa_program_buses(), [0, 1]);
        assert_eq!(graph.output_interleaved().len(), 288);
        graph
            .process_interleaved(7, 1728, &raw, &mix, 6, Some(&wet))
            .unwrap();
        assert!(graph.output_interleaved().iter().all(|x| *x == 0.));
        graph.quiesce_source().unwrap();
        assert!(graph.rearm_pa().is_err());
        assert!(graph.discontinuity(7, 0).is_err());
        graph.discontinuity(8, 0).unwrap();
        assert_eq!(graph.pa_v2_status().unwrap().unwrap().muted, 1);
        assert_eq!(graph.pa_program_buses(), [0, 1]);
        graph
            .process_interleaved(8, 0, &raw, &mix, 6, Some(&wet))
            .unwrap();
        assert!(graph.output_interleaved().iter().all(|x| *x == 0.));
        graph.rearm_pa().unwrap();
    }
}
#[test]
#[ignore = "explicit trusted owner libraries; host guard complements owner allocation guard"]
fn initial_and_replacement_boundaries_retain_all_heap_owners_without_allocations() {
    let (manifest, fixtures) = setup();
    let json = read(&fixtures, "matrix4x8");
    let next_json = read(&fixtures, "stereo3way");
    for inputs in [16, 32, 48] {
        let mut graph = ModuleGraph::load_configured(manifest.clone(), 7, 0, inputs).unwrap();
        let mut prepared = graph.prepare_pa_change(&json, vec![0, 1, 2, 3], 6).unwrap();
        let raw = vec![0.; inputs * 48];
        let mix = vec![0.05; 6 * 48];
        let wet = vec![0.02; 2 * 48];
        GUARD.with(|c| c.set(Some(0)));
        assert_eq!(graph.commit_pa_change(&mut prepared, 7, 0), 0);
        graph.rearm_pa().unwrap();
        for block in 0..6 {
            graph
                .process_interleaved(7, block * 48, &raw, &mix, 6, Some(&wet))
                .unwrap();
        }
        graph.mute_pa().unwrap();
        for block in 6..12 {
            graph
                .process_interleaved(7, block * 48, &raw, &mix, 6, Some(&wet))
                .unwrap();
        }
        let events = GUARD.with(|c| c.replace(None).unwrap());
        assert_eq!(events, 0);
        let mut next = graph.prepare_pa_change(&next_json, vec![0, 1], 6).unwrap();
        GUARD.with(|c| c.set(Some(0)));
        assert_eq!(graph.commit_pa_change(&mut next, 7, 576), 0);
        assert_eq!(graph.commit_pa_change(&mut next, 7, 576), -1);
        graph
            .process_interleaved(7, 576, &raw, &mix, 6, Some(&wet))
            .unwrap();
        let events = GUARD.with(|c| c.replace(None).unwrap());
        assert_eq!(events, 0);
        graph.retire_pa_changes();
    }
}

#[test]
#[ignore = "explicit trusted owner libraries and provider fixtures required"]
fn initial_preparation_waits_for_actual_commit_frame_and_refuses_live_legacy_switch() {
    let (manifest, fixtures) = setup();
    let json = read(&fixtures, "matrix4x8");
    let mut graph = ModuleGraph::load_configured(manifest, 7, 0, 16).unwrap();
    let mut prepared = graph.prepare_pa_change(&json, vec![0, 1, 2, 3], 4).unwrap();
    let raw = vec![0.; 16 * 48];
    let mix = vec![0.; 4 * 48];
    let wet = vec![0.; 2 * 48];
    graph
        .process_interleaved(7, 0, &raw, &mix, 4, Some(&wet))
        .unwrap();
    assert_eq!(graph.commit_pa_change(&mut prepared, 7, 48), -3);
    graph.mute_pa().unwrap();
    assert_eq!(graph.commit_pa_change(&mut prepared, 7, 0), -4);
    assert_eq!(graph.commit_pa_change(&mut prepared, 7, 48), 0);
    let status = graph.pa_v2_status().unwrap().unwrap();
    assert_eq!(status.applied_frame, 48);
    assert_eq!(status.next_frame, 48);
    assert_eq!(status.muted, 1);
    graph
        .process_interleaved(7, 48, &raw, &mix, 4, Some(&wet))
        .unwrap();
}

#[test]
#[ignore = "requires an independently built feedback-capable owner"]
fn single_feedback_notch_readback_and_bypass() {
    let path = PathBuf::from(std::env::var_os("GP_FEEDBACK_PA_LIBRARY").unwrap());
    let root = PathBuf::from(std::env::var_os("GP14_PA_FIXTURES").unwrap());
    let mut graph: serde_json::Value = serde_json::from_slice(&read(&root, "stereo3way")).unwrap();
    graph["outputs"].as_array_mut().unwrap().truncate(1);
    graph["outputs"][0]["splits_hz"] = serde_json::json!([]);
    graph["outputs"][0]["band"] = 0.into();
    let bypass = serde_json::to_vec(&graph).unwrap();
    graph["feedback_output"] = 0.into();
    let mut pa = Pa::load(&path, &serde_json::to_vec(&graph).unwrap(), 1, 0).unwrap();
    assert_eq!(pa.status().unwrap().feedback_output_plus_one, 1);
    assert_eq!(pa.rearm(1, 0), 0);
    let mut frame = 0;
    let mut input = [0.; 512];
    let mut out = [0.; 256];
    for _ in 0..40 {
        for (i, pair) in input.chunks_exact_mut(2).enumerate() {
            let n = (frame + i as u64) as f64;
            pair.fill(0.002 * (n / 4800.).exp() * (std::f64::consts::TAU * n / 48.).sin());
        }
        assert_eq!(pa.process(&input, &mut out, 1, frame), 0);
        frame += 256;
    }
    assert!(pa.status().unwrap().feedback_hz.abs_diff(1000) <= 4);
    let mut prepared = pa.prepare(&bypass).unwrap();
    assert_eq!(pa.mute(), 0);
    assert_eq!(pa.process(&input, &mut out, 1, frame), 0);
    assert_eq!(pa.commit(&mut prepared, 1, frame + 256), 0);
    let status = pa.status().unwrap();
    assert_eq!(
        (status.feedback_output_plus_one, status.feedback_hz),
        (0, 0)
    );
    pa.retire();
}
