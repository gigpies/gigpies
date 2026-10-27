# GigPies implementation plan

Original planning baseline **2026-10-04 / GP-2026-10-04.1**. The execution
checkpoint below supersedes the historical inspected baseline and launch cards. [Central inventory](MODULE_IMPLEMENTATION_MAP.md) ·
[Agreed contracts](../../reference/MODULE_CONTRACTS.md). Existing product roadmaps remain authoritative for
unrelated work; this plan owns only the GigPies integration increments below.

## Objective and boundary

Own the complete audio and lighting system, Stagebox mixer/host and integration.
Stagebox retains physical I/O, dry main/monitors, PA protection and local recording.
Brain hosts Desk, Lightdesk, Lux, analysis and richer FX. Manual operation needs no
automixer. Keep PA/FX/REC/Lux algorithms in their owners; no speculative mixer crate,
DAW runtime dependency, phone app, instrument host or independent audio clock in
this first increment.

## Current execution checkpoint — task0009

GP01/02/03, the private local subset of GP06, GP09 descriptor/lease ownership and
GP04 named raw analysis subscriptions are implemented and accepted as synthetic
software. Task0008's complete acceptance supersedes its earlier pending entries.
GP05 fixed-owner integration is software validated and independently reviewed;
see [its graph, health envelope and commands](../../reference/MODULE_GRAPH.md). Providers REC, FX
and PA have independently accepted and published source checkpoints. Root owns
central contracts, consumer acceptance, joint demonstration and publication.

Physical audio/controllers/displays, production authenticated remote control,
configurable PA/writable FX, scheduler/throughput and combined load remain separate.
Prior hardware H8 evidence retains its original stereo-only limits. No current
synthetic result expands that qualification.

## Historical source and evidence reviewed

Repository: `/home/shome/p/gigpies`. Inspected HEAD: `99eb0f08050a9a86039af9f1ba2a3541649ca7b6`.
Pre-existing edits: `docs/README.md` and untracked `docs/archive/prompts/MODULE_PLANNING_EXECUTION_PROMPT.md`; preserve both.

Owning documents: README.md, docs/STATUS.md, ARCHITECTURE.md, COMPONENTS.md, BRAIN_CONSOLE_PLAN.md, NEXT_SESSION.md, NODE_LAB.md, AUDIO_TRANSPORT.md, AUDIO_HARDWARE.md, PUBLICATION.md.

Source inspected: `src/transport/control.rs::{Command,Authority}`, `packet.rs::StreamSpec`, `handoff.rs::CaptureFanout`, `src/host/adapters.rs::{Dsp,Recorder}`, `src/host/mod.rs`, `src/automix/{render,dsp}.rs`.

Implemented/offline-validated: automix renderer and GPA1 packet, queue and recovery
paths. Integrated: versioned PA/FX/REC libraries in the stereo hardware host.
Hardware record H8 is qualified: 48-frame blocks, exact digital/recording checks,
249–251-frame electrical offset and two unexplained one-frame changes; weak right
return unresolved. It proves neither a complete band mixer nor dual-console load.
`transport::control::Authority` controls a test scalar. New schemas, real mixer
controls, show/role persistence and dual-surface adapters are absent.

These are source inspection and previously recorded results, not fresh builds or
physical acceptance. The planning session runs documentation checks only.

## Milestones and tasks

First useful milestone: independently started surfaces can identify one show and
exclusive roles in a headless harness without recalling state. Next: deterministic
real offline mixer plus authoritative controls; null Lux and read-only adapters;
recorder/FX/PA lifecycle integration; then production remote/device acceptance.

Task states are execution dependencies: READY has no missing software provider;
WAITING names its precise prerequisite; DEFERRED has an activation condition.
Source delivery and build reservation are additional launch prerequisites on a
peer. Every row has one owner, the repository named in its Owner column. A later
task starts only after the previous artifact is reviewed, never merely delivered.

