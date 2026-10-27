# Offline mixing

[Using GigPies](../README.md) · [Engineering](../development/README.md)

The offline tools operate on local multitrack files. They do not open an audio
interface or play a result automatically.

## Start with your sources

```sh
target/release/gigpies inspect /path/to/multitrack-folder
```

Before rendering, prepare settings that name your actual files and routing.
The bundled preset belongs to an example session; it does not identify arbitrary
instruments for you. Sources need a compatible rate and common timeline.

## Keep or change the sound

Keep a source that already works. EQ, compression, balance changes and effects
are optional choices, not mandatory stages inferred from an instrument name.
Use these command guides when you have a particular task:

| Task | Guide |
|---|---|
| Retain an existing prepared source mix | [Source preservation](SOURCE_PRESERVATION.md#use-and-recovery) |
| Measure or propose a balance change | [Balance commands](BALANCE_PASS.md#commands) |
| Prepare artistic effects | [Effects workflow](ARTISTIC_FX.md#implemented-workflow) |
| Export and check a finished file | [Delivery commands](SUMMING_DELIVERY.md#commands) |

With prepared settings, a frozen render uses:

```sh
target/release/gigpies render PREPARED.json SOURCE_DIRECTORY NEW_RENDER_DIRECTORY
```

Always choose a new output destination. Existing outputs are not overwritten.
Inspect the finished report before auditioning; an interrupted folder is not a
completed result. Listening decides whether the change helped.

The linked command references include technical detail. Earlier calibration,
unity-source and fixed headless-system experiments are under
[historical workflows](../archive/workflows/README.md), not required setup steps.
