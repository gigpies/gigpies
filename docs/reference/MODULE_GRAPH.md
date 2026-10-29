# Fixed owner-library offline graph

GP05 extends the actual GP03 eight-input synthetic engine and its GP04 raw tap.
The optional graph sends the same eight raw input samples to SHR REC, and the
actual stereo FOH mix to SHR FX's fixed wet-only delay. Dry plus wet then passes
through SHR PA's two-input, six-logical-output adapter. Monitor buses retain their
GP03 behavior; the PA limiter applies only to main logical outputs 0/1. Outputs
2..5 are silent under the accepted preset. No physical output is opened.

The separate `GP05-modules:1` query leaves every GP03 wire signature unchanged.
A default service explicitly reports module health unavailable. Build with
`hardware-host` for explicit `--modules /absolute/manifest.json` activation;
compiling that feature does not open a device. This graph does not activate the
qualified hardware executable or modify its routes.

```sh
CARGO_INCREMENTAL=0 cargo +1.97.1 build --locked -j1 --release --features hardware-host --bin gigpies-headless
install -d -m 700 /tmp/my-private-synthetic-show
./target/release/gigpies-headless --directory /tmp/my-private-synthetic-show \
  --show 11111111-1111-4111-8111-111111111111 --epoch 9 \
  --synthetic-source fouraux --analysis --modules /absolute/modules.json --ticks 2000
```

This is an explicitly invoked temporary private Unix endpoint, not an installed
service. Restart requires a new authority epoch. The provider libraries are
trusted executable code selected by absolute path. The activation manifest has
version 1 and `rec`, `fx`, `pa` artifacts, each with `library`, `library_sha256`,
`header`, `header_sha256`, `owner_manifest`, `owner_manifest_sha256`. Both the
header and library hash must occur in the hash-pinned owner's `files` manifest.
Verification and loading happen before processing. Independently build providers
at the commits pinned in [provider data](../../tests/fixtures/gp05/v1/providers.json);
no sibling path dependency, binary distribution or copied DSP is used. Exact
release hashes identify the accepted artifacts; rebuilding on another toolchain
or host can produce different hashes and requires its own review.

Query `status-request.json` from [the GP05 corpus](../../tests/fixtures/gp05/v1/README.md)
using the existing four-byte big-endian length framing on `audio.sock`. Health is
separate from the GP03 snapshot. It reports the actual quiesced FX/PA capability
and status queries, verified library hashes, actual prepared configuration,
readiness/activity and recorder progress. `available` describes the software
binding, while `readiness` reports ready/degraded/faulted. Physical verification is
always `unverified`. Writable FX parameters, FX rack, PA controls, measurement,
true-peak and acoustic protection are unavailable. The owner's linked −1 dBFS
sample ceiling is not calibrated amplifier or loudspeaker protection.

Recording uses `record_start` and `record_stop` in the GP05 envelope with the
existing show/module/epoch/writer/lease/request-ID/revision fields. Start/Stop
bodies contain a bounded opaque `take_id` and decimal-string `operation_id`.
Clients must first obtain the normal GP03 snapshot and FOH lease on the same
connection. Recording destinations are create-new takes under the owned private
service directory's `takes/`; no client paths are accepted. Request identity is
shared with GP03, while the take/operation pair fences lifecycle completion.
Exact retries retain the prior admission/outcome; changed contract/body with the
same request ID is rejected. A lost reply remains uncertain until readback.

`accepted_pending` means preparation/finalization was admitted. One bounded
lifecycle worker creates or consumes the recorder, outside processing and the
control pump. Successful readiness starts capture at an explicit whole 48-frame
boundary; successful finalization publishes the terminal owner outcome. Stop
during preparation prevents late readiness from reactivating capture and
finalizes an incomplete take. A stale operation cannot stop a newer take.
Client/lease loss does not stop admitted recording. Independent analysis loss,
slow Lux and Desk disconnection do not enter the recorder or PA/FX processing
path. No recording samples come from processed buses or analysis consumers.

