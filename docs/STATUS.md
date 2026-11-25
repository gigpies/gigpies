# What works today

[Using GigPies](README.md) · [Engineering evidence](acceptance/README.md)

**Experimental software, not yet a fully qualified live-show system.**

| Area | Available | Still unfinished or unverified |
|---|---|---|
| Audio desk | Channel EQ/compression, default-bypassed gate, recorded-WAV gate preparation, main-vocal assist (soundcheck-measured level with a bounded peak cut), fader/pan, measured meters (raw, pre-fader, Main pre-PA, every monitor; checked against the real PA/FX/REC modules), monitor sends, spectrum and 24-input-bank + Master selection strip | Physical controller bindings and listening acceptance, including the vocal assist with a real singer; Master software acceptance is tracked in GP-PANEL |
| PA | Configurable weighted routing, crossovers, input/speaker EQ, compression, sample limiting, independent output delay/polarity and mutes through SHR PA; one automatic feedback notch on a selected output | Physical speaker protection, full output mapping, acoustic alignment and rehearsal tuning of the feedback notch |
| Effects | Four stereo FX auxes through Brain with pre/post-fader sends, wet returns to Main and monitors, palette controls, tail-preserving bypass, panic and saved setups; [GP-FX](development/MODULE_IMPLEMENTATION_PLAN.md#gp-fx--writable-owner-effects-in-the-console) | Listening and sustained target-hardware performance |
| Recording | Reviewed Desk Start/Stop, aligned raw tracks, readiness/progress/fault/final outcome readback; standalone SHR REC capture, durable take handoff and playback software acceptance | Physical capture/playback, storage/load and power-loss durability |
| Talkback and listening | Brain audio routing and software recovery | Physical duplex device setup, delay and feedback checks |
| Lighting | Manual looks, cues, scene editing and optional music-following estimates | Physical DMX output and rehearsal accuracy of musical estimates |
| Measurement | Reference/mic analysis, delay/polarity and crossover-sum proposals, reviewed output EQ, imported calibrated sweeps, and bounded independent-clock USB auto-EQ with live remeasurement | Synchronized physical acquisition, representative-position workflow and full acoustic qualification |
| Offline mixing | Multitrack inspection, prepared renders, optional balance/tone/FX changes and export checks | Musical judgement remains with the listener |
| Controllers | Semantic pad menus and value entry in both desks; read-only MIDI acquisition from a learned profile file, with a learn monitor to capture the real device | Learned MiniLab profile, LEDs, disconnect recovery and two-desk operation on the actual controllers |
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

The E205U/Pi4 bench supports bounded independent-clock broadband acquisition,
conservative automatic cut proposals and production PA apply/remeasure. Continuous
playback uses an explicit very-low-level left pilot. Actual Brain FX wet/dry
relative timing has also been observed through the speaker. This uncalibrated,
single-position path is distinct from the unfinished synchronized reference/mic
workflow and representative-position combined-response qualification. Feedback
suppression and subharmonic synthesis are also unimplemented.
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

An earlier AudioBox stereo bench ran PA, FX and recording. Its right return and
small timing discontinuities remain unresolved, so it does not establish full
stereo or multichannel acceptance. Details stay in the
[hardware evidence](acceptance/AUDIO_HARDWARE.md).

## Development

[Current integration work](development/MODULE_IMPLEMENTATION_PLAN.md) ·
[Architecture](architecture/README.md) · [Acceptance records](acceptance/README.md)
