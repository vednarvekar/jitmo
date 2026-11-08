# Local Assistant Engine — 10 Command Bar Design

9 Nov 2026 · research and design, no code changed

## Summary

The owner finds the bar plain. This doc proposes a floating dark "pill" in the style of a Windows 11 widget. It sits just above the taskbar, can be dragged, remembers where it was put, resets after each command and has a close button.

Short version of the plan:

1. One transparent, borderless window. The pill, its radius and its shadow are drawn in CSS. No Mica or Acrylic in V1.
2. Most states change the pill's content, not its size. Only suggestions, confirm and choice grow the window, and they grow upward.
3. The window is created hidden, placed from Rust before it is shown, and shown when the page says it is ready.
4. Position is stored per monitor in a small JSON file, in a way that survives DPI and monitor changes.
5. After a result the bar hides itself after a delay, pauses on hover and typing, and resets. Errors, confirms and choices never auto-hide.

## How to read this doc

Every outside fact has a source tag like [S3]; the URLs are in the Sources table at the end. Labels:

| Label | Meaning |
|---|---|
| Evidence | I read the page and it says this |
| Snippet | Only seen in a search-result summary, page not read in full |
| Opinion | My design judgement. Not proven. Change it if testing disagrees |
| UNVERIFIED | I could not confirm it. Test before relying on it |
| Project | Taken from this repo's code or docs |

## What exists today

| Area | Today | Source |
|---|---|---|
| Window | 640 px wide, starts 72 px tall, no decorations, always on top, skip taskbar, `.center()`, not resizable | Project: `app/src-tauri/src/main.rs` |
| Height | React measures its content and calls `resize_bar` (72 to 600 px) | Project: `App.tsx`, `main.rs` |
| Look | Flat rectangle `#1e1e24`, 14 px padding, 8 px radius input, no shadow, no icons, no close button | Project: `styles.css` |
| Hide | Esc hides when nothing is running, hotkey toggles. No hide on focus loss. No auto-hide | Project |
| Reset | `bar-shown` event resets state and text, but only when the same web view is re-shown. Text stays in the box after a command ends | Project: `App.tsx` |
| Web view life | Destroyed 60 s after hide (SP11). SP11 result: about 155 MB private when alive, about 5 MB destroyed, about 1 s cold open, about 25 ms warm open | Project: `main.rs`, `8.1 - Decision Log.md` |
| Capabilities | Only `core:event:default`. No drag permission | Project: `capabilities/default.json` |
| Versions | tauri 2.12.5, tao 0.37.2, wry 0.57.1 | Project: `Cargo.lock` |

## 1. Visual design

### 1.1 Shape, size, type

| Item | V1 value | Basis |
|---|---|---|
| Pill size, idle | 560 x 56 px | Opinion. Fits a 14 to 16 px line plus a 40 px round button with 8 px padding |
| Pill radius, one row | 28 px (full pill) | Opinion. Owner asked for a pill |
| Pill radius, expanded | 20 px | Opinion. A 28 px radius on a tall card looks like a blob |
| Windows 11 default | 8 px for windows, flyouts and dialogs; 4 px for in-page controls | Evidence [S1] |
| Spacing unit | 4 px; padding 8 px; gap 8 px | Opinion, in line with Fluent's consistent gutters [S1] |
| Accent button | 40 px circle, 8 px from the pill edge | Opinion, from the reference style |
| Close (x) button | 28 px glyph in a 32 px hit area, ghost style | Opinion. Smaller than the accent button so it is not mistaken for the main action |
| Font | Segoe UI Variable, fallback Segoe UI | Evidence [S2]. In HTML the font must be named in CSS for optical sizing [S2] |
| Input text | 16 px / 22 px, regular | Opinion. Windows body is 14/20 and body large is 18/24 [S2] |
| Result and hint text | 14 px / 20 px; key hints 12 px / 16 px | Evidence [S2]. Windows minimum is 12 px regular, 14 px semibold [S2] |
| Casing | Sentence case | Evidence [S2] |
| Window margin for shadow | 16 px on every side, transparent | Opinion. Needed because the shadow is drawn by CSS inside the window |

The pill is deliberately not the 8 px Windows 11 radius. This is a choice for the owner's requested look, not a platform rule.

### 1.2 Shadow

CSS shadow: `0 8px 24px rgba(0,0,0,0.35), 0 0 0 1px <border>`. Opinion. Keep the blur radius at or below the 16 px margin plus the offset, or the shadow is clipped at the window edge and shows a hard line.

### 1.3 Themes

Follow the Windows app mode with `prefers-color-scheme`. Dark is the default look from the reference. Tokens are in section 5. All contrast ratios in section 5 were computed by me with the WCAG formula, not copied from a source.

### 1.4 Backdrop: Mica, Acrylic, or none

| Option | Availability | Cost and problems | Verdict |
|---|---|---|---|
| None, solid colour | Everywhere | None | Use in V1 |
| Mica / Mica Alt | Windows 11 22000+ for Mica Alt in apps [S3]; `DWMWA_SYSTEMBACKDROP_TYPE` needs build 22621 [S4] | Samples the wallpaper once, so it is cheap [S3]. But it is meant for long-lived windows, and it turns into a solid colour when the window deactivates, when transparency is off, in Battery Saver and below build 22000 [S3]. The bar is short-lived and often unfocused | Not for V1 |
| Acrylic / Blur through Tauri `windowEffects` | Tauri: needs a transparent window [S5]. Acrylic on Windows 10 1903+ and Windows 11 build 22000, blur on Windows 11 build 22621 | Tauri's own reference says both have bad performance when resizing or dragging [S6]. The bar is meant to be dragged and resized | Not for V1 |
| Any of the above with a CSS pill | | The effect fills the whole window rectangle, not the rounded pill, so the shadow margin would be tinted. This is my inference. One community guide says rounded corners do not work with `window-vibrancy` [S7] (Snippet) | Another reason to skip |

