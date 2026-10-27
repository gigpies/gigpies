# Existing components and dependencies

The 2026-10-03 [USB hardware integration](../../acceptance/AUDIO_HARDWARE.md) adds versioned
C interfaces in SHR PA/FX/REC and an explicit GigPies host. The libraries build
independently; their source and algorithms remain in their owning repositories.
Stereo actual-device evidence is separate from the broader planned modules below.

Sibling projects were inspected on 2026-10-01. The local inventory and README scope
were refreshed on 2026-10-02, with a source/test inspection of SHR PA's generator,
delays and polarity controls. This is not a fresh implementation audit of every
sibling. All remain in development. Their own source and status documents own
current details. The offline automixer adapts
narrow biquad/dynamics equations from SHR PA/DAW with preserved MIT notices; see
[attribution](../../../THIRD_PARTY.md). No sibling is linked as a dependency. The PA
development task below was added to SHR PA with explicit user authorization.

The 2026-10-04 surface review adds SHR Desk and SHR Lightdesk and inspects relevant
Lux/DAW/FX controller and lighting documentation. Its dual-console map is current
for 0.2.3; it does not renew every sibling's hardware acceptance.

## Module ownership

The intended system is modular across these projects. Develop each module in its
owning repository, then integrate the finished component into GigPies through a
defined interface. In particular, PA processing and measurement belong in SHR PA;
GigPies should not grow a second PA implementation. Standalone tools can remain
usable alongside the integrated system.

Record new module tasks in the owner's roadmap/status documents so development
continues there in future threads. Keep GigPies' integration intent and contracts
here. Changes to another repository still require authorization; local paths below
are relative to the GigPies root and do not introduce build dependencies.

| Owning checkout | Area and project documents |
|---|---|
| `../shr-desk` | Brain audio operator surface, full-HD TUI-style graphics, controller mapping and manual/automix takeover: `docs/BLUEPRINT.md`, `docs/SCREENS.md`, `docs/CONTROL_CONTRACT.md` |
| `../shr-lightdesk` | Brain lighting operator surface, fixture/group selection, programmer/playback presentation and lighting controller mapping: `docs/BLUEPRINT.md`, `docs/CONTROL_CONTRACT.md`, `docs/CAPABILITIES.md` |
| `../shr-pa` | PA DSP, crossover, generator, alignment, protection and measurement: `docs/STATUS.md`, `docs/DSP.md`, `docs/DRIVERACK_MAP.md` |
| `../shr-fx` | Send/return effects: `docs/PLAN.md`, `docs/ACCEPTANCE.md` |
| `../shr-daw` | Workstation DSP, graph, controllers and existing recording code: `docs/WORKSPACE_HANDOFF.md` |
| `../shr-lux` | Lighting engine: source analysis, fixture evaluation, cue/effect execution, arbitration and physical output (many remain planned): `idea.md`, `docs/index.md` |
| `../shr-rec` | Standalone multichannel recording/playback destination: `docs/STATUS.md`, `docs/PLAN.md` |
| `../shr-drums` | Drum engine, kit building and source provenance: `README.md`, `FORMAT.md`, `SOURCES.md` |
| `../shr-synth` | Synth engines and presets: `docs/HANDOFF.md`, `docs/PRESET_SCHEMA.md` |
| `../shr-sampler` | Sample instruments, SFZ import and hosting: `docs/NATIVE_PACKAGE_FORMAT.md`, `docs/LIVE_PROCESS_CONTRACT.md` |
| `../shr-tone-over-9000` | NAM/cabinet processing and prepared chains: `README.md` |
| `../shr-skills` | Development workflow skills: `README.md`, `AGENTS.md`; not an audio engine |

The shared media library is `../waves`; follow [local media ownership](../../development/LOCAL_MEDIA.md).
These local paths are ownership references, not build or CI requirements.

## Existing work and integration limits

