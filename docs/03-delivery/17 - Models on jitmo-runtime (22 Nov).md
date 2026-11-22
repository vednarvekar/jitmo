# 17. Models on jitmo-runtime (22 Nov 2026)

jitmo's language model now runs on jitmo-runtime (`V:\Code\jitmo-runtime`), not on a llama-server that jitmo starts itself. This doc records what moved, how it is wired, the numbers, and what is still open. Runtime API reference: `jitmo-runtime/docs/06-runtime-api.md`. Pull request: [#1](https://github.com/vednarvekar/jitmo/pull/1) (branch `runtime-backend`).

All numbers were measured on 22 Nov 2026 on the owner's laptop (i5-1135G7, 16 GB, Windows 11).

## 0. Summary

- Same model, old path and runtime give **the same result on every single case** of all three test sets.
- In the real app, jitmo started the runtime as its child, served two requests with 0 failures, and nothing was left running after close. Memory: **332 MB working set, 146 MB private** (rule: 1.2 GB).
- Held-out and day-in-the-life runs were a little faster through the runtime. The audit corpus was **slower** (196 s to 306 s). The cause is the runtime's pacer (section 5).
- jitmo's own llama-server launcher was removed after this end-to-end check passed. The runtime is now the only path.

## 1. What moved and what stayed

| Part | Where it runs now | Why |
|---|---|---|
| Tool-caller: pick (one digit) | jitmo-runtime | |
| Tool-caller: typed values (`fill`) | jitmo-runtime | |
| Writer (`answer_from`) | jitmo-runtime | |
| Router (fine-tuned MiniLM tool encoder, ONNX) | jitmo's model-worker | The runtime runs only llama.cpp models |
| NER tagger (ONNX) | jitmo's model-worker | Same reason |

## 2. Mapping

All three go to `POST /v1/chat/completions` on the base model. None uses a chain: chains need fixed prompts, and jitmo builds the tool list for each request.

| jitmo step | Request | Key settings |
|---|---|---|
| Pick | chat completion | GBNF digit grammar, `max_tokens` 1, `cache_prompt` |
| Values (`fill`) | chat completion | `response_format` `json_schema` |
| Writer (`answer_from`) | chat completion | |

Every request sends `X-Jitmo-Priority: interactive`.

## 3. How jitmo finds or starts the runtime

Code: `crates/llm/src/runtime.rs`.

1. **Connect.** If `runtime_url` is set, jitmo uses the runtime already running at that URL.
2. **Start as a child.** Otherwise jitmo starts `jitmo-runtime.exe` with `--port 0 --parent-pid <jitmo pid> --port-file <file> --log-file <file>`, and reads the line `JITMO_RUNTIME_LISTENING port=<n>`. When jitmo exits, the runtime and its model exit too.
3. **Warm up.** `POST /admin/load` loads the model before the first request.
4. **Errors fall back to the rules.** Any runtime error (for example `503 insufficient_memory`, `memory_pressure` or `queue_full`, `502 engine_error`) makes jitmo use its rule-based path for that request.

Config: `config/jitmo-runtime.toml`.

| Key | Value | Note |
|---|---|---|
| Model | `gemma-3-270m-it-router-Q4_K_M.gguf` | Fine-tuned Gemma 3 270M tool-picker (doc 16, section 3) |
| ctx | 2048 | |
| `no_repack` | false | Matches the old launcher |
| `flash_attn` | auto | Matches the old launcher |
| `load_estimate_mb` | 400 | Keeps the runtime's RAM check inside jitmo's 1.2 GB rule |

## 4. Settings and audit options

