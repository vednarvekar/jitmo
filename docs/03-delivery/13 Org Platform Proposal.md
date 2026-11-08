# 13 Org Platform Proposal: from personal assistant to company-wide assistant

8 Nov 2026 · prepared for Ved Narvekar · status: **reviewed, partly accepted (see below)**

> ## Status after review (8 Nov 2026): read this first
> The rest of this file is the original proposal, kept as written. Decisions made since (see 8.1, D26 to D34, and the HLD section "Resource and cloud rules"):
>
> | Item in this proposal | Decision |
> |---|---|
> | Device agent with small footprint, work on demand, polite background process | **Accepted** and measured (HLD, D27, D28) |
> | Cloud only for integrations and for low-confidence assist, local result when offline | **Accepted** (D26). Replaces "hybrid is the default" in 5.4: local stays the core, cloud is an extra |
> | Local-only mode and an admin off switch for cloud assist | **Accepted** |
> | Signing route, SmartScreen, Azure Artifact Signing not available in India | **Accepted** as an open task (D30) |
> | Ask-a-colleague as the hero feature | **Dropped for now.** No evidence of demand (see section 10, risk 1) |
> | Org control plane, relay, SCIM, phone to laptop | **Deferred** until buyer conversations confirm demand (D29) |
> | Pricing and first customer | **Open.** Pick one niche and interview 10 people first (D29, D31) |
>
> Still true and useful from this file: sections 3.1 to 3.3 (identity, transport and Windows deployment facts), 7 (threats), 12 (cheap experiments E1 to E9) and 15 (what was not verified).

## How to read this document

Every factual claim carries a tag, so you can see what is proven and what is a guess.

| Tag | Meaning |
|---|---|
| **[R]** | Measured in this repo (spike results, decision log, code) |
| **[P]** | Read directly on a primary source (Microsoft Learn, Tauri docs, Anthropic docs, iroh docs) during this research |
| **[S]** | Secondary source (blogs, news, vendor pages, search snippets). Plausible, not verified |
| **[A]** | My assumption or inference. Needs a test or a customer conversation |
| **[C]** | Computed by me from numbers tagged above. Check the inputs |

Two limits on the research itself. First, several pages were long and the fetch tool returned part of them as a machine summary; those facts are tagged **[S]** even when the page is Microsoft's. Second, two Microsoft Graph pages I wanted (`checkMemberGroups`, `getMemberGroups`) returned 404, so I could not confirm what they require.

---

## 1. Summary: what I recommend

1. **Treat the product as a platform with three layers**, not as one assistant with a feature list:
   - **Device agent**: small Rust service on each laptop that does tasks, enforces policy, holds the user's keys.
   - **Org control plane**: our server for identity, groups, policy, audit, and request routing. It never holds file contents.
   - **Transport**: encrypted device-to-device and phone-to-device connections, relayed only when a direct path fails.
2. **Colleague requests and phone-to-own-laptop are two capabilities on that platform**, alongside the everyday assistant tasks. All three share identity, approval, policy and audit. Build the platform once.
3. **Move from "fully local, no server" to "local-first, cloud-assisted, org-controlled".** Your own spikes show local models miss the bar on extraction and planning **[R]**, and cloud inference is cheap enough that cost is not the reason to avoid it **[C]**. Keep a local-only mode for customers who require it.
4. **Do not build more features yet. Prove three things first** (section 12): that IT buyers will let this run, that the transport works on real networks, and that cloud models fix the accuracy gap. Each is a few days of work, not months.
5. **The biggest risks are commercial, not technical**: selling to companies as a solo founder, the security review every buyer will run, and Microsoft shipping overlapping features (sections 4 and 10).

---

## 2. The product as I now understand it

A software layer installed on every laptop in a company. Employees type (later speak) what they want; the assistant does it. Beyond one person's tasks, it connects people inside the organisation:

| Capability | Example | Needs the org platform? |
|---|---|---|
| **A. Everyday assistant** | Open apps, find a file, email it, schedule a meeting, morning brief | No (works alone). This is the free, widely installed tier |
| **B. Ask a colleague** | "Get the Q3 budget from Priya." Priya's app shows who asked, she picks the file and approves; it transfers | Yes: identity, groups, relay, audit |
| **C. Phone to my own laptop** | On holiday, boss needs a file from your home laptop; you tell the app from your phone | Yes: pairing, relay, approval |
| **D. Org controls** | Admin sets who may ask whom, which folders may ever be shared, audit export | This is the platform itself |

Your commercial logic: capability A is free and spreads the install base; B, C and D are the paid org layer. That is a sound structure (it is how many team tools spread inside companies) **[A]**. The weak point is that capability A overlaps with what Microsoft sells (section 4).

---

## 3. Verified facts the design rests on

### 3.1 Identity and groups

