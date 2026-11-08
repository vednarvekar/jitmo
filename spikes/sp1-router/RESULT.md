# SP1 result: can a tiny router pick the right task?

**Verdict: CONDITIONAL PASS (borderline).** Accuracy clears 85% on the original held-out set (92% single-task) but drops to 81% on a fresh, more varied set written for this re-run; pooled over both it is 86% +/- 4.5. Speed passes comfortably; memory passes only with int8. Re-run on Windows, 8 Nov 2026. Treat as "works, needs real-user data and a confidence fallback", not as proven.

## Setup
- Router = frozen `all-MiniLM-L6-v2` encoder (ONNX, mean-pooled, normalised) + logistic-regression head (C=30) over 52 labels (A1..S5 plus OUT = not a V1 task). Chained requests are split at "and / then / commas"; the split is used only if every part has top-1 prob >= cutoff, no part is OUT and the parts differ. Code: `gen_data.py`, `run.py`, `final.py`, `final2.py` (this run), `bench_runtime.py`.
- Train: 5,300 template-generated rows (`gen_data.py`, seed 7, regenerated; the 5,088 left after removing exact duplicates of any eval sentence). No real user text.
- `data/val.txt` (122) and `data/test.txt` (129) were missing from the Windows copy; I recovered the originals from the WSL tree (`/home/vedvn/ve/spikes/sp1-router/data`). `val2.txt` (119) and `test2.txt` (133) are NEW sentences written for this re-run with looser phrasing (slang, ellipsis, indirect requests, more OUT cases). test2 was written before any run and scored once. Cutoff picked on val+val2 only (0.8).
- fastembed was dropped in favour of a direct onnxruntime + tokenizers embedder (fastembed imports the Unix-only `resource` module); same ONNX weights (Qdrant/all-MiniLM-L6-v2-onnx).

## Accuracy (exact label-set match; ONNX fp32 / int8)
| Set | All | Single-task | Chains |
|---|---|---|---|
| train (first 600, seen) | 99% | 99% | - |
| val (tuned: cutoff) | 116/122 = 95.1% / 93.4% | 105/107 / 104/107 | 11/15 / 10/15 |
| val2 (tuned: cutoff) | 104/119 = 87.4% / 85.7% | 99/112 / 99/112 | 5/7 / 3/7 |
| **test (original held-out)** | 114/129 = 88.4% / 86.8% | **103/112 = 92.0%** / 104/112 | 11/17 = 65% / 8/17 |
| **test2 (fresh held-out)** | 104/133 = 78.2% / 78.9% | **97/120 = 80.8%** / 98/120 | 7/13 = 54% / 7/13 |
| doc4 (seen during earlier tuning) | 70/81 = 86.4% / 85.2% | 68/72 = 94.4% | 2/9 / 1/9 |
| pooled test + test2 | 218/262 = 83.2% | **200/232 = 86.2%** | 18/30 = 60% |

Span-level (per label) on test2: precision 82%, recall 80%. Chains are reported separately as required and are weak (about 60%): the usual failure is one half of the chain being dropped or the pair collapsing to a combined label (e.g. A2+S4 -> W6).
Typical single-task errors: near-neighbour confusions (S1 volume/brightness vs A3 media volume, E5 vs F5, E7 vs M1/E5, A2 vs A3 for "next / queue up"), and OUT requests that look like in-scope ones ("dim the lights" -> S1, "stock price of Tesla" -> Q3, "set up a zoom meeting" -> C6). The original test set was the easier one; test2 is probably closer to real casual speech but is still same-author.

## Latency and memory (CPU only, 4 threads; machine was at ~100% CPU the whole time because another agent was running benchmarks, so treat as pessimistic and noisy)
| | model file | runtime RSS* | encode, 1 utterance | full request incl. chain split (n=262) |
|---|---|---|---|---|
| ONNX fp32 | 90 MB | 166 MB | ~9 ms | mean 13-16 ms, p95 27-40, max 86 |
| **ONNX int8 (dynamic quant)** | **23 MB** | **99 MB** | ~4.4 ms | mean 8 ms, p95 18 (one noisy rerun: mean 25, p95 100) |

*Standalone process with onnxruntime + tokenizer + numpy head, no sklearn or training data (about 54 MB is the Python/onnxruntime baseline). The in-script RSS with sklearn and the training matrix was 310-350 MB, which is not the product footprint. Latency target (<50 ms) passes by a wide margin even under load; memory target (<150 MB) passes for int8 (99 MB), fp32 is slightly over (166 MB). int8 costs about 0-1.5 points on test sets (within noise; it was not worse on test2).

## Confidence threshold recommendation (for D11)
Measured on all single-task requests of test + test2 + doc4 (n=304), top-1 probability of the head (fp32; int8 is within a point):

| top-1 prob >= | requests answered | correct among answered | wrong but answered |
|---|---|---|---|
| 0.5 | 92% | 92.1% | 22 |
| 0.6 | 86% | 93.1% | 18 |
| 0.7 | 79% | 94.2% | 14 |
| **0.8** | **71%** | **96.3%** | **8** |
| 0.9 | 57% | 98.3% | 3 |

Recommended policy: **>= 0.8 execute directly** (this is also the cutoff chosen on val and used for chain splitting); **0.5-0.8 show the top choice as a one-tap confirmation** ("Did you mean: Open WhatsApp chat?", ~21% of requests, only ~78% correct in that band); **< 0.5 fall back** to the LLM planner / "I did not get that" (~8% of requests, ~60% correct). For OUT, also treat predicted OUT as a refusal only at >= 0.8, otherwise ask. Because the head is trained on synthetic data it is probably over-confident on real speech, so re-calibrate (temperature scaling) on the first few hundred real logged requests before locking these numbers.

## Caveats
- All training, val and test sentences are written by the same author (an LLM agent / one person); templates and tests share vocabulary. Real users will be harder; expect a few points lower than test2 on real speech. The 20-point gap between train (99%) and test2 (81%) shows how much of the score is memorisation of template shapes.
- Original val/test were recovered from the WSL copy; I cannot independently prove test.txt was never used for tuning (the earlier docstring says it was scored once). test2 is the only set I can vouch for as untouched.
- 52 labels with ~100 train rows each; sample sizes of 112-120 singles give about +/-7 points of noise per set.
- Slot-heavy cases (names of apps, files) are irrelevant to the router but matter to SP2; the router only sees the whole sentence.
- Doc 5 fallback if accuracy must be higher: more and more varied training data (real logged requests, LLM-paraphrased sentences) or a slightly bigger encoder (bge-small / e5-small); both were not needed to meet latency.
