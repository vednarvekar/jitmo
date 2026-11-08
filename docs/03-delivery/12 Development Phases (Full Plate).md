# Local Assistant Engine — 12 Development Phases (Full Plate)

7 Nov 2026 · @Ved Narvekar

## Summary

This plan builds everything on the plate, from docs 00 through 00.13, in ten phases over about two years. It extends docs 08 (v1 build plan) and 11 (action plan) beyond v1, to the full end goal.

The order follows three rules: prove demand and tech before spending big, build foundations before the features that need them, and ship something useful to real users at the end of every phase.

| Stage | Phases | Months | Outcome |
|---|---|---|---|
| Get ready | 0–1 | 1–2 | Demand and tech proven |
| v1 | 2–4 | 3–8 | Windows launch with 49 tasks |
| v1.x | 5 | 9–11 | Stable, growing, more apps |
| v2 | 6–7 | 12–18 | Voice, automation, search, plugins, sync, Mac |
| v3 | 8 | 19–24 | Marketplace, phone, Enterprise, more languages |
| Long term | 9 | 25+ | Linux, vision, federated learning, partners |

Timelines are estimates. Each phase ends with a gate; the next phase starts only when it's passed. Later phases get re-planned using real user data.

## Roadmap at a glance

**Ten phases take the product from idea to full plate in about two years**

| Phase | Focus | When | Team |
|---|---|---|---|
| 0. Validate | prove demand: interviews, waitlist | Month 1 | 1–2 |
| 1. Prove | prove the tech: 10 spikes on an 8GB laptop | Month 2 | 2–3 |
| 2. Alpha | core engine and the first 15 tasks | Months 3–5 | 6–8 |
| 3. Beta | all 49 tasks, planner, learned defaults | Months 5–7 | 6–8 |
| 4. Launch v1 | hardening, private beta, public launch | Months 7–8 | 8–10 |
| 5. Harden and grow | stability, top-requested apps, first Team customers | Months 9–11 | 10–12 |
| 6. Smarter and voice | voice, automation, local search, more apps | Months 12–15 | 12–16 |
| 7. Platform and reach | plugins, MCP, sync, Mac, Business plan | Months 16–18 | 15–20 |
| 8. Ecosystem | marketplace, phone, Enterprise, Hindi | Months 19–24 | 20–30 |
| 9. Long term | Linux, vision, federated learning, partners | Month 25+ | 30+ |

Phases 0–4 are covered in detail by docs 08 and 11. Phases 5–9 are planned here and get re-planned from real user data after launch.

## How we tackle it

1. **Prove before spending.** Interviews and spikes come first. Big hiring and building start only after demand and tech are proven.
2. **Foundations first.** Things others depend on, like the capability system, signing and the policy engine, are built before the features that use them (see Dependency map).
3. **Design now, build later.** v1 already uses the interfaces that v2 and v3 need (addendum 00.A, now merged into the main docs), so later phases add features instead of rewriting.
4. **Thin slices, end to end.** Each feature is built as a working slice through every layer, from request to verified result, then widened. No half-built layers.
5. **Ship every phase.** Every phase ends with something real users can try, even if small.
6. **Gates, not dates.** A phase moves on only when its gate is passed. Dates are estimates; quality targets are not.
7. **Re-plan with real data.** After launch, user requests and usage counts decide the order inside phases 5–9.
8. **Cut scope, not quality.** If time runs short, ship fewer tasks that work every time.

## Workstreams and owners

The work is split into six workstreams. Each has one owner, even when it's a single person wearing several hats early on.

| Workstream | Covers | Main docs |
|---|---|---|
| Engine and models | Router, extractor, resolver, planner, memory, search, model runtime, scheduler | 00.3, 00.5, 00.7 |
| Apps and actions | Connectors, skills, selector packs, automation runner | 00.4, 00.8 |
| Experience | Quick bar, main window, voice, notifications, accessibility, onboarding | 00.1, 00.2, 00.9, 09 |
| Platform | Plugin host, MCP, local API, sync, update and team servers, Mac and Linux adapters | 00.11, 00.12, 06.1 |
| Trust | Security, privacy, policy engine, signing, compliance, testing | 00.6, 04 |
| Business | Interviews, marketing, pricing, payments, support, sales, legal | 10, 11, 00.10, 00.13 |

## Dependency map

This decides the order. A feature can't start until what it needs exists.

