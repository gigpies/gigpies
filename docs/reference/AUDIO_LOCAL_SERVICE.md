# Explicit local synthetic audio engine

Linux-only `gigpies-headless` is an opt-in GP06 local binding of the real GP03
renderer/authority. Existing default CLI remains unchanged. No devices, playback,
TCP listeners, host services or GPC1/GPA1 changes. This document describes the
base GP06 mode. Optional [named analysis](ANALYSIS_STREAM.md) and
[owner-library integration](MODULE_GRAPH.md) extend it only when explicitly
activated; [local integration](../archive/workflows/HEADLESS_INTEGRATION.md) documents real console
clients.
This is offline and unprotected, with no deadline or physical safety acceptance.

Create a private directory owned by the invoking user, mode0700, then explicitly:

```
cargo +1.97.1 run --locked --bin gigpies-headless -j1 -- \
  --directory /absolute/private/directory \
  --show 11111111-1111-4111-8111-111111111111 --epoch 10
```

Use the shared host build lock for Cargo. The already-built binary can be launched
explicitly without Cargo. Choose a new explicit epoch for each authority restart;
volatile state and grants are not persisted. Startup prints exact show/epoch/path.
`--ticks COUNT` is a bounded synthetic run; SIGINT/SIGTERM gracefully closes clients
and removes only its original socket inode. The base mode has no worker threads; optional analysis and module workers are
joined by their owning lifetimes.

Endpoint `audio.sock` mode0600 is in canonical absolute owned0700 nonsymlink dir.
Preexisting endpoints, symlinks and unowned/permissive directories are refused.
SO_PEERCRED must match effectiveUID. No authentication beyond this local boundary
is claimed; remote production auth remains a separate gate.

Frames are four-byte big-endian byte length followed by UTF-8 JSON,1..65536 bytes.
Requests are strict unchanged C-AUDIO:1; responses are correlated GP03-rendered:1
from AUDIO_RENDERED_WIRE.md. Malformed/oversized/incomplete frames disconnect.
A new connection must successfully snapshot this show/epoch, then grant its own
fresh writer before mutations. Connection-local writer/lease binding prevents
importing old leases or grant retries from another connection. One writer has one
scope; separate clients/writers can own FOH/monitor scopes. Same live connection
exact retries retain original admission/outcome. Pending and final completion are
separate frames; final applied occurs after real renderer boundary commit.

At most4 clients,32 queued control replies/client plus one latest telemetry frame;
telemetry never interleaves with a partially written frame. Work per synthetic tick:
≤4 accepts, one decoded frame/client, ≤8KiB read/write/client and48 generated mono
frames (input01 constant0.125, others0). Parsing, sockets, heap and reply retirement
occur outside Mixer::process. The engine advances even when no clients remain.
Every16 ticks, connected snapshot-ready clients receive latest read-context snapshot.
Incomplete input and stalled output deadlines are2000ms. Overflow disconnects the
client; authority/graph never wait for a reader. Telemetry carries actual renderer
coefficients and fresh absolute frame; unsupported meters remain unavailable.

Client loss does not undo an already admitted command: final delivery may be
uncertain, so reconnect queries a fresh snapshot, obtains a fresh writer grant
(after the old scope lease expires or is released), and drops unsent/pending UI
intent. It never replays an uncertain operation under a new ID. No autorun, output
arm or scene recall. Synthetic frame advancement and wall-clock authority leases
are distinct; no realtime scheduler certification is implied.

Validation uses temporary owned Unix endpoints and own bounded child processes,
with synthetic in-memory inputs. No production sockets, media or hardware tests.

Restart identity correction: `audio.owner` is a persistent private0600 regular
file with an exclusive OS flock held for the authority lifetime. `audio.identity`
is a small private0600 strict versioned show/high-epoch record. Startup synchronously
reserves a strictly greater epoch with create-new temporary file, file sync,
atomic rename and directory sync before binding the socket. Same/regressed epoch,
changed show in that directory, malformed/unowned/symlink records and competing
startup refuse. Crash releases ownership without removing the durable high-water.
Mixer state remains volatile. Identity files must survive restart. An existing owner with a missing identity
record refuses startup, including a crash during the initial reservation. The
owner marker is synced before the first record is reserved. Corrupt records fail
closed; startup never reconstructs or resets a lost high-water. Replacing the
entire private namespace is outside this restart guarantee. A crash may leave
its socket behind; startup preserves any preexisting endpoint and refuses until
the owner explicitly removes its verified dead endpoint. An unsuccessful
startup after reservation may consume an epoch, which must never be reused.


## Channel processing

The same endpoint accepts explicitly versioned
[GP07-processing:1](CHANNEL_PROCESSING.md) snapshot and atomic channel replacement
requests. The client uses its existing C-AUDIO connection, FOH grant, revision and
request-ID sequence. Before a new edit it must query a processing snapshot within
250 ms; exact retries still return their original outcome after that snapshot
ages. Separate processing snapshots are queried explicitly; legacy GP03 telemetry
is unchanged. Final completion follows actual boundary application, with readiness
and gain reduction observed separately after the 240-frame transition.

## Configurable composition successor

The explicit configurable provider uses C-AUDIO/rendered version2 and
GP07-processing version3, with capability-derived dimensions and coherent paged
snapshots. Legacy attachment remains version1/processing2. `GP14-structure:1`
adds separately scoped PA configuration and signal-to-output patch transactions;
physical remapping requires a reviewed reopen. See
[composition](MODULAR_PROCESSING.md) and [remote endpoints](REMOTE_PROCESSING.md).
Same-UID clients remain connection-bound; disconnect revokes their writer and
cancels retained prepared changes. Source recovery durably advances the epoch,
clears clients/grants and remains muted. Provider control/I/O runs outside the
bounded Mixer/ModuleGraph render functions.
