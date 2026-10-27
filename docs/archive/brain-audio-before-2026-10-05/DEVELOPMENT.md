# Development

## Console and engine ownership

GigPies owns final integration and show/control contracts. Audio surface work
belongs in `../shr-desk`, lighting surface work in `../shr-lightdesk`, and lighting
engine algorithms/output in `../shr-lux`. Both desks target one Brain with two
1920×1080 monitors and separate MIDI controllers. They currently build and run
offline independently; this CLI does not launch either desk. See
[Brain integration](BRAIN_CONSOLE_PLAN.md) before adding adapters.

## Explicit hardware host

The optional `hardware-host` feature builds `gigpies-hardware`, a bounded stereo
ALSA/module bench; see [its contract and acceptance](../../acceptance/AUDIO_HARDWARE.md). Build
owner libraries independently and pass explicit paths in private configuration.
Normal tests open no devices; actual measurements require session authorization
and resource reservation. `cargo run` still defaults to the offline GigPies CLI.

```sh
CARGO_INCREMENTAL=0 cargo test --locked --all-targets --features hardware-host
CARGO_INCREMENTAL=0 cargo clippy --locked --all-targets --features hardware-host -- -D warnings
CARGO_INCREMENTAL=0 cargo build --locked --release --features hardware-host --bin gigpies-hardware
```

`tests/host.rs` protects channel-wide fades, deadline selection, source precision,
frame ownership and bounded metric reporting. Historical studies and the opt-in
socket test retain their existing classification.

## Synthetic transport validation

`src/transport/` owns packet/control contracts and bounded worker handoffs; see
[the protocol and evidence](AUDIO_TRANSPORT.md). Normal `tests/transport.rs`
covers parsing, exact samples, channel/session admission, ordering, loss, frame
and sequence limits, drift diagnostics, wet envelopes and control recovery.
It opens no sockets or audio devices.

```sh
CARGO_INCREMENTAL=0 cargo test --locked --test transport
CARGO_INCREMENTAL=0 cargo test --locked --all-targets
```

The local socket-capacity regression is opt-in because it binds a loopback UDP
socket. It generates no external traffic or audio:

```sh
CARGO_INCREMENTAL=0 cargo test --locked --test transport_network -- --ignored
```

Paced two-Pi runners and their evidence remain private under
`artifacts/audio-transport/2026-10-03/`. They require a new mutually acknowledged
resource reservation before rerunning, with exact ports, candidate hashes and
hard deadlines. Neither CI nor the offline CLI starts a transport service.

## Layout

```text
src/                    Rust library and command entry point
tests/                  fast synthetic integration and CLI tests
examples/               instructions for runnable examples as they are added
sessions/               public session-format guidance; local/ is ignored
docs/                   maintained design, status and development guides
docs/assets/            current SVG diagrams and retained historical concept artwork
docs/archive/           preserved draft documents
recordings              ignored link to ../waves/recordings
artifacts/              ignored renders, reports and scratch evidence
archive/local/          ignored retired local experiments
.github/workflows/      hardware-free Rust CI
```

Keep current decisions in maintained docs. Preserve historically useful drafts under
`docs/archive/` with an index; do not treat them as active implementation requirements.
Use `archive/local/` for private retired experiments and `artifacts/` for disposable
outputs. Never move private recordings into the public documentation archive.

## Validation

Enable [publication hooks](../../development/PUBLICATION.md) with
`git config --local core.hooksPath .githooks` after checking for existing hooks.
Use `CARGO_INCREMENTAL=0` for local Rust builds/tests.

```sh
rustc -vV
cargo fmt --all -- --check
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python3 -m unittest discover -s scripts -p 'test_*.py'
cargo build --locked --release
```

Normal tests use generated synthetic WAVs and temporary directories. They protect
metadata interpretation, malformed input handling and CLI contracts. They never
start audio, MIDI, DMX, network services or access third-party music.

The agent selects tests based on changed behavior. Run focused regressions while
implementing; run the normal production suite for engine/rendering, shared models,
routing/persistence, concurrency, safety and broadly reused changes, and releases.
Historical auditions, exhaustive matrices, long benchmarks and private-media checks
belong behind explicit opt-in commands once added. Keep required current production
regressions in the default suite. Report run and intentionally skipped test classes.

## Scope and hardware

Sibling checkouts are references. Keep this project independently buildable; no local
path dependencies or assumptions that another repository has been installed.
Preserve source licences when adapting code. The skeleton starts no hardware.
Hardware sessions and audible playback require a specific authorized task.

