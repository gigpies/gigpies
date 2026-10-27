# Next session: dual consoles and integration

Current local handoff after **0.2.3**, 2026-10-04. The
[pre-implementation handoff](next_session-before-headless-2026-10-04.md)
is preserved. The preceding accumulated handoff is
[preserved unchanged](next_session-before-0.2.3-2026-10-04.md). Historical
execution prompts describe their original session scope; they do not authorize
new hardware tests or override the current module map.

For the next coordinated implementation thread, use the
[continuation and publication prompt](prompts/MODULE_CONTINUATION_EXECUTION_PROMPT.md).
It includes the accepted baseline, remaining software milestone, two independent
Pi4 sessions, build limits and explicit final commit/push instructions.

## First: continue from the accepted local integration

Read the [local integration instructions and evidence](workflows/HEADLESS_INTEGRATION.md),
[implementation map](plans/MODULE_IMPLEMENTATION_MAP.md), [Brain integration](../architecture/BRAIN_CONSOLES.md), [architecture](../architecture/ARCHITECTURE.md)
and [module ownership](../architecture/COMPONENTS.md). The intended Brain hosts **two 1920×1080
monitors and two independently assigned MIDI keyboard controllers**: SHR Desk
for audio and SHR Lightdesk for lighting. Manual operation stands on its own;
ASSIST proposes and AUTO requires explicit bounded authority.

| Owner | Current foundation | Concrete next work |
|---|---|---|
| `../shr-desk` | Real local GP03 client, scoped controls/previews/retries; 82 normal tests and actual engine IPC passed | Bind accepted actions/provider state into a native renderer and explicit controller roles; retain input/confirmation recovery |
| `../shr-lightdesk` | Real LX04 client, timed/durable state and commands; 81 normal tests and actual Lux IPC passed | Native renderer/editors/controller binding using accepted provider state; preserve Lux authority |
| `../shr-lux` | Null fixture/programmer/Hold/cue engine, timed release, bounded service and durable disarmed restart; 89 normal tests | LX-05 named analysis after GP-04; physical LX-06 only with explicit fixture/reservation |
| GigPies | Real eight-input offline mixer/private service; 243 normal Rust and 37 Python tests, joint console checks passed | GP-04 named streams, GP-05 PA/FX/REC binding, GP-09 native roles; GP-06 production remote design gate remains |

Read each sibling's README, blueprint/status and AGENTS before changing its code;
sibling writes need their own scope. Lightdesk must not become a second lighting
engine. Lux must not compete for its assigned pad LEDs. Controller identity and
active preset for the second device remain unverified. A stale or ambiguous
assignment stays unbound rather than sending notes to another desk/instrument.

Native HDMI/controller acceptance, fixture output and combined CPU/GPU/memory
acceptance are distinct gates. A drawn fixture or mock ACK establishes no DMX
output. Preserve source holds/current looks through mode/reconnect changes until
explicit release. Lighting failure must not block audio control or essential audio.

## Qualified stereo hardware handoff

Read [AUDIO_HARDWARE.md](../acceptance/AUDIO_HARDWARE.md) and [its owning plan](plans/AUDIO_HARDWARE_PLAN.md).
The selected USB host uses 48-frame / 1 ms processing, independent device capacity
and zero silent prefill, with real PA/FX/REC modules and sample-verified recovery.
The final ten-minute working-channel H8 run measured **5.19–5.23 ms**, with exact
software output and zero xruns/late wet returns. Two one-frame physical offset
changes remain unresolved. The historical 56 ms prefill/57 ms loopback is not the
current live-latency result.

The next useful physical check is a narrowly scoped USB transfer/feedback trace
around those changes under a fresh reservation. Existing one-second driver
snapshots are too coarse to identify the cause. Keep direct audio buffers fixed.
Read final H7/H8 conditions, retained xruns and the locked-page migration diagnosis
before extending the tested scope. Temporary settings were restored; no persistent
low-latency host profile was installed.

The prior session authorized the working channel. That does not grant a new
hardware session. The right return was about 69 dB weaker; verify a working route
before stereo physical acceptance. The USB microphone was absent. Acoustic PA,
clock, multichannel mixer, UI and complete-show acceptance remain separate.

## Offline listening handoff

The [summing plan](plans/SUMMING_PLAN.md) and [delivery contract](../guides/SUMMING_DELIVERY.md)
are implemented and offline-validated, now included in 0.2.3. The f64 summer,
channel choices and selected GigPies effects remain unchanged; no supported new
musical candidate was selected.

Use the retained private listening index
`artifacts/automix/summing-study/2026-10-03-engine/LISTEN.md`. It identifies six
complete mixes, seven established excerpts and exact historical comparisons.
Independent measurements range from −1.400441 to −1.395178 dBTP against the −1 dBTP
ceiling; the largest production/independent difference is 0.004922 dB. Deliveries
use static gain on the selected buses, with 0.4001 dB margin and no loudness target.

Obtain a fresh playback go, then record preference by passage and concern. Do not
rerender just to reopen this queue, or repeat historical planners without a new
hypothesis and stopping rules. Preserve source recordings, frozen settings, hashes
and useful evidence. Latest publication checks live in [VALIDATION.md](records/VALIDATION.md).
The unresolved earlier Pi crash still needs separate diagnosis; boot recovery and
zero current filesystem counters do not establish a complete offline scan.

## Peer work and resource ownership

Read `/home/shome/p/AGENTS.md` and [NODE_LAB.md](../development/NODE_LAB.md). Check the actual
hostname and live working trees; development coordinator/worker roles do not fix
the runtime Stagebox/Brain Pi assignment. The private ledger owns reservations,
exact source revisions and immutable review/acceptance records.

Use bounded `gigpies-peer` workers only within the current authorized scope; never
resume or take over an existing interactive session. Keep one owner per file and
resource. Shared load, hardware and restart experiments require a new explicit
reservation. Completed synthetic transport and stereo integration are recorded in
[transport](plans/AUDIO_TRANSPORT_PLAN.md) and [hardware](plans/AUDIO_HARDWARE_PLAN.md); do not
repeat them merely because an old execution prompt says to start them.
