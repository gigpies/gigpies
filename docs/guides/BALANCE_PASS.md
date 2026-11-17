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

## Measured vocal decisions (offline v1)

`python3 scripts/vocal_balance.py ANALYSIS CAPTURE.json NEW_DIRECTORY` proposes
faders and writes `decision.json` plus `settings.json`. It reuses frozen
`ensemble-analyze` state and coherent measurement-group spectra, not the compact
whole-song averages. No audition material selects this policy.

Rebuild/use the analyzer from this checkout to obtain `observation_groups`:

```sh
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 \
  cargo +1.97.1 run --locked -j1 -- ensemble-analyze PREPARED.json SOURCE_DIR ANALYSIS REQUEST.json
python3 scripts/vocal_balance.py ANALYSIS CAPTURE.json NEW_DIRECTORY
```


**Frozen engineering objective, v1:** in each accepted simultaneous window fit
`C log2(b/a)` to the accompaniment using the spectral calculator's weighted
median. Measure each lead/backing ensemble in 120–5000 Hz (contribution) and
1600–5000 Hz (articulation proxy). For each range the denominator is the larger
of actual accompaniment power and its fitted pink power. Each vocal ensemble's
acceptable ratio is −4 to −2 dB (equal standing for the two ensembles, not equal
microphone faders or individual singer loudness). Minimize duration-weighted
squared distance outside that interval, equally across the two ranges and groups.
Evaluate coherent lead+backing power separately; reject candidates exceeding
+1 dB against the same accompaniment denominator in either range where both
perform. An already excessive combined contribution may only decrease.
This is a falsifiable spectral contribution hypothesis, not proven intelligibility
or a kick/bass hierarchy. The backing target applies to its coherent ensemble
sum regardless of microphone/singer count.

Search common group fader deltas from −6 to +6 dB in 0.5 dB steps. Prefer the
smallest total move on ties; retain baseline unless loss improves by 0.05 dB².
Faders must remain within −60..+12 dB. Require at least five accepted windows and
0.5 seconds per vocal ensemble, raw power above −70 dBFS, and reviewed direct
performance evidence with confidence >=0.8. Refuse groups whose 10th–90th
percentile contribution spread exceeds 12 dB or whose two range errors demand
opposite directions: those need dynamics/tone review. Confidence is a capture
review score, not a probability. Missing, unreviewed or bleed-dominated evidence
cannot authorize a boost. An energy gate alone cannot distinguish singing.

Capture metadata binds `frozen_id`, supplies `training_end_frame` (a frozen
native-frame boundary), and supplies exactly three `groups` named
`lead`, `backing`, `accompaniment`. Each has `channels` (indices) partitioning all
source channels and `observation` (the original channel index for a singleton,
or the measured observation-group name for multiple channels). Add multi-channel
groups to `request.observation_groups`, each as `{"name":"backing",
"channels":[1,2]}`. These measurement-only groups may mix instrument families
and singers, must be disjoint, and consume the existing feature budget. They
never authorize tone correction. The command verifies their memberships.
Keep tone profiles and hierarchy absent for this workflow.
For each vocal group supply `performances`: objects with `singer`, `channels`,
`capture` (`direct` or `double`), `confidence`, and nonoverlapping `spans` in native
source frames `[start,end]`. These partition that vocal group; channels for the
same singer/doubles move together. Spans are reviewed soundcheck activity/bleed
annotations, not level targets. Every active microphone in a group must have
supported activity evidence in a window before that window can authorize its common move.
Alternatively, a performance may omit `spans` and supply:

```json
"activity": {
  "mode": "isolated_voice",
  "attestation": "voice_only_no_bleed",
  "review_basis": "Describe how this source was verified to contain only this vocal performance"
}
```

