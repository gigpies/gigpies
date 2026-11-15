# USB host integration and measurements

**The 1 ms host completed a ten-minute stereo bench with zero reported xruns,
late FX returns or recording gaps, and exact stored dry/combined output.**
Channel-1 electrical delay measured **5.19–5.23 ms**. Two one-frame physical
reference shifts remain a qualification; fixed converter timing is not certified.
The tested profile uses 48-frame processing, 192-frame ring capacity, zero silent
prefill and 6 ms wet admission, with the temporary memory settings below.
The earlier 56 ms prefill result is historical and rejected for live latency.

Unreleased, bounded bench integration of the independently built SHR modules.
The [execution plan](../archive/plans/AUDIO_HARDWARE_PLAN.md) continues the preceding
[synthetic transport phase](../reference/AUDIO_TRANSPORT.md). This implements an explicit ALSA host and real stereo module processing/recording.
The live audio and lighting consoles, complete mixer, multichannel stagebox and recorder UI remain
separate planned integrations.

## Topology and device scope

Pi 5 owns a PreSonus AudioBox USB 96 (USB 194f:0303, ALSA ID A96); Pi 4 runs
source-following SHR FX over the reserved Ethernet link. The interface has
two capture and two playback channels. Kernel descriptors list FL/FR,
S32_LE with **24 descriptor-declared bits**, and 44.1/48/88.2/96 kHz. The earlier v7
configuration opened both directions at **48 kHz, two channels, 384-frame periods
(8 ms) and 3072-frame buffers (64 ms)**. Earlier 16/32 ms buffers had retained
playback underruns. A separate negotiated ALSA query returned **32 significant bits**,
despite the USB descriptor's 24-bit declaration. A retained native S32 capture
contained only 0 or 3 in the low byte; its rounded PCM24 codes exactly matched
upper-24-bit extraction. This does not establish a 32-bit converter. The host now
explicitly extracts the upper 24 bits and hashes those bytes independently of
floating-point conversion. No resampling or host clock change occurs.

At the time of this earlier AudioBox bench, no Superlux microphone or other USB
audio interface was connected to either Pi, and the amplifier was disconnected.
For the different November 12 E205U/USB amp inventory and next-session procedure,
see the [acoustic bench](../development/ACOUSTIC_BENCH.md).

All earlier soak/fault trials initially had no established physical return. Afterward the user connected both outputs to inputs; the separate
physical checks below verified the left route and found a severely attenuated
right return. Converter clock lock and calibrated capture-to-output latency remain
unverified. No independent-device drift or ASRC experiment is possible or needed
for this source-following Brain.

