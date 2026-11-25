# GigPies integration plan

Updated 2026-11-25. This page owns every cross-module GigPies task: one card per
task, holding state, what is done, what is open and the next action. Module-only
work lives in the [owning module plans](MODULE_IMPLEMENTATION_MAP.md).
Dated execution logs, validation counts, hashes and failures up to 2026-11-24
are preserved verbatim in the [full record](../archive/plans/MODULE_IMPLEMENTATION_PLAN-2026-11-24.md).

**Runtime owners:** GigPies, SHR Desk, SHR Lightdesk, SHR PA, SHR FX, SHR REC, SHR Lux.
SHR DAW, Drums, Synth, Sampler, Tone Over 9000 and Skills are reference sources,
not runtime modules ([components](../architecture/COMPONENTS.md)).

## How to read and update this plan

- **States:** PLANNED · IN PROGRESS · BLOCKED (name the dependency) ·
  SOFTWARE ACCEPTED · ACCEPTED (name software, physical or musical scope).
  Passing software tests never closes a physical or listening gate.
- **Two queues.** *Software work* can be done at a desk with synthetic signals
  and owner libraries. *Qualification* needs a separately authorized session with
  real hardware, a room, musicians or a listener. Each gate appears once.
- Update the card with the change, not afterwards. Keep it short: outcome, done,
  open, next action, links. Put long evidence in an acceptance record or the archive.
