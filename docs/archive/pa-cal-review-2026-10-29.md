# GP-CAL-REVIEW — calibrated offline Desk review

State: **ACCEPTED (software only)**, 2026-10-29. Single-threaded canonical-checkout work.

Shared integration: bounded offline evidence submission to SHR PA, separate from authenticated GP20 captures; current-basis fencing and actual-owner application.

Plan: freeze additive versioned request semantics; implement owner → provider → Desk review; validate refusal, cancellation, stale completion, recovery and explicit muted application. No automatic apply/rearm. No devices (including null PCM), services, hardware or load tests.

Acceptance:
- [x] Bounded evidence and calibration provenance, common origin and configuration identity preserved.
- [x] Imported offline evidence visibly distinct from authenticated captures.
- [x] Refusal/cancellation/stale/recovery and actual-owner transaction regressions pass.
- [x] Required normal production and compatibility suites pass without device access.
- [x] Separate physical procedure documented; disposable outputs cleaned.

Evidence: baseline PA f6be5ae, Desk 511a811, GigPies bddb503; all three checkouts clean. PA published CI fails sibling-relative documentation links (fixed locally); Desk and GigPies published CI pass.

Next action: separately authorized physical qualification; commit/push only on user request. No pending software blocker in this scope.

## Integration and actual-owner evidence

Explicit startup version 2 admits up to four hash-pinned regular offline files,
1 MiB each, containing existing PA v2 `eq_sweeps` requests. Explicit GP20 v2
`measurement_offline_eq` selects one catalog ID. The host preserves imported
origins and calibration, separately fences current authority/configuration/buses,
and delegates fitting/candidate construction to SHR PA. No paths or sample arrays
travel through the control command. Results remain `offline_proposal`, never
capture records. Existing worker deadlines, cancellation and stale-result fences,
and muted GP14 prepare/review/commit with separate rearm are retained.

The final release `actual_keyboard_imported_sweep_review_to_muted_owner` episode
passed in 3.11 s: incompatible calibration refusal; recovery to valid evidence;
explicit imported label and calibration review; cancelled apply review with exact
unchanged basis; explicit muted commit; actual-owner generation 1 → 2 with exact
candidate readback; stale old result refused. Original imported source epoch 77,
map 88, reference offset 17 and first frame 10000 were preserved independently of
the host basis. Operator rearm was not sent. The initial driver key-name mistake
stopped at review with no configuration commit; the corrected run used a fresh
disposable authority identity. Production replay protection was unchanged.

The invented half-gain response uses magnitude correction +6.020599913279624 dB
and 23-sample arrival. The owner proposed one output-0 bell, 10734.375 Hz, -4 dB,
Q 0.5, for the damped-high-frequency target. Predicted RMS error changed
1.345679 → 0.174135 dB, with zero additional boost headroom. These are offline
predictions, not acoustic or microphone acceptance.

The existing release `actual_keyboard_measurement_to_muted_owner_apply` episode
passed in 17.87 s, preserving authenticated software capture, cancellation,
overlap, no-change, stale refusal and explicit alignment/EQ application. Direct
`gp20_owner` passed in 6.78 s, including reserved-slot isolation, actual owner
analysis/candidate, refusal, cancellation and recovery. All signals were invented;
no buffers went to an audio device. Provider processes were bounded to 120 s.

## Validation

All commands used Rust 1.97.1, committed lockfiles, CARGO_INCREMENTAL=0,
CARGO_BUILD_JOBS=1 and the shared nonblocking build lock in canonical checkouts.

| Owner | Normal Rust suites | Other checks |
|---|---|---|
| PA | 114 passed; four PCM-opening tests filtered | Clippy, release, C v1/v2 compatibility, frozen measurement corpus, docs, 5 Python tests |
| Desk | 351 default / 354 native passed; 25 opt-in ignored each | Native Clippy, release interoperability drivers, frozen producer corpora, 9 Python tests |
| GigPies | 485 default / 569 hardware-host passed; 21 / 48 opt-in ignored | All-feature Clippy, release provider, actual GP20 owner, 50 Python tests |

Formatting and complete candidate-index publication checks pass in all three
repositories; temporary indexes preserve the real Git indexes. Historical and
exhaustive research, long benchmarks, terminal/live/null-PCM scripts and physical,
playback, hardware, MIDI, DMX and load experiments were intentionally excluded.
No services or host settings changed. No commits or pushes were made.

