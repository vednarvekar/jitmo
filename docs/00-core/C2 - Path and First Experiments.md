# C2. Path and first experiments

18 Nov 2026. How we beat each blocker in [C1](C1%20-%20Blockers.md). Order matters: measure before building.

## The approach in one paragraph

Build our own **memory layer**: a budget manager, an SSD reader, a model-aware cache and a predictor that prefetches. Run the maths on existing kernels (ggml, from llama.cpp) at first, so we spend our effort on the new part. Replace kernels or write a full engine only when they block a target.

## Parts of the runtime

| Part | Job | Beats blocker |
|---|---|---|
| **Budget manager** | Knows every byte in use; never goes over; gives RAM back when Windows is short. Partly exists in `crates/llm` (1.2 GB Job Object cap, free-RAM check). | 7, 8 |
| **Model store** | Our own file layout: each expert / neuron group / layer is one aligned block, with an index. Converted from GGUF or safetensors. | 6 |
| **SSD reader** | Async, unbuffered, many reads in flight (Windows IoRing or overlapped I/O), straight into our buffers. | 5 |
| **Cache** | Keeps hot weights (attention, shared experts, hot neurons) pinned; keeps recently used experts; evicts by "needed next", not by age. | 3, 4 |
| **Predictor** | Guesses the next layer's experts or neurons while the current layer runs, and starts the reads early. | 2 |
| **Model pool** | One long-lived process holds many small models; switching is a pointer change if the weights are cached, a fast read if not. | 7 |
| **Kernels** | 4-bit / 8-bit matrix maths on CPU (AVX2, AVX-512, VNNI). ggml first. | 10 |

## Phases

| Phase | Goal | Done when |
|---|---|---|
| **0. Measure the ground** | Real RAM and SSD speeds on the owner's laptop (sequential and random, block sizes 64 KB to 4 MB, queue depth 1 to 32). Baseline: an MoE model in plain llama.cpp with memory mapping, under a 1.2 GB cap. | A table of numbers in this folder. |
| **1. Trace the experts** | Log which experts every layer picks, word by word, on real prompts. Simulate caches of different sizes and predictors. | We know the hit rate and prediction accuracy. **Go or no-go for capability B.** |
| **2. Fast small-model swap (capability A)** | Long-lived pool, weights used straight from the mapped file, no repack, shared buffers. | Switch ≤ 100 ms warm, ≤ 500 ms cold, measured. |
| **3. Streaming prototype** | Our store + reader + cache + predictor around ggml kernels, one MoE model. | Identical output to full load; speed vs full load measured. |
| **4. Close the gap** | Better predictor, smarter cache, fewer bits, hot-neuron split for dense models, our own kernels if needed. | Hits the C0 targets. |
| **5. Plug into jitmo** | The assistant uses the runtime for all its models. | The assistant's budget in doc 15 holds with the runtime. |

## First experiments (this week)

1. **SSD and RAM speed** on the owner's laptop. Without this every number in C1 is a guess.
2. **Pick 2–3 sparse test models**: a small MoE (for example OLMoE 1B-7B or Qwen MoE), one larger MoE (for example Qwen3 30B-A3B or gpt-oss-20b), and one dense model as the control. Check licences.
3. **Baseline**: run each fully in RAM and memory-mapped under a 1.2 GB cap; record words per second and peak memory.
4. **Expert trace**: dump routing decisions on 100 real prompts; compute cache hit rate for cache sizes 10–50% of the experts.

## Decisions waiting on the owner

| Decision | Recommendation |
|---|---|
| Where the runtime code lives | Its own repo, `V:\Code\jitmo-runtime` (folder already exists, empty) |
| Reuse ggml kernels or write our own from the start | Reuse first; the new idea is the memory layer, not the maths |
| Developer framework for others, or internal engine for jitmo | Internal first; open it up once it beats the targets |
| Approximate modes (skip weights) allowed? | Only opt-in and labelled; exact by default |
| Confirm the C0 targets | Confirm or change the numbers in C0 |
