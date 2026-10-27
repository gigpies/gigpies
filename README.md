# GigPies

![GigPies — audio and lighting terminal desks at a live-band stage](docs/assets/gigpies-hero.png)

*Concept artwork of the intended setup.*

**Mix the band. Run the lights. Keep control.**

**A terminal application (TUI) for live-band sound and lighting on Raspberry Pi.**
Audio and lighting desks, performer monitors, PA processing, effects and multitrack recording.
Manual operation comes first; assistance and automation are optional.

[**Using GigPies**](docs/README.md) · [**What works today**](docs/STATUS.md) · [Engineering](docs/development/README.md)

## The system

| At the desk | On stage |
|---|---|
| Audio and lighting screens, controllers, talkback and operator listening | Audio interface, channel processing, PA outputs, monitors and recording |

The Brain runs the desks; the Stagebox processes the audio. The reference setup
uses 16 analog inputs and 18 outputs at 48 kHz. Routing is configurable.

- **Mix:** channel EQ and compression, monitor sends, master EQ and effects.
- **Light:** build looks and scenes, with optional music-following behavior.
- **Record:** aligned raw tracks through the integrated recorder.
- **Work offline:** inspect multitracks, prepare settings and render mixes.

**Still experimental.** Integrated software checks and a limited stereo hardware
bench are complete. Full multichannel hardware, physical controllers/displays
and lighting output still need qualification. Recording controls are not yet
available from Desk. [Current limits](docs/STATUS.md).

## Channel view — running in a terminal

[![SHR Desk Channel view: four-band EQ, compressor, meters and physical-style controls](docs/assets/panel-desk-channel.png)](docs/assets/panel-desk-channel.png)

Actual SHR Desk terminal capture with deterministic software audio, approved
October 20, 2026. Click for full resolution.

## Start here

For the screens and controls, open [Using GigPies](docs/README.md).
There is no finished one-command live-system installer yet.

To inspect recordings without opening audio hardware:

```sh
cargo build --locked --release
target/release/gigpies inspect /path/to/multitrack-folder
```

Build from this checkout with its pinned Rust toolchain.
[Build instructions](docs/development/DEVELOPMENT.md) · [Offline mixing](docs/guides/README.md)

[Release notes](CHANGELOG.md) · [MIT license](LICENSE) · [Artwork and third-party notices](THIRD_PARTY.md)

[Contribute](https://github.com/gigpies/.github/blob/main/CONTRIBUTING.md) · [Report a bug](https://github.com/gigpies/gigpies/issues/new/choose)
