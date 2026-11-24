# Local Assistant Engine: docs index

Doc numbers (00.x, 1 to 12) are stable and are what the docs use when they say "see 07" or "SP2". Folders only group them. Note the two near-identical numbers: **09** is UX Flows, **9** is Legal and Platform Checks.

## Folders

### 00-core: the core of the project (read first)

Since 18 Nov 2026 the core is a new model runtime: many small models swapped fast, and big models streamed from the SSD with only the needed parts in RAM. The assistant is its first user.

| Doc | What it is |
|---|---|
| [C0 Vision](00-core/C0%20-%20Vision.md) | What the runtime is, what "works the same" means, targets, principles, what it is not |
| [C1 Blockers](00-core/C1%20-%20Blockers.md) | The physics limit (SSD vs RAM), the ranked blockers, what is not a blocker |
| [C2 Path and First Experiments](00-core/C2%20-%20Path%20and%20First%20Experiments.md) | Runtime parts, phases, first experiments, decisions waiting on the owner |

### 00-vision: the end goal (everything the finished product should have)

| Doc | What it is |
|---|---|
| [00 Full Product Map (End Goal)](00-vision/00%20Full%20Product%20Map%20%28End%20Goal%29.md) | Master list of all areas and features with version tags (v1, v2, Later); guiding principle |
| [00.1 Product Form and Platforms](00-vision/00.1%20Product%20Form%20and%20Platforms.md) | Desktop app, Mac, Linux, browser extension, CLI, phone; hardware tiers; install and update |
| [00.2 Input and Voice](00-vision/00.2%20Input%20and%20Voice.md) | Typed input, context capture, push-to-talk voice, wake word, input privacy |
| [00.3 Understanding Engine](00-vision/00.3%20Understanding%20Engine.md) | The five stages from request to checked plan; routing at scale; tests; edge cases |
| [00.4 Actions and App Coverage](00-vision/00.4%20Actions%20and%20App%20Coverage.md) | Connectors, control methods, every v1 app, sign-in, Undo, MCP as connector type |
| [00.5 Models and Runtime](00-vision/00.5%20Models%20and%20Runtime.md) | Model list, runtimes, scheduler, quantization, licenses, updates |
| [00.6 Safety, Privacy and Trust](00-vision/00.6%20Safety%2C%20Privacy%20and%20Trust.md) | Threat model, trick-instruction defense, data flows, permissions, plugin trust, signing |
| [00.7 Memory, Personalization and Local Search](00-vision/00.7%20Memory%2C%20Personalization%20and%20Local%20Search.md) | Learning, people map, preferences, local search, document Q&A, federated learning |
| [00.8 Automation](00-vision/00.8%20Automation.md) | Recipes, triggers, schedules, automation safety rules |
| [00.9 Output and Accessibility](00-vision/00.9%20Output%20and%20Accessibility.md) | Result surfaces, writing rules, notifications, spoken replies, accessibility, localization |
| [00.10 Teams and Enterprise](00-vision/00.10%20Teams%20and%20Enterprise.md) | Plans by company size, admin console, policies, SSO, deployment |
| [00.11 Developer Platform](00-vision/00.11%20Developer%20Platform.md) | Extension types, skill format, WASM plugins, MCP, marketplace, review pipeline |
| [00.12 Sync and Multiple Devices](00-vision/00.12%20Sync%20and%20Multiple%20Devices.md) | End-to-end encrypted sync, device pairing, phone companion |
| [00.13 Business and Operations](00-vision/00.13%20Business%20and%20Operations.md) | Accounts, payments, releases, analytics, support, infrastructure, costs |

### 01-v1-spec: what version 1 is

| Doc | What it is |
|---|---|
| [1 Problem & Target User](01-v1-spec/1%20-%20Problem%20%26%20Target%20User.md) | The problem, vision, first customer, V1 pains and constraints |
| [2 Scope](01-v1-spec/2%20-%20Scope.md) | The 49 V1 tasks (A, M, E, Q, F, C, R, W, S) with risk levels, and what is out of scope |
| [3 Requirements](01-v1-spec/3%20-%20Requirements.md) | FR, SR, PR, PS, RQ requirement IDs and the integration methods table |
| [4 Success Metrics and Test Set](01-v1-spec/4%20-%20Success%20Metrics%20and%20Test%20Set.md) | The 100-request test set (T01 to T100) and the pass metrics |

### 02-design: how V1 is built

