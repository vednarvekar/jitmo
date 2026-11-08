# SP3 result: can a 1-3B local model plan chained tasks on a CPU?

**Verdict: FAIL as run** (4/8 exact on T75-T82 against a target of 8/8; latency and memory not met or not validly measured). Not proof the idea is dead: the test gave the model all 55 skills at once, which is much harder than the designed "router sends top 5" flow, and 3B was never tried. Runs 7-8 Nov 2026.

## Setup
- Qwen2.5-1.5B-Instruct Q4_K_M (1.12GB GGUF, in `models/`), llama-cpp-python 0.3.36 prebuilt CPU wheel, 4 threads, ctx 4096, temperature 0. Windows venv `.venv` (Python 3.12, via uv; 3.14 not needed).
- `plan.py`: system prompt listing all 55 skills + 9 few-shot examples; GBNF grammar built from the skill catalog so output is always valid JSON with known skill ids and slot names. `eval.py`: scoring (exact skill sequence, step precision/recall, slot accuracy).
- T75-T82 gold was written from doc 4 (Expected column + task IDs); VAL (24 sentences) and EXTRA (10 sentences, `extra.py`) are by the same author and are not independent of the prompt.

## Numbers
| Set | Exact plan | Detail |
|---|---|---|
| **T75-T82 (final, one-shot)** | **4/8** (T75, T76, T77, T81) | step precision 0.67 / recall 0.71, slot accuracy 0.75, JSON valid 8/8 |
| VAL (24, prompt tuned on) | 17/24 = 71% | precision 0.88 / recall 0.84, strict slots 0.60 (lenient 0.67), JSON valid 24/24 |
| EXTRA (10 fresh, not tuned on) | 7/10 | misses: sent-to-Teams mapped to `workflow.forward_attachment`, "mute the volume" mapped to `system.dnd` (and lost 2 steps), "screenshot" mapped to `files.open` |

T75-T82 misses: T78 (archive newsletters + urgent -> two wrong workflow skills), T79 (`app.open` instead of `web.open`; arguably a gold ambiguity), T80 (screenshot -> `files.find`), T82 (spurious third step). The T75-T82 run was done once, in WSL by the previous agent (`test_1.5b.log`, `results/test_*.json`); eval.py refuses a second run, so I did not repeat it.

| Resource | Result | Target |
|---|---|---|
| Latency per plan (T75-T82, contended) | 108-422 s, median 114 s | < 5 s |
| Latency per plan (EXTRA, less contended, prefix cached) | median 16 s (12-23 s; 59 s for the cold first), ~30-75 output tokens, decode 3-5 tok/s | < 5 s |
| Peak RSS | 1.85 GB (val), 1.92 GB (extra), 2.06 GB (T75-T82, previous run) | < 2 GB |
| RSS after load | 1.40-1.88 GB | - |

## Latency caveat (most important)
No number above is a clean measurement. The box was at 85-100% CPU the whole time (other agents' Python/ML job in WSL, rustc, Chrome); my 4-thread process got only about a third of a core at times (measured: 6.75 CPU-seconds per 20 s wall during the val run). A 1.5B Q4 model should decode roughly 15-30 tok/s on 4 idle cores, which would put a 40-token plan near 2-3 s, but that is an expectation, not a measurement. What I could check: grammar on vs off made no difference to decode speed (4.7 / 3.1 vs 4.4 / 4.3 tok/s, two runs each), so the 55-alternative grammar is not the bottleneck. llama-cpp-python reuses the KV prefix between calls, so after the first call the ~1.7k-token prompt costs about 0.3-5 s of prefill instead of 20-50 s; the first call after load is slow and a saved prompt state would fix that. Re-run on a quiet machine (and the 8GB target laptop) before trusting any latency claim.

## Where it goes wrong
The 1.5B model confuses neighbouring skills when it sees the whole catalog (workflow.* vs primitives, `system.dnd` vs `system.volume`, `files.*` vs `system.screenshot`), drops the second step of two-verb sentences, and sometimes invents an extra step. It never produced invalid JSON or an unknown skill (grammar works), and `$stepN.result` chaining was correct whenever the skills were right.

## Not done
- Qwen2.5-3B: the copy in WSL is a truncated download (264MB), so it was neither copied nor tested. Needs a fresh download.
- Top-5 candidate catalog (the real design), ctx 2048, and any fine-tuning/LoRA on planning data.
- Clean timing run; a bigger and independent test set.

## Decision
Do not adopt 1.5B with this prompt as the only planner. Take the doc 5 fallback for now: **cover common chains with fixed templates keyed on router output pairs** (T75-T82 style: music+DND, find+send, calendar+message, organize+urgent, open+search, screenshot+email), and send rare chains to the "can't do that yet" prompt. Keep the LLM planner behind a flag. Cheap experiments that could change this: (1) give the planner only the router's top 5 skills, (2) try 3B, (3) a planning LoRA (ties into SP4), (4) fix latency with a saved prompt state and ctx 2048. Pass bar to revisit: 8/8 on T75-T82 plus at least 85% on an independent set, under 5 s and 2 GB on the target laptop.
