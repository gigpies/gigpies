# Offline musical balance

Measure an ensemble and optionally propose bounded fader changes from an explicit musical policy. Without a policy, preserve faders and report observations. See [source preservation](SOURCE_PRESERVATION.md).

These commands are components of the automixing work, not a complete unattended
musical-balance system. The default preserves existing faders. The experimental
role policy has a lead-presence/guitar relationship but no backing-vocal move;
the optional hierarchy below solves a supplied ordering rather than vocal
prominence. The [owning ensemble task](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-auto-ensemble--observation-led-whole-band-automixing)
tracks the measured decision workflow needed to remove per-song manual tuning.

## Formula-driven spectral candidate

The reference-free spectral experiment uses the production ensemble analyzer and
`scripts/spectral_balance.py`. It measures every channel before and after channel
DSP, reconstructs the coherent pre-exciter/pre-FX mix spectrum, and proposes
bounded reductions against an explicit **equal-power-per-octave** target. This is
a pink-spectrum engineering hypothesis, not a universal ideal curve or a reference
recording. It neither copies accepted audition gains nor learns from those mixes.

```sh
# PREPARED.json preserves the current processing, routing and effect controls.
target/release/gigpies ensemble-request request.json
target/release/gigpies ensemble-analyze PREPARED.json SOURCE_DIR scratch-analysis request.json
python3 scripts/spectral_balance.py compact scratch-analysis measured
python3 scripts/spectral_balance.py calculate measured/measurement.json calculated
# Render calculated/settings.json with the existing fixed output-gain workflow.
```

Keep `request.hierarchy` absent/null and tone profiles empty for this measurement
flow. Analysis admits prepared unity trims and preserves the native source
clock/timeline. Optional `compact --families families.json` binds filenames whose
role is `other` to a known musical family, such as `bass`, `hi_hat`, `keys` or
`synth`. This supplies identity, not a level or EQ prescription. Unknown families,
silent sources and room microphones do not receive automatic changes in this
first spectral policy. Related multi-microphone decisions need explicit grouping
before this workflow can claim general unattended mixing.

For band bounds `a,b`, the target power is `C * log2(b/a)`. `C` is fitted from the
octave-width-weighted median measured power per octave in supported bands between 25 Hz and 12 kHz.
The loss penalizes excess more than 3 dB above that target, weighted by octave
width. It does not boost absent content to fill every dip. A second cost protects
an instrument's energy in bands that were not excessive. The solver tests 0.5 dB
fader reductions and broad bell cuts, picks the largest supported loss reduction,
and stops when no admissible improvement remains or the finite step budget ends.
It limits fader reductions to 3 dB, each added bell to 3 dB and new bells to two
per channel, respecting the existing eight-band limit. Vocal articulation and
instrument fundamental ranges have a cumulative predicted 1 dB protection budget.
No source ordering relative to kick or bass is enforced.

The measured mix spectrum retains phase interactions: with `N > 2` channels,
`P_mix = (sum(P_rest_i) - sum(P_i)) / (N - 2)`. These spectra come from matching
production Hann windows. Window summaries are duration-weighted, including partial
tails. The candidate model holds baseline cross terms fixed, evaluates EQ at band
centres, and refuses material changes in strongly coherence-sensitive bands.
It abstains from EQ proposals when dynamic EQ is present or observed compression
exceeds its model's 3 dB reduction budget. These limits make it a conservative spectral proposal;
they are not proof that compression, excitation or effect returns will follow the
prediction exactly. Recalculate from the same frozen measurement; do not repeatedly refit the curve
from its own output and accumulate cuts. The audio uses the real production DSP, unchanged effects,
limiter and the previous fixed delivery gain, with no post-render level matching.

`measured/measurement.json` retains settings, input identities, activity, raw and
routed per-channel spectra and measured mix powers. `calculated/calculation.json`
records the target, each chosen move, cost changes and predicted spectrum;
`calculated/spectra.html` shows every channel and the mix/target comparison.
The optional compressed temporal trace emitted by compaction is disposable after
review when the retained summaries suffice. Remove the large `scratch-analysis`
output once compact evidence and source identities are retained.

Run `python3 -m unittest discover -s scripts -p 'test_spectral_balance.py'` for the
focused calculation/streaming regressions. This experiment does not yet implement
independent vocal/backing prominence, event recognition or a bounded live
controller. Its output is a calculated candidate for listening, not an automatic
claim of a musically correct mix. Those requirements remain in the owning task.

