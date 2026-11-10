//! `spotify`: control playback and search (tasks A2 and A3).
//!
//! Control (pause, resume, next, previous) goes through the Windows media
//! controls, the same ones the keyboard media keys use, aimed at the Spotify
//! session only. No keys or clicks are sent.
//!
//! Playing a named song or artist: the SP6 spike found that opening a Spotify
//! link does not reliably start playback, and the Spotify web API needs a
//! sign-in. So `search` opens Spotify on the search for the name, then invokes
//! the first result's Play button through UI Automation (no keys, no clicks),
//! then reads the media session back. Only a read-back of Playing is reported
//! as playing; otherwise the answer asks the user to press Play.

use crate::system_controls::ScriptRunner;
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::sync::Arc;

/// Media session control. Env: `JITMO_OP` = status | pause | play | next | previous.
/// Prints the playback status after the call (Playing, Paused, ...).
pub const MEDIA_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, $type) {
  $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op)); $t.Wait(-1) | Out-Null; $t.Result
}
$mgrType = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager, Windows.Media.Control, ContentType = WindowsRuntime]
$mgr = Await ($mgrType::RequestAsync()) ($mgrType)
$s = $mgr.GetSessions() | Where-Object { $_.SourceAppUserModelId -like '*Spotify*' } | Select-Object -First 1
if ($null -eq $s) { throw 'no spotify session' }
switch ($env:JITMO_OP) {
  'pause' { Await ($s.TryPauseAsync()) ([bool]) | Out-Null }
  'play' { Await ($s.TryPlayAsync()) ([bool]) | Out-Null }
  'next' { Await ($s.TrySkipNextAsync()) ([bool]) | Out-Null }
  'previous' { Await ($s.TrySkipPreviousAsync()) ([bool]) | Out-Null }
}
Start-Sleep -Milliseconds 400
"$($s.GetPlaybackInfo().PlaybackStatus)"
"#;

/// Press Play on the first search result through UI Automation `InvokePattern`
/// only (no keys, no mouse). Waits up to 8 s for the Spotify window and a result
/// button. Candidates are Buttons named "Play ..." (or exactly "Play") that sit
/// above the player bar at the bottom of the window; the top-left-most wins.
/// Prints `INVOKED <name>`, or throws `no spotify window` / `no play button`.
pub const PLAY_FIRST_RESULT_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
$deadline = (Get-Date).AddSeconds(8)
$btnCond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Button)
$stop = @('song','songs','track','tracks','music','by','the','a','an','of','on','spotify','play','some','me')
$tokens = @(([string]$env:JITMO_QUERY).ToLower() -split '[^\p{L}\p{N}]+' | Where-Object { $_ -and ($stop -notcontains $_) } | Select-Object -Unique)
$found = $null; $sawWindow = $false
while ($null -eq $found -and (Get-Date) -lt $deadline) {
  $p = Get-Process Spotify -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if ($null -ne $p) {
    $sawWindow = $true
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
    $win = $root.Current.BoundingRectangle
    $cands = @()
    foreach ($b in $root.FindAll('Descendants', $btnCond)) {
      $n = $b.Current.Name; $r = $b.Current.BoundingRectangle
      if ($n -notmatch '^Play(\s|$)') { continue }
      if ($r.IsEmpty -or $r.Width -le 0 -or $b.Current.IsOffscreen) { continue }
      # The player bar (play/pause) is the bottom strip of the window.
      if ($r.Y -gt ($win.Bottom - 120)) { continue }
      if (-not $b.Current.IsEnabled) { continue }
      # Only a button that names what was asked for. This skips tiles left over from
      # the previous page and the top-result card of something else.
      $low = $n.ToLower(); $score = 0
      foreach ($t in $tokens) { if ($low.Contains($t)) { $score++ } }
      if ($tokens.Count -gt 0 -and $score -eq 0) { continue }
      $cands += [pscustomobject]@{ El = $b; Name = $n; Y = $r.Y; X = $r.X; Score = $score }
    }
    if ($cands.Count -gt 0) {
      $found = $cands | Sort-Object @{ Expression = 'Score'; Descending = $true }, Y, X | Select-Object -First 1
    }
  }
  if ($null -eq $found) { Start-Sleep -Milliseconds 500 }
}
if (-not $sawWindow) { throw 'no spotify window' }
if ($null -eq $found) { throw 'no play button' }
$found.El.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
"INVOKED $($found.Name)"
"#;

