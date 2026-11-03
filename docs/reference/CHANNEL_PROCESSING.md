# GP07 channel processing (legacy v2/v4 and gate v5)

Accepted four-band GP-07 contract; software implementation and acceptance tracked below. Software-only, fixed 48000 Hz, eight mono inputs;
no physical/listening acceptance. Existing C-AUDIO:1 and GP03-rendered:1 bytes
remain unchanged. Processing is an explicit `GP07-processing:2` extension on the
same private local endpoint. Older providers may disconnect on a v2 probe; clients mark processing unavailable and require explicit reconnect without replay. A disconnect alone does not identify the unsupported version.

## Gate extension (v5)

Dynamic providers accept `GP07-processing:5` in addition to legacy v4. Desk's
current dynamic session probes v5. Legacy v2/v4 writes cannot contain gate
settings; an engine with non-default gate settings refuses legacy processing
requests with `unsupported_version`, rather than dropping state. Old persisted
engine intent (v1/v2) loads default-bypassed gates. Non-default gate intent uses
v3 and restores only after current topology/resource admission.

The optional `gate` config object defaults to bypass when absent. Its strict fields:

| Field | Bounds / meaning |
|---|---|
| bypass | boolean, default true |
| threshold_mdb | −90000..0 milli-dBFS, opening magnitude threshold |
| attack_us | 100..200000 microseconds, finite opening fade |
| hold_ms | 0..2000 ms continuously below closing threshold |
| release_ms | 1..5000 ms, finite closing fade |
| range_mdb | 0..90000 milli-dB attenuation floor |
| hysteresis_mdb | 0..24000 milli-dB below opening threshold |
| detector_hz | 0 bypasses filtering; otherwise 20..16000 Hz, below 0.45 × rate |
| detector_q_milli | 100..10000; detector-only bandpass Q |

`gate::Prepared::new` accepts 8–192 kHz; the live strip remains 48 kHz. Detector
magnitude is instantaneous absolute amplitude, optionally after one RBJ bandpass.
Hold bridges waveform zero crossings. Opening and closing use finite cubic
smoothstep gain ramps; retriggers restart opening from the current gain, with no
lockout. The bandpass never filters the audio itself. No lookahead is added.
Coefficients, thresholds and time counts are prepared outside rendering; the
sample loop has fixed storage and no log/exp/pow, allocations, locks or I/O.
Default bypass returns the input directly. Nonfinite core input produces zero;
the surrounding mixer retains its existing source-fault policy.

Gate follows EQ and precedes compressor. Raw recording, raw analysis and raw
post-mute sends remain upstream. Processed sends, PFL/AFL and FX sends include
the gate according to their existing tap. Crossfades keep one gate gain/detector/hold/ramp state outside both branches.
The detector follows the blended EQ signal and applies a common gain before
both compressors. Unrelated edits do not reset it or double gate evaluation.
Compressor reduction history is also retained. New source epochs reset state.
Optional channel `gate` readback contains `state` and positive `attenuation_mdb`,
separate from `gain_reduction_mdb`. The snapshot frame supplies observation time; state/attenuation is unavailable
during configuration crossfades or source faults.

`gate::prepare_soundcheck` accepts independently labelled measured hit, bleed and
decay summaries plus a separate evaluation passage (8..4096 values per array).
Opening threshold is calibration hit p10 minus 6 dB; calibration hit p10 must
exceed bleed p95 by at least 12 dB. Every evaluation hit must exceed threshold
by 3 dB, and every evaluation bleed measurement must remain 3 dB below it. Hold is decay p90
(clamped 20..2000 ms); release is half that decay (50..2500 ms). Confidence is
`clamp((separation−6)/18, 0, 1)`. Invalid/insufficient/overlapping evidence returns
bypass and a reason enum. Settings remain fixed until explicitly replaced.
`gigpies gate-prepare <measurements.json> <new-proposal.json>` exposes this rule
without audio I/O and refuses to overwrite its output. Input JSON (maximum
512 KiB) has `version: 1`, a stable `input`, distinct `calibration_id` and
`evaluation_id`, and numeric arrays `hits_db`, `bleed_db`, `decay_ms`,
`evaluation_hits_db`, `evaluation_bleed_db`. The passage IDs identify independently
measured pre-gate evidence; distinct names alone do not prove independent data.
Output carries those identities and `proposal` containing config, confidence,
reason and measured quantiles/evaluation counts. Insufficient separation exports
a bypassed proposal, not an enabled guess.