## Commands

Use a new directory for every run. The baseline must already have DI-only bass,
unity input trims, explicit channel/master HPF settings (including bypass),
`output_mode: "unmatched"`, and a −0.01 dBFS ceiling. Incompatible settings are
rejected, not silently converted. Pan, BWF timing, stereo linking, compression,
EQ, FX and master settings are preserved by the fader pass.

```sh
cargo build --locked --release
# Measure without rendering or changing settings:
target/release/gigpies balance-analyze baseline-settings.json source-dir new-analysis-dir
# With an explicit reviewed musical policy, search and measure a fader proposal:
target/release/gigpies balance-pass baseline-settings.json source-dir new-pass-dir policy.json
# Without a policy, both commands observe and preserve faders; no taste targets.
```

`A-settings.json` is the frozen fader candidate. The `A/` directory contains the
full render, independent export gain, channel/master histories and bus/FX meters.
`optimization.json` records evaluations, proposed fit/held-out results and acceptance.
Rejected proposals produce unchanged faders; their proposed metrics remain available
for diagnosis. `status.json` separately records technical completion, whether all
measurable policy targets were met, and an unset listener-preference field.
`decisions.json` records old/new faders and evidence references.

After rendering A, inspect its evidence before editing a copy as `B-settings.json`.
Log each processing edit's old/new value, evidence and tradeoff. Keep all settings
editable and avoid adding compressor compensation a second time. Render B with
`render`, then remeasure with `balance-analyze` and the same policy. A processing
hypothesis can be rejected without changing A. Neither command adds master notches,
channel normalization, matched listening copies, time alignment or polarity flips.

## Kick-led LUFS hierarchy

An explicit alternative to the band-ratio search is `loudness-pass`:

```sh
target/release/gigpies loudness-policy new-loudness-policy.json
target/release/gigpies loudness-pass baseline-settings.json source-dir new-loudness-pass new-loudness-policy.json
```

This optional command generates a legacy policy of **kick 2 LU above bass; all
other individual sources at least 1 LU below bass**. These are command defaults,
not the current audition intent: the user removed that musical hierarchy. See
[current balance intent](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-auto-ensemble--observation-led-whole-band-automixing). The policy contains `kick_to_bass_lu: 2`,
`bass_to_others_lu: 1` and `max_fader_change_db: 6`; all are editable. This is
an artistic choice, not a broadcasting standard or a universal mix rule.
The command keeps kick fixed, moves bass toward its offset, and only lowers
other sources that exceed their ceiling. Already quieter sources stay quieter.
It never equalizes all sources or uses the summed ensemble as the competitor.

The existing prepared DI-only unity-trim baseline contract above applies.
Exactly one kick and one DI bass are required; multiple microphones need an
explicit combined-source policy first. FX configurations are currently rejected,
so excitation or returns cannot silently escape measurement. The pass measures
native-rate production channel EQ/compression and fader/pan on the common BWF
timeline, before master processing and export. It uses the existing K-weighted
400 ms integrated meter, with absolute and relative gating independently for
each source. These are whole-recording contribution measurements, not
simultaneous playing-window comparisons, transient rankings or guarantees in
every section. Sparse sources and bleed still require contextual review.

`settings.json` contains the bounded candidate; `baseline.json` preserves the
input. `report.json` records before/after LUFS, exact desired levels, fader changes
and `policy_targets_met` from actual remeasurement (0.1 LU tolerance).
`status.json` is written last. A change-budget or fader limit can prevent the
requested hierarchy: the unmet target remains visible, and the candidate is
not labelled compliant. Missing kick/bass loudness produces no changes;
other silent/too-short sources remain unchanged and unresolved, so full
compliance stays false. The command saves settings and evidence without
rendering or playback. Render the selected settings with the existing command.
Without this explicit command, existing balance and render behavior is unchanged.

