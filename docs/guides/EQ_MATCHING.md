# Frozen EQ matching and tone maps

Implemented and offline-validated in 0.2.1. Maps express a musician's direction; a spectral difference
is not a diagnosis of a defective source. Listener and hardware acceptance remain
separate. The existing complete six-example checkpoint remains
`artifacts/automix/expert-fx-v1/LISTEN.md`.

## Initial map collection

The distributable collection contains original, explicitly **provisional directions**:

| ID, version | Kind and basis | Context and limits |
|---|---|---|
| `keep-current`, 1 | Preserve the entire frozen baseline | Any admitted instrument group; no added EQ |
| `dark-metal-guitar`, 1 | Source-relative, authored upper-mid reduction: a smooth −3 dB Gaussian direction centred at 2.5 kHz, width 0.85 octaves (standard deviation) | Recorded electric guitar; 125–6300 Hz; 0.75 dB tolerance. No metal recording establishes these numbers; this does not create distortion, palm muting or a different cabinet. |
| `gentle-acoustic`, 1 | Source-relative, authored −2 dB direction centred at 3.2 kHz, same width | Recorded acoustic guitar; same range/tolerance. Reduced attack-region presence may cost articulation. No folk/acoustic genre standard is claimed. |

These directions modify the frozen measured envelope before fitting. They are not
empirical reference spectra or imported console presets. Their settings, basis,
analysis version, uncertainty, capture assumptions and limitations are emitted by
`eq-maps`. The display name never selects a hidden algorithm. A compatible source,
weak evidence, unavailable capacity or failed validation can produce no added EQ.
Unknown tuning/register/technique remains unknown. The phrase checks are conservative
heuristics, not pitch transcription or a classifier of playing technique.

Measured maps are a distinct schema variant, imported from explicit local phrases.
Private measured envelopes, source hashes and study settings remain under ignored
`artifacts/` or `user/`; derived summaries are not automatically cleared for distribution.
The small local collection and real-audio decisions are recorded below after validation.

## Research catalogue

Sources checked on 2026-10-03; no audio datasets were downloaded for this work.

