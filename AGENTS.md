# GigPies working agreements

Read README.md, docs/STATUS.md and the relevant owning document before changing code.
Build part by part, starting with offline band automixing. Ask questions when a concrete
next step needs an answer; do not front-load the whole project's unknowns.

The system is modular across GigPies and the related `../shr-*` projects. Follow
the ownership map in docs/architecture/COMPONENTS.md: PA processing, measurement and alignment
are developed in `../shr-pa`, with the finished PA module intended for integration
here. Keep module tasks and algorithms in their owning project; avoid parallel
implementations in GigPies. Track integration and hardware acceptance separately.

## Product capacity and clock direction

The processing engine includes modular PA processing, alongside channel DSP,
mixing, buses, recording and transport. Include the owning PA module when a task
requires PA behavior; repository boundaries do not remove that product requirement.
The reference UMC1820 + ADA8200 setup targets 16 analog inputs and 18 analog outputs
at 48 kHz. Sixteen inputs is the minimum product target; 32 and 48 are growth
targets, not ceilings or claims of current hardware qualification. The original
output plan has flexible PA/monitor allocation, including six/eight PA outputs
and future matrix arrangements; do not hard-code a product monitor count.
Derive runtime dimensions from validated configuration and advertised capabilities.
Keep finite, justified realtime/resource/protocol bounds, but never promote a test
fixture, UI bank, legacy ABI or implementation slice into a universal product cap.
Use the soundcard clock as the reference and keep the ADAT expansion and all audio
paths on the same clock domain/source-frame timeline. A clock-source picker is not
required. Distinguish proven physical synchronization from software timing tests.

## Implementation and publication

Use Rust 1.97.1, edition 2024, and committed Cargo.lock. Keep sibling repositories
read-only unless the user explicitly authorizes changes there. Avoid path dependencies.
Distinguish planned, implemented, offline-validated and hardware-verified behavior.
Update focused documentation alongside behavior. Preserve original drafts in docs/archive.

Keep third-party recordings, source archives, private sessions and generated audio out
of Git and releases. Preserve local source notices. No automatic media downloads in CI.
No audio, MIDI, DMX, service or host-audio changes without authorization for that session.
Live audio callbacks must have bounded work and no allocations, locks or I/O.

The agent owns test selection. Keep fast production unit, contract, schema, safety,
recovery and focused regression tests in the default suite. Run focused tests during
implementation; run the complete normal suite for engine, rendering, shared schemas,
routing/persistence, concurrency, safety or broadly reused changes and before releases.
Historical research, auditions, exhaustive matrices, long benchmarks and disposable
renderers are opt-in once their evidence is recorded. Run them only when directly
relevant or requested. Reclassify slow one-time tests found in scoped work and document
their on-demand command. Report run and intentionally skipped classes.

Before committing inspect live Git state and staged content; preserve unrelated edits.
Never publish private audio or state. Publication follows the user's authorized scope.
Follow docs/development/PUBLICATION.md. Enable the versioned hooks when absent and run the
publication guard against the complete index. New scripts need a reviewed entry in
scripts/publication-policy.json; private user data and one-off runners stay ignored.

Task0015 clock direction: Brain has one local duplex sound-card owner for operator
talkback capture and monitor playback. Stagebox plus its ADAT expansion remains
the processing reference; Brain local I/O has a separate device epoch/timeline.
Both new crossings require bounded ASRC. Keep FX, raw REC and source analysis on
Stagebox frames. Desk controls the owner without opening PCM. Physical activation,
clock lock, socket mapping and acoustic qualification remain separate acceptance.

Keep product docs focused on current behavior, usage and limits. Use the folders
and indexes in docs/README.md; completed plans, prompts, handoffs and study results
belong in docs/archive/. Avoid appending work logs to current product pages.

## GigPies task tracking

Use `docs/development/MODULE_IMPLEMENTATION_PLAN.md` for GigPies work owned here. Keep each task plan,
implementation state, acceptance checklist, evidence and next action in the same
card; update it with the change. Shared integration tasks have one card in
GigPies, linked from contributor plans. STATUS, maps, handoffs and knowledge notes
route to task owners or preserve dated evidence; never mirror current task state.
Archive closed cards once; keep the active queue limited to open work. Reference
projects do not become GigPies runtime modules merely because code is reused.

## Reader-facing documentation

Keep README short and visual. Route musicians through the operating guide;
assume musical knowledge. Put protocols, implementation detail, validation and
work logs in engineering references or archives. Update the existing owning
guide instead of appending feature histories to the README.
