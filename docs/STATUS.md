# What works today

[Using GigPies](README.md) · [Engineering evidence](acceptance/README.md)

**Experimental software, not yet a fully qualified live-show system.**

| Area | Available | Still unfinished or unverified |
|---|---|---|
| Audio desk | Channel EQ/compression, default-bypassed gate (processing v5), recorded-WAV preparation controls, evidence review and actual v5 Desk mutation acceptance, fader/pan, meters, monitor sends, spectrum and 24-input-bank + Master selection strip | Physical controller bindings and listening acceptance; Master software acceptance is tracked in GP-PANEL |
| PA | Configurable weighted routing, crossovers, input/speaker EQ, compression, sample limiting, independent output delay/polarity and mutes through SHR PA | Physical speaker protection, full output mapping and acoustic alignment |
| Effects | Four stereo FX auxes through Brain, wet returns to Main, palette controls, tail-preserving bypass and panic; [owning GP-FX card](development/MODULE_IMPLEMENTATION_PLAN.md#gp-fx--writable-owner-effects-in-the-console) | Saved-show integration, listening and sustained target-hardware performance |
| Recording | Reviewed Desk Start/Stop, aligned raw tracks, readiness/progress/fault/final outcome readback; standalone SHR REC capture, durable take handoff and playback software acceptance | Physical capture/playback, storage/load and power-loss durability |
| Talkback and listening | Brain audio routing and software recovery | Physical duplex device setup, delay and feedback checks |
| Lighting | Manual looks, cues, scene editing and optional music-following estimates | Physical DMX output and rehearsal accuracy of musical estimates |
| Measurement | Software reference/mic analysis, delay/polarity and crossover-sum proposals, reviewed output EQ and imported calibrated repeated-sweep evidence | Physical synchronized microphone acquisition, automated excitation/capture and measured acoustic verification |
| Offline mixing | Multitrack inspection, prepared renders, optional balance/tone/FX changes and export checks | Musical judgement remains with the listener |
| Performer stations | Design proposal | Not implemented |

## PA processing and setup

The integrated PA module processes the configured speaker paths. Its standalone
application also provides **pink/white noise,
sine, impulse and logarithmic sweep generators**, with level and off/on controls.
Those standalone controls are distinct from GigPies measurement acquisition.

Offline analysis measures magnitude, phase, coherence and arrival, proposes
output delay/polarity and predicts crossover summation. Conservative output-EQ
proposals and calibrated repeated-sweep review are available. Desk can review
software measurement results and imported sweep EQ evidence through explicit
muted configuration transactions. Analysis does not automatically apply a change
or enable outputs.

The missing step is a complete physical measurement session: synchronized
reference/microphone acquisition, controlled excitation and fresh combined-response
verification. Feedback suppression and subharmonic synthesis are also unimplemented.
See [PA capabilities](https://github.com/gigpies/shr-pa/blob/main/docs/STATUS.md),
[standalone operation and signals](https://github.com/gigpies/shr-pa/blob/main/docs/RUNNING.md),
and [Desk setup review](https://github.com/gigpies/shr-desk/blob/main/docs/PA_MEASUREMENT.md).
The next integration steps are tracked in
[GP-PA-MEASURE](development/MODULE_IMPLEMENTATION_PLAN.md#gp-pa-measure--physical-measurement-workflow).

## Hardware scope

The reference rig targets **16 analog inputs and 18 outputs at 48 kHz**.
Larger software configurations have been tested; those are not hardware capacity
promises. Full UMC1820/ADA8200 mapping, clock lock and sustained operation still
need qualification.

A limited stereo USB bench has run PA, FX and recording. Its right return and
small timing discontinuities remain unresolved, so it does not establish full
stereo or multichannel acceptance. Details stay in the
[hardware evidence](acceptance/AUDIO_HARDWARE.md).

## Development

[Current integration work](development/MODULE_IMPLEMENTATION_PLAN.md) ·
[Architecture](architecture/README.md) · [Acceptance records](acceptance/README.md)
