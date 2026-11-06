# GigPies integration: plan and implementation

Updated 2026-10-28. This document owns cross-module integration tasks from planning
through implementation and acceptance. Module-only work stays in the
[owning module plan](MODULE_IMPLEMENTATION_MAP.md). The same task must not have a
second status table in an index, handoff, capability matrix or knowledge note.

## Scope

GigPies is the live-band sound and lighting system. Runtime owners are GigPies,
SHR Desk, SHR Lightdesk, SHR PA, SHR FX, SHR REC and SHR Lux.
SHR-DAW (including Player), Drums, Synth, Sampler, Tone Over 9000 and Skills are
reference/code sources, not GigPies runtime modules or unfinished integration
requirements. Reuse only a needed implementation with provenance; do not add
instrument hosting, a DAW process or their standalone backlogs to GigPies.
[Components](../architecture/COMPONENTS.md) owns the precise boundaries.

## How work is tracked

1. Give each task one stable ID, one owning document and one accountable owner.
   A cross-module task here names contributing repositories and their boundaries;
   do not mirror it as new tasks in each contributor's plan. A distinct provider
   algorithm/ABI task may live in its module plan and be a named dependency.
2. Keep outcome, scope, acceptance checklist, current state, evidence and exact next
   action in the same task card. Planning adds detail to that card; implementation
   updates it in place. Never append a contradictory second execution checkpoint.
3. Use PLANNED, IN PROGRESS, BLOCKED (with a concrete dependency), IMPLEMENTED,
   or ACCEPTED. Acceptance must name software, physical or musical scope. Passing
   software tests never closes an unperformed physical gate.
4. Keep only open work in the active queue. On completion, record the final result
   and evidence in its existing card, then archive that closed card once. Link the
   archive from the relevant technical contract; do not maintain a running done list.
5. STATUS summarizes product capabilities; historical handoffs and the module map provide
   routing. Contracts describe behavior; acceptance reports contain measurements
   and retained failures. They do not maintain another backlog. Knowledge routes
   to these sources. Update the owning card with the behavior change, not later.

## Baseline evidence

The configurable Stagebox graph, actual PA v2/raw REC/source-clock FX, authenticated
transport and Desk controls have [modular software acceptance](../acceptance/MODULAR_ENGINE_ACCEPTANCE.md).
Brain duplex/ASRC, PFL/AFL and talkback have [integrated software acceptance](../acceptance/BRAIN_AUDIO_ACCEPTANCE.md).
Both consoles have provider-backed native software; Lux has real null-output
lighting authority and named analysis. Physical multichannel, controller/display,
fixture, acoustic and complete-show acceptance remain separate.

The embedded FX contract is fixed stereo delay, not SHR FX's standalone rack.
Legacy rendered mixer snapshots leave meters unavailable; GP-METER provides
independent measured observations. Schematic graphs are not audio measurements. Offline automix results are not live automatic mixing.
See [dated evidence](../STATUS.md), [module contracts](../reference/MODULE_CONTRACTS.md) and source.
The [previous plan](../archive/tracking-before-2026-10-09/MODULE_IMPLEMENTATION_PLAN.md) preserves legacy GP-01..09/task0009 launch cards;
it is historical, not a queue to execute. Later accepted work supersedes its limits.

## SHA-2 dependency maintenance — November 6

PR [#5](https://github.com/gigpies/gigpies/pull/5) was deferred because sha2 0.11
removed `LowerHex` from its digest output. Current upstream
[release notes](https://github.com/RustCrypto/hashes/blob/sha2-v0.11.1/sha2/CHANGELOG.md)
confirm 0.11.1 is released, 0.11.2 remains unreleased, and MSRV 1.88 is supported by
our pinned Rust 1.97.1. The release also fixes AArch64 constant loading. Pin 0.11.1
with its reviewed lockfile; preserve the current serde/rtrb/libloading pins when
bringing the original Dependabot branch onto current main.

Implementation: one byte-oriented lowercase hex encoder replaces 71 digest-format
sites, including certificates, persisted identities, module hashes, wire request
fingerprints, diagnostics and device lock filenames. Algorithm and hashed inputs
are unchanged. Two native-library readers retain `io::copy` through a small digest
writer adapter because the new digest API also removed its `std::io::Write` impl.
Literal standard vectors, leading zeroes, the lock filename, streamed million-byte
input and read-error propagation have focused regressions.

Validation: hardware all-target compilation and all three focused compatibility
regressions passed locally. Merge gates are normal default/hardware tests, both
Clippy configurations and x86/ARM64 Required CI; the linked PR records their final
results and review for the exact merged revision.
Unchanged historical/exhaustive/audition and bridge opt-ins are excluded from this
formatting/dependency maintenance. Review and merge must use the
current PR head and required CI; no administrative merge bypass.
Other Dependabot PRs are merged or superseded, with no unresolved review threads;
all four security alerts are fixed. GP-H1 implementation was published separately
as `ecd471e`; this maintenance does not authorize T3 or physical activation.

## Active work and dependencies

| Task | Owner / contributors | State and next action |
|---|---|---|
| [GP-GATE](#gp-gate--lightweight-channel-gates-and-tom-decay-control) | GigPies channel DSP; Desk presentation | IN PROGRESS: core, live strip, v5 contract and preparation API; workflow acceptance remains |
| [GP-PANEL](#gp-panel--shared-physical-instrument-panel) | GigPies shared UI; Desk/Lightdesk consumers | IN PROGRESS: real-terminal proof and initial production adapters; acceptance gates remain |
| [GP-METER](#gp-meter--measured-audio-on-the-real-desk) | GigPies lead; SHR Desk consumer | IMPLEMENTED / software-validated; owner-artifact P4 BLOCKED: supply reviewed manifest |
| [GP-FX](#gp-fx--writable-owner-effects-in-the-console) | GigPies lead; SHR FX provider, SHR Desk consumer | SOFTWARE VALIDATED: four Brain auxes and Desk controls; physical acceptance separate |
| [GP-SHOW](#gp-show--audio-scenes-and-operator-recovery) | GigPies lead; SHR Desk consumer | IN PROGRESS: bounded saved FX intent and reviewed disarmed restore |
| [GP-AUTO-ENSEMBLE](#gp-auto-ensemble--observation-led-whole-band-automixing) | GigPies | IN PROGRESS: E0–E6 software validated; E7 recording checks complete, source review and musical qualification open |
| [GP-AUTO-BASS-LOW](#gp-auto-bass-low--observed-low-energy-and-definition) | GigPies | IMPLEMENTED / software-validated: independent low-energy objective and source-stage guards; musical acceptance pending |
| [GP-AUTO-LUFS](#gp-auto-lufs--offline-kick-led-source-hierarchy) | GigPies | IMPLEMENTED / software-validated: optional legacy hierarchy; superseded for current auditions |
| [GP-AUTO](#gp-auto--live-soundcheck-and-bounded-assistance) | GigPies lead; SHR Desk consumer | PLANNED: needs trustworthy measurements and an explicit musical objective |
| [GP-REC](#gp-rec--complete-integrated-recording-workflow) | GigPies lead; SHR REC provider, SHR Desk consumer | SOFTWARE VALIDATED: reviewed recording workflow, exact 8/16-track acceptance |
| [GP-H1](#gp-h1--physical-audio-qualification) | GigPies lead; PA/REC/FX owners as needed | PLANNED: separate physical session and observed rig required |
| [GP-H2](#gp-h2--physical-dual-console-operation) | GigPies lead; Desk/Lightdesk contributors | Physical input/profile integration and actual display/controller acceptance; see card |
| [GP-H3](#gp-h3--whole-show-qualification) | GigPies lead; all runtime owners | PLANNED: follows audio/console qualification and Lux LX-06 output |

## GP-METER — measured audio on the real Desk

State: **IMPLEMENTED / development-machine software-validated; owner-artifact P4 acceptance BLOCKED**.
Owner: GigPies integration. Contributor: SHR Desk client/presentation. This card
is the sole plan, checklist and evidence record. The user authorized scoped software
execution in GigPies and Desk on 2026-10-09. The subsequent user instruction
authorizes source commits and pushes; hardware activation remains excluded.

Outcome: actual channel, stereo main and every configured monitor level/clip
observation reaches the provider-backed native Desk alongside existing dynamics
feedback. Measured silence, unavailable, stale, invalid and clipped are distinct.
Software acceptance covers the production graph and authenticated delivery;
physical sound, device mapping and sustained deadlines remain separate.
Execution is on the development machine, not an RPi. P1–P4 require deterministic
software correctness checks only: no performance measurements, latency/CPU/RSS
benchmarks, two-Pi trials or hardware qualification. Synthetic signal assertions
verify meter arithmetic; they are not device measurements.

### Source findings and fixed design

- [Mixer::process_interleaved](../../src/mixer.rs) already has raw samples, the
  actual crossfaded `strip.tick` result, mute/fader/pan sums and raw post-mute
  monitor sends. [EngineAuthority48::snapshot](../../src/mixer_control.rs) sets
  `meters: None`; `RenderedSnapshot::validate` rejects populated meters. Keep
  C-AUDIO/rendered1/2 unchanged; filling that field would break compatibility.
- [LocalAudio::tick_raw_with_brain](../../src/local_audio.rs) sequences raw analysis,
  mixer, pre-talkback operator capture, module composition and monitor talkback.
  [ModuleGraph::process_interleaved_brain](../../src/module_graph.rs) adds actual wet
  return and FOH talkback before PA. Main meter must observe that sum, not the
  dry mixer output or `program_before_talkback`. No new PA/FX/REC ABI is needed.
- [ProcessingSnapshot](../../src/processing_wire.rs) and
  [channel processing observation](../../src/channel_processing.rs) already report
  detector gain reduction in nonnegative milli-dB, unavailable during transitions,
  bypass or fault. Preserve it and its independent freshness; it is not a level.
- [Paired readback](../../src/paired_readback.rs) is a strict committed raw/Brain
  authority snapshot. Meter reads must not modify it, its read-admission latches,
  processing freshness, lease maintenance, held proofs or mutation retry history.
- Desk's [provider decoder](../../../shr-desk/src/provider.rs),
  [remote connection](../../../shr-desk/src/remote.rs) and
  [frontend worker/Update](../../../shr-desk/src/frontend.rs) separate observations
  from commands. [render.rs](../../../shr-desk/src/render.rs) fixture meters are
  simulator-only. [modules::Worker](../../../shr-desk/src/modules.rs) supplies a
  bounded separate-worker precedent, but receipt-based freshness there is not
  sufficient for meters. Use a dedicated meter observer, not the control poll loop.

The following are the implemented GP-METER v1 boundaries. Acceptance evidence
and the remaining owner-artifact gate are recorded below:

| Tap ID / UI label | Exact sample boundary |
|---|---|
| `input ID + raw` / Input raw | Logical mapped f64 input before EQ, mute, fader; same source as raw REC/analysis, without modifying either |
| `input ID + processed` / Channel pre-fader | Actual `strip.tick` result including transition blend, before shared mute/fader/pan; never target coefficients |
| `main-l`, `main-r` / Main pre-PA | Actual dry + accepted wet + authorized FOH talkback immediately before PA input composition; without modules, actual dry main (do not invent wet processing) |
| `monitor ID` / Monitor bus | Each configured mono bus after shared mute/send/output safety gain and authorized monitor talkback, before physical patch/optional PA routing |

Main is explicitly **pre-PA**, not a physical output/protection meter; neither
meter nor clip proves ADC/DAC clipping, true peak, SPL or acoustic safety. PA output
metering is outside this slice. PFL/AFL/operator playback are unchanged and are
not substituted for these taps. Monitor IDs come from admitted topology/authority,
not screen banks or a fixed count. Carry physical/USB mapping through existing
structural readback rather than copying it into every observation.

Aggregate nonoverlapping 20 ms source windows (960 samples at 48 kHz; derive
`sample_rate / 50`, refusing nonintegral unsupported rates for telemetry only).
All samples contribute: peak=max(abs(x)), RMS=sqrt(mean(x*x)), clip count=number
with abs(x)>=1.0, including exactly full scale. Use overflow-safe scaled sum of
squares for finite over-range inputs. A nonfinite sample invalidates its tap for
that window; report invalid count and null levels, never measured silence. A graph
fault/quiescence invalidates affected processed/bus taps, even if safety emits
zeros. Valid muted output is measured zero. Raw evidence can remain valid when
only downstream processing faults. Publish only complete windows; discard partial
windows on gap, epoch/map change, reset or source recovery. Do not average across
identities or fill gaps. Existing DSP fault policy is unchanged.

Wire levels: integer milli-dBFS `round(20000*log10(amplitude))`, floor -120000;
include explicit `silent` (peak exactly zero), `below_floor`, and `over_range`
flags. Positive dBFS is valid. Peak/RMS null only for invalid/unavailable taps;
counts and sequence/frame/epoch/generation use decimal-string u64 counters.
Clip count is per window; Desk holds the red indication for one local second
only while data is fresh, with no provider latch/reset mutation. Missing windows
are visible loss, not a promise to capture every transient. No UI smoothing may
hide stale/invalid state or change numeric measurements.

New independent strict JSON contract `GP-METER:1`, `kind: meter_snapshot`:
read request names show/module/source epoch, query ID and expected map; writer,
lease, request_id and expected_revision are null. Reply echoes query and carries
show/module, source epoch, capability/map generation, topology identity/rate,
sequence, half-open `[first_frame,end_frame)`, acquisition age of the **first**
sample, publication loss count, validity/reason and ordered named tap records.
Remote reply remains inside the authenticated session envelope. No arbitrary
client-selected taps or new grants. A valid current authenticated reader may
observe the same inventory as existing snapshots; revoked/unpaired sessions fail.
Local binding retains private same-UID access. Unknown contract/version is a
bounded unsupported result; old providers stay usable with meters unavailable.
Old consumers never receive unsolicited new fields/messages.

Poll at most 25 Hz per observer, one outstanding query, total query/assembly
budget 100 ms; provider windows are 50 Hz. Provider computes acquisition age from
its own monotonic acquisition timestamp, never from latest request/serialization.
Desk conservatively adds the entire local query elapsed time, then elapsed time
since receipt: no comparison of unsynchronized host clocks. Freshness limit is
250 ms from window start. Keep original deadline across partial frames/pages;
repeated sequence may increase age but cannot reset its prior expiry. Reject
future/regressing frames, malformed windows, map/epoch/session mismatch, duplicate
IDs, unknown fields/keys, invalid numeric relationships and oversize inventories.
After identity change clear cached meters and await matching current topology plus
a complete new window. Reconnect/unsupported/stall cannot grant, renew, rearm,
recall or replay any mutation. A stopped source with responsive control becomes
stale; receipt alone never makes it live.

### Dependency-ordered delivery and acceptance

Each phase remains unchecked until its evidence is entered below. New files named
here are planned boundaries; existing links identify the inspected implementation.

**P1 — Provider contract and bounded acquisition (GigPies).** Add `src/metering.rs`
for prepared per-tap accumulators/window identity and `src/meter_wire.rs` for strict
request/reply validation; register in `src/lib.rs`. In `src/topology.rs` admission,
account for accumulators, two preallocated SPSC window slots and serialization
capacity using checked arithmetic and admitted counts. Use the existing
[analysis handoff](../../src/analysis_stream.rs) pattern, not its PCM protocol or Lux
subscriber limits. A full queue drops a whole meter window and increments loss;
never waits, allocates, locks, serializes, does I/O or retries on the render path.
Worker drains bounded slots and retains only the latest complete observation.
Strings, storage and tap maps are prepared off render; no per-sample strings/logs.

Instrument `Mixer::process_interleaved` for raw/processed taps and
`ModuleGraph::process_interleaved_brain` at its final main sum; finish monitor taps
in `LocalAudio::tick_raw_with_brain` after talkback. Add only bounded observation
hooks, with meter lifetime/reset owned by LocalAudio and source/map changes in
`quiesce_source`, `recover_source` and structural commit. Preserve arithmetic/order
and owner handles. Existing LocalAudio tick also performs control I/O; do not
misdescribe it as a realtime-safe callback or refactor the whole host. The new
sample hooks and handoff must be independently realtime-safe.

- [x] Analytic silence/DC/impulse/coherent sine and independent summation checks
  verify peak/RMS/clip/window boundaries, invalid/over-range behavior and reset.
  Amplitude .5 DC gives -6021 milli-dBFS peak/RMS; a whole-cycle .5 sine gives
  -9031 milli-dBFS RMS (rounding tolerance 1 milli-dB, floor checked separately).
- [x] Callback allocation guard plus bounded-capacity/drop tests prove no new
  allocation/lock/I/O; meter disabled, saturated and disconnected runs have exactly
  identical audio samples. Reject meter capacity independently of audio operation.
- [x] Add provider-produced synthetic `tests/fixtures/gp-meter/v1/` corpus with
  schema, actual source revision and SHA-256 manifest, including all refusal and
  identity/age cases. Additive contract documentation belongs in existing
  `docs/MODULE_CONTRACTS.md`; no second tracker.

**P2 — Provider delivery (depends on P1).** Extend LocalAudio's `Incoming` decode
and bounded query dispatch for the new contract; JSON/cache publication is outside
sample processing. Extend `src/remote/policy.rs::check_writer`,
`src/remote/host.rs::HostAuthority::dispatch` and existing remote framing in
`src/remote/wire.rs`/`src/snapshot_pages.rs` only where required. Reads must bypass
control admission/lease state changes in `src/remote/authority.rs`.
Keep 64 KiB frames and existing <=1 MiB/16-page immutable document bound, including
outer remote wrapper; meter assembly has the stricter 100 ms deadline. Admit only
inventories fitting the bound; oversized telemetry reports unavailable, never
silently truncates channels or rejects an otherwise valid audio topology.
Use existing finite connection/session capacities, no new per-client queues in
audio; one reply in flight, discard on timeout, immutable page identity throughout.

- [x] Actual private Unix and mutually authenticated QUIC requests decode the
  real provider corpus; unauthorized/revoked/wrong-session queries fail closed.
- [ ] Slow reader, partial page, queue saturation, duplicate query, missing window,
  source stall, epoch/map change and provider restart cannot refresh old data,
  splice pages, block audio or change lease expiry/readiness/revision/held proof.
- [x] Existing rendered/processing/paired fixtures and old client behavior remain
  accepted byte/schema-compatible. Freeze provider fixtures before Desk delivery.

**P3 — Desk observation and presentation (depends on accepted P2 fixtures).**
In `/home/shome/p/shr-desk`, add `src/metering.rs` codec/cache/observer worker,
registered in `src/lib.rs`; reuse `provider.rs` strict bounded document parsing and
`remote.rs::Connection` authenticated framing. Extend `src/local_audio.rs` transport
adapters only as needed for a separate read-only connection with no grant/renew.
A dedicated worker and latest-only UI mailbox prevent 100 ms meter timeouts from
stalling the existing control worker/50 ms held-action service. Respect the
existing 16-session deployment budget, including this observer; no automatic
capacity increase. Probe once per attach; unsupported stays unavailable until an
explicit new attach, while transient read failures recover without commands.

Extend `frontend.rs::Update`, `Frontend` scene construction and `render.rs` shared
meter primitives for actual banked input raw/processed selection, stereo main and
configured monitor buses. `native.rs::offscreen_at` consumes the same scene; leave
the simulator explicitly fixture-backed. Show tap label, peak/RMS, clip and age;
stale values may remain dimmed with STALE and age, invalid/unavailable use distinct
text, and silence uses an empty bar plus measured-silence indication. Keep GR
separate and independently aged. Meter recovery must not dismiss pending reviews,
change selection, synthesize input or restore held talkback.

- [x] Copy exact producer corpus/hashes into Desk `tests/fixtures/gp-meter/v1/`;
  test codec/freshness/recovery and legacy fallback before UI acceptance.
- [ ] Extend `tests/frontend.rs`, `tests/frontend_real.rs`, `tests/gp07_frontend.rs`
  and `tests/gp15_frontend.rs` as appropriate: all banks/buses, GR coexistence,
  clipping/silence/stale/invalid, context loss and held-action isolation.
- [x] CPU-native offscreen checks inspect actual provider-fed scenes at normal and
  resized viewports: readable labels, no clipping/overlap, no fabricated graphs.

**P4 — Integrated software acceptance (depends on P1–P3).** Extend existing
GigPies `tests/modular_engine.rs`, `tests/modular_owners.rs`, `tests/brain_owners.rs`
and `src/remote/tests.rs`; add focused `tests/metering.rs` in each changed owner.
Use finite synthetic signals with independently calculated expected values, not
another meter instance or decoder round trips as oracle. Exercise 16/17/32/48
inputs with independently varied monitor counts and nontrivial capture/output
maps, including last input/bus and bank boundaries. Check mute/fader/pan, EQ and
compressor transitions, actual FX return, FOH/monitor talkback and PA protection.

- [ ] Actual LocalAudio + owner PA/FX/REC artifacts → authenticated provider →
  independently built Desk client → native scene agrees with independent sample
  windows (1 milli-dB quantization tolerance; exact frame/clip counts). Run both
  local and remote delivery, not solely copied fixtures or a mock server.
- [ ] Compare every raw recorded PCM24 sample and analysis sample against original
  source-frame/input references exactly; check dry/protected output, PFL/AFL and
  no implicit talkback sidetone with meters attached, stalled and recovering.
- [ ] Source stop with control alive, Desk loss, worker loss, revoked session,
  source/provider restart, map change and delayed old replies become visibly
  stale/unavailable and resume only with fresh identity. Control leases still
  expire; no automatic grant/rearm/mutation replay. Preserve existing 150 ms
  held deadman, 50 ms heartbeat, 240-frame fade and 250 ms control freshness gates.

### Verification, evidence and exact next action

Before execution (2026-10-09), planning inspected the named seams and updated
only this card. The authorized execution preserved that existing planning diff
and updated this same record; no conflicting documentation edits were found.
During execution run focused normal meter tests first, then full normal default
and `hardware-host` suites in GigPies and default/`native` suites in Desk because
render/schema/shared state changes require them. For each configuration run
`cargo +1.97.1 test --locked -j1 --all-targets`, warnings-denied Clippy and release
build, plus `cargo +1.97.1 fmt --check` and each owner's publication Python checks.
Prefix every Cargo invocation with
`flock -xn /home/shome/p/.gigpies-build.lock env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0`;
serialize owners/configurations, keep normal targets, inspect disk per Desk policy.
Do not weaken deadlines or omit failed tests; record failures and skipped classes.

Actual owner/provider drivers are explicit finite development-host opt-ins using
reviewed artifact hashes and isolated synthetic loopback/private Unix endpoints.
No remote RPi endpoint or lab reservation is needed for this slice. Follow
[modular acceptance](../acceptance/MODULAR_ENGINE_ACCEPTANCE.md),
[Brain acceptance](../acceptance/BRAIN_AUDIO_ACCEPTANCE.md) and
[Desk development](../../../shr-desk/docs/DEVELOPMENT.md). Run required owner cases via
`--test modular_owners -- --include-ignored` and
`--test brain_owners -- --include-ignored` with their documented artifact inputs.
Native rendering uses inspected `/usr/share/vulkan/icd.d/lvp_icd.json` via
`VK_ICD_FILENAMES`, DISPLAY/WAYLAND_DISPLAY unset, and `native::offscreen_at`.
No physical PCM/MIDI/DMX/service activation. Full-show/load, real displays/controllers,
ADAT/socket/acoustic and listening gates require separate authorization; historical
music/research/exhaustive matrices remain opt-in and outside this slice.

| Phase | Implementation | Evidence / next action |
|---|---|---|
| P1 | IMPLEMENTED / software-validated | Analytic windows, allocation/drop guard, exact audio comparison, independent meter admission and frozen provider corpus |
| P2 | IMPLEMENTED / software-validated | Actual private Unix and mutual-TLS QUIC reads, revocation/read-only admission; strict versioned corpus and page/deadline bounds |
| P3 | IMPLEMENTED / software-validated | Independent codec/cache/observer; 20 fresh actual provider-fed scenes, 40 exact CPU-native RGBA comparisons; FOH topology attachment corrected |
| P4 | Independent dry software checks validated; owner-artifact acceptance BLOCKED | Actual 16/17/32/48 LocalAudio and independently built Desk/native driver pass; reviewed GP05 PA/FX/REC manifest absent |

Execution results (2026-10-09), local development machine:

| Owner/configuration | Complete normal all-target suite | Warnings-denied Clippy | Release build |
|---|---|---|---|
| GigPies default | 399 passed, 17 explicitly ignored | PASS | PASS |
| GigPies `hardware-host` | 476 passed, 32 explicitly ignored | PASS | PASS |
| Desk default | 249 passed, 13 explicitly ignored | PASS | PASS |
| Desk `native` | 251 passed, 13 explicitly ignored | PASS | PASS |

All used the exact locked serial command prefix above. Feature rows add
`--features hardware-host` or `--features native` to test/Clippy/build; Clippy
uses `--all-targets -- -D warnings`. Both `fmt --check` commands passed.
Publication Python regressions passed (GigPies 6, Desk 9); real complete indexes
and disposable candidate indexes including every named new file passed the guard.
Real indexes remain untouched. Local document targets and `git diff --check`
passed; source-linked knowledge routing and validation passed. Normal native
suites had display/ICD discovery unset; the explicit driver below selected CPU.

Provider evidence: analytic silence/DC/sine/impulse/floor/full-scale/over-range,
nonfinite and extreme-finite RMS; allocation/deallocation guard, two-slot drops,
partial/gap/reset discard, independent meter admission and exact unchanged audio
with capture disabled/saturated/disconnected. Actual mapped LocalAudio references
cover 16/3, 17/1, 32/5 and 48/7 input/monitor configurations. Actual private Unix
and mutual-TLS QUIC queries pass without control admission/revision changes;
revocation fails closed. The focused authenticated stopped-source test also
passed after adding a second read with rendering stopped: sequence/frames/taps
stay identical and first-sample age exceeds 250 ms while control remains alive.
Acquisition is stamped at block entry, before control servicing.

Consumer evidence: exact copied producer corpus; strict codec, query/identity,
regression/overlap, repeated-window expiry, 100 ms assembly deadline, page identity,
16-page refusal, one-second clip hold and independent input/monitor banks.
Meter state uses `Frontend::meters` and its own latest-only mailbox; the control
`Update` and held-action worker retain their existing ownership. Admitted
rendered-v2 topology supplies FOH inventory; structural readback is a fallback.
F11 opens meters, F12 switches raw/pre-fader, PageUp/Down selects monitor banks;
GR retains its independent processing age. Legacy rendered/processing/paired
fixtures and their null meter fields are unchanged.

Explicit opt-ins passed: GigPies `--test metering -- --ignored` with
`GP_METER_FIXTURE_OUT=tests/fixtures/gp-meter/v1` and
`GP_METER_TOPOLOGY_DIR=/home/shome/p/gigpies/artifacts/gp-meter/topologies`
(two exporters). Desk `--features native --test metering
actual_local_provider_native_scene -- --ignored --nocapture` used
`GP_METER_PROVIDER=/home/shome/p/gigpies/target/release/gigpies-headless`, that
same topology directory, `GP_METER_CPU=1`, and
`GP_METER_EVIDENCE_DIR=/home/shome/p/shr-desk/artifacts/gp-meter/screens`.
DISPLAY/WAYLAND_DISPLAY/VK_ADD_DRIVER_FILES were unset; VK_DRIVER_FILES and
VK_ICD_FILENAMES both named the inspected `/usr/share/vulkan/icd.d/lvp_icd.json`.
The independently built client consumed five actual private providers: 16/3,
17/1, 32/5, 48/7 and 17/13, with reversed capture/output maps. Independent
DC/dry-main/each-monitor peak/RMS, exact 960-frame windows and zero clip/invalid
counts passed. First/last inputs in both tap modes yielded 20 fresh scenes and
40 exact RGBA matches at 1920x1080 and 1280x720. Source-child stop made meters
stale. Retained normal/last-input/second-monitor-bank images were visually checked
for readable labels and distinct measured silence; scene bounds checks passed.

Provenance: `tests/fixtures/gp-meter/v1/manifest.json` in both repositories is
byte-identical, SHA-256
`5c72ac8bcb6de1710a3c01acc07fc4655a4ea2915c06df27d60e06b44732f0b7`.
It records the actual producer base revision plus every changed source witness
and fixture hash, explicitly marking this uncommitted slice. Final provider
executable SHA-256 is
`f77f91a006e18f87d24bd4dcde2b0ea594e17225d55a1c470880250525e78cd8`;
final native Desk release SHA-256 is
`6cb95e6bcc6fb9f8c8dc6580fddad9bcc30a9b36523f42d4d00c5e17ff2833ba`.
Each owner's ignored `artifacts/gp-meter/evidence.json` retains source/lock,
executable, corpus and log hashes; Desk also retains the actual native test
executable hash and 20 scene hashes. Commands/results and available failed logs
are under each owner's `artifacts/gp-meter/logs/`.

Failed attempts were resolved without changing production deadlines: a test
used nonexistent `Mixer::clone`; two constructed test fixtures had inconsistent
level/flag relationships or skipped monitor IDs; Clippy found nested conditionals and an
explicit eight-argument snapshot boundary. Tests/style were corrected. An
exporter invocation omitted its required output paths and was rerun explicitly.
48/13 failed existing control-snapshot admission; admitted 17/13 proves the second
monitor bank. The actual native run caught FOH waiting for structural metadata;
it now uses admitted rendered topology. CPU inspection paused the UI pump, so
the driver now awaits a fresh observation before each frozen scene. Final runs
passed. Disk review found ample free space; shared caches/evidence were preserved.

**BLOCKED artifact-dependent P4:** no reviewed `GP05_MANIFEST` (PA/FX/REC owner
artifacts) was supplied/found; `GP_PA_V2_FIXTURES` and `GP14_PA_FIXTURES` are not
configured for this session. Required `modular_owners`/`brain_owners
-- --include-ignored` drivers, actual owner wet/protected/REC/analysis sample
comparisons and owner-backed Desk local + remote acceptance were not run.
Mock-owner normal suites and dry-provider/native evidence do not replace them.
The corresponding P4 and comprehensive fault/recovery checkboxes remain open.
Other historical/exhaustive, hardware, listening, full-show/load and measurement
opt-ins were intentionally skipped under this card's scope. No PA/FX/REC ABI,
owner algorithm, device/service, dependency, commit, push or deployment changes.

Remaining uncertainties are verification gates, not open design alternatives:
inventory serialization bounds must pass deterministic size assertions in P1/P2;
freshness and timeout behavior use controlled clocks/fault injection, not loaded
latency measurements. Accepted development-host owner artifacts must be available
for P4. If absent, mark that artifact-dependent acceptance BLOCKED here and finish
independent software work. RPi performance and hardware qualification are deferred
to separately authorized sessions and do not gate development completion. No owner algorithm
change is anticipated; a required PA/FX/REC ABI change stops this slice for review.

Exact next action: supply the reviewed PA/FX/REC manifest and owner fixture paths,
then execute the remaining P4 owner drivers and owner-backed local/remote sample
acceptance. Independent development-machine software work is complete. Preserve live dirty
work, stage only named files if committing, and follow both owners' publication
guards. Deliver provider before consumer; Desk coordinator owns upstream pushes
under its continuation policy. The subsequent user instruction authorizes source commits and pushes for this
validated slice. Deployment and hardware activation remain excluded.
Knowledge already routes to this card; maintain that route without copying state.

### Standalone execution prompt

```text
Implement GP-METER P1–P4 in /home/shome/p/gigpies and /home/shome/p/shr-desk,
using the GP-METER card in gigpies/docs/MODULE_IMPLEMENTATION_PLAN.md as the
complete design and sole progress/evidence record. This instruction authorizes
scoped runtime changes in those two repositories and finite synthetic local/
authenticated loopback software acceptance on the development machine, not RPi.
No performance/device measurements, benchmarks, two-Pi trials, physical devices
or service deployment; use deterministic signal, capacity and clock assertions.
Read each AGENTS.md, the card and linked acceptance/contract sources; load the
GigPies zk index and inspect live Git before editing. Preserve other workers' work.
Deliver provider contract/acquisition, bounded authenticated observation, actual
Desk presentation, then independent 16/17/32/48 source-to-Desk/native/raw-sample
acceptance in that order. Preserve all authority, timing, raw REC/analysis,
PA/FX, PFL/AFL and talkback behavior; never refresh old audio from receipt time.
Run focused then required whole suites serially under the shared build lock,
Rust 1.97.1 --locked -j1 and CARGO_INCREMENTAL=0. Record exact evidence and opt-ins
in the card. Stop dependent work on incompatible source drift, required new owner
ABI, missing development-host artifact or a failed safety/identity gate; finish
independent checks without weakening limits. No new tracker, broad refactor,
dependency upgrade, FFT, automix, scenes, writable FX or new device owner.
Keep other siblings read-only; no model selection or agents. Local changes only:
report reviewed diffs/evidence and remaining gates; do not push, deploy or activate
hardware. Provider must be accepted before consumer publication in a later
explicitly authorized publication session. Update source-linked knowledge only
when needed and run /home/shome/Documents/knowledge/.zk/validate.sh.
```

## GP-FX — writable owner effects in the console

State: SOFTWARE VALIDATED (2026-11-06). Owner clarification: four stereo FX auxes,
each with per-input post-fader/post-mute sends, one selected wet effect and a
return to Main. The backend extension below adds independent wet sends to the
existing configurable monitor buses, which still have no return to Main. This does not expose SHR FX's standalone A/B slot rack.

Backend return-routing extension (November 6, software validated): each wet aux
now has a Main-only host gain and independently ramped sends to every configured
mono monitor. New monitor routes default silent; the shared SHR FX wet gain and
owner/tail semantics remain intact. The tap, strict opt-in contract, compatibility,
source readback and later Desk requirements are specified in
[REMOTE_FX_CONTROL](../reference/REMOTE_FX_CONTROL.md#independent-wet-return-routing-opt-in-readback-extension-1).
Desk consumer acceptance is recorded below; saved shows remain separate. GP-H1 local
T0–T2 acceptance remains complete; no extraction or T3 work was restarted.

Implementation/evidence for this extension uses canonical rpi5 `main` at
`830ec26458092fc474223c3b6e262eea1d79273f` plus this extension. All nine
PA/FX/REC library/header/owner-manifest identities match the existing approved
`user/node-lab/gp-h1-modules.json` (SHA256
`6765d4fbd73645cedc5b8f2ac4dbbdf9b660fae9b8258940aaace542c239c9e1`).
No owner algorithm, ABI or sibling source changed.

Normal regressions cover 0/1/3/5/17 monitors, exact synthetic sums and gains,
anti-phase fold-down, 240-frame ramps, default-silent compatibility, invalid
shapes/ranges, source-boundary allocation/destruction freedom, cancellation,
expiry, closed writer/owner fences, selected-monitor listening without excitation
feedback, and runtime-only recovery/restore. Actual local acceptance additionally
covers stale lease/revision/epoch, refusal, fault/panic, disarmed recovery/rearm,
legacy omitted-routing edits and Main mute with continuing monitor wet. Route-only
edits leave the actual owner's accepted request unchanged. That acceptance found
and fixed an existing recovery observation gap: inherited aux targets are now
copied into authoritative readback before the rebuilt source is detached.

Validation uses Rust 1.97.1, locked dependencies, one job, incremental disabled,
normal targets and the nonblocking shared `/home/shome/p/.gigpies-build.lock`.
Complete normal all-target suites passed: **hardware-host 692 passed/80 ignored;
default 557 passed/22 ignored**, zero failures. Both all-target Clippy
configurations passed with warnings denied. Formatting and whitespace checks
passed. Final optimized selection: **7 passed**, comprising two strict wire/
compatibility checks and five actual-owner cases: local authority/samples/fault/
recovery, clocked remote publication under controller stall, grouped four-aux
samples/loss, actual PA/FX/raw REC under saturated source return lanes with exact
PCM24 tails and epoch rearm, and authenticated four-aux TLS samples. Both local
and remote routes use the existing owners. Approved artifact identities were
rechecked before the final run; none changed.

Reproduce with the shared lock/environment above:
`cargo +1.97.1 test --locked -j1 --all-targets -- --test-threads=1` and the same
command with `--features hardware-host`; both
`cargo +1.97.1 clippy --locked -j1 --all-targets [--features hardware-host] -- -D warnings`.
For actual acceptance set `GP_FX_LIBRARY=/home/shome/p/shr-fx/target/release/libshr_fx.so`,
`GP05_MANIFEST=/home/shome/p/gigpies/user/node-lab/gp-h1-modules.json` and
`GP_PA_CONFIG=/home/shome/p/shr-pa/tests/fixtures/cpa/v2/stereo4way.json`, then run
`cargo +1.97.1 test --release --locked -j1 --features hardware-host --lib -- fx_aux actual_clocked_pa_fx_rec_survives_controller_stall_and_finalizes --include-ignored --test-threads=1`.
Unchanged historical/exhaustive/audition, long ASRC and unrelated artifact opt-ins
were intentionally skipped. No fixture generation was enabled.

Closeout: after all build/test tools were idle, checked the shared nonblocking
lock, live Git state, processes, open files/maps, inode/hardlink identity and
runtime symlink references. Removed 156 exact task-built test executables and the
temporary validation log, reclaiming **14,441,603,072 allocated bytes (13.45 GiB)**.
No referenced candidate needed retention. The canonical target retains **9.7 GiB**
of reusable dependency/debug and runtime artifacts; **41 GiB** is free. Test
fixtures cleaned up through their existing guards. Source, user data and runtime
artifacts remain; no cleanup-triggered rebuild was run. Implementation and
validation used the original checkout and branch. The subsequent user-authorized
commit/push preparation passed all 67 publication-policy tests; versioned commit
and push guards remain enabled.

Desk return controls (November 6, canonical rpi5, uncommitted software work):
implemented against backend `60b1fb95b1fdb3a47e70ffef1a177ab2ad70b4e4`
and Desk base `383d0f5c40827f12c044981fbae279488ea130ec`. No backend production
behavior or effect owner was changed. The production Frontend now exposes
Main-only and every configured monitor return for each of four auxes through
existing keyboard/controller/menu actions, with a moving monitor window and
complete reviewed targets. Shared SHR FX wet gain remains separate. Strict opt-in
decoding validates version, ordered identities, dimensions and finite levels.
Explicit legacy read preserves older palette/send workflows; transport/auth errors
never imply legacy support and mutations are not capability probes.

Settlement requires correlated acceptance followed by fresh exact applied targets,
host generation and owner state, including source/epoch/map/library/session fences.
Route-only changes need not advance DSP requests. Reconnect stays read-only,
never replays, grants or rearms; unknown outcomes require explicit reconciliation
and new review. Source/owner replacement invalidates old reviews. Pending,
refused, UNKNOWN, stale and disarmed states remain visible. Actual testing also
exposed and fixed Desk's stale raw-telemetry dispatch: an old observation is now
consumed without renewing freshness, rather than misclassified as a mutation reply.

Bounded acceptance uses the unchanged approved GP-H1 artifacts (all nine hashes
rechecked). Actual Desk Frontend keys/review/confirmation reach applied return
readback through local and authenticated loopback remote providers, with all four
auxes at Main 0.95, monitor-1 0.05 and monitor-2 0.10. The local source harness
checks 192 independent Main/monitor samples; remote acceptance checks 17,568 Main
samples and matching monitor outputs with zero native input/output drops.
Untouched settings/sends/other auxes and unchanged DSP request IDs are checked.
Read-only reconnect preserves the applied routes. Frozen actual producer snapshots
and exact reproduction commands live in Desk's
[fixture README](../../../shr-desk/tests/fixtures/gp-fx-aux/v1/README.md).
The final headless 16-input Frontend episode also passed all 24 reviewed edits,
including existing palette, send, tempo, bypass, panic and cancellation paths.
Snapshot SHA256: local
`f6f993e05e3dfaf0324fd0e9e9cfdfb185578b29f4851b13b4c7e2105c55dac0`;
remote `f25c38ed0460ff82c65c1fce86ba9cc299da01517a8f33408e58da031840836e`.
These are local software results, not hardware or two-node acceptance.

Four existing actual-owner regressions also passed: local authority/refusal/panic/
disarmed recovery/rearm and independent samples, clocked remote publication during
controller stall, four-aux source/Brain samples/loss, and PA/FX/raw REC with exact
PCM24 tails and epoch rearm. They preserve dry/Main-at-default/raw REC and the
operator-monitor semantics. Normal Frontend tests exercise rendered controls with
0/1/3/5/17/64 monitors and every aux, untouched-field preservation, stale/UNKNOWN/
refused/pending/disarmed displays; transport tests cover lost completion, wrong
target/owner, malformed reads, explicit legacy mode and replacement fences.

Desk validation uses Rust 1.97.1, locked dependencies, one build job,
`CARGO_INCREMENTAL=0`, normal targets and the nonblocking shared build lock.
Complete normal all-target suites passed: **default 383 tests / 29 ignored;
native 386 tests / 29 ignored**, zero failures. Native checks used CPU lavapipe
with display variables removed. Both all-target Clippy configurations passed with
warnings denied. Both release builds and formatting checks passed.
GigPies' six focused FX integration tests and hardware-host library/tests Clippy
with warnings denied also passed. Its production backend is unchanged, so its
complete normal default/hardware-host suite results above remain the backend
baseline; this task added only opt-in acceptance harnesses. Unchanged historical,
exhaustive, audition and unrelated actual-provider opt-ins were skipped.

Desk task closeout: all tools were idle; the shared lock, Git state, processes,
open files/maps, inode/hardlink identities and repository/runtime symlinks were
checked. Removed 82 exact task-run test executables and three temporary logs,
reclaiming **9,234,976,768 allocated bytes (8.60 GiB)**.
The two actual producer snapshots (about 34 KiB), reusable source/tests/docs,
approved module artifacts and required runtime executables remain. No cleaned
output was rebuilt. No scratch workspace or one-use helper was retained.
Normal targets retain approximately 13 GiB (Desk) and 9.8 GiB (GigPies) of reusable
dependencies/debug and runtime artifacts; approximately 41 GiB is free.

Persistence dependency: [GP-SHOW](#gp-show--audio-scenes-and-operator-recovery) owns the separate saved FX payload, bundle restore and Desk lifecycle; the return-control acceptance above did not validate persistence.
GP-H1 retains separately authorized T3 distributed timing/headroom and physical
mapping, protection, listening and sustained-load qualification. Local T0–T2
acceptance remains complete. No physical activation, playback, two-node traffic,
queue/deadline changes, host tuning, commits, publication or deployment occurred
in this Desk task. Owning consumer usage and contract details are in
[Desk FX](../../../shr-desk/docs/FX.md) and
[Desk control contract](../../../shr-desk/docs/CONTROL_CONTRACT.md).

| Dependency | Owner / deliverable | Acceptance |
|---|---|---|
| 1 | SHR FX native-f64 prepared palette, FX-02/03 | Actual C caller wet samples and lifecycle; existing evidence |
| 2 | GigPies four bounded aux sends and independent effect owners | Isolation, wet-only output, tails, publication, faults, raw REC/monitor preservation |
| 3 | GigPies scoped authority and versioned owner readback | Stale/refused edits, accepted versus applied, disconnect and shutdown |
| 4 | Desk send/return and palette controls | Actual producer, reviewed edits and fresh settlement; normal default/native suites |

Single-thread implementation in canonical checkouts; no peers, commits or physical
activation. Baselines: GigPies `f277a981c135f984172c868e5d2768494b34e99e`,
Desk `a42b91748895891cd6c344016b9e4ee5f4d10953`; preserve the existing SHR FX
working changes on `703be52ed3fffde053760ae0f6ac4378b46989df`.
Rust 1.97.1, locked dependencies, incremental off, normal targets, serialized
parent-held `/home/shome/p/.gigpies-build.lock`. Focused checks precede complete
normal production suites; physical aggregate performance and listening remain
separate acceptance. The existing fixed-delay compatibility path is retained.
Provider design and DSP belong to [SHR FX's plan](https://github.com/gigpies/shr-fx/blob/main/docs/GIGPIES_IMPLEMENTATION.md).

Implemented contract: [GP-FX-aux:1](../reference/REMOTE_FX_CONTROL.md), four
independent generic `shr_fx_v2_*` owners. Native f64; settings/status/capabilities/
parameter sizes 64/200/48/48 bytes. Existing v1 and GP21-specific delay adapters
remain separate. Stagebox sends four stereo auxes to Brain's four palette owners;
eight wet channels return to Stagebox Main before PA. Raw REC and monitor buses
retain their paths. An explicit source-host variant remains compatible.
Aux configuration is runtime state, not yet part of saved shows. Source recovery
retains sends but keeps outputs disarmed; Desk reconnect retains sound read-only.

Consumer wiring order is now implemented: hash-pinned owner admission → four
post-fader sends and wet returns → scoped boundary publication/readback → Desk
palette/parameter/tempo review → actual producer/sample checks. To use it later:
configure the exact library/hash in the Brain four-aux runner, attach Desk and explicitly
probe/grant FX control, then review an effect/send/return change. No automatic
activation, writer regrant or replay. Qualify aggregate CPU, physical mapping,
latency and dry+wet listening in a separately authorized session.

Validation findings: the initial full-suite run exposed a core resource-capability
regression; optional aux admission now leaves that published contract unchanged.
Recovery also retains actual mixer send targets, matching owner readback. The
actual Desk episode exposed a stale raw/FX snapshot pairing; paired fresh queries
now enforce the same shared authority revision. Panic remains available during
faulted or interrupted transitions and reports actual retained settings.


Verified software evidence (2026-10-29–30):

- Complete GigPies default normal suite: **490 passed, 21 opt-ins ignored**;
  hardware-host normal suite: **575 passed, 53 opt-ins ignored**. Desk
  default: **361 passed, 27 opt-ins ignored**; native: **364 passed, 27 opt-ins
  ignored**. All four complete `--locked --all-targets -- --test-threads=1`
  suites passed without deadline changes; native checks selected CPU lavapipe
  with display variables unset.

- Final actual local authority/sample/fault/panic/recovery and four independent
  palette-owner lifecycle tests also pass against the hash-pinned native library.
- Final optimized GigPies headless and both Desk default/native release builds
  passed. Formatting, changed Markdown links and `git diff --check` passed;
  Python publication checks: GigPies 50, Desk 9. Complete-index guards passed
  (770/554 tracked entries); these checks do not claim publication of unstaged work.
- All-target Clippy passes with warnings denied for GigPies default/hardware-host
  and Desk default/native. Focused normal tests cover exact legacy capabilities,
  four sends, monitor preservation, source recovery, allocation freedom,
  malformed-client isolation, explicit media admission and actual producer codecs.
- Optimized `actual_four_aux_source_brain_roundtrip_and_loss` passes actual f64
  owner processing through grouped send/return packets, exact dry+wet summing,
  stale/wrong-writer refusal, late preparation after Desk disconnect, and bounded
  return fade after Brain loss. Other effects continue after an unpermitted edit
  is cancelled; native owner state remains exact.
- Optimized `actual_four_aux_tls_brain_controls_and_samples` passes real
  authenticated loopback QUIC with delay, plate, tape echo and bright exciter:
  627 actual Brain blocks, 60,576 exact Main comparisons, all four wet energies
  nonzero and monitor output unchanged. Existing two-second control/lease and
  1500ms preparation deadlines are unchanged. The initial debug-only episode
  exposed the missing read-only command registration (fixed and regression-tested)
  and then timing/lease maintenance limits; optimized acceptance renews the lease
  explicitly and completes in the ordinary bounds.
- The final actual Desk Frontend episode passes against the rebuilt optimized
  synthetic GigPies executable: send edits, direct tape selection, musical timing,
  a second aux, bypass/panic, cancellation and read-only reconnect (5.06s).
- Desk's versioned `gp-fx-aux/v1/brain-owner.json` is an actual producer snapshot
  from that authenticated episode; `owner.json` retains the local-mode fixture.
  Normal tests verify both and fence changed media-session bindings.

Reproduce actual transport checks under the shared build lock with
`GP_FX_LIBRARY=/absolute/libshr_fx.so cargo +1.97.1 test --release --locked
--features hardware-host --lib actual_four_aux -- --ignored --test-threads=1`.
Exact provider/contract identity for these checks (uncommitted working trees on
the baselines above):

- `shr-fx/target/debug/libshr_fx.so` SHA256
  `ef18cec931a1483fdab49ffc136ada5424de696d3807ecd9352cc69e190fc89a`.
- `shr-fx/include/shr_fx.h` SHA256
  `e3fb2b267d5d7632ea3d7c5fdc722b9ffeb8e2cad772b04a696db807df21823f`.
- Desk actual local `gp-fx-aux/v1/owner.json` SHA256
  `d5b59898a5ef1ccdc2568efd2abb0eb2bcbbdb42a119b725e5d812d47e261bbd`;
  actual Brain `brain-owner.json` SHA256
  `a80137ba37696cdaafb5958fc4e56c1830d2a9fae3b880895c3605115420943b`.

- Final `gigpies/target/release/gigpies-headless` SHA256
  `ae0ec8508f520ff8b66cda231fd67da7890c5dce67c501cc5b4578729aee2b1d`.
- Final native `shr-desk/target/release/shr-desk` SHA256
  `246e73fe7adaf4ab45a11f068f12a0a73f859c1579b77ffdc590e0cf85104df7`;
  preceding default-feature build SHA256
  `9dff7201042ebeb4627017e3ea04b7fef71bc40291d5a4cee8c0503547b456d7`
  (replaced in the normal target path by the native build).

Additional actual-provider reproduction, using the same lock/environment:
`cargo +1.97.1 test --release --locked --features hardware-host --lib
actual_aux_authority -- --ignored`, then `cargo +1.97.1 test --locked
--features hardware-host --test fx_aux -- --ignored`, with `GP_FX_LIBRARY` set.
Desk: `GP_FX_PROVIDER_EXE=/absolute/gigpies-headless GP_FX_LIBRARY=/absolute/libshr_fx.so
cargo +1.97.1 test --locked --test fx_aux_actual -- --ignored --test-threads=1`.
All use owned synthetic endpoints; child processes and private temporary
directories were removed by their test guards. No task scratch runner, render,
log or core remains; final cleanup reclaimed 0 additional bytes. Normal reusable
outputs reviewed and retained: GigPies 17GiB, Desk 13GiB (debug symbols and both
feature suites plus release artifacts), FX 1005MiB; 36GiB free. No active build
or synthetic child remained, and no unrelated cache was removed.

No provider blocker remains for this bounded software path. Saved-show aux
persistence belongs to GP-SHOW; live aggregate admission and listening remain
separate gates, not implied by these tests.

Fixtures and normal tests require no physical endpoint. Historical auditions,
exhaustive research, load benchmarks, device activation and listening are excluded.

## GP-SHOW — audio scenes and operator recovery

State: IN PROGRESS overall. The first bounded slice is implemented and software-
validated: portable four-aux FX intent and reviewed, output-quiesced restore through
the production SHR Desk Frontend. Full audio scenes remain open. This task's new validation must stand on its own; GP-FX and GP-H1
acceptance above is baseline evidence only.

Implementation uses a separate `gigpies-fx-intent:1` file, with exactly four complete
configurations (all owner settings, tempo, input sends, Main and configured monitor
returns). Show UUID, exact audited library identity, ordered input IDs/capture slots/
physical-port labels and logical monitor IDs establish compatibility. No automatic
remap or legacy defaults. Current producer monitors are logical `monitor-N` buses
within the show namespace, not persistent device/person UUIDs. A changed count or
logical identity is incompatible. Physical output repatching does not rename the
logical monitor. Reusing every identity for a different musician/physical device is
not detectable by this producer model; no physical identity qualification is claimed.
Input ID, capture slot or physical-port replacement refuses even at unchanged count.
Physical-port labels preserve the producer’s nonempty, at-most-128-byte strings
(including spaces/case); they are compared exactly, not normalized or treated as
machine device-path configuration.

C-SHOW remains compatibility metadata; its optional C-FX module can reference the
canonical payload SHA256. Existing core/composed intent is independent and excludes
FX. Neither loading nor this restore invokes startup-only composed restoration.
Runtime histories/tails, grants/leases/epochs/generations, pending work, armed state,
held talkback, recorder execution, credentials and device paths are unrepresentable
in the saved payload. The file directory is machine-local configuration.

A single existing FX authority request carries the complete bundle. Local and remote
preparation validate all four before publication. Publication preflight is restricted
to the approved GP-H1 FX library hash; the audited native ABI refuses invalid pointers,
stale/duplicate request or revision, pending/retired/transition capacity and incompatible
rate/block shape. Typed token provenance fixes pointers/rate/block; serialized owners,
checked counters/revisions, settled status and reserved retirement wrappers discharge
all refusal conditions before the first publish. No native allocation/destruction or
JSON occurs on the source boundary. An unexpected native failure after this preflight
is uncertainty, never atomic-refusal or rollback evidence.

One bundle is outstanding; four prepared tokens plus four active and four retired
objects bound native storage at 96 MiB, plus existing owner structures and bounded
routing/configuration storage. Existing queue capacities, deadlines and DSP owners
are unchanged. Cancellation before publication retires prepared storage on control;
postpermit interruption/owner loss may leave partial application and is UNKNOWN.
Remote source publication retains a quiescence interlock until completion/retirement,
so concurrent rearm cannot pass its boundary while restoration is pending. Remote
bundles require the production autonomous source executor; controller-driven remote
compatibility execution refuses this new operation.

Desk provides explicit capability probe, sixteen file slots, save/replace, select,
load, full current/saved review, cancel and separate confirmation through existing
semantic menu/controller/input and paginated review. Slot capacity is a bounded UI
library, not an input/monitor product limit. Loading only reads the file. Save uses
fresh settled authoritative readback and durable temporary-write/fsync/rename/directory-
fsync; errors after replacement report durability uncertainty. Confirmation requires
an explicit FX grant, fresh identity/revision/lease and quiesced outputs, then correlated
acceptance plus exact fresh four-aux targets/generations/settlement. Lost completion
never becomes success from matching values. Reconnect is read-only; rearm remains a
separate reviewed action. Core mix, PA, lighting, recording, talkback and human holds
are outside this transaction.

Validation (2026-11-06, canonical rpi5; current uncommitted source, not baseline):

| Final normal all-target suite | Passed | Ignored opt-ins | Binaries |
| --- | ---: | ---: | ---: |
| GigPies default | 559 | 22 | 77 |
| GigPies hardware-host | 694 | 85 | 80 |
| SHR Desk default | 388 | 30 | 41 |
| SHR Desk native | 391 | 30 | 41 |

All four all-target Clippy gates (`-D warnings`), all four release builds and both
format checks passed. Every Cargo gate used Rust 1.97.1, `--locked -j1`,
`CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1`, normal targets and the shared nonblocking
`flock -xn /home/shome/p/.gigpies-build.lock`. Commands were `cargo +1.97.1 test
--locked -j1 --all-targets`, `cargo +1.97.1 clippy --locked -j1 --all-targets --
-D warnings` and `cargo +1.97.1 build --locked -j1 --release`, adding the indicated
feature before `--`. Native tests unset DISPLAY/WAYLAND_DISPLAY and explicitly
selected `/usr/share/vulkan/icd.d/lvp_icd.json` with both VK_DRIVER_FILES and
VK_ICD_FILENAMES. Focused saved-contract tests passed in both repositories (two
each), including complete non-null tempo and producer-compatible long/cased/spaced
port labels; all four complete gates were rerun after that compatibility correction.

All nine approved GP-H1 artifact hashes match. Publication regression tests
(6 GigPies, 9 Desk), complete-index/HEAD guards (804/568 entries), changed/untracked
worktree policy checks (16/21 files), 98 relative file links and 55 anchors passed.
No dependency, sibling owner or approved artifact was changed. Source branches
remain `main`; no commits/publication, worktrees or delegated sessions were used.

Frozen source evidence: GigPies HEAD `60b1fb95b1fdb3a47e70ffef1a177ab2ad70b4e4`
and Desk HEAD `383d0f5c40827f12c044981fbae279488ea130ec`, each with the preserved
uncommitted implementation. SHA256 tree digests over sorted unique Git-known plus
untracked nonignored `src/`, `tests/`, Cargo.toml and Cargo.lock files (feed each
relative UTF-8 path, NUL, then its binary SHA256) are:

- GigPies, 589 files: `978ed625b504f1039c02490e3d7115157a68051a2c00db21a311d3bbf65af784`.
- Desk, 484 files: `dc62f9dab3faae93416e8a9c6d6513fdb0a74e7344698f9c4b900897816e11e7`.
- Release Desk driver `target/release/deps/fx_aux_actual-dfadab3e07e1c7f8`:
  `23be39162b5ac6bba2182348b0f81e18d34b6647b7b1f7199f6761d7f6057af1`.

Final actual-provider software acceptance: all sixteen selected episodes have passing
results on the frozen release binaries. The production Frontend exercised save →
change all four → load/full paginated rendered preview → cancel without mutation,
then new preview → explicit single-bundle restore → correlated exact settled
four-aux readback, followed by read-only reconnect without replay/grant/rearm.
Local source checked **94,848 silent output samples** with nonzero synthetic capture;
authenticated loopback checked **499,200 silent output samples**, 2,584 Brain blocks,
zero source-egress refusals/drops, zero late wet packets, zero native input/output
drops and no source gap. Both preserved core/composed intent and Disarmed output
state. No runtime safety state was read from disk. Existing independent-return
actual workflows also passed. Release engine harness
`target/release/deps/gigpies-0fe0a176ba2ad145` SHA256 is
`9cddabac5b6236f950df6844bf90acf70ab9042af89f9b0b6dbd9031c8e65a08`.

There was **one failed final TLS attempt**, immediately following harness compilation:
source gap 99,168 → 99,648, 10 source-egress offer refusals and two late wet packets.
Generation changed 2 → 3 before confirmation, so the stale bundle review was refused;
the owner’s last ticket remained 8 (the preceding individual edits). All 1,352,832
observed output samples stayed silent and core intent was unchanged. The fresh,
bounded run above used identical binaries and unchanged bounds and passed. The
underlying transient scheduling/encoder stall was not isolated; the failure is
retained as a timing limitation, not erased by the passing run. This is functional
software acceptance, not timing robustness or hardware qualification.

Earlier debug loopback attempts also refused after source-egress pressure caused
source gaps/owner-generation changes (one diagnostic: 34 egress refusals / 5,862
encoded packets, first source gap 43,488 → 43,632). Those failures are not acceptance.
Production deadlines and queue bounds were unchanged. Earlier stale-driver schema,
transient cancellation-wait state and compile/Clippy failures were corrected; final
normal gates passed with those fixes. No automatic retry/replay was added.

Reproduction: in GigPies, use the same shared nonblocking lock and Cargo environment
above, set `GP_FX_LIBRARY` to the manifest’s approved FX library and
`GP_FX_DESK_TEST_EXE` to the recorded absolute Desk release driver, unset fixture-output
variables, and run each name separately with `cargo +1.97.1 test --locked -j1
--release --features hardware-host --lib NAME -- --ignored --nocapture --test-threads=1`:

```
actual_saved_fx_tls_frontend_and_silence
actual_bundle_refuses_before_any_publication_and_applies_all
actual_bundle_cancel_stale_owner_loss_and_application_interruption
actual_frontend_saved_fx_local_samples
actual_frontend_returns_local_samples
actual_four_aux_tls_brain_controls_and_samples
actual_aux_authority_boundary_samples_failed_edit_and_panic
actual_clocked_remote_aux_applies_during_controller_stall
actual_four_aux_source_brain_roundtrip_and_loss
actual_owner_reserved_window_preserves_continuity_without_output_service
actual_owner_gap_and_output_pressure_keep_exact_frame_witnesses
actual_owner_idle_refusal_retains_cause_without_source_or_controller_service
actual_owner_policy_revocation_does_not_need_controller_or_media_poll
actual_owner_proxy_disconnect_closes_without_controller_service
actual_owner_retirement_backpressure_preserves_closure
actual_owner_control_stall_closure_and_retirement
```

Unchanged historical/exhaustive/audition opt-ins were skipped. No physical activation,
audible playback, two-node traffic or GP-H1 T3 ran. No backend restart, DSP-owner
duplication, host tuning, queue enlargement, deadline relaxation or error suppression
was used. All nine approved artifact hashes and both frozen source digests were
rechecked after acceptance and remained unchanged.


Closeout: with tools idle, inspected processes, the shared lock, executable/fd/map
references and Git status. Removed 450 passed disposable normal-test harnesses
across two validation passes and 65 task logs, reclaiming **45,525,807,104 allocated
bytes cumulatively**. Some harnesses were rebuilt for the final compatibility fix;
this is cumulative deletion, not a claimed net reduction from the starting disk.
Final free space is 40 GiB. No saved-FX or remote temporary fixture directories
remain. Retained deliverables are source/tests, the owning docs and existing return
fixtures, approved artifacts, required runtime/release executables and reusable
acceptance drivers. Normal targets retain about 10 GiB (GigPies) and 14 GiB (Desk)
of runtime, diagnostics and useful shared build caches. Existing uncommitted work
and user data were preserved. No rebuild was performed to verify cleanup.

Remaining full-scene work: core/PA safe selection and review semantics, scene library
beyond this FX slice, coordinated cross-module transactions and separately authorized
physical qualification. No automatic startup recall, full live recall or GP-H1 T3.

## GP-AUTO — live soundcheck and bounded assistance

State: PLANNED. Outcome: use real measured sources to propose useful bounded mix
changes while preserving human control. Next: after GP-METER, select one defined
soundcheck/assistance behavior and connect existing offline logic where applicable.
Offline rule development is owned by [GP-AUTO-ENSEMBLE](#gp-auto-ensemble--observation-led-whole-band-automixing);
its results require a separate bounded live-integration contract here.
Scope authority machinery already exists; do not recreate it. Require held-out
engineering evidence and separate human listening before musical acceptance.

## GP-REC — complete integrated recording workflow

State: SOFTWARE VALIDATED, 2026-10-29. Local implementation and finite synthetic
acceptance complete. GigPies/Desk ownership is released for the deferred GP-FX
integration. No pushes, publication, service changes or physical activation.
SHR FX remained read-only; SHR REC required no provider changes.

Audit and plan: GP05 already owned asynchronous create-new preparation/finalization,
whole-source-frame admission, an independent REC observer and operation-fenced Stop.
Client/lease loss did not stop capture. Desk's old GP05 view was read-only and
legacy-local only. Reuse those owners and add the missing operator workflow;
standalone recorder/player completion is outside this task.

Acceptance checklist:
- [x] Recorder page and controller menu: generated new take, reviewed Start/Stop,
  existing FOH lease, shared request IDs and input-generation/revision fences.
- [x] Admission is separate from polled readiness/outcome. No automatic REC retry
  or reconnect replay; reconnect starts read-only. Failed nonce-less queries
  retire REC readback on that attachment. Lost state retains only stale recovery
  identity; send/receive uncertainty clears freshness.
- [x] Configured raw mapping, accepted/queued/written progress, all integrity
  counters and writer/host faults. Storage capacity/time and durability remain
  unknown. Raw REC ownership, source-clock alignment and recovery are unchanged.
- [x] Host draining progress stays visible while `finalizing` remains pending
  until the worker joins. Terminal errors cannot be overwritten by later polling.
  Callback processing is unchanged and its allocation/deallocation checks pass.
- [x] Actual provider acceptance covers no-lease and create-new refusal, cancelled
  preparation, stale operations, source faults, writer/publication error, drain,
  finalization and capture through Desk/lease loss. The actual Frontend reviews
  and confirms Start/Stop in both 8-input legacy and 16-input dynamic sessions,
  reconnects read-only and checks every stored PCM24 sample in every stem against
  the source-frame reference. It also exercises navigation from Sends to Recorder.
- [x] Required normal suites, warnings-denied lint, formatting, focused checks,
  documentation and repository cleanup completed.

Validation (Rust 1.97.1, committed Cargo.lock, canonical targets,
`CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1`, parent-held nonblocking
`/home/shome/p/.gigpies-build.lock`; no build-lock bypass):

| Complete normal suite | Passed | Opt-in tests skipped |
|---|---:|---:|
| GigPies default, all targets | 485 | 21 |
| GigPies hardware-host, all targets | 569 | 49 |
| Desk default, all targets, serial tests | 357 | 26 |
| Desk native, all targets, serial tests | 360 | 26 |

Both owners passed formatting and both feature configurations passed all-target
Clippy with warnings denied. Native checks used the inspected lavapipe CPU ICD
with DISPLAY/WAYLAND_DISPLAY unset. One initial parallel native run hit the
existing 50 ms Brain held-service assertion; its isolated rerun and both complete
serial suites passed without changing that deadline. Earlier driver failures
identified startup-before-ready attachment and GP05's existing floating-point
metadata; the driver now waits for the host readiness banner and Desk admits
only GP05 metadata floats while preserving integer authority/REC fields.

Final focused host checks: **11 passed**, including actual exact PCM, callback
allocation/deallocation, UDS reconnect, startup refusal and final publication
collision after all frames were written. Final actual Desk check: **1 passed**,
covering both profiles and all workflow/failure/sample checks in 6.21 seconds.
Accepted external libraries remain hash-pinned by
`user/module-workers/0009/pass3/A-GP05/modules.json`. The obsolete sounddesk
baseline manifest was initially refused because referenced artifacts were absent;
it supplied no acceptance. Reproduce with the lock/environment above:

```sh
GP05_MANIFEST=<manifest> cargo +1.97.1 test --locked --features hardware-host --test gp05 --test gp05_alloc --test gp05_local -- --include-ignored --test-threads=1
SHR_DESK_GP05=<built gigpies-headless> SHR_DESK_GP05_MANIFEST=<manifest> cargo +1.97.1 test --locked --test recorder -- --ignored --nocapture --test-threads=1
```

Shared interfaces: GP05 envelopes, REC C ABI and raw file format are unchanged.
Desk adds semantic REC actions and readback to its internal frontend/provider
boundary. Its strict transport admits existing GP05 floating-point FX/PA metadata
(including remote Reply wrappers), not floating-point authority/recording data.
No GP-FX behavior or ABI was implemented or changed. Owning usage and contracts:
Desk `docs/USING.md`, `docs/CONTROL_CONTRACT.md`, and
[host lifecycle](../reference/MODULE_GRAPH.md#desk-recording-operator).

Cleanup: after live Git/process/open-file/lock checks, removed 223 exact task-built
test executables (20,808,437,760 allocated bytes, about 19.4 GiB), plus temporary
logs. Synthetic takes and private test endpoints were removed by their fixtures.
Retained source/tests, this concise evidence and reproducible commands, application
binaries, provider bundles and normal dependency caches. Canonical targets remain
about 4.6 GiB (GigPies) and 3.9 GiB (Desk), with about 56 GiB free. No unknown,
user-media, FX or unrelated project data was cleaned, and cleaned tests were not
rebuilt merely to verify deletion.

Remaining gates: physical audio/controller/display, ADAT/USB mapping and clock
lock, sustained storage/load and power-loss qualification remain separate and
unverified. Historical research/auditions, exhaustive matrices, benchmarks and
unrelated provider/physical opt-ins were intentionally skipped. The new actual
workflow was exercised through local UDS; remote wrapper/transport regressions
passed, but no new two-node REC episode was claimed. Interrupted-host takes still
require the REC owner's evidence-preserving recovery; no automatic recovery or
standalone playback/UI is added. Next owner: deferred GP-FX integration may use
GigPies and Desk; this REC session has released implementation/wiring ownership.

## GP-H1 — physical audio qualification

State: PLANNED; requires a separate authorized physical session.
Next: observed Stagebox/Brain inventory and explicit socket/device maps, UMC/ADAT
clock relationship if that rig is present, duplex drift, electrical latency and
safe fault/rearm. Follow [modular physical procedure](../reference/MODULAR_PROCESSING.md#mapping-and-physical-acceptance)
and [Brain procedure](../reference/BRAIN_AUDIO.md#bounded-physical-followup).
Keep H8's unresolved timing/right-return evidence. Qualify sustained configured
hardware deadlines and acoustic protection; software profiles are not hardware proof.

### Ethernet evidence and transport decision, 2026-11-03

State: IN PROGRESS; cable baseline passed, combined-load software timing remains
OPEN. **Local T0–T2 implementation and device-free acceptance completed November 5.**
T3 distributed timing/headroom and physical qualification remain unexecuted.
Current implementation and exact next action are in the numbered source-owner
checklist below. The following continuation records preserve dated evidence.

Historical November 4 diagnostic follow-up: source-owner integration remained
unfinished; that diagnostic change established no source ownership transfer or
new external blocker. The four-aux diagnostic
review found that idle expiry/cancellation/closure retired a pending commit without
assigning a refusal witness; native processing/transition failures also lacked
specific witnesses. These paths now assign numeric causes, with native codes in
the execution report. A device-free actual-owner regression covers all three idle
causes without source blocks or controller service. Four focused actual-owner
checks and one exact four-aux loopback TLS check passed after all nine artifact
hashes were reverified. This does **not** establish the cause of the earlier
intermittent TLS failure. The 768-frame deadline, queues and mutation behavior are
unchanged. The immediate implementation target remains transfer of the complete
Stagebox graph through the production caller, including global prepared commits,
independent safety closure, coherent observations and controller retirement.
T0–T2 were still open at that checkpoint and T3 remains review-only. Hardware-host library/test Clippy
with warnings denied, formatting and whitespace checks passed. Full suites,
accelerated ASRC and unrelated opt-ins were not repeated for this diagnostic-only
change. Idle cleanup removed the new test executable after lock/process/reference
checks, reclaiming **126,074,880 allocated bytes**. Existing source, runtime
binaries and shared build artifacts remain; Git counts and both heads are unchanged.

Historical ABI continuation (November 4, user-authorized SHR PA changes): the active-PA
live-EQ ABI constraint is removed by an additive detached context interface.
Source-side capture copies fixed-size owner state into controller-prepared opaque
storage; controller coefficient preparation and readback no longer borrow the
active graph. Original instance/generation/epoch/frame and retirement fences remain.
GigPies loads the optional complete interface and uses it in its production PA
wrapper, retaining old-library synchronous compatibility. Explicit retirement
transfer retains controller-side destruction. See the [blueprint interface and
limits](../architecture/AUDIO_TRANSPORT_V1.md#source-owner-integration-constraint-active-pa-live-eq-preparation).
The actual PA test renders on one thread during detached controller preparation/
readback and checks stale rejection and retirement backpressure. This removes the
identified ABI dependency; the complete Stagebox source owner is still unfinished,
with no remaining blocker established by this change. T0–T2 were still open at that checkpoint; T3 was
not launched. The original combined LocalAudio/HostAuthority orchestration remains;
legacy FX, separate analysis, aggregate admission and full owner stall acceptance
remain outstanding. Current actual-owner inputs use the ignored
`user/node-lab/gp-h1-modules.json`; historical task0026 manifests were preserved.
Validation passed with Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0` and the
nonblocking shared build lock: SHR PA normal all-target suite **120 passed**;
GigPies default all-target suite **517 passed/22 ignored**; hardware-host
all-target suite **612 passed/59 ignored**. Both repositories passed all-target
Clippy with warnings denied; SHR PA release library, five Python tests, docs
checks, C header syntax, formatting and diff whitespace checks passed. After
reverifying all nine current-manifest artifact hashes, the actual detached PA
concurrency/retirement test, recording recovery through Brain loss/clock fault,
and four-aux source/Brain roundtrip/loss passed. Run the new actual test with
`GP_PA_LIBRARY=/home/shome/p/shr-pa/target/release/libshr_pa.so`,
`GP_PA_CONFIG=/home/shome/p/shr-pa/tests/fixtures/cpa/v2/stereo4way.json` and
`cargo test --locked -j1 --features hardware-host --lib
host::pa_v2::detached_tests::actual_detached_preparation_during_render_and_transferred_retirement
-- --ignored --exact`. Unchanged accelerated ASRC and unrelated historical,
audition and exhaustive opt-ins were skipped. The old/new master-EQ corpus was
not regenerated; its old-artifact acceptance was not rerun. The prior intermittent
four-aux TLS refusal was not rerun in this ABI change; the local queue-drop
mechanism is closed by the native-window correction in this card.
No physical, two-node, T3, host/service/network, commit or publication actions.
After process/open-file/map, runtime-alias, inode and shared-lock checks, cleanup
removed 174 task-generated test executables and temporary logs, reclaiming
**14,201,847,808 allocated bytes**. Shared build/debug artifacts and runtime binaries
remain: GigPies target **8,551,350,272 bytes**, SHR PA target **753,475,584 bytes**;
free space **45,739,470,848 bytes**. Retained deliverables are source/docs/tests,
the rebuilt PA library and two small ignored current-artifact receipts. Live
GigPies `main` / `d4adff4` has **32 modified tracked files and seven untracked files**;
SHR PA retains **eight modified tracked files**, nothing staged in either checkout.
The ABI fix is integrated in the existing synchronous production caller; the
requested whole-source/T0–T2 work was not complete at this historical increment;
it completed November 5.

Earlier source transaction increment (November 4; borrowed handle superseded below): the production LocalAudio caller
now uses separate mixer prepare/render/finish phases. Preparation validates the
reserved mixer/processing/send transaction. The render handle borrows only DSP
storage, numeric plan and receipt, retaining refused storage without destruction.
Finish installs authority and creates coherent end-of-quantum snapshots, retaining
the exact effective sample frame. Existing PA/FX/raw REC processing precedes mixer
completion retirement/serialization. Allocation/destruction guards cover commits,
lease expiry, invalid shape and source faults before application. The offline
compatibility API retains its historical boundary-sample snapshots; the existing
GP03 producer corpus and GP07 schema validation remain unchanged.

**This continuation has not completed the requested source owner integration.**
Preparation and completion still run in LocalAudio's combined caller. The subsequent
owned-state extraction below does not establish dedicated Stagebox controller-stall
acceptance. Next implementation must transfer the complete source graph and its
prepared external commits, with independent safety closure and bounded retirement;
then finish legacy FX, separate analysis and aggregate admission. T0–T2 were still open at that checkpoint,
and T3 remains review-only. Fresh default all-target validation passed **517 tests,
22 ignored** (Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0`, nonblocking shared
build lock). The first broad attempt exposed the offline snapshot-timing regression;
the compatibility fix passed the original GP03 corpus and GP07/send checks before
the successful full rerun. Hardware-host all-target validation passed **612 tests,
58 ignored**, including **155 library tests/13 ignored**. All nine PA/FX/REC
library/header/owner-manifest hashes matched immediately before actual-owner checks.
Recording recovery and four-aux source/Brain roundtrip passed; the fully qualified
`remote::aux_execution::tests::actual_owner_` filter passed all three cases. The
single actual four-aux TLS check passed, but the previously intermittent refusal
was not diagnosed at that checkpoint (see native-window closure in this card). Hardware-host all-target Clippy
with `-D warnings`, formatting and whitespace checks passed. Unchanged accelerated
ASRC and unrelated historical/audition/exhaustive opt-ins were intentionally skipped.
No T3, two-node traffic, physical activation, sibling edits, commits or publication.
After nonblocking build-lock, process/open-file/map, inode and runtime-alias checks,
cleanup removed 155 task-created test executables and temporary logs/inventory:
**13,766,758,400 allocated bytes reclaimed**. Canonical target retains
**8,548,806,656 allocated bytes** of shared build/debug artifacts and runtime
binaries; free space is **45,797,031,936 bytes**. No scratch inventory/log remains.
Live `main` / `d4adff4`: **31 modified tracked files, seven untracked files, nothing
staged**. Only the mixer transaction phase boundary was integrated in this
continuation; the requested complete source owner and subsequent T1/T2 work remain
unfinished, with no newly demonstrated external blocker.

Prior source-egress continuation: live rpi5 checkpoint matched `main` / `d4adff4`,
26 modified tracked files and six untracked files, nothing staged. The source
GPA1 encoder now runs on `gp-source-encode`, fed by four reusable prepared sample
blocks. Its output is a bounded 256-entry inline packet lane. Source overflow,
encoding refusal, packet overflow, discarded blocks and unconsumed shutdown
packets are separately counted; retired sessions accumulate numeric totals.
Source invalidation closes atomically without joining; controller negotiation
and shutdown join the worker. Existing actual-owner algorithms are unchanged.
This is a source egress boundary only: it does not complete the source render/
controller extraction, global prepared commits, legacy FX extraction, separate
analysis connection or aggregate admission. **T0–T2 were still open at that checkpoint.**
Power-loss recovery on November 4 verified the surviving source/document writes,
formatting, clean diff whitespace and all nine PA/FX/REC artifact hashes. The
background all-target validation was terminated by reboot before a retained final
result; it is not a passing run. Its temporary inventory was lost with `/tmp`.
Post-reboot hardware-host library suite: **152 passed/13 ignored**. A cold-start
replacement test previously expired its 20 ms lifetime before observing the stale
epoch; it now waits for the refusal within a separate bounded test deadline.
Production deadlines and the independent expiry assertions are unchanged.
The actual PA/FX/REC host passed with its encoder deliberately paused: four
queued blocks were discarded at closure and sixteen offers overflowed while
recording and source frames continued. Actual four-aux source/Brain roundtrip
passed; all three scoped actual FX execution/retirement/revocation guards passed.
The authenticated four-aux TLS test first failed with a generic boundary refusal,
then passed after adding numeric refusal witnesses (formatted off the FX owner).
That rerun supplied no causal evidence; the later native-window correction closes
the identified local queue-drop/source-gap mechanism.
A broad test-name filter also selected the unrelated GP21 corpus without its
required environment and failed setup; the corrected execution-module filter
passed all three intended cases. Full normal all-target acceptance has not been
re-established. Historical/audition/exhaustive opt-ins and unchanged accelerated
ASRC opt-ins were not run; T3 and physical/two-node tests were not launched.
Hardware-host all-target Clippy with warnings denied, formatting and diff checks
passed. After build-lock, process/map/open-file and inode/alias checks, cleanup
removed 77 task-created test executables: **5,928,148,992 allocated bytes reclaimed**.
Target retains **8,546,570,240 bytes** of shared build/debug artifacts and runtime
binaries; free space is **45,801,955,328 bytes**. No cleanup inventory/log remains.
Live `main` / `d4adff4`: **28 modified tracked files, seven untracked files,
nothing staged**. No commits, publication, sibling writes or host configuration
changes. At this historical increment the full T0–T2 implementation was unfinished
(completed November 5); power-loss
recovery has not demonstrated an ongoing external blocker.

Prior implementation continuation (power-loss recovery checked on rpi5):
`remote/aux_execution.rs` runs the actual four-aux palette on `gp-brain-fx`,
with authenticated media/assembly/encoding/hash on `gp-fx-media`. The production
runner selects this boundary for the four-aux configuration. Instance-free
prepared tokens transfer to serialized publication; native retirement transfer
stays on the FX owner and destruction stays controller-side after transfer/join.
Four-block sample lanes, bounded commit/retirement queues and independent atomic
closure are wired. Policy replacement publishes a separate generation fence before
acknowledgment; the FX owner checks it without a lock or controller/media poll.
Queue storage is touched before readiness. Completed state remains available
through observation saturation and an idle source. Assembly storage moves to the
media owner instead of remaining duplicated in the controller. The runner reports
queue/assembly layouts and process resources separately; aggregate resource
admission and steady-state headroom are not established.

Prior continuation validation: default all-targets **514 passed/22 ignored**; hardware-host
all-targets **608 passed/58 ignored**, including all 13 duplex/runtime regressions.
All nine PA/FX/REC artifact hashes were reverified after the power loss and again
before the final actual-owner checks. Three new actual FX execution tests passed
under the host allocation/destruction guard: stalled control/observation saturation,
retirement backpressure, and policy revocation without controller or media polling.
The production-wired authenticated loopback test passed with the 768-frame wet
deadline and a 100 ms controller stall while accepted wet returns continued.
The actual source/Brain roundtrip/loss and PA/FX/REC recording-recovery cases passed,
as did the original actual-aux commit/refusal/gap allocation guard. All eight
transport contract tests have therefore passed across normal and explicit runs.
Commands use Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0` and the nonblocking
shared build lock. All five accelerated release `brain_bridge` opt-ins passed in
**143.51 s**. Final hardware-host all-target Clippy with `-D warnings`, formatting
and diff checks passed. Unrelated historical, audition and exhaustive opt-ins
remained skipped.

Cleanup after process/open-file/map, inode, runtime-alias, Git and nonblocking
build-lock checks removed 156 task-generated test executables and the temporary
validation logs: **13,753,438,208 allocated bytes reclaimed**. Shared build/debug
artifacts and runtime binaries remain; target is **8,546,476,032 bytes (7.96 GiB)**,
with about **42.65 GiB free**. No scratch journal remains. Live HEAD is `d4adff4`,
with **26 modified tracked files and six untracked files**, nothing staged.

T0/T1/T2 were open at that checkpoint. Stagebox source/controller extraction, legacy Brain FX
extraction, global source prepared commits/off-thread retirement, separate analysis
connection and aggregate admission remained unfinished at that checkpoint; no
external blocker had been demonstrated. T3, two-node traffic/load and physical activation were not run.
No sibling edits, commits, publication or host/service/network changes occurred.

Previously implemented duplex boundary: `brain_runtime` uses dedicated sole duplex service and
media execution threads. BrainHost publishes copied numeric state; the controller
builds descriptions/JSON, hashes and playback evidence. Four-entry media, commit
and observation lanes have explicit overflow outcomes. Independent capture and
monitor closure generations share one atomic publication, preserving the other
crossing during selection/PTT changes. Local deadman expiry requires observed
closed state and a later active revision; unrelated edits cannot rearm it. Service
independently enforces one absolute run deadline across preparation and every
replacement; an expired replacement performs no service tick. Device
replacement joins the old owner before opening a fresh epoch.
Startup touches prepared samples/handoffs before readiness. A bounded consumer
slot preserves captured media across the accepted-generation handoff; every actual
send still performs the existing authentication/policy/descriptor checks.

The proposed all-node payload budget now counts both ends of both duplex
crossings: 9,216 bytes for those four lanes and 2,241,536 total bytes for the
16-input/eight-FX-channel/eight-window fixture. This corrects a pre-existing
4,608-byte omission, not a measured RSS value. The stereo fake service reports
88,888 known requested bytes, with library, metadata, description, network,
allocator and stack exclusions. Runtime separately samples process RSS/peak RSS,
CPU and faults. In the fresh deadline/replacement regression process, RSS was
5,046,272 bytes before and 14,057,472 after three service instances; minor faults
155→388 and major faults 27→74 include cold startup. Known prepared layouts are
not an explanation of that process-wide delta or proof of fault-free steady
service. No aggregate-system resource or scheduling acceptance follows.

Prior duplex continuation validation (Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0`,
nonblocking shared build lock): default all-target suite **514 passed/22 ignored**;
final hardware-host all-target suite **608 passed/55 ignored**, including all
13 runtime tests and the absolute-deadline replacement regression. Hardware-host
all-target Clippy with warnings denied, formatting and diff checks passed.
All nine PA/FX/REC library/header/owner-manifest hashes matched the local manifest
immediately before the actual-owner tests. Both named actual roundtrip/REC cases
passed; all eight transport contract cases passed including the actual aux
allocation/destruction guard. All five accelerated release bridge opt-ins passed
in **143.56 s**. Unrelated historical, audition, exhaustive and other environment
opt-ins remained ignored; T3, two-node traffic and physical tests were not launched.

The runtime tests cover authenticated loopback service during controller stall,
per-role closure, generation-handoff continuity, actual 18-output mapping under
the host allocation/destruction guard, saturated lanes, expiry/rearm, replacement
and finite shutdown. These checks validate the duplex extraction and unchanged
actual-owner algorithms; they do not validate independent source/FX execution.
T0/T1/T2 were open at that checkpoint: source/FX extraction, global prepared commits/off-thread
retirement, aggregate admission and T2 background/resource acceptance were not
complete at that checkpoint. The later November 5 acceptance below closes them.

Prior duplex continuation cleanup: after nonblocking build-lock, process/open-file/map,
inode and live Git checks, removed 156 task-generated test executables
and the temporary validation journal, reclaiming **13,690,220,544 allocated bytes**
(376,832 bytes from temporary logs/inventory). Retained normal runtime
binaries and shared build/debug artifacts; target is 7.95 GiB and
free space is 42.67 GiB. Existing source, user data and uncommitted work
remain. HEAD is `d4adff4`, with 24 modified tracked and five untracked files,
nothing staged; no commits/publication or sibling/configuration changes occurred.

Historical admission-only increment:
The current admission change makes registry and mailbox wet prefilters reject
deadline equality, matching final jitter-buffer admission. The registry regression
also verifies that a deadline refusal does not consume replay state. This change
does not establish source/FX or duplex execution isolation; T0–T2 were still open at that checkpoint.
Fresh validation of this admission change: default suite 512 passed/22 ignored,
plus two default example tests; hardware-host all-target suite 601 passed/55
ignored; three verified-library owner cases and all five accelerated release
bridge cases passed (bridge run 146.03s). Hardware-host all-target Clippy with
warnings denied, formatting and diff checks passed. PA/FX/REC library, header
and owner-manifest hashes matched the local manifest. These checks do not prove
the requested execution boundaries. After lock/process/reference checks, removed
152 task-generated test executables (13,484,118,016 allocated bytes); retained
runtime binaries and shared build/debug artifacts (target about 8.1 GiB).
Two-node traffic/load, physical activation, host configuration, sibling edits and
publication remain unauthorized. Work is direct single-agent on the canonical Pi5 checkout.
Private task0026 owns exact identities,
reservations, failed reports and sample checks. Source synchronization does not
reserve the link or authorize PCM, cable pulls or service changes. Pi5 coordinates
directly without peer workers. Record a finite reservation and acknowledgment on
both nodes before measurements; build first. Keep SSH on persistent Tailscale
aliases, and verify the dedicated Ethernet carrier, speed, addresses and route.

Measured cable baseline: both links1000/full, MTU1500;200/200 idle replies,
RTT median/p95/p99/max0.175/0.206/0.245/0.324ms; TCP936/942Mbit/s with no
retransmissions; UDP25/50/100Mbit/s with zero reported loss or NIC error/drop
deltas. RTT is not one-way latency. The graph uses16inputs, eight PA outputs,
five independent monitors and four stereo FX auxes from validated configuration;
these test dimensions are not product limits. Both earlier30s runs rejected two
late wet groups at the unchanged768-frame deadline while raw recording remained
exact and continuous. Source633360/output634128/admission634128 followed an8.598ms
source-tick gap and768frames of catch-up in9.489ms. This is compressed headroom,
not proof of wire loss or the sole cause.

Continuation after dependency review rebuilt the actual PA/FX/REC owners and
verified all13 manifest revisions on both Pis. Frame-correlated evidence closes
packet accounting in failed runs: for example,115,976 returned wet packets were
115,944 admitted +8 host-late +24 early deadline refusals, with zero unexplained
packets. NIC/UDP error deltas and interior traced network gaps were zero. These
are audio deadline rejections, not demonstrated wire loss. Admission/application
QUIC dequeue is not socket arrival; clocks from different nodes are never subtracted.
Commands, meters and analysis have their own contracts, not the wet-audio deadline.
They are traced only to identify interference through shared execution resources.

For source1933776, source-tick to QUIC-send call took17.514ms; the cached source
had already reached1934544, the intended wet-return frame. Brain processing was
approximately98µs. Separating network/control tasks from the source owner fixes
that shared-executor blockage. Later failures also showed source catch-up after
multi-millisecond render-wall pauses. A resource witness recorded6017µs wall,
301µs accounted thread CPU, one minor fault and one voluntary switch; accounting
granularity does not permit an exact waiting-time inference. Profiling identified
first-write page faults in lazily zeroed diagnostic storage and allocations for
live per-sample JSON. Diagnostic storage is now touched before timing, and sample
windows retain native numbers for serialization after the run. These observations
identify contributors; they do not establish one exclusive cause for every pause.

Additional fixes preserve structural completion across mailbox retirement, deliver
deferred owner replies through the proxy, and defer early meter reads to the
existing40ms interval instead of triggering reconnect churn. Session bounds and
rate limits are unchanged. Bounded FX shutdown reserves the existing16ms return
budget inside the requested duration so final returns can reach owner admission.
Neither the768-frame deadline nor the60-second synthetic-provider cap changes.

Final rebuilt candidate passed the5/30/60s sequence. The60s run admitted all
115,978 returned wet packets, with zero late/rejected packets, interior traced
media gaps or NIC/UDP error deltas;2,876,928 raw frames per input were independently
exact and contiguous. Source-tick to send-call maximum was3.833ms. Separate actual
Desk gate/FX/REC/reconnect passed with all55,976 wet packets admitted and a complete
338,880-frame/input take. Owned FX loss/restart preserved raw recording and dry PA,
then recovered actual wet samples with a fresh session. These are separate bounded
runs, not a continuous soak.

The final combined60s fake-duplex case passed exact16-channel raw recording, PFL
including input16, post-ASRC PFL gain error−0.042011dB, and1,488 distinct submitted
playback frames of simultaneous talkback/monitor/FX/REC evidence. All trace and
sample-window overflow counters were zero. It nevertheless FAILED clean timing:
89,986 returned wet packets =89,963 admitted +15 host-late +8 earlier deadline
refusals, with zero unexplained packets and zero NIC/UDP error deltas. One Brain
first-group-to-processing interval was4.460ms; provider send-to-return-dequeue
maximum16.849ms includes both directions, processing, scheduling and buffering.
Those durations are not one-way network latency. Remaining application scheduling/
queue interference under combined control/duplex load is unresolved; do not claim
complete software acceptance or advance to physical audio on this evidence.
Source epochs260048/260049 passed exact raw recording, stale command/media intent
rejection, read-only unarmed reconnect without a writer grant, and explicit output
rearm with fresh actual wet samples; Brain local epochs independently advanced1/2/3.
The next software action is to isolate combined-load receive/assembly and source
mailbox delays without putting control or analysis work under the audio deadline.
Only after clean combined timing and separate physical authorization: observe
the actual device/socket map, conservative5s L/R electrical smoke, then the
UMC1820/ADA8200 16-input/18-output mapping and ADAT lock if that rig is present.

Validation:505 default and591 hardware-host production tests passed;22/54 opt-ins
intentionally ignored. Formatting, all-target hardware-host Clippy with warnings
denied,67 publication-script tests and the release build passed. Historical research,
auditions, exhaustive matrices and physical opt-ins were not rerun. Focused tests
cover runtime isolation, retirement completion, deferred meter reads and trace bounds.

### Minimal transport blueprint and dependency-ordered implementation

Design: [dedicated-link audio v1](../architecture/AUDIO_TRANSPORT_V1.md).
Recommendation: retain authenticated QUIC DATAGRAM for v1 and isolate audio
execution; direct GPA1/GBA1 UDP is a conditional same-topology comparator.
The blueprint contains the exact wire layouts, session state machine, fixed queue
profile, clock/resource budgets, preliminary patent screen and go/no-go criteria.
This card is the sole implementation plan; private task0026 remains evidence.
No transport replacement, security downgrade or physical acceptance is approved
by writing this plan. The completed local T0–T2 implementation and acceptance
below supersede the inspected planning base.

Fresh planning inspection verified GigPies97a7949b06297ffc8f13f27bfc496f2a0f5a4585,
all13 local manifest heads/lock/toolchain hashes, and clean matching relevant
GigPies/Desk/PA/FX/REC heads on Pi4 through persistent Tailscale SSH. Exchange/hub
and peer exchange match26acd3ec8816e6a8e1e29a744d5ed9ad70f7f7d1. Runtime binary
identity was not requalified; reverify before trials. No workers, builds, load
traffic, PCM, host settings, source synchronization or publication in this task.

T0–T2 below are accepted locally. Execute the remaining milestones in dependency
order; each is a reviewable bounded change;
no sibling source writes are implicit. Keep Rust 1.97.1, edition2024, locked exact
owner artifacts and normal build directories. Before substantial builds inspect
free space/output sizes and acquire the existing build reservation; all agent
Rust commands use CARGO_INCREMENTAL=0. Before two-node trials obtain separate
traffic authorization and finite reservations, without disturbing other sessions.

| Milestone / dependencies | Owner/files and behavior | Validation and acceptance | Rollback / unresolved decisions |
|---|---|---|---|
| **T0 — freeze contracts and resource fixtures (locally accepted)**; first | GigPies `src/transport/{packet,buffer}.rs`, `src/remote/{media,brain_audio}.rs`, focused tests/fixtures and this blueprint. Record golden GPA1/GBA1 bytes, strict admission/deadline equality, exact production topology, layout and prepared-memory arithmetic. No runtime switch. | Focused normal codec/contract/safety tests; independent decode of vectors, invalid lengths/versions/floats, sequence wrap, u64 exhaustion, dimensions16/32/48, channel-map refusal. Acceptance: every blueprint field maps to implementation or an explicitly proposed change; no hidden MTU/channel/deadline default. Full normal suites if shared codec behavior changes. | Revert only owned fixture/contract changes, leave existing transport. Resolve exact common payload size from runtime capability before comparison; no product question needed. |
| **T1 — isolate source ingress and Brain FX execution (locally accepted)**; T0 | GigPies `remote/{proxy,host,brain,runner,session}.rs`, `source_executor.rs`, `remote/{aux_execution,fx_execution,source_clock}.rs`, `transport/handoff.rs`, `local_audio.rs`, `host/duplex.rs`. Extract fixed media ingress/assembly and prepared commits from JSON dispatch; dedicated FX/source owners; move encode/hash/analysis/serialization off audio. Preserve source cadence/debt and owner algorithms. One ordinary prepared commit/block; independent atomic closure. | Focused queue/order/revocation/retirement tests, allocation+destruction guards, actual PA/FX/REC samples, then complete normal default and hardware-host suites because execution/concurrency changed. Acceptance of the code slice: no control await/lock/allocation on audio path and all bounded outcomes counted. The **5s combined-load QUIC falsifier** belongs to T3, after T2 and separate traffic authorization; it is not required to complete the authorized T1 code gate. That later smoke needs zero wet misses and ≥192-frame margin before proposing duration escalation; failure disproves isolation sufficiency and requires diagnosis. | Feature/config rollback at process restart, fresh session/readback/rearm; no hot fallback. Resolved in local T1 acceptance: extracted LocalAudio authority/snapshot state preserves sole ownership and applied/readback coherence. Future owner ABI changes must retain these guarantees; no extraction restart is pending. |
| **T2 — isolate Brain duplex and background load (locally accepted)**; T1 code contract accepted | GigPies `brain_runtime.rs`, `bin/brain.rs`, `brain_audio/{host,bridge}.rs`, `remote/{brain_audio,session,diagnostics}.rs`, analysis/meter host taps. Give duplex device a sole dedicated service owner; fixed independent capture/monitor lanes, move control awaits and fake sample JSON out. Separate analysis connection/runtime within16-session cap. Keep REC push/lifecycle independent. | Focused fake partial-transfer, both bridge directions, stale selection/PTT, explicit arm and overload tests; full normal suites. Run relevant accelerated long bridge opt-ins since delivery assumptions change. Acceptance: numerical bridge thresholds unchanged, bounded occupancy/ratio, zero normal under/overflow, control stall cannot stall device service; aggregate bytes/CPU/faults documented. | Restore prior executable wiring at quiesced restart and fresh device/session epoch. Open: measured four-core headroom and thread scheduling; do not silently add realtime priority/affinity/mlock or enlarge queues. |
| **T3 — isolated QUIC reference acceptance**; T2 | GigPies actual runner/proxy/media and Desk consumer at recorded revision; PA/FX/REC exact unchanged owner artifacts. No sibling algorithms. Run existing actual-owner topology, freeze parameters and common grouping. | Authorized 5→30→60s independent gates, Desk+duplex+analysis+REC simultaneously. Trace-off/on comparison, per-layer timing/accounting, exact raw samples and independent monitor/actual wet evidence. Separate ≤60s owned-process restart, source/device replacement, stale injection, read-only reconnect/rearm, PTT and REC lifecycle cases. Acceptance: blueprint clean gates, ≥192-frame margin, complete accounting and no resource starvation. A failure stops escalation for diagnosis. | Keep current known source checkpoint/config and fail closed; T1/T2 fixes may remain only if their contracts pass, no physical clearance. Open: whether isolation alone resolves combined timing. This milestone can complete v1 without UDP. |
| **T4 — bounded direct-UDP comparison**; T3 baseline measured (a diagnosed QUIC failure may supply the baseline) | GigPies `transport/network.rs`, temporary adapter under `remote`, `remote/wire.rs`/control binding and runner selection. Share codecs/queues/DSP; implement blueprint `GP-media-bind:1` lab-only lifecycle. No historical scalar UDP control. | Parser/lifecycle/security-refusal tests and full normal suites. Explicit synthetic traffic authorization; identical graph/security-difference declaration; paired5s, then30s/60s only after clean earlier gates, reverse final ordering once. Acceptance: identify whether transport-local costs change and whether ≥96-frame margin improvement is repeatable. Reject physical mode for bare UDP. | Remove temporary adapter if no material gain; keep useful shared tests and one concise result. Open: measured transport attribution, not user preference. Bare UDP success is not production acceptance. |
| **T5 — conditional secure UDP qualification**; only material T4 benefit | GigPies transport/control integration plus reviewed dependency, `Cargo.toml`/`Cargo.lock`, protocol fixtures. Qualify maintained DTLS1.3 for existing packets; reconsider RTP/SRTP only if its established implementation demonstrably reduces work. Bind authenticated media association to reliable control/session; no custom cryptography. | Security review, forged/replayed/wrong-cert/revoked/session-confused packets, overhead/MTU and bounded resource tests, full normal suites and exact T3/T4 combined comparison with security enabled. Acceptance: preserve existing media authentication/integrity/confidentiality, all clean/recovery gates and material benefit retained. | If library/profile or benefit fails, retain isolated QUIC, delete candidate. Open: qualified implementation and complete secure profile, deployment-jurisdiction patent checks, contributor rights. Any intentional security reduction is a separate product decision, not an optimization. T5 is unnecessary for selected QUIC v1. |
| **T6 — decision, consolidation and separate hardware handoff**; T3, plus T5 if replacing | GigPies owning references, executable default/config schema and GP-H1; sibling fixture updates only with separate authorization. Select one production adapter, remove losing experiment branch/helpers, keep codecs and conformance corpus. | Complete normal suites for final execution/contract changes; independent implementation/review of packet/state vectors before replacement; publication policy only if later publication is authorized. Apply blueprint go/no-go exactly. Acceptance: one production path, concise result and reproducible commands, no private artifacts; retain H8/physical gates open. | Rollback to recorded QUIC configuration requires fresh session/epochs/readback and explicit rearm. Open hardware requirements are observed socket map/ADAT lock, electrical delay, safe fault handling and sustained actual graph; never infer them from software success. |

Current local acceptance reconciliation (November 5, completed):

- **Implemented and component-validated:** autonomous source ownership; bounded
  prepared/completion/retirement lanes; detached PA and REC lifecycle; source
  egress and authenticated handoffs; all supported legacy 48/96-frame FX shapes
  and four-aux native/media execution; dedicated Brain duplex service/media;
  independent read-only analysis connection/runtime and encoder. Do not restart
  these extractions. The native output-credit queue-drop/source-gap defect is closed.
- **Combined coverage implemented:** authenticated actual duplex/FX/raw REC,
  selected nonzero playback, held talkback, full source return lanes, exact
  talkback receipts, independent analysis closure, old FX identity refusal,
  source/device replacement and authenticated explicit output rearm. Stale device
  refusal, independent capture closure and final broad validation passed.
- **Resource reconciliation:** actual source media/encoder, native FX/assembly,
  duplex service and global executor layout reports identify each allocation once
  and explicitly exclude opaque owner/TLS/allocator storage. Process and per-thread
  RSS/CPU/fault/scheduling observations cover resident aggregate. Existing limits
  admit two audio owners, one independent observer and sixteen authenticated
  sessions; control-only sessions gain no media pools. The completed audit is below.
- **Local T0–T2 complete:** focused actual-owner, normal hardware/default, relevant
  accelerated bridge, Clippy and formatting gates passed. No local implementation
  or acceptance item remains open.
- **Outside this authorization:** T3 distributed 5/30/60-second clean timing and
  headroom qualification, physical activation, socket/clock/acoustic checks.
  These are separate gates, not local software implementation blockers.

Source-owner continuation checklist (local T0–T2 accepted):

1. **Complete — owned source extraction and component validation.**
   `mixer_control/source.rs` owns the production mixer, numeric epoch, plan and
   retained receipt; LocalAudio uses it instead of a borrowed quantum handle.
   Preallocated numeric observations now capture end-of-quantum coefficients,
   processing configuration/progress, sends and raw gate state. Controller
   snapshots, intent persistence and processing/send readiness consume the last
   observed state; pending source completion cannot appear applied in readback.
   Gate attenuation conversion remains on the controller. Historical offline
   boundary-sample snapshots retain their existing compatibility pump.
   `module_graph/source.rs` owns actual PA/local FX, raw recorder push, timeline,
   buffers and retired PA storage. Controller retains authority/protocol,
   preparation, recorder metadata/observer/finalizer and replies; media retains
   encoding/transport. `local_audio/source.rs` now owns routing scratch, talkback
   envelope/samples, monitor/send taps and meter acquisition; production routing
   delegates to it, with meter readback retained on the controller. Production
   startup now also transfers spectrum acquisition, analysis taps/signals and
   measurement capture into SourceRouting. Spectrum FFT/readback/worker joining
   and measurement analysis/subprocess lifecycle stay on the controller. The
   `RenderGraph` now owns SourceState, ModuleSource, SourceRouting and local aux
   storage. LocalAudio's numeric boundary operates on this aggregate;
   capture mapping, measurement offers, playback patching and output
   spectrum acquisition run inside it. It no longer borrows controller DSP owners.
   Authority, lifecycle workers and replies stay outside. HostAuthority now opts
   into persistent graph ownership: mixer, modules, routing and local aux remain
   detached across quanta and structural operations. Host startup now transfers the graph
   and HostSourceMedia to one dedicated `gp-source` worker. Other LocalAudio
   compatibility callers still reattach. Quiesced recovery retires/rebuilds the
   graph and reinstalls it on the same worker; autonomous capture scheduling is now wired through the production runner. Monitor/send/aux samples, bus readback
   and meter losses are copied observations; output safety/quiescence uses a copied
   ramp envelope. Output patches prepare against controller topology and publish
   their applied map from the returned prepared packet.
   ModuleGraph retains an API-only PA preparer and an owned detached PA observation/context;
   structural graph preparation and EQ JSON preparation/readback no longer borrow
   active PA for libraries exposing the detached ABI. Source capture and native EQ
   retirement return are explicit bounded operations; the compatibility pump is
   still synchronous. Legacy libraries retain synchronous compatibility only.
2. **Complete — bounded handoff, admission and retirement.** Owned mixer continues unchanged
   rendering with occupied completion/retirement, refuses new mixer/external
   admission until observation, rejects stale epoch and fences unvalidated/external
   boundaries. Mixer retired prepared storage now travels in the retained receipt
   for controller destruction; finish_render no longer takes retirement directly
   from the active mixer. Latest numeric observations advance during unchanged
   rendering without overwriting the retained application identity.
   Detached mixer commands and reusable completion packets bind the owner, epoch,
   exact frame and ticket. Refused prepared storage stays in the packet; source
   rendering continues after completion export. Receipts distinguish cancellation
   before application from revocation after application; the latter publishes the
   actual revision while preserving revoked grants. External structural permits use
   the same authority reservation and return numeric receipts before revision
   publication; replay, stale identity, occupied render state and cancellation
   refuse before application. Production patch/mute/rearm/PA/EQ now use that
   source boundary. Operator selection, prepared aux storage and four-bus send
   updates now travel in detached mixer commands; all allocated command payloads
   return for controller destruction. Returned observations include applied aux
   send targets; remote aux snapshots/acknowledgments no longer borrow the active
   mixer or publish queued targets before source completion. The worker now has globally ordered 16-command, 128-completion and
   16-retirement lanes, reserving both returns before a swap. Controller calls
   currently serialize one outstanding packet and wait for it; the source never
   takes the controller mutex. Source-issued exact/future-frame admission binds
   owner/epoch/ticket without publishing future time as readback. Expired or
   superseded commands retain storage and refuse before mutation. Future mixer
   application can span F and F+48 without a second controller validation. The runner now uses the autonomous source clock. Ordinary engine, structural,
   local aux and Brain preparation reserve future frames; expired storage returns
   without publication. Remote FX permission and four-aux send application now use source-issued
   permission and retained future-frame send packets; actual runner/TLS checks
   validate application and retained completion.
   Measurement uses one bounded capture admission and return slot;
   outstanding buffers block reuse until controller retirement, while source
   collection/completion needs no controller pump. Progress is atomic; raw offers
   do not inspect controller PA metadata. This measurement-local
   slot complements the globally reserved completion/readback/retirement lanes above.
3. **Complete — module lifecycle and combined local acceptance.** Recorder pushes no longer inspect
   controller strings; numeric source cancellation prevents late prepared handle
   activation and preserves the original fault through Stop/recovery. Independent
   source/session closure and module handoffs have component and actual-owner coverage.
   Numeric PA structural/EQ commit, mute and rearm now
   execute on ModuleSource. Owned ModuleObservation captures numeric PA/FX health;
   controller health queries no longer call active native status functions. Legacy
   status symbols resolve at load time. PA metadata is controller-owned. One owned
   module exchange carries REC activation/Stop, end-quantum observation and native
   EQ/structural retirement; controller completion schedules finalization and
   destroys returned storage. Returned EQ slots are emptied on the controller
   before reuse; successive actual EQ edits cover this lifecycle. PA structural
   completion must precede preparation
   of the next context exchange. Source closure survives controller teardown;
   stale module epochs cannot stop a newer recorder. LocalAudio teardown publishes
   its independent source closure before field destruction. Measurement cancellation/epoch invalidation/controller
   teardown independently close queued/active captures through a generation signal;
   stale buffers return for controller destruction. Analysis fault signals cannot
   join the worker from source acquisition. Proxy disconnect/drop now publishes
   terminal closure directly to the authenticated context's atomic render fence,
   independent of mailbox service or command capacity. Client close/drop, terminal
   control/media errors and FX reply-reader cancellation publish the same closure;
   reliable applied replies remain drainable. Fresh authentication cannot reopen
   an old fence. This covers existing consumers of that fence, not yet the
   complete Stagebox graph. Local aux now separates controller metadata and
   instance-free preparation from source Rack/native owners. Prepared source
   commands use the external permit; fixed status/retirement observations return
   native tokens for controller destruction. Routing resets, startup rearm and terminal mixer/module quiescence
   now execute as owned packets. Recovery rejoins only after source closure
   and reinstalls the rebuilt graph on the same source worker, preserving epoch
   ordering; synthetic autonomous epoch replacement now coordinates close/drain/rebuild/restart
   on that worker within the original run deadline, disarmed; physical reopen
   remains separate. Copied local deadlines expire on source. Terminal quiescence
   performs a bounded module exchange even without another capture block, returning
   raw REC/native retirement for controller finalization while the graph stays detached.
4. **Complete — production runner cutover and component validation.**
   `ProviderSource` now starts/services/stops the production `gp-source` owner;
   the runner timer services control/readback without submitting capture quanta.
   Startup works at frame zero. Synthetic capture uses elapsed cadence; the raw
   Duplex owner now transfers to the worker with bounded nonblocking partial
   capture/playback cursors, capture-anchored playback expiry and physical-map
   checks. Capture starts on the source worker after preparation, so controller
   allocation cannot consume active capture headroom. No physical device was activated or qualified.
   Ordinary engine, structural, local aux and Brain preparation use source-issued
   F+480 admission, retain return capacity and recheck authority at application.
   Independent hold/FOH session fences and copied deadlines silence talkback even
   when the media session stays live. Monitor generation changes silence the old
   stream at the source boundary and bound its retirement tail. Brain device
   configuration intent also commits at a future source boundary.
   Structural completion precedes the next PA context exchange. Late preparation
   returns its storage without publishing application. Media negotiation prepares
   storage on the controller, swaps at a future source frame, and retains the old
   controller identity until the source accepts it. Remote FX/aux permission now commits at a source-issued future frame;
   four-aux send changes retain their later negotiated frame independently of
   controller service. Writer/owner fences, cancellation and copied expiry close
   pending actions. Completed storage returns for controller retirement.
   Four-aux negotiation swaps mixer send mode and media storage atomically.
   Controller service exchanges current mixer/module/routing observations and REC
   lifecycle work without advancing source time. Final observations are collected
   after stop. Cumulative wet/drop accounting is sampled during service and at
   stop without double counting. Preallocated source output windows retain exact
   frame/energy/peak/wet totals, splitting on applied revision/map and Brain
   routing/hold changes, with explicit overflow. The `source_clock_v2` report
   carries these identities in source output windows; legacy sampled histories
   remain null. Synthetic recovery now stops/drains/rebuilds/restarts within the
   original deadline, preserves the previous report on restart failure, and
   requires explicit rearm.
   LocalAudio compatibility callers retain their existing pump. Duplex's source boundary now takes topology/epoch/frame and
   closure/process operations without requiring a LocalAudio reference; existing
   callers retain their behavior. HostAuthority now always extracts the source
   sample-egress endpoint on negotiation; the endpoint owns sample submission and
   recycling, while encoder joining stays on the controller. Authenticated negotiation now
   transfers the sole encoded-packet consumer to the network proxy; conversion and
   draining continue without controller service. Authenticated wet ingress now
   transfers a prepared stream/generation endpoint to the network proxy and
   submits to the existing source lane without mailbox service; controller
   receipt accounting remains bounded. Brain talkback/monitor endpoints now transfer
   to the proxy for clocked hosts; source-owned authorization and existing fixed
   lanes allow submission and encoding without mailbox service.
   Replacement closes and joins before endpoint destruction; independent closure
   reaches a full source lane. HostSourceMedia now owns wet assembly/gates,
   talkback ASRC, sample taps and source egress. Monitor encoding runs in media
   polling behind a prefaulted copied-sample SPSC lane, retaining the existing
   64-block bound (49,664 payload bytes); overflow refuses new blocks and counts
   them. Wet and talkback ingress now use prefaulted Copy SPSC lanes and fixed
   return receipts: 256 wet packets/receipts and 64 mono blocks/receipts,
   **671,296 bytes** of additional payload storage (ring metadata excluded).
   Source insertion rechecks negotiation generation, policy fence, exact wet
   deadline, talkback replay and hold deadline. Full return lanes stop ingress
   draining, not rendering; full ingress refuses/counts the newest item. Hashing
   and acceptance accounting consume controller receipts. Source wet playout also
   fences already-buffered samples after closure. Negotiation/replacement and
   policy/readback admission still depend on controller progress. Negotiation
   and replacement now swap prepared storage through owned worker commands;
   displaced storage returns to the controller, with numeric copied readback. Final runner shutdown drains receipt
   accounting and rejects/counts pending ingress without advancing source time.
   Same-bridge Brain renegotiation preserves queued generation/replay state; a
   replacement identity cannot inherit it. Legacy PA/FX wrappers permit exclusive transfer
   under their owner ABI contracts. Dedicated thread transfer is implemented;
   production source pacing is autonomous; combined authenticated lifecycle acceptance passed.
5. **Complete — local source and cross-owner acceptance.** Normal source/mixer/
   module guards cover allocation/destruction, exact application, stale epochs,
   full completion/retirement, closure and numeric readback. Actual runner source
   cases cover PA/local or remote FX/raw REC, autonomous progression under both
   saturated return lanes, exact raw tails, reconstruction and explicit rearm.
   The authenticated combined case adds actual Brain duplex/media, selected
   nonzero playback, held talkback, independent analysis, exact talkback receipts,
   stale FX/device refusal, replacement epochs and authenticated output rearm.
   Capture closure preserves monitor playback. Four-aux TLS retains explicit zero
   native-drop/source-gap assertions. Complete normal suites and the five relevant
   accelerated bridge cases passed. This is local software acceptance; T3 and
   physical qualification remain separate.
6. **Complete — compatibility, analysis and local resource accounting.** Legacy
   1..256-channel, 48/96-frame banks now use dedicated native execution and a
   four-block topology-sized recycle pool; configured stereo controls retain the
   same prepared commit/retirement path. Actual-library comparisons at both frame
   sizes and 1/2/3/16/48/256 channels passed, including closure and host allocation/
   destruction guards. The shared media adapter retains the existing native output
   credit reservation; the four-aux fixed-block storage is unchanged.
   Production Brain now attaches a separate read-only analysis connection on its
   own receive runtime, with a separate source encoder and four reusable source
   blocks. Host admission permits one analysis observer, independent of FX and
   duplex, within the unchanged 16-session cap. Source epoch/map and atomic policy
   fences close the observer without controller service. Its focused source/sample/
   duplicate-admission/closure/pressure checks and combined TLS acceptance passed.
   Process reports now include per-thread CPU ticks, faults and available Linux
   schedstat values, sampled off audio before worker shutdown. Actual source media/encoder,
   native FX/assembly, duplex and global queue layouts are reported separately, with
   opaque owner/TLS/allocator costs explicitly covered by measured process RSS
   rather than guessed byte totals. Existing session/owner/pending-transaction limits
   bound admission; source swaps reserve return storage before mutation. Local
   reconciliation and broad validation passed. Sustained distributed headroom is T3.

Stereo delay continuation (November 5): the production runner now transfers the
48-frame stereo `BrainFx` into `remote/fx_execution.rs`, with shared independent
media assembly/encoding and a sole native execution thread. Controller preparation
uses an instance-free token pinned to the same library. Exact-frame permit/commit,
independent closure, idle request/run expiry, retained completion and bounded
retirement preserve the existing relay protocol. Settled completion retains its
application status through later closure; native terminal faults are separate
from completion and propagate into runner failure. Other legacy dimensions retain
the compatibility path rather than acquiring a new product cap. Both execution
and runner use one absolute deadline. No sibling ABI/artifact change was needed.

The actual runner lifecycle tests preserve separate admitted local-four-aux and
legacy-remote configurations; production correctly excludes enabling both. The
remote case exchanges production encoded source packets with the real legacy FX
execution owner and returns wet through source ingress while PA/raw REC continue.
It exercises remote prepare/permit/settle with
controller observations stalled, nonzero native and source-rendered wet, retained
completion, exact raw REC tails, autonomous reconstruction on the existing worker,
old-handle/reply refusal, disarmed replacement and explicit rearm. Remote FX is
closed afterward; fresh remote-owner creation/renegotiation was not covered by that
fixture (the authenticated continuation below adds it). This is a
bounded device-free ownership test, not authenticated combined network-timing
acceptance. The intermittent hold-authority assertion now captures both its reply
and numeric source refusal receipt without changing production deadlines.

Authenticated recovery continuation (November 5, after reconciliation with the
completed stereo-extraction session): the new
`remote::tests::gp21_recovery_tests::actual_tls_fx_pending_close_and_replacement_after_source_recovery`
exercises production BrainWorker dedicated stereo execution/media over mutual TLS
loopback, with autonomous actual PA/raw REC. A controller stall preserves source
progress. Recovery closes a permitted future native transition before application,
retains refusal without generation advancement, rejects the old source handle and
starts the replacement epoch disarmed. A new authenticated connection negotiates
fresh media and creates a fresh FX owner; the old binding is refused. After explicit
rearm and observed catch-up to the live source frame, one new prepare/permit settles
at its exact requested frame. Both epochs produce nonzero actual wet/PA output;
eight PCM24 recording tails match the old source timeline exactly and report no
dropped REC frames. The original absolute source/FX run budgets remain bounded.

The initial test version deliberately entered BrainWorker directly: that was authenticated
media plus native-owner lifecycle coverage, not end-to-end Desk lease/relay
acceptance. The 2 ms synthetic fixture cadence is not a timing qualification. The
first fixture overflowed its nested async stack; moving synchronous setup out and
boxing futures fixed the harness with RUST_MIN_STACK unset. Its initial shared
controller/runtime also stalled Stagebox service during Brain library loading.
The fixture now uses the production two-worker network runtime and independent
Brain preparation. Issuing a transition immediately after initial reception/rearm
reproduced native source-gap refusal (cause 4), with input/output queue drops
(7/21 in the original fixture, 1/29 after runtime separation), no malformed/stale
groups, and no native terminal fault. The final test explicitly observes catch-up
before a single reviewed command; it neither retries refused mutations nor claims
zero startup drops. The native-window correction in this card subsequently closes the
local queue-drop mechanism. This run did not diagnose hold authority or enlarge
any deadline/queue.

Validation for this test-only continuation: the final exact actual-owner case
passed, then passed three bounded stability checks after the observed failures.
The normal hardware-host remote group passed **94 / 22 ignored**. All nine current
PA/FX/REC artifact hashes matched before execution. The complete suites from the
immediately preceding stereo-extraction closeout remain the production baseline;
production code did not change here. Unrelated historical, audition, exhaustive,
bridge and other actual-owner opt-ins were not rerun. Reproduce with the shared
nonblocking build lock, Rust 1.97.1, CARGO_INCREMENTAL=0, RUST_MIN_STACK unset,
GP05_MANIFEST=user/node-lab/gp-h1-modules.json and GP_PA_CONFIG pointing to the
actual PA stereo4way fixture:
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib remote::tests::gp21_recovery_tests::actual_tls_fx_pending_close_and_replacement_after_source_recovery -- --ignored --exact --nocapture`.

Recovery-test closeout: hardware-host library/test Clippy passed with warnings
denied; formatting and whitespace checks passed. After shared-lock, process,
open-file/mapping, hardlink/runtime-alias and Git checks, removed the task-created
library-test executable and compiler diagnostic, reclaiming **138,760,192 allocated
bytes**. The canonical target retains **8,813,375,488 bytes** of shared runtime/debug
build artifacts, with approximately 43 GiB free. No temporary fixture remains.
The user selected continuation in the original session; this session has released
ownership of the test and owning plan. The new test file, its registration and
this record are uncommitted; production code is unchanged by this continuation.

Authenticated Desk relay continuation (November 5): the recovery test now uses a
separate Desk TLS connection, ordinary authority readback and an FX lease, fresh
FX review, production prepare/permit envelopes and completion delivery through
the Brain reply channel. An unissued lease is refused. A permitted future edit
closes without generation advancement; Desk receives `unknown`, rather than a
false applied result. A new connection/owner refuses the old prepared reply and
old-epoch Desk request, remains disarmed until explicit rearm, then settles a
single reviewed edit at its exact requested frame. Actual PA, remote stereo FX
and raw REC run together. Each source completion/retirement lane is saturated for
32 quanta with controller observations held; native FX also advances. Both epochs
retain nonzero wet/PA evidence and all eight old-epoch PCM24 tails match exactly.

This extension exposed source-gap/stale-preparation refusals while encoded packets
still depended on controller mailbox draining. Authenticated negotiation now hands
the sole existing 256-packet consumer to the network proxy. No queue or deadline
was enlarged. Controller shutdown closes/joins/drains and retains exact cumulative
accounting; only network draining and controller shutdown share a mutex, never the
source or encoder. A normal regression transfers the consumer across threads,
checks exact packets, fills the lane, closes it with the endpoint still alive,
and reconciles every encoded packet. A delayed handoff preserves packets already
queued in the proxy ahead of the transferred consumer. Compatibility callers retain their drain.
The test uses production 50 ms observation publication and a bounded continuity
readiness window before review; no refused mutation is retried. Initial harness
failures included missing authority readback and stale reviewed reset counts.
Passing this local fixture alone did not diagnose the TLS/hold witnesses or prove
zero startup drops. At this checkpoint wet ingress and Brain media remained controller-serviced;
the wet-ingress continuation below removes the former dependency.

Running the actual cases together exposed a frame-zero startup bug hidden by
fresh-process time zero: REC/control preparation had advanced authority time,
but initial source admission still used zero. Startup now uses the prepared
source's current monotonic time. Recovery carries forward the old epoch's final
observed time and charges drain/reconstruction time without extending the original
run deadline or advancing the new source-frame origin. A fast normal regression
starts at frame zero after nonzero control time; the shared-process actual cases
cover REC startup and reconstruction as well.

Validation for this continuation: complete normal all-target suites passed
**674 / 76 ignored** with hardware-host and **550 / 22 ignored** with default
features, without normal-test filters. After the queued-prefix and startup-time
corrections, the complete hardware-host library passed **219 / 30 ignored**.
Both actual runner lifecycle cases and the authenticated recovery case passed
together in the final process, with RUST_MIN_STACK unset. The recovery fixture
also passed three consecutive checks earlier in this continuation. Both feature
configurations passed all-target Clippy with warnings denied. All nine current
artifact hashes matched before execution. Unchanged historical/audition/exhaustive,
corpus-generation and accelerated ASRC opt-ins were skipped; no offline DSP tests
were repeated after the focused closeout corrections.

The four-aux TLS compatibility case passed twice, then reproduced the existing
intermittent failure in the final batch: bus 2, ticket 3, `FX aux source gap`
(cause 1), **4 native input drops / 4 output drops**, 450 processed blocks,
1 report drop, native refusal code 0 and `media_fault: null`. Source egress
accounted for **916 encoded = 916 drained**, with zero block/packet drops,
refusals or unconsumed packets; wet admission was **882 accepted / 0 late /
0 rejected**. It was not rerun to obtain a green result. This retained signature is reproduced and closed by the native-window correction
below; the successful recovery case alone was not proof
of whole-owner or timing acceptance.

The earlier exact recovery command remains valid. The historical stereo TLS
fixture overflowed its large async test stack in the combined process; its
separate bounded 16 MiB test-stack check uses the existing
`user/modularization-20261008/desk-sample-proof/identity-stereo-pa.json` configuration
(two PA outputs); it passed **1 / 0 failed** with `RUST_MIN_STACK=16777216`.
No production stack, queue or deadline changes. After shared-lock, process,
open-file/map, inode and runtime-reference checks, cleanup removed the task-created
test executables (155 in the pre-cleanup inventory), reclaiming approximately
**13.18 GiB**. The canonical target retains **8,693,960,704 allocated bytes** of
shared build/runtime artifacts; **45,487,022,080 bytes** are free. No rebuild
followed cleanup. Source, existing uncommitted work and runtime binaries remain. Work stays single-threaded in canonical `main`;
no peer workers, hardware activation, two-node traffic, sibling writes, commits or
publication.

Authenticated wet-ingress continuation (November 5): traced the retained
four-aux witness to the separate four-block Brain media/native input and output
lanes. An input overflow drops the newest assembled source block; consuming the
next noncontiguous block produces source-gap cause 1 and refuses pending native
work. Output overflow independently drops a wet block. The retained 916/916 source
egress and 882 accepted/zero-late wet counts therefore do not establish native
continuity or localize the scheduling delay. Added fixed first/last input/output
dropped source frames and the first expected/received native gap pair to the
existing execution report. One diagnostic check before ingress extraction passed
(692 blocks, 67,584 Main comparisons); it did not reproduce or resolve the failure.
No queue, deadline, cadence or algorithm was changed.

Authenticated negotiation now transfers a prepared wet stream/generation endpoint
to the network proxy. Network submission enters the existing 256-packet source
lane without `AuthorityMailbox::service`. Source insertion retains generation,
policy and exact deadline validation. A short producer mutex coordinates network,
compatibility submission, overflow accounting and final closure; source execution
never acquires it. Existing 256 receipts retain exact payloads for controller
hashing/accounting; full receipts defer admission while rendering continues.
Shutdown closes submission before draining receipts and rejecting pending packets.
Review also found that the network future-frame filter still read a controller-
published cursor: after 1,536 frames of stalled service it could refuse valid wet
traffic. Source ingress now publishes its actual frame atomically, even with full
receipts, and the transferred endpoint supplies that cursor to network validation.
No new packet queue was added. At that continuation, Brain talkback/monitor
admission and polling still required the handoff implemented below.

New normal coverage transfers the wet endpoint across threads, holds controller
receipts, verifies full-lane refusal, continued wet rendering, stale-generation
rejection, policy revocation and terminal shutdown. The actual authenticated
Desk/Brain recovery fixture now requires at least 48 additional accepted wet receipts accumulated
while controller service and both source return lanes are stalled. The strengthened check passed
with actual PA/stereo FX/raw REC, old-epoch refusal and fresh-owner rearm. An
actual native-owner regression also deliberately omits frame 192 and fills the
four-block wet-output lane: it retains gap `(192, 240)` and exactly four output
drops at first/last source frames 240/384. This validates witness interpretation,
not the scheduling cause of the intermittent clean-run failure.

Validation: the complete normal hardware-host all-target suite passed **677 /
76 ignored**. The initial actual-owner batch passed all five selected checks
(native gap/overflow, both runner lifecycles, four-aux TLS and authenticated
recovery); four-aux produced 695 blocks and 67,392 Main comparisons. After the
live-cursor correction, ten focused checks passed, including strengthened recovery,
both actual runner lifecycles and ingress/closure tests. Default-feature remote
regressions passed **28 / 1 ignored**. A subsequent normal hardware library run
found a pre-existing fixture-time error in `clocked_legacy_permission_survives_controller_stall`:
its time-zero lease could expire when startup retained the media preparation's
nonzero source-control time. The clocked fixture now grants at that prepared time;
production lease duration is unchanged. Both feature configurations passed
all-target Clippy with warnings denied. The final cursor-corrected four-aux TLS
check passed (678 blocks, 67,200 Main comparisons); causal closure came later
from the native-window regression in this card. The final complete hardware-host
library passed **220 / 31 ignored** after the fixture and small error-return cleanup;
formatting and whitespace checks passed. Full offline integration targets were
not repeated after these focused corrections. All commands used the shared
nonblocking build lock, Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0` and
`RUST_MIN_STACK` unset for tests.

Reproduce the actual continuation with the existing manifest/PA fixture environment
and `GP_FX_LIBRARY=/home/shome/p/shr-fx/target/release/libshr_fx.so`, under that lock:
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib -- actual_owner_gap_and_output_pressure_keep_exact_frame_witnesses actual_runner_source_ actual_tls_fx_pending_close_and_replacement_after_source_recovery --include-ignored --test-threads=1 --nocapture`.
The four-aux diagnostic uses the previously documented exact test name separately.

Closeout: after build-lock, process, open-file/mapping, hardlink/symlink and live
Git checks, removed 80 task-built test executables (**8,359,972,864 allocated bytes**)
and temporary logs (**262,144 bytes**). The canonical target retains
**8,693,063,680 allocated bytes** of shared build/runtime artifacts; free space is
**45,485,191,168 bytes**. No rebuild followed cleanup. Source, prior uncommitted
work and runtime binaries remain. All work stayed on canonical `main` on rpi5;
no siblings, peers, physical devices, two-node traffic, commits or publication. The unchanged default offline DSP suite is not duplicated;
historical/audition/exhaustive and accelerated bridge opt-ins remain skipped.
All nine current module artifact hashes matched before actual-owner execution.

Brain handoff continuation (November 5): clocked-host negotiation now transfers
prepared GBA1 talkback/monitor endpoints to the authenticated proxy. Talkback
submits directly to the existing 64-block source lane; monitor encoding consumes
the existing 64-block copied-sample lane. No additional packet/sample queue or
playout allowance was added. Short producer/consumer mutexes belong exclusively
to network/controller endpoints; the source never takes them. Source-published
atomic authorization carries the current hold deadline and monitor silence state.
Source insertion and rendering independently recheck hold generation, copied
lease/FOH deadlines, policy fences and output safety. Same-bridge renegotiation
retains the shared replay cursor and monitor queue; replacement closes the old
endpoint before new ownership is exported. Shutdown serializes producer closure
before final receipt/pending/refusal accounting. Compatibility hosts retain their
synchronous admission/polling path.

Validation: the complete normal hardware-host all-target suite passed **679 /
77 ignored**. The default-feature library, binaries, examples and remaining
integration targets passed **364 / 18 ignored / seven unchanged automix library
cases skipped**, using the unchanged offline-target exclusions listed in the
stereo-extraction entry below. No source/control/transport target was omitted
from that second configuration. Focused handoff/monitor/hold checks passed **17**;
the full library run also exercised the added new-hold-generation refusal case.
Both configurations passed all-target Clippy with warnings denied; formatting
and diff whitespace checks passed.

All nine current module artifact hashes matched before the five selected actual
checks: native gap/drop witnesses, both runner lifecycles, authenticated recovery
and four-aux TLS. All **five passed**. Authenticated recovery retained the required
48 additional accepted wet receipts during 64 controller-stalled quanta with both
source return lanes pressured. Four-aux TLS processed **689 blocks / 67,872 Main
sample comparisons**; no queue-drop failure occurred in that run. Its later causal
closure is recorded below; this earlier pass alone supplied no diagnosis.
The five relevant accelerated release `brain_bridge` opt-ins passed in **193.27 s**;
other historical, audition, exhaustive, benchmark and unchanged actual-owner
opt-ins remained skipped.

Commands used canonical targets, Rust 1.97.1, `--locked -j1`,
`CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1`, `RUST_MIN_STACK` unset, and the shared
nonblocking build lock. Reproduce focused checks with
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib -- source_media_tests monitor_retirement_tests clocked_render_closes_talkback --test-threads=1`.
The native command above additionally selected
`actual_four_aux_tls_brain_controls_and_samples`; bridge opt-ins used
`cargo +1.97.1 test --release --locked -j1 --features hardware-host --test brain_bridge -- --ignored --test-threads=1`.

Continuation cleanup: after live Git, nonblocking shared-lock, process,
open-file/mapping, hardlink and symlink/runtime-alias checks, removed **138**
task-built test executables (**12,566,913,024 allocated bytes**) and temporary
logs/inventory (**245,760 bytes**). Canonical target retains **8,347,045,888 bytes**
of shared build/runtime artifacts; free space is **45,828,108,288 bytes**. No
rebuild followed cleanup. Source, prior work and required runtime binaries remain.
All source changes remain uncommitted on canonical rpi5 `main`. No siblings,
peers, physical devices, two-node traffic, commits or publication were involved.
T0–T2 and combined actual Brain duplex/FX/raw REC lifecycle acceptance were open
at this historical increment; both completed November 5.

Native-window correction (November 5): **the four-aux local queue-drop/source-gap
issue is closed.** Inspected both retained TLS witnesses and the newer first/last
drop and expected/received frame fields. Media previously received assembled
blocks without reserving either native input or eventual wet-output capacity;
output draining ran on a separate timer branch. A receive burst could fill input
before native ran, then refill input while the previous output remained undrained.
This does not require source-egress loss, malformed packets or a native fault.
The deterministic negative control reproduces **4 input drops / 4 output drops**
and gap **expected 192 / received 384** with the former admission rule. The same
schedule with reserved admission delivers all twelve contiguous frames with zero
drops/gaps. This is causal fault injection, not a passing rerun as diagnosis.

`aux_execution::MediaWorker` now reserves one existing output slot for every
accepted native block, counting queued input, executing work and queued output
together. Credit returns only when media consumes output; while all four credits
are occupied, receive is disabled and the existing output timer continues. The
shared stereo/four-aux media path gains two scalar counters, no sample/packet
queue, new thread, priority, deadline or playout allowance. Native execution stays
nonblocking; genuine network gaps, full-lane witnesses and source wet deadlines
remain enforced. A normal deterministic regression covers both scheduling
boundaries and the old-rule negative control. An actual four-aux regression holds
output service until all input slots are free, proves admission remains closed,
and drains twelve exact source identities with no native gap/drop. The TLS fixture
now explicitly asserts zero native input/output drops and no source-gap witness.
This closes this local admission defect, not hardware or T3 timing qualification.

Combined lifecycle continuation: the existing actual runner fixtures now include
production BrainHost/ASRC/fake-duplex service and GBA1 encoding through the actual
source proxy handoff alongside PA, FX and raw REC. Both source return lanes and
Brain playback/status evidence lanes are pressured without controller service.
Capture closes independently while monitor transfer and source rendering continue;
fresh source/device epochs require new negotiation and explicit rearm, and a
retained old device closure cannot close the replacement. Raw PCM24 tail checks,
exact submitted = accepted + refused talkback receipt accounting, and source-side
hold/epoch/deadline checks remain enforced. This local harness
uses the production source handoff with an in-process media pump and the existing
2 ms synthetic source cadence. Monitor packets carry the permitted silent stream;
this is lifecycle/receipt evidence, not nonzero monitor playback or clock-quality
acceptance. Authenticated
FX lifecycle coverage remains in the separate existing TLS recovery fixture.

Focused validation passed **7 / 0 failed**: deterministic old/fixed burst schedules,
actual reserved-window continuity, deliberate gap/output-overflow witnesses, both
combined actual runner lifecycles, authenticated FX/REC recovery and four-aux TLS.
The final TLS run processed **705 blocks / 69,120 Main comparisons**, with explicit
zero native input/output drops and no source gap. All nine PA/FX/REC artifact hashes
matched. The complete normal hardware-host all-target suite passed **680 / 78 ignored**.
Default-feature library/binaries/examples and the selected production integration
targets passed **364 / 18 ignored**, with the same 21 unchanged offline-target
exclusions and seven automix-library exclusions documented below. No normal
source/control/transport target was omitted. Both configurations passed all-target
Clippy with warnings denied; formatting and diff whitespace checks passed.
Unchanged accelerated ASRC, other historical/audition/exhaustive, corpus-generation
and unrelated actual-owner opt-ins were intentionally skipped. No two-node/T3 or
physical checks ran.

The first normal parallel library run exposed two pre-existing generation
assertions. Added the existing numeric source receipt to both failure messages.
A discriminating rerun reproduced the hold case with **code -10** at frame 2688:
the fixture requested Hold(2) while Hold(1) remained active. The hold contract
requires Release before a new Hold; prior success depended on incidental deadman
expiry. The fixture now explicitly releases generation 1, then admits generation
2 with a fresh revision, asserting successful receipts for both and retaining old
media-generation rejection. Production authority/deadlines are unchanged. The
corrected parallel hardware-host library passed **223 / 32 ignored**. The subsequent
serial run exposed a separate fixture timestamp error: structural preparation
returned final reason **clock** because the fixture reused its requested startup
time after startup had charged preparation time. Both that test and the monitor
selection fixture now read the prepared source's control time; the monitor test
starts explicitly at time 7 so a literal time-zero request is deterministically
invalid. Source receipt diagnostics remain in both assertions. The three focused
clock/selection/hold cases passed; production clocks, leases and deadlines remain
unchanged. Final normal validation serializes independent test cases while retaining
their explicit worker/concurrency assertions.

All commands used the shared nonblocking `/home/shome/p/.gigpies-build.lock`,
Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1` and canonical
targets. Reproduce the focused seven with the existing verified manifest/PA/FX
environment and:
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib -- media_native_window actual_owner_reserved_window actual_owner_gap_and_output_pressure actual_runner_source_ actual_four_aux_tls_brain_controls_and_samples actual_tls_fx_pending_close_and_replacement_after_source_recovery --include-ignored --test-threads=1 --nocapture`.
The final normal hardware command was
`cargo +1.97.1 test --locked -j1 --features hardware-host --all-targets -- --test-threads=1`.

Closeout: after Git, shared-lock, running-build, process executable/open-file/map,
symlink/runtime-alias and hardlink checks, removed **134 task-built test
executables (12,218,617,856 allocated bytes)** and temporary validation logs
(**311,296 bytes**), reclaiming **11.38 GiB** in total. No candidate was retained
because of an active reference, and the temporary lifecycle directories are gone.
The canonical target retains **8,699,158,528 bytes (8.10 GiB)** of shared build/debug
artifacts and required runtime binaries; free space is **45,471,997,952 bytes**.
No rebuild followed cleanup. Retained deliverables are the admission fix,
regressions/lifecycle harness, corrected fixtures and this existing evidence/card;
original source, uncommitted work, user data and historical witnesses are preserved.
Everything remains uncommitted on canonical rpi5 `main`. No peers, extra checkouts,
sibling writes, hardware activation, commits or publication were used.

Current continuation findings (November 5):

- The combined authenticated test uses actual PA/FX/REC and production Brain
  media/device service, selected nonzero PFL playback, held talkback, independent
  raw analysis, saturated source return lanes, exact talkback receipt reconciliation,
  source replacement, new device epoch/map and authenticated output rearm. It keeps
  the original FX-only recovery case separately selectable. Raw PCM24 tails remain
  exact and recording reports incomplete with zero dropped frames on source loss.
- Fixture corrections preserve authority: pending then final Brain replies;
  separate scoped monitor/talkback writers; actual device configuration generation;
  independent playback evidence drainage; 1 ms source cadence for the 48 kHz fake
  Brain clock. No production deadline or queue was enlarged.
- Brain negotiation previously reserved F+480 before allocating ASRC storage.
  Allocation now precedes the unchanged reservation. A deterministic 24-quantum
  preparation delay protects that ordering; authority and exact-frame checks remain.
- Monitor receive now requires existing four-block service-lane capacity. The old
  deterministic burst schedule drops four blocks/jumps to frame 384; the new one
  delivers twelve contiguous blocks. This is separate from the closed native
  four-aux output-credit defect.
- The pressure deficit had a causal CPU witness: controller consumed about 0.885 s
  CPU in a 1.2 s test, source egress refused four offers, native input/output drops
  remained zero. Source command exchange busy-yielded while awaiting future frames.
  Completion publication now unparks its controller after publishing both return
  lanes, outside guarded audio work. The controller parks; a 100 ms wake only checks
  worker failure, never changes a media deadline. A regression waits fifty source
  quanta while requiring caller CPU below 20 ms. The same-pressure combined fixture
  passed five bounded trials after this change; the focused 26-case batch also passed.
- Resource witness from that batch's eight-input combined pressure interval: 137,146 us wall,
  480,935 us process CPU, controller 3,191,406 ns CPU, 16 threads, RSS 48,857,088 → 48,922,624 B,
  +3 minor/+1 major faults. Known source media layouts 1,450,456 B, plus 512 B global
  executor queue payload; duplex/native/assembly have separate reports. This is a
  bounded local lifecycle witness, not zero-fault or sustained four-core headroom
  qualification. The major fault and scheduling waits remain visible for T3.

Final local acceptance (November 5):

- Complete normal hardware-host all-target suite: **684 passed / 80 ignored**;
  library component **227 / 34 ignored**.
- Default-feature library/binaries, examples and selected production integration
  targets: **365 passed / 18 ignored**, using the unchanged 21 offline-target and
  seven automix-library exclusions documented below. The selected targets supplied
  363 passes and the examples supplied two; no affected production target was omitted.
- Focused actual-owner/source/queue/media batches: **26 + 6 passed**. The final six
  include two actual source lifecycles, native credit checks, four-aux TLS (694
  blocks / 68,352 Main comparisons, explicit zero drop/gap assertions), and combined
  TLS with stale device refusal and continued selected playback after capture closure.
- All **five accelerated release bridge opt-ins passed** in 193.27 s. Both all-target
  Clippy configurations passed with warnings denied; formatting and whitespace passed.
  Unchanged historical, exhaustive, audition, benchmark and unrelated actual-owner
  opt-ins were intentionally skipped. No T3, two-node or physical execution occurred.

Final combined resource interval additionally recorded 151,503 us wall / 508,872 us CPU,
16 threads, RSS 50,937,856 → 51,003,392 B, +4 minor/+0 major faults and controller 3,305,882 ns
CPU. The preceding interval's major fault remains recorded above; neither bounded
sample is a sustained headroom claim. Runtime reports preserve per-thread CPU,
faults and scheduling waits and explicit known-layout exclusions for later T3 work.

Reproduce with Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1`,
canonical targets and the shared nonblocking `/home/shome/p/.gigpies-build.lock`:
`cargo +1.97.1 test --locked -j1 --features hardware-host --all-targets -- --test-threads=1`;
the documented default selection plus `--examples`;
`cargo +1.97.1 test --release --locked -j1 --features hardware-host --test brain_bridge -- --ignored --test-threads=1`;
and `cargo +1.97.1 clippy --locked -j1 --all-targets [--features hardware-host] -- -D warnings`
for each configuration. Focused hardware library filters are `source_executor::tests`,
`remote::fx_execution::tests`, `remote::source_egress::tests`, `independent_analysis_admission`,
`brain_preparation_does_not`, `actual_tls_combined`, `actual_tls_fx_pending`,
`brain_runtime::media::tests`, `actual_four_aux_tls_brain_controls_and_samples`,
`actual_runner_source_`, `media_native_window` and `actual_owner_reserved_window`,
with `--include-ignored --test-threads=1` and the existing verified manifest/PA/FX environment.

**Exact next action: no local T0–T2 implementation or acceptance work remains.**
T3's two-node clean 5→30→60 gates, deadline margins, distributed headroom and physical
clock/socket/acoustic/sustained qualification require their separate authorization.
Do not restart extraction or treat those external qualifications as local blockers.

Local criterion audit (accepted):

| Gate | Concrete evidence | Disposition |
|---|---|---|
| T0 wire/identity/length/float/deadline/sequence/exhaustion | Independent literal GPA1/GBA1 vectors in `tests/audio_transport_v1.rs`; normal remote replay/permission tests | Complete; wire unchanged |
| T0 dimensions/grouping/resources | 16/32/48 layouts at 1232/1152/1008; checked proposed budgets and actual jitter/assembly/source/native/duplex layouts | Complete; opaque/native allocations distinguished from measured RSS |
| T0 design-to-code map | Current direct GBA1 ingress vs explicitly proposed 32-slot/four-ms reorder; actual analysis pool 4 vs proposed 32 | Complete under T0's explicit-proposal criterion; proposals are not implementation guarantees |
| T1 source/native isolation | Actual PA/FX/REC, source progress through controller/full-return pressure, exact raw samples and allocation/destruction guards | Complete local software acceptance |
| T1 prepared/closure/retirement | Global 16/128/16 lanes, one outstanding controller packet, reserved returns, independent closure fences, source reconstruction | Complete; bounded ownership and refusal paths validated |
| T1 legacy shapes | Actual 48/96-frame 1/2/3/16/48/256-channel comparison, configured 96, closure and common output-credit media path | Complete |
| T2 duplex/bridges | Normal numeric/lifecycle guards; five accelerated cases; combined TLS nonzero playback, receipts, independent closure, stale device refusal and rearm | Complete local software acceptance |
| T2 analysis | Independent read-only connection/runtime/encoder, exact PCM, duplicate/control refusal and encoder-stall isolation | Complete; shared provider QUIC runtime explicitly retained |
| T2 resources/admission | Existing 16-session and sole-owner limits, checked layouts, per-thread/process RSS/CPU/fault/scheduling reports | Complete local accounting; sustained distributed headroom remains T3 |
| T3 and physical | Clean two-node timing/margins, distributed headroom, physical mapping/clock/acoustic/sustained qualification | Outside authorization; unexecuted |


Task closeout: after live Git, shared nonblocking lock, active-build, process
executable/open-file/mapping (including privileged read-only inspection), symlink
and hardlink checks, removed **136 task-built test executables**
(**12,264,513,536 allocated bytes**) and **294,912 bytes** of temporary logs:
**12,264,808,448 bytes / 11.42 GiB reclaimed**. No active references or aliases
required retaining a test executable; temporary lifecycle directories are gone.
The canonical target retains **8,696,565,760 bytes / 8.10 GiB** of shared build/debug
artifacts and required runtime binaries; free space is **45,466,451,968 bytes**.
No rebuild followed cleanup. Source, existing uncommitted work, user data, required
runtime artifacts and the concise findings in this card are retained. Work remains
uncommitted on canonical rpi5 `main`; no peers, extra checkouts, sibling writes,
physical activation, two-node traffic, commits or publication were used.


Stereo extraction validation (November 5): normal hardware-host all-targets passed
**666 / 75 ignored**; default-feature library/binaries and the remaining production
integration targets passed **362 / 18 ignored**. Both skipped the seven unchanged
automix library cases. The default run omitted the unchanged offline targets
already exercised by hardware-host: ambience, automix, balance, bass, bleed,
bleed_reference, delivery, drums, dynamic_eq, effects, ensemble, expert, inventory,
loudness_hierarchy, matching, preservation, preset_dsp, private_automix, reference,
tone and unity. No normal source/control/transport target was omitted from that
second configuration. Focused validation passed 34 owner/clock cases; the first
combined lifecycle fixture hit the valid local-four-aux exclusion. Splitting it
into the two admitted configurations passed both lifecycle cases, preserving that
production exclusion. GP05, PA-v2 and the two current-artifact GP21 delay-wrapper
checks passed **15**. The old-library and generic-without-delay GP21 fixtures were
not selected against the current artifact.

The focused command adds `remote::fx_execution::tests` and
`remote::aux_execution::tests` to the existing clock/actual-runner/module selectors,
with the verified artifact environment and `--include-ignored --test-threads=1`.
The four new legacy-owner checks cover retained settled status through closure and
observation saturation, idle cancellation/expiry/closure, absolute run expiry and
retirement pressure. Every native quantum/idle test execution uses the host
allocation/destruction guard; it does not intercept allocations inside the dylib.
Normal loopback TLS/authentication regressions ran. Historical/audition/exhaustive,
corpus-generation, accelerated release bridge and actual TLS witness/falsifier
opt-ins were intentionally not rerun. The earlier hold-authority timing failure was not diagnosed in that checkpoint.
The later native-window correction closes the local TLS queue-drop mechanism;
these earlier passing reruns alone did not establish combined timing acceptance.

Stereo extraction closeout: both configurations passed all-target Clippy with
warnings denied, formatting and whitespace checks. All builds used Rust 1.97.1,
`--locked -j1`, `CARGO_INCREMENTAL=0`, `RUST_MIN_STACK` unset and the shared
nonblocking build lock. All nine module bindings and six PA receipt entries were
reverified, and both TLS witnesses retain their recorded lengths and SHA-256.
After shared-lock/process/open-file/mapping/runtime-alias/inode checks, removal of
**128 task-created test executables** and **five temporary logs/inventory files**
reclaimed **11,961,311,232 allocated bytes (11.14 GiB)**. Canonical target retains
**8,813,375,488 bytes (8.21 GiB)** of shared runtime/build artifacts; free space is
**45,373,280,256 bytes (42.26 GiB)**. No task runner scratch remains. Retained
deliverables are the source/tests and this checklist, blueprint and transport
reference; no one-use runner was added. Live heads remain GigPies `main/d4adff4`
and SHR PA `main/5f487d5`, with **53 modified tracked files / 14 untracked entries**
in GigPies and the same eight modified SHR PA files. Nothing is staged. No
physical activation, two-node traffic, host/network/service changes, sibling
writes, commits or publication. T0–T2 were still open at that checkpoint; T3 remains review-only.

Previous continuation validation note (November 5): the first complete-build
hardware-host attempt reached the library tests and exposed a malformed new
synchronous revision fixture; the corrected output-route request passes. A later
normal library run had one intermittent failure in
`local_audio::clocked_control_tests::clocked_authority_release_and_writer_revocation_close_copied_hold`:
the second command observed revision **1**, expected **2** (`src/local_audio.rs`,
assertion after `clocked_control`). The immediate isolated rerun passed. No numeric
refusal receipt was captured at that assertion, so the cause remains unresolved;
passing reruns do not establish timing acceptance. Both prior TLS witnesses are historical evidence for the later native-window
correction. An overbroad remote opt-in selector also reached
three unrelated environment-gated cases (EQ manifest, prebuilt Desk and fixture
output destination) and failed setup; no corpus was generated. The intended
remote selection passed 96 checks, including the new actual-aux stall and
recovery/rearm cases. The normal hardware-host all-target rerun passed **665 / 70 ignored**, with
seven unchanged automix library cases skipped after passing earlier. After the
final negotiation correction, hardware-host library checks passed **209 / 24
ignored / seven unchanged automix skipped**; focused clock/owner checks passed
**26**, and actual GP05/PA-v2 checks passed **13**. Negotiation now reserves against
the next mixer source frame rather than the media owner's previous rendered
frame; its first emitted packet must equal the reserved frame exactly. This final
correction received source/actual-owner validation after the broad run. Default-feature
affected source/control validation passed **221 / 11 ignored**, skipping the same
seven unchanged automix library cases. Both configurations passed all-target
Clippy with warnings denied, formatting and whitespace checks. Commands used
Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0`, `RUST_MIN_STACK` unset and the
shared nonblocking build lock. Reproduce focused coverage with
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib -- clock
actual_runner_source actual_module_detached_pa_packet_and_retirement
--include-ignored --test-threads=1`, plus the GP05/PA-v2 selectors and current
artifact environment. All nine artifact bindings and six PA receipt entries were
reverified before the final actual-owner group and at closeout. Historical,
audition, exhaustive, corpus-generation, accelerated release bridge and actual
TLS witness/falsifier opt-ins were not intentionally selected; the accidental
fixture selector failed before generation as noted above. Normal loopback
TLS/authentication tests ran. No physical activation, two-node traffic, host/
network/service changes, sibling writes, staging, commits or publication.

Previous continuation closeout: shared-lock, process, open-file/mapping, runtime
alias and inode checks preceded removal of **98 task-created test executables**,
five temporary logs/inventory files and one failed-setup temporary fixture.
Reclaimed **10,110,099,456 allocated bytes (9.42 GiB)**. Canonical target retains
**8,811,261,952 bytes (8.21 GiB)** of shared runtime/debug/build artifacts; free
space is **45,385,015,296 bytes (42.27 GiB)**. Source/tests and the updated owning
checklist/blueprint/reference remain; no task scratch directories or logs remain.
Both historical TLS witnesses retain their original lengths and SHA-256 values.
Live heads remain GigPies `main/d4adff4` and SHR PA `main/5f487d5`, with **52 modified
tracked files and 13 untracked entries** in GigPies, the same eight modified SHR PA
files, and nothing staged. Whole-source acceptance remains open.

Runner cutover validation (November 5; Rust 1.97.1, locked dependencies, one build
job, incremental off and shared nonblocking lock): the complete normal hardware-host
all-target run passed **665 / 69 ignored**. Final hardware-host library checks
passed **202 / 23 ignored** with seven unchanged automix cases skipped; selected
actual-owner checks passed **17** (runner lifecycle, clock PA/FX/REC, repeated PA
replacement/context, detached module, GP05 and PA v2). These final checks cover
queued monitor silence/closure and source-owned physical-start wiring added after
the broad compilation. Default-feature affected paths passed **277 / 14 ignored**,
again skipping those seven unchanged automix cases. The earlier focused clock
selection passed **18**, including independent rendered hold closure, expired
preparation and device intent. Both original TLS witnesses and all nine module/six
PA receipt bindings remain unchanged. Actual-owner TLS witness/falsifier opt-ins
were not rerun; normal loopback authentication regressions ran with the normal suite. No physical activation,
two-node traffic, host/service/network changes, sibling writes, staging, commits
or publication. Historical/audition/exhaustive/corpus and accelerated release
bridge opt-ins were intentionally skipped. Both configurations passed all-target
Clippy with warnings denied; formatting and whitespace checks passed.
Reproduce the new actual runner case with the artifact environment recorded below:
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib actual_runner_source
-- --include-ignored --test-threads=1`, with `RUST_MIN_STACK` unset and the same lock.

After shared-lock, process, open-file/mapping and inode checks, task cleanup removed
**104 test executables and four temporary logs**, reclaiming **10,388,705,280
allocated bytes (9.68 GiB)**. No task fixture directories remain. Retained
source/tests and this checklist/blueprint are the deliverables; shared runtime/debug
artifacts, historical receipts and both TLS witnesses are preserved. The canonical
target retains **8,808,665,088 bytes (8.20 GiB)** for shared libraries/runtime builds;
free space is about **42.28 GiB**. No rebuild was performed to verify cleanup.

Earlier autonomous owner continuation (November 5; historical baseline): the
following validation predates the runner cutover and is not whole-source acceptance.
The complete normal hardware-host all-target invocation passed **657 tests / 67
ignored**. The bounded clock cases include ordinary preparation continuing past
its expired admission, local lease/session cancellation before delayed commit,
independent stop, and actual PA/local-aux/raw-REC under both saturated return
lanes. This is owner-API evidence; the runner still uses synchronous quanta.
Default-feature affected-path selection passed **336 / 16 ignored**, excluding
unchanged automix already covered by the broad run. Final focused/owner checks
passed **18** (clock API 4, GP05 10, PA v2 3, detached module 1), with
`RUST_MIN_STACK` unset and all nine artifact bindings/six PA receipt entries
reverified. The later clock-frame overflow guard passed **4 / 1 ignored**; its
actual-library case was already covered above. Preparation time is charged from
entry, before buffer allocation, so it cannot extend copied local deadlines.
Both configurations' all-target Clippy, formatting and whitespace checks passed.
Reproduce the new actual case using the current manifest/config bindings and
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib source_clock --
--include-ignored --test-threads=1`, under the shared nonblocking build lock with
`CARGO_INCREMENTAL=0`. Unchanged historical/audition/exhaustive/corpus and release
bridge opt-ins were skipped. TLS was not rerun at that checkpoint; both historical witnesses remain
byte-for-byte unchanged. No sibling source/ABI/artifacts, physical activation,
two-node traffic, host/service/network settings, staging, commits or publication.

After process, open-file/mapping, runtime-alias/inode and shared-lock checks,
cleanup removed **124 task-created test executables** and **nine temporary
logs/inventory files**, reclaiming **11,603,509,248 allocated bytes (10.81 GiB)**.
No task fixture directories remain. Retained deliverables are source/tests and
this checklist/blueprint; shared debug/runtime artifacts, receipts and both TLS
witnesses remain. Target retains **8,681,078,784 bytes (8.08 GiB)**, justified by
current shared debug libraries and runtime builds; free space is about **42.43
GiB**. The final deadline-anchor check used Clippy metadata only and did not
recreate cleaned test executables.



Dedicated source-executor continuation validation (November 5; Rust 1.97.1,
`--locked -j1`, incremental off, canonical targets and nonblocking shared lock):
RenderGraph and HostSourceMedia now share the production `gp-source` worker, with
16 prepared-command slots, 128 numeric completion slots and 16 retirement slots;
return capacity is checked before mutation. Owned commands cover routing reset,
media negotiation/replacement and terminal retirement. Source-issued admission
rejects stale/expired preparation; the future-frame regression renders unchanged
frames before application without another controller pump. Production still uses
current-frame synchronous rendezvous. Structural acknowledgments precede graph
reattachment/next PA exchange, and refused module exchanges restore preparation
and retirement reservations. Recovery reinstalls the graph on the same worker.
Graph/quantum storage is boxed: actual TLS exposed construction-stack overflow,
fixed without increasing the default test-thread stack. Final owner validation
ran with `RUST_MIN_STACK` unset.

Normal hardware-host coverage across the broad run and corrected reruns:
**644 passed/66 ignored** (not one uninterrupted full-suite invocation). Final
compiled affected-path selection: **454 passed/62 ignored**; default-feature
selection: **349 passed/18 ignored**. Seven unchanged automix library cases and
unchanged integration algorithms were covered by the broad run rather than
repeated in affected selections. Reverified all nine artifact bindings and six
PA receipt entries before actual-owner tests: **23 passed** (library 10, GP05 10,
PA v2 3), including shared-worker recovery and actual detached PA/FX/REC paths.
Final isolated TLS checked 690 blocks and 67,680 Main samples. Before that pass,
a TLS source-gap refusal recurred with input drops 1/output drops 5 while a
read-only GDB layout inspection overlapped; that run did not isolate scheduling.
The native-window regression in this card subsequently reproduces and fixes the local
queue-drop mechanism. Preserve
`user/node-lab/gp-h1-source-executor-tls-witness.log` (2,133 bytes, SHA256
`0414aa8f150cc6797fbda800073fd1cd18035739fedbfdfbac0deb13af392398`), alongside the
unchanged original 2,040-byte TLS witness. The isolated rerun alone did not diagnose that failure or establish whole-source acceptance. Historical/audition/exhaustive,
old-artifact corpus and unchanged accelerated release bridge opt-ins were
intentionally skipped. No physical activation, two-node traffic/load or T3 work.
Both configurations passed all-target Clippy with warnings denied; formatting and
whitespace checks passed. The final lint-only closure simplification did not
change tested behavior.

After nonblocking build-lock, idle-process, open-file/map and runtime-alias checks,
removed **127 task-created test executables**, **10 temporary logs** and **seven
known disposable fixture directories**, reclaiming **11,755,495,424 allocated
bytes (10.95 GiB)**. Retained production source, runtime binaries, shared debug/
build artifacts, native bindings/receipts and both small TLS witnesses. Canonical
target remains **8,799,387,648 bytes (8.20 GiB)**; free space approximately
**42.31 GiB**. No rebuilding after cleanup. Final Git state: GigPies `d4adff4`,
51 tracked modifications/10 untracked entries; SHR PA `5f487d5`, eight preexisting
modified files. Nothing staged, committed or published; no sibling writes.

Persistent-graph/ingress continuation validation (November 4; Rust 1.97.1,
`--locked -j1`, incremental off, canonical targets and nonblocking shared lock):
complete normal hardware-host `--all-targets` **638 passed/66 ignored**. Subsequent
actual-owner checks exposed terminal recorder retirement without a final capture
block and two remote aux readbacks still borrowing the detached mixer. A bounded
terminal module exchange and copied applied FX-send observations fixed those paths.
Final affected hardware source/control/transport targets (library, binaries and
all integrations except unchanged offline mixing/analysis algorithms already run
in the complete suite): **453 passed/62 ignored**. Final default library and
source/control/transport targets: **202 passed/11 ignored**. Both configurations
passed all-target Clippy with warnings denied; formatting and whitespace passed.
The GP06 disconnect fixture now polls within one second at fixed provider time,
so it checks socket EOF without assuming one scheduler tick or substituting idle
expiry. Ingress fixtures explicitly service the source boundary before asserting
accepted counts/readiness and retain the existing wet fade expectation.

Reverified all nine artifact bindings and six PA receipt entries before the final
actual-owner group: **22 passed** (GP05 10, PA v2 3, detached module 3, combined
owned kernel 1, successive-EQ Unix/remote campaign 1, recording/Brain-loss recovery
1, persistent local aux 1, four-aux roundtrip/loss 1, exact TLS 1). New normal tests
cover detached patch/safety/operator/FX observations, saturated media ingress and
receipts, expiry, replacement generation, same-bridge renegotiation and final
queued-ingress retirement. These results do not establish dedicated source
scheduling or whole-source independence. All five accelerated release
`brain_bridge` opt-ins passed in **193.28 s**. Unrelated historical, audition,
exhaustive and old-artifact corpus opt-ins were intentionally skipped. No physical,
two-node or T3 work ran; sibling sources and owner artifacts were unchanged.

After idle-process/open-file/map, runtime-alias, durable-reference and nonblocking
build-lock checks, removed **94 task-created test executables** and
**14 temporary logs/helpers/inventory files**, reclaiming
**9,435,815,936 allocated bytes (8.79 GiB)**. Retained canonical runtime
binaries and shared debug/build artifacts; target remains **8,778,784,768 bytes
(8.18 GiB)** and free space approximately **42.34 GiB**. Historical receipts and
the historical **2,040-byte TLS witness** remain unchanged. Its recorded harness
path is invocation history; the newly rebuilt harness at that reused path was
removed, without modifying the witness. GigPies remains `main/d4adff4`,
48 modified tracked files and nine untracked entries; SHR PA remains `main/5f487d5`
with its existing eight modified files. Nothing staged, committed or published.

Owning-graph/command continuation validation (November 4; Rust 1.97.1,
`--locked -j1`, incremental off, canonical targets and nonblocking shared lock):
normal hardware-host `--all-targets` **631 passed/66 ignored**. Actual successive-EQ
acceptance then exposed reuse of an occupied returned retirement slot; controller
retirement/reset and a second-commit/third-preparation regression fixed it. Final
hardware source/control/transport targets (library, binaries and all integration
targets except the already-passed unchanged offline mixing/analysis algorithms)
**450 passed/62 ignored**; default `--lib` plus `gp05`, `master_eq`, `brain_control`,
`gp07_local`, `sends_local`, `structural_control`, `structural_producer`,
`modular_engine`, `metering` and `spectrum`: **139 passed/6 ignored**. Final library
validation after lint-only cleanup: **174 passed/20 ignored**. Both configurations
passed all-target Clippy with warnings denied; formatting and whitespace passed.
New guarded regressions cover detached prepared application, revocation before
versus after commit, external receipt ordering/replay, owned capture/playback with
32 controller-stalled quanta and controller-drop closure, and host media saturation.
The new mapping test required valid slot permutations and separate private authority
directories; the new fader race test required a value distinct from the initial
−6 dB. Those fixture corrections did not weaken production checks.

Reverified all nine PA/FX/REC library/header/owner-manifest bindings and the six
PA receipt entries before the actual-owner group: **22 passed** (GP05 10, PA v2 3,
detached module 3, combined owned kernel 1, actual master-EQ Unix/remote campaign 1,
recording/Brain-loss recovery 1, local aux 1, four-aux roundtrip/loss 1, exact TLS 1).
The local-aux test is under `local_audio::fx_aux::actual_tests`; an initial filter
selected zero cases and was corrected with the exact full name. Actual inputs remain
`user/node-lab/gp-h1-modules.json` and the existing owner fixtures; no sibling source,
ABI, library or historical receipt changed. The TLS case passed **708 blocks/69,504
Main sample comparisons**; the local queue-drop mechanism is now closed in this card and
its 2,040-byte historical witness is unchanged. All five accelerated release `brain_bridge`
opt-ins passed in **143.56 s**. Unrelated historical, audition, exhaustive and
old-artifact corpus opt-ins were skipped; no physical, two-node or T3 work ran.
This validates the wired ownership/command/lifecycle changes, not a persistent
production source executor or whole-source controller-stall acceptance.

Continuation cleanup, after idle-process/open-file/map, inode, runtime-alias,
durable-reference and nonblocking build-lock checks: removed 91 task-created test
executables and eight temporary logs/inventory files, reclaiming **8,980,271,104
allocated bytes (8.36 GiB)**. Normal runtime binaries, shared debug/build artifacts,
historical receipts and the historical TLS witness remain. Target retains
**8,655,732,736 bytes (8.06 GiB)**; free space is **42.46 GiB**. Live GigPies remains
`main/d4adff4`, 46 modified tracked files and nine untracked entries; SHR PA remains
`main/5f487d5` with its existing eight modified files. Nothing is staged.

Numeric-render/observation/egress continuation validation (Rust 1.97.1,
`--locked -j1`, incremental off, canonical targets/shared nonblocking lock):
hardware-host all-target check passed; final normal hardware-host `--all-targets`
**624 passed/65 ignored**. Default `--lib` plus `gp05`, `master_eq`,
`brain_control`, `gp07_local`, `sends_local`, `structural_control`, `metering` and
`spectrum`: **117 passed/5 ignored**. Unchanged automix integrations ran only in
the hardware-host suite. Actual-owner group **23 passed**: GP05 (10),
`module_graph_pa_v2` (3), module detached/numeric PA (2), combined numeric kernel
(1), local aux (1), scoped `remote::aux_execution::tests::actual_owner_` (5), and
the exact four-aux TLS test (1), with required owner bindings and
`--include-ignored`. The final TLS run checked **727 blocks/71,424 Main samples**;
the local queue-drop mechanism is now closed in this card and its 2,040-byte witness
is unchanged. Library/header/owner-manifest hashes and the private PA source
receipt matched. No sibling code/ABI or owner artifacts changed. ASRC delivery is
unchanged: accelerated bridge and SHR PA suites were not repeated. Historical,
audition, exhaustive, physical and two-node/T3 tests remained skipped.
Review restored output clearing on preparation refusal, preserved the existing
end-frame talkback authorization semantics and replaced the local-aux numeric
failure-path reset with the existing bounded reset. The final suites include
those fixes. Formatting/whitespace passed; no fixture changes. This remains a
synchronous production integration checkpoint, **not completion of the requested
whole-source owner/global admission/lifecycle/caller transfer**. T0–T2 were still open at that checkpoint.
The combined kernel check is repeatable with hardware-host `--lib
actual_combined_render_pa_fx_raw_rec_without_controller_pump -- --include-ignored`
and the verified GP05/PA bindings above. After all workers/builds/tests were idle,
checked processes, the shared lock, open files, tracked paths and executable
provenance. Removed 88 task-built test executables, three logs and one inventory:
**8,865,681,408 allocated bytes reclaimed**. Preserved source/tests/docs, historical
receipts/TLS witness and shared runtime/build artifacts. Canonical target retains
**8,556,134,400 allocated bytes**; 43 GiB is free. No rebuild followed cleanup.
Live state: GigPies main/d4adff4, **45 modified tracked/9 untracked entries**;
SHR PA main/5f487d5 retains its original eight modifications. Nothing staged,
committed or published.

Detached-PA/session-closure continuation validation (Rust 1.97.1, locked,
one job, incremental off, canonical targets and the nonblocking shared lock):
hardware-host all-target check passed; normal hardware-host all-target tests
**621 passed/63 ignored**. Default-feature `--lib --test gp05 --test master_eq`:
**84 passed/2 ignored**; unchanged shared automix integrations passed in the
hardware-host run and were not repeated. Formatting/whitespace passed; no fixtures
changed. Native-library group **17 passed**: `--lib
actual_module_detached_pa_packet_and_retirement`, `--lib
remote::aux_execution::tests::actual_owner_`, `--test gp05` (all with
`--include-ignored`), plus the exact four-aux TLS test. The module regression checks
actual PA/local-FX samples and guarded capture/retirement; GP05 covers raw REC
lifecycle/faults separately. These do **not** establish whole-source acceptance.
PA/FX/REC library/header/owner-manifest bindings and the private PA source receipt
matched; no sibling source or owner binaries changed. ASRC execution/delivery is
unchanged, so accelerated ASRC and SHR PA normal suites were not rerun. Historical,
audition, exhaustive, physical and two-node/T3 tests remained skipped.
The exact local four-aux TLS test passed with **72,480 Main sample comparisons**.
The local queue-drop/source-gap mechanism is now closed in this card; its 2,040-byte historical witness is
unchanged. Production still requires controller prepare/finish and synchronous
external commits; no whole-source rendering through controller stalls is claimed.
After all validation/agents were idle, checked the shared lock, processes, open
files, tracked paths and executable provenance. Removed 82 task-built test
executables plus three logs and one temporary inventory: **8,272,543,744 allocated
bytes reclaimed**. Preserved shared runtime/build artifacts, source/tests/docs and
historical evidence; canonical target retains **8,555,651,072 allocated bytes**,
with 43 GiB free. No rebuild after cleanup. Live state: GigPies main/d4adff4,
44 modified tracked/9 untracked entries; SHR PA main/5f487d5 retains its original
eight modifications. Nothing staged, committed or published.

Mixer-observation continuation validation (Rust 1.97.1, `--locked -j1`,
`CARGO_INCREMENTAL=0`, canonical target/shared nonblocking build lock): focused
`--features hardware-host --lib mixer_control::` **6 passed**, including
allocation/destruction guards, retained retirement and pending-readback isolation.
Hardware-host `--all-targets`: **620 passed/61 ignored**. Default-feature `--lib`
plus `gp03`, `gp03_alloc`, `gp07`, `gp07_alloc`, `gp07_local`, `sends`,
`sends_alloc`, `sends_local`, `gate`: **147 passed/5 ignored**. The unchanged
shared automix integration suite was not repeated under default features after
passing the complete hardware-host suite. Formatting/whitespace passed; no fixture
regeneration. Reverified PA/FX/REC library/header/owner-manifest bindings and the
private PA source receipt. Exact local
`remote::tests::fx_aux_tls_tests::actual_four_aux_tls_brain_controls_and_samples`
with `--exact --include-ignored --nocapture` and `GP_FX_LIBRARY` passed once
(**1 test**, 72,384 Main sample comparisons). The later native-window regression
supplies the causal evidence for the local queue-drop mechanism; the 2,040-byte
historical witness is preserved. No ASRC execution/delivery
or SHR PA changes: their suites were not repeated. Historical/audition/exhaustive
opt-ins, physical and two-node/T3 work were skipped. These checks validate the
mixer observation/retirement change, not complete source independence.
After process, shared-lock, open-file and tracked-path checks, removed 89 task-built
test executables and three temporary logs: **8,926,613,504 allocated bytes reclaimed**.
Retained source/tests/docs, historical receipts/TLS witness and shared runtime/build
artifacts. Canonical target retains 8,556,298,240 allocated bytes; 43 GiB is free.
No rebuild after cleanup.

Preceding source-acquisition continuation validation used Rust 1.97.1, locked dependencies,
one job, incremental off, canonical target and the shared nonblocking build lock.
Focused measurement unit tests: **9 passed**; detached capture allocation/destruction:
**1 passed**; measurement contracts/provider: **5 passed**; spectrum integration:
**2 passed/1 ignored**. Verified PA/FX/REC library/header/manifest bindings and the
private detached-PA source receipt. The actual PA measurement test
`gp20_owner -- --include-ignored` passed with the unchanged historical two-output
graph (SHA256 `c8be83f3908e2da937d2ccd088d7203fd3cb649aef6b832ca85bfa08c64b5ba7`)
and a temporary pinned configuration for the existing `shr-pa-measure` executable
(SHA256 `ef773d4e7a9ddfd9f0a52504f088d9599b56ef55d0af29a33518f70f539a7d83`).
No fixture or owner binary changed. Default all-target: **525 passed/22 ignored**
before the final progress-publication ordering and startup-storage corrections.
Hardware-host validation exposed a stack overflow in
`remote::runner::output_window_tests::legacy_provider_config_defaults_fake_and_physical_denies_before_io`.
Boxing spectrum acquisition at startup fixed the exact regression with normal test
stack settings; it adds no allocation to acquisition. Final hardware-host all-target:
**620 passed/61 ignored**. Final default-feature library: **77 passed/2 ignored**.
The actual PA measurement case was repeated against the final executable and passed
(**1 test**) after rechecking library/header/manifest/source bindings. Formatting
and whitespace checks passed. These are synchronous integration/acquisition results,
not whole-source independence.
ASRC execution/delivery and SHR PA are unchanged; accelerated ASRC and the SHR PA
normal suite are intentionally skipped. Unrelated historical/audition/exhaustive
opt-ins, physical and two-node/T3 tests are skipped. The exact four-aux TLS
reproduction was not rerun during that acquisition continuation; the latest
check at that checkpoint is recorded above; native-window closure is recorded in this card.

The exact four-aux TLS check FAILED: bus 0 accepted; bus 1 request `5`, epoch `9`,
progressed preparing → prepared → unknown, reason **`FX aux source gap`**, null
`effective_frame`. This is numeric refusal cause 1: the FX executor received a
frame different from its expected next frame. It does not locate the upstream
loss or establish the cause of the older bus-3 generic refusal. Failure cleanup
in `remote/fx_aux_tls_tests.rs` now joins owners and prints existing execution,
media and source-egress reports before preserving the assertion failure; it does
not retry mutations or change deadlines/queues. One explicit diagnostic reproduction passed (1 test); the queue-drop mechanism
was subsequently reproduced and closed by the native-window regression. Exact failed output and both outcomes are retained in ignored
`user/node-lab/gp-h1-tls-witness.log`.
After shared-lock, process, open-file and mapping checks, cleanup removed
155 task-built test executables plus six temporary files (validation logs, pinned
measurement config and generated protocol corpus): **13,810,147,328 allocated bytes
reclaimed**. No rebuild after cleanup. Retained source/tests/docs, historical receipts,
the unchanged **2,040-byte** TLS witness and shared runtime/build artifacts:
GigPies target **8,554,262,528 bytes**, SHR PA target **753,475,584 bytes**. These
normal shared outputs remain justified; free space is approximately43GiB.
GigPies remains `main/d4adff4`, **40 modified tracked / 9 untracked entries**, nothing
staged. SHR PA remains `main/5f487d5`, its original **8 modified tracked files**,
nothing staged. No commits, publication, extra checkouts, workers, sibling writes,
hardware activation or host/network changes. No dedicated Stagebox execution owner
was wired; the requested combined production source isolation remains incomplete.




Acceptance checklist:

- [x] Reviewable architecture, execution map, exact existing binary media profile,
  conditional UDP session design, clock/safety/resource limits and alternatives.
- [x] Preliminary mechanism-specific patent/openness review, with unverified status
  and licensing adoption clearly separated from engineering claims.
- [x] T0 contract vectors/resource fixtures accepted.
- [x] T1 source/FX execution, prepared commits, atomic closure and off-thread
  retirement accepted by local code validation; traffic acceptance belongs to T3.
- [x] T2 independent duplex/background execution and both ASRC crossings accepted.
- [ ] T3 clean combined5/30/60 gates, samples and recovery accepted.
- [ ] T4 comparison completed or explicitly declined as unnecessary after T3.
- [ ] T5 security-preserving replacement accepted, or QUIC retained and candidate removed.
- [ ] T6 one production path and concise handoff; physical qualification remains separate.

Next action: local T0–T2 and lifecycle/analysis/resource acceptance are complete.
Preserve the implemented execution boundaries. Only separately authorized T3
distributed timing and later physical qualification remain.
Do not send two-node test traffic during code preparation. A separately authorized
5s pass qualifies proposing30s, not an automatic run or a jump to60s; failures
require diagnosis before escalation. Only audio uses the wet-return deadline;
commands/meters/analysis retain their own contracts. Keep768frames and the60s
synthetic cap. Never subtract unsynchronized clocks or label application dequeue
as socket arrival. Minimum4ms wet margin and material2ms replacement benefit are
new proposed acceptance thresholds, not historical results.

Historical local implementation increment before the dedicated duplex service
(not T1/T2 acceptance):

Continuation inspected live HEAD `d4adff4239b71e143e9a33e8105d5a0624fb5e9f`:
two dependency-maintenance commits follow the planning base. Existing unstaged
work was preserved. The task0026 manifest still describes the historical base;
it is not a current source/artifact receipt. PA/FX/REC library, header and owner
manifest SHA-256 values freshly match `ethernet-0026/modules.json` locally.
All twelve sibling heads/lock/toolchain hashes still match that manifest and their
trees are clean. No peer inspection, source synchronization or sibling writes
were needed.

- Added an independent GPA1 FX-send vector and malformed-header/sequence-origin
  coverage. Actual jitter/assembly requested-layout accounting now includes tags,
  alignment, spec capacity and samples; this is still not aggregate prepared
  memory or RSS. GBA1 reorder behavior is explicitly marked as proposed.
- Provider evidence now includes lifetime counts for all four proxy overflow
  lanes, including sessions retired during the run. A shared fixed ledger avoids
  a final-sample race without retaining a growing session history.
- BrainAux swaps prepared configurations without cloning at commit, retains old
  storage until controller polling, and defers failed-token cancellation and owner
  reclamation outside processing. Bounded ABI wrappers preserve allocation-free
  error paths. The occupied pending slot blocks further preparation until its
  result/retirement is consumed. This does not introduce separate threads.
- T0–T2 were incomplete at that checkpoint. Global prepared commits, atomic closure, dedicated
  source/FX execution, actual off-thread retirement, independent duplex service
  and aggregate resource qualification are still absent. No sibling ABI blocker
  was found; these are unfinished GigPies implementation requirements.

Continuation validation: default all-target suite **514 passed/22 ignored**;
hardware-host **601 passed/55 ignored**; zero failures. Five focused proxy tests
and all eight `audio_transport_v1` tests passed, including the actual-library
allocation/destruction guard with stalled polling, unpermitted prepared tokens,
source gaps before publication and during transition, and panic. Both targeted
actual-owner regressions passed (`actual_four_aux_source_brain_roundtrip_and_loss`
and `actual_host_recording_survives_brain_loss_and_finalizes_clock_fault`). All
five accelerated release `brain_bridge` opt-ins passed in 146.02 s test execution.
Hardware-host all-target Clippy with warnings denied, formatting and diff checks
passed; the embedded T3 schema/example validates and retains unresolved payload
capacity. These local checks do not establish dedicated execution, global safety
closure, aggregate headroom, combined-network timing or physical acceptance.

Run the new actual-owner guard with verified `GP_FX_LIBRARY` and the same build
lock/toolchain/environment below: `cargo +1.97.1 test --locked -j1 --features
hardware-host --test audio_transport_v1 -- --include-ignored`. Historical research,
audition/exhaustive matrices, other unrelated environment opt-ins, two-node
traffic and physical tests were intentionally not run. No commits, publication,
workers, extra checkouts, sibling writes or host configuration changes occurred.

Continuation cleanup: after the build-lock, process/open-file, inode and live Git
checks, removed 156 task-built test executables and the temporary validation
journal, reclaiming **13,587,185,664 allocated bytes**. Normal target retains approximately
7.8 GiB of runtime executables and useful library/debug/build artifacts;
free space is approximately 42.9 GiB. Private evidence and all initial
uncommitted work remain. HEAD is unchanged at `d4adff4`; 12 tracked files are modified
and 3 are untracked, with nothing staged. T0–T2 were still open at that checkpoint.

Earlier prerequisite increment and historical validation:

- Preserved the pre-existing blueprint/reference edits on HEAD `97a7949` and
  verified all 13 local canonical revisions/lock/toolchain hashes against the
  private manifest. Fresh read-only Tailscale SSH confirmed Pi4 GigPies at the
  same clean revision. Historical executable receipts were not reused as proof
  of current binaries. Private task0026/final receipts and concise evidence read.
- T0 adds independent literal GPA1/GBA1 vectors, truncation/identity/float/deadline/
  wrap/exhaustion tests and checked payload arithmetic at 16/32/48 inputs. Existing
  suites retain authentication, role/map/replay/rearm tests. The blueprint records
  sequence-origin, strict final-admission and algorithmic-delay reconciliations.
- T1 prerequisites replace allocating proxy ingress payloads with inline Copy
  datagrams, count all four proxy queue overflow paths without optional tracing,
  and use fixed 32-slot/bitmap Brain FX assembly with a prepared completion buffer.
  Allocation/destruction tests cover assembly success, overflow and refusals.
  These are not dedicated source/FX execution or four-block role-only handoffs.
- Remaining T1 work is in GigPies, not a demonstrated sibling ABI obstruction:
  split `OfflineEngine::process_interleaved` authority cloning/readback/completions
  from source commits; split LocalAudio control and module lifecycle; move FX
  preparation/hash/serialization and retirement to independent execution;
  add global prepared commits, copied coherent observations, non-droppable closure
  generations and bounded retirement reservation. Do not move the whole allocating
  pump and mark it complete. Source-clock and safety semantics remain unchanged.
- T2 is not implemented: `brain_runtime::run` still shares device service with
  control awaits and diagnostics. Both ASRC bridges and explicit device/rearm
  contracts remain as before. Aggregate four-core scheduling headroom is unmeasured.
- A credential-free T3 metadata schema/example and refusal/pass criteria are in the
  owning blueprint. It is deliberately not an executable runner configuration;
  unresolved payload capacity and unverified runtime artifacts prohibit launch.
  No T3 traffic, T4/T5 adapter or physical work was run.

Validation: focused vectors/transport and proxy regressions passed; complete
normal default suite 511 passed/22 ignored, hardware-host 598 passed/54 ignored,
zero failures. All five accelerated release `brain_bridge` opt-ins passed in
146.08s. Hardware-host all-target Clippy with warnings denied, fmt and diff checks
passed. The embedded T3 JSON schema/example validates. Fresh local library/header/
manifest SHA-256 checks matched the recorded PA/FX/REC artifacts; both targeted
actual-owner opt-ins passed: `actual_four_aux_source_brain_roundtrip_and_loss`
and `actual_host_recording_survives_brain_loss_and_finalizes_clock_fault`.
These are device-free harnesses, not the combined two-node graph.

Reproduce with the existing nonblocking `/home/shome/p/.gigpies-build.lock`,
`CARGO_INCREMENTAL=0`, Rust 1.97.1, `--locked -j1`: normal `test --all-targets`
with and without `--features hardware-host`; focused `--test audio_transport_v1`;
release hardware-host `--test brain_bridge -- --ignored`; the two named owner
`--lib TEST -- --ignored` cases with verified `GP_FX_LIBRARY`/`GP05_MANIFEST`.
Unrelated historical/audition/exhaustive and other environment opt-ins stayed
ignored; no physical or two-node traffic test was launched. No commits or
publication were performed. These historical checks alone did not complete isolation. T0/T1/T2 acceptance
subsequently completed November 5; the current checklist records that result.

Closeout: after build-lock/process/open-file checks, removed 156 task-built test
executables and the disposable logs/reference copy, reclaiming 13,558,190,080
allocated bytes. Normal target remains 6.3 GiB for runtime/debug/dependency
artifacts; free space is 45 GiB. Existing private evidence and all initial
uncommitted blueprint/reference edits are preserved. No new scratch directories,
worker sessions, checkouts, sibling changes, commits or publication were created.

Remaining acceptance outside the cable-only phase: H8 timing discontinuities and
attenuated right return; real ADAT/socket mapping; independent duplex drift and
electrical/acoustic latency; aggregate FX/PA/REC deadlines; controller/LED behavior
(GP-H2), acoustic protection and whole-show recovery (GP-H3). Master gain/speed
provider integration and saved-show work remain software tasks, not cable tests.
The independent dev-PC lane is SHR Lux LX-06's software-only output-worker slice;
its owning module card defines the scope. No audio-side repository is shared with it.

## GP-H2 — physical dual-console operation

State: IN PROGRESS; semantic menu software is implemented; physical acquisition
and learned mappings are incomplete, and physical acceptance
requires a separate authorized session.

Required outcome: complete live operation from the two assigned MIDI controllers,
without a computer keyboard or mouse. Computer shortcuts remain optional adapters
to the same semantic actions, with identical authority/review/recovery checks.

Software and hardware acceptance:
- [x] Provide labelled semantic menus and bounded literal value entry for current
  Desk/Lightdesk operations; keep unsupported owner capabilities unavailable.
- [x] Replace Desk Channel's synthesized key events with typed actions; expose
  grant/release, reconnect, review scrolling and display controls semantically.
- [x] Route Lightdesk value entry, multi-selection, grants/modes, cue/palette
  recording, GO, release masks, blackout release and scene management through
  its existing Lux worker and material-review fences.
- [x] Test stale generation/target, held/repeated pad messages, wrong endpoints,
  missing role, exact entry and hidden-review refusal. Verify an actual Lux
  null-output value/review/apply episode from bound pad messages.
- [ ] Connect production OS MIDI acquisition/profile loading to the role-fenced
  complete-message adapters; capture the actual devices' messages and encodings.
- [ ] Qualify the intended direct sixteen-encoder Channel mapping, device identity,
  pad banks, LEDs, disconnect/release recovery and simultaneous two-desk operation.

Software change: both consoles now have labelled eight-pad semantic menus,
printable character entry and explicit endpoint/channel/note bindings, with no
assumed MiniLab preset. Desk's Channel targets now dispatch typed actions instead
of synthesizing keyboard events. Its formerly key-only grant/release/reconnect,
review-page and display controls have semantic commands. Lightdesk uses the same
Lux worker for menu operations, including mixed/multi-selection value entry,
reviewed master/checkpoint, release masks and scene grants/import/export.
A completed controller action and its release share one Lightdesk queue item,
so a release cannot erase the action's refusal. Menus cannot mark hidden reviews
as presented. Generation, target and press/release fences remain required.

Evidence and remaining boundaries: `shr-desk/src/controller_surface.rs`,
`shr-desk/src/frontend/controller.rs`, `shr-lightdesk/src/controller_surface.rs`,
and both owners' `docs/CONTROLLER.md#production-semantic-menu`. The adapter APIs
accept complete messages; no production OS MIDI acquisition worker/profile loader
or verified physical binding is claimed. The native compatibility window retains
its old input adapter; the new menu presentation is integrated in the terminal.
Provider-unadvertised actions (including palette recall/group capabilities) remain
unavailable, not simulated. Do not close this card on menu or identity tests alone.

Software verification (2026-10-27): both owners' default and `--features native`
normal suites (Desk: 347 default / 350 native, 24 opt-ins ignored in each;
Lightdesk: 138 default / 139 native, 6 / 7 opt-ins ignored), all-target
warning-denied Clippy in both configurations, formatting, and focused
controller/entry/role/review regressions. Menu scene previews were
inspected at 1920×1080. Explicit actual-provider check:
`cargo +1.97.1 test --locked --offline --test native_provider controller_menu_real_provider_value_and_material_review -- --ignored --nocapture`
in Lightdesk, with absolute `GP_ROLE_BROKER` and `GP_LUX_PROVIDER` paths. It uses
only owned synthetic/null-output children and private sockets; no physical MIDI,
audio, DMX or window is opened. The tested broker SHA256 is
`58d0a2d9fcea3b834209a1c6f72fdd3a32066927730e4ffbdfada30d10a68a4a`;
the tested Lux service SHA256 is
`9c861fb2f654fe1d0075ca502b48bf087552eb13fbfaf3a27463d60708af97e4`.
Other historical/exhaustive/actual-provider campaigns and physical tests were
intentionally not rerun. Console-local ignored `artifacts/controller-menu/`
retains test logs and menu previews; this is software evidence, not hardware acceptance.

Controller hookup is deferred at the operator's request (2026-10-27): reuse
SHR DAW's Arturia MiniLab mkII learned mapping when the powered-off Raspberry Pi
is available. The public DAW mkII profile intentionally contains no guessed
notes/CCs; no learned file was found locally. No input-driver change was made
during that follow-up investigation.

Next: connect learned profiles through role-fenced input workers, then verify
the two actual display/controller identities, separate input/LED
ownership, focus/held-release, restart/reconnect and operator usability. Follow
[roles](../reference/ROLE_BINDING.md) and each console's controller/native contract. Injected
role descriptors and offscreen rendering do not qualify physical operation.

## GP-H3 — whole-show qualification

State: PLANNED; requires qualified audio/console paths and a reserved session.
Next: measure the actual combined Stagebox recording/protected audio and Brain
FX/Lux/analysis/two-console workload, deadlines, control responsiveness, memory,
thermal behavior and failures. Physical lighting requires Lux LX-06 first.
Use [Brain responsibilities](../architecture/BRAIN_CONSOLES.md) and owner acceptance limits.
Include ordinary and burst activity from the
[bounded persistent journal](NODE_LAB.md#pi-5-reboot-diagnostics), enabled on Pi 5
with explicit operator authorization on 2026-10-20. On-disk marker verification
passed; live-audio latency and survival across an actual reset remain unqualified.

## Verification and handoff

Follow each changed owner's AGENTS.md and current test policy. Use focused normal
regressions during work and required complete normal suites for shared render,
schema, transport, routing or safety changes. Actual-owner/provider/consumer checks
must use the production graph. Keep historical auditions, exhaustive research,
physical and long-load campaigns explicit. Serialize builds under the shared lock;
use pinned Rust, locked dependencies, jobs=1 and CARGO_INCREMENTAL=0.

Before completion, update this task's state/checklist/evidence/next action, preserve
other workers' edits, inspect named diffs and publication guards, and maintain only
source-linked knowledge routing. Follow the owner's publication authorization;
source acceptance does not deploy services or authorize device activation.


## GP-GATE — lightweight channel gates and tom decay control

State: **IN PROGRESS**, implementation started 2026-10-30. Owner: GigPies channel
DSP and soundcheck automation; Desk consumes processing v5. Core, live strip,
persistence and measured-proposal preparation/import are implemented; software validation and
remaining workflow integration are tracked below. Accepted V8 settings/audio unchanged.

Outcome: reduce between-hit bleed with a smooth, retriggerable closing fade,
without losing intended quiet hits or adding material multichannel CPU load.
“Hold” retains an open gate; “release” is the closing fade; “range” limits the
attenuation rather than requiring complete silence. Hysteresis gives opening
and closing different thresholds to avoid chatter.

### Processing and modes

- Default bypass, zero added audio delay, fixed per-channel state. One reusable
  DSP core serves the live channel path and any offline adapter; no separate
  audition-only gate implementation. Provisional position: after channel EQ,
  before compressor/makeup. Audit all send taps, source-analysis taps and bypass
  routing before freezing this ordering. Raw REC and raw source analysis remain
  upstream and untouched. Clearly label post-gate analysis taps.
- Basic gate: bounded detector envelope, separate open/close thresholds,
  attack/open/hold/release/closed state machine and finite attenuation range.
  Hold counts consecutive time below the closing threshold; return above it
  resets hold. A new opening crossing during release smoothly reopens from
  current gain. No gain discontinuities on retrigger, bypass or updates.
- Tom mode: optional detector-only band selection (one prepared RBJ bandpass in the implemented core), fast attack,
  minimum open time, then a bounded hold and release envelope chosen for the
  drum's decay. Audio itself is not band-limited. Specify retrigger and tail
  protection explicitly; rolls and quiet hits must not be rejected by a rigid
  lockout. Detector filtering improves discrimination but cannot guarantee
  separation of overlapping instruments.
- Smart preparation: soundcheck worker uses pre-gate evidence to estimate the
  target drum band, inter-hit bleed floor, quiet-hit separation and decay times.
  Use bounded histograms/envelope summaries and robust quantiles; emit measured
  evidence, confidence, parameters and reason codes. Split calibration/evaluation
  passages. No song names, reference recordings or listener-entered per-song
  offsets in the decision rule. Insufficient separation leaves auto-gating
  bypassed with an explanation; a musician can still choose manual settings.
  Freeze prepared settings for performance initially. Continuous learning,
  FFT/ML detection, lookahead and oversampling are outside the first slice.

### Ordered implementation and acceptance

1. [x] Freeze detector/time definitions, parameter units/ranges, sample-rate
   preparation, state transitions and numerical guards in focused unit tests.
   Specify a finite range floor and a finite-duration smooth release reaching
   that floor. Precompute coefficients/thresholds off the audio thread; no
   per-sample log/exp/pow, allocations, locks, I/O or unbounded loops.
2. [x] Implement the reusable core with a cheap bypass path. Preserve detector
   history where valid across updates; audit current channel crossfades, which
   temporarily process old and new branches, so unrelated EQ edits cannot reset
   a gate and cut a transient. Reset only with defined source-epoch semantics.
   Bound NaN/Inf, denormals and state recovery. If stereo is exposed, use a linked
   detector and common gain; do not gate the two sides independently by default.
3. [x] Extend strict configuration, preparation, show persistence, wire snapshots
   and provider edits together. Existing shows load bypassed through explicit
   version/migration policy; reject incompatible writes rather than silently
   dropping fields. Expose gate mode, state, attenuation and detector freshness
   separately from compressor gain reduction. Preserve review/permission guards.
4. [ ] Add soundcheck preparation and causal block-stream evaluation. Compare
   weak/strong hits, ringing tails, cymbal/snare bleed, rolls, retriggers, silent
   inputs, varying noise and insufficient evidence. Measure missed intended hits,
   false openings and tail attenuation; retain original signal as the reference
   for preservation, not a commercial song. Do not tune by post-render scans.
5. [x] Add a labelled Gate page/mode to the existing compressor-area controls:
   column 5 threshold/attack, 6 hold/release, 7 range/hysteresis. Keep compressor
   controls intact when its mode is selected. Detector band and manual/tom/auto
   options live in the menu. Show open/hold/closing/closed and actual attenuation.
   Do not require unverified hardware Shift events to operate the gate.
6. [x] Benchmark baseline vs bypassed/basic/tom modes at 16/24/32/48 channels,
   48 kHz and supported block sizes. Include all channels triggering together,
   active tails and simultaneous configuration crossfades. Report added CPU time
   per block, deadline margin, distribution/max, and fixed state bytes/channel
   on the target Pi. Reserve a provisional added-cost budget of <=5% of the
   block deadline at 48 channels; this is an acceptance target, not a measured
   capacity claim. If exceeded, optimize or narrow enabled modes before adoption.
7. [x] Run focused DSP/contract tests during work and the complete normal
   production suite for the engine/schema integration. Keep long benchmarks and
   listening renderers opt-in with a documented command. Listening/hardware
   acceptance remains distinct; do not regenerate accepted mixes automatically.

### Current implementation and remaining acceptance

`src/gate.rs` owns the reusable prepared DSP and measured-summary preparation API.
`channel_processing::Strip` applies it after EQ/before compression, preserving
one shared gate history across configuration crossfades. Processing v5 carries gate config and
separate state/attenuation readback; engine intent v3 persists it. Legacy intent
loads bypassed and legacy writes cannot silently discard gate settings.
Desk has the six labelled Gate controls, detector frequency/Q menu fields,
reviewed edits and separate attenuation text. See [parameter and preparation
contract](../reference/CHANNEL_PROCESSING.md#gate-extension-v5).

The bounded `gate-prepare` CLI exports identified measured-summary proposals.
Desk's Manual/Tom/Automatic Import menu uses existing local draft/review gates;
Automatic shows confidence/reason and checks input and passage identities.

Raw-cue capture, calibration-only detector selection and causal held-out WAV
evaluation now supplement summary import. Cues identify isolated target/bleed
passages; arbitrary mixtures remain deliberately ambiguous. The v2 file workflow
rejects duplicate decoded PCM and changed second-read evidence. No device capture
or accepted-mix rendering occurs. See the owning processing reference.

Actual synthetic producer-to-Desk v5 acceptance now covers Manual and imported
reviewed gate edits, correlated confirmed readback, unchanged other inputs,
draft selection fences, read-only reconnect without replay, and stale refusal.
At that gate baseline, Master speed/provider acceptance remained unfinished.
The [November 6 GP-PANEL continuation](#master-gain-and-slow-strip--2026-11-06)
owns its implementation and current software evidence; physical/listening
qualification remains separate.

Final continuation production validation (Rust 1.97.1, shared build lock,
`CARGO_INCREMENTAL=0`, `--locked -j1 --all-targets`): GigPies default **503 passed,
22 ignored**; `hardware-host` **588 passed, 54 ignored**. Desk default **375 passed,
28 ignored**; `native` **378 passed, 28 ignored**. All runs finished without
failures. Focused Desk checks: controller flow **6**, Frontend output-failure
isolation **1**, gate decoding/evidence consistency **2**. The actual v5 driver
passed again against the rebuilt producer. Historical/exhaustive campaigns,
physical devices and long benchmarks were intentionally skipped; no accepted
mixes or audition renders were generated. The shared widget was unchanged in
this continuation; its prior 13-test result is retained, not presented as a new run.

Warnings-denied all-target Clippy passed for GigPies `hardware-host` and Desk
both default/native; rustfmt checks passed from each canonical checkout. Closeout
held the shared build lock, checked process executables/open file references,
and removed **233** task-run test executables plus temporary logs: **22,308,847,616
allocated bytes (20.78 GiB)** reclaimed. Applications, reusable build libraries,
original ignored media and the supplied screenshot remain. Targets retain about
4.8 GiB (GigPies) and 13 GiB (Desk) of applications/reusable libraries; free space
is about 47 GiB. No source commits,
publication, device activation, controller-memory changes or audition renders occurred.

Continuation evidence (2026-10-30): focused gate integration **8 passed**.
Causal 48 kHz stream: **5/5** intended hits preserved, worst energy change
**−0.008905 dB**, tested ringing-tail change **0 dB**, isolated bleed **−24 dB**.
Partitions 1/47/48/128/256/1024 samples produce identical output. Independent raw
cue workflow: **8/8 hits**, **0/8 false-opening passages**, worst hit energy
change **−0.213963 dB**. A single held-out bleed transient now refuses auto-gating
instead of being hidden by p95. Existing strip timing benchmark below was not
rerun: sample arithmetic is unchanged, and new capture/evaluation is control-side.

Actual v5 test: build `cargo build --locked -j1 --example spectrum-source` in
GigPies, then in Desk run `GP_GATE_SOURCE=/home/shome/p/gigpies/target/debug/examples/spectrum-source
cargo test --locked -j1 --test gp07_frontend gate_v5_actual -- --ignored --nocapture`.
Use the shared build lock and `CARGO_INCREMENTAL=0` for both commands. The test
owns a temporary private endpoint and deterministic 16-input producer, cleans it
on exit, and opens no hardware. It passed **1/1**; this is actual transport and
frontend acceptance, beyond the frozen codec fixture.

Software validation (2026-10-30): complete default and `hardware-host` normal
production suites were run (497 and 582 cases respectively, with 22/54 opt-ins
skipped). Frozen pre-gate admission-size comparisons initially failed; their
original bytes were retained and the exact added state reservation was checked
in focused reruns. Final feature-library run: **139 passed, 10 ignored**; corrected
Brain fixtures: **12 passed**; gate/CLI integration: **4 passed**. These include the
new identified-proposal unit/CLI cases added after the complete runs. No unresolved
test failure remains. Warnings-denied all-target hardware-host Clippy passed.

Desk: **371 default tests passed, 27 ignored** in the final run. The native suite
passed **373, 27 ignored** before the last generic bank-projection regression was
added; that regression passed in the final default run. The final unavailable-Master recording projection then passed all three focused
keybed cases. Both all-target Clippy configurations (default/native) passed with
warnings denied. Shared-widget
suite: **13 passed**. Gate timing/retrigger/quiet rolls, crossfade preservation,
allocation guards, v5/legacy persistence, prepared import/review, complete
controller messages, Shift loss, rate independence, gain guards, output failure/
coalescing and keybed identity/status cases are covered. Physical tests, historical
auditions, exhaustive research and unrelated opt-in campaigns were intentionally
skipped; only the gate overhead benchmark below was explicitly selected.

Reproduce normal suites with `CARGO_INCREMENTAL=0 cargo test --locked -j1
--all-targets` (add `--features hardware-host` in GigPies or `--features native`
in Desk); focused CLI: `cargo test --locked -j1 --test gate`. No accepted mixes or
controller memories were changed, and no hardware messages were sent.

Closeout removed 22,521,180,160 allocated bytes (20.97 GiB) of task-run test
executables, temporary widget output and disposable logs/fixtures. Original media,
authored source, applications and useful build libraries remain. Normal build
directories retain about 4.7 GiB (GigPies) and 13 GiB (Desk); no unrelated checkout
or running application output was cleaned. Final free space is about 47 GiB.

Validation benchmark (2026-10-30, Raspberry Pi 5 Model B Rev 1.1, Rust 1.97.1,
release profile, 48 kHz): 20 warm-up blocks and 500 measured blocks per case.
All strips trigger together, continue through tails, and retrigger. Crossfade
cases request a change every block (more often than production admission allows).
Coefficient preparation is outside timing; EQ and compressor are bypassed in
all cases to isolate gate overhead. Baseline executes the same strip layout with
the gate call disabled. Values below are microseconds; added p99 is
the difference of independently timed p99 distributions, not a per-block pairing.

| Inputs | Frames | Baseline median/p99 | Bypass Δp99 | Basic Δp99 | Tom Δp99 | XF baseline p99 | Basic XF Δp99 | Tom XF Δp99 | Tom XF max | Tom XF p99 margin |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 16 | 48 | 5.65/5.67 | 1.63 | 5.44 | 7.20 | 10.30 | 8.37 | 12.04 | 26.39 | 97.77% |
| 16 | 128 | 15.02/15.02 | 4.28 | 14.72 | 19.78 | 23.76 | 26.06 | 35.98 | 62.80 | 97.76% |
| 16 | 256 | 30.00/30.00 | 10.05 | 31.20 | 40.65 | 47.17 | 47.72 | 66.46 | 115.28 | 97.87% |
| 24 | 48 | 8.22/8.24 | 2.09 | 8.28 | 11.26 | 14.00 | 13.56 | 20.54 | 38.39 | 96.55% |
| 24 | 128 | 21.85/21.89 | 6.09 | 23.35 | 30.54 | 36.31 | 37.28 | 50.98 | 89.17 | 96.73% |
| 24 | 256 | 43.67/45.63 | 11.80 | 42.85 | 58.67 | 68.85 | 71.78 | 99.43 | 184.03 | 96.84% |
| 32 | 48 | 10.78/10.78 | 2.89 | 11.13 | 14.69 | 18.45 | 20.13 | 27.41 | 48.65 | 95.41% |
| 32 | 128 | 28.67/28.68 | 9.17 | 30.72 | 40.22 | 48.17 | 47.87 | 68.65 | 118.06 | 95.62% |
| 32 | 256 | 57.31/59.19 | 16.24 | 57.59 | 77.20 | 90.31 | 94.50 | 131.54 | 222.29 | 95.84% |
| 48 | 48 | 15.89/15.91 | 6.20 | 21.22 | 24.22 | 32.28 | 31.15 | 35.89 | 69.04 | 93.18% |
| 48 | 128 | 42.31/44.09 | 11.83 | 42.89 | 59.41 | 70.46 | 72.43 | 101.02 | 172.81 | 93.57% |
| 48 | 256 | 84.59/87.17 | 29.92 | 85.87 | 118.13 | 133.80 | 141.37 | 198.65 | 335.18 | 93.77% |

At 48 channels, worst-mode incremental p99 uses **3.59%, 3.79%, 3.72%** of the
48/128/256-frame deadlines (1.000/2.667/5.333 ms): below the provisional 5% target
in this run. Gate state is **168 bytes/channel**, complete strip **1,144 bytes**
(previous strip 672 bytes); admission reserves two strips per input. The benchmark
is deliberately opt-in and does not establish a whole-engine/PA/FX/transport
budget, sustained physical operation or clock lock. No audio was rendered.

Reproduce: `CARGO_INCREMENTAL=0 cargo test --locked -j1 --release --lib
 gate_benchmark -- --ignored --nocapture`.

## GP-PANEL — shared physical instrument panel

State: **IN PROGRESS**. Owner: GigPies integration/shared presentation package;
contributors: SHR Desk and SHR Lightdesk. Authorized 2026-10-11: implementation
in the three canonical repositories, one thread, no delegation/worktrees/clones,
no commits/publication/deployment, hardware or host service/audio changes.
Preserve the existing GP-UI-COMPARE record and both contributor-plan edits.

Outcome: compact Ratatui production consoles with reusable authored colour PUA
material tiles, ordinary readable text and actual provider state. An actual
terminal specimen must pass visual/cell acceptance before production migration.

### Three-row keybed flow plan

2026-10-30: screenshot inspected and user intent incorporated, including solo
and mute in the status row. **Shared widget and Desk integration implemented;
software/physical acceptance remains bounded below.** Contract: [three-row channel keybed](../reference/INSTRUMENT_PANEL.md#three-row-channel-keybed).
This task owns the shared widget and Desk integration plan; no Lightdesk channel
semantics are inferred. The inspection table records the pre-implementation gaps;
the current checkpoint is under Controller/keybed implementation below.

| Inspected implementation | Difference from requested flow |
|---|---|
| `shr-desk/src/midi.rs`, fixture decoder | Note-on produces numbered Select; fixture has 36 inputs and no explicit Master key. It is not a learned hardware map. |
| `shr-desk/src/frontend.rs`, navigation and action dispatch | Uses 12-input banks and input-index selection; direct `Action::Select` is unavailable in real provider mode. Master EQ is a separate action path. |
| `shr-desk/src/channel_panel.rs` | Selected-channel controls have fixed layout/hit areas; no persistent three-row keybed. Reserve space without clipping pots, EQ or review controls. |
| `shr-desk/src/modules.rs::Recording` and `frontend/recorder.rs` | Take mapping/state and counters exist, with a dedicated Recorder page. They do not supply independent per-track arm/durability states or the requested keybed indicators. |
| `shr-desk/src/frontend.rs`, mute action | Existing mute action can inform the corresponding readout after confirmation. Solo requires an owner/contract audit; selection and monitor routing are not solo evidence. |
| GigPies `src/instrument-panel/` | Shared cell rendering is the widget owner; a new screenshot asset or copied consumer implementation is unnecessary. |

Implement in dependency order:

1. [x] Define stable-ID input-or-Master selection and independent R/S/M
   projections. Audit confirmed mute and solo ownership, freshness and pending
   semantics; identify any missing solo contract before claiming support.
2. [ ] Build the pure three-row shared widget and cell hit map. Fit 24+Master,
   including three simultaneous status characters on black keys, green selection
   and monochrome fallback. Validate a specimen at the supported 160×49 geometry (45 content rows plus keybed/footer)
   and minimum width; retain full identity/bank context in existing header/footer.
3. [x] Connect Desk production selection through the common action path, keeping
   draft/review guards, cancellation and generation fences. Master opens the
   appropriate master context without granting permissions or changing audio.
   Map the existing 12-input detail views to the same selected ID; add explicit
   24-slot overview paging for larger configurations and dim unavailable slots.
4. [x] Keep status refreshed when Recorder is closed. Audit polling/subscription
   ownership, then project recording membership by stable ID and current take/
   epoch. Show preparing/finalizing, stale and faults honestly. Connect confirmed
   mute/solo without optimistic indicators or adding status-cell toggle actions.
5. [ ] Test widget geometry/hits, selected channel 15, Master, simultaneous RSM,
   selected-and-muted/soloed/recording, 16/24/32/48 input maps, bank transitions,
   reordered inputs, stale/disconnected/fault state and blocked draft navigation.
   Verify all original Channel, Master, EQ, FX, send and Recorder flows remain
   reachable. A 24-slot UI fixture does not claim 24-track recorder acceptance.
6. [ ] Inspect the real terminal layout and navigation. Keep learned physical
   note mapping/device tests as a separate authorized acceptance step; fixture
   note numbers and this screenshot are not hardware evidence.

Additional flow direction: [EQ Shift and slow master speed control](../reference/INSTRUMENT_PANEL.md#rotary-and-master-strip-flow-direction).
These bindings need verified physical input events. No compressor Shift action
is agreed; Gate is implemented in a separate labelled mode under GP-GATE.

Next acceptance: inspect the integrated terminal at 160×49 and qualify the
physical controller profile. Shared widget, stable selection, paging and status
projection are implemented; the detailed checkpoint below separates software
from physical evidence. Original screenshots remain in ignored artifacts; no
bitmap substitutes were generated.

### Concrete execution plan

1. Audit production native input/render/provider paths, fonts, action maps and
   workflow tests. Freeze the old 100-screen benchmark inputs and classify their
   synthetic data. Record every production page/modal and its authority boundary
   in `docs/reference/INSTRUMENT_PANEL.md` before migrating it.
2. Add an independently buildable shared `shr-instrument-panel` package inside
   GigPies `src/instrument-panel/`, with pinned Ratatui/Crossterm and its lockfile.
   Use GigPies' existing target directory and parent-held nonblocking build lock.
   Keep tokens, widgets, asset registry and protocol session in this one owner.
   After proof, distribute a versioned, checksum-verified Cargo directory-source
   package to both consumers (exact version, no sibling path dependency); no
   handwritten duplicated controls. Publication remains forbidden this session.
3. Author original material/LED/control tiles with editable geometric sources,
   bounded one-cell footprints and PUA-B registry away from Nerd/Powerline ranges.
   Generate colour font/protocol payloads reproducibly, retain licences and
   verify candidate text-font licences. Probe actual support; never infer it from
   TERM or a directory name. Parse bounded replies, track owned registrations,
   unregister only owned slots, restore terminal modes on errors/normal exit.
4. Build a real Ratatui specimen: metal plate, stateful raised/pressed/focused/
   disabled/latched controls, recessed display, four on/off LED colours, red
   gradient/glow, fractional VU/spectrum/fader. Synthetic motion must be labelled
   specimen data. Check clipping, minimum size, resize, focus and keyboard/mouse.
5. Launch in the selected real terminal on this Pi; inspect screenshots at
   multiple cell/font sizes. Verify tile joins, widths, gradients, bounded glow,
   probing/cleanup and direct/SSH/multiplexer behavior. Select the primary
   terminal/font contract only from this proof. No production migration before
   it passes. Missing actual display/terminal access is an explicit dependency.
6. Migrate Desk then Lightdesk through their existing semantic event/worker paths.
   Preserve all pages, exact edits/reviews, Hold/grants/caps/freshness/source loss,
   owner-confirmed vs pending state, null/disarmed output and disconnect safety.
   Missing spectrum remains unavailable. Terminal key-release limitations must
   fail closed for held talkback; never synthesize a held safety proof.
7. Focused tests while developing; complete normal shared-package and both
   default/native owning suites after production rendering changes; fmt/Clippy.
   Compare identical 100-screen page/meter/spectrum/interaction/idle workloads:
   application construction/diff/bytes and terminal presentation separately,
   including startup registration, CPU/RSS. Sink writes are application-only.
8. Retain concise screenshots, launch commands, asset notices, measurements and
   limitations; clean only task-owned disposable intermediates. Close/archive
   this card only after required software/terminal gates pass.

### Single-screen material revision — 2026-10-12 (active)

The earlier font appearance was rejected as visually flat. The operator has now
accepted the cached-material screenshots and requested removal of redundant
titles. This increment is
restricted to selected Channel; no broader migration, commits, deployment,
hardware, services or MIDI activation. Canonical checkouts only, one thread.

Inspection: the custom COLRv0 tiles and rounded knobs are real. Flat `panel()`
fills occupy most pixels, buttons use square edge strips, and three-cell lamps
remain narrow stepped octagons. Font transport success is not visual acceptance.

Additional operator direction: model the MiniLab as eight columns. Columns
1–4 pair frequency/gain (encoders 1–4 / 9–12); 5–7 pair threshold/attack,
ratio/release and knee/makeup; 8 pairs pan/level. Exactly eight contextual pads
align below. Secondary 9–16 contains less-used commands. Mute belongs in Output;
level remains the primary rotary. The owning controller document defines intended
positions and explicitly records missing learned MIDI/LED mappings.

Decision after actual comparison: retain Ratatui as the sole frontend/input/text
owner, with **Channel-only cached Kitty material placements**, explicitly enabled
by `SHR_CHANNEL_MATERIALS=rio-kitty-v1`. The proposal preceded implementation;
there is no production Notcurses migration and no shared package/version change.
Original MIT plates, wells, buttons, rounded pots and LED lenses are authored
offline in Desk. The 119-asset repertoire is 668,960 base64 bytes; assets are
uploaded on first use and reused. No per-frame asset generation, whole-screen
framebuffer or independent second terminal owner exists. Lightdesk is unchanged.

Notcurses C 3.0.11 was compiled and run as ARM64, independently in Rio 0.5.28.
The reusable C probe compares cell/half-cell gradients and the identical material
plate. In this configuration ordinary text over the pixel plane caused rectangular
holes and image retransmission: 100 unchanged renders 44.106 ms / 800 bytes;
100 text-changing renders 80.368 ms / 4,324,476 bytes. These are library render /
blocking-write totals, **not observed presentation latency**, and not equivalent
to the Desk workload. Planes/subcells improve terminal composition, not material
shading itself. C/Rust versions, licences, source examples and reproducible probe
commands are in the [existing reference](../reference/INSTRUMENT_PANEL.md#notcurses-comparison).
Rust wrapper source compatibility was inspected; its FFI was not compiled.

Polish following visual acceptance: removed the numbered section headings and
their raised title strips, the redundant signal-flow sentence, graph heading,
repeated band captions and idle “Channel control” / “Owner ready” captions.
Channel identity, parameter names/units, menu actions, values and operational
states remain. Protection/epoch/revision now occupy the former signal-flow line.
The renderer, cache strategy and action semantics are unchanged. The performance
figures below describe the accepted material build before this label cleanup;
removing titles is not a backend speed change. The same two final screenshot
paths are replaced in place after real-terminal inspection. The title-cleanup
revision again passes all 332 default / 335 native normal tests (22 opt-ins
excluded per configuration), both warnings-denied Clippy configurations,
formatting, both release builds and both proposed-index publication guards.
Removed the polish pass’s completed test executables, logs and duplicate PNGs
after the checks: 7,788,910,943 file bytes; the final two PNG paths are retained.

Final connected proof, real Rio/Xvfb on this Pi, tiny-skia backend:

- [Channel / pads 1–8](../assets/panel-desk-channel.png): owner revision 1,
  EQ enabled, band 1 +3 dB, compressor enabled with settled ratio 4:1.
- [Edited ratio / pads 9–16](../assets/panel-desk-channel-draft.png): owner revision
  3 after reviewed mute and Hold; selected local ratio 6:1 remains distinct from
  settled/target 4:1. Independent read-only GP07 readback confirms the owner ratio
  remains 4:1. Green/yellow/red/blue lenses and dark counterparts are visible.

These are actual production Desk controls connected to a private software owner,
with injected role descriptors and synthetic software input. Meter/GR values are
owner observations, not renderer inventions; the graph is calculated settings
response, not spectrum. Physical controller/display/audio qualification is not
claimed. Grants, full review paging, Hold/manual ownership, caps, stale gates and
null/disarmed behavior remain in existing owner paths. No device was opened.

Inspected ordinary text, seams, bevels, plate edges, LED states and numeric wells
at 16/20/24 px. The lower numeric bevel was moved below the digit baseline after
the small-font check. Eight aligned pads and all 24 processing field targets are
covered by normal regression tests. Actual Band menu and secondary-layer clicks,
review/apply, owner readback, plain-text fallback, provider loss and shrink/restore
were exercised. Provider loss clears materials and returns to the existing
readable unavailable page. Rio reports 44 rows after resizing to the original
1122-pixel height; restore to 1146 pixels supplies the required 45 rows. The guard
correctly removes controls/materials below minimum and restores them afterward.

Reproduce final state 1 from GigPies:

```sh
/usr/bin/python3 scripts/preview_panel_providers.py desk --channel --materials --cpu-terminal --suffix channel --keys g,e,k,i,i,3,Return,i,i,i,i,i,i,i,i,i,i,i,i,i,i,i,k,i,i,4,Return,F4,Next,Return
```

For state 2 append `,m,Return,h,Return,e` followed by 19 `,i` gestures and
`,6,Return`, and add `--click 1750,102` to select pads 9–16. These are paced
software gestures, not claims of MIDI binding. For a read-only starting preview,
omit `--keys`; add `--font-size 16` or `20` for smaller checks.

Validation: normal Desk default **332 passed**, native **335 passed**, 22 opt-ins
excluded in each; default/native warnings-denied Clippy, both release builds, formatting and Python
publication regressions (Desk 9, GigPies 6) passed. The first native run printed every passing
suite but its launcher returned 143; a clean reserved rerun exited 0. Historical,
hardware and unrelated actual-provider tests remain opt-in. Only the explicitly
requested bounded rendering experiment/measurement is run separately.

Release sink measurement (same production renderer, explicit synthetic fixture
workloads, 34 warmup + 100 measured frames; drawing/diff/encoding, no terminal):

| Workload | Material median / p95 | ANSI + material bytes / 100 warm frames |
|---|---|---:|
| Unchanged | 1.306 / 1.954 ms | 1,900 |
| Rotating ratio | 1.315 / 1.343 ms | 13,108 |
| Changing GR meter | 1.310 / 1.323 ms | 4,980 |

Stable frames send zero material bytes (19 bytes/frame are backend cursor output).
First material frame is 444,746 bytes for the control fixture, including 434,685
material bytes; the finite rotated-position cache fills during warmup. The same
layout using colour glyphs instead of bitmap materials measures 1.006–1.014 ms
median, 12,912 bytes first frame and 1,900 / 14,705 / 4,980 warm bytes respectively.
This isolates the approximately 0.3 ms material application cost; it does not
measure terminal drawing or network performance.

Actual release terminal runs (100 frames each, 160×45, Rio 0.5.28 tiny-skia):

| Workload | Observed median / p95 | TTY application median / p95 | Total bytes, including cold start + observer marker |
|---|---|---|---:|
| Unchanged + marker | 69.25 / 82.66 ms | 1.355 / 1.472 ms | 467,254 |
| Rotating ratio | 73.66 / 97.13 ms | 1.370 / 1.471 ms | 605,941 |
| Changing GR meter | 68.44 / 81.77 ms | 1.353 / 1.426 ms | 471,965 |
| Colour-font ratio comparison | 49.33 / 59.36 ms | 1.121 / 1.304 ms | 63,425 |

The observer reads the real X framebuffer and ACKs a changing marker through
terminal input; timings include readback/polling/scheduling, exclude physical
scanout, and are not a refresh-rate qualification. Even “unchanged” presentation
changes the observer row. Material startup-to-visible was 200–301 ms and first
TTY frame 452,912 bytes (including observer); comparison 191 ms / 29,633 bytes.
Rio plus child peak RSS was 62,064–64,144 KiB versus 30,480 KiB; user CPU over
100 frames was 4.55–5.00 s versus 2.49 s. Counts cover frame text/material writes; console setup/cleanup and observer
input ACKs are excluded. Local PTY bytes are not network traffic estimates. Cached materials improve appearance at measurable terminal
cost, so this remains opt-in pending visual review and actual display qualification.

Reproduce the separately labelled synthetic performance workload:

```sh
# From shr-desk, under the usual nonblocking build reservation:
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo +1.97.1 test --offline --locked --release -j1 --test frontend --no-run
# Use the executable printed by Cargo; run from GigPies:
/usr/bin/python3 scripts/measure_panel_terminal.py --channel-test ../shr-desk/target/release/deps/frontend-392bae132e49f501 --workload controls --cpu-terminal --name channel-controls
```

Repeat with `unchanged`/`meters`; `--plain` supplies the colour-font comparison.
Run the executable directly with `channel_material_performance --ignored --exact
--nocapture` for warm sink results. No fixture measurements enter production state.

Final checks: generated assets reproduce byte-for-byte; 44 local documentation
file links resolve; Python syntax, whitespace and complete proposed-index
publication guards passed in both repositories without changing their real index.
No commit, push or outgoing publication was attempted. After inspecting processes,
open files and the shared build lock, removed 71 completed test executables,
Notcurses scratch/dependencies, intermediate renders and measurement JSONs:
**7,820,830,577 file bytes** reclaimed (7,820,427,264 allocated bytes). Desk's normal
target remains 3.9 GiB with runnable debug/release binaries and reusable libraries;
existing Rio/font tools remain 22 MiB. Retained only the two final screenshot
replacements, reusable source/commands and this owning result record.

### Selected Channel vertical separation — 2026-10-20

Approved meter refinement: all four meters have 16 segments, centred titles on
row 3 (first black row), bars on rows 4–19 and readings on row 20. The master’s
original positions remain fixed. Level scales match from 0 to −60 dBFS; GR uses
0…24 dB downward. Cached LED strips show green below −12, amber below −4 and red
up to 0; dim strips describe the scale without implying a fresh observation.
Six original sprites add 1,596 base64 bytes (124 assets / 710,924 bytes total).
Assets upload once and placements are diffed; ordinary text retains the colours.
OUT/PRE-FADER now reads actual `input-NN:processed` samples after EQ/compression,
before mute/fader/pan, instead of repeating raw input or estimating output from GR.
No DSP/authority/MIDI/hardware path changed.

Both final files in `docs/assets/` are actual Rio/tiny-skia/Xvfb production captures
of deterministic software audio, not hardware measurements:
- `panel-desk-channel.png` (378,397 bytes): hot input peak −1.0 dBFS, RMS −4.0,
  actual GR 7.2 dB and processed output −8.2 dBFS; +3 dB EQ, threshold −12 dB,
  ratio 4:1. Input reaches red; output stays lower.
- `panel-desk-channel-draft.png` (383,314 bytes): same aligned geometry, muted/Hold,
  pads 9–16 and local 6:1 draft; independent owner readback remains 4:1. Pre-fader
  observation intentionally remains active under channel mute.

The operator approved the refinement and authorized source commit/push. Reproduce:

```sh
/usr/bin/python3 scripts/preview_panel_providers.py desk --channel --materials --spectrum-hot --cpu-terminal --suffix channel --keys g,e,k,i,i,3,Return,i,i,i,i,i,i,i,i,i,i,i,i,i,i,i,k,i,minus,1,2,Return,i,4,Return,F4,Next,Return
```

Append the documented mute/Hold/draft gestures and `--click 540,78` for secondary
pads. The `hot` example mode is software-only; normal `tones`/`sequence` are
unchanged. Helpers remove private runtime state; only two final PNGs are retained.
The preceding iteration’s cleanup removed 47 disposable files (3,528,265,386 bytes).

CI investigation: [GigPies run 39404031575](https://github.com/gigpies/gigpies/actions/runs/39404031575)
failed `tests/gp06.rs:231`, expecting zero clients after one nonblocking tick on a
malformed header. The harness now waits for actual EOF/reset under a one-second
wall-clock deadline, holding provider time at zero so incomplete-frame timeout
cannot mask missing length refusal. Production limits/timeouts are unchanged.
Desk’s preceding CI passed; GigPies’ hardware-host job was cancelled by fail-fast.
Publication validation passed: Desk 334 default / 337 native tests (23 opt-ins
ignored each); GigPies 445 default tests (20 opt-ins ignored) and 13 GP06 tests
with hardware-host. Both feature configurations passed all-target Clippy with
warnings denied and formatting. Python suites passed 50 GigPies / 9 Desk tests;
complete-index publication guards passed. Unrelated historical, exhaustive and
physical hardware tests stayed opt-in. No production DSP changed; hosted CI runs
the full hardware-host suite and release builds. Desk source is pushed as
`12e6a64`; the GigPies source is ready for the authorized commit/push.
Cleanup under the idle shared lock removed 138 newly created test executables
(12,780,671,848 file bytes / 12,777,668,608 allocated bytes), preserving application
binaries, shared caches, authored source and both final screenshots.

Earlier vertical-separation evidence (superseded appearance; retained measurements):

The operator rejected the cross-section EQ expansion. This revision restores
three independent full-height metal plates: EQ columns 1–4, compressor 5–7,
output 8. The EQ keeps 16 usable rows and the genuine spectrum behind the curve,
within its own 63-cell width. Three recessed meters above the compressor pots
show existing raw input peak/RMS in dBFS and actual provider gain reduction in dB.
Reduction fills downward in amber; raw meters rise in cyan/amber. These are
independently fresh observations, not estimated compressor output. Unavailable
values leave an unlit scale and dash. Output keeps 16 segments; sixteen pots,
eight aligned columns, one-row contextual pads and review/authority paths stay
in place. Cached material authoring now contains 118 assets / 709,328 base64 bytes.
No DSP/provider, MIDI mapping, hardware or service changes are involved.

Actual production terminal capture on rpi5 used Rio 0.5.28/tiny-skia under Xvfb,
with the existing deterministic software PCM provider. Inspected 16/20/24 px,
plate seams, numerical contrast, primary/secondary pads, compressor menu,
minimum-size pause and resize recovery (20 px restore requires 1608×966 outer
window dimensions). The two final files were overwritten in `docs/assets/`:

- `panel-desk-channel.png`: 383,689 bytes; applied EQ +3 dB / compressor 4:1,
  real raw peak −6.0 dBFS, RMS −9.0 dBFS and owner GR 7.9 dB.
- `panel-desk-channel-draft.png`: 386,194 bytes; reviewed mute/Hold and secondary
  pads, local ratio 6:1 while independent owner readback remains 4:1. The raw meter
  observer was stale at this capture and correctly shows unlit scales/dashes;
  the independently fresh spectrum and actual owner GR remain visible.

Focused frontend checks passed (7 / 1 ignored), including the section boundary,
GR direction and stale clearing. Complete default Desk suite passed **334 / 23
ignored** and native **337 / 23 ignored**; both all-target Clippy configurations
passed with warnings denied. Builds used Rust 1.97.1, `--locked`, `-j1`,
`CARGO_INCREMENTAL=0`, canonical targets and the shared reservation. Unchanged
provider/DSP suites and unrelated historical/exhaustive/hardware opt-ins were not
rerun. Formatting and both working-tree whitespace checks passed. This subsection
owns the replacement screenshots and measurements; October 12 appearance and
rendering costs below describe the superseded wide-EQ layout.

Release rendering evidence before the header-framing correction, with compilation
idle, uses the same
reproducible `channel_material_performance` and `measure_panel_terminal.py`
commands below. Sink runs have 34 warmup + 100 measured frames; actual Rio runs
have 100 acknowledged frames. Fixtures are explicitly synthetic benchmark input;
the final screenshots instead use the real software-audio provider path.

| Workload | Sink median / p95, µs | Warm sink bytes / 100 | TTY application median / p95, µs | TTY bytes / 100, including cold upload | X observation median / p95, ms |
|---|---:|---:|---:|---:|---:|
| Unchanged | 1,043 / 1,424 | 1,900 | 1,162 / 1,361 | 518,439 | 78.85 / 116.09 |
| Controls | 1,045 / 1,068 | 10,682 | 1,190 / 1,744 | 654,263 | 83.80 / 111.73 |
| Meters | 1,048 / 1,071 | 13,118 | 1,181 / 2,046 | 535,087 | 79.20 / 103.79 |
| Spectrum | 1,161 / 1,193 | 152,810 | 1,353 / 1,506 | 798,183 | 81.16 / 116.29 |

First TTY frames sent 497,584–501,164 bytes. Material traffic across 100 frames
was 471,319 bytes, or 603,815 with changing knob angles; assets remain cached,
not uploaded every frame. Terminal startup-to-visible was 222.41–478.94 ms and
peak RSS 66,736–72,736 KiB. X observation includes framebuffer readback/polling,
scheduling and the changing marker row; it excludes physical scanout. These
are separate application/transport/presentation observations, not a claim of
unchanged speed. Analysis and provider transport code were unchanged, so their
October 12 measurements were not rerun or relabelled as new evidence.

Closeout: after build/display processes exited, checked the shared lock and
`/proc` references, then removed 86 task-created disposable files: **8,034,784,623
file bytes / 8,033,243,136 allocated bytes**. Removed only this run's logged test
executables, duplicate captures, benchmark JSON and temporary logs. Desk's normal
build cache is 3.9 GiB; reusable Rio/font tools remain 22 MiB; free space is 53 GiB.
Source, licences, required application executables and earlier uncommitted work
were preserved. Only the two final screenshots remain. No rebuild followed
cleanup. No commit, push, deployment or hardware activation was performed.

Header framing correction: the top now has an empty border row, then the
labels/LEDs row, then an empty row before the display wells. The earlier change
put the border below the labels and was rejected. Both final captures were
replaced and inspected with the corrected arrangement. Reproduce the secondary
pad layer with `--click 540,78` (its header control is now below the border). The knobs, values and one-row pad strip
keep their coordinates; displays keep their bottom edge, with a 16-row EQ graph,
16 output segments and 13 compressor-meter segments. A focused regression checks
both empty header rows and the existing control positions. Focused tests passed;
complete normal suites passed (334 default / 337 native, 23 opt-ins ignored in
each), as did both warnings-denied all-target Clippy checks. The operator approved
the final capture on October 20 and explicitly authorized commit/push. The
approved image is embedded in `docs/reference/INSTRUMENT_PANEL.md`. Publication
guard regression suites passed (GigPies 6, Desk 9). Both default/native release
builds passed, as did the complete-index publication guards and local document
link checks. The accepted Channel state is ready for the authorized source
commits; this does not deploy binaries or qualify physical hardware.

Final cleanup after checking the shared lock and process references removed 80
task-created test executables, duplicate captures and logs: 8,024,184,915 file
bytes / 8,022,634,496 allocated bytes. Required default/native application output,
source and licences remain; Desk's shared build cache is 3.9 GiB and free space
is 53 GiB. The approved documentation image was preserved exactly. No rebuild
followed cleanup.

### Selected Channel refinement and real spectrum — 2026-10-12

This continuation supersedes the title-cleanup layout/evidence above. Canonical
GigPies and Desk only; existing edits preserved. No hardware, services, commits,
pushes or deployment. The approved cached metal renderer remains; Ratatui is the
only terminal/input owner. The first four rows and redundant inspector captions
are removed. Eight contextual pads occupy one row, with the 9–16 software layer
retained. Eight aligned physical columns preserve all sixteen pot assignments.
The enlarged graph uses 17 rows across columns 1–7; the level meter has 17 segments.
Draft values and typed entry remain in their numerical wells; full review,
confirmation, grants, caps, freshness, Hold and manual ownership retain existing
owner paths. No new MIDI binding is claimed. Arturia's manual documents the bank
selection and subsequent pad functions, but no separate switch message; Desk's
current decoder has no such mapping. Device/preset verification remains open in
`shr-desk/docs/CONTROLLER.md`, outside the operating screen.

GigPies now supplies optional `GP-SPECTRUM:1` from actual logical raw source PCM,
before processing/fader/mute. A fixed two-window SPSC handoff feeds the existing
FFT on a worker. It computes 4096-point periodic-Hann peak spectra every 4800
source frames, max-aggregated into 64 logarithmic bins, 20 Hz–20 kHz. Units are
millidBFS peak amplitude (bin-centred full-scale sine = 0 dBFS), not acoustic SPL.
Callback copies are bounded and allocation/lock/I/O-free. Obsolete work is dropped;
source age, frames, input/map/epoch and query identity fence observations. Desk
polls at 10 Hz, expires at 350 ms including round-trip age, and renders coloured
levels behind the EQ curve with 500 ms peak hold / 24 dB/s decay. Missing or stale
data clears the plot without fake motion or a banner. Unsupported local providers
remain usable via independent control/meter connections. The remote bridge has
no spectrum capability yet and honestly omits it. One active selected analysis
tap per provider is the explicit resource bound, not a product channel cap.

The reusable `examples/spectrum-source.rs` supplies deterministic software PCM
through the actual provider capture path; synthetic generation exists only in
this example and tests/benchmarks. The owning contract and repeatable commands
are in `docs/reference/INSTRUMENT_PANEL.md#selected-channel-spectrum-contract`.

Actual software proof: the real provider transport verified 93.75, 996.09375
and 9996.09375 Hz bin-centred tones at −6.021 dBFS, a change to −18.062 dBFS,
silence and a new observer connection. The actual source → FFT/provider → Desk
consumer test received **59 distinct windows in six seconds**, verified peak
hold/decay, source loss and a fresh provider epoch. The callback heap guard passed.
These are deterministic software-audio results, not a live hardware measurement.

The final two files were overwritten after inspecting the actual production Desk
in ARM64 Rio 0.5.28/tiny-skia under owned Xvfb on this Pi:

- `docs/assets/panel-desk-channel.png`: owner revision 1, EQ band 1 +3 dB,
  compressor 4:1, fresh raw 996.09375 Hz spectrum/peak and pads 1–8.
- `docs/assets/panel-desk-channel-draft.png`: owner revision 3 after reviewed mute
  and Hold, local selected ratio 6:1 and pads 9–16. Independent readback still
  reports settled 4:1. Raw spectrum remains present through mute, as specified.

Inspected 16/20/24 px text/values, plate seams, pot geometry, graph contrast,
Band and secondary menus, full review/apply, minimum-size pause, shrink/restore,
older-provider fallback and provider loss. No browser, generated artwork or
buffer replay serves as appearance evidence. The preview helper now sizes Xvfb
for the largest requested resize and waits for actual preview readiness before
an owned-provider disconnect; its former fixed delay raced startup under load.

Reproduce final state 1 with the earlier applied-state key sequence and add
`--spectrum`. State 2 appends the earlier mute/Hold/draft gestures and uses
`--click 540,54` for the relocated layer control. These are software input events,
not learned MIDI bindings. The retained fixtures are explicitly software-only.

Validation on the frozen production sources used Rust 1.97.1, `--locked`,
`CARGO_INCREMENTAL=0`, `-j1`, canonical targets and the shared build reservation.
Complete normal `cargo test --all-targets` suites passed: GigPies **445 passed /
20 ignored**, `--features hardware-host` **529 / 46**; Desk default **334 / 23**,
`--features native` **337 / 23**. All-target Clippy with `-D warnings` passed both
owning configurations. The hardware-host feature suite used software fixtures;
no physical device was activated. Focused allocation, contract, layout and
recovery regressions passed, as did the explicit actual-provider spectrum opt-in.
Unrelated historical, exhaustive, audition, physical-device and GPU opt-ins stayed
skipped. Formatting and both working-tree whitespace checks passed.

After the disconnect, native validation found one truncated cached test ELF
(`actions`); it was rebuilt and the complete native suite passed. No production
source change was required. Earlier complete-suite totals above were retained
from the completed pre-disconnect runs; native was rerun because its completion
was not preserved.

Release cost evidence on rpi5, measured after compilation was idle:

- FFT plus 64-band aggregation: **283 µs median / 285 µs p95**, 100 warmup +
  1,000 measured windows. Selected callback copy/handoff: **177 µs total for
  1,000 × 48-frame blocks** (this measures the bounded offer, not FFT work).
- Actual local provider transport: nine queries, **748 µs median / 1,097 µs max**
  round trip, **5,548 framed reply bytes** total, including initial unavailable
  replies. This small sample includes provider scheduling and JSON; it is not
  network or physical capture latency.

The explicit synthetic rendering benchmark uses the production renderer, 160×45
cells, cached materials, 34 warmup + 100 measured sink frames. Values below are
observations, not an assertion of unchanged speed or statistical equivalence.

| Workload | Sink render median / p95, µs | Warm sink bytes / 100 frames | Actual TTY app median / p95, µs | TTY bytes / 100 frames, including cold upload |
|---|---:|---:|---:|---:|
| Unchanged | 1,363 / 1,404 | 1,900 | 1,134 / 1,210 | 440,167 |
| Changing controls | 1,033 / 1,072 | 10,682 | 1,150 / 1,258 | 575,991 |
| Moving meters | 1,031 / 1,068 | 6,022 | 1,134 / 1,206 | 447,219 |
| Moving spectrum | 1,153 / 1,193 | 172,783 | 1,342 / 1,436 | 718,329 |

Actual Rio 0.5.28/tiny-skia presentation, 100 acknowledged frames per workload:

| Workload | X framebuffer observation median / p95, ms | Startup to first visible, ms | Peak RSS, KiB |
|---|---:|---:|---:|
| Unchanged | 78.03 / 103.28 | 464.07 | 75,872 |
| Changing controls | 87.68 / 113.73 | 236.08 | 82,512 |
| Moving meters | 74.42 / 96.68 | 213.58 | 74,816 |
| Moving spectrum | 79.85 / 102.69 | 223.07 | 75,840 |

Presentation includes X framebuffer readback, polling and scheduling, excludes
physical scanout, and adds one changing marker row even for the unchanged case.
TTY first frames were 419,315–423,751 bytes; cached material traffic over all 100
frames was 399,679 bytes except controls (532,175 bytes as new knob angles entered
the cache). No full-screen bitmap or static-material upload occurs every frame.
The sink and real-terminal measurements are separate; cold uploads and marker
traffic must not be compared directly with warmed sink deltas.

Reproduce cost evidence under the shared reservation with `CARGO_INCREMENTAL=0`
and `CARGO_BUILD_JOBS=1`: in GigPies,
`cargo +1.97.1 test --locked --release -j1 --test spectrum -- --ignored --nocapture`
and the same command without `--ignored` for actual provider transport. In Desk,
`cargo +1.97.1 test --locked --release -j1 --test frontend channel_material_performance -- --ignored --exact --nocapture`.
The resulting executable was `shr-desk/target/release/deps/frontend-392bae132e49f501`.
From GigPies, run `scripts/measure_panel_terminal.py --channel-test EXECUTABLE
--workload WORKLOAD --cpu-terminal --name channel-WORKLOAD` using `/usr/bin/python3`
for each of `unchanged`, `controls`, `meters`, `spectrum`. Synthetic benchmark
fixtures are explicitly labelled and are not the final appearance proof.

Closeout: build/display/source processes were idle and the shared lock plus
`/proc` references were checked before cleanup. Removed **231 task-created
files: 19,385,921,041 file bytes / 19,381,252,096 allocated bytes**. These were
Cargo-identified test executables produced after this task checkpoint, redundant
captures, benchmark JSON and temporary logs; source, required application/source
executables, licences and unrelated edits were preserved. Remaining normal build
caches are 4.6 GiB (GigPies) and 3.9 GiB (Desk); the reusable local Rio/font preview
tools occupy 22 MiB. Free space is 53 GiB. No rebuild was performed after cleanup.

The only final screenshots are `docs/assets/panel-desk-channel.png` (315,652 bytes)
and `docs/assets/panel-desk-channel-draft.png` (320,079 bytes), with reproducible
source/commands and this owning record retained. The requested Channel refinement
and local selected-spectrum path are implemented and software-validated. Physical
MiniLab bank messages, acoustic calibration and remote spectrum transport are not
claimed; no physical audio/MIDI, running services, commits, pushes or deployment
were performed. Work did not expand to other screens.

### Channel-screen correction — 2026-10-12

The operator rejected the initial screenshots. They establish terminal/font
transport only, not visual acceptance. Work now concentrates on one production
Desk selected-channel page before any further Lightdesk migration. No inert
feature placeholders: controls shown must have an existing complete owner path.

1. Use retained selected-channel references and actual GP07/GP03 capabilities;
   compose EQ, compressor, sends and output strip from typed owner snapshots.
   Preserve all modal/review precedence and scope/freshness/input fences.
2. Author reusable rounded metallic controls, substantial rotary encoders and
   fader parts in the shared glyph source. Keep ordinary labels and numbers;
   show independent owner, draft, selected and pending states. No screen bitmap.
3. Implement the Channel page and keyboard/mouse targeting through existing
   semantic actions. Omit unimplemented source gain/HPF/gate/polarity rather
   than advertise inert controls. Existing implemented controls retain stale
   and read-only states when their owner/authority is lost.
4. Capture and inspect the real Rio page with the actual private software owner,
   including processing, edits and fallback. Iterate appearance before claiming
   acceptance. Keep concise actual PNGs and one reproducible preview command.
5. Run focused page/input/state tests, normal owning suites and shared component
   checks; document this bounded screen and remaining overall GP-PANEL work.

### Operator feedback — keyboard-free channel panel

The operator accepts the rotary appearance, but rejects shortcut clutter and
cannot distinguish the current tiny status dots as physical LEDs. Next increment:
1. Remove shortcut prefixes, field numbers and keyboard instructions from the
   selected-channel operating surface; preserve semantic bindings internally.
2. Use the authored illuminated/dark lens assets beside explicit owner-state
   captions, distinguishing confirmed processing/bypass from a detached draft.
3. Primary input confirmed by the operator: MIDI. Remove the pointer-first
   assumption. Keep the existing controller plan as mapping owner; do not label
   unimplemented physical assignments or claim keyboard-free hardware operation.
   Review/confirmation and exact edits remain subject to existing safety gates.
4. Validate the changed production rendering and controls, inspect a replacement
   actual-terminal capture, replace superseded evidence and clean task outputs.

Presentation correction implemented in shared 0.3.1/font 0.3: 443 tiles,
three-cell lenses, plain channel labels, no keyboard legend. Replaced the same
two actual-terminal captures; fallback inspected and discarded. Shared 12 and
Desk default 331 normal tests passed, with shared/default Clippy passing. The
updated owner-vs-draft lamp regression passed in the default suite. Candidate
publication guards and vendor checksums passed. The operator then requested no
further screen corrections and moved to cleanup; remaining native/Lightdesk
validation was stopped, not claimed passed for this revision. The earlier
0.2.1 full-suite result remains historical evidence only. No new MIDI port or
physical mapping was implemented or activated. The operator also rejected the
local-draft explanatory row; leave that visual change for later as requested.
Removed 2,215,489,088 bytes of completed validation executables plus 3,001,697,304
bytes from 19 completed binaries in the interrupted native build. The
validation process and temporary log are gone; the shared build lock is free.

### Publication validation — 2026-10-12

The user subsequently authorized committing, pushing and synchronizing all thirteen
canonical repositories on both Pis, excluding rpi-remote. The previously interrupted
0.3.1 validation is now complete: shared 12, Desk default 331/native 334 and
Lightdesk default 135/native 136 normal tests passed. Formatting, warning-denied
all-target Clippy and release builds passed for all five configurations. Commands:
`cargo fmt --all --check`, `cargo test --offline --locked --all-targets -j1`,
`cargo clippy --offline --locked --all-targets -j1 -- -D warnings`, and
`cargo build --offline --locked --release -j1`; consumer native runs add
`--features native`. Shared commands use `--manifest-path src/instrument-panel/Cargo.toml`
and the normal GigPies target directory. All ran on Pi5 under the shared build
reservation with Rust 1.97.1 and CARGO_INCREMENTAL=0.

Publication-policy tests passed (6/9/9), complete-index guards passed, and both
vendored packages match the shared source and their checksum manifests. The
verbatim third-party DejaVu notice retains its upstream whitespace; authored
files pass whitespace checks. Historical/exhaustive, provider-executable opt-ins,
physical hardware and software-GPU opt-ins were skipped. Existing render evidence
was retained without generating another set. Removed 10,234,398,024 bytes of
completed test executables and the temporary validation log. These results
supersede the interrupted gate status above; remaining GP-PANEL acceptance stays
open. Private synchronization task 0023 records exact final source identities.

### Channel validation and closeout — 2026-10-12

Implemented the dedicated production channel page and shared 0.2.1 controls.
Actual Rio captures: [settled channel](../assets/panel-desk-channel.png) and
[mouse-focused local draft](../assets/panel-desk-channel-draft.png). Independent
GP07 readback confirmed EQ +3 dB and compression 4:1, revision 1, ready/settled.
Input telemetry in the capture is explicitly stale; no live input claim.
Fallback, disconnected state and minimum-size pause were visually inspected.

Normal all-target suites passed: shared 11; Desk default 331/native 334
(21 opt-in tests skipped in each); Lightdesk default 135/native 136
(5/6 opt-in tests skipped). Clippy with warnings denied passed all five builds.
Publication-policy Python tests passed (GigPies 6, each consumer 9).
Historical/exhaustive/hardware suites were intentionally not run.
The cleaned presentation helper completed its 100-frame old-scene replay:
45.79 ms median, 63.87 ms p95 observation, 344.80 ms startup, 119616 KiB RSS.
This includes X readback/polling and is not a benchmark of the new Channel page
or physical display latency. The broader performance/SSH/physical gates remain open.

Cleanup removed 10,231,885,256 bytes of completed test executables and
11,273,163 bytes across 305 disposable files and 51 scratch directories.
Final sources, two captures, font and Rio remain; task scratch, intermediate
captures, JSON dumps, logs and completed test executables are removed. Preview
and measurement helpers now use automatically removed temporary directories.
This bounded channel implementation is complete; operator visual acceptance and
the remaining shared migration are still open. Do not archive GP-PANEL as done.

### Acceptance and current evidence

- [x] Live status/disk/process audit: rpi5, 32 GiB free; targets GigPies 4.2 GiB,
  Desk 2.4 GiB, Lightdesk 1.4 GiB. No active Cargo found at initial inspection.
- [x] Confirmed drift: Desk `native::Gpu::render` calls `raster::rgba`; Lightdesk
  `native::upload` calls `render::ppm` then expands RGB to RGBA. Both upload whole
  1920x1080 textures. Their blueprints explicitly chose winit/wgpu TUI appearance.
- [x] Upstream example inspected: `ratatui/Cargo.toml` depends on Crossterm/libc,
  not Ratatui; `main.rs` prints cooked-mode lines after probing. Its comment that
  flush implies an atomic PTY message must not become our correctness assumption.
- [x] Shared Ratatui package, 77 authored tiles, reproducible COLRv0 font,
  stateful specimen, one-cell registry and source licence notices implemented.
- [x] Actual Rio/Pi specimen captured; 16/20/24 px joins corrected with exact
  font metrics; fallback, keyboard/mouse, resize and minimum-size checked; superseded captures removed.
- [x] Direct Glyph Protocol registration/normal cleanup exercised: 77 slots
  empty after exit. Geometry fails square-fit inspection; remains experimental.
- [x] Initial production adapters use existing owner workers/actions, no app
  pixel framebuffer. Real offline/null-owner screenshots retain read-only,
  freshness, unavailable measurements and physical UNKNOWN states.
- [ ] Full page-by-page physical-control redesign and all operator-path visual
  acceptance. Current production pages retain their source text layouts inside
  shared plates; this is not the completed instrument-panel migration.
- [x] Final normal suites, rustfmt/Clippy and candidate publication checks pass.
  Shared 11; Desk 329 default/332 native; Lightdesk 135 default/136 native.
  Six matched 100-frame application and actual-terminal workloads recorded;
  terminal timings include observation overhead and exclude physical scanout.
- [ ] SSH path qualification, delayed protocol/reconnect cleanup, complete
  interaction workload/presentation comparison with previous native renderer.
- [x] Task-owned test executables, duplicate generated images/data and temporary
  font/Python caches cleaned; runnable binaries, source and unique evidence kept.
- [ ] Closure/archive after remaining acceptance, not before.

### Desk presentation rate — 2026-10-20

User requested CPU-first graphical knob refresh at no more than 10 Hz. Desk's
terminal loop previously rendered after every input event as well as each 40 ms
idle poll. The consumer now gates scene construction, Ratatui and material
submission at 100 ms after the preceding submission; ordinary input bursts cannot
bypass it and missed frames are not replayed. The fullscreen EQ/RTA interaction
exception is documented below. Provider/input pumping retains its 40 ms
maximum idle wait independently. Resize invalidates mouse targets until redraw;
review presentation remains tied to actual submission. Ten Hz is the selected
compromise; five Hz would add up to 200 ms visual feedback delay.

The material audit found upload-once assets and placement diffs already present.
A focused regression checks that rotating one knob replaces only its placement,
with no stationary plate/neighbor replacement and no unchanged-frame bytes.
Validation: Desk `cargo +1.97.1 test --locked -j1 --all-targets` passed
336 tests across 36 binaries, zero failures, 23 opt-ins ignored, including the
new deadline and placement regressions. `cargo +1.97.1 fmt --check` and
`cargo +1.97.1 clippy --locked -j1 --all-targets -- -D warnings` passed.
Build/test commands used the parent build lock and `CARGO_INCREMENTAL=0`.
Hardware, actual-provider, historical/benchmark opt-ins and the unchanged native
window feature were intentionally not exercised. After checking idle resources,
task test executables were removed (2,246,008,832 allocated bytes recovered);
normal build dependencies and runnable application remain, with target at 3.9 GiB.
No physical devices, deployment or whole-host CPU qualification is included.

### Fullscreen EQ/RTA — 2026-10-20

Authorized scope: canonical GigPies and SHR Desk, single-threaded; preserve other
edits. Source commits and pushes were subsequently authorized on 2026-10-20,
including all pending project changes. Hardware and playback remain excluded;
the separately authorized journal service change is documented in Node lab.

- [x] Extend GP-SPECTRUM with explicit v2 native FFT bins and topology-derived
  source inventory; preserve v1 wire shape and labelled consumer fallback.
- [x] Analyze raw inputs, main L/R before PA and configured post-patch playback
  slots through the actual bounded source handoff. Keep 4096-point Hann/10 Hz,
  source/map/epoch/frame/age fences, and clear queued observations on selection
  changes and recovered epochs. FFT/serialization remain outside callbacks.
- [x] Route read-only spectrum requests through the authenticated production
  remote host and the Desk observer, independently of control authority.
- [x] Fullscreen Channel EQ/RTA with F11 entry/exit, width minus two graph columns,
  fixed dBFS gradient, separate internal EQ gain scale, discrete coloured points
  with nominal Q-width brackets and existing detached edit/review/cancel paths.
  Colour transitions span the fixed range: green −72, yellow −54, orange −36,
  red −18 dBFS (dark green −96, brighter red 0); no normalization.
  One compact contextual label is the only text; no combined EQ curve or text
  panels. Editing replaces the source/target label with selected frequency/gain/Q. Fullscreen
  `[`/`]` select analysis independently; ordinary pan and PA Master EQ keys remain.
- [x] Focused native-bin mapping, freshness, source reset, resizing and actual
  local/remote provider-to-Desk acceptance. Retained visual evidence is the
  actual Ratatui buffer raster in `shr-desk/artifacts/screens/fullscreen-rta.png`,
  with a local draft over actual deterministic broadband provider analysis.
  This replaces the rejected curve-dominant design and its excessive text.
- [x] Final complete production suites, static checks and task artifact cleanup.

The corrected-view local source/FFT/provider/Desk test observed 59 unique windows in six seconds
and passed levels, silence, hold/decay, loss and new-epoch recovery. The actual
mutual-TLS HostAuthority → Desk consumer passed the 16-input/18-output reference
topology (36 source choices), raw input, main left and configured analog output 1:
−6.021 dBFS raw, −15.031 dBFS after the default fader/pan, and 11 unique windows
per one-second sampling interval (including interval boundaries). The connected
Frontend attached without a writer grant, loaded GP07, rendered fullscreen, and
changed to silent input 2 with cleared peaks while retaining EQ target input 1.
The test leaves authority revision zero.
The fullscreen semantic-edit test preserves the actual target, retains the draft
through an analysis-source change, clears the observation and cancels the draft.
The inspected render is software evidence, not a physical terminal/HDMI or acoustic
qualification. One shared selected analysis tap per provider remains the bounded
resource; competing source selections can make observations unavailable.

Final Rust 1.97.1 normal suites (`CARGO_INCREMENTAL=0`, `--locked -j1`,
`-- --test-threads=1`): GigPies default **446 passed / 20 ignored** and
`hardware-host` **530 passed / 47 ignored**; Desk corrected-view default **341 passed / 24 ignored**
and `native` **344 passed / 24 ignored**. Final runs had zero failures. The unchanged
held-worker timing test failed once under the native run (33 ms send age), then
passed both its focused rerun and the complete native rerun. Both repositories
passed rustfmt and all-target Clippy with warnings denied in the initial pass;
the correction reran native all-target Clippy and the software-source example check.
Focused spectrum, mapping, freshness and rendering regressions passed; the
explicit actual local and authenticated remote acceptance tests above also passed.
Historical/exhaustive benchmarks, unrelated opt-in evidence renderers and physical
tests remained skipped. Native tests ran without a display, with software Vulkan;
no hardware activation, playback, service changes or acoustic tests occurred.

Post-reboot verification on 2026-10-20 covered the final colour-stop adjustment,
whose subsequent native run had been interrupted. Both repositories passed
`cargo +1.97.1 fmt --all --check`, `test --locked --all-targets -j1 --
--test-threads=1`, `clippy --locked --all-targets -j1 -- -D warnings` and
`build --locked --release -j1`, repeating the latter three with `--features
hardware-host` (GigPies) or `--features native` (Desk). All four normal test counts
above were reproduced with zero failures; Python unittest discovery passed
50 GigPies and 9 Desk tests. Builds used `CARGO_INCREMENTAL=0` and the shared lock;
native used software Vulkan without display variables. Previously passed explicit
provider acceptance was not rerun; unrelated opt-ins and physical tests stayed
excluded. Validation is complete; there is no stalled build or remaining software
check from the interrupted run.

After processes and the shared lock were idle, recovery cleanup removed 171
regenerated test executables, reclaiming 15,949,643,776 allocated bytes (14.85 GiB).
Application builds, shared dependencies, the existing preview and all source edits
were retained. Normal targets remain 4.6 GiB (GigPies) and 7.1 GiB (Desk, including
existing native dependencies and pre-existing cached outputs); free space is
50 GiB. No rebuild followed this cleanup. The separately authorized host logging
change is documented in [Node lab](NODE_LAB.md#pi-5-reboot-diagnostics).

After checking Git state, the shared build lock and process/open-file references,
the initial cleanup removed 234 task-created files (18.19 GiB); correction cleanup
removed another 85 test executables, proof-build aliases and temporary files,
reclaiming **8,288,608,256 allocated bytes (7.72 GiB)**. The single retained
visual is the **16,973-byte** fullscreen PNG; its reproducible acceptance/export
path is below. Shared normal build dependencies, application binaries and debug
artifacts remain (GigPies **4.5 GiB**, Desk **3.9 GiB**); free space is **53 GiB**.
No rebuild followed that initial cleanup. Source edits were preserved for the
subsequently authorized publication.

Owning usage/contract: [selected-channel spectrum](../reference/INSTRUMENT_PANEL.md#selected-channel-spectrum-contract)
and [Desk fullscreen usage](../../../shr-desk/docs/reference/TERMINAL.md#fullscreen-eq--rta).
Repeat actual local acceptance with `GP_SPECTRUM_SOURCE` set to the canonical
`target/debug/examples/spectrum-source` and Desk's ignored
`actual_provider_spectrum_recovery` test; optional `GP_SPECTRUM_RENDER` exports
its actual 160×45 cell buffer for visual review. With export enabled the software
source uses `rta-sequence`: the existing tone/level/silence sequence followed by
bounded deterministic multitone plus noise input, analyzed by the actual FFT.
No spectrum values are fabricated in Desk. Build Desk's `tests/spectrum`
executable, set `GP_DESK_SPECTRUM_TEST` to that exact executable, and run GigPies'
`--features hardware-host --lib remote::tests::actual_remote_spectrum_desk -- --ignored --nocapture`
for authenticated software-only acceptance. Credentials and subprocess endpoints
are private temporary test resources and are removed by the harness.

### Controller/keybed implementation — 2026-10-30

#### Master gain and slow strip — 2026-11-06

Production software implementation now adds explicitly advertised GP-Master:1,
stable Main gain, strict milli-dB bounds (−60..+6 dB), existing FOH/revision/epoch/
source admission, source-observed current/target coefficients and 240-frame
smoothing. The sole source graph applies gain once after coherent dry/local or
remote wet Main summing and before PA/talkback. Independent monitor returns,
FX excitation, raw recording and input analysis retain their taps. Core intent
v4 saves only configuration; v1–3 restore unity Master and v4 requires Master.
Restores remain disarmed, with no saved authority, movement or held controls.

Desk supports explicit Master selection, semantic menu, guarded level, detached
exact entry/review and production rendering. The existing speed model now uses
real elapsed time through one outstanding provider operation, a separate queued
motion cancellation token, correlated completion and fresh settled readback.
Center/loss discards unsent motion; uncertain submitted work is not replayed.
Concurrent revisions and other operator actions stop the strip. Input/application
stalls also discard the sub-milli-dB coalescing remainder, with a focused Frontend
regression for an application stall despite fresh input. Master pan/mute,
physical MIDI acquisition, return-to-center verification and feel remain open.
[Producer/tap contract](../reference/INSTRUMENT_PANEL.md#master-gain-software-contract)
and [Desk workflow](../../../shr-desk/docs/CONTROLLER.md#master-selection-and-editing)
own current usage; this card remains open for the rest of GP-PANEL.

Final software validation used the uncommitted implementation on GigPies base
`c9f1d8d9465c556afd1594b066cad8f7bc2507b2` and Desk base
`47ba05b726e6d72a6790b6277bcd16d666f99bc4`, on rpi5. Prior acceptance records
below are baseline evidence only. All nine library/header/owner-manifest hashes
in `user/node-lab/gp-h1-modules.json` were reverified before the final episodes;
no sibling owner was rebuilt.

| Normal all-target suite (serial) | Passed | Ignored opt-ins | Targets |
| --- | ---: | ---: | ---: |
| GigPies default | 561 | 22 | 77 |
| GigPies hardware-host | 696 | 89 | 80 |
| Desk default | 395 | 31 | 41 |
| Desk native | 398 | 31 | 41 |

All four warnings-denied all-target Clippy gates and all four required release
builds passed. Both `cargo fmt --all -- --check` gates passed. Complete-index
publication checks passed (807 GigPies / 574 Desk entries), with zero outgoing
commits; changed-worktree policy, relative documentation links and publication
regressions (6 / 9 tests) also passed. Native validation used inspected runtime
prerequisites, unset DISPLAY/WAYLAND_DISPLAY/VK_ADD_DRIVER_FILES and the CPU
`/usr/share/vulkan/icd.d/lvp_icd.json` for both `VK_DRIVER_FILES` and
`VK_ICD_FILENAMES`. Missing X11
pkg-config development metadata did not prevent the prescribed native gates.

Four actual-provider opt-ins passed separately on the final driver:

- Local Frontend: semantic selection, exact review/entry, real bitmap rendering,
  speed/center, draft cancellation, active controller loss, explicit regrant and
  read-only reconnect without replay; **192 independent Main/monitor sample
  comparisons**, wet energy **0.08372954771698594**, unchanged raw capture.
- Authenticated loopback: the same Frontend workflow with four actual FX owners;
  **5702 FX blocks**, **17568 Main sample comparisons**, nonzero wet contribution
  on all buses, zero native input/output drops and no first-source gap.
- Disarmed autonomous authenticated loopback: **1089024 silent output samples**,
  unchanged core/composed intent except the explicitly reviewed Master setting,
  no rearm or replay, zero native input/output drops and no first-source gap.
- Autonomous actual PA/FX/REC source: **32 quanta / 1536 frames**, independent
  scalar dry-plus-wet Master relation, unchanged sends/monitor/talkback paths,
  PA reference comparison, no source allocation/destruction, and **eight 24-bit
  raw WAV tracks** exactly matching input, zero dropped recording frames.

Acceptance executables retained under the normal release target directories:
GigPies `deps/gigpies-0fe0a176ba2ad145`, SHA-256
`dedb646e0c14aa6e8d123cf957e551be5101146d411f0672f5680f542b1c1567`;
Desk `deps/fx_aux_actual-dfadab3e07e1c7f8`, SHA-256
`12ffa9e3b08ab359d63e0d537a9a055f8b9c9345456ad9c77e894188e39f6ea1`.

Unchanged historical/exhaustive matrices, audition generation, long benchmark
and hardware/device campaigns were intentionally skipped. No physical activation,
audible playback, MIDI device, controller memory write, two-node traffic, T3,
backend restart, host tuning, commit or push occurred. These software results do
not qualify physical safety, controller return-to-center behavior or feel.

Failure evidence (not counted as acceptance): an initial parallel Desk run
missed an unchanged held-talkback deadline (31 ms versus 30 ms); serial validation
is used below. The first hardware-host library run had 232 passes, three fixture
mismatches and 43 ignored opt-ins. Existing producer generators refreshed eight
atomic/held-proof snapshots with unity Master and the current normalized strip
memory estimate; Desk received identical bytes (67 atomic / 30 held-proof JSON
files verified). No authority or maintenance outcome changed. The first final Desk run also
exposed five stale hashes in its separate atomic PROVENANCE record; these were
refreshed from the verified producer bytes, retaining the original corpus lineage.

Early actual-provider attempts exposed test-driver sequencing errors: comparing
an unsettled ramp, requesting a new edit while cancellation remained UNKNOWN,
reusing a retired writer identity, and confirming immediately after a costly
bitmap render with an expiring observation. The driver now waits for observed
settlement, uses production read-only Reconnect with its fresh writer, observes
lease expiry without replay, explicitly regrants, and consumes fresh observations
before independent input/confirmation events. A 244 ms-old observation correctly
refused motion 20 ms later; this guard remains unchanged. No queue or deadline
was relaxed. An initial idle-timeout hypothesis was disproved by writer-ownership
tracing. Debug loopback attempts reported source-egress gaps. The reused baseline
twenty-edit FX route driver also refused stale generation/review before Master;
that separate campaign is unchanged. Dedicated Master episodes configure actual
FX owners through validated producer operations and exercise only the requested
Master workflow. Final optimized acceptance results below supersede those failed
attempts, without qualifying hardware timing or feel.

The source audit also found that adding wet Main to the direct-output path needed
an explicit disarm closure. Direct Main/listening and the final mapped output now
remain zero at a closed safety boundary, including nonzero wet/native tails;
Master edits never change that safety state. Raw capture remains ahead of it.

Closeout retained source, normal producer/consumer fixtures, owning documentation,
production release binaries and the two acceptance executables above. After
checking processes, the nonblocking shared lock, open/mapped references and Git
status, cleanup removed 220 task-created disposable debug test executables plus
24 task logs/inventories: **22339768320 allocated bytes reclaimed** (about
20.8 GiB). Free space is 39 GiB. The normal GigPies/Desk targets retain about
11/14 GiB of pre-existing or useful shared compilation caches and required
executables; no blanket target removal or rebuild for cleanup verification was
performed. Temporary synthetic acceptance recordings were removed by their test
owners. Both canonical checkouts remain on main, with uncommitted changes only.

Normal gates in each repository used these commands under the shared lock and
environment below, repeated with `--features hardware-host` (GigPies) or
`--features native` (Desk) before the test/compiler argument separator:

```sh
cargo +1.97.1 test --locked -j1 --all-targets -- --test-threads=1
cargo +1.97.1 clippy --locked -j1 --all-targets -- -D warnings
cargo +1.97.1 build --locked -j1 --release
cargo +1.97.1 fmt --all -- --check
```

Reproduction uses Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0`, normal target
directories and `flock -xn /home/shome/p/.gigpies-build.lock`. Build Desk's
`--release --test fx_aux_actual --no-run` and provide that executable as
`GP_FX_DESK_TEST_EXE`, with the approved `GP_FX_LIBRARY`. Run each GigPies
`--release --features hardware-host --lib` opt-in separately with `-- --ignored --exact`:

- `local_audio::fx_aux::frontend_tests::actual_frontend_master_local_samples`
- `remote::tests::fx_aux_tls_tests::actual_master_tls_frontend_and_samples`
- `remote::tests::fx_aux_tls_tests::actual_master_tls_frontend_disarmed_silence`
- `local_audio::source::tests::actual_master_pa_fx_raw_rec_without_controller_pump`
  (also set `GP05_MANIFEST` to the approved manifest and `GP_PA_CONFIG` to the
  approved PA stereo configuration).


Continuation: `FeedbackWorker` now owns the explicit injected output on an
independent thread. Frontend attaches/detaches it and publishes confirmed action
projections in the normal pump, repainting across generations. Latest-state
coalescing and bounded batches remain; blocked sends cannot block input or
nonblocking detachment. Endpoint implementations must bound their own send time.
No OS MIDI port or device memory was opened/changed. The pure speed model now
integrates prior velocity through centre transitions, retains elapsed motion,
and drops stale-input/application pending motion. Dead zone/max speed remain
8% / ±0.5 dB/s. Dedicated provider Master target, coherent post-FX/pre-PA gain,
persistence and speed-to-provider integration **were unfinished at this baseline**;
the November 6 implementation and fresh validation above supersede that gap.
Selected-Master pan/mute remain explicitly unavailable, with no input fallback.


Shared package 0.3.2 adds a three-row 24-slot-plus-Master character keybed. Desk
consumes its checksum-verified package; Lightdesk remains unchanged at 0.3.1.
Desk maps displayed slots to provider input IDs, preserves draft/review fences,
projects fresh mute/recording evidence and shows unknown solo (no provider solo
contract). PageUp/PageDown changes the display bank independently of 12-input
detail views. The standard terminal reserves four rows for keybed plus footer
and requires 160×49 cells. Original ignored screenshot media is preserved.

Desk `controller_flow` supplies endpoint-qualified profiles, explicit held-Shift
messages, alternate rotary CC normalization, two pad banks, release edges,
relative gain guards, a tested Master speed model and a bounded/coalesced output
transport. No hardware messages were sent. All physical bindings remain
unqualified until captured. At this October 30 baseline, the frontend refused Master gain/pan because the
provider had no dedicated Master target. The November 6 work above adds gain and
speed integration; pan/mute remain unavailable. Gate mode/manual/tom/
automatic-import actions are available through the controller menu. Physical
feel/LED/Shift qualification remains open. Input context changes reset held
controls; a known held Shift must release before a new context accepts rotations.

### Retained evidence and next action

User authorized installing tools on 2026-10-11; no TV connection was required for
software terminal proof. Xvfb, xdotool, Pillow/fontTools and Mesa software Vulkan
were provisioned; Rio 0.5.28 is extracted only into ignored task artifacts.
No host audio/service, physical output, deployment, publication or Git index was
changed. Shared source is distributed as exact-version checksum-verified Cargo
directory packages to the two authorized consumers; no path dependencies.

[Shared contract and runnable previews](../reference/INSTRUMENT_PANEL.md) owns
usage, font authoring/licences and deployment limits. The
[dated evidence record](../archive/records/2026-10-11-gp-panel-proof.md) retains
actual screenshots and separately labelled application/terminal measurements.
The original GP-UI-COMPARE record and contributor changes remain preserved.

Next: complete the page-level physical controls and outstanding acceptance above. Keep this card open while
those requirements remain; do not archive it as a completed migration.

## GP-AUTO-LUFS — offline kick-led source hierarchy

State: **IMPLEMENTED / software-validated; musical policy superseded for current auditions**.
Owner: GigPies. Historical scope authorized 2026-10-28: offline automixer rules,
kick 2 LU above bass and all other individual sources at least 1 LU below bass.
The user has removed this hierarchy from the current mixing intent; it remains
an optional implemented policy, not an active requirement. Current intent and
listening feedback are owned by GP-AUTO-ENSEMBLE below.

Implementation and usage: [owning balance guide](../guides/BALANCE_PASS.md#kick-led-lufs-hierarchy),
`src/automix/hierarchy.rs`. Reuse production channel DSP/routing and integrated
K-weighting; hold kick fixed, target bass offset, cap other sources, preserve
quieter sources. Bounded fader moves, actual remeasurement and explicit unmet
objectives. No hidden tonal processing or change to existing no-policy behavior.
Bass energy/definition diagnosis remains owned by
[the existing bass pass](../guides/BASS_KICK.md).

Acceptance:
- [x] Explicit editable 2 LU / 1 LU policy and CLI entry points.
- [x] Reject ambiguous anchors/unsupported FX; preserve missing measurements.
- [x] Retain baseline and candidate; expose bounded failures rather than claim success.
- [x] Focused regressions and normal production suite pass (449 passed, 20 opt-in ignored);
  Python production checks: 50 passed. CLI defaults/non-overwrite, formatting and
  all-target Clippy with warnings denied pass. Private renders, auditions and
  hardware qualification were intentionally not run.

The former listening gate requiring vocals below bass is superseded. No listener
verdict is inferred from LUFS.

Next action: retain this optional capability; do not apply it to current auditions
unless explicitly selected again. Physical activation and live automation remain
separate from this offline rule.

## GP-AUTO-BASS-LOW — observed low energy and definition

State: **IMPLEMENTED / software-validated; musical acceptance pending**.
Owner: GigPies. Authorized 2026-10-28 to improve automixer decisions from
observations and account for bass energy below 100 Hz. Scope is offline software.

Owning contract and commands: [bass low-frequency intent](../guides/BASS_KICK.md#explicit-low-frequency-intent).
Implementation: `src/automix/bass.rs`. Observation: the existing objective only
compared definition with 100–400 Hz body; the retained study separately showed
low-note filtering, weak definition and modest compressor action. Its raw media
and local evidence folder were unavailable here; no new song outcome is claimed.

- [x] Measure below-25 and 25–100 Hz energy at every stage; preserve nine-band output.
- [x] Optional independent low/definition intent; historical policies remain body-only.
- [x] Conditional bounded low shelves; protect fundamentals and attack, prevent
  increased pre-compressor low load, and preserve training/held-out separation.
- [x] Distinguish accepted partial improvement from reaching median tone targets.
- [x] Focused controlled-signal regressions and complete normal suite pass:
  453 Rust tests passed, 20 historical/opt-in tests skipped; 50 Python tests passed.
  Formatting, all-target Clippy with warnings denied, publication index and
  knowledge validation pass.
- [ ] Listening on actual material against current musical intent; no kick/bass/vocal
  LUFS hierarchy is required for current auditions.

Next action: obtain an available selected source for musical acceptance, with
explicit tonal intent; no automatic source normalization,
compression retuning, playback or physical activation is part of this change.

## GP-AUTO-ENSEMBLE — observation-led whole-band automixing

State: **IN PROGRESS; E0–E6 offline software validated; E7 real-recording checks
complete on Raspberry Pi, source review and musical qualification open, 2026-10-28**. Software evidence is
recorded in this card; implementation does not imply musical acceptance.
Owner: GigPies. This card alone owns the implementation sequence, acceptance,
evidence and next action. [Research record](../archive/studies/AUTOMIX_ROLE_RESEARCH_2026-10-28.md)
contains sources R1–R9 and access limitations, not another backlog.

### Outcome and authorization

Extend observation-led assistance across kick, bass, snare, toms, overheads/rooms,
electric/acoustic guitars, lead/backing vocals and explicitly identified remaining
instruments. Preserve useful tone and playing while reducing demonstrated problems
in energy distribution, dynamics and simultaneous competition. Every role must
produce useful evidence, an applicability decision, and either a bounded supported
proposal or an actionable abstention; a universal EQ curve is not the objective.

The 2026-10-28 request authorizes **research, documentation and this plan now**;
implementation is for a new thread invoking this card. Once invoked for execution,
E0–E7 below are the offline implementation scope, including necessary production
DSP additions, synthetic tests and private offline validation on available inputs.
Complete independent software work when recordings are unavailable; preserve the
real-source/listening gate explicitly. No per-phase permission is required within
that scope. Publication, hardware/playback activation and sibling changes remain
subject to the repository's existing authorization rules. Do not infer a live
rollout from successful offline work; [GP-AUTO](#gp-auto--live-soundcheck-and-bounded-assistance)
owns later live integration, with PA behavior in SHR PA and wet DSP in SHR FX.

**Current balance intent (user reconfirmed 2026-10-30): the previous musical
hierarchy constraints are removed. The balance is even, with vocals already
raised; there is no required kick-over-bass offset or vocal-below-bass ceiling.**
“Even” describes the requested balance, not a new equal-LUFS target for every
source. Do not reinstate the historical 2 LU / 1 LU policy from old plans,
command defaults or validation records. Technical safety protections are separate
from these removed musical constraints.

**Automation requirement (user reconfirmed 2026-10-30): source levels must be
calculated from soundcheck evidence and a reusable musical objective, without
per-song listener instructions.** V6 is an accepted listening checkpoint for these
three recordings, not a calibration/reference target or universal +1 dB rule.
The user explicitly rejected using these songs as reference targets and requested
measurement-driven spectral calculation against an explicit target curve. Its existing vocal gain stays in
those settings; do not repeatedly apply it or treat it as an implemented automatic
policy. Earlier V5 +3/+5 dB lead adjustments were manual artistic decisions, not
outputs of a reproducible automatic level calculation. Backing balance remains
unresolved by the algorithm; do not substitute another hand-picked gain step.

### Calculated musical balance — required completion work

The [2026-10-30 decision-path audit](../archive/studies/AUTOMIX_DECISION_AUDIT_2026-10-30.md)
traces source settings, calibration, tone/dynamics, balance, FX, rendering and
listening provenance. Existing E0–E6 validation does not establish unattended
musical balance. The default balance path preserves faders, the experimental
path omits backing-vocal moves, and the ensemble fader path implements only an
explicit hierarchy. These are implementation gaps, not requests for the listener
to provide per-channel target dB values.

Next action: implement a measured level-decision slice for lead and backing vocals
within the ensemble, with an explicit reusable objective and traceable calculations;
then extend that contract across the band. Keep V6 as a listening checkpoint,
not a calibration target, hard-coded answer or unseen qualification recording.

- [ ] **Source/control provenance:** distinguish measured calibration, preset,
  inherited fader, operator override and solver-selected gain. No inherited offset
  is silently accepted as the desired balance. Preserve genuine operator choices
  explicitly; grouping must distinguish direct/room captures, doubles and separate
  singers. Track processing stage and source identity for each measurement.
- [ ] **Activity and context:** measure each performance and its actual simultaneous
  accompaniment after current channel processing; inspect direct voice separately
  from dedicated/shared returns. Cover quiet/strong phrases, sparse/dense sections,
  solo backing entries and overlap with the lead. A first lead channel's activity
  cannot stand in for every singer. Missing/bleed-dominated evidence must not boost
  an idle microphone or be treated as measured silence of the singer.
- [ ] **A reusable objective:** define measurable broadband contribution,
  articulation/competition and phrase-coverage terms, separately for lead and
  backing performances. Select and document numerical targets through development
  evidence and independent listening qualification, not per-song guesses or raw
  V6 fader matching. Equal standing does not mean equal input RMS/LUFS or equal
  fader positions. Spectral proxies must not be called proven audibility.
- [ ] **Calculated actions:** derive gain candidates from target-versus-measured
  errors; evaluate a bounded joint choice for the whole mix, including competing
  sources and backing ensemble sum. Separate persistent level deficiency from
  excessive dynamics, tonal competition and wetness. Account for nonlinear DSP
  and post-fader returns; a simple dB subtraction is only valid for a compatible
  linear level metric with fixed context. Record observations, objective, chosen
  delta, constraints/confidence and predicted/resulting metrics for every move.
- [ ] **Automatic completion:** one soundcheck decision workflow supplies the
  reusable policy and decisions; it must not require the listener to enter an EQ
  target, a backing offset or another +1 dB after each attempt. Insufficient
  evidence should retain a safe state and report the specific missing capture,
  rather than claim mixing success or silently preserve a bad preset as optimal.
- [ ] **Validation:** focused regressions must cover artificially raised/lowered
  input levels in the linear case, inherited backing-fader offsets, changed singer
  counts/coherent groups, sparse phrases, bleed, quiet words, dense accompaniment,
  post-fader FX, missing evidence and bounded repeatability. Verify both correction
  of a known level error and preservation of a balanced case. Freeze policy before
  held-out passages and reserve unseen sessions for generalization. V6 preference
  is development evidence; acceptance must not depend on song names or exact gains.
- [ ] **Live contract in GP-AUTO:** bounded source-frame observations and asynchronous
  decisions, confidence gating, deadbands and controlled gain ramps, current-state
  revisions and manual override. Soundcheck data must suffice; no dependence on
  future whole-song audio, completed WAV exports or blocking render checks. Keep
  audio running independently, avoid gain chasing and retain the existing limiter
  without using it as the musical-balance objective. PA/acoustic acceptance remains
  separate from electrical mix evidence.


### Reference-free spectral calculation, 2026-10-30

User-authorized next experiment: calculate a mix from measured channel spectra
and an explicit spectral curve; do not use the three audition songs as reference
profiles. The initial implementation is `scripts/spectral_balance.py`, reusing
production `ensemble-analyze` observations. The owning
[balance guide](../guides/BALANCE_PASS.md#formula-driven-spectral-candidate) defines
the formula, bounded search, preservation, model limitations and reproducible
commands. Existing V6 controls are the starting state, not target fader values.

Measurements include all channel stages and simultaneous rest-of-mix spectra;
compaction retains raw/routed spectra, identities and duration-weighted coherent
mix powers. The solver fits the vertical position of an equal-power-per-octave
curve, selects supported fader/EQ reductions, and emits settings plus a per-channel
spectral report and decision record. It does not invoke the removed hierarchy,
reference learning, remastering or post-render audio scans. The vocal/backing
prominence and live-control checklist above remains open; a spectral candidate
must not be misreported as completion of those different requirements.

Validation: 17 focused spectral/streaming regressions and the complete normal
Python suite (67 passed); normal Rust production suite (490 passed, 21 existing
opt-in tests ignored); complete temporary publication index (780 entries) and
whitespace checks passed. The spectral target is octave-width weighted, so splitting
a frequency band cannot change the fitted reference level. Strong-coherence,
silence, identity, level-scale, dynamic-processing and EQ-capacity guards are tested.

All 64 routed channels across the three local sessions were measured on their full
native timelines. The calculated changes are:

| Session | Formula-selected changes from V6 |
|---|---|
| Complainiacs | Kick fader -0.5 dB; bass bell about 205 Hz/-1 dB; first electric guitar bell about 205 Hz/-0.5 dB |
| Catbite | Bass bell about 110 Hz/-1 dB |
| Dark Ride | Kick fader -0.5 dB plus 50 Hz/-0.5 dB bell; first bass path bell about 110 Hz/-1 dB |

All added bells use Q 0.8. Vocals, effects, other controls and fixed final output
gains are retained. These are the outputs of the same formula and guards, not
hand-entered song corrections. Numerical excess-loss predictions fall from
4.265 to 1.782, 2.386 to 1.436 and 1.660 to 0.289 respectively; they are not musical
quality ratings or post-render measurements. V7 calculated auditions and channel
spectral reports are ready in ignored `listen/` and
`artifacts/calculated-mix/`. The listener subsequently accepted V7 as much better. No post-render
audio scans, historical/exhaustive campaigns, playback or hardware tests ran.
V7 is the accepted listening checkpoint; no audition recording is a solver
reference target. V6 settings remain reproducible from the frozen measurement.

The listener-requested final mix is V8: +1 dB on every `lead_vocal`
and `backing_vocal` fader, including the whisper double, from V7. Vocal room,
all EQ, effects/send controls, other faders and fixed output gains are unchanged.
This is an explicit listener adjustment, not a newly calculated universal vocal
rule. Its settings are checked for that exact delta; no reanalysis, post-render
audio scans or new production-suite run is needed for this settings-only change.
Current output paths/settings are recorded in ignored `artifacts/full-production.json`;
On 2026-10-30 the listener accepted V8 for all three songs as the best result
and suitable as final. V8 supersedes V7 as the accepted final listening result;
this does not establish live automation or hardware acceptance.

### Repository audit and reuse

The following describes inspected code on the planning date, not new acceptance.
Read these owners before edits; preserve the preceding thread's bass and hierarchy
work wherever it currently resides in Git. Recheck live files rather than assume
an old release binary contains those changes.

| Existing owner | Actual reusable behavior | Gap this task addresses |
|---|---|---|
| `src/automix/bass.rs`, [bass guide](../guides/BASS_KICK.md) | Note support, staged energy, optional definition/low intent, bounded EQ and actual-DSP guards; below-25/25–100 Hz accounting | Role-specific logic is not a generic audibility model; low target is optional, note support can be uncertain, and baseline identity/repeated-run protection need consolidation |
| `src/automix/drums.rs`, [drum guide](../guides/DRUMS.md) | 10 ms envelopes, clustered events, attack/body/recovery, joint dynamics validation | Current EQ probes only reduce two existing boosts; general explicit kick/snare tone objectives are absent; reported FFT bands start at 25 Hz and stop at 8 kHz |
| `tone.rs`, `expert.rs`, [source rules](../guides/SOURCE_RULES.md) | Explicit electric/acoustic guitar profiles; coherent primary/secondary capture; body/presence and compression-relief rules; decay guards | One broad tone ratio misses lows, harshness, register changes and ensemble conflicts; expert families currently cover guitars only |
| `matching.rs`, `matching/{model,measure,fit,check}.rs`, [matching guide](../guides/EQ_MATCHING.md) | Frozen identities, supplied maps, supported bands, amount/reset, explicit passages; contexts include guitar, DI bass and voice | Voice map applicability is not a vocal fault detector; matching is not automatic source diagnosis; reuse the state/identity pattern rather than invent another incompatible format |
| `bleed.rs`, `bleed_reference.rs`, [bleed guide](../guides/SNARE_BLEED.md) | Coincidence/conditional-reference evidence, ambiguous events, protected trials | No validated general-purpose bleed remover; ghost notes and unisons must remain protected |
| `balance.rs`, [balance guide](../guides/BALANCE_PASS.md) | Simultaneous-window band covariance, activity and bounded relationship search | Overlapping filter bands are proxies; several role-specific stages and final bundle interactions remain separate |
| `hierarchy.rs`, [GP-AUTO-LUFS](#gp-auto-lufs--offline-kick-led-source-hierarchy) | Explicit 2 LU / 1 LU defaults, bounded faders, actual source-LUFS remeasurement | Single kick/DI bass, unity trim/no FX baseline; independent whole-song gates can conceal section failures; target failure is reported but candidate is still written |
| `observation.rs`, `render.rs`, `dsp.rs`, `true_peak.rs`, `delivery.rs` | Production-stage taps, signed sums, mono observations, linked strips, LUFS and finalized-file checks | Unify observations with correction passes; avoid a parallel renderer or calling a pre-export meter a finished-file result |
| `ambience.rs`, `effects.rs` and their guides | Explicit offline FX policies, returns and existing source identities | Include fixed existing FX in verification before proposing new wet changes; room microphone, printed FX and generated return are distinct |

The 453 Rust / 50 Python successes recorded for the preceding bass revision are
baseline software evidence only, not completion of any phase below. The documented
`../waves` library and old `artifacts/automix/bass-kick-v1/` evidence were unavailable
in that thread. Verify availability in E0; do not reconstruct recordings from an
archived report or let missing media prevent synthetic/software progress.

### Shared measurement and decision contract

1. **Identity and intent.** Bind channel IDs/files/hashes, BWF offsets, native rate,
   stereo routing, instrument/performance identity, related microphones and capture
   assumptions. A filename or `Role` alone cannot prove two paths are independent
   instruments. Retain unknown identity. Profile fields name measurable objectives,
   supported registers, confidence requirements and allowed controls; null means
   disabled, not a hidden style preset. Existing body-only policies stay compatible.
2. **Energy and time.** Record raw, post-filter/pre-compressor, post-compressor before
   and after makeup, routed contribution, coherent group, direct+FX, master and final
   export observations. Expose DC separately where possible, below-25 Hz energy,
   the existing low bands, body/definition/high bands and total 0–Nyquist accounting.
   State Hann normalization/bin resolution, include DC/Nyquist correctly, and mark
   bands beyond native Nyquist unsupported. Do not silently resample or double-count
   overlapping filters as disjoint power. Longer spectral windows and shorter event
   envelopes serve different purposes; retain tails and all-sample peak guards.
3. **Loudness is distinct.** Keep standard integrated, momentary and short-term LUFS
   quantities defined by R1/R2. Also report common-window energy comparisons, active
   occupancy and per-section distributions. A hit or masked subset shorter than the
   standard window is an event energy metric, not a new kind of LUFS. Preserve the
   explicitly selected optional whole-source hierarchy while exposing its
   section/activity limitations; current auditions do not select that policy.
4. **Context.** Use fixed raw/baseline activity/event masks, quiet/strong strata,
   attacks/sustains/fades, supported pitch/register and simultaneous competitors.
   Compare coherent source groups and the actual remaining mix, including signed
   interaction/mono compatibility. Do not sum independent dB values or call a source
   soloed through nonlinear master processing its additive mix contribution.
5. **Intervention evidence.** Separate measured deviation from a proposed cause.
   Before blaming sub-driven compression, compare controlled EQ-only or detector-only
   alternatives through actual DSP with other controls frozen. Likewise distinguish
   strong high-frequency bleed from wanted attack. High energy, correlation, a narrow
   spectral peak or a changed crest factor alone is not a defect classification.
6. **Freeze and validate.** Reuse matching identities for baseline/source/policy/
   algorithm versions. Select on training only, with a no-change candidate. Freeze
   the entire proposed bundle before its final held-out measurement. Prefer training
   passages before held-out passages to avoid later training inheriting held-out DSP
   history; do not reset processing at arbitrary crop boundaries. Legacy alternating
   splits are historical development evidence, not independently clean validation.
   Held-out failure cannot select a runner-up or retune a threshold. Reused validation
   becomes development evidence; reserve unseen material for final qualification.
7. **Bounded, recoverable decisions.** Enforce per-control and cumulative budgets
   against the immutable baseline, not just the previous pass. Do not append EQ or
   repeat makeup indefinitely. Identical baseline+policy+source reproduces the same
   result; reset restores original settings. Report support, candidate/applied deltas,
   target coverage, regressions, unresolved objectives and rejection reasons separately.
   A partial improvement may be saved as a candidate, but cannot be marked all-targets
   compliant. Missing evidence is not zero energy or a successful target.
8. **Actual composition.** Tone, compression, fader, excitation, sends and master
   processing interact. Validate the final combination through production processing
   and finalized PCM. Common export attenuation does not repair tonal imbalance,
   clipped input or destructive microphone cancellation. True-peak checks, level
   budgets and musical preference remain separate outcomes.

### Role-by-role work specification

The frequency regions below are **initial analysis hypotheses**, not boost/cut
prescriptions, musical target ranges or standards. Keep full-band observations;
adapt supported regions to known tuning/register and sample rate, and make profile
values explicit. No role automatically acquires bass's numeric ratio or a default
high-pass cutoff. R5 motivates capture-dependent interpretation; the choices below
are ours and must survive the indicated tests.

| Role / source grouping | Required observations and likely failure cases | Candidate actions and mandatory preservation checks |
|---|---|---|
| Kick, including explicitly grouped inside/outside paths | Below-25 headroom; roughly 25–120 weight, 120–400 body, 2–5 kHz attack; event duration, double hits, sustain, bass overlap; excessive EQ versus naturally deep kick | Bounded broad EQ/HPF trials only with intent; existing threshold relief; later detector filtering/dynamic bands if justified. Preserve early impact, useful fundamental/weight, double hits and coherent mic sum; never raise sustained sub just to reach LUFS |
| Bass DI / declared amp group | Existing low/body/definition measurements, repeated notes, attack/sustain, low-note support, kick overlap and pre-compressor load | Reuse GP-AUTO-BASS-LOW. Admit amp paths only after group identity and preserved coherent-sum validation replace current DI-only restrictions; do not silently mute or normalize them. Harmonic generation remains separately enabled, bounded and distortion-tested |
| Snare, including top/bottom paths | Roughly 100–350 body, 1–6 kHz attack, 4–12 kHz wires/brightness, ring decay, kick/hat bleed; ghost notes, rolls and flams | Add explicit body/attack/tail objectives and broad EQ candidates. Preserve ghost notes and expressive decay; distinguish desired ring from persistent unwanted resonance. No automatic gate/subtraction/polarity flip from energy or correlation alone |
| Toms and tuned percussion | Per-hit/register body and attack, decay to the next hit, sympathetic ringing and snare/kick coincidence | Reuse event framework with per-tom identity; preserve fills and low fundamentals. Static tone/dynamics first; expansion only for validated isolated-hit evidence with protected quiet events |
| Overheads, hi-hat/cymbals | Whole-kit capture, stereo coherence/mono loss, roughly 3–12 kHz activity and upper-band sustain, snare impact, room contribution | Treat overheads as kit microphones, not isolated cymbals. Broad or later dynamic harshness relief must retain cymbal texture, snare contribution and stereo image. Hi-hat needs explicit identity under current `Other` metadata until a justified schema extension |
| Drum room / vocal room / acoustic ambience | Direct/room relationship, late energy, stereo/mono behavior, sustained bleed and decay | Preserve intended space; use bounded tonal/level proposals with direct-source context. No dereverberation or extra FX inferred from a name; no per-room equal-loudness normalization |
| Electric rhythm / lead guitar | Low contamination versus register support; existing 100–400 body / 800–3200 presence; 2–6 kHz emphasis; chords, sustained notes, palm-muted events, fades, vocal/bass overlap | Extend `expert`/`tone`, retain multi-path identity and existing decay protection. Broad static relief, explicit dynamics relief and later time-selective cuts. Protect low-register body, pick definition and distortion character; no exact technique claims from sparse features |
| Acoustic guitar | Low-body versus handling/proximity energy, 1–5 kHz articulation, upper string noise, stereo/pickup/mic relationship, quiet fingerstyle/strums | Preserve resonance, quiet articulation and natural decay; distinguish intermittent noise from sustained tone before HPF, EQ or dynamic candidates. Do not use electric-guitar settings by default |
| Lead / backing vocals | Phrase activity, low/plosive events, body, roughly 1–4 kHz articulation, 4–10 kHz sibilance, breaths, quiet words and competing guitars/returns | Explicit tone/compression policies; event-sensitive de-essing later. Protect vowels, consonants, breath and quiet phrases; cymbal bleed must not become a sibilance detector. Resolve competition against current musical intent; no vocal-below-bass ceiling applies to current auditions |
| Keys, synths, piano, strings, winds and other declared sources | Register, attack/sustain, stereo content, broad-band support, intentional bass/bright voicing and competing roles | Current `Other` needs an explicit family/capture/profile binding. Generic measurements always available; add supported family rules with fixtures, starting from representative available material. Unknown or unsupported sources preserve sound and report which evidence/profile is missing |
| Generated returns / printed effects | Send-linked timing, wet/dry level, spectral/tail competition, processing already printed into source | Observe existing generated FX in composition. Reuse the owning FX APIs for allowed changes. Never count a return as a second independent instrument or silently double-process a printed effect |

### Execution phases and gates

Implement in the order below. A gate describes required evidence, not a request for
another approval. Update the evidence and next-action fields in this card after
material changes. Do not leave an implemented detector inaccessible from the
normal offline preparation workflow.

**E0 — freeze the starting point and measurement truth.**
Audit the preceding bass/LUFS changes, their output semantics and tests. Verify local
source availability and retain notices. Use matching's identity and preservation
contracts to define a versioned analysis request/result shared by the new workflow.
Set a configurable finite analysis budget derived from admitted channel counts and
sample rates; retain at least the 16-input product target. Existing 8-instrument,
4-secondary or fixture limits must not become a universal product ceiling. Check
FFT total-energy accounting, LUFS reference cases, silence/short clips, native-rate
window boundaries, BWF offsets and DC/Nyquist. Record unresolved disagreements.
Gate: reproducible baseline and independent meter/renderer checks; no new correction.

**E1 — shared observations, profiles and one offline entry point.**
Extract reusable measurement/feature contracts from the existing passes, keeping
legacy output readers/commands compatible or explicitly versioned. Define profile
fields for role/group identity, optional band ratios/relative directions, transient
and dynamics budgets, supported content, allowed actions and evidence minimums.
Provide analysis-only and prepare-candidate modes through one CLI workflow, with
machine-readable results and a short human report. Reuse/refactor `source-analyze`
and `source-pass` where suitable; choose exact CLI syntax here before implementing,
then document it in the existing source-rules guide. No-policy means observation
and unchanged settings. Gate: fixtures for every role route correctly, unsupported
profiles abstain explicitly, baseline hashes and reset/re-run semantics work.

**E2 — kick, bass and snare as the first complete slice.**
Add explicit drum tone objectives, sub/body/attack/tail measurements and the bounded
candidate families from the role table. Keep baseline event masks and note support;
validate quiet hits, overlapping hits and low-end competition in the same mix.
Use controlled EQ-versus-compressor comparisons to explain proposed processing.
Expose the existing bass low objective through the shared request without silently
enabling it in legacy policies. Support an explicitly selected optional hierarchy after
processing; retain separate processing-only and level-only evidence. The historical
2 LU / 1 LU policy is not the current audition objective.
Gate: end-to-end synthetic band example detects and improves a known fault while
preserving a matched no-fault example, ghosts/fundamentals and all technical budgets.

**E3 — guitars and vocals.**
Extend guitar source rules with low/high/phrase-context features and explicit
competition observations. Add actual vocal profiles and event evidence instead of
assuming that voice EQ-map support is a vocal rule. First ship useful static
proposals and dynamics evidence; report event-only issues requiring E6 distinctly.
Gate: vocal consonants/quiet phrases, guitar fades/register/body and grouped paths
survive injected and clean cases; one role's improvement cannot mask another role's
protected feature. Commands expose both candidates and abstentions.

**E4 — toms, overheads, rooms and the remaining families.**
Apply the common contracts to every row above. Keep mixed microphones coherent and
validate bleed ambiguity explicitly. Add explicit family bindings for known `Other`
inputs; implement supported rules rather than pretending an unknown preset covers
all instruments. Complete acoustic-guitar and backing-vocal checks separately from
electric/lead fixtures. Gate: each current `Role` and each admitted extra family has
an automated applicability, correction/no-change and protection test; uncovered
families remain visible as unsupported, not marked done by generic analysis alone.

**E5 — whole-band coordinator and fixed FX verification.**
Select source candidates on training evidence, then evaluate the assembled mix.
First enforce technical protections and any explicitly selected musical policy,
then improve declared tone/competition objectives, then prefer smaller changes. Do not let one weighted
aggregate cancel a failed hard constraint or a regression on another role. Diagnose
whether a conflict calls for source tone, competing-source relief or fader change.
Retain simultaneous source/rest-of-mix spectral overlap as a labelled proxy; do not
claim a psychoacoustic audibility model. No automatic panning, time shifts or source
removal. Generalize hierarchy measurements to declared instrument groups and
existing FX with explicit meter-stage semantics; preserve the original per-channel
mode and its results. Multi-mic grouping must not silently change what an
explicitly selected hierarchy means. If mapping is ambiguous, require identity resolution before moves.

Include a bounded auditory-model feasibility experiment using R9: compare the
simple overlap proxy with an auditory-filter/partial-loudness approach on controlled
target-plus-masker signals. Read the complete method before implementing; record
source/code licensing, presentation assumptions, required level calibration and
runtime. Check frequency-separated/overlapping maskers, low-frequency masking and
temporal cases independently of the optimizer. Without calibrated playback or
sufficiently isolated inputs, retain an explicitly relative competition estimate.
Promote the model only if it adds validated discrimination; record a concrete
reason if it remains experimental. Musical preference still needs E7, and an
uncalibrated model cannot report a listener's absolute audible/inaudible threshold.

Start with at most 128 cheap spectral proposals and four actual-DSP training
finalists per instrument, and at most two bounded ensemble refinement rounds.
These are editable per-run search budgets, not product channel caps or a quality
claim. Record runtime/evaluation counts and unmet objectives on exhaustion. Freeze
one final bundle before validation; no held-out fallback. Repeated runs operate
from the same baseline. Gate: final actual processing, FX and export pass all
protected checks; section violations and impossible target combinations are exposed.

**E6 — time-selective correction where static candidates are inadequate.**
Retain controlled static-vs-dynamic evidence first. Implement bounded dynamic EQ /
frequency-selective detector processing for supported transient resonance/low-load
cases and a vocal de-essing path with event confidence and stereo linking. If
cross-source sidechaining is needed, bind the source identity and detector band,
cap depth and duration, and verify release into pauses. Gate/expander, transient
shaping and harmonic generation are further candidates only when their distinct
source evidence and guards exist; do not enable all processors by role name.

New processors must live in the production processing owner, with offline renderer
parity; do not simulate the effect only in an analyzer. Version schemas, preserve
old-session defaults, freeze explicit makeup and support exact bypass/reset. Define
latency, lookahead, state transitions, resource bounds and common-timeline behavior.
Test pumping, zipper noise, tail/recovery, stereo movement, distortion/aliasing,
nonfinite recovery and sample/block partition invariance. Any path used in a live
callback must retain bounded work without allocation/locks/I/O. No live activation
or new FX-engine fork follows from this phase. Gate: evidence of benefit over the
best admitted static candidate on targeted cases, with protected clean cases and
reported artifacts/limitations; otherwise retain the static result and explain why.

**E7 — representative recordings and musical qualification.**
When sources are available, choose a small varied corpus with identity, notices,
source hashes and known capture limitations. Include clean and problematic examples,
quiet/strong passages, fills, simultaneous kick/bass, vocals with guitars and room
bleed. Do not claim broad support from one song or synthetic tones. Initially use
three independent sessions where available; report actual per-role coverage and
leave absent techniques/families unqualified. Freeze development choices before a
new session/listening set; do not tune repeatedly against the qualification set.

Prepare aligned baseline/candidate excerpts and full-song checks in ignored local
storage. Keep the original delivery levels and, separately, labelled whole-mix
loudness-matched listening copies when needed to test a level-bias hypothesis;
never normalize individual stems or overwrite accepted exports. Verify PCM, timing,
true peaks, routing and exact processing deltas before listening. Compare unchanged,
static and admitted dynamic alternatives in randomized order, allowing no preference
and free comments on impact, definition, harshness, naturalness and intelligibility.
Record the user's verdict separately from metric success. A small personal listening
test is not a standards-compliant perceptual study. Missing media/listening leaves
this gate open while completed software remains reviewable.

### Validation and completion checklist

- [x] Repository audit and dated primary-source research linked; no new audio/DSP
  implementation performed as part of the planning turn.
- [x] E0: independent meter, spectral accounting, source identity and baseline checks.
- [x] E1: shared workflow, explicit intent, compatibility, deterministic re-run/reset.
- [x] E2: kick/bass/snare decisions and required loudness ordering work together.
- [x] E3: guitar and lead-vocal static rules preserve tested synthetic quiet/attack content; real-source qualification remains E7.
- [x] E4: all role-table rows have supported rules or specific remaining evidence gaps;
  unsupported status does not imply musical acceptance.
- [x] E5: assembled mix and fixed FX offline-validated; conflicts and coverage failures explicit; auditory experiment retained outside production.
- [x] E6: justified dynamic processing has renderer parity and its own protection tests;
  each deferred mechanism has evidence-based disposition, not an untested quality claim.
- [x] E7 recording exercise: three full native sessions and controlled real-guitar preservation/recovery; outcomes and coverage limits recorded below.
- [ ] E7 musical qualification: source-review gates, representative intent/event evidence and listener judgments remain separate from technical execution.

Required regression matrix: DC/sub energy, independent FFT power sums, minimum/odd
sample rates, silence/short/tail windows, absent harmonics, 16/32/48-channel configured
fixtures, mono/stereo phase cancellation, multi-mic grouping, clipped PCM versus
valid float headroom, BWF offsets, whole-group gain invariance where applicable,
no-change/unknown intent, insufficient confidence, sparse drums, ghost notes/flams,
held-out veto, compressor memory across split boundaries, source mutation after
planning, stale policy/baseline, repeated-run budget bypass, EQ capacity, near-gate
LUFS changes, incompatible objectives, missing anchors, FX tails and final-file peaks.
Use analytic/synthetic truth and manually labelled events in addition to the
optimizer's own metric; include deliberately failed candidates and clean signals.

During implementation run focused tests for the touched owners. Before completing
shared schema/rendering/DSP phases, run the complete normal Rust suite and required
feature checks, Clippy, formatting and relevant script tests under the documented
build lock. Historical media matrices/long benchmarks remain opt-in; record run and
skipped classes. Reuse `tests/{bass,drums,expert,tone,balance,preservation,matching,
loudness_hierarchy}.rs` where applicable; confirm exact filenames before editing.
Performance tests must state input count/rate, CPU/memory, analysis duration and
hardware; development-machine success is not Raspberry Pi or live acoustic proof.

### Evidence and next action

Planning evidence: source/guide inspection above; research R1–R9 with explicit
retrieval limits. No new real-audio measurement, listening, hardware work, dependency
change or implementation test run was claimed for this documentation-only turn. Local
plan/research links and anchors, whitespace, publication-index and knowledge
validation passed; runtime tests were intentionally not rerun for planning prose.
Keep implementation measurements and exact verification commands in this card as
phases execute; private signal files and source identities stay in ignored artifacts.

Execution evidence, 2026-10-28:

- User invoked offline E0–E7; no per-phase approval is needed. Pre-existing
  bass/hierarchy/research edits are preserved. The documented `../waves` and
  `artifacts/automix/bass-kick-v1` paths are absent; no replacement media acquired.
- E0 adds `src/automix/ensemble/`: versioned request/frozen identities reuse the
  matching SHA-256 contract, bind session/request/capture declarations, native WAV
  channels and BWF offsets, and admit timeline work/source bytes before processing.
  Configurable budgets include BWF silence and fixed FX tails; defaults admit the
  existing offline Session limit of 64 channels, including 16/32/48 fixtures. This
  is an offline format limit, not a product ceiling. Unknown identity stays unknown.
- The new disjoint spectrum includes DC/Nyquist and below-25 Hz; symmetric-Hann
  normalization is the actual window square sum. Partial windows retain their
  actual length; zero padding changes bin spacing, not underlying resolution.
  Unsupported bands return null. Total power is window-weighted Parseval power,
  distinct from unwindowed RMS. Legacy relative spectral readers remain unchanged.
- Fixed a shared loudness timing defect: integer-floor 100 ms hops accumulated
  drift at odd native rates. Rational native-frame boundaries and sample-weighted
  windows now preserve timing within one frame; added ungated momentary maximum.
  Source: R2, sections 2.2–2.3, directly inspected again. Existing 997 Hz reference
  tests and independent DFT/Parseval tests constrain measurement truth.
- Baseline focused command: `flock -xn /home/shome/p/.gigpies-build.lock env
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 cargo test --locked --test bass
  --test loudness_hierarchy --test preservation`: 12 passed. A first new capacity
  fixture used the example's 44.1 kHz session with 48 kHz files; admission correctly
  rejected it. Fixture corrected to 48 kHz, with no relaxed production validation.
- Audit disagreement retained: legacy bass/drum passes use alternating 12 s
  development partitions and different spectral summaries. They cannot supply
  independently clean qualification for the new workflow. E1/E2 must use explicit
  training-before-held-out spans and continuous production DSP history. Legacy
  command/report semantics remain compatible; no historical result is relabelled.
- E0 normal suite: `cargo test --locked --all-targets --features hardware-host`
  under the build lock/environment above: **542 passed, 47 opt-in ignored**.
  Warning-denied Clippy with the same targets/feature passed. Python script suite:
  **50 passed**. Publication index guard and knowledge validation passed. Historical
  media, owner-artifact, network and physical tests stayed opt-in; no playback.
- E1 CLI contract selected before implementation: `ensemble-analyze <baseline.json>
  <source-dir> <new-output-dir> <request.json>` freezes inputs; `ensemble-prepare
  <state.json> <source-dir> <new-output-dir>` always uses that immutable baseline;
  `ensemble-reset <state.json> <new-output-dir>` restores it. Old commands stay
  compatible. E1: nine focused tests passed; the complete default suite passed;
  feature-enabled warning-denied Clippy passed. Profiles without validated rules
  explicitly abstain. Stage data includes FFT/window normalization, all-sample
  peaks, separate DC mean/bin power, and integer-contact versus float-headroom
  counts. These observations do not imply a final composed-mix result.

E2 implementation evidence:

- `ensemble/correct.rs` adds at most three bounded broad EQ trials and one
  compressor-only intervention per supported single-source profile. Original
  controls/makeup stay fixed; the compressor comparison is not an enabled action.
  Candidate selection uses explicit training only, with baseline activity and
  exactly paired windows. Per-passage objective errors cannot cancel each other.
- Bass reuses the existing 16384-sample note estimator and retains a separate
  fundamental guard. The common spectral window now uses that same length;
  feature admission derives its count from the owning constant. A test exposed
  the estimator's incompatible initial 8192-sample input; corrected by reusing
  its required length, without weakening its note confidence test.
- All-sample event energy/peak and protected spectral loss/rise are hard checks.
  One frozen bundle receives held-out validation; failure restores baseline
  settings with a reviewable `candidate.json`, never a held-out runner-up.
- The existing hierarchy is proposed from training after processing, with separate
  level-only evidence and final whole-source compliance. Missing/ambiguous
  performance identities or FX abstain. Fixed FX/grouped semantics remain E5 work.
  The production renderer and finalized-delivery checker verify accepted output.
- Eleven focused tests passed, including known kick fault/clean/held-out-veto
  cases and jointly corrected bass/snare with ghost-event/fundamental guards.
  The initial hierarchy fixture exposed an unreachable >6 dB move and correctly
  reported failure; the reachable counterpart and full feature suite passed.
  Correction-specific temporal/role coverage remains subject to the phase gate.

- E2 full normal feature suite passed, including all 11 ensemble tests. Final
  review moved whole-file PCM contact into the final safety veto so held-out
  contacts cannot influence training selection. Settings are now installed only
  after finalized export/source verification; rejected proposals remain separate.

E3 implementation evidence:

- Electric rhythm/lead guitar and lead-vocal profiles use explicit static intent,
  registered pitch-region protection, quiet/attack/fade event budgets and
  per-passage checks. No role name chooses a curve. Cases needing selective
  processing explicitly retain static/baseline results pending event evidence.
- Simultaneous source/rest spectra and signed cross-power use actual routed
  channel samples; an independent anti-phase fixture verifies cancellation.
  The overlap fraction is labelled a spectral proxy, not calibrated audibility.
  This remains before excitation/FX/master; E5 owns the complete composition.
- Fourteen focused ensemble tests passed, including paired fault/clean synthetic
  guitar fades and vocal quiet phrases/consonant bursts, and exact preservation
  when related microphones cannot yet be validated as a coherent group. Amp
  paths likewise abstain pending DI/amp group validation. No real speech/song
  or physical playback qualification is claimed.
- E3 normal feature suite: **551 passed, 47 opt-in ignored** under the documented
  lock/environment. Final warning-denied feature Clippy passed; after tightening
  unsupported-profile compliance reporting, all **14 ensemble regressions passed**
  again (29.00 s). Final Python suite: **50 passed**. Formatting, whitespace,
  publication and knowledge validation passed. Publication checked both the actual
  complete index and a temporary complete index including the new files (760
  entries); the actual index was preserved. No commit or push was performed.
- Tests used synthetic 48 kHz mono/stereo sources, including 2 s / 2–3-channel
  correction fixtures and 16/32/48-channel admission fixtures. Host: AMD Ryzen
  5 5600G, 12 logical CPUs, 62 GiB RAM; serialized compilation, normal test runner.
  These timings are regression execution evidence, not a resource benchmark or Pi
  performance qualification. No historical media matrix, owner-artifact, socket,
  hardware or audible-playback test was activated.
- **Prior invocation stopped here: E4–E7 were unfinished.** Remaining software is
  acoustic/backing/tom/kit/room/other-family qualification, coherent group/FX
  coordination, section hierarchy, the R9 feasibility experiment and justified
  production time-selective processing with its artifact/parity checks. The R9
  author review is reachable; no auditory model or experiment was implemented.
  Missing recordings/listener input affects E7, not authorization for E4–E6.

### Continuation evidence, 2026-10-28

- Preserved the incoming uncommitted E0–E3, bass and hierarchy changes in place.
- E4 extends explicit static tone trials to toms, acoustic guitar, backing vocals,
  overheads/rooms, hi-hat, piano, keys, synth, strings and winds. These are bounded
  declared-band interventions, not automatic instrument fault diagnoses. Mixed
  microphone profiles require reviewed `isolated_ensemble_capture` identity;
  ambiguous bleed and printed effects abstain with specific evidence requests.
  Distinct register/envelope/stereo synthetic cases exercise correction, clean
  preservation and quiet decay. The 12-family paired regression passed (119.04 s);
  an initial tom fixture exposed leakage into its declared low band and was
  revised as development evidence, not held-out qualification. Musical/technique
  coverage still requires E7.

- E5 adds coherent profile spectra and event sums, shared bounded EQ, explicit
  `grouped_hierarchy`, direct-plus-dedicated-return LUFS and per-section LUFS with
  continuous K-filter history. Selection gates complete blocks from declared training
  passages only, without joining gaps or admitting held-out levels. Shared/unknown
  return attribution abstains.
  Whole-source compliance cannot conceal a section conflict. Original independent
  channel hierarchy remains available. Composition verifies every production
  stage and common 100 ms window through the **same explicit delivery limiter
  policy as export**, plus mono compatibility and final PCM. Abrupt-source release
  tails caused a real veto; the corresponding tapered-source case passes.
  Search remains one bounded source round (no ensemble refinement), at most three
  static trials or two static plus one dynamic trial, plus one compressor-only
  comparison. `budget.max_static_trials` permits smaller runs; every rejection,
  evaluation count, elapsed selection time and unmet objective is explicit.
- E5 auditory feasibility: eight controlled spectral cases plus unmasked controls
  were executed using separately compiled GPL reference kernels, with the complete
  executed kernel source inspected. The [research follow-up](../archive/studies/AUTOMIX_ROLE_RESEARCH_2026-10-28.md#bounded-auditory-feasibility-follow-up)
  records source pin, licensing, assumptions, ratios and 0.487462 s runtime.
  No code/dependency entered production. Frequency discrimination improves over
  the coarse proxy on these cases, but waveform temporal validation, calibrated
  presentation and source isolation remain insufficient: **not promoted**.
- E6 adds one optional versioned production `Channel.dynamic_eq` band: fixed
  bell-cut path, band detector, smoothed stereo-linked crossfade, 0–6 dB depth,
  no makeup/lookahead/sample delay, bounded allocation-free sample processing.
  Existing sessions omit it and retain exact defaults. Reviewed isolated vocal,
  transient-resonance or low-load evidence and disjoint problem/protection passages
  are frozen in request v2. At most four actual-DSP finalists are evaluated per
  profile. Dynamic correction must beat the best admissible static result;
  held-out evidence only vetoes. Standalone tests cover native rates, 50 ms bursts,
  quiet recovery, linked stereo, exact bypass/reset/partitioning, nonfinite reset
  and steady-state agreement with the linear cut (no additional steady harmonics).
  End-to-end high-band and low-load fixtures beat static cuts that would attenuate
  protected quiet passages. This does not classify real sibilants or prove freedom
  from modulation artifacts. Brief events still need adequate declared spectral
  passage coverage; missing evidence abstains. No live integration was activated.
- E6 deferred mechanisms: cross-source sidechains, gates/expanders, transient
  shapers and new harmonic generation have no demonstrated independent need in
  these reviewed cases. Shared-microphone ambiguity, quiet events and existing
  static/dynamic solutions argue against enabling them by role. Existing FX is
  fixed, not forked. No untested musical benefit is claimed for deferred controls.
- E7 independent tooling: `ensemble-listen-pack` verifies the state/candidate,
  rechecks finalized deliveries, renders the baseline under the same delivery
  policy and writes randomized, exact PCM A/B excerpts from held-out passages.
  It measures excerpt peaks (including boundary effects), retains original levels,
  rejects overwrite and records blank preference/comments plus a separate key.
  No playback occurs. Loudness-matched comparisons and a separate admitted static
  alternative can follow a concrete level-bias/listening need; no stem normalization.
- E7 source boundary: the user confirmed recordings are **on the Raspberry Pi**
  and will continue source testing there. No real sessions or listening verdicts
  were available here: real-source coverage **0 sessions**, musical acceptance
  **pending**. No replacement recordings were downloaded. Request/state v2 requires
  fresh analysis from original settings; legacy Session defaults remain compatible.
- The full
  12-family matrix was exercised before being made opt-in as a recorded exhaustive
  study; the normal suite retains tom/acoustic/backing/overhead correction and
  clean/protection cases, all-role routing, mixed-capture refusals and shared guards.
  On demand: `cargo +1.97.1 test --locked --test ensemble
  all_remaining_family_register_envelope_matrix -- --ignored --exact` under the
  documented build lock/environment.

Final validation uses Rust 1.97.1 and `Cargo.lock`, serialized with
`flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1`:

- `cargo +1.97.1 test --locked`: **480 passed, 0 failed, 21 ignored**.
- `cargo +1.97.1 test --locked --features hardware-host`: **564 passed, 0 failed,
  48 ignored**; no physical device activation.
- `python3 -m unittest discover -s scripts -p 'test_*.py'`: **50 passed**.
- `cargo +1.97.1 clippy --locked --all-targets --all-features -- -D warnings`
  and `cargo +1.97.1 fmt --all -- --check`: passed.
- Whitespace, complete temporary publication index (**768 entries**, preserving
  the actual staging area), actual index (**752 entries**) and knowledge links
  (**19 projects, 94 notes, 0 broken links**) passed. Versioned hooks remain enabled.
- Final review added exact limiter-policy parity, unique profile names, excerpt
  boundary peaks, short dynamic bursts and training-passage-only hierarchy
  regressions. The parity fixture was corrected to explicitly select matched
  output so it actually exercises the legacy limiter difference.
- Local logs and auditory probe evidence live in ignored
  `artifacts/automix/ensemble-validation-20261028/`; generated audio and the
  external GPL reference source are not publication inputs.
- Intentionally skipped: opt-in hardware/playback/physical synchronization,
  historical media auditions, long clock stress/benchmarks and fixture generators.
  The exhaustive family matrix has the separate recorded run above. No Pi
  runtime/resource or acoustic qualification is claimed by workstation tests.

### Raspberry Pi continuation, 2026-10-28

- Original local WAVs are available. Three previously used research sessions were
  analyzed over their full native timelines: 12/16/13 routed inputs at
  44.1/48/96 kHz, respectively. This is real-recording development coverage, not
  three previously unseen qualification sessions. Frozen requests, original source
  hashes, BWF identities, per-role observations and reproducible commands remain in
  ignored `artifacts/automix-validation.json` (records keyed by their former relative paths).
- Historical validation requests preserved established routing and declared the
  then-selected 2 LU/1 LU hierarchy with the existing 6 dB fader bound. This
  musical policy is superseded for current auditions; these are historical results.
  Related vocal/room and kit-ambience paths are grouped coherently. Unknown bleed and processing history remain
  explicit; no source-specific tone fault or dynamic-event identity is invented.
- Source observation found 54/5/0 full-scale PCM samples across these three
  routed sessions. The first two preparations retained their baselines under
  the source-review veto. The first could not reach the hierarchy within its fader budget;
  the second lacked training LUFS for several declared parts, including quiet
  backing-vocal, tom and synth paths. Full-scale contact is not itself a diagnosis
  of recoverable clipping.
- The third session produced a bounded fader proposal, then failed composition
  protection in **116 distinct 100 ms windows** (464 stage-window failures across
  four stages). Peak rise reached **2.492 dB** against a 1 dB budget; energy loss
  reached **6.460 dB** against a 6 dB budget. Its saved settings exactly match the
  baseline. **No full-band proposal was automatically accepted.** The budgets and
  held-out passages were retained unchanged. Private composition evidence records
  the failing frames and both measured levels for review.
- The original fader-only diagnostic excerpts were technically verified but
  rejected by the listener as an unsuitable comparison, then retired. This is
  not an A/B preference or approval of the rejected proposal.
- The revised full-production auditions cover Complainiacs, Catbite and Dark Ride.
  V5 produced three full mixes alongside three `*-before.wav` references;
  the current revision is identified in the balance intent above and `listen/README.txt`. Every reference was rebuilt and verified with added channel/master
  filters, static/dynamic EQ, compression and FX disabled; the former references
  had inherited filters, and Dark Ride also had legacy EQ/compression. Those
  misleading comparators and the previous full mixes are retired.
- V5 removes the remaining +0.5 dB kick/bass offsets: both faders drop 0.5 dB
  from V4, including both explicitly selected Dark Ride bass files. Main lead
  faders rise 3 dB for Complainiacs/Catbite and 5 dB for Dark Ride. Backing vocals,
  vocal room and the named Dark Ride whisper double retain their V4 levels.
  Named lead sources already have lead routing; historical bus evidence and saved
  faders indicate recessed lead balance, particularly against Dark Ride backing
  vocals. This is a balance diagnosis, not newly measured vocal intelligibility.
- Main-lead reverb/echo sends compensate the fader increase and fall another 2 dB,
  bringing the direct voice forward without raising those returns. Lead reverb
  predelay remains 45 ms (backing 35 ms). A parallel lead chorus uses 0.25 Hz,
  0.6 ms depth, 17 ms base delay and -23 dB sends, filtered at 180 Hz / 6.5 kHz.
  The dry voice remains intact; subtlety and musical balance await listening.
- Existing channel EQ/compression, excitation, +2 dB / 8 kHz shelf and master
  limiter settings remain unchanged. The previous final output gain is fixed
  for each song. The plain renderer's causal float bus is exported at that gain;
  its unused automatic normalized copy is discarded. No balance search, loudness
  matching, independent peak scan or other post-render audio analysis runs for
  these control edits. The renderer still emits built-in meters, which do not
  choose the delivered gain. V5 makes no new integrated-loudness or true-peak
  measurement claim. WAV structure, duration, hashes and exact prepared settings
  are checked and retained in ignored `artifacts/full-production.json`. No engine
  code changes or new production-suite run are needed for this settings revision;
  historical/exhaustive tests remain opt-in.
- Live-operation direction: apply prepared gain/control changes directly to the
  running DSP. Metering and analysis may inform automation asynchronously, but
  must not block audio or re-optimize a master after each gain edit. The limiter
  catches peaks; it does not determine musical balance. These offline auditions
  exercise the requested sound changes and do not establish live integration.
- These user-authorized artistic revisions are offline auditions, not accepted
  outputs of the guarded automatic correction workflow. That workflow's previous
  vetoes, held-out evidence and bounds remain unchanged. Musical acceptance is
  pending; no playback or physical activation was performed here.
- DSP-only timing was investigated after the listener questioned render speed.
  The listener clarified that 2x was an informal estimate, not an acceptance
  requirement; this hardware targets **16 channels at 48 kHz**, excluding
  39-track and 96 kHz live operation. The measurements below therefore do not
  establish pass/fail for the intended live workload.
  A release-built, in-memory Strip/Rack microbenchmark on Pi5 CPU 3 excludes WAV
  I/O, observers, LUFS, logging and export checks. Two approximately eight-second
  runs after warmup measured the v2 configurations: Complainiacs (12 inputs, 44.1 kHz) at **4.13-4.16x**,
  Catbite (13 inputs, 96 kHz) at **1.57x**, and Dark Ride (39 inputs, 44.1 kHz)
  at **1.60x**. Catbite's 48-frame block p99 is 531-536 us against a 500 us
  deadline. Normal scheduling also produces occasional outliers. This is a short
  offline-DSP microbenchmark, not a complete 48 kHz live graph, PA/transport or
  hardware qualification. The compact private record retains reproducible harness
  source and conditions; no standalone runner or extra output tree is retained.
- A controlled +3 dB guitar bell selected the expected −3 dB inverse in training.
  A separately checked diagnostic render of that rejected proposal reproduces
  **all 10,056,212 unmodified-reference PCM24 samples exactly**. Independent
  FFmpeg/libsoxr checks passed for both files. Maximum float-bus difference is
  1.863e−9. The original waveform is a technical reference, not a listening verdict.
  The held-out ending changes its natural band ratio from about +3.3 to −8.0 dB;
  the fixed training-derived target therefore vetoes the inverse. The original
  control remains unchanged. Targets and protections were not retuned against
  held-out data; diagnostic export does not override automatic rejection.
- Fixed two integration regressions: explicit source preservation now clears
  dynamic EQ, and `all_targets_met` includes every declared hierarchy section,
  including missing evidence. Exact PCM preservation and a whole-song-pass /
  section-fail end-to-end regression pass.
- Coverage is 41 routed inputs across 12 declared families: kick, bass DI, snare,
  toms, overheads, drum room, electric guitar, lead/backing vocals, vocal room,
  keys and synth. These are capture declarations, not successful correction or
  listening qualification for every family. Acoustic guitar, hi-hat, piano,
  strings, winds and printed-effects profiles remain unqualified here. Reviewed
  real dynamic-event identities and dynamic-processing listening are still absent.
- The first complete default run exposed a scheduler-dependent spectrum test:
  a fixed sleep did not guarantee a complete window after a level change. The test
  now waits for an acknowledged source-frame transition and observes the existing
  query-rate limit. Production spectrum behavior is unchanged.
- Final Rust 1.97.1 checks under the shared build lock, `CARGO_INCREMENTAL=0` and
  `CARGO_BUILD_JOBS=1`: `cargo test --locked --all-targets` **482 passed / 21 ignored**;
  the same command with `--features hardware-host` **566 passed / 48 ignored**;
  all-target/all-feature Clippy with `-D warnings` and formatting passed.
  Python's normal script suite passed **50 tests**. Historical auditions,
  exhaustive family matrices, long benchmarks, owner-artifact fixtures and
  physical/playback tests were intentionally skipped. Offline elapsed times under
  concurrent development checks are not realtime capacity or hardware acceptance.
- Repeated analysis/prepare observations were bit-identical in all six native
  and controlled runs. All 41 original WAVs (2,241,434,386 bytes) still match their
  frozen hashes. Compact observations, requests/states, complete decisions,
  failing-window evidence and delivery verification receipts are consolidated in
  `artifacts/automix-validation.json`. The short diagnostic clips are retired;
  only the six requested full-song comparison WAVs remain in `listen/`.
  The obsolete automix output tree, one-use runners, analysis environment and
  completed test executables were removed. Historical frozen settings and authored
  patches remain in `artifacts/automix-history.json`. Earlier artifact paths in
  dated study records describe retired outputs, not current listening locations.
  Original media and unresolved physical bench evidence are preserved.

**Next action: obtain musical judgments on the revised productions. Assess live
processing against the intended 16-channel/48-kHz configuration on this hardware,
without imposing the earlier informal 2x estimate as an acceptance requirement.**
Use the private results and diagnostics before declaring a preferred mix. Any
revised intent or source selection requires a fresh v2 analysis and new independent
qualification material; do not turn these held-out vetoes into a parameter search.
Keep absent families, real dynamic-event processing and the experimental auditory
model unqualified. No new permission is needed for the already authorized offline
scope; publication and physical activation remain outside this invocation.


[GP-CAL-REVIEW — calibrated offline Desk review](../archive/pa-cal-review-2026-10-29.md) has software acceptance; physical qualification remains separate.
