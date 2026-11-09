# extractor_ner

Token-classification (BIO) slot extractor: a small encoder (all-MiniLM-L6-v2) fine-tuned on a synthetic corpus, exported to
int8 ONNX, run by `crates/model-worker/src/ner.rs`. It is the shipped extractor (it replaced GLiNER-small, see D33 in the
decision log); `rules.rs` / `refine.rs` are the fallback when the model folder is missing.

All commands use the SP2 venv (torch, transformers, onnxruntime, gliner), from this folder:

```
PY=../../spikes/sp2-extractor/.venv/Scripts/python.exe

# 1. corpus (no sentence of fresh.txt is read; --exclude-spans only drops pool entries equal to a fresh gold span,
#    so evaluation is entity-disjoint)
$PY -I gen_data.py --n 8000 --seed 1 --out data/synth_train.txt --exclude-spans ../../spikes/sp2-extractor/data/fresh.txt

# 2. fine-tune (CPU; 8000 sentences x 4 epochs took 32 min on a heavily shared PC, expect ~5-10 min on an idle one)
$PY -I train_ner.py --base sentence-transformers/all-MiniLM-L6-v2 --train data/synth_train.txt --out work/minilm_l6 --epochs 4

# 3. export + int8 quantise into models/ner (git-ignored)
$PY -I export_onnx.py --model work/minilm_l6 --out ../../models/ner

# 4. score the model alone, memory and latency (DEV/TEST halves of fresh.txt; aggregates only), dump raw spans
$PY -I eval_onnx.py --dir ../../models/ner --dump work/preds.json

# 5. hybrid with the Rust rules, and the live Rust runtime
NER_PREDS=work/preds.json cargo test -p jitmo-model-worker --release --test ner_eval -- --ignored --nocapture
```

Other files: `vocab.py` (entity pools), `ner_common.py` (parsing, BIO, decoding, scoring; the DEV = odd lines, TEST = even
lines split lives in `fresh_split()`), `train_shared_head.py` (head on the router's frozen int8 encoder),
`eval_gliner.py` (GLiNER baselines in the same split). Training sentences are plain `ID|text with [type|span]` lines,
`data/synth_train.txt` is generated and not meant to be committed.

Blind-set discipline: only aggregates are printed for TEST. Do not tune on TEST; use DEV (`ner_common.show_errors`).
`fresh.txt` has been read in part during earlier work (SP2), so treat even TEST as optimistic.

## Where the model files go

The model is NOT committed (`models/` is git-ignored). Step 3 above writes `model_int8.onnx` (about 23 MB), `tokenizer.json`
and `labels.json` into `models/ner/`. The worker looks for that folder in this order: `--extractor DIR`, `JITMO_NER_DIR`,
`models/ner` next to `jitmo-model-worker.exe`, `models/ner` in the working directory. Without it the extractor runs on the rules
only and logs a warning (never an error). Tests that need the model (`tests/extractor_ner.rs`, `tests/ner_eval.rs`,
`tests/split_eval.rs`) skip with a message when it is absent, so CI without models stays green; CI never needs the model files
unless someone later wants the extractor score gated there (then restore `models/ner` from a cache or release asset before
`cargo test`, and run `split_eval` with `-- --ignored`).

## Scoring through the real extractor path

```
SKIP_TEST=1 MODES=rules,ner,ner+lists,ner+rules:date_time/duration/number/setting+lists GAZ=gold \
  cargo test -p jitmo-model-worker --release --test split_eval -- --ignored --nocapture
OWN_SET=dev4_set.txt SHOW=0 MODES=rules,ner cargo test -p jitmo-model-worker --release --test split_eval -- --ignored --nocapture
```

(`MODES`, `GAZ`/`@gold`, `OWN_SET`, `SKIP_TEST` are described at the top of `tests/split_eval.rs`.) Score TEST (drop `SKIP_TEST`)
rarely and read only the aggregate. Memory and latency of the real worker: `tools/perf/measure_worker_memory.ps1`.
