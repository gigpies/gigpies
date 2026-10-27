# GigPies module contracts

Decision set **GP-2026-10-04.1**, documentation only. These are agreed planning
baselines for the tasks in [the implementation map](../archive/plans/MODULE_IMPLEMENTATION_MAP.md),
not implemented APIs or new hardware acceptance. One owner changes a definition;
consumers propose changes to that owner and wait for a reviewed version. Existing
GPA1/GPH1/GPC1/GPK1 and versioned C symbols retain their exact behavior.

## Registry and acceptance

| ID | Version / status | Definition owner; producer → consumer | Normative source; tasks |
|---|---|---|---|
| C-SHOW | 1, READY for implementation | GigPies; coordinator → all core modules | This document, Show; GP-01, DS-03, LD-03, LX-03 |
| C-ROLE | 1, READY for pure model | GigPies; assignment registry → Desk/Lightdesk | This document, Roles; GP-01, GP-09, DS-02, LD-02 |
| C-AUDIO | 1, READY for offline model/codec/graph | GigPies; Stagebox authority ↔ Desk | This document, Audio; GP-02, GP-03, DS-03, DS-04 |
| GP07-processing | 2, accepted four-band contract | GigPies → SHR Desk; manual FOH EQ/dynamics | [Channel processing](CHANNEL_PROCESSING.md); preserves C-AUDIO:1 / GP03-rendered:1 |
| C-LIGHT | 1, READY for null-output authority | SHR Lux; Lux ↔ Lightdesk | This document, Lighting, plus Lightdesk CONTROL_CONTRACT.md arbitration; LX-01..LX-04, LD-03, LD-04 |
| C-REC | 1, READY for software lifecycle/status | SHR REC; recorder → GigPies → Desk | This document, Recorder; REC-01, REC-02, GP-05, DS-05; raw v1 remains RAW_RECORDER.md |
| C-FX | 1, READY for current ABI/descriptor; writable ABI unresolved B-FX | SHR FX; FX ↔ GigPies; Desk via GigPies | This document, FX; FX-01..FX-03, GP-05, DS-05; include/shr_fx.h and ARCHITECTURE.md own v1 |
| C-PA | 1, READY for current ABI/descriptor; configurable ABI unresolved B-PA | SHR PA; PA ↔ GigPies; Desk via GigPies | This document, PA; PA-01..PA-03, GP-05, DS-05; src/shr_pa.h and EMBEDDING.md own v1 |
| C-ANALYSIS | 1, READY for bounded subscription adapter | GigPies owns stream/tap identity; Lux owns lighting analysis | This document, Analysis; GP-04, LX-05, DS-03 |
| C-CUE | 0, DEFERRED, no first-wave consumer | GigPies; explicit named events → separately scoped owners | This document, Optional cues; GP-08 |
| C-REMOTE | 0, unresolved B-NET for new production remote writes | GigPies; Brain ↔ Stagebox | Existing AUDIO_TRANSPORT.md remains normative for prototype; GP-06 |

READY means enough decisions to implement the named software slice. Acceptance
requires provider code and consumer tests at exact revisions, not merely this
status. No shared framework/crate is selected. Implement in each owner, use exact
ABI artifacts or reviewed versioned fixtures; never sibling Cargo path dependencies.

Examples E01–E10 below are **normative semantic vectors**, not implemented fixture
files. The provider creates a small synthetic fixture corpus in its first schema
task: explicit inputs, prior state, commands and expected outputs/refusals. Record
contract ID/version, provider revision and SHA-256 of every file. Consumers copy
those reviewed data files with provenance, retaining the same hashes; they do not
retype examples or import provider source. Extra local cases may extend coverage.
Provider tests exercise real owner logic; consumer tests exercise decoding and
presentation against that corpus. Mock round trips alone do not pass replacement.
A change to a vector needs owner review, a versioned decision and all affected tests.

## Common envelope and local binding

New semantic contracts use bounded UTF-8 JSON outside real-time threads, with
snake_case fields, integer quantities and exact schema versions. Domain IDs are
ASCII `[a-z0-9][a-z0-9._-]{0,63}`; human labels are ≤128 UTF-8 bytes, without control
characters. `show_id` is a canonical lowercase UUID string. Epochs, revisions,
source frames, sequence numbers and request IDs are unsigned 64-bit integers,
encoded as decimal **strings** to avoid loss in consumers. No wrap: renew epoch
before exhaustion. Distinguish `null`/unavailable from a measured zero.

An envelope names `contract`, `version`, `show_id`, `module`, `epoch`, `writer`,
`lease`, `request_id`, `expected_revision`, `kind`, `body`. Observations use
`sequence` and measurement timing instead of writer authority. Reject unknown
versions, fields, duplicate JSON keys, invalid UTF-8/IDs, oversize arrays or trailing
content before changing state. Reject nonfinite numbers; quantities below are
integers. Limits: 64 KiB per message, depth ≤12, 64 targets/transaction, four
simultaneous writer sessions/authority, one pending mutation per writer, 64 cached
responses/writer plus a retained sequence high-water mark. Providers may advertise
smaller capacity and must reject larger requests atomically. Snapshots too large
for one message use ≤16 pages, each with the same snapshot revision and page count;
accept the complete set within 2 seconds or discard it, never combine revisions.
These are conservative implementation bounds, not throughput acceptance.

