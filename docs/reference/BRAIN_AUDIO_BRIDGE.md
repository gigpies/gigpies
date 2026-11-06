# Brain device and clock bridge

Acceptance declared before implementation (2026-10-05): the 48 kHz f64 sinc
path must retain gain within 0.1 dB through 18 kHz, reject the 23–24 kHz
stopband by at least 80 dB, and keep an inactive channel below −100 dBFS.
Reference signals and independent least-squares tone projections establish these
thresholds, rather than comparison with another instance of the same resampler.
Admitted steady skew is ±1000 ppm; the hard controller bound is ±1500 ppm.
No raw recording or source-clock FX is resampled.

Rubato 5.0.1 is pinned with default features disabled (no FFT or logging). Its
[official realtime guidance](https://docs.rs/rubato/5.0.1/rubato/)
and reviewed published source specify preallocated `process_into_buffer`, variable
ratio and MIT OR Apache-2.0 licensing. The selected Async sinc uses 256 taps,
squared Blackman-Harris window, 128 phases, cubic interpolation and cutoff 0.875
of Nyquist. Coefficients, history, input and output storage are prepared once.
Ratio changes and reset reuse that storage. Both allocator operations must remain
absent from push, render and failure handling; retirement belongs to the controller.

The signed ratio is destination/output frames per source/input frame. Faster source
clocks therefore require a smaller ratio. Every bridge binds source epoch,
destination epoch and route generation. Integer source and destination cursors are
never replaced with wall-clock timestamps. Absolute device-to-device phase is
unknown: source cursor, queue occupancy and filter delay describe the current
mapping, not hardware lock or one-way network latency.

Each crossing has independent storage and controller. Fixed 48-frame resampler
quanta preserve arbitrary caller partitioning. Ring capacity, target occupancy and
maximum caller block are validated finite resources; the initial profile targets
960 queued source frames and permits 4096, separately from the 128-frame filter
latency. Startup requires target plus the resampler's next input requirement and
explicit arm. Network/scheduling jitter is absorbed by the ring; a two-second
low-pass occupancy filter feeds a slow PI drift controller, limited to 100 ppm/s.
No routine sample insertion, duplication or deletion corrects skew. Exhaustion,
overflow, discontinuity or sustained controller saturation closes the path.
A 240-output-frame fade reaches silence; old queued data is discarded, and recovery
requires a different route/device identity, fresh prefill and explicit arm.

Only configuration intent is serializable. Runtime identity, readback readiness,
arm state and held talkback are not restored by configuration loading. The raw ALSA
adapter opens both directions of one explicit `hw:` endpoint, verifies negotiated
format/rate/width/period/buffer and card/device identity, and never picks a default.
Actual physical socket labels, shared-clock and direct-monitoring observations
must be supplied by a separate physical acceptance procedure. Fake and raw devices
implement the same nonblocking bounded service boundary.

## Host boundary and bounds

`BrainHost<D: DuplexDevice>` prepares one process-owned endpoint. It accepts an
explicit epoch and configuration generation, starts disarmed, and requires matching
`DeviceReadback` before `arm`. `BrainRender` receives mapped mono capture and supplies
stereo monitor frames on separate integer cursors. The network adapter connects
these callbacks to authenticated media; it does not clock FX from these callbacks.
Capture and playback each receive at most one nonblocking driver call and one
period of render work per service call. Partial progress in either direction does
not prevent servicing the other. More than 100 ms without progress, a backward
service clock, driver xrun, nonfinite PCM or disconnect invalidates the whole local
duplex epoch. Stop/reopen cannot rearm the same host. This is controller-side driver
service, not a claim that ALSA I/O belongs in a realtime DSP callback.

The `gigpies-brain` executable runs this service on its sole dedicated duplex
thread. A separate media worker owns GBA1 decoding/encoding and the authenticated
channel; controller awaits, device-intent persistence, hashes and JSON never run
in the service quantum. Independent four-block capture and monitor lanes drop new
blocks on overflow and count them. Four-entry copied control/status lanes bound
ordinary work; service applies at most one ordinary state update per quantum.
An atomic closure generation bypasses queued updates. Local deadman expiry closes
audio and requires observed closed state followed by a later explicit active
revision; changing an unrelated revision cannot restore an expired path. The
service also enforces the configured finite duration without controller progress.

Replacement stops and joins the old owner before preparing a fresh device epoch.
Service observations copy numeric state and its applied revision; the controller
joins static device descriptions and serializes telemetry. Reports distinguish
known prepared layouts (with exclusions) from measured process RSS/peak RSS, CPU
and faults. Playback evidence labels the service revision at device-drain time and
the contemporaneous controller snapshot; neither label claims a sample-exact
control transition within an already submitted driver period. Full observation
lanes skip publication while retaining cumulative counters for later readback.

Native signed16, signed32 and float32 little-endian formats are explicit. Conversion
normalizes the signed container width; it does not claim converter significant bits.
The adapter rejects a negotiated profile differing from the requested one. Reported
capabilities describe that negotiated profile, not every possible hardware mode.
Only the supplied physical records can establish socket mapping, shared duplex
clock setup and absence of unintended direct monitoring. Software never manufactures
those observations from nominal sample-rate equality.

Transport admission bounds are 256 capture/playback slots, a 48–1008-frame period
in multiples of 48, and at most 48000 device-buffer frames. They limit prepared memory
and per-service work, not a universal product channel count. Widening them requires
memory/work review and focused tests. Bridge channels are independently bounded to 8,
caller blocks to 4096 frames and ring storage to 48000 frames; the production crossings
use one talkback channel and two operator-monitor channels. A stereo request requires
two distinct physical slots and IDs; this increment does not implement mono playback.
Unmapped device playback slots are always silent.

Local microphone gain is −60…+20 dB; monitor gain is −90…0 dB with optional −20 dB
dim. Both start muted. Gain/mute transitions are bounded to at most 240 samples;
physical submission is peak-clamped to ±1 to prevent integer overflow. Local monitor
settings never modify Stagebox mixes. Capture and outgoing peaks are distinct;
playback peak describes submitted samples, not measured acoustic output.

## Reproducible validation

Run every Cargo command using the repository's parent-held nonblocking build lock,
Rust 1.97.1, locked dependencies, one job and incremental compilation disabled.
Normal `--test brain_bridge` protects numerical gain/stopband/isolation, impulse
area/delay, DC/chirp/silence, arbitrary render partitioning, both allocation operations,
short drift/jitter acquisition, epochs, five-ms failure fade, physical-map admission,
fake-device partial transfers, stale readback and no-progress recovery.

Five longer synthetic evidence cases are opt-in after their one-time results:
`production_bridge_tracks_both_1000ppm_signs_and_separate_jitter`,
`outside_admitted_clock_skew_fails_closed`,
`slow_rate_ramp_crosses_both_signs_without_sample_corrections`,
`actual_fake_duplex_and_both_bridges_follow_independent_integer_clocks`, and
`long_virtual_clocks_exceed_uncompensated_buffer_exhaustion`.
Run these after changes to the bridge, controller or their acceptance assumptions:

```sh
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
  cargo +1.97.1 test --locked -j1 --release --features hardware-host \
  --test brain_bridge -- --ignored
```

Virtual time is accelerated without sleeps. Tests do not open PCM, microphone,
headphones, speakers, MIDI or displays. Independent physical drift, actual latency
and sustained hardware deadlines remain separate acceptance gates.

The 48-frame production packet profile reaches initial readiness at 1056 queued
frames (Rubato's initial input request is 49; consumers use `status.ready`, not a
hard-coded prefill count). A zero-skew test originally overfilled1152frames and
mistook the resulting occupancy-acquisition correction for oscillator drift at 5 s;
that failed observation is retained in private task evidence. The corrected test
starts at the first ready packet boundary without changing its 300 ppm transient
bound. Displayed ratio-derived skew includes acquisition/latency correction and is
not a calibrated hardware frequency measurement. Queue and filter latency use
nominal-rate milliseconds; physical mapping uncertainty remains unknown and physical
clock lock is explicitly false/unverified. Source cursor identifies consumed input,
not a claim that the latest output sample belongs to that exact source frame.

A unity-clock analytic 997 Hz phase regression declares maximum absolute waveform
error 0.001 at 0.5 amplitude. The reviewed coefficient-grid convention gives fractional
delay 127−1/128 frames (integer cursor advance precedes output; the phase-zero sinc
is centered at 128−127/128). This is an analytic reference, not delay fitted from
produced samples. Rubato's reported 128-frame delay is a conservative budget within
two frames of that reference. An independent impulse-area/peak test additionally
checks the declared delay budget, without assuming exact raw-sample preservation.

At a selection/disarm boundary, the owner also calls
`BrainHost::invalidate_monitor_output`. It returns the first unsubmitted local
device frame and silences the remainder of any partially written prepared period.
Capture and both device cursors continue. Already submitted PCM cannot be withdrawn
without a duplex discontinuity; its tail is separately bounded by configured
`buffer_frames`, not by ASRC queue latency. The partial-write regression verifies
this boundary exactly. Physical output latency remains unqualified.

Raw opening additionally offers controller-only `open_detailed`: failures identify
capture/playback direction, operation, ALSA errno/message, or requested-versus-
negotiated values. The compact callback error enum does not allocate strings.
Unsupported format, rate, stream width, period and buffer are distinct failures.
Opening failure invalidates the prepared host epoch; retry requires fresh preparation.

The combined clock opt-in
`actual_fake_duplex_and_both_bridges_follow_independent_integer_clocks` uses actual
FakeDuplex/BrainHost capture and playback callbacks, mono talkback and stereo monitor
bridges, permuted physical slots and fixed 48-frame media groups. Its separate
integer virtual oscillators cover both signs of 10/100/1000 ppm, with finite runs
longer than the selected queue could survive without rate correction. It checks
actual submitted stereo samples, returned talkback samples and unmapped silence.

For that fixed-packet combined test, declared clock tracking error is below 30 ppm
for the mean applied correction over the settled second half of each trial.
Instantaneous queue correction is deliberately not treated as an oscillator
measurement: 48-frame packet cadence introduces bounded scheduling-phase steps.
Every admitted crossing must retain bounded occupancy and zero under/overruns;
constant reference samples check conversion gain and channel/socket isolation,
while the independent numerical suite supplies tone/phase/chirp/impulse references.
