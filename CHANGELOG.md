# Changelog

## Unreleased

- Measured meters are now checked against the real PA/FX/REC modules: Main
  pre-PA and every monitor match independent references at 16/17/32/48 inputs,
  and identity changes, slow readers and repeated reads cannot refresh old windows.
- Shared panel 0.3.3 adds a read-only MIDI reader. Both desks accept a learned
  controller profile (`--controller-profile`); Desk imports the learned SHR-DAW
  MiniLab mapping (`--controller-import-daw`) and adds a read-only `--controller-learn`. Physical controller acceptance remains open.
- `scripts/owner_manifest.py` records owner-library manifests from clean
  committed checkouts, so owner-backed tests no longer depend on stale hashes.
- The integration plan and development guide are condensed; their full history
  is archived.

## 0.2.3 — 2026-10-04

- Defines GigPies as the complete audio and lighting system: SHR Desk owns the
  audio surface, SHR Lightdesk the lighting surface, and SHR Lux the lighting
  engine. One Brain hosts two 1920×1080 displays and two independently assigned
  MIDI keyboard controllers. MANUAL, ASSIST and bounded AUTO are operating modes.
- Replaces the README hero and system diagram with accurate, original SVG maps;
  archives superseded plans and clarifies current versus historical documentation.
- Includes previously local GPA1 audio/control transport and the explicit optional
  stereo USB host using independently built PA/FX/REC modules. The recorded bench
  passed its ten-minute H8 gate; 5.19–5.23 ms working-channel delay still had two
  unresolved one-frame changes. This is not complete-show or multichannel acceptance.
- Includes summing observations, explicit delivery controls and offline true-peak
  finalization, preserving the source settings and selected effects.
- Records separate display/controller ownership, show compatibility, recovery and
  combined workload gates. The two surface projects remain independent; their
  mock authorities and screen drafts are not bundled live consoles or a Lux engine.

Native console integration, full mixer/lighting controls, physical lighting and
combined two-surface hardware acceptance remain planned. No new hardware or
listening tests were performed for this version; publication checks are recorded
in [validation](docs/archive/records/VALIDATION.md).

## 0.2.2 — 2026-10-03

- Paired EQ diagnostics and readable saved-report summaries separate spectral
  changes from phrase variation, compression and FX consequences.
- FX reviews expose individual decay/wet-level residuals, control limits and missing
  observations independently of ensemble eligibility and listener acceptance.
- FX measurement includes the master stage, rejects master reduction and checks
  complete passage coverage.
- Frozen source/report identities, completion records, interruption recovery and
  read-only plan verification/audits protect offline preparation.
- New FX/EQ plans require training before held out; saved EQ states retain exact reset.
- Summing-engine investigation, true-peak delivery and export-policy work are planned,
  with an execution prompt. These features are not implemented by this release.

At the user's request, old local renders were retired while original recordings,
settings and evidence were preserved. New listening mixes will follow the engine
work. Musical acceptance and hardware validation remain pending.

## 0.2.1 — 2026-10-03

- Deterministic offline EQ matching against versioned measured maps or provisional
  artistic directions, with bounded broad filters fitted through production DSP.
- Frozen baseline and source identities, persisted proposals, exact 0% and reset,
  and a 0–100% amount that scales proposed filter gains in dB.
- Existing EQ, effects, unity input trims and coherent groups are retained;
  insufficient evidence and filter-capacity limits produce explicit abstention or errors.
- CLI map import, selection and amount controls, plus an accessible local HTML
  review with response curves and a downloadable selection.
- Reference provenance, licensing, capture limits and uncertainty documented;
  authored style maps remain provisional artistic choices.
- Synthetic correction and production regressions validated. The deliberately
  coloured Dark Ride pilot abstained; all six existing complete mixes are retained.
- Planned MiniLab console controls and second-Pi LAN/protocol work documented.

Musical acceptance remains pending. EQ matching does not recreate distortion,
dynamics, performance or ambience. Live control and hardware validation remain planned.

## 0.2.0 — 2026-10-03

- Offline soundcheck, frozen settings, production DSP and complete stereo rendering.
- Source preservation: optional offline filters, explicit tone/balance intent,
  independent makeup/faders and a legitimate unchanged-source selection.
- Contextual bass, drum, coherent ensemble and snare-spill analysis, with bounded
  proposals, held-out verification and explicit uncertainty.
- Expert-selected artistic reverb/delay/chorus recipes, calibrated through the
  production engines while retaining accepted direct tone and balance.
- Verified listening checkpoints with independently finalized −0.01 dBFS sample
  peaks, exact parent-gain excerpts and declared effects tails. Legacy loudness-matched
  experiments remain explicit historical options.
- Reusable local evidence and listening tools; recordings, private settings and
  generated outputs remain outside Git and releases.
- Publication checks in Git hooks and CI reject private paths, media, common secret
  formats and scripts absent from the reviewed publication list.
- Documented musician review/tone-map proposals and modular SHR PA ownership.

This is an experimental offline release. Live GigPies integration, adaptive EQ
matching and acoustic hardware validation remain pending. Passing software checks
does not establish musical acceptance or true-peak compliance.

## 0.1.0 — 2026-10-01

First public foundation: Rust library/CLI, read-only WAV header inventory, synthetic
regression tests, Linux CI, project documentation and preserved concept artwork/draft.

Automixing and live-show functionality remain under development. This release has no
live audio, network, hardware or automatic mixing engine.
