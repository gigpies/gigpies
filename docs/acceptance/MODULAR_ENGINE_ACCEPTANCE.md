# Modular engine acceptance

Scope clarification: this task0014 record covers the Stagebox source-clock graph
and device-free FX worker. Brain's separate local duplex sound card and its two
ASRC crossings are tracked in [Brain audio](../reference/BRAIN_AUDIO.md). The historical
single-domain checks below do not establish synchronization between those cards.


Task0014 has passed the required software and two-Pi functional acceptance.
The original eight-input milestone, PA v1 ABI and historical evidence remain
intact. Physical UMC1820/ADA8200 mapping, converter synchronization, acoustic
protection and sustained hardware deadlines remain unqualified.

## Reviewed architecture

An independent source review selected evolution of the existing Mixer/Authority,
an owner-native configurable SHR PA successor, and mutually authenticated QUIC
for both control streams and GPA1 datagrams. Shared prepared topology translates
stable physical/source/strip/bus/module/output IDs into bounded render indices.
Preparation, JSON, network, credential work and state retirement stay off render.
Structural changes quiesce and retain old state on refusal; successful replacement
and recovery remain muted until explicit rearm. Brain follows device source frames.

## Requirement and evidence matrix

| Requirement | Owner/code boundary | Required acceptance | State |
|---|---|---|---|
| Configurable strips and monitor buses | GigPies mixer, authority, processing schemas | Every input at 16/32/48 and a nonfixture count; independent EQ/dynamics, isolation, neutral exactness, ramps and partition equivalence | Focused pass: `modular_engine` covers16/17/32/48; complete default suite passes |
| Resource admission and prepared ownership | GigPies topology/render, SHR PA prepared ABI | Checked dimensions/budgets; precise refusal; no render alloc/free; saturated pending/retirement slots | Focused pass: `gp07_alloc`, `modular_engine`, actual PA/module graph; complete hardware-feature software suite passes |
|16/18 analog patch | GigPies topology/device adapters | Unique channel patterns through explicit configurable USB/socket permutations; manufacturer18/20 reference separately labelled unverified; duplicate writers refused | Software permutation/reference tests pass; attached UMC/ADA mapping unverified |
| Single synthetic/device production graph | GigPies module graph/host | Same code path, raw REC/analysis, dry+wet before PA, silent unmapped outputs | Actual PA/REC/FX host and composed producer tests pass; physical adapter unopened |
| Six/eight PA outputs and4x8 matrix | SHR PA config/graph/DSP | Independent complex crossover branch/sum references; weights/headroom, gain/EQ/delay/polarity and final protection | Owner normal79 tests and actual loaded graph pass; PA source published with green CI |
| PA successor ABI | SHR PA ffi/header; GigPies adapter | Real release-linked C caller and actual loaded library; exact sizes/version/lifetimes; old v1 bytes/behavior | Actual C v1/v2 callers and loaded host pass; exact owner hashes retained privately |
| Scoped PA and output routing control | GigPies authority/Desk | Dedicated grants; atomic prepared commit/readback and actual boundary samples; retained muted rollback | Actual local producer and authenticated Desk on both Pis pass at 16/32/48, with final boundary samples and readback |
| Authenticated commands/media | GigPies remote/Desk transport | Actual paired peers, wrong/revoked peer refusal, bounded framing, capability/descriptor exchange, no reconnect replay | Remote security/authority regressions and actual two-Pi Desk/session negotiation pass |
| Real Brain FX and analysis | GigPies remote/host, unchanged SHR FX | Grouped source-indexed media on both Pis, exact identity, actual wet samples; loss/reorder/stalls/restart preserve dry/REC | Actual loaded FX/REC, fault regressions and two-Pi grouped media/restart pass at 16/32/48 |
| One clock domain | GigPies clock/provider/host, Desk health | Fake-device faults exercise production mute, incomplete recording, new epoch/map, rejection of old intents/audio and explicit rearm | Synthetic production fault/recovery tests pass; physical clock/ADAT lock remains unknown |
| Dynamic Desk and protected review | SHR Desk client/frontend | Inputs16/17/32/33/48, bank navigation, PA/patch/clock controls, keyboard/controller parity and CPU-headless resize | Default/native consumer suites, CPU-headless presentation and actual high-channel/PA/patch/reconnect checks pass |
| Persistence and compatibility | GigPies schemas/restore, Desk | Safe restore of intended mappings/config; mismatch refusal; no unmute/grant replay; old fixture preservation | Focused safe restore, mismatch, legacy corpus and unsupported-version refusal tests pass |
| Publication and synchronization | Each owner/coordinator | Normal production suites, Clippy/fmt/release/C/docs/guards, independent final review, exact upstream/CI and twelve repos on both Pis | Reviewed source and validation evidence; exact publication/CI revisions and receiving acknowledgments are recorded in the private task0014 completion manifest |

## Capacity reporting

