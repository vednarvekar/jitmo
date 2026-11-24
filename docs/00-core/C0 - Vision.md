# C0. Vision: the runtime

18 Nov 2026 · owner: Ved Narvekar · status: **this is the core of the project**. Everything else (the jitmo assistant, its UI, its skills) sits on top of it.

## The vision in one line

**Run any number of small models, and models far bigger than the RAM you give them, on an ordinary laptop, as if RAM were unlimited.**

RAM is not avoided. RAM is used as a smart, model-aware cache. The SSD is the store. The runtime decides, every moment, which weights must be in RAM, loads them before they are needed, and drops them when they are not.

## What we are building

A new model runtime (an engine like llama.cpp or ONNX Runtime, but built around memory, not around one resident model). Working name: **jitmo runtime**. It does two things.

| | Capability | What "done" feels like |
|---|---|---|
| **A** | **Many small models, swapped fast.** Load, unload and switch between many small specialist models (router, tagger, tool-caller, writer, speech, voice). | Switching feels instant. Only the models in use cost RAM. |
| **B** | **Big models, streamed.** Run a model whose weights do not fit in the RAM budget. Only the parts needed right now (and next) are in RAM; the rest is read from the SSD just in time. | Same answers as a fully loaded model, at close to the same speed. |

## What "works the same" means (the bar we hold)

1. **Same output.** Streaming never changes the maths. Given the same model, prompt and settings, the answer is identical to a fully loaded run. Any approximation (skipping weights, guessing) is a separate, opt-in mode and must say so.
2. **Close to the same speed.** Speed loss from streaming is measured against the same model fully in RAM, on the same laptop.
3. **The PC stays smooth.** The runtime never pushes Windows into paging, never takes all CPU cores, and gives RAM back the moment Windows is short.

## Proposed targets (to confirm, see C2)

Measured on the owner's laptop (Intel i5-1135G7, 4 cores, 16 GB) and on an 8 GB laptop.

| | Target |
|---|---|
| RAM budget for the runtime | set by the user or machine; 1.2 GB on an 8 GB laptop (the existing hard rule) |
| Switch between small models (weights still in Windows' file cache) | ≤ 100 ms |
| Switch between small models (cold, from SSD) | ≤ 500 ms |
| Big model size vs RAM budget | at least 3× the budget, later 10× |
| Big model output vs full load | identical |
| Big model speed vs full load | at least 70% of full-RAM tokens per second |
| First word of an answer | ≤ 1.5 s |
| Idle | nothing loaded but what is pinned; 0% CPU |

## Principles

- **RAM is a cache, not a home.** Every byte in RAM has a reason to be there right now.
- **Predict, then prefetch.** Know what is needed next before it is needed. A load you wait for is a failure; a load hidden behind work is free.
- **Touch fewer bytes per word.** Speed comes from reading less, not only from reading faster (sparse models, experts, hot neurons, smaller numbers).
- **Model-aware, not OS-aware.** Windows paging does not know what a layer or an expert is. The runtime does, so it decides.
- **Exact by default.** Speed tricks that change answers are opt-in and labelled.
- **Measure everything.** Every claim has a number from the target laptop. If a target fails, say so with the number.
- **We will rebuild what we must.** If llama.cpp or ONNX Runtime cannot do it, we write our own parts, up to a full engine.

## What it is not

- Not the assistant. jitmo the assistant is the **first user** of the runtime, not the runtime itself.
- Not a training framework. It runs models; training stays in `training/`.
- Not cloud. Everything runs on the device.
- Not a model. We use open models and may convert them into our own file layout.

## How it relates to the existing docs

- [00.5 Models and Runtime](../00-vision/00.5%20Models%20and%20Runtime.md) describes a scheduler that loads whole models. Capability A replaces and extends it.
- [13 End-to-End Audit, section 10.2](../03-delivery/13%20-%20End-to-End%20Audit.md) marked "mixture-of-experts streamed from the SSD" as *Not now*. That verdict is **reversed**: it is now the centre of capability B.
- [15 Low-Memory Engine Brief](../03-delivery/15%20-%20Low-Memory%20Engine%20Brief.md) budget and rules still hold for the assistant.

Next: [C1 Blockers](C1%20-%20Blockers.md) · [C2 Path and First Experiments](C2%20-%20Path%20and%20First%20Experiments.md)
