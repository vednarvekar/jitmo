//! The skills audit harness: drives the REAL `Flow` and the REAL router over a corpus of
//! messy inputs and records, for every input, what the user would see and which connector
//! calls would be made.
//!
//! Deterministic and offline:
//! - the clock is fixed (Monday 9 Nov 2026, 10:00 local);
//! - connectors that act on the PC (Spotify, browser, Do Not Disturb, volume, radios, lock,
//!   screenshot) are the REAL connectors, but their PowerShell runner is a simulator that only
//!   records what it was asked to do (`script:` lines in the report);
//! - reminders and settings are real, on a throw-away database file;
//! - weather and fact lookups are real connectors over an HTTP stub that records the URL
//!   (so the report shows exactly which words would leave the device) and answers "offline";
//! - opening apps and file actions go to recording stand-ins (no app is launched, no file moves).
//!
//! The corpus is `testset/audit_corpus.txt`, one case per line:
//! `skill | kind | input | expectation | note`. See [`parse_expect`] for the expectation words.

use jitmo_connectors::fake::FakeConnector;
use jitmo_connectors::spotify;
use jitmo_connectors::system_controls::{self, ScriptRunner};
use jitmo_connectors::web_lookup::{Http, HttpError};
use jitmo_connectors::{Args, ConnError, Connector, Health, Output};
use jitmo_core::flow::{Flow, Prompter};
use jitmo_core::known::Known;
use jitmo_core::resolver::{ContextSource, FileRef};
use jitmo_core::router::{decide, RouteDecision, Router};
use jitmo_ipc::{Choice, CoreEvent};
use jitmo_model_worker::{WorkerClient, WorkerLauncher};
use jitmo_skills::Skill;
use jitmo_store::Store;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Case {
    pub line: usize,
    pub skill: String,
    pub kind: String,
    pub input: String,
    pub expect: String,
    pub note: String,
}

/// Expand the placeholders a corpus line may use for awkward inputs.
pub fn expand_input(raw: &str) -> String {
    let t = raw.trim();
    if t == "<empty>" {
        return String::new();
    }
    if t == "<spaces>" {
        return "     ".into();
    }
    if let Some(rest) = t.strip_prefix("<long:").and_then(|r| r.strip_suffix('>')) {
        let (n, word) = rest.split_once(':').unwrap_or(("100", "word"));
        let n: usize = n.parse().unwrap_or(100);
        return vec![word; n].join(" ");
    }
    t.replace("<tab>", "\t").replace("<nl>", "\n")
}

pub fn load_corpus(text: &str) -> Vec<Case> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = l.splitn(5, '|').map(str::trim).collect();
        if parts.len() < 4 {
            eprintln!("corpus line {}: needs at least 4 fields: {l}", i + 1);
            continue;
        }
        out.push(Case {
            line: i + 1,
            skill: parts[0].to_string(),
            kind: parts[1].to_string(),
            input: expand_input(parts[2]),
            expect: parts[3].to_string(),
            note: parts.get(4).unwrap_or(&"").to_string(),
        });
    }
    out
}

/// What a case expects. Words, separated by `;`:
///
/// Primary outcome (first word; alternatives with `/`, e.g. `ask/decline`):
/// - `run:<skill.id>`  the task ran and ended in Done (a route "Did you mean" that the
///   user approves is allowed; the global rules still judge its text)
/// - `ask`             the user is asked something (choice or typed question) and nothing
///   is executed before the answer
/// - `decline`         clear "can't do that", nothing executed, nothing asked
/// - `fail:<skill.id>` ends in Failed after routing to this skill (an honest error)
///
/// Modifiers:
/// - `call=connector.action`  that connector call was made
/// - `nocall=connector.action` it was NOT made
/// - `arg=key~text`  / `arg=key!~text`  an argument of the matching call contains / lacks text
/// - `msg~text` / `msg!~text`   the final message contains / lacks text (case-insensitive)
/// - `ui~text`  some question, choice label or confirm text contains text
/// - `opt~text` some choice label contains text
/// - `noexec`   no connector executed anything
/// - `pick=text` / `say=text` answer the first choice containing text / the typed question
/// - `env=flag` simulator flag (nospotify, noplay, stuck, nobrightness, unlock, dnd_stuck, dnd_missing)
/// - `undo`     press Undo after the run;  `pending=N` reminders left afterwards
#[derive(Debug, Default, Clone)]
pub struct Expect {
    pub primary: Vec<String>,
    pub calls: Vec<String>,
    pub nocalls: Vec<String>,
    pub args: Vec<(String, bool, String)>, // key, must_contain, text
    pub msg: Vec<(bool, String)>,
    pub ui: Vec<String>,
    pub opt: Vec<String>,
    pub noexec: bool,
    pub pick: Option<String>,
    pub say: Option<String>,
    pub flags: HashSet<String>,
    pub undo: bool,
    pub pending: Option<usize>,
    pub urls: Vec<(bool, String)>,
}