| Doc | What it is |
|---|---|
| [6 High-Level Design](02-design/6%20-%20High-Level%20Design.md) | Architecture, components, request flow, model tiers, plugin host |
| [6.1 Tech Stack](02-design/6.1%20-%20Tech%20Stack.md) | Languages, libraries, models per task, update server, CI; WASM plugin sandbox |
| [7 Low-Level Design](02-design/7%20-%20Low-Level%20Design.md) | Processes, skill file format, capabilities, router, resolver, planner, policy, connector trait, tables, errors |
| [09 UX Flows](02-design/09%20UX%20Flows.md) | Screens, onboarding, confirm, question, Undo and error flows, shortcuts |

### 03-delivery: how and when it gets built

| Doc | What it is |
|---|---|
| [5 Feasibility Spikes](03-delivery/5%20-%20Feasibility%20Spikes.md) | The 10 spikes (SP1 to SP10) that test the riskiest ideas |
| [8 Development Phases](03-delivery/8%20-%20Development%20Phases.md) | The V1 build plan: phases 0 to 5 with tests per phase |
| [8.1 Decision Log](03-delivery/8.1%20-%20Decision%20Log.md) | Every open decision from docs 1 to 7, with recommendations and answers |
| [11 Action Plan](03-delivery/11%20Action%20Plan.md) | 33-week plan across build, users, marketing and business, with go or no-go checks; pre-implementation gap list |
| [12 Development Phases (Full Plate)](03-delivery/12%20Development%20Phases%20%28Full%20Plate%29.md) | End-goal roadmap: ten phases over about two years (V1 is phases 0 to 4; 8 and 11 give the detail) |
| [16 Audit and Engine Work (14 Nov)](03-delivery/16%20-%20Audit%20and%20Engine%20Work%20%2814%20Nov%29.md) | Second end-to-end audit: findings, 13 tool-choosing approaches measured, the fine-tuned tool encoder, voice and orb plans, next steps, decisions waiting on the owner |
| [17 Models on jitmo-runtime (22 Nov)](03-delivery/17%20-%20Models%20on%20jitmo-runtime%20%2822%20Nov%29.md) | The language model moves to jitmo-runtime: what moved, request mapping, how jitmo starts the runtime, settings, identical results on all three sets, real-app check, open items |

Relationship: **8** is the V1 engineering build; **12** is the whole-product roadmap that contains V1 as phases 0 to 4; **11** is the calendar and business side of the same V1 journey.

### 04-business-legal

| Doc | What it is |
|---|---|
| [9 Legal and Platform Checks](04-business-legal/9%20-%20Legal%20and%20Platform%20Checks.md) | Researched verdicts on WhatsApp, Spotify, YouTube, Google, Microsoft, GDPR and DPDP |
| [10 Go-to-Market and Pricing](04-business-legal/10%20Go-to-Market%20and%20Pricing.md) | Positioning, first audience, plans and pricing, costs, launch plan, metrics |

### _archive: superseded sources (safe to delete once reviewed)

`00.A Addendum - Additions to Merge.md` and `0 - pre-implementation.md`. Their content now lives in the docs listed under "Moved and merged". A full copy of the original docs folder is also in `../docs-backup-2026-11-08`.

## Where to find what

| Topic | Go to |
|---|---|
| What is in v1 / which tasks | 2 Scope |
| Requirement IDs (FR, SR, PR, PS, RQ) | 3 Requirements |
| Test cases T01 to T100 | 4 Success Metrics and Test Set |
| Risky assumptions and spikes SP1 to SP10 | 5 Feasibility Spikes |
| Architecture and components | 6 HLD |
| Libraries, languages, hosting | 6.1 Tech Stack |
| Skill file format, capabilities, tables, error messages | 7 LLD |
| Screens and user flows | 09 UX Flows |
| Phases, tests per phase, schedule slips | 8 Development Phases |
| Open decisions and their answers | 8.1 Decision Log |
| Week-by-week plan, go or no-go | 11 Action Plan |
| Whole-product roadmap, team growth | 12 Full Plate |
| How requests are understood (route, extract, resolve, plan) | 00.3 |
| App connectors, OAuth, Undo per action | 00.4 |
| Models, memory budget, scheduler | 00.5 |
| Security, privacy, data flows, abuse | 00.6 |
| Memory, learning, local search | 00.7 |
| Automation and recipes | 00.8 |
| Notifications, accessibility, writing rules | 00.9 |
| Team and company features | 00.10 |
| Plugins, MCP, marketplace | 00.11 |
| Sync, phone companion | 00.12 |
| Payments, releases, support, costs | 00.13 |
| Terms of service, API verification, privacy law | 9 Legal and Platform Checks |
| Pricing and launch | 10 Go-to-Market and Pricing |

