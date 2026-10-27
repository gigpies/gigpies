# Validation and acceptance records

## Integrated modules and native software — 2026-10-04

The [current implementation checkpoint](../plans/MODULE_IMPLEMENTATION_MAP.md#execution-checkpoint--2026-10-04)
and [integration evidence](../workflows/HEADLESS_INTEGRATION.md#evidence-and-remaining-work)
record the complete owner test counts, actual-provider checks and combined recovery
result for this source milestone. Core runtime owners ran their normal suites,
matching native/device-free host feature checks, formatting, warnings-denied Clippy
and release builds. Hardware, historical media, auditions and long/load experiments
remain separate. Rust 1.97.1, committed lockfiles, disabled incremental compilation
and one parent-held build slot per host were used.

The twelve owning repositories publish their own reviewed source and plans. Each
complete index and outgoing history is checked; Desk/Lightdesk have independent
publication guards with exact font/licence exceptions. Private worker records,
provider binaries, recordings, role state and generated renders remain outside Git.
This milestone adds no version bump, tag, binary release or deployment. Earlier
entries below preserve their original acceptance and publication scope.

## Photorealistic hero correction — 2026-10-04

Replaced the schematic README hero with the requested generated photographic
scene and archived the SVG banner. Visual inspection covers the two distinct
screen/controller pairs, single foreground Brain, separate Stagebox/I/O rack
with local screen, phone controls, lit instrument stage, PA and monitor wedges.
The image remains an intended-system visualization rather than hardware evidence.

The PNG signature, 1672×941 dimensions, SHA-256 and 2.17 MiB size were checked;
its exact path is added to the reviewed artwork list. All 37 Python tests pass,
including publication-policy regressions. Local current-document links and staged
whitespace pass, with the full-index publication guard enabled. Runtime code and
package versions are unchanged, so the 0.2.3 Rust/build evidence below is reused.
No hardware, playback, shared load, long research or new Rust tests were run for
this artwork/documentation-only correction. The existing v0.2.3 tag is retained.

## 0.2.3 publication checks — 2026-10-04

Actual host **rpi5**, Rust 1.97.1, edition 2024, committed Cargo.lock and
`CARGO_INCREMENTAL=0`. This pass changes documentation, original SVG artwork
and package version metadata; existing transport/hardware code is included in
the outgoing state, with its prior qualified evidence preserved.

- Complete normal Rust suite with the optional host code compiled:
  `cargo test --locked --all-targets --features hardware-host` — **209 passed**.
  Three historical private-media tests and one opt-in local socket test were
  intentionally ignored. No audio/MIDI/DMX device was opened.
- Complete Python suite — **37 passed**. Formatting, locked check and
  all-target Clippy with `hardware-host` and warnings denied passed.
- Locked optimized CLI build passed and reports `gigpies 0.2.3`. Manifest and
  lockfile package versions agree. The optional hardware-host binary is also
  built for publication; building it does not run a device session.
- All 55 Markdown documents were scanned for local references and current
  system/version wording. The 387 local link references have no missing current
  targets. Archived snapshots retain their original `docs/` link context; the
  preserved drafts are historical, not current operating instructions.
- Hero and system SVGs were parsed, rendered and visually inspected. The diagrams
  show both display/controller pairs, separate surface/engine owners, independent
  performer monitor buses and Stagebox-local recording. They claim intended
  topology, not live-console or physical-lighting acceptance.
- Full-index and outgoing-history publication checks run with the versioned hooks.
  Named staged files are reviewed; recordings, private state, generated media,
  one-off review tooling and sibling source remain outside the publication.

The independent Desk/Lightdesk sources are not bundled or pushed by this GigPies
publication. Their earlier offline validation is recorded by their owners. Native
windows, real lighting authority/output, controller/display assignment and combined
CPU/GPU/memory acceptance remain planned. No historical research matrix, new music
render, playback, load test, physical output or host/service configuration ran.

The superseded publication logs/previews were removed during the October 12
cleanup; the results above remain the acceptance record. For commit
`eb32afdfb0520ccf3c3df7f7a264c6496407a387`, recorded CI runs 37196979807
(main) and 37196979868 (v0.2.3) both succeeded.

## Summing and delivery execution — 2026-10-03, unreleased

The [owning plan](../plans/SUMMING_PLAN.md) and [delivery contract](../../guides/SUMMING_DELIVERY.md)
record the engineering result. Rust 1.97.1, the committed lockfile and
`CARGO_INCREMENTAL=0` were used throughout.

- Formatting, locked check, all-target Clippy with warnings denied and release
  build pass. The complete normal suite passes **154 Rust tests** and **37 Python
  tests**. Three unrelated historical media tests remain intentionally ignored.
- The applicable opt-in true-peak study passes 280 waveform/rate cases plus 20
  burst cases. Maximum reference difference is 0.106964 dB against the declared
  0.2 dB limit. Reference refinement changes burst results by at most 0.000274 dB.
- Independent neutral routing passes synthetic 1–64-channel checks and all twelve
  complete SOURCE/FINAL scalar replays. All six reconstructed SOURCE/FINAL PCM
  and float buses match the retained historical hashes exactly.
- Six new complete mixes retain frozen processing and GigPies effects. Production
  and independent final-PCM meters pass the −1 dBTP ceiling. Independent readings
  range from −1.400441 to −1.395178 dBTP; maximum disagreement is 0.004922 dB.
  Static conversion, source identities, full timelines, tails and seven exact
  excerpts pass. Both master limiters retain zero reduction on these selections.
- The local handoff is `artifacts/automix/summing-study/2026-10-03-engine/REPORT.md`;
  its listening index, cleanup manifest and completion record identify retained
  files. Original recordings and prior retained evidence are unchanged.
- The complete proposed source tree passes the publication guard in a disposable
  index. The real Git index is unchanged; versioned hooks remain enabled.

Listener preference remains not reviewed and hardware remains unverified.
No playback, host/service changes, sibling writes, push or publication occurred.

## 0.2.2 publication checks — 2026-10-03

Rust 1.97.1, committed lockfile and `CARGO_INCREMENTAL=0`:

- Formatting, locked check, all-target Clippy with warnings denied and optimized
  build passed. The executable reports `gigpies 0.2.2`; package versions agree.
- Complete normal Rust suite: **145 passed**, three historical private-media tests
  intentionally ignored. Complete normal Python suite: **34 passed**.
- The complete staged tree passes the publication guard with versioned hooks enabled.
  Reviewed source, tests and documentation are staged explicitly. No private settings,
  recordings, generated audio or one-off runner is published.
- Cargo source-package listing contains no private/media paths. Staged bytes match
  the reviewed working files; local documentation links and whitespace checks pass.

The release includes the existing paired EQ diagnostic and the new FX calibration,
master/coverage, recovery and saved-report work. It does not implement the separately
planned summing/true-peak engine changes or claim a listener preference.

The user explicitly retired all old generated renders before the next engine study:
234 regular render paths and 52 links, recovering about 4.19 GiB. Original media,
frozen settings, hashes and scalar evidence remain. Checks preserved 428 original/
library/test-input metadata records and 2,473 nonaudio evidence files. The private
retirement manifest is under `artifacts/automix/render-retirement-2026-10-03/`.

No new real-audio evaluation, full-song render, playback, exhaustive historical
study or hardware test was run for this publication. Synthetic tests use temporary
media and clean it up. Filesystem counters remained zero; the prior crash cause
is unresolved and a forced offline full scan has not been established.
Superseded local publication logs were removed after summarizing these results. Remote CI is separate
from these local results and must be checked for the pushed revision.

## 0.2.1 publication checks — 2026-10-03

Rust 1.97.1, committed lockfile and `CARGO_INCREMENTAL=0`:

- Formatting, locked check, warning-denied all-target Clippy and release build passed.
- Complete normal Rust suite: **126 passed**, including nine matching regressions;
  three historical private-media tests intentionally ignored. Complete normal
  Python suite: **34 passed**.
- Release executable reports `gigpies 0.2.1`; manifest and lockfile versions agree.
- The complete staged tree passes the publication guard. The local review page's
  inline script was reviewed and added to the explicit script list; it downloads
  a selection request and has no network, playback or hardware control path.
- Cargo source-package listing contains no private directories, recordings or audio
  archives. Changed-document local link targets and staged whitespace checks pass.

The [matching evidence](../studies/EQ_MATCHING.md#validation-and-bounded-pilot--2026-10-03)
records synthetic recovery, the real pilot's conservative miss and preserved mix
comparators. New full-song auditions, historical exhaustive renderers, playback and
hardware checks were intentionally skipped for publication. Musical acceptance is
pending; no true-peak claim is made. Existing recordings, maps, private evidence and
complete exports remain local. Superseded local publication logs were removed. Recorded CI run 37076858828
succeeded for commit `6682409b828d6105825568b07c8ccbada7d54112`.

Versioned commit/push hooks are enabled locally. Remote CI is separate from these
local results and must be checked for the pushed revision.

## 0.2.0 publication checks — 2026-10-03

Rust 1.97.1, committed lockfile and `CARGO_INCREMENTAL=0`:

- Formatting, locked check, warning-denied all-target Clippy and release build passed.
- Complete normal Rust suite: 117 passed; three historical private-media tests
  intentionally ignored. Complete normal Python suite: 34 passed.
- Release executable reports `gigpies 0.2.0`; application and lockfile versions agree.
- Complete staged Git tree and existing commit history passed the publication
  guard. No tracked path matches the private/generated ignore rules. Synthetic
  ignore probes cover user state, nested artifacts, recordings, caches and media.
- Cargo source-package listing contains no private directories, recordings or
  generated archives. No binary release attachment or local audio is published.
- Publication regressions cover force-added private paths, unreviewed scripts,
  executable files without an extension, staged secrets hidden by clean working
  copies, renamed audio, symlinks and leaks deleted by a later outgoing commit.

Versioned commit/push hooks are enabled locally; CI also checks publication
boundaries. Fresh clones need the documented [hook setup](../../development/PUBLICATION.md).
The script list contains maintained tools and synthetic tests. One-off session
runners, recordings and complete listening exports remain ignored local data.

Historical media auditions, new full-song rendering, benchmarks, playback and
physical hardware checks were intentionally skipped for publication. Prior media
evidence is retained in its owning documents; this release check does not renew
listener acceptance, PA validation or a true-peak claim. Remote CI is separate from
these local results and must be checked for the pushed revision.

## Original foundation evidence

Local host: AArch64 Linux; rustc 1.97.1 (8bab26f4f), LLVM 22.1.6.

Passed for v0.1.0:

- Formatting, locked Cargo check and warning-denied Clippy across all targets.
- Complete normal suite: four synthetic integration/CLI regression tests.
- Locked release build.
- Source archive SHA-256 matched the existing SHR Lux provenance before local extraction.
- Release CLI inventoried all 13 original Complainiacs WAVs: 15 audio channels,
  24-bit PCM, 44.1 kHz. Inventory is local and excluded from publication.
- Local Markdown link targets and publication boundary checks.

The WAV inventory reads headers; it does not validate the complete sample payload or
prove alignment. Archive extraction checked ZIP member integrity while reading.

No historical/exhaustive tests exist yet. Live audio, hardware, listening, long-running
benchmarks and automixing acceptance were intentionally outside this foundation pass.
GitHub Actions separately records the hosted Linux checks for the published commit.

## First offline automixer — 2026-10-01

Passed on the same pinned Rust toolchain:

- Complete normal suite: 14 synthetic DSP, file, schema, persistence, alignment and
  CLI tests. Calibration prefix equality checks causality; linked stereo tests check
  image preservation; output checks enforce finite samples and sample-peak headroom.
- Formatting, locked check, warning-denied Clippy across all targets and release build.
- Explicit opt-in local source variations: low/high levels, silence, pauses, jumps and
  clipped material. Trim stays bounded, silence does not raise trim, clipping is reported.
- Complete native-rate band soundcheck with an explicit stop, then a frozen render.
- Exported full-song listening files remeasured for matched loudness, length and
  headroom. All 13 original WAVs compared byte-for-byte by hash against the archive.

Private-media tests remain ignored in the normal suite and were run explicitly for
this change. [On-demand commands](../workflows/AUTOMIX.md#tests-and-limits) are documented. No
historical/exhaustive benchmark suite was needed. Detailed measurements, source hashes,
prepared settings and audio are ignored local artifacts, not publication content.
Listening, hardware acceptance, true-peak validation and live-device tests remain
unperformed; these tests do not establish musical quality or acoustic safety.

## Effects and deterministic review pass — 2026-10-01

- Complete normal suite passed: 23 synthetic tests, including oversampled excitation,
  distinct reverb decay, vocal predelay constraints, deterministic wet routing,
  maximizer linking/bounds and the full automated review/save/compare workflow.
- The spectral policy accepts sustained synthetic buildup and rejects silence,
  smooth noise, brief peaks and protected bass fundamentals. Corrections are bounded.
- Formatting, locked check, warning-denied Clippy and release build passed.
- Explicit full-band `finish` experiment completed. Its Rust code selected and logged
  spectral/return corrections, then rendered and checked the result. No plot-derived
  correction was supplied to that algorithm. Detailed findings remain local.
- The opt-in FX export check passed for timeline, equal loudness and clipping/headroom.
- Rendering the original settings with effects disabled reproduced all four first-pass
  WAVs byte-for-byte, verified locally by SHA-256.

The normal suite ignores all three private-media tests. Only the new FX export check
was explicitly invoked in this pass. Historical source variations and the old export
check were intentionally skipped; a direct legacy equivalence check covered the
changed renderer. No playback, hardware or true-peak acceptance was performed.

```sh
cargo test --locked --release --test private_automix fx_pass_listening_files -- --ignored --nocapture
```

See [FX_PASS.md](../../guides/FX_PASS.md) for reproducing the automatic review and local comparison.

## Unity-source revision — 2026-10-01

- All 28 normal synthetic tests passed, including five new unity routing, shared
  peak export, measured compression, partial-window envelope and workflow tests.
- Formatting, warning-denied Clippy and locked release build passed.
- Full local DI-only band pass rendered twice to fresh destinations. Code checked
  envelope and spectral measurements; maximizer reduction remained about 2 dB.
- Local PCM export verification checks finite samples, length, sample headroom and
  identical measured export gain against the preserved float sums.
- Three older private-media tests were intentionally skipped: their historical
  matched comparisons/source variations do not describe this new workflow.

Measurements do not establish listening acceptance. See [UNITY_PASS.md](../workflows/UNITY_PASS.md).

## Independent final output level correction

Unmatched exports now use their own measured sample peaks, with a −0.01 dBFS
ceiling in the unity workflow. The regression deliberately lowers the processed
bus by 6 dB and verifies that final export compensates it independently of raw.
All 29 normal tests passed (including the focused silence regression), with Clippy; old private matching
auditions remain intentionally skipped. A fresh full-song render checks both PCM
peaks and finite output. Raw float samples are identical to the preceding pass;
processed differs by at most 1.5e-8 after JSON settings reload (floating rounding).

## Optional live master EQ candidate

Normal admission/fallback tests: `cargo test --locked --test master_eq`.
Authenticated endpoint regressions are in the hardware-host normal library suite.
Bounded actual owner opt-ins require explicitly hash-verified GP_EQ_MANIFEST,
GP_EQ_OLD_MANIFEST, GP_PA_V2_FIXTURES and private GP_EQ_CORPUS:

```sh
cargo test --locked --features hardware-host --test master_eq -- --include-ignored --test-threads=1
cargo test --locked --features hardware-host --lib actual_new_owner_unix_remote_success_cancellation_completion_and_cached_final_ownership -- --ignored --test-threads=1
```

These selected campaigns use synthetic exact PCM24 source samples, local Unix
frames and the actual authenticated receive codec/policy/endpoint, never physical
PCM or external network load. Historical owner/corpus/physical classes remain
independent opt-ins. Follow [the contract](../../reference/MASTER_EQ_PRODUCER.md) and pin actual
source/library/header/fixture hashes before consumer freeze. Final full default
and hardware-host suites, both Clippy/release variants and publication checks
remain mandatory; focused success does not replace those gates.

### Task0018 C2 normal-gate checkpoint — 2026-10-07

At unchanged producer/corpus tip `f5e7da51628208ce456af6976d93738cfb958765`,
`cargo +1.97.1 test --locked -j1 --all-targets -- --test-threads=1` passed
411 tests across 59 binaries (17 ignored); the same command with
`--features hardware-host` passed 487 across 62 binaries (35 ignored). Both
`cargo +1.97.1 clippy --locked -j1 --all-targets` configurations, default and
hardware-host, passed with `-- -D warnings`. Both complete normal suites reported
zero failures. All used `CARGO_INCREMENTAL=0`, one build job, canonical target
and the parent-held shared build lock.

C1 owner91, owner Clippy/releases/allocation/ABI/goldens and selected actual
old/new-owner/Unix/policy-authenticated endpoint campaigns are retained with
unchanged protected-source and artifact pins. Historical/private-media/load and
physical campaigns were intentionally skipped. Release gates and coordinator
acceptance remain pending; complete software validation is not yet claimed.

### Explicit Desk provider witness

`tests/desk_operator_witness.rs` is an ignored, external-driver test harness over
actual LocalAudio and hash-verified PA/FX/REC libraries. It opens no device and
starts muted/disarmed. The fixed synthetic witness topology (17 inputs, three
monitors, six PA outputs at 48 kHz) is a test case, not a product capacity limit.
Only the first input contains a periodic, exact PCM24-quantized 1 kHz signal.
The harness grants no authority and performs no operator rearm.

Commit reviewed harness source before compiling so the executable embeds its
full source revision. With Rust 1.97.1, committed lockfile, one build job,
`CARGO_INCREMENTAL=0`, canonical target and parent-held shared build lock:

```sh
GP_DESK_WITNESS_SOURCE=<full-source-commit> cargo +1.97.1 test --locked -j1 --features hardware-host --test desk_operator_witness --no-run
GP_DESK_WITNESS_SOURCE=<full-source-commit> cargo +1.97.1 clippy --locked -j1 --features hardware-host --test desk_operator_witness -- -D warnings
```

Retain the exact test executable separately with source SHA, executable SHA-256
and command manifest. Launch that pinned binary with absolute trusted
`GP_EQ_MANIFEST`, `GP_PA_V2_FIXTURES` and `GP_DESK_WITNESS_DIR` (a new empty,
owned mode0700 directory), selecting only
`--ignored --exact serve_desk_operator_witness --nocapture --test-threads=1`.
Keep stdout/stderr outside the initial directory. Check ready provenance before
requests. Atomically publish strict `capture-request.json` containing a unique
alphanumeric/hyphen ID (at most 32 characters) and `blocks` in 1..32. Wait for
`capture-ID.json` before another request. Up to 16 captures and 180 wall seconds
are allowed. Stop only after the final response by creating an empty `stop`;
require exit 0 and `summary.json`. Preserve every failed trial and `failure.json`,
including any retained partial PCM. Completion capture remains owned until
successful publication; timeout assertions run inside the panic catcher.

For settled samples require matching epoch/revision on every block, stable
relevant state/maps and no relevant ramps. A muted one-block baseline smoke
establishes startup/provenance, silence and quiesced/disarmed state only. The
later Desk Frontend driver owns actual controls and affected/unaffected sample
assertions. This harness is not physical, acoustic, listening, deadline or full
Frontend integration acceptance; allocation evidence remains owner/narrow commit
scope and authenticated endpoint evidence does not add a TLS handshake.


### Task0018 C3 completion — 2026-10-07

The sole remaining production gate passed:
`cargo +1.97.1 build --release --locked -j1 --features hardware-host` (16m54s,
exit 0). Four hardware-feature executables and the passing C2 default executable
are preserved separately. The unchanged production code retains C2's complete
411-test default  / 487-test hardware suites (17 / 35 ignored), both all-target
warning-denied Clippy runs and default release; C1 retains PA 91, PA Clippy/releases,
PA 5 / GP 50 Python checks, C callers, owner allocation guards and selected actual
old/new-owner 3 and Unix/policy-authenticated endpoint 1 campaigns. Protected source,
consumer fixtures, twelve owner goldens and accepted historical A/B artifact pins
were rechecked. These unchanged successful campaigns were intentionally not rerun.

`cargo +1.97.1 build --release --locked -j1 --lib` in the PA owner, without
`owner-allocation-guard`, restored the canonical public target to the exact normal
library used by actual tests (SHA-256
`7bd1cfcb950961d8106dcdcaa21c2ed05abc7afe052d628e81f06fa85b087e91`). All seven
optional EQ symbols are present; the separately retained private guarded library
is unchanged. No Cargo output was overwritten manually.

The new ignored witness passed focused labeled compilation and hardware-host
Clippy with `-D warnings`; its provenance label is optional for ordinary builds
and mandatory when running the explicit witness. After preserving the labeled
executable, the same focused compile/Clippy commands with the label absent verify
normal test-build compatibility. The initial Clippy rejection and corrected source
pin/binary remain recorded. The muted smoke passed one capture (48 source frames,
11 playback channels, 528 exact-zero samples), strict source/binary/owner manifest/
library/fixture hashes, stable identities/maps/intent and quiesced/disarmed actual
owner/provider state, followed by explicit stop, summary and exit 0. One preceding
private-checker trial failed before capture and remains retained with no PCM claim.
No grant, rearm or Frontend action was performed. Capture publication ownership and
caught timeout behavior remain in the installed harness.

All new Cargo checks use Rust 1.97.1, `--locked -j1`, `CARGO_INCREMENTAL=0`,
`CARGO_BUILD_JOBS=1`, canonical targets and parent-held shared build lock. Historical,
private-media, exhaustive/load, physical audio/MIDI/DMX, listening and hardware
qualification campaigns were intentionally skipped. Full default/hardware suite
counts above refer to the unchanged producer before addition of the explicit ignored
external-driver witness; no duplicate full suite is claimed for the harness.
Final complete-index and outgoing-range publication guards cover scoped local commits.
Coordinator review/fixture freeze and later DeskD actual Frontend/sample integration
remain separate from completed producer SOFTWARE validation and muted baseline smoke.
