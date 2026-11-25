# Development

[Engineering index](README.md) · [Integration plan](MODULE_IMPLEMENTATION_PLAN.md) · [Publication](PUBLICATION.md)

GigPies owns integration, the processing engine and show/control contracts.
Audio desk work lives in `../shr-desk`, lighting desk in `../shr-lightdesk`,
lighting engine in `../shr-lux`, PA in `../shr-pa`, effects in `../shr-fx` and
recording in `../shr-rec` ([components](../architecture/COMPONENTS.md)). Each
repository builds on its own: no path dependencies.

## Toolchain and build lock

Rust 1.97.1 (pinned in `rust-toolchain.toml`), edition 2024, committed `Cargo.lock`.
The Pis share one nonblocking build lock; a busy lock means wait, not retry harder.

```sh
B="flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1"
$B cargo +1.97.1 test --locked -j1 --all-targets
$B cargo +1.97.1 test --locked -j1 --all-targets --features hardware-host
$B cargo +1.97.1 clippy --locked -j1 --all-targets --features hardware-host -- -D warnings
cargo fmt --all -- --check
python3 -m unittest discover -s scripts -p 'test_*.py'
$B cargo +1.97.1 build --locked -j1 --release
```

Pi 5 has 2 GiB RAM: run one Cargo job at a time. Review disk use below 20 GiB free
or above 5 GiB in a target directory; remove only task-built test executables.

## Test classes

The agent selects tests by changed behavior and reports what ran and what was skipped.

| Class | Runs | When |
|---|---|---|
| Normal | `cargo test --all-targets` (both configurations), Python `test_*.py` | Always for shared engine, schema, routing, transport, safety or rendering changes; CI runs both |
| Owner-artifact | `-- --ignored` with `GP05_MANIFEST` etc. | When the change touches PA/FX/REC integration or meters (see below) |
| Long virtual-clock | `--release --test brain_bridge -- --ignored` | When the Brain bridge/ASRC or its assumptions change |
| Historical / audition / exhaustive | Named `--ignored` tests, private media | Only when directly relevant or requested |
| Recorded vocal assist | `GP_VOCAL_STEMS=/abs/stem.wav@start[:…] cargo test --release --test vocal_assist recorded -- --ignored --nocapture` (statistics only) | When the vocal-assist detector or soundcheck rule changes |
| Two-node / physical | Lab runners, real devices | Only in a separately authorized, reserved session |

Normal tests generate synthetic WAVs in temporary directories. They never open
audio, MIDI, DMX, sockets to other hosts or third-party music.

## Owner-artifact tests

These load the real SHR PA/FX/REC libraries. Build each owner's release library
from a clean committed checkout, then record a fresh manifest:

```sh
for r in shr-pa shr-fx shr-rec; do (cd ../$r && $B cargo +1.97.1 build --locked -j1 --release --lib); done
python3 scripts/owner_manifest.py artifacts/owners          # writes artifacts/owners/modules.json
export GP05_MANIFEST=$PWD/artifacts/owners/modules.json
export GP_PA_V2_FIXTURES=/home/shome/p/shr-pa/tests/fixtures/cpa/v2 GP14_PA_FIXTURES=$GP_PA_V2_FIXTURES
```

The manifest pins each library, header and owner revision by SHA-256; loading
fails if a file changes. Then run the relevant targets with
`--features hardware-host -- --ignored --test-threads=1`:

| Target | Proves |
|---|---|
| `--test modular_owners` | PA v2 retirement/rearm, FX bank, REC takes, monitor taps into the PA matrix |
| `--test brain_owners` | Brain duplex with actual owners and raw analysis |
| `--test metering actual_owner` | GP-METER Main pre-PA/monitor levels and PA output against independent instances |
| `--test gp07_frontend` | Desk frontend edits, monitors, FX wet+dry→PA, raw PCM24 stems (also needs `GP07_DESK_DRIVER`) |
| `--test module_graph_pa_v2 single_feedback_notch` | Feedback-notch readback and bypass through real SHR PA (set `GP_FEEDBACK_PA_LIBRARY` to the PA library) |
| `--lib actual_four_aux` | Four Brain FX auxes over authenticated transport (needs `GP_FX_LIBRARY`) |

Never make untrusted library loading implicit in CI.

## Synthetic transport validation