## Moved and merged

Old name (in `docs/`) to new path. All filenames are unchanged, only the folder is new, except the two merged away.

| Old | New |
|---|---|
| 00 Full Product Map (End Goal).md | 00-vision/ (same name) |
| 00.1 to 00.13 (all) | 00-vision/ (same names) |
| 00.A Addendum - Additions to Merge.md | Merged into: 00 (guiding principle, developer platform rows), 00.3 (routing at scale, rules-first extraction), 00.4 (MCP connector type, pointers), 00.5 (runtimes), 00.6 (capability link), 00.11 (MCP table, open decisions), 01-v1-spec/2 (v1 only note), 3 (PR9, PS9), 02-design/6 (plugin host, Mac timing, memory note), 6.1 (WASM ideas, native UI test), 7 (capabilities, fields, Shell), 04/10 (web lookup cost), 03/11 (spikes in parallel). Original in `_archive/`. Not merged because the target is being edited by another agent: 5 Feasibility Spikes (SP2 rules-first and SP8 native UI test are noted in 00.3 and 6.1 instead) |
| 0 - pre-implementation.md | Merged into 03-delivery/11 Action Plan, "Pre-implementation gaps" table. Original in `_archive/` |
| 1, 2, 3, 4 | 01-v1-spec/ (same names) |
| 6, 6.1, 7, 09 UX Flows | 02-design/ (same names) |
| 5, 8, 8.1, 11, 12 | 03-delivery/ (same names) |
| 9 Legal and Platform Checks, 10 Go-to-Market and Pricing | 04-business-legal/ (same names) |

## De-duplication done

- 00.1 and 00.2 summary version-tag tables removed; they were subsets of the tables in 00 (now linked).
- 00.4 "Approval by Google and Microsoft" table and the WhatsApp, Spotify, YouTube, Google and Microsoft rows of its legal table replaced by links to 9 Legal and Platform Checks (the canonical research). Rows for Slack, Telegram, Discord, weather and search APIs stay in 00.4.
- Addendum A3 and A5 text was already largely in 00.6 and 00.11; one copy kept (07 for the capability rules, 00.11 for MCP), the other points to it.
- Not touched (other agent's files): the "Legal and platform checks" table inside 5 Feasibility Spikes still overlaps 9 and could become a link later.

## Known conflicts between docs (not resolved here)

1. First customer and size of V1: 1 says Microsoft 365 office workers and 20 to 30 tasks; 2 has 49 tasks, both suites and personal users; 10 targets freelancers and founders first. (8.1 D1 to D3)
2. One shared encoder (6, 7) versus separate embedding model plus GLiNER extractor (6.1, 00.3, 00.5). (8.1 D4)
3. Download size: 5 SP10 still says under 3GB with all models; 3 PR9 and 00.1 now say small installer, about 4GB disk; 00.13 says 2 to 3GB of models per install.
4. Schedule: 11 (alpha weeks 10 to 19, launch weeks 28 to 33), 10 (alpha weeks 6 to 15, private beta weeks 24 to 29), 12 (alpha months 3 to 5, launch months 7 to 8), 8 (about 7 months).
5. Team size: 8, 11 and 12 assume a 6 to 8 person team; 8.1 D9 answers "just me".
6. Confidence thresholds: 07 declines at out-of-scope 0.60 and asks below 0.50; 00.3 declines when nothing is above 0.50 and asks when the top two are within 0.10.
7. Legal versus scope: 9 advises avoiding `gmail.readonly` and `gmail.modify`, but tasks E1 to E4 need to read mail; 9 says the Spotify Web API is likely not allowed for voice-style control, while 3 and 00.4 keep it as an alternative; 9 treats WhatsApp automation as Risky while 8.1 D8 keeps it with "user presses Send"; 9 says GDPR likely does not apply and assumes an India-first market, while 00.6 lists GDPR as a v1 duty and the launch market is undecided.
8. Google verification: 00.4 used to say restricted scopes may need a yearly outside assessment; 9 says a paid CASA audit, avoidable for a local-only app (confirm with Google). 9 is now the only version.
9. Doc numbering: 5 and 8 refer to go-to-market as "09"; it is 10. The UX doc is 09 and the legal doc is 9.
10. 5 says 10 spikes; 8 proposes an SP11 that is not yet in 5.

Source gaps from the original screenshots (text cut off, not recoverable here): the "When" column in the 00 Output table, part of the "Design now, build later" table in 00, the end of 07's and 00.A's open decisions, clipped cells in several 00.x tables, clipped lines in 6.1, and the empty Open decisions heading in 09.
