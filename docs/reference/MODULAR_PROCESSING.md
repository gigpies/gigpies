# Configurable processing composition

The configurable path composes the existing channel Mixer and Authority with the
loaded SHR PA, FX and REC owners. `EngineTopology` separates logical strip IDs,
transport slots, physical port labels, monitor buses, PA ports and output patches.
The legacy eight-input/two-monitor profile remains an explicit compatibility
identity. Dimensions alone do not select the legacy protocol.

The executable synthetic host and explicit raw ALSA duplex adapter both feed
`LocalAudio::tick_with_capture`. Capture slots are translated to contiguous raw
strips before recording, analysis and channel DSP. Musical four-band EQ and
compression stay in GigPies; speaker EQ/crossover/matrix/delay/protection stay in
SHR PA. Monitor buses are independently configured. The final patch writes each
physical output once; unassigned slots are silent. Measurement slots do not
implicitly become program strips.

## Mapping and physical acceptance

`reference_16_18` describes the manufacturer's expected 48 kHz transport layout,
not an observed Linux device. Logical analog inputs 1–8 use zero-based capture
slots 0–7; logical analog inputs 9–16 use slots 10–17. Logical analog outputs 1–10
use playback slots 0–9, and outputs 11–18 use slots 12–19. S/PDIF slots are separate.
Playback slots 0/1 are MAIN OUT L/R; slots 2–9 are LINE OUT 3–10.
The ADA8200 supplies the eight analog sockets at playback slots 12–19.
Headphone mirrors add no independently addressable output. Every reference output
starts unpatched. Six PA outputs, eight PA outputs, monitors and spare outputs
are editable signal assignments, not permanent socket classes.

The expected reference map is explicit about direction and numbering:

| Direction | Logical analog IDs | Physical sockets | Zero-based USB slots | Manufacturer channel numbers |
|---|---|---|---|---|
| Capture | Inputs 1–8 | UMC1820 analog inputs 1–8 | 0–7 | 1–8 |
| Capture | Inputs 9–16 | ADA8200 inputs 1–8 | 10–17 | 11–18 |
| Playback | Outputs 1–2 | UMC1820 MAIN OUT L/R | 0–1 | 1–2 |
| Playback | Outputs 3–10 | UMC1820 LINE OUT 3–10 | 2–9 | 3–10 |
| Playback | Outputs 11–18 | ADA8200 outputs 1–8 | 12–19 | 13–20 |

S/PDIF occupies capture slots 8–9 and playback slots 10–11; these are excluded
from this analog profile. Every playback row starts with no assigned signal.
This table is a manufacturer reference awaiting actual Linux/socket verification.

A different observed USB order changes the physical configuration, not channel
DSP. Runtime output patch commands change sources on existing stable ports;
changing physical identities/slots requires a quiesced, reviewed topology reopen.
The raw device adapter requires `operator-verified` mapping plus a
`DeviceAcceptance` record binding the exact topology SHA256, raw device name,
observed upper-24-bit S32LE format, socket test record and single-clock setup record.
It checks both ALSA directions resolve to the same card/device and refuses changed
negotiated dimensions. Each pump also checks the active provider against the
accepted physical map and PA inventory, refusing direct Main routes that bypass
configured PA protection. This manual evidence does not manufacture readable ADAT
lock telemetry. No hardware was activated to implement this adapter.

