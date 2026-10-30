# FX aux control and legacy remote delay

## Four stereo auxes — GP-FX-aux:1

The four-aux path provides **four stereo FX buses**, separate from configured
monitor buses. Every logical input has a 0..1 send to each aux, initially zero.
Sends follow channel processing, fader, mute and pan, with 240-frame level ramps.
Stagebox sends eight explicitly ordered channels (`fx-1-left`, `fx-1-right`,
through `fx-4-right`) to Brain. Brain owns four native-f64 SHR FX instances and
returns eight wet channels on the same source-frame timeline. Stagebox admits
those returns through the existing bounded jitter/loss gates, sums them into
Main before PA, and never feeds them back into sends. Raw REC and monitor paths
are unchanged. Monitor sends keep GP18's independent taps and physical outputs;
they have no return to Main.

In the Brain runner configuration, set `fx_library` and `fx_aux_sha256` to the
exact library and SHA256. This selects four auxes instead of the existing stereo
path; `fx_control_sha256` remains the mutually exclusive legacy GP21 selector.
Both peers need explicit media permissions; Brain also needs `fx_configuration`
permission for owner control. Negotiation alone grants no operator writer lease.
Grouped media uses the existing GPA1 transport; network samples are float32,
converted to the owner's native f64 at its boundary. The source clock remains
Stagebox's; Brain processes complete received source blocks, never a free-running
render clock. Missing/stale groups reset affected continuity and returns fade to
silence through the existing bounded loss gates. All eight channels retain their
identity in transport; summation happens only at Stagebox Main.

An optional local source-host variant remains available for offline checks or
explicit local deployment: `gigpies-headless --fx-aux-library ABS_SO
--fx-aux-sha256 SHA256`, or provider configuration
`"fx_aux":{"library":"ABS_SO","sha256":"SHA256"}`. It excludes simultaneous
remote FX media. Without either four-aux selection, existing fixed local FX and
GP21 Brain stereo delay retain their behavior. The `hardware-host` feature is a
build requirement, not authorization to open a device; synthetic checks own none.

Loading hashes an open inode, loads that same inode, and retains its file
descriptor until the owner instances are destroyed. All four owners must
resolve the complete `shr_fx_v2_*` palette API, independently of the incompatible
`shr_fx_delay_v2_*` namespace. Settings/status/capabilities/parameter layouts are
64/200/48/48 bytes, ABI version2, 48kHz, 48-frame stereo f64 blocks. Algorithm
support, defaults and bounds come from the loaded owner, not a Desk sound list.
The adapter admits at most8MiB per prepared object; four active/four retired and
one caller-held pending object bound effect storage to72MiB plus owner structs.
The optional aux extension admits its additional send storage/work against the
existing mixer budget without changing published core resource capabilities.
Absent this mode, no per-input FX send state is provisioned. These are storage
bounds, not a four-effect CPU/deadline or listening qualification.

Use the ordinary eleven-key authority envelope with contract `GP-FX-aux`,
version1. `fx_aux_snapshot` has null writer/lease/request/revision and empty body.
`fx_aux_configure` requires the existing `fx_configuration` lease, authenticated
permission on remote connections, fresh same-session read, source epoch and
shared authority revision. The body contains exactly `bus` (0..3),
`map_revision`, `library_sha256`, `expected_generation`, `configuration_json`
and `panic`, plus `owner_binding` for remote owners. The binding is the SHA256 of
the exact negotiated media descriptor and library hash; local requests omit it.
This pins the Brain TLS session, source epoch, capability/map generations and
channel layout, so a reconnected owner cannot accept an old edit even when its
bus generation starts at zero. The hash input is serde JSON UTF-8 for the tuple
`[MediaDescriptor, library_sha256]` with the admitted stream order. Counters are decimal strings. Configuration and panic are exclusive.
Configuration is bounded opaque JSON: exact `settings` (the versioned owner
settings), `sends` in advertised logical input order, and nullable `tempo`.
Tempo is `{bpm,denominator,modifier}`: BPM30..300; 1/1 through1/64; straight0,
dotted1, triplet2. Digital/tape delay only. The owner helper validates the
resolved `time_ms`; values beyond1..2000ms refuse instead of clamping. Free time
uses `tempo:null`. Network `return_delay_frames` is a separate explicit transport
schedule, added to intentional effect delay; no unmeasured path compensation is
subtracted. There is no external MIDI-clock acquisition in this host mode.