Before publication inspect staged files, confirm music/private data are absent,
run `git diff --cached --check`, and run the normal checks above. Version 0.2.3
includes offline tools and an explicit optional hardware bench, with both consoles
still in development. Release notes must distinguish
implemented, offline-validated and physically verified behavior.
Run `python3 scripts/check_publication.py` after staging. The guard checks the
actual Git blobs, private directory boundaries and the reviewed script list.

## Offline automixer

See [AUTOMIX.md](../workflows/AUTOMIX.md) for commands and explicit private-media test opt-ins.
`src/automix/config.rs` owns editable settings and validation, `dsp.rs` owns signal
processing, and `render.rs` owns offline files/timeline/reports. Production tests
remain hardware-free. Failed render directories are partial and must not be published.

The optional FX/review extension is documented in [FX_PASS.md](../../guides/FX_PASS.md).
`effects.rs` owns configuration/routing; attributed static engines are in
`fx_engines.rs` and `exciter.rs`; `analysis.rs` owns deterministic measurements and
bounded review rules. `scripts/analyze_mix.py` is optional plotting only and cannot
change settings. All correction decisions remain in Rust and are covered by the
normal synthetic suite.

The [musical balance pass](../../guides/BALANCE_PASS.md) is owned by `automix/balance.rs`. It
reuses native source reading/routing and DSP, buffers synchronized energy covariance,
and searches musical faders without touching input trims. `scripts/balance_excerpts.py`
is an optional PCM excerpt/export verification tool; it never changes mix settings.

The [manufacturer-reference experiment](../studies/PRESET_EXPERIMENT.md) adds explicit shelf
shapes and local parameter mapping. Numerical collections and media stay ignored.

The [tone pass](../../guides/TONE_PASS.md) is owned by `automix/tone.rs`. It reuses the existing
FFT and production DSP, accepts explicit instrument-path identity, and has focused
normal regressions in `tests/tone.rs`. Its spectral targets are editable intent
choices in a separate policy; it does not change the session persistence schema.

The [source-rule coordinator](../../guides/SOURCE_RULES.md), `automix/expert.rs`, owns explicit
profile applicability, sustained-compression plans, source advice and joint
validation. `tone.rs` owns guitar spectral measurements and EQ search. New rules
must separate detection, proposed changes, actual validation and remaining defects;
never assign processing from display names. `tests/expert.rs` uses synthetic media
and is part of the normal suite. Public profile/rule examples are our parameters;
manufacturer collections and private media stay local.

Independent finished-mix review is owned by `automix/reference.rs`; it must never
import or modify processing settings. `tests/reference.rs` exercises alignment and
file contracts with synthetic audio. `scripts/play_pair.py` owns prepared two-clip
playback; its normal tests inject a fake player and never open hardware. See
[reference review](../../guides/REFERENCE_REVIEW.md) for confidence limits and commands.

Source audio is shared across sibling projects through `../waves`; see
[local media layout](../../development/LOCAL_MEDIA.md). Generated evidence stays project-local.

The [DI bass/kick extension](../../guides/BASS_KICK.md) is owned by `automix/bass.rs`. Its separate
policy keeps bass intent distinct from guitar profiles. It reuses production DSP
and the expert module's repeated-contact thresholds. `tests/bass.rs` protects
current decisions and guards in the normal suite. `scripts/bass_evidence.py` only
summarizes/plots saved measurements; it cannot select or modify processing.

The [drum extension](../../guides/DRUMS.md) is owned by `automix/drums.rs`, with the explicit
rhythmic-offset calculation in `balance.rs`. It uses production source/strip/routing
DSP and the existing bass FFT helpers. `tests/drums.rs` belongs in the normal suite;
`scripts/drum_evidence.py` summarizes saved measurements without choosing settings.
`drum-verify` checks a frozen pilot candidate and cannot search against held-out data.

`automix/bleed.rs` owns [conditional snare-spill analysis](../../guides/SNARE_BLEED.md), bounded
static probes and diagnostic waveform predictors. `tests/bleed.rs` is normal synthetic
coverage. `scripts/snare_bleed_evidence.py` summarizes fixed masks without selecting
settings. The renderer has no new gate, expander or reference-cancellation path.

