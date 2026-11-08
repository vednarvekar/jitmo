# SP7 - Toggle Do Not Disturb on Win11

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