Memory and CPU cost of a transparent window or of a backdrop: UNVERIFIED. I found no measured numbers. Measure with the SP11 method: private bytes with the bar visible, opaque vs transparent window, three runs each, plus idle CPU for 60 s. Suggested limit: no more than 10 MB above the 155 MB alive baseline and 0% idle CPU. Opinion.

Command Palette in PowerToys defaults to an Acrylic backdrop and also offers Transparent, Mica and Mica Alt, with an opacity slider [S8]. That shows the effect is achievable; it does not say what it costs.

### 1.5 Transparent window, shadow and corners: pitfalls

| Pitfall | What is known | What to do |
|---|---|---|
| OS shadow on an undecorated window | Tauri: `shadow: true` gives an undecorated window a 1 px white border and, on Windows 11, rounded corners; `shadow: false` has no effect on decorated windows [S5][S9] | Use `decorations(false)`, `shadow(false)`. Draw the shadow in CSS. Check there is no stray border (a blog reports one persisting even with shadow off [S10] (Snippet, UNVERIFIED)) |
| DWM corner preference does not help | `DWMWA_WINDOW_CORNER_PREFERENCE` needs Windows build 22000 [S4][S11]. But Microsoft says an app with per-pixel alpha layering or window regions cannot be rounded by DWM, even if it calls the API [S12] | Do not call it for the pill. CSS does the rounding. Whether a WebView2 transparent window counts as "per-pixel alpha" for DWM: UNVERIFIED. A cheap test: call it and look |
| Fallback if transparency misbehaves | Opaque, undecorated, `shadow(true)` gives the OS 8 px rounded corners and shadow with no transparency [S5][S12] | Keep as plan B. It loses the pill shape |
| White flash on create or show | WebView2's default background is white; `DefaultBackgroundColor` can be set; only fully transparent or fully opaque is supported [S13]. Tauri: `backgroundColor` alpha is ignored for the web view unless it is 0 [S5]. Tauri config notes `noRedirectionBitmap` can avoid a white flash when creating a transparent window [S5] | Create with `visible(false)`, `transparent(true)`, web view background alpha 0. Show only after the page reports ready [S14][S15]. Exact builder name for `noRedirectionBitmap` in tauri 2.12.5: UNVERIFIED |
| Flicker on resize | One developer reports a transparent WebView2 window flickered and jumped when resized often, and avoided resizing [S16] (Snippet). Tauri also made native undecorated-window resizing faster in later runtime releases [S17] (Snippet) | Resize rarely: most states keep the same size (section 2.8). Set size and position in one call with Win32 `SetWindowPos` through the HWND, because Tauri has no single set-rect call. Opinion |
| Transparent margin eats clicks | A window rectangle receives mouse input even where it is transparent. Whether WebView2 passes clicks through fully transparent pixels: UNVERIFIED | Keep the margin at 16 px. Tauri has `set_ignore_cursor_events` but it is all-or-nothing [S18] |
| DPI | Tauri sizes in logical pixels convert with the monitor's scale factor [S18]. Moving a window between monitors with different scales changed its size in an older window-state plugin, fixed in 2.2.2 [S19] (Snippet) | Store positions as ratios, recompute on scale change (section 3.3) |

### 1.6 Accessibility

| Topic | Rule | Basis |
|---|---|---|
| Text contrast | At least 4.5:1 for normal text (WCAG 1.4.3). I did not read that page; I know the figure from the standard | UNVERIFIED in this research |
| Control and focus contrast | At least 3:1 against adjacent colours; focus indicators too; 2.999 does not pass | Evidence [S20] |
| Focus ring | 2 px, 2 px offset, always visible on keyboard focus; token `ring` | Opinion, passes 3:1 |
| Status messages | Result, progress and error text must be exposed so a screen reader announces it without moving focus. Use `role="status"` for results and state, `role="alert"` only for errors that need action, and a `log` or live region for progress. Do not use `alert` or `aria-live="assertive"` for unimportant text (listed as a failure) | Evidence [S21] |
| Time limits | An auto-hide is a time limit. WCAG 2.2.1 wants the user able to turn it off, adjust it to 10 times the default, or extend it with at least 20 s warning and 10 extensions | Evidence [S22]. Whether a status toast is covered by this criterion is a judgement. Be safe: add a setting "Keep results open" |
| Reduced motion | `prefers-reduced-motion: reduce` removes the show fade and the spinner motion (use a static "Working…" text plus a stepped indicator). Windows 11 maps it to Settings, Accessibility, Visual effects, Animation effects | Evidence [S23] (the page does not state the mapping outright) |
| Keyboard only | Every control reachable by Tab, every action has a key (table in 2.10). Tab order: input, accent button, close | Project principle 7 in `09 UX Flows.md` |
| Target size | 32 px hit area for the close button. WCAG 2.5.8 minimum is 24 px | UNVERIFIED (from memory, page not read) |
| High contrast mode | Use `forced-colors` media query; draw borders with `CanvasText` and focus with `Highlight` | Opinion |
| Screen reader semantics | Input has an accessible name "Ask jitmo". Pill is `role="search"` or `role="dialog"` with label. Confirm card is `role="alertdialog"` | Opinion. Test with Narrator |