Desk's Gate menu offers Manual, Tom and Automatic Import. Manual disables the
detector filter; Tom requires an explicitly entered measured detector band.
Automatic Import accepts the absolute proposal path, checks the selected input
and distinct passage identities, displays confidence/reason, and fills only the
local draft. Review/Apply still governs the v5 write. The resulting settings are
frozen; there is no runtime learner. Raw-cue preparation is also available with input JSON `version: 2`: the same
`input`, `calibration_id`, `evaluation_id` plus `hits`, `bleed`,
`evaluation_hits`, `evaluation_bleed` arrays of 8–32 absolute local WAV paths each.
Each file is one independently cued isolated hit/full tail or a non-target bleed
passage: mono 48 kHz PCM16/24/32, complete 10 ms windows, at most 40.96 seconds.
The CLI only reads files; `Capture::push` is the bounded software acquisition
boundary and opens no device. Uncued mixed recordings cannot establish source
identity and are not automatically classified.

Capture retains at most 4096 peak windows for each of nine detector candidates:
full-band, 60/90/135/200/300/450/675/1000 Hz, Q 0.7. Coefficients come from the
runtime gate preparation. Calibration alone selects the largest quiet-hit p10
minus bleed p95 separation; each bleed passage contributes its maximum, so a
short transient cannot hide inside a passage percentile. Peak and last window
within 20 dB of peak determine hit strength and decay. Silent or truncated tails
refuse preparation. Candidate frequency denotes detector discrimination, not a
claim to identify the drum's physical fundamental.

Decoded PCM hashes must be unique across all cues, including calibration and
held-out evidence. The selected frozen settings then run through the same causal
`Gate::tick` core on each original evaluation WAV, after a silent settling period.
Any hit with more than 1 dB total energy loss, or any bleed passage opening above
the attenuation floor, returns bypass with `causal_evaluation_failed`. The output
includes `causal_evaluation` counts and worst hit energy change; Desk checks these
on import. The second read must match the captured PCM hash. This bounds software
acceptance; it does not establish separation in arbitrary performances.

`Gate::process_block` is an offline adapter over the exact live sample core.
It does not add gating to historical automix settings or regenerate mixes.
No audition songs, post-render tuning or reference-song targets are used.

## Signal and state

Raw PCM24 REC and named analysis remain before all processing, byte-identical.
FOH: raw -> four independent parametric bells -> gate (v5) -> compressor (including explicit makeup) -> shared
mute -> FOH fader/pan -> existing sum -> existing FX -> existing PA.
Monitors: raw -> shared mute -> existing independent sends. Advertised processing
tap is `foh-post-eq-dynamics-v2`; monitor tap is `raw-post-mute-v1`.
No monitor selector, latency/lookahead, normalization or automatic makeup.

Reuse GigPies automix Biquad and compressor static law. Fixed storage; prepare
coefficients off render. Independent EQ and compressor bypass default true.
EQ bypass removes all four filters; compressor bypass removes reduction and
makeup. Neutral enabled EQ is identity; ratio1/makeup0 compressor is identity.
The compressor uses instantaneous mono magnitude, floor -240 dBFS, static soft
knee and existing dB reduction attack/release recursion. Gain reduction excludes
makeup and is positive attenuation, never an output-level meter.

