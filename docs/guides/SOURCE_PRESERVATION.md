# Source preservation and evidence for intervention

The unchanged source is an eligible finished choice. Offline processing requires explicit intent or supported evidence, and channel/master high-pass filters may be bypassed. Requested [artistic effects](ARTISTIC_FX.md) remain compatible with source preservation.

## What we are deciding

A successful recording can already contain the musician's intended sound. A good
source and a good ensemble are valid finished choices. A role identifies a source's
job; it does not establish a defect, processing requirement or preferred balance.
The selected baseline must compete with every proposed intervention.

Keep five questions separate:

| Question | Useful evidence | Limit |
|---|---|---|
| Where did this sound come from? | Original files and notices, acquisition hashes, session notes, musician/producer accounts, verified input assignments | File integrity and a `raw` label do not certify untreated capture. A waveform cannot reconstruct its prior chain. |
| Is there a technical problem? | Invalid/nonfinite data, repeated PCM contact, known routing error, contextual noise/overload observations, actual processor envelopes | Clipping below full scale, intentional distortion, ringing versus a played note, and recoverability often remain unknown. Report contact without claiming repair. |
| Is a musical change wanted? | Musician intent, listener comparison, attack/body/sustain and articulation in the song, arrangement and section transitions | Crest, spectra, occupancy and whole-song medians cannot establish preferred tone or balance. |
| What protection is required? | File/schema/routing integrity and export limits; separately, a live system's verified operating limits | Offline HPFs do not establish feedback stability, loudspeaker protection or safe acoustic level. |
| Could a processor help? | A specific problem, a plausible mechanism, a bounded proposal, actual DSP comparison and contextual listening | Technical success is eligibility for listening, not musical improvement. Effects are optional artistic decisions. |

The performer, listener and operator supply intention and preference. The software
owns reproducible measurements, bounded changes, provenance and failure reporting.
Unknown printed EQ/compression is recorded as unknown. Low crest or a shaped spectrum
can be deliberate sound design; neither enables an “already processed” detector.
Already designed sounds may still need a supported correction in a new ensemble.
Earlier user-prescribed knob moves and numerical mixing preferences remain withdrawn.
Their historical provenance is retained without treating them as current objectives.

## Required reasoning for a proposed correction

Record this before selecting parameters:

1. **Problem and confidence:** what was observed, in which source/section and by whom;
   distinguish supplied listening evidence, reported provenance, measured behavior
   and inference. A threshold exceedance is relative to its declared policy.
2. **Mechanism and expected benefit:** why this processor or fader could address that
   observation. Do not prescribe compression from an instrument label or reverb from
   an assumed dry capture.
3. **Musical cost:** attack, sustain, body, articulation, bass support, bleed, stereo
   interaction, section development and export-headroom consequences as applicable.
4. **Baseline victory:** insufficient evidence, no requested change, no material
   supported benefit, a protection failure or adverse listening retains the baseline.
   Equal scores also retain it. Preserve the previous selection for recovery.
5. **Bounded test:** freeze a small candidate budget and sections, compare processing
   at fixed faders, assess artistic balance separately, then check the combined
   selection. Holdout can veto; it cannot choose a retry. A failed correction does
   not justify widening a guard or silently changing the baseline.

Measure raw events/sections once and reuse those anchors. Include quiet/strong and
sparse/dense passages where relevant. Follow HPF → EQ → compression → makeup →
fader/pan → coherent ensemble → master → constant export gain. Independent powers
omit phase interaction; evaluate actual sums. For sampled drums, energy rises remain
proxies for musical events. For ambiguous microphone mixtures, abstain from claiming
source separation. Do not optimize a track while ignoring its place in the song.

## Default behavior

- Explicit `preserve-source` clears static and dynamic EQ, bypasses compression and filters, and removes effects while retaining the supplied faders and pan.
- Baselines require prepared unmatched settings, unity trims, verified routing where asserted and the −0.01 dBFS export ceiling. High-pass filters may be zero or any schema-valid explicit setting; validation does not insert or remove them.
- Bass and drum measurements use the configured master filter. A zero-frequency high-pass is an exact identity filter.
- Missing balance intent supplies no numerical role relationships and performs no search. Missing tone intent is `unknown`, with a null target and no proposed EQ.
- Default `finish` observes with zero EQ/return-movement budgets and no master-reduction budget. Explicit policies can enable bounded review; export protection remains active.
- Preserve the previous selected settings and source identities for recovery. Historical trial policies remain opt-in and do not establish preferred musical balance.