## 2. Behaviour

### 2.1 What other launchers do

| Product | Open on which monitor | Hide on focus loss | Reopen text | Reference |
|---|---|---|---|---|
| PowerToys Run | Setting: primary, monitor with mouse, or monitor with focused window | Not stated on the page I read | Setting "Clear the previous query on opening" | Evidence [S24] |
| PowerToys Command Palette | Setting: mouse monitor, primary, focused-window monitor, don't move, or last position | Not stated | Settings: "Keep search text when reopened", "Select search text when opened", "Return home after closing" (never, immediately, or 10 to 180 s) | Evidence [S8] |
| Command Palette Esc | Setting: clear text first then go back, always go back, always dismiss, or always hide | | | Evidence [S8] |
| Command Palette compact mode | Opens as a small search box and grows only when results need room; vertical position can be chosen, default 75% from the bottom | | | Evidence [S8][S25] |
| Command Palette numbers | Alt+1 to Alt+9 run the visible item; Alt+Shift+1 to 9 select it | | | Evidence [S8][S25] |
| Command Palette hotkey | Pressing the shortcut again hides it; can be bound to the Copilot key | | | Evidence [S8] |
| PowerToys Run / CmdPal elevated apps | Focus may fail when the hotkey is pressed over an elevated window; fix is a different keyboard hook mode that still does not work over admin windows for CmdPal | | | Evidence [S8][S24] |
| Raycast | Opens on the display that has keyboard focus | Not confirmed | "Pop to Root Search" timer (Mac v1 UI, third party) | Snippet [S26]. UNVERIFIED for current versions |
| Alfred | Setting to show on active display, default display or mouse display; option to save the position when dragged, which users turn off when the window ends up off-screen after sleep or a display change | Not confirmed | | Snippet [S27] |
| Flow Launcher | Options exist for hide on focus loss and last-query style (names not confirmed) | | | Snippet [S28]. UNVERIFIED |
| macOS Spotlight | I found nothing in this research | | | UNVERIFIED |

What this suggests (Opinion): the three monitor choices are standard, remembering position is a user setting not a default in at least Alfred, and "reset on reopen" is the safer default because every product exposes it as a setting.

### 2.2 Monitor choice

Default: the monitor that holds the foreground window when the hotkey is pressed. If there is none, or it is the desktop, use the monitor under the mouse. Opinion. Reason: the user presses the hotkey with a keyboard, and the app they are working in is on the monitor they are looking at. The mouse can be parked anywhere. The owner's brief says "the mouse or the foreground window"; make it a setting later with the same three choices PowerToys Run offers [S24].

The code already reads the foreground window before the bar opens (`note_selection` calls `foreground_window()` in `jitmo_adapter_windows::explorer`). Reuse it. Project.

### 2.3 Showing, hiding, closing

| Event | Action |
|---|---|
| Hotkey, bar hidden | Choose monitor, compute position (section 3.2), show, focus, emit `bar-shown` |
| Hotkey, bar visible | Hide (as today) |
| Esc, typing or done | Hide and reset |
| Esc, working, confirm, choice | Cancel the request (as today). A second Esc hides |
| Close (x) | Same as Esc. If a request is running, cancel it first. Never leave something running invisibly |
| Focus lost, state idle, typing, done, failed | Hide after 150 ms (ignore short focus flicker). Do not reset text if the user had typed something; keep it for the next open for 30 s. Opinion |
| Focus lost, state working | Stay visible. Opening an app often steals focus, and the user still wants to see "Opened Notepad" |
| Focus lost, confirm or choice | Stay visible; dim the pill to 70% and keep the question readable. A hidden question is a silent stall. Opinion |

Hide on focus loss is not verified as the default of any competitor (table 2.1). It is common in launchers, but I could not confirm it from the pages I read.

### 2.4 Result, auto-hide and reset

| Rule | Value | Basis |
|---|---|---|
| Result without Undo | Show 5 s, then hide and reset | Opinion. Pick a number, then adjust from testing |
| Long result | Add 40 ms per character over 60, capped at 12 s | Opinion |
| Result with Undo | Stay open until the Undo window ends (10 s, `UNDO_WINDOW_MS`), then 1 s more | Project (10 s in doc 09 and `state.ts`) |
| Failed | Stay 12 s, or until Esc | Opinion. Errors must be readable and carry a next step (doc 09) |
| Confirm, choice | Never auto-hide | Project principle 3 and 4 in doc 09 |
| Pause | Mouse over the pill, keyboard focus inside the pill, or any keypress pauses the timer; leaving restarts it at 3 s | Opinion |
| Typing during a result | Clears the result at once and starts a new request line | Opinion |
| Setting | "Keep results open" turns auto-hide off. Add before beta | WCAG 2.2.1 [S22] |
| Reset | When the timer fires or the user closes: clear text, clear state, clear undo, return to idle | Owner request |
| Clear text after submit | Clear the input when a request is accepted. On failure, put the original text back in the input so it can be edited | Opinion |

### 2.5 Reopen: reset or keep text

Default: open empty. Up arrow brings back earlier requests. Reason: the owner wants a reset after each command, and Up arrow already exists in doc 09. Command Palette makes both options settings [S8]. Add the setting "Keep text when reopened" later. When the bar was hidden by focus loss with unsent text, keep that text for 30 s (rule in 2.3). Opinion.

