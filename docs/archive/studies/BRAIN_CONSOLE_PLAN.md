# Brain consoles: SHR Desk and SHR Lightdesk

Updated 2026-10-06. **GigPies is the complete system with both an audio console
and a lighting console for human operation.** Automation is one operating mode
inside each console. The intended Brain has **two 1920×1080 monitors and two
independently assigned MIDI keyboard controllers**, normally one pairing for
audio and one for lighting. Both consoles run on the same Brain. The small
local display continues to serve the PA unit.

The preceding single-surface integration draft is preserved
[unchanged](../brain-console-before-lightdesk-2026-10-04.md); the earlier
[pre-Desk plan](../brain-console-before-shr-desk-2026-10-04.md) also remains.

## Brain sound-card ownership

Alongside the two consoles, Brain has one local duplex sound card: microphone
capture for talkback and playback for operator monitoring. GigPies owns both
directions in one device process; SHR Desk exposes setup, selection and safety
state through the existing authority/client layers. The card model, physical
channel map and hardware monitoring settings require actual observation.
Brain local I/O has a distinct device clock, bridged in both directions to the
Stagebox reference. Brain FX continues to follow Stagebox source frames.
See [Brain audio integration](../../reference/BRAIN_AUDIO.md); the earlier console plan is
[preserved](../brain-audio-before-2026-10-05/BRAIN_CONSOLE_PLAN.md).

## Current software checkpoint

