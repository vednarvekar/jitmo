# Router model sources

`models/` is gitignored, so the pieces the Tier 0 router needs are assembled from two places:

| File | Where it lives | Notes |
|---|---|---|
| `head.json` | **committed here** (`models-src/router/head.json`, about 200 KB) | The logistic-regression head: `classes`, `coef`, `b`. Written by `tools/router/train_router.py`. |
| `model_int8.onnx` (23 MB) | `spikes/sp1-router/models/model_int8.onnx` (not committed) | The frozen `all-MiniLM-L6-v2` encoder, dynamically quantised to int8. Source weights: Qdrant/all-MiniLM-L6-v2-onnx (`model.onnx`), quantised with `onnxruntime.quantization.quantize_dynamic(..., QuantType.QInt8)` (`spikes/sp1-router/final2.py int8` does this). |
| `tokenizer.json` | `spikes/sp1-router/models/tokenizer.json` (not committed) | Same Hugging Face repo. |

Assemble `models/router/` (what `crates/model-worker` loads) with:

```powershell
pwsh tools/router/fetch_models.ps1                       # uses spikes/sp1-router/models in this checkout
pwsh tools/router/fetch_models.ps1 -SpikeDir D:\path\to\spikes\sp1-router\models   # another checkout
```

The encoder files never change when the head is retrained; only `head.json` does. Tests that need
the models skip with a message when `models/router/model_int8.onnx` is missing.

To retrain the head see `tools/router/README.md`.
