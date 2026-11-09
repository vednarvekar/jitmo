# jitmo (working name)

- What it is: a local-first Windows desktop assistant. The user presses a hotkey, types a request, and it performs the action itself (open apps, system settings like Do Not Disturb, find/move/rename/delete files, reminders and timers, play music, quick calculations and lookups). It does not chat; it acts.
- Who it is for: NOT CHOSEN YET. Candidates being considered: Windows office workers at small companies; small businesses in India that run on WhatsApp, Excel, Gmail; teams in privacy-restricted fields (law, finance, clinics) that have banned cloud AI tools. Open question for the founder.
- What it sells, at what price: PRICE NOT CHOSEN. Idea: free personal tier to spread installs, paid tier later. No number has been tested with any buyer.
- Where and how: Windows 11 desktop app, downloaded and installed. Works on 8 GB laptops. Delivery and distribution not decided; code signing route (India-based founder) unresolved.
- Budget and constraints: solo founder in India (the user), no revenue, no customers or interviews yet, token and time limited. Windows only for v1.

## How it works (technical)
- Everything runs on the user's laptop. Small models plus rules and fixed skill templates. Cheapest-first ladder: exact-match cache, rules and the user's own contacts/apps/files, tiny router model, extractor model, planner only for chains. Most requests should need no AI model.
- The AI runs in a separate helper process that starts on demand, runs at low priority, and exits when idle, so it never slows a developer's PC with many windows open. Idle app is about 14 MB.
- Safety by design: a policy engine in plain code, confirmation before anything is sent, Undo.
- Cloud only for online integrations (Outlook, Gmail, Teams) and optionally for low-confidence requests, with an off switch. Not the core.
- Differentiator claimed: privacy and local speed, runs on modest hardware, acts across apps instead of answering questions.

## Measured so far (on the founder's dev PC, not the 8 GB target laptop)
- Task router: 81% to 92% correct (fresh test set vs original test set); chained requests about 60%.
- Detail extractor (names, files, times): about 63% on a fresh blind set. Rules plus the user's own contact/app lists are the plan for the rest.
- 1.5B-parameter planner for multi-step requests: failed (4 of 8 tests, too slow, 2 GB memory).
- Idle memory 14 MB; with the model helper loaded about 300 MB; command bar window about 160 MB while open.
- The founder can control Do Not Disturb, open apps, and Spotify playback; WhatsApp only by opening the chat (user presses Send).
- Competition noted: Windows Copilot and Microsoft 365 Copilot (about $21 per user per month for small businesses) give much of the basic assistant for free or bundled.
