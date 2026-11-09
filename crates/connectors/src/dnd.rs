//! `dnd`: Do Not Disturb on and off (task S4), with an optional duration.
//!
//! SP7 found that the registry and CloudStore tricks do not work on Windows
//! 11. What works is UI Automation on the Settings "Do not disturb" switch
//! (`ms-settings:notifications`), flipped with the TogglePattern and verified
//! by reading the switch back. No keystrokes or mouse clicks are sent.
//!
//! The user's sentence is parsed here into on/off plus an optional duration.
//! The script is fixed and gets its value only through the `JITMO_STATE`
//! environment variable.
//!
//! Durations ("for an hour", "until 6pm") are understood but NOT honoured: nothing
//! turns DND off by itself yet, so the answer says so and tells the user to turn it
//! off. Settings opens visibly for 3 to 5 seconds on each call.

use crate::system_controls::ScriptRunner;
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::sync::Arc;

/// Opens Settings > Notifications, finds the Do not disturb switch by its
/// AutomationId and sets it with the UIA TogglePattern (no input events).
/// Env: `JITMO_STATE` = On | Off | Get. Prints `was=<On|Off> now=<On|Off>`
/// or `switch-not-found`. Settings is closed afterwards.
pub const DND_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
# Opening Settings takes the keyboard focus away from the command bar. Remember who had it
# and hand it back before Settings is closed, or the bar cannot be typed into.
Add-Type -Namespace Jitmo -Name Fg -MemberDefinition @'
[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
[DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool f);
[DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
'@
$before = [Jitmo.Fg]::GetForegroundWindow()
function GiveFocusBack {
  $now = [Jitmo.Fg]::GetForegroundWindow()
  if ($before -eq [IntPtr]::Zero -or $now -eq $before) { return }
  $me = [Jitmo.Fg]::GetCurrentThreadId()
  $them = [Jitmo.Fg]::GetWindowThreadProcessId($now, [IntPtr]::Zero)
  if ($them -ne 0 -and $them -ne $me) { [void][Jitmo.Fg]::AttachThreadInput($me, $them, $true) }
  [void][Jitmo.Fg]::BringWindowToTop($before)
  [void][Jitmo.Fg]::SetForegroundWindow($before)
  if ($them -ne 0 -and $them -ne $me) { [void][Jitmo.Fg]::AttachThreadInput($me, $them, $false) }
}
$ID = 'SystemSettings_Notifications_QuietHours_MuteNotification_Enabled_ToggleSwitch'
$A = [Windows.Automation.AutomationElement]
function GetSwitch {
  Start-Process 'ms-settings:notifications'
  for ($i = 0; $i -lt 50; $i++) {
    Start-Sleep -Milliseconds 400
    $w = $A::RootElement.FindFirst('Children', (New-Object Windows.Automation.PropertyCondition($A::NameProperty, 'Settings')))
    if ($w) {
      $s = $w.FindFirst('Descendants', (New-Object Windows.Automation.PropertyCondition($A::AutomationIdProperty, $ID)))
      if ($s) { return $s }
    }
  }
  return $null
}
function State($s) { $s.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState }
$s = GetSwitch
if (-not $s) { 'switch-not-found'; exit 0 }
$was = State $s
if ($env:JITMO_STATE -eq 'On' -or $env:JITMO_STATE -eq 'Off') {
  if ($was -ne $env:JITMO_STATE) {
    $s.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 1200
  }
}
$now = State $s
GiveFocusBack
Get-Process SystemSettings -ErrorAction SilentlyContinue | Stop-Process -Force
"was=$was now=$now"
"#;

/// How long Do Not Disturb should stay on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    /// For this many minutes.
    Minutes(i64),
    /// Until this clock time (today, or tomorrow if already past).
    At(u32, u32),
}

/// What the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub on: bool,
    pub span: Option<Span>,
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == ':' || c == '.'))
        .map(|w| w.trim_matches('.').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn number(w: &str) -> Option<f64> {
    match w {
        "a" | "an" | "one" => Some(1.0),
        "two" => Some(2.0),
        "three" => Some(3.0),
        "four" => Some(4.0),
        "five" => Some(5.0),
        "six" => Some(6.0),
        "ten" => Some(10.0),
        "fifteen" => Some(15.0),
        "twenty" => Some(20.0),
        "thirty" => Some(30.0),
        _ => w.parse().ok(),
    }
}

fn unit(u: &str) -> Option<f64> {
    match u {
        "hour" | "hours" | "hr" | "hrs" | "h" => Some(60.0),
        "minute" | "minutes" | "min" | "mins" | "m" => Some(1.0),
        _ => None,
    }
}

fn clock(rest: &[String]) -> Option<Span> {
    let t = rest.join("");
    let (digits, pm, am) = if let Some(d) = t.strip_suffix("pm") {
        (d, true, false)
    } else if let Some(d) = t.strip_suffix("am") {
        (d, false, true)
    } else {
        (t.as_str(), false, false)
    };
    let (h, m) = match digits.split_once([':', '.']) {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (digits.parse::<u32>().ok()?, 0),
    };
    let h = if pm && h < 12 {
        h + 12
    } else if am && h == 12 {
        0
    } else {
        h
    };
    (h < 24 && m < 60).then_some(Span::At(h, m))
}

/// "for an hour", "for 1 hr", "for 30 minutes", "for half an hour",
/// "for an hour and a half", "until 6pm", "until 18:30".
pub fn parse_span(text: &str) -> Option<Span> {
    let w = words(text);
    if let Some(i) = w.iter().position(|x| x == "until" || x == "till") {
        return clock(&w[i + 1..]);
    }
    let joined = w.join(" ");
    if joined.contains("half an hour") || joined.contains("half hour") {
        return Some(Span::Minutes(30));
    }
    for i in 0..w.len() {
        // "30 minutes", "an hour", or glued: "45m", "2h"
        let (num, per) = match (number(&w[i]), w.get(i + 1).and_then(|n| unit(n))) {
            (Some(n), Some(p)) => (n, p),
            _ => match w[i].find(|c: char| c.is_alphabetic()) {
                Some(p) if p > 0 => match (w[i][..p].parse::<f64>(), unit(&w[i][p..])) {
                    (Ok(n), Some(u)) => (n, u),
                    _ => continue,
                },
                _ => continue,
            },
        };
        let mut mins = (num * per).round() as i64;
        if per == 60.0 && joined.contains("and a half") {
            mins += 30;
        }
        return (mins > 0 && mins <= 24 * 60).then_some(Span::Minutes(mins));
    }
    None
}

/// Turn the user's sentence into on/off plus an optional duration.
pub fn parse_request(text: &str) -> Option<Request> {
    let w = words(text);
    let any = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    let do_not = w.windows(2).any(|p| p[0] == "do" && p[1] == "not");
    let mentions_dnd = any(&["disturb", "disturbing", "dnd"]) || do_not;
    let mentions_notifs = any(&["notification", "notifications", "notifs"]);
    if !(mentions_dnd || mentions_notifs) {
        return None;
    }
    let quiet = any(&[
        "shut", "mute", "silence", "silent", "quiet", "hush", "pause", "block",
    ]) || any(&["don't", "dont"])
        || do_not;
    let off_word = any(&[
        "off", "disable", "disabled", "resume", "unmute", "end", "cancel",
    ]);
    let on_word = any(&["on", "enable", "enabled", "activate", "start"]);
    let on = match (on_word, off_word) {
        (true, true) => return None,
        (false, true) => false,
        // "turn on notifications" wants them back; "turn on do not disturb" does not.
        (true, false) => mentions_dnd || quiet,
        (false, false) => quiet || mentions_dnd,
    };
    let span = if on { parse_span(text) } else { None };
    Some(Request { on, span })
}

/// Read the script output: `(was_on, now_on)`.
pub fn parse_state(out: &str) -> Result<(bool, bool), ConnError> {
    if out.contains("switch-not-found") {
        return Err(ConnError::UiChanged("Do Not Disturb".into()));
    }
    let find = |key: &str| -> Option<bool> {
        let rest = out.split(key).nth(1)?;
        let v: String = rest.chars().take_while(|c| c.is_alphabetic()).collect();
        match v.as_str() {
            "On" => Some(true),
            "Off" => Some(false),
            _ => None,
        }
    };
    match (find("was="), find("now=")) {
        (Some(w), Some(n)) => Ok((w, n)),
        _ => Err(ConnError::Failed(
            "Couldn't read the Do Not Disturb setting.".into(),
        )),
    }
}

pub struct Dnd {
    runner: Arc<dyn ScriptRunner>,
}

impl Dnd {
    pub fn new(runner: Arc<dyn ScriptRunner>) -> Self {
        Dnd { runner }
    }

    fn run(&self, state: &str) -> Result<(bool, bool), ConnError> {
        let out = self
            .runner
            .run(DND_SCRIPT, &[("JITMO_STATE", state)])
            .map_err(|e| ConnError::Failed(format!("Couldn't change Do Not Disturb: {e}")))?;
        parse_state(&out)
    }
}

fn text(args: &Args) -> Result<&str, ConnError> {
    args.get("text")
        .map(String::as_str)
        .ok_or_else(|| ConnError::MissingArg("text".into()))
}

fn ask() -> ConnError {
    ConnError::Failed("Do you want Do Not Disturb on or off?".into())
}

impl Connector for Dnd {
    fn id(&self) -> &str {
        "dnd"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec {
            name: "set".into(),
            required_args: vec!["text".into()],
        }]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        if action != "set" {
            return Err(ConnError::UnknownAction(action.to_string()));
        }
        let req = parse_request(text(args)?).ok_or_else(ask)?;
        let (was, is) = self.run(if req.on { "On" } else { "Off" })?;
        if is != req.on {
            return Err(ConnError::Failed(
                "Do Not Disturb didn't change. Please flip it in Settings > System > Notifications."
                    .into(),
            ));
        }
        let mut out: std::collections::BTreeMap<String, String> = [
            ("was".to_string(), was.to_string()),
            ("state".to_string(), req.on.to_string()),
        ]
        .into();
        let mut message = format!("Do Not Disturb is {}.", if req.on { "on" } else { "off" });
        if req.span.is_some() {
            // Nothing turns it off by itself yet, so never promise an end time.
            message = "Do Not Disturb is on. I can't turn it off at a set time yet, so say 'turn off Do Not Disturb' when you're done.".into();
        }
        out.insert("message".into(), message);
        Ok(Output(out))
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        if action != "set" {
            return Err(ConnError::UnknownAction(action.to_string()));
        }
        parse_request(text(args)?).ok_or_else(ask)?;
        // `execute` already read the switch back in the same Settings session and fails unless
        // it matches the request. A second pass here would only open Settings again, in view,
        // for 3 to 5 seconds, to learn what is already known.
        Ok(true)
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        if action != "set" {
            return None;
        }
        let was: bool = output.0.get("was")?.parse().ok()?;
        Some(
            self.run(if was { "On" } else { "Off" })
                .and_then(|(_, now)| {
                    if now == was {
                        Ok(())
                    } else {
                        Err(ConnError::Failed("Couldn't restore Do Not Disturb.".into()))
                    }
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        calls: Mutex<Vec<String>>,
        /// Current switch state; the fake applies On/Off like the script.
        state: Mutex<bool>,
        stuck: bool,
    }

    impl Fake {
        fn new(start: bool, stuck: bool) -> Arc<Self> {
            Arc::new(Fake {
                calls: Mutex::new(vec![]),
                state: Mutex::new(start),
                stuck,
            })
        }
    }

    impl ScriptRunner for Fake {
        fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
            assert_eq!(script, DND_SCRIPT);
            let want = env.iter().find(|(k, _)| *k == "JITMO_STATE").unwrap().1;
            self.calls.lock().unwrap().push(want.to_string());
            let mut s = self.state.lock().unwrap();
            let was = *s;
            if !self.stuck {
                match want {
                    "On" => *s = true,
                    "Off" => *s = false,
                    _ => {}
                }
            }
            let f = |b: bool| if b { "On" } else { "Off" };
            Ok(format!("was={} now={}", f(was), f(*s)))
        }
    }

    fn dnd(r: &Arc<Fake>) -> Dnd {
        Dnd::new(r.clone())
    }

    fn args(t: &str) -> Args {
        [("text".to_string(), t.to_string())].into()
    }

    #[test]
    fn durations_are_understood() {
        use Span::*;
        let cases = [
            ("Don't disturb me for an hour.", Some(Minutes(60))),
            ("shut notifications up for 1 hr", Some(Minutes(60))),
            ("dnd for 30 minutes", Some(Minutes(30))),
            ("do not disturb for 2 hours", Some(Minutes(120))),
            ("don't disturb me for half an hour", Some(Minutes(30))),
            ("dnd for 45m", Some(Minutes(45))),
            ("dnd for an hour and a half", Some(Minutes(90))),
            ("dnd until 6pm", Some(At(18, 0))),
            ("dnd until 6:30 pm", Some(At(18, 30))),
            ("dnd until 18:30", Some(At(18, 30))),
            ("dnd until 12am", Some(At(0, 0))),
            ("do not disturb", None),
            ("dnd until banana", None),
        ];
        for (t, want) in cases {
            assert_eq!(parse_span(t), want, "{t}");
        }
    }

    #[test]
    fn requests_are_understood() {
        let on = |span| Some(Request { on: true, span });
        let off = Some(Request {
            on: false,
            span: None,
        });
        assert_eq!(
            parse_request("Don't disturb me for an hour."),
            on(Some(Span::Minutes(60)))
        );
        assert_eq!(
            parse_request("shut notifications up for 1 hr"),
            on(Some(Span::Minutes(60)))
        );
        assert_eq!(parse_request("turn on do not disturb"), on(None));
        assert_eq!(parse_request("mute notifications"), on(None));
        assert_eq!(parse_request("turn off do not disturb"), off);
        assert_eq!(parse_request("dnd off"), off);
        assert_eq!(parse_request("turn notifications back on"), off);
        assert_eq!(parse_request("end do not disturb"), off);
        assert_eq!(parse_request("what time is it"), None);
        assert_eq!(parse_request("turn on and off dnd"), None);
    }

    #[test]
    fn state_output_is_parsed() {
        assert_eq!(parse_state("was=Off now=On").unwrap(), (false, true));
        assert_eq!(parse_state("was=On now=On").unwrap(), (true, true));
        assert!(matches!(
            parse_state("switch-not-found"),
            Err(ConnError::UiChanged(_))
        ));
        assert!(parse_state("garbage").is_err());
    }

    #[test]
    fn turning_on_for_an_hour_does_not_promise_an_end_time() {
        let r = Fake::new(false, false);
        let out = dnd(&r)
            .execute("set", &args("Don't disturb me for an hour."))
            .unwrap();
        // No end time is promised: nothing turns it off by itself yet.
        assert_eq!(
            out.0["message"],
            "Do Not Disturb is on. I can't turn it off at a set time yet, so say 'turn off Do Not Disturb' when you're done."
        );
        assert_eq!(out.0["was"], "false");
        assert!(!out.0.contains_key("until"));
        assert_eq!(*r.calls.lock().unwrap(), vec!["On"]);
    }

    #[test]
    fn without_a_duration_there_is_no_until() {
        let r = Fake::new(false, false);
        let out = dnd(&r)
            .execute("set", &args("turn on do not disturb"))
            .unwrap();
        assert_eq!(out.0["message"], "Do Not Disturb is on.");
        assert!(!out.0.contains_key("until"));
    }

    #[test]
    fn a_switch_that_does_not_move_is_reported_not_assumed() {
        let r = Fake::new(false, true);
        assert!(dnd(&r).execute("set", &args("dnd on")).is_err());
    }

    #[test]
    fn execute_reads_the_state_back_once_check_opens_nothing_and_undo_restores_it() {
        let r = Fake::new(false, false);
        let c = dnd(&r);
        let out = c.execute("set", &args("dnd on")).unwrap();
        // One Settings session for the whole request: the check must not open another.
        assert!(c.check("set", &args("dnd on")).unwrap());
        assert_eq!(*r.calls.lock().unwrap(), vec!["On"]);
        assert!(*r.state.lock().unwrap());
        c.undo("set", &out).unwrap().unwrap();
        assert!(!*r.state.lock().unwrap());
        // An unclear request is still refused by the check.
        assert!(c.check("set", &args("hello")).is_err());
    }

    #[test]
    fn an_unclear_request_asks_and_runs_nothing() {
        let r = Fake::new(false, false);
        assert!(dnd(&r).execute("set", &args("hello")).is_err());
        assert!(dnd(&r).execute("set", &Args::new()).is_err());
        assert!(r.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn the_script_sends_no_input_and_holds_no_user_text() {
        for banned in [
            "SendKeys",
            "SendInput",
            "mouse_event",
            "keybd_event",
            "{request}",
        ] {
            assert!(!DND_SCRIPT.contains(banned), "{banned}");
        }
    }
}
