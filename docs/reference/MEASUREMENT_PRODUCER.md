# GP20 measurement provider v1

This additive software-only control path connects reserved common-frame capture
pairs to the standalone SHR PA measurement owner. No device owner, excitation,
output route, gain, audio/MIDI/DMX action or automatic proposal application is added.
SHR PA owns analysis and candidate generation. Physical provider runners expose no
measurement enable option; acquisition/timing qualification remains separate.

## Frozen wire

Contract `GP20-measurement`, version integer1, uses the existing private same-UID
Unix audio endpoint and four-byte big-endian framing/paged replies. Every request
has exactly `contract`, `version`, `show_id`, `module`, `epoch`, `writer`, `lease`,
`request_id`, `expected_revision`, `kind`, `body`. Identity and canonical counter
strings follow C-AUDIO:2. Read requests require null writer/lease/request_id/expected_revision fields. Mutations
require the same connection's snapshot/writer/lease, PaConfiguration scope, current
revision and a measurement snapshot no older than250ms. The existing shared
begin_external/commit_external authority handles dedup/fingerprint and one boundary
reservation; this is not a synthetic lease-renew authorization side effect.

| kind | exact body |
|---|---|
| `measurement_snapshot` | `{}` |
| `measurement_result` | `{"id":"capture-or-proposal-id"}` |
| `capture_start` | `{"id":"a","reference_input":"input-01","mic_capture_slot":16,"output_index":0,"position_id":"p1","samples":32768,"options":{"max_arrival_samples":2048,"band_hz":[100,10000]}}` |
| `capture_cancel` | `{"id":"active-id"}` |
| `measurement_propose` | `{"id":"proposal-id","pairs":[["anchor-id","target-id"]]}` |

IDs are1..128 ASCII graphic bytes. Samples32768..65536; max arrival1..2048 samples;
integer-Hz band20..20000, ascending. Up to8 positions,16 retained records, one active
capture or owner job. No retries or automatic restarts. Result IDs cannot be reused
while retained. The microphone slot must belong to topology.measurement_slots and
cannot overlap any program strip; reference_input is an actual canonical strip ID.
Output index identifies an assigned output of the admitted PA v2 owner graph.

Replies contain exactly `contract`, `version`, `context`, `state`, `reason`,
`revision`, `effective_frame`, `snapshot`, `result`. Context is exactly the existing RequestContext fields: show_id, module, epoch,
writer, lease, request_id, expected_revision (no command kind). Reply states are `snapshot`, `result`, `pending`, `final`. Pending/final
boundary replies correlate exact request IDs; final reason null plus effective_frame
means accepted. It does not mean analysis succeeded. Read the record terminal state.

Snapshot fields: `available`, `software_only` (true), `reason`, `active_id`,
`capture_progress`, `results`, `current_basis`, `reserved_mic_slots`,
`reference_inputs`. Progress is null or `{id,received_samples,requested_samples,
output_index}`. Results is an array of summaries `{id,kind,state,reason}`;
kind is `capture` or `proposal`. Record states are `capturing`, `analyzing`,
`measured`, `proposed`, `no_change`, `refused`, `cancelled`, `invalidated`.
Cancellation/invalidation retain a reason and never silently erase the operation.

A detailed result has exactly `summary`, `basis`, `owner_result_json`,
`candidate_configuration_json`. Owner result is a nullable opaque string capped at
65536 bytes. Candidate is a nullable owner-validated graph JSON string capped at
49152 bytes. **All outer control JSON remains integer-only.** Finite floating-point
owner documents are decoded with a separate bounded duplicate-safe parser; shared
old control contracts are unchanged. Measured does not imply proposal eligibility;
owner_result_json preserves the PA result's eligibility and reasons.

Basis fields are `source_epoch`, `map_revision`, `graph_generation` (canonical
counter strings), `clock_domain` (`software-common-source`), `configuration_json`
(exact admitted owner graph), `program_buses` (unchanged integer indexes). Every
capture binds the actual PA status generation, exact graph/configuration/bus map,
source epoch and topology revision. Captures from different bases cannot combine.
Lease expiry, disconnect/revoke, cancellation, source discontinuity or changed basis
invalidate outstanding work/proposals; late worker results are discarded.