This opts into `vocal-eligibility-v1`: automatic energy-window eligibility only
for an externally reviewed isolated direct vocal or double, containing no other
performance, instrument bleed or material non-vocal noise. It removes manual
phrase annotation for that supported capture; it is **not singing/bleed
recognition**. Confidence remains the supplied capture-review score (at least
0.8), never a classifier probability. The unchanged −70 dBFS raw floor excludes
unusable windows; it cannot verify isolation. Mixed stage microphones require reviewed spans or the capture-specific contrast
evidence below; unknown captures, rooms and bleed abstain. An unsupported
capture label cannot be rescued by the isolation declaration. Do not use this
mode merely because a microphone has a vocal role. Nonempty spans alongside
isolation, unknown modes and incomplete attestations are rejected.

Each microphone must supply five eligible windows totaling at least 0.5 seconds
**in each split**, in a supported simultaneous group window. Below-floor members
do not supply performance evidence. This prevents an idle or training-only
member from receiving an unvalidated group boost. Per-microphone reason counts
and split coverage are retained in `activity_evidence`; unreviewed energy stays
explicitly uncertain. Raw-source eligibility is unchanged by faders and must
match in production replay. No threshold is fitted to training or held-out data.

**Capture-specific spectral contrast (experimental):** mixed direct microphones
may instead use `capture-spectral-contrast-v1` when independently reviewed
positive and negative examples establish stable, distinguishable raw spectral
shapes for that microphone in this capture. This is a supervised capture-specific
classifier, not general singing recognition. Review must distinguish the intended
direct-dominant performance from bleed-only sound, including other singers, and
establish that microphone placement, source timbre and bleed conditions remain
representative. A voice-like bleed source can have the same spectrum as direct
voice; these observations cannot resolve that case. Use reviewed spans or abstain
when the assumption is not defensible. Synthetic separation does not qualify a
real stage microphone.

Replace `spans` with, for example, these native-frame annotations at 48 kHz:

```json
"activity": {
  "mode": "spectral_contrast",
  "assumption": "stable_capture_spectral_separation",
  "review_basis": "Describe independent direct/bleed review and capture stability",
  "calibration_end_frame": 163840,
  "examples": [
    {"span": [0,81920], "label": "direct"},
    {"span": [81920,163840], "label": "bleed"},
    {"span": [245760,327680], "label": "bleed"},
    {"span": [327680,409600], "label": "direct"}
  ]
}
```

Here `training_end_frame` is 245760; each labeled span contains five complete
16,384-frame production windows. Examples must be ordered, disjoint and
entirely within the calibration prefix or held-out split; labels cannot leak
into decision training. Each microphone, including alternate microphones of one
performance, fits its own templates. Both classes require five above-floor
calibration windows totaling 0.5 seconds. Features are square roots of normalized
raw band powers across the frozen spectral bands; level is not a class feature.
Each template is their duration-weighted mean. Distance is Euclidean distance
divided by √2. Every calibration feature must lie within 0.10 of its template;
templates must be at least 0.30 apart. Separation exceeds twice the acceptance
radius, leaving at least a 0.10 gap between class support regions. These fixed
engineering bounds restrict support; they are not learned confidence probabilities
or musically qualified thresholds. At most 4096
calibration windows per class/microphone are retained during fitting, then freed.

The templates freeze at the calibration boundary, before balance training.
Calibration windows never authorize that microphone's move. Subsequent raw
windows within exactly one template's 0.10 radius classify as `contrast_direct`
or `contrast_bleed`; out-of-support/mixed shapes abstain as `ambiguous_spectrum`.
Only direct classifications count toward the existing per-microphone and coherent
group coverage. An energetic negative or ambiguous member prevents that group's
window from authorizing a move. Five windows totaling 0.5 seconds of **each**
held-out labeled class must classify correctly; even an abstention on a reviewed
held-out example vetoes the workflow. No held-out update or threshold refit occurs.
Low review confidence and unsupported capture identities still refuse.