The command requires a stable explicit raw USB card ID and matching kernel USB
identity/stream descriptor. It never chooses the default device, Bluetooth or
HDMI. No mixer controls, persistent services, NIC settings or clock daemons are
changed. ALSA status timestamps use CLOCK_MONOTONIC; reported audio timestamps
and USB feedback frequency are driver observations, not calibrated converter
timestamps or measured clock lock. See the
[ALSA format/status contract](https://www.alsa-project.org/alsa-doc/alsa-lib/pcm.html).

## Implemented slice

`src/host/` owns the bounded frame/routing contracts and the optional host.
`gigpies-hardware` is built only with `--features hardware-host`; the ordinary
offline CLI and normal tests open no hardware. Libraries are explicitly supplied
trusted local executables with versioned C symbols. No sibling path dependency
or second DSP engine implementation is added to GigPies.

| Owner | Interface used and precision |
| --- | --- |
| SHR PA | `shr_pa_v1_*`: native f64 stereo input, six logical outputs, full-range unity processing, 5 ms startup ramp and existing −1 dBFS linked sample limiter; no fixed algorithmic delay |
| SHR FX | `shr_fx_v1_*`: existing wet-only stereo Digital Delay, **20 ms / 960 frames** first tap, 0.25 feedback, 0.35 damping and 0.5 wet gain; shared delay implementation specializes **f64 DSP/state** for integration; the existing rack retains f32 |
| SHR REC | `shr_rec_v1_*`: preallocated whole-block queue, independent disk worker, mono PCM24 stems, source epoch/frame journal, explicit gap/fault counters, no-replace finalization and recovery into a new directory |

The initial smoke used generated audio only. Subsequent trials explicitly chose
`capture_with_probe`: captured stereo at a digital factor of 0.125 plus distinct
997/1499 Hz signals bounded at −36 dBFS, quantized to PCM24 before DSP. This
digital gain does not change analogue input gain or repair ADC clipping.
The captured inputs were quiet and no ADC full-scale samples were observed in
the short trials. There is no implicit microphone monitoring mode.

One PA instance supplies a dry audit tap. A separate instance processes the
same source plus wet returns at a factor of 0.25, so the final combined path
passes through existing PA protection. The bench additionally caps submitted
output at ±0.25 (about −12 dBFS). Logical PA outputs 0/1 map directly to physical
outputs 0/1; the other four logical channels are not summed into the stereo card.

The take has **eight software stems, only two of which are physical inputs**:
ADC L/R, actual PA source L/R, PA dry L/R, and DAC-submitted L/R. The last pair
records the intended PCM block, not a return measurement from the DAC. After a
failed partial write, its physical delivery is unknown. Reports distinguish
captured, recorder-accepted and fully written playback blocks within the requested
take. The bounded quiet shutdown transfers are outside that recorded interval.

## Timeline, startup and recovery

Actual USB capture transfers pace source frames. The current low-latency host
uses 48-frame blocks and one GPA1 float32 FX packet per block. Brain has no audio
device and follows these IDs. The H5 trials use source frame +192 (**4 ms**) for
wet return admission. This does not delay the local dry path. The test effect's
intentional **20 ms** first echo is additional. Historical 8/16 ms admission
trials below used different host pacing and must retain their own results.

Before starting the PCM streams, the host prepares libraries, files and queues,
then requires a fresh control snapshot followed by an acknowledged command.
Capture starts first. With `prefill_periods: 0`, playback starts as soon as the
first processed block is submitted; no silent blocks are queued ahead of it.
ALSA ring capacity is a separate setting. The audio-owned jitter buffer admits
queued returns before rendering their output block; earlier frames are refused.
Arrival never advances its cursor. Each wet channel independently fades the last
valid sample to zero over 240 frames (5 ms); fresh data ramps back in. Brain
rejects wrong epochs, duplicate/backward frames and inconsistent sequence IDs;
forward gaps reset its FX history. It withholds returns while control is unsynced.

The network worker and recorder worker have independent bounded queues. A full
network queue cannot block dry processing or recording. The render section,
including fault returns, performs bounded arithmetic and queue/module calls
without allocation, deallocation, locks or I/O. ALSA reads/writes, waits, status
queries and audit hashing are outside that section. Fixed-capacity timing and
status records are filled without allocation during streaming and serialized
after PCM stops.
This is a direct ALSA driver loop, not a claim about a JACK callback or hard
real-time scheduling guarantees. Full host service time and capture intervals
are reported separately from render time.

Brain process replacement leaves the PA process, source epoch, DSP configuration,
recorder and authoritative prototype control state alive. A new monotonically
increasing lease obtains a fresh snapshot. **Control values remain the existing
prototype test parameter; acknowledgments do not apply a real mixer setting.**
The bench DSP configuration is fixed for the run. PA/device restart requires
new handles, a new recording and a fresh source epoch. There is no automatic
xrun concealment or attempt to append discontinuous capture as a complete take.

## Actual-device checks

Release libraries and host ran without competing builds under private task 0004,
reservations H1/H2. The first tests below used period 192/buffer 768 and 8 ms return
admission and the initial **f32 FX** library, with no system tuning.

| Trial | Captured/retained frames | Result |
| --- | ---: | --- |
| 3 s generated-source smoke | 144000 | Zero xruns, drops or missing wet packets; all eight PCM hashes and exact PA/FX/DAC replay match |
| 10 s capture + probe | 480000 | Zero xruns, drops or missing wet packets; exact dry and combined-output replay match |
| 15 s packet faults + 250 ms Brain stall | 720000 | Raw/tap hashes and dry replay match; 295 missing wet packets, 260 expired returns refused; both channels recover |
| 15 s actual Brain termination/replacement | 720000 | Raw/tap hashes and dry replay match; 1144 missing wet packets, three socket errors during outage, two fresh snapshots; both channels recover |
| Deliberate 100 ms local driver stall | 96000 before stop | One ALSA capture xrun; nonzero process result and **incomplete** finalized take with host fault retained |

File verification reads every stem, checks format/length and payload SHA-256
against bytes submitted by the host, examines every source/file journal entry,
and independently replays the owning PA library from the recorded source.
The no-fault trials also exactly reproduce the final recorded DAC blocks through
the owning FX and final PA libraries. Thus continuity evidence includes actual
stored samples; it is not just increasing block counters. Without a known
physical return signal, it cannot certify an undetected converter-side anomaly.

For the known 20 ms packet-loss burst, measured residual wet output in **both**
recorded channels reaches exactly zero after 240 frames. Maximum deviation from
the expected linear fade using the quantized last sample is 0.50/0.692 PCM24 LSB.
Both channels have sustained zero wet output during outages and nonzero wet output
after recovery. Dry replay remains bit-exact throughout. The local xrun take is
preserved and verifies exactly through its final retained block; it is classified
as incomplete. The xrun counter counts detected ALSA errors before immediate
stop, not every possible underrun/overrun in both streams after a stall.

The 10 s capture trial measured render p99/max 0.540/0.722 ms and complete
post-read host service p99/max 0.629/0.843 ms against a 4 ms period. Network RTT
p99/max was 0.459/1.303 ms. Capture service intervals reached 7.609 ms despite
zero xruns; buffers absorb scheduling variation. These values do not establish
physical output at an 8 ms deadline or guarantee every host wakeup within 4 ms.

Candidate v3's fault RTT histogram stored only 0–10 ms bins. A rank above that
range incorrectly displayed the maximum as a percentile; that value is only an
upper bound. Candidate v4 reports such percentiles as unavailable, retaining
overflow counts and the uncapped maximum. Earlier evidence remains unchanged.

The first **600 s soak at 8 ms admission** retained 28.8 million frames with zero
xruns, recorder drops or journal gaps. All 600000 network packets returned, but
**one expired and produced one missing wet block**. The original zero-loss target
therefore failed. All eight PCM hashes and dry replay match; 190 DAC-submitted
samples differ from uninterrupted FX replay. The original take is retained.

RTT p99/max was 0.455/7.717 ms; render p99/max 0.654/1.150 ms; complete post-read
host service p99/max 0.734/1.286 ms. Capture intervals reached 8.588 ms. The
7.717 ms RTT leaves little margin in an 8 ms budget with 4 ms capture batches and
host wakeup variation. These aggregate observations motivate the declared 16 ms
comparison; they do not identify a guaranteed worst case or prove a particular
scheduler cause. The revised candidate also makes upper-24-bit capture extraction
explicit and checks a separate native ADC byte hash.

### Revised 16 ms admission and device-buffer comparison

At period 192, a fresh 30 s/1.44-million-frame comparison passed with zero xruns,
missing wet packets or file/replay differences. Repeated 15 s packet-stall and
Brain-restart trials each preserved 720000 frames, direct ADC hashes and exact
dry replay. Faults caused 283 missing/248 expired returns; restart caused 1148
missing returns, four socket errors and two fresh snapshots. Both wet channels
recovered. The known loss burst again faded to zero in 240 frames, with maximum
0.525/0.600 PCM24 LSB error against the quantized previous sample.

The first attempted 600 s run at 16 ms admission **failed with a playback EPIPE**
after 10.28 s: 493248 frames fully submitted, 493440 captured/recorded. All retained
samples match direct ADC hashes and dry/intended-output replay, with no journal
gaps or wet losses, but the last block's physical delivery is unknown. REC marks
the take incomplete with an external host fault. Render maximum was 3.709 ms,
with 0.554 ms p99; the last completed host-service maximum was 0.932 ms. Those
aggregates do not prove the underrun's scheduling cause or include the failed
cycle in the completed-cycle maximum. The host stopped; no gap was concealed.
The private supervisor originally timed out waiting for Brain after this early
exit. Its original log is retained; exact task-process cleanup followed. The
supervisor now persists local observations before bounded peer cleanup.

The revised device condition is period 384/buffer 1536 (8/32 ms), retaining 16 ms
wet admission and the same exact v5 binaries. This gives additional buffering
margin without clock/service tuning. A 30 s comparison passed with 1.44 million
frames, no xruns, no missing wet returns and exact ADC/dry/DAC verification.
Render p99/max was 1.148/1.264 ms; full post-read service 1.319/1.437 ms. Capture
intervals reached 16.007 ms, overflowing the 10 ms histogram; p95/p99 are therefore
unavailable, not substituted with a maximum.

The subsequent 600 s run at that condition completed 28.8 million frames with
zero xruns, recorder/network queue drops, missing wet packets or expired returns.
All 600000 FX packets returned. Render p99/max was 1.164/5.979 ms and complete
post-read service 1.329/6.257 ms. RTT p99/max was 0.574/11.018 ms; one RTT value
overflowed the histogram, while p99 remained in range. Capture intervals reached
15.995 ms, with p95/p99 outside the histogram. Observed playback queue delay was
408–1152 frames (8.5–24 ms). This run still uses the initial f32 FX library;
all direct ADC hashes, eight stem hashes, journal entries and full dry/DAC replay
match exactly. The native f64 follow-up is recorded separately below.

The failed 8 ms wet deadline and failed 4 ms device-period run remain failed
observations. Changing these separate budgets does not establish physical
capture-to-speaker latency or hard real-time guarantees.

### Native f64 precision completion

The final FX implementation shares the existing delay algorithm through static
f32/f64 specialization. Integration coefficients, delay buffers, feedback/filter
history and arithmetic are f64; the existing rack remains f32. The C ABI, 960-frame
first tap and effect settings are unchanged. GPA1 still deliberately encodes FX
sends and returns as float32. This separates internal DSP precision from the
chosen transport format.

SHR FX commit `6510ead9cea8e28a92d992a4147cda17a074c86f` passed 101 normal tests,
including sub-f32 input detail, f64 feedback coefficients, no-allocation/reset
checks and exact original f32 rack sample bits. Formatter, Clippy, release and
dynamic ABI checks passed. Because shared hot paths changed, the opt-in ten-case
cost matrix was rerun: finite/fault checks passed, but the sixteen-hall rack's
8.566 ms maximum exceeded its 5.333 ms nominal period. That heavy configuration
remains unaccepted; it is separate from this stereo delay bench.

Candidate v6 keeps the exact v5 host and PA/REC libraries and supplies this new FX
library under a separate immutable filename/hash. Its 30 s comparison preserved
1.44 million frames with exact direct ADC/stem hashes, dry/DAC replay and zero
xruns or wet losses. The 15 s packet-stall test preserved 720000 frames with
exact ADC/dry/journal checks, 299 missing and 264 expired returns.

The next Brain-restart attempt **failed with another playback underrun** at
5 seconds: 240000 frames were fully submitted, 240384 retained. All retained
samples and intended-output replay match, but the incomplete take cannot count
as successful restart continuity. Render maximum was 6.826 ms and capture-read
maximum 14.974 ms. The driver had initially primed 24 ms of output and its lead
varied with USB batching. These observations justify more device buffering;
they do not prove a particular kernel scheduling cause or implicate FX precision.
A separate deliberate 100 ms stall again stopped cleanly at 96000 frames and
retained an exact incomplete take.

Candidate v7 adds an explicit `buffer_periods` field, restricted to four or eight,
and verifies exact device negotiation. The revised setting is eight periods:
**3072 frames / 64 ms buffer**, with 2688 frames / 56 ms of initial silence.
Period remains 384 frames / 8 ms; wet admission remains 768 frames / 16 ms.
This increases device latency and is a conservative bench setting, not acceptance
of the smaller-buffer latency target. Shutdown flushes the selected buffer length.
Failed playback cycles now contribute to host-service timing and get a separate
report field; earlier completed-cycle maxima omitted the failed cycle.

H2 renews the same selected device/link reservation through 19:00 UTC, each trial
still bounded to 660 s. The v7 30 s smoke retained 1.44 million frames with exact ADC/stem hashes,
dry/DAC replay and zero xruns or missing wet returns. Both 15 s fault trials
retained 720000 frames each with exact raw/dry/journal checks and no xruns.
Packet faults caused 299 missing/264 expired returns; restart caused 1136 missing
returns, three socket errors and two fresh snapshots. Both wet channels recovered: the final continuous nonzero wet intervals were
22272 frames (0.464 s) after the last packet fault and 408960 frames (8.52 s)
after restart. The known loss burst reached zero in 240 frames, within 0.821/0.767 PCM24 LSB of
the expected fade. Recovery is checked over the final 9600 source frames after
the last injected fault; a previous block-count window included another programmed
loss when the period doubled, so that original assessment is retained and corrected.
The deliberate 100 ms driver stall again retained an exact 96000-frame incomplete
take.

The **final native f64 v7 600 s soak passed**: 28.8 million captured, recorded
and fully submitted frames; zero detected xruns, missing/expired wet packets,
recorder/network/return queue drops or file errors. All 600000 packets returned.
Direct ADC byte hashes, all eight stored PCM hashes, the complete source journal,
dry replay and combined DAC-submitted replay match exactly. No ADC full-scale
samples or recorder clipping were observed.

Render p99/max was **1.123/4.949 ms** against an 8 ms period; complete post-read
host service was **1.290/5.233 ms**. Network RTT p99/max was **0.565/1.977 ms**;
Brain FX processing p99/max was 0.029/0.122 ms. Playback queue delay ranged from
2018 to 2646 frames (**42.042–55.125 ms**). Capture service intervals reached
17.053 ms; 37500 of 74999 exceeded the histogram's 10 ms range, so even p50 is
unavailable. Buffering absorbed that service variation in this bounded run.

Pi 5 process CPU ended at 20.2% of one core, maximum RSS 12.31 MiB and observed
temperature 57.3–61.7 °C. Pi 4 CPU was 13.37% of one core over its observed
589.71 s interval, RSS 4.04 MiB and temperature 50.1–54.5 °C. Peer observation
began about 14 s after process start. These values describe this stereo delay
configuration, not a full rack, maximum channel count or full-show guarantee.


## Historical large-buffer physical loopback

After the final soak, the user reported cables from both main outputs to the
corresponding inputs, input gain at minimum, Mixer fully Playback and Main at
noon. H3 separately reserved this changed condition. The host did not alter those
physical controls. The AudioBox inputs are mic/instrument inputs; its
[manual, pages 3–4](https://pae-web.presonusmusic.com/downloads/products/pdf/AudioBoxUSB96_OwnersManual_EN_29122021.pdf)
warns against direct line-level sources. This user-installed connection was tested
with conservative generated levels, starting at −72 dBFS peak. Captured audio
never feeds playback in these physical trials.

A private generated-only ALSA probe sent independent left/right pseudorandom
bursts for eight seconds at 48 kHz, period 384/buffer 3072, initially −72 dBFS and
then −54 dBFS after reviewing levels. It stops on any ALSA fault or captured
sample above 0.02 FS. Both runs retained 384000 frames without faults. At −54 dBFS,
ADC peaks were −49.01/−67.71 dBFS, well below the stop level. The left output maps
to input1: both separated bursts have correlation 0.799 and the same 2739-frame
(**57.0625 ms**) reference-to-capture offset. This includes 2688 frames of output
prefill, separately issued stream starts, USB transfers and converters. It is a
buffered physical loopback observation, not isolated converter latency or a
calibrated capture-to-speaker measurement. Do not subtract prefill and call the
remainder converter delay. The small/noisy right correlation does not establish
a usable right-channel route or a reliable latency.

The final unmodified PA/FX/REC host then ran generated-only for 12 s with the
same 384/3072/768 configuration. A measured-level gate predicted a peak below
0.037 FS before the −36 dBFS source trial. All 576000 frames, native ADC hashes,
eight stems, dry/DAC replay and journal entries verified exactly; no xruns,
missing/expired wet returns or clipping occurred. Actual ADC peaks were
0.017325/0.000828 FS. Analysis of the stable 997/1499 Hz tones measured left return
gain −0.674 dB and right return gain −69.440 dB relative to each submitted output.
The right path is **68.77 dB below the left** and remains unresolved. Those tone
measurements do not distinguish a cable/connector/input issue from another
analogue route problem; submitted digital channels are both present. The user
suspects a faulty cable and directed further physical acceptance to use channel 1. Continuous
tones alone do not disambiguate delay.

Original raw captures, probe sources/hashes and sample/tone analyses stay private.
The first probe's elapsed field includes file-save time and its startup timestamps
precede the start calls; neither is a hardware timestamp. The second probe fixes
the elapsed measurement and labels call-before timestamps explicitly. Timing
claims above use sample correlation, not those first-run wall-clock fields.

## H4: repaired pacing and minimum-buffer trials

The old alternating short/~16 ms capture service intervals came from eager partial
reads followed by a full-period `avail_min` wait. The repaired transfer loop waits
for the complete remaining block before consuming it, preserves offsets on short
transfers, and stops on a device error or bounded timeout. Typed ALSA handles are
created once; per-second diagnostic records use preallocated typed storage and are
serialized after PCM stops. SHR PA's standalone host received the same pacing fix.

Ring capacity and queued audio are explicit separate values. Capture starts first;
playback starts after the first processed block has been submitted. Zero silent
prefill adds no silence ahead of that block. There is no additional application
playout queue between capture, local DSP and the ALSA write; recorder and network
queues are independent consumers. This does not remove buffering inside ALSA,
the USB driver or the interface.

The host supports periods 48/96/192/384 and capacities 2/3/4/8 periods. H4 tests
start at 48 frames, the current one-packet processing floor; the device advertises
smaller periods, which this host has not validated. Wet admission supports 1–16 ms
in 1 ms steps and must be at least one capture period. Both transport endpoints
must use the updated validator. The test effect's intentional 20 ms delay is
separate from this admission allowance and from local dry latency.

Physical H4 trials emit a deterministic −54 dBFS coded signal on channel 1 only,
with alternating quiet and active seconds. ADC data never feeds playback. The
right physical route remains unresolved. Both digital channels still traverse the
modules and are recorded; a silent right output is not a right-channel hardware
acceptance result.

Ordinary scheduling with a 48-frame period and 96-frame ring failed both with one
silent prefill period (528 retained frames) and with zero prefill (816 frames).
The zero-prefill run had 1.401 ms maximum playback wait despite 0.105 ms maximum
render time. Increasing capacity to 144 frames while retaining zero prefill first
passed 8 s: all 384000 frames and actual PA/FX/DAC replay matched, no xruns or wet
loss. Two trusted physical correlation windows measured 249–257 frames / 5.19–5.35 ms;
a third one-second window had weak correlation. A following 30 s attempt stopped
after 108672 frames: capture xrun, 2.333 ms maximum render versus 0.113 ms p99,
and only 0.034 ms maximum write wait. The incomplete recording remains exact.
These observations motivate a scheduling comparison, without proving preemption
as the sole cause.

`audio_fifo_priority: 20` explicitly applies FIFO scheduling only to the audio
thread, after network/recorder workers exist and before PCM starts. Missing/null
preserves ordinary scheduling. Other priorities are rejected. The host restores
the saved policy after stopping PCM and before joining workers; activation and
restoration failures are reported as faults. No process-wide, service or kernel
scheduling settings are changed.

FIFO alone did not meet the low-latency reliability gate. At 48/144 frames and
zero prefill, 30 s runs with 1 ms and 2 ms wet allowance missed two and four wet
returns respectively. The latter also changed physical offset from 257 to
364–365 frames around 18–19 s despite zero reported xruns. A subsequent 4 ms
wet-allowance run failed playback after 317904 fully written frames. Adding one
silent period passed 8 s but failed capture after 1351152 frames in the longer
trial. Those failures remain retained; exact software stems alone do not prove
uninterrupted physical output.

## H5: continuous physical reference and thread diagnostics

Candidate v12 retains the repaired transfer path and uses `left_continuous`: one
quiet startup second followed by deterministic −54 dBFS channel-1 noise. The ADC
never feeds playback, and the right digital output is zero. Offline analysis
compares recorded intended DAC channel 1 with its physical ADC return in 100 ms
windows every 50 ms throughout the take. Correlation below 0.6 is flagged for
review. The first quiet second and final 85–135 ms are outside that analysis.
Correlation measures frame offset, not converter clock lock or calibrated
capture-to-speaker latency; small shifts can weaken windows and require inspection.

Preallocated diagnostics retain the most recent 4096 blocks and first 64 timing
outliers, with overflow counts. Read/render/write wall time and thread CPU time
are separate, including failed reads, inter-block gaps and quiet shutdown.
PCM availability/delay is a driver observation. A channel-1 input peak over 0.02 FS
stops the generated probe and preserves its exact S32 alarm block, including an
alarm during shutdown outside the normal eight recorded stems. No level alarm
occurred in the reported successful trials.

`audio_cpu` optionally restricts only the audio thread to one CPU already in its
allowed mask. The H5 comparison chose CPU3; network and recorder workers retained
their original masks. CPU affinity and FIFO20 are saved and restored before
worker joins. Activation, rollback and restoration have explicit error reports;
failure cannot masquerade as a successful restore. IRQ affinity, other processes,
kernel settings and persistent services are unchanged.

At 48-frame periods, 144-frame capacity, zero silent prefill and 4 ms wet allowance,
both unrestricted and CPU3 runs passed 30 s with exact stored samples and no xruns
or missing wet packets. The unrestricted physical offset was 249–257 frames
(5.1875–5.3542 ms), with four weak windows at small transitions. CPU3 had three
weak startup windows; all 497 windows after 5 s measured exactly 257 frames.
These short passes did not establish reliability: a CPU3 packet-fault trial later
failed capture after 294912 frames. Its failed read took 3.558 ms wall time but
only 50.778 µs thread CPU; render maximum was 0.164 ms. The incomplete take remains
sample-exact. This points to delayed capture service rather than expensive DSP,
without identifying the kernel/USB cause.

The CPU3 two-period retry failed after 720 frames. Playback writes took up to
1.401 ms wall time but about 6.5 µs thread CPU, while capture backlog grew to the
96-frame capacity. This configuration fails with the current synchronous transfer
path. It does not establish the smallest buffer this interface could support
with a different host or driver design.

Four-period capacity (192 frames) retains the 48-frame processing block and zero
silent prefill. Its 16 s packet/Brain-stall trial and 16 s Brain restart passed
without USB xruns or recorder/queue faults. All raw hashes, eight stems, dry replay
and journals matched. The deliberate 20 ms loss faded wet output to zero over
240 frames; recorded residual error was at most 0.621 PCM24 LSB. Wet audio recovered
in both trials, and Brain restart required two acknowledged snapshots. Physical
offset stayed at 249 frames (5.1875 ms) in every analyzed window of both trials.
The seven network errors during termination/restart are retained as expected
fault observations, not erased from the report.

A deliberate 100 ms audio-thread stall stopped with one xrun and an incomplete
96000-frame take; all retained samples and journal entries verified. A fresh
epoch and directory then passed 30 s, with no xruns/wet losses and all 577 physical
windows at 249 frames. The four-period capacity did not add a period to measured
latency in these comparisons. This is spare ring capacity, not four periods of
deliberately queued silence.

The subsequent 600 s attempt failed after **36.914 s**, with 1771872 fully written
and 1771920 captured/recorded frames. Playback exhausted while a render took
3.597 ms wall time but only 0.191 ms thread CPU. There were no missing wet packets
or recorder drops. Every retained PCM hash, dry/DAC reference and journal entry
verified; the take is explicitly incomplete and the last prepared block is not
claimed delivered. All 715 analyzed physical windows before the excluded tail
were at 249 frames. This is a failed reliability gate despite the stable measured
latency before the fault.

H6 adds calling-thread fault and context-switch counters around render, and a
reviewed optional process-local memory-lock comparison. These distinguish paging,
blocking and preemption more directly; locking is not assumed to fix scheduling
delays. H6 changed no global CPU, IRQ, governor, memory or service setting.

## H6: a traced memory-migration stall

The instrumented 48/192/zero-prefill baseline passed 30 s at 5.1875 ms, with 22
minor faults and two voluntary switches inside observed render intervals.
Optional `audio_memory_lock: true` uses `mlockall` only in the fresh owned CLI
process, after buffers/workers exist and before FIFO/PCM. It refuses preexisting
current or future locking, changes no memory limit, and verifies restoration
after PCM stops. Normal tests use mock locking backends. Missing/false leaves
memory policy unchanged. The loaded modules must share exclusive ownership of
memory-lock APIs with this host. Recorded counter totals cover valid render
sample pairs; nonzero `usage_errors` means incomplete observation.

Locking 145008 KiB removed most startup faults but did not fix reliability. The
first locked trial stopped after 220272 fully written frames. A following
30 s diagnostic, with perf attached only to the owned audio TID, failed after
867744 fully written / 867792 recorded frames. The failing render took 6.468 ms
wall time and 0.276 ms thread CPU, with one minor fault and one voluntary switch.
The kernel trace showed **6.219611 ms in `migration_entry_wait_on_locked`** during
a data access through SHR PA's `memset` linkage entry. Its instruction is a GOT
load; this is not a measurement of slow `memset` computation. The fault path's
`do_swap_page` name also covers migration entries and does not establish disk
swap-in. The trace includes diagnostic overhead and is retained separately.
Both failed takes verified exactly, and memory locking returned to zero.

The running kernel allows compaction of locked pages
(`compact_unevictable_allowed=1`). Linux documents that such migration can block
tasks on minor faults even when their memory is locked. Ordinary locking and
pre-touching do not prevent it. This identifies a specific next comparison;
the traced migration alone does not identify its initiator. See the
[kernel setting](https://docs.kernel.org/admin-guide/sysctl/vm.html#compact-unevictable-allowed)
and [locked-page migration](https://docs.kernel.org/mm/unevictable-lru.html#migrating-mlocked-pages).

H7 received a fresh, mutually accepted reservation for changing only that key
from 1 to 0
during each trial, with process memory locking enabled and identical audio
settings. It first checks that other processes have no locked memory. A separate
privileged helper owns the key; audio still runs as the ordinary user. Pipe close,
parent exit, SIGINT/SIGTERM/SIGHUP or the bounded deadline triggers restoration.
The handlers do not raise; a private signal pipe wakes the wait, and handled
signals are blocked during restoration. Fake-key lifecycle tests covered pipe
closure, timeout, SIGTERM, SIGHUP and repeated signals during restoration.
SIGKILL, helper OOM death and host failure require coordinator recovery.
Different observed external values are preserved. No persistent configuration,
IRQ, governor, service, NIC, clock, Bluetooth or TV-audio change is part of this
comparison. The setting does not prevent every possible migration source.

## H7: low-latency comparison and recovery

The first 30 s trial with locked-page compaction disabled passed 1.44 million
frames, zero xruns, zero missing wet returns and exact stored samples. Every one
of 577 analyzed channel-1 windows measured **249 frames / 5.1875 ms**. Render
p99/max was 0.138/0.203 ms and full post-read service p99/max 0.171/0.254 ms.
Observed render intervals had no minor/major faults or context switches.

A separate 144-frame-capacity comparison also passed 30 s of software checks.
Its physical offset changed from 249 to 257 frames during startup, with three
weak windows; all 497 windows after 5 s measured 257 frames / 5.3542 ms. The
192-frame ring is selected because it measured lower, stable delay while keeping
the same 48-frame processing block and zero silence prefill. Capacity is not
queued occupancy. No additional application playback queue is present.

| Selected H7 setting | Value |
| --- | --- |
| Physical I/O | AudioBox USB 96, 48 kHz, stereo S32_LE with upper-24-bit capture |
| Processing period | 48 frames / 1 ms |
| ALSA ring capacity | 192 frames / 4 ms, independently configured |
| Silent prefill | 0 frames; start playback after the first processed block |
| Dry application playout queue | None |
| Wet return admission | 192 frames / 4 ms; does not delay the dry path |
| Test effect | Stereo f64 delay, intentional first echo at 960 frames / 20 ms |
| Audio-thread scheduling | FIFO20, CPU3; saved state restored after PCM stops |
| Process memory | Locked during streaming; 145008 KiB observed, then restored to zero |
| Temporary kernel comparison | `compact_unevictable_allowed=0` during each trial, original 1 restored afterward |
| Physical reference | Channel 1 only, −54 dBFS generated signal; right digital output zero |

The 16 s packet/stall and 16 s Brain restart trials each retained 768000 frames
with exact ADC/stem hashes, dry replay and journals, and no USB xrun or recording
queue drop. Packet faults caused 296 missing and 260 expired wet packets; restart
caused 1141 missing, one expired, seven socket errors and two fresh snapshots.
The deliberate 20 ms loss faded wet output to zero over 240 frames, with maximum
error 0.621 PCM24 LSB against the quantized previous sample. Wet output recovered
in the final 9600 frames of both takes. All 297 physical windows in each trial
remained at 249 frames.

A deliberate 100 ms audio-thread stall stopped after 96000 frames with one xrun
and an explicitly incomplete take. Its retained ADC/stem/dry/journal checks were
exact. A new process, epoch and take then passed 30 s with zero losses/xruns and
all 577 physical windows at 249 frames. Each completed trial confirmed applied
memory locking, final locked memory zero, helper exit zero and kernel key 1.
The 600 s run completed **28.8 million frames with zero USB xruns or queue drops**.
All stored PCM/ADC hashes, dry replay and journal entries were exact. All 11977
physical windows measured 249 frames / 5.1875 ms, with no weak windows or detected
persistent offset steps. The first quiet second and last 100 ms remain outside
the correlation coverage. Render p99/max was 0.134/0.299 ms; full post-read service
p99/max 0.168/0.393 ms. All observed render fault and context-switch totals were
zero, with zero observation errors. The earlier multi-millisecond render stalls
did not recur under this condition.

However, **the 4 ms wet-return gate failed**: two returns arrived after admission,
although all 600000 were received. RTT p99/max was 0.428/4.156 ms; Brain FX compute
max was 0.077 ms. There were 189 intended-DAC sample differences from uninterrupted
FX replay, consistent with the retained deadline-loss evidence. This is not a
zero-loss integrated pass, nor a trace identifying which network worker stalled.
The kernel key and memory lock were restored. H8 uses a separately acknowledged
reservation for the same direct audio configuration with only wet admission increased to 288 frames / 6 ms, about
1.84 ms above this observed RTT maximum. It adds no dry-path queue or silence.
Fresh reservation and short recovery checks precede another bounded soak.

## H8: revised wet deadline at the same device latency

H8 received a fresh peer/coordinator reservation through 22:30 UTC. Only remote
wet admission changes from 192 to 288 frames (4 to 6 ms); the v13 executable,
owner libraries, direct device path, 48/192 frames and zero prefill are unchanged.
All temporary memory/scheduling conditions and restoration gates remain those
in the table above. The separate network and recorder queues never hold up dry
processing. ALSA and USB still have their required transfer buffers; this work
does not claim to remove buffering inside the driver or interface.

The 30 s comparison and fresh 30 s recovery each passed 1.44 million frames,
zero xruns/missing returns and exact ADC/stem/dry/DAC/journal checks. Every one
of 577 physical windows in each trial measured 249 frames / 5.1875 ms.

The repeated 16 s packet/stall and Brain-restart trials each passed 768000 frames
with no USB xrun, exact stored/dry samples and all 297 physical windows at 249.
Injected faults caused 290 missing/254 expired returns; restart caused 1151
missing, zero expired, seven socket errors and two snapshots. The 240-frame wet
fade again reached zero with maximum quantized-reference error 0.621 LSB, and
wet output recovered through the final 9600 frames of both recordings.

The intentional device stall again produced one xrun, exactly 96000 retained
frames and an incomplete take. Its stored samples and 17 pre-fault physical
windows verified. The analysis summary correctly requests review whenever a
host fault exists; a private orchestration assertion initially treated that
expected flag as a failed physical check. Inspection confirmed the expected
fault and exact evidence before fresh recovery. The original log is retained;
no measured code or recording was changed to make the check pass.

The H8 **600 s soak completed 28.8 million frames with zero xruns, missing or
expired wet returns, network errors or queue drops**. All 600000 returns arrived.
The eight PCM hashes, direct upper-24-bit ADC hashes, independent dry and combined
output replay, and every journal frame matched exactly. The take finalized
complete. Render p99/max was 0.135/0.290 ms; full post-read service 0.168/0.385 ms.
RTT p99/max was 0.426/3.732 ms, and Brain FX compute 0.018/0.085 ms. Render usage
observations counted no minor/major faults or context switches, with zero errors.

Physical timing has a retained qualification. Of 11977 correlation windows,
11975 were trusted at 249–251 frames (**5.1875–5.2292 ms**); two 100 ms windows
were weak. A separate 10 ms analysis of both neighborhoods found one-frame
changes around 513.71 s and 520.30 s. The first transition still had one weak
10 ms window; the second was trusted. No persistent change of 24 frames or more
was detected. The analyzer's `requires_review` flag remains true, and its original
thresholds/results are unchanged. This establishes the reported latency range,
not perfectly fixed physical offset, converter clock lock or absence of sample
slips. The cause of those two small changes remains unresolved. H7's preceding
ten-minute physical result stayed at 249 frames but failed its 4 ms wet deadline;
these are separate observations, not interchangeable passes.

All H8 trials confirmed applied memory locking, final locked memory zero, saved
thread settings restored, helper exit zero and original kernel key 1. Final
read-only checks on both Pis found task audio/perf/guard processes stopped,
reserved ports and the selected PCM free, Bluetooth disconnected and both nodes
reachable. H8 resources were released before expiry. No persistent system
profile was installed. The demonstrated scope is this bounded stereo bench;
the qualified physical result and heavier-load limits remain explicit. Final
peer review independently matched 65 canonical metadata hashes, confirmed the
bounded USB/software/wet result and retained the unresolved physical-confidence
gate. Original PCM replay was performed by the coordinator; the peer reviewed
its hash-bound results without transferring recordings.

## Build, test and evidence

Build each owner independently with Rust 1.97.1 and its committed lockfile, then:

```sh
CARGO_INCREMENTAL=0 cargo build --locked --release --features hardware-host --bin gigpies-hardware
target/release/gigpies-hardware --help
# Only during an explicitly authorized device/link reservation:
target/release/gigpies-hardware brain /private/brain-config.json
target/release/gigpies-hardware pa /private/pa-config.json
```

The JSON schemas are `PaConfig` in `src/host/device.rs` and `Brain` in the binary;
unknown fields fail. Explicit paths/endpoints, bounded duration, fresh epoch and
create-only report/take destinations are required. `stall_device_after_frames`
is an explicit bench fault injection; leave it null for ordinary measurements.
The private supervisor records exact commands/configurations and bounds each
process; it is not installed as a service or included in source publication.

The final normal suites passed: GigPies 209 Rust/37 Python, SHR PA 67 Rust/4
Python, SHR FX 101 Rust, and SHR REC 23 Rust tests. Four GigPies historical/media
or socket-capacity opt-ins were intentionally skipped because their protected
paths were unchanged; the directly affected FX cost matrix ran separately.
Formatting, warning-denied Clippy, locked release builds and publication checks
passed. The final ADC endpoint counter also ignores unused low container bits
for both signs, with focused endpoint tests. Earlier quiet captures were far from
full scale. A focused regression also rejects a fresh, non-late packet carrying the wrong
return budget. The low-latency changes add transfer-progress, independent prefill,
continuous probe, bounded diagnostics, and thread scheduling/restoration tests.
Each measured binary has its own retained source manifest.

Normal tests cover wet routing/loss, PCM precision descriptors, bounded metrics
and all preceding transport contracts without devices. Owner suites cover ABI
capacity/precision, allocation freedom, resets, recorder gaps, disk failure,
partial publication and recovery after killing an owned child. Historical music,
auditions, exhaustive cost matrices and unrelated media studies remain opt-in.

Private evidence: `artifacts/audio-hardware/2026-10-03/`, including candidate
source/binary manifests, original captures, failed xrun take, trial commands,
opened ALSA parameters, sample verification, fault assessment and peer records.
No recordings or machine configuration enter source Git. No public push/release.

## Remaining acceptance

The left electrical return is measured at 5.19–5.23 ms under the H8 profile,
with the two small physical offset changes above still unresolved. The
right return remains about 69 dB weaker and needs a working physical route before
stereo channel/latency acceptance. Isolated converter latency and clock lock remain
unmeasured. The absent second interface prevents independent-device clock measurements. Calibrated microphone/speaker measurement and acoustic PA
acceptance remain separate. Full channel count, 18/64-channel recording, mixer
automation, live control parameter adapters, UI, analysis fanout, device hotplug
recovery and full-show reliability are not certified by this stereo bench.

## Multichannel adapter and qualification boundary

`host::duplex::Duplex` is an explicit same-raw-device multichannel adapter for the
same LocalAudio production composition used by synthetic tests. It does not
select or activate a device automatically. A SHA-bound manual DeviceAcceptance
record is required before opening; manufacturer-reference profiles refuse.
Capture drives frames; ALSA short-transfer/deadline faults quiesce the provider.
Logical output assignments can change without changing the observed device map;
physical-map changes require a verified reopen. The controller-side synchronous
pump is not a qualified realtime callback/deadline implementation.

The UMC1820/ADA8200 16-input/18-output rig is not hardware-qualified by this
increment. The exact mapping, bidirectional ADAT, soundcard-master relationship,
physical lock and bounded safe acceptance procedure are documented in
[Modular processing](../reference/MODULAR_PROCESSING.md#mapping-and-physical-acceptance).