Dimensions represent separate resources. Mixer strips do not define USB ports,
monitor count, PA outputs, recorder tracks, network group size or UI bank size.
Each implementation bound must report its origin, units, admission reason and
expansion path. SHR REC v1 currently accepts1–64 tracks,8192 frames per push and
64MiB queued samples; an optional recorder refusal must never silently trim a
show. GPA1 protocol channel indices and datagram MTU are transport bounds.
Software correctness at 48 inputs is required; hardware deadline capacity remains
unqualified until measured on the actual configured graph.

**Core DSP cost per 48-frame block — 2026-11-25.** Opt-in `tests/source_capacity.rs`
(release, one non-realtime thread): every strip with gate, four EQ bands,
compressor and vocal assist engaged on a −12 dBFS signal. Core mixer only:
strips, sends and buses; no metering, spectrum, PA/FX modules, transport or
control. Budget is the 1000 µs period. p99 µs (share of period):

| Inputs × monitors | Pi 5 | Pi 4 |
|---|---|---|
| 16 × 10 | 131 (13%) | 357 (36%) |
| 16 × 16 | 151 (15%) | 405 (41%) |
| 24 × 10 | 203 (20%) | 531 (53%) |
| 32 × 10 | 272 (27%) | 728 (73%) |
| 32 × 16 | 315 (32%) | 842 (84%) |
| 48 × 11 | 437 (44%) | 1090 (109%) |
| 64 × 7 | 575 (57%) | 1318 (132%) |

Cost scales about linearly with inputs (≈9 µs per input on Pi 5, ≈24 µs on Pi 4,
plus ≈0.2 µs on Pi 5 and ≈0.5 µs on Pi 4 per input per monitor). The product
target is Pi 5 at up to about 24 inputs (main card ≤ 8 plus two 8-channel ADAT
expansions): core DSP at 24 × 10 is ≈20% of the period, leaving the rest for
metering, spectrum, PA/FX and transport, which still need measuring on the full
graph. Pi 4 is a development node only, never a product host. The 32–64-input
rows are headroom evidence, not targets. Single multi-millisecond maxima came from
preemption of the non-realtime test thread. Topology admission is a static budget,
not a measurement of the host; the 1 MiB snapshot assembly only binds well above
the product range (48 inputs: at most 11 monitors). An admitted topology the host
cannot render shows as `late_blocks`, then closes on debt or xrun. Not physical
or full-graph qualification.

## Validation classes

Focused production regressions run during implementation. Complete normal suites
run for every changed engine/schema/authority/concurrency owner, including affected
consumers. Actual trusted-library, producer-fixture, CPU-headless and reserved
bounded two-Pi functional checks run explicitly. Historical private music, auditions,
exhaustive research and long benchmarks remain opt-in unless directly affected.
Physical device/output, speakers, MIDI/DMX, clock switches, operator windows, host
tuning, deployment and full-show combined-load qualification are not authorized
by this software increment.

## Physical mapping evidence

Physical sockets, USB transport slots, contiguous logical analog channels and
processing destinations have independent identities. Neither a manufacturer's
channel table nor a historical stereo fixture supplies an observed Linux patch.
USB indices and signal/socket assignments require independent validation.
The implementation supports reviewed explicit patches and arbitrary permutation
tests without constraining DSP or Desk to the expected device ordering.
No PA/monitor output allocation is mandatory.