### 2.6 History and suggestions

| Feature | Design | Notes |
|---|---|---|
| History | Up arrow in an empty box (or caret at position 0) walks back through the last 50 requests; Down walks forward; Enter runs. Typing edits the recalled text | Doc 09 lists "Up arrow: bring back the last request". Not built (grep of `app/src` found no ArrowUp handler). Keep history in Rust or in the action log, not React state, because the web view is destroyed 60 s after hiding |
| Privacy | History is the action log (FR10). Clearing the log clears history | Project |
| Suggestions | As the user types, match against each skill's `examples` and show up to 3 rows. Rows with `{slot}` placeholders show as templates ("move {file} to {folder}"); Tab or Right arrow copies the example into the box with the first slot selected | Project: skills such as `files.move.yaml` have `examples:` with `{file}` and `{folder}` slots. Opinion on the interaction. Match with simple prefix and word match on the Rust side; no model call |
| Empty-state hints | A single line of three examples picked from the skills the user has not yet used | Opinion |
| First run | Box pre-filled and selected with "Open Notepad" (doc 09 onboarding step 3). After three successful requests, placeholder is "Ask jitmo…" | Project (doc 09). Remove the dev-build text from the placeholder in release builds |

### 2.7 Progress, confirm, choice, error

| State | Behaviour |
|---|---|
| Working under 400 ms | Show nothing new. Avoid a flash of a spinner |
| Working over 400 ms | Pill morphs: spinner, "Working…" or the step text, red round Cancel button (Esc). Same idea as the "call in progress" reference pill |
| Working over 5 s | Add elapsed seconds; for chains show "Step 2 of 3: …" (doc 09 progress list) |
| Confirm | Card above the pill with who, what, where; Send (Enter) and Cancel (Esc); Enter ignored for the first 400 ms (existing `CONFIRM_GUARD_MS`) |
| Choice ("Did you mean") | Up to 3 rows, each with a number badge 1 to 3, a label and a short hint, plus "Someone else". Keys 1 to 9 pick (already built). Arrow keys move, Enter picks |
| Error | One line: what happened and what to do (doc 09 table). Button when there is a fix. No error codes |
| Not sure | "I didn't get that. Did you mean…" with the choice rows (doc 09) |

Number keys while the input has focus: in the choice state the input is disabled in the current code, so the keys reach the window handler. Keep that, or typing "1" would go into the box. Project.

### 2.8 Window height per state

| State | Window content height (excluding margin) | Resize needed |
|---|---|---|
| Idle, typing | 56 | no |
| Working | 56 (pill morphs) | no |
| Done, Undo | 56 (pill morphs) | no |
| Failed | 56 plus 8 gap plus 28 error row | yes, once |
| Suggestions | 56 plus 8 plus up to 3 x 36 | yes |
| Confirm | 56 plus 8 plus about 100 | yes |
| Choice | 56 plus 8 plus about 44 per row | yes |

When the height changes, keep the bottom edge fixed: new top = old top minus the height change. The input row therefore never moves under the user's eyes. Opinion.

### 2.9 Always on top without stealing focus

| Question | Answer | Basis |
|---|---|---|
| Can the bar take focus on hotkey? | Windows lets a process set the foreground window only under conditions, including: it received the last input event, or the foreground lock timeout has expired. It can still be denied. It will not force the window while the user works in another window; it flashes the taskbar instead | Evidence [S29] |
| Elevated windows | A hotkey pressed over an admin window may not give focus | Evidence [S8][S24]. Document it as a known limit |
| After a command that opens another app | The new app takes focus. The bar must not take it back | Opinion. Never call `set_focus` after a request ends |
| Keep Undo clickable without stealing focus | Tauri has `set_focusable`; its docs only warn about macOS [S18]. Using it to make the done state non-activating: UNVERIFIED on Windows | Backlog item, test first |
| Stay on top | `always_on_top(true)` as today. Do not set it on a window that is hidden for 60 s and destroyed; it is set again in `create_bar` | Project |
| Full-screen apps | Both PowerToys utilities offer "ignore shortcut in full-screen" [S8][S24]. Add as a setting; default off | Opinion |

### 2.10 Keys

| Key | Action | Status |
|---|---|---|
| Hotkey | Open or close | Built |
| Enter | Run, or Send on a confirm | Built |
| Esc | Cancel, then hide | Built |
| Up / Down | History | Not built (doc 09) |
| Tab / Right arrow | Accept suggestion | New |
| 1 to 9 | Pick a choice | Built |
| Ctrl+Z | Undo while it shows | Listed in doc 09; not built (no `ctrlKey` use in `app/src`) |
| Alt+1 to Alt+9 | Pick a suggestion row without leaving the box | New, borrowed from Command Palette [S8] |
| Ctrl+Backspace | Delete a word | Browser default |
| Double-click on drag area | Reset to default position | New |

## 3. Window mechanics in Tauri 2 on Windows

### 3.1 Creating and showing

Recommended order. Opinion, built from the sources in section 1.5.

1. Build the window with `visible(false)`, `transparent(true)`, `decorations(false)`, `shadow(false)`, `always_on_top(true)`, `skip_taskbar(true)`, `resizable(false)`, `focus(false)`, size = pill plus margin.
2. In Rust, pick the monitor, compute and set the position (3.2), before the page loads or at least before showing.
3. The page loads and calls a `bar_ready` command once React has rendered and the font is loaded.
4. Rust shows the window and sets focus; the page focuses the input.
5. On later opens (web view alive) do steps 2 and 4 and emit `bar-shown`.

