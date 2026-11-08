//! `system_controls`: volume and brightness (S1), lock (S2), Wi-Fi and
//! Bluetooth (S3) and screenshots (S5). Do Not Disturb (S4) is not built yet.
//!
//! The connector turns the user's words into a small, fixed operation and runs
//! a fixed PowerShell script with the values passed as environment variables.
//! Nothing the user says is ever pasted into a script.
//!
//! Scripts run through a [`ScriptRunner`], so the parsing and the choice of
//! script are unit-tested without touching the PC. The scripts themselves
//! (Core Audio, WMI, WinRT radios) were written from documented patterns and
//! have not all been run on a real machine; the status is in the Decision Log.

use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::sync::Arc;

/// Runs a PowerShell script with environment values. Returns its trimmed output.
pub trait ScriptRunner: Send + Sync {
    fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String>;
}

/// The real runner (Windows only).
pub struct PowerShell;

impl ScriptRunner for PowerShell {
    #[cfg(windows)]
    fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    #[cfg(not(windows))]
    fn run(&self, _: &str, _: &[(&str, &str)]) -> Result<String, String> {
        Err("system controls need Windows".into())
    }
}

// ---------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------

/// Master volume through Core Audio. Env: `JITMO_OP` = get | up | down | set |
/// mute | unmute, `JITMO_N` = points (0-100). Prints `<percent> <muted>`.
pub const VOLUME_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IAudioEndpointVolume {
  int f(); int g(); int h(); int i();
  int SetMasterVolumeLevelScalar(float level, Guid ctx);
  int j();
  int GetMasterVolumeLevelScalar(out float level);
  int k(); int l(); int m(); int n();
  int SetMute([MarshalAs(UnmanagedType.Bool)] bool mute, Guid ctx);
  int GetMute(out bool mute);
}
[Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDevice { int Activate(ref Guid id, int clsCtx, int activationParams, out IAudioEndpointVolume aev); }
[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDeviceEnumerator { int f(); int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice endpoint); }
[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumeratorComObject { }
public class JitmoAudio {
  static IAudioEndpointVolume Vol() {
    var e = new MMDeviceEnumeratorComObject() as IMMDeviceEnumerator;
    IMMDevice d; e.GetDefaultAudioEndpoint(0, 1, out d);
    var iid = typeof(IAudioEndpointVolume).GUID; IAudioEndpointVolume v; d.Activate(ref iid, 23, 0, out v); return v;
  }
  public static string Do(string op, int n) {
    var v = Vol(); float cur; v.GetMasterVolumeLevelScalar(out cur);
    int pct = (int)Math.Round(cur * 100);
    if (op == "up") pct = Math.Min(100, pct + n);
    else if (op == "down") pct = Math.Max(0, pct - n);
    else if (op == "set") pct = Math.Max(0, Math.Min(100, n));
    if (op == "up" || op == "down" || op == "set") v.SetMasterVolumeLevelScalar(pct / 100f, Guid.Empty);
    if (op == "mute") v.SetMute(true, Guid.Empty);
    if (op == "unmute") v.SetMute(false, Guid.Empty);
    if (op == "up" || op == "set") { bool m; v.GetMute(out m); if (m && pct > 0) v.SetMute(false, Guid.Empty); }
    bool muted; v.GetMute(out muted);
    v.GetMasterVolumeLevelScalar(out cur);
    return ((int)Math.Round(cur * 100)) + " " + (muted ? "muted" : "unmuted");
  }
}
'@
[JitmoAudio]::Do($env:JITMO_OP, [int]$env:JITMO_N)
"#;

/// Screen brightness through WMI (laptop panels). Same env as the volume script.
pub const BRIGHTNESS_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$cur = (Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness | Select-Object -First 1).CurrentBrightness
if ($null -eq $cur) { throw 'no adjustable screen' }
$n = [int]$env:JITMO_N
switch ($env:JITMO_OP) {
  'up' { $new = [Math]::Min(100, $cur + $n) }
  'down' { $new = [Math]::Max(0, $cur - $n) }
  'set' { $new = [Math]::Max(0, [Math]::Min(100, $n)) }
  default { $new = $cur }
}
if ($new -ne $cur) {
  $m = Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Select-Object -First 1
  Invoke-CimMethod -InputObject $m -MethodName WmiSetBrightness -Arguments @{ Timeout = 1; Brightness = $new } | Out-Null
}
"$new"
"#;

/// Wi-Fi or Bluetooth on or off through the Windows radio API.
/// Env: `JITMO_KIND` = WiFi | Bluetooth, `JITMO_STATE` = On | Off | Get.
/// Prints the state after the change.
pub const RADIO_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, $type) {
  $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op)); $t.Wait(-1) | Out-Null; $t.Result
}
[Windows.Devices.Radios.Radio, Windows.System.Devices, ContentType = WindowsRuntime] | Out-Null
[Windows.Devices.Radios.RadioAccessStatus, Windows.System.Devices, ContentType = WindowsRuntime] | Out-Null
$access = Await ([Windows.Devices.Radios.Radio]::RequestAccessAsync()) ([Windows.Devices.Radios.RadioAccessStatus])
if ($access -ne 'Allowed') { throw "radio access: $access" }
$radios = Await ([Windows.Devices.Radios.Radio]::GetRadiosAsync()) ([System.Collections.Generic.IReadOnlyList[Windows.Devices.Radios.Radio]])
$r = $radios | Where-Object { $_.Kind -eq $env:JITMO_KIND } | Select-Object -First 1
if ($null -eq $r) { throw 'no such radio' }
if ($env:JITMO_STATE -ne 'Get') {
  Await ($r.SetStateAsync($env:JITMO_STATE)) ([Windows.Devices.Radios.RadioAccessStatus]) | Out-Null
}
"$($r.State)"
"#;