The library is retained until all handles, observers and lifecycle workers are
gone. The independent recorder observer survives consuming finish. Progress is
cached at no more than 10 Hz, including while draining. Host `finalizing` and
`pending` remain until its finalizer joins; owner atomics alone cannot announce
host completion. An independent terminal read after finish preserves its outcome.
Accepted frames mean admitted PCM; written frames cover all stems and the owner
journal; durable frames stay null, including after finalization. Counters are
individually monotonic and weakly consistent during capture. Completion is not a
power-loss durability certification.

Processing is fixed at 48-frame source boundaries, 48 kHz. Duplicate, unexpected
frame or epoch input is refused without advancing. Explicit control-side
`discontinuity` resets FX, recreates PA and finalizes any active take incomplete;
a new take is required under the new mapping/epoch. PA numeric fault silences and
latches until recreation. FX error mutes wet while dry continues to PA. All owner
return codes remain visible in queried status. Preparation/query/finalization
allocate or perform I/O only off the bounded process function.

Validation uses synthetic PCM and temporary owned directories/sockets. Normal
codec, authority and ABI-layout tests require no provider installation. The
actual accepted-library tests are explicit because trusted dynamic libraries are
external artifacts:

```sh
CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked -j1 --test gp05
CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked -j1 --features hardware-host --test gp05
GP05_MANIFEST=/absolute/modules.json CARGO_INCREMENTAL=0 cargo +1.97.1 test --locked -j1 \
  --features hardware-host --test gp05 --test gp05_alloc -- --ignored
```

Physical device/controller/window, throughput/scheduler/shared-load, historical
media, auditions and long exhaustive tests remain separate. The root coordinator
owns independent review, joint Desk/Lux demonstration and source publication.

Review regressions additionally retain terminal take state across later source
recreation, preserve the actual host cause for cancellation during preparation,
and keep a quiesced handle available for cleanup if the finalizer thread cannot
be spawned. Source rejection requires explicit recreation before starting another
take. These are runtime corrections; the accepted status envelope stays unchanged.
Cancellation during preparation publishes `finalizing` before handing the ready
raw recorder to its finish worker. A concurrent progress observation cannot turn
that cancelled operation into host recording readiness or a completed start reply.

## Configurable owner composition

The graph now admits configured raw track dimensions, retaining unchanged REC v1
limits explicitly. Actual raw strips feed REC before processing. Prepared SHR PA
v2 supplies independent output ports, stereo3-way/stereo4-way and weighted routing
nodes. Program-bus indices are explicit: FOH0/1 include actual dry+wet summation;
2+ are monitors. Owner JSON, capabilities, status and bus mapping are read back.
No speaker DSP is duplicated in GigPies. v1 remains available for legacy callers.

Runtime PA changes prepare owner state and host buffers off render, commit at the
shared source boundary while quiesced, retain both old allocations, and retire on
the controller. Fresh-epoch recovery restores configuration intent muted. Actual
remote wet blocks (including absent-peer silence) suppress local FX fallback.
See [composition](MODULAR_PROCESSING.md) and its acceptance matrix.

## Desk recording operator

Desk's Recorder page uses the same GP05 Start/Stop envelopes and FOH lease, on
the primary local or authenticated remote connection. A complete presented review
pins the raw track order, authority revision, take and operation. A newly generated
take name avoids typing; the server still enforces create-new storage. Stop during
preparation cancels admission and waits for cleanup; a second Start is refused
while preparing, recording or finalizing.

Command replies are admission/lifecycle evidence only. Desk polls the exact take
readback independently, shows accepted minus written as queued frames, retains
unknown durability and distinguishes writer/storage, source and integrity faults.
The queued count is an observation of asynchronous counters, not disk capacity.
Free-space bytes, remaining time and power-loss durability are not advertised.
Startup failure appears as a finalized error; detailed retained filesystem
evidence and copy recovery remain SHR REC's responsibility.

Desk never retries a recording mutation automatically. A lost reply is unknown;
reconnect read-only and inspect the current take before issuing a new reviewed
command. A failed nonce-less status query retires local write authority so late
responses cannot refresh a new intent. Disconnect/lease loss leaves host capture
running. Host restart does not claim recovery or completion of an interrupted
take. GP05 wire versions, REC C ABI and raw file format are unchanged.
