//! Runs one test in a fresh setup and judges it by the rules in doc 04
//! ("How a test passes"). Every test starts from a clean state: a new
//! in-memory store and new connectors, so nothing carries over (the "reset").

use crate::spec::{CallSpec, Expect, Spec};
use jitmo_connectors::fake::{Call, FakeConnector};
use jitmo_connectors::Connector;
use jitmo_core::flow::{Flow, Prompter};
use jitmo_core::resolver::{ContextSource, FileRef};
use jitmo_core::router::Router;
use jitmo_ipc::{Choice, CoreEvent};
use jitmo_model_worker::{WorkerClient, WorkerLauncher};
use jitmo_skills::{Risk, Skill};
use jitmo_store::Store;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RunResult {
    pub passed: bool,
    /// Why it failed (empty when it passed).
    pub reasons: Vec<String>,
    /// The one-line result the engine reported.
    pub message: String,
    pub elapsed_ms: f64,
    /// Time from the start of the request until the first connector execute call
    /// began (None when nothing ran, or the connector was real and cannot be observed).
    pub time_to_first_action_ms: Option<f64>,
    /// A send ran although no confirm was shown (target: 0, RQ2).
    pub sent_without_confirm: bool,
    pub crashed: bool,
}

/// Load every `*.yaml` skill in a folder (no signature check: dev skills).
pub fn load_skills(dir: &Path) -> Result<Vec<Skill>, String> {
    let mut skills = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
    {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        skills.push(Skill::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    Ok(skills)
}

/// Stands in for the user: records what it was shown and answers from a script.
struct Recorder {
    answers: VecDeque<bool>,
    confirms: Vec<String>,
    asked_question: bool,
}

impl Prompter for Recorder {
    fn confirm(&mut self, text: &str) -> bool {
        self.confirms.push(text.to_string());
        self.answers.pop_front().unwrap_or(true)
    }
    fn choose(&mut self, _question: &str, _choices: &[Choice]) -> Option<String> {
        self.asked_question = true;
        None // dismiss: nothing may happen before an answer
    }
}

#[cfg(windows)]
fn live_connector(id: &str) -> Option<Arc<dyn Connector>> {
    (id == "windows_system")
        .then(|| Arc::new(jitmo_connectors::windows_system::WindowsSystem) as Arc<dyn Connector>)
}

#[cfg(not(windows))]
fn live_connector(_id: &str) -> Option<Arc<dyn Connector>> {
    None
}

fn one_line(s: &str) -> bool {
    !s.trim().is_empty() && !s.contains('\n')
}

/// Run one test from a clean state.
pub fn run_case(skills: &[Skill], spec: &Spec, live: bool) -> RunResult {
    run_case_with(skills, spec, live, None)
}

/// Files that exist only for one test (`files:` in the spec), newest last.
struct TestFiles {
    files: Vec<FileRef>,
    selected: Vec<FileRef>,
    folders: Vec<FileRef>,
}

impl TestFiles {
    fn new(spec: &Spec) -> Self {
        let make = |names: &[String]| -> Vec<FileRef> {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| FileRef {
                    name: n.clone(),
                    path: format!("C:/Docs/{n}"),
                    modified: 1000 + i as u64,
                })
                .collect()
        };
        TestFiles {
            files: make(&spec.files),
            selected: make(&spec.selected),
            folders: make(&spec.folders),
        }
    }
}

impl ContextSource for TestFiles {
    fn selected_file(&self) -> Option<FileRef> {
        self.selected.first().cloned()
    }
    fn selected_files(&self) -> Vec<FileRef> {
        self.selected.clone()
    }
    fn searchable_folders(&self) -> Vec<FileRef> {
        self.folders.clone()
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
}

/// Like [`run_case`], with the real model worker doing the routing when given.
pub fn run_case_with(
    skills: &[Skill],
    spec: &Spec,
    live: bool,
    worker: Option<&WorkerLauncher>,
) -> RunResult {
    let started = Instant::now();

    // Fresh connectors: fakes, except those the test asks to be real in --live mode.
    let mut ids = BTreeSet::new();
    for s in skills {
        ids.extend(s.steps.iter().map(|st| st.connector.clone()));
        ids.extend(s.check.iter().map(|c| c.connector.clone()));
    }
    let mut map: HashMap<String, Arc<dyn Connector>> = HashMap::new();
    let mut fakes: Vec<(String, Arc<FakeConnector>)> = Vec::new();
    for id in ids {
        // Pure and offline, so it is always the real one.
        if id == "local_calc" {
            map.insert(id, Arc::new(jitmo_connectors::local_calc::LocalCalc::new()));
            continue;
        }
        if live && spec.live.contains(&id) {
            if let Some(real) = live_connector(&id) {
                map.insert(id, real);
                continue;
            }
        }
        let fake = Arc::new(FakeConnector::new(&id));
        map.insert(id.clone(), fake.clone());
        fakes.push((id, fake));
    }

    let store = Store::open_in_memory().expect("in-memory store");
    for c in &spec.contacts {
        store
            .upsert_contact(&c.name, c.phone.as_deref(), "manual")
            .expect("save a test contact");
    }
    let mut router = Router::new(skills.to_vec());
    if let Some(launcher) = worker {
        router = router.with_worker(WorkerClient::new(launcher.clone(), Duration::from_secs(10)));
    }
    let apps = spec.apps.clone();
    let mut flow = Flow::new(router, store, map)
        .with_context(TestFiles::new(spec))
        .with_app_names(move || apps.clone());
    let mut user = Recorder {
        answers: spec.confirm_answers.iter().copied().collect(),
        confirms: Vec::new(),
        asked_question: false,
    };

    let handle_started = Instant::now();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        flow.handle(1, &spec.request, &mut user)
    }));
    let time_to_first_action_ms = fakes
        .iter()
        .filter_map(|(_, f)| f.first_execute_at())
        .min()
        .map(|t| t.saturating_duration_since(handle_started).as_secs_f64() * 1000.0);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    let Ok(event) = outcome else {
        return RunResult {
            passed: false,
            reasons: vec!["the engine crashed".into()],
            message: String::new(),
            elapsed_ms,
            time_to_first_action_ms: None,
            sent_without_confirm: false,
            crashed: true,
        };
    };

    let (is_done, message) = match &event {
        CoreEvent::Done { message, .. } => (true, message.clone()),
        CoreEvent::Failed { message, .. } => (false, message.clone()),
        other => (false, format!("{other:?}")),
    };

    // What the connectors were asked to do.
    let executes: Vec<(String, String, jitmo_connectors::Args)> = fakes
        .iter()
        .flat_map(|(id, f)| {
            f.calls().into_iter().filter_map(|c| match c {
                Call::Execute { action, args } => Some((id.clone(), action, args)),
                _ => None,
            })
        })
        .collect();
    let checked = fakes.iter().any(|(_, f)| f.check_count() > 0);

    let logged = flow
        .store()
        .recent_actions(1)
        .ok()
        .and_then(|mut v| v.pop());
    let logged_skill = logged.as_ref().and_then(|l| l.skill_id.clone());
    let skill_risk = logged_skill
        .as_deref()
        .and_then(|id| skills.iter().find(|s| s.id == id))
        .map(|s| s.risk);
    let sent_without_confirm =
        skill_risk == Some(Risk::SendsOrShares) && !executes.is_empty() && user.confirms.is_empty();

    let mut why: Vec<String> = Vec::new();
    if !one_line(&message) {
        why.push("the result was not one short line".into());
    }
    if sent_without_confirm {
        why.push("a send ran without a confirm".into());
    }
    if let Some(needle) = &spec.confirm_contains {
        let needle = needle.to_lowercase();
        if !user
            .confirms
            .iter()
            .any(|c| c.to_lowercase().contains(&needle))
        {
            why.push(format!(
                "no confirm card mentioned `{needle}` (saw {:?})",
                user.confirms
            ));
        }
    }

    match spec.expect {
        Expect::Ran => {
            if !is_done || message == "Cancelled." {
                why.push(format!("expected the task to run, but got: {message}"));
            }
            match &spec.skill {
                None => why.push("the spec has no `skill` for a `ran` test".into()),
                Some(want) if logged_skill.as_ref() != Some(want) => {
                    why.push(format!("wrong task: expected {want}, got {logged_skill:?}"));
                }
                _ => {}
            }
            if let Some(part) = &spec.message_contains {
                if !message.to_lowercase().contains(&part.to_lowercase()) {
                    why.push(format!(
                        "the answer should mention `{part}`, but was: {message}"
                    ));
                }
            }
            check_calls(&spec.calls, &executes, &fakes, &mut why);
            let wants_check = logged_skill
                .as_deref()
                .and_then(|id| skills.iter().find(|s| s.id == id))
                .is_some_and(|s| s.check.is_some());
            let check_is_fake = fakes.iter().any(|(id, _)| {
                logged_skill
                    .as_deref()
                    .and_then(|sid| skills.iter().find(|s| s.id == sid))
                    .and_then(|s| s.check.as_ref())
                    .is_some_and(|c| &c.connector == id)
            });
            if wants_check && check_is_fake && !checked {
                why.push("the result was never checked".into());
            }
            if skill_risk == Some(Risk::SendsOrShares) && user.confirms.is_empty() {
                why.push("no confirm was shown before a send".into());
            }
        }
        Expect::Declined => {
            if is_done {
                why.push(format!("expected a clear decline, but got: {message}"));
            }
            if !executes.is_empty() || !user.confirms.is_empty() || user.asked_question {
                why.push("something happened although the request should be declined".into());
            }
        }
        Expect::Cancelled => {
            if user.confirms.is_empty() {
                why.push("no confirm was shown".into());
            }
            if message != "Cancelled." {
                why.push(format!("expected Cancelled., got: {message}"));
            }
            if !executes.is_empty() {
                why.push("something ran after Cancel".into());
            }
        }
        Expect::AsksQuestion => {
            if !user.asked_question {
                why.push("no question was asked".into());
            }
            if !executes.is_empty() {
                why.push("something ran before the question was answered".into());
            }
        }
    }

    RunResult {
        passed: why.is_empty(),
        reasons: why,
        message,
        elapsed_ms,
        time_to_first_action_ms,
        sent_without_confirm,
        crashed: false,
    }
}

