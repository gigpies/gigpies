# First offline automixer

Historical workflow retained for reproduction. For current use, start at
[Using GigPies](../../README.md) and [current status](../../STATUS.md).


For explicit delivery controls, true-peak export and production stage observations,
see [summing and delivery](../../guides/SUMMING_DELIVERY.md). The legacy `render` behavior below
remains available for exact replay of frozen sessions.

For current offline source decisions, see [source preservation](../../guides/SOURCE_PRESERVATION.md).
The preset, calibration and matching workflow below is the historical first experiment;
new sources do not require those processors.

The new [manufacturer-reference experiment](../studies/PRESET_EXPERIMENT.md) separates published
parameters, DSP adaptations, processing evidence and static fader changes.

For the current DI-only comparison with unity source gains, 90/40 Hz low cuts and
**no loudness matching**, use [UNITY_PASS.md](UNITY_PASS.md). The settings and
matching commands below describe the earlier experiments.


This documents the first pass. For optional FX and code-driven spectral corrections,
see [the effects/review pass](../../guides/FX_PASS.md).

Implemented and offline-validated; listening acceptance is pending. No live device,
network, service or host-audio code is involved. Rust 1.97.1, no sibling dependency.

## Reproduce the local experiment

Run from the repository root. Output directories and settings files must be new:
existing outputs are never overwritten. Create another destination for another take.
All example outputs below are ignored by Git.

```sh
cargo build --locked --release
mkdir -p artifacts/automix
# Skip this command if you already have the editable initial settings.
target/release/gigpies preset artifacts/automix/settings.json
target/release/gigpies soundcheck artifacts/automix/settings.json \
  recordings/sessions/complainiacs-etc artifacts/automix/soundcheck-final 90
target/release/gigpies render artifacts/automix/soundcheck-final/prepared.json \
  recordings/sessions/complainiacs-etc artifacts/automix/show
```

`soundcheck` starts from neutral and processes the complete timeline in synchronized
blocks. The last argument finishes soundcheck at that many seconds; the remaining
song uses the last applied trims. `prepared.json` saves those exact trims, all channel
processing, routing and master settings. `render` requires `prepared: true` and holds
input trims for the entire song. It represents another performance after rehearsal;
it does not retroactively claim causal gains for the rehearsal's earlier samples.
An inactive channel stays at its initial trim and should be checked before a show.
Review activity history before accepting a preparation that missed a musician.
Activity fields describe calibration: after freezing they hold their last state; a
frozen-show render has no calibration activity. Per-block level meters remain live.

Files in each output directory:

| File | Meaning |
|---|---|
| `bypass.wav` | Neutral input trim, original sources, documented pan/faders and master level. No calibrated trim, EQ, compressor or limiter. |
| `processed.wav` | Same routing and faders, with calibrated/frozen trim, channel DSP and master sample-peak control. |
| `bypass-matched.wav`, `processed-matched.wav` | Static whole-program loudness matching of those two signals; start listening here. |
| `initial-settings.json`, `prepared.json` | Editable starting state and saved preparation; versioned and range-checked. |
| `gain-history.csv` | Every block: reference RMS, group peak, activity, active duration, starting/applied/next trim and frozen flag. |
| `channel-history.csv` | Every channel/block: pre/post-strip RMS/peak and cumulative maximum compressor reduction. Channel index follows settings. |
| `master-history.csv` | Every block: master output peak, cumulative maximum reduction and affected-frame count. |
| `measurements.json`, `report.txt` | Timeline, padding, clipping/full-scale counts, channel/master measurements and comparison definitions. |

WAV exports are stereo 24-bit PCM at the source rate. No dither is added; at this
precision its omission is negligible for this experiment. If the bypass sum needs
headroom, the exporter applies one common static attenuation to **both** base WAVs
and records it. This export safety step is outside the causal engine. The measured
bus values precede that export gain; explicit export peak values follow it.
`bypass_lufs` and `processed_lufs` are legacy **pre-export bus** fields;
`loudness_meter_stage: "before_export_gain"` and the text report make that stage
explicit. Measure the finalized PCM separately for finished-file loudness.
RMS/peak report floors are −240 dBFS for digital silence.
Temporary 32-bit float bus WAVs are removed on success. Failed runs may leave a
partial directory; a completed run has `report.txt`. Do not use partial runs as evidence.

## Causal calibration

The neutral initial digital trim is **0 dB**, an explicit model of the starting
position, not a conversion from an analogue knob's “11 o'clock” position. No
microphone sensitivity, preamp gain, SPL or physical knob calibration is inferred.

Defaults in `calibration` are editable independently of the musical faders:

- Aim for −24 dBFS active RMS, subject to a −9 dBFS peak target.
- Reject blocks below −65 dBFS RMS or more than 18 dB below recent activity.
  Recent activity decays at 1 dB/s. Below-threshold quiet is labelled inactive;
  audible quiet above the threshold can settle upward. The activity threshold must
  be lowered deliberately if a genuinely quiet instrument falls below it.
