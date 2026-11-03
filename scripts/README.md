# Maintained scripts

These are reviewed reusable tools. `publication-policy.json` is the explicit
publication list; `check_publication.py` checks Git contents before publication.
Put one-off session/render runners and generated results under ignored `artifacts/`.

| Scripts | Purpose |
|---|---|
| `analyze_mix.py`, `bass_evidence.py`, `drum_evidence.py` | Optional analysis and summaries of supplied audio or saved measurements |
| `balance_excerpts.py`, `listening_checkpoint.py` | Exact parent-sample excerpts and verified listening checkpoints |
| `verify_delivery.py` | Independent final PCM true-peak verification with local FFmpeg/libsoxr; no playback or audio output |
| `ensemble_review.py`, `snare_context.py` | Descriptive ensemble and source-context review |
| `spectral_balance.py` | Explicit formula-driven spectral proposals from saved production observations; no reference recordings or playback; [contract](../docs/guides/BALANCE_PASS.md) |
| `preset_experiment.py` | Explicit historical preset experiments; no automatic media downloads |
| `snare_bleed_evidence.py`, `snare_temporal_evidence.py` | Opt-in saved-evidence audits; no processing selection or playback |
| `play_pair.py` | Explicit two-clip playback with preflight; never run by normal tests with a real player |
| `check_scene_director.py` | Opt-in finite synthetic scene/recurrence/authority/recovery episode; explicit executable paths and actual Lightdesk presentation, null/disarmed |
| `check_music_analysis.py` | Opt-in real synthetic Tap → Lux → Lightdesk music/source-loss integration; explicit executable/helper paths, null output and no hardware |
| `check_publication.py` | Git index/outgoing-history checks for private/generated content |
| `../.github/actions/ci-alerts/notify.py`, `test_ci_alerts.py` | Shared GitHub CI failure/recovery notifier and isolated API regressions; see [CI alerts](../docs/development/CI_ALERTS.md) |
| `test_*.py` | Fast synthetic regressions; normal suite, no physical audio devices |

`analysis-requirements.txt` contains optional plotting/analysis dependencies.
Normal Python tests use the standard library:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
```

See [publication rules](../docs/development/PUBLICATION.md) before adding or changing scripts.

Build `examples/music-analysis-source.rs` with the normal shared build reservation,
then invoke the music check with absolute `--source`, `--lux`, `--lightdesk`,
`--lux-check-helper` (Lux's reviewed `scripts/check-lux-service.py`) and an ignored
`--evidence` output path. It uses the production analysis Tap/publisher, generated
PCM24 and bounded owned children; it never runs in the default suite.

The scene episode adds the source example's optional `scenes` argument. Its
calibration, quiet arrangement, build, sustained drive, quiet return, contrasting
arrangement and drive return are PCM changes only: no scene or section label is
injected into Lux. `check_scene_director.py` requires those four explicit
executable/helper paths plus `--role-provider` for the existing GP09 broker and
ignored absolute `--evidence` and `--preview` paths. The role request uses explicit
synthetic display/controller descriptors; it performs no device discovery.
It runs for about 100 seconds; it is an explicit software integration check,
not part of the default suite or a physical/load qualification.