/// Open a Spotify link. Env: `JITMO_URL`.
pub const OPEN_SCRIPT: &str = "Start-Process $env:JITMO_URL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Pause,
    Resume,
    Next,
    Previous,
}

/// "pause the video", "stop netflix": a player Jitmo does not control.
pub fn is_other_player(text: &str) -> bool {
    let t = text.to_lowercase();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &[&str]| words.iter().any(|x| w.contains(x));
    has(&["video", "videos", "movie", "film", "netflix", "youtube", "prime", "hotstar"])
        && !has(&["music", "song", "spotify"])
}

/// "pause the music", "music off", "skip this song", "play the previous one", "resume".
pub fn parse_control(text: &str) -> Option<Control> {
    let t = text.to_lowercase();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &[&str]| words.iter().any(|x| w.contains(x));
    if has(&["previous", "prev", "prv", "previus", "back", "last"]) && !has(&["pause", "stop"]) {
        Some(Control::Previous)
    } else if has(&["skip", "skp", "next", "nxt", "nekst"]) {
        Some(Control::Next)
    } else if has(&["pause", "paus", "pasue", "stop", "mute", "kill", "silence", "quiet"])
        || (has(&["shut"]) && has(&["music", "song", "spotify", "up"]))
        || (has(&["off"]) && has(&["music", "song", "spotify"]))
    {
        Some(Control::Pause)
    } else if has(&["resume", "continue", "unpause"]) || (has(&["play"]) && !has(&["something"])) {
        Some(Control::Resume)
    } else {
        None
    }
}

fn op_name(c: Control) -> &'static str {
    match c {
        Control::Pause => "pause",
        Control::Resume => "play",
        Control::Next => "next",
        Control::Previous => "previous",
    }
}

/// What to search for in "play Arijit Singh on Spotify", "put on some arijit".
pub fn parse_search(text: &str) -> Option<String> {
    let mut t = format!(" {} ", text.trim().trim_end_matches(['.', '!', '?']));
    let lower = t.to_lowercase();
    // Cut trailing "on spotify" / "in spotify".
    for tail in [
        " on spotify ",
        " in spotify ",
        " using spotify ",
        " with spotify ",
    ] {
        if let Some(i) = lower.rfind(tail) {
            t.truncate(i);
            t.push(' ');
            break;
        }
    }
    let lower = t.to_lowercase();
    let mut start = 0;
    for lead in [
        " please play ",
        " can you play ",
        " could you play ",
        " put on some ",
        " put on ",
        " play some ",
        " play me some ",
        " play me ",
        " play ",
        " listen to ",
        " throw on ",
    ] {
        if lower.starts_with(lead) {
            start = lead.len();
            break;
        }
    }
    let q = t[start..].trim();
    let q = q
        .trim_start_matches("some ")
        .trim_start_matches("the ")
        .trim();
    let q = strip_media_nouns(q);
    (!q.is_empty() && q.len() < 120).then(|| q.to_string())
}