/// Save the whole screen as a PNG. Env: `JITMO_PATH`. Prints the path.
pub const SCREENSHOT_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
New-Item -ItemType Directory -Force -Path (Split-Path $env:JITMO_PATH) | Out-Null
$bmp.Save($env:JITMO_PATH, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
$env:JITMO_PATH
"#;

/// Lock the workstation.
pub const LOCK_SCRIPT: &str = "rundll32.exe user32.dll,LockWorkStation";

/// True while the lock screen is up (its process only exists then).
pub const IS_LOCKED_SCRIPT: &str =
    "if (Get-Process LogonUI -ErrorAction SilentlyContinue) { 'locked' } else { 'unlocked' }";

// ---------------------------------------------------------------------------
// Understanding the request
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Volume,
    Brightness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Up(u32),
    Down(u32),
    Set(u32),
    Mute,
    Unmute,
}

/// How far "a bit louder" moves things when no number is given.
const STEP: u32 = 10;

fn first_number(t: &str) -> Option<u32> {
    let mut digits = String::new();
    for c in t.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if !digits.is_empty() {
            break;
        }
    }
    digits.parse().ok().map(|n: u32| n.min(100))
}

/// "turn the volume down", "louder", "set brightness to 40", "mute", "dim the screen".
pub fn parse_adjust(text: &str) -> Option<(Target, Op)> {
    let t = text.to_lowercase();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &[&str]| words.iter().any(|x| w.contains(x));
    let target = if has(&[
        "brightness",
        "brighter",
        "dim",
        "dimmer",
        "darker",
        "screen",
    ]) {
        Target::Brightness
    } else if has(&[
        "volume", "louder", "quieter", "softer", "mute", "unmute", "sound", "audio",
    ]) {
        Target::Volume
    } else {
        return None;
    };
    let n = first_number(&t);
    let op = if has(&["unmute"]) {
        Op::Unmute
    } else if has(&["mute", "silence"]) && target == Target::Volume {
        Op::Mute
    } else if let (Some(n), true) = (n, t.contains(" to ") || has(&["set", "at"])) {
        Op::Set(n)
    } else if has(&[
        "up", "raise", "increase", "higher", "louder", "brighter", "more",
    ]) {
        Op::Up(n.unwrap_or(STEP))
    } else if has(&[
        "down", "lower", "decrease", "quieter", "softer", "dim", "dimmer", "darker", "less",
        "reduce",
    ]) {
        Op::Down(n.unwrap_or(STEP))
    } else {
        return None;
    };
    Some((target, op))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radio {
    WiFi,
    Bluetooth,
}

/// "turn on bluetooth", "wifi off", "disable wi-fi".
pub fn parse_radio(text: &str) -> Option<(Radio, bool)> {
    let t = text.to_lowercase().replace(['-', ' '], "");
    let radio = if t.contains("bluetooth") {
        Radio::Bluetooth
    } else if t.contains("wifi") || t.contains("wireless") {
        Radio::WiFi
    } else {
        return None;
    };
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_string)
        .collect();
    let any = |w: &[&str]| words.iter().any(|x| w.contains(&x.as_str()));
    let off = any(&["off", "disable", "disconnect", "kill"]);
    let on = any(&["on", "enable", "connect", "activate"]);
    match (on, off) {
        (true, false) => Some((radio, true)),
        (false, true) => Some((radio, false)),
        _ => None, // "turn wifi" or "on and off": ask rather than guess
    }
}

