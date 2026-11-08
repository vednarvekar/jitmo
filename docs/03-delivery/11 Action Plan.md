# Local Assistant Engine — 11 Action Plan

6 Nov 2026 · @Ved Narvekar

## Summary

This is the plan to go from documents to a launched product in about 33 weeks. It runs four workstreams in parallel: build, users, marketing and business.

The key idea: marketing and user work start this week, not after the product is built. The first 4 weeks test demand before any big money or hiring. Each stage ends with a go or no-go check.

## Plan at a glance

**Five stages to launch, about 33 weeks, across four workstreams**

| | Validate<br>Weeks 1–4 | Prove<br>Weeks 5–9 | Alpha<br>Weeks 10–19 | Beta<br>Weeks 20–27 | Launch<br>Weeks 28–33 |
|---|---|---|---|---|---|
| **Build** | Close decisions, pick models | 10 spikes on 8GB laptop | Core + 15 tasks | All 49 tasks + planner | Hardening, installer |
| **Users** | 20 interviews, price test | 5 design partners signed | Partners try alpha weekly | Usability tests, all 49 tasks | Private beta, then public |
| **Marketing** | Name, landing page, waitlist | Build in public, weekly demos | Demo videos, grow waitlist | Creators get early access | Product Hunt, Hacker News |
| **Business** | Budget, legal, WhatsApp check | Funding, hire spike team | Hire full team, Google verify | Payments and pricing | Team plan outreach |

Read each column as one stage, top to bottom. All four workstreams move every week, so the product, its first users and its audience grow together.

## Pre-implementation gaps and where each is handled

The earliest gap review listed what had to be addressed before coding. Each item and its home:

| Gap | When | Where it is handled |
|---|---|---|
| Talk to real users: validate with 10–20 target users (do they want it, which tasks matter, would they pay) | Must do before coding | Weeks 1–4 below |
| WhatsApp and legal check; start Google API verification early (it can take weeks) | Must do before coding | Weeks 1–4 below; [9 Legal and Platform Checks](../04-business-legal/9%20-%20Legal%20and%20Platform%20Checks.md) |
| Close the open decisions (Google or Microsoft first, React or native UI, the alpha tasks) in a single decision log | Must do before coding | [8.1 Decision Log](8.1%20-%20Decision%20Log.md) |
| UX flows: onboarding, app sign-ins, permissions, confirm screens, error messages (onboarding is where most users drop off) | Must do before coding | [09 UX Flows](../02-design/09%20UX%20Flows.md) |
| Budget and funding: about 7 months of a 6–8 person team, plus GPUs, cloud, code signing and legal needs a number | Must do before coding | Weeks 1–4 and 5–9 below |
| Go-to-market and pricing: free vs paid, and how to reach the first users | In parallel, early | [10 Go-to-Market and Pricing](../04-business-legal/10%20Go-to-Market%20and%20Pricing.md) |
| Security threat model: how the engine could be attacked or misused | In parallel, early | [00.6, Threat model](../00-vision/00.6%20Safety%2C%20Privacy%20and%20Trust.md) |
| Privacy-safe usage stats: opt-in, anonymous counts of which tasks work or fail | In parallel, early | [00.13, Privacy-safe product analytics](../00-vision/00.13%20Business%20and%20Operations.md) |
| Engineering basics: repo layout, code review rules, definition of done | In parallel, early | Repo layout in [8 Phase 1](8%20-%20Development%20Phases.md); definition of done in [12](12%20Development%20Phases%20%28Full%20Plate%29.md) |
| Test hardware: buy a couple of real 8GB Windows laptops now | In parallel, early | Weeks 5–9 below |
| Product name and brand | Can wait | This week (name shortlist) |
| Beta support and feedback channel | Can wait | [00.13, Customer support](../00-vision/00.13%20Business%20and%20Operations.md) |

The review's closing question was whether to tackle the decision log or the user interview plan first. The decision log now exists (8.1), and the interview script is in "This week" below.

## This week

Small, concrete steps. Each takes a few hours at most.

- [ ] Shortlist 3 names. Check trademarks, .com or .ai domains, and the Microsoft Store.
- [ ] Buy the domain for the chosen name.
- [ ] Make a one-page landing page with the pitch and a waitlist form.
- [ ] Write a 10-question user interview script.
- [ ] Message 30 people who fit the first group (freelancers, founders, solo professionals). Aim for 20 interviews.
- [ ] Create LinkedIn and X accounts for the product. Post the first "why I'm building this" note.
- [ ] Email a lawyer about WhatsApp's automation terms.
- [ ] Read Google's app verification rules for Gmail and Drive access, and note the timeline.
- [ ] Start the feasibility spikes ([05](5%20-%20Feasibility%20Spikes.md)) now, in parallel with the remaining docs. Real evidence early beats more planning.

## Weeks 1–4: Validate

