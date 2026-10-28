# DI bass definition and kick interaction

Analyze verified DI bass and kick relationships, then evaluate bounded corrections against explicit intent. Channel and master high-pass filters may be bypassed; measurements follow configured processing. Unknown identity or insufficient evidence prevents correction.

## Energy versus audible definition

A bass fader changes sub, body and definition together. Strong lows can occupy
headroom and drive the full-band compressor while weak harmonics remain hard to
follow. The analysis therefore keeps low-frequency bands separate from
100–400 Hz body and 400–1600 Hz definition, and inspects compressor reduction
and crest loss before accepting an EQ proposal. It does not try to flatten all
bands or make every bass note equally loud. EQ can rebalance existing harmonics;
it cannot reconstruct absent attack or harmonics, and this pass does not synthesize
them. Source changes or a separately selected nonlinear treatment need their own
review when rebalancing existing content is insufficient.

The [kick-led loudness policy](BALANCE_PASS.md#kick-led-lufs-hierarchy) sets bass
2 LU below kick and other individual sources at least 1 LU below bass. Run that
measurement after tone decisions: LUFS positioning and spectral definition solve
different problems. Neither measurement establishes that bass is unmasked in the
ensemble or translates to a particular loudspeaker.

## Explicit low-frequency intent

The body band starts at 100 Hz; the analysis does not. The original nine bands
include **25–40, 40–65 and 65–100 Hz**. Every measured stage now also reports
`low_energy`: below-25 Hz power (including DC), 25–100 Hz power, the fraction of
full-spectrum power below 100 Hz, and 400–1600 Hz definition relative to all
energy below 100 Hz. Silence has no meaningful fraction or ratio (`null`).
One-sided FFT power counts DC and Nyquist once. These are Hann-window spectral
power estimates, not perceptual loudness, rectangular-time RMS or evidence that
all low-frequency content is unwanted. Frequency resolution is sample-rate / 16384.

`bass-analyze` now writes `diagnosis.json` as well as full measurements, including
training-only low/definition balance and low-energy share. Inspect the raw,
post-EQ/pre-compressor, post-compressor and post-master-HPF taps together. A master
HPF can remove lows from the output after those lows already drove compression.
The new validator rejects added pre-compressor below-100 Hz power above 0.1 dB
in active windows when the explicit low-deficiency condition triggered correction.
It does not infer that sub caused compressor action merely from correlation.

To evaluate low-heavy bass independently of the body balance, add this optional
field to the explicit bass policy:

```json
"definition_low_db": [-18, -6]
```

This example means 400–1600 Hz definition should sit between 18 and 6 dB below
all below-100 Hz power. It is a provisional test range, **not a universal bass
preset or a value derived from the old song study**. Choose intent from source
observations and listening. Existing policies without this field retain their
body-only objective; low measurements are still reported. Large low share by
itself never authorizes cutting a musical fundamental.

Either explicit ratio being below its range in at least 70% of eligible training
windows can trigger the search. When low deficiency triggers, the grid adds 45
candidates: broad −1/−2/−4 dB shelves at 35/60/90 Hz, alone or with +2/+4 dB
700/1000 Hz definition bells. The existing repeated-note fundamental and spread
limits remain unchanged, so an over-cut fundamental rejects a candidate. Predicted
below-100 Hz power may not rise by more than 0.1 dB in eligible training windows.
Both objectives enter the cost; each retains its per-window regression guard.

Actual DSP validates the single frozen proposal on both splits. The combined
median target-violation improvement must reach 0.25 dB in each split; neither
individual median violation may worsen by more than 0.1 dB. Output, crest,
compressor, source-contact and note-support guards still apply. `accepted` means
bounded improvement; `median_targets_met` separately states whether both split
medians reached all configured ranges within 0.1 dB. It does not claim every
window met intent or that the result sounds preferable.

The retained [song study](../archive/studies/BASS_KICK.md) motivates separating
low support, weak definition and compression rather than assuming one cause.
The source/evidence folder was unavailable for the 2026-10-28 revision, so this
revision has controlled-signal validation, not a fresh measurement or audition
of that song. Regression coverage includes a source with acceptable body balance
but excessive low/definition ratio, below-25 Hz energy/DC accounting, preserved
legacy intent, held-out independence and rejection of hidden pre-compressor load.

## Implemented measurement and decisions

`src/automix/bass.rs` owns the offline rule. It reuses production source reading,
BWF alignment, biquads, stereo-linked compression, routing and FFT. It does not
modify the renderer, session schema, master chain or hardware.

```sh
CARGO_INCREMENTAL=0 cargo build --locked --release
# Diagnosis only; start/end are seconds on the common source timeline.
target/release/gigpies bass-analyze prepared.json sources new-evidence bass-policy.json 0 120
# Propose once, validate actual DSP, save settings; does not render or play.
target/release/gigpies bass-correct prepared.json sources new-review bass-policy.json 0 120
# Render only after reviewing the recorded result.
target/release/gigpies render new-review/settings.json sources new-render
```

All output directories must be new. Both measurement commands require the existing
prepared DI-only unity-source contract, explicit channel/master HPF settings
(including bypass), unmatched −0.01 dBFS finalization and no FX configuration.
Explicit indices **and filenames** must match bass DI and kick roles.

Example policy; choose indices, filenames and intent for the supplied session:

```json
{
  "bass": 7,
  "bass_file": "08_BassDI.wav",
  "kick": 0,
  "kick_file": "01_Kick.wav",
  "definition_body_db": [-12, -4],
  "max_eq_db": 4,
  "max_output_rise_db": 3,
  "max_note_spread_db": 2,
  "max_added_reduction_db": 1,
  "max_fader_db": 1.5,
  "recorded_source": true
}
```

The numerical definition range is a provisional engineering interpretation of
“more useful midrange”, not an approved style profile. These are our adaptations;
none of the new settings is labelled manufacturer-published or official.
`recorded_source: false` with a substantial correction requests another capture and
retains the old settings. This flag does not infer the physical signal path.

### Evidence and confidence

- Stream from the common origin so a pilot retains earlier filter/compressor
  history. A requested interval cannot reset an envelope at its beginning.
  The end is bounded to ten minutes. No resampling or source normalization occurs.
- Use native 16384-sample non-overlapping Hann FFT windows (371.5 ms at 44.1 kHz).
  Average stereo powers without a mono cancellation. Log nine bands from 25 Hz
  to 8 kHz and eleven stages: raw bass, EQ, compressor with makeup, routing,
  bass after master HPF, raw kick, kick/guitars/vocals after master HPF, full mix
  before/after that HPF. Instrument contributions exclude common master gain;
  the complete mix includes it. Measurements omit incomplete trailing FFT windows;
  the render keeps the full source timeline and normal renderer padding.
- Log actual linked reduction at 10 ms intervals, including mean and maximum.
  Measurements separate compressor reduction from explicit makeup and fader gain.
- Pitch analysis averages short blocks to about 6–8 kHz **for analysis only**,
  then searches normalized autocorrelation between 30 and 160 Hz. It requires
  periodicity at least 0.80, an observed fundamental with at least 3% of
  25–3200 Hz power, at least 65% concentration around ten harmonics, and pitch
  within 40 cents of an equal-tempered note. No claim of concert tuning follows.
  The averaging filter is not a general resampler; high-frequency interference,
  missing fundamentals, simultaneous notes, detuning and octave ambiguity can
  defeat the estimate. Stereo pitch follows the left source channel; stereo
  spectral power still uses both channels. Unreliable windows cannot vote.
- Even 12-second sections train; odd sections validate. Boundary-crossing windows
  cannot vote. Raw activity must exceed −65 dBFS and training p95 minus 30 dB.
  Require 12 training windows and three pitches repeated at least three times;
  held-out validation requires eight windows and two repeated pitches. At least
  70% of training votes must be below either explicit definition/body or configured
  definition/low range.
  These counts and fractions are evidence heuristics, not calibrated probabilities.

The current estimator does not identify note boundaries, instrument technique,
mic placement, pickup choice or a defective source. Within-song held-out checks do not establish generalization to other recordings.

### Fixed correction budget and guards

The base 54-entry grid contains no change; broad +2/+4 dB definition bells at 700/1000 Hz;
optional broad −2/−4 dB at 180 Hz; optional +4 dB/Q 3 bells at 45/55/65 Hz;
three standalone narrow low boosts; and two broad low-shelf relief alternatives.
Gain/capacity limits can remove entries. No centre frequency is fitted to a
spectral peak. Existing EQ remains, with at most eight total channel bands.

Training spectra predict the linear response. The objective minimizes median
summed squared distance from the configured ranges plus `0.025 * sum(gain²)`. Predictions
cannot add excessive output or worsen an eligible window's tone deviation by
more than 1 dB. Repeated-note median fundamental changes must stay within
−1 to +3 dB, with at most the policy's spread between pitches. Those limits are
conservative engineering budgets, not a claim that every note should have equal
amplitude or identical EQ gain.

The chosen candidate alone runs through actual DSP; held-out data never selects a
retry. Both splits must pass the combined-median improvement and individual non-regression
checks described above. The same fixed baseline
note/activity mask validates support. All windows with raw level above −65 dBFS,
including unpitched transients and the fading ending, retain output-rise and
1.5 dB crest-loss guards. Below-activity windows cannot gain more than 1 dB.
Actual 10 ms peak compressor reduction cannot increase beyond the policy budget;
full-mix peak cannot increase more than 1 dB. These checks do not certify inaudible
distortion, phase relationships at every frequency or preferred articulation.

The system records diagnosis, all proposals, candidate measurement, outcome and
applied settings separately. A separately measured fader-only proposal is never
combined silently with EQ. Neither rule changes makeup or input trim.

## Historical evidence

The [dated study and original results](../archive/studies/BASS_KICK.md) retain experiment-specific settings and validation history.