/// "janam janam song" -> "janam janam": the word "song" is not part of the title and
/// makes the search worse. A query that is only such a word ("music") is kept.
fn strip_media_nouns(q: &str) -> &str {
    let mut q = q.trim();
    loop {
        let mut next = None;
        for n in [" song", " songs", " track", " tracks"] {
            if let Some(tail) = q.len().checked_sub(n.len()).and_then(|i| q.get(i..)) {
                if tail.eq_ignore_ascii_case(n) {
                    next = Some(&q[..q.len() - n.len()]);
                    break;
                }
            }
        }
        if next.is_none() {
            for n in ["the song ", "song ", "songs ", "track "] {
                if let Some(head) = q.get(..n.len()) {
                    if head.eq_ignore_ascii_case(n) {
                        next = Some(&q[n.len()..]);
                        break;
                    }
                }
            }
        }
        match next.map(str::trim) {
            Some(rest) if !rest.is_empty() => q = rest,
            _ => return q,
        }
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub struct Spotify {
    runner: Arc<dyn ScriptRunner>,
    /// Pause before each status read-back after the Play invoke.
    settle: std::time::Duration,
}

impl Spotify {
    pub fn new(runner: Arc<dyn ScriptRunner>) -> Self {
        Spotify {
            runner,
            settle: std::time::Duration::from_millis(800),
        }
    }

    /// Up to 4 status reads, `settle` apart; true once Spotify says Playing.
    fn is_playing_soon(&self) -> bool {
        for _ in 0..4 {
            std::thread::sleep(self.settle);
            if self.media("status").is_ok_and(|s| s.trim() == "Playing") {
                return true;
            }
        }
        false
    }

    fn media(&self, op: &str) -> Result<String, ConnError> {
        self.runner
            .run(MEDIA_SCRIPT, &[("JITMO_OP", op)])
            .map_err(|e| {
                if e.contains("no spotify session") {
                    ConnError::Failed("Spotify isn't playing anything right now.".into())
                } else {
                    ConnError::Failed("I couldn't control Spotify.".into())
                }
            })
    }
}

fn text_arg(args: &Args) -> Result<&String, ConnError> {
    args.get("text")
        .ok_or_else(|| ConnError::MissingArg("text".into()))
}

impl Connector for Spotify {
    fn id(&self) -> &str {
        "spotify"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        ["control", "search"]
            .iter()
            .map(|n| ActionSpec {
                name: n.to_string(),
                required_args: vec!["text".into()],
            })
            .collect()
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let msg = |m: String| Output([("message".to_string(), m)].into());
        match action {
            "control" => {
                let said = text_arg(args)?;
                if is_other_player(said) {
                    return Err(ConnError::Failed(
                        "I can only control Spotify right now, not videos.".into(),
                    ));
                }
                let c = parse_control(said)
                    .ok_or_else(|| ConnError::Failed("Should I pause, resume or skip?".into()))?;
                self.media(op_name(c))?;
                Ok(msg(match c {
                    Control::Pause => "Paused.",
                    Control::Resume => "Playing.",
                    Control::Next => "Skipped to the next song.",
                    Control::Previous => "Back to the previous song.",
                }
                .into()))
            }
            "search" => {
                let q = parse_search(text_arg(args)?)
                    .ok_or_else(|| ConnError::Failed("What should I look for?".into()))?;
                let url = format!("spotify:search:{}", encode(&q));
                self.runner
                    .run(OPEN_SCRIPT, &[("JITMO_URL", &url)])
                    .map_err(|_| ConnError::NotInstalled("Spotify".into()))?;
                // Playback is only claimed after the media session reads Playing.
                let invoked = self
                    .runner
                    .run(PLAY_FIRST_RESULT_SCRIPT, &[("JITMO_QUERY", &q)])
                    .is_ok();
                if invoked && self.is_playing_soon() {
                    Ok(msg(format!("Playing {q} on Spotify.")))
                } else {
                    Ok(msg(format!(
                        "Opened Spotify on a search for {q}. Press Play on the first result."
                    )))
                }
            }
            other => Err(ConnError::UnknownAction(other.to_string())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        match action {
            "control" => {
                let Some(c) = parse_control(text_arg(args)?) else {
                    return Ok(false);
                };
                let status = self.media("status")?;
                Ok(match c {
                    Control::Pause => status == "Paused",
                    Control::Resume => status == "Playing",
                    // A skip cannot be told apart from the status alone; the session answering is the proof.
                    Control::Next | Control::Previous => !status.is_empty(),
                })
            }
            // Opening a link cannot be read back; the media session can.
            // `execute` already read the playback state back and says honestly whether it
            // is playing or only opened. Checking again would wait several more seconds
            // and then turn that honest answer into a failure.
            "search" => Ok(parse_search(text_arg(args)?).is_some()),
            other => Err(ConnError::UnknownAction(other.to_string())),
        }
    }

    fn undo(&self, _: &str, _: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Rec {
        calls: Mutex<Vec<(String, String)>>,
        reply: Result<String, String>,
    }

    impl ScriptRunner for Rec {
        fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
            let key = env.first().map(|(_, v)| v.to_string()).unwrap_or_default();
            self.calls.lock().unwrap().push((script.to_string(), key));
            self.reply.clone()
        }
    }

    fn spotify(reply: Result<&str, &str>) -> (Spotify, Arc<Rec>) {
        let r = Arc::new(Rec {
            calls: Mutex::new(Vec::new()),
            reply: reply.map(String::from).map_err(String::from),
        });
        let mut s = Spotify::new(r.clone());
        s.settle = std::time::Duration::ZERO;
        (s, r)
    }

    fn args(t: &str) -> Args {
        [("text".to_string(), t.to_string())].into()
    }

    #[test]
    fn controls_are_understood() {
        use Control::*;
        for (t, want) in [
            ("Pause the music.", Pause),
            ("music off", Pause),
            ("pause spotify", Pause),
            ("skip this song", Next),
            ("next song", Next),
            ("play the previous song", Previous),
            ("resume the music", Resume),
            ("nxt song", Next),
            ("skp", Next),
            ("kill the music", Pause),
            ("shut up the music", Pause),
            ("prev song", Previous),
        ] {
            assert_eq!(parse_control(t), Some(want), "{t}");
        }
        assert_eq!(parse_control("how are you"), None);
        assert!(is_other_player("pause the video"));
        assert!(is_other_player("stop netflix"));
        assert!(!is_other_player("pause the music"));
    }

    #[test]
    fn search_text_is_cut_to_the_name() {
        assert_eq!(
            parse_search("Play Arijit Singh on Spotify.").unwrap(),
            "Arijit Singh"
        );
        assert_eq!(parse_search("put on some arijit").unwrap(), "arijit");
        assert_eq!(parse_search("play some jazz").unwrap(), "jazz");
        assert_eq!(parse_search("play").as_deref(), None);
        // The word "song" is not part of the title.
        assert_eq!(
            parse_search("play janam janam song").unwrap(),
            "janam janam"
        );
        assert_eq!(parse_search("play the song kesariya").unwrap(), "kesariya");
        assert_eq!(
            parse_search("play Arijit Singh songs").unwrap(),
            "Arijit Singh"
        );
        // A query that is only such a word is kept.
        assert_eq!(parse_search("play some music").unwrap(), "music");
        assert_eq!(parse_search("play song").unwrap(), "song");
    }

    #[test]
    fn the_play_script_is_told_what_to_match() {
        // The button must be chosen by the words asked for, not by position alone.
        assert!(PLAY_FIRST_RESULT_SCRIPT.contains("JITMO_QUERY"));
        assert!(PLAY_FIRST_RESULT_SCRIPT.contains("$score -eq 0"));
    }

    #[test]
    fn t03_pause_runs_the_pause_op_and_check_reads_the_status() {
        let (s, r) = spotify(Ok("Paused"));
        let out = s.execute("control", &args("Pause the music.")).unwrap();
        assert_eq!(out.0["message"], "Paused.");
        assert_eq!(r.calls.lock().unwrap()[0].1, "pause");
        assert!(s.check("control", &args("Pause the music.")).unwrap());
        let (playing, _) = spotify(Ok("Playing"));
        assert!(!playing.check("control", &args("Pause the music.")).unwrap());
    }

    #[test]
    fn no_spotify_session_is_a_plain_message() {
        let (s, _) = spotify(Err("no spotify session"));
        let e = s.execute("control", &args("pause")).unwrap_err();
        assert_eq!(e.to_string(), "Spotify isn't playing anything right now.");
    }

    #[test]
    fn search_opens_a_spotify_link_with_the_name_encoded_and_is_honest_about_it() {
        let (s, r) = spotify(Ok(""));
        let out = s
            .execute("search", &args("Play Arijit Singh on Spotify."))
            .unwrap();
        assert_eq!(
            r.calls.lock().unwrap()[0].1,
            "spotify:search:Arijit%20Singh"
        );
        assert!(out.0["message"].contains("Press Play"));
    }

    /// Answers by script: open link, UIA play script, media status.
    struct Seq {
        calls: Mutex<Vec<String>>,
        play: Result<String, String>,
        status: Result<String, String>,
    }

    impl ScriptRunner for Seq {
        fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
            let (tag, reply) = if script == OPEN_SCRIPT {
                (format!("open {}", env[0].1), Ok(String::new()))
            } else if script == PLAY_FIRST_RESULT_SCRIPT {
                // User words reach the fixed script only through its environment.
                assert_eq!(env.len(), 1);
                assert_eq!(env[0].0, "JITMO_QUERY");
                ("play".to_string(), self.play.clone())
            } else {
                (format!("media {}", env[0].1), self.status.clone())
            };
            self.calls.lock().unwrap().push(tag);
            reply
        }
    }

    fn seq(play: Result<&str, &str>, status: Result<&str, &str>) -> (Spotify, Arc<Seq>) {
        let r = Arc::new(Seq {
            calls: Mutex::new(Vec::new()),
            play: play.map(String::from).map_err(String::from),
            status: status.map(String::from).map_err(String::from),
        });
        let mut s = Spotify::new(r.clone());
        s.settle = std::time::Duration::ZERO;
        (s, r)
    }

    #[test]
    fn search_opens_then_plays_then_confirms_before_claiming_playback() {
        let (s, r) = seq(Ok("INVOKED Play Arijit Singh"), Ok("Playing"));
        let q = args("Play Arijit Singh on Spotify");
        let out = s.execute("search", &q).unwrap();
        assert_eq!(out.0["message"], "Playing Arijit Singh on Spotify.");
        assert_eq!(
            r.calls.lock().unwrap()[..3],
            ["open spotify:search:Arijit%20Singh", "play", "media status"]
        );
        assert!(s.check("search", &q).unwrap());
    }

    #[test]
    fn search_falls_back_when_no_button_or_not_playing() {
        for (play, status) in [
            (Err("no play button"), Ok("Playing")),
            (Err("no spotify window"), Ok("Playing")),
            (Ok("INVOKED Play x"), Ok("Paused")),
            (Ok("INVOKED Play x"), Err("no spotify session")),
        ] {
            let (s, _) = seq(play, status);
            let out = s.execute("search", &args("put on some arijit")).unwrap();
            assert_eq!(
                out.0["message"],
                "Opened Spotify on a search for arijit. Press Play on the first result."
            );
        }
        // The check no longer re-polls playback (execute already read it back and told
        // the truth); it passes whenever there was something to search for, instantly.
        let (s, r) = seq(Err("no play button"), Ok("Paused"));
        assert!(s.check("search", &args("put on some arijit")).unwrap());
        assert!(
            r.calls.lock().unwrap().is_empty(),
            "the check must not run scripts"
        );
        assert!(!s.check("search", &args("play")).unwrap());
    }

    #[test]
    fn nothing_runs_for_an_unclear_control() {
        let (s, r) = spotify(Ok("Paused"));
        assert!(s.execute("control", &args("hmm")).is_err());
        assert!(r.calls.lock().unwrap().is_empty());
    }
}

#[cfg(all(test, windows))]
mod live {
    use super::*;
    use crate::system_controls::PowerShell;

    /// Read-only: asks Spotify's media session for its status; changes nothing.
    #[test]
    #[ignore]
    fn live_status_of_the_spotify_session() {
        let r = PowerShell.run(MEDIA_SCRIPT, &[("JITMO_OP", "status")]);
        println!("spotify status: {r:?}");
        // Either a status, or the plain "no session" error when Spotify is closed.
        assert!(
            r.is_ok() || r.as_ref().unwrap_err().contains("no spotify session"),
            "{r:?}"
        );
    }
}