pub fn parse_expect(s: &str) -> Expect {
    let mut e = Expect::default();
    for (i, w) in s
        .split(';')
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .enumerate()
    {
        if i == 0 {
            e.primary = w.split('/').map(|x| x.trim().to_string()).collect();
            continue;
        }
        // "msg~paused/run:system.volume": a later piece that starts with an outcome word is
        // another acceptable outcome, not part of the modifier.
        let mut w = w;
        if let Some(at) = w.find('/') {
            let tail = &w[at + 1..];
            let outcome = ["ask", "decline", "done", "run:", "fail:"]
                .iter()
                .any(|k| tail.starts_with(k));
            if outcome {
                for alt in tail.split('/') {
                    e.primary.push(alt.trim().to_string());
                }
                w = &w[..at];
            }
        }
        if let Some(v) = w.strip_prefix("call=") {
            e.calls.push(v.to_string());
        } else if let Some(v) = w.strip_prefix("nocall=") {
            e.nocalls.push(v.to_string());
        } else if let Some(v) = w.strip_prefix("arg=") {
            if let Some((k, t)) = v.split_once("!~") {
                e.args.push((k.to_string(), false, t.to_lowercase()));
            } else if let Some((k, t)) = v.split_once('~') {
                e.args.push((k.to_string(), true, t.to_lowercase()));
            }
        } else if let Some(v) = w.strip_prefix("msg!~") {
            e.msg.push((false, v.to_lowercase()));
        } else if let Some(v) = w.strip_prefix("msg~") {
            e.msg.push((true, v.to_lowercase()));
        } else if let Some(v) = w.strip_prefix("ui~") {
            e.ui.push(v.to_lowercase());
        } else if let Some(v) = w.strip_prefix("opt~") {
            e.opt.push(v.to_lowercase());
        } else if w == "noexec" {
            e.noexec = true;
        } else if let Some(v) = w.strip_prefix("pick=") {
            e.pick = Some(v.to_lowercase());
        } else if let Some(v) = w.strip_prefix("say=") {
            e.say = Some(v.to_string());
        } else if let Some(v) = w.strip_prefix("env=") {
            e.flags.insert(v.to_string());
        } else if w == "undo" {
            e.undo = true;
        } else if let Some(v) = w.strip_prefix("url!~") {
            e.urls.push((false, v.to_lowercase()));
        } else if let Some(v) = w.strip_prefix("url~") {
            e.urls.push((true, v.to_lowercase()));
        } else if let Some(v) = w.strip_prefix("pending=") {
            e.pending = v.parse().ok();
        }
    }
    e
}

// ---------------------------------------------------------------------------
// Recording the world
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CallRec {
    pub connector: String,
    pub action: String,
    pub args: Vec<(String, String)>,
    /// `Ok(message)` or `Err(error text)`; `None` for script lines.
    pub result: Option<Result<String, String>>,
    pub script: bool,
}

type Log = Arc<Mutex<Vec<CallRec>>>;

/// Wraps a connector (or stands in for one): records every execute and check.
struct Rec {
    id: String,
    inner: Option<Arc<dyn Connector>>,
    stand_in: FakeConnector,
    log: Log,
    checks: Arc<AtomicUsize>,
    ms: Arc<AtomicUsize>,
}

impl Connector for Rec {
    fn id(&self) -> &str {
        &self.id
    }
    fn actions(&self) -> Vec<jitmo_connectors::ActionSpec> {
        self.inner
            .as_ref()
            .map_or_else(|| self.stand_in.actions(), |c| c.actions())
    }
    fn health(&self) -> Health {
        self.inner
            .as_ref()
            .map_or_else(|| self.stand_in.health(), |c| c.health())
    }
    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let t0 = Instant::now();
        let r = match &self.inner {
            Some(c) => c.execute(action, args),
            None => self.stand_in.execute(action, args),
        };
        self.ms
            .fetch_add(t0.elapsed().as_millis() as usize, Ordering::SeqCst);
        let result = match &r {
            Ok(o) => Ok(o.0.get("message").cloned().unwrap_or_default()),
            Err(e) => Err(e.to_string()),
        };
        self.log.lock().unwrap().push(CallRec {
            connector: self.id.clone(),
            action: action.to_string(),
            args: args.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            result: Some(result),
            script: false,
        });
        r
    }
    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        let t0 = Instant::now();
        let r = match &self.inner {
            Some(c) => c.check(action, args),
            None => self.stand_in.check(action, args),
        };
        self.ms
            .fetch_add(t0.elapsed().as_millis() as usize, Ordering::SeqCst);
        r
    }
    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        match &self.inner {
            Some(c) => c.undo(action, output),
            None => self.stand_in.undo(action, output),
        }
    }
}

/// Simulates the PC behind every PowerShell script, and records what was asked.
struct Sim {
    log: Log,
    flags: HashSet<String>,
    state: Mutex<SimState>,
    pictures: PathBuf,
}

struct SimState {
    vol: i64,
    muted: bool,
    bright: i64,
    wifi: bool,
    bt: bool,
    dnd: bool,
    media: &'static str,
}

impl Sim {
    fn note(&self, name: &str, env: &[(&str, &str)], result: &Result<String, String>) {
        self.log.lock().unwrap().push(CallRec {
            connector: "script".into(),
            action: name.into(),
            args: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            result: Some(result.clone()),
            script: true,
        });
    }
}

fn envv<'a>(env: &'a [(&str, &str)], k: &str) -> &'a str {
    env.iter().find(|(n, _)| *n == k).map_or("", |(_, v)| *v)
}

