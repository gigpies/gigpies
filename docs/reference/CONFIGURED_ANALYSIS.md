# Configured named raw analysis

C-ANALYSIS:2 is an additive configured attachment for the existing four sources,
ordered `kick`, `bass`, `guitar-1`, `guitar-2`. It does not introduce another mixer,
analyzer, source clock or device owner. Version 1 and its original fixture/request
bytes retain their exact semantics.

The strict startup mapping is `{"version":1,"inputs":["input-17","input-03",
"input-09","input-01"]}`. All four entries must be distinct canonical logical IDs
present in the admitted topology. IDs follow the existing unsigned 16-bit logical
port domain; this is not a claim that that many strips meet resource admission.
Aliases such as `input-001`, physical USB slots and measurement-only slots are
refused. The producer resolves IDs through its validated logical-strip inventory;
capture-slot permutations do not change the meaning of a logical source.

`gigpies-headless --analysis-map ABS_JSON` selects configured analysis independently
of `--synthetic-source fouraux`. Its existing `--analysis` flag retains the original
explicit fouraux path and cannot be combined with `--analysis-map`. Map JSON is
bounded to 4096 bytes, with duplicate/unknown fields rejected. The authenticated
provider run configuration accepts optional `analysis_mapping` with the same
schema, publishing from its existing local source owner into the same private
directory. This publisher is local same-UID IPC: monotonic acquisition ages are
same-host values, not a cross-node clock synchronization protocol.
The runner rejects `analysis_mapping` combined with `physical_device` before
device, identity-file, credential or listener side effects. Physical capture does
not yet propagate converter/buffer acquisition age; labeling such buffered data
fresh would be incorrect. This admission remains software-only until that
separate timing boundary is implemented and accepted.

Admission occurs before the first processed frame. The descriptor retains its
original field set, with `version:2`, `subscription:"lux.aux.v2"`, actual source
epoch, first frame and topology map revision, and positive calibration revision.
The exact framed subscription request is
`{"subscription":"lux.aux.v2","version":2}`. Each endpoint admits only its own
version, without negotiation or silent downgrade. Version 1 clients cannot receive
version 2 data under a legacy identity. GAW1 and PCM packet layout remain unchanged:
four PCM24 channels, ten 48-frame packets per window, source-frame sequence,
two-window staging, whole-window loss counts, and the existing 100 ms acquisition
age bound. Rendering, FOH controls and incoming packet arrival cannot advance the
analysis timeline independently of the source owner.
The software source timestamp is taken when the provider receives the borrowed
block, before controller work, and retained through publication. It does not
establish the age of samples buffered by an external/physical caller.

The tap reads raw logical input before mute, fader and processing. Float input is
rounded to signed PCM24 only within [-1, 8388607/8388608]. Nonfinite/out-of-range
samples, timestamp/encoding failures, source quiescence or changed topology map
stop the publisher; they cannot leave a live publisher with a stalled cursor or
silently relabel queued samples. No thread join/retirement is introduced into
the processing step: an atomic stop is requested and ownership is retired at
teardown. Analysis failure does not stop otherwise valid mixer processing.
Source recovery uses a fresh epoch and drops the old attachment. Mapping selection
requires an explicit fresh provider startup; no calibration or AUTO grant is
restored or inferred from retained values.

Consumer admission must validate the exact expected ordered configured mapping
on every connection before using the descriptor. A source/map/epoch change must
invalidate calibration and grants. Lux owns analysis and lighting decisions;
Lightdesk owns complete operator review and must validate both current descriptors
and retained contribution provenance. The corresponding Lux/Lightdesk configured
schema/corpus must be explicitly accepted before claiming consumer compatibility.

Normal production checks cover nonfixture 17-input and 32/48-input mappings,
physical-slot permutation, exact raw PCM, wrong-version refusal, topology change,
invalid mapping/sample input, source progression and allocation-free tap work.
The fixtures under `tests/fixtures/gp04/v2` are producer-checked. They do not prove
physical I/O, microphone mapping, lighting output or whole-host deadlines.

## Seven-feed configuration (C-ANALYSIS:3)

The local named subscription now accepts **4–8 feeds**, including the requested
kick, bass, two guitars, lead vocal, synth and backing vocals. Eight is the current
1200-byte GPA1 packet budget (48-byte header plus 48 PCM24 frames), not an engine
or product channel limit. Larger subscriptions need grouped packets; the separate
raw transport already supports grouping. The Lux analysis core separately admits
up to sixteen configured feeds. Neither bound establishes hardware capacity.

Use this version-2 provider mapping with `--analysis-map` (or `analysis_mapping`):

```json
{"version":2,"inputs":["input-01","input-02","input-03","input-04","input-05","input-06","input-07"],"sources":["kick","bass","guitar-1","guitar-2","vocal","synth","backs"]}
```

The matching Lux startup `analysis` binding uses the same arrays and **version 3**.
The first four roles retain their existing ordering; additional names are distinct
ASCII letters/digits/hyphens, 1–32 bytes. Every input is distinct and must exist in
the admitted topology. This is explicit source assignment, not source separation.
Calibration must observe every selected feed. Reconnection validates both ordered
arrays. Version-1 mappings and version-2 descriptors remain byte-compatible.

The version-3 descriptor uses `lux.aux.v3`; its exact attachment request is
`{"subscription":"lux.aux.v3","version":3}`. GAW1 retains its 40-byte header and
ten source-clock packets. Each packet is `48 + 144 * channels` bytes; the entire
window is `40 + 10 * (48 + 144 * channels)` bytes (10,600 for seven feeds).
Queue lengths, whole-window loss, map/epoch invalidation and 100 ms acquisition
freshness limits remain unchanged. Packet encoding and source publication retain
preallocated storage and no allocation, locks or I/O in `Tap::offer_interleaved`.
Wire framing allocates on the separate transport worker.

All selected feeds contribute to Lux's fast activity/pause cues and, except kick,
harmonic/register/arrangement analysis. Lightdesk validates and displays owner
estimates. The existing four-role lighting intensity director remains unchanged;
new predictions are observations, not automatic cue or output authority.
Physical capture admission restrictions above still apply.