Tauri documents the `visible: false` then show pattern for avoiding flashes [S14][S30]. The window-state plugin suggests the same [S30]. A white window at show is a known upstream problem, and one related wry change is still open [S15] (Snippet).

### 3.2 Default position

Inputs from the Tauri `Window` API: `cursor_position()` (physical, can be negative), `monitor_from_point(x, y)`, `available_monitors()`, `current_monitor()`, and `Monitor::work_area()` / `position()` / `size()` / `scale_factor()` [S18][S31]. `work_area` exists in tauri 2.12.5, the version in `Cargo.lock`; the page does not say since which release [S31].

Steps, all in physical pixels:

1. Find the monitor (2.2): foreground window with Win32 `MonitorFromWindow`, else `monitor_from_point(cursor)`.
2. `w = work_area`. Window size in physical pixels = logical size x `scale_factor`.
3. `x = w.x + (w.width - win_w) / 2`.
4. `y = w.y + w.height - win_h - gap`, with `gap = 8 logical px`.
5. The work area is the part of the monitor not covered by the taskbar and other app bars, and `GetMonitorInfo` `rcWork` gives it [S32]. So this is above the taskbar for a taskbar at the bottom, and it is still inside the work area when the taskbar is on the left, right or top (the pill is then centred in the remaining area).
6. Auto-hide taskbar: one Microsoft Q&A report says the work-area rectangle from `SystemParametersInfo` did not exclude the taskbar when auto-hide was on [S32] (Snippet), and Chromium works around it with `ABM_GETAUTOHIDEBAR`, which can fail on multi-monitor setups [S32] (Snippet). Rule: if `work_area` equals the full monitor rectangle, use `gap = 56 logical px` so the pill clears a popped-up taskbar. The 56 px figure assumes the Windows 11 default taskbar height of 48 px plus 8: UNVERIFIED. Test with auto-hide on, and on a second monitor.
7. Clamp so the whole pill (not the margin) is inside `work_area`.

### 3.3 Dragging and remembering the position

Dragging:

| Item | Detail |
|---|---|
| Attribute | `data-tauri-drag-region` on the pill background and the left glyph. It applies only to the element it is on, not its children. Buttons and inputs keep working [S33] |
| Permission | Add `core:window:allow-start-dragging` to `capabilities/default.json` [S33] |
| Touch and pen | A CSS `app-region: drag` rule on `[data-tauri-drag-region]` makes dragging work with touch and pen on Windows [S33] |
| Not draggable | Input, buttons, suggestion rows, text the user may select |

What to save: after a drag ends, not on every move. `WindowEvent::Moved` fires repeatedly during a drag; debounce 400 ms. Whether `Moved` is documented on the page I read: no, that page only mentions `CloseRequested` [S18]. Check the tauri 2.12.5 `WindowEvent` enum.

Saved record, one per monitor, in `bar-position.json` in the app data folder:

```
{ "monitors": { "<key>": { "fx": 0.50, "fy": 0.97 } } }
```

- `key` = monitor name + physical size + physical position of its top-left. Tauri's `Monitor::name()` exists [S31]; whether the name is stable on Windows: UNVERIFIED, which is why size and position are in the key.
- `fx`, `fy` = pill centre as a fraction of that monitor's work area. A fraction survives a resolution or scale change better than pixels. This is the same idea as storing logical coordinates and converting with the current scale factor, as the Bevy window manager does [S34] (Snippet, other framework). Opinion that fractions are better.
- On open: if the chosen monitor has a record, convert and clamp (3.2 step 7); if not, use the default position.
- Off-screen recovery: the clamp covers it. A record whose monitor is gone is ignored, not deleted, so docking again restores it. Alfred users hit off-screen windows after sleep or display changes and turn position saving off [S27] (Snippet). The clamp avoids needing that.
- Reset: double-click the drag area, or a tray menu item "Reset bar position" (Phase 5).
- Handle `ScaleFactorChanged`: recompute size and position. UNVERIFIED which event name tauri 2.12.5 uses on Windows; the page I read does not list it [S18].

Why not `tauri-plugin-window-state`: it saves on app close and restores after the window is created, and saves size as well [S30]. This app rarely closes, destroys its window often and sets its own height. A fixed-position-per-monitor record is simpler. Opinion. It did fix restore bugs with a top-left corner outside the monitor and with scale changes on a second monitor [S19] (Snippet), so it is not bad, only a poor fit.

### 3.4 Skip taskbar, corners, shadow

| Item | Setting |
|---|---|
| Skip taskbar | `skip_taskbar(true)` (Windows and Linux) [S5]. Already set |
| Corner preference | Not used for the transparent pill (1.5). If plan B (opaque, `shadow(true)`) is used, Windows 11 rounds it at 8 px [S5][S1] |
| `DWMWA_BORDER_COLOR` | Can set the window border colour or suppress it with `DWMWA_COLOR_NONE`, build 22000+ [S4]. Only relevant for plan B |
| Shadow | CSS (transparent plan) or OS (plan B) |
| Dark title colours | `DWMWA_USE_IMMERSIVE_DARK_MODE` is for the system frame [S4]; not needed without a frame |

### 3.5 The 60-second web view destroy (SP11)