The `activity_evidence` record retains templates, policy, calibration counts,
held-out counts/errors and refusal reasons. Production replay compares the baseline
record with the original frozen proposal and the candidate record with that
baseline, so agreeing replay runs cannot silently replace the original templates.
Confidence remains the external review score. The classifier does not separate
signals, mute bleed, gate audio, estimate direct-to-bleed ratio or assign individual
singer gains. Complete coherent group sums and continuous FX history remain the
measured quantity, including residual bleed. Accepted settings still require the
existing fader-only production replay, lineup, composition and tail checks.
Generalization to changed capture conditions is unsupported; representative real
capture/classifier and musical qualification remain open.

**Reviewed spectral states (experimental):** `activity.mode: "spectral_states"`
opts into `capture-spectral-states-v1`. It requires the explicit assumption
`stable_capture_spectral_states`, the same independent review basis and calibration
prefix, and a nonempty `state` string (at most 64 characters) on every example.
States describe independently reviewed recurring spectral conditions within a
class, such as two direct timbres or two bleed sources. They are not singer
identities, inferred phonemes, or new fader groups. Declare states from calibration
review before decision training; held-out examples cannot introduce a new state.
Use this only when those finite states represent the stable capture. The original
single-template `spectral_contrast` mode remains available with its existing bounds.

Fit a duration-weighted template per `(label, state)` per microphone using the
same raw features, 0.10 radius and 0.30 minimum separation. Require that separation
between **every direct/bleed template pair**. Same-class support may overlap;
its union classifies as that class, without interpolation between states. Every
state independently requires five complete windows / 0.5 seconds in calibration
and held-out review. A held-out example must fit its declared state's template,
not just another state of the same class. Below-floor reviewed calibration
examples refuse instead of disappearing from the fit. At most eight states per
class and 4096 calibration windows total per class/microphone are supported.
State names carry no numerical information and never select targets or gains.
For example, a direct state `"voice"` and bleed states `"low"` / `"mid"` at 48 kHz
can use calibration spans `[0,81920]`, `[81920,163840]`, `[163840,245760]`,
`calibration_end_frame: 245760` and `training_end_frame: 327680`. Supply independent
held-out examples of the same states after frame 327680, each containing at least
five full production windows. Each example has `label: "direct"` or `"bleed"`
and `state` as well as its `span`; the state name alone never supplies a class.

Models freeze before decision training; no clustering, state discovery, radius
expansion or held-out refitting occurs. Unsupported spectra abstain; overlapping
opposite-class support, sparse states or any held-out error refuse the workflow.
Per-state templates, calibration/held-out coverage and errors are retained under
`spectral_contrast` in activity evidence and compared through production replay.
The review confidence remains external, not a model probability. Group eligibility,
lineup protection, common bounded actions, coherent signals, FX history and all
production acceptance checks are unchanged. This supports reviewed multimodal
spectra; it cannot distinguish spectrally identical direct voice and bleed or
qualify general singing recognition.

For two or more fitted bleed states, `bleed-power-mixture-exclusion-v1` also
requires separation from their possible **nonnegative band-power mixtures**.
Pairwise template separation alone is insufficient: two distinct bleed spectra
can combine into a direct-like spectrum. Before decision training, each bleed
template coordinate is bounded by `center ± √2 × 0.10`, clipped to `[0,1]`.
The coordinate extrema across states enclose the square-root features of every
convex mixture of normalized powers inside those support balls. Distance from
each direct template to that box, using the existing Euclidean / √2 metric,
must exceed `0.10 + 1e-12`; otherwise the entire workflow retains baseline
controls with an inspectable refusal. The small extra margin is numerical,
not a learned threshold. All direct states must pass independently.

This conservative box can include impossible spectra and therefore refuse
otherwise distinguishable captures. Refusal means separation was not certified;
it does not establish that a reviewed direct example is bleed. The guard only
excludes actions: it never interpolates direct support or classifies a mixture
as bleed. Bounds and per-direct-state distance lower bounds are retained under
`spectral_contrast.bleed_mixture_exclusion`, frozen from calibration and checked
against the original proposal in production replay. Held-out observations never
change the box. Existing per-state review, external confidence, grouping and
action bounds remain mandatory. Coherent interference/cancellation, changing
transfer paths and unrepresented bleed spectra are **not covered** by this
power-mixture enclosure; capture review and representative qualification remain
necessary. No new isolation assumption is inferred from passing this guard.

