# SP10 — Install size budget (limit: under 3 GB with all models)

## Verdict
**PASS (with a recommended config).** Recommended bundle is ~1.5 GB (about 50% of the limit). The maximum-quality config (3B planner + medium extractor) is ~2.83 GB: it technically fits but leaves no headroom, so it should be a first-use download instead.

## Numbers
M = measured (HF API / local files, 2026-11-07). E = estimate (not measured; no Windows build exists yet).

| Component | Choice | Size (MB) | Basis |
|---|---|---:|---|
| Tauri app (exe + React UI assets) | Tauri 2, release, LTO | 12 | E (typical 5-15) |
| Core + model-worker Rust binaries | 2 exes, release | 15 | E |
| ONNX Runtime | onnxruntime.dll (CPU) | 15 | E; official win-x64 zip is 82.9 MB M but includes headers/libs/pdb, runtime DLL alone ~12-15 |
| llama.cpp | ggml/llama CPU DLLs (AVX2 + fallback variants) | 20 | E |
| Embedding model (router) | bge-small-en-v1.5 int8 ONNX (Xenova) | 33.8 | M |
| Alt embedding | all-MiniLM-L6-v2 qint8 ONNX | 23.0 | M |
| Extractor | GLiNER small v2.1 int8 ONNX (onnx-community) | 183.4 | M (fp32 611, fp16 306) |
| Alt extractor | GLiNER medium v2.1 int8 | ~450 | E (fp32 safetensors is 1.5 GB locally, ratio like small) |
| Planner (low) | Llama-3.2-1B Q4_K_M | 807.7 | M |
| Planner (rec.) | Qwen2.5-1.5B-Instruct Q4_K_M | 1117.3 | M |
| Planner (high) | Qwen2.5-3B Q4_K_M 2104.9 / Llama-3.2-3B Q4_K_M 2019.4 | 2105 | M |
| LoRA adapters | 3 jobs (planner, summarizer, reply) x 10-50 MB | 30-150 | design doc 06 |
| SQLCipher | statically linked into core (rusqlite bundled-sqlcipher) | 2 | E |
| Skill files, tokenizers, signatures | YAML + tokenizer.json | 5 | E |
| WebView2 | NOT bundled (Win11 has it; Evergreen bootstrapper ~2 MB if needed) | 0-2 | Tauri docs |

Totals:

| Config | Contents | Total |
|---|---|---:|
| A. Lean | MiniLM + GLiNER-s + 1B Q4 + 30 MB adapters + runtimes | ~1.11 GB |
| **B. Recommended** | bge-small + GLiNER-s + Qwen2.5-1.5B Q4_K_M + 100 MB adapters + runtimes (~69) | **~1.50 GB** (~1.6 with 150 MB adapters) |
| C. Max | bge + MiniLM + GLiNER-m int8 + 3B Q4_K_M + 150 MB adapters + runtimes | **~2.83 GB** |

Limit 3 GB (3000 MB): B uses ~50%; C ~94%.

Caveats:
- GGUF/ONNX weights do not compress, so installer size is about the same as on-disk size. An MSI that unpacks to disk needs ~2x transiently (installer + extracted); with B this is ~3 GB of temporary disk, fine on disk but should be stated. NSIS/Tauri bundler streams to the target, so peak is lower.
- Updates: adapters are 10-50 MB, so most updates are small. Replacing the base model is a ~1.1 GB download, so it should be rare and delta-free (GGUF does not diff well).
- Gen-time memory limit (2 GB peak) is SP8, not this spike; 1.5B Q4 mmap fits, 3B Q4 (2.1 GB file) does not fit the 2 GB budget on 8 GB laptops anyway. That independently rules out C as a default.

## Decision
Ship config B. The size limit does not force any change to the plan. Keep Tier 0 (bge-small + GLiNER-small int8, ~220 MB on disk) in the installer, since it handles most requests.

## Fallback (first-use download)
If SP1-SP4 force a bigger model (GLiNER-medium, 3B) or the installer needs to be under ~1 GB for distribution:
- Installer carries Tauri app + runtimes + Tier 0 encoders only (~300 MB).
- Planner GGUF + adapters (Tier 1, ~1.2 GB) download on first need (first chained request or summary), from the signed-package CDN (06 HLD update server), Ed25519 verified, resumable, with a progress prompt in the command bar. Known-task skills work fully offline before the download.
- Cost: first chained/summary request needs network once; offline-first marketing claim must say "after one-time model download".