Use pure encoded-message harnesses first. The first **local** integration binding
is a private Unix-domain stream socket with a 4-byte big-endian payload length,
0600 socket in an owned 0700 directory, verified same-UID peer credentials, and
no pathname supplied by an untrusted client. A stalled reader is disconnected;
32 queued replies maximum, latest-only telemetry per subscription. No socket,
parser, filesystem call or blocking wait enters an audio callback/render operation.
An explicit test binding uses temporary paths; opening a production endpoint is
not a side effect of ordinary tests or starting a surface simulator.

Same-UID access is local process trust, not cryptographic remote authentication.
New Brain–Stagebox remote write framing/authentication is B-NET. Do not enlarge
GPC1 or tunnel the JSON through its scalar field. GPA1 audio is unchanged.

Grants are engine-issued, per named scope; UI selection never creates one. One
writer holds a scope. Initial lease duration is 2000 ms with renewals at 500 ms,
measured against the authority's monotonic clock. Grant responses include remaining
milliseconds, not a cross-host wall-clock expiry. Expiry removes permission for
new writes, preserving human holds/applied state. New session or authority restart
requires snapshot plus new grant; reject the old epoch/lease even if IDs match.

Reply kinds: `applied` (committed authoritative state, with effective frame/tick),
`accepted_pending` (lifecycle work started, not completion), `rejected`, `conflict`,
`busy`. Reasons include wrong_show, version, epoch, lease, scope, target, range,
stale_revision, reused_id, expired_id, unavailable and capacity. Refusals leave
state/revision unchanged. A repeated ID with identical semantic payload returns
the cached original outcome; changed payload is reused_id. Evicted IDs are refused,
never executed anew. After validating show/epoch/writer/lease, cache lookup precedes revision conflict
checking. New request IDs start at 1 and strictly increase per writer session.
Any uncached ID at or below its high-water mark is expired_id, including reordered
requests; replies from older cached commands cannot regress a newer client snapshot.
E03R: after request 41, request 43 at expected revision 13 applies revision 14;
late uncached request 42 is refused even with expected revision 14, leaving state
at 14. An exact retry of 41 returns its original revision-13 result without a new
write or rollback. A fresh writer session has a new identity, not a reset ID space
under the old lease.

Retry the identical envelope at 100, 250 and 500 ms while the lease is valid;
then show uncertain delivery and query a fresh snapshot. Never assign a new ID
merely to retry. Discrete Stop/GO/blackout/release must receive an explicit reply
or remain visibly uncertain. Coalesce only unsent absolute scalar intents for
the same target/context. Reconnect drops unsent/pending intent and requires fresh
input release/pickup; it does not recall a scene or replay an uncertain operation.
Control and telemetry use separate bounded queues. Observation age uses local
receipt time plus declared acquisition basis; wall clocks cannot prove latency.

## Show

C-SHOW is a compatibility manifest, not a merged engine scene. It records
`format: gigpies-show`, `version: 1`, `show_id`, `manifest_revision`, module IDs,
required contract versions and content identities of separately owned state.
No local hardware paths, keys, socket endpoints or controller serials enter a
portable show. Local role settings are C-ROLE. Use ≤16 modules; duplicate identities,
unknown mandatory versions and missing required modules refuse activation.
Unknown optional modules are shown unavailable and cannot receive commands.

E01: show `11111111-1111-4111-8111-111111111111`, manifest revision `1`, requires
`audio/C-AUDIO:1`, `lighting/C-LIGHT:1`; a lighting client with C-LIGHT:2 attaches
read-only with `version`, without changing either engine. Loading a matching
manifest is still not a scene recall, output arm, recorder start or authority grant.

Persistence uses private create-new temporary file, flush/sync, atomic replacement
and directory-sync error reporting, outside callbacks. Refuse future schemas;
explicit migration writes a new destination preserving the original. A crash
retains the last valid manifest. Engine snapshots include `durability: volatile |
checkpointed | error`; `applied` alone does not claim durable storage. Each engine
owns its checkpoint and safe restart. Shared manifest mismatch cannot overwrite it.

## Roles

C-ROLE owns private role assignment only: `audio-desk` and `lighting-desk`, each
with one display identity and one MIDI controller identity/profile. Display
connector plus EDID identity and controller stable identity must be verified;
enumeration order is never identity. Missing/duplicate serials require labelled
operator assignment, persisted with generation. Pure fixtures use invented IDs.

E02: controller `test-controller-a` at assignment generation `7` belongs to
`audio-desk`. A lighting claim at generation `7` returns `conflict`; audio retains
ownership. A stale release at generation `6` does nothing. Duplicate EDIDs keep
both displays unbound pending explicit choice; no automatic swap or focus steal.

Atomic exclusive claims use per-device OS locks plus a generation-checked private
registry update. A crashed process releases its lock, not the saved intended role.
Reclaim requires identity/profile check and a new generation. Each desk owns its
own input and LED worker/queue. Lux integrated mode must not open either desk's
MIDI outputs. Input overflow clears held gestures and requires release/pickup;
LED updates coalesce to newest desired state. No automatic outgoing preset writes.
No keyboard/MIDI note is forwarded to an instrument. Ordinary keyboard focus
remains the window system's single focus; no two-keyboard seat is promised.