Complete channel replacement commits at the strict next 48-frame boundary.
Old and freshly prepared target branches run with a linear output crossfade over
240 frames: boundary sample old, endpoint sample exactly target. Target EQ filter histories start zero; gate and compressor histories are retained.
Old histories continue during the fade. No
coefficient interpolation. The local service always renders48 frames per tick.
The offline Mixer accepts equal-length caller-owned slices, with fixed per-sample
work and checked frame-counter growth; partitioning the same samples is equivalent. A new uncached processing edit while any processing fade is active
returns backpressure without consuming an ID; retry after fresh ready status.
Identity/live lease checks and exact pending/final retry lookup precede this check.
This bounds storage and prevents discontinuous midfade retargeting. Unchanged
config is a no-op DSP transition but still a valid authority transaction. No heap
ownership/destruction, locks, I/O or unbounded work in Mixer::process. Partitioning
must not affect output. Nonfinite source/arithmetic latches the existing zero-output
fault. Engine restart resets histories/settings to defaults; no automatic replay.

## Parameters

All values integers, all fields mandatory; booleans only for bypass. Stable IDs
are input-01 through input-08. One complete channel per atomic edit.

`config` has exactly `eq_bypass`; `band1_hz`, `band1_gain_mdb`,
`band1_q_milli`, `band1_bypass`, and the corresponding `band2`, `band3`, `band4`
fields; `compressor_bypass`, `threshold_mdb`, `ratio_milli`, `knee_mdb`,
`attack_us`, `release_ms`, `makeup_mdb`. All 24 fields are mandatory.

| Fields | Range, inclusive | Default |
|---|---|---|
| eq_bypass, compressor_bypass | boolean | true |
| bandN_bypass | boolean | false |
| bandN_hz | 20..20000 Hz, integer | 120, 500, 2000, 8000 for bands 1..4 |
| bandN_gain_mdb | -12000..12000 milli-dB, step100 | 0 |
| bandN_q_milli | 100..10000, step100 | 1000 |
| threshold_mdb | -60000..0 milli-dBFS, step100 | -18000 |
| ratio_milli | 1000..20000, step100 | 1000 |
| knee_mdb | 0..18000 milli-dB, step100 | 6000 |
| attack_us | 100..200000 microseconds, step100 | 10000 |
| release_ms | 10..2000 milliseconds, step1 | 100 |
| makeup_mdb | -12000..12000 milli-dB, step100 | 0 |

Each band uses the existing GigPies RBJ bell biquad, with independent frequency,
gain, Q and bypass across the full range. Bands execute in stable 1..4 order;
crossed frequencies never reorder identities. Zero gain or per-band bypass
prepares exact identity. Global bypass skips the complete EQ. There are no shelves
or implicit conversions from the former shelf/bell/shelf settings. Makeup applies
after compression and only when the compressor is enabled.

## Strict wire and authority

Requests have exactly the C-AUDIO envelope keys, with `contract` =
`GP07-processing`, `version` = 2 and `module` = `audio`. `kind` is
`processing_snapshot` with empty `body` and null writer/lease/request/revision,
or `processing_set` with body `{ "input": "input-01", "config": CONFIG }` and
all four authority fields present. Counter encodings and identity validation
are unchanged. Duplicate/unknown/missing keys, wrong integer types, invalid ranges,
nonfinite/trailing/oversized/deep input reject atomically. 64KiB/depth12 bounds.

Snapshot proves capability; a client must receive it before exposing edits.
Mutation requires the connection's existing fresh C-AUDIO FOH grant and fresh
processing snapshot. It shares C-AUDIO/GP05 writer high-water/dedup history, global
revision and one pending renderer transaction. Scope is FOH regardless of modes;
these are explicit manual processing settings, retained across mode/hold changes.
Processing automation/proposals are not advertised. No implicit lease renewal.
Identity/live lease before cache, cache before revision; changed payload or cross-
contract reuse refuses. Backpressure is not admission and consumes no ID.
At boundary revalidate lease/revision; cancelled/refused processing leaves DSP and
revision unchanged. Successful commit increments shared revision exactly once.
New terminal `stale_snapshot`/`faulted` refusals consume their shared ID without
changing revision or extending a lease; malformed codec input is not admission.
Connection loss alone does not cancel admitted work. Reconnect drops pending and
unsent intent; fresh snapshot/new writer/grant, no replay.