The [local integrated milestone](../workflows/HEADLESS_INTEGRATION.md) established the initial
eight-input mixer, owner-library graph, named analysis, Lux null-output authority
and both provider-backed native frontends. The later
[modular engine acceptance](../../acceptance/MODULAR_ENGINE_ACCEPTANCE.md) covers configurable
16/32/48-input software processing and authenticated source-clock FX/analysis.
Task0015 implements Brain's separate duplex endpoint, two ASRC crossings and
operator controls; [integrated Brain acceptance](../../acceptance/BRAIN_AUDIO_ACCEPTANCE.md) remains
in progress. Process-held role leases use injected descriptors. Physical
displays/controllers, fixture output and combined console load remain unverified.
The foundation assessment and backlog below are preserved as the original planning
baseline; use the
[implementation map](../plans/MODULE_IMPLEMENTATION_MAP.md#execution-checkpoint--2026-10-04)
for completed tasks and remaining gates.

## Retained screen references — 2026-10-06

The development-PC reference collections are integrated on both Pis. They add
manufacturer screen images, per-screen observations, source/version notes, local
HTML galleries and hash-pinned manifests. They are design evidence; they do not
change either console's implemented UI or establish physical operation.

| Owner | Reference collection | Questions it helps answer |
|---|---|---|
| SHR Desk | [12 audio-console screens](https://github.com/PaolaShultz/shr-desk/tree/main/docs/console-screen-reference) from Yamaha, Allen & Heath, Soundcraft and DiGiCo | Bank versus selected-channel detail, exact EQ/dynamics units, physical/logical patching and clear PFL/AFL destination context. |
| SHR Lightdesk | [16 lighting-console screens](https://github.com/PaolaShultz/shr-lightdesk/tree/main/docs/console-screen-reference) from MA, ChamSys, Avolites, Onyx and ETC | Stable fixture identity, capability-aware columns, programmer versus output state, colour/phaser controls and cue/playback context. |

Future layouts should preserve selection and focus as separate states, identify the
current destination/value layer, and distinguish a requested edit from applied,
held or measured state. Exact units belong beside precise controls; dense overview
screens need a readable selected-detail view. These are design hypotheses to test
against the owning screen plans and actual capabilities, not new implementation
requirements for task0015. Reference bank sizes never define product capacity.
Each library records image provenance and limits; old manual screenshots do not
establish a current console version or physical output behavior.

## Original foundation checkpoint

| Owner | Scope | Current evidence |
|---|---|---|
| GigPies | Complete product, live integration, shared show/role contracts and acceptance | Offline audio and qualified stereo USB bench; dual-desk integration planned |
| `../shr-desk` | Audio operator surface, selection, controller actions, manual/automix presentation | Independent offline simulator and three full-HD drafts; native/live integration planned |
| `../shr-lightdesk` | Lighting operator surface, fixture/group selection, programmer/playback presentation, lighting controller actions | Independent Rust offline loop, synthetic authority, controller primitives and seven full-HD drafts |
| `../shr-lux` | Lighting-specific analysis, direction, fixture evaluation, cue/effect execution, arbitration and physical output | Recorded-source analysis/show/pad previews; full fixture/manual/cue and DMX paths still missing |

Lightdesk's static programmer/playback/automation model is an explicit **mock
lighting authority** for surface validation. It does not complete Lux's cue engine,
fixture profiles, fades, effects or output path. Existing siblings were read-only.
Do not route fixture algorithms into the UI to avoid finishing the owner contract.

These local links require the sibling checkout and are not path dependencies.
They do not imply that the sibling code is bundled or publicly hosted with this
GigPies version; the integration requirements on this page stand on their own:

| SHR Desk | SHR Lightdesk |
|---|---|
| [README](../../../../shr-desk/README.md) | [README](../../../../shr-lightdesk/README.md) |
| [Blueprint](../../../../shr-desk/docs/BLUEPRINT.md) | [Owning blueprint and dual-desk budgets](../../../../shr-lightdesk/docs/BLUEPRINT.md) |
| [Console study](../../../../shr-desk/docs/CONSOLE_STUDY.md) | [MA/MagicQ/Titan/Eos/ONYX screen/workflow study](../../../../shr-lightdesk/docs/CONSOLE_STUDY.md) |
| [Screens](../../../../shr-desk/docs/SCREENS.md) | [Screen map and reproducible operating loop](../../../../shr-lightdesk/docs/SCREENS.md) |
| [Controller](../../../../shr-desk/docs/CONTROLLER.md) | [Lighting MIDI/ownership plan](../../../../shr-lightdesk/docs/CONTROLLER.md) |
| [Control contract](../../../../shr-desk/docs/CONTROL_CONTRACT.md) | [Lighting authority/recovery contract](../../../../shr-lightdesk/docs/CONTROL_CONTRACT.md) |
| [Status](../../../../shr-desk/docs/STATUS.md) | [Status and validation](../../../../shr-lightdesk/docs/STATUS.md) |
| | [Capability matrix](../../../../shr-lightdesk/docs/CAPABILITIES.md) |

## Coexistence contract

Use separate surface processes, input/LED workers, focus and selection. Persistent
roles bind verified display/controller identities, not monitor order or ALSA port
numbers. Duplicate monitor identities or identical controllers without reliable
serials need explicit labelled assignment. A lighting reconnect/restart must not
steal the audio display, replay old commands or forward notes/LED traffic to the
audio desk/instruments. Keyboard operation remains complete without MIDI.

Lux must not compete for Lightdesk's assigned pad LEDs. Preserve Lux standalone
operation, but its integrated engine adapter must have no claim on those outputs.
No physical device identity, active preset or feedback protocol for the **second**
controller has been verified. Existing MiniLab mkII and MiniLab 3 evidence is scoped
to its recorded device/session, not automatically reusable as a fresh mapping.

Both desks share show UUID/module compatibility, theme/font/action vocabulary and
useful health summaries. Keep audio and lighting control domains distinct:
lighting programmer intensity can override a brighter cue even at zero, while
ordinary playback intensities can merge HTP; color/position require explicit
activation/priority semantics. Mode changes preserve the look; release is scoped
and deliberate. Lux owns timing/arbitration and fixture-safe output behavior.
A drawing, ACK or packet submission does not verify physical DMX/light.

Cross-system cues are opt-in named events with target scopes, epoch/expiry and
individual results. No light GO changes audio by default. Lux consumes named,
fresh source-analysis subscriptions from GigPies rather than Lightdesk opening
audio devices. Lighting redraw, failure and telemetry overload cannot block
Stagebox real-time audio, protection, recording or audio control input.

The [Lightdesk blueprint](../../../../shr-lightdesk/docs/BLUEPRINT.md#resource-and-responsiveness-targets--unverified)
sets the proposed combined acceptance profile and measurable budgets: two active
1080p/60 Hz surfaces, aggregate render CPU/GPU p99 ≤8 ms each per refresh,
combined surface PSS ≤384 MiB, local MIDI dispatch p99 ≤5 ms and visible feedback
p99 ≤33 ms. Whole-Brain FX/Lux/analysis/transport headroom must also pass. These
are **unmeasured targets**, not evidence that a particular Pi meets them.

## Original integration backlog with owners

| ID / next owner | Concrete work and acceptance boundary |
|---|---|
| GI-A / GigPies | Complete real audio capability/parameter schema, scopes, engine-held overrides and ramps. GPC1 still controls a test scalar, not mixer parameters |
| LX1 / SHR Lux | Versioned fixture/patch capabilities, manual programmer, persistent holds, static cue/palette model, per-attribute source/arbitration and fixture-aware master/blackout; null output first |
| LX2 / SHR Lux | Engine release-preview tokens, timing/effect ownership, bounded grants/proposals, show persistence and explicit output-loss/rearm policy; real output remains a separate gate |
| GI-L / GigPies + Lux | Agree versioned lighting control/observation schema, show identity, read-only adapter, then validated commands; matching synthetic contract fixtures on both ends |
| GI-R / GigPies | Persistent audio/lighting display and controller role assignment, atomic ownership, duplicate identity workflow, independent workers/reconnect; no shared hot input queue |
| UI-N / each surface owner | Native renderer and complete keyboard/controller focus/action workflow over existing font/scene models; measure before extracting a small common renderer/profile package |
| GI-S / GigPies + Lux | Named audio-analysis subscriptions and optional explicit cross-system events; freshness, partial completion and no implicit authority grants |
| H1 / GigPies | Separately authorized dual-monitor/controller assignment/LED/restart usability test |
| H2 / Lux + GigPies | Known fixture patch, output arming/loss/recovery and physical acceptance; no generic blackout/hold-last assumption |
| H3 / GigPies | Reserved combined CPU/GPU/memory/control/audio/recording workload and fault acceptance on chosen Brain |

Recording work in this backlog does not authorize sibling changes or a hardware
session. Do offline UI/contract work first; use the private ledger for any later
shared resource reservation. Preserve failed targets and distinguish offline,
integrated and physical acceptance.

The initial offline foundation checkpoint ran locally on **rpi5**. Existing GigPies/Desk work and
interactive sessions were preserved. No audio/MIDI/DMX output, service, host font,
display configuration, shared hardware/load test, publication or deployment ran
in that implementation session. The later 0.2.3 checkpoint publishes the GigPies
documentation with the revised hero/system map; physical gates remain unchanged.