- Require 250 ms accumulated activity before average-based upward adjustment.
  Active power uses a 1.5-second exponential average; gaps do not lower it.
- Bound trim to −30…+18 dB. Upward motion is at most 1 dB/s, downward 60 dB/s.
  Peak memory decays at 1 dB/s, protecting intermittent drum transients.
- Decisions see only a completed block. Apply them across the **next** block with
  a linear ramp in dB (1024 frames by default). The very first block uses neutral
  trim; neither future blocks nor whole-song measurements can affect it.
- A group uses its reference track's active RMS and the maximum peak of **all**
  members. Stereo RMS combines both channels; stereo peaks take the larger side.
  A shared gain preserves their relative input levels. Channel compression is
  stereo-linked, but separate microphones have separate channel compressors.

A surprise peak can precede the next calibration update. Floating-point processing
retains it internally; the final sample limiter catches output overs. Input converter
clipping is irreversible. Reports count full-scale/over-range samples as evidence,
not proof of the original waveform's shape. Sustained loud bleed cannot reliably be
distinguished from a musician by these level heuristics; there is no source separator.
Toms with long bleed passages deserve particular listening and history review.

## Routing, timing and musical balance

The supplied example names 13 original files explicitly; nothing guesses roles from
file order. To use other material, edit file paths, roles, groups and processing.
Group names use ASCII letters, digits, underscores or hyphens.
`role` is the semantic label; editing it alone does not replace its numeric preset.
BWF sample time references are read. If all are present, the earliest is the origin
and differences become leading zero padding. If none are present, sample zero is
assumed common and the report records null references. Mixed presence is rejected.
The first source session has BWF reference zero on every track. Unequal tails are
zero-padded to the latest source end. No resampling, polarity changes, delay estimation,
independent silence trimming or extra reverb tails are introduced. Acoustic microphone
arrival differences remain intact; metadata alignment does not prove acoustic phase.

| Source | Calibration group/reference | Fader dB | Pan |
|---|---|---:|---:|
| Kick | kick/self | −3 | centre |
| Snare | snare/self | −5 | centre |
| Stereo overheads | ambience/overheads | −10 | preserve stereo |
| Stereo drum room | ambience/overheads | −20 | preserve stereo |
| Toms 1/2/3 | separate/self | −9 each | −0.45 / 0 / +0.45 |
| Bass DI | bass/DI | −4 | centre |
| Bass amp | bass/DI | −14 | centre |
| Guitar 1, provisionally rhythm | rhythm/self | −9 | −0.65 |
| Guitar 2, provisionally lead | lead/self | −9 | +0.65 |
| Lead vocal | vocal/lead | −1 | centre |
| Vocal room | vocal/lead | −24 | centre |

Mono pan uses constant power (−3 dB on each side at centre). Stereo pan is a balance
control: centre passes L/R unchanged and turning it attenuates the opposite side.
Bass DI anchors the bass, with amp colour below it. Room sources are supportive;
linked calibration prevents normalizing a room microphone into a second lead source.
The vocal room can still cap the group's gain if it has the larger peak. Guitar roles
are provisional editorial assignments to audition, not inferred performance labels.

## Signal chain and editable presets

**Input calibration trim → 12 dB/oct Butterworth HPF → parametric bells → linked
compressor with explicit makeup → musical fader/pan → stereo sum → fixed master
level → stereo-linked sample-peak limiter.**

Every HPF frequency, EQ frequency/Q/gain, compressor threshold/ratio/knee/timing/makeup,
fader and pan appears in JSON. Filters run separately on stereo sides with identical
coefficients. The compressor uses the larger instantaneous absolute side, a soft knee
and attack/release smoothing of gain reduction in dB. There is no automatic makeup,
auto fader compensation, gate, effects send or automatic spectral carving.

The following are **our engineering choices**, not claimed factory values. All bells
have Q 0.8–1.0 (exact values in JSON); all compressors use a 6 dB soft knee.

| Role | HPF Hz | Bells: Hz / dB | Threshold dBFS / ratio | Attack / release ms | Makeup dB |
|---|---:|---|---|---|---:|
| Kick | 30 | 65/+2, 280/−3, 3000/+1.5 | −20 / 4:1 | 18 / 100 | 1 |
| Snare | 65 | 100/+3, 400/−2, 1000/+3 | −24 / 5:1 | 8 / 120 | 3 |
| Toms | 45 | 320/−3, 3500/+1.5 | −18 / 3:1 | 15 / 180 | 0 |
| Overheads | 120 | 350/−2 | −16 / 1.5:1 | 30 / 200 | 0 |
| Drum room | 140 | 400/−2 | −20 / 2:1 | 25 / 220 | 0 |
| Bass DI | 30 | 300/−3, 900/+1.5 | −24 / 4:1 | 25 / 160 | 2 |
| Bass amp | 40 | 300/−3, 1800/+1 | −24 / 4:1 | 25 / 160 | 2 |
| Rhythm guitar | 95 | 300/−2, 2200/−2 | −20 / 2:1 | 25 / 180 | 0 |
| Lead guitar | 110 | 350/−2, 1400/+1.5 | −20 / 2:1 | 20 / 150 | 0 |
| Lead vocal | 100 | 300/−2, 2800/+2 | −24 / 3.5:1 | 12 / 100 | 2 |
| Vocal room | 180 | 400/−2, 2800/−1 | −18 / 1.5:1 | 25 / 200 | 0 |