| Task / priority / state | Owner | Work area, inputs and required artifact | Output and measurable acceptance |
|---|---|---|---|
| GP-01 / P0 / ACCEPTED SOFTWARE | GigPies | Add `src/show.rs`, `src/roles.rs`, expose from `src/lib.rs`; synthetic contract tests. Inputs C-SHOW:1/E01 and C-ROLE:1/E02, existing serde/sha2. | Strict manifest/role model, atomic private persistence and generation-checked claim model; wrong version/duplicate identity/stale release leave state exact. No devices or sockets. Publish E01/E02 fixtures and exact hashes. |
| GP-02 / P0 / ACCEPTED SOFTWARE | GigPies | Add `src/control_model.rs` and tests; preserve `src/transport/control.rs` v1. C-AUDIO:1/E03 and common envelope. | Bounded decoding/authorization/revision/dedup and snapshot types, provider fixture corpus; reject malformed/expired/overflow atomically. Explicit offline authority harness, no claim of mixer application. Strong design review of parser/lease boundary. |
| GP-03 / P0 / ACCEPTED SOFTWARE | GigPies | GP-02 accepted C-AUDIO:1 corpus. Add small `src/mixer.rs`, use established f64 arithmetic/provenance in automix; `src/lib.rs`. | Eight synthetic mono sources → stereo main + two pre-fader monitor buses; real gain/pan/mute/send controls with 240-frame ramps, holds and block acknowledgements. Known sample/reference, partition, fault, no-allocation tests; UI loss leaves graph running. No hardware host wiring yet. |
| GP-04 / P1 / ACCEPTED SOFTWARE | GigPies | GP-03 stable source/tap IDs; C-ANALYSIS:1/E09; extend `src/transport/{packet,handoff}.rs` via adapter, not format change. | Named four-source bounded subscription descriptor and synthetic PCM window delivery; drop/epoch/map-change tests, recorder/audio never backpressured. Publish same E09 data for LX-05. |
| GP-05 / P1 / ACCEPTED SOFTWARE | GigPies | GP-03, REC-02 C-REC:1 query artifact, FX-01 C-FX:1 and PA-01 C-PA:1 descriptors; explicit library hashes. `src/host/adapters.rs`, offline integration harness. | Read-only provider health and real synthetic start/stop, fixed wet+dry→PA protected processing, exact recorded stems. Writable FX/configurable PA substeps additionally require FX-03/PA-02 accepted new headers; do not gate fixed-v1 read-only work on them. |
| GP-06 / P1 / PRIVATE LOCAL ACCEPTED; REMOTE WAITING | GigPies | GP-01..GP-03, C-REMOTE/B-NET review; control adapter outside callback. | First approve remote auth/framing/pairing and threat boundary, then implement bounded transport preserving existing GPA1 and explicit legacy refusal. Two endpoint fixtures, lost ACK/restart/slow consumer tests. No new remote production writes until review passes. |
| GP-07 / P2 / PARTIAL | GigPies | GP-03/GP-05, DS-04 acceptance; C-AUDIO:1 extension review. `src/mixer.rs`/host mapping. | Four-band correction software-validated: [prepared FOH EQ/dynamics](../../reference/CHANNEL_PROCESSING.md), versioned authority/readback and Desk editing on all eight inputs. Remaining slices: monitor/PFL scope; atomic muted route/scene transactions; capability-based physical channel map. Each slice has independent tests/review; never infer three-way device routing from logical PA outputs. Phone controls remain separate. |
| GP-08 / P3 / DEFERRED | GigPies | C-CUE:0; activation requires useful named event consumers after GP-06 and LX-04. | Review event scopes/expiry/individual failure results before any schema/driver. E10 no implicit cross-domain action remains protected. |
| GP-09 / P1 / ACCEPTED DESCRIPTOR/LEASE SOFTWARE | GigPies | GP-01 C-ROLE:1 E02 accepted; `src/roles/native.rs`, private assignment registry and adapter tests. | Implement descriptor-based display/controller identity adapter, process-held exclusive locks and generation-checked role handoff. Own the low-rate assignment plane only; desks keep independent input/LED workers. Synthetic enumeration/lock/crash tests first; real enumeration/open remains explicit GP-H2. Publish role-binding artifact for DS-02/LD-02. |
| GP-H1 / P2 / DEFERRED | GigPies | GP-05/GP-06 plus fresh operator session/reservation. | Qualified stereo continuation and then explicit multichannel/clock/acoustic acceptance; preserve H8 and right-route failures. |
| GP-H2 / P2 / DEFERRED | GigPies | DS-02/LD-02, GP-09 accepted native role adapter and B-DEVICE evidence. | Two 1080p monitors/two verified MIDI controllers, separate LED owners, focus/restart/reconnect usability. No display settings changed by build tests. |
| GP-H3 / P2 / DEFERRED | GigPies | GP-H1/GP-H2, LX-06, production module revisions, explicit shared reservation. | Whole-Brain FX/Lux/analysis/two-desks with Stagebox recording: deadline, PSS, CPU/GPU, thermal and fault acceptance against BRAIN_CONSOLE_PLAN targets. Runtime Pi assignment only after evidence. |