GP-09 adds the independently consumed
[`gigpies-role-lease:1` process protocol](ROLE_BINDING.md), while the pure saved
C-ROLE:1 registry remains compatible. Each surface owns a separate child that
holds both device locks, verifies injected descriptors and persists a new grant
generation before replying. Static JSON grants confer no authority. Verify every
500 ms and stop input/LED/write authority on timeout, malformed response or child
loss; reacquisition requires fresh verification and input release. A live grant
generation is independent of unrelated registry updates. Ordinary release, EOF
and crash retain intended assignments; explicit `forget` uses the current registry
CAS and releases locks before a replacement acquisition. Lost native metadata,
changed descriptor pins, replaced lock inodes and capacity exhaustion fail closed.
The versioned provider corpus uses synthetic identities. Real enumeration, device
opening and dual-display/controller acceptance remain GP-H2.

## Audio

C-AUDIO makes GigPies the mixer/authority owner. SHR PA protects speaker feeds;
SHR DAW supplies references, neither owns the complete GigPies band mixer.
Initial offline graph: eight mono inputs, stereo main and two mono pre-fader
monitor buses. Capacity negotiation may advertise fewer; 36 Desk fixtures are
not 36 real inputs. Both monitor taps are after the channel mute envelope and before FOH fader/pan.
FOH fader/pan changes do not affect monitor sends; channel mute silences main and
both monitors with the same 240-frame mute envelope. Per-monitor send level is
independent. E03M: input-01 alone, send-1 at 0 mdb, centered FOH at −6000 mdb gives
monitor-1 gain 1 and each main gain 10^(−6/20)/sqrt(2). Changing FOH to −3000 mdb
leaves monitor-1 gain 1; channel mute ramps both paths to zero. This first tap is
raw channel audio with mute; GP-07 must explicitly version its position relative
to added channel EQ/dynamics before routing them into monitors.
Full channel EQ/dynamics, PFL, scene/routing transactions and
multichannel devices are subsequent GP-07 work. First writable parameters:
`fader_mdb` −60000..12000 (step 100), `pan_percent` −100..100 (step 1), `mute` boolean,
and named monitor-send gains with the same mdb bound. IDs do not depend on order.
Use explicit mute for silence; −60 dB is finite gain. Pan is equal-power:
angle = (pan_percent + 100) × pi/400; left=cos(angle), right=sin(angle).

E03: audio epoch `9`, revision `12`, frame `48000`, input `input-01`, FOH grant;
Set fader from −6000 to −3000 mdb with request `41`, expected revision `12`.
At the next 48-frame boundary accept revision `13`, target −3000, ramp_frames
`240`, effective_frame `48048`. Ramp linear amplitude from the current coefficient
to 10^(−3/20), exactly at its end; report ramp target separately from current gain.
An identical request returns that outcome; −3001 is range/step refusal, expected
revision `11` is conflict, wrong monitor scope is scope refusal. Pan changes ramp
both coefficients over 240 frames; mute ramps gain to zero over 240 frames.
Safety fault silence can be immediate. This is software policy awaiting tests.

Manual edits create persistent per-parameter holds. MANUAL applies no musical
automation; ASSIST only proposes; AUTO requires explicit bounds. A mode change
captures current values into holds, preserving mix. Mute cannot be undone by
automation. Return-to-auto requires an engine preview tied to revision/scope,
valid 2000 ms on that authority clock, showing destination and ramp ≥240 frames.
Cancel/expiry/conflict preserves holds. Protection never becomes automatable.
Engine keeps applied mix/holds when a surface or control connection dies.

Observations specify tap ID, unit, epoch, frame interval, sequence, validity and
age. `sample_peak_mdbfs` is sample peak, never true peak; silence is explicit
`silent: true` with value null. Meter values expire after 250 ms receipt age.
Do not infer zero gain reduction, healthy recording or real FFT from absence.
Initial snapshots preserve actual value, target, proposal and owner separately.

## GP-METER:1 measured audio

This independent read-only extension leaves C-AUDIO/rendered1/2 `meters: null`
and the existing GP07 gain-reduction contract unchanged. The strict request and
reply are specified in [meter_wire.rs](../../src/meter_wire.rs) and the producer
[schema/corpus](../../tests/fixtures/gp-meter/v1/README.md). The sole implementation
and acceptance record is the [GP-METER card](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-meter--measured-audio-on-the-real-desk).

Requests use `contract: GP-METER`, `version: 1`, `kind: meter_snapshot`, show/module,
source epoch, query ID and expected map. Writer, lease, request ID and expected
revision are required nulls. Local same-UID and remote authenticated readers use
this identical payload; the latter retains the existing session envelope and
policy revocation checks. Reads never admit control, grant/renew a lease, refresh
paired/processing state, rearm or replay mutations. Unsupported versions receive
a bounded unavailable result. Replies are solicited only.

Each complete nonoverlapping window contains `sample_rate / 50` source frames.
Raw and actual pre-fader strip samples are ordered `input-NN:raw`,
`input-NN:processed`; then `main-l`, `main-r`, then each configured `monitor-N`.
Main observes the accepted wet/FOH-talkback sum before PA; without owner modules,
it observes the actual dry main. Monitors observe post-send/output gain and
monitor talkback before physical patch. Physical mapping stays in structural
readback. This is sample peak/RMS, not true peak, converter clipping, SPL or PA
protection evidence.

