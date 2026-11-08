# tools

Scripts and fixtures used to build and check the models. Nothing here ships in the app.

| Folder | What it is |
|---|---|
| `router/` | Trains the router head (`train_router.py`), the training data, the held-out set and the metrics. See its README. |
| `extractor/` | `dump_reference.py` writes the Python reference outputs for the GLiNER extractor; the two JSON files are the fixtures the Rust tests compare against. |
| `perf/` | `measure_worker_memory.ps1`: working set and private memory of the model worker. |

Model files themselves are not in git. The trained router head is in `models-src/router/`; the encoder files come from the spike folders (see `models-src/router/README.md`).
