# Shared instrument panel

The shared package and specimen live in `src/instrument-panel/`; Desk and
Lightdesk expose explicit Ratatui entry points over their existing provider
workers. Providers retain authority. Full physical-control migration is tracked
in [GP-PANEL](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-panel--shared-physical-instrument-panel).

## Three-row channel keybed

Design contract agreed 2026-10-30; shared widget 0.3.2 and Desk adapter implemented.
Physical display/input qualification remains pending. The local user
reference is `artifacts/screenschot-midi-keys-ctrl.png`. This is a compact on-screen
strip that can sit within the console, not a requirement to attach a keyboard.
It shows 24 channel keys plus a separate Master key in exactly three character
rows. Pressing/clicking a key selects its target. Selection never toggles REC,
solo or mute. Physical MIDI mapping is a separate learned binding.

The sketch has channel labels 1–24 represented by repeating units digits,
followed by `M` for Master. Channel 15 is green-selected. Red `R` marks appear
under channels 1–22; these are illustrative states, not a required recording
configuration. Preserve the piano-key silhouette and compact character layout.
The other local image, `artifacts/architecture.png`, is a historical conceptual
architecture sketch; its “planned” labels do not establish current implementation
or hardware acceptance.

| Row | Meaning |
|---|---|
| 1 | Channel identity / Master label, within the keyboard outline |
| 2 | Independent fixed positions `RSM`: recording, solo, mute; simultaneous states remain visible |
| 3 | Key bodies and divisions completing the keyboard silhouette |

Each key needs room for all three status characters, including black keys.
Use green key fill for selection in this widget; keep readable text and a
non-colour selected marker in the label cell for monochrome fallback. Existing
blue interaction focus elsewhere remains unchanged. `M` in row 1 means Master;
`M` in row 2 means mute. Do not multiplex or blink between R/S/M. An inactive
flag is a blank in its reserved position; unavailable/stale state uses `?`,
and a recording fault uses `!` in the R position. Preparing/finalizing uses
`P`/`F` there. These conventions are implemented by the shared character widget.
The existing header/footer supplies the full selected channel number/name,
bank range and detailed state; no fourth keybed row is required.

Reserve three rows above the existing footer/status area in regular Desk views.
Reflow the surrounding layout rather than cover controls. Review/modal views
retain priority and their existing navigation rules: selection cannot silently
retarget an open draft or confirmation. The minimum supported terminal geometry
must fit the strip and existing controls; smaller windows show the existing
resize requirement rather than drop states or truncate keys.

The 24 slots are a display bank, not engine capacity. Map slots to authoritative
stable input IDs. With fewer inputs, unused keys are visibly unavailable. With
more, bank navigation exposes successive groups of 24, with Master anchored at
the right. Full channel identity must disambiguate repeated digits. Use an
explicit input-or-Master selection target; Master is never input 25. Reconcile
the existing 12-channel detail/meter pages with the selected stable ID rather
than globally replacing every bank size with 24.

Project status from provider-confirmed state independently of selection:

- `R` means that this input belongs to the fresh active take's mapping while
  recording. Validate take identity, source epoch and freshness; it is not an
  armed flag or proof of individual-track durability. Existing take counters and
  fault details remain on the Recorder page. Master has no R indication unless
  a separate supported master-recording contract exists.
- `S` means confirmed channel solo under a defined provider solo contract.
  Do not infer it from the monitor source, selected channel or a local action.
  Audit the owning solo implementation before connecting it; missing support
  stays unavailable, not falsely off.
- `M` means confirmed channel/master mute for the corresponding target. Do not
  substitute talkback or monitor mute. Pending commands do not become confirmed
  status; stale or missing evidence does not become an inactive blank.

