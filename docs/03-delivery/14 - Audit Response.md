# 14. Response to the end-to-end audit (10 Nov 2026)

What was done about each finding in [13 - End-to-End Audit](13%20-%20End-to-End%20Audit.md), and what is still open. Nothing here is "done" until it is checked on the owner's own PC and accounts.

## Done

| Audit item | What changed | Where |
|---|---|---|
| P0-4 send confirms show the raw request | Mail and Teams confirms show the exact words that will be sent; a send with no message asks "What should it say?" first | `normalize::message_body`, `flow.rs`, `skills/mail.send.yaml`, `skills/chat.post.yaml` |
| P0-4 "set my status", "join my meeting" became a Teams post | Refused plainly ("I can't change your Teams status yet") | `normalize::plain_reply` |
| P0-3 confidently wrong facts | "what does X mean" uses a dictionary (Wiktionary). How/why questions, explanations, sums and conversions answer "I'm not sure, say search for …" instead of a random Wikipedia intro | `web_lookup.rs` |
| UX: the request disappears | The request stays above every answer, question and confirm | `App.tsx`, `obsidian.css` |
| Quick win: typed undo | "undo" / "undo that" run the Undo button's action | `service.rs` |
| P1-2 no conversation | A follow-up ("what about tomorrow", "and in Mumbai", "again") becomes a full sentence from the last request that worked (10 minutes), then runs the normal pipeline | `followup.rs`, `flow.rs` |
| P1-4 Google Workspace | Gmail, Google Calendar and Drive behind the same connectors, so every mail, calendar and file skill works on Google; Google sign-in (loopback + PKCE); see [8.3](8.3%20-%20Google%20Setup.md) | `google.rs`, `google_account.rs`, `oauth.rs`, `registry.rs` |
| P1-4 WhatsApp | The chat opens with the text typed (or the file copied) and the person presses Send. Nothing is automated or linked, so nothing can get a number banned. Needs a saved number with country code; without one WhatsApp opens on its chat list | `whatsapp.rs`, `skills/whatsapp.*.yaml` |
| P1-4 meeting heads-up | The bar tells you about a meeting ten minutes before it starts, once per meeting (off with setting `meeting_headsup` = `off`) | `scheduler.rs`, `ms365_cal.rs` (`heads_up`) |
| P0-1 a model behind the engine | The engine has an `Llm` seam: when the router is unsure or declined, a local model gets the router's best three tools and picks one (schema-constrained: only a real skill id or "none"). A picked tool still goes through the same guards, slots, policy, confirm and checks. It never writes free text for the user: an answer from a model's memory is a guess, so answers wait for the grounded writer. The model runs as a `llama-server` child on loopback, below-normal priority, at most 3 threads, stopped after 90 idle seconds | `llm.rs`, `crates/llm`, `flow.rs` |

## Model size and the memory budget

The owner's rule (doc 15): peak over 1.2 GB for the whole process tree is a failed build. The 4B model is dropped. Candidates are tiny tool-callers (FunctionGemma 270M, Gemma 3 270M and 1B, Qwen3.5-0.8B), measured with `jitmo-llmbench` for accuracy, speed and memory; a model becomes the default only if it passes the gate in doc 15 step 2. Numbers are written here when measured.

### Measured (this laptop, 15.7 GB, 3 threads, context 768, batch 128, 262 held-out requests, 10 Nov 2026)