`scripts/snare_temporal_evidence.py` audits frozen attack/decay representations and
retained-rise support. Its separate fit/evaluate commands never select processing;
`test_snare_temporal_evidence.py` is fast normal coverage for training isolation,
quiet/compound protection, malformed evidence and source-identifiability limits.
See the [temporal follow-up](../studies/SNARE_BLEED.md#temporal-follow-up-frozen-representation-audit).

`automix/bleed_reference.rs` owns frozen joint-reference diagnostic fitting and
full-source evaluation. It reuses `bleed::measure`, keeps sample support inside each
split, and never changes settings or audio. `tests/bleed_reference.rs` is normal fast
synthetic coverage for identifiability, reference contamination and saved-model bounds.

`balance-source-analyze` shares source reading/routing with the processed balance
analyzer but bypasses DSP. `scripts/ensemble_review.py` is a descriptive observer
of those measurements; it cannot select processing or populate listener preference.
`tests/balance.rs` checks SOURCE against production renderer samples and coherent
sums; `scripts/test_ensemble_review.py` checks fixed raw activity, section conflicts,
export-stage separation and provenance/alignment refusal. Both are normal fast tests.
See [Complainiacs reassessment](../studies/COMPLAINIACS_REASSESSMENT.md) for the current use.

The [subsequent workflow review](../studies/COMPLAINIACS_WORKFLOW_REVIEW.md) adds coherent
snare/remaining-ensemble groups and `scripts/snare_context.py`. Its normal tests
protect fixed-event context, signed interaction, silence and split independence.
Listening specifications now pin original SOURCE settings/WAV hashes; preparation
rechecks every parent and metadata file before writing its completion manifest.
Unknown rhythmic intent withholds fader moves without vetoing independent
fixed-fader processing. Existing technical guards remain unchanged.

`automix/preservation.rs` owns explicit unchanged-source preparation. The shared
offline admission contract allows schema-valid HPFs including bypass; production
measurement must use the configured filter. `tests/preservation.rs` checks exact
SOURCE-as-FINAL samples and analyzer/renderer agreement. Missing tone/balance intent
and default FX review preserve settings; explicit historical experiments remain
reproducible. See [source preservation](../../guides/SOURCE_PRESERVATION.md).

`automix/ambience.rs` owns explicit artistic FX proposals, engine-decay calibration,
training-only return calibration and held-out ensemble checks. It preserves the
accepted direct settings. `tests/ambience.rs` verifies production render agreement,
zero amount, source identity, invalid input/FX-return refusal, held-out isolation
and overload rejection. The checkpoint contract accepts an explicitly declared
production FX tail while retaining pinned SOURCE files; Python regressions reject
unexplained duration changes, altered offsets and inconsistent tail metadata.
`ambience::calibration` reports individual decay/return limits and amount-aware
bus observations; `ambience::review` writes the separate eligibility, target and
pending-acceptance results. Normal synthetic coverage includes unreachable decay,
return-floor limitation, actual measured ratios and missing held-out evidence.
`ambience::persistence` pins inputs/reports and writes the completion record last;
`ambience-check` verifies it without rendering. Normal tests cover changed inputs,
partial/failed plans, no overwrite and master-limiter observer/render agreement.
Chronological splits prevent continuous FX history from carrying held-out audio
into later training. The generic content hashes live in `automix::identity`;
EQ matching retains its existing identity API through re-exports.
`ambience::validation` owns complete-window coverage and named ensemble failures.
`ambience::audit` reads saved evidence without audio access or a new eligibility
decision. Its normal tests cover legacy missing observations, bounded returns,
zero amount, rejected plans, inconsistent saved evidence and output preservation.

`automix::matching` owns [frozen EQ matching](../../guides/EQ_MATCHING.md), map imports and the
local review page. `tests/matching.rs` covers production-DSP preservation, recovery,
noise/cancellation abstention, persistence and nonlinear interactions. SHA-256 pins
inputs/settings; serde_json float round trips preserve those identities. The page
exports a selection request for offline validation and has no live control path.
`matching::compare_eq` adds a read-only comparison of identical recordings with
group EQ as the only permitted settings difference. Paired spectral dispersion is
diagnostic evidence, never a target or fit tolerance. Normal matching regressions
cover exact identity, nonlinear interactions, settings refusal, missing evidence
and held-out isolation; private phrase-comparability studies remain opt-in.
`matching::comparison_review` formats new or saved diagnostics with identity and
scope checks, explicit missing evidence and named compressor/FX consequences.
New matching plans require chronological splits; historical state validation and
exact reset retain the existing contract. Newly written HTML reviews disclose
interleaved historical evidence. The shared ordering predicate lives in
`automix/mod.rs`; diagnostics can describe legacy passages without refitting them.

`automix::delivery` owns the versioned export sidecar and completed-delivery checks;
`true_peak` owns the streaming interpolation meter. `render` keeps the legacy path
and supplies the policy-aware render and scalar-only observation entry points.
`observation` taps actual production states and reconciles peak contributions.
The normal `tests/summing.rs` and `tests/delivery.rs` protect independent arithmetic,
conversion, mode independence, static output identity and failure recovery.
The policy checkpoint and external-meter script have synthetic Python regressions;
normal tests do not require FFmpeg. See [commands and contracts](../../guides/SUMMING_DELIVERY.md).

## GP07 channel processing checks

Normal `gp07`, `gp07_alloc` and `gp07_local` tests protect DSP references,
transition/neutral behavior, every input, allocation-free rendering, strict wire,
shared authority/retry history and private-endpoint recovery. Run Cargo under the
parent-held nonblocking build lock with the existing Rust1.97.1, locked, jobs1 and
incremental0 policy. Default and hardware-host complete suites remain required.

The ignored `gp07_frontend` acceptance test requires an explicitly hash-verified
Desk test executable (`GP07_DESK_DRIVER`) and actual owner libraries (`GP05_MANIFEST`).
It runs the actual LocalAudio pump and Desk frontend, checks every monitor block,
independent actual FX wet+dry→PA output, raw analysis and all eight recorded PCM24
stems. See the v2 fixture README for the opt-in producer-corpus regeneration command.
The v1 corpus is retained unchanged for explicit unsupported-version checks; its
historical generator is reproducible at the archived v1 source checkpoint.
No hardware or operator windows are opened. Private task manifests bind artifacts
and results; tests cannot substitute fixtures for actual-provider acceptance.


After independently building and checking the Desk `gp07_frontend` test binary and
owner-library manifest, run from this repository with explicit absolute paths:

```sh
flock -xn /home/shome/p/.gigpies-build.lock \
  env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 \
  GP05_MANIFEST=/absolute/private/modules.json \
  GP07_DESK_DRIVER=/absolute/private/gp07_frontend-test-binary \
  GP07_ACCEPTANCE_EVIDENCE=/absolute/private/acceptance.json \
  cargo +1.97.1 test --locked -j1 --features hardware-host \
  --test gp07_frontend -- --ignored --exact \
  actual_frontend_processing_preserves_raw_and_monitors_and_module_order
```

The test requires at least seventeen real frontend edits, checks every actual
final-reply sample-frame boundary against provider observations,
and replays each physical input through an independent slot-zero mixer to verify
channel mapping across the entire captured FOH timeline. A separate direct-form-I
bell reference checks the full EQ-only timeline, including crossfades, all four
band controls, crossed frequencies and bypass; neutral/bypass are bit exact.
The same source drives independent actual FX wet-plus-dry -> PA comparisons. It checks recording
coverage and retained settings after the driver exits and the lease expires.

## Configurable processing and remote integration

The `hardware-host` feature also exposes the shared multichannel `host::duplex`
adapter and explicitly configured synthetic `gigpies-remote` provider/Brain runner.
Neither starts by default or qualifies a physical device. See
[composition and mapping](MODULAR_PROCESSING.md), [remote transport](REMOTE_TRANSPORT.md)
and the [acceptance matrix](MODULAR_ENGINE_ACCEPTANCE.md).

Serialize all local Rust validation through the parent-held nonblocking lock:

```sh
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked -j1 --all-targets
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked -j1 --all-targets --features hardware-host
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo +1.97.1 clippy --locked -j1 --all-targets --features hardware-host -- -D warnings
```

A busy lock means defer the command. `modular_engine`, `structural_control`,
`gp07_alloc` and remote unit regressions belong in the normal production suite.
`modular_owners`, `module_graph_pa_v2` and `structural_producer` explicitly require
trusted independently built owner artifacts: set `GP05_MANIFEST`,
`GP_PA_V2_FIXTURES` and `GP14_PA_FIXTURES`, plus a private `GP14_PRODUCER_DIR`
for producer output, then run the selected test with `--features hardware-host`
and `-- --ignored` under the same lock. Do not make untrusted library loading
or generated acceptance artifacts implicit in CI. Tests document their exact
required inputs. Actual cross-node checks require a bounded reservation on both
hosts; physical PCM remains a separate authorization and acceptance step.
