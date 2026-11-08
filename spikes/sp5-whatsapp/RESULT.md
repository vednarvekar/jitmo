# SP5 - WhatsApp desktop drive

**Verdict: BLOCKED - needs a logged-in, visible WhatsApp window (partial evidence only). Target (20/20 sends) NOT measured; no Send was ever pressed.**

## Evidence (2026-11-07, Win11 25H2)
- Installed: WhatsApp Desktop (Store, `5319275A.WhatsAppDesktop`). `whatsapp:` protocol handler registered.
- `Start-Process "whatsapp://send?phone=15555550100&text=..."` (fictitious number) launches/activates `WhatsApp.Root` (WebView2-based app). Deep link with phone+prefill text is the documented way to open at the right chat with text prefilled.
- UIA (System.Windows.Automation) reached only the native shell: title bar, Minimize/Maximize/Close and one `Pane id=WebView` (8 descendants). Chat list, compose box and Send button were NOT exposed in 3 passes; later passes saw 4 descendants (window likely hidden/minimized to tray). Login state could not be determined (no window content visible).
- Interop caveat: powershell.exe from WSL intermittently fails with `UtilAcceptVsock accept4 failed 110`; scripts need a retry wrapper.

## Decision
Plan the **fallback as the primary path**: deep link `whatsapp://send?phone=&text=` (opens right chat, prefilled), user presses Send. Do not count on UIA reading the compose box; WebView2 only exposes web content when accessibility is on (try setting env `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--force-renderer-accessibility` - not possible for a Store app we do not launch, so likely unreliable). Legal doc rates automation Risky; human Send is also the legal mitigation.

## User must do to finish
1. Open WhatsApp Desktop, confirm it is logged in, keep window visible (not minimized).
2. Name a test contact/number you own; re-run `probe.ps1 -Phone <number>` and check the UIA dump lists an Edit (compose) control. If it does, try typing via ValuePattern/SendKeys and stop before Send.
3. Only if you explicitly approve a test account can the 20-send run happen.

Scripts: `probe.ps1` (open link + UIA dump), `dump_again.ps1`, `shot.ps1` (screenshot; captures whole desktop - careful).