// ---------------------------------------------------------------------------
// Connector
// ---------------------------------------------------------------------------

pub struct SystemControls {
    runner: Arc<dyn ScriptRunner>,
    /// Where screenshots go.
    pictures: std::path::PathBuf,
    now: Box<dyn Fn() -> String + Send + Sync>,
}

impl SystemControls {
    pub fn new(runner: Arc<dyn ScriptRunner>, pictures: std::path::PathBuf) -> Self {
        SystemControls {
            runner,
            pictures,
            now: Box::new(|| chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()),
        }
    }

    pub fn with_stamp(mut self, now: impl Fn() -> String + Send + Sync + 'static) -> Self {
        self.now = Box::new(now);
        self
    }

    fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, ConnError> {
        self.runner
            .run(script, env)
            .map_err(|e| ConnError::Failed(friendly(&e)))
    }
}

/// Short, plain reasons instead of PowerShell error text.
fn friendly(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("no adjustable screen") || l.contains("wmimonitorbrightness") {
        "This screen's brightness can't be changed from here.".into()
    } else if l.contains("no such radio") {
        "I couldn't find that on this PC.".into()
    } else if l.contains("radio access") {
        "Windows didn't let me change that. Check Settings > Privacy.".into()
    } else {
        "I couldn't change that setting.".into()
    }
}

fn arg<'a>(args: &'a Args, name: &str) -> Result<&'a String, ConnError> {
    args.get(name)
        .ok_or_else(|| ConnError::MissingArg(name.to_string()))
}

const NOT_UNDERSTOOD: &str = "I'm not sure what to change. Try “turn the volume down”.";

