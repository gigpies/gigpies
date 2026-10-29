# Components and code references

GigPies has seven runtime owners. This document defines boundaries and routes
plans; it does not duplicate task states. Use [the integration plan](../development/MODULE_IMPLEMENTATION_PLAN.md)
for cross-module work and [the module map](../development/MODULE_IMPLEMENTATION_MAP.md) for owner tasks.

## Module ownership

| Runtime owner | Responsibility and contract |
|---|---|
| GigPies | Stagebox devices/clock, channel DSP, mixer/buses, shared authority, transport, Brain duplex/ASRC and final composition; [architecture](ARCHITECTURE.md) |
| SHR Desk | Brain audio console/controller presentation; [owning plan](https://github.com/gigpies/shr-desk/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR Lightdesk | Brain lighting console/controller presentation; [owning plan](https://github.com/gigpies/shr-lightdesk/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR PA | Speaker DSP, crossovers, weighted routing, output protection, measurement and alignment; [embedding v2](https://github.com/gigpies/shr-pa/blob/main/docs/EMBEDDING_V2.md) |
| SHR FX | Wet-effects algorithms and prepared embedding; [owning plan](https://github.com/gigpies/shr-fx/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR REC | Raw recording worker, progress, finalization and recovery; [raw contract](https://github.com/gigpies/shr-rec/blob/main/docs/RAW_RECORDER.md) |
| SHR Lux | Lighting analysis/authority, fixture capabilities, cue/effect timing and physical output; [owning plan](https://github.com/gigpies/shr-lux/blob/main/docs/notes/0027-gigpies-implementation.md) |

Implement owner algorithms in their repository, then consume a defined interface.
GigPies must not grow another PA/FX/REC/Lux engine; a console must not become its
provider. A module's standalone device host or UI is not automatically embedded.
PA/FX/REC libraries build independently, with reviewed exact artifact identities;
there are no sibling path dependencies. Source changes remain subject to each
repository's instructions and the user's task scope.

Shared console presentation is owned in GigPies `src/instrument-panel/`: Ratatui
controls, tokens and authored font tiles. Desk and Lightdesk consume frozen,
checksum-verified directory-source packages without sibling path dependencies.
Their semantic actions/provider workers remain in their own repositories; the
shared package has no engine or output authority. See the
[shared presentation contract](../reference/INSTRUMENT_PANEL.md) and
[GP-PANEL acceptance card](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-panel--shared-physical-instrument-panel).
This library is not an eighth runtime owner.

## Existing work and integration limits

The actual configurable source graph consumes SHR PA v2, raw SHR REC and
source-clock SHR FX. PA v2 has dynamic program inputs, explicit weighted DAG sums
and independent speaker outputs. Standalone PA presets and v1 remain compatible.
The embedded FX interface is fixed stereo f64 delay with read-only status; SHR FX's
two-engine/eight-slot standalone rack needs a prepared writable embedding contract
before GigPies/Desk can control it. Recorder lifecycle/progress are real; standalone
recorder shell/player completion is not a prerequisite for raw host recording.

Desk/Lightdesk have provider-backed native software. Lux has real private
null-output authority, release/recovery and named-source analysis; physical DMX
is not implemented by its pure range encoder. Source-boundary, clock and failure
behavior belong to [modular processing](../reference/MODULAR_PROCESSING.md),
[Brain audio](../reference/BRAIN_AUDIO.md), [authenticated transport](../reference/REMOTE_TRANSPORT.md) and
[the accepted owner graph](../reference/MODULE_GRAPH.md). Their dated acceptance records
separate software/two-Pi checks from physical multichannel/ADAT/acoustic/show load.

## Reference projects, not runtime modules

| Reference | Reuse only when a concrete GigPies task needs it |
|---|---|
| SHR-DAW, including Player | MIDI controller decoding/pickup/learn, audio setup/lifecycle, bounded effects/recording or DSP patterns |
| SHR Drums | Relevant bounded engine/package patterns; no drum instrument requirement |
| SHR Synth | Relevant render/event/host safety patterns; no synthesizer or preset catalog requirement |
| SHR Sampler | Relevant prepared-data, queue and process-lifecycle patterns; no sample player/importer requirement |
| SHR Tone Over 9000 | Relevant prepared-chain/control/host patterns; no NAM processor/model catalog requirement |
| SHR Skills | Development workflow material; never an audio or lighting runtime dependency |

Listing these repositories does not make their standalone features unfinished
GigPies work. Their earlier speculative GigPies activation plans are historical
proposals, not the current product scope. Do not import their processes, UI,
backlogs, private state or media along with the needed code. Inspect source/tests,
licenses and notices, adapt narrowly and preserve provenance. A new runtime
requirement needs an explicit product decision rather than a free worker lane.
The automixer already adapts narrow PA/DAW equations with [attribution](../../THIRD_PARTY.md).

The shared media library is `../waves`; follow [local media ownership](../development/LOCAL_MEDIA.md).
No media library or source clone is a build/CI download requirement.

## PA module and planned integration

Current PA processing is integrated through C-PA v2. Measurement/phase/delay alignment
has a distinct [owning task plan](https://github.com/gigpies/shr-pa/blob/main/docs/PHASE_ALIGNMENT.md).
The offline analyzer and review proposals live there; GigPies integrates them
through the existing [GP20 software provider](../reference/MEASUREMENT_PRODUCER.md).
Physical synchronized microphone capture remains separately unqualified. V2 independent output controls are implemented;
the older standalone pair controls retain their own contract. Neither is proof of
measured acoustic alignment or physical speaker protection.

## Libraries

Active dependencies are intentionally small:

| Library | Current use |
|---|---|
| hound 3.5.1 | WAV metadata reading; already used by SHR projects |
| serde 1.0.229 | Typed report serialization |
| serde_json 1.0.151 | JSON settings/reports with exact float round trips |
| sha2 0.10.9 | SHA-256 identities for frozen matching inputs/settings |
| rtrb 0.4.0 | Bounded independent worker queues |
| socket2 0.6.5 | Explicit transport socket capacity |
| alsa 0.11.0, libloading 0.7.4, libc 0.2.189 | Optional `hardware-host` device/module boundary |

Rust 1.97.1 and edition 2024 match the current related projects.
`Cargo.lock` owns the complete resolution.

## Configurable processing ownership

The modular composition includes PA: GigPies owns channel/bus topology, physical
patching, the shared source clock/authority and authenticated endpoints; SHR PA
owns configurable speaker DSP and prepared C-PA v2; SHR Desk owns dynamic operator
presentation; unchanged SHR REC/FX own raw recording and effects. Six/eight-output
and 4×8 PA profiles are examples rather than capacity ceilings. See
[composition](../reference/MODULAR_PROCESSING.md), [remote contract](../reference/REMOTE_PROCESSING.md) and
[acceptance matrix](../acceptance/MODULAR_ENGINE_ACCEPTANCE.md). A source boundary is not a reason
to omit required PA behavior, and software synchronization is not physical ADAT
lock evidence.