**Lineup protection (`vocal-lineup-protection-v1`):** each accepted window also
records the sorted eligible microphone indices for lead and backing. An
unsupported/inactive group has an empty list. The pair of lists distinguishes
solo entries, overlap and changes of active singer/capture while retaining the
measured complete coherent group signal, including fixed wet history. It does
not isolate an individual singer's contribution or identify what caused energy.
Capture assumptions and confidence/refusal rules above remain prerequisites.

The unchanged duration-weighted two-range loss is also computed separately for
each observed lineup and active group. A training candidate must not increase
any such loss (1e-9 numerical slack). Held-out lineups, including ones absent
from training, may veto an increase exceeding the existing 0.05 dB² tolerance;
they never select a new candidate. Replay requires identical per-window lineups
and checks actual lineup losses (1e-6 training slack, 0.05 held-out tolerance),
in addition to prediction, composition and tail checks. Metrics retain lineup,
per-group frames and losses in `context_losses`; `held_out_context_vetoes` names
failed contexts. This conservative guard applies even to short observed entries;
it is not a claim of sufficient per-lineup statistical coverage. Existing
per-microphone and overlap minimum coverage still applies. If no common move
passes, controls remain unchanged and the objective may remain unresolved.
No new targets, independent faders or equal division among singers are introduced.

Separate singers retain distinct `singer` identities; doubles retain their
singer identity and `double` capture label; alternate microphones of one
performance stay together in its `channels`. The default common mode preserves
all within-group offsets. The optional solo-anchored mode below can move separate
singers; it still links every capture/double of the same singer. Room balance,
relative direct/double balance and wetness decisions remain open.

**Direct-state protection (`vocal-state-protection-v1`):** for `spectral_states`,
each eligible window retains every matching frozen direct template per microphone
in `state_support`. Within each observed lineup, the complete coherent group's
existing two-range loss is protected separately on the windows supporting each
microphone/state. This prevents improvement in a frequent direct timbre from
hiding damage to another timbre of the same lineup. It is a conditional ensemble
measurement, not an isolated singer's power or a state-specific musical target.

Same-class overlap contributes to every matching support; there is no nearest-state
assignment, interpolation or Cartesian product of states across microphones.
These extra constraints do not multiply a window's weight in the main objective.
They protect per-microphone state contexts, not every joint combination of states
across microphones or every individual window within a state.
At most eight memberships per microphone/window are added. Calibration, bleed,
ambiguous, below-floor and invalid-group windows authorize no state context.
The existing review assumptions, external confidence and per-microphone split
coverage remain mandatory. A short state context receives protection without a
statistical sufficiency claim. State names carry no target information.

Training requires non-regression in every observed state context (1e-9 slack).
Held-out contexts, including supports/lineups not seen in decision training, may
veto at 0.05 dB² but never refit. Production replay requires identical per-window
memberships against the frozen proposal and candidate, and checks actual state
losses with 1e-6 training slack / 0.05 held-out tolerance. `state_losses` records
lineup, group, microphone, state, frames and loss; `held_out_state_vetoes` identifies
failed contexts. Common bounded faders, the reusable objective, attribution,
continuous FX history, composition and tails remain unchanged. If no common move
passes, retain baseline controls. These are spectral support contexts, not phoneme,
phrase or singer recognition; musical and representative-capture acceptance remain
open.

Supply `provenance`, one entry per channel: `inherited`, `preset`,
`measured_calibration`, `operator_override`, or `solver_selected`. Any explicit
override locks its whole group in common mode or its whole singer in solo-anchored
mode; all original provenance/faders remain in the
record. No inherited backing offset becomes a target.

