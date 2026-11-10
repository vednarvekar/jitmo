# 13. End-to-End Audit (10 Nov 2026)

A strict audit of jitmo against the owner's stated goal: **a personal assistant at your fingertips for
everyone — a student and an office worker alike — that answers questions, helps read and understand study
material, opens apps, messages friends, checks schedules, plays music, browses and acts on websites, connects
to the company workspace (Microsoft 365 or Google), gives meeting heads-ups, takes notes, shares files and
answers messages; local models only, a smart engine rather than a rule follower, the best UI/UX.**

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
