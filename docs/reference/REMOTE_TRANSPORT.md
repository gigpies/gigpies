# Authenticated remote commands and media

For the proposed execution-isolated v1 and conditional direct-UDP comparison, see
[the audio transport blueprint](../architecture/AUDIO_TRANSPORT_V1.md) and
[GP-H1](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-h1--physical-audio-qualification).
Those plans do not change the implemented contract described here.

The `remote` module carries actual GigPies authority requests and GPA1 audio over
mutually authenticated QUIC connections. Quinn and rustls require TLS 1.3,
explicit CA trust and a separately provisioned SHA256 leaf-certificate permission
record. Early data and TLS session resumption are disabled. Bind addresses must be
explicit private or loopback addresses. Credentials, peer policies and synthetic
run reports belong outside Git.

`RemoteServer` authenticates the peer before constructing `AuthenticatedContext`.
Its permissions limit which existing authority scopes the peer may acquire; they
are not themselves write grants. Mutations use the session-derived writer and a
live engine lease. C-AUDIO readback is required before writes, and processing and
structural writes require their corresponding recent snapshots. Policy replacement
invalidates contexts at owner dispatch as well as on the network worker, including
commands already queued when revocation occurs. Disconnect cancels matching pending
engine and structural work without stopping local source processing or recording.

The server admits at most 16 concurrent authenticated or handshaking sessions.
This finite deployment budget accommodates six independently scoped control clients,
one Brain duplex worker, one source-clock FX worker, one independent analysis
observer, and seven control/reconnect-overlap slots. The previous four-slot budget could not admit even the
three separate operator-monitor, talkback-destination and talkback-FOH controllers
alongside both workers. This is a reviewed resource limit, not an input-channel,
monitor-bus or universal product capacity. A seventeenth connection is refused
without evicting existing sessions; dropping a session returns its permit.

The shared authority admits at most 16 simultaneous live writers, matching the
transport deployment budget. This includes FOH, operator-monitor, talkback-destination,
talkback-FOH and two independently scoped performer controllers concurrently.
Local and remote writers share this budget; authenticated admission is still not a
grant. Each live writer has bounded replay/maintenance state, and the separate
1024-session lifetime history bound remains unchanged. A seventeenth live writer
is refused without eviction; expired writers return live capacity. Topology and
scope permissions remain independently validated.

Per session, QUIC permits one bidirectional control stream and no unidirectional
streams, bounds receive/send windows to 65540 bytes and each datagram direction
to 256 × 1232 bytes. Authority handoff reserves 16 commands, 128 completions,
256 legacy media slots per direction and 64 Brain media slots per direction.
Thus the 16-session budget bounds aggregate queue slots to 256 commands,
2048 completions, 8192 legacy-media slots and 2048 Brain-media slots. Queue entries
have separately bounded payloads; these counts are not a claim of total process
RSS. Large control snapshot assembly retains its separate 1 MiB limit. Existing
authority read-state and Brain negotiation admission use the same session budget.
Memory, source-owner mailbox service time and reconnect isolation require validation
when raising this limit; permissions, leases, freshness and media ownership remain
independent of connection admission.

`authority_channel` connects a network session to the source/control owner through
bounded SPSC queues. `AuthorityMailbox::service` runs on the controller independently of source
capture progression in the remote runner. `HostAuthority` wraps the same `LocalAudio` used by local providers;
there is no second mixer or authority. Network workers never own the audio clock.
Generation changes drain completed replies, retire queued intent and require a new
connection and explicit grants. No command is automatically replayed.

After authenticated HostAuthority media negotiation, the proxy receives the sole
encoded-source packet consumer through its bounded event lane. Network polling
then drains the existing 256-packet lane without controller mailbox service.
Controller shutdown closes the lane, joins encoding and counts/destroys unconsumed
packets; a mutex coordinates that shutdown with network draining only. Source
rendering, encoding and atomic closure do not acquire it. Compatibility endpoints
keep their existing controller drain. Authenticated wet negotiation also transfers
an immutable stream/generation endpoint to the network proxy. It submits directly
to the existing source ingress lane; a producer mutex coordinates compatibility
submission, overflow accounting and shutdown only, never source rendering.
The source publishes its actual frame atomically for network late/future filtering,
including under receipt pressure. Source generation/policy/deadline validation
remains authoritative. Brain media
still requires mailbox service; this does not qualify combined timing or separate
analysis.

