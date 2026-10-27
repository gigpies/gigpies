# Local audio and lighting integration

Historical workflow retained for reproduction. For current use, start at
[Using GigPies](../../README.md) and [current status](../../STATUS.md).


This 2026-10-04 milestone connects the real eight-input GigPies mixer, independent
Desk/Lightdesk clients, Lux null-output authority, named analysis and actual owner
REC/FX/PA libraries. Native frontends are software-validated with offscreen CPU
rendering. No physical audio, MIDI, DMX or operator display is opened by the checks
below. The [current implementation map](../plans/MODULE_IMPLEMENTATION_MAP.md#execution-checkpoint--2026-10-04)
records current acceptance; the [prior client-only milestone](../headless-integration-before-modules-2026-10-04.md)
is preserved.

| Owner | Implemented path | Boundary |
|---|---|---|
| GigPies | Eight raw inputs, per-input FOH EQ/dynamics, stereo FOH, two monitor sends, private control, GP09 live roles, GP04 named analysis and GP05 fixed owner graph | Synthetic source; logical main-only PA sample limiter; meters and physical verification unavailable |
| SHR Desk | Real GP03 actions, opt-in GP07 channel editing, native/headless views, protected confirmations and read-only GP05 module status | No recorder writes, writable PA/FX controls or mixer/arbitration implementation |
| SHR Lux | Fixture/programmer/Hold/cue/playback authority, timed release, durable restart and named source automation | Explicit calibration/grants; null/disarmed output; musical beat/downbeat/harmony unavailable |
| SHR Lightdesk | Real Lux actions, native/headless views, complete reviews and read-only LX05 analysis/provenance | Lux remains the owner of output and arbitration |
| SHR REC / FX / PA | Actual independently built owner libraries, retained progress observer and versioned capability/status queries | Fixed existing DSP; recorder durability unknown; acoustic and true-peak protection unavailable |

Services use owned canonical `0700` directories, `0600` sockets, same-user validation,
bounded frames/connections and deadlines. The local binding does not close the
production remote authentication gate. Show/module/epoch, writer/lease, request
identity and revision fence writes. Audio applies commands at 48-frame boundaries
and ramps for 240 frames; monitor sends remain post-mute and pre-FOH-fader.
Admission is separate from rendered or lifecycle completion. Exact retries retain
identity; a reconnected client does not replay uncertain commands as new writes.

## Channel processing

[GP07-processing:2](../../reference/CHANNEL_PROCESSING.md) adds atomic four-band fully parametric EQ and compressor
configuration on the existing audio socket. It shares the FOH lease, revision and
request history with GP03; transport delivery is not application. At 48 kHz, a
prepared edit applies at the next 48-frame boundary and crossfades for 240 frames.
Raw input remains the REC/analysis source. FOH uses EQ → compressor → shared mute
→ fader/pan → sum → fixed FX wet/dry → PA. Monitors remain raw → shared mute → send.

The Desk processing view is opt-in with `--processing`; see the owner’s
`docs/NATIVE_FRONTEND.md` for selection, field editing and explicit apply/cancel.
Bypass, settled/current and committed/target values, readiness and compressor gain
reduction come from the provider. Unknown, bypassed or transitioning GR is unavailable.
Processing does not add a monitor tap selector, automated DSP writes or writable
PA/FX controls.

## Run an explicit local session

Build each owning repository independently with Rust 1.97.1 and its Cargo.lock.
Follow the [one-build-per-host procedure](../plans/PARALLEL_WORK_PLAN.md#one-build-slot-per-host)
before Cargo; use `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=1` and `-j 1`.
The relevant release binaries are `gigpies-headless`, `shr-desk`, `lux-service`
and `shr-lightdesk`. No sibling path dependency or provider source copy is needed.

Create two fresh private directories, one per service, and provide their absolute
canonical paths. The synthetic Lux profile uses the show ID below. In separately
owned terminals, launch:

```sh
gigpies-headless --directory /absolute/private/audio \
  --show 11111111-1111-4111-8111-111111111111 --epoch 1
lux-service --synthetic-private-dir /absolute/private/lighting --durable
```

Both directories must already exist with the stated ownership and mode. Keep
their identity/checkpoint files when restarting. Audio requires an explicitly
increasing epoch; Lux reserves the next epoch itself. A crash may leave the owned
socket path: remove only that known endpoint after confirming its service has
exited. Never remove identity state to bypass a refused restart.

An audio batch script can contain:

```text
grant
input-release
set input-01 fader -3000
wait 50
status
release
```

Run it with a fresh writer identity:

```sh
shr-desk --audio-local /absolute/private/audio/audio.sock \
  11111111-1111-4111-8111-111111111111 1 desk-session-1 foh \
  --script /absolute/private/audio-commands.txt
```

The fader unit is integer milli-dB. A `monitor1` or `monitor2` writer can only
edit its named monitor send. Mode changes and release previews require explicit
confirmation. See [Desk provider instructions](https://github.com/PaolaShultz/shr-desk/blob/main/docs/PROVIDER_CLIENT.md)
for the full bounded script vocabulary and [audio service rules](../../reference/AUDIO_LOCAL_SERVICE.md)
for lifetime, framing, restart and failure behavior.

A lighting batch can contain:

```text
grant
select fixture-11
touch intensity 700
record cue look-1
go look-1 playback-1
touch intensity 0
clearHold
preview intensity
release
release-input
wait 650
refresh
status
checkpoint
quit
```

Run against the service's current epoch:

```sh
shr-lightdesk lux-control --socket-path /absolute/private/lighting/lux.sock \
  --show-id 11111111-1111-4111-8111-111111111111 --epoch 1 \
  --script /absolute/private/lighting-commands.txt
```

Intensity is integer tenths of a percent. This example stores a cue, activates it,
establishes a manual zero, reviews the engine's release and waits for its endpoint.
Checkpointing preserves the current look, master and blackout. Restart restores
holds under a new epoch with no playback, writer grant or in-progress transition;
output stays `null_disarmed`. See the [Lightdesk owning plan](https://github.com/PaolaShultz/shr-lightdesk/blob/main/docs/GIGPIES_IMPLEMENTATION.md)
and [Lux owning plan](https://github.com/PaolaShultz/shr-lux/blob/main/docs/notes/0027-gigpies-implementation.md).

## Named analysis and owner modules

The preceding default audio service keeps optional module health unavailable.
For the full synthetic graph, build `gigpies-headless` with `hardware-host` and
activate explicitly accepted libraries using [MODULE_GRAPH.md](../../reference/MODULE_GRAPH.md).
That feature build does not open a device. Build each library in its owning
repository, review its header/corpus and pin exact library/header/manifest hashes;
private release binaries are not distributed with this source.

```sh
gigpies-headless --directory /absolute/private/audio \
  --show 11111111-1111-4111-8111-111111111111 --epoch 1 \
  --synthetic-source fouraux --analysis --modules /absolute/modules.json
lux-service --synthetic-private-dir /absolute/private/lighting --durable \
  --analysis /absolute/private/audio/analysis.sock
shr-desk --modules-status /absolute/private/audio/audio.sock \
  11111111-1111-4111-8111-111111111111 1
```

Use fresh directories/epochs or follow the restart rules above. The named tap is
raw pre-fader kick/bass/guitar-1/guitar-2 from inputs 01..04 of the same eight-input
source. Analysis attachment does not change source samples. Whole windows carry
oldest acquisition time; age above 100 ms, loss or changed identities requires fresh
attachment/calibration. Audio and recording never wait for an analysis consumer.
See [the exact stream](../../reference/ANALYSIS_STREAM.md) and [Lux's accepted wire](https://github.com/PaolaShultz/shr-lux/blob/main/tests/fixtures/lx05/v1/WIRE.md).

REC start/stop uses the GP05 envelope under the connection's current FOH lease.
The provider chooses create-new private take paths and keeps eight exact raw
PCM24 stems. Preparation/finish run outside processing; admitted recording survives
client/lease loss. Status distinguishes accepted/written counts from unknown
durability. Desk only reads these fields. FX contributes its fixed wet-only delay
once into dry FOH; actual PA receives the sum and exposes its fixed logical outputs.
Physical, acoustic and monitor protection are not implied.

Lux's opt-in `lx05-v1` listener requires explicit soundcheck and a fresh bounded
intensity grant before AUTO can act. Human Hold/programmer values win. Loss freezes
the current contribution and clears authority; reconnect does not rearm. Per-value
provenance stays bound to the original source/window/calibration. Lightdesk displays
that information without implementing another analyzer or arbitration engine.

## Native frontend boundary

Each console's optional `native` feature uses its existing scenes/font/actions,
real provider client and the [GP09 process-held role protocol](../../reference/ROLE_BINDING.md).
Live lease verification fences stale input/output authority; injected inventories
remain assertions, not actual controller/display discovery. See the owning
[Desk native instructions](https://github.com/PaolaShultz/shr-desk/blob/main/docs/NATIVE_FRONTEND.md)
and [Lightdesk development instructions](https://github.com/PaolaShultz/shr-lightdesk/blob/main/docs/DEVELOPMENT.md).
The validated headless/software path does not authorize launching windows or
opening physical controllers. Real HDMI/MIDI/LED acceptance remains separate.

## Evidence and remaining work

| Owner | Complete normal software checks | Additional acceptance |
|---|---|---|
| GigPies | 278 default Rust; 301 with `hardware-host`; 39 Python | GP09 process recovery, exact GP04 PCM/age/loss, four actual GP05 library/socket regressions and release lifecycle |
| SHR Desk | 97 default Rust; 99 with `native`; 9 publication checks | Real provider/role loss, independent module health, protected actions and CPU offscreen frames |
| SHR Lightdesk | 104 default Rust; 105 with `native`; 9 publication checks | Actual Lux/GP09 clients, complete reviews and LX05 analysis/provenance rendering |
| SHR Lux | 103 Rust; 8 terminal checks; 6 simulation checks | Actual GP04 analysis, calibration/AUTO/loss and durable null-output restart |
| SHR REC | 30 Rust | Exact PCM24 C harness, lifecycle/observer behavior and terminal checks |
| SHR FX | 105 Rust | Versioned ABI C harness at six sample rates; fixed-delay behavior |
| SHR PA | 70 Rust; 5 Python; 8 terminal checks | Descriptor/status C harness, null-output and synthetic offline processing |

Formatting, warnings-denied Clippy and release builds passed for the changed runtime
owners, including both frontend feature configurations and the device-free host.
The five optional/reference owners received reviewed documentation only.

The combined actual-executable demonstration passed with the accepted GP09 role
broker, GigPies/Desk/Lux/Lightdesk and all three owner libraries. Desk changed FOH
and its independent monitor send; Lux received and calibrated actual named PCM,
applied a bounded AUTO contribution and preserved a human Hold. Lightdesk completed
a reviewed release and checkpoint. Killing the owned Lux child left audio control
and recording working; restart restored the intended Hold/master/blackout under a
new epoch, disarmed and without replay.

Recorder start/retry/stop/finalization passed. All eight raw PCM24 stems matched
every expected source sample: **303,744 frames per stem**, with equal accepted
and written counts and unknown durability. Actual FX/PA descriptors and library
hashes were checked; separate graph tests verify the wet/dry samples into the PA
reference. Every owned child joined before temporary recordings/endpoints were
removed. This demonstrates bounded software behavior, not physical or load safety.

Normal suites cover production contracts, safety, recovery, schema, routing,
concurrency and rendering. Native and device-free host feature suites passed,
along with warnings-denied Clippy, fmt and release builds. Explicit actual-library,
private-IPC, role-child loss and CPU graphics checks use exact accepted artifacts.
Provider SHA manifests and concise private results retain reproducibility without
publishing executables, recordings, private state or one-off runners.

Historical research media, auditions, exhaustive matrices and long benchmarks
were intentionally skipped. Physical audio/MIDI/DMX/controllers/displays, production
remote authentication and combined-load/scheduler acceptance remain separate.
Temporary synthetic recordings and endpoints are removed after owned children
exit. This is bounded functional software evidence, not full-show hardware safety.
