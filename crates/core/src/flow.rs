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
use crate::resolver::{self, ContextSource, NoContext, Resolution, SlotValue};
use crate::router::{decide, RouteDecision, Router};
use crate::verifier::{verify, Verdict};
use jitmo_connectors::{Args, ConnError, Connector, Health, Output};
use jitmo_ipc::{Choice, CoreEvent, RequestId, Span};
use jitmo_skills::{Skill, Slot, Undo};
use jitmo_store::{Decision, Store};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How the flow talks to the user mid-request.
pub trait Prompter {
    /// Show a confirm card. `true` means approved; `false` means Cancel.
    fn confirm(&mut self, text: &str) -> bool;
    /// Ask one question. `None` means the user dismissed it.
    fn choose(&mut self, question: &str, choices: &[Choice]) -> Option<String>;
}

/// What Undo needs: every step that ran (Undo walks them backwards) and when
/// the window closes.
struct UndoRecord {
    /// The action-log row to clear. `None` if writing the log failed.
    log_id: Option<i64>,
    /// (connector, action, output) of each step, in the order they ran.
    steps: Vec<(String, String, Output)>,
    expires: Instant,
}

/// Slot values by name. A template that needs a field a value does not have
/// (a contact's number before contacts are resolved) is refused up front
/// instead of being guessed.
type Slots = BTreeMap<String, SlotValue>;

pub struct Flow {
    router: Router,
    store: Store,
    connectors: HashMap<String, Arc<dyn Connector>>,
    clock: Box<dyn Fn() -> String + Send>,
    last_undo: Option<UndoRecord>,
    /// Overrides the policy's Undo window (tests).
    undo_window: Option<Duration>,
    context: Box<dyn ContextSource>,
}

impl Flow {
    pub fn new(
        router: Router,
        store: Store,
        connectors: HashMap<String, Arc<dyn Connector>>,
    ) -> Self {
        Flow {
            router,
            store,
            connectors,
            clock: Box::new(iso_now),
            last_undo: None,
            undo_window: None,
            context: Box::new(NoContext),
        }
    }

    /// Where selected files, downloads and file names come from.
    pub fn with_context(mut self, context: impl ContextSource + 'static) -> Self {
        self.context = Box::new(context);
        self
    }

