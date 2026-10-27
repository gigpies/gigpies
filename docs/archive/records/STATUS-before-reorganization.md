# Status and next steps

## Task0020 module integration — software acceptance

Configured [C-ANALYSIS:2](../../reference/CONFIGURED_ANALYSIS.md) binds four named raw sources to
explicit admitted topology inputs. Lux and Lightdesk consume the frozen producer
contract with source/map/lifetime and freshness checks. Actual configured-chain
software acceptance passed; legacy v1 remains separate. Physical configured
publication remains unavailable pending capture-age qualification.

The opt-in [GP20 measurement provider](../../reference/MEASUREMENT_PRODUCER.md) borrows bounded
raw reference/reserved microphone-slot samples from the existing source tick.
The hash-pinned SHR PA owner performs analysis, multi-position proposals and
exact candidate validation off the render path. Desk's actual software episode
passed capture/cancellation, proposal, unchanged-result and stale-basis handling,
and the existing muted GP14 whole-configuration review. Rearm remains separate.

[GP21 remote FX control](../../reference/REMOTE_FX_CONTROL.md) reaches one actual Brain stereo
owner through prepare, permit, applied and settled observations. Dedicated
authority and exact source/lifetime/ticket identity protect atomic stereo edits,
tail-preserving bypass and selected/both history reset. Actual Desk controls,
old-owner fallback and post-permit owner-loss episodes passed. Independent sample
checks cover selected wet response, PA composition and unchanged raw REC.

Combined normal suites cover 431 default and 513 hardware-host tests. A repaired
panic-fixture scheduling test passed separately under both features; all other
normal tests passed on identical production inputs. Both all-target warning-denied
Clippy configurations, both release builds, Python and index guards passed. Seven
selected actual-owner integration cases passed on Pi4, including exact PA dry+wet
composition, 16 unchanged raw recording tracks, two-position measurement and the
PA replacement allocation boundary. Private exact synchronization and its receiving
receipt are recorded in the batch ledger. Historical campaigns remain opt-in.
No physical playback, microphone capture, MIDI/DMX output, acoustic alignment or
hardware timing qualification is claimed.

The October 12 hygiene pass removed superseded task0020 build-only and
panic-fixture repair bundles, plus Desk's pre-correction trial directory. The
accepted behavior and gate counts above, final corrected Desk episodes and
private integration receipts remain. No hardware evidence was removed.

## Live master EQ — task0018 software validation

The optional owner extension and GP18-master-eq:1 provider path are implemented
and independently reviewed. Narrow stereo admission, independent lifetime/
generation/map pins, retained metadata, shared authority/reservation and disarmed
recovery are described in [the producer contract](../../reference/MASTER_EQ_PRODUCER.md).
Actual old-library fallback and new-owner synthetic integration passed. Complete
normal suites passed (411 default / 487 hardware-host), and both all-target Clippy
configurations passed with warnings denied on the unchanged producing source.
Both release variants passed. The normal PA library was restored through Cargo
and matches the artifact used by actual producer tests. The ignored actual-provider
Desk witness compiled with warning-denied focused Clippy; its muted baseline
captured 528 exact-zero playback samples with quiesced/disarmed actual owner state.
Producer review and contract freeze are complete at the accepted checkpoint
`42e038a80373fd9317692d5da770b7079d0205d0`, paired with SHR PA
`85ed759499b5223e30a53e2dbea1e6fec03f04f9`. The later witness-only correction
`eeb62520f3f8e3cf36d2e490da143a93364886c8` moves evidence publication off the
provider loop; its two focused regressions, Clippy and muted actual-owner smoke
passed without changing production code or fixtures. Desk records its complete
Frontend, rendering and actual-provider acceptance independently of these producer
gates.
No physical audio, listening or measured deadline acceptance is claimed.

## Monitor/aux tap routing — task0018 producer

Configured engines implement independent raw post-mute, processed pre-fader and
processed post-fader mono sends through GP18-sends:1, with shared monitor authority,
revision/dedup, bounded 240-frame transitions, committed-only version2 intent and
disarmed recovery. Configured processing explicitly advances to GP07:4; legacy
8/2 processing2 and raw monitor bytes remain unchanged. [Contract and validation](../../reference/MONITOR_SENDS.md).
Producer review and corpus freeze are complete. The corrective pass repaired
synthetic-source block reuse without changing source bytes or deadlines,
added mixed transport ownership, nondefault composed restore, oversized paging and
strict transition validation. Final normal validation passed 409 default and 484
hardware-host tests, with 17/33 explicit opt-ins skipped. Three separately selected
actual-owner send/Brain/provider tests passed against hash-verified PA/FX/REC
artifacts and synthetic samples. Both release variants, warning-denied Clippy
configurations, formatting, 50 Python tests and publication checks passed.
Historical corpus provenance is retained; the separate corrected corpus pins its
committed producing source. Both corpora retain their original hashes and pass
explicit decoder/source-hash verification. Exact commands, counts and private
evidence are recorded in the bounded handoff. Desk integration uses the reviewed
corrected corpus; historical producing revisions and bytes remain unchanged.
No physical mapping, listening, audio-clock lock or sustained hardware deadline claim.

## Control-capacity and CI review fixes — 2026-10-06

Shared authority now admits 16 live writers under a named deployment resource bound,
matching the transport session budget. The six documented controller scopes can
coexist; focused coverage fills all slots, refuses the next grant without eviction,
maintains existing leases and recovers capacity after expiry. This is offline
validation, not a new network-load or physical acceptance campaign.