Read-only Pi5 inventory on2026-10-05 found a PreSonus AudioBox USB96 with two
capture/playback channels; no UMC1820/ADA8200 was attached. No PCM was opened.
The UMC reference therefore remains unverified. The manufacturer's
[UMC1820 guide](https://www.bhphotovideo.com/lit_files/155647.pdf), controls13/23
and Digital I/O Routing, documents ADAT capture11–18 and playback13–20 at 48kHz,
with S/PDIF occupying separate slots. This does not establish the actual attached
Linux device order. A reference mapping must never silently become an accepted
physical map. Logical input 09 means the ninth admitted source, not USB slot 9.

## Integrated software evidence — 2026-10-05

The actual Desk on Pi4 controlled the processing authority on Pi5 over mutually
authenticated QUIC. The same loaded PA v2, REC and FX owners were used throughout.
No physical PCM, display window, MIDI/DMX, host tuning or deployment was activated.

| Inputs | Capture/playback transport slots in the fixture | Source frames | Exact recorded frames per input | Independent checks |
|---|---|---:|---:|---:|
|16 reference analog|18/20, noncontiguous analog map|1,440,048|1,436,832|94 passed|
|32 software|32/15|1,440,000|1,434,816|125 passed|
|48 software|48/15|2,639,472|2,630,928|157 passed|

These dimensions belong to the declared fixtures, not product ceilings. Each
software topology has independently configured strips, five monitor buses and
eight PA ports. The reference starts with all eighteen analog outputs unpatched.
Its baseline stayed exactly silent; the reviewed Desk patch made the selected
PA destination nonzero while every unassigned playback slot, including both
S/PDIF slots, stayed exactly zero. Actual Linux/socket mapping remains unverified.

The 32/48 profiles also prove weighted routing independently of the DSP algorithm.
Desk imported a real owner node summing 0.5 × input 0 + 0.25 × input 1, with a gain edit,
then patched an additional physical model port to PA0. The verifier resolves all
sources through the complete topology: these fixtures place PA0/PA1 at playback
slots7/8, after Main and monitor ports. The selected additional slot 2 exactly
matched PA0 sample for sample. In the 48-input run, the measured PA0/PA1 ratio was
0.6956254690668727 versus the independent gain/weight prediction 0.6956254690668728;
maximum first-block residual was 6.78×10⁻²¹. The same-band reference and common
ramp test amplitude and phase without copying crossover code. Reference16 uses
its own silence/patch proof and the 32/48 companion proofs for this relationship.

Actual scoped operations covered rearm, mute and observed quiescence, complete
owner PA configuration, output patch, fresh reconnect/readback and explicit rearm.
Each final revision was checked against its effective source-frame boundary.
A separate successful48-input run edited all24 four-band/compressor fields on
inputs16,17,32,33 and48, alternating keyboard and injected-controller paths,
checking no other target changed, settled readiness and read-only reconnect.

Each profile ran two separate two-second Brain children using actual SHR FX.
Every admitted raw analysis channel had zero source-sample mismatches, and both
sessions produced nonzero wet samples accepted and rendered by the processing
node. FX's960-frame intentional delay is separate from the 768-frame negotiated
return delay. The source clock and REC timeline continued across both sessions.
After Brain exit, bounded source windows proved an absent media session, zero
wet contribution and nonzero mapped PA output, both during restart and afterward.
Every recorded PCM24 sample was independently checked against its logical input
and original source frame; no gaps, drops or invalid blocks occurred. Intentional
source shutdown finalized these temporary takes as incomplete. Retained hashes,
timelines, results and failure logs replace disposable synthetic WAVs.

The finite Brain child queues QUIC close and exits without awaiting endpoint
transport drain. A500 ms restart attempt was correctly refused while the old media
owner remained active. The accepted loss/restart runs used a2.5 s observation gap
around the configured 2 s idle timeout, and checked actual source-window absence;
the wall-clock gap alone was not treated as proof. No timeout was weakened.

## Validation and measured scope

At the core source freeze, the complete default suite/CI passed 336 tests with 10
opt-in tests ignored; the hardware-feature software aggregate passed 363 with 23
ignored. Actual-owner/producer checks passed 25 and final safety/envelope checks8.
The subsequent reporting-only source-window collector passed 12 focused remote
tests, including its two new bounded-window tests, plus formatting and
warnings-denied hardware-feature Clippy. Both core release configurations passed.
SHR PA passed 79 normal tests, actual release-linked C v1/v2 callers and its script
checks. Desk passed 138 default and140 native tests, with 9 opt-in tests ignored in
each; its actual producer, operator and CPU-native checks were explicitly run.
Final owner checks and upstream CI are retained with exact source/binary hashes
in the completion record. Historical music, auditions, exhaustive research and
long benchmarks were intentionally not rerun.

The Pi4 optimized Desk decoded the captured48-channel document in 34–40 ms. The
actual larger configured-PA paired read completed in 58.8–60.4 ms in the measured
run. Debug decoding took297–307 ms and safely failed the unchanged250 ms freshness
limit. Acceptance therefore used the optimized executable; these observations
are not a general latency guarantee. The initial development runs exposed and
retained failures in envelope paging, snapshot/final schema handling, paired
freshness, status coalescing and test-driver ramp/readback correlation. The final
successful profiles used independently reviewed corrections and exact source
manifests, not relaxed thresholds or substituted data.

Finite synthetic source work used 48-frame blocks and a1 ms configured period.
For16/32/48 inputs, observed processing-node user+system CPU times were6.40/7.72/
19.18 s over30.21/30.38/55.35 s wall time; peak RSS was 9,376/11,424/13,984 KiB.
The 48-input Brain children used about1.41–1.42 s CPU over2.04–2.05 s wall time,
with 11,268 KiB peak RSS. These are bounded functional observations with recording
and brief Brain activity, not full-show load benchmarks or hardware deadlines.
The diagnostic collector retains at most 1,024 windows of 4,800 source frames,
splitting on revision/map/clock/quiescence/session changes and reporting overflow;
all accepted runs had zero overflow. Collection occurs outside DSP rendering.

The exact remaining physical procedure is in
[Modular processing](../reference/MODULAR_PROCESSING.md#mapping-and-physical-acceptance):
verify the actual sockets/USB indices and hardware monitoring path, establish the
UMC/ADA single-clock relationship and observed lock, test safe discontinuity and
rearm, then qualify sustained configured hardware deadlines under separate session
authorization. Software correctness does not establish any of those observations.