## Startup and execution

`gigpies-headless --measurement-owner ABS_JSON` explicitly enables the software
worker at startup, together with configured topology, reserved measurement slot and
actual module manifest/PA setup. The startup object is exactly:
`{"version":1,"mode":"software-only","owner_executable":"ABS_PATH",
"owner_sha256":"LOWERCASE_SHA256"}`. No environment search or fallback executable.
Default/old PA owners report unavailable. Current headless synthetic mic slot is
zero unless an explicit software witness injects a paired test source; it does not
invent microphone evidence or start physical capture.

The startup-created serial worker verifies the executable before spawning each
bounded job. Pipes are nonblocking, request<=8MiB, owner response<=256KiB, each child
has a5s deadline and is killed/reaped on failure/cancellation. Only the owned child
is affected. Capture has a5s control-time deadline, source copies are preallocated,
and work/completion queues are bounded. The offer path only copies at most48 pairs
and records faults; analysis, hashing, subprocess I/O and buffer retirement occur
outside it. These statements do not claim allocation-free whole-host control ticks.

PA `analyze` and `propose` operations are followed, for proposed/no-change outcomes,
by PA's owner-validated `candidate` operation. Returned exact capture metadata,
finite/bounded spectra, graph identity, sparse changes and protected-field equality
are checked before admission. Use candidate_configuration_json plus the retained
program_buses only through the existing GP14 muted whole-configuration review.
Check current basis again before offering review. Rearm remains a separate explicit
operator action. No-change does not require a redundant configuration transaction.

## Acceptance

Normal GP20 tests cover strict integer outer wire, reserved-slot admission, bounded
exact pair copying, clipping/timeline/cancellation/deadline and unavailable authority.
`gp20_owner` is explicit because it loads independently built trusted owner artifacts:

```sh
GP05_MANIFEST=ABS_MODULES GP20_OWNER_CONFIG=ABS_OWNER GP20_GRAPH=ABS_GRAPH \
GP20_CORPUS_DIR=ABS_PRIVATE_CORPUS cargo test --locked --features hardware-host \
  --test gp20_owner -- --ignored
```

Its deterministic source passes actual PA DSP with known delay/inversion into the
reserved slot, uses two positions, checks owner proposal/candidate/cancellation and
source isolation, and emits actual correlated producer exchanges. Signals are
invented; no hardware endpoints, audible output or clock qualification are involved.

### Bounded software interoperability driver

Build the `measurement-provider` example with `--features hardware-host`; this build
feature loads module libraries but the example opens no physical devices:

```text
measurement-provider PRIVATE_0700_DIR MODULE_MANIFEST OWNER_CONFIG GRAPH_JSON READY_JSON DURATION_MS
```

Duration is 1000..120000ms. READY_JSON must be a new file directly in the private
directory. It reports the same-process socket, show/epoch, owner hash and graph
basis. The fixed invented source feeds actual PA DSP; the selected PA output is
borrowed on reserved microphone slot16 with canonical reference input-01. Capture
selection changes before its first admitted frame. The witness follows explicit
GP14 graph changes, independently renders the new actual PA graph, and reports
settled output-pair differences in `READY_JSON` with extension `.final.json` on
bounded completion or SIGTERM to its owned PID. No buffer is sent to an audio device.
Use a graph with two assigned full-range outputs for this two-output test fixture;
this is not a product topology limit.

The source offer has a counting allocator regression including completion and
invalidated input. Whole-host control pumping still allocates for protocol replies,
record admission and terminal retirement. Proposal graph parsing and candidate job
preparation occur before the command boundary. Retained capture reuse/cancellation
requires the original writer and lease; another PA writer cannot reuse its evidence.

## Explicit GP20 request version 2 additions

Version 1 wire, owner capture records and old clients remain unchanged. Version 2
requests additionally admit `measurement_overlap` with the same `{id,pairs}` body,
and `measurement_eq` with `{id,captures,target}` (1–4 capture IDs; target `flat`,
`bass_shaped`, or `damped_high_frequency`). Replies retain the existing version 1
transport envelope; successor owner documents remain opaque bounded JSON.
Only explicit v2 commands enable these operations; old providers may refuse them.