For Brain, the authenticated `GP-FX-aux-owner:1` relay first requests actual owner
preparation. A permit commits authority, then schedules the send ramps and Brain
publication at the same future source boundary (4800-frame control lead). It is
not evidence of DSP application. Correlated owner completion, source send
readback and settled owner status are required before success; loss after permit
is UNKNOWN. A delivered permit is not rolled back on disconnection; fresh
readback merges actual Stagebox send targets with actual Brain effect state.
Preparation, deadlines and outgoing control queues remain bounded.

For the local variant, preparation and validation occur before reservation; at the exact source
boundary the existing authority rechecks the lease/revision before publication
and send ramps. One outstanding control edit is allowed; an individual aux must
settle before another configuration edit to it; panic can interrupt a transition
or clear a fault. An owner fault after remote publication yields UNKNOWN and releases
the outstanding ticket so an explicit fresh probe and panic can recover it. Unchanged settings do not replace its DSP when
only sends change. Rejected/stale edits keep sounding state unchanged.
`preparing` is the remote preparation request; `prepared` follows actual owner
preparation (or local admission). `accepted` is correlated publication/readback, and neither
is settlement. Snapshot exposes per-aux host generation/applied_generation,
complete target configuration and actual owner status (current/target,
accepted/applied request, transition, fault). Desk requires correlated acceptance
then fresh exact generation/target and settled owner readback. Lost completion
is UNKNOWN; there is no automatic retry/regrant/replay.

Provider bypass alone stops excitation on the next nonempty block and drains
history. Other settings use bounded 480-frame fade-out/swap/480-frame fade-in;
only one DSP state ticks per aux/frame. Panic clears the selected instance,
not persistent mute. If a replacement has not swapped yet, panic cancels it and
readback retains the actual active settings; an already swapped state remains.
Host control settlement after reset does not relabel an interrupted provider
request as applied. Fault status remains separate from control settlement. Failed process spans zero that aux return; other buses
continue. Source loss quiesces returns, clears histories and invalidates pending
intent; recovery requires a new epoch and preserves configured send targets while
outputs remain disarmed until explicitly rearmed. Aux configuration is runtime
state: UI reconnect preserves it; process restart or admission of a new Brain
owner session starts with zero sends. The
existing saved core-mix schema does not persist these auxes. Queries, preparation, reclamation and
library teardown stay outside sample processing. All instance calls serialize;
successful publish consumes the token, refusal leaves it caller-owned, and
retired tokens are freed by the control pump. Shutdown drops owners after the
source loop stops. UI disconnection does not stop already sounding effects.

Focused software checks use `tests/fx_aux.rs`; actual owner and authority/sample
checks require `GP_FX_LIBRARY` and their named ignored tests. Desk's repeatable
`fx_aux_actual` test launches an owned synthetic headless provider. The shared
[GP-FX card](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-fx--writable-owner-effects-in-the-console)
owns validation state; physical placement, aggregate cost and listening remain
separate acceptance.

## Legacy actual remote FOH FX — GP21-fx:1

This additive software path controls the actual Brain stereo owner processing
`foh-left`, `foh-right`. Each channel has independent delay, feedback, damping,
wet gain and bypass. It does not describe a generic rack or any audio device.
GP05 continues to report the unchanged local fixed fallback, including its
read-only parameter interpretation. PA remains after dry plus admitted wet;
raw recording branches before FX. Hardware acceptance remains separate.

## Admission and authority

The Brain runner's optional `fx_control_sha256` is the lowercase SHA256 of its
explicit `fx_library`. Omit it for existing media-only behavior. Resolve every
`shr_fx_delay_v2_*` symbol and exact capability/layout before advertising controls; old,
partial or incompatible extensions retain v1 media with unavailable GP21.
The published generic `shr_fx_v2_*` API has different signatures and is never
called through this source-timeline stereo-delay adapter. A library exposing
only that generic API retains v1 media; it does not advertise GP21 controls.
On Linux the loaded inode is the open file that was hashed. A replacement owner
needs a fresh authenticated media session. Media negotiation does not grant FX
write authority. Pair the actual owner and operator separately with explicit
`fx_configuration` permission; the operator must obtain a C-AUDIO version2
`fx_configuration` scope lease. Existing `fx` permission covers media only.

