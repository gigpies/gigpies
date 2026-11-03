# SHR instrument panel

Shared Ratatui widgets, original material glyphs and terminal lifecycle for Desk
and Lightdesk. This package lives in GigPies; providers and audio/lighting
algorithms do not belong here.

From GigPies, use `CARGO_TARGET_DIR=/home/shome/p/gigpies/target`,
`CARGO_INCREMENTAL=0`, Rust 1.97.1 and the shared build lock:

```
flock -xn /home/shome/p/.gigpies-build.lock env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_TARGET_DIR=/home/shome/p/gigpies/target cargo +1.97.1 test --locked --manifest-path src/instrument-panel/Cargo.toml -j1 --all-targets
```

`panel-specimen --font` requires the explicitly selected SHR Panel Mono font;
`--glyphs` exercises experimental protocol registration; no flag uses readable
ordinary Unicode. These modes use specimen data and never open providers/devices.

Author geometry with `scripts/build_panel_glyphs.py`. Optional font generation
uses Debian fontTools 4.57.0 and the licensed unmodified DejaVu Sans Mono input;
it renames the derivative SHR Panel Mono and retains the source licence metadata.
The generated font is a local deployment artifact, not proprietary artwork.

Run `python3 scripts/package_panel.py` after shared source changes to refresh
both authorized consumers' immutable-checksum Cargo directory packages. Consumers
use exact versions (Desk `=0.3.2` with keybed; unchanged Lightdesk `=0.3.1`) from the `shr-panel` source, locally replaced by `vendor/`;
there is no registry upload or sibling path dependency. The `.invalid` index is
intentionally non-networked: builds must use the included source replacement.
Run Cargo from each consumer root so it reads that root's `.cargo/config.toml`.
The package script never changes Git indexes or other dependencies.

The [shared UI contract](../../../docs/reference/INSTRUMENT_PANEL.md) describes
provenance, rendering assumptions and the workflow inventory. Acceptance belongs
to [GP-PANEL](../../../docs/development/MODULE_IMPLEMENTATION_PLAN.md#gp-panel--shared-physical-instrument-panel).