| Feature | Needs first | Phase |
|---|---|---|
| Any send or share | Policy engine, confirm card, origin labels | 2 |
| Any connector | Connector interface, health checks, Undo store | 2 |
| Planner and chained requests | Router, resolver, model scheduler | 3 |
| Learned defaults | Encrypted store, action log | 2–3 |
| Updates of any kind | Signing keys and verification | 4 |
| Team plan | Accounts, licensing, payments, team service | 4 |
| Voice | Unified input format, model scheduler | 6 |
| Automation | Recipes, runner, policy rules for automation | 6 |
| Local search and document answers | Embedding model, encrypted store, indexer | 6 |
| Follow-up conversation | Short-term memory format (designed in v1) | 6 |
| Browser extension | Capability checks, native messaging | 6 |
| WASM plugins | Capability system, WIT interfaces (designed in v1) | 7 |
| MCP tools | Capability system, origin labels | 7 |
| Sync | Encryption, keystore, merge rules | 7 |
| Mac app | Clean OS adapter boundary (kept since v1) | 7 |
| Business plan | Team service, policies, single sign-on | 7 |
| Marketplace | Plugin host, review pipeline, payouts | 8 |
| Phone companion | Sync keys, relay, push | 8 |
| Event triggers | Automation runner; email and file watchers | 8 |
| Enterprise plan | Business plan, SCIM, data regions | 8 |
| Federated learning | Many active users; secure aggregation | 9 |

## Phases 0 and 1: Validate and Prove

Months 1–2. Team: 1–3 people. Details in docs 05 and 11.

| Workstream | Phase 0: Validate (month 1) | Phase 1: Prove (month 2) |
|---|---|---|
| Engine and models | Shortlist open-source models; check licenses | Spikes SP1–SP4 and SP8: router, extraction, planner, memory budget |
| Apps and actions | List top apps from interviews | Spikes SP5–SP7: WhatsApp, Spotify, Do Not Disturb |
| Experience | Clickable mockups of the quick bar and confirm card | Test mockups with 5 users |
| Platform | Merge addendum into docs (done, see the docs README) | Spike SP10: install size; pick UI framework (SP8) |
| Trust | WhatsApp legal question | Spike SP9: trick-instruction blocking |
| Business | 20 interviews; name; landing page; waitlist | Funding or budget; start hiring; register company |

**Gates:** 12 of 20 interviewees would use it weekly; 6 would pay; 300+ waitlist (end of phase 0). 8 of 10 spikes pass; budget secured; 1,000+ waitlist (end of phase 1).

## Phase 2: Alpha

Months 3–5. Team: 6–8. Goal: the core engine working end to end, with the 15 alpha tasks from doc 08.

| Workstream | Deliverables |
|---|---|
| Engine and models | Model worker; scheduler with pinned models; clean-up, router and extractor; resolver for files and contacts; act-ask-decline table |
| Apps and actions | Connector interface; Windows system, files, browser, math; reminders; Gmail and Outlook; Spotify; WhatsApp (with fallback) |
| Experience | Quick bar, result line, confirm card, question chips, Undo toast; first-run onboarding; tray |
| Platform | Core engine service; IPC; local store; installer; capability fields in skill files; WIT interfaces drafted |
| Trust | Policy engine; origin labels; confirm UI that can't be faked; encrypted store; action log; trick-instruction tests in CI |
| Business | Build in public; weekly demos; design partners use each alpha build; Google and Microsoft app verification started |

**Gate:** the 15 alpha tasks pass 90% on their tests; memory under 2GB; no send without a confirm.

## Phase 3: Beta

Months 5–7. Team: 6–8. Goal: all 49 v1 tasks, chained requests and learned defaults.

| Workstream | Deliverables |
|---|---|
| Engine and models | Planner with templates, plan cache and the 1–3B model; prompt caching; learned defaults and personal phrases; facts the user asks to remember |
| Apps and actions | All remaining v1 connectors: calendars, Drive, OneDrive, Teams, YouTube, web lookups; cross-app workflows |
| Experience | Progress list for chains; info cards; morning brief; main window: connectors, settings, privacy panel, history |
| Platform | Update server; license service; web lookup proxy; model downloads from no-fee storage |
| Trust | Network allowlist; scrubber for lookups and logs; capability declarations on every skill |
| Business | Usability tests of all flows; creators get early access; payments set up |

**Gate:** T01–T82 at 90%+ (doc 04); no sends without a confirm; memory and speed targets met.

## Phase 4: Launch v1

Months 7–8. Team: 8–10. Goal: a safe, polished Windows launch.

| Workstream | Deliverables |
|---|---|
| Engine and models | Speed and memory tuning; quantization choices final; staged model rollout |
| Apps and actions | Connector fixes from beta; nightly connector tests; selector-pack hot fixes |
| Experience | Accessibility pass (keyboard, screen readers, high contrast); onboarding polish; help center |
| Platform | Signed updates with offline keys; staged rollout and rollback; Team plan basics (team service, admin console basics, team billing) |
| Trust | Outside penetration test; red team; incident plan rehearsed; privacy policy and terms; security page |
| Business | Private beta (50–100 users), then public launch on Product Hunt and Hacker News; support desk and on-call live |