The destroy timer stays. It keeps idle memory near 5 MB instead of about 155 MB [Project: `8.1 - Decision Log.md`]. Consequences for this design:

| Consequence | Handling |
|---|---|
| React state vanishes on destroy | History, last position, settings live in Rust or on disk, not in React |
| First open after 60 s takes about 1 s | The window stays hidden until `bar_ready`; no empty pill is shown. If 1 s feels slow, test a longer timeout as a setting. Do not shorten it |
| Position must be set before the page exists | Done in Rust (3.2) |
| `bar-shown` only reaches a live page | A new page starts at idle by itself; keep both paths producing the same state |
| Transparency or shadow margin may change memory | Measure (1.4) |
| A pending Undo when the page dies | Undo is only offered for 10 s, shorter than the 60 s timer. The log keeps older undo (doc 09) |

## 4. Backlog

Phases: doc `03-delivery/8 - Development Phases.md` defines Phases 0 to 5 only. "Phase 6+" below means after the beta; it does not exist in that doc yet. Phase 2 is the current phase (offline tasks).

| # | Item | Why | Effort | Risk | Phase |
|---|---|---|---|---|---|
| 1 | Pill look: transparent window, CSS radius, shadow, tokens, icons | Owner's main complaint | M | Medium: WebView2 transparency flicker, memory | 2 |
| 2 | Close (x) button and clear-after-command reset | Owner request | S | Low | 2 |
| 3 | Create hidden, show on ready, no white flash | Needed for a polished look | S | Low | 2 |
| 4 | Default position above taskbar on the right monitor (work area) | Owner request | M | Medium: auto-hide taskbar, mixed DPI | 2 |
| 5 | Drag and per-monitor saved position with clamp | Owner request | M | Medium: monitor identity, scale changes | 2 |
| 6 | Auto-hide with pause on hover and typing; keep open while Undo shows | Doc 09 principle 9 | S | Low | 2 |
| 7 | Hide on focus loss with the exceptions in 2.3 | Expected launcher behaviour | S | Medium: focus flicker, app launch steals focus | 2 |
| 8 | Up/Down history | In doc 09, not built | S | Low (needs Rust-side storage) | 2 |
| 9 | Ctrl+Z for Undo | In doc 09, not built | S | Low | 2 |
| 10 | Morphing pill for working and done (red cancel, Undo chip) | Fewer resizes, matches reference | M | Low | 2 |
| 11 | Bottom-anchored growth for confirm, choice, suggestions via `SetWindowPos` | Stable input position | M | Medium | 2 |
| 12 | Screen reader pass: live regions, labels, Narrator test | Doc 00.9 accessibility | S | Low | 2 |
| 13 | Reduced motion and forced-colors styles | Accessibility | S | Low | 2 |
| 14 | Measure memory and CPU with transparent window (SP11 harness) | Evidence gap, 1.4 | S | Low | 2 |
| 15 | Suggestions from skill `examples` with slot templates | Teaches what it can do; fewer typos | M | Low | 3 |
| 16 | Progress list for chained requests | Doc 09 | M | Low | 3 |
| 17 | Confirm card with Edit (Tab) | Doc 09 | M | Medium | 3 |
| 18 | Connect card for first use of an app | Doc 09 | M | Low | 4 |
| 19 | Settings: monitor choice, keep text on reopen, keep results open, ignore in full-screen, reset position | Matches PowerToys settings [S8][S24] | M | Low | 5 |
| 20 | Action log view as history source | Doc 09 privacy | M | Low | 5 |
| 21 | Tray menu: reset bar position, pause | Recovery from a bad position | S | Low | 5 |
| 22 | Light theme sign-off and high contrast test | Accessibility | S | Low | 5 |
| 23 | Non-activating done state so Undo does not steal focus | Click Undo without leaving the app | M | High: UNVERIFIED on Windows | 6+ |
| 24 | Optional backdrop (Mica or Acrylic) behind a setting | Looks closer to system widgets | M | High: drag lag [S6], rectangle fill | 6+ |
| 25 | Compact and expanded modes like Command Palette | Less screen use | M | Low | 6+ |
| 26 | Quick access shelf of pinned or recent requests | Faster repeat tasks [S8] | M | Low | 6+ |
| 27 | Copilot key binding | Fits Windows 11; Command Palette supports it [S8] | S | High: UNVERIFIED for third-party apps | 6+ |
| 28 | Voice button (mic icon) | In the owner's reference row of icons | L | High | 6+ |
| 30 | Per-app context chip ("Using: Explorer selection, 3 files") | Shows what "this" means | M | Medium | 3 |

## 5. V1 visual spec

### 5.1 Tokens

Contrast numbers computed by me using the WCAG relative-luminance formula (Opinion on the choices; the ratios are arithmetic).