## Foundational filter decision

The old filters aimed to restrict low-frequency content, leave kick/DI space and
provide a consistent processed baseline. They originated in a unity-source experiment,
not a demonstrated requirement for every offline file. The schema and renderer already
supported bypass. A 90 Hz filter can remove useful tom, guitar, keyboard or vocal body;
a 40 Hz filter can attenuate fundamentals and change coherent low-frequency phase.
Those costs matter even when whole-band RMS changes little.

Replacement: select a filter for documented unwanted content or an explicit artistic
choice, test it in context and keep bypass eligible. Frequency content below a corner
is not itself unwanted. The Dark Ride audit attributes only about 0.32 dB of excerpt
bass RMS loss to the master HPF; it does not blame filters for all lost power. The
larger inherited EQ/makeup/export interaction remains the established finding.

This revision is an offline admissibility change, made before new candidate evaluation.
It does not increase correction budgets, remove PCM-contact reporting, relax existing
crest guards or silently waive a failed candidate. SOURCE preservation does not need
to “improve” on the rejected FINAL's crest: the listener-preferred SOURCE is the
unchanged musical reference. Old crest failures against other baselines remain failures.

On stage, if the instrument and captured signal already sound good, keep that tone
and adjust balance only for a stated ensemble need. A real input overload calls for
checking the stage that overloads; EQ cannot reconstruct missing detail. Monitor/PA
feedback control and system-specific protection remain independent, continuously
required functions in the intended live architecture. This offline work neither
implements nor validates them. No hardware claim follows from filter bypass offline.

## Use and recovery

```sh
CARGO_INCREMENTAL=0 cargo +1.97.1 build --locked --release
# Supply the pinned, initial SOURCE routing with prepared unity input trims,
# unity master, verified selected paths and unmatched −0.01 dBFS export settings.
target/release/gigpies preserve-source INITIAL_SOURCE.json NEW_SELECTION_DIRECTORY
target/release/gigpies render NEW_SELECTION_DIRECTORY/settings.json SOURCE_DIRECTORY NEW_RENDER_DIRECTORY
# No supplied balance policy: measure and preserve artistic faders.
target/release/gigpies balance-analyze NEW_SELECTION_DIRECTORY/settings.json SOURCE_DIRECTORY NEW_ANALYSIS_DIRECTORY
```

This command does not know which later faders were the original SOURCE faders.
The listening checkpoint's existing pinned SOURCE settings/WAV hashes remain
mandatory. Select original routing deliberately; never reset a liked mix merely
because preservation is now available. There is no implicit reset inside `render`.

All outputs require fresh destinations. Failed preparation has no ready checkpoint;
recover with the frozen inputs and a new directory. Playback remains a separate
explicit request. Original sources and previous settings/exports remain available.
An unchanged source export may be byte-identical to FINAL; label that as the outcome
instead of inventing a second mix to make the comparison look productive.

## Validation scope

Normal synthetic tests cover exact SOURCE/FINAL PCM and float equality with mixed
mono/stereo routing, bass/drum measurement agreement with production renders at
0/40/65 Hz, preserved unknown source identity, nonunity/routing/nonfinite/export
refusal, missing tone/balance intent, no default master/FX shaping, and a known
injected compressor fault that still receives bounded actual-DSP relief. Existing
BWF, clipping/contact, transient, linked compression, fader/makeup, recovery and
held-out tests remain in the complete normal suite.

Private Dark Ride probes and selected complete export are explicit opt-in evidence.
Old auditions, exhaustive searches and unrelated snare-identifiability experiments
are not rerun. Verified results of the bounded pass are recorded below.
No listening or hardware acceptance is implied.

## Historical evidence

The [dated study and original results](../archive/studies/SOURCE_PRESERVATION.md) retain experiment-specific settings and validation history.
