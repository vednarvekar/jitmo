# Local Assistant Engine — 10 Go-to-Market and Pricing

6 Nov 2026 · @Ved Narvekar

## Summary

Lead with privacy and speed: a helper that runs on your own laptop, even an 8GB one. Reach early adopters first through a free plan, then convert power users and small teams to paid plans.

Because all AI runs on the user's device, each extra user costs almost nothing. That lets us price well below cloud assistants and still keep high margins.

All prices here are starting guesses. They get tested in user interviews and the beta.

## Positioning

**Pitch:** "Your computer helper that does the boring clicks in seconds. Private, because it runs on your laptop."

| Alternative | Their strength | Our edge |
|---|---|---|
| Cloud AI assistants (ChatGPT, Gemini, Claude apps) | Very smart, great at open questions | Acts on your apps and files directly; private; works offline; no usage limits |
| OS built-in assistants (Copilot in Windows, Apple Intelligence) | Pre-installed, free | Works across third-party apps like WhatsApp and Spotify; runs on older 8GB laptops |
| Cloud desktop agents | Can handle complex tasks | No data leaves the device; much lower cost; faster on everyday tasks |
| Local hobby agents (open-source) | Free, private | Polished, reliable, safe confirms, works out of the box |
| Automation tools (Zapier, Power Automate) | Powerful workflows | No setup; just type what you want |

We don't try to beat cloud AI at answering hard questions. We win at doing everyday computer tasks fast and privately.

## Who to reach first

Start with people who sit between both target groups: freelancers, startup founders and solo professionals. They use the assistant for personal tasks and work tasks, they try new tools early, and they often pay for productivity apps.

| Order | Group | Why now |
|---|---|---|
| 1 | Freelancers, founders, solo professionals | Early adopters; feel both personal and work pain; quick to give feedback |
| 2 | Productivity and privacy fans | Spread the word online; value "runs on my laptop" |
| 3 | Small teams and startups (2–20 people) | Paid team plans; often brought in by group 1 |
| 4 | General personal users | Largest group; reached later through word of mouth |

## Pricing plans

Three plans. Prices are starting guesses in US dollars, to be tested.

| | Free | Pro | Team |
|---|---|---|---|
| Price (guess) | $0 | $5–8 per month | $10–15 per user per month |
| For | Trying it out | Individuals and power users | Startups and small businesses |
| Tasks | Apps, media, quick info, files, reminders, system controls | All 49 tasks | All 49 tasks |
| Sending (messages, email, files) | 20 sends per month | Unlimited | Unlimited |
| Chained requests and morning brief | No | Yes | Yes |
| Work suites (Google, Microsoft) | Read only | Full | Full |
| Team features | No | No | Shared skills; admin console in a later version |
| Support | Community | Email | Priority email |

Why this split: free covers the "wow" moments that bring people in. Sending and work tasks are where the real time savings are, so those sit in paid plans.

Option to test: a one-time lifetime license for Pro, popular with privacy-minded buyers of local apps.

## Costs and margins

The biggest advantage of running locally: no AI bill per request. Cloud assistants pay for every question; we don't. (Web lookups are the one per-request cost.)

| Cost | Scales with | Size |
|---|---|---|
| AI compute per request | Nothing; it runs on the user's laptop | Zero |
| Web lookups (weather, search) | Lookups per day | Small but not zero; these do cost per request, so cap them per plan (see [00.4, Web lookups](../00-vision/00.4%20Actions%20and%20App%20Coverage.md)) |
| Downloads (installer, model updates) | Users and update size | Small; most updates are a few MB |
| Update server and database | Users, slowly | Small |
| Payment fees | Revenue | A few percent of each payment |
| Support | Paying users | Grows over time; keep low with good in-app help |
| Team salaries | Not users | The main cost |

So margins on each paid user should be high. The real question is reaching enough paying users to cover the team. That number gets worked out once the budget is set.

## Launch plan

The launch follows the build phases in 08, so marketing starts while the product is still being built.

1. **Now: interviews and waitlist.** Talk to 10–20 target users. Put up a simple landing page with a waitlist and a short demo video.
2. **During alpha (weeks 6–15): build in public.** Share progress on LinkedIn, X and Reddit. Show real tasks running on an 8GB laptop. Grow the waitlist.
3. **Launch prep (weeks 24–29): private beta.** Invite 50–100 people from the waitlist, mostly freelancers and founders. Fix what they report.
4. **Public launch.** Launch on Product Hunt and Hacker News on the same week. Reach out to productivity YouTubers and newsletters with early access.
5. **After launch: teams.** Reach startups through founder communities and accelerators. Offer a free first month on the Team plan.

## Growth

- **Referrals:** each friend who installs gives both people one free month of Pro.
- **Time-saved report:** a weekly note on the device, like "You saved 47 minutes this week." Users can choose to share it. The number is calculated locally and never sent anywhere by us.
- **Team spread:** one person on a team uses it, colleagues notice, and the team upgrades together.
- **Shareable shortcuts (later):** users share routines, like a "Friday report" workflow, as links others can install.
- **Content:** short videos of real tasks done in seconds. These show the product better than any description.

## Metrics

These come from the opt-in, anonymous usage counts. No request text or personal data is collected. Targets are starting guesses for the beta.

| Metric | What it shows | Beta target (guess) |
|---|---|---|
| Waitlist signups | Interest before launch | 2,000+ |
| Activation | First task done within 60 seconds of install | 80%+ of installs |
| Tasks per active user per day | Whether it becomes a habit | 5+ |
| Day-7 retention | Still using it after a week | 40%+ |
| Day-30 retention | Still using it after a month | 25%+ |
| Task success rate | Matches the 04 test results in real life | 90%+ |
| Free to paid | Willingness to pay | 3–5% of active users |

If activation or day-7 retention misses, fix the product before spending on marketing.

## Risks and open decisions

| Risk | Response |
|---|---|
| Microsoft or Apple build similar features into the OS | Win on third-party apps, older laptops and privacy; move fast |
| Personal users won't pay | Lean on Team plans; test a lifetime license |
| WhatsApp terms block automation | "User presses Send" mode, or drop it, before launch |
| Users don't trust an assistant with their apps | Lead with privacy, confirms, Undo and the visible action log |

Open decisions:

- [ ] Test prices in user interviews: what would they pay for Pro and Team?
- [ ] Main launch market and currency: India, global, or both?
- [ ] Payment provider: Stripe, Razorpay, or both
- [ ] Product name and landing page, needed before the waitlist
- [ ] Lifetime license: offer it at launch or not?

*Merged from: 00.A Addendum, item A7 (web lookups have a per-request cost).*
