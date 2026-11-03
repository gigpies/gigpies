# Automix decision-path review, 2026-10-30

Source and saved-settings audit prompted by the V6 listening review. No new audio
analysis, renders, playback, builds or tests were run. This record distinguishes
implemented calculations from artistic decisions; it does not validate a new
algorithm. Current implementation work stays in
[GP-AUTO-ENSEMBLE](../../development/MODULE_IMPLEMENTATION_PLAN.md#gp-auto-ensemble--observation-led-whole-band-automixing),
with live integration in GP-AUTO.

## Finding

The delivered auditions combine measured processing, inherited mix settings and
manual control revisions. They are not outputs of a complete automatic musical
balance decision system. V5's unequal lead boosts were artistic interventions:
+3 dB for Complainiacs/Catbite and +5 dB for Dark Ride. Saved provenance mentions
recessed leads and historical bus evidence, but supplies no reproducible rule
that calculates those exact amounts. V6's +1 dB on every vocal was requested by
the listener. Its approval establishes a preferred revision on these recordings,
not a general vocal-gain formula or demonstrated unattended operation.

## End-to-end trace

| Stage / source owner | What is actually calculated | Missing decision or limitation |
|---|---|---|
| `config.rs`: `Channel::preset`, `example`; explicit session settings | Role-selected DSP presets; example faders and routing; actual sessions supply their own controls | Role/source identity and starting controls are inputs, not inferred optimal balance. A filename or preset is not measured musical evidence. |
| `dsp.rs`: `Calibrator::observe`; `render.rs`: `run_inner` | During the legacy soundcheck path, active reference RMS drives group trim toward a configured RMS target, with peak cap and slew limits; prepared rendering uses saved trims | Input calibration is not ensemble audibility. A common RMS target cannot establish the desired musical contribution; prepared renders do not rerun balance decisions. |
| `unity.rs`: `prepare`, `calibrate_compressors` | An alternative workflow zeros source faders/trims; thresholds follow active post-EQ peak percentiles and role-specific reduction amounts; makeup compensates measured loss | A measured threshold still depends on a supplied artistic reduction target. This alternative path is not proof that current audition faders were calculated. |
| `ensemble/profile.rs`, `ensemble/correct.rs` | Explicit capture/family/tone targets admit bounded EQ/dynamic trials; production DSP and held-out guards evaluate candidates | Tone-target compliance is not a source-level or vocal-prominence objective. Intent must currently be supplied numerically. |
| `balance.rs`: `Policy::for_session` | Clears all relationships and preserves faders by default | No production musical-balance objective is selected automatically. Existing inherited offsets therefore survive. |
| `balance.rs`: `trial_for_session`, `evaluate_subset`, `optimize` | Experimental fixed relationship ranges, simultaneous band energy/covariance, active windows, bounded coordinate search; median violation is scored, with other percentiles reported | Lead relation is presence against guitars, not broadband voice against the whole accompaniment. Backing vocals have no explicit relationship or movable coordinate. Lead activity uses the first selected lead channel. Quiet-word percentiles are reported but not the primary fitting objective. |
| `hierarchy.rs`, `ensemble/group_levels.rs` | Target-minus-measured-LUFS deltas for an explicitly selected kick/bass/other hierarchy; grouped paths and sections can be measured | This calculates compliance with supplied offsets, not preferred vocal balance. The musical policy was withdrawn for current auditions. Removing it supplies no replacement balance objective. |
| `ensemble/workflow.rs`, `ensemble/correct.rs` | Frozen baseline, explicit requests, source review, trial/held-out and composition vetoes; fader proposals are inside the optional hierarchy branch | No standalone lead/backing audibility-driven fader solver exists in this ensemble path. With no hierarchy or eligible tone objective, preparation writes observations and preserves the baseline. |
| `effects.rs`, `analysis.rs`, saved FX settings | Configured role-based sends/algorithms; an optional pass measures wet/source energy and adjusts returns toward supplied targets | Wetness targets and artistic choices remain inputs. Post-fader sends mean fader changes also affect wet contributions; V5 manually compensated lead sends, whereas V6 left controls unchanged. |
| `render.rs`, `delivery.rs`, V6 reproduction command | DSP executes saved controls; delivery manages output gain/peaks. V6 uses the causal float bus with the previous fixed export gain | Renderer success, loudness or peak compliance cannot establish musical balance. V6 deliberately skipped post-render analysis; that is consistent with a live-style control audition, not an automatic decision test. |
| `ensemble/listening.rs`, private listener record | Offline listening comparisons/recordkeeping | No preference-learning path turns the V6 verdict into a generalizable policy. Human approval has not trained or implemented an automatic controller. |

These are separate available workflows, not one serial pipeline that every
render executes. The current V6 reproduction uses prepared settings and the
plain renderer, not automatic soundcheck followed by ensemble optimization.

## Vocal fader provenance

The private owning record is ignored `artifacts/full-production.json`; media and
private settings remain outside Git. Before-settings snapshots and stored
revision deltas show:

| Song | Inherited lead/backing gap | V5 change | V6 change | Current gap |
|---|---|---|---|---|
| Catbite | Lead 0 dB, each backing -6 dB | Main lead +3 dB, backs unchanged | All vocals +1 dB | 9 dB |
| Dark Ride | Main leads and backing tracks 0 dB | Main leads +5 dB, backs/whisper unchanged | All vocals +1 dB | 5 dB |
| Complainiacs | No separate backing performance; lead and vocal room | Main lead +3 dB, room unchanged | Lead and room +1 dB | Not a lead/backing comparison |

Earlier common source gain changes preserved the lead/backing differences.
Catbite's original -6 dB backing setting is present in the saved baseline, but
its original selection rationale is not established by that record. The gap is
therefore inherited, not an automatic result. These are fader differences;
source recording level, processing, simultaneous singer count and FX prevent
interpreting them as perceived loudness differences.

## Consequence for the next implementation

Build an explicit measured balance objective before claiming automatic musical
mixing. Reuse existing source identity, activity, production DSP, grouping,
measurement and bounded-control infrastructure, but do not treat the current
presets, discarded hierarchy or fixed audition increments as that objective.
The implementation and acceptance checklist is in the owning task card.

Review checks: inspected source branches and saved revision provenance; reviewed
documentation diff and local links. Production, historical, render and hardware
tests were intentionally not run for this code-reading/documentation review.