| Fact | Tag | Source |
|---|---|---|
| Entra ID tokens carry a `groups` claim only up to **200 groups** (JWT) / 150 (SAML); above that the claim is dropped and an overage marker (`hasgroups` or `_claim_names`) appears. Apps must then call Microsoft Graph. A missing claim must not be read as "no groups" | **[P]** | [Zero Trust: group claims](https://learn.microsoft.com/en-us/security/zero-trust/develop/configure-tokens-group-claims-app-roles), [ID token claims](https://learn.microsoft.com/en-us/entra/identity-platform/id-token-claims-reference) |
| "Group assignment requires at least a **P1** level license. A free tenant can't assign groups to an application." This affects the "only the Sales group" filter on small tenants | **[P]** | same Zero Trust page |
| `GET /me/transitiveMemberOf`: least-privileged delegated permission listed is `User.Read`; broader ones (`Group.Read.All`, `GroupMember.Read.All`, `Directory.Read.All`) are higher privileged. Work/school accounts only | **[P]** | [List transitiveMemberOf](https://learn.microsoft.com/en-us/graph/api/user-list-transitivememberof?view=graph-rest-1.0) |
| Entra can push users **and groups** to a custom (non-gallery) app via SCIM 2.0; groups need PATCH support; only users/groups assigned to the app are provisioned | **[P]** | [SCIM provisioning](https://learn.microsoft.com/en-us/entra/identity/app-provisioning/use-scim-to-provision-users-and-groups) |
| SCIM sync runs about every **40 minutes** | **[S]** | summary of the same page, not verbatim |
| Google: Directory API `members.hasMember` and Cloud Identity `checkTransitiveMembership` exist; transitive-membership features are limited to certain Workspace Enterprise editions | **[S]** | [hasMember](https://developers.google.com/workspace/admin/directory/reference/rest/v1/members/hasMember), search snippets. **Google's OAuth scope classification for these scopes: not found. Unknown.** |

**What this means for your "same group or department only" rule:**
- Large Entra tenants with ≤200 groups per user: read the `groups` claim from the signed ID token. No extra consent. Easy.
- Users with >200 groups: fall back to Graph (`/me/transitiveMemberOf`, `User.Read`).
- Small tenants on Entra Free: groups cannot be assigned to the app, so the clean group filter may be unavailable. This is exactly where your fallback (our own groups and departments) becomes the default, not the exception **[A]**.
- Enterprise-grade path: SCIM from Entra into our control plane. Needs the customer's admin to set it up once. Treat as the Business-tier feature.
- Google Workspace: plan to use our own groups first; add Google group sync only after confirming scope verification cost **[A]**.

### 3.2 Transport

| Fact | Tag | Source |
|---|---|---|
| iroh: Rust, Apache-2.0 or MIT, latest release **v1.4.1 (Oct 2026)**, 1.0 line since June 2026 | **[P]** (dates partly inferred) | [releases](https://github.com/n0-computer/iroh/releases), [repo](https://github.com/n0-computer/iroh) |
| All connections authenticated and encrypted end to end (QUIC + TLS 1.3); endpoint identity is an Ed25519 public key; relay servers "cannot read" traffic | **[P]** | [iroh docs](https://docs.iroh.computer/what-is-iroh), [relays](https://docs.iroh.computer/concepts/relays) |
| Relay can be self-hosted (open-source binary). Hosted shared relays: **$19/month** with 100 GB egress, then $0.09/GB, 10,000 concurrent connections | **[P]** | [shared relays](https://docs.iroh.computer/iroh-services/relays/shared.md) |
| Docs claim "roughly 9 out of 10" networks allow a direct connection; this is a claim, not a published measurement | **[P]** (claim) | [NAT traversal](https://docs.iroh.computer/concepts/nat-traversal) |
| An independent study of libp2p hole punching (not iroh) found about **70% ± 7%** success | **[S]** | [arXiv 2510.27500](https://arxiv.org/pdf/2510.27500v1) |
| Tailscale: direct connections "well north of 90%" in typical conditions, relayed fallback otherwise | **[S]** | [Tailscale blog](https://tailscale.com/blog/peer-relays-beta) |
| iroh Windows support statement for the library: **not found** on the pages read. Application-level allowlist of peers: **not found** | gap | needs a test |
| OpenMLS (RFC 9420 group encryption in Rust) exists, is actively maintained, a project-backed post says it has been audited; details unconfirmed | **[S]** | [OpenMLS](https://github.com/openmls/openmls) |

**Reading:** expect most connections to go direct and some, perhaps 10–30%, to stay relayed. Corporate networks with strict firewalls are the likely relayed cases **[A]**. The design must work correctly when relayed, because that is the case you cannot avoid.

### 3.3 Windows deployment

| Fact | Tag | Source |
|---|---|---|
| Tauri builds MSI (WiX v3) and NSIS. Per-machine vs per-user is documented only for NSIS (`installMode: perMachine`, requires admin, installs to Program Files). **No documented per-machine option or silent switch for the Tauri WiX MSI** | **[P]** | [Tauri Windows installer](https://v2.tauri.app/distribute/windows-installer/) |
| Intune installs Win32 apps as SYSTEM and needs a working silent command | **[S]** | search results |
| Windows 10 (Apr 2018+) and 11 ship WebView2; offline installer adds ~127 MB | **[P]** | Tauri page |
| **Azure Artifact Signing (Microsoft's managed code signing): public-trust certificates are available to organisations in the US, Canada, EU, UK, Australia, NZ, Japan, South Korea, Singapore, Switzerland, Norway, Israel. Individuals must be in the US or Canada. India is not listed** | **[P]** | [quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart) |
| Tauri: unsigned apps trigger SmartScreen warnings and Smart App Control blocks them. OV certificates remain available to individuals; EV no longer gives instant SmartScreen reputation since 2024 | **[P]** | [Tauri signing](https://v2.tauri.app/distribute/sign/windows/) |
| Endpoint DLP can restrict named apps from copying protected files to clipboard/USB/network, and can auto-quarantine files touched by restricted apps | **[S]** | [Purview endpoint DLP](https://learn.microsoft.com/en-us/purview/endpoint-dlp-learn-about) (search snippets) |

**Consequences:** (a) an enterprise installer must be tested as a real silent per-machine package before you promise IT anything; (b) as an India-based seller you cannot use Microsoft's managed signing today, so you need an OV certificate from a commercial authority, or a company registered in a supported country; (c) customers with Purview DLP may block or quarantine files our app touches; this has to be tested, not assumed.

### 3.4 Competition and market

| Fact | Tag | Source |
|---|---|---|
| Microsoft 365 Copilot is about **$30/user/month** (annual), plus a Microsoft 365 base licence; "Copilot Business" for orgs ≤300 users about **$21** (promo prices lower) | **[S]** | [Microsoft Tech Community](https://techcommunity.microsoft.com/t5/microsoft-365-copilot-blog/introducing-microsoft-365-copilot-business-empowering-small-and/ba-p/4469700) and price-comparison blogs |
| Windows 11 is adding MCP support: an on-device registry, default connectors (File Explorer, Settings), agent containment, and admin control via Intune. **Status: prerelease.** The page describes client-to-server access only, **not** agent-to-agent or cross-device | **[P]** | [MCP on Windows](https://learn.microsoft.com/en-us/windows/ai/mcp/overview) |
| Microsoft Entra Agent ID and Agent 365: identity and governance for AI agents, supporting OAuth, MCP and A2A; parts in preview | **[S]** | [Entra Agent ID](https://learn.microsoft.com/en-us/entra/agent-id/what-is-microsoft-entra-agent-id) (search snippet) |
| Dropbox and OneDrive "file requests" exist, but they are **uploads into the requester's own cloud folder**, not pulls from another person's device | **[S]** | vendor help pages |
| Microsoft **retired OneDrive "Fetch files"** (remote access to files on your PC) on 31 Jul 2020; the PC had to be on and online. No reason was published | **[S]** | [Ghacks](https://www.ghacks.net/2020/07/14/microsoft-retires-onedrives-fetch-files-feature/), [PCWorld](https://www.pcworld.com/article/3566873/microsoft-onedrives-fetch-feature-goes-away-at-the-end-of-july.html) |
| I did **not** find a product that does "request a file from a colleague's laptop with owner approval" in one search. One search is not proof that none exists | gap | verify with a proper competitor sweep |

**Honest reading:**
- Capability A (everyday assistant) is crowded and Microsoft gives much of it away inside Windows. Do not sell it; use it to spread.
- Capability B (ask a colleague) looks like open ground, but "open ground" can also mean "nobody wants it". The retirement of OneDrive Fetch is a warning sign that remote pull from a device had little use or too much risk. The difference is that your version is request-based and approved by the owner, which is much safer than silent remote fetch.
- Capability D (org controls and audit) is what a buyer pays for, and it is where Microsoft's agent governance is heading too. Expect to be compared against Entra Agent ID and Intune.

---

## 4. What the buyer actually evaluates

I expect the real decision-maker to be IT or security, not the end user **[A]**. Their first questions, in the order I expect them:

1. Where does our data go? (This is why the "no file contents on our servers, end-to-end encrypted" rule must be absolute.)
2. Who can request files from whom, and who approved what? (Group rules, audit log.)
3. Can an attacker abuse the approval prompt? (See section 7.)
4. Does it work with our Intune, Entra, DLP and proxy? (Section 3.3 tests.)
5. Do you have SOC 2 or a pen test report? Reported first-year SOC 2 cost: auditor about $20k–$45k, plus a compliance platform about $10k–$100k per year, wide ranges, vendor-sourced **[S]**. You will not have this at launch; plan to answer questionnaires honestly and sell to small companies first, who ask less.
6. What happens if you disappear? (Escrow, local-only mode, open-source core: decision pending in doc 10.)

---

## 5. Architecture

### 5.1 Overview

```
Employee laptop                         Our servers (small, Rust)              Colleague laptop
┌───────────────────┐                  ┌───────────────────────┐            ┌───────────────────┐
│ Device agent      │ ◄── control ───► │ Org control plane      │ ◄─control─►│ Device agent      │
│  • policy engine  │   (TLS, small)   │  • identity (OIDC)     │            │  • policy engine  │
│  • task skills    │                  │  • groups / SCIM       │            │  • approval UI    │
│  • approval UI    │                  │  • signed policy       │            │  • local file     │
│  • local file     │                  │  • request routing     │            │    search         │
│    search         │                  │  • metadata audit log  │            └─────────▲─────────┘
└─────────▲─────────┘                  └───────────────────────┘                      │
          │                                                                           │
          └──────────── file bytes: encrypted, direct if possible, relay if not ─────┘
Phone (own devices) ── paired to the same user's agent over the same transport
```

Principles:
- **Control and data are separate.** The control plane carries tiny messages ("Rahul asks Priya for Q3 budget"). File bytes never touch our database.
- **The server decides who may ask whom; the device decides what leaves.** Both must agree. A compromised server cannot take files, because the owner's device still has to approve and encrypt.
- **Everything is a signed, short-lived ticket.** A request carries the requester's identity, the target, the text, a timestamp and an expiry, signed by the control plane.

### 5.2 Request flow (capability B)

1. Rahul types "get the Q3 budget from Priya".
2. His agent parses the intent locally (name, what). It sends a request to the control plane.
3. Control plane checks: Is Rahul allowed to ask Priya? (same group/department rule, rate limits, Priya not blocked). If yes, it signs a ticket and queues it for Priya's device.
4. Priya's agent shows a card: **who** (org-verified name), **what** (Rahul's exact words), **when**. It searches her laptop locally and shows candidate files. **She chooses** which file, or declines.
5. If she approves, her agent opens an encrypted connection to Rahul's device (direct, else relay) and sends the chosen file. Both sides write an audit record.
6. Rahul's agent verifies the file hash and shows "Received".

Rules:
- Rahul's agent **never browses Priya's disk**. It sends text; her agent finds candidates.
- Text from Rahul is **content, not a command** (extends rule 1 in doc 00.6). It can only produce a card for Priya.
- Priya sees a **visible, unfakeable confirm** from the core engine (doc 00.6), never a plain notification button.
- If Priya is offline, the request waits (encrypted to her device key), expires after a set time, and Rahul sees "waiting for Priya".

### 5.3 Phone to my own laptop (capability C)

Evidence that shapes this, because "someone powers on my laptop" is the hard part:

| Fact | Tag | Source |
|---|---|---|
| Standard Wake-on-LAN uses broadcast packets that routers block between networks; it does not cross the internet without special router setup | **[S]** | [Wikipedia: Wake-on-LAN](https://www.wikipedia.org/wiki/Wake-on-LAN), vendor guides |
| Modern Standby laptops can stay connected while asleep; on battery, at least some HP models drop Wi-Fi in Modern Standby, hibernate and shutdown | **[S]** | [Intel](https://www.intel.com/content/www/us/en/support/articles/000059062.html), HP notes in search results |
| After a cold boot, the BitLocker pre-boot PIN (if set) and then Windows sign-in come first; user-session apps start only after sign-in. Autologon avoids this but weakens security, and enterprise hardening baselines require a BitLocker PIN | **[S]** | [DISA STIG WN11-00-000031](https://stigaview.com/products/win11/v2r8/WN11-00-000031) |

**What this means in plain terms:**
- If the laptop is **fully shut down**, nobody can use it remotely until a person types the BitLocker PIN and signs in. Our app cannot start before that. This will not change, and no honest vendor can promise otherwise.
- If it is **asleep or hibernated with the user still signed in**, powering it on typically returns to the lock screen with the signed-in session restored, so background processes of that session resume. I believe this is true **[A]**, but it must be tested on real laptops before it goes in any marketing (experiment E5).
- So the realistic promise is: **"Leave it asleep or hibernated, not shut down, plugged in, on Wi-Fi it knows. Anyone can press the power button and your phone can reach it."** Tasks that need the screen unlocked (UI automation) will not work; file and API tasks will.

Phone surface options (decide after a spike): installed web app (PWA), native app, or approvals inside Microsoft Teams **[A]**. A web app is the cheapest start. Teams-based approvals would avoid shipping a phone app at all but need a Teams app and add a Microsoft dependency.

My view on demand: the "boss needs a file while I'm on holiday" scenario is real but rare, and cloud sync (OneDrive known-folder backup) already covers most of it. Retired OneDrive Fetch is the cautionary example. **Build this after the org platform, because it costs little once pairing and relay exist, and it is a strong demo, not a reason to buy.** Re-rank it after customer interviews.

### 5.4 Compute: where models run

Your spikes **[R]**:

| Spike | Result |
|---|---|
| SP1 router (MiniLM + logistic head) | conditional pass: 92% on the original set, 80.8% on a fresh set, chains ~60% |
| SP2 extractor (GLiNER-small) | **fail**: 62/100 on a fresh blind set; 710 MB RSS on ONNX int8 |
| SP3 planner (Qwen 1.5B) | **fail**: 4/8 on T75–T82; peak RSS 1.85–2.06 GB |
| SP11/Phase 1 | idle 14 MB with the web view destroyed, ~350 MB with the bar open |
| Real open-app run | ~7 s against a 0.5 s target; not yet profiled |

Cloud price list **[P]** from [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing), per million tokens:

| Model | Input | Output |
|---|---|---|
| Haiku 5.5 (prompts ≤100k) | $0.10 | $0.50 |
| Sonnet 5.5 | $2 | $10 |

**Cost per user per month [C]**, assuming 1,500 input and 150 output tokens per request and 440 requests per month (20 a day, 22 working days; both are my assumptions):

| Model | Per request | Per user per month |
|---|---|---|
| Haiku 5.5 | $0.000225 | about **$0.10** |
| Sonnet 5.5 | $0.0045 | about **$2.00** |

So inference cost is not a barrier at any realistic price. Prompt caching and batch discounts would lower it further **[P]**.

Data handling facts **[P]** ([API and data retention](https://platform.claude.com/docs/en/manage-claude/api-and-data-retention)): prompts and outputs are not retained by default, except for "Covered Models", which require 30-day retention. Zero data retention is a per-organisation contract arranged through sales, not self-serve. I could not confirm whether Haiku 5.5 or Sonnet 5.5 are Covered Models; they were not named in the exclusions I saw. **Confirm with Anthropic before you promise zero retention to any customer.**

**Recommendation: three modes, chosen by the org admin.**

| Mode | What runs where | For |
|---|---|---|
| **Local-only** | Tier-0 router and rules on the laptop (about 100 MB, ~8 ms router **[R]**); unknown requests get "can't do that" | Regulated customers, or anyone who says no to cloud |
| **Hybrid (default)** | Tier-0 local. Request text (scrubbed of file contents and attachments) goes to a cloud model for extraction, planning, summaries | Most companies |
| **Org-hosted** (later) | Same as hybrid, but through the customer's own cloud account | Large customers |

Why this beats all-local for a company product:
- It removes the ~1.5 GB install and the 2 GB peak, which your own doc 00.10 identifies as the thing that makes IT say no.
- It fixes the two failed spikes with a measurement (experiment E7), not a hope.
- It keeps your privacy story honest: files and email bodies stay on the device unless a task like "summarise this email" requires sending that text, which the admin can switch off.

Cost of this choice: the old promise "no external AI calls" is gone for default customers, and your docs 00.6, 00.5 and the landing copy must change. It is the single biggest positioning change in this proposal.

### 5.5 Resource footprint (the "must not slow laptops" requirement)

| Item | Target | Evidence |
|---|---|---|
| Idle, no window | ≤30 MB working set | 14.4 MB measured **[R]** |
| Bar open | ≤200 MB private, closes after 60 s idle | ~160 MB private, ~350 MB working set measured **[R]** |
| Always-on CPU | ~0% when idle; no polling loops | not yet measured |
| Local models | none resident by default in hybrid mode | design choice |
| Disk | ≤150 MB without local models | estimate **[A]** from SP10 component sizes **[R]** |
| Network | one idle control connection, a few KB per hour | design choice **[A]** |

Not yet measured on the 8 GB laptop (SP8 not run **[R]**). Treat the table as a budget to test, not a result.

### 5.6 Deployment

- Build a **per-machine installer** and test silent install under an Intune-style SYSTEM context before the first pilot (experiment E3). Prefer the NSIS `perMachine` path unless a WiX per-machine route is proven, because only NSIS is documented **[P]**.
- Run the agent as a normal per-user process started at sign-in, plus an optional small Windows service for reliable reconnection **[A]**.
- Signing: buy an OV certificate now, or register a company in a supported country to use Azure Artifact Signing **[P]**. Do this early; certificate validation takes time.
- Publish: firewall domain list, proxy behaviour, SBOM (already in doc 00.6).

---

## 6. Data and privacy decisions this changes

Your current docs say admins "can't see anyone's requests" (docs 00.6, 00.10). That works for personal requests but **conflicts with cross-user requests**, where the company legitimately needs to know who asked whom for what. My proposal:

| Data | Who sees it |
|---|---|
| A person's private assistant requests ("play music", "remind me") | Only that person (unchanged) |
| Cross-user request metadata: requester, target, request text, outcome, time, file name and hash | Org admins, with a clear notice to every employee |
| File contents | Nobody but sender and receiver. Not stored on our servers |
| Request text on our servers | Visible to us in operation (needed for routing); minimise retention; state it in the DPA |

This is a deliberate trade: the audit trail is what the buyer pays for. Say it plainly in the product and the privacy policy. Indian DPDP rules phase in through **13 May 2027** per doc 9 **[R]**, and a server that sees names and request text is a data fiduciary function, so budget for a lawyer and a short privacy policy before the first pilot.

---

## 7. Threats specific to this design

| Threat | How it works | Defence |
|---|---|---|
| **Approval fatigue / social engineering** | Attacker (or a careless colleague) spams requests until the owner taps Approve. This is the same pattern as MFA fatigue attacks (e.g. the 2022 Uber breach **[S]**) | The owner must choose the file; no one-tap "approve all"; rate limits per requester; cooldown after a decline; show org-verified identity and exact words; optional number-matching style check for sensitive requests |
| **Impersonation** | Someone fakes a colleague's name | Identity comes only from the signed org token, never from request text |
| **Prompt injection through a request** | Request text tries to instruct the target's assistant | Request text is content; it can only create a card; the owner's choice drives everything |
| **Stolen laptop or token** | Attacker uses a lost device to request or approve | Device keys in Windows protected storage; remote revoke from the control plane; short ticket lifetimes |
| **Insider mass-harvest** | An employee requests many files from many people | Per-user and per-group quotas; anomaly alerts to admins; audit export |
| **Our server compromised** | Attacker routes fake requests | Tickets are signed with a key the server alone cannot misuse (offline root, device-verified); the owner's device still must approve; files are end-to-end encrypted |
| **DLP clash** | Corporate DLP blocks or quarantines files our app handles | Test early (E4); document how IT allow-lists the app |
| **Phone loss (capability C)** | Attacker with the phone controls the laptop | Phone paired to laptop by in-person code; biometric lock in the phone app; admin-set limits on what the phone may fetch; remote unpair |

All of this needs an **outside security review before any customer pilot with real company data**. Budget for it.

---

## 8. Capability portfolio: what could make this the best

**My honest view.** Companies do not line up because a product has the most features. They line up because it removes one expensive, frequent pain better than anything else, and it is safe to say yes to. Your strongest assets are the *network* (colleague-to-colleague actions only work if everyone has it) and the *governance* (policy, approval, audit). Features that don't use either are things Microsoft can ship for free.

Scoring each idea on five tests (higher is better, 1–5; my judgement **[A]**, to be corrected by customer interviews):

| # | Capability | Pain frequency | Uses network or governance | Hard for Microsoft/Google to copy | Buildable solo with Claude | Trust risk (5 = low) | Verdict |
|---|---|---|---|---|---|---|---|
| 1 | **Ask a colleague** (file, answer, status) with approval | 4 | 5 | 4 | 4 | 3 | **Core hero** |
| 2 | **Org policy, groups, audit export** | 5 (to IT) | 5 | 3 | 4 | 5 | **Core: what gets paid for** |
| 3 | **Phone to my own laptop** | 2 | 4 | 3 | 3 | 3 | **Second wave: great demo** |
| 4 | **Owner-approved search across colleagues' laptops** ("who has the latest pricing deck?"). Each laptop answers from its own local index, nothing is centralised | 4 | 5 | 5 | 2 | 2 | **Strong but later; high privacy risk, high wow** |
| 5 | **Shared team recipes** ("weekly client report") published by admins to a group | 4 | 4 | 3 | 4 | 4 | **Good second wave; also a retention lock-in** |
| 6 | **Governed tool access for employees' own AI agents/MCP**: admins decide which tools an agent may call, with audit | 4 (growing) | 5 | 2 (Microsoft is building this **[P]**) | 3 | 4 | **Watch Microsoft; do not lead with it** |
| 7 | **Safe external sharing**: one place where "send to client" always goes through policy and confirm, with a log | 5 | 4 | 3 | 4 | 4 | **Strong; extends what you already built (confirm, Undo, policy)** |
| 8 | **Employee off-boarding handover**: files and tasks flagged and routed when someone leaves | 3 | 4 | 3 | 3 | 3 | **Idea only; validate with HR/IT** |
| 9 | **Hindi/English mixed-language requests** | 3 (India-first) | 1 | 3 | 3 | 5 | **Good India differentiator; later** |
| 10 | **Voice** | 2 | 1 | 1 | 3 | 4 | **Not a moat; later** |
| 11 | **Morning brief, inbox triage, scheduling** | 5 | 1 | 1 | 4 | 3 | **Table stakes; free tier only** |
| 12 | **Reminders, timers, system controls** | 2 | 1 | 1 | 5 | 5 | **Free tier filler** |
| 13 | **WhatsApp automation** | 4 | 1 | 2 | 2 | 1 | **Drop for enterprise (legal doc 9 rates it Risky)** |

Reading the table:
- **Rows 1, 2, 7 are the business.** Ask a colleague, org policy and audit, and governed external sharing all use the same approval + policy + audit pipeline you have already started. Together they sell as "every file or message that leaves a person goes through one safe, logged, fast path".
- **Rows 4 and 5 are the next moat layer.** Federated, owner-approved search is something a central indexer cannot do, because the data is on laptops and policy forbids centralising it. It is also the most dangerous feature; build it only after approvals are trusted.
- **Rows 11 and 12 are the free tier.** Keep them few and reliable. Do not invest to beat Copilot here.
- **Row 13 should not be in an enterprise product.**

The quality bar that makes a company say yes: each capability must pass these five questions before it ships.
1. Does it need approval and audit by design, not as an afterthought?
2. Does it work correctly when the other side is offline, slow or says no?
3. Can an admin turn it off per group?
4. Is the failure message one clear line?
5. Can it be undone, or does it clearly say it can't?

---

## 9. Business model

| Item | Proposal | Evidence / status |
|---|---|---|
| Free tier | Everyday assistant, a handful of reliable tasks, personal use | Spreads installs. Needs low support cost **[A]** |
| Paid tier | Per-user per month: colleague requests, groups, policy, audit, phone | Price unknown. Copilot is ~$30 and Copilot Business ~$21 **[S]**; you must price well under or sell something it does not do. Test with buyers **[A]** |
| Unit cost | Cloud inference about $0.10–$2 per user per month **[C]**; relay: iroh's published shared-relay pricing is $19/month for 100 GB then $0.09/GB **[P]**; if each user moves ~100 MB/month, 1,000 users is ~100 GB **[C]** | Infrastructure cost is small relative to any realistic price |
| Real costs | Your time, legal, security review, signing certificate, SOC 2 later | SOC 2 first year about $25k–$100k+ **[S]** |
| First customers | Companies of 10–200 people: fast decisions, little procurement, still feel the pain | **[A]** |
| Selling | You are a solo founder. Claude can write the code; it cannot sit in a customer meeting or sign a contract. Plan to spend roughly half your time on customers | **[A]** |

---

## 10. Risks, ranked

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | Buyers don't want request-based file exchange (existing sharing is "good enough") | Medium | Fatal for the hero feature | Talk to 5–8 IT/ops people **before** building the server (E1) |
| 2 | Security review blocks adoption | High | Fatal per deal | Pen test, DPA, clear threat model, small-company-first |
| 3 | Microsoft ships overlapping features inside Windows/Copilot/Entra | High over 2 years | Big | Avoid competing on single-user tasks; stay cross-platform later; own the audit/approval workflow |
| 4 | Local AI accuracy stays too low | Medium | High | Hybrid mode; measure with cloud models (E7) |
| 5 | Transport fails on corporate networks | Medium | High | Relay fallback always works; test on real office networks (E2) |
| 6 | Code-signing and installer friction (India, MSI, SmartScreen) | Certain | Medium | Buy OV certificate or register abroad now; test silent install (E3) |
| 7 | Solo founder bandwidth | High | High | Keep v1 tiny; one pilot at a time; hire help once revenue exists |
| 8 | Legal exposure on WhatsApp/Spotify automation | Known | Medium | Remove WhatsApp from enterprise scope |
| 9 | Laptops asleep or off when requests arrive | Certain | Medium | Queue, expiry, honest status ("Priya's laptop is offline"), notify when back |

---

## 11. Changes this implies for existing docs

| Doc | Change |
|---|---|
| 00 / 00.5 / 00.6 | Replace "fully local, no external AI" with the three modes in 5.4; update data-flow table; add control plane and relay to the allowed outbound list |
| 00.6 | Amend rule 1 ("only the user starts actions"): a remote request may *propose* to another user, never act. Replace "admins can't see requests" with the split in section 6 |
| 00.10 | Move org service, groups, signed policy, audit from "v2/Business" to the first paid release |
| 00.12 | Pull relay, device pairing and key management forward; phone companion becomes capability C |
| 6 / 6.1 / 7 | Add control plane, ticket format, transport trait (iroh behind an interface), device key enrolment |
| 2 Scope | Re-cut the 49 tasks: a free-tier subset plus the org capabilities in section 8 |
| 10 | Rewrite pricing and ICP around buyers (IT/ops), not freelancers |
| 8.1 | Record D-new decisions below |

Also resolve the earlier documented conflicts (first customer, one vs two models, schedule, team size); this proposal assumes **solo founder** and **hybrid compute**.

---

## 12. Validation plan: cheap experiments before building more

Each experiment has a pass condition and a decision. Order matters.

| # | Experiment | How | Pass | If it fails |
|---|---|---|---|---|
| **E1** | **Buyer conversations** | 5–8 people who administer IT or operations at companies of 10–200. Show a 60-second mock of "ask a colleague". Ask: would you allow it? what would you require? who signs off? what do you use now? what would you pay? | At least 4 say yes if conditions X are met, and name a price | Rework the hero feature or target |
| **E2** | **Transport spike (SP12)** | Rust prototype with iroh behind a `Transport` trait; two Windows machines on different networks (home, mobile hotspot, a corporate-style firewalled network); transfer 5 MB, 500 MB; measure direct vs relayed rate, time, CPU, memory. Also try a plain WebSocket relay | ≥70% direct; relayed path completes 500 MB; idle CPU ~0 | Use the simple relay for v1 (always relayed) and accept higher egress; cost in 9 is small |
| **E3** | **Silent install** | Build per-machine NSIS and MSI; install under a SYSTEM context via a test Intune tenant or `psexec -s`; confirm Program Files path and clean uninstall | Both silent, no UI, tracked by detection rule | Change packaging before any pilot |
| **E4** | **DLP clash** | Test tenant with Purview endpoint DLP and a sensitivity label; see if the app can read/send a labelled file | Behaviour understood and documentable | Add an "IT allow-list" guide; consider labelled-file prompts |
| **E5** | **Laptop wake behaviour** | On 3 different laptops (Modern Standby and not): sleep, hibernate, shutdown; press power; check whether the agent reconnects before and after unlock; test on Wi-Fi and Ethernet | Agent reconnects from sleep/hibernate with user signed in | Narrow the promise (plugged in, awake, or sleep only) or drop capability C |
| **E6** | **Groups** | Free Entra developer/test tenant (and a free-tier tenant): sign in via OIDC, read `groups`, test >200 groups with the overage path, test unassigned groups on a free tenant | Group-filtered requests work in both cases, or fallback to our own groups proven | Make our own groups the default |
| **E7** | **Cloud accuracy** | Run T01–T100 through the router, extractor and planner using Haiku 5.5 and Sonnet 5.5 with the same prompts/skill list; compare with SP1–SP3 | ≥95% on single tasks, ≥90% on T75–T82, cost per request as in 5.4 | If still low, fix prompts and skill descriptions; local-only mode stays Tier-0 only |
| **E8** | **Resource check on the 8 GB laptop (SP8)** | Run the real agent for a day with typical apps open; log CPU, memory, battery | Meets the table in 5.5 | Cut features until it does |
| **E9** | **Approval-abuse drill** | Script 50 requests against a test user; try fatigue and impersonation tricks | Rate limits and cooldowns hold; no file moves without a choice | Strengthen the approval card |
| **E10** | **Microsoft landscape sweep** | A proper search of Windows MCP/agent features, Teams, Copilot, Google Workspace, Tailscale/Taildrop, Dropbox, remote-access tools, for anything resembling B and C | A clear list of overlaps and gaps | Reposition |

I can build E2, E3, E6, E7 and E8 in this repo myself. E1, E4's tenant setup, E5 and E9's human parts need you.

---

## 13. Revised plan for a solo founder with Claude

| Stage | What | Gate to move on |
|---|---|---|
| **0. Now (1–2 weeks)** | Finish Phase 1 gate (CI green on GitHub, T01 through the real bar). Run E3, E7, E8. Start E1 conversations in parallel. Order the signing certificate | E7 shows a path to accuracy; E1 has at least 3 conversations scheduled |
| **1. Transport + identity (3–5 weeks)** | E2 and E6. Control plane skeleton (Axum + Postgres): sign-in, device enrolment, groups (ours + Entra claim), signed tickets, audit log | Two laptops exchange a file through the whole flow with a real approval |
| **2. Hero loop + hardening (4–6 weeks)** | Request cards, local file search, offline queue, rate limits, admin policy file, admin web page (minimal). Outside review of the approval flow and key handling | E9 passes; outside review has no unresolved high findings |
| **3. Pilot (6–8 weeks)** | 2–3 small companies, real data, weekly feedback. Add free-tier task subset | At least 2 pilots use it weekly for 4 weeks, with willingness to pay |
| **4. Paid launch** | Pricing, billing, DPA, privacy policy, support, status page | Revenue covers infrastructure and certificate |
| **5. Second wave** | Phone to laptop, shared recipes, safe external sharing, federated search | Re-rank by pilot data |

Dates are intentionally absent: with one builder, each gate decides the next step, as in doc 12.

---

## 14. Decisions I need from you

1. **Compute**: accept hybrid as default with a local-only mode? (Changes the "no external AI" promise.)
2. **Privacy trade**: accept that admins see cross-user request metadata, with clear notice? (Section 6.)
3. **First customer**: companies of 10–200 people, Windows, Microsoft 365? Or Google Workspace first?
4. **Signing**: buy an OV certificate in your name, or incorporate somewhere in the Azure-supported list?
5. **Where to run the control plane**: region and hosting provider (affects customer questions and data law).
6. **Drop WhatsApp** from the enterprise scope?
7. **Ask-a-colleague scope for v1**: files only, or also short text answers ("what's the client's address?")? Text answers are more useful and riskier.
8. **Hero order**: ship capability B first and C second, as I recommend, or flip it?

---

## 15. What I could not verify (so you don't rely on it)

- iroh's real direct-connection rate on corporate networks, Windows support for the library, and per-peer allow-listing at the connection level.
- Whether Haiku 5.5 / Sonnet 5.5 count as "Covered Models" with 30-day retention.
- Google's OAuth verification class for group-read scopes.
- Whether any product already offers approved colleague-to-colleague device requests.
- Tauri WiX MSI per-machine behaviour and silent install flags.
- SCIM sync cadence (40 min) and group size limits; Entra licence needed for SCIM provisioning.
- What `checkMemberGroups` / `getMemberGroups` require (pages 404).
- Windows behaviour after sleep/hibernate with a locked session (section 5.3 is inference).
- SOC 2 timelines (only cost ranges, from vendor sources).
- Demand: no customer evidence exists yet in this repo.

## 16. Source list

- Microsoft Learn: [group claims and overage](https://learn.microsoft.com/en-us/security/zero-trust/develop/configure-tokens-group-claims-app-roles), [ID token claims](https://learn.microsoft.com/en-us/entra/identity-platform/id-token-claims-reference), [transitiveMemberOf](https://learn.microsoft.com/en-us/graph/api/user-list-transitivememberof?view=graph-rest-1.0), [SCIM](https://learn.microsoft.com/en-us/entra/identity/app-provisioning/use-scim-to-provision-users-and-groups), [MCP on Windows](https://learn.microsoft.com/en-us/windows/ai/mcp/overview), [Artifact Signing quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart), [Entra Agent ID](https://learn.microsoft.com/en-us/entra/agent-id/what-is-microsoft-entra-agent-id)
- Tauri: [Windows installer](https://v2.tauri.app/distribute/windows-installer/), [Windows signing](https://v2.tauri.app/distribute/sign/windows/)
- iroh: [docs](https://docs.iroh.computer/what-is-iroh), [relays](https://docs.iroh.computer/concepts/relays), [NAT traversal](https://docs.iroh.computer/concepts/nat-traversal), [shared relay pricing](https://docs.iroh.computer/iroh-services/relays/shared.md), [repo](https://github.com/n0-computer/iroh)
- Anthropic: [pricing](https://platform.claude.com/docs/en/about-claude/pricing), [API and data retention](https://platform.claude.com/docs/en/manage-claude/api-and-data-retention)
- Secondary: [libp2p hole punching study](https://arxiv.org/pdf/2510.27500v1), [Tailscale peer relays](https://tailscale.com/blog/peer-relays-beta), [OpenMLS](https://github.com/openmls/openmls), [Ghacks on OneDrive Fetch](https://www.ghacks.net/2020/07/14/microsoft-retires-onedrives-fetch-files-feature/), [Microsoft 365 Copilot Business](https://techcommunity.microsoft.com/t5/microsoft-365-copilot-blog/introducing-microsoft-365-copilot-business-empowering-small-and/ba-p/4469700), [DISA STIG BitLocker PIN](https://stigaview.com/products/win11/v2r8/WN11-00-000031), [Intel on Modern Standby](https://www.intel.com/content/www/us/en/support/articles/000059062.html)
- Repo evidence: `spikes/*/RESULT.md`, `docs/03-delivery/8.1 - Decision Log.md`, `docs/00-vision/00.6`, `00.10`, `00.12`, `docs/04-business-legal/9 - Legal and Platform Checks.md`