CI now checks default and hardware-host builds, normal tests, Clippy and release
compilation. Hardware-host includes authenticated permission/replay/lease-expiry,
held-proof and committed paired-readback regressions; physical activation and
external owner-artifact campaigns remain opt-in. See [transport](../../reference/REMOTE_TRANSPORT.md)
and [development](../../development/DEVELOPMENT.md).

Local offline validation: **391 default / 466 hardware-host tests passed**,
with 15/30 opt-ins intentionally skipped. Both warning-denied Clippy
configurations, formatting and 39 Python tests passed. Reproduce with
`CARGO_INCREMENTAL=0 cargo test --locked -j1 --all-targets`, then the same command
with `--features hardware-host`, under the shared build lock. Historical, physical,
external-artifact and explicit rendering campaigns were not run. No publication
or physical/native-window activation was performed.

## Brain local duplex audio — software validated, 2026-10-06

Brain owns one duplex sound card for talkback and operator listening, crossing a
separate clock domain through two bounded ASRC paths. Stagebox remains the DSP,
PA, raw recording and source-clock FX/analysis reference. Device/routing/authority,
authenticated media and actual Desk controls are implemented.

Independent contract, combined source and artifact reviews passed. Producer gates
passed 390 default/465 hardware-host tests, four actual-owner tests, both Clippy
configurations, releases and 39 Python tests. Desk passed 240 default/242 native
tests, explicit CPU-headless rendering, both Clippy configurations, release driver/
applications and nine Python tests. Normal opt-ins were intentionally excluded.

Trials 37–43 passed all seven scenarios on the same frozen candidate: simultaneous
16/32/48 inputs, duplex stall/restart, controller stall and Stagebox restart.
Independent checks cover actual audio/overlap, exact raw samples, safe closure,
continued dry/PA/FX/REC where applicable and fresh unarmed recovery. Earlier passes
and failures remain separate evidence; no freshness or safety limit was relaxed.

Desk's 12 and Lightdesk's 16 development-PC screen references were integrated and
inspected. CI was added to the eight canonical repositories that lacked it, and
all eight hosted workflows completed successfully. Source publication follows
provider before consumer; exact publication/CI and twelve-repository receiving
receipts belong to the private task ledger. No physical PCM was opened. See
[acceptance](../../acceptance/BRAIN_AUDIO_ACCEPTANCE.md) for measurements, retained failures and
separate hardware gates.

## Configurable modular processing — 2026-10-05

The shared Mixer/Authority now admits configured strip and monitor counts, explicit
transport/socket/signal maps, larger versioned readback, durable source recovery
and separately scoped prepared PA/output transactions. Actual owner-native PA v2,
raw REC, Brain FX and authenticated QUIC media use the same LocalAudio source path.

Software and actual two-Pi acceptance passed for 16, 32 and 48 inputs, plus a 17-input
nonfixture regression. Desk performed real PA configuration/weighted routing,
physical-model patch changes and all 24 EQ/compressor controls on inputs 16, 17, 32,
33 and 48. Independent sample checks covered every REC/analysis input, PA gain/phase
relationships, silent unassigned outputs and dry/protected output through Brain
loss/restart. CPU-headless native presentation and complete production suites pass.
The [acceptance matrix](../../acceptance/MODULAR_ENGINE_ACCEPTANCE.md) records exact scope, failures,
validation classes and measured resource observations.

The 16-input/18-output reference starts unpatched and models the noncontiguous
manufacturer USB layout. No UMC1820/ADA8200 was attached or activated. Actual socket
mapping, hardware monitoring, shared ADAT clock/lock and hardware deadlines remain
separate physical acceptance. Neither profile size nor the UI bank is a product cap.
See [composition](../../reference/MODULAR_PROCESSING.md) and [authenticated protocol](../../reference/REMOTE_TRANSPORT.md).

The dated milestones below retain their original narrower scopes and evidence.

## Channel processing — 2026-10-05

The first GP-07 slice adds [FOH channel EQ and dynamics](../../reference/CHANNEL_PROCESSING.md)
on all eight mono inputs. Four independent parametric EQ bands and a compressor reuse the existing
GigPies DSP. Neutral/bypassed defaults preserve the established mixer. Edits
prepare off the audio path, apply at a strict 48-frame boundary and crossfade for
240 frames at 48 kHz, with explicit makeup gain and no lookahead.

The versioned processing envelope shares existing FOH authority, revision and
retry history. Desk edits a complete local draft, presents a protected review,
and reports provider-confirmed current/target values, readiness and detector gain
reduction. Keyboard and injected controller actions reach the same authority.
A delayed read-only observation can recover without replaying a mutation.

Raw REC and named analysis stay before processing. FOH is EQ → compressor → shared
mute → fader/pan → sum → fixed FX wet/dry → PA. Monitor sends retain their raw,
post-mute source. Meters and physical sound quality remain unverified. Task0013 actual Desk/provider/sample and CPU native layout validation passed;
the earlier v1 acceptance is
preserved in the archived contract. This slice does not complete GP-07 scenes, PFL,
routing/channel expansion or writable PA/FX.

## Integrated software milestone — 2026-10-04

GigPies now provides descriptor-bound process-held roles (GP-09), bounded named
PCM analysis (GP-04), and an optional graph using the actual SHR REC, FX and PA
libraries (GP-05). Recording preserves all eight raw input streams; the stereo
FOH sum passes through fixed wet/dry FX and the owner's logical PA main outputs.
Desk displays the provider's recorder, FX and PA health read-only (DS-05).
The unchanged monitor paths and control authority remain separate.