Reproduce using [the provider contract](../reference/MEASUREMENT_PRODUCER.md#imported-calibrated-sweep-review-explicit-v2).
The initial graph is exactly `basis_configuration` in SHR PA's frozen
`tests/fixtures/measurement/v1/proposal.json`. Generate the calibrated request
with PA's `offline-review-fixture GRAPH_JSON 1`; catalog it as `imported`.
For `refused`, add a second position with distinct capture/position IDs and a
conflicting calibration provenance. Hash both complete files at startup.
Use existing hash-pinned module-manifest conventions for PA/FX/REC; no sibling
Cargo path dependencies or repository copies are required.

```sh
cargo test --locked --all-targets
cargo test --locked --features hardware-host --all-targets
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo build --locked --release --features hardware-host --example measurement-provider
GP05_MANIFEST=ABS_MODULES GP20_OWNER_CONFIG=ABS_OWNER GP20_GRAPH=ABS_GRAPH GP20_CORPUS_DIR=ABS_PRIVATE_CORPUS cargo test --locked --features hardware-host --test gp20_owner -- --ignored
# Each Desk episode uses a fresh bounded provider and disposable authority directory:
GP20_EPISODE_DIR=ABS_PRIVATE_DIR cargo test --locked --release --test measurement_actual_frontend actual_keyboard_imported_sweep_review_to_muted_owner -- --ignored
GP20_EPISODE_DIR=ABS_PRIVATE_DIR cargo test --locked --release --test measurement_actual_frontend actual_keyboard_measurement_to_muted_owner_apply -- --ignored
```

## Published CI and remaining qualification

[PA CI](https://github.com/gigpies/shr-pa/actions/runs/40906285938) failed at
f6be5ae because sibling-relative documentation links are absent on isolated
runners. The local fix passes with outside-repository paths treated as absent;
remote CI cannot validate this uncommitted fix until publication is requested.
[Desk CI](https://github.com/gigpies/shr-desk/actions/runs/40906303718) passed at
511a811, and [GigPies CI](https://github.com/gigpies/gigpies/actions/runs/40906337571)
passed at bddb503. No remote run was restarted on unchanged source.

The [separate physical procedure](https://github.com/gigpies/shr-pa/blob/main/docs/PHASE_ALIGNMENT.md#physical-qualification-after-imported-sweep-review)
remains pending: actual clock/reference alignment and socket map, original mic
calibration and gain/headroom, isolated sequential repeated capture, explicit
muted application and separate rearm, fresh combined-response measurements,
and independent acoustic/synchronization/protection acceptance. Offline timing
flags, calibration files and software predictions close none of those gates.

Module records: [PA](https://github.com/gigpies/shr-pa/blob/main/docs/archive/pa-cal-review-2026-10-29.md)
and [Desk](https://github.com/gigpies/shr-desk/blob/main/docs/archive/pa-cal-review-2026-10-29.md).

## Tested identities

PA library SHA256: `20e31538552e529dbc135ddf7977f31bba06d072572926ad1975a8b5409987a4`.
Analyzer SHA256: `ef773d4e7a9ddfd9f0a52504f088d9599b56ef55d0af29a33518f70f539a7d83`.

Final source-input SHA256 (sorted path + NUL + bytes + NUL for src/, tests/,
examples/, .cargo/, Cargo files, toolchain and build.rs):

- shr-pa: `21b2761b7b692cfd29b21d6b27432a61274a961e5811ae0e683336ca8b28f47b`
- shr-desk: `92c5d47abb018d32f597b58087f1658bd1796723243b4d3b15849b4155aeae44`
- gigpies: `0a53398a5760c85640ea50b7e39278241fd516ebc08d6720c0b92623550b8f24`

Desk production code matches the full suites; only the ignored driver key spelling
was subsequently corrected and both affected release drivers rerun.

## Retention

After processes, build lock and open users were checked, 240 completed
debug test executables were removed, reclaiming 20,942,532,608 allocated bytes
(19.50 GiB). Reusable library/dependency caches and intended release runtimes
remain in canonical targets; no unrelated output or user data was removed.
Temporary manifests, generated sweeps, protocol dumps, render receipts and logs
are consolidated into this record and removed at closeout. The maintained
fixture producer, source tests, guides and these owning acceptance records remain.

Final scratch cleanup removed 3,160,959 logical bytes (3,538,944 allocated bytes).
Retained normal targets: GigPies 4.6 GiB, Desk 3.9 GiB, PA 669 MiB; approximately
56 GiB free. No cleaned test executables were rebuilt merely to verify cleanup.