| Token | Dark | Light | Used for |
|---|---|---|---|
| `--pill-bg` | `#202026` | `#FAFAFC` | Pill and card background |
| `--chip-bg` | `#2C2C34` | `#EDEDF2` | Suggestion rows, choice rows, buttons |
| `--chip-hover` | `#37373F` | `#E0E0E8` | Hover |
| `--text` | `#F3F3F7` | `#1B1B21` | Main text. 14.6:1 dark, 16.4:1 light on `--pill-bg` |
| `--muted` | `#A6A6B4` | `#5C5C6A` | Hints, key labels. 6.7:1 dark, 6.3:1 light |
| `--accent` | `#7050F5` | `#6842F5` | Round action button. White icon on it: 5.1:1 dark, 5.7:1 light. Button against pill: 3.2:1 dark, 5.5:1 light |
| `--accent-hover` | `#8266FF` | `#5530E0` | Hover |
| `--on-accent` | `#FFFFFF` | `#FFFFFF` | Icon on the accent button |
| `--danger` | `#D13438` | `#C42B31` | Round Cancel/end button (white icon: 4.9:1 dark, 5.6:1 light) |
| `--error-text` | `#FF8A8A` | `#B3261E` | Failed message. 7.1:1 dark, 6.3:1 light |
| `--ok` | `#5BD28C` | `#1B7F4B` | Check icon. 8.5:1 dark, 4.8:1 light |
| `--ring` | `#B7A6FF` | `#4B2FD0` | Focus ring. 7.7:1 dark, 7.7:1 light |
| `--border` | `#4A4A55` | `#8A8A96` | 1 px pill outline. 1.9:1 dark, 3.3:1 light. Decorative in dark, so under 3:1 is accepted; the shadow and fill define the edge |
| `--shadow` | `0 8px 24px rgba(0,0,0,.45)` | `0 8px 24px rgba(0,0,0,.18)` | Pill shadow |

Dark text on `--chip-bg`: 12.5:1; muted on chip: 5.8:1. Light: 14.7:1 and 5.6:1.

### 5.2 Measures

| Item | Value |
|---|---|
| Window width | 592 px (560 pill + 2 x 16 margin) |
| Window height | content height + 32 px margin |
| Pill | 560 x 56, radius 28 idle; radius 20 when a card is attached |
| Padding | 8 px |
| Glyph slot | 40 px wide, 20 px icon |
| Round buttons | accent and cancel 40 px; close 28 px (32 px hit area) |
| Input | 16/22, no border, transparent background |
| Rows (suggestion, choice) | 36 px high, 12 px side padding, radius 8 |
| Confirm card | 12 px padding, 8 px gap, buttons 32 px high, radius 6 |
| Show motion | 120 ms, opacity 0 to 1 and 8 px upward slide; none when reduced motion is on |
| Hide motion | none (the window is hidden at once) |

### 5.3 States and sketches

The bottom edge of the pill is fixed; cards grow upward. `(o)` is a round button, `[ ]` a rectangular button.

**Idle**
```
+------------------------------------------------------+
| (j)  Ask jitmo...                          (>)    x  |
+------------------------------------------------------+
```
Glyph, input, accent button (disabled look until text exists), close.

**Typing, with suggestions**
```
+------------------------------------------------------+
|  move {file} to the {folder} folder           Alt+1  |
|  move {file} to {folder}                      Alt+2  |
+------------------------------------------------------+
| (j)  move|                                 (>)    x  |
+------------------------------------------------------+
```
Tab accepts the first row. Up and Down move through rows or history.

**Working** (under 400 ms nothing changes; after that)
```
+------------------------------------------------------+
| (~)  Opening Spotify...   4 s                  (X)   |
+------------------------------------------------------+
```
Spinner, text, red round Cancel (Esc). Close is hidden; Cancel is the only exit.

**Confirm**
```
+------------------------------------------------------+
|  Send Q3_report.pdf (2.4 MB) to Ved on WhatsApp?     |
|  Message: "Here is the report"                       |
|              [ Edit  Tab ] [ Cancel  Esc ] [ Send  Enter ] |
+------------------------------------------------------+
| (j)  Send this file to Ved                (>)    x   |
+------------------------------------------------------+
```
Send is the accent colour. The text typed stays visible, input disabled. Enter ignored for 400 ms.

**Choice**
```
+------------------------------------------------------+
|  Which Rahul?                                        |
|  (1) Rahul Mehta     usually on WhatsApp             |
|  (2) Rahul Shah      work email                      |
|  (3) Someone else                                    |
+------------------------------------------------------+
| (j)  Send this to Rahul                   (>)    x   |
+------------------------------------------------------+
```
Keys 1 to 3 pick. Esc cancels.

**Done** (no Undo)
```
+------------------------------------------------------+
| (v)  Playing Arijit Singh on Spotify               x |
+------------------------------------------------------+
```
Check icon in `--ok`. Hides after 5 s, resets to empty. Announced through `role="status"`.

**Done with Undo**
```
+------------------------------------------------------+
| (v)  Moved 4 PDFs to invoices        [ Undo 8 ]    x |
+------------------------------------------------------+
```
Stays until the countdown ends plus 1 s. Ctrl+Z or the button undoes. After Undo: "Undone. Files are back where they were." for 5 s.

**Failed**
```
+------------------------------------------------------+
|  (!) Found the file, but WhatsApp isn't open.        |
|      [ Open WhatsApp and retry ]                     |
+------------------------------------------------------+
| (j)  Send budget to Rahul                 (>)    x   |
+------------------------------------------------------+
```
Original text is back in the input. Message in `--error-text`, 12 s or Esc, `role="alert"` only here.

### 5.4 Checklist before this counts as done

- Pill looks right on 100%, 125%, 150% and 200% scaling, and on two monitors with different scaling.
- No white flash on first open, on reopen, and on cold open after the 60 s destroy.
- Opens above the taskbar with the taskbar at bottom, left, right, top and auto-hide.
- Drag, close, reopen: same place. Unplug the monitor: bar returns inside a visible work area.
- Memory with the bar visible within 10 MB of the SP11 baseline (155 MB private); 0% CPU idle.
- Narrator reads results and errors without moving focus.
- Reduced motion and high contrast checked.

