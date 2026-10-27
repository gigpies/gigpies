# Source, artwork and media

The Rust foundation, code-authored SVG system diagram and archived SVG banners
are original project work under MIT, including the 0.2.3 dual-console revision.
Their depicted console screens/controllers are schematic original
artwork, not manufacturer screenshots. The original v0.1.0 foundation copied no
sibling code; later DSP adaptations are attributed below. Dependency versions are locked in Cargo.lock;
upstream dependency licences continue to apply.

- hound: Apache-2.0 WAV library.
- serde and serde_json: MIT OR Apache-2.0 serialization libraries.
- rtrb 0.4.0: MIT OR Apache-2.0 bounded single-producer/single-consumer queues.
- socket2 0.6.5: MIT OR Apache-2.0 socket configuration.
- Transitive dependencies: see each package's licence metadata in the locked graph.

`docs/assets/gigpies-concept.png` is AI-generated concept artwork supplied by the
project owner. It is preserved as a concept reference, with limitations documented in
`docs/archive/studies/CONCEPT_REVIEW.md`. Third-party names and marks belong to their owners; no
endorsement or ownership of those marks is claimed.

`docs/assets/gigpies-hero.png` is the replacement photorealistic hero generated
with the built-in image generation tool on 2026-10-04. It uses the owner-supplied
concept and the projects' original offline Desk/Lightdesk screen drafts as visual
references. It depicts an intended setup, not a photographed hardware test.
The exact prompt is `docs/assets/gigpies-hero-prompt.txt`; limitations are in the
[visual review](docs/archive/studies/CONCEPT_REVIEW.md). Rendered manufacturer names and device
designs imply no endorsement or verified model/mapping. No manufacturer photograph
or manual was downloaded or bundled for this revision.

Hero PNG SHA-256: `0ee023cd70312a9c3820ad11960525dbfe99114f8a5bf1a5b7ba68dd8d34aa9b`.

The Complainiacs / Dark Ride recordings are local educational experiment material.
Their source notices remain with local files. They are not covered by this project's
MIT licence and are not included in Git, tests, CI, packages or releases.

## Offline automixer adaptations

`src/automix/dsp.rs` adapts normalized transposed-direct-form biquads, HPF/bell
coefficient equations and soft-knee compression from:

- [SHR PA](https://github.com/gigpies/shr-pa), `src/dsp.rs`, inspected commit
  `ccc816a97f1957549a2a515d94a672dc78c59012`.
  [Preserved MIT licence](licenses/shr-pa-MIT.txt).
- [SHR DAW](https://github.com/PaolaShultz/shr-daw), `src/effects/compressor.rs`,
  `eq.rs`, `channel.rs`, inspected commit `c3d2e6517cc60c7a75a319655246a54130437875`.
  Stereo maximum detection and attack/release smoothing inform our linked compressor.
  [Preserved MIT licence](licenses/shr-daw-MIT.txt).

SHR DAW gate/mixing and SHR PA metering/offline rendering were reviewed as references.
SHR FX `src/dsp.rs` at `9b9a2a94f8fe79389fba03d065924c0dd335599f` was inspected
for effects/send-return structure during the first pass.
The subsequent effects pass adapts its `src/dsp.rs` Ring/Tap/allpass/comb reverb and
chorus implementations into `src/automix/fx_engines.rs`, and its four-times-oversampled
`src/exciter.rs` into `src/automix/exciter.rs`, including harmonic/alias tests. Static
configuration and local type paths replace host controls; unused reset paths are omitted.
[Preserved SHR FX MIT licence](licenses/shr-fx-MIT.txt).
The new delay, routing, wet-return calibration and deterministic spectral review are
GigPies implementations; no sibling checkout is changed or linked.
The gain estimator, routing, file workflow and preset choices are GigPies work.
Manufacturer guidance and the loudness standard are linked in [AUTOMIX](docs/archive/workflows/AUTOMIX.md).

## Offline true-peak measurement

The windowed-sinc estimator in `src/automix/true_peak.rs` is original GigPies code,
following the oversampling guidance of ITU-R BS.1770-5 Annex 2. It does not copy
the standard's example FIR table or add a library dependency. The independently
invoked FFmpeg/libsoxr and SciPy verification tools retain their upstream licences
and are not vendored or downloaded by CI. See [method and references](docs/guides/SUMMING_DELIVERY.md).

The shelf coefficient equations follow Robert Bristow-Johnson's mathematical
[Audio EQ Cookbook, published by W3C](https://www.w3.org/TR/audio-eq-cookbook/).
Our implementation fixes shelf slope to S=1. No manufacturer DSP code, firmware,
manual or preset collection is included. See [reference-use boundaries](docs/archive/studies/PRESET_EXPERIMENT.md).

## Brain asynchronous sample-rate conversion

Rubato5.0.1, copyright HEnquist and contributors, is used under its
MIT OR Apache-2.0 licence. The published crate and committed Cargo.lock pin the
implementation. Its asynchronous sinc path supplies preallocated variable-ratio
conversion; no owner PA/FX/REC algorithms are copied.
[Official source and licences](https://github.com/HEnquist/rubato/tree/v5.0.1) ·
[versioned API](https://docs.rs/rubato/5.0.1/rubato/).