HostAuthority transfers its numeric RenderGraph and source media to one dedicated
`gp-source` worker across source quanta. Controller
readback uses copied safety, topology and sample observations; local aux preparation
and returned native retirement stay controller-side. Owned commands return through bounded global completion/retirement lanes; the
remote runner now starts autonomous source capture/render progression. Controller
service exchanges copied observations and recorder lifecycle without pacing audio.
Ordinary mixer, structural, local aux and Brain requests prepare for a future
source boundary; expired preparation refuses before application. Remote FX
permission uses that boundary; four-aux sends retain a separately fenced source
packet until their negotiated frame, then return an actual completion receipt.
Physical capture is
implemented with nonblocking partial transfers but remains unqualified.
Runner `source_clock_v2` reports retain exact output windows correlated with
source-applied revision and Brain routing/hold identity, plus cumulative wet/drop
accounting. Old controller-sampled window fields remain null. Synthetic epoch
reconstruction closes and drains the old owner, retires media identity and starts
a disarmed replacement on the same worker within the original run deadline.
Physical reopen and complete cross-owner/whole-source acceptance remain open.
Wet/talkback admission crosses additional prefaulted fixed ingress/receipt lanes
(256 wet datagrams and 64 mono blocks in each direction; 671,296 payload bytes,
excluding ring and endpoint metadata). Source insertion rechecks generation, policy and deadline;
full receipt lanes defer insertion while rendering continues. Accepted counters and
wet hashes advance when controller receipts are consumed, rather than on enqueue.
Final source shutdown closes network submission before draining receipts and
counts remaining queued admissions as rejected. Producer overflow counts are
collected exactly once with controller receipts; attempts after terminal shutdown
return an error without changing final admission totals. Receipt saturation still
bounds how long wet admission can progress without controller accounting.

Provider reports include `media_queue_drops` for GPA1 ingress/egress and GBA1
capture/monitor handoffs. Counters accumulate across retired sessions in one fixed
runner ledger and are sampled after networking stops. They are independent of
optional timing traces; clean timing acceptance requires all four to be zero.
They do not count socket loss or replace host deadline/refusal accounting.

Control frames use a four-byte big-endian length followed by strict JSON, at most
65536 bytes. Large replies use the existing GP14 snapshot-page protocol with a
1 MiB assembly bound. Duplicate keys and depth checks apply after reassembly too.
GP-REMOTE version 1 wraps the provider's exact C-AUDIO, GP07-processing, GP05-modules
and GP14-structure contracts; unsupported versions are refused. Legacy contracts
remain explicit and cannot truncate a dynamic provider into a legacy shape.

Media negotiation binds the fresh TLS-exporter session, source epoch, capability
and map generations, stable channel IDs, roles, encoding and source-frame delay.
GPA1 datagrams are protected by QUIC. Groups fit the negotiated QUIC datagram size;
there are at most 64 groups and 256 selected channels per role. These are bounded
transport resources, not a product channel ceiling. Analysis is exact PCM24, FX
send/return is float32. The composed host advertises 48-frame blocks; the standalone
transport also supports 96. Replayed, wrong-direction, stale-generation and late
wet packets cannot advance the source clock or enter a replacement source epoch.

The host taps admitted raw capture slots for analysis and actual pre-wet FOH buses
for FX. `BrainWorker` assembles source-frame groups and calls the owning FX library
through `host::BrainFx`; wet return enters the existing graph before PA. Its 960-frame
intentional FX delay is separate from negotiated network return delay. Missing wet
fades through `WetGate` while local dry processing and REC continue. Source recovery
clears previous media and fade history and requires explicit output rearm.

