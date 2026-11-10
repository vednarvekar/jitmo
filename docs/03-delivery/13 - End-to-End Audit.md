# 13. End-to-End Audit (10 Nov 2026)

A strict audit of jitmo against the owner's stated goal: **a personal assistant at your fingertips for
everyone — a student and an office worker alike — that answers questions, helps read and understand study
material, opens apps, messages friends, checks schedules, plays music, browses and acts on websites, connects
to the company workspace (Microsoft 365 or Google), gives meeting heads-ups, takes notes, shares files and
answers messages; local models only, a smart engine rather than a rule follower, the best UI/UX.**

> **Status (10 Nov 2026, evening).** Many findings below have since been addressed; see
> [14 - Audit Response](14%20-%20Audit%20Response.md). Section 6's "one 3–4B model" advice is **superseded**
> by section 10: the owner's hard limit is that jitmo must never slow the PC, so the design is now a
> cascade of small specialist models inside a fixed memory budget.

## 0. Verdict in one paragraph

The *engineering discipline* is excellent: a clean Rust workspace, a real policy engine, confirms, Undo,
verification, a model process with idle unload, ~550 passing tests, an audit harness and a polished bar. The
*product direction* has drifted away from the goal. What exists is a well-tested **command launcher for about
50 fixed phrases**, held together by **~5,500 lines of hand-written rules** around a **closed-set classifier**.
It cannot generate a single sentence, so everything a student needs most (explain, summarise, quiz, translate,
read a PDF) is impossible by design, and every new phrasing needs a new rule. The fix is not more rules: it is
to put a small local language model with constrained tool calling at the centre, keep the existing safety
layer around it, and use rules only as a fast cache.

## 1. What was done for this audit

| Check | Result |
|---|---|
| Read docs 00–12, PRODUCT.md, README, every crate, the skills, the Tauri shell and the UI | — |
| `cargo test --workspace` | all green (~550 tests, 0 failed) |
| New **vision corpus** (`testset/vision_corpus.txt`, 119 day-in-the-life requests: student, worker, natural speech, follow-ups) through the real engine and real model worker (`jitmo-audit --all-skills`) | **72 / 119 pass (60%)**; student 32/60, worker 29/39 (against fake accounts), natural 11/20 |
| Live web answers for 12 student questions (`live_one`) | 5 right, 3 vague, **4 confidently wrong** |
| Bar UI in browser preview: typing, question, confirm, answer states | looks premium; 4 UX problems (section 5) |
| Market check: small on-device models and Windows agent APIs (Nov 2026) | section 7 |

The previous skills audit (doc 9) measured 56% on its 643 messy inputs. The two numbers agree: roughly
**4 in 10 realistic requests fail**, and the failures are structural, not bugs.

## 2. What is genuinely good (keep it)

- **Safety core in plain code**: risk levels, confirm before every send, Undo window, action log, blocked
  permanent deletes. This is the part competitors get wrong; it is the right moat.
- **Verifier and honest failures**: "I couldn't confirm that it worked" beats a fake "Done".
- **Process split**: model worker as its own low-priority process with idle exit; 14 MB idle with the bar closed.
- **Skill files + connector trait**: a sane extension point.
- **Test culture**: the audit harness that drives the real `Flow` over a simulated PC is a real asset. Reuse it
  for everything below.
- **UI craft**: glass bar, keyboard-first (Enter / Esc / 1–9), compact chips, sound, reduced motion.

## 3. Critical findings (ranked)

### P0-1. The "smart engine" is a rules engine

Evidence:
- `normalize.rs`, `clarify.rs`, `hinglish.rs`, `rules.rs`, `refine.rs`: **5,496 lines** of keyword lists and
  regexes. The model glue (`router.rs`, `ner.rs`, `extractor.rs`) is **1,022 lines**.