Goal: prove people want this and will pay, before spending on engineers.

| Workstream | To-dos |
|---|---|
| Users | Run 20 interviews. Ask which tasks waste their time, which apps they use, and what they'd pay. Record the answers in one sheet. |
| Build | Close the open decisions from docs 01–10 in a single decision log. Shortlist open-source models to test. |
| Marketing | Launch the landing page. Post twice a week about the problem and the plan. Share a mockup video of the command bar. |
| Business | Get the WhatsApp legal answer. Draft a budget for 8 months. Decide: bootstrap or raise money. |

**Exit criteria (all must be true to move on):**

1. At least 12 of 20 interviewees say they'd use it weekly.
2. At least 6 say they'd pay at the guessed Pro or Team price.
3. At least 300 waitlist signups.
4. The WhatsApp question has a clear answer, and scope is adjusted if needed.

## Weeks 5–9: Prove

Goal: prove the tech works on an 8GB laptop, and line up money and people.

| Workstream | To-dos |
|---|---|
| Build | Run the 10 spikes from doc 05 with two engineers. Buy two 8GB Windows test laptops. Update the HLD and LLD with real numbers. |
| Users | Sign up 5 design partners from the interviews: people who agree to try every early build and give feedback weekly. |
| Marketing | Post a short video of each spike that works, like "playing a song from one typed line, on an 8GB laptop." |
| Business | Secure funding or commit savings. Start hiring the full team from doc 08. Register the company if not done. Start Google and Microsoft app registration. |

**Exit criteria:**

1. At least 8 of 10 spikes pass, and no blocking failure is left without a fallback.
2. Budget for the next 6 months is secured.
3. At least 3 of the remaining hires have accepted offers.
4. At least 1,000 waitlist signups.

## Weeks 10–33: Build and launch

The build follows doc 08. Marketing and users run alongside it.

| Stage | Build | Users | Marketing | Business |
|---|---|---|---|---|
| Alpha (weeks 10–19) | Core engine + 15 alpha tasks | Design partners use each alpha build | Weekly demo videos; grow waitlist | Finish hiring; Google API verification |
| Beta (weeks 20–27) | All 49 tasks, planner, defaults | Usability tests of all flows with 5 users | Give 10–20 creators early access | Set up payments and plans |
| Launch (weeks 28–33) | Hardening, installer, update server | Private beta with 50–100 waitlist users, then public | Product Hunt and Hacker News launch week | Team plan outreach to startups |

Each stage still has to pass its gate from doc 08 before the next one starts.

## Weekly rhythm

The same simple routine every week keeps all four workstreams moving.

| Day | What happens |
|---|---|
| Monday | Pick the week's top 3 goals, one per workstream at least. Keep the list short. |
| Tuesday–Thursday | Build. Talk to at least 2 users or design partners. |
| Wednesday | Post one update or demo publicly. |
| Friday | Demo what got built, even if small. Update the metrics sheet: waitlist, interviews, test-set score, spend. |
| Friday | Write down any decisions made in the decision log. |

Rule of thumb: If a week ends with no user conversation and no public post, the next week fixes that first.

## Go or no-go checkpoints

Deciding in advance what "failure" looks like stops months of work on the wrong thing.

| When | If this happens | Then |
|---|---|---|
| End of week 4 | Fewer than 8 of 20 people would use it weekly | Stop. Re-talk to users about their real pain, and change the product idea before building. |
| End of week 4 | Interest is strong, but few would pay | Continue, but shift focus to Team plans and test other prices. |
| End of week 9 | 3 or more spikes fail with no workable fallback | Narrow scope to what works, or raise the minimum hardware. |
| End of week 9 | No funding and no savings to cover 6 months | Shrink to a 2–3 person team and a smaller v1 of the 15 alpha tasks. |
| End of week 19 | Alpha misses its gate twice | Cut tasks, not quality. Ship fewer tasks that work every time. |
| End of private beta (about week 31) | Private beta day-7 retention under 20% | Delay public launch. Fix the top 3 complaints first. |

## Tools and inputs needed

Simple tools are enough at the start:

| Need | Tool |
|---|---|
| Plans and docs | These docs (01–11) |
| Tasks and sprints | GitHub Projects or Linear |
| Decision log and metrics | One shared spreadsheet |
| Landing page and waitlist | A no-code site builder with a form |
| Interviews | Video calls, with notes in the same spreadsheet |
| Code | GitHub, as in 06.1 |

Inputs needed from you, to fine-tune this plan:

- [ ] Are you building solo for now, or with co-founders?
- [ ] Will you write code yourself, or lead and hire engineers?
- [ ] Rough budget available, or plans to raise money?
- [ ] How many hours a week can you give this right now?

*Merged from: 0 - pre-implementation (gap list, now the table above) and 00.A Addendum item A6 (start spikes in parallel).*
