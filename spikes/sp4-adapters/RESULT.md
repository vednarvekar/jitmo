# SP4 result: can LoRA adapters swap fast on a CPU?

**Verdict: PASS WITH CAVEATS on the swap itself** (swap under 1 ms against a 200 ms target; no per-token slowdown seen). Memory flatness is plausible but **not cleanly proven**, and everything was done with untrained random adapters on a busy machine. Runs 7-8 Nov 2026.

## Setup
- Base: Qwen2.5-1.5B-Instruct Q4_K_M via llama-cpp-python 0.3.36 (prebuilt CPU wheel, Windows venv `.venv`, Python 3.12), 4 threads, ctx 512. The base GGUF lives in `..\sp3-planner\models\`.
- Adapters: 3 **randomly initialised, untrained** LoRA GGUFs (`make_adapters.py`; rank 8/8/16, q and v projections, all 28 layers, 4.4/4.4/8.7MB). Valid for swap time and memory only; they teach nothing. Generations differ per adapter, so they are really applied.
- `bench2.py` (this round, data `result_r2.json`): (1) 500 hot swaps rotating a/b/c, each followed by a 1-token decode so the adapter enters the compute graph; (2) interleaved no-adapter vs adapter 8-token generations to get per-token overhead; (3) 100 cold cycles (load from disk, apply, decode, detach, free). Memory is sampled as USS (private bytes) and RSS. The earlier round (`bench.py`, done in WSL) agrees on swap time.

## Numbers
| Metric | Result | Target |
|---|---|---|
| Hot swap call (500 swaps) | median 0.05 ms, mean 0.08, p95 0.19, max 0.85 ms | < 200 ms |
| Cold: load adapter from disk + apply (100 cycles) | median 7.7 ms, p95 12.5, max 14 ms | < 200 ms |
| First-time adapter load | 25 / 25 / 30 ms | - |
| Per-token cost, no adapter vs adapter (interleaved) | 156 vs 155 ms/token (ratio 0.99) | no big slowdown |
| Memory per resident adapter | about +6 MB USS each (+17 MB for all 3) | - |
| Hot swaps, USS first -> last | 728 -> 808 MB (+81 MB), range 726-835 MB | flat |
| Hot swaps, RSS first -> last | 1675 -> 1755 MB, range 1673-1782 MB | flat |
| Cold cycles, USS first -> last | 709 -> 732 MB (+23 MB), max 790 MB | flat |

## How to read the memory rows
RSS is dominated by the memory-mapped 1.1GB model (pages come and go under OS pressure), so I also report USS. Over 500 swaps USS rose about 80 MB and over 100 cold cycles about 23 MB. That is small against 1.5-1.8GB, and most likely just model and KV pages being touched or swapped in on a loaded box, but I recorded only every 10th sample and did not run a long enough series to rule out a slow leak. The earlier round showed +17 MB over 30 cold cycles and a -511 MB RSS "drift" from OS paging, so RSS alone is unreliable here. Call it "no evidence of a leak, not demonstrated flat to within a few MB".

## Caveats
- Machine at 85-100% CPU throughout (other agents' ML job, rustc, Chrome). Absolute ms/token (156, about 6 tok/s) is pessimistic; the swap-call times and the no-adapter-vs-adapter ratio are less affected because the comparison is interleaved.
- Random adapters on only q and v with rank 8-16 are a small LoRA. A real adapter touching all linear layers (or rank 64) adds more low-rank matmuls at compute time (llama.cpp applies LoRA unmerged), so the per-token overhead will be higher than the 0-1% seen; re-measure with the real trained adapter. Swap cost stays O(1).
- llama.cpp logs "lora cannot use buft CPU_REPACK, fallback to CPU" so adapter tensors sit in plain memory; works, slightly less optimised.
- Nothing here says whether an adapter improves router/extractor/planner quality (that is a separate training question, relevant to SP2's and SP3's shortfalls).
- Single-thread-of-control test only: no swap while a generation is in flight, no concurrent requests.

## Decision
Keep the plan: **one base GGUF plus hot-swappable LoRA adapters**; hot swap is effectively free and a cold load from disk is about 10 ms, so adapters can be loaded on demand and not all kept resident. Before committing: (1) repeat with a real trained adapter, (2) run a longer 2,000+ swap soak on a quiet machine with full memory sampling, (3) check the per-token overhead then. Fallback per doc 5 if overhead is too high: merge each adapter into its own small GGUF and keep one loaded at a time (about 1.1GB each, so only 1-2 fit under the 2GB budget).