Peak and RMS use integer milli-dBFS, rounded with a -120000 floor. Silence has
numeric floor values and `silent: true`; null levels mean invalid/unavailable.
`below_floor` and `over_range` describe peak amplitude. Exactly full scale counts
as a clip; clips and nonfinite samples are counted per window. Nonfinite samples
invalidate that tap; downstream faults/quiescence invalidate affected taps without
inventing measured silence. A valid shared mute produces measured zero. Scaled
sums of squares avoid finite over-range overflow. Decimal-string u64 fields
preserve frame, sequence, epoch, generation, age and loss identities.

Two preallocated SPSC slots drop whole windows on saturation. Sample observation
and handoff contain no allocation, locks, I/O, strings or serialization. LocalAudio
owns reset on source/map changes; control-side draining retains only the latest
complete window and quantizes/serializes it. Existing LocalAudio control I/O is
not a realtime-safe callback. Independent topology meter admission includes the
accumulators, capture scratch and serialization reservation; failure disables
telemetry while preserving audio admission and legacy resource documents.

Meter documents use the existing immutable page protocol, at most sixteen 8192-byte
segments and 64 KiB frames, within the existing 1 MiB outer assembly bound. Prepared
capacity reserves 512 bytes per tap plus fixed envelope space and refuses telemetry
that cannot fit. This telemetry bound is independent of product input/bus capacity.
The observer has one outstanding read at no more than 25 Hz, with one 100 ms
query/assembly deadline. Provider acquisition age is from the first sample's
monotonic timestamp; Desk adds the entire query elapsed time and time since receipt.
Freshness expires at 250 ms from that start. Repeated windows cannot move their
previous expiry or reduce conservative age. Source stalls remain stale even when
control responds. Desk holds a clip for one local second only while data is fresh;
repeated windows do not relatch it. Identity loss clears the cache and requires a
matching topology and new complete observation. The dedicated observer adds one
session within the unchanged deployment budget.

## Lighting

C-LIGHT authority belongs only to Lux. Start with a **null-output**, static rig;
no uDMX worker or LED claim. Support ≤32 fixtures, ≤8 playbacks, ≤32 static cues
and ≤32 copied palettes, ≤64 attribute targets/transaction. Negotiate actual limits.
Fixture IDs remain stable through reorder; patch_revision is separate from state
revision. Validate complete patch, address footprint (one 512-slot universe first),
overlap, mode, defaults and capability groups before replacement. Invented fixtures
are explicitly synthetic; real personalities require independently verified maps.

Retain Lightdesk units: intensity and RGB are 0..1000 tenths of a percent;
pan/tilt/zoom are tenths of degrees within the capability's advertised interval.
Synthetic default pan ±2700, tilt ±1350, zoom 50..450 are fixtures, not universal
hardware ranges. Color and position coherent groups commit atomically; RGB here
means logical emitter proportions, not calibrated photometry. Lux owns conversion.

E04: playback 1 intensity 700, playback 2 intensity 500 ⇒ resolved 700, provenance
lists both contributors and winner 1. A touched programmer value 0 ⇒ resolved 0,
source programmer even though playback remains active. Clear-to-Hold moves that
0 into Hold, removes it from recordable programmer and preserves output. With
master 500, unheld base 700 ⇒ final intent 350. Blackout ⇒ intensity 0 while
retaining color/position. Submitted/observed stay null for the null sink.

Equal-priority playback intensities merge HTP; ties list all contributors.
Non-intensity coherent groups use engine activation order, not packet arrival.
Playback level 0 leaves non-intensity active until explicit Off. Priority order:
programmer > Hold > explicitly granted AUTO > ordinary playback > fixture default.
Apply intensity masters then fixture-aware blackout/protection. ASSIST is only a
proposal. Mode/grant changes freeze current pre-master look; neither releases holds.
Record copies only programmer masks; update does not alter already-running copied
playback until another GO. Palettes are copies for v1, not live references.

E05: fixture 11 intensity Hold=0, destination playback=700, revision `20`.
Preview-release token `preview-7` ties show/epoch/patch/revision/targets, destination,
transition_ms=500 and authority-local 2000 ms expiry. Commit at revision `20`
removes only the reviewed hold and schedules a 500 ms linear logical-intensity
transition; any state/patch change invalidates the token. No client-computed
preview is accepted. Native live zero-time release requires explicit zero choice;
the existing simulator's immediate release is labelled mock-only. Integer output
rounds nearest, halves away from zero. Advance Lux on injected monotonic elapsed
microseconds, fixed 10 ms logical ticks; UI redraw never advances engine time.
Late updates compute the current transition position without replaying old flashes.
LX-02 may expose preview-only tokens; release_commit stays unavailable until
LX-04 implements the declared timing. Moving-fixture paths/effects remain unavailable until LX-04's reviewed extension.

Snapshots distinguish selected (client), programmer, Hold, stored, playing,
resolved, final_intent, submitted and observed. Include per-attribute source,
contributors, inhibit/clamp and output state. Preview/ACK is never physical light.
Blackout persists across UI loss. Engine restart loads validated durable state,
cancels transients, starts disarmed, reports physical state unknown and requires
explicit reviewed rearm. Persistence of programmer/holds/cues/master/blackout is
LX-04; before it lands snapshots say volatile, never restart-safe.

Output-loss policy remains B-FIXTURE: UI/link loss preserves engine state; actual
engine/USB/power/fixture-loss behavior cannot be guessed. No generic zero-universe
or automatic rearm. Flash/strobe and richer effects are not advertised in v1.