Status cells are readouts. Existing reviewed actions own recording and other
changes. Selection must preserve provider permissions, generation fences and
edit cancellation/review behavior. Mouse, ordinary keyboard navigation and any
subsequently learned MIDI keys feed the same semantic selection path. No MIDI
anchor note, velocity action, hold action or hardware qualification is inferred
from the drawing. Implementation and acceptance are owned by
[GP-PANEL](../development/MODULE_IMPLEMENTATION_PLAN.md#three-row-keybed-flow-plan).

## Rotary and master-strip flow direction

Operator direction, 2026-10-30; explicit profile/semantic bindings are implemented
in Desk, with physical bindings still unqualified. EQ columns 1–4
retain frequency above gain. With verified Shift held, the upper rotary edits
Q and the lower rotary edits gain in 0.1 dB decoded steps without acceleration.
Shift decoding must cover the actual device's down/up messages and alternate
encoder messages; it is not yet a qualified Desk input path.

The proposed pitch-strip master control requests a gain-change speed, not an
absolute fader position. Center stops movement; either side raises/lowers gain;
return to center stops at the resulting gain. The operator explicitly requires
**a slow maximum speed**, even at full displacement. The provisional software model uses a quadratic curve, 8% dead zone and
maximum 0.5 dB/s, with a 250 ms stale-input stop. The dedicated provider Master gain target and production Desk speed path are
implemented; hands-on feel remains pending. Disconnect/context loss
must stop movement. Return-to-center behavior must be verified for the preset.

Current compressor columns are 5: threshold/attack, 6: ratio/release, and
7: knee/makeup (upper/lower). All six parameters already have ordinary controls;
no additional Shift function is agreed. Column 8 remains pan/level for the
labelled selected target. The pitch-strip proposal independently targets Master.
Processing v5 adds a default-bypassed channel gate; analysis activity gates and
transport fades remain distinct. Implementation and remaining acceptance are in
[GP-GATE](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-gate--lightweight-channel-gates-and-tom-decay-control).
The implemented labelled Gate mode reuses columns 5–7 for threshold/attack,
hold/release and range/hysteresis; compressor controls remain in Compressor mode.

## Rendering contract

The selected Desk Channel has a bounded, opt-in material path described below.
It supersedes the colour-font appearance for this screen only. The following
shared font contract remains the other pages’ renderer and a separate fallback.


Ratatui owns cell layout, widget rendering and incremental terminal updates.
Labels/values remain ordinary text. Original reusable colour glyphs supply
material boundaries, LED lenses, bounded glow and fader parts. No whole-screen
image upload or per-frame glyph generation is used. The current repertoire is 443 tiles,
246,740 base64 payload bytes before protocol headers in experimental protocol mode.
The primary installed-font path uploads no glyphs at startup or during updates.

Supported local presentation: Rio 0.5.28 ARM64/X11 with the generated **SHR Panel
Mono COLRv0** font, 16/20/24 px, default line spacing. Explicit application-local
fontconfig and `SHR_PANEL_FONT='SHR Panel Mono:0.3'` select the tiles. Ordinary
Unicode is the fallback; unsupported terminals must not set that marker.
COLRv1 failed the selected build's colour rendering. Glyph Protocol registration
works, but square fitting breaks tile geometry, so it is experimental only.
Direct Rio is the qualified key-release path. Private tmux is qualified for
rendering/legacy input only: its missing release events cause talkback and
confirmations to be refused. SSH remains unqualified.

COLRv0 layers approximate gradients with bounded bands. Authoring/protocol tiles
use 1000-unit square coordinates; the installed font maps them to 1024-unit
advances and a 2048-unit line box. Ordinary labels use matching solid faces;
no text is baked into tiles. Lit/off LEDs include explicit ON/OFF text. Selected
blue means interaction focus, never proof of an owner ACK. Pending/confirmed,
failed/disconnected/stale and unavailable provider states must remain independent
from the local button appearance. Missing measurements never become zero.

## Asset ownership and reproducibility

Run `python3 scripts/build_panel_glyphs.py` from GigPies. Standard-library Python
writes deterministic original MIT geometry to `src/instrument-panel/src/assets.rs`
and the codepoint registry `src/instrument-panel/assets/registry.json`. There are
no font downloads, host-font edits or generated screen images in that build.
The geometric source is editable, unhinted simple TrueType outlines plus generated
OpenType COLRv0/CPAL tables in protocol containers. The control geometry contains no proprietary or third-party artwork. The
optional text-font assembly uses the separately licensed DejaVu foundation below. U+101000..U+1011BA avoids the current
Nerd Font BMP/PUA-A allocations; query every slot before claiming it because
private-use ranges are not globally exclusive.

Font foundations examined:

- [Hack licence](https://github.com/source-foundry/Hack/blob/master/LICENSE.md):
  MIT additions, public-domain DejaVu changes and Bitstream Vera conditions.
- [Fira Code licence](https://github.com/tonsky/FiraCode/blob/master/LICENSE):
  SIL OFL 1.1. Suitable text foundation, unnecessary to modify for protocol tiles.
- [Powerline Extra Symbols licence](https://github.com/ryanoasis/powerline-extra-symbols/blob/master/LICENSE):
  MIT; geometric separators do not supply the requested material controls.
- Installed DejaVu Sans Mono: Bitstream Vera licence with public-domain DejaVu
  changes, verified in `/usr/share/doc/fonts-dejavu-core/copyright`. The derived font is renamed and retains the licence notice.

## Production workflow preservation inventory

Use source/current native contracts, not simulator screen counts, during migration.

| Consumer and current owner | Workflows that must remain reachable |
|---|---|
| Desk `frontend.rs`, `actions.rs`, `native.rs` | Mix, Channel, Health/Analysis; selection and dynamic banks; FOH/monitor scopes; fader/pan/mute; detached exact dB/pan editors; explicit grants/release/reconnect; MANUAL/ASSIST/AUTO and scoped reviewed release; full review paging and presentation coverage |
| Desk processing/master EQ/sends | Four channel bells/compressor and independent bypass; eight parametric/31 graphic master bands; linked/independent L/R; setup mute and separate rearm; narrow live EQ; per-send tap and value review |
| Desk topology/Brain/meters | Structure/device mapping, physical vs logical/USB identities; Brain duplex review; PFL/AFL/listen; held talkback with release/focus-loss safety; raw/pre-fader GP-METER and independently fresh gain reduction; monitor banking |
| Desk FX/measurement | Prepared FX configuration and permit/applied/settled state; tail bypass/history reset; PA measurement capture/review/candidates; capability/freshness/refusal paths |
| Lightdesk `frontend.rs`, `native_actions.rs`, `native.rs` | Stage, Programmer, Library, Playbacks, Health; selection/bank; capability-checked exact attribute entry; grant; record/GO; explicit zero/Hold; playback/Auto return review; mode/blackout review; full context-bound review paging; console commands and reconnect |
| Lightdesk music/scenes | Read-only named analysis/harmonic/recurrence provenance; source loss; scene inventory, detached map editing, nonmutating owner preview; real null/disarmed output remains labelled physical UNKNOWN |
| Both | Independent provider workers, bounded queues, role gates, no UI restart recall, no lost-input replay, pending vs confirmed, owner epochs/caps/freshness, keyboard/focus/resize and complete review visibility |

The specimen's meter/spectrum motion is explicitly synthetic. It does not create
a spectrum capability in production. Existing providers supply only their actual
advertised observations. A legacy terminal without release events cannot safely
carry held talkback: that action must remain unavailable unless a qualified input
protocol supplies the required edges/focus lifecycle.

## Investigation provenance

[Glyph Protocol specification](https://github.com/raphamorim/rio/blob/59203ccd5fa1e6b760a738c1ad653bb29f2f3465/specs/glyph-protocol.md)
and the corresponding `rio-vt/src/ansi/glyph_protocol.rs` were inspected. It
advertises glyf/COLRv0/COLRv1, supports bounded containers and per-slot clearing.
The [upstream example](https://github.com/raphamorim/glyph-protocol-examples/tree/c4f18b5fde7902af1d01f0c2fcc2a8dadd1a08ad/ratatui)
is plain Crossterm/libc output, not a Ratatui integration. Its comment equating
flush with atomic PTY writes is not a portable guarantee; our parser is incremental.
The original [introduction](https://rapha.land/introducing-glyph-protocol-for-terminals/)
is design background, not a terminal compatibility certificate.

Desk's previous path is `native::Gpu::render` → `raster::rgba` → whole texture;
Lightdesk's is `native::upload` → `render::ppm` → RGB/RGBA expansion → whole texture.
Their blueprint's native-wgpu direction explains the drift. The earlier
[comparison](../archive/records/2026-10-11-gp-ui-compare.md) measures application
work only. Its buffer-replay previews are not real terminal screenshots.

## Local font proof and launch boundary

Actual Rio 0.5.28 ARM64/X11 with Mesa lavapipe on rpi5 rendered COLRv0 from
an application-local SHR Panel Mono font. The initial protocol screenshot
exposed square-fit glyphs: Rio's parser accepts registration but does not
implement the documented `aw/lh/size=stretch` fields. Its current
`glyph_cell_scale` uses uniform em/bounding-box containment. Protocol registration
therefore remains experimental; it is not the primary material path.

SHR Panel Mono adds only the registered PUA tiles to DejaVu Sans Mono and renames
the font. Letter/number outlines are consistently rescaled to an exact half-em advance
and one-em line box, preventing fractional-advance seams at qualified even pixel
sizes (16, 20 and 24). Odd sizes and extra terminal line spacing are unqualified. The font is made available only
to the launched process via FONTCONFIG_FILE; no desktop-wide font setting changes.
`SHR_PANEL_FONT=SHR Panel Mono:0.3` explicitly selects colour tiles in the console;
without that contract the renderer uses ordinary Unicode. Never set it for an
unverified terminal/font combination. `NO_COLOR` must be unset for colour mode.

Production entry points are `shr-desk --terminal` with the same provider arguments
as `--native`, and `shr-lightdesk terminal` with the same provider arguments as
`native`. No default invocation connects. Existing native/offscreen paths remain
available during acceptance. The first adapters preserve the source scene's
160x45 review geometry through shared cells; they do not rasterize a framebuffer.
This transitional layout is not yet the final cell-native redesign of every page.
Keyboard release support is queried, not assumed. Unsupported held talkback and
Lightdesk confirmation are refused visibly; the ordinary fallback remains readable.
The specimen exercises mouse interaction; the production Channel page supports
field selection and existing action buttons. The primary operator input is MIDI,
not a computer keyboard. Physical MIDI channel bindings remain unqualified.

Reproduce local font and specimen from GigPies:

```sh
/usr/bin/python3 scripts/build_panel_glyphs.py --font /usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf --out artifacts/instrument-panel/fonts/SHRPanelMono.ttf
# Build panel-specimen with the package README's locked, parent-held command.
/usr/bin/python3 scripts/preview_panel.py --name specimen-final
```

The authoring input SHA-256 is
`54bf827eb99404e8f430c330ad30f063334f637eba0109b6a18d4f566a8e9dd8`.
The included `src/instrument-panel/assets/DEJAVU-LICENSE.txt` retains the foundation
licence; the original control geometry/code is MIT. No Nerd/Powerline outlines
are bundled. Font generation is independent of protocol-payload generation.
The preview command opens only an owned Xvfb/Rio process pair and forces software
lavapipe; it does not open audio/MIDI/DMX or change a host service.

## Preview commands and qualification boundary

These captures are from the actual ARM64 Rio process on this Pi, using Xvfb
and explicit software terminal backends (tiny-skia or Mesa lavapipe). They are terminal framebuffer screenshots, not browser
mockups or Ratatui buffer exports. They do not qualify HDMI, physical GPU scanout
or a TV. The primary font uses COLRv0; the local COLRv1 experiment rendered flat
monochrome silhouettes and is **not supported** by this selected build.

Provisioned tools: Debian Xvfb, xdotool, Pillow, fontTools and Mesa Vulkan;
Rio 0.5.28 ARM64 X11 was extracted only into
`artifacts/instrument-panel/tools/rio`. The release Debian archive SHA-256 is
`8f838db9aeba500c4836c46dac42201f81470a7cdd6df2d10697b0608e66fbae`.
No host terminal configuration or font selection was changed. Release download:
`https://github.com/raphamorim/rio/releases/download/v0.5.28/rioterm_0.5.28_arm64_x11.deb`.
Verify the upstream checksum before extraction with `dpkg-deb -x` into that path.

```sh
# Actual-terminal specimen, exits after retaining its screenshot:
/usr/bin/python3 scripts/preview_panel.py --name specimen-final
/usr/bin/python3 scripts/preview_panel.py --name specimen-small --font-size 16 --width 1280 --height 760
/usr/bin/python3 scripts/preview_panel.py --name specimen-large --font-size 24 --width 1928 --height 1122
# Explicit private offline owners and read-only production UIs; no writer grant:
/usr/bin/python3 scripts/preview_panel_providers.py desk lightdesk
# Ordinary readable fallback, with no PUA rendering request:
/usr/bin/python3 scripts/preview_panel.py --name specimen-fallback --command /home/shome/p/gigpies/target/debug/panel-specimen
```

The provider preview requires existing `gigpies-headless`, `lux-service`,
`shr-desk` and `shr-lightdesk` binaries at their documented canonical target paths.
It neither builds nor changes the Lux repository. Desk's captured eight-input
legacy fixture is not a product capacity limit. Lightdesk's fixture inventory is
explicitly synthetic and its actual owner reports null/disarmed output.

Direct Rio and an isolated tmux 3.5a server were exercised. The private tmux
configuration enables RGB and extended keys; it is not installed into the user's
normal tmux configuration. SSH qualification remains open: a strict host-verified
localhost connection was refused by public-key authentication. No authentication,
SSH service or key configuration was changed. Do not claim SSH or another
terminal/multiplexer combination inherits the direct-render qualification.

The installed font needs no per-frame or startup glyph upload. The experimental
Glyph Protocol session probes support and each empty slot, registers a bounded
repertoire, and clears only its own slots on normal exit. Fragmented
replies and bounded malformed input are unit-tested. Timeout/reconnect cleanup
and delayed replies are not qualified; use the installed-font primary contract.

The current 0.2 font is byte-reproducible with SHA-256
`361665c643fa77d53f4000f0c7e9b2c27e41d7c2d82e29828dc19821c9be66e8`.
Font-size captures at 16/20/24 px show continuous boundaries after the metric
correction. The palette is visually distinct with bounded highlights/glow;
framebuffer RGB is affected by Rio's colour pipeline and is not bit-exact or a
calibrated physical-display measurement.

The historical 0.1 experimental protocol run registered all 77 tiles in 173.5 ms (30,584
base64 payload bytes, plus protocol headers/queries). A post-exit query in the
same PTY confirmed **all 77 slots empty**. The updated probe reads the current
registry; the expanded 443-slot protocol path remains unqualified. Run explicitly:

```sh
/usr/bin/python3 scripts/preview_panel.py --name specimen-protocol --family 'DejaVu Sans Mono' --system-fonts-only --command sh -c '/home/shome/p/gigpies/target/debug/panel-specimen --glyphs; /usr/bin/python3 /home/shome/p/gigpies/scripts/probe_panel_cleanup.py /home/shome/p/gigpies/artifacts/instrument-panel/protocol-cleanup.json'
```

Its screenshot also demonstrates why it is experimental: uniform square fitting
leaves gaps in the fader track and compresses LEDs vertically. Normal registration
and cleanup succeeding does not establish usable tile geometry.

For this headless Pi, append `--cpu-terminal` to the specimen, provider-preview
or terminal-measurement commands to use Rio's internal tiny-skia renderer. It
preserved COLRv0 appearance and observed all six 100-frame workloads at median
45.5–47.0 ms including framebuffer readback, versus 100.5 ms for the lavapipe
Desk-pages run. This is a terminal backend choice: both applications still emit
Ratatui cell updates. See the [dated evidence and limits](../archive/records/2026-10-11-gp-panel-proof.md).

Current actual renders: [Desk channel](../assets/panel-desk-channel.png) and
[local draft/focus](../assets/panel-desk-channel-draft.png). Rejected earlier
compositions and disposable protocol captures were removed after their findings
were recorded; the source and repeatable commands remain.


## Desk selected-channel page

![Selected Channel panel with coloured LED meters and separate EQ, compressor and output sections](../assets/panel-desk-channel.png)

Layout approved October 20, 2026. This updated actual-terminal capture shows
the aligned 16-segment meters, centred titles and measured post-processing
output. The meter refinement is also approved.
Actual production terminal capture using deterministic
software audio; no physical audio or MIDI qualification is implied.

The dedicated Ratatui Channel page requires the existing `--processing` capability
probe and FOH context. Four bell bands (Hz/dB/Q and independent bypass), global EQ
bypass, compressor threshold/ratio/knee/attack/release/makeup/bypass, pan, fader,
mute, Hold and monitor-send navigation use the existing owner paths. Unsupported
preamp gain, phantom, polarity, HPF and gate are not advertised as inert controls.
Other scopes and all exact-entry/review/modal pages retain their existing routes.

Click a parameter or encoder to open/select its detached processing field. U/I
select fields; J/K adjust fine, N/P coarse; numeric entry plus Enter accepts text
locally. F4 opens the complete existing review; only a separate Enter after all
review pages are presented can confirm. Clicking the output level/pan opens the
existing exact-value editor. Mute/Hold/release still use owner review. The encoder
is a field-selection target; continuous mouse dragging is not implemented.

The graph is the calculated four-bell **settings response**, using the existing
Desk EQ-response calculator and the owner's advertised sample rate and RBJ-bell convention. Its window
is ±18 dB. The separately sourced spectrum described below sits behind it; the
curve itself remains a settings response. Draft settings are amber, selected
values cyan; full owner/draft differences remain in the existing review.
Gain reduction reads the owner detector independently of drafts. Input sample
peak is the real GP-METER observation; stale data cannot light a live meter.

The 0.3 font preserves corrected colour-layer side bearings and supplies bounded 6×3-cell
metal encoders with 17 indexed positions, a 6×2 slider cap and reusable button
edges. Regenerate the local font before setting the `:0.3` marker. No glyph is
created or uploaded per frame. The historical 77-slot protocol measurements are
not measurements of the expanded repertoire; downloaded glyphs remain experimental.

Reproduce a real terminal capture with the private software engine and explicit
injected role descriptors (no physical devices):

```sh
/usr/bin/python3 scripts/preview_panel_providers.py desk --channel --cpu-terminal --suffix channel
```

`--keys` sends paced keyboard gestures, `--click X,Y` targets a terminal cell in
window pixels, `--fallback` selects ordinary text, `--disconnect` drops only the
owned private provider, and `--resize WIDTH,HEIGHT` exercises recovery; a semicolon-separated sequence
checks shrink/restore. Rio may report one fewer row after a pixel resize than
at startup: `--resize '1000,700;1928,1146'` restores 160×45 at 24 px.
The minimum-size guard follows actual terminal cells, not requested pixels. A channel
capture prints a concise independent read-only GP07 owner readback. Temporary
X/terminal configs, logs and private synthetic-owner state are removed on exit;
only the requested PNG remains. No JSON dump is retained by default.
The regular launch remains `shr-desk --terminal ENDPOINT SHOW EPOCH WRITER foh
--processing`, with the documented role-provider options. Preview role descriptors
are synthetic assertions, not evidence of a connected controller/display.


### MIDI-first presentation

The operator chose MIDI as the primary input. The Channel operating page has no
computer-keyboard shortcut strip or shortcut prefixes; the keyboard bindings
remain a development/accessibility path described here, not the panel layout.
The existing controller plan owns physical mapping. Its channel rotary assignment
is still proposed: this rendering change neither opens a MIDI port nor claims a
working learned hardware profile. Physical column numbers describe the intended MiniLab layout, not learned bindings.

Shared three-cell indicator lenses use original metallic bezel/dark-lens geometry
and bounded green/yellow/red/blue illumination (24 appended tiles). EQ, compressor
and band lamps follow independently fresh **settled owner** processing, never a
local draft. Mute indicates the owner's mute target; Hold indicates the owner hold.
Blue means a selected draft parameter; amber means a local unapplied draft.
Explicit captions and values remain readable without colour. Unlit lenses remain
visible. The new font requires `SHR_PANEL_FONT='SHR Panel Mono:0.3'`.


## Channel cached materials and MiniLab columns

Set `SHR_CHANNEL_MATERIALS=rio-kitty-v1` for the selected Channel experiment.
Ratatui remains the sole terminal input/text/diff owner. Original MIT PNG plates,
recessed wells, rounded pots, raised/latched buttons and four-colour LED lenses
are composed behind ordinary text using Rio’s Kitty graphics protocol. Shading
is calculated **offline in the application asset authoring tool**, not by a
terminal shader engine. No full-screen framebuffer exists. Each used asset is
uploaded once; subsequent frames diff placements and reuse the cache. Stable
frames emit zero material bytes. The finite repertoire is 118 assets, 709,328
base64 payload bytes; first use uploads only the needed subset.

Authoring source: `../shr-desk/scripts/build_channel_materials.py` (system Python
with Pillow), generated `src/channel_material_assets.rs`, renderer
`src/channel_material.rs`. These contain original geometry, no third-party art,
text or fabricated measurements. This explicit Rio 0.5.28 path has been inspected
at 16/20/24 px on ARM64 using a real terminal under Xvfb. It is not a claim about
arbitrary Kitty-compatible terminals, SSH, physical scanout or connected MIDI.

The [Desk controller design](../../../shr-desk/docs/CONTROLLER.md#selected-channel-eight-physical-columns)
owns the eight-column layout: EQ frequency/gain in columns 1–4, compressor
threshold/attack, ratio/release and knee/makeup in 5–7, pan/level in 8. All eight
pads align below; menus are contextual and the 9–16 layer holds less-used commands.
Physical MIDI messages, encoder scaling/pickup and pad LED feedback remain to be
mapped and qualified. Software menus use existing reviewed actions and owner
readback; no new binding is implied.

```sh
/usr/bin/python3 scripts/preview_panel_providers.py desk --channel --materials --cpu-terminal --suffix channel
```

Add `--font-size 16` or `20` for smaller normal-size checks. The source supports
repeatable private software-owner captures, not hardware activation. For normal
connected use, keep the documented provider/role invocation and add the explicit
material environment variable. See GP-PANEL for final screenshots, measurements
and the remaining visual-review gate. Other screens are outside this increment.

### Selected-channel spectrum contract

GigPies `src/spectrum.rs` owns the analysis; Desk `src/spectrum.rs` only validates
observations and maintains display peak holds. The existing lighting C-ANALYSIS
stream is a four-source subscription and cannot select an arbitrary logical
input, so it remains unchanged. The new path reuses GigPies' existing radix-2 FFT.

`GP-SPECTRUM:2` is the optional read-only spectrum extension on private same-UID
`audio.sock` and the existing mutually authenticated remote observer connection.
The request carries `contract`, `version`, `show_id`, `epoch`, `map`, `query`, and
`input`. The last field retains its legacy name but identifies the independent
**analysis source**, not the EQ target. Unsigned counters and strict unknown-field
rejection remain. Responses echo identity/query and return `supported:true`,
`sample_rate`, `tap`, `units:"millidBFS-peak-Hann-max"`, source `first_frame` and
exclusive `end_frame`, `acquisition_age_ms`, `valid`, and legacy 64-band `bins`.
Version 2 additionally returns `fft_bins` (2049 integer amplitudes, indices 0..2048
of the same 4096-point FFT) and `sources` (`id`, `label`, `tap`). Missing/stale data
has `valid:false` with both arrays empty. The inventory comes from admitted topology:

- Stable input IDs: `raw-pre-processing`, after capture-slot mapping, before EQ,
  compressor, fader and mute.
- `main:left` / `main:right`: `main-pre-pa`, the actual main meter tap including
  composed main processing/returns when present, before owner PA.
- `output:<stable-output-ID>`: `physical-output-post-patch`, the configured
  playback slot after processing, output safety and route patching. Labels include
  physical port and configured Main/Monitor/PA route; unassigned slots are actual
  digital zero, not an inferred physical measurement.

The reference rate remains 48 kHz. FFT length, periodic Hann window, amplitude
normalization and 10 Hz acquisition are unchanged. DC/Nyquist endpoints use 2/N;
positive interior bins use 4/N. Desk maps half-bin boundaries `(k ± 0.5) * rate / 4096`
onto each 20 Hz–20 kHz logarithmic display column and takes the maximum of
intersecting bins. Thus low bins span several columns and high bins combine; screen
width never claims additional frequency resolution. The 64 legacy bars are not
stretched when native bins are available. This is peak amplitude, not band power.

Version 1 remains accepted with its original fields and 64-band response, without
v2 fields. Desk probes v2 on its independent observer connection; if that initial
probe fails it makes one v1 attempt for a raw input on a new connection. A successful
v1 response is labelled `v1` (64 coarse bands); failures are unavailable, not proof of an
unsupported version. No command/meter connection is reused or mutated. A discovered
observer reconnects read-only after loss, clearing display history. TLS pairing,
revocation, session epoch/map checks, bounded framing and query correlation apply
to remote observations. Queries are limited to one per 80 ms per connection/session;
Desk requests every 100 ms. The consumer observation budget is 128 KiB and at most
4096 source descriptors (an observer protocol budget, not an engine channel cap).

F11 on the Channel panel opens/closes fullscreen EQ/RTA; `[` and `]` cycle analysis
sources independently of the edited channel. At 160 columns the graph has 158
interior columns; it adapts down to 60×18, while ordinary control/review screens
retain their 160×45 minimum. One compact contextual header is the only text string; the rest is the plot. Spectrum
colour uses fixed dBFS stops −96/−72/−54/−36/−18/0, interpolated green→yellow→orange→red,
with no normalization. Filled RTA columns sit behind discrete coloured EQ points
on a separate ±18 dB gain scale; fullscreen has no combined response curve.
Horizontal brackets show nominal Q bandwidth (f/Q, geometrically centred),
with a heavier selected bracket and hollow/dashed bypassed controls. Source/tap
and EQ target appear in the header; during editing it shows the selected EQ
frequency/gain/Q and entry instead. Missing observations clear the spectrum.
Clock/frame fences remain enforced without displaying telemetry panels. Existing grants, detached edits, Apply/full review/confirmation
and cancellation retain authority. Fullscreen interaction may redraw independently;
analysis remains 10 Hz. Modal review always takes precedence over the graph.

The raw-input tap takes logical source samples before channel EQ, compressor, fader and mute,
following the source-frame timeline after physical-slot mapping. It is digital
sample amplitude, **not dB SPL or a calibrated acoustic measurement**. At 48 kHz,
a 4096-sample periodic Hann window starts every 4800 source frames (10 Hz).
Positive FFT amplitudes use `4/N` normalization; a bin-centred sinusoid with peak
1 is 0 dBFS. Each of 64 equal-log intervals between 20 Hz and 20 kHz takes its
maximum FFT-bin amplitude. This is peak-bin aggregation, not integrated band
power or PSD. Empty intervals/silence floor at −96 dBFS; finite display range is
−96..+24 dBFS, with visual bars clipped at 0. Resolution is 11.71875 Hz, so the
lowest intervals may contain no FFT bin. Nonfinite or >16 peak source samples
invalidate acquisition; no overflowing FFT is published.

One selected tap per provider is a bounded analysis resource. Multiple observers
requesting different sources compete for that tap; every reply checks the actual
input/map and omits mismatched data. Selection expires one second after the last
query. Source changes and recovered epochs fence queued windows as well as the
latest observation; Desk clears observations and peak holds on source changes. The callback performs fixed storage copies and nonblocking SPSC offers;
it allocates nothing, locks nothing and performs no I/O. The two-window queue
cannot grow. The worker drains at most two windows, analyzes only the newest,
and publishes one latest observation; full queues drop work. Desk retires its
bounded in-flight observer read before replacing the worker, so rapid source/page
changes do not accumulate observer threads. FFT, JSON and
mutex-protected latest publication occur outside audio processing. Queries have
an 80 ms minimum spacing and the existing bounded reply queue/deadline.

Age is measured conservatively from receipt of the first source block using the
monotonic clock; it includes collection/worker time, and Desk additionally charges
the request round trip. This does not prove physical ADC capture age. Observations
expire after 350 ms and repeated source frames cannot renew them. Selection,
map/epoch changes, primary loss and reconnect clear display history. Peak markers
hold 500 ms of source time then decay 24 dB/s; stale/absent data clears bars and
markers without continued animation. The small existing graph legend shows a
dash for unavailable data; no status banner adds a row.

The final Channel layout has 16 usable graph rows, 16 meter segments, two aligned
rows of eight pots and one row of eight contextual pads. The old four-row header
and SETTLED/TARGET/EDITED/ENTRY inspector are removed. Channel identity and the
software pad-layer selector sit in the plate edge; typed numbers appear inside
the selected value well. Full review/confirmation, grants, caps, Hold, manual
ownership and null/disarmed output remain in their existing menus/owner paths.
Three separate vertical metal plates follow the physical columns: EQ 1–4,
compressor 5–7, output 8. The top is an empty border row, a row of labels/LEDs, then an empty row
before the display wells. Knobs and pads retain their existing coordinates. The EQ spectrum/response stays inside its 63×16-cell
graph. Above the compressor pots, three recessed 13-segment meters show raw input
peak/RMS (dBFS) and actual owner gain reduction (dB, filling downward). Raw levels
are not presented as post-compressor measurements. Each observation keeps its
existing freshness gate; unavailable scales remain unlit with a dash. Column 8
retains its independent 16-segment raw sample peak meter. Materials remain cached and Ratatui owns all terminal input/text.

Software proof and actual terminal captures (no physical devices):

```sh
# Under the shared build reservation, canonical GigPies target:
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 cargo +1.97.1 build --locked -j1 --example spectrum-source
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked -j1 --test spectrum -- --nocapture
/usr/bin/python3 scripts/preview_panel_providers.py desk --channel --materials --spectrum --cpu-terminal --suffix channel
# Opt-in cost evidence; synthetic PCM remains exclusively test/example input:
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked --release -j1 --test spectrum -- --ignored --nocapture
```

The existing [GP-PANEL card](../development/MODULE_IMPLEMENTATION_PLAN.md#gp-panel--shared-physical-instrument-panel)
owns the final measurements, validation, screenshot states and limitations.

### Notcurses comparison

Studied upstream [Notcurses](https://github.com/dankamongmen/notcurses/tree/v3.0.11),
its [plane](https://github.com/dankamongmen/notcurses/blob/v3.0.11/doc/man/man3/notcurses_plane.3.md),
[visual](https://github.com/dankamongmen/notcurses/blob/v3.0.11/doc/man/man3/notcurses_visual.3.md)
and [render](https://github.com/dankamongmen/notcurses/blob/v3.0.11/doc/man/man3/notcurses_render.3.md)
documentation, `src/demo/trans.c` and `src/poc/gradients.c`, plus
[libnotcurses-sys](https://github.com/dankamongmen/libnotcurses-sys).
Planes are ordered cell matrices with alpha compositing; cell gradients and
half-cell gradients remain spatially discrete. Bitmap visuals select terminal
protocols such as Kitty/Sixel; ordinary/subcell text still depends on font
metrics and glyph coverage. Incremental rendering compares composed cells and
optimizes escapes; bitmap damage can entail larger retransmission. These are
terminal rendering facilities, not a GPU material/shader engine.

C 3.0.11 was built and run on this ARM64 Pi. Rust wrapper 3.11.0 declares edition
2021 / Rust 1.65 and its build script requires C >=3.0.11; release documentation
pairs it with C 3.0.11. Those declared requirements are consistent with the pinned Rust 1.97.1
toolchain; this experiment did not compile the Rust FFI.
Notcurses is Apache-2.0 with source exceptions in COPYRIGHT (including MIT
neofetch data); the wrapper is MIT OR Apache-2.0. No dependency was added to Desk.

The standalone MIT probe `src/instrument-panel/examples/material_compare.c`
compares cell and half-cell gradients with the same authored EQ plate and a text
plane. It independently owns its terminal; it never runs inside Ratatui. On Rio,
text damaged the pixel plane with visible rectangular holes and caused image
retransmission in this configuration. The successful cached-placement prototype
therefore remains the bounded Channel choice. This is evidence about this probe
and terminal, not a universal limit on Notcurses. A broad backend migration would
require a separate architecture decision and equivalent owner/workflow checks.

Reproduce with C 3.0.11 installed in an isolated prefix (no host services):

```sh
cc -O2 -Wall -Wextra -Werror $(pkg-config --cflags notcurses-core) src/instrument-panel/examples/material_compare.c $(pkg-config --libs notcurses-core) -o /tmp/material-compare
# Export the existing EQ asset, no image synthesis at runtime:
/usr/bin/python3 - <<'PYASSET'
import base64, io, re
from pathlib import Path
from PIL import Image
s = Path('../shr-desk/src/channel_material_assets.rs').read_text()
data = re.search(r'\("eq", \d+, \d+, "([^"]+)"\)', s)[1]
Path('/tmp/eq.rgba').write_bytes(Image.open(io.BytesIO(base64.b64decode(data))).convert('RGBA').resize((936,864)).tobytes())
PYASSET
/usr/bin/python3 scripts/preview_panel.py --name notcurses-compare --font-size 24 --width 2200 --height 1020 --cpu-terminal --keys '' --command /tmp/material-compare /tmp/eq.rgba /tmp/notcurses-stats.txt
```

Remove those temporary probe outputs after inspection. CMake qualification used
`USE_MULTIMEDIA=none`, `USE_CXX=off`, `USE_DOCTEST=off`, `USE_PANDOC=off`,
`USE_POC=off`, `BUILD_EXECUTABLES=off`, `BUILD_TESTING=off`, `USE_STATIC=off`,
`USE_DEFLATE=on`, Release; local libunistring/ncurses/libdeflate headers, no host
installation. Source tar SHA256:
`acc8809b457935a44c4dcaf0ee505ada23594f08aa2ae610acb6f2355afd550a`.

## Master gain software contract

LocalAudio advertises `master_gain` (GP-Master version 1) in rendered snapshots,
with stable target `main` and source-observed current/target linear nanogain.
The nested authority target is exactly `{"parameter":"master_gain"}`; it is not
an input, mute, pan or PA EQ operation. Ordinary C-AUDIO Set/FOH authority,
revision, epoch, writer/lease and source-owner admission fences apply. Values
are integer milli-dB from −60000 through +6000 inclusive. Unsupported providers
omit this capability; Desk refuses Master edits against them.

Preparation computes the linear coefficient off-source. The source applies one
240-frame (5 ms at 48 kHz) linear ramp to coherent stereo dry + FX Main returns
before PA and before talkback. Local legacy FX, local four-aux and authenticated
remote returns share that position. Independent monitor returns, pre-Master FX
sends, raw recording and input analysis retain their original taps. Main listening
is post-Master/pre-talkback; Main metering/spectrum is post-Master/pre-PA (including
FOH talkback when present). Output safety remains independent; gain never rearms.
No JSON, allocation, locking or destruction is added to source execution.

Core engine intent version 4 includes the Master target configuration only.
Versions 1–3 cannot contain that target; production loading upgrades their absent
Master to unity. Version 4 requires exactly one Master target. Restore discards
leases, holds, pending work and output arming. The FX-only saved-intent format is
unchanged. Master pan/mute and physical controller bindings remain unavailable.

Disarmed source boundaries close direct Main/listening and final mapped playback,
including wet and native PA tails. Master configuration never opens this closure.