Dry predictions apply after current channel DSP and pan, before master processing.
The three partition sums and their measured complements recover signed pair cross
powers, retaining coherent singers and lead/backing interactions. Channel
compression/dynamic EQ precede the moved fader and remain unchanged.

For configured offline FX, set `"vocal_fx_attribution": true` in the analysis
request **before** `ensemble-analyze`, then bind the capture to that new frozen
identity. Legacy dry observations with configured FX explicitly refuse. This mode
is measurement-only and cannot be combined with tone profiles, dynamic trials or
hierarchy correction. It uses the existing production rack for source-local
nonlinear excitation **before** fader/pan and for fixed post-fader delay, reverb
and deterministic chorus returns. Excitation is measured at its unchanged input,
not predicted by scaling the nonlinear input. The objective then measures each
coherent group's direct-plus-wet signal before master processing. Wet spectral
energy is not proof of direct-word intelligibility or an optimal wet/dry balance.

Attribution method `source-local-excitation-linear-returns-v1` is frozen:
independent return states receive each actual excited/routed channel; their
outputs must reconstruct the shared production return sample by sample within
`1e-7 + 1e-5 * sum(abs(component))`. These are fixed numerical tolerances, not
fitted musical targets. The supported return algorithms are linear in audio
(up to floating point and near-zero cleaning); chorus timing is independent of
input. Shared nonlinear effects are unsupported: no leave-one-out marginal
response is represented as an additive contribution. Master EQ/maximizer/limiter
are checked only in the complete production composition, never attributed to
individual sources. Additional rack work counts against the analysis work budget.

Every state runs continuously from native frame zero, through the configured tail,
without resetting at windows, activity spans or the training split. For FX,
require at least 10 ms after the last source frame and every attributed path's
peak in the final 10 ms <= `1e-5` (−100 dBFS). An incomplete tail refuses;
extending it requires a new frozen analysis/capture, not refitting the rejected
candidate. Records retain method, bounds, maximum reconstruction error, coverage
and tail evidence. The full production master output must also end below the
same peak floor over its final 10 ms. Sends, returns, excitation and all other
controls stay fixed.

Without `--source`, dry output is only a candidate and actual resulting metrics
remain absent. **FX output retains baseline controls until production replay**.
Add `--source SOURCE_DIR --analyzer target/debug/gigpies` to propose, replay and
validate in one offline workflow. `settings.json` contains accepted controls or
the unchanged baseline; `decision.json` retains proposed settings, accepted
controls/provenance, actual training/held-out metrics, prediction errors,
attribution evidence, composition checks and explicit rejection reasons.
The workflow never plays audio.

The replay command `ensemble-remeasure STATE CANDIDATE SOURCE_DIR NEW_DIRECTORY`
validates the original frozen identity, permits only fader changes, checks exact
source identities/offsets/timeline and request, and replays both baseline and
candidate through continuous production processing. Production composition
validation includes the actual master limiter and configured tails. Temporary
observations/render traces are removed after retaining the compact record.

Frozen replay policy `vocal-replay-v1` allows at most **0.01 dB** absolute
prediction error for every active lead/backing contribution/articulation window
and coherent combined window. This is numerical replay slack, not a musical
deadband. Baseline evidence must reproduce exactly; coverage/activity must match.
Actual training must retain the 0.05 dB² improvement and not regress either
group (1e-6 dB² numerical slack). Held-out group loss may rise at most 0.05 dB².
Every combined window must stay below max(baseline, +1 dB), with 1e-6 dB slack.
The frozen request's composition protections independently veto peak rise,
mix-energy loss or mono loss through nonlinear master DSP. Missing/failed replay
retains baseline; no re-search occurs. Acceptance means these offline electrical
checks passed, not final export or musical qualification.
Fit only windows ending before the training boundary. Require five windows and
0.5 seconds per group and for simultaneous lead/backing overlap separately on
each side; omit boundary-crossing windows. Missing overlap refuses a joint
decision. Missing routed power despite raw activity refuses a level correction.
The held-out side may veto a proposal when either group loss increases by more
than 0.05 dB² or any combined window exceeds max(its baseline, +1 dB). Never refit
a vetoed proposal. Training also protects each group loss and each combined
window. The record includes policy hash, sufficient window evidence, baseline,
proposed held-out metrics and, when replay is requested, actual resulting metrics.
Repeated calculation from the same frozen evidence is deterministic; its output
must not be fed back as new baseline without a new measured capture.