| App setting | What it does |
|---|---|
| `runtime_url` | Connect to a runtime already running at this URL |
| `runtime_exe` | Path to `jitmo-runtime.exe` to start as a child. Default: next to `jitmo-app.exe`, else `<app data>\models\runtime\` |
| `runtime_config` | Runtime config file to start it with. Default: `<app data>\models\jitmo-runtime.toml`, else next to `jitmo-app.exe` |
| `runtime_model` | Model name sent in requests. Default `jitmo-router` |
| `llm_model` | Passed to the runtime as `--model` (override). Default: the first `.gguf` in `<app data>\models` |
| `llm_server` | Passed to the runtime as `--llama-server` (override) |
| `llm_prompt` = `long`, `llm_grammar` = `off` | Long pick prompt / no digit grammar, for a model not fine-tuned on jitmo's tools |
| `llm` = `off` | No language model; rules only |

The old `llm_backend` and `llm_threads` settings are gone: there is only one path, and the runtime config sets the threads.

| `jitmo-audit` option | What it does |
|---|---|
| `--llm-runtime <url\|exe>` | Use a running runtime (URL) or start one (exe path). Default: the runtime built in `../jitmo-runtime` |
| `--llm-model <gguf>`, `--llm-server <exe>` | Passed to the runtime as overrides; `--llm-model` alone turns the language model on |
| `--llm-runtime-config <toml>` | Runtime config to start it with |

## 5. Results: old path vs runtime

Same model, same sets, full engine. Every case gave the same result on both paths.

| Set | Old path | Runtime |
|---|---|---|
| Held-out (262) | 159 | 159 |
| Day-in-the-life, `vision_corpus` (119) | 72 | 72 |
| Audit corpus (712) | 573 | 573 |

| Set | Old path | Runtime | Change |
|---|---|---|---|
| Held-out | 87 s | 78 s | 10% faster |
| Day-in-the-life | 34 s | 31 s | 9% faster |
| Audit corpus | 196 s | 306 s | **56% slower** |

**Cause: the runtime's pacer.** The audit itself keeps the CPU busy, so the pacer caps the model. Same build, same set, run back to back (single runs; builds were running during some of them, so treat them as rough):

| Audit corpus, 712 cases | Time |
|---|---|
| Old path (jitmo's own llama-server) | 366 s |
| Runtime, pacer on (default) | 450 s |
| Runtime, pacer off (`[pacer] mode = "off"`) | 318 s |

With the pacer off the runtime is as fast as the old path or faster. The pacer is doing its job (keeping the PC usable); a batch test run is the one place where that costs time. Use `mode = "off"` in a copy of the config for timing runs.


## 6. Real app check

| Check | Result |
|---|---|
| Opening the bar | jitmo started `jitmo-runtime.exe` as its child, which loaded the model |
| Runtime with model loaded | 332 MB working set, 146 MB private (rule: 1.2 GB) |
| "set a timer for ten minutes" | served by the runtime, 382 ms |
| "play some lofi beats" | served by the runtime, 672 ms |
| Failures | 0 |
| Closing the app | the runtime and its model exited; nothing left running |
| Pacer | heavy level the whole time ("other processes use 100% CPU"): the model got 50% of its CPU, about 24 tok/s |

The pacer reading looks wrong for a light session. It was raised with the runtime team.

## 7. Gotcha: Microsoft Store Python and the settings database

The Microsoft Store Python on this PC silently redirects writes under `AppData\Roaming` to a private copy. Editing the app's settings database with it changes nothing the app can see, and gives no error. Use Node (`node:sqlite`) or the app itself.

## 8. Open items

1. **Audit-corpus time** (section 5): explained, the pacer. Nothing to fix in jitmo.
2. **Pacer reading** in the real app (section 6): heavy level during a light session.
3. **The "score drop" was the audit tool's default router.** `jitmo-audit` loaded `models/router` (the older frozen encoder); the app and the model worker use `models/tools` (the fine-tuned tool encoder). Fixed: the audit, `jitmo-gendata` and `jitmo-llmbench` now pick `models/tools` when it is there, and the audit header prints the folder. Final numbers, runtime only, `models/tools`: **held-out 172/262, day-in-the-life 81/119, audit corpus 584/712** (doc 16: 175 / 81 / 584). Without the language model: 173 / 81 / 581. The one held-out case the model costs is "how do vaccines work": the model tries a web lookup, which the offline harness scores as a fail (the harness scoring issue from doc 16).
4. **Runtime features jitmo could adopt later:**

| Feature | Use in jitmo |
|---|---|
| Cancel: `DELETE /v1/requests/{id}` | Stop the model when the user presses Esc (next) |
| `X-Jitmo-Priority: background` | Work the user is not waiting for (next) |
| Chains with `vars` | Writer steps with fixed prompts, such as question plus passage |
| `/v1/embeddings` | Replace the ONNX router, only after its vectors are compared with the ONNX MiniLM |
| LoRA adapters | One base model with per-task adapters instead of several files |