| Source | Evidence, provenance and capture | Terms and suitability |
|---|---|---|
| [FabFilter EQ Match](https://www.fabfilter.com/help/pro-q/using/eqmatch) | Primary algorithm/workflow documentation: stored or recorded spectra, time averaging, limited fitted bands and adjustable detail | Documentation establishes a matching method, not a tone library. No proprietary code, preset values or spectra were copied. |
| [iZotope Match EQ](https://s3.amazonaws.com/izotopedownloads/docs/ozone9/en/match-eq/index.html) | Primary documentation: captured reference/input spectra, restricted matching range, smoothing and amount. Warns against chasing every peak/valley | Method reference only. GigPies uses its own bounded biquads and defines amount as band-gain scaling; no transfer-function equivalence is claimed. |
| [MedleyDB description](https://medleydb.weebly.com/description.html) and [terms](https://medleydb.weebly.com/downloads.html) | Artist/studio multitracks with metadata, 44.1 kHz/16-bit audio. RAW, processed STEMS and full MIX have different roles; RAW may have timing edits and live captures may contain bleed | Website specifies CC BY-NC-SA 4.0 and requests no republication without consent. Local sample notice also records conflicting sample-record metadata; retain the more restrictive private research scope. Select known RAW paths and document existing processing. A full mix cannot define an isolated guitar. |
| [Cambridge-MT library](https://cambridge-mt.com/ms3/mtk/) | Artist-contributed multitracks across styles. Exact chains and capture relationships vary; “raw” does not establish untreated sound | Current page returned HTTP 403. Existing acquisition notices and original archive readmes establish educational/private use, with commercial permission required. No new distribution right inferred. Local Dark Ride and other material stays private. |
| [GuitarSet v1.1.0](https://zenodo.org/records/3371780), [capture description](https://guitarset.weebly.com/) | Annotated acoustic phrases, about 30 seconds, six players, comping/soloing, multiple styles. Separate hexaphonic pickup and Neumann U-87 microphone captures; debleeded pickup variants are separate | Record API identifies CC BY 4.0; code repository MIT is a separate licence. Suitable future phrase/register/technique tests with attribution. Acoustic mic/pickup recordings do not establish an amplified metal target. Known annotation errors are listed on the record. No audio imported here. |
| [NSynth](https://magenta.withgoogle.com/datasets/nsynth) | Google-generated notes from commercial sample libraries; four-second, monophonic 16 kHz examples with pitch/velocity/family and brightness tags | Dataset is CC BY 4.0. Useful controlled pitch/velocity stress material; isolated notes, limited bandwidth and partly heuristic labels make it unsuitable as a finished guitar-style target. “Dark” is not a metal label. No audio imported here. |
| Existing local EQ presets | Lists of filter parameters describe a transfer response | They do not specify the spectrum of an instrument fed through that response. They cannot be relabelled as measured tone maps. No manufacturer collection is redistributed. |

The Cambridge terms above are grounded in retained acquisition notices, not a newly
successful web fetch. An import records the actual recording licence/citation and
capture conditions. No source catalogue alone proves that a chosen phrase is isolated,
representative, dry or musically desirable.

## Signal and fitting contract

`automix::matching` is an offline analysis/controller module. It reuses native WAV
reading, BWF alignment, routing, production `Strip`, `Biquad::equalizer`, excitation,
send/return rack and master processing. `Strip::tick_with_eq_tap` exposes its actual
EQ output while advancing the same compressor used by normal rendering. Matching
compares that pre-compressor EQ envelope. Actual post-compressor/FX outcomes are
measured separately; a linear prediction cannot predict their envelopes.

Analysis version 1 deliberately fixes these settings:

- Non-overlapping 8192-sample Hann windows, with no observation crossing a declared
  passage boundary. DSP runs continuously from sample zero. A ten-minute retained
  observation budget bounds offline evidence memory. Very short/low-rate phrases
  can lack enough windows and abstain.
- Left/right powers are combined without folding stereo to mono. Each declared
  input group's paths are summed coherently; independent path powers also expose
  cancellation. The same appended filters are applied to every group member and
  both sides of stereo files. Timing, pan, relative faders and baseline filters remain.
- 28 third-octave sample points from 31.25 Hz to 16 kHz. Each averages linear PSD
  with a triangular kernel spanning **one octave**, then converts to dB. Robust
  medians of level-centred windows describe the phrase. This avoids fitting FFT
  bins or selecting centres from individual harmonics.
- Raw activity exceeds −65 dBFS and training p95 minus 24 dB. Windows with local
  spectral flatness at least 0.35, or one bin carrying at least 30% of relevant
  power, do not establish a broad musical envelope. At least eight windows, three
  active seconds and eight supported frequency points are required. Median robust
  variation across 125–1000 Hz must reach 0.35 dB to avoid promoting a single steady
  note to a phrase target. These gates can withhold some valid noisy/distorted or
  unusually steady performances; they do not certify source identity.
- Band support requires power within 30 dB of the strongest smoothed band in at
  least 70% of active windows, with robust variation at most 6 dB. Frequencies near
  the FFT resolution floor or above 0.4 × sample rate are unsupported. A boost
  additionally needs support and less than 9 dB coherent cancellation in 90% of
  windows. The dense fitted-response grid permits at most 0.25 dB positive skirt
  leakage into unsupported/cancelled regions; it never targets those regions.
- Remove the common median level difference **only in analysis**. Neither source
  samples nor input trim, makeup, fader or export gain is normalized for fitting.
  Measured-map tolerance is the greater of map uncertainty and source variation
  capped at 3 dB; map uncertainty itself is never reduced. Relative authored
  directions retain their declared tolerance because they request a change to the
  same phrase, rather than comparing different performances.
- A fixed, deterministic greedy search fits at most three Q=0.7 bells. Centres are
  125/200/315/500/800/1250/2000/3150/5000/8000 Hz within supported map/rate limits;
  each gain is within ±3 dB in 0.25 dB steps. Predictions use **production biquad
  coefficients**, not a Gaussian EQ approximation. Total response is bounded to
  ±6 dB on a dense logarithmic grid. The score is squared residual outside the
  uncertainty interval plus `0.025 × sum(gain²)`; no change wins ties.
- Residual RMS below 0.5 dB needs no correction. A proposal must improve residual
  RMS by at least 0.3 dB and 20%. All needed slots must fit every affected channel's
  eight-band capacity. Capacity failure is explicit; no existing band is deleted.

The Gaussian describes the authored **direction**, not the fitted filter response.
Broad filter skirts remain physical and are shown in the review curve. This is a
small bounded fit, not an arbitrary FIR inverse or a guarantee of a global optimum.
It cannot reconstruct distortion, dynamics, performance, missing frequencies or ambience.
There is no “already processed” detector.

## Amount, state and recovery

- **0% returns the frozen baseline exactly**, without appending zero-gain filters.
  Existing EQ, compressor, effects, routing and all gain controls are preserved.
- **100% applies the full bounded proposal** if actual validation permits it.
  Intermediate amounts multiply each proposed gain in dB by `amount / 100`.
  The composite biquad response need not scale perfectly point by point; the
  displayed curve is calculated from the real coefficients at that amount.
- Every projection starts with the embedded baseline, never the previous slider
  result. Reset restores that baseline. Switching profiles resets the page to 0%.
- `state.json` stores the complete baseline and SHA-256 identity, original file
  hashes, request/context, map identity/version and content, full bounded proposal,
  checks, selected amount and latest validation attempt. Float JSON round trips are
  exact. A stale page/map, changed baseline or changed recordings is refused.
- `settings.json` is the ordinary production renderer's derived settings. It stays
  compatible with the existing session schema. Keep its accompanying state for
  further edits; reimporting rendered audio or chaining new plans from an applied
  selection is a different baseline and must be deliberate.
- New output directories/files are required. A failed selected amount saves its
  failed checks and restores 0%; it never silently compensates with another control.
  The original state and mixes remain available if a write or process fails.

SHA-256 identities detect stale/changed content; they are not digital signatures or
permissions to apply a setting to hardware. The module is outside any audio callback.

## Commands and local review

```sh
CARGO_INCREMENTAL=0 cargo +1.97.1 build --locked --release
target/release/gigpies eq-maps
target/release/gigpies eq-map-check dark-metal-guitar
target/release/gigpies eq-map-import baseline.json SOURCES new-map.json import-spec.json
target/release/gigpies eq-match-plan baseline.json SOURCES new-review request.json
# Open new-review/review.html locally; no server or automatic playback.
target/release/gigpies eq-match-amount new-review/state.json SOURCES new-half dark-metal-guitar 50
target/release/gigpies eq-match-apply new-review/state.json SOURCES new-choice selection.json
target/release/gigpies eq-match-reset new-choice/state.json new-reset
target/release/gigpies render new-choice/settings.json SOURCES new-render
```

The page provides a labelled profile selector, keyboard-operable slider and number
input, reset, response curve, filter table, provenance and status in text. It uses
precomputed production-response curves for every integer percentage. Saving downloads
only the small selection request; the CLI validates it before writing DSP settings.
There is no web service, device control, audio playback or performer ownership/auth
system. The wider [performer workflow](../architecture/PERFORMER_REVIEW.md) remains planned.

A request is explicit about its input group and split. For example, adapt these
indices and files to an actual verified assignment:

```json
{
  "group": {
    "name": "guitar station", "inputs": [{"channel": 0, "file": "guitar.wav"}],
    "identity_basis": "Operator-verified station assignment",
    "context": {"instrument": "electric_guitar", "capture": "recorded_track",
      "tuning": "unknown", "register_hz": null, "technique": "picked chord phrases"},
    "representative_phrases": "Quiet and strong chords in two registers"
  },
  "training": [[0, 12], [24, 36]], "held_out": [[12, 24], [36, 48]],
  "maps": ["keep-current", "dark-metal-guitar", "user/my-reference.json"],
  "routing_basis": "Verified source inventory; no supplied FX returns",
  "excluded_fx_returns": ["supplied-return.wav"]
}
```

`eq-map-import` takes an `ImportSpec`: `id`, positive `version`, `label`, `basis`,
that same `group`, `passages`, `supported_hz`, `limitations`, and `sources` (each with
`citation`, `locator`, `license`, `distribution`, `capture_notes`). Distribution is
`private_only`, `redistributable` or `original_authored`; this is recorded provenance,
not an automated licence judgement. It measures only the specified group's existing
EQ output, records the actual analysis settings, sample rate, phrase intervals,
active duration, uncertainty and source/baseline hashes, and writes no audio.

Use representative isolated phrases, consistent tuning/register and playing technique,
and separate microphone/DI contexts. Measured maps require compatible capture and
instrument, overlapping known register, and matching declared tuning/technique.
Unknowns remain in the map's limitations. Full-mix references must not be assigned
an isolated instrument identity. Imported JSON with another analysis version, missing
provenance, invalid dimensions/ranges or unsupported values fails validation.

## Ensemble validation

Training fixes the proposal. Held-out passages only accept or veto it. A new intermediate
amount is measured through the production chain; the full proposal and an identical
previously checked amount reuse their frozen checks after source/state identity
verification. Slider movement never starts a new fit. Existing compressor, excitation, FX sends/returns, master settings and
faders are retained. Input trims remain unity and verified DI routing is preserved.
Unknown Dark Ride bass capture remains unknown; it cannot enter a DI-specific map.

Reports include body/presence, level and crest, compressor reduction, generated return
levels, ensemble low-band power and L/R correlation. Fixed 20 ms raw energy-rise
anchors give attack (0–40 ms), body (40–120 ms) and sustain (120–300 ms) comparisons.
These are articulation proxies; rapid notes/compound events can overlap them.
They do not identify individual performances or establish preferred musical articulation.

Each passage needs 1.5 active seconds and eight supported points. Spectral error
cannot increase more than 0.15 dB; a material remaining deviation needs measured
progress. Group window level change is bounded to 3.05 dB, crest loss to 1.5 dB,
added compression to 1 dB, ensemble level/peak rise to 2 dB, L/R correlation change
to 0.10 and added master reduction to 0.1 dB. Event stage changes are bounded to
±3 dB. Repeated PCM contacts retain the existing 0.1%/three-window veto.
These are declared experiment guards, not musical preference or hardware safety.

Full auditions use the existing production renderer and independently finalize to
−0.01 dBFS **sample peak**. No equal-LUFS target, matched copy or source normalization
is used. Exact excerpts retain parent samples and gain. FX excitation may alter the
final peak and therefore the common export gain; that consequence must be reported.

## Rejected assumptions

A good finished mix does not need a correction to demonstrate this feature. The user
explicitly redirected the real-audio trial toward a deliberately coloured copy.
The unmodified complete FINAL and SOURCE stay eligible and retained. A lower spectral
error alone cannot select a preferred mix. Genre words, filenames, a low crest factor,
“raw” labels and a manufacturer's filter preset do not establish a defective input,
a reference spectrum, or a distribution right.

## Comparing EQ on identical recordings

Implemented in 0.2.2: `eq-match-compare` observes a known EQ change on identical
recordings. It separates variation across phrases from variation in a paired
processing difference. It does not select filters, make a map or change the
existing fitter's uncertainty, thresholds or saved-state schema.

```sh
target/release/gigpies eq-match-compare reference.json changed.json SOURCES NEW_REPORT comparison-request.json
```

The comparison request has the same `group`, `training`, `held_out`, `routing_basis`
and `excluded_fx_returns` fields as a matching request, with no `maps` field.
Only EQ on the named group may differ between settings. All other settings must
match, including files, rate, faders, pan, trims, compression, master and FX. Both
measurements use one source directory, continuous DSP from zero and identical
window boundaries. SHA-256 checks bracket measurement; unequal sample anchors or
changed recordings fail. Existing output directories are refused.

For every aligned, active window, the observer subtracts reference from changed
pre-compressor EQ spectra. It removes the median common difference in analysis
only. It reports median spectral-shape change and scaled median absolute deviation
of these paired differences, alongside each signal's phrase dispersion on the
same windows. This paired spread is descriptive; it is neither a confidence
interval nor a replacement for independent-reference uncertainty.

Activity is frozen from reference training. The reference's existing noise/tonal
gates apply. Windows need eight jointly supported points in 125–6300 Hz, with
less than 9 dB cancellation in both observations. A reported band needs eight
windows and support in 70% of paired windows. The representative-phrase flag also
requires three active seconds, eight supported bands and the existing 0.35 dB
reference phrase-variation minimum. Sparse/unsupported bands report null, and
silence cannot masquerade as an exact match. Joint support is descriptive and
cannot choose an EQ fit. Held-out observations never affect training statistics.

`comparison.json` retains both settings, hashes, group/split identity, the training
summary and per-passage spectra and production-DSP interactions. The two original
measurement files permit reconstruction. Level, crest, compression, coherent
ensemble low-band power, stereo, generated returns and existing raw-event
attack/body/sustain proxies are reported separately from spectral dispersion.
No settings, acceptance decision or audio is written. Full-song export gain is
unmeasured by this command; it needs complete independently finalized exports.

New comparisons also write `COMPARISON.md`. It names each FX return and separates
paired spectral spread, signed level changes, added compression/master action,
crest/stereo changes and raw-event stage proxies. Missing observations display
as unmeasured. The recorded instrument/capture context and input assignments stay
visible; no filename supplies a new capture identity or musical verdict.

An existing comparison can be summarized without opening recordings:

```sh
target/release/gigpies eq-match-compare-review SAVED_COMPARISON.json NEW_SUMMARY.md
```

This command checks saved settings identities, the EQ-only scope, passage labels,
spectral support and observation structure before creating the new file. The
summary pins the exact report bytes by SHA-256. It does not authenticate the
original measurement history or verify current source files. Existing files are
never overwritten. The original injection and candidate comparisons were both
summarized this way, with their source JSON unchanged; summaries are in
`artifacts/automix/fx-calibration-review-2026-10-03/comparison-reviews/`.

This diagnostic requires the same recorded performance. It can attribute a known
processing change without making the broader claim that independent players,
notes or captures have become comparable. Unknown Dark Ride capture identities
remain unknown. A future method for independent reference comparability needs its
own frozen study and production-DSP/listening validation.

### Continuous history and new planning admission

New `eq-match-plan` requests require every training passage to end at or before
the first held-out passage starts. Nonoverlap alone is insufficient when channel
filters, compressors and effects run continuously from sample zero. The fitter,
maps, search budget, uncertainty and numerical protection limits are unchanged.
The admission rule is shared with the artistic FX workflow.

A fixed synthetic probe changed only samples at 8.096–8.192 s, inside an earlier
held-out passage. With existing 125 Hz / +12 dB / Q 10 channel EQ, later training
had identical raw anchors but one processed spectral window changed by 0.006126 dB.
The pooled training shape changed by only 1.26×10⁻⁹ dB. This demonstrates a causal
dependency; it establishes no meaningful correction or preferred sound. There was
no corrective fit, parameter retry or real-audio evaluation. Evidence and the
declared two-case budget are in the local `eq-history-probe/` session evidence.

Saved states retain their existing schema, frozen identities and exact reset.
They remain readable and usable with their original evidence; newly written HTML
reviews disclose an interleaved historical split. Reset of the pre-change synthetic
state was verified without recordings, preserving its baseline and frozen identity.
Diagnostic comparisons can still describe interleaved recordings and label that
history; they make no new fitting or independent-validation decision. The saved
Dark Ride fresh-reference study already uses chronological passages and is unchanged.

### Fresh reference study after 0.2.1 — 2026-10-03

One fixed reference phrase now yields a bounded partial correction of the known
Dark Ride injection. The unmodified production receives no EQ. This establishes
technical recovery in this controlled case; the source production is not diagnosed
as defective, and musical acceptance remains pending.

Before measurement, the frozen study plan (the generated output tree is now retired) fixed one
reference at **132–144 s**, training at **132–144 and 156–168 s**, and held out
at **228–240 and 264–276 s**. None overlaps the initial EQ pilot's intervals.
The song was previously studied in full, so these are fresh within-song checks,
not independent blind material. Chronological selection preceded measurement;
no pitch/technique equivalence was inferred. The reference uses one declared phrase
instead of pooling three passages. Its measured uncertainty remains intact.

Budget: one settings-copy injection of **+4 dB / Q 0.7 / 2 kHz**, one recovery
fit, one clean control, zero held-out retries, zero fader/FX recalibration and zero
new auditions. The fitter, uncertainty rules and every numerical guard are unchanged
from `6682409`. The paired diagnostic supplies no parameters to this fit.

The proposed correction is **−1.5 dB at 2 kHz and +0.25 dB at 500 Hz**, both Q 0.7,
appended to the injected settings copy. All four passage checks pass. Training
residual RMS outside uncertainty falls from **0.599 to 0.024 dB** in prediction.
The unchanged production's training residual is **0 dB**, with no proposed filters.
In the diagnostic recovery review, 0% and reset restore the injected settings copy.
The separate clean-control review and retained FINAL preserve the original production.

| Actual candidate versus injected copy | Held out 228–240 s | Held out 264–276 s |
|---|---:|---:|
| Residual RMS outside uncertainty, before → after | 0.411 → 0.000 dB | 0.360 → 0.091 dB |
| Guitar-group RMS change before export | −0.497 dB | −0.433 dB |
| Body/presence change | +1.170 dB | +1.231 dB |
| Generated guitar-reverb return change | −0.582 dB | −0.527 dB |
| Ensemble RMS change | −0.075 dB | −0.094 dB |
| Largest window crest loss | 0.694 dB | 0.638 dB |
| Added compressor / master reduction | 0 / 0 dB | 0 / 0 dB |

The residual metric excludes the reference interval; zero does not mean the entire
+4 dB coloration was inverted. Guitar compression remains bypassed. The return
change is altered excitation at fixed sends/returns. Held-out attack/body/sustain
proxies change by **−0.104/−0.289/−0.179 dB** over six events and
**−0.015/−0.176/−0.430 dB** over one event respectively. These few raw-energy
anchors do not establish general articulation preservation. The second training
passage has no eligible event anchors. Stereo and low-band checks remain in the
private report; no numerical result supplies listener approval.

The diagnostic isolates the injected change on identical recorded windows: the
training median shape rise at 2 kHz is **+2.631 dB**, with **0.105 dB** paired
dispersion versus **2.028 dB** original phrase dispersion on those windows.
The common spectral offset, removed only in analysis, is **+1.206 dB**. Held-out
shape rises are **+2.592 and +2.656 dB**, each with about **0.13 dB** paired
dispersion. These scaled median absolute deviations describe variability, not
confidence intervals. They are not substituted for the map's uncertainty.

After correction, the paired 2 kHz shape excess over the original tone remains
**+1.529/+1.544 dB** in held out. Guitar RMS remains **+0.891/+0.772 dB** and
its generated reverb **+1.138/+1.014 dB** above the original production before
export. The candidate retains coloration and stronger FX excitation; the low
uncertainty-residual score must not be read as complete restoration or preferred sound.

The initial study wrote no audio. A subsequent user request authorized two complete
diagnostic exports from these frozen settings, with zero new fits or FX calibration.
Each includes all eight existing FX buses and six seconds of tail, independently
finalized to −0.01 dBFS sample peak. Coloured/corrected export gains are
**−3.720112/−3.341220 dB**; finished loudness measures **−14.8/−14.7 LUFS** with
no loudness targeting. The correction lowers full-song guitar-reverb RMS by
**0.680 dB before export**; its **0.379 dB** extra export gain leaves a **0.301 dB**
decrease in that return in the finished file. All channel/master compressor reductions
remain zero. Other FX return meters are identical before export.

The index `artifacts/automix/eq-audition-2026-10-03/LISTEN.md` links both full mixes
and exact 228–240 s parent-sample excerpts, plus the existing clean production for
context. All 39 original-source hashes and 83 comparator pins match. The six existing
FINALs and SOURCE exports retain their exact hashes and export gains (**0 dB change**);
the original seven-pair queue remains available. These diagnostic mixes do not
replace the healthy production, and neither has been accepted by listening.

Normal validation passes **130 Rust tests and 34 Python tests**, including 13
matching regressions. Three historical media tests and unrelated exhaustive renderers
remain opt-in. Formatting, Clippy with warnings denied and the locked release build
pass. The Pi reboot interrupted a subsequent paired observation, after both planning
commands had completed. Boot-time filesystem recovery and saved-file hashes were
checked before resuming the identical observation; neither fit was rerun or retuned.
Local details are in `artifacts/automix/eq-comparability-v1/REPORT.md` and its
`filesystem-check/REPORT.md`.

### Remaining questions

The initial pilot's broad reference variability hid a known change; that conservative
miss remains evidence. The fresh fixed-phrase result supports partial recovery of
one controlled coloration without demonstrating general repair of poor recordings.
Further separately budgeted studies need independently comparable captures and
phrases; dispersion from identical recordings must not replace their reference uncertainty.
Different tunings, registers, techniques, noisy distortion and independent performers
need broader evidence before these profiles become a general-purpose tone library.

Musicians still need to judge body, pick attack, sustain, articulation, bass support,
stereo relationships and the combined supplied/generated ambience. The authored dark
and acoustic directions are provisional and have no new real-audio listening approval.
The current six mixes remain the listening comparators. Hardware, live parameter
smoothing and the multi-musician workflow are still unverified.

## Historical evidence

The [dated study and original results](../archive/studies/EQ_MATCHING.md) retain experiment-specific settings and validation history.
