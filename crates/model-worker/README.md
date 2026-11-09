# jitmo-model-worker

Separate model process (no network): Tier 0 router and extractor.

## Model files (not in git)

`models/` is git-ignored. For development, copy the files from the spike folders:

- Router: `models/router/` needs `model_int8.onnx`, `tokenizer.json`, `head.json`.
- Extractor: populate `models/extractor/` from `spikes/sp2-extractor/models/gliner_small_onnx/`:
  copy `onnx/model_int8.onnx` (as `model_int8.onnx`), `tokenizer.json` and `gliner_config.json`.
  To keep the files elsewhere, set `JITMO_EXTRACTOR_DIR` (or pass `--extractor DIR` to the worker).
  Without the files the extractor falls back to rules only, and the model tests skip with a message.

## Extractor

`rules.rs` (pure Rust: dates, times, durations, numbers, settings, closed lists, sentence frames; a port of
the SP2 spike, kept so it can be checked against the Python), `refine.rs` (second rule pass: more frames
for query, media, message, event and file, a shared tail-trimming step, small repairs; this is what the
worker runs after the first pass) plus `gliner.rs` (GLiNER-small int8 on ONNX Runtime). The model is loaded
on the first `Extract` request only, and goes away when the worker exits after idle. The Python parity test
checks the first pass alone (`LazyExtractor::extract_classic`); the shipped path is `LazyExtractor::extract`
(`rules::extract_v2`).

Tuning data: `tools/extractor/dev_set.txt` and `dev2_set.txt` (written for tuning; the SP2 `fresh.txt` is the
blind set and is only scored, never tuned on). Compare configurations with
`cargo test -p jitmo-model-worker --release --test ablate -- --ignored --nocapture` (`SETS=dev_set.txt,fresh.txt`),
look at single sentences with `TEXT="a|b" cargo test -p jitmo-model-worker --release --test show_one -- --ignored --nocapture`,
and sweep GLiNER label wording and per-type thresholds with `--test extractor_model sweep_gliner`. The Python reference outputs are in
`tools/extractor/extractor_reference.json` and `tools/extractor/extractor_parity.json` (regenerate with
`tools/extractor/dump_reference.py`). Score it with
`cargo test -p jitmo-model-worker --release --test extractor_model eval_sp2 -- --ignored --nocapture`.