impl Connector for SystemControls {
    fn id(&self) -> &str {
        "system_controls"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        let a = |n: &str, r: &[&str]| ActionSpec {
            name: n.into(),
            required_args: r.iter().map(|s| s.to_string()).collect(),
        };
        vec![
            a("adjust", &["text"]),
            a("lock", &[]),
            a("radio", &["text"]),
            a("screenshot", &[]),
        ]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let msg = |m: String| Output([("message".to_string(), m)].into());
        match action {
            "adjust" => {
                let (target, op) = parse_adjust(arg(args, "text")?)
                    .ok_or_else(|| ConnError::Failed(NOT_UNDERSTOOD.into()))?;
                let (name, n) = match op {
                    Op::Up(n) => ("up", n),
                    Op::Down(n) => ("down", n),
                    Op::Set(n) => ("set", n),
                    Op::Mute => ("mute", 0),
                    Op::Unmute => ("unmute", 0),
                };
                let n = n.to_string();
                let env = [("JITMO_OP", name), ("JITMO_N", n.as_str())];
                match target {
                    Target::Volume => {
                        let out = self.run(VOLUME_SCRIPT, &env)?;
                        let (pct, state) = out.split_once(' ').unwrap_or((&out, ""));
                        Ok(msg(if state == "muted" {
                            format!("Volume is muted (at {pct}%).")
                        } else {
                            format!("Volume is at {pct}%.")
                        }))
                    }
                    Target::Brightness => {
                        let out = self.run(BRIGHTNESS_SCRIPT, &env)?;
                        Ok(msg(format!("Brightness is at {out}%.")))
                    }
                }
            }
            "lock" => {
                self.run(LOCK_SCRIPT, &[])?;
                Ok(msg("Locked.".into()))
            }
            "radio" => {
                let (radio, on) = parse_radio(arg(args, "text")?)
                    .ok_or_else(|| ConnError::Failed("Do you want it on or off?".into()))?;
                let (kind, label) = match radio {
                    Radio::WiFi => ("WiFi", "Wi-Fi"),
                    Radio::Bluetooth => ("Bluetooth", "Bluetooth"),
                };
                let state = if on { "On" } else { "Off" };
                let out = self.run(
                    RADIO_SCRIPT,
                    &[("JITMO_KIND", kind), ("JITMO_STATE", state)],
                )?;
                Ok(Output(
                    [
                        (
                            "message".to_string(),
                            format!("{label} is {}.", out.to_lowercase()),
                        ),
                        ("kind".to_string(), kind.to_string()),
                        ("was".to_string(), (!on).to_string()),
                    ]
                    .into(),
                ))
            }
            "screenshot" => {
                let path = self
                    .pictures
                    .join("Screenshots")
                    .join(format!("jitmo-{}.png", (self.now)()));
                let p = path.display().to_string();
                self.run(SCREENSHOT_SCRIPT, &[("JITMO_PATH", &p)])?;
                Ok(Output(
                    [
                        ("message".to_string(), format!("Screenshot saved to {p}")),
                        ("path".to_string(), p),
                    ]
                    .into(),
                ))
            }
            other => Err(ConnError::UnknownAction(other.to_string())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        match action {
            // The state can be read back; whether it changed by the right amount cannot
            // (a check sees only the arguments), so this proves the control is reachable.
            "adjust" => {
                let (target, _) = parse_adjust(arg(args, "text")?)
                    .ok_or_else(|| ConnError::Failed(NOT_UNDERSTOOD.into()))?;
                let env = [("JITMO_OP", "get"), ("JITMO_N", "0")];
                let script = match target {
                    Target::Volume => VOLUME_SCRIPT,
                    Target::Brightness => BRIGHTNESS_SCRIPT,
                };
                Ok(self.run(script, &env).is_ok())
            }
            "lock" => {
                // The lock screen takes a moment to appear.
                for _ in 0..6 {
                    if self.run(IS_LOCKED_SCRIPT, &[])? == "locked" {
                        return Ok(true);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Ok(false)
            }
            "radio" => {
                let (radio, on) = parse_radio(arg(args, "text")?)
                    .ok_or_else(|| ConnError::Failed("Do you want it on or off?".into()))?;
                let kind = match radio {
                    Radio::WiFi => "WiFi",
                    Radio::Bluetooth => "Bluetooth",
                };
                let state = self.run(
                    RADIO_SCRIPT,
                    &[("JITMO_KIND", kind), ("JITMO_STATE", "Get")],
                )?;
                Ok(state.eq_ignore_ascii_case(if on { "On" } else { "Off" }))
            }
            "screenshot" => {
                let dir = self.pictures.join("Screenshots");
                let recent = std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("jitmo-"))
                    .filter_map(|e| e.metadata().ok()?.modified().ok())
                    .any(|t| t.elapsed().is_ok_and(|d| d.as_secs() < 60));
                Ok(recent)
            }
            other => Err(ConnError::UnknownAction(other.to_string())),
        }
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        match action {
            // A screenshot is undone by deleting the file it made.
            "screenshot" => {
                let path = output.0.get("path")?;
                Some(
                    std::fs::remove_file(path)
                        .map_err(|e| ConnError::Failed(format!("Couldn't delete it: {e}"))),
                )
            }
            // A radio is undone by putting it back.
            "radio" => {
                let kind = output.0.get("kind")?;
                let was_on: bool = output.0.get("was")?.parse().ok()?;
                let state = if was_on { "On" } else { "Off" };
                Some(
                    self.run(
                        RADIO_SCRIPT,
                        &[("JITMO_KIND", kind), ("JITMO_STATE", state)],
                    )
                    .map(|_| ()),
                )
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    type Call = (String, Vec<(String, String)>);

    struct Recorder {
        calls: Mutex<Vec<Call>>,
        reply: Mutex<Result<String, String>>,
    }

    impl Recorder {
        fn replying(r: Result<&str, &str>) -> Arc<Self> {
            Arc::new(Recorder {
                calls: Mutex::new(Vec::new()),
                reply: Mutex::new(r.map(String::from).map_err(String::from)),
            })
        }
    }

    impl ScriptRunner for Recorder {
        fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
            self.calls.lock().unwrap().push((
                script.to_string(),
                env.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            self.reply.lock().unwrap().clone()
        }
    }

    fn controls(r: &Arc<Recorder>) -> SystemControls {
        SystemControls::new(r.clone(), std::env::temp_dir().join("jitmo-sys-test"))
            .with_stamp(|| "STAMP".into())
    }

    fn args(text: &str) -> Args {
        [("text".to_string(), text.to_string())].into()
    }

    fn env_of(r: &Recorder, key: &str) -> String {
        r.calls
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .1
            .iter()
            .find(|(k, _)| k == key)
            .unwrap()
            .1
            .clone()
    }

    #[test]
    fn volume_and_brightness_requests_are_understood() {
        use Op::*;
        use Target::*;
        let cases = [
            ("Turn the volume down.", (Volume, Down(10))),
            ("louder", (Volume, Up(10))),
            ("turn the volume up by 25", (Volume, Up(25))),
            ("set the volume to 30", (Volume, Set(30))),
            ("mute", (Volume, Mute)),
            ("unmute the sound", (Volume, Unmute)),
            ("make the screen brighter", (Brightness, Up(10))),
            ("dim the screen", (Brightness, Down(10))),
            ("set brightness to 40%", (Brightness, Set(40))),
        ];
        for (text, want) in cases {
            assert_eq!(parse_adjust(text), Some(want), "{text}");
        }
        assert_eq!(parse_adjust("hello there"), None);
        assert_eq!(
            parse_adjust("volume"),
            None,
            "no direction: ask, do not guess"
        );
    }

    #[test]
    fn radio_requests_are_understood() {
        assert_eq!(
            parse_radio("Turn on Bluetooth."),
            Some((Radio::Bluetooth, true))
        );
        assert_eq!(parse_radio("wifi off"), Some((Radio::WiFi, false)));
        assert_eq!(parse_radio("disable Wi-Fi"), Some((Radio::WiFi, false)));
        assert_eq!(parse_radio("turn wifi"), None);
        assert_eq!(parse_radio("turn on and off wifi"), None);
        assert_eq!(parse_radio("turn on the lights"), None);
    }

    #[test]
    fn t45_volume_down_runs_the_volume_script_with_values_in_the_environment() {
        let r = Recorder::replying(Ok("40 unmuted"));
        let out = controls(&r)
            .execute("adjust", &args("Turn the volume down."))
            .unwrap();
        assert_eq!(out.0["message"], "Volume is at 40%.");
        assert_eq!(env_of(&r, "JITMO_OP"), "down");
        assert_eq!(env_of(&r, "JITMO_N"), "10");
        assert!(r.calls.lock().unwrap()[0]
            .0
            .contains("IAudioEndpointVolume"));
    }

    #[test]
    fn brightness_uses_the_brightness_script_and_explains_failures() {
        let r = Recorder::replying(Ok("60"));
        let out = controls(&r)
            .execute("adjust", &args("make it brighter, screen"))
            .unwrap();
        assert_eq!(out.0["message"], "Brightness is at 60%.");
        assert!(r.calls.lock().unwrap()[0].0.contains("WmiSetBrightness"));
        let r = Recorder::replying(Err("no adjustable screen"));
        let e = controls(&r)
            .execute("adjust", &args("dim the screen"))
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "This screen's brightness can't be changed from here."
        );
    }

    #[test]
    fn t47_bluetooth_on_and_undo_puts_it_back() {
        let r = Recorder::replying(Ok("On"));
        let c = controls(&r);
        let out = c.execute("radio", &args("Turn on Bluetooth.")).unwrap();
        assert_eq!(out.0["message"], "Bluetooth is on.");
        assert_eq!(env_of(&r, "JITMO_KIND"), "Bluetooth");
        assert_eq!(env_of(&r, "JITMO_STATE"), "On");
        c.undo("radio", &out).unwrap().unwrap();
        assert_eq!(env_of(&r, "JITMO_STATE"), "Off");
    }

    #[test]
    fn radio_check_reads_the_state_back() {
        let on = Recorder::replying(Ok("On"));
        assert!(controls(&on)
            .check("radio", &args("turn on bluetooth"))
            .unwrap());
        assert!(!controls(&on)
            .check("radio", &args("turn off bluetooth"))
            .unwrap());
        assert_eq!(env_of(&on, "JITMO_STATE"), "Get");
    }

    #[test]
    fn an_unclear_radio_request_asks_instead_of_acting() {
        let r = Recorder::replying(Ok("On"));
        assert!(controls(&r).execute("radio", &args("turn wifi")).is_err());
        assert!(r.calls.lock().unwrap().is_empty(), "no script may run");
    }

    #[test]
    fn t46_lock_runs_the_lock_command_and_check_waits_for_the_lock_screen() {
        let r = Recorder::replying(Ok("locked"));
        let c = controls(&r);
        c.execute("lock", &Args::new()).unwrap();
        assert_eq!(r.calls.lock().unwrap()[0].0, LOCK_SCRIPT);
        assert!(c.check("lock", &Args::new()).unwrap());
    }

    #[test]
    fn screenshot_goes_to_pictures_and_undo_deletes_it() {
        let r = Recorder::replying(Ok(""));
        let c = controls(&r);
        let out = c.execute("screenshot", &Args::new()).unwrap();
        let path = out.0["path"].clone();
        assert!(path.ends_with("jitmo-STAMP.png"), "{path}");
        assert!(out.0["message"].starts_with("Screenshot saved to "));
        assert_eq!(env_of(&r, "JITMO_PATH"), path);
        // Pretend the script made the file, then undo.
        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
        std::fs::write(&path, "x").unwrap();
        assert!(c.check("screenshot", &Args::new()).unwrap());
        c.undo("screenshot", &out).unwrap().unwrap();
        assert!(!std::path::Path::new(&path).exists());
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("jitmo-sys-test"));
    }

    #[test]
    fn user_words_never_reach_a_script() {
        let r = Recorder::replying(Ok("50 unmuted"));
        let evil = "turn the volume down; Remove-Item C:\\ -Recurse";
        let _ = controls(&r).execute("adjust", &args(evil));
        for (script, env) in r.calls.lock().unwrap().iter() {
            assert!(!script.contains("Remove-Item"));
            assert!(env.iter().all(|(_, v)| !v.contains("Remove-Item")));
        }
    }
}

#[cfg(all(test, windows))]
mod live {
    use super::*;

    /// Read-only: runs each script in its `get` mode, which changes nothing.
    /// `cargo test -p jitmo-connectors live_read_only -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_read_only_checks_of_the_scripts() {
        let ps = PowerShell;
        let vol = ps.run(VOLUME_SCRIPT, &[("JITMO_OP", "get"), ("JITMO_N", "0")]);
        println!("volume: {vol:?}");
        let bri = ps.run(BRIGHTNESS_SCRIPT, &[("JITMO_OP", "get"), ("JITMO_N", "0")]);
        println!("brightness: {bri:?}");
        for kind in ["WiFi", "Bluetooth"] {
            let r = ps.run(
                RADIO_SCRIPT,
                &[("JITMO_KIND", kind), ("JITMO_STATE", "Get")],
            );
            println!("{kind}: {r:?}");
        }
        println!("locked: {:?}", ps.run(IS_LOCKED_SCRIPT, &[]));
        assert!(vol.is_ok(), "the volume script must at least run: {vol:?}");
    }
}
