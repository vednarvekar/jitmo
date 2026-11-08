//! The flow: one request from text to a one-line result.
//!
//! route -> (ask / confirm the task) -> fill slots -> policy -> health ->
//! run steps -> verify -> log. Every request ends in exactly one
//! [`CoreEvent::Done`] or [`CoreEvent::Failed`]; nothing fails silently.
//!
//! Phase 1 uses the stub router and a simple slot filler (spans by type). The
//! real resolver and models replace those in Phase 2 without changing this
//! shape.
//!
//! The flow is synchronous. Anything that needs the user goes through
//! [`Prompter`], which the service layer implements over the UI pipe.

use crate::policy::{self, Outcome, StepFacts};
use crate::router::{decide, RouteDecision, StubRouter};
use crate::verifier::{verify, Verdict};
use jitmo_connectors::{Args, ConnError, Connector, Health, Output};
use jitmo_ipc::{Choice, CoreEvent, RequestId, Span};
use jitmo_skills::{Risk, Skill, Slot, Undo};
use jitmo_store::{Decision, Store, StoreError};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// How the flow talks to the user mid-request.
pub trait Prompter {
    /// Show a confirm card. `true` means approved; `false` means Cancel.
    fn confirm(&mut self, text: &str) -> bool;
    /// Ask one question. `None` means the user dismissed it.
    fn choose(&mut self, question: &str, choices: &[Choice]) -> Option<String>;
}

struct UndoRecord {
    log_id: i64,
    connector: String,
    action: String,
    output: Output,
}

pub struct Flow {
    router: StubRouter,
    store: Store,
    connectors: HashMap<String, Arc<dyn Connector>>,
    clock: Box<dyn Fn() -> String + Send>,
    last_undo: Option<UndoRecord>,
}

impl Flow {
    pub fn new(router: StubRouter, store: Store, connectors: HashMap<String, Arc<dyn Connector>>) -> Self {
        Flow { router, store, connectors, clock: Box::new(iso_now), last_undo: None }
    }

