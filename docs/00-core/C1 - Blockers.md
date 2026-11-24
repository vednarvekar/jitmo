# C1. Blockers: what stops the vision today

18 Nov 2026. Numbers marked *est.* are estimates to be measured (see [C2](C2%20-%20Path%20and%20First%20Experiments.md)). The rest come from docs 13, 14 and 16.

## The one hard blocker: physics

To write **one word**, a normal (dense) model reads **every weight once**. So speed is roughly:

> words per second ≈ (memory speed) ÷ (bytes of weights read per word)

| Where the weights are | Speed of reading *est.* | A 4 GB model (7B at 4-bit) |
|---|---|---|
| RAM (laptop, dual channel) | 40–60 GB/s | ~10–15 words/s |
| Fast NVMe SSD (PCIe 4) | 5–7 GB/s | ~1.5 words/s |
| Typical laptop NVMe (PCIe 3) | 2–3.5 GB/s | ~0.5–0.8 words/s |

**The SSD is 10–20× slower than RAM.** So streaming a *dense* big model from the SSD can never feel the same. No code fixes this.

**The way around it:** read fewer bytes per word. That is only possible when the model does not need all its weights for every word:

- **Mixture-of-experts (MoE) models.** Each word uses only a few "experts". Example: a 30B model that uses about 3B per word. Only the active part must be in RAM.
- **Activation sparsity.** In some models most neurons are near zero for a given word. Keep the often-used ("hot") neurons in RAM and fetch the rest only when needed.
- **Fewer bits per weight** (2-bit, 1.58-bit): fewer bytes to move.
- **Reuse across words.** Experts and neurons used for one word are often used again soon. A good cache turns most reads into RAM hits.

So the vision is reachable for **sparse** models, and for dense models only with approximations. This is the main constraint on everything below.

## Blockers, ranked

| # | Blocker | Why it stops us | How bad |
|---|---|---|---|
| 1 | **Disk is 10–20× slower than RAM** | Dense big models cannot stream at full speed (above). | Hard limit. Pick sparse models. |
| 2 | **We don't know what comes next early enough** | Which experts or neurons a word needs is decided at run time, layer by layer. To hide the SSD we must guess the next layer's needs while the current layer computes. How well this can be predicted is unknown. | Unknown. The make-or-break research question. |
| 3 | **We don't know the cache hit rate** | If consecutive words reuse the same experts, a small RAM cache works. If not, every word goes to the SSD. Never measured on our models and prompts. | Unknown. Measure first. |
| 4 | **Existing engines assume the whole model is in RAM** | llama.cpp and ONNX Runtime load the model and leave paging to Windows. They have no expert cache, no prefetch, no budget. | Must build our own memory layer (or engine). |
| 5 | **Windows paging is the wrong tool** | Memory-mapped files fault in small pages (4–64 KB), one at a time, while the CPU waits, and Windows evicts by age, not by what the model needs next. | Must do our own async reads (IoRing or unbuffered overlapped I/O) into our own buffers. |
| 6 | **Model file layout is not built for streaming** | In GGUF, the pieces we want to fetch together (one expert, one group of neurons) are not stored as single big, aligned blocks. Small scattered reads are slow on SSDs. | Need our own file layout, converted from GGUF or safetensors. |
| 7 | **Switching small models is slow today** | Measured: 1.2–2.7 s to load a 270M–0.8B model (new process, setup, warm-up). Private memory is about 2× the file (937 MB for a 508 MB file): weights copied out of the mapped file, buffers, context. | Fixable: one long-lived runtime, weights used straight from the mapped file, shared buffers. |
| 8 | **Weights are not the only memory** | The context memory (KV cache) and work buffers grow with prompt length (0.8B model: 937 MB at context 4096, 580 MB at 768). | Must be budgeted and possibly streamed too. |
| 9 | **The right models may not exist in small sizes** | Good, openly licensed MoE or sparse models that fit a laptop are few. Our benefit depends on them. | Medium. Survey and pick. |
| 10 | **Fast CPU maths is a lot of work** | If we write our own engine, matrix maths for 4-bit weights must use AVX2 / AVX-512 / VNNI to be fast. llama.cpp's kernels took years. | Large effort. Reuse ggml kernels first, rewrite only if they block us. |
| 11 | **No benchmark for the new goal** | Today's benchmarks measure tool accuracy, not streaming speed, hit rate or switch time. The SSD and RAM speed of the owner's laptop are not even measured. | Easy. First task. |
| 12 | **Solo builder, big scope** | An engine is years of work for a team. | Real. Cut scope hard: one model, one laptop, one number at a time. |

## Not blockers

- **SSD wear.** Reading does not wear out an SSD in any meaningful way. Only writing does.
- **Exactness.** Streaming changes *where* weights come from, not the maths. Identical output is achievable.
- **Prompt reading (prefill).** It processes many words per weight read, so streaming hurts it much less than word-by-word writing.

## What this means

The vision is real for **sparse models** and for **many small models**. The unknowns are blockers 2 and 3. Measure them before building anything big.
