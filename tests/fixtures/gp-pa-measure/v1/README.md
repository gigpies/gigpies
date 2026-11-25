# GP-PA-MEASURE capture/excitation corpus v1 (producer)

Generated from owner code by `tests/pa_measure_session.rs::write_fixture_v1` and
checked by `fixture_v1`. It contains `GP-PA-MEASURE-CAPTURE:1` plans (four-repeat
calibrated and single uncalibrated), strict refusals (float level, above the
−30 dBFS bound, unconfirmed audible output, unknown field), terminal records of
restored complete captures, and generator samples at fixed indexes. SHR PA keeps
byte-identical copies of `record-captured.json`, `record-single.json` and
`excitation-samples.json` and checks that its sweep owner accepts the capture
metadata and reproduces the samples. Invented identities; no audio. Change only
through a reviewed new version.

| File | SHA-256 |
|---|---|
| plan-repeated.json | df867f0f70e2b64b2829cb851f22ab2d85dc1983bd9ee251d9c2829f2b4cf0ab |
| plan-single.json | 2ab3ff72202febc0c5e771b345080f36fd06ab702e80e2c1eb42f313a42314e6 |
| reject-float-level.json | 6dafed17572b682e288bb5a9408d8eeeb15766541aabf16dee8bde93ab1b0af4 |
| reject-too-loud.json | 5831543e0cc49016cc700c8bbbcd512dc1122202795104c1ba910b49a73a4cd1 |
| reject-unconfirmed.json | 2037fe8bb71759bd68110bed9cb4cb8aa2f2830adefd3a2edece61660ef3ecf4 |
| reject-extra-field.json | dde36ee2a28215199651ffd7d75cd299609cbb1c2668b3594bef7df5bd14bbba |
| record-captured.json | a7282fb4489c9f8e96be59ad05e1285c3def08bc908821bd8a6f4038c00a6971 |
| record-single.json | 9902c47fcdb5871b9bc20b9ccea627f3d9f2deb95508099efa7578d9b0faf9eb |
| excitation-samples.json | 33bfc93bf45147e36046415c651e24f0806377fb303dfa6858bc397fea3d293d |