- A card whose software work is finished moves its physical gates into the
  matching qualification card (GP-H1/H2/H3, GP-PA-MEASURE, listening) and is
  then summarised under [Software-accepted cards](#software-accepted-cards).

## Board

### Software work

| Task | State | Next action |
|---|---|---|
| [GP-PA-MEASURE](#gp-pa-measure--physical-measurement-workflow) | IN PROGRESS | Host acquisition driving the frozen capture session (step 2) |
| [GP-AUTO](#gp-auto--live-soundcheck-and-bounded-assistance) | SOFTWARE ACCEPTED | Listening with a real singer (qualification) |
| [GP-AUTO-ENSEMBLE](#gp-auto-ensemble--observation-led-whole-band-automixing) | IN PROGRESS | Backing vocals and the rest of the band; wetness objective |
| [GP-PANEL](#gp-panel--shared-physical-instrument-panel) | IN PROGRESS | Page-by-page physical-control redesign |
| [GP-H2](#gp-h2--physical-dual-console-operation) | IN PROGRESS | Software done; learned SHR-DAW mapping imported; hardware checks remain |

### Qualification sessions (need explicit authorization)

| Session | Card | What it proves |
|---|---|---|
| Two-node transport timing | [GP-H1 T3](#gp-h1--physical-audio-qualification) | **Accepted** 2026-11-24 (5/30/56 s); persist host settings on both Pis |
| Stagebox audio rig | [GP-H1](#gp-h1--physical-audio-qualification) | Socket maps, UMC/ADAT clock, latency, sustained deadlines, REC/FX/PA on hardware |
| Controllers and displays | [GP-H2](#gp-h2--physical-dual-console-operation) | Learned MiniLab profiles, LEDs, disconnect recovery, two desks at once |
| Rehearsal with PA | [GP-PA-FEEDBACK](#gp-pa-feedback--one-notch-on-one-output) | Cut frequency, response time and false triggers with musicians |
| Acoustic measurement | [GP-PA-MEASURE](#gp-pa-measure--physical-measurement-workflow) | Synchronized reference/mic at representative positions |
| Listening | [GP-AUTO](#gp-auto--live-soundcheck-and-bounded-assistance) vocal assist, [GP-AUTO-ENSEMBLE](#gp-auto-ensemble--observation-led-whole-band-automixing), [GP-GATE](#gp-gate--lightweight-channel-gates-and-tom-decay-control), [bass](#gp-auto-bass-low--observed-low-energy-and-definition) | Musical judgement on unseen material; gate cues on real drums |
| Whole show | [GP-H3](#gp-h3--whole-show-qualification) | All owners together: load, thermals, failures, lighting output |

---

## GP-PA-MEASURE — physical measurement workflow

**State:** IN PROGRESS. Owner GigPies (capture/workflow); SHR PA (analysis,
[PA-ALIGN](https://github.com/gigpies/shr-pa/blob/main/docs/PHASE_ALIGNMENT.md#pa-align--measured-crossover-alignment));
SHR Desk (operator review). Interfaces: [measurement producer](../reference/MEASUREMENT_PRODUCER.md).

**Outcome:** measure each speaker path with an isolated mic against a synchronized
reference, review bounded corrections, apply them muted, then measure the real
combined result.

**Done**
- Software GP20 capture, overlap/EQ review, imported calibrated sweeps (archived
  [GP-PA-SETUP](../archive/pa-setup-2026-10-29.md)).
- Bounded independent-clock USB bench (E205U on Pi5, Pi4 amp): auto-EQ proposals,
  production PA apply and live remeasurement; Brain FX wet/dry relative timing;
  offline comparison of runs by microphone position ([acoustic bench](ACOUSTIC_BENCH.md)).
  Uncalibrated, single position — not absolute latency or GP20 timing.

- Step 1 capture/excitation contract `GP-PA-MEASURE-CAPTURE:1` (2026-11-25,
  `src/measurement_session.rs`, [reference](../reference/MEASUREMENT_PRODUCER.md#physical-capture-session-contract-gp-pa-measure-capture1)):
  source-frame excitation reference, one admitted speaker path, reserved mic slot
  never routed, calibration provenance, owner-bounded sweep level/length/tail,
  cancel and fault stops, mandatory restore of the prior route, Stagebox clock only.
  Producer corpus `tests/fixtures/gp-pa-measure/v1/`; SHR PA `sweep` and
  `sweep_repeated` accept its records and reproduce its generator samples.

**Open — software**
2. Host acquisition off the callback, with deterministic loopback tests for
   isolation, timing, cancellation, lease loss and stale results.
3. Desk measure → review → apply muted → remeasure sequence, including reconnect.

**Open — qualification:** electrical timing/socket mapping (with GP-H1), then
controlled speaker/mic trials at several positions with before/after sums.

**Next:** step 2 host acquisition driving the session off the callback. FX-path
probes are not in v1; they keep using pre-fader FX send B with its fader at −60 dB
(not channel mute).

## GP-PA-FEEDBACK — one notch on one output

**State:** SOFTWARE ACCEPTED (2026-11-21). Owner GigPies; SHR PA guard; Desk F9.
One configurable SHR PA v2 feedback guard on one output, with bypass, GigPies
structural readback and Desk output selector/applied-frequency display
([operator controls](../reference/MODULAR_PROCESSING.md#automatic-feedback-notch)).
Tested on a simulated 1 kHz acoustic loop, held guitar/vowel/flute-like material
and the real owner library.

**Open — qualification:** cut frequency, response time and false triggers on a
representative PA with musicians.

## GP-METER — measured audio on the real Desk

**State:** SOFTWARE ACCEPTED (2026-11-24, owner-backed). Owner GigPies; consumer SHR Desk.
Contract: [GP-METER:1](../reference/MODULE_CONTRACTS.md).

**Outcome:** real channel (raw and pre-fader), Main pre-PA and every configured
monitor level/clip reaches Desk. Silence, unavailable, stale, invalid and clipped
are distinct states. Main is pre-PA: not a physical output, true-peak or SPL meter.

**Done**
- P1 provider: 20 ms windows, allocation-free hooks, independent admission,
  frozen corpus `tests/fixtures/gp-meter/v1/`.
- P2 delivery: private Unix and mutual-TLS QUIC reads that never touch leases,
  revisions or held proofs; revocation fails closed; epoch recovery, slow readers
  and repeated reads cannot refresh, splice or block (`tests/metering.rs`).
- P3 Desk: independent codec/cache/observer, F11 meters, CPU-native scene checks;
  frontend coverage (2026-11-25, Desk `src/frontend/meter_tests.rs`) of clip (red,
  raw and processed), silence (no bar), stale meters and stale control (dimmed),
  invalid (amber), every unavailable reason, GR aged independently of meters,
  provider-generation/topology loss clearing meters and clip hold, and meter
  browsing that never sends control, holds or disturbs held state and drafts.
- P4: actual LocalAudio at 16/17/32/48 inputs with independent Desk/native driver;
  **owner-backed** check with hash-verified PA/FX/REC libraries — Main pre-PA equals
  an independent mixer + independent FX instance, PA output equals an independent
  PA instance, every monitor matches. Owner revisions: shr-pa `7aafa69`,
  shr-fx `555b8f9`, shr-rec `3a1ca10`; the existing `modular_owners` and
  `brain_owners` drivers also pass with that manifest.

**Open:** none in software. Physical meters on the rig are part of GP-H1.

**Run owner-backed checks:** see [owner-artifact tests](DEVELOPMENT.md#owner-artifact-tests).

## GP-FX — writable owner effects in the console

**State:** SOFTWARE ACCEPTED (2026-11-13). Owner GigPies; SHR FX provider; Desk consumer.
Contract: [remote FX control](../reference/REMOTE_FX_CONTROL.md).

Four stereo FX auxes through Brain, each with per-input pre/post-fader post-mute
sends, one selected wet effect, a Main return and independently ramped sends to
every monitor. Tail-preserving bypass, panic, saved intent (v1/v2) and Desk review.

**Open — qualification:** listening and sustained target-hardware load (GP-H1/H3).

## GP-SHOW — audio scenes and operator recovery

**State:** SOFTWARE ACCEPTED (2026-11-12). Owner GigPies; Desk consumer.

Sixteen private setup slots. Desk reviews current versus saved values for the full
field inventory (faders, pans, mutes, sends and taps, channel processing, routing,
complete PA graph, FX auxes, Brain listening), restores each granted scope under a
provider-owned output hold and verifies settled readback. Cross-scope recall is
deliberately non-atomic with a durable before-send receipt; outputs stay disarmed.

**Open — qualification:** physical recall on the rig (GP-H1).

## GP-AUTO — live soundcheck and bounded assistance

**State:** SOFTWARE ACCEPTED (2026-11-25) for the main-vocal rule. Owner GigPies;
consumer SHR Desk. Contract: [vocal assist v6](../reference/CHANNEL_PROCESSING.md#vocal-assist-extension-v6).

**Main-vocal rule (user, 2026-11-21):** measure the main vocal while the singer
soundchecks and keep that level. The only automatic change on this mic is a
level reduction when it peaks. The soundcheck is a whole loud song; the whole
take is measured and its maximum is the kept level.

**Done**
- Engine: `GP07-processing:6` `vocal_assist` stage after the compressor, with a
  10 ms detector, threshold = measured reference + headroom, ∞:1 hold at the
  threshold floored at a 12 dB-bounded max cut, attack/release and automatic
  recovery; bit-exact below threshold; allocation-free (`gp07_alloc`). Readback
  state, current/deepest cut and reduction events. Intent v3 persistence.
- Soundcheck: `prepare_soundcheck` over GP-METER processed-tap windows with
  stale/unavailable/clip/silence/duration/quiet refusals and provenance
  (origin, evidence windows, rule ID, measurements); `vocal-prepare[-stream]`.
- Recovery (`tests/vocal_assist.rs`): lease loss refuses edits while the engine
  keeps the applied assist; retried IDs never reapply; operator EQ edit and
  disable mid-reduction neither step nor reset; intent restore keeps settings
  but starts at unity; older versions refused once configured.
- Desk: Channel page Vocal assist mode on the shared instrument panel (keep,
  headroom, max cut, attack, recovery, on/off; live CUT meter and state line);
  V arms/stops a whole-song soundcheck from GP-METER, GigPies measures, C opens the
  protected review of the measured settings, Z drafts the disabled assist into
  review (Enter confirms). Context, epoch or meter identity loss discards a
  take; nothing applies without review.
- Corpus `tests/fixtures/vocal-assist/v1` (Desk copy byte-identical); real
  GigPies process reproduces the producer document from Desk's job runner.

**Decisions:** kept level = maximum 20 ms RMS of the whole loud-song take
(user); headroom 0 dB and max cut 6 dB defaults, 12 dB hard bound (agent);
cut lives in the strip, never on the operator's fader; soundcheck requires the
assist bypassed so the take is unreduced; revert keeps the settings and disables
them, and still goes through review; windows at the Desk observer rate (≤25 Hz,
up to five minutes).

**Evidence:** recorded stems (opt-in `recorded_lead_vocal_soundcheck_and_assist`,
statistics only), each whole song as the soundcheck and then played through the
assist: Complainiacs lead → keep −13.0 dBFS, reducing <0.1 % of samples, max cut
0.5 dB; Catbite lead → keep −12.0 dBFS, <0.1 %, max 1.0 dB.

**Open — qualification:** listening with a real singer on the rig (Listening
row): threshold/headroom feel, attack/recovery audibility, soundcheck procedure.

**Next:** other sources and ensemble assistance stay in GP-AUTO-ENSEMBLE until a
new live rule is decided.

## GP-REC — complete integrated recording workflow

**State:** SOFTWARE ACCEPTED (2026-10-29). Owner GigPies; SHR REC; Desk consumer.

Reviewed Desk Start/Stop with the FOH lease, readiness/progress/fault/final
outcome, exact 8/16-track raw capture, no reconnect replay, capture survives Desk
and lease loss. Storage time and durability are reported unknown, not guessed.

**Open — qualification:** physical capture, storage load and power-loss behavior (GP-H1).

## GP-GATE — lightweight channel gates and tom decay control

**State:** SOFTWARE ACCEPTED (2026-11-12). Owner GigPies channel DSP; Desk presentation.
Contract: [gate extension v5](../reference/CHANNEL_PROCESSING.md#gate-extension-v5).

Default-bypassed, zero-latency gate after EQ with hold, release, range, hysteresis
and detector filter; processing v5 and intent v3. `gate-prepare` and Desk's
Manual/Tom/Automatic import build proposals from recorded, identified passages.

**Open — qualification:** physical cue acquisition on representative drums and listening.

## GP-PANEL — shared physical instrument panel

**State:** IN PROGRESS. Owner GigPies (`src/instrument-panel`, vendored as
`shr-instrument-panel`); consumers Desk and Lightdesk.
Contract and previews: [instrument panel](../reference/INSTRUMENT_PANEL.md).

**Done:** shared Ratatui package with 77 authored tiles and font, three-row keybed,
selected-Channel material panel, fullscreen EQ/RTA, Master gain strip, 10 Hz
presentation; real-terminal captures ([evidence](../archive/records/2026-10-11-gp-panel-proof.md)).
Package 0.3.3 adds the shared read-only MIDI reader used by GP-H2.
Desk Mix page on the shared plates (approved 2026-11-25): topology-filled
12-strip banks, measured meter states, FOH/monitor scopes, column-8 selected target,
MIDI-first pads; frontend tests and `--mix` preview ([contract](../reference/INSTRUMENT_PANEL.md#desk-mix-page)).

**Open — software:** page-by-page physical-control redesign of the remaining pages;
SSH-path and reconnect cleanup checks; workload comparison with the old native renderer.

**Next:** redesign the next production page on the shared plates.

## GP-H1 — physical audio qualification

**State:** PLANNED (needs an authorized physical session). Owner GigPies; PA/REC/FX as needed.
Procedures: [modular physical](../reference/MODULAR_PROCESSING.md#mapping-and-physical-acceptance),
[Brain](../reference/BRAIN_AUDIO.md#bounded-physical-followup), [hardware evidence](../acceptance/AUDIO_HARDWARE.md).

**Transport (Ethernet).** Cable baseline passed. Local T0–T2 accepted 2026-11-05:
contract vectors, dedicated source/FX execution with prepared commits and atomic
closure, independent duplex service with both ASRC crossings
([transport blueprint](../architecture/AUDIO_TRANSPORT_V1.md)).

| Step | State |
|---|---|
| T3 combined two-node timing 5 s → 30 s → 56 s | **ACCEPTED 2026-11-24** (software timing, synthetic audio) |
| T4 alternative comparison | **Declined**: QUIC keeps ≥384 frames margin, twice the 192-frame GO; no replacement can show the required benefit |
| T5 replacement or retain QUIC | **QUIC retained**; no candidate transport exists to remove |
| T6 one production path | **Done**: authenticated QUIC datagrams with realtime workers (handoff below) |

**T3 result** (ledger tasks 0030–0031; both nodes at `fbdcc1e`, SHR REC
`f927505`). Runs 5/30/56/56 s: 0 late or rejected wet packets, minimum wet margin
432/384/384/432 frames against the 768-frame deadline, no source-egress refusals,
all 16 raw REC channels bit-exact, FX without incomplete or stale blocks, both
nodes unthrottled. The first attempts failed at 30 s (margin down to 48–144
frames, egress refusals, FX resets). Three causes were found and fixed:

1. Media workers ran at normal priority → opt-in `media_realtime_priority`
   (source = N, network = N−2, workers = N−5; network above compute).
2. SHR REC issued ~17,000 tiny writes a second → buffered take files.
3. Realtime threads waited on page faults and page migration behind ext4 journal
   commits → `mlockall(MCL_CURRENT|FUTURE|ONFAULT)` and
   `vm.compact_unevictable_allowed=0`.

**Production path handoff (T6).** Run Stagebox and Brain `gigpies-remote` with
`media_realtime_priority` (70 used), an RLIMIT_RTPRIO ≥ that value and unlimited
RLIMIT_MEMLOCK (`@audio` limits; for a service use `LimitRTPRIO`/`LimitMEMLOCK`),
and `vm.compact_unevictable_allowed=0` set persistently. Done 2026-11-25 on both
Pis: `@audio - rtprio 95` / `memlock unlimited` in
`/etc/security/limits.d/95-shr-audio.conf` (Pi 4 added; Pi 5 already had it) and
`/etc/sysctl.d/95-gigpies-media.conf`. The limits apply to PAM sessions (console,
`su -`, OpenSSH, services with `LimitRTPRIO`/`LimitMEMLOCK`); **Tailscale SSH
sessions bypass `pam_limits`** and get rtprio 0, so do not launch realtime runs
directly from them. Every run report shows these in `media_realtime`. This is software timing evidence, not physical audio acceptance.

Keep 768-frame wet deadline and the 60 s synthetic cap; never subtract
unsynchronized clocks. Proposed thresholds: ≥4 ms wet margin, ≥2 ms benefit for a
replacement.

**Rig checks:** Stagebox/Brain inventory and socket maps, UMC1820/ADAT clock
relationship, duplex drift, electrical latency, safe fault/rearm, sustained
deadlines, and the open AudioBox right-return/timing discontinuities (H8).
Also carries the physical gates of GP-FX, GP-SHOW, GP-REC and GP-METER (measured
levels against known electrical signals on the rig).

## GP-H2 — physical dual-console operation

**State:** IN PROGRESS. Owner GigPies; Desk and Lightdesk.
Contracts: [roles](../reference/ROLE_BINDING.md), Desk/Lightdesk `docs/CONTROLLER.md`.

**Outcome:** run the show from the two assigned MIDI controllers without a
computer keyboard or mouse; shortcuts are optional adapters to the same actions.

**Done**
- Labelled eight-pad semantic menus and literal value entry in both consoles;
  typed actions instead of synthesized keys; Lightdesk menus through the Lux worker.
- Stale generation/target, held/repeated pads, wrong endpoint and hidden-review refusals.
- Shared read-only MIDI reader (panel 0.3.3): running status, realtime/SysEx
  filtering, generation per connection, loss events, by-id device resolution.
- Desk production path: strict learned profile file (`SHR-DESK-CONTROLLER` v1),
  `--terminal … --controller-profile FILE` feeds the existing role-fenced adapter;
  loss fences the controller; `--controller-learn BY_ID [SECONDS]` prints messages
  read-only so the operator can capture the real device.

- Lightdesk: strict pad profile (`SHR-LIGHTDESK-CONTROLLER` v1) and
  `terminal … --controller-profile FILE` through `Surface::midi` and its role fences.

**Open — software:** none beyond a Lightdesk FIFO-level acquisition test.

- Desk `--controller-import-daw` converts the learned SHR-DAW MiniLab mkII mapping
  (rotaries 1–16 on channel 10, Shift alternate CC 7, pads 36–43). Private profiles
  for both desks are in `user/controllers/`.

**Open — qualification:** on the connected MiniLab, confirm the two values SHR-DAW
never learned (pads bank 2 = 44–51, key channel/octave), then the sixteen-encoder
Channel mapping, LEDs, disconnect/release recovery and two desks at once.

## GP-H3 — whole-show qualification

**State:** PLANNED; follows GP-H1/H2 and Lux LX-06 lighting output.

Measure the combined Stagebox recording/protected audio and Brain FX/Lux/analysis/
two-console workload: deadlines, control responsiveness, memory, thermals and
failures, using the [persistent journal](NODE_LAB.md#pi-5-reboot-diagnostics).
Also carries GP-MUSIC-2 rehearsal accuracy and combined-host headroom.

## GP-AUTO-ENSEMBLE — observation-led whole-band automixing

**State:** IN PROGRESS. Owner GigPies. Offline only; live use belongs to GP-AUTO.
Guides: [balance pass](../guides/BALANCE_PASS.md), [FX pass](../guides/FX_PASS.md).

**Main vocal:** governed by the 2026-11-21 rule in [GP-AUTO](#gp-auto--live-soundcheck-and-bounded-assistance).
It supersedes all lead-vocal proposals and their classifier, capture-review and
listening gates; those offline experiments are historical evidence. Backing vocals
keep their own scope.

**Intent (user, reconfirmed 2026-10-30):** levels are *calculated* from soundcheck
evidence and a reusable objective — a formula-driven spectral target, not the
audition songs. No fixed kick/bass/vocal hierarchy. V6/V8 are listening
checkpoints, never calibration targets. Lead and backing vocals are reviewed
separately in context.

**Done (software):** E0–E6 role pipeline; E7 real-recording checks on Pi; reference-free
spectral calculation; capture-specific contrast and reviewed spectral states;
bleed power-mixture exclusion; bounded measured vocal decisions with lineup
protection; solo-anchored and contextual singer balancing; coherent vocal FX
returns and protected send selection through replay.

**Open — software**
- Backing-vocal and double/room decisions with a justified performance objective.
- Wetness, tone and dynamics decisions; broader-band measured decisions.

**Open — listening:** unseen-material qualification of the spectral objective,
backing-vocal decisions and the wetness hypothesis.

**Next:** backing-vocal objective, then the wetness objective.
Decision audit: [2026-10-30](../archive/studies/AUTOMIX_DECISION_AUDIT_2026-10-30.md).

## GP-AUTO-BASS-LOW — observed low energy and definition

**State:** SOFTWARE ACCEPTED; listening pending. `src/automix/bass.rs`,
[bass guide](../guides/BASS_KICK.md#explicit-low-frequency-intent).
Measures below-25 Hz and 25–100 Hz energy at every stage with conditional bounded
low shelves. **Open:** listening on selected real material.

## GP-AUTO-LUFS — offline kick-led source hierarchy

**State:** SOFTWARE ACCEPTED; superseded. Optional `src/automix/hierarchy.rs`
([guide](../guides/BALANCE_PASS.md#kick-led-lufs-hierarchy)). Not used for current
auditions unless explicitly selected.

## GP-MUSIC-2 — seven-feed musical context for Lightdesk

**State:** SOFTWARE ACCEPTED. Owner GigPies source contract; Lux analysis; Lightdesk.
Seven configured feeds (kick, bass, guitar 1/2, lead vocal, synth, backing vocals)
over C-ANALYSIS:3 with key/chord ambiguity gates, register/energy direction,
pause/resume cues and section candidates. Rehearsal accuracy moves to GP-H3.

## Software-accepted cards

GP-METER, GP-FX, GP-SHOW, GP-REC, GP-GATE, GP-AUTO (main vocal), GP-AUTO-BASS-LOW,
GP-AUTO-LUFS and GP-MUSIC-2 have no remaining software gate except where listed in their card.
Earlier closed work: [GP-PA-SETUP](../archive/pa-setup-2026-10-29.md),
[legacy GP-01..09](../archive/tracking-before-2026-10-09/MODULE_IMPLEMENTATION_PLAN.md).

## Working rules

- Follow each owner's AGENTS.md. Serialize Cargo under
  `flock -xn /home/shome/p/.gigpies-build.lock`, Rust 1.97.1, `--locked -j1`,
  `CARGO_INCREMENTAL=0` (see [development](DEVELOPMENT.md)).
- Run focused tests while working; complete normal suites for shared engine,
  schema, transport, routing or safety changes.
- Software acceptance never deploys services or activates hardware.