**Gate:** full 04 test set passes every target; beta day-7 retention 20%+; no open serious security findings.

## Phase 5: Harden and grow (v1.x)

Months 9–11. Team: 10–12. Goal: make v1 rock solid, add the apps users ask for most, and grow.

| Workstream | Deliverables |
|---|---|
| Engine and models | Fix top routing and extraction misses from real use; first fine-tunes if needed; quarterly model re-check |
| Apps and actions | Top requested apps, likely Slack, Telegram and Notion; Windows ARM build if users need it; connector upkeep |
| Experience | Fixes from support tickets; empty-bar tips; better error messages |
| Platform | Export and import of settings and memory; crash and connector health dashboards; operations engineer on board |
| Trust | Bug bounty; per-app permissions screen; security lead owns ongoing reviews |
| Business | Referral program; pricing tests; first Team customers; laptop-maker conversations; v2 planning from real data |

**Gate:** day-30 retention and free-to-paid conversion near doc 10 targets; crash-free sessions above 99%; v2 scope confirmed from user data.

## Phase 6: Smarter and voice (v2, part 1)

Months 12–15. Team: 12–16. Goal: the assistant understands more, remembers more, and can be spoken to.

| Workstream | Deliverables | Docs |
|---|---|---|
| Engine and models | Follow-up conversation; people map; preferences; local search by meaning; ask about your documents; GPU and NPU acceleration | 00.3, 00.5, 00.7 |
| Apps and actions | Saved shortcuts; schedules; recipe builder with dry runs; routine suggestions; Office and Google documents; Zoom, Meet, Todoist, Microsoft To Do | 00.4, 00.8 |
| Experience | Push-to-talk voice; spoken replies; voice-only mode; selected text as context; right-click menu; drag and drop; daily view; full screen reader support | 00.2, 00.9 |
| Platform | Browser extension for Chrome and Edge; search index storage | 00.1, 00.7 |
| Trust | Voice privacy rules; automation safety rules; per-capability limits | 00.6, 00.8 |
| Business | Launch campaign for voice and automation; accessibility partners | 10 |

**Gate:** voice text under 1 second on 8GB laptops; automation tests all pass, including trick emails; search beats name-only search clearly.

## Phase 7: Platform and reach (v2, part 2)

Months 16–18. Team: 15–20. Goal: open the platform to developers, and reach Mac users, multi-device users and growing companies.

| Workstream | Deliverables | Docs |
|---|---|---|
| Engine and models | Vector index for thousands of skills; two-step routing by category | 00.3, Routing at scale (merged from 00.A A4) |
| Apps and actions | Public skill format; validator; first community and partner skills | 00.11 |
| Experience | Mac app (macOS adapter, Accessibility permissions onboarding); main window updates for sync and devices | 00.1, 00.12 |
| Platform | WASM plugin host (Rust plugins first); MCP client; local API and command line; developer tool and docs; encrypted sync with device pairing and recovery key | 00.11, 00.12 |
| Trust | Full capability enforcement for plugins and MCP; outside review of sync encryption; sandbox escape tests | 00.6, 07 LLD, Capabilities (merged from 00.A A3) |
| Business | Business plan: full policies, groups, single sign-on, MSI and update rings; developer community; hackathon | 00.10, 00.11 |

**Gate:** plugins and MCP tools pass all sandbox and capability tests; sync passes the outside encryption review; first Business customers onboarded.

## Phase 8: Ecosystem (v3)

Months 19–24. Team: 20–30. Goal: an ecosystem that grows itself, the assistant in every pocket, and readiness for large companies.

| Workstream | Deliverables | Docs |
|---|---|---|
| Engine and models | Hindi and mixed Hindi-English; multilingual models; proactive nudges (opt-in) | 00.3, 00.8 |
| Apps and actions | Event triggers (email, calendar, files, system); conditions and loops; team automations and recipe sharing; connector toolkit for app makers | 00.8, 00.11 |
| Experience | Phone companion for Android and iPhone; wake word (opt-in); on-screen highlights and show-me mode; translated UI | 00.2, 00.9, 00.12 |
| Platform | Marketplace with review pipeline and payouts; plugins in Python and JavaScript; relay and push; MCP server mode if decided | 00.11, 00.12 |
| Trust | SOC 2; marketplace review team; relay encryption review; red team on plugins | 00.6 |
| Business | Enterprise plan: SCIM, data regions, approval flows, audit export; partner program; laptop-maker pre-install pilots | 00.10, 00.13 |