## Validation and failure behavior

During GP-01 use focused new show/role regressions, then the complete normal
production suite because persistence/roles are shared. Reproducible commands:
`cargo +1.97.1 fmt --all -- --check`; `CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked --all-targets -j 1`;
`CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked --all-targets --features hardware-host -j 1`
when host integration changes (tests remain device-free);
`python3 -m unittest discover -s scripts -p 'test_*.py'`;
`python3 scripts/check_publication.py`. Clippy uses matching feature set and `-D warnings`.
Rendering/engine/transport/schema changes require full normal classes. Historical
ignored tests retain their documented explicit commands in VALIDATION.md.

GP-03 callback has no allocation/deallocation, locks or I/O; prepared changes and retirement stay outside. Numerical fault cannot bypass PA. Test independent queue overload, stale grants/epochs, partial snapshots, recorder finalization, mode continuity and fresh-snapshot recovery. No one-frame physical uncertainty is erased by software tests.

Historical research, auditions, exhaustive matrices, long soaks, full-show renders
and physical/combined-load checks are intentionally outside the normal software
milestones unless their protected behavior changes. Retain their owning documented
on-demand commands; no private media download or test hardware side effect.
Independent builds retain lockfiles and existing repository editions; this plan
does not upgrade dependencies/editions or replace existing intra-repository workspace
paths. The ban is on new sibling-repository path dependencies.

## Resources, review and recovery of work

Local lane A, rpi5 NVMe/2 GiB resource class. One build, `-j 1`; provisional compiler ceiling 1.2 GiB RSS, ≤1 GiB incremental disk growth for first model task. Existing GigPies target is 7.6 GiB and needs review before later builds, not deletion. Local lane B is a different Lux checkout. Coordinator owns central contract files; other lanes submit proposals.

Independent fallback: GP-02 source review/tests design while waiting for the build slot. No unbounded render or research assignment.
Before builds check free space and target size; below 20 GiB free or above 5 GiB
output is a review, not permission to delete another task's cache. No reduced
coverage/debug information to make a budget appear to pass.

Handoff: exact changed files, commit plus patch hashes or bounded source manifest
if uncommitted, contract IDs/versions and provider-fixture hashes, commands/results,
intentional skipped classes, remaining limits and next task/owner. Stage only named
owned changes if a later implementation session commits; no public push is implied.
Receiving owner reviews independently and writes an immutable private-ledger
acknowledgement. Interrupted work stays visible with last completed acceptance
criterion; never reset/stash/clean another session or replay an uncertain mutation.


## Historical first-task implementation launch prompt

Host/cwd assignments and source preparation are in GigPies PARALLEL_WORK_PLAN.md.
This is a prompt for a later user-started session; no implementation worker has
been started by the planning pass.

```text
Work only in the current gigpies checkout. Read AGENTS.md (if present), the
owning docs and docs/archive/plans/MODULE_IMPLEMENTATION_PLAN.md, then the referenced GP-2026-10-04.1 contracts.
Implement only GP-01; keep progress and evidence in this plan. Check hostname,
HEAD/source manifest, live Git state, active ownership and current contract hashes
before edits; preserve other sessions and unrelated work. If this is a delivered
snapshot, verify its handoff manifest and separate receiving acknowledgement first.
Reserve this host's one build slot as described in GigPies PARALLEL_WORK_PLAN.md;
use Rust 1.97.1, Cargo.lock, CARGO_INCREMENTAL=0 and cargo -j 1 in normal target/.
Run focused checks during work and the required normal suite for changed behavior.
No sibling writes, sibling path dependencies or unilateral contract changes.
No audio/MIDI/DMX/playback/device/display/service changes or shared load tests.
When blocked report exact provider/task/version mismatch and continue only the
independent fallback GP-02 source review/tests design while waiting for the build slot within this repository; do not fake acceptance.
Stop after the scoped task and reviewable handoff, before later milestones,
physical operations, publication or deployment. Do not stage unrelated files or
claim mock, planned or incomplete behavior is a finished engine.
```

