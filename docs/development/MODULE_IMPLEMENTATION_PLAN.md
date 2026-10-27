# GigPies integration: plan and implementation

Updated 2026-10-11. This document owns cross-module integration tasks from planning
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

## Active work and dependencies

| Task | Owner / contributors | State and next action |
|---|---|---|
| [GP-PANEL](#gp-panel--shared-physical-instrument-panel) | GigPies shared UI; Desk/Lightdesk consumers | IN PROGRESS: real-terminal proof and initial production adapters; acceptance gates remain |
| [GP-METER](#gp-meter--measured-audio-on-the-real-desk) | GigPies lead; SHR Desk consumer | IMPLEMENTED / software-validated; owner-artifact P4 BLOCKED: supply reviewed manifest |
| [GP-FX](#gp-fx--writable-owner-effects-in-the-console) | GigPies lead; SHR FX provider, SHR Desk consumer | PLANNED: depends on owner FX-02/03 prepared writable contract |
| [GP-SHOW](#gp-show--audio-scenes-and-operator-recovery) | GigPies lead; SHR Desk consumer | PLANNED: define scene contents, safes and reviewed recall before implementation |
| [GP-AUTO](#gp-auto--live-soundcheck-and-bounded-assistance) | GigPies lead; SHR Desk consumer | PLANNED: needs trustworthy measurements and an explicit musical objective |
| [GP-REC](#gp-rec--complete-integrated-recording-workflow) | GigPies lead; SHR REC provider, SHR Desk consumer | PLANNED: audit existing host lifecycle before defining missing operator actions |
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

State: PLANNED. Outcome: real prepared wet-effects edits and readback from Desk
through GigPies into SHR FX, preserving independent dry/protected/recorded paths.
Provider design and DSP belong to [SHR FX's plan](https://github.com/gigpies/shr-fx/blob/main/docs/GIGPIES_IMPLEMENTATION.md).
Next: resolve FX-02/03 units, f64 preparation/publication/retirement, bounds and
fault/tail semantics before specifying the host and Desk adapter. Acceptance must
check actual wet samples and failed edits, not just ACKs or standalone rack tests.

## GP-SHOW — audio scenes and operator recovery

State: PLANNED. Outcome: browse/preview/cancel/recall a bounded audio scene with
explicit safes, revision checks and truthful readback. Next: define saved versus
machine-only state and retained human holds. Exclude automatic unmute, resumed
recording/talkback and implicit cross-system lighting actions. Test atomic refusal,
interrupted recall, reconnect and recovery against actual providers.

## GP-AUTO — live soundcheck and bounded assistance

State: PLANNED. Outcome: use real measured sources to propose useful bounded mix
changes while preserving human control. Next: after GP-METER, select one defined
soundcheck/assistance behavior and connect existing offline logic where applicable.
Scope authority machinery already exists; do not recreate it. Require held-out
engineering evidence and separate human listening before musical acceptance.

## GP-REC — complete integrated recording workflow

State: PLANNED. Outcome: safe operator start/stop, readiness, written-versus-queued
progress, integrity and finalized outcome for actual raw REC. Next: inspect existing
host actions and Desk read-only health, then plan only missing operator/storage
feedback. REC owns writer/recovery; standalone player and shell completion do not
gate this task. Unknown durability must stay unknown.

## GP-H1 — physical audio qualification

State: PLANNED; requires a separate authorized physical session.
Next: observed Stagebox/Brain inventory and explicit socket/device maps, UMC/ADAT
clock relationship if that rig is present, duplex drift, electrical latency and
safe fault/rearm. Follow [modular physical procedure](../reference/MODULAR_PROCESSING.md#mapping-and-physical-acceptance)
and [Brain procedure](../reference/BRAIN_AUDIO.md#bounded-physical-followup).
Keep H8's unresolved timing/right-return evidence. Qualify sustained configured
hardware deadlines and acoustic protection; software profiles are not hardware proof.

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


## GP-PANEL — shared physical instrument panel

State: **IN PROGRESS**. Owner: GigPies integration/shared presentation package;
contributors: SHR Desk and SHR Lightdesk. Authorized 2026-10-11: implementation
in the three canonical repositories, one thread, no delegation/worktrees/clones,
no commits/publication/deployment, hardware or host service/audio changes.
Preserve the existing GP-UI-COMPARE record and both contributor-plan edits.

Outcome: compact Ratatui production consoles with reusable authored colour PUA
material tiles, ordinary readable text and actual provider state. An actual
terminal specimen must pass visual/cell acceptance before production migration.

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
