# Using GigPies

[Project home](../README.md) · [Current limits](STATUS.md) · [Engineering](development/README.md)

GigPies is still in development. Use the guides below for the implemented
controls; a complete live-show setup is not yet a finished product.

| I want to… | Open |
|---|---|
| Set up the audio desk | [Open SHR Desk](https://github.com/gigpies/shr-desk/blob/main/docs/TERMINAL.md) |
| EQ or compress a channel | [Channel controls](https://github.com/gigpies/shr-desk/blob/main/docs/USING.md#channel-eq-and-compression) |
| Check levels or inspect a spectrum | [Meters and spectrum](https://github.com/gigpies/shr-desk/blob/main/docs/USING.md#meters-and-spectrum) |
| Adjust a monitor mix | [Monitor sends](https://github.com/gigpies/shr-desk/blob/main/docs/USING.md#monitor-sends) |
| Adjust the main PA EQ | [Master EQ](https://github.com/gigpies/shr-desk/blob/main/docs/USING.md#master-eq) |
| Set up the standalone PA processor | [PA guide](https://github.com/gigpies/shr-pa/blob/main/docs/RUNNING.md) |
| Adjust GigPies effects | [Desk FX controls](https://github.com/gigpies/shr-desk/blob/main/docs/USING.md#effects) |
| Use the separate two-engine effects rack | [SHR FX guide](https://github.com/gigpies/shr-fx/blob/main/docs/OPERATING_GUIDE.md) |
| Build lighting looks and cues | [Lightdesk guide](https://github.com/gigpies/shr-lightdesk/blob/main/docs/USING.md) |
| Preview music-following lighting | [Lux previews](https://github.com/gigpies/shr-lux/blob/main/docs/USING.md) |
| Work on recorded multitracks | [Offline mixing](guides/README.md) |

## Controls and computer keyboards

The intended live desks use their assigned MIDI controllers: knobs, pads and
piano keys for selection. A computer keyboard is an optional fallback, not a
requirement for live operation. Shortcut lists describe the current software
interface; they do not redefine that intended workflow.

Both consoles provide labelled semantic menus and value entry without synthesizing
computer keypresses. Physical MIDI acquisition and learned device mappings remain
unfinished. Complete controller-only operation on the actual rig is an
open [console acceptance requirement](development/MODULE_IMPLEMENTATION_PLAN.md#gp-h2--physical-dual-console-operation),
not an already delivered capability.

## Before a session

A configured live system needs its audio connections, output assignments and
control permissions prepared. Connecting a desk starts read-only; it does not
restore a mix or enable outputs automatically. Service configuration currently
requires the [engineering setup](development/README.md#system-setup).

On a configured desk, check the selected channel/output and current connection
state before reviewing a change. Missing or stale meters are not silence.

## Recording

The integrated engine can write aligned raw tracks. **Desk has no Record/Stop
controls yet**, and the standalone REC application cannot record or play.
Recording currently needs an explicitly configured engine session; see
[recording integration](reference/MODULAR_PROCESSING.md) and
[REC's current scope](https://github.com/gigpies/shr-rec/blob/main/docs/LOCAL_SETUP.md).

## If something stops responding

Cancel an unsubmitted edit, check the connection, then reconnect and review the
current state before trying again. A lost reply does not prove that a submitted
change failed. Reconnecting does not automatically replay it.

The desks control separate processing engines. Closing a desk is not a reliable
way to stop audio or recording.