## Progress

- 2026-10-04: source and owner documents inspected; plan written. Implementation
  tasks remain in the states above. Physical evidence retains its original limits.

### GP-01 / Lane A / 2026-10-04

Implemented in the isolated `gigpies-gp01-0007` worktree at source baseline
`99eb0f08050a9a86039af9f1ba2a3541649ca7b6`, pending coordinator review.
`src/show.rs` provides bounded strict C-SHOW:1 decoding, decimal-string counters,
module compatibility checks and portable state identities. Attachment returns
metadata only; version mismatch is read-only and unknown optional modules remain
unavailable. `src/roles.rs` provides C-ROLE:1 pure intended assignments with
exact generation checks, exclusive role/controller/connector identities, explicit
labelled duplicate-EDID choices and no automatic enumeration-order binding.
Both models support private create-new/synced/atomic checkpoints, with explicit
error reporting when directory sync leaves durability uncertain. Future schemas
are refused; no migration implementation is supplied.

E01/E02 synthetic prior-state fixtures and refusal semantics are documented in
`tests/fixtures/gp01/README.md`; hashes are reproducible with
`sha256sum tests/fixtures/gp01/e0{1,2}.json`. Provider regressions are
`cargo +1.97.1 test --locked --test gp01 -j 1`, always under the host build lock.
Exact source manifest and validation results are retained in the private Lane A
`handoff.md`. Central contract SHA-256 remains
`54a887f4a300d049d3ba67077a2e0446f7a92c57c41cef9d3b7da13735a092ac`.

This is pure/offline software, not integrated or hardware-verified behavior.
The native identity/profile adapter, OS-held per-device locks and crash/reclaim
lifecycle remain GP-09; a saved intended assignment is not a live device claim.
No engine recall, grant, recorder start, output arm, socket or device operation
is added. GP-02 and GP-09 remain separate reviewed milestones. Coordinator is
next review owner; Lane B receives the next Pi5 build turn after explicit release.

GP-01 validation: five focused regressions and the complete default-feature normal
suite passed (193 Rust tests; four existing media/socket opt-ins ignored), plus
37 Python tests, formatting, publication guard and warning-denied Clippy.
All build-producing commands used Rust 1.97.1, locked dependencies, one job,
CARGO_INCREMENTAL=0 and the shared parent-held nonblocking build lock. First build:
24.58 s, max RSS 665808 KiB, zero swaps. Total retained target growth 994151948
bytes; final free space 30772670464 bytes. Hardware-host feature validation was
not run because host integration did not change. Historical/exhaustive/media,
live I/O and hardware/load classes were intentionally skipped.

### Task0008 / Lane A / GP-01 correction and GP-02

Task0008 sustained-software authorization supersedes the historical task0007
first-task stopping rule above; physical gates and owner boundaries remain.
GP-01 optional compatibility correction is source-accepted by root at checkpoint
R1: optional unavailable/version-mismatched modules retain unusable per-module
status without disabling compatible required modules. Required version mismatch
still yields global read-only; missing required module still rejects attachment.

GP-02 implemented as a **pure offline authority harness**, not a mixer. Typed
bounded encoded C-AUDIO:1 commands, 2000ms injected-monotonic leases, exclusive
FOH/two monitor scopes, revision/refusal atomicity, identity-before-cache and
cache-before-revision retries, E03R high-water refusal,64 replies/session,
1024 writer identities/epoch (new epoch required at capacity), expiry-preview
cleanup, human holds, ASSIST proposals and AUTO bounds are present. One synchronous
mutation at a time; no pending DSP work is falsely acknowledged. Preview release
is revision/scope/session/expiry-bound; commit is unavailable pending GP-03 timing.
Actual rendered state/effective frame/meters remain unavailable. Target state is
volatile, without persistence/restart guarantees. See AUDIO_CONTROL_WIRE.md for
actual bodies and tests/fixtures/gp02/v1 for exact encoded corpus/provenance.
Snapshot/page validation protects inventory, uniqueness, coherent revision and
local receipt freshness. GP-02 final validation/review is recorded in the private
handoff; provider acceptance and consumer transfer require root review of exact
final hashes. GP-03 is deferred until that review, with no DSP implementation here.
No audio/GPC1/GPA1 changes, live endpoints, devices, native graphics or sibling
source/dependency changes. The normal production suite includes encoded authority
and malformed-provider regressions; one-time corpus generation is opt-in/ignored
with the reproducible command in its README. Hardware verification remains absent.