The agreed opt-in LX05 extension uses schema `lx05-v1`; legacy listeners retain
their existing schemas. Named PCM analysis supplies read-only ASSIST proposals.
AUTO needs explicit mode selection plus a fresh, scoped intensity grant, bound
to the lighting writer/lease, patch, analysis attachment and completed calibration.
Calibration requires at least 50 contiguous windows, observed signal on all four
sources and a bounded 10-second soundcheck. Source age is measured from the oldest
packet's acquisition time, with a 100 ms limit. Beat/downbeat/harmony remain
unavailable.

Auto mode preserves genuine programmer/Hold masks. A separate intensity layer
freezes unmasked mode-entry values, then records active analysis or retained
analysis provenance. It stays below human masks and above ordinary playback.
A grant does not jump the retained value; a cap below that contribution is
refused. Fresh automatic updates move at most 20 intensity units per 10 ms owner
tick, never more than once per tick or by replaying missed updates. Grants expire
within 2 seconds. Source/calibration/identity/lease loss revokes authority before
another step and retains the exact layer contribution. Reconnection cannot rearm
it. Human values outside the automatic cap remain untouched. An overlapping
reviewed release freezes its underlying AUTO destination until completion, or
explicitly cancels/freezes the transition before changing that destination.
Manual/Assist transitions retain their existing continuity semantics; restart
restores intended looks disarmed with no active automatic grant.