Reply exact keys: `contract`, `version`, `context`, `state`, `reason`, `ticket`,
`effective_frame`, `ramp_frames`, `revision`, `snapshot`. Contract/version as request;
context has exactly existing GP03 RequestContext keys and echoes full identity.
State `pending`, `final` or `backpressure`. Reason null on success/pending/pressure;
otherwise bounded lowercase ASCII/underscore refusal string, length1..64
(same authority reasons plus `stale_snapshot` and `faulted`). Ticket/frame
canonical decimal strings or null; ramp_frames 240 or null; revision canonical
string. Pending has timing, no snapshot. Pressure has no timing/snapshot. Final
successful set preserves pending timing and includes snapshot; final refused has
reason and no timing/snapshot. Snapshot success final has null timing and snapshot.
A final means boundary applied, not crossfade finished. Old replies cannot refresh
or regress newer readback. All identity/timing/revision coherence is validated.

Snapshot exact keys: `show_id`, `epoch`, `revision`, `sequence`, `frame`,
`sample_rate`, `foh_tap`, `monitor_tap`, `faulted`, `channels`.
Sequence shares monotonic provider observation sequence; frame is next sample.
Rate48000 and tap strings above are exact. Channels contains eight ordered objects
with exact keys `input`, `current`, `target`, `transition_remaining_frames`,
`ready`, `gain_reduction_mdb`. Current is last settled CONFIG; target is committed
CONFIG. During fade current stays old (the actual output is a blend); remaining
0..240 is measured at snapshot frame; ready iff zero. Current equals target when
ready. GR is nullable integer0..240000 milli-dB: target branch detector reduction
rounded to nearest and saturated at240000 for reporting only (DSP attenuation is
not clipped); this is bounded detector feedback, not a calibrated input meter.
Valid only when ready, compressor enabled and not faulted;
otherwise null. UI must label null bypassed/transition/unavailable as applicable.
Freshness250ms local receipt, frame/sequence monotonicity required. No signal
meters are invented. Faulted processing disables editing until explicit recovery.

## Operator acceptance

Desk Channel page edits a complete local draft, explicit Apply/review/confirm or
Cancel. Display units Hz/dB/Q/ratio/ms and bypass; provider-confirmed config and
transition/GR are separate from draft/pending/failed. Selection/page/bank/revision,
role, connection and generation fence drafts and confirmations. Keyboard and
injected controller use the same semantic actions. Physical devices remain closed.
Producer fixture version `tests/fixtures/gp07/v2` is frozen from real owner code
before consumer decoding acceptance; record file SHA256 and provider source pin.
Real frontend/provider/sample tests must prove changed FOH, second-channel
isolation, monitor independence, exact raw REC/analysis, FX/PA order and reconnect.

## Legacy policy

The provider supports processing version 2 only. For a bounded, valid outer GP07
envelope with an unsupported integer version, it validates the outer identity,
command kind and authority-field shape before returning the existing reply keys:
requested contract/version and echoed context, `state: "final"`,
`reason: "unsupported_version"`, the actual shared revision, and null ticket,
effective frame, ramp and snapshot. The dedicated refusal path does not deserialize
Config, consult retry caches, admit authority, consume an ID, renew a lease or
change processing/revision. It consumes the whole frame and keeps GP03 usable on
the connection. Malformed outer envelopes retain the existing disconnect policy.

The old Desk can decode this v1 refusal and disable processing while preserving
GP03. New Desk refuses unsolicited v1 replies. An unchanged old provider closes
the connection on a v2 probe; the new client shows processing unavailable and uses
explicit reconnect without replay. No v1 readback or shelf approximation is invented.
The original `tests/fixtures/gp07/v1` corpus remains unchanged as compatibility
evidence. The [archived v1 contract](../archive/channel-processing-v1-2026-10-05.md)
retains its dated acceptance, not a claim of continuing v1 processing support.

The internal processing history marker includes version 2. GP03, GP05 and v2
processing still share a writer's ID history, revision and lease. Cross-contract
ID reuse refuses; an unsupported-version request neither retrieves a cached v2
success nor reserves its ID. Explicit regressions cover both directions.