/// Every expected call must have happened, in order, with the expected arguments.
/// Calls on a connector that is real in this run cannot be observed and are skipped.
fn check_calls(
    wanted: &[CallSpec],
    executes: &[(String, String, jitmo_connectors::Args)],
    fakes: &[(String, Arc<FakeConnector>)],
    why: &mut Vec<String>,
) {
    let mut cursor: HashMap<&str, usize> = HashMap::new();
    for w in wanted {
        if !fakes.iter().any(|(id, _)| id == &w.connector) {
            continue; // real connector: not observable
        }
        let seen: Vec<_> = executes
            .iter()
            .filter(|(id, _, _)| id == &w.connector)
            .collect();
        let at = cursor.entry(w.connector.as_str()).or_insert(0);
        match seen.get(*at) {
            None => why.push(format!("missing call {}.{}", w.connector, w.action)),
            Some((_, action, args)) => {
                if action != &w.action {
                    why.push(format!(
                        "expected {}.{}, got {}",
                        w.connector, w.action, action
                    ));
                }
                for (k, v) in &w.args {
                    if args.get(k) != Some(v) {
                        why.push(format!(
                            "{}.{}: expected {k}={v}, got {:?}",
                            w.connector,
                            w.action,
                            args.get(k)
                        ));
                    }
                }
            }
        }
        *at += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec;

    fn skills() -> Vec<Skill> {
        load_skills(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills")).unwrap()
    }

    fn specs() -> std::collections::BTreeMap<String, Spec> {
        spec::load(include_str!("../../../testset/automated.yaml")).unwrap()
    }

    #[test]
    fn t01_passes_with_fake_connectors() {
        let r = run_case(&skills(), &specs()["T01"], false);
        assert!(r.passed, "{:?}", r.reasons);
        assert_eq!(r.message, "Done.");
    }

    #[test]
    fn t95_cancel_passes_and_nothing_ran() {
        let r = run_case(&skills(), &specs()["T95"], false);
        assert!(r.passed, "{:?}", r.reasons);
        assert!(!r.sent_without_confirm);
    }

    #[test]
    fn all_out_of_scope_tests_pass() {
        for (id, s) in specs()
            .iter()
            .filter(|(id, _)| ("T83"..="T90").contains(&id.as_str()))
        {
            let r = run_case(&skills(), s, false);
            assert!(r.passed, "{id}: {:?}", r.reasons);
        }
    }

    #[test]
    fn t96_a_send_to_everyone_is_declined_by_the_recipient_limit() {
        let r = run_case(&skills(), &specs()["T96"], false);
        assert!(r.passed, "{:?}", r.reasons);
        assert_eq!(r.message, "I can't send to more than 10 people at once.");
    }

    #[test]
    fn the_wrong_skill_is_a_failure_with_a_reason() {
        let mut s = specs()["T01"].clone();
        s.skill = Some("music.control".into());
        let r = run_case(&skills(), &s, false);
        assert!(!r.passed);
        assert!(
            r.reasons.iter().any(|m| m.contains("wrong task")),
            "{:?}",
            r.reasons
        );
    }

    #[test]
    fn wrong_arguments_are_a_failure() {
        let mut s = specs()["T01"].clone();
        s.calls[0].args.insert("app".into(), "notepad".into());
        let r = run_case(&skills(), &s, false);
        assert!(!r.passed);
        assert!(
            r.reasons.iter().any(|m| m.contains("expected app=notepad")),
            "{:?}",
            r.reasons
        );
    }

    #[test]
    fn expecting_a_decline_when_the_task_runs_is_a_failure() {
        let mut s = specs()["T01"].clone();
        s.expect = Expect::Declined;
        s.calls.clear();
        let r = run_case(&skills(), &s, false);
        assert!(!r.passed);
    }

    #[test]
    fn each_test_starts_clean() {
        // Two runs in a row see the same result: nothing carries over.
        let a = run_case(&skills(), &specs()["T01"], false);
        let b = run_case(&skills(), &specs()["T01"], false);
        assert_eq!((a.passed, a.message), (b.passed, b.message));
    }
}