| Project | Useful existing work | Integration limit |
|---|---|---|
| shr-desk | Independent Rust surface, optional native frontend, real GigPies control and read-only module status using the existing Terminus font | Physical display/controller acceptance and writable module controls remain separate; no mixing DSP belongs here |
| shr-lightdesk | Independent Rust lighting surface, optional native frontend, real Lux control and read-only analysis/provenance | Physical display/controller/fixture acceptance remains separate; Lux owns timing, arbitration and output |
| [shr-pa](https://github.com/PaolaShultz/shr-pa) | Rust 2-input/6-output PA DSP, EQ, crossovers, pink/white generators, pair delay/polarity, compression, limiting, presets and offline rendering | Fixed routing and linked L/R pair settings; setup-mic/RTA/automatic alignment and physical acoustic acceptance remain pending; direct ALSA transport |
| [shr-fx](https://github.com/PaolaShultz/shr-fx) | Two wet-only engines, up to eight parallel slots each; reverb, delay, chorus, exciter | Send/return semantics; JACK host; listening and live acceptance remain |
| [shr-daw](https://github.com/PaolaShultz/shr-daw) | Channel processing, EQ, dynamics, audio graph, controller support and recording | Coupled to workstation models; adapt narrowly and preserve provenance |
| shr-lux | Offline replay, named live-source analysis subscription, fixture/programmer/cue authority, timed release and durable restart | Physical DMX and hardware acceptance remain pending; current integration uses null/disarmed output |
| shr-rec | Bounded raw PCM24 recording/recovery library and application shell | Library integrated in the stereo bench; standalone recording UI and playback remain pending |
| shr-drums, shr-synth, shr-sampler | Separate instrument engines and offline render/host patterns | Useful references, not automixing prerequisites |
| shr-tone-over-9000 | NAM live processor, prepared chain changes | Guitar processing; no need to integrate for the first mix experiment |
| shr-skills | Development workflow material | Not an audio runtime component |

## PA module and planned integration

The user requested phase measurement and small-delay correction as a **SHR PA
development task**, with the finished PA module intended for GigPies integration.
The canonical task is [SHR PA's phase-alignment plan](https://github.com/PaolaShultz/shr-pa/blob/main/docs/PHASE_ALIGNMENT.md),
also linked from its `AGENTS.md`, status, P4/P5 roadmap and function map.
Locally, the same task is `../shr-pa/docs/PHASE_ALIGNMENT.md` from this repository.

SHR PA already supplies pink noise, pair polarity and 0–10 ms pair delays at
nearest-sample resolution. Its task adds synchronized reference/mic measurement,
phase/confidence analysis and a verified delay/polarity proposal stage. L/R settings
are currently paired; independent-output changes require an explicit contract update.

GigPies will integrate the finished module rather than duplicate those algorithms.
The bench uses a versioned C host interface; phase-measurement and live-control integration remain pending. Preserve one audio-device
owner, explicit timing/protection/state contracts and standalone PA operation.
Module implementation, GigPies integration and acoustic hardware acceptance are
separate milestones; none is completed by recording this task.

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

Possible future libraries include `jack` where a JACK host is needed and
`signal-hook` for shutdown handling. Native graphical surfaces choose their own
backend; a TUI appearance does not require a terminal UI runtime in GigPies.
Select one audio-device owner; do not independently attach PA and FX device transports
and assume they form one low-latency mixer. Pure DSP and host transport are separate.
The [transport prototype](AUDIO_TRANSPORT.md) adds rtrb 0.4.0 for independent
bounded worker queues and socket2 0.6.5 for per-socket receive capacity.
GigPies owns packet/control contracts and adapters. SHR PA, SHR FX and SHR REC
now expose the PA, source-frame FX and local NVMe recorder adapters used by
the stereo bench. Owner source changed within the authorized task; no sibling
algorithms were duplicated here. Optional alsa 0.11.0/libloading 0.7.4 dependencies
serve the explicit hardware-host feature.
ASRC and a network-clock implementation remain unselected integration work.

The [Brain console integration plan](BRAIN_CONSOLE_PLAN.md) delegates audio surface
ownership to `../shr-desk` and lighting surface ownership to `../shr-lightdesk`
(2026-10-04). Both consoles share one Brain with two 1080p monitors and two separately
assigned controllers. They own layout, navigation, mapping and authority presentation.
GigPies owns cross-module integration/contracts; Lux owns lighting arbitration,
fixture/cue/effect execution and output. Missing Lux work is recorded in the
integration backlog, not duplicated in Lightdesk.
The surface projects now use optional `winit` + `wgpu` native frontends;
their renderers also export offline SVG drafts. These are not GigPies dependencies.
See the [current software checkpoint](../workflows/HEADLESS_INTEGRATION.md) for validation and
physical acceptance limits. PA DSP, FX, recorder and lighting
algorithms retain their owners. The Stagebox/PA node's mixer graph is distinct
from the `shr-pa` speaker processor; a future mixer-core extraction is a separate
task. No algorithms or source files were moved in this surface checkpoint.

## Configurable processing ownership

The modular composition includes PA: GigPies owns channel/bus topology, physical
patching, the shared source clock/authority and authenticated endpoints; SHR PA
owns configurable speaker DSP and prepared C-PA v2; SHR Desk owns dynamic operator
presentation; unchanged SHR REC/FX own raw recording and effects. Six/eight-output
and 4×8 PA profiles are examples rather than capacity ceilings. See
[composition](MODULAR_PROCESSING.md), [remote contract](REMOTE_PROCESSING.md) and
[acceptance matrix](MODULAR_ENGINE_ACCEPTANCE.md). A source boundary is not a reason
to omit required PA behavior, and software synchronization is not physical ADAT
lock evidence.