Final GP-02 offline validation:6 GP-01 +11 GP-02 focused tests passed;
complete normal Rust all-targets suite207 passed,0 failed,5 ignored;37 normal
Python tests passed; Clippy all-targets with -D warnings, formatting and diff
checks passed. One-time corpus generation explicitly ran; normal encoded corpus
replay passed. Publication guard passed180 existing index entries, without
staging/publishing the new implementation. The5 ignored tests are3 private-media
historical tests,1 UDP socket-capacity test and1 GP-02 corpus generator. No release,
hardware-host, physical, native graphics, exhaustive or long benchmark work ran.
A final focused regression enforces first grant ID1 and retained high-water at
u64 max. Source review at checkpoint GP02-R2 accepted the harness/corpus; final
hashes include that correction and subsequent style-only repairs for root review.

### Task0008 / Lane A / GP-03 in progress

Implemented a fixed-storage eight-mono f64 graph in `src/mixer.rs`, stereo FOH
and two independent post-shared-mute/pre-FOH monitor sends. Prepared atomic
transactions use strict next48-frame boundaries, 240-frame linear coefficient
ramps rebased from actual boundary values, exact endpoints and checked frame
counters. Public render process has Copy error values and no heap ownership,
parsing, locks or I/O; nonfinite arithmetic latches fail-closed zero output.
This offline graph is **unprotected**, without host/PA integration or hardware
safety acceptance.

`src/mixer_control.rs` connects actual C-AUDIO requests to the renderer using a
single-owner offline pump. Admission remains pending until graph/authority commit;
current authority is revalidated before boundary rendering, including preview
expiry. All new authority mutations are fenced while pending; outer backpressure
explicitly means not admitted and consumes no request history. Exact retries retain
original rendered outcome in bounded per-writer caches; snapshot sequence and
monotonic control time persist. Mode changes freeze actual boundary coefficients;
reviewed release-preview commits ramp proposal destinations and remove holds.
No automatic algorithm writes occur. GP02 pure unavailable harness/corpus remains
available unchanged. Typed nanogain observations and capability wrapper are in
[AUDIO_RENDERED_WIRE.md](../../reference/AUDIO_RENDERED_WIRE.md).

GP03 is offline-validated and accepted by root plus independent audio reviewer:
229 normal Rust tests passed,6 opt-ins skipped,37 Python tests passed. Typed
corpus/interface and subsequent style/provenance amendment were separately
accepted. Render allocation/error-path, independent arithmetic/partition, lease,
preview, per-writer retry and hold/mode/release regressions passed. GP06 local
binding is the authorized subsequent milestone below; remote/network,
native host, PA/FX/REC/analysis and physical acceptance remain deferred. Early review
found and corrected allocation on render errors, stale staged time/sequence,
preview-expiry-at-commit, global retry eviction and uncached pending refusals.
Exact checkpoint/source hashes and final evidence live in private task0008 pass2/A.

### Task0008 / Lane A / GP-06 local-only synthetic binding

Implemented `src/local_audio.rs` and opt-in `gigpies-headless` binary, independently
of Lux service code. Explicit show/epoch startup, private canonical owned0700
directory/0600 socket, same effective UID peer credential, strict four-byte BE
1..65536-byte JSON frames, at most4 clients/32 control replies plus latest telemetry,
bounded read/write/parse/render work and2000ms incomplete/stalled deadlines.
Connection requires fresh snapshot and writer grant; old connection identities and
leases cannot be imported on reconnect. Accepted commands operate the real GP03
renderer and final responses follow boundary commit. Synthetic input01=0.125,
remaining inputs zero;48 frames per pump keep mix running after client loss. No
callback parser/IPC/heap retirement. Graceful exit removes only owned socket inode;
there are no worker threads. Existing default CLI unchanged; pinned existing libc
is required without enabling hardware-host dependencies. Cargo.lock retained.