    /// Replace the clock (tests).
    pub fn with_clock(mut self, clock: impl Fn() -> String + Send + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Handle one request and return its final event.
    pub fn handle(&mut self, id: RequestId, text: &str, prompter: &mut dyn Prompter) -> CoreEvent {
        match self.run(id, text, prompter) {
            Ok(event) => event,
            Err(_) => CoreEvent::Failed { id, message: "Something went wrong.".into() },
        }
    }

    /// Undo the last undoable action, if any. Returns the one-line result.
    pub fn undo(&mut self) -> String {
        let Some(rec) = self.last_undo.take() else {
            return "Nothing to undo.".into();
        };
        let Some(conn) = self.connectors.get(&rec.connector).cloned() else {
            return "That can't be undone.".into();
        };
        match conn.undo(&rec.action, &rec.output) {
            None => "That can't be undone.".into(),
            Some(Err(e)) => e.to_string(),
            Some(Ok(())) => {
                let _ = self.store.clear_undo_token(rec.log_id);
                "Undone.".into()
            }
        }
    }

    fn log(
        &self,
        request: &str,
        skill_id: Option<&str>,
        decision: Decision,
        result: &str,
        undo_token: Option<&str>,
    ) -> Result<i64, StoreError> {
        self.store
            .log_action(&(self.clock)(), request, skill_id, decision, Some(result), undo_token)
    }

    fn fail(&self, id: RequestId, request: &str, skill_id: Option<&str>, message: &str) -> Result<CoreEvent, StoreError> {
        self.log(request, skill_id, Decision::Failed, message, None)?;
        Ok(CoreEvent::Failed { id, message: message.into() })
    }

    fn cancelled(&self, id: RequestId, request: &str, skill_id: Option<&str>) -> Result<CoreEvent, StoreError> {
        self.log(request, skill_id, Decision::Cancelled, "Cancelled.", None)?;
        Ok(CoreEvent::Done { id, message: "Cancelled.".into(), undoable: false })
    }

    fn run(&mut self, id: RequestId, text: &str, p: &mut dyn Prompter) -> Result<CoreEvent, StoreError> {
        // 1. Route.
        let understanding = self.router.route(text);
        let skill_id = match decide(&understanding) {
            RouteDecision::Decline => return self.fail(id, text, None, "I can't do that yet."),
            RouteDecision::Chained => {
                return self.fail(id, text, None, "I can't plan that right now. Try a simpler request.")
            }
            RouteDecision::Run { skill_id } => skill_id,
            RouteDecision::ConfirmTask { skill_id } => {
                if !p.confirm(&format!("Did you mean: {}?", self.describe(&skill_id))) {
                    return self.cancelled(id, text, Some(&skill_id));
                }
                skill_id
            }
            RouteDecision::Ask { skill_ids } => {
                let choices: Vec<Choice> = skill_ids
                    .iter()
                    .map(|s| Choice { id: s.clone(), label: self.describe(s) })
                    .collect();
                match p.choose("What would you like to do?", &choices) {
                    Some(chosen) if skill_ids.contains(&chosen) => chosen,
                    _ => return self.cancelled(id, text, None),
                }
            }
        };
        let Some(skill) = self.router.skill(&skill_id).cloned() else {
            return self.fail(id, text, Some(&skill_id), "Something went wrong.");
        };

        // 2. Fill slots.
        let slots = match resolve_slots(&skill, &understanding.spans) {
            Ok(s) => s,
            Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
        };

        // 3. Policy.
        let recipients = u32::from(skill.risk == Risk::SendsOrShares);
        let facts = StepFacts::from_skill(&skill, false, recipients);
        let outcome = policy::evaluate_and_log(&self.store, &(self.clock)(), text, &facts)?;
        let mut confirmed = false;
        match outcome {
            Outcome::Block { reason, .. } => {
                return Ok(CoreEvent::Failed { id, message: reason.message().into() });
            }
            Outcome::Confirm => {
                let template = skill.confirm.as_ref().map_or("Go ahead?", |c| c.text.as_str());
                let card = match fill(template, &slots) {
                    Ok(t) => t,
                    Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
                };
                if !p.confirm(&card) {
                    return self.cancelled(id, text, Some(&skill.id));
                }
                confirmed = true;
            }
            Outcome::RunThenUndo { .. } | Outcome::Run => {}
        }

        // 4. Run the steps.
        let mut last_output: Option<(String, String, Output)> = None;
        for step in &skill.steps {
            let Some(conn) = self.connectors.get(&step.connector).cloned() else {
                return self.fail(id, text, Some(&skill.id), &format!("{} isn't available.", step.connector));
            };
            if let Some(err) = health_error(conn.as_ref(), &step.connector) {
                return self.fail(id, text, Some(&skill.id), &err.to_string());
            }
            let args = match fill_args(&step.args, &slots) {
                Ok(a) => a,
                Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
            };
            match conn.execute(&step.action, &args) {
                Ok(out) => last_output = Some((step.connector.clone(), step.action.clone(), out)),
                Err(e) => return self.fail(id, text, Some(&skill.id), &e.to_string()),
            }
        }

        // 5. Verify.
        if let Some(check) = &skill.check {
            let Some(conn) = self.connectors.get(&check.connector).cloned() else {
                return self.fail(id, text, Some(&skill.id), &format!("{} isn't available.", check.connector));
            };
            let args = match fill_args(&check.args, &slots) {
                Ok(a) => a,
                Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
            };
            let msg = format!("I couldn't confirm that it worked. Please check {}.", check.connector);
            if let Verdict::NotVerified { message } = verify(conn.as_ref(), &check.action, &args, &msg) {
                return self.fail(id, text, Some(&skill.id), &message);
            }
        }

        // 6. Log and report.
        let can_undo = matches!(outcome, Outcome::RunThenUndo { .. })
            && matches!(skill.undo, Undo::Steps(_))
            && last_output.is_some();
        let decision = if confirmed { Decision::Confirmed } else { Decision::Ran };
        let log_id = self.log(text, Some(&skill.id), decision, "Done.", can_undo.then_some("available"))?;
        self.last_undo = match (can_undo, last_output) {
            (true, Some((connector, action, output))) => Some(UndoRecord { log_id, connector, action, output }),
            _ => None,
        };
        Ok(CoreEvent::Done { id, message: "Done.".into(), undoable: can_undo })
    }

    /// A short human label for a skill, used in questions.
    fn describe(&self, skill_id: &str) -> String {
        self.router
            .skill(skill_id)
            .and_then(|s| s.examples.first().cloned())
            .unwrap_or_else(|| skill_id.to_string())
    }
}

/// Fill each slot from the spans the router found (first unused span of the
/// slot's type), or from the skill's fixed value.
fn resolve_slots(skill: &Skill, spans: &[Span]) -> Result<BTreeMap<String, String>, String> {
    let mut used = vec![false; spans.len()];
    let mut out = BTreeMap::new();
    for (name, slot) in &skill.slots {
        match slot {
            Slot::Fixed(v) => {
                out.insert(name.clone(), v.clone());
            }
            Slot::Param(sp) => {
                let found = spans
                    .iter()
                    .enumerate()
                    .find(|(i, s)| !used[*i] && s.kind == sp.kind);
                match found {
                    Some((i, s)) => {
                        used[i] = true;
                        out.insert(name.clone(), s.text.clone());
                    }
                    None if sp.required => return Err(format!("I'm missing the {name}.")),
                    None => {}
                }
            }
        }
    }
    Ok(out)
}

/// Replace `{slot}` in a template. An unknown slot is an error, never a guess.
fn fill(template: &str, slots: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            return Ok(out);
        };
        let key = &after[..j];
        match slots.get(key) {
            Some(v) => out.push_str(v),
            None => return Err(format!("I'm missing the {key}.")),
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn fill_args(args: &BTreeMap<String, String>, slots: &BTreeMap<String, String>) -> Result<Args, String> {
    args.iter().map(|(k, v)| Ok((k.clone(), fill(v, slots)?))).collect()
}

fn health_error(conn: &dyn Connector, name: &str) -> Option<ConnError> {
    match conn.health() {
        Health::Ready => None,
        Health::NotInstalled => Some(ConnError::NotInstalled(name.into())),
        Health::NotSignedIn => Some(ConnError::NotSignedIn(name.into())),
        Health::Unreachable => Some(ConnError::Failed(format!("I can't reach {name} right now."))),
    }
}

/// Current UTC time as an ISO-8601 string.
pub fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso_from_secs(secs)
}

/// Unix seconds to `YYYY-MM-DDTHH:MM:SSZ` (days-to-date after Howard Hinnant).
pub fn iso_from_secs(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jitmo_connectors::fake::{Call, FakeConnector};

    const OPEN: &str = include_str!("../../../skills/apps.open.yaml");
    const PAUSE: &str = include_str!("../../../skills/music.pause.yaml");
    const SEND: &str = include_str!("../../../skills/stub.send.yaml");

    /// Scripted user: answers confirms from a list and records what it was shown.
    #[derive(Default)]
    struct TestUser {
        confirm_answers: Vec<bool>,
        shown: Vec<String>,
        choice: Option<String>,
        choices_seen: Vec<String>,
    }

    impl Prompter for TestUser {
        fn confirm(&mut self, text: &str) -> bool {
            self.shown.push(text.to_string());
            if self.confirm_answers.is_empty() { true } else { self.confirm_answers.remove(0) }
        }
        fn choose(&mut self, _q: &str, choices: &[Choice]) -> Option<String> {
            self.choices_seen = choices.iter().map(|c| c.id.clone()).collect();
            self.choice.clone()
        }
    }

    fn user(answers: &[bool]) -> TestUser {
        TestUser { confirm_answers: answers.to_vec(), ..TestUser::default() }
    }

    fn flow_with(yamls: &[&str], conns: Vec<(&str, Arc<FakeConnector>)>) -> Flow {
        let skills = yamls.iter().map(|y| Skill::parse(y).unwrap()).collect();
        let map = conns
            .into_iter()
            .map(|(k, v)| (k.to_string(), v as Arc<dyn Connector>))
            .collect();
        Flow::new(StubRouter::new(skills), Store::open_in_memory().unwrap(), map)
            .with_clock(|| "2026-11-08T12:00:00Z".into())
    }

    fn log_decisions(f: &Flow) -> Vec<String> {
        f.store().recent_actions(20).unwrap().into_iter().rev().map(|r| r.decision).collect()
    }

    fn done(ev: &CoreEvent) -> (&str, bool) {
        match ev {
            CoreEvent::Done { message, undoable, .. } => (message, *undoable),
            other => panic!("expected Done, got {other:?}"),
        }
    }

    fn failed(ev: &CoreEvent) -> &str {
        match ev {
            CoreEvent::Failed { message, .. } => message,
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // --- T01 ------------------------------------------------------------------

    #[test]
    fn t01_open_spotify_runs_end_to_end() {
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", sys.clone())]);
        let ev = flow.handle(1, "Open Spotify.", &mut user(&[]));
        assert_eq!(done(&ev), ("Done.", false));
        let calls = sys.calls();
        assert_eq!(
            calls[0],
            Call::Execute { action: "open_app".into(), args: Args::from([("app".into(), "spotify".into())]) }
        );
        assert!(matches!(&calls[1], Call::Check { action, .. } if action == "app_is_running"));
        assert_eq!(log_decisions(&flow), vec!["ran"]);
    }

    // --- confirm and cancel ---------------------------------------------------

    #[test]
    fn send_shows_its_confirm_and_runs_on_approve() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[SEND], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        let ev = flow.handle(2, "send hello world to ved", &mut u);
        assert_eq!(u.shown, vec!["Send \"hello world\" to ved?"]);
        assert_eq!(done(&ev).0, "Done.");
        assert_eq!(m.execute_count(), 1);
        assert_eq!(log_decisions(&flow), vec!["confirmed"]);
    }

    #[test]
    fn cancel_on_the_confirm_does_nothing() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[SEND], vec![("stub_messenger", m.clone())]);
        let ev = flow.handle(3, "send hello to ved", &mut user(&[false]));
        assert_eq!(done(&ev).0, "Cancelled.");
        assert!(m.calls().is_empty(), "nothing may be called after Cancel");
        assert_eq!(log_decisions(&flow), vec!["cancelled"]);
    }