impl ScriptRunner for Sim {
    fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
        let mut s = self.state.lock().unwrap();
        let (name, r): (&str, Result<String, String>) = if script == spotify::MEDIA_SCRIPT {
            let op = envv(env, "JITMO_OP");
            if self.flags.contains("nospotify") {
                ("media", Err("no spotify session".into()))
            } else {
                match op {
                    "pause" => s.media = "Paused",
                    "play" => s.media = "Playing",
                    _ => {}
                }
                ("media", Ok(s.media.to_string()))
            }
        } else if script == spotify::PLAY_FIRST_RESULT_SCRIPT {
            if self.flags.contains("noplay") {
                ("press_play", Err("no play button".into()))
            } else {
                if !self.flags.contains("stuck") {
                    s.media = "Playing";
                }
                ("press_play", Ok("INVOKED Play Top result".into()))
            }
        } else if script == spotify::OPEN_SCRIPT {
            if self.flags.contains("nolink") {
                ("open_url", Err("no handler".into()))
            } else {
                ("open_url", Ok(String::new()))
            }
        } else if script == system_controls::VOLUME_SCRIPT {
            let n: i64 = envv(env, "JITMO_N").parse().unwrap_or(0);
            match envv(env, "JITMO_OP") {
                "up" => {
                    s.vol = (s.vol + n).min(100);
                    s.muted = false;
                }
                "down" => s.vol = (s.vol - n).max(0),
                "set" => {
                    s.vol = n.clamp(0, 100);
                    s.muted = false;
                }
                "mute" => s.muted = true,
                "unmute" => s.muted = false,
                _ => {}
            }
            (
                "volume",
                Ok(format!(
                    "{} {}",
                    s.vol,
                    if s.muted { "muted" } else { "unmuted" }
                )),
            )
        } else if script == system_controls::BRIGHTNESS_SCRIPT {
            if self.flags.contains("nobrightness") {
                ("brightness", Err("no adjustable screen".into()))
            } else {
                let n: i64 = envv(env, "JITMO_N").parse().unwrap_or(0);
                match envv(env, "JITMO_OP") {
                    "up" => s.bright = (s.bright + n).min(100),
                    "down" => s.bright = (s.bright - n).max(0),
                    "set" => s.bright = n.clamp(0, 100),
                    _ => {}
                }
                ("brightness", Ok(s.bright.to_string()))
            }
        } else if script == system_controls::RADIO_SCRIPT {
            let slot = if envv(env, "JITMO_KIND") == "Bluetooth" {
                &mut s.bt
            } else {
                &mut s.wifi
            };
            match envv(env, "JITMO_STATE") {
                "On" => *slot = true,
                "Off" => *slot = false,
                _ => {}
            }
            ("radio", Ok(if *slot { "On" } else { "Off" }.to_string()))
        } else if script == system_controls::LOCK_SCRIPT {
            ("lock", Ok(String::new()))
        } else if script == system_controls::IS_LOCKED_SCRIPT {
            (
                "is_locked",
                Ok(if self.flags.contains("unlock") {
                    "unlocked"
                } else {
                    "locked"
                }
                .into()),
            )
        } else if script == system_controls::SCREENSHOT_SCRIPT {
            let p = envv(env, "JITMO_PATH");
            if let Some(parent) = Path::new(p).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(p, b"png");
            let _ = &self.pictures;
            ("screenshot", Ok(p.to_string()))
        } else if script == jitmo_connectors::dnd::DND_SCRIPT {
            if self.flags.contains("dnd_missing") {
                ("dnd", Ok("switch-not-found".into()))
            } else {
                let was = s.dnd;
                match envv(env, "JITMO_STATE") {
                    "On" if !self.flags.contains("dnd_stuck") => s.dnd = true,
                    "Off" if !self.flags.contains("dnd_stuck") => s.dnd = false,
                    _ => {}
                }
                let f = |b: bool| if b { "On" } else { "Off" };
                ("dnd", Ok(format!("was={} now={}", f(was), f(s.dnd))))
            }
        } else {
            ("unknown_script", Ok(String::new()))
        };
        drop(s);
        self.note(name, env, &r);
        r
    }
}

/// Records the URL of every lookup and answers "offline".
struct StubHttp {
    log: Log,
}

impl Http for StubHttp {
    fn get(&self, url: &str) -> Result<String, HttpError> {
        self.log.lock().unwrap().push(CallRec {
            connector: "http".into(),
            action: "GET".into(),
            args: vec![("url".into(), url.to_string())],
            result: None,
            script: true,
        });
        Err(HttpError::Offline)
    }
}

// ---------------------------------------------------------------------------
// The fixture: apps, files, folders
// ---------------------------------------------------------------------------

pub const APPS: &[&str] = &[
    "Spotify",
    "Calculator",
    "Notepad",
    "Google Chrome",
    "Microsoft Edge",
    "Visual Studio Code",
    "WhatsApp",
    "Settings",
    "File Explorer",
    "Word",
    "Excel",
    "Task Manager",
    "Paint",
    "Discord",
    "Telegram",
    "Zoom",
    "VLC media player",
    "Snipping Tool",
];

struct Fixture {
    files: Vec<FileRef>,
    folders: Vec<FileRef>,
}

impl Fixture {
    fn new(root: &Path) -> Self {
        let docs = root.join("Documents");
        let downloads = root.join("Downloads");
        let _ = std::fs::create_dir_all(&docs);
        let _ = std::fs::create_dir_all(&downloads);
        let names = [
            "invoice_march.pdf",
            "invoice_april.pdf",
            "resume.docx",
            "report.pdf",
            "budget.xlsx",
            "photo.jpg",
            "notes.txt",
            "Tax return 2025.pdf",
            "ticket.pdf",
        ];
        let mut files = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let p = docs.join(n);
            let _ = std::fs::write(&p, b"x");
            files.push(FileRef {
                name: n.to_string(),
                path: p.display().to_string(),
                modified: 1000 + i as u64,
            });
        }
        for n in ["setup.exe", "ticket.pdf", "song.mp3"] {
            let _ = std::fs::write(downloads.join(n), b"x");
        }
        let mut folders = Vec::new();
        for (i, n) in [
            "Documents",
            "Invoices",
            "Downloads",
            "Desktop",
            "Pictures",
            "Work",
        ]
        .iter()
        .enumerate()
        {
            let p = root.join(n);
            let _ = std::fs::create_dir_all(&p);
            folders.push(FileRef {
                name: n.to_string(),
                path: p.display().to_string(),
                modified: 1000 + i as u64,
            });
        }
        Fixture { files, folders }
    }
}

impl ContextSource for Fixture {
    fn selected_file(&self) -> Option<FileRef> {
        None
    }
    fn recent_downloads(&self, limit: usize) -> Vec<FileRef> {
        let mut f = self.files.clone();
        f.sort_by_key(|x| std::cmp::Reverse(x.modified));
        f.truncate(limit);
        f
    }
    fn searchable_files(&self) -> Vec<FileRef> {
        self.files.clone()
    }
    fn searchable_folders(&self) -> Vec<FileRef> {
        self.folders.clone()
    }
}