`src/transport/` owns packet/control contracts and bounded worker handoffs
([protocol](../reference/AUDIO_TRANSPORT.md), [v1 blueprint](../architecture/AUDIO_TRANSPORT_V1.md)).
`tests/transport.rs` covers parsing, exact samples, admission, ordering, loss,
drift diagnostics and control recovery without sockets. The loopback-socket
capacity check is opt-in: `cargo test --locked --test transport_network -- --ignored`.
Two-Pi runners stay private under `artifacts/audio-transport/` and need a new
mutually acknowledged reservation before any rerun.

## Hardware host and Brain

The optional `hardware-host` feature builds `gigpies-hardware` (bounded ALSA/module
bench), `gigpies-remote` (configured provider/Brain runner) and `gigpies-brain`
(fake duplex adapter over the production host, ASRC and transport). Physical
opening needs `--activate-physical` and session authorization; no service is
installed. See [hardware evidence](../acceptance/AUDIO_HARDWARE.md),
[modular processing](../reference/MODULAR_PROCESSING.md) and [Brain audio](../reference/BRAIN_AUDIO.md).

## Code map

| Area | Source | Contract / guide | Tests |
|---|---|---|---|
| Mixer, channel strip, gate | `mixer.rs`, `channel_processing.rs`, `gate.rs` | [channel processing](../reference/CHANNEL_PROCESSING.md) | `gp07*`, `gate`, `dynamic_eq` |
| Control authority, wire | `mixer_control.rs`, `control_model.rs`, `processing_wire.rs` | [control wire](../reference/AUDIO_CONTROL_WIRE.md) | `gp02`–`gp07`, `structural_control` |
| Engine host, module graph | `local_audio.rs`, `local_audio/`, `module_graph.rs`, `host/` | [module graph](../reference/MODULE_GRAPH.md), [contracts](../reference/MODULE_CONTRACTS.md) | `modular_engine`, `modular_owners` |
| Meters, spectrum | `metering.rs`, `meter_wire.rs`, `spectrum.rs` | [contracts](../reference/MODULE_CONTRACTS.md) | `metering` |
| Remote and Brain | `remote/`, `brain_runtime/`, `transport/` | [remote transport](../reference/REMOTE_TRANSPORT.md), [Brain bridge](../reference/BRAIN_AUDIO_BRIDGE.md) | `remote::tests`, `brain_bridge`, `transport` |
| PA measurement capture | `measurement_capture.rs` | [measurement producer](../reference/MEASUREMENT_PRODUCER.md) | `gp20_*` |
| Offline automix | `automix/` (`balance`, `tone`, `bass`, `drums`, `bleed`, `ambience`, `matching`, `delivery`, …) | [offline guides](../guides/README.md) | `balance`, `tone`, `bass`, `drums`, `ensemble`, … |
| Shared console UI and MIDI | `src/instrument-panel/` (package `shr-instrument-panel`) | [instrument panel](../reference/INSTRUMENT_PANEL.md) | crate tests |

Automix rules separate detection, proposed change, actual validation and remaining
defects. Python scripts under `scripts/` only summarise or plot saved measurements;
all processing decisions stay in Rust. The previous module-by-module tour is in
the [archived guide](../archive/records/DEVELOPMENT-before-2026-11-24.md#offline-automixer).

The shared panel package is vendored into consumers with
`python3 scripts/package_panel.py [shr-desk|shr-lightdesk]`; bump its version and
the consumer's exact requirement together.

## Repository layout

```text
src/            library, CLI and binaries      tests/      fast synthetic integration tests
docs/           maintained documentation        scripts/    reviewed helper scripts
examples/       runnable example notes          sessions/   public session format (local/ ignored)
recordings      ignored link to ../waves         artifacts/  ignored evidence and scratch
archive/local/  ignored retired experiments     .github/    hardware-free CI
```

## Documentation rules

- `docs/README.md` and `STATUS.md` are for musicians; `architecture/` design,
  `guides/` operation, `reference/` contracts, `acceptance/` evidence,
  `development/` contributor procedures, `archive/` history.
- Describe current behavior, usage and limits. No task logs, daily test counts or
  commit receipts on product pages; task state lives only in the integration plan.
- When moving a document, archive the original, repair links, and never change a
  frozen fixture's recorded path or hash.
