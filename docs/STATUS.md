# What works today

[Using GigPies](README.md) · [Engineering evidence](acceptance/README.md)

**Experimental software, not yet a fully qualified live-show system.**

| Area | Available | Still unfinished or unverified |
|---|---|---|
| Audio desk | Channel EQ/compression, fader/pan, meters, monitor sends and spectrum | Physical display/controller setup and listening acceptance |
| PA | Configurable routing, output controls and master EQ | Speaker protection and acoustic alignment in a real system |
| Effects | Integrated stereo effects, tail-preserving bypass and panic | Listening and sustained target-hardware performance |
| Recording | Aligned raw tracks in the engine | Desk Record/Stop controls and standalone recorder playback |
| Talkback and listening | Brain audio routing and software recovery | Physical duplex device setup, delay and feedback checks |
| Lighting | Manual looks, cues, scene editing and optional music-following estimates | Physical DMX output and rehearsal accuracy of musical estimates |
| Measurement | Software capture, distance-aware alignment and reviewed output-EQ proposals | Actual microphone/acoustic acceptance |
| Offline mixing | Multitrack inspection, prepared renders, optional balance/tone/FX changes and export checks | Musical judgement remains with the listener |
| Performer stations | Design proposal | Not implemented |

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
