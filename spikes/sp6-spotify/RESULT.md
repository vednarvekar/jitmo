# SP6 - Play a specific song via Spotify

**UPDATE 8 Nov: the user watched the app run and reports it works on his machine with the app left alone; earlier failures were partly his own closing the window. Treat as WORKING (informal, not a measured 9/10). Plan type does not matter: only the desktop UI is driven. Add the Play-click fallback and re-measure when convenient.**

**Verdict: FAIL on target as scripted (4/7 = 57% each of two runs, target 9/10); likely PASS WITH CHANGES once a UIA Play-click fallback is added (not yet proven). Caveat: the logged-in account is Premium, so the "without Premium" question is NOT answered.**

## Evidence
- Installed: Spotify (Store, `SpotifyAB.SpotifyMusic`), logged in (window title "Spotify Premium", user library visible). Not a free account -> free-tier behavior unverified.
- `Start-Process spotify:track:<id>` launches/navigates the app and, when it works, starts playback in 1.6-3.5s; title becomes "Artist - Song" (good, cheap success signal).
- Run 1 (URI only): 4/7 hits (Never Gonna Give You Up, Blinding Lights, Billie Jean, Africa); Shape of You, Yellow, Bohemian Rhapsody stayed at "Spotify Premium" (paused) for 20s.
- Run 2 (URI + media Play/Pause key after 6s): 4/7; kick helped Yellow once but misses moved elsewhere. Failures are inconsistent (not alternating, not track-specific), so autoplay on URI navigation is a race. Do not rely on it.
- Media key via keybd_event VK_MEDIA_PLAY_PAUSE (0xB3) works (paused playback).
- UIA: Spotify (CEF) exposes the web UI (466 descendants, e.g. ComboBox "What do you want to play?", Buttons, DataGrid) after first query. A Play-button Invoke was attempted (`uia_play.ps1`) but that run self-resolved at 6s so the click path was not exercised.
- Spotify exited by itself ~27s after one launch in the first probe (observed once); treat the process as restartable.

## Decision
Keep plan "spotify: URI" but change target to: launch URI -> poll title 6s -> if not playing, UIA-invoke the Play button on the track page -> else media key -> else tell user. Re-measure 10 tries. `spotify:search:<q>` fallback (user taps Play) remains for free tier. Do not use Web API (Premium + voice-control ban per doc 9).

## User must do
- To test the no-Premium claim: log in with a Free account (or log out Premium) and re-run `trials.ps1 -Kick`.
- Confirm OK for audio to play during tests.

Scripts: `probe.ps1`, `trials.ps1 [-Kick]`, `uia_dump.ps1`, `run_dump*.ps1`, `uia_play.ps1`, `media_pause.ps1`.
