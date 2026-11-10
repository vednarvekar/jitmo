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

## Still open

- Real-account checks: Google sign-in, sends and calendar writes have only been tested against recorded answers.
- Typed tool arguments instead of `{request}` (P0-1): the model picks tools today but does not yet fill arguments.
- Local file index, PDF reading and study helpers (explain, quiz, flashcards).
- Browser actions, voice, meeting notes, a concurrent core (P1-5).
- A held-out set of real requests as the gate for each phase.
