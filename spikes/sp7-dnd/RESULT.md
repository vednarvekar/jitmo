# SP7 - Toggle Do Not Disturb on Win11

**UPDATE 8 Nov: FAIL.** Live test with DND off: with NOC_GLOBAL_SETTING_TOASTS_ENABLED=0 the test toast still popped up, and the real Do Not Disturb button did not change. That key is the "Get notifications from apps" master switch, not the DND button, and it was not honored live. Win+N then UIA found no DND element (152 elements). Win11 has no documented DND API; undocumented WNF/CloudStore methods are build-fragile and untested. Decision: V1 does not promise to toggle DND. Fallback: per-app notification mute, or open the Windows notification settings page for the user. Revisit WNF later.

**Verdict: PASS WITH CAVEAT - registry toggle works 10/10 on write/readback; actual toast suppression NOT visually verified.**

## Evidence (Win11 25H2 build 26200)
- Mechanism: `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\NOC_GLOBAL_SETTING_TOASTS_ENABLED` DWORD, 0 = banners off (DND on), 1/absent = on. No admin needed.
- `dnd_toggle.ps1`: 10 on/off cycles, each read back correctly (10/10). Original state (value absent = DND off) restored afterward; verified absent.
- `ToastNotifier.Setting` returned n/a from PowerShell, so no programmatic confirmation of the shell reacting. I could not observe a toast, so "shell honors it live" is unconfirmed.
- Not tried: Focus Assist/WNF (`WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED`, undocumented, build-fragile), CloudStore blobs.

## Decision
Use the registry toggle as the V1 DND implementation (simple, reversible, per-user). Needs a human check once: run `dnd_toggle.ps1 -Cycles 1` paused mid-state and confirm the bell icon in the notification center shows DND and a test toast is suppressed. Always restore original value on exit. Fallback: per-app mute (`...\Notifications\Settings\<AppId>\Enabled=0`).

## User must do
Verify visually that a toast is suppressed while value=0 (e.g. `New-BurntToastNotification` or Settings > System > Notifications > "Send test"); also that Win11 version differences (23H2 vs 24H2+) honor the key.
