# Intended architecture

GigPies includes an **audio digital mixing surface and a lighting control surface**
for human operation. Both run on one Brain, with a separate Full-HD screen and
MIDI keyboard controller for each. Automation is an optional mode within them.

![GigPies system and module map](../assets/architecture.svg)

## Brain local audio and two device clocks

Brain owns one duplex sound-card endpoint for operator talkback capture and
monitor playback. Stagebox owns its own interface and ADAT expansion. These are
independent clock domains unless physical synchronization is separately proven.
Nominal 48 kHz, network timestamps and NTP do not prove converter lock.
Stagebox remains the reference for channel DSP, mixing, PA, raw recording,
analysis and source-indexed FX. Brain local I/O uses its own epoch and frame IDs,
with bounded asynchronous conversion at both talkback and monitoring crossings.
Desk controls the device owner; neither Desk nor the FX worker opens another PCM.
Current work and acceptance gates are recorded in [Brain audio](../reference/BRAIN_AUDIO.md).
The [previous architecture](../archive/brain-audio-before-2026-10-05/ARCHITECTURE.md)
is preserved.

## Configurable processing engine

The existing Mixer and authority now provision logical strips and independent
monitor buses from validated topology. Explicit `legacy8-2` compatibility retains
historical fixtures; new16/17/32/48software profiles use the same strip processing,
prepared boundaries, authority and raw source identity. Each admitted input retains
four parametric EQ bands and compression. Physical stream width, logical input
count, PA output count, monitor count and control-writer capacity are separate
quantities, each with a declared admission bound.

Topology separates logical strip IDs, physical socket names and configurable USB
slot indices. The UMC1820/ADA8200 manufacturer reference has16logical analog inputs
and18analog outputs; its expected USB permutation is unverified and its output
patch initially empty. It never implies an observed Linux map, a fixed PA/monitor
socket partition, or physical converter synchronization. The attached reference
rig must be verified independently before activation. Measurement slots cannot
implicitly enter program paths, and unmapped outputs remain silent.

The common source adapter maps a single interface's capture into logical raw strips,
runs the same authority/Mixer/module graph as synthetic input, and maps declared
logical destinations back to playback slots. Recorder and selected raw analysis
remain independent of channel DSP and Brain availability. Structural changes are
prepared off render and require reviewed output quiescence; explicit recovery
retains intent but never grants permission to unmute. Clock health distinguishes
the source epoch/frame timeline from unknown physical ADAT lock evidence.

This section describes implemented composition and its software-test scope. The
older stereo hardware record below remains the qualified physical evidence; it
does not qualify the configurable16/18rig or48-channel realtime throughput.

## Implemented USB bench boundary

The [hardware host](../acceptance/AUDIO_HARDWARE.md) now opens one explicitly selected stereo
USB interface, runs independently built SHR PA/FX/REC libraries through versioned
C interfaces and records real samples on the PA node. In that historical bench, the Brain FX worker followed the PA's USB
source-frame timeline without a local audio device. This does not describe the
complete Brain product, which also owns a local duplex endpoint. The render section uses bounded
module calls/queues; driver I/O and disk/network work stay outside it. This is a
measured stereo bench; the separately implemented local mixer and native consoles
have device-free software validation, described below.
The host preserves dry processing and recording through Brain failure, fades both
wet channels and requires fresh control state. Device faults close the old take
as incomplete and require a fresh epoch. Prototype control does not yet change
real mixer parameters. The host processes 48-frame / 1 ms blocks with independent
ALSA ring capacity and zero silent prefill; the dry path has no additional
application playout queue. Network and recording queues serve independent workers.
Channel-1 electrical delay and reliability are measured separately in the hardware
record; acoustic acceptance remains open.