Both use the same writer/lease-bound records, source/configuration basis, serial
worker, cancellation, deadlines and late-result rejection. The owner receives
C-PA-MEASUREMENT v2 operations. Overlap returns C-PA-OVERLAP-REVIEW:1 with the
original alignment proposal and predicted spectra. EQ returns C-PA-EQ-PROPOSAL:1
and, only for proposed changes, C-PA-EQ-CANDIDATE:1. EQ is never represented as a
delay/polarity change. The host checks exact basis and that only unused output
PEQ slots change within the owner contract's gain/Q/frequency bounds. Protected
configuration equality and separate GP14 muted review/commit/rearm still apply.

Physical capture remains unavailable. Calibration/sweep files are offline owner
inputs, not authenticated GP20 records; no new capture owner or automatic sweep
playback is added. Numerical contracts live in SHR PA `docs/OFFLINE_SETUP.md`.

[PA setup software acceptance](../archive/pa-setup-2026-10-29.md) records the v2 owner/Desk workflow.

## Imported calibrated sweep review (explicit v2)

`measurement_offline_eq` uses GP20 request version 2 with exact body
`{"id":"review-id","evidence_id":"catalog-id"}`. This requests analysis only.
The provider delegates the complete existing `C-PA-MEASUREMENT:2 eq_sweeps`
request to SHR PA, then its existing `eq_candidate` operation. It implements no
sweep extraction, calibration interpolation or fitting. The source files are
operator-supplied offline evidence, **not authenticated GP20 captures**.

Startup configuration version 2 adds `offline_evidence`, at most four entries:
`{"id":"catalog-id","path":"/absolute/request.json","sha256":"lowercase SHA256"}`.
Version 1 remains unchanged and cannot contain a nonempty catalog. Each entry is
a regular, non-symlink file of at most 1 MiB; its exact bytes must match the hash.
Files are read once at explicit startup, duplicate-safe decoded, and retained
immutably in memory. Unknown IDs refuse. No file paths are accepted from Desk,
no runtime reload occurs, and no filesystem work occurs on the source offer path.
The existing 64 KiB integer-only control frames and bounded serial worker remain.

Each file is a complete owner `eq_sweeps` request with one to four complete
`C-PA-SWEEP-RESULT:1` documents, calibration, exact base configuration and revision.
The file configuration/revision must match the current actual owner; changing
configuration requires new evidence, never silently rebasing old measurements.
Imported source epoch, map, clock domain, reference tap, offset and first frame are
preserved. SHR PA validates common origins and calibration consistency across
positions. These declarations and file hashes establish identity, not physical truth.
The host separately binds the review job to current source/map/graph/program-bus
identity and the writer lease. Authenticated capture proposal commands cannot use
an imported result as a capture record.

Results have kind `offline_proposal`, retaining the evidence ID/hash in their
reason and the owner calibration and original capture metadata in the EQ document.
Old clients may refuse this explicit successor kind; no fallback relabels it.
Current Desk displays imported origin/calibration and includes them in the final
muted GP14 review. No-change/refused results carry no candidate. Pending-job
cancellation uses the existing explicitly confirmed `capture_cancel` command;
revocation, disconnect, stale basis and late-result rejection use existing fences.
No analysis request applies or rearms. GP14 commit remains explicit and muted;
physical verification and rearm remain separate operator actions.

For reproducible invented integration input, SHR PA's
`offline-review-fixture GRAPH_JSON REVISION` example emits a complete owner request.
It models a half-gain path with 23 samples arrival and a declared +6.0206 dB
magnitude correction, preserving a distinct invented timing origin. Use a second
copy with mismatched calibration between positions for refusal checks. Keep generated
files private, hash them into the startup catalog, and remove them after validation.
The opt-in Desk `actual_keyboard_imported_sweep_review_to_muted_owner` driver expects
catalog IDs `imported` and `refused` and uses the bounded provider above.