Offline IPC/process validation passed: fragmentation/malformed lengths, incomplete
frame timeout, client capacity, output pressure/disconnect, scope/lease/retry,
reconnect refusal, filesystem/symlink/unowned path protection, preserved renderer
on client loss, and actual child-process grant/set/rendered completion/snapshot.
Normal current suite236 passed,0 failed,6 opt-ins skipped; final interleaving
regression and release artifact validation are recorded in the private handoff.
Root strong local source/binary acceptance is pending; DS04 receives binary only
through coordinator review. Usage and limits are in AUDIO_LOCAL_SERVICE.md. No TCP,
GPA1/GPC1 change, device, host audio/display/service, remote auth, native graphics,
PA/FX/REC/GP04 or hardware acceptance. Volatile state requires new epoch on restart.

GP06 review correction at pass2 deadline: coordinator rejected restart identity
reuse (explicit epoch alone could reset lease/history). Added private exclusive
`audio.owner` flock and durable synced atomic `audio.identity` high-epoch reservation
before socket bind, with same/regressed epoch refusal and ownership/file checks.
This corrective source and restart regressions are **implemented but unvalidated**:
B pass3 owns the build turn and root explicitly requires continuation after this
worker deadline. The earlier236 normal/8 IPC/Clippy/release evidence applies to
pre-guard source only. The pre-guard release executable is not accepted for Desk
or delivery. Root must validate corrected persistence/concurrency behavior (focused,
full normal, Clippy/fmt, abrupt crash/restart cases), rebuild the single artifact and
independently accept checkpoint-GP03-local-R2 before provider binary delivery.

### Task0008 / Lane A / pass3 durable GP06 and preview interoperability

This pass supersedes the preceding unvalidated guard status. The lost-registry
finding was confirmed and corrected: startup distinguishes exclusive create-new
owner from an existing owner, durably syncs the first owner marker, and rejects
missing identity whenever that owner already existed, including interrupted first
reservation. The private0600 regular owner holds its exclusive advisory lock for
all of LocalAudio's lifetime. Bounded strict identity metadata records one show
and high epoch; greater epochs are atomically file-synced/renamed/directory-synced
before listening. Wrong show/schema, duplicate or unknown fields, malformed or
exhausted counters, symlinks, hardlinks, ownership/permission errors and lost
registry refuse without a usable listener or identity reset. Mixer/hold state
remains volatile. A stale crash socket is preserved and refuses startup; only its
verified dead owner may explicitly remove it. Entire namespace replacement remains
outside the guarantee. No renderer callback I/O, allocation or lock was introduced.

Offline validation:13 focused GP06 tests passed, including actual child-process
SIGKILL crashes and two subsequent starts that refuse old grant/set bytes, even
when fresh current-epoch permission recreates writer/lease1. First-reservation
interruption is tested with its persisted owner-only state. Regressions also cover
lost/corrupt identity, lock contention, u64 exhaustion, foreign/stale endpoint
preservation, frame fragmentation/deadlines, bounded client/reply pressure,
partial telemetry framing and retained mix through client loss. Complete normal
Rust all-targets suite243 passed,0 failed,7 ignored;37 normal Python tests passed.
Formatting passed. Final Clippy, single rebuilt release executable and release
process demonstration results are bound to the private frozen local checkpoint;
coordinator review gates delivery to Desk. The previous pre-guard binary is stale
and must never be delivered under this checkpoint.

The independently reviewed exact Preview/ReplyBody documentation and actual
producer amendment are in AUDIO_CONTROL_WIRE.md, AUDIO_RENDERED_WIRE.md and
`tests/fixtures/gp03/preview-v1`. The amendment exercises assist holds, proposals,
preview/cancel/refusal, paired scope+remaining renew, actual48-frame pending/final
release and240-frame endpoint, plus expiry at boundary with a live renewed lease.
Strict decoding checks every response. The accepted original E03 rendered corpus
bytes are unchanged. Client preview deadline is first send plus returned remaining,
never receipt+2000 or retry extension. Root accepted the exact amendment at
pass3/A/checkpoint-GP03-preview-data.json; Desk interoperability is a separate
consumer review, with provider fixes retained in this owner checkout.