Status: the software node boundaries are implemented; full physical integration
remains unqualified. Offline soundcheck/rendering is implemented in `src/automix/`;
see [the automixer](../archive/workflows/AUTOMIX.md).
The [component map](COMPONENTS.md) records module ownership across the `../shr-*`
projects and their intended integration into GigPies.
The [Brain console integration plan](BRAIN_CONSOLES.md) records the developing
`../shr-desk` audio and `../shr-lightdesk` lighting surfaces: two 1920×1080
monitors and two independently assigned MIDI keyboard controllers on one Brain,
local NVMe recording on PA and richer FX on Brain. The surfaces own their human
interfaces; GigPies owns integration/contracts and SHR Lux owns lighting authority.
Both surfaces now have optional native frontends and actual local provider
clients. The configurable mixer, independent monitor sends, REC/FX/PA graph, named
analysis and null-output Lux authority are implemented and software-validated.
The authenticated QUIC path and source-clock FX/analysis have
[16/32/48-input software acceptance](../acceptance/MODULAR_ENGINE_ACCEPTANCE.md).
[Local integration](../archive/workflows/HEADLESS_INTEGRATION.md) retains the earlier foundation evidence.
Physical console integration and the separate [Brain duplex acceptance](../acceptance/BRAIN_AUDIO_ACCEPTANCE.md)
remain distinct gates; production remote authentication is implemented.
The [preceding architecture draft](../archive/architecture-before-brain-console-2026-10-03.md)
is preserved for context. The remaining sections describe the intended complete
system; software acceptance does not establish its physical topology.

## Stagebox / Mixer

Own physical audio I/O, f64 channel processing/mixing, monitor buses, PA processing,
local protection and multitrack recording to local NVMe. Keep the dry monitor and
main audio paths local. Own FX send taps and return routing; richer send/return
engines run on Brain, with a defined reduced local FX mode for Brain failure.
The small display serves local PA operation. Recording uses a bounded worker path
independent of Brain communication and the audio callback.
Prepare structural changes outside the audio callback and apply bounded changes at
block boundaries. Continuous parameters need defined smoothing.

The live callback must avoid allocation, deallocation, locks, filesystem access,
logging and unbounded work. These are implementation requirements, not performance
claims about the complete system's performance.

## Brain / Show

Two human-operated consoles run on Brain: audio in `../shr-desk`, lighting in
`../shr-lightdesk`. Each has its own Full-HD display/controller pairing, selection,
focus and independent process. Automation is a mode inside these consoles.
The audio desk displays applied state and sends scoped requests; the core mixes.
AUTO, ASSIST and MANUAL operation, parameter holds and explicit return to automation are
specified in its blueprint. Manual operation must not require an automixer.
Local mixer authority, bounded ramps and the physical Stagebox adapter/executable
are implemented and tested through fake device boundaries. Physical activation,
socket mapping, clock arrangements and sustained deadlines remain unqualified.
Physical Stagebox/PA ownership does not make `shr-pa` the owner of a complete
band-mixing engine.

Own soundcheck analysis and prepared mix calculation, both full-HD (1920 × 1080)
HDMI consoles, operator and performer controls, doctor/analysis services, the SHR
Lux lighting engine, richer send/return FX and shared show metadata. The audio
console commands and observes the PA recorder; audio is written on PA. Send validated parameter/state
updates to the Stagebox. Brain is outside the essential dry main/monitor path;
its FX have separate latency and failure behavior. Music-analysis extensions are
future work with their own task.

Lightdesk sends requests to SHR Lux; fixture evaluation, cue/effect execution,
lighting arbitration and physical output remain in Lux. Its first static mock
authority is retained as a simulator; the real local client uses Lux
fixture/programmer/Hold/cue authority with null output. MANUAL operation is independent of
audio-reactive automation. Shared show identity and named cross-system cues do
not grant one desk authority over the other. Display/controller assignment uses
verified roles and independent workers; lighting failure/redraw must not block
audio input, real-time processing, protection or recording. See the
[dual-console integration backlog](BRAIN_CONSOLES.md) for missing contracts,
combined unverified budgets and separately scoped physical acceptance.

The intended failure behavior is to preserve the last valid Stagebox mix, local
protection and recording when Brain communication disappears. Define wet-return
fades and local fallback FX separately. A UI restart must refresh authoritative
state without recalling an old mix. Reconnection and manual override semantics
are tested for local and authenticated remote provider boundaries. Brain duplex
fault/restart acceptance and physical recovery retain their separate gates in
the [acceptance record](../acceptance/BRAIN_AUDIO_ACCEPTANCE.md).

## Connection and timing

A dedicated Ethernet link carries source audio for analysis, FX sends/returns and
control/status messages. Primary multitrack recording stays local to PA. Separate
Wi-Fi serves performer access and optional venue services.
The production [remote transport](../reference/REMOTE_TRANSPORT.md) uses mutual-TLS QUIC,
reliable control streams and separately admitted analysis, FX, talkback and monitor
media roles. Packets carry epochs, channel groups and explicit deadlines. Bounded
queues, loss fades and fresh-state recovery are implemented. The earlier GPA1 UDP
and acknowledged UDP-control path in the [audio transport history](../reference/AUDIO_TRANSPORT.md)
belongs to the historical source-clock FX bench; it is not the production authority.

