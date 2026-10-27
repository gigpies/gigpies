# Module plan routing

This file routes ownership. It does not track task status. Cross-module integration
is planned and updated in [one integration plan](MODULE_IMPLEMENTATION_PLAN.md).
Module-only tasks are planned and updated in their owning document below.

| Runtime owner | Responsibility | Owning plan |
|---|---|---|
| GigPies | Stagebox mixer, audio clocks/devices, shared authority/transport and final integration | [Integration plan](MODULE_IMPLEMENTATION_PLAN.md) |
| SHR Desk | Audio console and controller presentation | [Desk plan](https://github.com/gigpies/shr-desk/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR Lightdesk | Lighting console and controller presentation | [Lightdesk plan](https://github.com/gigpies/shr-lightdesk/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR PA | Speaker DSP, measurement and alignment | [PA plan](https://github.com/gigpies/shr-pa/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR FX | Wet-effects DSP and prepared embedding | [FX plan](https://github.com/gigpies/shr-fx/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR REC | Raw writer, progress, finalization and recovery | [REC plan](https://github.com/gigpies/shr-rec/blob/main/docs/GIGPIES_IMPLEMENTATION.md) |
| SHR Lux | Lighting authority, cue/effect execution, analysis and physical output | [Lux plan](https://github.com/gigpies/shr-lux/blob/main/docs/notes/0027-gigpies-implementation.md) |

SHR-DAW/Player, Drums, Synth, Sampler, Tone Over 9000 and Skills are reference/code
sources, not GigPies runtime dependencies or incomplete modules. Earlier speculative
activation plans do not create product requirements. See [components](../architecture/COMPONENTS.md).

## Execution checkpoint — 2026-10-04

This legacy link now routes to the [historical map](../archive/tracking-before-2026-10-09/MODULE_IMPLEMENTATION_MAP.md#execution-checkpoint--2026-10-04).
Use the integration plan for active work and the specific technical acceptance
record for evidence. The archived twelve-repository launch scheme is not current
scope, model/delegation advice or permission to execute old prompts.