Desk and Lightdesk have optional native frontends over their real provider clients,
including resize/recovery, bounded input/output, complete protected reviews and
explicit role leases. Native rendering is checked offscreen with a CPU Vulkan
adapter. Real displays, controller enumeration, MIDI and LED output are unverified.

Lux consumes actual named PCM through its existing analysis and show policy.
Calibration and bounded intensity AUTO grants require explicit authority; human
programmer/Hold values win. Source loss, stale acquisition timestamps, identity
changes and expired grants freeze the last contribution without automatic rearming.
Lightdesk decodes this opt-in schema while keeping Lux responsible for arbitration.

REC-01/02 expose correlated lifecycle and retained, non-owning progress observation.
FX-01 and PA-01 expose exact versioned capabilities/status with their existing DSP.
Configurable FX racks, writable PA controls, acoustic measurements, true-peak protection
and wider graph expansion remain unavailable. The PA integration applies its
sample limiter to logical main outputs only; it establishes no physical protection.

The prior GP-01/02/03, private GP-06 subset, LX-01..04 and console clients remain
in place. Production remote authentication, physical I/O, combined-load/scheduler
qualification and optional instruments are subsequent increments. No version bump,
tag, binary release or deployment accompanies this source milestone.

See the [current implementation map](../plans/MODULE_IMPLEMENTATION_MAP.md#execution-checkpoint--2026-10-04)
and [local integration instructions](../workflows/HEADLESS_INTEGRATION.md) for accepted checks,
reproduction and remaining gates. Dated entries below retain their original scope.

## Previous local-engine checkpoint — 2026-10-04

The substantial software continuation now includes GP-01/02/03 and the private
local subset of GP-06 in GigPies, LX-01/02/03/04 in Lux, and real provider clients
in Desk and Lightdesk. All four normal suites and actual cross-process checks
passed, including continued audio control while Lux restarts and restores its
checkpoint disarmed. Reviewed source is saved in all four owning repositories.
Follow the
[current implementation map](../plans/MODULE_IMPLEMENTATION_MAP.md#execution-checkpoint--2026-10-04)
and [explicit local session instructions](../workflows/HEADLESS_INTEGRATION.md).

Audio has eight mono inputs, stereo FOH, two post-mute pre-fader monitor sends,
48-frame command boundaries and 240-frame coefficient ramps. Lux owns actual
programmer/Hold/cue/playback state, timed release and durable disarmed recovery.
These paths use synthetic audio and null lighting output. Native display/controller
binding, PA/FX/REC binding into this graph, production remote authentication and
physical/combined-load acceptance remain open. No version bump or public release
is part of this work. Dated entries below preserve their original evidence.

## 0.2.3 — dual-console integration checkpoint, 2026-10-04

The complete GigPies product includes **audio and lighting digital operator
surfaces** on one Brain: two 1920×1080 monitors and two separately assigned MIDI
keyboard controllers. SHR Desk owns audio UI, SHR Lightdesk lighting UI, and SHR
Lux lighting execution/output. Automation is one mode within each console.

This version brings the previously local summing/delivery, GPA1 transport and
qualified stereo USB host work into the publication checkpoint. The new hero and
system map show both console pairings, Stagebox-local recording and the separate
lighting engine. Both surface implementations remain independent sibling projects;
they are not bundled live applications in GigPies.

Native windows/controller integration, full mixer/Lux contracts, physical lighting
and combined load acceptance remain open. Read [Brain integration](../../architecture/BRAIN_CONSOLES.md),
[current next steps](../plans/NEXT_SESSION.md), [0.2.3 changes](../../../CHANGELOG.md) and
[publication validation](VALIDATION.md). The dated entries below preserve their
original scope; “unreleased” in those historical records describes that session,
not the current publication state. No new hardware acceptance was performed here.

## Dual-console Brain / SHR Lightdesk — offline foundation, 2026-10-04

GigPies explicitly includes human-operated audio and lighting consoles on one Brain,
with two 1920×1080 monitors and two separately assigned MIDI keyboard controllers.
SHR Desk owns audio UI; the new `../shr-lightdesk` owns lighting UI; SHR Lux owns
the lighting engine. Automation is one mode within each console.

Lightdesk now has a source-linked MA/MagicQ/Titan/Eos/ONYX screen/workflow study,
blueprint, screen map, controller plan and capability/integration backlog. Its
independent Rust offline loop implements synthetic fixture selection, programmer
holds, static look storage/playback, bounded automation proposals, explicit release,
request/recovery states and seven state-driven Full-HD drafts with the existing
licensed font. Its authority is a mock, not a completed Lux engine.

Native rendering, full controller navigation, real engine contracts, cue timing,
physical lighting and combined hardware/load acceptance remain planned. Exact
validation and next steps live in [Lightdesk status](../../../../shr-lightdesk/docs/STATUS.md);
[the Brain integration plan](../../architecture/BRAIN_CONSOLES.md) owns cross-module work. Existing
sibling projects remained read-only. No device output, host configuration, shared
load test, publication or deployment occurred. The older audio-desk checkpoint
below retains its historical scope.

## SHR Desk module — offline foundation, 2026-10-04

The requested Brain control surface now has its own `../shr-desk` project.
Its blueprint, console-reference study, screen map and MiniLab plan distinguish
existing module capabilities from missing live interfaces and optional scope.
An independent Rust simulator implements channel selection, command/ACK state,
manual fader holds, Auto/Assist/Manual transitions, MIDI input primitives and
three state-driven 1920×1080 SVG screen drafts with the existing Terminus font.
The [integration plan](../../architecture/BRAIN_CONSOLES.md) links the owning documents.

This is implemented/offline-validated surface work, not a native GPU window or
live mixing console. Controller/HDMI acceptance and real engine adapters remain
planned. No audio, MIDI, DMX, service or host-font state changed. GigPies remains
the final product and live integration owner; module DSP remains with its owners.
The October 3 console checkpoint below is historical.

## USB host integration — unreleased, actual-device bench, 2026-10-03

The [hardware plan](../plans/AUDIO_HARDWARE_PLAN.md) and [measured host contract](../../acceptance/AUDIO_HARDWARE.md)
connect selected stereo USB capture/output with independently built SHR PA, FX
and REC libraries. PA and the integrated delay use f64 DSP; Brain follows USB
source frames. A bounded independent worker writes sample-verified PCM24 stems.

The low-latency host processes 48-frame / 1 ms blocks at 48 kHz, with independent
192-frame ALSA capacity, zero silent prefill and no extra application playback
queue. The working channel measured 249 frames / **5.1875 ms** in short tests.
A 144-frame-capacity comparison settled at 257 frames, so less capacity did not
reduce measured latency. The old 57.0625 ms result included 56 ms of silent
prefill; its ten-minute soak remains historical correctness evidence.

Earlier low-latency attempts failed. A focused trace caught a 6.219611 ms wait for
kernel page migration despite memory locking. H7 compared the same host with
locked-page compaction temporarily disabled and process memory locked, restoring
both after each trial. Packet/Brain stalls, Brain restart, explicit incomplete
finalization after a device xrun and fresh recovery passed with exact stored
samples. The ten-minute H7 run passed USB/recording/physical checks but missed two
4 ms wet deadlines. H8 then passed ten minutes at 6 ms wet admission with zero
xruns/losses/gaps and exact combined-output replay. Physical delay was 5.19–5.23 ms, with two
one-frame changes still unresolved; fixed physical timing is not certified.

The right return is about 69 dB weaker; physical measurements cover channel 1.
No second USB interface is present. Converter-only latency, clock lock, acoustics,
full mixer controls and full-show reliability remain unverified. Earlier console
edits are preserved separately; no public push or release occurred.

## Audio transport — unreleased, synthetic validation, 2026-10-03

Implemented [GPA1 audio and bounded UDP control](../../reference/AUDIO_TRANSPORT.md), fixed
48 kHz source-frame identity, PCM24/float32 codecs, bounded queues, deadline
admission, wet fade and fresh-state recovery. Untuned two-Pi link measurements,
paced bidirectional trials and application faults are recorded in the
[owning plan](../plans/AUDIO_TRANSPORT_PLAN.md). The final measured envelope and failed
targets are stated in the transport document. SSH/Git remains development
coordination. That preceding phase did not implement physical I/O or real modules; the
hardware continuation above records the later stereo integration. Full-show
reliability remains unverified.

## Historical single-console Brain plan — 2026-10-03

The [then-current console draft](../brain-console-before-shr-desk-2026-10-04.md) fixed Brain's display at
1920 × 1080 over HDMI; the small display belongs to PA. It proposes native GPU
rendering with a TUI-style layout, custom glyphs, spectrogram/stereo panels and
MIDI control. The backlog starts with synthetic state and an offline console,
then adds live contracts and module integration. No console code is implemented.

The planned node split now puts the mixer, protection and local NVMe recorder on
PA, with the console, analysis/doctor, lighting and richer FX on Brain. Performance
figures are targets; live and hardware acceptance remain pending. Updated peer
instructions and the installed runner were inspected locally; no peer task or
hardware check was launched. This checkpoint changes documentation only.

## Summing and delivery — unreleased, offline-validated, 2026-10-03

The [summing plan](../plans/SUMMING_PLAN.md) has passed its engineering phases. Independent routing and
arithmetic checks retain the existing `f64` summer. The new [delivery sidecar](../../guides/SUMMING_DELIVERY.md)
separates final limiting, static gain, peak basis and optional comparison copies.
It adds a validated streaming true-peak estimator, final PCM checks, production
stage/peak observations and explicit legacy replay. Frozen session identities remain.

All six complete scalar replays match the retained processing reports, including
zero reduction from both master limiters. 154 normal Rust tests and 37 Python tests
pass, alongside formatting, Clippy and the locked release build. The independent
meter study passes 300 synthetic cases; worst reference difference is 0.106964 dB.
No new musical correction is supported by this study. Existing channel settings,
source timing, stereo and requested GigPies effects are retained. PA alignment
remains in SHR PA; listening and hardware acceptance remain separate.

All six historical SOURCE/FINAL comparisons reproduce the retired PCM and float
hashes. Six new complete −1 dBTP deliveries pass production and independent meters,
using static finalization with 0.4001 dB reserved margin and no loudness target.
The current local listening index is
`artifacts/automix/summing-study/2026-10-03-engine/LISTEN.md`; it provides full mixes,
seven exact excerpts and the reconstructed historical references. Playback requires
a fresh go. Listener preference is not reviewed; hardware remains unverified.

The earlier user-requested render retirement remains recorded under
`artifacts/automix/render-retirement-2026-10-03/`. All 2,473 retained evidence files
and metadata for 428 original/library/test files remain unchanged. Previous listening
indexes are historical. Cleanup and reproducible evidence are recorded beside the
new index in `REPORT.md`, `cleanup.json` and `complete.json`.

## 0.2.2 — FX calibration and review correctness, offline-validated

Artistic FX plans now separate ensemble eligibility from individual decay and
return targets. Human and JSON reports expose signed residuals, engine/gain bounds,
amount-adjusted targets and actual per-passage bus outcomes. Missing evidence stays
unmeasured. The observer includes the real master stage and requires zero master
reduction. Complete measured-window coverage is checked explicitly.

New plans pin settings, policy and source hashes, publish `ready.json` last, and
retain evidence on failure or interruption. `ambience-check` verifies saved artifacts
and current source bytes. `ambience-audit` reads saved scalar evidence without audio.
It reproduces the known limits in the six current plans, including Dark Ride's two
minimum-decay limits and +4.023632 dB vocal-echo residual. See
[individual reports, recovery and historical audits](../../guides/ARTISTIC_FX.md#individual-target-reports-022).

Continuous DSP can carry earlier held-out audio into later training. Fixed synthetic
probes demonstrated this in both the FX calibration and existing channel EQ paths.
New FX and EQ plans therefore require chronological splits. Saved EQ states retain
their frozen identity and exact reset; review pages disclose historical interleaving.
The fitter, recipes, gain bounds and EQ tolerances are unchanged. The EQ comparison
also gains `COMPARISON.md` and a saved-report summary command, separating shape,
levels, compression/FX consequences and missing evidence.

These are offline implementation and verification results. Original recordings and
frozen SOURCE/FINAL settings remain preserved. The user subsequently retired all
generated audio; see the next step above. Individual
artistic targets, independent-reference comparability, listener preference and
hardware acceptance remain separate open questions. No new real-audio candidate,
render or playback was made. The session plan, synthetic probes and readable saved
audits are retained in `artifacts/automix/fx-calibration-review-2026-10-03/`.

145 normal Rust tests and 34 Python tests pass; three historical private-media tests
remain opt-in. Formatting, Clippy and the locked release build pass. Release CLI
plan/check/audit passed on generated sources. Before retirement, hash checks matched 94 comparator/diagnostic artifacts,
39 routed Dark Ride originals and 62 saved reports. Their recorded identities remain.

Filesystem checks still report zero current ext4 error/warning counters and no
SMART media errors. The crash cause remains unresolved; boot recovery is not a
forced offline scan. These changes are included in 0.2.2.

## 0.2.2 — reference comparability evidence

The new `eq-match-compare` command separates known EQ changes on identical recordings
from phrase dispersion and reports compressor/FX/ensemble consequences. It supplies
no target or fitting tolerance. A predeclared fresh-phrase Dark Ride study produces
a bounded partial correction of deliberately injected coloration; its clean control
adds no EQ. All existing guards pass, with no fitter change or held-out retry.
See [method, results and limits](../../guides/EQ_MATCHING.md#fresh-reference-study-after-021--2026-10-03).

At that checkpoint, 130 normal Rust tests and 34 Python tests passed; three historical media tests remain
opt-in. At that time SOURCEs, six complete FINALs and the seven-pair listening queue
were preserved. On the user's subsequent render request, two complete Dark Ride diagnostic
mixes were exported from the frozen coloured/corrected settings, with unchanged FX.
Both passed full export verification at −0.01 dBFS sample peak; their export gains are
−3.720112 and −3.341220 dB. The historical index remains in
`artifacts/automix/eq-audition-2026-10-03/LISTEN.md`; audio files were later retired.
The clean production and diagnostic settings remain separate. Musical acceptance is
pending. Playback needs a fresh user “go.” The Pi crash interrupted only a later
diagnostic observation; boot recovery and retained hashes were checked
before continuing with frozen inputs. Its cause remains unresolved.

The [FX parameter audit](../studies/ARTISTIC_FX.md#parameter-audit--2026-10-03) records how the
saved recipes and bounded calibration chose the effects, including unreachable
individual decay/wet-level targets. No FX setting was changed for this audit.

## 0.2.1 — frozen EQ matching, offline-validated

Versioned measured maps and provisional artistic directions now feed a deterministic
broad-filter matcher using production EQ response. Saved baseline/source identities,
0–100% amount, exact reset, capacity refusal, CLI commands and a local HTML review
page are implemented. Existing EQ, effects, input groups, trims and routing are retained.
See [algorithm, research, controls and evidence](../../guides/EQ_MATCHING.md).

126 normal Rust and 34 Python tests pass. Synthetic injected coloration receives a
bounded correction. The deliberately coloured Dark Ride pilot abstains because its
reference uncertainty masks the known change; this conservative miss is retained,
with no retuning or new audition. All six current complete productions remain unchanged;
five focused regressions and Dark Ride's clean control match retained production
observations. No listening improvement or hardware readiness is claimed.

See [release validation](VALIDATION.md#021-publication-checks--2026-10-03) and
[release notes](../../../CHANGELOG.md). Publication hooks and CI enforce private-data
and reviewed-script boundaries.

## 0.2.0 — previous offline release

The previous release introduced the source-preserving decision workflow, contextual
analysis, explicit artistic FX and verified listening tools described below.
Its local release checks passed: 117 Rust tests, 34 Python tests, formatting, Clippy
and the locked release build. See [0.2.0 validation](VALIDATION.md#020-publication-checks--2026-10-03).
Musical and hardware acceptance remain separate.

## Next hardware session

Two-Pi baseline and synthetic transport work are documented in
[AUDIO_TRANSPORT.md](../../reference/AUDIO_TRANSPORT.md). Next integrate the owning PA/FX/REC
modules, then obtain session authorization for muted 48 kHz/24-bit capture,
clock/channel verification, same-clock loopback latency and real NVMe
continuity through a Brain process restart. MIDI/HDMI acceptance remains
separate; see [next-session notes](../plans/NEXT_SESSION.md).

## PA module ownership and planned integration

The user confirmed the modular direction on 2026-10-02: develop PA processing,
measurement and alignment in SHR PA, then integrate its finished module here.
The new reference/mic phase-measurement and delay/polarity task is recorded in
`../shr-pa/docs/PHASE_ALIGNMENT.md` and its P4/P5 roadmap. See
[module ownership and integration](../../architecture/COMPONENTS.md#pa-module-and-planned-integration).
Measurement, automatic alignment, GigPies integration and acoustic acceptance remain
pending. Only documentation changed; audio code, host settings and mixes are untouched.

## Current artistic requirement — finished mixes with GigPies effects

The listener clarified that full mixes include expert-selected GigPies effects.
The previous checkpoint's full-duration exports had no GigPies FX and did not meet
that artistic requirement. Source preservation remains the channel-tone baseline.
The new [artistic FX pass](../../guides/ARTISTIC_FX.md) selects effects from explicit instrument
and style profiles, calibrates our production engines, and freezes settings before
held-out verification. All six complete renders and the exact-clip checkpoint are
verified in `artifacts/automix/expert-fx-v1/LISTEN.md`. Supplied FX-return tracks remain excluded;
no rendered reference mix supplies effects. Listener acceptance is pending.
117 Rust and 30 Python tests pass, plus Clippy, formatting and the locked release
build. Source hashes, direct-path retention, full peaks and generated tails pass.

## Current offline decision model — source preservation

Dark Ride now selects the unchanged SOURCE as FINAL. The bounded two-proposal
reassessment rejects the HPF-only control for lack of a demonstrated benefit and
retains the other five mixes. Complete production exports and the verified six-example
checkpoint are indexed in `artifacts/automix/source-preservation-v1/LISTEN.md`.
Listener acceptance remains separate and pending.

Offline reviews admit explicit HPF choices including bypass. Missing tone/balance
intent and default master/FX review preserve settings; `preserve-source` prepares
an unchanged ensemble without automatic role processing. Bass/drum measurement
uses the configured master filter. Prior numerical protection budgets are retained.
See [decision failures, contract revision and evidence](../../guides/SOURCE_PRESERVATION.md).

## Planned musician review after soundcheck

The existing QR station connections could serve each musician a review of their
own instrument: keep the sound or submit preferences, mark ready, then receive a
shared mix preview once everyone is ready. The first version uses structured controls
and deterministic rules; optional small-model interpretation remains research.
Phone preview, personal monitors and applying changes to the live mix have distinct
scopes. See [the recorded proposal and validation needs](../../architecture/PERFORMER_REVIEW.md).
No network, performer UI, model runtime or live application was implemented here.

The sections below record earlier experiments and their original requirements.

## Listening follow-up — Dark Ride source audit

After authorized playback, the listener reported good progress overall but preferred
Dark Ride SOURCE, saying FINAL lost power. A source audit confirms authentic archive
files and producer-described sampled drums/designed tones; “raw” does not guarantee
untreated capture. Production-stage analysis reproduces both stored float buses
exactly and identifies strong kick compression, guitar EQ/makeup, and greater export
attenuation with reduced bass/kit support. No mix or engine change was made in this
audit. Dark Ride FINAL needs reassessment; the other five have no newly reported
objection. See [evidence and limits](../studies/MULTITRACKS.md#dark-ride-source-and-processing-audit--2026-10-02).

## Previous checkpoint — independent Complainiacs reassessment

The latest snare observation was investigated from the revised FINAL, whose extra
+2 dB drum step was already removed. A frozen pass selects snare makeup
+3.5 → +1.906326 dB at unchanged faders; a broad-EQ alternative fails protection.
The complete six-example SOURCE → selected FINAL set is verified, with a second
Complainiacs ending passage. Other examples retain their mixes. Listener acceptance
is pending; earlier numerical mixing preferences are withdrawn.

Implemented repairs separate unknown musical balance from fixed-fader processing
verification, expose coherent snare/ensemble contributions, pin historical SOURCE
identity and label pre-export loudness. 105 normal Rust and 29 Python tests pass;
five-example regressions and full export/clip verification pass. Superseded processed
audio is retired after verification, preserving raw/SOURCE mixes and decision evidence.
No playback, host-audio changes, commit or push. See
[current findings, selection and limits](../studies/COMPLAINIACS_WORKFLOW_REVIEW.md).

The sections below retain the history of earlier passes and their original intent.

## v0.1.0 — first public foundation

Implemented:

- Standalone Rust library and CLI; pinned toolchain and locked dependencies.
- Read-only WAV header inventory for a file or a flat directory, with JSON output.
- Synthetic regression tests and Linux CI; no hardware required.
- Architecture, source-material notes, dependency map and preserved concept sources.

## First offline automixer — implemented, offline-validated

- Synchronized native-rate streaming soundcheck with bounded activity-aware trim,
  linked calibration groups, causal history and explicit freeze/save.
- Editable role presets: HPF, parametric EQ, stereo-linked compression, pan/faders.
- BWF time-reference alignment, zero-padded tails and conservative master peak control.
- Frozen-settings rendering, full-song local A/B WAVs and loudness-matched copies.
- Measurements and gain/reduction histories; input clipping is reported, not repaired.
- Synthetic DSP/routing/persistence tests; opt-in deterministic private-media variations.

The first full-band experiment has been rendered and its file/measurement contracts
checked. Listening is the next evaluation; tests do not establish musical quality.
See [workflow, presets and limitations](../workflows/AUTOMIX.md).

Not implemented: live audio or device transport, networking, web/TUI/controller,
recording, lighting integration, gates or true-peak limiting.
No live latency, acoustic safety, listening acceptance or Pi headroom is claimed.

Next: review the new manufacturer-reference SOURCE → OUR MIX comparisons and record
listener preference separately from policy compliance. Earlier rejected candidates
remain preserved as historical evidence. Hardware integration remains separate.

The [archived blueprint](../blueprints/blueprint-v2.md) retains the wider scope.

## Optional effects and automatic review pass

Implemented: separate plate/chamber/hall reverbs, vocal predelay validation, chorus,
filtered delay, oversampled excitation and modest master maximization. The Rust
`finish` command analyzes the preliminary mix, applies bounded spectral/return/dynamics
rules, rerenders and records its decisions and post-checks. Corrections are made by
code; no spectrogram interpretation or AI decision is in that loop.
[Commands, algorithms and limitations](../../guides/FX_PASS.md). Listening acceptance remains pending.

## Unity-source revision

Implemented and offline-validated: DI-only bass, unity input trims and faders,
90 Hz channel HPFs except kick/bass, 40 Hz processed master HPF, measured compressor
thresholds and loss compensation, independent final PCM output leveling to −0.01 dBFS, preserved
float sums, and code-generated envelope/spectral reports. No loudness matching in
this workflow. [Commands and limits](../workflows/UNITY_PASS.md). Listening acceptance remains open.

## Musical balance pass

Implemented and offline-validated: synchronized role-band and event measurements,
explicit tom-bleed uncertainty, microphone covariance, bounded static kit/group
fader search, held-out section regression checks, event-level compressor/master
reduction, and exact unmatched OLD/A/B excerpts. The local processing candidate
partially restores guitar low-mid cuts after rendering the fader-only candidate.
Initial policy targets remain hypotheses and some remain unmet; listener preference
is pending. No playback or hardware changes. [Workflow and limits](../../guides/BALANCE_PASS.md).

## Manufacturer-reference experiment

Implemented and offline-validated: explicit low/high shelves, broad-Q bells,
traceable local Yamaha parameter mapping, unsupported-parameter audits and a bounded
training-only compression adaptation. Six local sessions were assessed; five received
full-song processing and separate fader experiments. Exact SOURCE → OUR MIX excerpts
and two independent produced-reference comparisons are ready locally. The normal
suite passes (41 Rust and four Python tests); listener preference remains unknown.

Only Complainiacs' fader proposal was accepted. Rainfall met its applicable balance
policy without a fader change; other band targets remain unmet, and Phoenix has no
applicable ensemble policy. Dark Ride's full-band render is deferred without a
verified bass DI. [Methods, reference-use limits and results](../studies/PRESET_EXPERIMENT.md).
Manufacturer collections, recordings, settings and detailed private evidence remain
local. No console emulation or hardware verification is claimed.

## Instrument tone preparation

Implemented offline: explicit known guitar input groups, intent-conditioned
body/presence measurement, a fixed-budget broad-EQ search, actual DSP validation and
held-out rejection. Original routing, faders and secondary paths are preserved.
The listener reported improved overall balance from the manufacturer-reference pass
but insufficient Complainiacs guitar body. The new pass addresses that automatically;
its full-song correction improved held-out body/presence by 1.6 dB while the
selected tone range remains unmet. Listening preference remains pending. Other fault
classes are future work; no ringing fault is asserted in this source. See [tone preparation](../../guides/TONE_PASS.md).

## Source rules and source-first advice

Implemented offline: explicit numerical profiles with instrument/capture matching,
body/presence correction, sustained-compression relief, PCM full-scale-contact
abstention, combined-rule validation and separate remaining-target reports. Profile
names do not select processing. Large live-capture mismatches produce setup-specific
source-adjustment advice and retain current settings for a repeat soundcheck.
Historical recordings can receive a bounded improvement with limitations recorded.
Style catalogues, other instrument families, live musician UI and physical capture
control remain planned. See [source rules](../../guides/SOURCE_RULES.md).

Offline validation: 68 Rust and four Python tests pass. Five local guitar pilots
and a full-song Complainiacs check preserve the existing accepted full-song settings;
a labelled injected compression fault demonstrates bounded partial relief. Reports
separate remaining target deviations from accepted changes. No new listening round
or playback was added. Detailed outcomes are in [source rules](../../guides/SOURCE_RULES.md).

## Guitar decay revision

The prior ending veto treated a fading spectrum as steady tone. The new temporal
rule preserves dynamics checks on that fade while allowing a stronger static guitar
correction. Complainiacs now accepts +5 dB at 300 Hz / −3 dB at 2400 Hz; held-out
body/presence improves from −8.49 to −2.36 dB. One current → new mix pair is ready
locally. Listener preference is pending. The four other saved session pilots retain
their settings. 71 Rust and four Python tests pass. See [evidence, rules and
limitations](../studies/TONE_DECAY.md).

## Independent supplied-reference review

Implemented offline: bounded event-envelope alignment, confidence/ambiguity checks,
matched-section tone/dynamics/stereo measurements and exact native-gain excerpts.
Reference measurements never select mix settings. Complainiacs aligns at about
+0.10 s in eight main sections; its fading ending is ambiguous. A prepared two-clip
player now completes preflight before either clip starts. 76 Rust and eight Python
tests pass. This work and all supplied media remain local, without a push. See
[reference review](../../guides/REFERENCE_REVIEW.md).

## Shared source library

267 loose WAVs and the original recording library now live in `../waves`, available
to GigPies and shr-lux. Existing GigPies recording paths remain valid through an
ignored symlink; the shared catalogue has canonical paths. No audio was rewritten
or pushed. See [layout and verification](../../development/LOCAL_MEDIA.md).

## Bass DI and kick review

Implemented and offline-validated: synchronized processing-stage measurements,
confidence-limited note estimates, bounded bass EQ, note-support/transient guards
and a separately measured bass-fader option. Complainiacs selects +4 dB at 700 Hz
and −2 dB at 180 Hz, Q 0.7, with all previous guitar/drum settings retained.
Held-out definition/body improves 3.89 dB; the provisional intent range remains
unmet. A localized PCM-contact safeguard was reconciled with the existing repeated
contact rule, with the unchanged candidate revalidated and the source limitation
retained. One verified 12-second current → bass-corrected pair is ready; no playback,
commit or push. 79 Rust and eight Python tests pass. See [bass/kick evidence and
limitations](../studies/BASS_KICK.md).

## Kick/snare rhythmic emphasis

Implemented and offline-validated: synchronized drum processing-stage evidence,
bounded static-EQ sensitivity probes, recovery-conditioned compressor relief and
explicit processed-output rhythmic offsets with frozen-candidate full-song checks.
Complainiacs retains existing drum EQ/compression and adds +2 dB at each kick/snare
fader, preserving the accepted guitar and latest bass processing. Full-song peaks
require 1.682 dB more export attenuation; the finalized mix is about 1.2 LU quieter.
One exact baseline → new 12-second pair is ready, without playback. Neutral-balance
perception, snare tone preference and listener acceptance remain unresolved.
86 Rust and eight Python tests pass. See [drum evidence and tradeoffs](../../guides/DRUMS.md).

## Confirmed snare bleed and event-detector repair

The listener confirms snare bleed. A deeper review found that a weak precursor could
lock out a stronger hit and corrupt quiet-hit/body statistics; the detector now
retains competing rises, protects compound events and keeps independent crest guards.
Three bounded static snare EQ candidates fail protection/confidence checks and remain
unapplied. Frozen low-band waveform predictors explain about 22% (kick reference)
and 40% (overhead reference) of weak-event energy in 42 held-out events, but do not
establish safe source separation. No gate, expander or subtraction is enabled.
The existing rhythmic pair passes revalidation. 94 Rust and eight Python tests pass.
See [snare-bleed evidence and failure handling](../../guides/SNARE_BLEED.md).

## Temporal bleed identifiability and recovery follow-up

The next bounded experiment compares frozen attack and four-phase temporal shapes.
The 24–36 s training passage supplies 28 attack references but no strong events with
isolated 240 ms support; temporal fitting abstains. Attack shapes still overlap
quiet kick-coincident hypotheses, so source removal remains unsupported. A separate
recovery defect now stops the preceding tail at the next cluster's weak precursor,
removing six kick and eight snare invalid recovery votes. Full-song production-DSP
revalidation retains the unchanged +2 dB rhythmic candidate and its export tradeoff.
95 Rust and 16 Python tests pass; three historical tests remain opt-in. No new audio,
playback, commit or push. See [temporal evidence and required soundcheck](../studies/SNARE_BLEED.md#temporal-follow-up-frozen-representation-audit).

## Joint-reference bleed diagnostics

Implemented and offline-validated: a fixed-delay joint kick/overhead predictor,
complete split-contained sample support, bounded fitting and frozen full-source
evaluation. On 42 held-out weak hypotheses, median low-band energy explained rises
to 47.75%, but hypothetical subtraction increases energy in 76/123 strong events
and has large protected-event/gap changes. Source identity remains insufficient;
no processing or listening export was selected. Known-source regressions protect
against contaminated references and correlated unison playing. 102 Rust and 16
Python tests pass; three historical tests remain opt-in. See
[joint-reference methods and failures](../studies/SNARE_BLEED.md#joint-kickoverhead-reference-experiment).

## Complete six-example listening checkpoint

All six established examples now have full SOURCE and FINAL MIX exports plus one
verified synchronized excerpt pair. Dark Ride receives its first complete render,
retaining both complementary bass parts without inventing DI/amp identity. Phoenix
accepts a full-song guitar correction at fixed faders; the other examples preserve
their latest defensible baselines. No snare cleanup or new static balance retry was
applied. Final files are independently peak-finalized and clips retain exact parent
PCM. 102 Rust and 20 Python tests pass; three historical tests remain opt-in.
Listener acceptance remains pending; nothing was played, committed or pushed.
See [selection, routing and validation](../studies/LISTENING_CHECKPOINT.md); the private listening
index is `artifacts/automix/listening-checkpoint-v1/LISTEN.md`.

## Complainiacs reassessment after checkpoint listening

The listener found SOURCE better in some passages and FINAL too forward in the
voice, less prominent in guitars and lacking bass/body. A local audit traced the
largest relationship changes to earlier faders, overlapping vocal EQ, the guitar
presence correction and drum-driven export attenuation. A bounded five-proposal
pass selects one new **fader-only** FINAL: kick/snare −2 dB, both guitar paths +2 dB,
bass +1 dB and lead vocal −3 dB relative to the checkpoint. Processing probes were
not selected; failed protections remain recorded. The old artistic balance policy
still fails and is not presented as listener preference.

Implemented: actual SOURCE bypass analysis and fixed-window paired ensemble reports
with section, quiet/strong, coherent-microphone and export-stage evidence. Validated:
103 normal Rust and 24 Python tests, unchanged prior measurements on Rainfall/Catbite,
and exact full exports/clips at 24–36 and 90–102 seconds. Five other examples retain
their settings. New listener acceptance remains pending; no playback or publication.
See [audit, settings, decisions and limits](../studies/COMPLAINIACS_REASSESSMENT.md). Private
listening index: `artifacts/automix/complainiacs-reassessment-v1/LISTEN.md`.