    // --- policy ---------------------------------------------------------------

    #[test]
    fn permanent_delete_is_blocked_and_logged() {
        let yaml = OPEN
            .replace("id: apps.open", "id: files.erase")
            .replace("- apps.launch", "- files.delete:permanent")
            .replace("open {app}", "erase {app}");
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[&yaml], vec![("windows_system", sys.clone())]);
        let ev = flow.handle(4, "erase notes", &mut user(&[]));
        assert_eq!(failed(&ev), "I can't permanently delete things.");
        assert!(sys.calls().is_empty());
        assert_eq!(log_decisions(&flow), vec!["blocked"]);
    }

    // --- failures --------------------------------------------------------------

    #[test]
    fn failed_check_is_reported_after_one_retry() {
        let sys = Arc::new(FakeConnector::new("windows_system").default_check(false));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", sys.clone())]);
        let ev = flow.handle(5, "open spotify", &mut user(&[]));
        assert!(failed(&ev).starts_with("I couldn't confirm that it worked."));
        assert_eq!(sys.check_count(), 2);
        assert_eq!(sys.execute_count(), 1, "the action is never re-run");
        assert_eq!(log_decisions(&flow), vec!["failed"]);
    }

    #[test]
    fn app_not_installed_stops_before_running() {
        let sys = Arc::new(FakeConnector::new("windows_system").with_health(Health::NotInstalled));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", sys.clone())]);
        let ev = flow.handle(6, "open spotify", &mut user(&[]));
        assert!(failed(&ev).contains("isn't installed"));
        assert_eq!(sys.execute_count(), 0);
    }

    #[test]
    fn connector_error_becomes_a_one_line_message() {
        let sys = Arc::new(
            FakeConnector::new("windows_system").fail_execute_with(ConnError::Failed("Couldn't open it.".into())),
        );
        let mut flow = flow_with(&[OPEN], vec![("windows_system", sys)]);
        assert_eq!(failed(&flow.handle(7, "open spotify", &mut user(&[]))), "Couldn't open it.");
    }

    #[test]
    fn missing_connector_is_reported_not_a_panic() {
        let mut flow = flow_with(&[OPEN], vec![]);
        assert!(failed(&flow.handle(8, "open spotify", &mut user(&[]))).contains("isn't available"));
    }

    #[test]
    fn out_of_scope_and_chained_requests_are_declined_with_a_message() {
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[OPEN, PAUSE], vec![("windows_system", sys.clone())]);
        assert_eq!(failed(&flow.handle(9, "what is the capital of france", &mut user(&[]))), "I can't do that yet.");
        assert!(failed(&flow.handle(10, "open spotify and then pause the music", &mut user(&[]))).contains("simpler request"));
        assert!(sys.calls().is_empty());
    }

    // --- confirm the task / ask ------------------------------------------------

    #[test]
    fn medium_confidence_asks_to_confirm_the_task() {
        let sp = Arc::new(FakeConnector::new("spotify"));
        let mut flow = flow_with(&[PAUSE], vec![("spotify", sp.clone())]);
        let mut no = user(&[false]);
        assert_eq!(done(&flow.handle(11, "the music pause", &mut no)).0, "Cancelled.");
        assert!(no.shown[0].starts_with("Did you mean:"));
        assert_eq!(sp.execute_count(), 0);
        let mut yes = user(&[true]);
        assert_eq!(done(&flow.handle(12, "the music pause", &mut yes)).0, "Done.");
        assert_eq!(sp.execute_count(), 1);
    }

    #[test]
    fn low_confidence_asks_a_question_and_runs_the_choice() {
        let sp = Arc::new(FakeConnector::new("spotify"));
        let mut flow = flow_with(&[PAUSE], vec![("spotify", sp.clone())]);
        let mut u = TestUser { choice: Some("music.pause".into()), ..TestUser::default() };
        assert_eq!(done(&flow.handle(13, "pause music", &mut u)).0, "Done.");
        assert_eq!(u.choices_seen, vec!["music.pause"]);
        assert_eq!(sp.execute_count(), 1);
        // Dismissing the question does nothing.
        let mut dismiss = TestUser::default();
        assert_eq!(done(&flow.handle(14, "pause music", &mut dismiss)).0, "Cancelled.");
        assert_eq!(sp.execute_count(), 1);
    }

    // --- undo --------------------------------------------------------------------

    fn undoable_open() -> String {
        OPEN.replace("risk: safe", "risk: changes_data")
            .replace("undo: none", "undo:\n  - connector: windows_system\n    action: close_app\n")
    }

    #[test]
    fn data_change_offers_undo_and_undo_works_once() {
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let mut flow = flow_with(&[&undoable_open()], vec![("windows_system", sys.clone())]);
        let ev = flow.handle(15, "open spotify", &mut user(&[]));
        assert_eq!(done(&ev), ("Done.", true));
        assert_eq!(flow.store().recent_actions(1).unwrap()[0].undo_token.as_deref(), Some("available"));
        assert_eq!(flow.undo(), "Undone.");
        assert_eq!(flow.store().recent_actions(1).unwrap()[0].undo_token, None);
        assert_eq!(flow.undo(), "Nothing to undo.");
    }

    #[test]
    fn undo_reports_when_the_connector_cannot_undo() {
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[&undoable_open()], vec![("windows_system", sys)]);
        flow.handle(16, "open spotify", &mut user(&[]));
        assert_eq!(flow.undo(), "That can't be undone.");
    }

    #[test]
    fn safe_skill_has_no_undo() {
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let mut flow = flow_with(&[OPEN], vec![("windows_system", sys)]);
        flow.handle(17, "open spotify", &mut user(&[]));
        assert_eq!(flow.undo(), "Nothing to undo.");
    }

    // --- helpers -------------------------------------------------------------------

    #[test]
    fn fill_replaces_slots_and_never_guesses() {
        let slots = BTreeMap::from([("a".to_string(), "x".to_string())]);
        assert_eq!(fill("{a} and {a}!", &slots).unwrap(), "x and x!");
        assert_eq!(fill("no slots", &slots).unwrap(), "no slots");
        assert_eq!(fill("{b}", &slots).unwrap_err(), "I'm missing the b.");
        assert_eq!(fill("odd { text", &slots).unwrap(), "odd { text");
    }

    #[test]
    fn iso_time_formats_known_dates() {
        assert_eq!(iso_from_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_secs(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(iso_from_secs(1_782_000_000), "2026-06-21T00:00:00Z");
    }
}