- The router head (`models/router/head.json`) is a logistic regression over **50 fixed classes named after the
  spec's task IDs** (`A1`…`W6`, `OUT`). A new skill cannot be routed until the head is retrained; a skill with
  no task ID (`settings.place`) only works by exact example match. This contradicts doc 00.3 ("thousands of
  skills", example-similarity routing) and makes the skill format a false promise.
- `flow.rs` branches on **error-message text** 11 times (`message.starts_with("How long should the timer run")`,
  `"What should the email say"`, `"Which day?"` …). Rewording a message silently breaks a flow.
- Per-skill keyword guards inside `flow.rs` (`calendar.move` needs "move/push/shift…", `work.send_report` needs
  "manager/boss", `files.email` vs `mail.send` by file extensions).
- 39 of 50 skills pass the **whole sentence** as `{request}` and declare no slots, so every connector re-parses
  English on its own (`message_text()` splits on " that ", " about "…).

Why it matters: this is whack-a-mole. Each fix covers one phrasing; the space of phrasings is unbounded,
especially with Hinglish and typos. Accuracy plateaus (doc 9: 50% → 56% after a big round of fixes).

### P0-2. No generation at all, so the student half of the goal is impossible

There is no language model that writes text. Consequences seen in the vision corpus:
"explain the chapter i have open", "make flashcards from my biology notes", "quiz me on the French Revolution",
"translate good morning to French", "give me 3 tips to study", "how do I make pasta", "solve 2x + 5 = 15",
"draft a polite reply declining the vendor proposal" — all declined or failed. Email "summaries" are the first
two sentences of the body (`extract_summary`); drafts are the user's own words copied after "saying".

### P0-3. Answers are confidently wrong

`web.fact` returns the intro of whichever Wikipedia page the search hits first. Live results:

| Question | Answer shown |
|---|---|
| what does ephemeral mean | "A lake is often a naturally occurring… body of water" |
| how do vaccines work | "The Vaccines are an English indie rock band…" |
| how many grams in a pound | "The pound sign is the symbol for the pound…" |
| what's the derivative of x squared | generic definition of "derivative", no "2x" |
| explain Newton's second law simply | intro to all three laws |

The bar then shows the answer **without the question**, with a source chip, so a wrong answer looks
authoritative. For a student this is worse than "I don't know".

### P0-4. Send skills fill messages with whatever the router guessed

- "set my status to busy on teams" → routed to `chat.post` @0.84 → confirm card *"Post this in Teams: set my
  status to busy on teams"*. "join my next teams meeting" → *"Did you mean: Post in Teams?"* then a post
  confirm. One careless Enter posts garbage to a work channel. The confirm protects against the wrong
  recipient, not against the wrong *intent*.
- The email confirm card shows the **raw request** ("Email Rahul Sharma “email rahul that the report is
  ready”?"), not the body that will be sent ("The report is ready."), and no subject. SR4 ("confirm with
  recipient and preview") is not met.
- The subject is the first 60 characters of the body (`graph.rs:390`).
- Send verification (`ms365.rs:825`) passes if *any* earlier message to that person exists.

### P1-1. "Urgent email" will be empty for real users

`urgent = importance == "high"` (`graph.rs:291`). Almost nobody sets that flag; real urgency comes from sender,
deadline words, direct-to-me, replies waiting. This is a classification job for a model with the user's own
signals, not a flag.

### P1-2. No conversation

Each request is independent. Failed in the corpus: "weather in pune → what about tomorrow", "find my resume →
send it to rahul", "remind me to call mom → make it 7pm instead", "undo that" (routed to *music control*).
An assistant without "it / that / instead / again" does not feel like an assistant.

### P1-3. The out-of-scope class swallows real requests

`OUT` scored 1.00 on "text rahul on whatsapp that i'll be late", "i need to focus for an hour", "any new
messages on whatsapp"; 0.91 on "how do i make pasta". A closed classifier puts everything unfamiliar into OUT,
so coverage gaps look like refusals rather than "I can't do that *yet*, but here is the closest".

### P1-4. Coverage gaps against the stated goal

| Goal item | State |
|---|---|
| Answer questions, explain study material | wrong or missing (P0-2, P0-3) |
| Read / summarise a PDF or open document | missing |
| Message friends (WhatsApp, Telegram, Instagram, SMS) | missing (WhatsApp is Phase 4; "text rahul" is declined) |
| Class schedule | missing (no Google Classroom / timetable / Google Calendar) |
| Play songs | Spotify via UI clicks only; YouTube search works |
| Browse and *act* on websites ("visit the platform and do what he asks") | open/search only; no page reading or actions |
| Google Workspace | not started |
| Microsoft 365 | built against fakes; **never signed in on a real account**; OneDrive and Teams not on Graph |
| Meeting heads-up / reminders before meetings | missing (no calendar watcher) |
| Take notes / meeting notes | missing (no notes store, no audio capture, no transcription) |
| Close apps, battery, brightness by feel ("too dark"), recycle bin, rename, zip | missing or misrouted |
| Voice | out of V1 by decision; for "fingertips" it should not be |

### P1-5. Core is single-threaded and blocking

One `Flow` on one thread; prompts block it; a slow web or Graph call freezes the bar (doc 9, G12). Fine for
one-shot commands, fatal for proactive features (heads-ups, background sync, streaming answers).

### P2. Smaller issues

- Contacts come only from vCards and past use; no M365 / Google / WhatsApp contact sync, so "email Rahul" fails
  for most new users.
- Context is only the Explorer selection and Downloads: no active window, clipboard, open document, browser
  tab, or screen text. "Summarise this" has nothing to read.
- Prompt-injection defence is a keyword list (`has_instructions_for_an_assistant`). Fine today; once a model
  reads mail it must be structural (content never in the instruction channel, tools disabled while
  summarising untrusted text).
- Skills load unsigned in dev; the signing path is untested end to end.
- The calculator falls through to Wikipedia for unit conversions it cannot parse ("grams in a pound").
- Several `jitmo-model-worker.exe` processes were alive at once during this audit (some from a concurrent run);
  worth one check that workers always exit when their parent does.

## 4. Plan and process problems

1. **The docs describe a different product.** Doc 1 targets M365 office workers with 20–30 tasks; doc 2 has 49
   tasks for "personal users and startups"; the owner's goal is students *and* workers. Pick one first user.
2. **The plan defers the one thing the goal needs most.** The 1–3B model is Phase 3 Track B and "the first to cut
   if the schedule slips". For the stated goal it is the foundation, not a stretch.
3. **Tests measure the spec, not the user.** The 100 tests and the 643-line corpus were written by the same
   people who wrote the rules, phrased like the examples. Passing 88/100 says little about real use (60% on the
   vision corpus, 56% on messy wording). Gate on held-out, real-user requests instead.
4. **Plan size vs team.** Docs assume a 6–8 person team, four different schedules, 540 KB of documentation;
   the answer in D9 is "just me". Scope must match one builder: fewer connectors, more generality.
5. **"Local only" is right, but "no generation" is not implied by it.** A 3–4B model at 4-bit runs in ~2–2.5 GB
   on CPU, faster on NPUs. The 2 GB peak budget in PR2 needs a deliberate re-decision (tiered hardware).

## 5. UI / UX findings

- **The request disappears** once a question, confirm or answer shows (placeholder becomes "pick one below…").
  The user loses what they asked; a wrong answer cannot be spotted. Keep the request visible as a header line.
- **Confirm cards show raw text** (P0-4). Show recipient, subject and the exact body, editable before sending.
- **"Ask Jitmo anything…" overpromises** for a 50-command launcher. Either deliver (section 6) or say "Open apps,
  set timers, ask a question…" with rotating real examples.
- **No history / thread view.** A small expandable transcript (last 5 turns, with Undo per turn) enables
  follow-ups and trust.
- **No first-run onboarding** of what it can do and which accounts to connect.
- **Orb** reads grey and small in the obsidian look, against the pinned "colourful, clearly visible" decision.
- Strengths to keep: keyboard numbers on chips, compact one-line results, motion and sound polish.

## 6. Recommended architecture: a smart engine that stays local

Keep the Rust core, policy, confirms, Undo, verifier, connector trait and harness. Change the understanding
layer from *classify, then regex* to *retrieve, then constrained tool call*.

```
request + conversation state + context (active window, selection, clipboard)
   │
   ├─ 1. Fast path: exact / cached request → skill + args          (rules become a cache, not the engine)
   ├─ 2. Retrieve top-k tools: embedding search over tool descriptions + examples (open set; a new skill
   │     needs no retraining). EmbeddingGemma / bge-small, sqlite-vec or HNSW.
   ├─ 3. Small LLM (3–4B, 4-bit; 1–2B on 8 GB machines) does a JSON-schema / GBNF-constrained tool call:
   │     {"tool":"mail.send","args":{"to":"Rahul","subject":"…","body":"…"}} — or {"tool":"answer"} or
   │     {"ask":"which Rahul?"}. Multi-step plans use the same format (≤5 steps, $step links you already have).
   ├─ 4. Code validates against the tool schema: missing required arg → ask (generated from the schema,
   │     not from error strings); resolver turns "Rahul" into an exact contact; policy decides run / undo / confirm.
   ├─ 5. Execute, verify, report; store the turn's entities for "it / that / instead".
   └─ Generative tools run on the same model: answer, explain, summarise, translate, quiz, flashcards, rewrite,
      draft reply — grounded in retrieved text (local files, the open PDF, Wikipedia passages) with citations,
      and "I'm not sure" when the passage does not support the answer.
```

Concrete changes:

1. **Typed tool schemas instead of `{request}`.** Each skill declares JSON-schema args (to, subject, body,
   when, duration, query…). Connectors receive typed values and stop parsing English. This deletes most of
   `message_text()`, `clarify.rs` and the error-string branches.
2. **Open-set routing.** Retrieval over skill descriptions replaces the 50-class head; the head can remain as a
   prior feature. Out-of-scope = "no tool fits", answered with the closest tool or the general `answer` tool.
3. **One small model, several jobs.** Pick by measurement on your own tool schemas (BFCL-style, held-out set):
   current candidates are Qwen3.5-4B, Gemma 4 E4B/E2B, Phi-4-mini; FunctionGemma-class 270M models for an
   always-warm first pass on low-end machines. Run via llama.cpp (CPU) and Windows ML / Foundry Local where an
   NPU exists. LoRA adapters for routing-args and for summarise/draft if base quality is short.
4. **Conversation state.** Last N turns and their resolved entities, passed to the model and the resolver.
5. **Local knowledge.** Index Documents/Downloads/Desktop (PDF, DOCX, PPTX, TXT) into chunks + embeddings in
   the encrypted store; "explain chapter 3", "what did I note about the client", "find the budget sheet from
   last week" become retrieval + generation.
6. **MCP as a connector type now, not v2.** Wrap every MCP tool in your policy gate (risk level per tool,
   confirm on sends). This buys Google Workspace, Notion, Spotify, GitHub, Jira and others without writing each
   connector, and lets you use Windows' own agent connectors (File Explorer, Settings) where available.
7. **Browser actions.** Drive the user's Edge/Chrome via CDP: read the accessibility tree, let the model pick
   the next element, confirm before any submit/purchase/post. This is what "visit the platform and do what he
   asks" needs; keep it opt-in per site.
8. **Desktop apps without APIs.** UI Automation tree + the same model loop (as SP5 planned for WhatsApp),
   always with "open the chat and let me press Send" as the fallback.
9. **Voice and proactive layer.** Push-to-talk with a local ASR (whisper.cpp / Moonshine / Parakeet-class) and
   a local TTS (Piper / Kokoro-class). A background scheduler (separate from the request thread) for meeting
   heads-ups with join links, morning brief, reminders; meeting notes = system-audio capture → local
   transcript → summary + action items (with a visible "recording" indicator and consent).
10. **Make Core concurrent.** One request task per turn, cancellable; streaming answers to the bar; background
    jobs never block the bar.

Memory budget on an 8 GB laptop: encoder + retrieval ~150 MB warm; 1–2B model ~1–1.5 GB on demand; 3–4B on
16 GB machines or NPUs. Tier by hardware at install and say so in settings.

## 7. Positioning

- Windows itself now ships MCP-based agent connectors and Copilot; Raycast/PowerToys own the "fast launcher".
  jitmo cannot win on breadth or on launching apps. It can win on **private + fast + safe actions**: a local
  model that never sends your mail or files to a cloud, confirms every send, offers Undo, works offline, and
  understands Hinglish and messy typing.
- Pick **one first user** and make them love it. Recommendation: **students** (no admin consent, no paid
  tenant, Google-heavy, high tolerance for a beta, strong word of mouth, and the offline study helper is
  unique). Office workers need Entra admin consent and M365 verification, which slows feedback loops for a
  solo builder; make them the second wave.
- Lead with three hero moments, not 50 commands: "explain this PDF / quiz me", "what's next today and remind
  me", "message my friend / play my music". Each must work 95% of the time on messy input.

## 8. Roadmap (solo builder, about 12 weeks)

| Weeks | Goal | Done when |
|---|---|---|
| 1–2 | **Model spike**: 3 candidate small models, llama.cpp in the worker, constrained tool calls over the existing 50 skills with typed schemas | ≥90% correct tool + args on the vision corpus and 200 held-out real phrasings; p50 < 1.5 s on the 8 GB laptop |
| 3–4 | **Swap the understanding layer**: retrieval routing, schema-driven asking, conversation state; rules kept only as the fast-path cache | vision corpus ≥85%; messy-wording corpus ≥85%; error-string branches deleted |
| 5–6 | **Grounded answers + study helper**: local file index, "explain / summarise / quiz / flashcards / translate", Wikipedia passages with citation and "not sure" | 0 confidently wrong answers on a 100-question student set; every answer shows the question |
| 7–8 | **Real accounts**: Google sign-in (Calendar, Gmail read, Drive) via MCP or direct; M365 signed in for real; contacts sync; confirm cards with exact preview | one week of daily use on the owner's real accounts with zero wrong sends |
| 9–10 | **Proactive + voice**: meeting heads-up, morning brief, push-to-talk | heads-up fires 5–10 min before 100% of real meetings for a week |
| 11–12 | **Browser actions + messaging fallback** (open chat, user presses Send); 10-student private beta | weekly active use and task success logged (opt-in, local) |

Change the gates: every phase is judged on **held-out real requests** (collect them from beta users with
consent), not on the spec's own test phrasings.

## 9. Quick wins this week (independent of the redesign)

1. Show the user's request above every question, confirm and answer.
2. Email confirm: show the exact body and subject that will be sent.
3. Refuse to post/send when the extracted message equals the whole request (catches the Teams cases).
4. `web.fact`: require the page title to match the question's subject; otherwise say "I'm not sure" and offer
   a web search. Add a dictionary source for "what does X mean".
5. Typed "undo" / "undo that" → the Undo path.
6. Calendar watcher toast 10 minutes before each meeting (already have the scheduler and toasts).
7. Change the placeholder to honest rotating examples.
8. Add `testset/vision_corpus.txt` (written for this audit) to the audit run and track it.

---

## 10. The low-memory engine (added 10 Nov 2026, evening)

**Owner's hard rule:** jitmo must never slow the PC. Over 2 GB is useless. The goal is many small,
specialised models that load and unload quickly and still match a big model on *our* tasks.

### 10.1 Budget (the contract every change is measured against)

| | Target | Hard limit |
|---|---|---|
| Idle, bar closed (whole process tree) | ≤ 60 MB | 100 MB |
| Idle, bar open, encoder warm | ≤ 150 MB | 200 MB |
| Peak during any request | ≤ 900 MB | 1.2 GB (Job Object cap on the model process) |
| CPU while idle | 0% | 0.5% |
| CPU threads during a request | physical cores − 1, at most 3 on a 4-core laptop | never all cores |
| Action requests (open, timer, volume, send…) | p50 ≤ 300 ms, p95 ≤ 800 ms | — |
| Answers (explain, summarise) | first words ≤ 1.5 s, streamed | — |
| Wrong actions (wrong tool, or wrong person on a send) | 0 | 0 |

Before loading anything over 300 MB the engine checks free physical memory (`GlobalMemoryStatusEx`). If
loading would leave less than about 1.5 GB free, it uses the next smaller tier or offers a web search, and it
never pushes the PC into paging.

### 10.2 What "load only part of a model" can and cannot mean

| Idea | Verdict | Why |
|---|---|---|
| Load only the layers a request needs | **No**, for normal models | Every generated word passes through every layer. Skipping layers costs a lot of quality unless the model was trained for it. |
| Memory-map the weights and let Windows page them | **Yes, already free** | llama.cpp and ONNX Runtime map the file by default. When the model process exits, the file pages usually stay in the Windows standby cache, so a reload is fast, and Windows can drop them under pressure. This *is* fast load and unload. |
| One base model plus small swappable adapters (LoRA) | **Yes, the main lever** | Several specialists for the memory of one model. llama-server can choose adapters per request. Each adapter is 5–50 MB. |
| Models built to keep part of their weights on disk (Gemma 3n and Gemma 4 "E" models, per-layer embeddings) | **Yes, test it** | About 5B parameters running with about 2B in memory. Check what llama.cpp actually keeps resident on a CPU. |
| 1.58-bit models (BitNet b1.58 2B4T) | **Yes, test it** | About 0.4 GB non-embedding memory and fast CPU decoding, but it needs bitnet.cpp, not llama.cpp. |
| Mixture-of-experts streamed from the SSD | **Not now** | Needs all experts in RAM or slow random SSD reads. Research, not a two-week task. |
| One encoder pass feeding several small heads | **Yes, already half built** | The router and the NER tagger are two separate MiniLM models today; one shared encoder with several heads halves that cost. |

### 10.3 The design: a cascade of specialists

Each request starts at the cheapest level and only climbs when the level below is not confident.

| Level | Job | Model | RAM | Resident |
|---|---|---|---|---|
| **L0** | Exact and learned requests ("open spotify", things the user confirmed before) | cache plus true parsers (dates, numbers, units, sums) | ~0 | always |
| **L1** | Understand: which tool (open set), which slots, answer or action, confidence | **one** small encoder (MiniLM-class, int8, ~25 MB) with heads for tool retrieval, a generic slot tagger (person, time, duration, app, file, message, query, place, number) and calibrated confidence | ~60–100 MB | while the bar is open, and 60 s after |
| **L2** | Fill typed tool arguments when L1 is unsure; short plans (at most 5 steps) | tiny tool-caller, 270M–0.8B, **fine-tuned on jitmo's own tool schemas**, grammar-constrained JSON | ~200–550 MB | on demand, pre-warmed when the bar opens if memory allows; unloaded after 60–90 s |
| **L3** | Write: grounded answers, explain, summarise, draft replies, quiz | the L2 base with a writing adapter if the quality is enough; otherwise a 1.5–2B model (or BitNet 2B) | ~0.4–1.1 GB | only for writing tasks; never resident together with a second generator; unloaded after 30–60 s |

Rules that **parse** (dates, durations, numbers, units, maths, URLs) stay: they are exact and cheap. Rules
that **understand** (keyword lists that guess intent, error-string branches, per-skill guards, the
`wants_an_answer` word list) are replaced by L1 and L2, then deleted.

### 10.4 How small models reach big-model precision on our tasks

1. **Specialise and distil.** Generate training data offline with a large teacher model from the tool
   schemas (paraphrases, typos, Hinglish, slang, partial commands, chains), plus real requests users have
   confirmed. Fine-tuned tiny tool-callers are reported to improve sharply on narrow tasks (FunctionGemma 270M:
   58% to 85% on Mobile Actions; one community distillation reached 96.7% on a multi-turn home-control task,
   above its 120B teacher). Small models win on narrow, repetitive agent work; keep a general model only for
   open writing.
2. **Make wrong output impossible.** Grammar or JSON-schema decoding: only real tool ids and valid argument
   shapes can come out.
3. **Show the model less.** Give L2 the top 3 tool schemas from L1 (about 200 tokens), not the whole catalog
   (1,500 or more). On a laptop CPU most of the wait is the model *reading* the prompt (see 10.8).
4. **Check in code, then ask.** Arguments are validated against the schema and the resolver (the contact
   exists, the time parses). If the L1 tagger and L2 disagree, or confidence is low, ask one question. Never
   guess on a send.
5. **Ground every answer.** Small models are good readers and bad memorisers. L3 answers only from retrieved
   text (local files, Wiktionary, Wikipedia passages), quotes it, and says "I'm not sure" otherwise.
6. **Learn locally.** Every confirmed result becomes an L0 cache entry (instant next time) and a training
   example for the next adapter update. The engine gets faster and more precise with use.

### 10.5 The scheduler (the part to invent)

- **Budget manager:** one place that knows what is loaded, its size and the free RAM. It refuses to load a
  second generator, and puts a Job Object memory cap on the model process.
- **Pre-warm on the hotkey:** people take 2–4 s to type. Start mapping L2 when the bar opens (if memory
  allows) so the load is hidden behind typing.
- **Unload on pressure:** subscribe to `CreateMemoryResourceNotification(LowMemoryResourceNotification)` and
  drop L3, then L2, at once.
- **CPU manners:** threads = physical cores − 1 (at most 3 on 4 cores); normal priority only while a request
  runs, EcoQoS or below-normal otherwise; nothing runs while idle.
- **Prompt-prefix cache:** keep the fixed instruction part of each L2 and L3 prompt cached (slot save and
  restore). Hybrid linear-attention models (Qwen3.5) may not reuse prefix caches in llama.cpp; measure it, and
  prefer a pure-attention model if caching fails.
- **Stream** L3 output to the bar so the first words show while the rest is written.

### 10.6 How to measure (the gate for every change)

A benchmark that reports, for each candidate and tier: tool accuracy and **argument** accuracy on a
**held-out** set the rules were never tuned on; ask rate; wrong-action rate; p50 and p95 latency split into
cold load, prompt reading and writing, with and without the prefix cache; **peak private memory of the whole
process tree**; CPU % during the request and while idle. Run it under a 1.2 GB Job Object cap with a browser
open, to simulate an 8 GB laptop.

### 10.7 Candidates to test (Nov 2026)

| Slot | Candidates |
|---|---|
| L1 encoder | the current MiniLM-L6 int8, shared by both heads; EmbeddingGemma-300M only if retrieval is weak; GLiNER2 (~205M, schema-driven tagging and classification in one pass) as a comparison |
| L2 tool-caller | FunctionGemma 270M (fine-tuned), Qwen3.5-0.8B (fine-tuned), Gemma 3 270M or 1B |
| L3 writer | the L2 base plus a writing adapter; Qwen3.5-2B; Gemma 3n or 4 E2B; BitNet b1.58 2B4T |

### 10.8 Measured on the owner's laptop

Intel i5-1135G7 (4 cores), 16 GB, llama-server from `V:\jitmo-sandbox`, Q4_K_M files, **1 thread**. The coding
agent's own benchmark was running on 3 threads at the same time, so the times are slower than on an idle PC;
compare them with each other, not with the budget. Memory use and token counts do not depend on the shared CPU.

| | Qwen3.5-0.8B (508 MB file) | Gemma 3 270M (241 MB file) |
|---|---|---|
| Private memory, context 4096 | **937 MB** | **197 MB** |
| Working set, context 4096 | 1,020 MB | 400 MB |
| Private memory, context 768, batch 128 (the agent's own run, 3 threads) | 580 MB | — |
| Load to ready, then reload right after stopping | 2.7 s, 2.7 s | 1.8 s, 1.4 s |
| First request, whole catalog (~1,800 tokens) | 24.8 s reading the prompt | 11.6 s reading the prompt |
| First request, top 3 tools (~145 tokens) | 2.5 s reading the prompt | 1.0 s reading the prompt |
| Next request, same instructions | 14–20 new tokens read (cache reused) | cache reused for the whole catalog; **not** reused for the top-3 prompt (148 tokens re-read) |
| Output with no schema and no fine-tuning | chats ("I cannot open Notepad for you…") | empty or chatty |

What this shows:

1. **The prompt is the cost.** The whole catalog takes about 10× longer to read than the top 3 tools. Keep
   prompts short and put the fixed instructions first, so the prefix cache can reuse them.
2. **Context size is a memory lever.** The same 0.8B model uses 937 MB private at context 4096 and 580 MB at
   768. Keep the context at 512–1,024 for tool calls.
3. **The prefix cache works on Qwen3.5** (the hybrid-model worry did not hold here), but it does not behave the
   same on every model and chat template. Check it per model in the benchmark.
4. **Private memory is well above the file size** (937 MB for a 508 MB file). Part of it is compute buffers and
   context; part may be weights copied out of the memory-mapped file (CPU weight repacking). Test whether
   keeping the weights mapped (no repacking) lowers private memory without slowing reading too much: mapped
   pages are cheaper for the PC because Windows can drop and reload them.
5. **Loading takes 1.4–2.7 s** even straight after a stop. Hide it: pre-warm when the bar opens.
6. **Untuned tiny models are useless for tool calls without constrained output.** Grammar or JSON-schema
   decoding and fine-tuning are required, not optional.
7. The `Qwen3.5-2B-Q4_K_M.gguf` in the sandbox is **incomplete** (828 MB of about 1.28 GB, last written
   22:50) and must be downloaded again before any 2B result means anything.

Measured with `llmprobe.ps1` (Claude's scratchpad, 10 Nov 2026 ~23:20).
