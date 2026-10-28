# Automixer expansion: research reviewed 2026-10-28

Dated research record, not an implementation specification or current task tracker.
The single execution plan is [GP-AUTO-ENSEMBLE](../../development/MODULE_IMPLEMENTATION_PLAN.md#gp-auto-ensemble--observation-led-whole-band-automixing).
No media or proprietary processing code was downloaded. The engineering choices
below are our adaptations; the sources do not establish our musical targets.

## Primary sources and applicability

| ID | Source and access actually obtained | Supported finding | Application and limit |
|---|---|---|---|
| R1 | [ITU-R BS.1770-5, November 2023](https://www.itu.int/rec/R-REC-BS.1770-5-202311-I/en). Official publication/status page verified; PDF fetch timed out. Indexed specification passages were available. | Programme loudness and true-peak measurement standard; K-weighted level is a defined meter quantity. | Retain standard LUFS separately from tone and event metrics. Do not attribute our kick/bass offsets, instrument targets or an audibility model to ITU. Use R2 for directly inspected metering details. |
| R2 | [EBU Tech 3341, November 2023](https://tech.ebu.ch/docs/tech/tech3341.pdf), PDF sections 2.1–2.3 and 2.9 inspected. | Momentary measurement uses 400 ms; short-term uses 3 s; integrated gating uses overlapping blocks, an absolute floor and a relative threshold. The document supplies calibration cases and tolerances. | Verify meters with independently calculated signals. Event attack energy on shorter spans is not LUFS. A selected activity-mask energy ratio must have its own label. Broadcast programme targets do not set instrument balance. |
| R3 | Giannoulis, Massberg and Reiss, [Digital Dynamic Range Compressor Design—A Tutorial and Analysis](https://aes.org/publications/elibrary/elibrary-page/?id=16354), JAES 60(6), 2012. Publisher abstract and indexed first-page text inspected; the old author PDF redirects to a faculty homepage. | Compressors are nonlinear systems with memory; detector/smoothing/topology choices change behavior. | Measure actual production envelopes and retain continuous history when validating EQ/dynamics changes. A static spectral response or threshold formula only screens proposals. No published attack/release preset or exact algorithm reproduction is claimed. |
| R4 | Perez Gonzalez and Reiss, [Improved Control for Selective Minimization of Masking Using Inter-Channel Dependency Effects](https://dafx.de/paper-archive/2008/papers/dafx08_15.pdf), DAFx-08. PDF introduction, implementation discussion and conclusions inspected. | Cross-channel spectral relationships can drive selective adjustments. The authors explicitly describe their spectral overlap measure as excluding a psychoacoustic masking model. | Begin with simultaneous source/rest-of-mix overlap diagnostics. Label them as proxies, retain coherence and source grouping, and require listening. The paper does not validate our proxy, numerical budgets or a claim that every overlap should disappear. |
| R5 | Shure, [Microphone Techniques for Recording](https://www.shure.com/en-US/docs/education/Microphone-Techniques-for-Recording), indexed educational text and [publisher PDF](https://www.shure.com/damfiles/default/global/documents/publications/en/performance-production/microphone_techniques_for_recording_english.pdf-bb0469316afdb6118691d2f3f5e3ff01.pdf) drum sections inspected. Live HTML exposes mostly a shell. | Capture placement, proximity and interference change timbre; drum low-frequency support and attack are distinct aspects. | Analyze kick weight and attack separately, protect coherent multi-mic sums, and condition source advice on known capture. Illustrative frequency regions are detector starting hypotheses, not automatic boost instructions or proof of mic position. |
| R6 | FabFilter, [Pro-Q Dynamic EQ](https://www.fabfilter.com/help/pro-q/using/dynamic-eq) and [EQ Match](https://www.fabfilter.com/help/pro-q/using/eqmatch), manufacturer documentation inspected. | Dynamic bands have signal-dependent gain and bounded controls; matching compares captured spectra and fits a limited EQ representation. | Consider bounded dynamic correction only after static trials demonstrate a temporal conflict. Preserve baseline/amount/reset and supported-band limits. Documentation describes a workflow, not an open implementation, licensed preset collection or validated universal tone. |
| R7 | FabFilter, [Pro-DS advanced controls](https://www.fabfilter.com/help/pro-ds/using/advancedcontrols), manufacturer documentation inspected. | Full-band and split-band attenuation have different effects; sidechain choice, stereo linking and lookahead matter. | Vocal sibilance needs event-specific detection and consonant/brightness guards. High-band energy alone cannot identify sibilance. Any new de-esser requires explicit latency/timing and bypass contracts; do not reproduce proprietary detection claims. |
| R8 | De Man and Reiss, [The Mix Evaluation Dataset](https://dafx.de/paper-archive/2017/papers/DAFx17_paper_49.pdf), DAFx-17, sections 1–3 inspected from the conference archive after the author-hosted PDF timed out. | Mix preferences were evaluated with randomized, aligned stimuli and listening comments; a universally correct reference mix was not assumed. | Separate objective regression tests from listener preference, retain unchanged mixes, and evaluate natural recordings. Use a small blinded paired comparison; do not call it standards-compliant MUSHRA. The paper's media are not automatically part of this project or authorized for download/republication. |
| R9 | Glasberg/Moore, [time-varying audibility model](https://aes.org/publications/elibrary-page/?id=13391), 2005, indexed publisher abstract; direct page fetch timed out. Moore's [author review of Cambridge loudness models](https://journals.sagepub.com/doi/full/10.1177/2331216514550620), 2014, introduction, auditory-filter and partial-loudness sections inspected. | Partial loudness models distinguish a target from its background using auditory excitation and temporal processing; spectral overlap alone is insufficient. | Evaluate an auditory-model extension with declared presentation/calibration assumptions and separate target/background inputs. Bleeding microphones do not provide isolated targets automatically. Do not treat an abstract as an implementable specification or claim individual audibility from uncalibrated digital levels. |

## Evidence exclusions

[Automatic Minimisation of Masking in Multitrack Audio using Subgroups](https://arxiv.org/abs/1803.09960)
is explicitly withdrawn, with an intellectual-property ownership note. The live
record was checked. It is not an implementation authority or a source of validated
results for this work. Search-result claims alone do not override that status.

Manufacturer examples and commercial feature descriptions provide terminology and
possible mechanisms, not evidence that a particular frequency, ratio or processing
amount improves this band. Neither a matched spectral curve nor a reduced overlap
score proves preferred sound. R9 supplies a route for a deeper auditory-model evaluation; implementation still
needs complete equations, calibration assumptions and independent tests.

## Conclusions for the task

The proposed engineering direction is shared measurement with role-specific
interpretation, followed by whole-band validation. Measure low-frequency resource
use, useful tone, time envelopes and concurrent competitors separately. Keep source
preservation eligible; select the smallest supported intervention. Extend static
processing first, then add time-selective processing for demonstrated cases where
one static setting cannot satisfy the declared intent. These are design decisions
inferred from the reviewed material and repository evidence, not quotations or
claims that the cited authors implemented GigPies.

The role audit, proposed band regions, work order, budgets and acceptance criteria
remain only in the owning task card. The retained local studies under this folder
supply historical observations; unavailable source recordings prevent fresh claims
about those recordings.

## Bounded auditory feasibility follow-up

The complete executed reference kernels were inspected in
[Dominic Ward's Loudness source](https://github.com/deeuu/loudness/tree/82de790f79c5b358040861e8bdb906a55009b117),
commit `82de790f79c5b358040861e8bdb906a55009b117`, GPL-3.0-or-later.
A separate temporary C++ executable used its weighting, multi-source roex,
MGB1997 partial-loudness, integration and GM2002 attack/release modules. No reference
code, dependency, media or binary was copied into GigPies production code.
The publisher's complete 1997/2005 papers remain access-restricted; this is an
implementation feasibility probe, not a verified reproduction of either standard
or the complete 2005TV waveform model.

Eight exact two-line spectral cases used an assumed 60 dB SPL target, 60/80 dB
maskers, monaural free-field weighting, 0.25-Cam filter spacing and 1 ms updates.
The baseline proxy below is the analytic spectral-line limit of the existing
non-overlapping analysis bands. Higher partial/unmasked ratios mean less modeled
suppression; the overlap proxy has the opposite direction.

| Target / masker | Spectral overlap | Partial / unmasked |
|---|---:|---:|
| 1000 / 1000 Hz, steady | 1 | 0.310701 |
| 1000 / 1100 Hz, steady | 1 | 0.418861 |
| 1000 / 4000 Hz, steady | 0 | 0.999765 |
| 1000 / 60 Hz, 80 dB masker | 0 | 0.999846 |
| 60 / 1000 Hz, 80 dB masker | 0 | 1.000000 |
| 60 / 100 Hz, steady | 0 | 0.032914 |
| Coincident 500 ms 1000 Hz bursts | 1 | 0.310701 |
| 1000 Hz masker ends before target starts | 0 | 1.000000 |

The probe ran in **0.487462 s** on this development host, for sixteen 2 s modeled
sequences (each case plus its unmasked control); compilation is excluded. This
is neither waveform-model throughput nor a Raspberry Pi benchmark. Local inputs,
source pin, harness/hash, CSV, comparison JSON and timing are retained under
ignored `artifacts/automix/ensemble-validation-20261028/r9-*`.

**Disposition: experimental, not promoted.** The filter model distinguishes
nearby frequencies that share an analysis band and exposes low-frequency
competition across our band boundary. The preceding-masker result also exposes
this probe's missing excitation history: smoothing stationary spectra is not the
full temporal waveform model. Actual use still needs calibrated presentation,
isolated target/background evidence, waveform/reference validation and a suitable
integration/licensing decision. Bleeding microphones and uncalibrated recordings
cannot supply an absolute listener threshold. Musical preference remains separate.
