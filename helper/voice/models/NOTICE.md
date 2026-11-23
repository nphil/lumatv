# Wake word models

`okay_nabu` and `hey_jarvis` are microWakeWord v2 models (a TensorFlow Lite model and its JSON manifest), published by the ESPHome
project at <https://github.com/esphome/micro-wake-word-models> under the Apache License 2.0. They are the files LumaShow's
`wake-word` crate tests against, copied unchanged, and are compiled into the engine (`src/models.rs`).

The cutoffs in the manifests are the published ones (0.97 for both); the engine's three sensitivities are set against them
(`strict` 0.99, `normal` 0.97, `lenient` 0.85).

openWakeWord models are not used: their licence (CC BY-NC-SA 4.0) does not allow it.