Candidates for the model: the router's best 2 plus the best word-overlap match (recall of the right tool among 3: **91.6%**; the router's best 3 alone: 84.9%; word overlap alone: 72%). The pick is ONE token (the tool's number). Models are **untuned**; fine-tuning is the next step (`training/`).

| Model (Q4_K_M) | Tool right, all 262 | Tool right when it was among the candidates | Wrong actions | Pick time p50 / p95 | Peak memory (working set / private) | Typed arguments right (right tool given) | Argument time p50 |
|---|---|---|---|---|---|---|---|
| FunctionGemma 270M | 42.0% | 53.4% | 58.0% | **194 / 262 ms** | **588 / 355 MB** | 21.9% | 984 ms |
| Gemma 3 270M | 33.6% | 42.7% | 64.1% | **152 / 255 ms** | **499 / 266 MB** | 24.7% | 1069 ms |
| Gemma 3 1B | 33.2% | 40.8% | 66.4% | 665 / 982 ms (fails 400 ms) | 1151 / 413 MB (fails 600 MB) | 68.5% | 2715 ms (fails) |
| Qwen3.5-0.8B (earlier run, 3 word-overlap candidates, JSON-schema pick) | 51.1% | 82.7% | 48.9% | 878 / 1374 ms (fails) | 883 / 565 MB (fails 600 MB) | not run | not run |

Gates (doc 15, step 2: peak at most 600 MB, warm p50 at most 400 ms; step 5: tool at least 95%, arguments at least 90%, no wrong sends):

- **Speed and memory pass for the 270M models** with the one-token pick (150 to 190 ms, under 600 MB). They fail for 0.8B and 1B.
- **Accuracy fails for every untuned model**, as expected (they were never trained on jitmo's tools). Nothing is the default yet and no model is installed in the app.
- **A grammar on Qwen3.5 made the server use about 6 GB** (one run measured 6,168 MB working set). The grammar is now off by default; a tuned picker writes the digit by itself and the engine checks it in code.
- **Typed arguments are slow on Gemma** (about 1 s): 262k-word vocabulary, about 30 tokens of JSON. The engine only asks for arguments when its own rules are unsure, and a tuned model should write shorter values; the gate (p50 300 ms for actions) is not met by the argument step yet.
- Memory variants (100 requests, FunctionGemma): default 588/355 MB, 194 ms; `--no-repack` 530/304 MB, 283 ms; q8 KV cache with flash attention 467/235 MB, 330 ms. Memory-mapped weights are the llama.cpp default; turning off repacking saves about 50 MB private memory at some cost in reading time.
- Prompt-prefix cache: with and without it the times differ by less than 25% for every model, so the design does not rely on it (the prompt is short on purpose).
- Cold load 1.2 to 1.7 s for the 270M and 1B models (2.6 to 2.8 s with the variant flags); the app warms the model when the bar opens.

**What is needed to pass the accuracy gate:** fine-tuning on `training/router_train.jsonl` and `router_train_args.jsonl` (3,180 and 840 examples, never from the held-out set) using the Colab notebook, then `jitmo-llmbench` on the held-out set. Even a perfect picker is capped by candidate recall (91.6% with three candidates, 93.8% with five); the shared encoder (step 3) is the way past that and has not been built.

## Also done after doc 15

- Typed arguments: 14 skills declare `tool: args + phrase`; a model fills them as schema-constrained JSON and the engine builds the skill's own sentence and runs it through the normal pipeline (missing required values are asked from the skill's own question). The old `message.starts_with(...)` branches and keyword guards in `flow.rs` are **kept**: with no model installed they are the only path, so they go only once the model path is the default and passes its gate.
- Learned requests: a request a model worked out that then succeeded is remembered (300 most recent) and next time skips the router and the model; a remembered request that fails is forgotten.
- Budget: free-memory check before loading (model file + 1.5 GB must stay free), a 1.2 GB Job Object cap that also kills the model process with jitmo, at most 3 threads, 90 s idle unload, warm-up when the bar opens, unload when Windows signals low memory.
- Grounded writer: for how/why/explain questions the engine fetches Wikipedia intros, keeps the page that covers the question's words, lets the model write from it, and shows the answer with its source only if every figure and most words come from the passage; otherwise "I'm not sure". Needs an installed model to do anything.

## Still open

- **Fine-tuning** (needs your Colab; see `training/README.md`) and the held-out accuracy gate. No model is installed in the app.
- The shared MiniLM encoder (step 3). Candidate recall caps accuracy at 91.6% for three candidates.
- Argument speed (about 1 s on Gemma models) and accuracy before tuning.
- Held-out set has 262 requests (the brief asked for 300), and 73 have expected arguments.
- Real-account checks of Google, WhatsApp, sends and calendar writes; OneDrive/Teams through Graph; local file index and PDF reading; browser actions; voice; a concurrent core.