Stagebox's sound card and ADAT expansion provide the processing reference.
Raw recording, analysis and Brain FX retain Stagebox source frames. Brain's local
duplex endpoint has a separate device epoch and clock; bounded Rubato ASRC bridges
talkback into Stagebox and the operator feed into Brain playback. Software clock
and numerical evidence is recorded in [Brain audio acceptance](../acceptance/BRAIN_AUDIO_ACCEPTANCE.md).
No network timestamp or equal nominal sample rate proves physical clock lock.
Multichannel physical mapping, drift, sustained deadlines and acoustic behavior
remain separate acceptance work.

## Audio and soundcheck

The intended live rate is strictly 48 kHz, with 24-bit capture and f64 mixer DSP.
The stereo bench records PCM24 through SHR REC; broader show/tap persistence
contracts remain integration work. The network format is defined above.
Preserve source recording rates and precision for initial offline work; resampling
will be a deliberate, documented preparation step.
The reference hardware in the blueprint is UMC1820 + ADA8200 over ADAT, subject to
physical channel, clock and latency verification. Do not infer routing from enumeration.

Soundcheck produces a prepared baseline. Live rules react to meaningful exceptions
within defined bounds. Musical intent, mix objectives and correction policies will
be worked out through small experiments on real tracks.
A muted-PA pass can analyze source signals; room response, feedback and speaker
alignment require their own measurement/setup workflow.
PA measurement and alignment are developed in SHR PA. Its pending task adds
synchronized reference/mic capture and phase analysis to its existing generator,
pair delay/polarity and crossover DSP. GigPies is intended to integrate the finished
PA module; see [ownership and integration](COMPONENTS.md#pa-module-and-planned-integration).
This remains separate from artistic instrument EQ matching. Live integration is pending.

## Boundaries to grow later

Keep offline source reading and analysis separate from DSP rendering, device transport,
control/UI, lighting and recording. Start in one Rust package; extract crates when an
actual integration boundary benefits from it. There are no empty live-engine crates.
Extra DSP nodes and venue business services remain optional future work.

## Proposed instrument-station soundcheck

Design direction, not an implemented web/monitor feature: each instrument station
has a separate local QR address. Setup maps that station to its known primary and
secondary inputs, performer, stage position and monitor output. Scanning it opens
the correct instrument and an immediately editable default monitor mix. The
performer supplies instrument/style/tone intent and plays a soundcheck; the Brain
measures the known sources and prepares an editable first pass, followed by a band
context check. The offline [tone pass](../guides/TONE_PASS.md) implements the first limited
intent-to-processing step for guitar body and presence.

Personal monitor sends/tone and shared source/FOH processing need distinct control
scope. Performer edits must survive subsequent automatic analysis. Multiple phones
may remain connected and monitor controls remain available during soundcheck;
acoustic measurement steps must be coordinated. Monitor calibration belongs to the
mapped speaker and physical location and requires measurement; a stage-position
label or an initial visual EQ choice is not acoustic calibration. Phone/Brain loss
must preserve the Stagebox's last valid audio state.

### Review after soundcheck

The proposed [musician review after soundcheck](PERFORMER_REVIEW.md) uses existing
station connections to collect preferences for each instrument. Once everyone is
ready, the Brain prepares a shared ensemble preview for phones/headphones. Explicit
“keep my sound” and deterministic controls come first; optional model assistance is
future research. Preview acceptance remains separate from applying a live revision.
This extension, like the station interface, is planned.

### Source-first preparation

The [offline source-rule coordinator](../guides/SOURCE_RULES.md) can request a source
adjustment when confident measurements imply substantial EQ, or an input-path
review when PCM repeatedly touches full scale. For a known amp mic, distinguish
what the player hears at the amp from what the microphone captures: compare both,
change one amp/placement variable if needed, and repeat the same soundcheck phrases.
No physical cause is inferred from the spectrum alone. A live source-review request
keeps existing processing while waiting for a fresh capture; personal monitor
adjustments remain available. The current implementation produces the advice in
local reports; web delivery, capture-state coordination and hardware integration
remain planned.
