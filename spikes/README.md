# spikes

The Phase 0 experiments that decided the design. They are kept for their evidence, not as
product code: nothing in `crates/` or `app/` depends on them, and each folder has its own
`RESULT.md` or notes. Python environments and model files here are ignored by git and can be
recreated.

| Spike | Question | Outcome (details in the Decision Log) |
|---|---|---|
| `sp1-router` | Can a small encoder pick the right task? | Conditional pass; the router in `crates/model-worker` came from it. |
| `sp2-extractor` | Can a small model pull out names, files and times? | Failed on accuracy (62% on a blind set); rules plus lists are used instead. |
| `sp3-planner` | Can a 1.5B planner chain tasks? | Failed; V1 uses fixed templates. |
| `sp4-adapters` | Can one base model swap small adapters? | See the Decision Log. |
| `sp5-whatsapp` | Can WhatsApp be driven from the desktop? | Works as a fallback: open the chat, the user presses Send. |
| `sp6-spotify` | Can Spotify play a named song? | Works informally; the Play-button route is in `crates/connectors/src/spotify.rs`. |
| `sp7-dnd` | Can Do Not Disturb be toggled? | Pass, through UI Automation on the Settings switch. |
| `sp8-pipeline` | Do all parts fit the memory and time budget together? | Plan only; footprint scripts. |
| `sp9-injection` | How does the planner fare against prompt injection? | See the Decision Log. |
| `sp10-size` | How big is the installer? | See `sp10-size/RESULT.md`. |
| `sp11-tauri` | Does Tauri fit the memory budget and open fast? | Pass with changes: destroy the web view when hidden. |

Doc: [Feasibility Spikes](../docs/03-delivery/5%20-%20Feasibility%20Spikes.md) and
[Decision Log](../docs/03-delivery/8.1%20-%20Decision%20Log.md).