// ---------------------------------------------------------------------------
// One case
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Prompt {
    Confirm(String),
    Choose {
        question: String,
        labels: Vec<String>,
    },
    Ask {
        question: String,
        hint: String,
    },
}

struct User {
    prompts: Vec<Prompt>,
    pick: Option<String>,
    say: Option<String>,
}

impl Prompter for User {
    fn confirm(&mut self, text: &str) -> bool {
        self.prompts.push(Prompt::Confirm(text.to_string()));
        true
    }
    fn choose(&mut self, question: &str, choices: &[Choice]) -> Option<String> {
        self.prompts.push(Prompt::Choose {
            question: question.to_string(),
            labels: choices.iter().map(|c| c.label.clone()).collect(),
        });
        let want = self.pick.clone()?;
        choices
            .iter()
            .find(|c| c.label.to_lowercase().contains(&want))
            .map(|c| c.id.clone())
    }
    fn ask_text(&mut self, question: &str, hint: &str) -> Option<String> {
        self.prompts.push(Prompt::Ask {
            question: question.to_string(),
            hint: hint.to_string(),
        });
        self.say.take()
    }
}

#[derive(Debug, Clone)]
pub struct Observed {
    pub case: Case,
    pub label: String,
    pub score: f32,
    pub out_of_scope: f32,
    pub decision: String,
    pub spans: Vec<(String, String)>,
    pub prompts: Vec<Prompt>,
    pub calls: Vec<CallRec>,
    pub done: bool,
    pub message: String,
    pub undoable: bool,
    pub logged_skill: Option<String>,
    pub undo_message: Option<String>,
    pub pending_reminders: usize,
    pub checks: usize,
    pub elapsed_ms: u64,
    pub connector_ms: u64,
    pub failures: Vec<String>,
}

impl Observed {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    /// Everything the user would read for this input.
    pub fn user_texts(&self) -> Vec<String> {
        let mut v = Vec::new();
        for p in &self.prompts {
            match p {
                Prompt::Confirm(t) => v.push(t.clone()),
                Prompt::Choose { question, labels } => {
                    v.push(question.clone());
                    v.extend(labels.iter().cloned());
                }
                Prompt::Ask { question, .. } => v.push(question.clone()),
            }
        }
        v.push(self.message.clone());
        v
    }

    pub fn exec_calls(&self) -> impl Iterator<Item = &CallRec> {
        self.calls.iter().filter(|c| !c.script)
    }

    /// One compact line of what happened.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!(
            "route={}@{:.2} oos={:.2} -> {}",
            self.label, self.score, self.out_of_scope, self.decision
        )];
        for p in &self.prompts {
            parts.push(match p {
                Prompt::Confirm(t) => format!("CONFIRM \"{t}\""),
                Prompt::Choose { question, labels } => {
                    format!("CHOICE \"{question}\" [{}]", labels.join(" | "))
                }
                Prompt::Ask { question, .. } => format!("ASK \"{question}\""),
            });
        }
        for c in &self.calls {
            let args = c
                .args
                .iter()
                .map(|(k, v)| format!("{k}={v:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let res = match &c.result {
                Some(Ok(m)) if !m.is_empty() => format!(" => \"{m}\""),
                Some(Err(e)) => format!(" => ERR \"{e}\""),
                _ => String::new(),
            };
            if c.script {
                parts.push(format!("script:{}({args})", c.action));
            } else {
                parts.push(format!("CALL {}.{}({args}){res}", c.connector, c.action));
            }
        }
        parts.push(format!(
            "{} \"{}\"{}",
            if self.done { "DONE" } else { "FAILED" },
            self.message,
            if self.undoable { " [undo]" } else { "" }
        ));
        if let Some(u) = &self.undo_message {
            parts.push(format!("UNDO \"{u}\""));
        }
        parts.join("  ;  ")
    }
}

/// Words copied from the skills' example sentences that must never show up in text built for
/// a different request.
const EXAMPLE_WORDS: &[&str] = &[
    "arijit",
    "singh",
    "mkbhd",
    "mom",
    "vitamins",
    "pasta",
    "australia",
    "everest",
    "pride",
    "prejudice",
    "pune",
    "tokyo",
    "york",
    "amazon",
    "github",
    "mumbai",
    "ved",
    "jazz",
    "mom's",
    "invoices",
    "tmrw",
];