Normal replay protects both corpora. One-time generators are ignored and their
on-demand commands are documented beside each corpus; the preview generator ran
explicitly because its evidence was added. Historical/private-media research,
exhaustive matrices, long benchmarks, UDP capacity opt-in, hardware-host/native
GPU, devices/live audio/DMX/MIDI/display/load classes were intentionally skipped.
The default local service remains opt-in, same-UID private Unix only,4 clients,
64KiB frames,32 replies/client plus latest telemetry,2000ms incomplete/stalled
limits and a48-frame synthetic pump. No TCP/production remote auth, service
installation, host/PA/FX/REC/analysis integration or physical safety acceptance.
Source remains uncommitted/unpublished; prior implementation/planning is preserved.

Final pass3 validation: warning-denied all-targets Clippy passed; the single normal
release `gigpies-headless` build passed with existing profiles/target. That actual
release process demonstrated grant/set rendered completion, client loss with mix
retained, fresh snapshot/new writer grant, assist/proposal/preview, paired renew,
release boundary completion and removed hold, plus same-epoch CLI refusal without
listener or identity change. The private `release-demo.py` command/transcript and
exact binary SHA are retained in checkpoint-GP03-local-R3; no repository runner
or new publication-policy entry was introduced. Complete candidate publication
index guard passed245 entries without changing the live index. Build slot released
at15:06:54 UTC; normal shared target grew about85MiB this pass to10,942,783,734
bytes, with27GiB free. Existing target and debug/profile choices remain intact.
Temporary owned demo endpoints and child processes were cleaned after evidence.
Local provider artifact delivery and actual Desk interoperability still require
coordinator acceptance; no earlier pre-guard evidence is substituted for this run.


Root and independent reviewer accepted the complete synthetic local provider and
exact guarded release artifact at15:10:05UTC, pass3/A coordinator-review-GP03-local-R3.
Root delivers that binary to Desk; actual Desk interoperability remains its own
consumer acceptance. The final documentation-only clarification states that
successful applied commits retain pending timing, while fully correlated terminal
non-applied outcomes may carry null ticket/frame/ramp because no render commit
occurred. The accepted producer expiry corpus already exercises that behavior;
no source behavior, corpus bytes or executable changed for this clarification.

## GP-09 task0009 checkpoint — injected native role leases

`src/roles/native.rs` and `gigpies-role-lease` implement the descriptor-only private
assignment plane beside the accepted C-ROLE:1 pure model. Both surfaces consume a
versioned executable/JSON-line protocol, not copied role algorithms. Live locks,
lease-specific generations, independent global CAS updates, retained crash intent,
explicit forget/reassign, strict private storage and inode verification are detailed
in [ROLE_BINDING.md](../../reference/ROLE_BINDING.md). `tests/gp09_native.rs` and the versioned
invented corpus cover the independent-process boundary. Root accepted corrected
R3 source and protocol; GP-H2 physical enumeration/open and windows/controllers
remain deferred. GP-04 is
separate next work and is not implemented by this checkpoint.

GP-09 software validation:18 focused tests;282 complete normal production tests
with `hardware-host` (zero failures,7 intentional historical/corpus/socket ignores);
38 Python tests; matching warning-denied Clippy, formatting and complete host-enabled
release build. An independent client replay passed8 exact corpus steps against the
retained release executable, covering concurrent audio/lighting leases, unchanged
live generation after other-role updates, kill/reclaim and retained EOF intent.
Root owns artifact promotion and Desk/Lightdesk integration; no hardware or source
publication is claimed by this worker. Reproducible standalone commands and lifetime
obligations are in ROLE_BINDING.md. Historical media/generation/UDP/exhaustive/load
and physical acceptance tests were intentionally skipped; normal host tests opened
no endpoints. Existing shared target output was retained; no shared cache cleanup.

## GP-04 task0009 — named bounded raw input subscriptions

The GP04-local:1 proposal in [ANALYSIS_STREAM.md](../../reference/ANALYSIS_STREAM.md) supplies actual
signed PCM24 synthetic input to the real eight-input mixer and its observer tap.
Explicit `--synthetic-source fouraux` selects the source independently of analysis;
`--analysis` attaches the named four-source observer only when available. GPA1
packet identity/format and the default GP03 source are preserved. Two-window SPSC
staging, an independent private Unix transport thread and per-reader whole-window
loss bound analysis work without making audio/REC wait. The same-host oldest
first-packet acquisition timestamp, strict source/map/calibration identity and
100ms age limit prevent delayed PCM from becoming fresh on receipt. Thread
failure/panic is unavailable analysis while mixer frame/control progress continues.