LUFS discounts very low frequencies but cannot determine whether bass notes are
clear against the other instruments. A sub-heavy bass may still consume headroom
or trigger broadband compression while lacking definition. Use the
[bass diagnosis and correction pass](BASS_KICK.md) to inspect low-band energy,
100–400 Hz body, 400–1600 Hz definition, fundamentals, crest loss and compressor
reduction before selecting tone changes; remeasure a LUFS hierarchy only if
that optional policy was explicitly selected.
Do not compensate for weak definition by blindly increasing the whole bass.
The current bass rule is an optional bounded EQ proposal with held-out guards,
not an automatic perceptual masking model or a guarantee of audible articulation.

Validation: `cargo test --locked --test loudness_hierarchy` covers source ordering,
retained quieter sources, gain-offset invariance, missing anchors, change bounds,
actual DSP remeasurement, non-overwrite, unresolved silence and K-weighting's
low-frequency attenuation. Musical acceptance requires auditioning actual material.

## Measurements and confidence

Analysis runs the production channel strips, excitation, fader/pan routing, FX and
master processing at native rate. Each 20 ms window records:

- Per-channel input and pre/post-compressor RMS in dBFS, peak, instantaneous mean/max
  gain reduction in dB, stereo correlation, activity, confidence and event flags.
- Routed stereo covariance for broadband and approximate 40–120, 120–500, 500–1500
  and 1500–5000 Hz bands. Band filters subtract adjacent second-order Butterworth
  HPF outputs; they overlap and are not rectangular FFT bands. Group energies include
  covariance cross terms, so correlated microphone cancellation is retained.
- Separate close drums, overheads, drum room, bass, guitars, direct vocal, vocal room
  and summed generated-return energies in `groups.csv`. Excitation is included in
  each source; its separate residual meter and per-return totals are in the renderer's
  `measurements.json`. The analysis covers the source timeline; render meters also
  cover the configured FX tail.
- Master mean/max reduction and pre-export RMS in `master.csv`.

Activity requires post-EQ/pre-compressor RMS above −75 dBFS and within 30 dB of
that source's p95 window. Confidence is an explicit heuristic score, **not a calibrated
probability**: 0.85 for activity separated from the source's p10 by 6 dB, or sustained
activity with p10 at least 3 dB above the absolute floor; otherwise 0.55. Rejected
activity has zero confidence; ambiguous tom bleed is capped at 0.35. The default
search requires 0.65. Very quiet playing below the floor remains unresolved; the
floor and relative margin are editable. These rules do not identify words or notes.

Drum events require a 5 dB rise over the preceding 60 ms and 120 ms between triggers.
The event anchor is the strongest role-band window in the following 60 ms, with
20 ms resolution. Kick uses the low band; snare/toms use the body band. Reported
attack/body/decay windows are 0–20, 20–80 and 80–240 ms from that anchor. Thus the
attack measurement describes the early peak region, not a sample-exact acoustic
onset. Phrase-activity starts use 0–100, 100–300 and 300–600 ms envelopes.
Adjacent events can overlap the decay; inspect `events.json` and `windows.csv`.

Tom bleed rejection explicitly compares coincident kick/snare events (±20 ms).
With at least three reference events, it estimates a median tom/reference band-energy
ratio. A coincident tom candidate within 6 dB of that reference is marked ambiguous.
Only accepted tom events and their next 240 ms qualify as active tom playing.
This can reject genuine unison hits and miss delayed/independent bleed. It neither
separates sources nor estimates a clean tom waveform. `microphone-relationships.json`
reports simultaneous correlation, event ratios and actual summed power versus
incoherent power for related drum and vocal microphones. Negative sum differences
flag possible cancellation, not a proven phase defect. No automatic correction follows.

Active-source summaries describe each source; **they are not cross-instrument fader
targets**. Ratios used for optimization always use identical windows on both sides.
In particular, an intermittent tom's whole-song RMS is never compared to a continuous
guitar to choose its fader.

## Listening evidence

Each finished PCM file is independently finalized from its own sample peak to
−0.01 dBFS by the existing exporter. No raw-reference gain or LUFS target is used.
Float sums can exceed full scale without clipping. This is **sample-peak** protection;
no true-peak claim is made. Keep the baseline and previous renders intact.

```sh
python3 scripts/balance_excerpts.py OLD.wav A.wav B.wav new-excerpts-dir 12 36 72
```

This optional standard-library script checks matching formats/frame counts and the
finished sample peaks, then copies exact 12-second PCM slices. It writes a manifest
with full paths, hashes, frame positions and explicit **OLD → A → B** order at each
start. Excerpts retain their full-song gain; they are not individually normalized.
The script verifies the copied PCM bytes and never plays audio.