Before physical use, obtain session authorization, disconnect or safely mute
amplification, inventory the actual UMC1820/ADA8200 and negotiated channels, and
verify one physical input/output at a time with unique low-level patterns and
loopback/measurement evidence. Record socket-to-slot direction/index, S/PDIF gaps
and headphone mirrors. The [UMC1820 manufacturer guide](https://www.bhphotovideo.com/lit_files/155647.pdf)
distinguishes MAIN OUT L/R from the direct computer LINE OUT 3–10. MAIN outputs
follow the hardware MONITORING mix and MAIN OUT level/DIM/MUTE controls. Before
using them for software-processed audio, verify and record MONITORING at PB 1–2
(playback only), with no direct-input contribution that bypasses software mute or
PA protection. Verify the physical controls and resulting signal path in the socket
test record; software cannot observe their positions. Use the UMC soundcard clock
as reference, with ADA8200
following that clock over the required ADAT connection and return audio over the
opposite connection. Inspect the physical selector and lock indication; equal
nominal rates do not prove lock. Record the verified relationship, no clock loop,
and the actual mode. Changing optical mode can reset the interface.

Then test deliberate controlled lock loss/device reopen with safe outputs, fresh
epoch and map generation, finalized/incomplete recording status, rejection of old
commands/wet packets, and explicit rearm. Qualify the configured graph's sustained
48 kHz deadline separately. Software tests do not establish converter lock, socket
mapping, acoustic protection or 48-input physical throughput.

## Structural authority

`GP14-structure:1` shares C-AUDIO's show/epoch/writer/lease/request/revision history.
`structural_snapshot` is observational. `pa_set`, `output_mute` and `output_rearm`
require `pa_configuration`; `output_patch` requires `output_routes`. A channel
lease grants neither. Writes need a structural snapshot at most 250 ms old on
that connection. Pending and final replies identify the actual sample boundary.
Retries bind the complete body; revoked writers cannot leave prepared actions
for a later writer to execute.

`pa_set` contains the owner's exact versioned JSON as `configuration_json`, plus
`program_buses`: 0/1 are FOH left/right after dry+wet summation, 2 and above are
monitor buses. The opaque JSON string preserves owner floating-point units without
weakening the integer command grammar. Owner validation and buffer preparation
happen before commit. PA outputs must match the admitted module ports. All scope,
revision and lease checks repeat at the application boundary.

Mute first, wait until quiesced, prepare/commit, inspect final readback, then rearm
explicitly. Failed preparation leaves the old configuration and revision intact.
Output patch commits swap a validated prepared topology and retain old storage
until controller retirement. PA commits retain both retired owner state and host
buffers. Owner output ramps cover wet tails after protection and delay. Recovery
never replays grants or unmutes. Restored intent includes topology, channel targets,
four-band configuration, opaque PA configuration and bus map; runtime epochs,
credentials, grants and pending commands are not persisted.

## Clock and controller boundary

Capture supplies source epoch/frame progression. Brain consumes indexed sends
and returns samples for negotiated deadlines; packet arrival cannot advance the
render cursor. Algorithmic FX delay and transport delay are distinct. Clock
status exposes nominal rate, source epoch/frame, continuity, mapping evidence and
ADAT lock. Unknown physical lock remains unknown. Fault recovery durably reserves
a new epoch, increments map generation and leaves outputs disarmed.

`Mixer::process_interleaved` and `ModuleGraph::process_interleaved`, including
owner DSP, are the bounded render sections. Prepared swaps have separate allocation
and retirement guards. LocalAudio and the synchronous ALSA pump are controller-side
orchestration: they parse commands, manage snapshots, perform I/O and allocate
outside those render sections. They are not registered realtime callbacks, and
the complete pump has no qualified hardware deadline claim. A future callback
host must retain this control/render separation and measure its scheduling budget.

See [acceptance](../acceptance/MODULAR_ENGINE_ACCEPTANCE.md) for results and remaining gates,
[PA ABI v2](../../../shr-pa/docs/EMBEDDING_V2.md) for the owner schema and exact limits,
and [authenticated transport](REMOTE_PROCESSING.md) for Brain control/media.

## Authenticated physical source composition

The raw duplex adapter also drives `HostAuthority::process_source` through
`pump_authority_with`, preserving the same capture, format conversion, protected
physical map and fault handling. Each captured 48-frame block services pending
control with a fresh monotonic timestamp before the authenticated Brain bridges
and shared graph run. Timer polling and network arrival never advance physical
source frames. Capture and playback must resolve to the same card/device/subdevice.

The bounded authenticated executable exposes this adapter through an explicit
`physical_device` configuration plus `--activate-physical`; see
[Brain operation](BRAIN_AUDIO.md). A shared durable epoch ledger prevents reuse
across process restarts and excludes competing owners. Outputs start closed.
Transfer, mapping or rendering failure quiesces the source and stops both PCM
streams. Reopening requires a fresh identity and explicit output rearm.

Unsubmitted playback expires one configured period after capture completes,
including conversion, control and rendering time. The transfer checks again after
availability queries and before every write or partial-write retry. Expired data
is discarded through the fault path. The current authenticated runner uses a
48-frame period, so this admission budget is 1 ms; it is conservative and may
fail under load. Capture has a two-second controller I/O timeout, and an ALSA wait
may detect an expired deadline later. Neither bound qualifies physical scheduling
or acoustic closure. Already submitted PCM has its separate device-buffer tail.

## Output allocation examples

The mapping regression constructs both six PA ports plus twelve monitor outputs,
and eight PA ports plus ten monitor outputs, through the 18 analog sockets of the
reference transport. It deliberately interleaves signal roles and permutes sockets,
checking every analog output while S/PDIF remains silent. These are editable
examples: leaving any subset unassigned is valid, monitor counts are independently
admitted, and physical use still requires observed mapping acceptance. Monitor
buses patched directly are monitor buses, not a claim that SHR PA speaker
protection has been applied; route a monitor through an owner program/output when
that protection is required. Physical mains configured with PA cannot bypass it
through a direct Main patch.

## Automatic feedback notch

In SHR Desk, open the F9 PA editor and select a speaker output or Bypass in the first field. The readback shows the applied frequency.

For direct structural control with a feedback-capable SHR PA library, set `"feedback_output": 0` in the PA
graph for the first speaker output (1 selects the second). Use the existing
`pa_set` operation and explicit output rearm. In `structural_snapshot.pa_status`,
`feedback_output_plus_one` identifies the selected output, or zero for bypass.
`feedback_hz` shows the applied cut frequency, or zero while waiting. To bypass
and clear the cut, omit `feedback_output` and reapply the graph. This structural
change briefly mutes the PA. Existing graphs default to bypass.