fn words_of(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .replace("'s", "")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Rules that apply to every case (relevance, honesty, hygiene).
fn global_rules(o: &Observed, examples: &HashSet<String>, failures: &mut Vec<String>) {
    let input_words = words_of(&o.case.input);
    // G1: example words that the user did not type.
    for t in o.user_texts() {
        for w in words_of(&t) {
            if EXAMPLE_WORDS.contains(&w.as_str()) && !input_words.contains(&w) {
                failures.push(format!("G1 leaked example word '{w}' in: \"{t}\""));
            }
        }
        // G2: raw placeholders.
        if t.contains('{') || t.contains('}') {
            failures.push(format!("G2 raw placeholder shown to the user: \"{t}\""));
        }
    }
    // G3: a confirm or a choice label that is a skill's example sentence, word for word, and not
    // what the user typed.
    let norm = |s: &str| -> String {
        s.trim_start_matches("Did you mean:")
            .trim()
            .trim_end_matches(['?', '.'])
            .to_lowercase()
    };
    let typed = norm(&o.case.input);
    for p in &o.prompts {
        let shown: Vec<&String> = match p {
            Prompt::Confirm(t) if t.starts_with("Did you mean") => vec![t],
            Prompt::Choose { labels, .. } => labels.iter().collect(),
            _ => Vec::new(),
        };
        for t in shown {
            let n = norm(t);
            if examples.contains(&n) && n != typed {
                failures.push(format!(
                    "G3 a skill's example sentence is shown instead of the user's request: \"{t}\""
                ));
            }
        }
    }
    // G5: raw system text in a message.
    let low = o.message.to_lowercase();
    for bad in [
        "exception",
        "powershell",
        "0x8",
        "at line:",
        "errorrecord",
        "panicked",
        "os error",
        "stack",
    ] {
        if low.contains(bad) {
            failures.push(format!(
                "G5 raw system text in the result: \"{}\"",
                o.message
            ));
            break;
        }
    }
    // G4: result shape.
    if o.message.trim().is_empty() {
        failures.push("G4 empty result".into());
    } else if o.message.contains('\n') || o.message.chars().count() > 220 {
        failures.push(format!(
            "G4 result is not one short line ({} chars)",
            o.message.chars().count()
        ));
    }
    // G9: did something, then said it failed.
    if !o.done
        && !o.case.input.contains(" && ")
        && o.exec_calls().any(|c| matches!(c.result, Some(Ok(_))))
    {
        failures.push(format!(
            "G9 the action ran but the user is told it failed: \"{}\"",
            o.message
        ));
    }
    // G10: a question shown as an error (the bar closes, there is no way to answer).
    if !o.done && o.message.trim_end().ends_with('?') {
        failures.push(format!(
            "G10 a question delivered as a failure: \"{}\"",
            o.message
        ));
    }
    // G11: "Done." for something that gave the user nothing to read back.
    if o.done && o.message == "Done." && o.exec_calls().any(|c| c.connector == "reminders") {
        failures.push("G11 bare Done. for a reminder".into());
    }
    // G12: slow.
    if o.connector_ms > 3000 {
        failures.push(format!(
            "G12 connectors blocked the core thread for {} ms",
            o.connector_ms
        ));
    }
}

fn check_expect(o: &Observed, ex: &Expect, failures: &mut Vec<String>) {
    let asked = o
        .prompts
        .iter()
        .any(|p| matches!(p, Prompt::Choose { .. } | Prompt::Ask { .. }));
    let executed = o.exec_calls().next().is_some();
    let mut ok = ex.primary.is_empty();
    let mut first_ok = ex.primary.is_empty();
    let mut wanted = Vec::new();
    for (n, want) in ex.primary.iter().enumerate() {
        wanted.push(want.clone());
        let this = if let Some(id) = want.strip_prefix("run:") {
            o.done && o.logged_skill.as_deref() == Some(id)
        } else if want == "done" {
            o.done
        } else if want == "ask" {
            asked && (!executed || ex.pick.is_some() || ex.say.is_some())
        } else if want == "decline" {
            !o.done
                && !executed
                && o.prompts.is_empty()
                && (o.message.contains("can't do that") || o.message.contains("can't plan"))
        } else if let Some(id) = want.strip_prefix("fail:") {
            !o.done && o.logged_skill.as_deref() == Some(id)
        } else {
            false
        };
        ok |= this;
        if n == 0 {
            first_ok = this;
        }
    }
    if !ok {
        failures.push(format!(
            "E expected {} but got: {}",
            wanted.join(" or "),
            o.summary()
        ));
    }
    if !first_ok {
        // An alternative outcome was accepted; the modifiers describe the first one only.
        return;
    }
    for c in &ex.calls {
        if !o
            .exec_calls()
            .any(|x| format!("{}.{}", x.connector, x.action) == *c)
        {
            failures.push(format!("E expected a call to {c}"));
        }
    }
    for c in &ex.nocalls {
        if o.exec_calls()
            .any(|x| format!("{}.{}", x.connector, x.action) == *c)
        {
            failures.push(format!("E unexpected call to {c}"));
        }
    }
    for (k, must, text) in &ex.args {
        let found = o.exec_calls().any(|c| {
            c.args
                .iter()
                .any(|(n, v)| n == k && v.to_lowercase().contains(text))
        });
        if *must && !found {
            failures.push(format!("E no call had {k} containing \"{text}\""));
        }
        if !*must && found {
            failures.push(format!(
                "E a call had {k} containing \"{text}\" (should not)"
            ));
        }
    }
    for (must, text) in &ex.msg {
        let has = o.message.to_lowercase().contains(text);
        if *must && !has {
            failures.push(format!("E message lacks \"{text}\": \"{}\"", o.message));
        }
        if !*must && has {
            failures.push(format!("E message contains \"{text}\": \"{}\"", o.message));
        }
    }
    for t in &ex.ui {
        if !o.user_texts().iter().any(|x| x.to_lowercase().contains(t)) {
            failures.push(format!("E nothing shown contains \"{t}\""));
        }
    }
    for t in &ex.opt {
        let has = o.prompts.iter().any(|p| match p {
            Prompt::Choose { labels, .. } => labels.iter().any(|l| l.to_lowercase().contains(t)),
            _ => false,
        });
        if !has {
            failures.push(format!("E no quick-choice option contains \"{t}\""));
        }
    }
    for (must, text) in &ex.urls {
        let found = o.calls.iter().any(|c| {
            c.script
                && c.args.iter().any(|(k, v)| {
                    (k == "JITMO_URL" || k == "url")
                        && v.replace("%20", " ").to_lowercase().contains(text)
                })
        });
        if *must && !found {
            failures.push(format!("E no URL contained \"{text}\""));
        }
        if !*must && found {
            failures.push(format!("E a URL contained \"{text}\" (should not)"));
        }
    }
    if ex.noexec && executed {
        failures.push("E something executed".into());
    }
    if let Some(n) = ex.pending {
        if o.pending_reminders != n {
            failures.push(format!(
                "E {} reminders pending, expected {n}",
                o.pending_reminders
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------

pub struct Config {
    pub root: PathBuf,
    pub worker: Option<WorkerLauncher>,
    pub threads: usize,
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixed_now() -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .with_ymd_and_hms(2026, 11, 9, 10, 0, 0)
        .earliest()
        .expect("fixed time")
}

pub fn load_skills(dir: &Path) -> Result<Vec<Skill>, String> {
    crate::run::load_skills(dir)
}

fn new_router(skills: &[Skill], worker: &Option<WorkerLauncher>) -> Router {
    let r = Router::new(skills.to_vec());
    match worker {
        Some(l) => r.with_worker(WorkerClient::new(l.clone(), Duration::from_secs(15))),
        None => r,
    }
}

fn known_note() -> jitmo_ipc::ContextNote {
    Known {
        apps: APPS.iter().map(|s| s.to_string()).collect(),
        contacts: Vec::new(),
    }
    .note()
}

/// Everything one worker thread keeps between cases.
pub struct Lane {
    skills: Vec<Skill>,
    worker: Option<WorkerLauncher>,
    diag: Router,
}

impl Lane {
    pub fn new(skills: Vec<Skill>, worker: Option<WorkerLauncher>) -> Self {
        let diag = new_router(&skills, &worker);
        Lane {
            skills,
            worker,
            diag,
        }
    }

    pub fn run(&mut self, case: &Case) -> Observed {
        let ex = parse_expect(&case.expect);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("jitmo-audit-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let db = dir.join("jitmo.db");
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let checks = Arc::new(AtomicUsize::new(0));
        let ms = Arc::new(AtomicUsize::new(0));

        // Diagnostics: what the router thinks (same call the flow makes).
        let u = self
            .diag
            .route_with(case.input.split(" && ").last().unwrap_or(""), &known_note());
        let (label, score) = u
            .tasks
            .first()
            .map_or(("-".to_string(), 0.0), |t| (t.id.clone(), t.score));
        let decision = match decide(&u) {
            RouteDecision::Run { .. } => "RUN",
            RouteDecision::ConfirmTask { .. } => "CONFIRM",
            RouteDecision::Ask { .. } => "ASK",
            RouteDecision::Decline => "DECLINE",
            RouteDecision::Chained => "CHAINED",
        }
        .to_string();
        let spans: Vec<(String, String)> = u
            .spans
            .iter()
            .map(|s| (s.kind.clone(), s.text.clone()))
            .collect();

        // The world.
        let sim = Arc::new(Sim {
            log: log.clone(),
            flags: ex.flags.clone(),
            state: Mutex::new(SimState {
                vol: 40,
                muted: false,
                bright: 60,
                wifi: true,
                bt: false,
                dnd: false,
                media: "Paused",
            }),
            pictures: dir.join("Pictures"),
        });
        let now = fixed_now;
        let rec = |id: &str, inner: Option<Arc<dyn Connector>>| -> (String, Arc<dyn Connector>) {
            (
                id.to_string(),
                Arc::new(Rec {
                    id: id.to_string(),
                    inner,
                    stand_in: if id == "stub_messenger" {
                        FakeConnector::new(id).with_answer(
                            "This was a test: nothing was really sent. Real messaging is not built yet.",
                        )
                    } else {
                        FakeConnector::new(id)
                    },
                    log: log.clone(),
                    checks: checks.clone(),
                    ms: ms.clone(),
                }) as Arc<dyn Connector>,
            )
        };
        let place_db = db.clone();
        let web = jitmo_connectors::web_lookup::WebLookup::with_http(
            Box::new(StubHttp { log: log.clone() }),
            None,
        )
        .with_place_source(move || {
            Store::open(&place_db, None)
                .ok()
                .and_then(|s| s.get_setting("default_place").ok().flatten())
        })
        .with_clock(move || now().with_timezone(&chrono::Utc));
        let calc = jitmo_connectors::local_calc::LocalCalc::with_now(move || {
            now().with_timezone(&chrono::Utc)
        });
        let connectors: std::collections::HashMap<String, Arc<dyn Connector>> = [
            rec("windows_system", Some(Arc::new(AppsSim))),
            rec(
                "files",
                Some(Arc::new(FilesSim(
                    jitmo_connectors::files::Files::new(dir.join("Downloads")).with_now(now),
                ))),
            ),
            rec("stub_messenger", None),
            rec("whatsapp_desktop", None),
            rec("web_lookup", Some(Arc::new(web))),
            rec("local_calc", Some(Arc::new(calc))),
            rec(
                "browser",
                Some(Arc::new(jitmo_connectors::browser::Browser::new(
                    sim.clone(),
                ))),
            ),
            rec(
                "spotify",
                Some(Arc::new(jitmo_connectors::spotify::Spotify::new(
                    sim.clone(),
                ))),
            ),
            rec(
                "dnd",
                Some(Arc::new(
                    jitmo_connectors::dnd::Dnd::new(sim.clone()).with_clock(now),
                )),
            ),
            rec(
                "system_controls",
                Some(Arc::new(
                    jitmo_connectors::system_controls::SystemControls::new(
                        sim.clone(),
                        dir.join("Pictures"),
                    ),
                )),
            ),
            rec(
                "settings",
                Some(Arc::new(jitmo_connectors::settings::Settings::new(
                    db.clone(),
                ))),
            ),
            rec(
                "reminders",
                Some(Arc::new(
                    jitmo_connectors::reminders::Reminders::new(db.clone()).with_now(now),
                )),
            ),
        ]
        .into_iter()
        .collect();

        let store = Store::open(&db, None).expect("store");
        if ex.flags.contains("city_pune") {
            let _ = store.set_setting("default_place", "Pune");
        }
        let router = new_router(&self.skills, &self.worker);
        let clock = now()
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let mut flow = Flow::new(router, store, connectors)
            .with_context(Fixture::new(&dir))
            .with_app_names(|| APPS.iter().map(|s| s.to_string()).collect())
            .with_clock(move || clock.clone());
        let mut user = User {
            prompts: Vec::new(),
            pick: ex.pick.clone(),
            say: ex.say.clone(),
        };
        let started = Instant::now();
        let parts: Vec<String> = case.input.split(" && ").map(str::to_string).collect();
        let event = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut last = None;
            for (i, part) in parts.iter().enumerate() {
                user.prompts.clear();
                last = Some(flow.handle(1 + i as u64, part, &mut user));
            }
            last.expect("at least one request")
        }));
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let (done, message, undoable) = match event {
            Ok(CoreEvent::Done {
                message, undoable, ..
            }) => (true, message, undoable),
            Ok(CoreEvent::Failed { message, .. }) => (false, message, false),
            Ok(other) => (false, format!("{other:?}"), false),
            Err(_) => (false, "PANIC: the engine crashed".to_string(), false),
        };
        let logged_skill = flow
            .store()
            .recent_actions(1)
            .ok()
            .and_then(|mut v| v.pop())
            .and_then(|l| l.skill_id);
        let undo_message = (ex.undo && undoable).then(|| flow.undo());
        let pending_reminders = flow
            .store()
            .pending_reminders()
            .map(|v| v.len())
            .unwrap_or(0);
        let calls = log.lock().unwrap().clone();
        drop(flow);
        let _ = std::fs::remove_dir_all(&dir);

        let mut o = Observed {
            case: case.clone(),
            label,
            score,
            out_of_scope: u.out_of_scope,
            decision,
            spans,
            prompts: user.prompts,
            calls,
            done,
            message,
            undoable,
            logged_skill,
            undo_message,
            pending_reminders,
            checks: checks.load(Ordering::SeqCst),
            elapsed_ms,
            connector_ms: ms.load(Ordering::SeqCst) as u64,
            failures: Vec::new(),
        };
        let mut failures = Vec::new();
        if o.message.starts_with("PANIC") {
            failures.push("G0 the engine panicked".into());
        }
        check_expect(&o, &ex, &mut failures);
        let examples: HashSet<String> = self
            .skills
            .iter()
            .flat_map(|s| s.examples.iter())
            .map(|e| e.trim().trim_end_matches(['?', '.']).to_lowercase())
            .collect();
        global_rules(&o, &examples, &mut failures);
        o.failures = failures;
        o
    }
}

/// Run every case on `threads` lanes. Results come back in corpus order.
pub fn run_all(cfg: &Config, skills: &[Skill], cases: &[Case]) -> Vec<Observed> {
    let next = Arc::new(AtomicUsize::new(0));
    let results: Arc<Mutex<Vec<Option<Observed>>>> =
        Arc::new(Mutex::new((0..cases.len()).map(|_| None).collect()));
    std::thread::scope(|scope| {
        for _ in 0..cfg.threads.max(1) {
            let next = next.clone();
            let results = results.clone();
            scope.spawn(move || {
                let mut lane = Lane::new(skills.to_vec(), cfg.worker.clone());
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(case) = cases.get(i) else { break };
                    let o = lane.run(case);
                    results.lock().unwrap()[i] = Some(o);
                }
            });
        }
    });
    let v = results.lock().unwrap().drain(..).flatten().collect();
    v
}

// ---------------------------------------------------------------------------
// Static lints of the skill files
// ---------------------------------------------------------------------------

pub fn lint_skills(skills: &[Skill]) -> Vec<String> {
    let mut out = Vec::new();
    for s in skills {
        let first = s.examples.first().cloned().unwrap_or_default();
        if s.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
            out.push(format!(
                "L1 {}: no `title:`, so a route confirm or choice would show the example \"{first}\" (raw placeholders included)",
                s.id
            ));
        }
        let uses_request = s
            .steps
            .iter()
            .any(|st| st.args.values().any(|v| v.contains("{request}")));
        if uses_request && s.slots.is_empty() {
            out.push(format!(
                "L2 {}: passes the whole sentence as {{request}} and declares no slots, so nothing is validated or asked before it runs",
                s.id
            ));
        }
        if let (Some(c), Some(st)) = (&s.check, s.steps.last()) {
            // These connectors' `check()` only re-parses the request (browser, weather/fact, calculator):
            // it cannot fail after the action worked, and it cannot catch an action that did not.
            let weak = ["browser", "web_lookup", "local_calc"];
            if c.connector == st.connector
                && c.action == st.action
                && weak.contains(&c.connector.as_str())
            {
                out.push(format!(
                    "L3 {}: the check ({}.{}) only re-parses the request; it never looks at the world, so it proves nothing",
                    s.id, c.connector, c.action
                ));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

pub struct Tally {
    pub pass: usize,
    pub fail: usize,
}

pub fn per_skill(results: &[Observed]) -> BTreeMap<String, BTreeMap<String, Tally>> {
    let mut m: BTreeMap<String, BTreeMap<String, Tally>> = BTreeMap::new();
    for o in results {
        let t = m
            .entry(o.case.skill.clone())
            .or_default()
            .entry(o.case.kind.clone())
            .or_insert(Tally { pass: 0, fail: 0 });
        if o.passed() {
            t.pass += 1;
        } else {
            t.fail += 1;
        }
    }
    m
}

pub const KINDS: &[&str] = &[
    "clean",
    "typo",
    "partial",
    "slang",
    "ambiguous",
    "adversarial",
];

pub fn render_table(results: &[Observed]) -> String {
    let m = per_skill(results);
    let mut s = String::new();
    s.push_str(&format!(
        "{:<22} {:>5} {:>5} {:>5}   {}\n",
        "skill", "total", "pass", "fail", "per kind (pass/total)"
    ));
    let (mut tp, mut tf) = (0, 0);
    for (skill, kinds) in &m {
        let pass: usize = kinds.values().map(|t| t.pass).sum();
        let fail: usize = kinds.values().map(|t| t.fail).sum();
        tp += pass;
        tf += fail;
        let per: Vec<String> = KINDS
            .iter()
            .filter_map(|k| {
                kinds
                    .get(*k)
                    .map(|t| format!("{k} {}/{}", t.pass, t.pass + t.fail))
            })
            .collect();
        s.push_str(&format!(
            "{:<22} {:>5} {:>5} {:>5}   {}\n",
            skill,
            pass + fail,
            pass,
            fail,
            per.join(", ")
        ));
    }
    s.push_str(&format!(
        "{:<22} {:>5} {:>5} {:>5}\n",
        "ALL",
        tp + tf,
        tp,
        tf
    ));
    s
}

pub fn render_failures(results: &[Observed]) -> String {
    let mut s = String::new();
    let mut last = String::new();
    for o in results.iter().filter(|o| !o.passed()) {
        if o.case.skill != last {
            s.push_str(&format!("\n=== {} ===\n", o.case.skill));
            last = o.case.skill.clone();
        }
        s.push_str(&format!(
            "FAIL [{}] (line {}) {:?}\n  expected: {}   {}\n  observed: {}\n",
            o.case.kind,
            o.case.line,
            o.case.input,
            o.case.expect,
            if o.case.note.is_empty() {
                String::new()
            } else {
                format!("// {}", o.case.note)
            },
            o.summary()
        ));
        for f in &o.failures {
            if !f.starts_with("E expected") {
                s.push_str(&format!("  rule: {f}\n"));
            }
        }
    }
    s
}

/// Every case, passing or not, as Markdown (the appendix of the audit report).
pub fn render_markdown(results: &[Observed]) -> String {
    let mut s = String::from("| skill | kind | input | router | decision | what happens | verdict |\n|---|---|---|---|---|---|---|\n");
    let esc = |t: &str| t.replace('|', "\\|").replace('\n', " ");
    for o in results {
        let what = o
            .summary()
            .split("  ;  ")
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ; ");
        s.push_str(&format!(
            "| {} | {} | `{}` | {}@{:.2} | {} | {} | {} |\n",
            o.case.skill,
            o.case.kind,
            esc(&o.case.input),
            o.label,
            o.score,
            o.decision,
            esc(&what),
            if o.passed() {
                "pass".to_string()
            } else {
                format!("FAIL: {}", esc(&o.failures.join(" / ")))
            }
        ));
    }
    s
}

// ---------------------------------------------------------------------------
// Stand-ins that keep their promises without touching the PC
// ---------------------------------------------------------------------------

/// Opens an app only if it is on the fixture's installed list (as the real lookup would).
struct AppsSim;

impl Connector for AppsSim {
    fn id(&self) -> &str {
        "windows_system"
    }
    fn actions(&self) -> Vec<jitmo_connectors::ActionSpec> {
        Vec::new()
    }
    fn health(&self) -> Health {
        Health::Ready
    }
    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let wanted = args
            .get("app")
            .ok_or_else(|| ConnError::MissingArg("app".into()))?;
        if action != "open_app" {
            return Err(ConnError::UnknownAction(action.into()));
        }
        let norm = |s: &str| s.to_lowercase().replace([' ', '-'], "");
        match APPS.iter().find(|a| norm(a) == norm(wanted)) {
            Some(a) => Ok(Output([("app".to_string(), a.to_string())].into())),
            None => Err(ConnError::NotInstalled(wanted.clone())),
        }
    }
    fn check(&self, _: &str, _: &Args) -> Result<bool, ConnError> {
        Ok(true)
    }
    fn undo(&self, _: &str, _: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

/// The real files connector, except that revealing/opening only checks the file exists
/// (no Explorer window, no default app is launched).
struct FilesSim(jitmo_connectors::files::Files);

impl Connector for FilesSim {
    fn id(&self) -> &str {
        "files"
    }
    fn actions(&self) -> Vec<jitmo_connectors::ActionSpec> {
        self.0.actions()
    }
    fn health(&self) -> Health {
        Health::Ready
    }
    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        match action {
            "reveal" | "open" => {
                let path = args
                    .get("path")
                    .ok_or_else(|| ConnError::MissingArg("path".into()))?;
                let name = Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !Path::new(path).exists() {
                    return Err(ConnError::Failed(format!("I can't find {name} any more.")));
                }
                let m = if action == "open" {
                    format!("Opened {name}.")
                } else {
                    format!("Found {name}.")
                };
                Ok(Output([("message".to_string(), m)].into()))
            }
            _ => self.0.execute(action, args),
        }
    }
    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        self.0.check(action, args)
    }
    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        self.0.undo(action, output)
    }
}

/// How often each global rule (G..) and expectation (E) fired, with one example input each.
pub fn render_rule_hits(results: &[Observed]) -> String {
    let mut hits: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for o in results {
        for f in &o.failures {
            let code = f.split_whitespace().next().unwrap_or("?").to_string();
            let e = hits
                .entry(code)
                .or_insert((0, o.case.input.chars().take(60).collect()));
            e.0 += 1;
        }
    }
    let names = [
        ("E", "expectation not met"),
        ("G0", "engine panicked"),
        ("G1", "example word the user never typed shown to the user"),
        ("G2", "raw {placeholder} shown to the user"),
        (
            "G3",
            "a skill example sentence shown instead of the request",
        ),
        ("G4", "result empty or not one short line"),
        ("G5", "raw system text in a result"),
        ("G9", "action ran but the user is told it failed"),
        ("G10", "question delivered as a failure"),
        ("G11", "bare Done. for a reminder"),
        ("G12", "connectors blocked the core thread over 3 s"),
    ];
    let mut s = String::from("Rule hits (cases that tripped each rule)\n");
    for (code, label) in names {
        if let Some((n, example)) = hits.get(code) {
            s.push_str(&format!("  {code:<4} {n:>4}  {label}   e.g. {example:?}\n"));
        }
    }
    s
}