    /// Replace the clock (tests).
    pub fn with_clock(mut self, clock: impl Fn() -> String + Send + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Replace the Undo window that the policy sets (tests).
    pub fn with_undo_window(mut self, window: Duration) -> Self {
        self.undo_window = Some(window);
        self
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Handle one request and return its final event.
    pub fn handle(&mut self, id: RequestId, text: &str, prompter: &mut dyn Prompter) -> CoreEvent {
        self.run(id, text, prompter)
    }

    /// Undo the last undoable action, if any. Returns the one-line result.
    ///
    /// Steps are undone newest first. Only works inside the policy's Undo
    /// window (10 seconds), so a forgotten action is never reversed later.
    pub fn undo(&mut self) -> String {
        let Some(rec) = self.last_undo.take() else {
            return "Nothing to undo.".into();
        };
        if Instant::now() > rec.expires {
            self.clear_token(rec.log_id);
            return "Undo is no longer available.".into();
        }
        let (mut undone, mut skipped) = (0, 0);
        for (connector, action, output) in rec.steps.iter().rev() {
            let Some(conn) = self.connectors.get(connector) else {
                skipped += 1;
                continue;
            };
            match conn.undo(action, output) {
                None => skipped += 1,
                Some(Ok(())) => undone += 1,
                Some(Err(e)) => {
                    // Stop here: undoing earlier steps without this one could
                    // leave things in a state the user did not expect.
                    if undone > 0 {
                        self.clear_token(rec.log_id);
                    }
                    return if undone > 0 {
                        format!("{e} Part of it was undone.")
                    } else {
                        e.to_string()
                    };
                }
            }
        }
        if undone == 0 {
            return "That can't be undone.".into();
        }
        self.clear_token(rec.log_id);
        if skipped > 0 {
            "Undone, except for steps that can't be undone.".into()
        } else {
            "Undone.".into()
        }
    }

    fn clear_token(&self, log_id: Option<i64>) {
        if let Some(id) = log_id {
            if let Err(e) = self.store.clear_undo_token(id) {
                eprintln!("could not update the action log: {e}");
            }
        }
    }

    /// Write to the action log. A broken log must never change what the user
    /// is told about an action, so a failure is reported on stderr and returns
    /// `None`; the caller decides whether the user needs a warning.
    fn log(
        &self,
        request: &str,
        skill_id: Option<&str>,
        decision: Decision,
        result: &str,
        undo_token: Option<&str>,
    ) -> Option<i64> {
        match self.store.log_action(
            &(self.clock)(),
            request,
            skill_id,
            decision,
            Some(result),
            undo_token,
        ) {
            Ok(id) => Some(id),
            Err(e) => {
                eprintln!("could not write to the action log: {e}");
                None
            }
        }
    }

    fn fail(
        &self,
        id: RequestId,
        request: &str,
        skill_id: Option<&str>,
        message: &str,
    ) -> CoreEvent {
        self.log(request, skill_id, Decision::Failed, message, None);
        CoreEvent::Failed {
            id,
            message: message.into(),
        }
    }

    fn cancelled(&self, id: RequestId, request: &str, skill_id: Option<&str>) -> CoreEvent {
        self.log(request, skill_id, Decision::Cancelled, "Cancelled.", None);
        CoreEvent::Done {
            id,
            message: "Cancelled.".into(),
            undoable: false,
        }
    }

    fn run(&mut self, id: RequestId, text: &str, p: &mut dyn Prompter) -> CoreEvent {
        // 1. Route.
        let understanding = self.router.route(text);
        let skill_id = match decide(&understanding) {
            RouteDecision::Decline => return self.fail(id, text, None, "I can't do that yet."),
            RouteDecision::Chained => {
                return self.fail(
                    id,
                    text,
                    None,
                    "I can't plan that right now. Try a simpler request.",
                )
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
                    .map(|s| Choice {
                        id: s.clone(),
                        label: self.describe(s),
                    })
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
        let mut slots = match resolve_slots(
            &skill,
            &understanding.spans,
            self.context.as_ref(),
            &self.store,
            p,
        ) {
            Ok(s) => s,
            Err(SlotError::Message(msg)) => return self.fail(id, text, Some(&skill.id), &msg),
            Err(SlotError::Cancelled) => return self.cancelled(id, text, Some(&skill.id)),
        };
        slots.insert("request".to_string(), SlotValue::from_text(text));

        // 3. Policy. A broken log must not unblock anything, so if the block
        //    cannot be logged the decision still stands.
        let facts = StepFacts::from_skill(&skill, false, recipient_count(&skill, &slots));
        let outcome = policy::evaluate_and_log(&self.store, &(self.clock)(), text, &facts)
            .unwrap_or_else(|e| {
                eprintln!("could not write to the action log: {e}");
                policy::evaluate(&facts)
            });
        if let Outcome::Block { reason, .. } = outcome {
            return CoreEvent::Failed {
                id,
                message: reason.message().into(),
            };
        }

        // Every template must be fillable before the user is asked anything, so
        // nobody confirms a send that would then stop halfway.
        if let Err(msg) = check_templates(&skill, &slots) {
            return self.fail(id, text, Some(&skill.id), &msg);
        }

        let mut confirmed = false;
        if outcome == Outcome::Confirm {
            let template = skill
                .confirm
                .as_ref()
                .map_or("Go ahead?", |c| c.text.as_str());
            let card = match fill(template, &slots) {
                Ok(t) => t,
                Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
            };
            if !p.confirm(&card) {
                return self.cancelled(id, text, Some(&skill.id));
            }
            confirmed = true;
        }

        // 4. Run the steps.
        let mut executed: Vec<(String, String, Output)> = Vec::new();
        for step in &skill.steps {
            let Some(conn) = self.connectors.get(&step.connector).cloned() else {
                return self.fail(
                    id,
                    text,
                    Some(&skill.id),
                    &format!("{} isn't available.", step.connector),
                );
            };
            if let Some(err) = health_error(conn.as_ref(), &step.connector) {
                return self.fail(id, text, Some(&skill.id), &err.to_string());
            }
            let args = match fill_args(&step.args, &slots) {
                Ok(a) => a,
                Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
            };
            match conn.execute(&step.action, &args) {
                Ok(out) => executed.push((step.connector.clone(), step.action.clone(), out)),
                Err(e) => return self.fail(id, text, Some(&skill.id), &e.to_string()),
            }
        }

        // 5. Verify.
        if let Some(check) = &skill.check {
            let Some(conn) = self.connectors.get(&check.connector).cloned() else {
                return self.fail(
                    id,
                    text,
                    Some(&skill.id),
                    &format!("{} isn't available.", check.connector),
                );
            };
            let args = match fill_args(&check.args, &slots) {
                Ok(a) => a,
                Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
            };
            let msg = format!(
                "I couldn't confirm that it worked. Please check {}.",
                check.connector
            );
            if let Verdict::NotVerified { message } =
                verify(conn.as_ref(), &check.action, &args, &msg)
            {
                return self.fail(id, text, Some(&skill.id), &message);
            }
        }

        // 6. Log and report. The action already happened and was verified, so a
        //    log failure is a warning on a successful result, never an error.
        // A read-only task answers with the text its last step returned.
        let answer = executed
            .iter()
            .rev()
            .find_map(|(_, _, out)| out.0.get("message").cloned());
        let undo_seconds = match outcome {
            Outcome::RunThenUndo { seconds } => Some(seconds),
            _ => None,
        };
        let can_undo =
            undo_seconds.is_some() && matches!(skill.undo, Undo::Steps(_)) && !executed.is_empty();
        let decision = if confirmed {
            Decision::Confirmed
        } else {
            Decision::Ran
        };
        let log_id = self.log(
            text,
            Some(&skill.id),
            decision,
            "Done.",
            can_undo.then_some("available"),
        );
        self.last_undo = match (can_undo, undo_seconds) {
            (true, Some(seconds)) => Some(UndoRecord {
                log_id,
                steps: executed,
                expires: Instant::now()
                    + self
                        .undo_window
                        .unwrap_or_else(|| Duration::from_secs(u64::from(seconds))),
            }),
            _ => None,
        };
        let base = answer.unwrap_or_else(|| "Done.".to_string());
        let message = if log_id.is_some() {
            base
        } else {
            format!("{base} (I couldn't save this to the activity log.)")
        };
        CoreEvent::Done {
            id,
            message,
            undoable: can_undo,
        }
    }

    /// A short human label for a skill, used in questions.
    fn describe(&self, skill_id: &str) -> String {
        self.router
            .skill(skill_id)
            .and_then(|s| s.examples.first().cloned())
            .unwrap_or_else(|| skill_id.to_string())
    }
}

/// Why slot filling stopped.
enum SlotError {
    /// A one-line message for the user.
    Message(String),
    /// The user dismissed the question.
    Cancelled,
}

/// Fill each slot from the spans the router found (first unused span of the
/// slot's type), or from the skill's fixed value. Real things (files) come from
/// the resolver; if several fit equally the user is asked one question (FR4).
fn resolve_slots(
    skill: &Skill,
    spans: &[Span],
    ctx: &dyn ContextSource,
    store: &Store,
    p: &mut dyn Prompter,
) -> Result<Slots, SlotError> {
    let mut used = vec![false; spans.len()];
    let mut out = Slots::new();
    for (name, slot) in &skill.slots {
        match slot {
            Slot::Fixed(v) => {
                out.insert(name.clone(), SlotValue::from_text(v));
            }
            Slot::Param(sp) => {
                let found = spans
                    .iter()
                    .enumerate()
                    .find(|(i, s)| !used[*i] && s.kind == sp.kind);
                let said = found.map(|(i, s)| {
                    used[i] = true;
                    s.text.as_str()
                });
                let usual = store
                    .get_setting(&format!("usual.{}", sp.kind))
                    .ok()
                    .flatten();
                match resolver::resolve(&sp.resolve, said, ctx, usual.as_deref()) {
                    Resolution::Found(v) => {
                        out.insert(name.clone(), v);
                    }
                    Resolution::Choose { question, options } => {
                        let choices: Vec<Choice> = options
                            .iter()
                            .map(|(id, label, _)| Choice {
                                id: id.clone(),
                                label: label.clone(),
                            })
                            .collect();
                        let picked = p.choose(&question, &choices).ok_or(SlotError::Cancelled)?;
                        let Some((_, _, v)) = options.into_iter().find(|(id, _, _)| *id == picked)
                        else {
                            return Err(SlotError::Cancelled);
                        };
                        out.insert(name.clone(), v);
                    }
                    Resolution::NotFound(words) if sp.required => {
                        return Err(SlotError::Message(match words {
                            Some(w) => format!("I couldn't find {w}."),
                            None => format!("I'm missing the {name}."),
                        }))
                    }
                    Resolution::NotFound(_) => {}
                }
            }
        }
    }
    Ok(out)
}

/// Replace `{slot}` or `{slot.field}` in a template. A slot or field that is
/// not known is an error, never a guess.
fn fill(template: &str, slots: &Slots) -> Result<String, String> {
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
        let value = match key.split_once('.') {
            Some((slot, field)) => slots.get(slot).and_then(|v| v.fields.get(field)),
            None => slots.get(key).map(|v| &v.text),
        };
        match value {
            Some(v) => out.push_str(v),
            None => return Err(format!("I'm missing the {key}.")),
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn fill_args(args: &BTreeMap<String, String>, slots: &Slots) -> Result<Args, String> {
    args.iter()
        .map(|(k, v)| Ok((k.clone(), fill(v, slots)?)))
        .collect()
}

/// Fill every template in the skill (confirm text, step and check arguments)
/// without running anything, so a gap shows up before the user is asked to
/// confirm. The first gap is reported as a one-line message.
fn check_templates(skill: &Skill, slots: &Slots) -> Result<(), String> {
    if let Some(c) = &skill.confirm {
        fill(&c.text, slots)?;
    }
    for step in skill.steps.iter().chain(skill.check.iter()) {
        fill_args(&step.args, slots)?;
    }
    Ok(())
}

/// How many people a send goes to, read from the skill's `contact` slots.
///
/// Phase 1 stand-in for the resolver. Names split by commas, `&` or "and"
/// are counted. "all", "everyone" and "contacts" mean the whole address book,
/// which is always over the limit. Anything that is not a send counts as 1.
fn recipient_count(skill: &Skill, slots: &Slots) -> u32 {
    let mut total: Option<u32> = None;
    for (name, slot) in &skill.slots {
        if matches!(slot, Slot::Param(sp) if sp.kind == "contact") {
            if let Some(v) = slots.get(name) {
                total = Some(total.unwrap_or(0).saturating_add(people_in(&v.text)));
            }
        }
    }
    total.unwrap_or(1)
}

fn people_in(text: &str) -> u32 {
    let lower = text.to_lowercase();
    let whole_book = lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| matches!(w, "all" | "everyone" | "everybody" | "contacts"));
    if whole_book {
        return u32::MAX;
    }
    let names = lower
        .split([',', '&'])
        .flat_map(|part| part.split(" and "))
        .filter(|name| !name.trim().is_empty())
        .count();
    names.max(1) as u32
}

fn health_error(conn: &dyn Connector, name: &str) -> Option<ConnError> {
    match conn.health() {
        Health::Ready => None,
        Health::NotInstalled => Some(ConnError::NotInstalled(name.into())),
        Health::NotSignedIn => Some(ConnError::NotSignedIn(name.into())),
        Health::Unreachable => Some(ConnError::Failed(format!(
            "I can't reach {name} right now."
        ))),
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
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
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
            if self.confirm_answers.is_empty() {
                true
            } else {
                self.confirm_answers.remove(0)
            }
        }
        fn choose(&mut self, _q: &str, choices: &[Choice]) -> Option<String> {
            self.choices_seen = choices.iter().map(|c| c.id.clone()).collect();
            self.choice.clone()
        }
    }

    fn user(answers: &[bool]) -> TestUser {
        TestUser {
            confirm_answers: answers.to_vec(),
            ..TestUser::default()
        }
    }

    fn flow_with(yamls: &[&str], conns: Vec<(&str, Arc<FakeConnector>)>) -> Flow {
        let skills = yamls.iter().map(|y| Skill::parse(y).unwrap()).collect();
        let map = conns
            .into_iter()
            .map(|(k, v)| (k.to_string(), v as Arc<dyn Connector>))
            .collect();
        Flow::new(Router::new(skills), Store::open_in_memory().unwrap(), map)
            .with_clock(|| "2026-11-08T12:00:00Z".into())
    }

    fn log_decisions(f: &Flow) -> Vec<String> {
        f.store()
            .recent_actions(20)
            .unwrap()
            .into_iter()
            .rev()
            .map(|r| r.decision)
            .collect()
    }

    fn done(ev: &CoreEvent) -> (&str, bool) {
        match ev {
            CoreEvent::Done {
                message, undoable, ..
            } => (message, *undoable),
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
            Call::Execute {
                action: "open_app".into(),
                args: Args::from([("app".into(), "spotify".into())])
            }
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
            FakeConnector::new("windows_system")
                .fail_execute_with(ConnError::Failed("Couldn't open it.".into())),
        );
        let mut flow = flow_with(&[OPEN], vec![("windows_system", sys)]);
        assert_eq!(
            failed(&flow.handle(7, "open spotify", &mut user(&[]))),
            "Couldn't open it."
        );
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
        assert_eq!(
            failed(&flow.handle(9, "what is the capital of france", &mut user(&[]))),
            "I can't do that yet."
        );
        assert!(
            failed(&flow.handle(10, "open spotify and then pause the music", &mut user(&[])))
                .contains("simpler request")
        );
        assert!(sys.calls().is_empty());
    }

    // --- confirm the task / ask ------------------------------------------------

    #[test]
    fn medium_confidence_asks_to_confirm_the_task() {
        let sp = Arc::new(FakeConnector::new("spotify"));
        let mut flow = flow_with(&[PAUSE], vec![("spotify", sp.clone())]);
        let mut no = user(&[false]);
        assert_eq!(
            done(&flow.handle(11, "the music pause", &mut no)).0,
            "Cancelled."
        );
        assert!(no.shown[0].starts_with("Did you mean:"));
        assert_eq!(sp.execute_count(), 0);
        let mut yes = user(&[true]);
        assert_eq!(
            done(&flow.handle(12, "the music pause", &mut yes)).0,
            "Done."
        );
        assert_eq!(sp.execute_count(), 1);
    }

    #[test]
    fn low_confidence_asks_a_question_and_runs_the_choice() {
        let sp = Arc::new(FakeConnector::new("spotify"));
        let mut flow = flow_with(&[PAUSE], vec![("spotify", sp.clone())]);
        let mut u = TestUser {
            choice: Some("music.pause".into()),
            ..TestUser::default()
        };
        assert_eq!(done(&flow.handle(13, "pause music", &mut u)).0, "Done.");
        assert_eq!(u.choices_seen, vec!["music.pause"]);
        assert_eq!(sp.execute_count(), 1);
        // Dismissing the question does nothing.
        let mut dismiss = TestUser::default();
        assert_eq!(
            done(&flow.handle(14, "pause music", &mut dismiss)).0,
            "Cancelled."
        );
        assert_eq!(sp.execute_count(), 1);
    }

    // --- undo --------------------------------------------------------------------

    fn undoable_open() -> String {
        OPEN.replace("risk: safe", "risk: changes_data").replace(
            "undo: none",
            "undo:\n  - connector: windows_system\n    action: close_app\n",
        )
    }

    #[test]
    fn data_change_offers_undo_and_undo_works_once() {
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let mut flow = flow_with(&[&undoable_open()], vec![("windows_system", sys.clone())]);
        let ev = flow.handle(15, "open spotify", &mut user(&[]));
        assert_eq!(done(&ev), ("Done.", true));
        assert_eq!(
            flow.store().recent_actions(1).unwrap()[0]
                .undo_token
                .as_deref(),
            Some("available")
        );
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
        let slots = Slots::from([("a".to_string(), SlotValue::from_text("x"))]);
        assert_eq!(fill("{a} and {a}!", &slots).unwrap(), "x and x!");
        assert_eq!(fill("no slots", &slots).unwrap(), "no slots");
        assert_eq!(fill("{b}", &slots).unwrap_err(), "I'm missing the b.");
        assert_eq!(fill("odd { text", &slots).unwrap(), "odd { text");
    }

    // --- review fixes ---------------------------------------------------------

    const WHATSAPP: &str = include_str!("../../../skills/whatsapp.send_file.yaml");

    #[test]
    fn dotted_fields_are_filled_from_the_slot() {
        let slots = Slots::from([("file".to_string(), SlotValue::from_text("report.pdf"))]);
        assert_eq!(fill("{file.name}", &slots).unwrap(), "report.pdf");
        assert_eq!(fill("{file}", &slots).unwrap(), "report.pdf");
        // A field the stub cannot know is refused, never guessed from the name.
        assert_eq!(
            fill("{file.path}", &slots).unwrap_err(),
            "I'm missing the file.path."
        );
        assert_eq!(
            fill("{nope.name}", &slots).unwrap_err(),
            "I'm missing the nope.name."
        );
    }

    /// A fixed file context for resolver tests.
    struct Files {
        selected: Option<crate::resolver::FileRef>,
        files: Vec<crate::resolver::FileRef>,
    }

    impl ContextSource for Files {
        fn selected_file(&self) -> Option<crate::resolver::FileRef> {
            self.selected.clone()
        }
        fn recent_downloads(&self, limit: usize) -> Vec<crate::resolver::FileRef> {
            self.files.iter().take(limit).cloned().collect()
        }
        fn searchable_files(&self) -> Vec<crate::resolver::FileRef> {
            self.files.clone()
        }
    }

    fn file_ref(name: &str) -> crate::resolver::FileRef {
        crate::resolver::FileRef {
            name: name.into(),
            path: format!("C:/Docs/{name}"),
            modified: 1,
        }
    }

    fn whatsapp_flow(ctx: Files) -> (Flow, Arc<FakeConnector>) {
        let wa = Arc::new(FakeConnector::new("whatsapp_desktop"));
        let flow = flow_with(&[WHATSAPP], vec![("whatsapp_desktop", wa.clone())]).with_context(ctx);
        (flow, wa)
    }

    const MOVE: &str = include_str!("../../../skills/files.move.yaml");

    struct WithFolders(Files, Vec<crate::resolver::FileRef>);

    impl ContextSource for WithFolders {
        fn selected_file(&self) -> Option<crate::resolver::FileRef> {
            self.0.selected_file()
        }
        fn recent_downloads(&self, n: usize) -> Vec<crate::resolver::FileRef> {
            self.0.recent_downloads(n)
        }
        fn searchable_files(&self) -> Vec<crate::resolver::FileRef> {
            self.0.searchable_files()
        }
        fn searchable_folders(&self) -> Vec<crate::resolver::FileRef> {
            self.1.clone()
        }
    }

    fn move_flow(folders: Vec<crate::resolver::FileRef>) -> (Flow, Arc<FakeConnector>) {
        let files = Arc::new(FakeConnector::new("files").undoable());
        let ctx = WithFolders(
            Files {
                selected: None,
                files: vec![file_ref("report.pdf")],
            },
            folders,
        );
        let flow = flow_with(&[MOVE], vec![("files", files.clone())]).with_context(ctx);
        (flow, files)
    }

    #[test]
    fn moving_a_file_into_a_named_folder_runs_then_offers_undo() {
        let (mut flow, files) = move_flow(vec![file_ref("Invoices")]);
        let mut u = user(&[]);
        let ev = flow.handle(40, "move report.pdf to the Invoices folder", &mut u);
        assert!(
            u.shown.is_empty(),
            "a move runs and offers Undo; it does not ask first"
        );
        assert_eq!(done(&ev), ("Done.", true));
        let moved = files.calls().into_iter().any(|c| {
            matches!(&c, Call::Execute { action, args }
                if action == "move"
                    && args.get("path").map(String::as_str) == Some("C:/Docs/report.pdf")
                    && args.get("dir").map(String::as_str) == Some("C:/Docs/Invoices"))
        });
        assert!(moved);
        assert_eq!(flow.undo(), "Undone.");
    }

    #[test]
    fn a_folder_that_does_not_exist_is_not_guessed() {
        let (mut flow, files) = move_flow(vec![file_ref("Receipts")]);
        let mut u = user(&[]);
        let ev = flow.handle(41, "move report.pdf to the Invoices folder", &mut u);
        assert_eq!(failed(&ev), "I couldn't find invoices.");
        assert!(files.calls().is_empty());
    }

    #[test]
    fn a_named_file_is_found_and_its_path_reaches_the_connector() {
        let (mut flow, wa) = whatsapp_flow(Files {
            selected: None,
            files: vec![file_ref("report.pdf")],
        });
        let mut u = user(&[true]);
        let ev = flow.handle(30, "send report.pdf to ved on whatsapp", &mut u);
        assert_eq!(u.shown, vec!["Send report.pdf to ved on WhatsApp?"]);
        assert_eq!(done(&ev).0, "Done.");
        let sent = wa.calls().into_iter().any(|c| {
            matches!(&c, Call::Execute { action, args }
                if action == "attach_and_send" && args.get("file").map(String::as_str) == Some("C:/Docs/report.pdf"))
        });
        assert!(sent, "the file path must be passed to the connector");
    }

    #[test]
    fn this_file_means_the_selected_file() {
        let (mut flow, _wa) = whatsapp_flow(Files {
            selected: Some(file_ref("picked.docx")),
            files: vec![file_ref("other.pdf")],
        });
        let mut u = user(&[true]);
        let ev = flow.handle(32, "send this to ved on whatsapp", &mut u);
        assert_eq!(u.shown, vec!["Send picked.docx to ved on WhatsApp?"]);
        assert_eq!(done(&ev).0, "Done.");
    }

    #[test]
    fn two_equal_files_ask_one_question_and_use_the_answer() {
        let (mut flow, wa) = whatsapp_flow(Files {
            selected: None,
            files: vec![file_ref("Invoice March.pdf"), file_ref("Invoice April.pdf")],
        });
        let mut u = user(&[true]);
        u.choice = Some("C:/Docs/Invoice April.pdf".into());
        let ev = flow.handle(33, "send invoice to ved on whatsapp", &mut u);
        assert_eq!(u.choices_seen.len(), 2);
        assert_eq!(u.shown, vec!["Send Invoice April.pdf to ved on WhatsApp?"]);
        assert_eq!(done(&ev).0, "Done.");
        assert_eq!(wa.execute_count(), 2);
    }

    #[test]
    fn dismissing_the_question_cancels_and_sends_nothing() {
        let (mut flow, wa) = whatsapp_flow(Files {
            selected: None,
            files: vec![file_ref("Invoice March.pdf"), file_ref("Invoice April.pdf")],
        });
        let mut u = user(&[true]);
        u.choice = None;
        let ev = flow.handle(34, "send invoice to ved on whatsapp", &mut u);
        assert_eq!(done(&ev).0, "Cancelled.");
        assert!(wa.calls().is_empty());
    }

    #[test]
    fn a_file_that_does_not_exist_is_not_guessed_and_nothing_is_confirmed() {
        let (mut flow, wa) = whatsapp_flow(Files {
            selected: None,
            files: vec![file_ref("holiday.jpg")],
        });
        let mut u = user(&[true]);
        let ev = flow.handle(31, "send report.pdf to ved on whatsapp", &mut u);
        assert_eq!(failed(&ev), "I couldn't find report.pdf.");
        assert!(
            u.shown.is_empty(),
            "no confirm for a send that cannot finish"
        );
        assert!(wa.calls().is_empty());
    }

    #[test]
    fn recipients_are_counted_from_the_contact_slot() {
        let skill = Skill::parse(SEND).unwrap();
        let count = |who: &str| {
            let slots = Slots::from([("contact".to_string(), SlotValue::from_text(who))]);
            recipient_count(&skill, &slots)
        };
        assert_eq!(count("ved"), 1);
        assert_eq!(count("ved and rahul"), 2);
        assert_eq!(count("ved, rahul & priya"), 3);
        assert_eq!(count("all my contacts"), u32::MAX);
        assert_eq!(count("everyone"), u32::MAX);
        assert_eq!(
            count("Allison"),
            1,
            "a name that merely contains `all` is one person"
        );
        assert_eq!(
            recipient_count(&Skill::parse(OPEN).unwrap(), &Slots::new()),
            1
        );
    }

    #[test]
    fn rule_3_now_blocks_a_send_to_too_many_people() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[SEND], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        let ev = flow.handle(32, "send hi to all my contacts", &mut u);
        assert_eq!(failed(&ev), "I can't send to more than 10 people at once.");
        assert!(u.shown.is_empty() && m.calls().is_empty());
        assert_eq!(log_decisions(&flow), vec!["blocked"]);

        let eleven = "a, b, c, d, e, f, g, h, i, j, k";
        let ev = flow.handle(33, &format!("send hi to {eleven}"), &mut user(&[true]));
        assert!(failed(&ev).contains("more than 10"));

        // Ten people still need a confirm, and then go through.
        let ten = "a, b, c, d, e, f, g, h, i, j";
        let mut u = user(&[true]);
        let ev = flow.handle(34, &format!("send hi to {ten}"), &mut u);
        assert_eq!(done(&ev).0, "Done.");
        assert_eq!(u.shown.len(), 1);
    }

    #[test]
    fn undo_is_refused_after_the_window_closes() {
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let mut flow = flow_with(&[&undoable_open()], vec![("windows_system", sys.clone())])
            .with_undo_window(Duration::ZERO);
        flow.handle(40, "open spotify", &mut user(&[]));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(flow.undo(), "Undo is no longer available.");
        assert!(
            !sys.calls().iter().any(|c| matches!(c, Call::Undo { .. })),
            "nothing may be undone late"
        );
        assert_eq!(flow.store().recent_actions(1).unwrap()[0].undo_token, None);
        assert_eq!(flow.undo(), "Nothing to undo.");
    }

    #[test]
    fn the_default_undo_window_comes_from_the_policy() {
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let mut flow = flow_with(&[&undoable_open()], vec![("windows_system", sys)]);
        flow.handle(41, "open spotify", &mut user(&[]));
        let secs = flow
            .last_undo
            .as_ref()
            .unwrap()
            .expires
            .saturating_duration_since(Instant::now())
            .as_secs_f32();
        assert!(
            (8.0..=10.0).contains(&secs),
            "window was {secs}s, expected about 10"
        );
    }

    #[test]
    fn a_broken_log_does_not_turn_success_into_failure() {
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let mut flow = flow_with(&[&undoable_open()], vec![("windows_system", sys)]);
        flow.store().break_action_log_for_tests();
        let ev = flow.handle(42, "open spotify", &mut user(&[]));
        assert_eq!(
            done(&ev),
            ("Done. (I couldn't save this to the activity log.)", true)
        );
        assert_eq!(flow.undo(), "Undone.", "Undo still works without a log row");
    }

    #[test]
    fn a_broken_log_keeps_the_real_message_for_failures_blocks_and_cancels() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[SEND], vec![("stub_messenger", m.clone())]);
        flow.store().break_action_log_for_tests();
        assert_eq!(
            failed(&flow.handle(43, "what is the capital of france", &mut user(&[]))),
            "I can't do that yet."
        );
        assert_eq!(
            failed(&flow.handle(44, "send hi to everyone", &mut user(&[]))),
            "I can't send to more than 10 people at once."
        );
        assert_eq!(
            done(&flow.handle(45, "send hi to ved", &mut user(&[false]))).0,
            "Cancelled."
        );
        assert_eq!(m.execute_count(), 0);
    }

    fn two_step_undoable() -> String {
        undoable_open().replace(
            "check:",
            "  - connector: spotify\n    action: focus\ncheck:",
        )
    }

    #[test]
    fn undo_reverses_every_step_newest_first() {
        // Steps: windows_system.open_app, then spotify.focus. Undo steps are declared in the skill.
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let sp = Arc::new(FakeConnector::new("spotify").undoable());
        let mut flow = flow_with(
            &[&two_step_undoable()],
            vec![("windows_system", sys.clone()), ("spotify", sp.clone())],
        );
        assert!(done(&flow.handle(50, "open spotify", &mut user(&[]))).1);
        assert_eq!(flow.undo(), "Undone.");
        assert!(sp
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Undo { action } if action == "focus")));
        assert!(sys
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Undo { action } if action == "open_app")));
    }

    #[test]
    fn partial_undo_is_reported_honestly() {
        let sys = Arc::new(FakeConnector::new("windows_system").undoable());
        let sp = Arc::new(FakeConnector::new("spotify")); // cannot undo
        let mut flow = flow_with(
            &[&two_step_undoable()],
            vec![("windows_system", sys), ("spotify", sp)],
        );
        flow.handle(51, "open spotify", &mut user(&[]));
        assert_eq!(
            flow.undo(),
            "Undone, except for steps that can't be undone."
        );
    }

    #[test]
    fn nothing_undoable_says_so() {
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let sp = Arc::new(FakeConnector::new("spotify"));
        let mut flow = flow_with(
            &[&two_step_undoable()],
            vec![("windows_system", sys), ("spotify", sp)],
        );
        flow.handle(52, "open spotify", &mut user(&[]));
        assert_eq!(flow.undo(), "That can't be undone.");
    }

    #[test]
    fn every_shipped_skill_can_be_filled_from_its_own_example() {
        // Catches a template that references something the flow can never supply.
        // Skills may still need fields only the Phase 2 resolver provides; those
        // are listed here so the gap is visible and cannot grow unnoticed.
        let needs_resolver = ["whatsapp.send_file"];
        for yaml in [OPEN, PAUSE, SEND, WHATSAPP] {
            let skill = Skill::parse(yaml).unwrap();
            let slots: Slots = skill
                .slots
                .iter()
                .map(|(n, s)| {
                    let text = match s {
                        Slot::Fixed(v) => v.clone(),
                        Slot::Param(_) => "sample".into(),
                    };
                    (n.clone(), SlotValue::from_text(&text))
                })
                .collect();
            let filled = check_templates(&skill, &slots);
            if needs_resolver.contains(&skill.id.as_str()) {
                assert!(
                    filled.is_err(),
                    "{}: remove it from `needs_resolver`",
                    skill.id
                );
            } else {
                assert!(filled.is_ok(), "{}: {filled:?}", skill.id);
            }
        }
    }

    #[test]
    fn iso_time_formats_known_dates() {
        assert_eq!(iso_from_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_secs(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(iso_from_secs(1_782_000_000), "2026-06-21T00:00:00Z");
    }
}
