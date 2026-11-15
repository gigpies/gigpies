# Attended single-speaker USB acoustic bench

Reusable acquisition procedure owned by [GP-PA-MEASURE](MODULE_IMPLEMENTATION_PLAN.md#gp-pa-measure--physical-measurement-workflow).
The November 14 acquisition results and remaining route blocker are recorded in
[the owning card](MODULE_IMPLEMENTATION_PLAN.md#gp-pa-measure--physical-measurement-workflow).
This bench supports uncalibrated spectra and comparative experiments. SHR PA owns
independent-clock EQ proposals; GigPies owns explicit live application and capture.
This path does not establish synchronized GP20 capture.

## Supported pilot-mode acquisition and auto-EQ

The production acoustic adapter has an explicit `pilot_bench: true` option.
Physical invocation also requires a nonempty `mapping_confirmation` recording the
operator-confirmed output map and its evidence; USB enumeration alone never
asserts speaker mapping. Software-only runs retain synthetic mapping provenance.
Default behavior retains exact inactive-channel silence, a −30 dBFS output ceiling
and the original 20-second limit. Pilot mode selects the right output, requires
at least four seconds before excitation and injects only the fixed −80 dBFS,
440 Hz PCM16 left pilot, starting at prefill and continuing across graph mutes
and gaps. Graph output on the inactive channel must still be exactly zero.
Pilot mode admits a reviewed −24 dBFS final-output ceiling, −30 dBFS burst probes
and sessions no longer than 60 seconds. Its explicit playback buffer is 4,800
frames (100 ms) with 48-frame periods; the ordinary adapter retains 960 frames.
Buffer margin is not a measured latency or an acceptance threshold.
This is an explicit bench exception,
not a change to ordinary output isolation or a calibrated acoustic level.

An optional `excitation` mono PCM16/48 kHz WAV (one to three seconds, peak no
higher than −24 dBFS) is loaded and checked before opening playback. It repeats
on the probe source-frame schedule with mandatory gaps. No automatic level
increase or normalization occurs. The source clock is paced by the playback
card; the E205U observation clock remains independent. Both mic channels are
recorded, and the owner analyzer selects channel 1 (zero-based `channel: 0`).

Prepare provider, Brain and explicit control JSON files and trusted owner
artifacts before activation. `scripts/acoustic_bench.py live` starts finite stereo
capture, then the continuous production provider and actual network Brain owner.
It checks PCM ownership first, observes capture readiness, records both channels,
rejects logged xruns and clipping, and terminates only its children. Remote
playback has a finite timeout even after SSH loss. SIGINT/SIGTERM cancels the
local run; the release check is recorded in `session.json`. After closing PCM, a successful
finite provider allows existing Brain connections up to two seconds to finish
their independently started deadlines; it admits no new sessions or rendering.
Review provider/Brain
fault and packet-loss reports before calling a capture valid. No service, mixer,
amp or gain changes are performed. One invocation does not authorize looping;
the operator's aggregate ownership/excitation budget applies across invocations.

```sh
python3 scripts/acoustic_bench.py live /absolute/private/new-capture \
  --provider /absolute/private/provider-config.json \
  --binary /absolute/private/gigpies-remote \
  --ready /absolute/private/ready.json \
  --brain /absolute/private/brain-config.json \
  --control /absolute/private/control-config.json \
  --local-binary /home/shome/p/gigpies/target/release/gigpies-remote \
  --seconds 20 --authorize-audio
```

Remote paths address Pi4 over `gigpies-pi4`; local paths address Pi5. Use fresh
report/readiness paths. The provider duration must finish within capture duration.
SHR PA's `shr-pa-measure --acoustic-eq request.json` accepts adequate repeated
broadband windows against the exact excitation spectrum. Its
[owner contract](../../../shr-pa/docs/OFFLINE_SETUP.md#independent-clock-usb-bench-eq)
defines the declared target and conservative cut-only limits. Review the actual
proposal before passing its path as `pa_proposal` in acoustic-control config.
Preflight the actual candidate through the owner DSP: cut-only EQ can still
increase waveform peaks through phase changes. A peak refusal does not authorize
raising the ceiling. If less excitation is needed, prepare one fixed lower source
and acquire a fresh matched before/after pair; never normalize captures separately.
The controller checks the current PA basis, mutes, applies through `pa_set`, checks
candidate readback while muted, and issues a separate rearm. Failures never retry
or automatically rearm. This uses the production PA transaction; it is distinct
from the synchronized GP20/Desk acquisition wizard.

Current implementation, software results and physical outcomes are recorded only
in the owning GP-PA-MEASURE card. Earlier attempt/handoff sections below preserve
their historical constraints; the current session's explicit authorization and
limits take precedence.

The accepted short Brain runs used a 24 ms scheduled return, leaving eight slots
inside the existing 32-slot jitter horizon. The 32 ms boundary remains a separate
investigation; this is a resource/scheduling choice, not a latency acceptance
threshold. Use actual owner bypass and known delay settings, retain both arrivals
in one microphone recording, and report variability separately from practical
suitability. Do not infer absolute latency or clock lock from those observations.

## Verified inventory, 2026-11-12; rechecked 2026-11-14

Read-only inspection on Pi5 and Pi4 over the existing Tailscale SSH alias found:

| Node | Device | USB formats and state |
| --- | --- | --- |
| Pi5 | Superlux E205U, `hw:E205U,0` | Capture **two** channels and playback two channels, PCM16, 44.1/48 kHz; both stopped |
| Pi4 | C-Media USB HIFI AUDIO, `hw:AUDIO,0` | Playback stereo PCM16/24 at 44.1/48/96 kHz; advertised capture mono, electrical purpose unverified; both stopped |

The user identifies Pi4's connected amp as the Douk ST-01 Pro. Enumeration alone
cannot prove its physical socket/speaker mapping. Use 48 kHz PCM16 for this bench.
The mic has one acoustic capsule, but USB advertises stereo: **do not request
mono directly or assume the two recorded channels are identical**. Record both,
compare, then select one for analysis. Never sum them without checking polarity.

The initial inventory reported Pi5 `Mic Capture Volume` raw `34,34`, range
0..34, declared 0..+34 dB. The current November 14 bench uses **20,20**; preserve
that setting. `Mic Capture Switch` is on. `Mic Playback Switch` (monitoring)
is off. Headphone playback volume is at declared 0 dB. PipeWire holds the mic's
control device, but no PCM handle was open. Nothing was changed. Recheck tomorrow;
do not stop services or steal a busy PCM device automatically.
Pi4 reports mic monitoring off and speaker playback switch on. No PCM owner was
found. Its advertised capture port is **not** an established electrical reference.

### Microphone response and gain

The [manufacturer's E205U manual, page 3](https://img.audiomania.ru/data/superlux_e205u.pdf)
contains the frequency-response graph. It specifies a supercardioid back-electret
mic, 30 Hz–18 kHz and 16-bit/48 kHz. Page 5 identifies the knob as headphone
volume; page 7 directs the logo/LED side toward the source and describes input
level adjustment. The [manufacturer datasheet](https://images.thomann.de/pics/atg/atgdata/document/specs/328609_datasheet_e205u_v2_en_online.pdf)
also lists these specifications. These are E205U documents, not E205UMKII specs.

The published graph is a nominal response, not individual calibration. Do not
invent a flat calibration file or feed digitized marketing curves into automatic
room correction. Directionality, positioning and proximity affect the reading.
ALSA gain is adjustable, but descriptors do not establish whether attenuation
precedes the ADC or merely scales digital samples. Lower gain may improve
headroom; it does not necessarily improve acoustic signal-to-noise ratio or cure
capsule/preamp overload. Measure quiet-room and signal recordings at several
settings with unchanged source level before drawing conclusions.

## Amp wake-up constraint verified November 14

The operator places the mic in front of the **right** speaker. On this rig,
opening and closing playback around short samples loses excitation during amp
wake-up. Keep one PCM owner open through warm-up and all paired trials.
Continuous digital zeros alone did not resolve right-only starts: a −80 dBFS
440 Hz left pilot enabled repeated right output, whereas exact-zero left and an
alternating ±1 PCM16 LSB Nyquist signal did not. After four seconds of the low
pilot, three six-second −24 dBFS right 880 Hz tones measured −29.94, −29.95 and
−29.92 dBFS at mic gain 20,20. Amp controls stayed unchanged. This is a measured
bench workaround; the responsible internal component is not yet isolated.

Record the pilot explicitly: it makes the unused channel nonzero, and is not
strict digital channel isolation. Keep mic monitoring off. The older
`capture --play` command below opens a new playback stream per file and has no
warm-up/keepalive support; its short pilots are unsuitable for qualifying this
amp. Do not interpret silence from that procedure as a broken right output.
The older session-specific −24 dBFS diagnostic did not change the legacy tool's
−30 dBFS limit. The explicit production pilot-mode exception is documented above;
current timing and EQ acceptance belongs to GP-PA-MEASURE.

## Quick continuation handoff (operator request, November 14)

**Time budget: ten minutes wall time, at most five minutes exclusive PCM ownership
and ninety seconds of audible excitation.** Prepare files and inspect existing
artifacts before taking audio devices. No hardware playback was started while
writing this handoff. The previous owned player and capture processes are stopped;
the socket was removed. Recheck ownership before any restart. Do not interrupt
music or another owner. Finish by closing only this task's devices so music can
resume. No broad rebuild, full suite, long soak, extra gain matrix or repeat of
already established routing experiments in this quick pass. If a check needs
engineering changes, record it as pending instead of starting a development cycle.

The quick pass is now recorded in the owning card under **November 14 quick
physical response / PA comparison**. The operator clarified that useful auto-EQ
with this uncalibrated mic is wanted, and that no arbitrary 3 ms target applies.
Calibration uncertainty must be disclosed; it is not a blanket ban on constrained
auto-EQ. The current production acquisition/adapter blockers remain separate.

### Exact working acquisition recipe

- Pi5 capture: `arecord -D hw:E205U,0 -f S16_LE -r48000 -c2`; select
  recorded channel 1 (zero-based index 0) for analysis without summing channels.
  Final mic capture gain is raw `20,20`; direct monitoring is off. Leave amp
  volume/tone and mic placement unchanged.
- Pi4 output: stereo PCM16, 48 kHz, `hw:AUDIO,0` (the successful existing
  helper uses `hw:1,0`; verify these still identify the same USB device).
  SSH is **`gigpies-pi4`**, never a fallback LAN SSH route.
- Existing private Pi4 helper:
  `/home/shome/p/gigpies/user/node-lab/acoustic-bench/continuous-player.py`.
  It owns one `aplay` process with 200 ms buffer / 20 ms periods, sends zeros
  between queued WAVs and accepts JSON over `/tmp/gigpies-acoustic-player.sock`.
  Start under a finite `timeout --signal=TERM --kill-after=2 120 python3 -u ...`
  after checking no current owner/socket. Do not delete a live socket.
- Socket request `{"status":true}` reports the PCM PID and remaining queued
  bytes. Request `{"wav":"/home/shome/p/gigpies/user/node-lab/acoustic-bench/right-low-pilot.wav"}`
  queues the verified stimulus. Use a Python Unix socket over SSH, with proper
  shell quoting. Require `accepted:true`; do not queue while busy. Paths must
  stay under the helper's own directory. Its actual limit is stereo PCM16,
  48 kHz, at most 30 seconds, peak no greater than −18 dBFS; this is a ceiling,
  not a recommendation to increase the verified −24 dBFS diagnostic level.
- The verified `right-low-pilot.wav` is already on Pi4 and locally under
  `user/node-lab/acoustic-bench/current/continuous/`. It is **28 seconds**:
  left 440 Hz at −80 dBFS continuously; right 880 Hz at −24 dBFS at
  seconds 4–10, 12–18 and 20–26, with 20 ms fades. Start recording before
  queuing. The successful capture was 34 seconds with a three-second lead.
  Reuse it only if a route check is needed; do not spend 28 seconds proving
  the same route again by default. New short sequences must retain the four-second
  pilot lead and pilot across gaps; that lead must recur after reopening output.
- The helper's zeros between files are **not a keepalive** for this amp. Put
  the −80 dBFS left pilot inside the entire planned WAV, including warm-up and
  gaps. Merely keeping ALSA open did not fix this. A one-bit Nyquist signal also
  failed. Do not silently describe the non-selected channel as digitally silent.
- At completion, stop the exact owned helper/aplay PIDs, verify they exited,
  then remove their stale socket if necessary. The helper does not have a stop
  request, and termination can leave its socket behind. Never broad `pkill`.

### What is ready and what must not be assumed

`target/release/gigpies-remote` was rebuilt successfully (6.2 MiB executable;
5m04s build). Existing source fixes and suite outcomes are recorded in the owning
GP-PA-MEASURE card. Do not rebuild or rerun them merely to resume this session.
The actual Brain/PA/FX path is software-validated, **not acoustically qualified**.
`src/host/acoustic_playback.rs` currently requires exact silence on the inactive
output, caps final output at −30 dBFS and limits physical sessions to 20 seconds.
`acoustic_probe.rs` permits −60..−42 dBFS probes. Thus the working bench pilot
cannot simply be inserted into this production adapter: it will correctly refuse
it. The built executable is not proof of a ready live configuration/workaround.
Do not run a known incompatible trial or weaken its isolation/level checks to
finish quickly. A reviewed adapter change belongs to a subsequent implementation
pass if no compatible, already-supported route is ready.

For the quick pass, prioritize short repeated uncalibrated response/noise
captures using the established standalone route. A broad-cut before/after check
must use actual SHR PA rendering and identical level/window/placement; label
physical replay of an offline render accurately. Do actual aux/Main relative
spacing only if the real FX path can run with the verified wake-up requirements
inside this budget; otherwise leave that acceptance pending with the concrete
adapter incompatibility above. Never substitute synthetic echoes for FX proof.
No absolute latency claim across independent USB clocks. No automatic room EQ
from the nominal mic curve, no boosts, and no multi-position claim from this
single mic location. The manufacturer curve and manual are already linked above
and retained privately; do not research or download them again.

The owning task card is the single acceptance record. Keep only concise metrics,
reproducible commands and useful private source/capture evidence. Report separately
what is implemented, software-validated, physically observed and still untested.

## Session sequence

Allow roughly 45–60 minutes, with quiet intervals. No unattended schedule exists.

1. **Placement and route, 5 minutes.** Choose left or right speaker. Start with
   the mic roughly 0.5–1 m away, at tweeter height, logo/LED facing it, supported
   off the desk. Record distance, angle, speaker model, room position, selected
   channel and amp tone/volume positions. Keep both speakers correctly connected;
   silence the unused channel digitally. Select USB on the amp, initially minimum
   volume. Do not route the mic to the amp. Keep amp settings fixed across pairs.
2. **Noise and gain, 10 minutes.** Capture silence with amp off, then on at the
   intended low setting. Save the original mic controls. Tomorrow, explicitly
   set `amixer -c E205U cset name='Mic Capture Volume' 0,0` as a first trial;
   repeat at raw 10 and 20 if needed. Capture two channels. Compare noise,
   clipping and a fixed modest source; choose the lowest gain giving usable
   signal. Do not restore +34 dB blindly after the experiment. Record the final
   chosen setting and restore the original only if the operator wants it.
3. **Single-speaker pilot, 5 minutes.** Play one 1-second faded 1 kHz pilot at
   −42 dBFS peak. Raise the physical amp only until clearly audible and
   comfortable. Verify that only the intended speaker sounds. Target captured
   peaks around −24..−12 dBFS; reject near-clipping captures (within 3 dB of full
   scale), audible distress or unexpected routing. Digital level is not SPL:
   no calibrated loudness guarantee is possible with this mic. Keep ordinary
   conversation comfortable and stop immediately if uncomfortable. Do not
   compensate a noisy capture by automatically increasing excitation.
4. **RTA-style observations and response, 10 minutes.** Record the six-second
   band-limited pink signal three times, then the six-second 80 Hz–12 kHz sweep
   three times with unchanged placement/gain. Inspect per-channel levels and
   saved spectrograms; use identical windows/scales for comparisons. This is
   saved spectrum inspection, not a calibrated live transfer-function RTA.
   Compare against the quiet recordings; exclude weak bands rather than boost
   noise. Require useful signal at least roughly 15 dB above background in the
   bands being interpreted. Repeatability matters more than a smooth-looking plot.
5. **EQ comparisons, 10 minutes.** First use the offline SoX demonstration below
   to check the comparison workflow. For an acoustic trial use a reviewed,
   broad cut only (for example −3 dB), preserve headroom and remeasure at the
   same position plus two nearby positions. Avoid boosting deep room nulls or
   flattening every treble feature of this uncalibrated directional mic.
   Existing SHR PA owns actual speaker EQ and proposal fitting. Changes through
   the integrated system need its existing muted review/apply/rearm sequence.
6. **Timing/FX, 10 minutes.** Use short faded bursts, never full-scale clicks.
   A dry event and distinguishable wet repeat in the **same recording** can
   establish their relative spacing in mic frames. Repeat several times;
   reflections may be mistaken for FX. Record a bypass baseline and compare.
   A synthetic SoX echo validates observation only, not SHR FX. An actual SHR FX
   result must pass the production owner and record its configuration and frame
   origin. Wet predelay/echo spacing, algorithmic delay and host/transport delay
   are separate quantities. Current standalone file playback bypasses GigPies
   and SHR FX, so it cannot qualify their live processing latencies.

Start with 80 Hz, not a high-energy sub-bass trial. If low-volume signal-to-noise
is insufficient, move closer while retaining a useful whole-speaker geometry or
average longer/repeated observations. Do not silently raise level. No automatic
feedback loop, normalization of microphone recordings, AGC, or noise suppression.

## Prepared tools and commands

Run from the canonical GigPies checkout on Pi5. `acoustic_bench.py` needs Python's
standard library and existing SoX; the hardware action also needs ALSA utilities
and existing strict SSH access. No package, audio service or persistent config
installation is required. Each output directory must be new, preventing overwrite.
Generated audio and measurements stay under ignored `user/node-lab/acoustic-bench/`.
The prepared left-channel signals already exist there; regenerate for right if
that is the selected speaker:

```sh
python3 scripts/acoustic_bench.py prepare user/node-lab/acoustic-bench/right-signals --channel right
```

Default peak is −42 dBFS. The explicit preparation range is −60..−30 dBFS; capture
checks the actual file's peak, duration, format and inactive-channel silence.
No default audio device is used. SoX generates the stimuli; they are **not** the
special exact floating-point SHR PA sweep protocol.

Tomorrow, after checking the physical setup and current session authorization:

```sh
# Capture only: eight seconds, both mic channels. This starts microphone acquisition.
python3 scripts/acoustic_bench.py capture user/node-lab/acoustic-bench/noise-g0 --authorize-audio
# Capture starts first; playback starts only after recorded PCM begins arriving.
python3 scripts/acoustic_bench.py capture user/node-lab/acoustic-bench/pilot-01 --play user/node-lab/acoustic-bench/signals/pilot.wav --channel left --authorize-audio
python3 scripts/acoustic_bench.py capture user/node-lab/acoustic-bench/pink-01 --play user/node-lab/acoustic-bench/signals/pink.wav --channel left --authorize-audio
python3 scripts/acoustic_bench.py capture user/node-lab/acoustic-bench/sweep-01 --play user/node-lab/acoustic-bench/signals/sweep.wav --channel left --authorize-audio
python3 scripts/acoustic_bench.py inspect user/node-lab/acoustic-bench/sweep-01/capture.wav
python3 scripts/acoustic_bench.py report user/node-lab/acoustic-bench/sweep-01/capture.wav user/node-lab/acoustic-bench/sweep-01/report --mic-channel 1
```

Choose `--mic-channel 2` when warranted by the initial channel check. Reports
contain dBFS levels, full-scale counts, exact channel equality and a SoX
spectrogram. They contain no SPL, acoustic calibration, EQ recommendation or
absolute delay claim. Keep the raw stereo capture as the reference for review.

Capture is finite (maximum 18 seconds for accepted playback); playback has a
remote 15-second timeout plus one second kill grace even after SSH interruption.
Ctrl-C terminates local children; the amp volume remains the immediate physical
stop. There is no microphone-to-speaker path. Nonzero subprocess exit, wrong
capture length or reported xrun leaves the session incomplete. This is not
sample-level dropout detection: unreported USB faults cannot be excluded. The
script does not change or restore controls, retry playback, or claim clock lock.
The guard flag records deliberate invocation, not permission for unattended use.

### Offline RTA/EQ/echo simulation

These commands write files only. They exercise a known cut and a known 200 ms
echo without changing production DSP, pretending room calibration, or playing
sound. Use once per output name; retain only useful final comparison evidence.

```sh
sox -D user/node-lab/acoustic-bench/signals/pink.wav user/node-lab/acoustic-bench/pink-cut.wav equalizer 1000 1q -3
python3 scripts/acoustic_bench.py report user/node-lab/acoustic-bench/pink-cut.wav user/node-lab/acoustic-bench/pink-cut-report --mic-channel 1
sox -D user/node-lab/acoustic-bench/signals/burst.wav user/node-lab/acoustic-bench/burst-echo.wav echo 0.7 1 200 0.3
python3 scripts/acoustic_bench.py inspect user/node-lab/acoustic-bench/burst-echo.wav
```

The same capture command accepts bounded mono-routed PCM16 files from a reviewed
actual owner render, validating their final peak and silent opposite channel.
Do not normalize EQ before/after independently; that would erase level changes.

## Real Brain FX experiment: dry acoustic reference plus wet return

Correction after the November 13 routing discussion: independent mic/playback
clocks do **not** prevent measuring wet-versus-dry relative delay when both arrive
through the same speaker into the same mic recording. The standalone player above
cannot exercise Brain, but the production four-aux route can. See
[the actual aux contract](../reference/REMOTE_FX_CONTROL.md).

Proposed production-path trial:

1. Generate two distinguishable, short faded probe bursts from the **same source
   frame**: A feeds dry Main, B feeds an FX aux. The suggested 1 kHz/2 kHz carriers
   are identifiers, not timing references by themselves; use known burst envelopes
   or coded probes and a known onset separation (initially zero).
2. Send B through the real source transport → Brain SHR FX owner → wet return
   admission → Main → PA → selected single speaker. Record A and the returned B
   together on one mic channel. Preserve dry/wet source-frame taps and actual
   effect settings, scheduled return delay, missing/late packet and xrun counters.
3. Measure the difference between their identified arrivals, subtracting any
   deliberately introduced source-onset difference. Common DAC, acoustic travel
   and capture-start offset largely cancel. Different carrier frequencies can
   have different speaker/room/filter group delays; use a same-probe dry+wet trial
   with a clearly separated delay repeat as a cross-check, and repeat captures.
4. Start with the actual digital-delay owner, a known long first repeat and no
   feedback. Report measured extra wet-path delay, intentional effect delay and
   scheduled transport delay separately. Reverb onset and modulation require
   different estimators; do not call every wet peak processing latency.

The per-input/per-aux pre-fader extension now supplies the needed send tap.
Select pre-fader for B's FX send and lower B's channel fader; keep B unmuted.
Current fader minimum is -60 dB, so measure residual direct B leakage rather than
claiming exact zero. Use the corresponding actual source-frame dry tap as a second
reference and confirm that the intended FX return dominates B's direct leakage.
Channel mute still stops excitation; existing tails drain unless deliberately cleared.
See the [contract](../reference/REMOTE_FX_CONTROL.md#per-input-prepost-fader-sends-send-tap-extension-1)
and [Desk controls](../../../shr-desk/docs/FX.md). This software capability does
not by itself supply the physical output adapter or qualify timing.

Prototype source-frame probes and an explicit PCM16 playback adapter now exist in
`src/host/acoustic_{probe,playback}.rs`, with authenticated bench control in
`src/remote/runner/acoustic_control.rs`. SHR PA owns the independent-clock arrival,
leakage and spectrum estimators in its `src/acoustic.rs`. The generic
`acoustic_bench.py` player does not exercise that production path. Consult the
owning card for current validation and remaining failures before activation.
No additional physical reference cable is required for the **relative** test;
absolute source-to-mic latency remains a separate measurement.

## What this wiring cannot yet establish

Pi4 USB playback and Pi5 mic capture have independent device epochs/clocks. Same
nominal sample rate, SSH, Tailscale and NTP do not create a common sample origin.
Aligning each response by its own peak hides latency and drift. Do not import
these recordings as synchronized GP20 records or mark SHR PA common-origin flags
true to get an EQ proposal accepted. Existing repeated-sweep extraction requires
an exact reference, common timing and a very quiet tail; this bench meets none of
those requirements merely by producing a WAV file.

For absolute electrical/acoustic system delay, obtain a real common-clock
reference capture. Options for a later approved wiring step: a verified shared
interface with separate reference and measurement inputs; or investigate mic
headphone playback to the amp's **line input**, with real duplex timing qualification.
The second changes the topology and still needs converter-offset validation.
Never connect speaker outputs to a USB mic/headphone input. Pi4's advertised
mono capture needs physical jack/level verification before use. Keep distance
fixed; acoustic propagation is approximately 2.9 ms/metre, not software delay.

Production FX/ASRC latency acceptance needs the real source-frame taps and owner
path, not this generic player. The existing hardware host's earlier AudioBox
bench numbers do not apply to these devices. These remain explicit next gates
in GP-PA-MEASURE and GP-H1, not promises for tomorrow's present wiring.

## Validation and retention

Fast standard-library tests:

```sh
python3 -m unittest discover -s scripts -p 'test_acoustic_bench.py'
```

Preparation validation uses real SoX offline to generate and inspect all four
signals and render a spectrum. Hardware capture/playback is deliberately deferred
to the attended session. No engine/schema/routing code changed; Rust production,
historical audition, exhaustive and hardware/load suites are outside this change.
Retain selected final captures, signal sources/commands, and one brief outcome in
the owning task card. Remove failed trials/raw duplicates after review. Do not
publish recordings, per-machine mixer snapshots or generated plots.

## Continuation prompt: complete the real Brain FX bench

Use this after the pre/post-fader implementation and its software validation:

> Complete the GigPies acoustic measurement bench using my current wiring:
> Superlux E205U on Pi5; Douk ST-01 Pro USB amp on Pi4; one selected speaker.
> Work across GigPies, SHR Desk, SHR PA and SHR FX as needed, with implementation
> in its owning module. Keep the existing single-thread workflow and canonical
> checkouts; preserve pending work. Read the parent/repository instructions,
> GP-FX and GP-PA-MEASURE cards, ACOUSTIC_BENCH.md and REMOTE_FX_CONTROL.md first.
>
> Finish the code, not just another plan. Use the newly implemented per-input
> pre/post FX sends. Generate distinguishable bounded bursts A/B on one known
> source-frame timeline: A goes dry to Main; B goes through a pre-fader aux with
> its channel fader down, through the actual network Brain SHR FX owner, back
> through production wet-return admission, Main and SHR PA to one physical amp
> channel. Keep B unmuted. Quantify the current -60 dB fader-floor leakage; do not
> claim digital silence. Do not replace the real FX path with SoX echo or playback
> of a previously processed WAV. Also run a same-probe dry/FX-repeat cross-check.
>
> Inspect the existing physical host before extending it: earlier AudioBox
> native-S32 duplex assumptions do not match these devices. Prepare a bounded
> explicit playback adapter if required, with Pi4's output soundcard driving the
> source timeline and Brain following source frames. Pi5 mic capture is a separate
> observation clock, never feedback to Main. Validate the exact USB endpoints,
> formats, stereo playback, microphone channel mapping and host ownership. No
> automatic service, mixer, clock or network tuning. Never infer physical capture
> wiring from Pi4's advertised mono input.
>
> Implement offline arrival/relative-delay, spectra, repeatability and before/after
> comparison through the owning measurement module. Preserve raw stereo mic WAVs
> and source-frame dry/send/wet/final-output references, effect settings, source
> revision and library hashes. Distinguish intentional effect delay, scheduled
> transport delay, observed relative extra wet-path delay and absolute latency.
> Common acoustic/capture offset cancels in a same-recording difference; bound
> clock drift and frequency-dependent arrival bias rather than silently aligning
> away delay. Do not claim absolute latency, calibrated SPL or calibrated mic
> response. The manufacturer curve is nominal, not an individual calibration file.
>
> First prove software behavior: actual Brain DSP/transport; source-synchronous
> probes; one silent output channel; pre/post, fader, mute and tail behavior; no
> allocations/locks/I/O in callbacks; bounded cancellation/deadlines, no stale
> returns, loss/xrun reporting, gain/peak limits and no automatic output rearm.
> Add meaningful focused tests and run the required normal suites. Leave hardware
> stopped during preparation. Report readiness and obtain my placement/channel/
> low-amp-volume confirmation before the first physical trial.
>
> Then perform attended, quiet tests: gain/noise trials (E205U was at +34 dB),
> verify its two advertised capture channels, quiet pilot starting at -42 dBFS,
> repeated pink/sweeps, actual FX relative timing at two known delay settings,
> and a conservative reviewed EQ cut followed by fresh measurements. Keep the
> unused speaker channel digitally silent. Stop on discomfort, clipping, bad
> signal-to-noise, unexpected route, xrun or loss; do not automatically raise
> excitation. No sharp/full-scale clicks or feedback. Report measured results
> with uncertainty and explicit refusals, and update the existing owning card.
> Retain only selected captures, reproducible commands and concise results;
> remove task-created scratch and never publish private recordings.