**Gate:** marketplace live with reviewed skills and working payouts; phone companion passes security review; first Enterprise customer live.

## Phase 9: Long term

Month 25 onward. Team: 30+. Ongoing work, ordered each quarter by user demand and business value.

| Area | Items | Docs |
|---|---|---|
| Reach | Linux app; more languages; more regions | 00.1, 00.9 |
| Any app | Screen understanding fallback with a small vision model; browser actions like filling forms | 00.4 |
| Smarter over time | Federated learning across opted-in users; offline knowledge pack | 00.7 |
| Power users | Developer tools (Git, VS Code, terminal); optional cloud model and cloud runner, off by default | 00.4, 00.5, 00.8 |
| Memory | Encrypted memory sync; action log sync (if decided) | 00.12 |
| Trust | ISO 27001; yearly transparency report | 00.6 |
| Growth | Laptop-maker pre-installs at scale; app-maker partnerships; grants for missing apps | 00.11, 00.13 |

## Ways of working

### Cadence

| Rhythm | What happens |
|---|---|
| Daily | Short check-in per workstream |
| Every 2 weeks | Sprint: plan, build, demo working software to the whole team |
| Weekly | Friday metrics review and decision log update (doc 11) |
| Monthly | Roadmap review: is the phase on track for its gate? |
| Each phase end | Gate review; re-plan the next phase with real data |
| Quarterly | Model re-check; dependency and license audit; security review |

### Definition of done

A feature is done only when all of these are true:

1. It works end to end on the 8GB test laptop, within memory and speed budgets.
2. Its tests pass, including the relevant doc 04 tests and trick-instruction tests.
3. It meets the accessibility checks (doc 00.9).
4. Anything that sends, shares or reads widely has a security review.
5. Its text follows the writing rules; the help center is updated.
6. It's measured: opt-in counts show if it's used and whether it works.

### Releases

A monthly release train for the app; selector packs and skills whenever needed; staged rollouts and rollback for everything (doc 00.13).

## Team growth by phase

**The team grows from 2 to about 30 people over two years**

People per phase (low estimate to high estimate):

| Phase | People |
|---|---|
| 0. Validate | 1–2 |
| 1. Prove | 2–3 |
| 2. Alpha | 6–8 |
| 3. Beta | 6–8 |
| 4. Launch v1 | 8–10 (v1 launch) |
| 5. Harden and grow | 10–12 |
| 6. Smarter and voice | 12–16 |
| 7. Platform and reach | 15–20 |
| 8. Ecosystem | 20–30 |
| 9. Long term | 30+ |

<!-- Note: the photos show only the top of the text after this chart's intro; a sentence listing when new roles join (support lead before the beta, marketing before launch, operations after launch, developer relations and sales in v2, partnerships in v3) appears partly cut off ("…near launch, operations after launch, developer relations and sales in v2, partnerships in v3."). -->

## Risks across phases

| Risk | Phases most affected | Response |
|---|---|---|
| Demand weaker than hoped | 0–1 | Stop or change the idea at the phase 0 gate, before spending big |
| Small models miss accuracy targets | 1–3 | Fine-tune cheaply; narrow scope; templates over planning |
| App terms block automation (WhatsApp and others) | 1, 4, 5 | "User presses Send" mode; prefer apps with official APIs |
| Funding runs short | 2–7 | Gates protect spending; shrink to the alpha 15 tasks if needed |
| Connectors break with app updates | 4 onward | Nightly tests, selector packs, an owner per connector |
| Too much in v2 at once | 6–7 | Split v2 into two phases; re-plan from real data; cut scope, not quality |
| Big OS makers ship similar features | 5 onward | Win on third-party apps, older laptops, privacy and the developer ecosystem |
| Security incident | Any | Defenses from doc 00.6; rehearsed incident plan; kill switch |
| Platform takes off slower than hoped | 7–8 | Grants, partner connectors, MCP for existing tools |
| Team growth outpaces process | 6–8 | Clear owners per workstream; definition of done; gates |

## Open decisions and what to do first

### Decisions that shape this plan

- [ ] Bootstrap or raise money, and how much (sets the team size per phase)
- [ ] Main launch market: India, global, or both
- [ ] Product name (needed before phase 0 ends)
- [ ] Open-source the engine core (affects phase 7 developer adoption)
- [ ] Which v2 items move earlier if users ask, like voice or Mac

### What to do first

1. Start the phase 0 checklist from doc 11 this week: interviews, name shortlist, landing page, waitlist, WhatsApp legal question.
2. ~~Merge the addendum (00.A) into the main docs.~~ Done: see the docs README.
3. Build one decision log from every doc's open decisions.