## Software signal acceptance — 2026-10-05

The actual Desk frontend drove 17 atomic changes through keyboard and injected
semantic field actions: two frequency/gain/Q settings for each of four individual
bands, a crossed-frequency cascade, each band's bypass, global bypass, enabled
neutral EQ, compression with positive GR, and a second channel. Complete reviews
preceded confirmation. Exact final-reply frames matched independently observed
provider boundaries. Context cancellation and reconnect did not replay a change;
revision 17/settings remained stable through the subsequent lease-expiry interval.

The joint run compared 8,172 full blocks. Eight isolated input-mapping references
matched the complete FOH timeline exactly. A separate direct-form-I bell reference
checked 304,848 EQ-only samples and transitions against the production direct-form-II
engine; maximum absolute error was 3.06e-16. Every active single-band/cascade setting
produced nontrivial changed samples; settled neutral and global bypass matched the
reference bit-for-bit. These are synthetic numerical checks, not listening evidence.

All eight raw PCM24 stems matched 391,968 source frames each and covered the
complete edit interval, with complete finalization and no gaps/drops/faults.
Monitors stayed bit-exact and raw named analysis matched before/during/after.
Independent actual FX wet-plus-dry -> PA output matched each processed block;
those algorithms and their order are unchanged, while their FOH input follows EQ.
Timing-dependent counts describe this run, not fixed thresholds.

Complete normal GigPies suites pass: 303 default and 326 hardware-host tests.
Formatting, both warnings-denied Clippy configurations, default/hardware-host release and
39 Python publication checks pass; four actual owner-library GP05 regressions pass.
Historical media, auditions, exhaustive research, long benchmarks, physical
endpoints/windows and shared-load experiments remain intentionally excluded.
The v2 fixture generator was explicitly run once; unrelated historical generators
remain opt-in. Actual CPU native rendering passed exact pixel comparisons for
both Channel and complete protected Review at 1920×1080, 960×540, 540×960 and
3840×2160, plus zero-size suspension, without opening an operator window.

The new Desk also passed against the old published provider: a failed v2 probe
shows processing unavailable, and explicit GP03-only reconnect permits normal
protected controls without replay. Fresh paired observations and original revision
pinning remain required. A debug driver rendering delay initially prevented a
review from being marked presented; refreshing the same review after rendering
fixed the test while preserving production freshness and review guards. Native
validation also exposed texture-coordinate rounding during resize and unnecessary
release traffic from local injected field edits. Desk now maps pixels from fragment
position and viewport dimensions, and local edits match keyboard queue behavior;
Apply still requires fresh provider state and the complete protected review.

No physical audio, MIDI, display, acoustic or combined-load acceptance is implied.
Source synchronization is not deployment. Scenes, PFL, routing/channel expansion,
writable PA/FX and production remote authentication remain separate GP-07 work.

## Configurable engine extension — GP07-processing:4

The production `Mixer` now provisions a strip for every admitted logical input.
Each retains exactly the four parametric bands, individual/global bypass and
compressor described above. Config and smoothing are unchanged.
[GP18 sends](MONITOR_SENDS.md) adds per-send mono tap selection; processing4
advertises `per-send-gp18-v1`. Version4 uses the same request/reply fields and a capability-derived,
ordered channel vector. IDs remain contiguous `input-01` through the configured
count; they never expose USB gaps. Versions1 (historical three-band) and2 are not
silently reinterpreted. The explicit legacy8/2 engine retains processing2; a
configured provider refuses processing2 and processing3 with `unsupported_version`.
The historical version3 corpus remains unchanged; consumers must explicitly upgrade.

Software regressions exercise each strip in 16, 17, 32 and48 input profiles,
including high-index fourth-band processing and independent monitor sends.
Allocation/deallocation guards cover prepared application and retained retirement
in the same configurable Mixer. This is software correctness, not a measured
48-channel device deadline or listening acceptance. Producer fixture generation
remains opt-in and must be repeated after the final provider source is frozen.