R3 focused software validation passed11 GP04 core/allocation/actual-process tests
plus2 local slow-reader/hard-expiry/panic regressions, and38 normal Python tests.
The complete corrected normal host-enabled Rust suite passed295 tests with7
intentional historical/corpus/socket ignores. Formatting verification and
warning-denied host-enabled all-targets Clippy and the complete host-enabled release
build passed. The actual release process delivered1920 exact signed samples to an
independent private client, which observed202ms-old kernel-buffered data as stale
while another reader received10ms-old audio and provider progress continued. The
child joined and temporary endpoints were removed. Root accepted source/wire/data;
final style-only Clippy deltas/release artifact promotion are root reviewed. Exact E09 descriptor/hex corpus is in
`tests/fixtures/gp04/v1`; root owns central contract promotion and independent Lux
consumer acceptance. No hardware/shared-load or source publication is claimed.
GP05 remains a distinct subsequent pass after GP04 handback and owner artifact
acceptance; no owner algorithms were copied into this analysis adapter.


### Task0009 / A-GP05 / fixed-owner software integration

The actual GP03 render pump now has explicit hash-manifest activation of accepted
REC/FX/PA libraries. GP04 and recording share the unprocessed engine input block;
FOH feeds fixed wet-only FX, dry+wet feeds actual two-input/six-output PA. Additive
GP05-modules:1 read-only health leaves GP03 wire fields exact. FOH-leased recorder
commands share request-ID high-water/dedup with GP03, but take/operation identity
separately fences lifecycle outcomes. Create/finish run on a bounded worker,
observer status survives finish, and client/analysis loss never controls capture.

Source/timeline rejection marks an active take incomplete. Explicit discontinuity
cancels pending preparation, resets FX and recreates PA off processing, then
requires a fresh take. PA faults are latched and health remains physically
unverified. Writable PA/FX and acoustic/true-peak protection remain unavailable.

Final focused actual-library tests passed4 (including private UDS), the default
normal suite passed278, the matching device-free hardware-host normal suite passed301,
and Python passed38. Warning-denied feature all-targets Clippy and the normal
release build passed. Root independent runtime review accepted the source; the
actual release process demonstrated start/readiness, continued capture across
client/lease loss and terminal finalization. Final focused post-style checks passed10 including all4 explicit actual-library
regressions; final fmt check passed. Root Desk/Lux joint acceptance/publication
remain separate. Eleven feature-suite opt-in tests were intentionally skipped there;
the four GP05 actual-library tests ran separately with explicit accepted artifacts.
Historical media/corpus regeneration and UDP capacity experiments stayed skipped. Reproducible standalone commands and
provider commit/hash pins are in MODULE_GRAPH.md and the GP05 fixture corpus.
No source commits/pushes are performed by this worker.


## Coordinator final integrated acceptance

The worker checkpoints above retain their original handoff scope. The final
coordinator pass accepted both native software frontends, read-only module and
analysis compatibility, and the combined actual-process demonstration. Current
counts, reproduction and remaining physical/remote gates are in
[HEADLESS_INTEGRATION.md](../workflows/HEADLESS_INTEGRATION.md) and the
[execution checkpoint](MODULE_IMPLEMENTATION_MAP.md#execution-checkpoint--2026-10-04).
The complete Python suite now passes 39 checks, including the publication-guard
Rust-attribute regression; runtime source and accepted release hashes are unchanged.

## Configurable increment superseding GP-03's shape

The original eight-strip GP-03 milestone is preserved as compatibility evidence.
It is not a product ceiling. The 2026-10-05 modular composition includes configurable
strips/monitors/output maps, actual SHR PA v2, shared authority/clock recovery,
authenticated Brain control/media and dynamic Desk. Its requirement-to-code/test
acceptance is maintained in [MODULAR_ENGINE_ACCEPTANCE](../../acceptance/MODULAR_ENGINE_ACCEPTANCE.md).
Physical UMC1820/ADA8200 socket/clock and graph deadline qualification are distinct
next acceptance work, not implied by 16/32/48-input software tests.
