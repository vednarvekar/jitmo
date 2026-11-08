# SP11 result: does the Tauri command bar fit the memory budget and open fast from a hotkey?

**Verdict: PASS WITH CHANGES.** Hiding the window does not save memory; destroying the web view does. Run 8 Nov 2026, Windows 11, Tauri 2.12 (tauri-cli 2.x), WebView2, release build.

## Setup
- Hello-world app (`src-tauri/`, static `dist/index.html`, one text input), borderless always-on-top 640x72 bar, global shortcut via `tauri-plugin-global-shortcut`. Release exe is 3.2 MB.
- Hotkey injected with `keybd_event` by `measure.ps1`; memory = app process + its child `msedgewebview2.exe` processes (matched on the app identifier).
- Variant A (`main_hidden_variant.rs.txt`): window created hidden, hotkey does show/hide.
- Variant B (`src-tauri/src/main.rs`): no window at start; hotkey builds the window, hotkey again destroys it; app stays alive in the background.

## Numbers
| State | Processes | Working set | Private memory |
|---|---|---|---|
| A: bar hidden (idle) | 7 | ~350 MB | ~155 MB |
| A: bar shown | 7 | ~347 MB | ~153 MB |
| **B: no window (idle)** | **1** | **~11-24 MB** | **~2-5 MB** |
| B: bar shown | 7 | ~340 MB | ~158 MB |

- Hotkey to visible: **A ~20-30 ms** (window already exists). **B ~0.9-1.3 s** cold (create window + start WebView2).
- Ctrl+Alt+Space was already registered by another app on this machine (registration failed). Ctrl+Alt+J worked. The default hotkey must be configurable and registration failure must be handled.

## What it means
- 06's ~40 MB share for the command bar is far exceeded while the web view lives (~155 MB private, ~340 MB working set, shared pages counted once by the OS but not in this sum).
- Idle budget (300 MB) holds only if the web view is destroyed when hidden (variant B), at the price of ~1 s to open. Or keep it alive and rebalance the idle budget (e.g. 150 MB for UI + ~100 MB core + encoder).
- Teardown is lazy: WebView2 child processes linger a few seconds after `destroy()`, so "after N cycles" counts were noisy.

## Decision
Recommend: keep the web view alive for a short time after use (e.g. 60 s), then destroy it, so repeat requests open in ~25 ms and idle drops to ~5 MB. Alternatively a tiny native bar for the typing step is an option if 1 s cold open is not acceptable (cf. SP8 native UI question). PR1 must state whether idle is measured with or without the web view. Revisit once the real UI (React) is in; React adds JS heap on top.

## Caveats
Machine was busy with other jobs (builds, Python benchmarks), so memory is fine but cold-start time is likely pessimistic. Not tested on the 8 GB laptop. Plain HTML UI, not React. Focus-stealing behaviour of `set_focus` from another foreground app was not checked.