For the local experiment, settings, analysis, processing decisions and full-song
A/B renders are under `artifacts/automix/musical-balance-v2/`. Private state and media
stay ignored. The local `evidence/` report records measured outcomes and unresolved
conflicts; it is not a listening verdict. Listen for kit impact versus cymbal/room
level, tom fills, guitar weight during singing, vocal-room prominence and FX tails.

## Research applicability

Reviewed 2026-10-01:

- [Moffat and Sandler, AES 20355](https://aes.org/publications/elibrary-page/?id=20355):
  the accessible abstract identifies interference between live drum microphones as
  a problem for treating each track as an independent source. Its proposed equal
  perceptual loudness objective is explicitly outside this experiment's requirements.
  We inspected the abstract, not a subscriber-only full paper; no source-separation
  implementation or numerical target is attributed to it.
- [Ronan et al., arXiv:1803.09960](https://arxiv.org/abs/1803.09960): the current version
  is withdrawn, with an intellectual-property ownership note. Its abstract discusses
  a psychoacoustically inspired masking metric and subgrouping. Our ratio proxy does
  not implement that model, reproduce its results or inherit its validation.
- [De Man, PhD thesis](https://www.brechtdeman.com/publications/pdf/PhD-thesis.pdf):
  the retrievable indexed perceptual-evaluation section compares automatic and human
  mixes using listening tests. Full-PDF retrieval failed in this session; applicability
  is limited to that section. It supports keeping perceptual evaluation separate from
  technical measurements, not the numerical ranges above. The normalized-input
  comparison described there is not used here.

## Validation

`cargo test --locked --test balance` covers known buried-hit and excessive-room
faults, vocal/guitar competition, no-change, silence, low confidence, quiet playing,
coincident bleed, stereo/phase relationships, global-gain invariance, bounded and
repeatable search, held-out veto and evidence non-overwrite. Synthetic audio checks
activity and routing independently of the optimizer's objective. The full normal
production suite also covers BWF routing, linked DSP, FX, persistence and export.

Private full-song renders and excerpt verification are explicit opt-in preparation,
not normal CI tests. Historical source-variation and older audition tests remain
ignored unless their own protected behavior changes. No playback, host-audio change,
hardware verification or listener preference follows from passing these tests.

## SOURCE and paired ensemble review

The [Complainiacs reassessment](../archive/studies/COMPLAINIACS_REASSESSMENT.md) adds
`balance-source-analyze INITIAL_SOURCE.json SOURCES NEW_DIRECTORY` and the
`scripts/ensemble_review.py` observer. SOURCE bypasses all DSP while retaining the
supplied initial pan/faders. Processed measurement and optimizer behavior remain.
Group CSVs add coherent `drums` and `vocal_sum` groups and an
`incoherent_broadband_dbfs` column; existing fields retain their values.

The paired report fixes activity to explicit SOURCE evidence with training-only
thresholds, checks identical raw windows/routing, and retains section and raw
quiet/strong strata. Export gain is separate from pre-master group energy. It never
turns a relationship into a preferred target or a listener verdict. Existing
optimizer failures remain failures even when a separately documented artistic
selection follows revised listener direction. See the reassessment for that
choice, its remaining room-ratio failures and the bounded rejected EQ probes.

Group CSVs also expose `snare`, `kick`, `toms`, `drum_ambience`,
`drums_without_snare`, `ensemble` and `ensemble_without_snare`. Both ensemble groups
include generated FX. Coherent sums retain signed microphone interaction; the
power of the sum need not equal the sum of separate powers. These are mixtures,
not separated snare or bleed estimates. All groups are before master/export.

`scripts/snare_context.py SPEC.json training|held_out NEW_REPORT.json` summarizes
fixed baseline events in these groups. SPEC supplies `rate`, `source_analysis`,
`diagnosis` and `variants` with `id`/`analysis`. It requires explicit SOURCE bypass
evidence, unchanged raw inputs and complete event windows; training determines
quiet/strong and vocal-activity thresholds. Reports retain section, rapid/compound
contexts, signed interaction and insufficient counts without selecting settings.

## Historical evidence

The [dated study and original results](../archive/studies/BALANCE_PASS.md) retain experiment-specific settings and validation history.
