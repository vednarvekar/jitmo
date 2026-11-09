# tools

Scripts and fixtures used to build and check the models. Nothing here ships in the app.

| Folder | What it is |
|---|---|
| `router/` | Trains the router head (`train_router.py`), the training data, the held-out set and the metrics. See its README. |
| `extractor/` | The evaluation sets for the extractor: the self-written sets (`dev_set.txt` to `dev5_set.txt`) and `fresh_gold_fixes.txt`, used by `crates/model-worker/tests/split_eval.rs`. |
| `extractor_ner/` | Generates the training sentences and trains and exports the MiniLM tagger that is the extractor. See its README. |
| `perf/` | `measure_worker_memory.ps1`: working set and private memory of the model worker. |

Model files themselves are not in git. The trained router head is in `models-src/router/`; the encoder files come from the spike folders (see `models-src/router/README.md`).