**Independent singers (`solo-anchored-vocal-v1`, experimental):** set
`decision_mode: "solo_anchored"` in the capture. Before freezing analysis, add
`observation_pairs` to the analyzer request: every distinct pair of vocal channel
indices, ordered smaller first, with no duplicates. For vocal channels 0, 1, 2,
use `[[0,1],[0,2],[1,2]]`. These are additional measurement requests, not changed
capture attestations. The analyzer measures each coherent routed pair sum using
the same native frames, Hann windows, DSP and optional attributed FX as the source
powers. Pair records count against the existing feature budget; legacy requests
omit them. They do not alter audio or authorize other correction modes.

For channels a,b, signed cross power is `P(a+b)-P(a)-P(b)`. The consumer verifies
pair bounds and reconstruction of the existing lead/backing groups and combined
cross power, then assembles each singer's coherent power and every singer-pair
cross term. A candidate uses `sum(g_i² P_i) + sum(g_i g_j cross_ij)`; cancellation
is retained rather than treating microphones as independent energy. Doubles and
alternate captures with the same `singer` identity share one gain and keep their
existing relative faders. An override on any capture locks that singer. Accompaniment
controls remain fixed. Every vocal microphone still needs the original reviewed
eligibility and split coverage; raw activity is never inferred from a fader.

A **solo anchor** means that singer is the sole eligible singer in its own lead
or backing group, with the group's other microphones below the existing raw floor.
The other vocal group may perform. Every singer requires at least five such windows
and 0.5 seconds in both training and held-out splits. The measured singer includes
all its captures and attributed return history, not an estimated clean waveform.
Residual returns from other singers remain in the full group predictions. Missing
solo coverage refuses independent decisions; simultaneous ensemble power is never
divided by singer count to invent a target.

On solo anchors, use the same −4..−2 dB contribution/articulation interval against
the measured accompaniment/pink denominator. This is a new, explicit engineering
hypothesis for transferring solo contribution to ensemble overlap, not a proven
musical target. Minimize existing ensemble loss plus the mean singer solo loss.
Protect every singer's solo loss, both ensemble losses, observed lineups/direct
states and the combined ceiling. Excessive solo spread or opposing range errors
refuse gain correction. No song names, accepted auditions, input loudness equality
or operator-supplied singer offsets enter the calculation.

At most eight singer units are supported by this offline solver's work budget,
not a product input limit. Search the existing ±6 dB / 0.5 dB grid within fader
bounds using up to twelve deterministic coordinate/paired-step passes. Paired
steps permit redistribution under a ceiling without applying an intermediate
state. Prefer smaller total moves on ties; require 0.05 loss improvement. This
bounded search does not claim a global optimum. Solo and overlap objectives can
conflict; `objective_met: false` remains explicit even after a useful improvement.
Held-out data only vetoes (0.05 loss tolerance), never chooses gains or refits.

`units`, `singer_policy`, `search`, window `singer_power`/`singer_cross`/`solo` and
split `anchors` retain the identities, measurements, coverage and losses. With
`--source`, production replay checks each singer and reconstructed group against
the prediction (0.01 dB), unchanged accompaniment and eligibility, actual split
losses, composition and FX tails. Rejection preserves baseline controls. Dry
results without replay are proposals; FX still requires replay for acceptance.
Independent real-capture and musical qualification remains open.

Validation: `python3 -m unittest discover -s scripts -p 'test_vocal_balance.py'`.
Live automation, general voice/bleed identification, singer balancing without solo
anchors, shared nonlinear wet attribution, wetness decisions and musical
qualification remain separate work.

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