## Contradictions with current code and docs

| # | Where | Conflict |
|---|---|---|
| 1 | `09 UX Flows.md` Screens table | Says the bar is "centered near the top of the screen". Owner wants centred above the taskbar. Code uses `.center()` (screen centre). Update doc 09 |
| 2 | `09 UX Flows.md` principle 9 | "The bar closes itself when the job is done." Code never auto-hides, and leaves old text in the box |
| 3 | `09 UX Flows.md` keyboard table | Lists Up arrow and Ctrl+Z. Neither is in `app/src` |
| 4 | `09 UX Flows.md` Undo toast | Described as a separate small bar. Code shows an inline "Undo (n)" button in the result line |
| 5 | `09 UX Flows.md` confirm | Tab = Edit. Not built |
| 6 | `main.rs` `create_bar` | Window is created visible, then focused. That risks the white flash. No `visible(false)` |
| 7 | `resize_bar` | Clamps height 72 to 600 and sets only the size; a bottom-anchored layout needs position and size set together |
| 8 | `capabilities/default.json` | No drag permission, so `data-tauri-drag-region` would do nothing |
| 9 | `App.tsx` | Placeholder text says "(dev build…)" and would ship |
| 10 | `03-delivery/8 - Development Phases.md` | Has only Phases 0 to 5. The brief mentions Phase 6 onward; nothing defines it |
| 11 | `02-design/7 - Low-Level Design.md` Store paragraph | Still says encryption is not the default and the key is not read from Credential Manager. Recent commits made `encrypted-db` a feature that release builds turn on and read the key from Credential Manager (git log). Not about the bar, but stale |
| 12 | `Cargo.toml` | `tauri = "2"` only; `Monitor::work_area` was confirmed on 2.12.5 only (the version in `Cargo.lock`). Pin a minimum |

## What I could not confirm

- Cost in MB or CPU of a transparent WebView2 window, Mica or Acrylic.
- Whether DWM treats a transparent WebView2 window as per-pixel alpha, and whether transparent pixels pass clicks through.
- The exact tauri 2.12.5 names for the `noRedirectionBitmap` builder option and the scale-change window event.
- Raycast, Alfred and Flow Launcher hide-on-blur defaults. macOS Spotlight behaviour.
- Windows 11 taskbar height and the work-area value with auto-hide on a given machine.
- `set_focusable(false)` behaviour on Windows for a non-activating result.
- WCAG 1.4.3 (4.5:1) and 2.5.8 (24 px) were not read in this pass.

## Sources

| Tag | URL |
|---|---|
| S1 | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry |
| S2 | https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography |
| S3 | https://learn.microsoft.com/en-us/windows/apps/design/style/mica |
| S4 | https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute |
| S5 | https://v2.tauri.app/reference/config/ (WindowConfig) |
| S6 | https://v2.tauri.app/reference/javascript/api/namespacewindow/ (Effects notes, as quoted in search results) |
| S7 | https://www.tabnews.com.br/Derlys/bely-launcher-como-implementei-glassmorphism-com-acrylic-blur-no-tauri-2-windows |
| S8 | https://learn.microsoft.com/en-us/windows/powertoys/command-palette/settings |
| S9 | https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindow.html (`set_shadow`) |
| S10 | https://luckydraw.tcl.com/blog/tauri-window-shadows-broken-on |
| S11 | https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_window_corner_preference |
| S12 | https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-rounded-corners |
| S13 | https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controlleroptions3 |
| S14 | https://github.com/tauri-apps/tauri/issues/1564 |
| S15 | https://github.com/tauri-apps/tauri/commit/f22ea2998619cc09c2d426f7b42211a80eed578e and https://github.com/tauri-apps/wry/pull/1742 |
| S16 | https://www.producthunt.com/p/tauri-2/i-built-sticky-notes-that-hide-in-your-windows-screen-edge-with-tauri-2-6-lessons-learned |
| S17 | https://v2.tauri.app/release/tauri-runtime-wry/v2.0.0-beta.19/ |
| S18 | https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindow.html |
| S19 | https://v2.tauri.app/release/window-state/v2.2.2/ and https://v2.tauri.app/release/window-state/v2.0.0-beta.0/ |
| S20 | https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html |
| S21 | https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html |
| S22 | https://www.w3.org/WAI/WCAG22/Understanding/timing-adjustable.html |
| S23 | https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-reduced-motion |
| S24 | https://learn.microsoft.com/en-us/windows/powertoys/run |
| S25 | https://learn.microsoft.com/en-us/windows/powertoys/command-palette/overview |
| S26 | https://manual.raycast.com/windows/settings (search summary only; the page itself did not show window settings) |
| S27 | https://www.alfredforum.com/ (threads on saving window position and display choice, search summary only) |
| S28 | https://github.com/Flow-Launcher/Flow.Launcher (search summary only) |
| S29 | https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow |
| S30 | https://v2.tauri.app/plugin/window-state/ |
| S31 | https://docs.rs/tauri/latest/tauri/window/struct.Monitor.html |
| S32 | https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-monitorinfo, https://learn.microsoft.com/en-us/answers/questions/760561/how-do-i-find-the-size-of-the-available-display-ex, https://chromium.googlesource.com/git/chromium/+/trunk/ui/views/widget/monitor_win.cc |
| S33 | https://v2.tauri.app/learn/window-customization/ |
| S34 | https://docs.rs/bevy_window_manager |