Use the standard eleven-key authority envelope, contract `GP21-fx`, version1,
module `audio`. `fx_snapshot` uses null writer, lease, request_id and
expected_revision, and an empty body. `fx_configure` uses the authenticated
writer and current lease/request/revision. Its body has exactly:

- `binding`: session, source_epoch, capability_generation, map_generation,
  owner_instance (decimal-string counters), library_sha256, abi_version2,
  config_size104, status_size168, capabilities_size128, and ordered
  channels `["foh-left","foh-right"]`.
- `expected_generation` and `expected_reset_count`: decimal-string counters;
  initial generation0 is valid.
- `lead_frames`: decimal string,48..48000 inclusive, divisible by48.
  Default operator policy is4800 source frames (100ms at48kHz).
- `configuration_json`: an opaque JSON string, or null for panic. At most4096
  bytes; exact object `{"channels":[LEFT,RIGHT]}`. Each channel has delay_ms
  1..500, feedback0..0.85, damping0..0.99, wet_gain0..1 and boolean bypass.
  All values must be finite; duplicate/unknown fields refuse.
- `panic_mask`: null for configuration, or integer1(left),2(right),3(both).
  Panic clears selected history and filter state; it is not persistent mute.

The integer-only outer codec is unchanged. Fractional DSP values occur only
inside the bounded opaque strings. The owner admits both channels atomically.
A same-session GP21 snapshot less than250ms old, exact binding/generation/reset
count, current actual-owner observation, settled owner and independent scope
are required. Neither GP05 nor ordinary mixer readback substitutes.

## Ticket and frame semantics

One mutation may be outstanding. Preparation reserves shared authority without
stalling dry audio or recording. Brain prepares the complete token off processing.
After PREPARED, the source owner revalidates lease, revision, owner and deadline,
commits authorization once at a source boundary and sends irrevocable PERMIT.
`permitted` means authorization intent only. It never means owner application.

Brain applies only immediately before the complete source block whose first
frame equals apply_frame. Late/missing permits, skipped target, source gaps,
stale generations and mismatched lifetime refuse; the frame never slides.
Preparation/ACK budget is2000ms measured by each node's own monotonic clock;
no raw monotonic timestamps cross machines. The lead policy is a software
acceptance bound, not a physical deadline qualification.

Configuration changes ramp/crossfade for960 source frames at48kHz. Readback
reports actual applied_source_frame, exact exclusive settled_source_frame,
applied/settled generations, next_source_frame, reset_count, remaining_frames
and target owner_json. Bypass ramps excitation down over20ms, then drains old
tails. Control settlement does not mean tail silence. Panic completion separately
pins its exact effective frame, selected mask and owner reset count increment.

Replies distinguish preparing, permitted, applied, settled, refused, cancelled
and unknown. Mutation replies include the original RequestContext, ticket,
apply_frame, current authority revision and actual owner observation. Completion
revision may exceed the authorization revision when another allowed control
commits after PERMIT; consumers must not assume exact expected_revision+1. Snapshot replies
include source show/epoch/revision/frame, availability, observation and pending
ticket. `authorization_replayed` is only the shared authority's historical
outcome, including after relay-history eviction. Its nested `applied` never
means DSP application. Read fresh actual-owner state to resolve uncertainty.

Cancellation is possible only before permit. A cancelled request consumes the
shared authority request ID without advancing revision. After permit, session
or ACK loss becomes unknown; never automatically resend, regrant or restore a
write. A new write needs fresh owner observation and explicit operator review.
The existing wet gate fences old sessions and fades to dry on return loss.
All token allocation/retirement, loading, hashing and JSON happen off the owner
process/commit boundary. Owner calls remain serialized.

## Validation

Normal focused tests cover strict parameter bounds, dedicated authorization,
freshness, binding, cancellation without dry interruption and lost-ACK unknown.
Explicit actual-owner tests additionally exercise independent channel samples,
exact960-frame settlement, panic, source ordering, legacy equivalence and old
library fallback. Run `cargo test --features hardware-host --lib gp21` normally.
Actual artifact tests require `GP21_FX_LIBRARY` and their documented `--ignored`
selection; corpus generation requires explicit `GP21_FIXTURE_OUTPUT` as well.
These tests are synthetic and never open a physical device.
