# 18. Model bake-off (28 Nov 2026)

Which small model should pick jitmo's tools and fill their values? Measured on the owner's laptop (i5-1135G7, 16 GB, Iris Xe, Windows 11).

## Setup

`jitmo-llmbench`, held-out set (262), candidates = fine-tuned encoder's top 3 (recall@3 97.8%), digit grammar, compact pick prompt, 4 threads, ctx 1024, batch 256, llama.cpp b12606. All models **untuned** except the 270M. "Values" = typed arguments, right tool given (73 requests, 99 values).

## Results, CPU

| Model | Tool pick | Values (requests / values) | Pick p50 | Peak RAM |
|---|---|---|---|---|
| Gemma 3 270M, tuned (shipped) | **77.5%** | 89.0% / 91.9% | **134 ms** | 0.5 GB |
| Gemma 3n E2B | 72.9% | 86.3% / 87.9% | 966 ms | 2.9 GB |
| SmallThinker 4B-A0.6B (Q4_0), `--cache-ram 256` | 72.5% | 89.0% / 90.9% | 303 ms | 4.6 GB working set, 2.4 GB private |
| Granite 4.0-H Micro, `--cache-ram 256`, 3 threads | 71.8% | **97.3% / 98.0%** | 1379 ms | 3.8 GB working set, 2.0 GB private (8.2 GB with the default cache) |
| LFM2 2.6B | 45.0% | 65.8% / 72.7% | 872 ms | 2.8 GB |
| Gemma 3 1B | 33.6% | 68.5% / 73.7% | 474 ms | 1.1 GB |
| Qwen3.5-2B, `--cache-ram 256` | 26.7% | 87.7% / 89.9% | 631 ms | 2.2 GB |
| LFM2 1.2B | 11.8% | 72.6% / 77.8% | 637 ms | 1.3 GB |

The encoder alone picks 84% (doc 16, row 13). No language model beats it at picking.

## Results, Vulkan (Iris Xe)

| Model | Pick p50 CPU → GPU | Peak RAM on GPU |
|---|---|---|
| Gemma 270M tuned | 134 → 79 ms | 0.9 GB |
| Gemma 3n E2B | 966 → 772 ms | 3.7 GB (private 4.1 GB) |
| LFM2 1.2B | 637 → 348 ms | 1.2 GB |
| LFM2 2.6B | 872 → 438 ms | 2.0 GB |

Same accuracy. The iGPU shares system RAM, so GPU memory counts against the budget. Not worth it on 8 GB; the runtime agent found the same.

## Runtime: in-process engine vs llama-server

`jitmo-audit`, held-out, Gemma 270M on jitmo-runtime: **175/262 on both**; 157.6 s (llama-server) vs **48.4 s** (`engine = "inproc"`).

## Findings

1. **Pick with the encoder, not the language model.** Use the language model for values only.
2. **Granite 4.0-H Micro is the best value-filler, untuned** (97% vs 89%). Worth a fine-tune and a RAM check.
3. **The Qwen3.5 "7.5 GB blow-up" was llama-server's prompt cache** (`--cache-ram` default 8192 MB; hybrid models save large states). `--cache-ram 256` takes Qwen from 8 GB to 2.2 GB with the same score. The old local Qwen3.5-2B file was also truncated (deleted).
4. **Untuned models don't fit the compact prompt.** LFM2 answers "0 = no tool" on most cases; picks of untuned models are not a fair measure.
5. **The in-process engine is 3.3× faster** with identical results.

## Open

- Granite: `--cache-ram 256` alone works (same score, 2.0 GB private). Adding `--ctx-checkpoints 4` made the server die after ~150 cases; don't use it.
- SmallThinker 4B-A0.6B: as good as the 270M on values (89%), not better; 2.4 GB. Not worth it.
- Next: fine-tune Granite (or Gemma 3n) for values; route values to it, picks to the encoder.
