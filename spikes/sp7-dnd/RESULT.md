# SP7 result: can the engine toggle Do Not Disturb on Windows 11?

**Verdict: PASS (method: UI Automation on the Settings switch).** Win11 25H2 build 26200, 8 Nov 2026.

## What works
`dnd_settings_uia.ps1` opens `ms-settings:notifications`, finds the **Do not disturb** switch by its AutomationId (`SystemSettings_Notifications_QuietHours_MuteNotification_Enabled_ToggleSwitch`) and flips it with the UIA TogglePattern. No admin rights, no registry writes.
- Live test (`-Mode test`): switch Off -> On -> Off. After each step the switch read back correctly and the live shell state (WNF `WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED`, read-only) moved 0 -> 1 -> 0. The user saw the real DND button turn on.
- Every change is verified by reading the switch back and the WNF state, so a failure is reported, not assumed away.
- Modes: `status`, `on`, `off`, `test`. Takes about 3-5 s per toggle (Settings has to open); close Settings afterwards.

## Approaches tried first that do NOT work
- `NOC_GLOBAL_SETTING_TOASTS_ENABLED` registry value: this is the "Get notifications from apps and other senders" switch, not DND. Test toasts still appeared and the DND button did not move.
- Rewriting the CloudStore `quiethourssettings` blob plus restarting `WpnUserService_*` (the method community tools use): the blob and service restart worked, but the live state stayed 0 and toasts still appeared on this build. Undocumented and build-fragile anyway.
- Win+N and the taskbar clock via UIA: the Notification Center did not expose the bell button.
- `Windows.UI.Shell.FocusSessionManager`: a different feature (Focus sessions) and locked behind a Microsoft unlock token.

## Notes
- With DND on, Windows sends notifications silently to the notification center (priority list still allowed through). The test toasts came from PowerShell and the user saw both, so what DND hides on screen was not isolated here; that is Windows' own behaviour once the switch is on.
- The Settings page layout can change between Windows builds; the AutomationId is stable on 26200, so keep the status check as a self-test and fall back to opening the page for the user.
- The WNF read is undocumented; use it only as a check, never as a requirement.

## Decision
Use the UIA Settings switch for T-tasks that turn DND on/off (S-tasks), verified by read-back. Fallback: open `ms-settings:notifications` and tell the user to flip it.

## UX note (from user testing)
The Settings window visibly opens and closes (~3-5 s), which looks like a glitch. For Phase 2: minimize or move the window off-screen right after launch (untested), and show "Turning on Do Not Disturb..." then a result line in the command bar. Keep the read-back check.
