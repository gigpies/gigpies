# Brain consoles: SHR Desk and SHR Lightdesk

Updated 2026-10-06. **GigPies is the complete system with both an audio console
and a lighting console for human operation.** Automation is one operating mode
inside each console. The intended Brain has **two 1920×1080 monitors and two
independently assigned MIDI keyboard controllers**, normally one pairing for
audio and one for lighting. Both consoles run on the same Brain. The small
local display continues to serve the PA unit.

The preceding single-surface integration draft is preserved
[unchanged](../archive/brain-console-before-lightdesk-2026-10-04.md); the earlier
[pre-Desk plan](../archive/brain-console-before-shr-desk-2026-10-04.md) also remains.

## Brain sound-card ownership

Alongside the two consoles, Brain has one local duplex sound card: microphone
capture for talkback and playback for operator monitoring. GigPies owns both
directions in one device process; SHR Desk exposes setup, selection and safety
state through the existing authority/client layers. The card model, physical
channel map and hardware monitoring settings require actual observation.
Brain local I/O has a distinct device clock, bridged in both directions to the
Stagebox reference. Brain FX continues to follow Stagebox source frames.
See [Brain audio integration](../reference/BRAIN_AUDIO.md); the earlier console plan is
[preserved](../archive/brain-audio-before-2026-10-05/BRAIN_CONSOLE_PLAN.md).

## Current software checkpoint

The [local integrated milestone](../archive/workflows/HEADLESS_INTEGRATION.md) established the initial
eight-input mixer, owner-library graph, named analysis, Lux null-output authority
and both provider-backed native frontends. The later
[modular engine acceptance](../acceptance/MODULAR_ENGINE_ACCEPTANCE.md) covers configurable
16/32/48-input software processing and authenticated source-clock FX/analysis.
Task0015 implements Brain's separate duplex endpoint, two ASRC crossings and
operator controls; [integrated Brain acceptance](../acceptance/BRAIN_AUDIO_ACCEPTANCE.md) passed software validation. Process-held role leases use injected descriptors. Physical
displays/controllers, fixture output and combined console load remain unverified.

## Coexistence contract

Use separate surface processes, input/LED workers, focus and selection. Persistent
roles bind verified display/controller identities, not monitor order or ALSA port
numbers. Duplicate monitor identities or identical controllers without reliable
serials need explicit labelled assignment. A lighting reconnect/restart must not
steal the audio display, replay old commands or forward notes/LED traffic to the
audio desk/instruments. Computer-keyboard operation remains an optional fallback
without MIDI. The intended live workflow must also be complete with the assigned
MIDI controllers and no computer keyboard or mouse. Input adapters must dispatch
the same semantic actions and preserve review, authority and recovery fences;
controller operation must not depend on synthesizing computer keypresses.
This is a product requirement, not a claim of current controller completeness.

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

The [Lightdesk blueprint](../../../shr-lightdesk/docs/BLUEPRINT.md#resource-and-responsiveness-targets--unverified)
sets the proposed combined acceptance profile and measurable budgets: two active
1080p/60 Hz surfaces, aggregate render CPU/GPU p99 ≤8 ms each per refresh,
combined surface PSS ≤384 MiB, local MIDI dispatch p99 ≤5 ms and visible feedback
p99 ≤33 ms. Whole-Brain FX/Lux/analysis/transport headroom must also pass. These
are **unmeasured targets**, not evidence that a particular Pi meets them.

## Historical evidence

The [dated study and original results](../archive/studies/BRAIN_CONSOLE_PLAN.md) retain experiment-specific settings and validation history.