Kick weight near 65 Hz and bass definition higher up help distinguish the low end.
Broad low-mid cuts reduce buildup without allocating disjoint frequency bands. Guitar
and room faders plus modest presence shaping leave space for the vocal. No processor
tries to optimize spectral separation mathematically; overlap is normal.

The snare follows the requested body near 100 Hz and forward texture near 1 kHz.
Its 8 ms attack lets some initial crack through while the 5:1 ratio controls the body.
A faster attack would soften the hit; a slower attack would emphasize it. The 120 ms
release recovers between many hits while retaining some sustain control. Makeup
raises the compressed texture and wires once, by 3 dB. Listen for lost attack, pumping,
excess ringing, cardboard tone or exaggerated cymbal bleed before changing it.

### Primary-source research

Consulted 2026-10-01:

- [Yamaha: Equalization](https://hub.yamaha.com/proaudio/livesound/eq/) recommends
  HPFs for vocals/guitars and offers starting regions for bass/kick low-mid reduction,
  tom boxiness and overhead low-frequency control. Its snare suggestions differ from
  our requested 100 Hz/1 kHz direction. We use the broad principles with smaller cuts;
  no numerical table here is presented as a Yamaha preset.
- [Yamaha: Playing in a Band](https://usa.yamaha.com/products/contents/proaudio/musicianspa/setting/band_pa.html)
  discusses band balance, tom panning and using compression on kick/snare and bass/guitar.
- [Yamaha: Compressors](https://usa.yamaha.com/products/contents/proaudio/musicianspa/effects/compressor.html)
  describes bass consistency and drum attack applications. Exact thresholds, ratios,
  knee widths and timing here are our choices for the calibrated digital levels.

## Master and listening comparison

A fixed −6 dB master level leaves summing headroom. Final peak control uses immediate
linked attenuation with a 100 ms release and −2 dBFS ceiling. It has no lookahead or
latency and is **sample-peak**, not true-peak, limiting. It is a conservative emergency
control, not a loudness maximizer. Large reduction can distort or pump; history makes
that inspectable. No live acoustic protection or feedback safety is claimed.

Listening copies use stereo K-weighted integrated loudness with 400 ms windows,
100 ms hops, a −70 LUFS absolute gate and a relative gate 10 LU below the first gated
average, following the loudness portion of
[ITU-R BS.1770](https://www.itu.int/rec/R-REC-BS.1770-5-202311-I/en).
The native-rate filter implementation is checked against the 997 Hz reference and
stereo power relation; it is not a certified broadcast meter. No true-peak estimator
is implemented. Silence and programmes shorter than a window return no loudness.

The common listening target is −23 LUFS, lowered if either file would exceed −2 dBFS
sample peak. One constant gain is applied per entire file. Both music and pauses stay
on the same timeline. All loudness decisions happen **after** the causal render and
never feed input calibration. Quantized exports are remeasured in the local audition
check. Matching cannot make differing frequency balance sound equally loud in every
moment; switch at the same timestamp and judge tone/dynamics as well as level.

## Tests and limits

Normal `cargo test --locked --all-targets` uses only synthetic material. It covers
calibration silence/quiet/bleed/jumps/bounds, causal prefix equality, stereo linking,
filter and compressor behavior, limiter headroom, loudness, frozen state, source
integrity, BWF offsets, tail padding, invalid schema and output non-overwrite.

Explicit local checks (not CI, not the normal suite):

```sh
# Eight seconds of original kick/snare starting at 30 seconds, transformed locally
# into low/high, silence, pause, jump and clipped cases. New destination required.
cargo test --locked --release --test private_automix deterministic_source_variations -- --ignored --nocapture
# After the full-song commands above:
cargo test --locked --release --test private_automix full_song_listening_files -- --ignored --nocapture
# Repeating variations:
GIGPIES_VARIATIONS_OUT=artifacts/automix/robustness-2 \
  cargo test --locked --release --test private_automix deterministic_source_variations -- --ignored --nocapture
```

These tests prove bounded behavior and file contracts, not mix quality. Listen next
for vocal intelligibility, snare crack versus “shusta,” kick/bass definition, guitar
role balance, tom bleed/fills, room wash and natural dynamics. No audible playback,
hardware, live latency, Raspberry Pi capacity or venue safety has been verified.
