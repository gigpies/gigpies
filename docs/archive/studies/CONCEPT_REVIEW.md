# Visual map and historical concept review

The current README [hero](../../assets/gigpies-hero.png) is an AI-generated,
photorealistic visualization of the **intended complete system**. It replaces the
schematic banner after the requested visual correction. The separate
[system diagram](../../assets/architecture.svg) remains a code-authored SVG for precise
module ownership. Neither image establishes hardware acceptance.

The hero shows two physical LCDs and two MIDI piano keyboards in front of a lit
band stage, with one Brain Pi between the desks. A second processing/Stagebox Pi
has its own small screen and audio I/O rack; physical cable looms, three-way PA
stacks, stage wedges, instruments and a personal-monitor phone show the setting.
Device panels, speaker arrangements and cables are illustrative rather than a
verified installation plan. The generated screens use the actual Desk/Lightdesk
drafts as visual references, with added future-looking detail.

Generated 2026-10-04 with the built-in image generation tool, using the preserved
original concept plus the two projects' offline Mix/Stage screenshots. The exact
[generation prompt](../../assets/gigpies-hero-prompt.txt) is retained. The selected
1672×941 PNG is about 2.17 MiB and is explicitly reviewed in the publication policy.
The superseded [schematic banner](../assets/gigpies-banner-schematic-2026-10-04.svg)
is preserved; the technical system diagram and engine ownership are unchanged.

- One Brain hosts the audio surface **SHR Desk** and lighting surface **SHR Lightdesk**.
  Each gets its own 1920×1080 screen and independently assigned MIDI controller.
  The pictured controllers describe the intended vocabulary, not verified device models.
- Human operation comes first. MANUAL, ASSIST and bounded AUTO are console modes.
- GigPies owns integration and the band mixer. Stagebox owns local audio I/O,
  monitor paths, PA protection and recording to NVMe; its small display is local PA UI.
- **SHR Lux** owns fixture evaluation, cue/effect execution, lighting arbitration
  and physical output. Lightdesk owns the operator surface, not another engine.
- Audio FX/source transport and lighting control have distinct contracts. Lighting
  redraw/failure must not block audio control, protection or recording.

Both surfaces now have optional native frontends and real local provider clients.
Lux implements manual fixture control with null output; see
[local integration](../workflows/HEADLESS_INTEGRATION.md) for software acceptance. Physical
lighting and combined display/controller/load acceptance remain pending.
The stereo audio bench has its own qualified evidence in [AUDIO_HARDWARE.md](../../acceptance/AUDIO_HARDWARE.md).

## Preserved original artwork

The supplied AI-generated [original concept image](../../assets/gigpies-concept.png)
remains unchanged as historical artwork, linked here instead of promoted in the
README. The [earlier review](../concept_review-before-0.2.3-2026-10-04.md)
and [older SVGs](../README.md) preserve the previous design context.

Its generated device panels, labels and some mappings are inaccurate. In particular:

- The Brain calculates prepared mix state; Stagebox validates/applies it. The new
  design has two human-operated consoles rather than an automation-only Brain.
- Live behavior uses deterministic rules. External AI is optional outside the
  essential live path; a generic “AI-assisted” label does not describe the authority model.
- The old RTP/PTP labels are not the selected implementation. The prototype uses
  GPA1 UDP and the PA source-frame clock; no PTP or audio ASRC was installed.
- ADAT direction, channel availability and clock master require actual device
  documentation and acceptance; the drawing is not a wiring guide.
- “4×8” PA routing was an ambition. Existing SHR PA's 2-input/6-output speaker
  processor does not establish a complete GigPies band mixer or that routing.
- Primary recording is now Stagebox-local NVMe. Recording taps, physical outputs
  and fault policies come from explicit contracts, not decorative arrows.
- Muted source analysis does not establish acoustic response, speaker protection,
  feedback behavior, lighting coverage or dependable unattended operation.

Third-party names and rendered device designs in either image imply no endorsement.
Read current
[architecture](../../architecture/ARCHITECTURE.md), [owners](../../architecture/COMPONENTS.md) and
[status](../../STATUS.md) for implementation and acceptance boundaries.