The production Brain runner attaches analysis on a separate authenticated connection
and receive runtime. Provider admission allows one analysis-only observer, with its
own encoder and four-block sample pool; it rejects control commands on that
connection. Pausing or closing analysis does not close FX or duplex. Source/map or
policy replacement fences its source tap immediately. Analysis and audio endpoints
still share the provider's existing QUIC runtime; no per-session worker thread or
extra session budget is introduced. Runtime reports include prepared media layouts
and pre-shutdown process/per-thread CPU, RSS, fault and scheduling observations;
opaque owner/TLS/allocator costs remain part of measured RSS, not guessed layouts.

Provider media reports retain the first and last rejected-late wet packet's
source epoch, packet source/output frames and source frame at admission. These
two fixed-size witnesses remain null until a late packet is observed and persist
with the cumulative counters across source recovery. They locate a refusal on
the source timeline; they do not measure network transit or identify whether
sender processing, scheduling or transport caused it. Admission and the negotiated
return budget are unchanged; no per-packet log or unbounded storage is added.

With `hardware-host`, `gigpies-remote --config PRIVATE.json` defaults to bounded,
device-free provider and Brain FX modes for functional acceptance. The synthetic
provider duration is at most60 seconds and its source runs independently of clients.
An explicitly configured physical provider additionally requires
`--activate-physical`; it advances the same authority from captured device frames
and reserves a durable source epoch. See [executable device composition](BRAIN_AUDIO.md#executable-device-composition)
for physical configuration, the separately bounded24-hour mode and source-agnostic FX.
Optional `record_take` starts the real REC owner, immediately releases its initial
FOH grant, and finalizes an explicitly incomplete take at source end. Initial
outputs remain safe until a client explicitly rearms them. Reports include exact
analysis sample comparisons, per-channel counts, packet hashes, wet acceptance and
source-frame evidence. These runs do not establish physical synchronization or
realtime hardware qualification.

After source/network quiescence, a requested recording gets a five-second absolute
monotonic observation deadline. Nonblocking lifecycle/status polls and asynchronous
sleeps (at most 5 ms, capped by remaining time) wait for the requested take's actual
terminal status. Preparing, pending, unknown, missing or malformed status cannot
produce success. Timeout or observation/terminal errors set the report fault used
by the CLI's nonzero exit, retaining the last observed status and any prior fault.
Expected source-end `incomplete` with host fault 6 remains allowed; no durable
counter or complete outcome is fabricated. Drain, fsync and publication remain in
the owning worker. This budget does not bound filesystem operations, report saving,
owner Drop (which may still join off the control path), or total process lifetime;
the external hard process deadline remains authoritative.

For bounded software diagnostics, a provider configuration may explicitly set
`"provider_timing": true`. It defaults to false and is rejected for physical
providers or durations above 60 seconds. The trace preallocates 377,057 numeric
64-byte slots (317,056 control and 60,001 source records, about 23 MiB), records
outside sample loops, and writes `provider_timing` in the final report. Final
report materialization needs additional temporary allocations. Disabled mode skips
trace storage, timestamps and statistics reads, while retaining fixed token
metadata in the existing bounded control queues. The enabled trace records
explicit session/request identities, control queue/dispatch/serialization/write
stages, connection-wide QUIC counters and aggregate source-tick costs. Deferred
completions have ordinal zero and retain their available request ID; they are
not attributed to a guessed request by queue order.

Trace timestamps use one process-local monotonic clock. Stage durations include
scheduling and preemption; they are neither CPU time nor one-way network latency.
Local write completion does not prove remote receipt. QUIC counters are aggregate
corroboration, not proof of a particular stream's stall cause. Capacity overflow,
incomplete records and remaining active tasks are explicit: a full trace may
truncate late-run control evidence, and shutdown's bounded 100 ms wait does not
prove complete teardown when `active_tasks` remains nonzero. The reported append
cost excludes failed-capacity attempts, token extraction, statistics reads and
timestamp-call overhead; per-event microsecond truncation also applies.
Desk's corresponding opt-in diagnostics are documented in its native frontend
contract. Instrumentation perturbs timing, and the partial append-cost measurement
is not its total overhead. These diagnostics do not relax authority or timing
requirements.

The `gigpies-remote` executable services controller work on the calling thread
and uses two Tokio worker threads for spawned QUIC and control tasks. Autonomous
source processing and the stereo delay/four-aux FX owners have dedicated threads. This keeps
network progress independent of synchronous source render/control work and burst
catch-up. Bounded SPSC authority queues connect network workers to controller
service; fixed host ingress/receipt lanes then feed source media. Network workers
neither render nor advance the source clock. This isolation is
not a realtime scheduling or physical latency guarantee.
An orderly map retirement preserves queued structural completions while stopping
Brain-media polling. A bounded FX run reserves its final return-budget interval
inside the configured duration as a connection hold after production stops. This
avoids discarding the last dequeued block when the mailbox retires on close; it
is not delivery acknowledgment. Reports expose `shutdown_hold_ms`; sent/admitted
packet equality still requires independent provider evidence. Fault exits remain
fail-closed and do not take this graceful hold.

Meter observations retain a40ms minimum production interval per authenticated
session. One early request may wait for that interval; a second outstanding read
is refused. The pending table is bounded by `MAX_CONNECTIONS`, is cleared on
disconnect, and rechecks authorization before producing a fresh snapshot. This
handles setup/arrival jitter without increasing the observation rate or forcing
a reconnect loop. Pending reads never reserve a writer lease or modify audio.

An explicit `"media_timing": true` enables a separate fixed 480,000-event lane
in bounded synthetic provider/Brain runs only. It adds about29.3 MiB of numeric
storage per enabled process; final JSON materialization costs additional memory.
Numeric trace pages are touched before source timing starts, avoiding lazy zero-page
faults on first append. The control lane accommodates four simultaneous Desk scopes
and owner traffic; source, control and media overflow remain explicit failures.
Output and revision evidence each retain at most2048 windows, covering60s of owner
observations plus100ms buckets and control changes. Brain sample windows retain
native floats during the run and convert to JSON only afterward.
The provider retains source/control records in `provider_timing`; Brain writes
`timing`. Source capacity includes the immediate initial tick plus60,000 timed
ticks, without extending the60-second run cap. Media records join by authenticated
session and source frame; the report/negotiated descriptor supplies source epoch.
They cover source send calls, application QUIC dequeue, mailbox admission, Brain
assembly/processing/send, pre-host refusal and queue pressure. Normal events sample
the first channel group; refusal/pressure events retain every non-analysis group.
Trace overflow remains explicit and must not be interpreted as missing wire media.
Stage37 records source-render thread resource deltas: source frame in ordinal,
CPU microseconds in request, minor faults in session, voluntary context switches
in kind, involuntary switches in a and major faults in b. These optional Linux
`getrusage(RUSAGE_THREAD)` reads add measurement overhead and run only in synthetic
media diagnostics. They distinguish wall time from accounted thread CPU; they
do not establish realtime scheduling guarantees.

QUIC dequeue is later than socket arrival; a send call precedes actual wire
transmission. Only timestamps from the same process may be subtracted. Provider
send-to-return-dequeue includes both directions, Brain processing and scheduling;
it is neither QUIC RTT nor physical latency. Host late counters cover host
admission only: transport/mailbox prefilters refuse packets whose output frame
is at or before their observed source cursor. Deadline equality is rejected at
all three boundaries; prefilter acceptance does not reserve final admission.
Clean diagnostic acceptance must also check those refusal events and correlated
source-frame completeness, not just the CLI exit status or host counters.

Focused normal tests: `cargo +1.97.1 test --locked --features hardware-host --lib remote::tests`.
Actual owner REC evidence is opt-in with `GP05_MANIFEST` set to an accepted manifest:
`cargo +1.97.1 test --locked --features hardware-host --lib actual_host_recording_survives_brain_loss_and_finalizes_clock_fault -- --ignored`.
Run the complete normal default and hardware-host suites for integration changes.

## Brain duplex audio extension

[Brain audio](BRAIN_AUDIO.md) adds GP15-brain control, GP15-device configuration
relay and GP15-media negotiation over the same authority and mutual-TLS protocol.
The separate duplex owner carries GBA1 talkback and operator-monitor packets in
independent bounded queues; the source-clock FX owner and independent read-only
analysis observer remain admitted. FX permission does not authorize talkback. Each crossing uses the
[bounded ASRC bridge](BRAIN_AUDIO_BRIDGE.md), with both device epochs and map
identities. The Stagebox source clock remains the processing reference; Brain's
local duplex card has its own clock. This extension is undergoing integrated
software acceptance; it does not confer physical qualification.

Monitor packet identity follows selection generation; talkback identity follows
held-action generation. An unrelated PTT change therefore preserves monitor
continuity, including already queued monitor packets. Queue preservation requires
an unchanged authenticated owner/session, both device epochs/maps, monitor
selection, format and enabled monitor role; talkback bridge replacement is independent.
Preserving the same live authenticated talkback bridge also preserves its ingress
generation and replay cursor; replacing its identity refuses old queued blocks.
A newly negotiated monitor identity or disabling the role discards those queued samples.
At a source selection/disarm change before renegotiation, a still-live authenticated
duplex owner instead receives a bounded silent retirement of its old monitor identity.
The source publishes copied monitor blocks to a prefaulted 64-entry SPSC lane;
media polling encodes GBA1 packets. Full publication refuses the newest block and
increments the monitor-drop counter. During retirement, media polling encodes
queued samples as exact zero while preserving identity, source indices and order;
new source blocks also contain zero samples for each actual 48-frame source block. It never sends the new selected tap under the old identity. Retirement
ends at the first of 250 monotonic milliseconds or 12,000 source frames from the
first changed-selection block; repeated changes or old negotiation retries cannot
extend it. Missing/revoked owner permission, stale (over 250 ms) or faulted/disarmed
device observation, or changed source/device epoch or map closes the stream.
New negotiation retains the existing invalidation, fresh prefill and explicit rearm
requirements. Packets already transferred to the proxy/network and samples already
submitted to a device cannot be withdrawn; this is not an atomic cross-node mute
claim. ASRC counters, timeline checks and acceptance thresholds are unchanged.
Control negotiation still has its own monotonically increasing
acceptance generation. Device replacement requires a matching epoch/map observation
acknowledgment, closed readback, and a subsequent explicit arm revision. No stored
intent or queued pre-replacement snapshot can authorize playback.

`capture_queue_dropped` is an unsigned count of48-frame microphone blocks refused
by the bounded capture queue since the duplex process started. The runtime and producer fixtures share its telemetry
encoder; consumers validate it explicitly alongside the other device counters.

All wire telemetry uses the existing strict integer JSON grammar: amplitude in
nano units, output/input ratio in parts per billion, signed estimated skew in
parts per billion, nominal queue/filter latency in microseconds, and nullable
physical-mapping uncertainty in milliframes. These are neither measured one-way
network latency nor proof of hardware clock lock. Producer fixtures live under
`tests/fixtures/brain-v1` and `tests/fixtures/brain-device-v1`.

## GP15-held-proof:1 additive controller read

HostAuthority implements a strict standalone proof envelope (8192 encoded bytes),
restricted to authenticated TalkbackDestinations with an exact live writer lease.
The request echoes attachment generations and a reviewed configuration digest;
its positive query_id is separate from mutation request IDs. Successful query IDs
increase per authenticated session, with at most MAX_CONNECTIONS tracking entries,
removed on disconnect. Refusals do not advance the accepted query ID or freshness.
No transport framing, QUIC window, task topology or mutation deadline changes.

The reply context is the entire exact request. A proof contains one committed
revision/source frame and a fixed scalar witness; a refusal has no witness and
one of identity, permission, scope, lease, clock, query_id, config_changed or
capacity. Malformed/unsupported envelopes use existing outer transport refusal.
All authentication, independent FOH permission and render-boundary mutation checks
remain authoritative. A compact proof cannot replace full raw UI/configuration
readback or authorize a grant. See [Brain control](BRAIN_AUDIO_CONTROL.md#compact-held-proof-gp15-held-proof1)
for digest/lifecycle boundaries and producer fixture generation. Implementation
is not final combined-network or physical acceptance.


## Atomic controller operations

The composed `HostAuthority` adds `GP15-lease-maintenance:1` and
`GP15-paired-readback:1` over the existing authenticated control stream and paging.
Their requests bind the current TLS session-derived writer, show/source epoch and
capability/map generations. No payload identity creates authentication. Current
policy and exact scoped live lease are rechecked before replaying maintenance;
read-only paired observation requires no write grant.

Maintenance accepts all canonical existing C-AUDIO scopes: FOH, named monitors
1/2, dynamic `{"monitor": N}` for u16 N >= 3, PA configuration, output routes and
the three Brain scopes. The live lease must already match configured topology
and the exact requested scope; current `scope_permission` authorization applies
before every retry. Malformed/unknown scope forms and noncanonical monitor aliases
never maintain a lease. No monitor cap or new grant is introduced.

Maintenance is serialized with the existing owner. It only extends an admitted
existing lease, without a revision compare or revision increment. Replay lookup
precedes refusal of new work while any ordinary/processing/external transaction
is staged. Its per-live-lease 64-entry history follows current authority clones;
retries do not renew twice or outlive lease revocation. It never updates raw,
Brain or held-read freshness. Legacy C-AUDIO revision comparison remains unchanged.
The added scopes use atomic maintenance in explicitly requested GP15 paired mode,
not merely because an attachment has QUIC/held identity. Both passive and
pre-command renewal use that mode consistently; a failed GP15 attempt cannot
select legacy fallback. Non-GP15 renewal remains unchanged. Older providers may
refuse added scopes and clients must expose the refusal without regranting.

Paired observation captures committed raw and Brain state synchronously and
checks common revision/frame before encoding the whole response within 1 MiB.
Admission also accounts for the existing GP-REMOTE Reply wrapper inside that same
transport assembly limit; no extra topology clone is needed to size its overhead.
Raw and Brain read latches are admitted together only after successful construction;
the independent processing-read timestamp is neither replaced nor refreshed.
A separate monotonically increasing nonce map has at most `MAX_CONNECTIONS`
entries and clears on disconnect. No large response cache is kept. Maintenance
is bounded to 8192 bytes per request/reply. Existing frame/page/queue bounds stay
unchanged. Unsupported versions fail closed through the existing outer refusal.

Focused producer validation (under the shared build lock, nonincremental, one job):
`cargo +1.97.1 test --locked -j1 --features hardware-host --lib atomic` and
`cargo +1.97.1 test --locked -j1 --test atomic_control`.
These tests are normal production regressions. Full integration suites and network
acceptance remain the coordinator's subsequent combined-source gates. See
[Brain control](BRAIN_AUDIO_CONTROL.md#atomic-lease-maintenance-and-paired-readback)
for consumer deadline and held-proof separation requirements.

### Bounded ingress preparation

Proxy GPA1/GBA1 ingress now copies into inline `MediaDatagram` ring entries;
dequeue/overflow cannot free packet payload storage. Existing 256/64 capacities
remain unchanged. `media_queue_drops()` on proxy/mailbox returns separate ingress
and egress drop counts regardless of diagnostic tracing. These are per-channel
observations; the runner also aggregates them across retired sessions in its fixed
lifetime ledger. Neither form replaces end-to-end trial accounting.

Brain grouped FX assembly uses 32 prepared sample slots, an exact group bitmap
and `push_into` with a reusable completion buffer. Incomplete eviction, duplicate,
stale and shape refusal paths are bounded. Compatibility `push` remains allocating
for non-audio consumers. For stereo 48-frame delay and four-aux operation, assembly, encoding and hashes
run on the media worker; native execution is separate from controller preparation
and retirement. Other legacy shapes retain the compatibility worker. These
ownership boundaries do not establish complete acceptance of the
[dedicated execution design](../architecture/AUDIO_TRANSPORT_V1.md) or T1/T2
acceptance. The normal `audio_transport_v1` test target owns literal wire vectors,
checked payload budgets and assembly allocation/destruction guards.
