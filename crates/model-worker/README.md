# jitmo-model-worker

Separate model process (no network): Tier 0 router and extractor.

## Model files (not in git)

`models/` is git-ignored.

- Router: `models/router/` needs `model_int8.onnx`, `tokenizer.json`, `head.json` (see `models-src/router/README.md`).
- Extractor (NER): `models/ner/` needs `model_int8.onnx` (about 23 MB), `tokenizer.json` and `labels.json`. Reproduce them with the
  commands in `tools/extractor_ner/README.md` (generate corpus, fine-tune MiniLM-L6, `export_onnx.py --out ../../models/ner`).
  To keep the files elsewhere, set `JITMO_NER_DIR` (or pass `--extractor DIR` to the worker). The app finds `models/ner` next to
  `jitmo-model-worker.exe`, so ship the folder beside the worker.
  Without the files the extractor falls back to the rules only (a warning is logged, never an error), and the model tests skip with a message.
- CI (`.github/workflows/ci.yml`) has no model files: router and NER tests skip, the testset runner uses fake connectors and no worker,
  so nothing needs to change there. If a CI job should score the extractor, restore `models/ner` (cache or release asset) and run
  `cargo test -p jitmo-model-worker --release --test split_eval -- --ignored`.

## Extractor

`ner.rs` is the shipped extractor: a MiniLM-L6 BIO tagger (int8 ONNX on ONNX Runtime) that tags the slot spans
(app, contact, date_time, duration, event, file, folder, media, message, number, place, query, setting, website). It is loaded on the
first `Extract` request only and goes away when the worker exits after idle. `extractor.rs` (`LazyExtractor`) runs it and applies the
closed-list correction from the apps and contacts the app passes in the context (a span containing a known name becomes that app or
contact, snapped to the name). If the model folder is missing or fails to load, `rules.rs` (dates, times, durations, numbers, settings,
closed lists, sentence frames) and `refine.rs` (second rule pass) answer instead (`rules::extract_v2`).
Span kinds map to skill slot types in `rules::to_slot_span` (`file` becomes `file_ref`; `message` and `query` are turned into `text`
by Core for a free-text slot); all other names are used as they are.

Numbers (D33): on the SP2 `fresh.txt` split (DEV = odd lines, TEST = even lines) sentence-exact DEV 94% / TEST 86%, about 3 ms
per request and about 95 MB for the whole worker with the extractor loaded.

Score and compare combinations through the real path with
`cargo test -p jitmo-model-worker --release --test split_eval -- --ignored --nocapture` (options in the file header; `MODES=rules,ner,ner+lists,...`).
The self-written sets `tools/extractor/dev_set.txt` to `dev5_set.txt` are sanity checks only (the rules were tuned on them): `OWN_SET=dev4_set.txt`.
Look at single sentences with `TEXT="a|b" cargo test -p jitmo-model-worker --release --test show_one -- --ignored --nocapture`.
Model-loading tests must not share a process with the router unit tests (one ONNX environment per process): the router tests share
one engine inside the library test binary, `tests/extractor_ner.rs` shares one static environment, `tests/ner_eval.rs` and
`tests/split_eval.rs` are their own binaries.