The accepted owner [wire and output corpus](https://github.com/gigpies/shr-lux/tree/main/tests/fixtures/lx05/v1)
defines nullable analysis status and per-contribution source/window/calibration
provenance. Retained contributions keep their original provenance when a new
source attaches. Lightdesk consumes this schema read-only; calibration and AUTO
grant controls are not added to the surface in this increment. Normal opt-in
snapshots fit 15 pages, reserving the sixteenth for bounded loss/freeze state.
Automatic updates validate a complete candidate before commit and recheck source
age after analysis and encoding. Capacity refusal freezes the readable prior
contribution and revokes its grant.

The accepted Lux provider source is
[`1f0a5b4`](https://github.com/gigpies/shr-lux/commit/1f0a5b40d660f31971b1c8666d4c45723f0aadde).
Its matching release passed actual named-source calibration, explicit AUTO,
loss retention, human override and disarmed restart checks.

### Optional musical estimates

Lux `--musical-analysis` with an explicit `--analysis` source selects additive
`lx05-music-v1` or configured `lx05-music-v2`. These publish `lux.analysis.v2`
with a separate `musical` object; existing LX05 startup and fields remain
compatible. This extension consumes the same C-ANALYSIS raw PCM path. GigPies
does not duplicate harmonic analysis, and Lightdesk does not open audio devices.

Lux estimates twelve pitch-class strengths, major/minor chords, contextual key
and integer Camelot identity, signed fifths movement, and recurring arrangement
identities with tentative verse/chorus labels. Individual estimates may be null.
Confidence is a heuristic support score, not a calibrated probability. The old
`beat`, `downbeat` and `harmony` fields remain null; the versioned `musical` object
owns the richer estimates. Legacy consumers must not be connected to the new
schema without the matching consumer implementation.

`musical.source_frame` is the latest spectral window's exclusive source end,
not the latest received PCM end. A tracking estimate must be within 6144 source
frames (128 ms) of the current source-window end and within the same source
identity. The spectral window spans 256 ms; this is not zero-latency recognition.
Unavailable/stale input clears published musical estimates and temporal history.
Fresh calibration and the existing explicit authority gates are still required.
Harmonic tension/resolution can adjust the existing intensity proposal by at most
50 units; grant caps, slew bounds, programmer/Hold and loss retention still apply.
Section candidates are presented for interpretation and do not independently
trigger automatic lighting changes.

Algorithm and exact owner contract:
[Lux musical analysis](../../../shr-lux/docs/notes/0034-musical-analysis.md) and
[configured/music schema](../../../shr-lux/docs/notes/0033-configured-analysis.md).
The [GP-MUSIC card](../archive/records/2026-10-11-gp-music.md)
records synthetic software integration acceptance and qualification limits.

### Optional authored musical scenes

The additional `--scene-director` opt-in selects `lx06-scenes-v1` or configured
`lx06-scenes-v2`. It requires musical analysis and an explicit source. Legacy
startup retains its schema; older consumers must reject the new schema. The
matching Lightdesk client accepts both legacy and scene versions, with scene
commands available only on the latter.

Lux owns the bounded version-1 show map, change interpretation, deterministic
selection, local recurring-section memory and elapsed fades. Mood/palette RGB
and lift are authored independently. C-LIGHT's existing leased command envelope
carries atomic `scene_map_replace`, nonmutating `scene_preview` and finite scoped
`scene_grant`. Compact command replies and paginated owner inventory preserve
existing transport limits. Scene INT/RGB enters the existing automatic layer;
programmer/Hold, caps, calibration, source freshness and loss remain authoritative.
The private authored map survives restart; selection, section memory, transitions
and grants do not. Physical output remains null/disarmed and beat/downbeat remain
unavailable. See [musical scenes](MUSICAL_SCENES.md) for architecture, authoring,
the exact owner contract and the reproducible software episode.

## Recorder

Keep `shr_rec_v1_*`, raw format `shr-rec-raw` v1 and RAW_RECORDER.md semantics.
C-REC adds operation identity and a status seam; the Stagebox still supplies all
samples from its single device owner. A take ID is an opaque stable ID, not an
arbitrary remote path. Host chooses a create-new private destination and maps
ordered stem IDs to named input/tap IDs. Initial integrated rate is 48000 Hz,
PCM24 container, declared source_valid_bits or unknown, ≤64 channels subject to
advertised capacity. No shell/player capability is inferred from the raw library.

States: stopped → preparing → recording → finalizing → stopped, with explicit
retained complete/incomplete/error outcome. Arm edits only while stopped; unknown
or unresolved armed sources refuse Start. `operation_id` fences readiness/completion
from retired workers. A Stop while preparing waits for producer quiescence and
writer cleanup; late ready cannot revive capture. Playback and recording exclusive.

E06: Start take `take-01`, operation `17`, epoch `9`, mapping
`channel-001=input-01/raw`, `channel-002=input-02/raw`. `accepted_pending` means
preparation only. Worker ready then starts capture at frame `48048`.
`accepted_frames=96`, `written_frames=48`, `durable_frames=null` is valid progress;
received packets are not written frames and a write is not fsync. Stop requests
quiescence at a whole-period boundary, then finish drains/joins exactly once.
Only successful finalization plus valid matching results makes complete. Duplicate
Start never creates a second directory; duplicate Stop never finishes twice.
Operation `16` ready/completion is rejected without disturbing `17`.

Status contains take ID, operation ID, source epoch, state, ordered mapping,
accepted/written frames, known durable checkpoint or null, drop/gap/invalid/clip
counters, host fault and retained outcome. Progress ≤10 Hz on control worker;
no scanning growing files from UI. REC-02 adds a safe snapshot/query without
changing push ownership or v1 signatures. Overflow/disk/flush faults preserve
retained evidence; interrupted recovery always copies to a new destination and
remains recovered-incomplete. Recording continues on Desk/Brain loss; its status
becomes stale at the client. Start/Stop replies distinguish acceptance from completion.

REC-01/02 software acceptance adds the owner's `shr_rec_v2_observer_*` ABI in
[`shr_rec.h`](https://github.com/gigpies/shr-rec/blob/main/include/shr_rec.h),
without changing raw v1 or any v1 signature. Create an observer while its producer
is quiesced; create/retain/release occur off the callback. Each observer owns
independent atomic state and remains queryable after consuming finish. A snapshot
requires a live retained observer, an unaliased output and the exact output size;
release must not race use. Hosts retain take/operation/mapping identity separately.
Queries are bounded and limited to 10 Hz. Written frames cover all stems and the
timeline; durable frames remain unknown, including after finish. Terminal outcomes
cover complete, incomplete and error, including dropped handles and writer panic.
The owner's E06 corpus and direct C sample check establish software acceptance;
the standalone shell/player and physical recording gates remain separate.

## FX

E07 retains the exact v1 adapter: stereo interleaved f64, 8000..192000 Hz,
max block 1..8192, fixed 20 ms Digital Delay, feedback .25, damping .35, wet gain .5.
At 48 kHz impulse frame 0 produces first wet sample at frame 960. Report
`intentional_delay_frames=960`, `adapter_buffer_frames=0`; network admission is
separate. Descriptor identity `fx-a/fixed-delay-v1`; every parameter is read-only
until a new owner ABI is accepted. Supported reset clears history on gap/epoch/
worker restart; host rejects duplicate frames first and fades/mutes wet on error.
Never pass dry through a bypass or expose all sixteen standalone slots as embedded.

FX-01 adds `shr_fx_v1_capabilities` and `shr_fx_v1_status`, each requiring
version 1 and the exact caller output size (112 and 40 bytes respectively).
The owner header defines the fixed layout. Status is queried with a live,
exclusively owned, quiesced handle; it is not a concurrent observer. It reports
the last process result and persistent clear reason/count, including explicit
reset and rejected capacity, pointer or sample input. Query rejection leaves
output untouched, including overlap with engine-owned buffers. Capabilities
identify the fixed wet-only delay and unavailable writable/rack controls.
The owner E07 C corpus validates the release library without changing v1
processing signatures or DSP arithmetic.

FX-02 specifies writable v2 using the existing model's validated ranges/units and
prepared control mechanisms: wet bypass drains tails, panic clears, continuous
controls smooth, structural edits are prepared and retired off-thread. Replies
must distinguish command consumption from ramp completion and preserve engine B
when A changes. No speculative parameter range or C layout is frozen here.
B-FX asks which minimal prepared v2 control surface can expose the existing
f64 adapter/rack without violating precision, memory and bounded-retirement rules;
FX owns a reviewed header/descriptor example before FX-03 or writable GP-05 work.
Current fixed v1 integration remains usable while that decision is open.

## PA

E08 retains PA v1: stereo f64 → six logical f64 outputs; full-range 0/1 active,
2..5 silent, −1 dBFS sample limiter, 5 ms startup ramp, zero fixed algorithmic
frames. Rates 8000..192000, prepared frames 1..8192. Invalid pointers/bounds refuse;
numerical fault silences the whole block and latches until explicit recreation.
Host retains the library while handles live and never races process/destroy.
PA must follow the dry+wet sum when its ceiling is required. A sample limiter is
not calibrated speaker/true-peak protection. Descriptor advertises fixed preset,
logical outputs and unavailable configurable controls/measurement.

PA-01 adds `shr_pa_v1_descriptor` and `shr_pa_v1_status`. Both require version 1
and the exact caller output size: 80 and 24 bytes respectively. The owner header
defines the fixed-width layout. Descriptor output distinguishes two active logical
outputs from physical channels and identifies unsupported controls, measurement,
acoustic protection and true-peak limiting. Status reports the actual prepared
rate/block size and latched fault/recreation requirement. It requires a live,
exclusively owned, quiesced handle; this is not concurrent telemetry. Rejected
queries leave output untouched, including overlaps with engine-owned storage.
The owner E08 C corpus validates the release library and unchanged v1 behavior.

PA-02 decides a separate prepared configurable ABI using current schema v3,
linked L/R pair controls and `Handoff`/`Prepared`; preserve v1. B-PA is its exact
transaction/readback ABI, ownership/retirement and muted structural transition
contract. PA owns review; GP-05 cannot claim configurable three-way integration
before this passes. Phase/alignment remains PHASE_ALIGNMENT.md, not this API's
implicit capability. PA-03 develops synchronized offline measurement and refuses
unconfident proposals; setup mic never enters program/monitor routing.

## Analysis

The additive [configured named-source contract](CONFIGURED_ANALYSIS.md) selects
C-ANALYSIS:2/lux.aux.v2 explicitly, preserving the historical v1 contract below.
It binds four semantic sources to actual configured raw-strip IDs, with distinct
requests and startup-only source admission. Consumer compatibility requires the
separately reviewed Lux/Lightdesk amendment; it is not inferred from a producer
fixture. Physical acquisition-age propagation remains a separate gate.

C-ANALYSIS reuses GPA1 role=analysis/PCM24 and source-frame identity. It does not
make Lightdesk an audio client or duplicate Lux's source analyzer. Initial named
subscription `lux.aux.v1` maps exactly four stable source IDs in order: kick,
bass, guitar-1, guitar-2; explicit mapping/tap `raw-pre-fader`. The provider may
refuse unavailable sources; no channel-number guessing or silent substitution.
`lux.aux.v1` requires exactly 48000 Hz; other rates are refused before attachment,
with no implicit resampling. E09R: an otherwise matching 44100-Hz descriptor is
rejected and cannot advance Analyzer or change the held lighting look.
General band source sets can be a later negotiated version.

E09: descriptor epoch `9`, stream `3`, rate 48000, first frame `48000`, four ordered
sources and calibration_revision `2`. Lux assembles ten contiguous 48-frame
packets into its existing 480-frame/10 ms analysis window. A missing packet marks
the window invalid; never fill it with silence and infer a musical ending.
Reject overlap, reordered stale data, wrong epoch or a tap-map revision change.
On gap invalidate/reset temporal estimates, preserve manual lighting and hold the
current automatic look; resume proposals only after fresh calibration and the
owner's confidence gate. Do not infer beats/downbeats from energy alone.

Bound staging to two 480-frame windows/subscriber, at most eight subscribers in
the initial service. A slow subscriber drops whole windows and increments a loss
counter without backpressuring audio, recording or another subscriber. Feature
snapshots carry analysis/version, source/window frame interval, mapping/calibration
revision, validity, confidence 0..1000, acquisition-age limit 100 ms and loss counts.
Meters use their explicit units; unavailable estimates are null. Lux owns
calibration/feature extraction; GigPies supplies observed sample metadata. Optional
Desk analysis is read-only with matching tap/units; no FFT graph until a provider
actually publishes the declared measurement convention.

GP-04 supplies the accepted `GP04-local:1` private Unix subscription described in
[ANALYSIS_STREAM.md](ANALYSIS_STREAM.md). Its explicit `fouraux` synthetic source
feeds both the mixer and the raw tap; attaching analysis does not select or change
the source. A separate transport worker consumes the two-window SPSC handoff.
Each subscriber receives a descriptor followed by bounded 6,280-byte windows
containing ten unchanged GPA1 packets. Every window carries the first packet's
acquisition time in the same host's Linux monotonic clock. Consumers reject age
above 100 ms, future/regressed times, changed identity and discontinuities;
receipt time cannot refresh buffered old PCM. Partial writes have a hard age
limit and close the connection on expiry. Transport loss or worker termination
leaves the audio graph running. E09 contains exact invented signed PCM/hex
vectors and refusal cases for independent Lux consumption.

## Fixed module integration

The agreed GP-05 service uses a separately queried `GP05-modules:1` envelope on
the private audio endpoint. Existing GP03 envelopes and strict snapshot fields
remain compatible. Module status identifies the show, module, epoch, source
frame, actual library hashes and prepared configuration. Unsupported controls
remain unavailable; loaded libraries, successful processing and physical output
verification are distinct states. With no configured libraries, module health
is explicitly unavailable.

Recording writes require the connection's current FOH lease and revision.
GP03 and GP05 share request high-water and retry identity: an identical retry
returns its original admission/outcome, while a reused ID with another contract
or body refuses. Request identity is separate from the take/lifecycle operation.
Stop targets the active operation, and acceptance remains distinct from completed
preparation or finalization. Client or lease loss does not stop an admitted take.
The host chooses a create-new destination under its configured private root;
commands cannot supply filesystem paths.

REC preparation and consuming finish stay off the render path. Stop first
quiesces pushes, and the retained observer/library outlive terminal status queries.
Observer progress is cached at most 10 Hz. FX/PA queries occur with their handles
quiesced, reporting real descriptor/status results and faults. Raw stems and the
GP04 raw tap share the same eight-input source mapping. Fixed wet plus dry feeds
the actual PA main path; its sample limiter does not establish physical or
true-peak protection, nor protection of separate monitor outputs.

The exact [GP05 wire corpus](../../tests/fixtures/gp05/v1/README.md) retains integer
precision for counters and distinguishes unavailable, ready, degraded and faulted
states. A discontinuity cancels preparation or makes an active take incomplete;
an already finalized take remains terminal. Recording durability stays unknown,
including after finalization. Desk queries this status through a separate bounded
read-only connection, so module health does not consume mixer replies or delay
operator commands.

Accepted owner source revisions for this integration are
[REC `df93dc4`](https://github.com/gigpies/shr-rec/commit/df93dc4f8ca487e52d6f7c26ee9aa75462e8856b),
[FX `e501e09`](https://github.com/gigpies/shr-fx/commit/e501e09b6721340a95516abc86da2c0480b6ddb4)
and [PA `44a111f`](https://github.com/gigpies/shr-pa/commit/44a111f0e2e2d640d3fff1378a7f6131b3df3151).
Their headers and synthetic corpora belong to those repositories. Libraries are
built independently and loaded by explicit paths and checked hashes; no copied
provider algorithm or sibling path dependency belongs in GigPies or either desk.

## Optional cues

C-CUE stays disabled. E10 describes only an activation test: a named lighting GO
must produce **zero** audio/recorder commands without an explicit show mapping.
A future request needs show/epoch/event ID, target scope, expiry, individual
per-owner results and no cross-engine atomicity claim. GP-08 must review useful
consumers before selecting scheduling, retries and persistence. No macros to
arbitrary devices, shells or unbounded actions.

## Blocking decisions and review owners

| Blocker | Exact decision / owner | Affected tasks; independent work |
|---|---|---|
| B-NET | GigPies must choose authenticated production Brain–Stagebox transport, credentials/pairing and bounded framing; preserve GPA1 and prototype ABI | GP-06 remote writes; DS-04 live attachment; offline GP-01..GP-05 and all UI work continue |
| B-PA | PA must approve exact versioned prepare/apply/readback/lifetime interface | PA-02 decision, then GP-05 configurable PA; fixed v1 and PA-03 analysis remain independent |
| B-FX | FX must approve exact minimal writable f64 embedding and resource envelope | FX-02 decision then FX-03/GP-05 writable FX; fixed v1 remains available |
| B-FIXTURE | Lux plus operator: actual fixture models/modes/maps, output-loss and recovery look | Physical LX-06 only; synthetic LX-01..LX-05 remain READY by dependency |
| B-DEVICE | GigPies plus operator: verified two displays/controllers and eventual Stagebox/Brain assignment | GP-H1/H2/H3 physical gates; pure C-ROLE and UI tasks proceed |

B-NET/B-PA/B-FX require stronger design review by their named owners, not a user
questionnaire or two workers inventing opposite endpoints. B-FIXTURE/B-DEVICE
need concrete equipment facts when those physical gates are scheduled. No answer
is needed to begin the first software wave. Shared show versioning, units,
manual-hold policy, local framing, null-output scope and initial resource limits
are decided here; workers must not silently reopen them.

### LX05 release-capability compatibility

Analysis metadata does not imply timed release. Actual `lux-service --analysis`
without `--timed`/`--durable` advertises `lx05-v1` and `analysis` with no `release`
or `checkpoint` field. For this opt-in schema, consumers accept absent release
and refuse timed preview; when present, the complete existing release structure
must validate. Durable checkpoint capability remains separately validated.
Legacy LX03/LX04 required fields stay exact. `--durable` already enables timing
and must not be combined with `--timed`. The owner corpus includes the actual
[untimed inventory](https://github.com/gigpies/shr-lux/blob/main/tests/fixtures/lx05/v1/untimed-absent.json).

## Configurable composition contracts (2026-10-05)

C-AUDIO/rendered2 and GP07-processing3 explicitly advertise admitted topology;
legacy versions preserve their fixed inventory and refuse incompatible shapes.
GP05-modules2 reports dynamic raw tracks and actual PA v2 dimensions/configuration;
a legacy module query cannot silently consume a dynamic module shape.
GP14-structure1 carries separately scoped PA/patch transactions on the same
revision/lease/request history. GP-REMOTE1 carries these actual contracts and
negotiated GPA1 media inside mutually authenticated QUIC. Its pairing permission
ceiling never substitutes for a live authority lease.

The additive SHR PA C ABI v2 is owned by SHR PA and preserves v1 semantics. The
host retains its loaded library for every prepared/active/retired handle. It
validates native f64 dimensions, sample rate, block bound, ABI version/size and
owner limits before preparing the composition. Raw REC and stereo FX interfaces
remain unchanged; host adapters provide admitted REC tracks and explicit FX pairs.
See [composition](MODULAR_PROCESSING.md), [remote](REMOTE_PROCESSING.md) and
[owner ABI](../../../shr-pa/docs/EMBEDDING_V2.md) for precise boundaries and limitations.
