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

use crate::known::Known;
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
    /// Ask one short free-text question (a required slot the extractor missed).
    /// `hint` is a placeholder for the answer field. `None` means the user dismissed it.
    fn ask_text(&mut self, question: &str, hint: &str) -> Option<String>;
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
    /// Names of installed apps, asked for at each request (the list changes as apps
    /// are installed). Empty until the app supplies one.
    app_names: Box<dyn Fn() -> Vec<String> + Send>,
    /// The skill a saved plan names for the step now running: it is used as the route, so
    /// the router cannot decline a step the plan already chose.
    forced_skill: Option<String>,
    /// The third way to plan a chain (after saved templates and part-wise routing): a model.
    drafter: Option<Box<dyn crate::plan::PlanDrafter>>,
    /// The last request that worked, for follow-ups ("what about tomorrow").
    last_ok: Option<(String, Instant)>,
    /// A local language model, consulted only when the router is unsure or declined.
    llm: Option<Box<dyn crate::llm::Llm>>,
    /// Requests a model worked out that then worked (see [`crate::learned`]).
    learned: Option<crate::learned::Learned>,
    /// Word-overlap retrieval for the model's candidates (built on first use).
    retriever: Option<crate::retrieval::Retriever>,
    /// What to remember if the request now running succeeds: (key, skill, typed sentence).
    pending_learn: Option<(String, String, Option<String>)>,
    /// The key of a remembered request now running (forgotten if it fails).
    cache_hit: Option<String>,
    /// Typed values the model already gave with its tool pick: (skill, values). The sentence
    /// built from them then runs without asking the model a second time.
    pending_typed: Option<(String, BTreeMap<String, String>)>,
    /// Ask the model for a tool when the router declined too (off by default: see `run`).
    llm_on_decline: bool,
    /// Ask the model for a tool when the router is unsure. Off by default: on the day-in-the-life
    /// set it turned questions the router would have asked into wrong actions (doc 16).
    llm_picks: bool,
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
            app_names: Box::new(Vec::new),
            forced_skill: None,
            drafter: None,
            last_ok: None,
            llm: None,
            learned: None,
            retriever: None,
            pending_learn: None,
            cache_hit: None,
            pending_typed: None,
            llm_on_decline: false,
            llm_picks: false,
        }
    }

    /// Plug in a local language model: it picks a tool when the router is unsure or has
    /// declined, or answers a question no tool covers. Everything it picks is still guarded,
    /// resolved, confirmed and checked like any other request.
    pub fn with_llm(mut self, llm: Box<dyn crate::llm::Llm>) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Also let the model pick a tool when the router declined. Off by default: on the
    /// held-out set a small model overruling the tool encoder's "no tool fits" turned
    /// out-of-scope requests into wrong actions far more often than it rescued real ones.
    pub fn with_llm_on_decline(mut self, on: bool) -> Self {
        self.llm_on_decline = on;
        self
    }

    /// Let the model pick a tool when the router is unsure. Off by default: the model then only
    /// fills typed values, and an unsure route asks the user to choose.
    pub fn with_llm_picks(mut self, on: bool) -> Self {
        self.llm_picks = on;
        self
    }

    /// Plug in a plan drafter (the model planner). Its plans are checked before they run.
    pub fn with_drafter(mut self, drafter: Box<dyn crate::plan::PlanDrafter>) -> Self {
        self.drafter = Some(drafter);
        self
    }

    /// Where selected files, downloads and file names come from.
    pub fn with_context(mut self, context: impl ContextSource + 'static) -> Self {
        self.context = Box::new(context);
        self
    }

    /// Where the names of installed apps come from. Asked at every request, so a
    /// newly installed app is known at once. Used as a closed list for the router and
    /// for `named_app`.
    pub fn with_app_names(mut self, names: impl Fn() -> Vec<String> + Send + 'static) -> Self {
        self.app_names = Box::new(names);
        self
    }

    /// The closed lists for this request: installed apps and saved contacts.
    fn known(&self) -> Known {
        let mut apps = (self.app_names)();
        apps.retain(|a| a.trim().chars().count() >= 2);
        apps.sort_by_key(|a| a.to_lowercase());
        apps.dedup_by_key(|a| a.to_lowercase());
        Known {
            apps,
            contacts: self.store.contacts().unwrap_or_else(|e| {
                eprintln!("could not read the contacts: {e}");
                Vec::new()
            }),
        }
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
        // A short follow-up becomes a full sentence from the last request that worked
        // (for ten minutes); everything after that sees only the full sentence.
        let rewritten = self
            .last_ok
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(600))
            .and_then(|(last, _)| crate::followup::rewrite(last, text));
        let text = rewritten.as_deref().unwrap_or(text);
        self.pending_learn = None;
        self.cache_hit = None;
        self.pending_typed = None;
        // A cancel that came after the last request finished is not for this one.
        let _ = self.llm_cancelled();
        let event = self.run(id, text, prompter);
        if matches!(event, CoreEvent::Done { .. }) {
            self.last_ok = Some((text.to_string(), Instant::now()));
        }
        self.learn_from(&event);
        // Sunrise, sunset and the moon with no place known answer "Which city? Try ...":
        // ask it instead, then answer for that city.
        if let CoreEvent::Done { message, .. } = &event {
            if message.starts_with("Which city?") {
                return match ask_text_twice(prompter, "Which city?", "For example: Pune") {
                    Some(city) => self.run(id, &format!("{text} in {city}"), prompter),
                    None => self.cancelled(id, text, None),
                };
            }
        }
        event
    }

    /// A written answer grounded in retrieved text, with its source; `None` when there is no
    /// passage that covers the question, no answer, or the answer is not supported.
    fn grounded_answer(&mut self, id: RequestId, text: &str) -> Option<CoreEvent> {
        let conn = self.connectors.get("web_lookup")?.clone();
        let args: Args = [("text".to_string(), text.to_string())].into();
        let out = conn.execute("passages", &args).ok()?;
        let n: usize = out.0.get("n")?.parse().ok()?;
        // The page that covers the most of the question; weak coverage means a different thing
        // with a similar name.
        let (best, cover) = (0..n)
            .map(|i| {
                let page = format!(
                    "{} {}",
                    out.0.get(&format!("title{i}")).map_or("", String::as_str),
                    out.0.get(&format!("text{i}")).map_or("", String::as_str)
                );
                (i, crate::grounding::coverage(text, &page))
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        if cover < 0.6 {
            return None;
        }
        let passage = out.0.get(&format!("text{best}"))?.clone();
        let url = out
            .0
            .get(&format!("url{best}"))
            .cloned()
            .unwrap_or_default();
        let answer = self.llm.as_mut()?.answer_from(text, &passage);
        if self.llm_cancelled() {
            return Some(self.cancelled(id, text, Some("web.fact")));
        }
        let answer = answer?;
        if !crate::grounding::supported(&answer, &passage) {
            return None;
        }
        let message = format!(
            "{} (Wikipedia, CC BY-SA 4.0: {url})",
            answer.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        self.log(text, Some("web.fact"), Decision::Ran, &message, None);
        Some(CoreEvent::Done {
            id,
            message,
            undoable: false,
        })
    }

    fn learned_mut(&mut self) -> &mut crate::learned::Learned {
        if self.learned.is_none() {
            let saved = self
                .store
                .get_setting("learned_requests")
                .ok()
                .flatten()
                .unwrap_or_default();
            self.learned = Some(crate::learned::Learned::load(&saved));
        }
        self.learned.get_or_insert_with(Default::default)
    }

    fn save_learned(&mut self) {
        let dump = self.learned_mut().dump();
        if let Err(e) = self.store.set_setting("learned_requests", &dump) {
            eprintln!("could not save the learned requests: {e}");
        }
    }

    /// After a request: remember what a model worked out if it succeeded, and forget a
    /// remembered request that failed.
    fn learn_from(&mut self, event: &CoreEvent) {
        let pending = self.pending_learn.take();
        let hit = self.cache_hit.take();
        match event {
            CoreEvent::Done { .. } => {
                if let Some((key, skill, sentence)) = pending {
                    self.learned_mut().put(&key, &skill, sentence.as_deref());
                    self.save_learned();
                }
            }
            CoreEvent::Failed { .. } => {
                if let Some(key) = hit {
                    self.learned_mut().remove(&key);
                    self.save_learned();
                }
            }
            _ => {}
        }
    }

    /// The skill's own sentence for typed values from a model. A required value that is missing
    /// is asked for with the skill's question; `None` when the user gives up.
    fn sentence_from_call(
        &mut self,
        tool: &str,
        mut args: BTreeMap<String, String>,
        p: &mut dyn Prompter,
    ) -> Option<String> {
        let spec = self.router.skill(tool)?.tool.clone()?;
        // Values outside the skill's own list are dropped: a model cannot add arguments.
        args.retain(|k, _| spec.args.contains_key(k));
        for (name, a) in &spec.args {
            if a.required && args.get(name).is_none_or(|v| v.trim().is_empty()) {
                let question = a
                    .ask
                    .clone()
                    .unwrap_or_else(|| format!("What is the {name}?"));
                let answer = ask_text_twice(p, &question, &a.description)?;
                args.insert(name.clone(), answer);
            }
        }
        spec.render(&args).ok()
    }

    /// Typed values for a skill whose steps take them (`{name?}`), filled by the model from the
    /// words of the request. Only the skill's own names are kept. `None` when there is no
    /// model, the skill takes no typed values, or the model left a required value empty: the
    /// connector then reads the whole sentence, as it did before (and asks what is missing).
    fn typed_args(&mut self, skill_id: &str, text: &str) -> Option<BTreeMap<String, String>> {
        let skill = self.router.skill(skill_id)?.clone();
        if !skill.takes_typed() {
            return None;
        }
        let spec = skill.tool.as_ref()?;
        let given = self
            .pending_typed
            .take()
            .filter(|(id, _)| id == skill_id)
            .map(|(_, v)| v);
        let mut values = match given {
            Some(v) => v,
            None => self.llm.as_mut()?.fill(text, &tool_brief(&skill))?,
        };
        values.retain(|k, v| spec.args.contains_key(k) && !v.trim().is_empty());
        // A value the request does not contain is the model's invention ("play despacito"
        // filled as "the latest album"): it is dropped, never used.
        values.retain(|_, v| grounded_in(v, text));
        // Words to send that are only the recipient ("email the team" → "the team") are not
        // a message.
        if let Some(body) = values.get("body").map(|b| b.trim().to_lowercase()) {
            let is_person = values
                .iter()
                .any(|(k, v)| k != "body" && v.trim().to_lowercase() == body);
            if is_person {
                values.remove("body");
            }
        }
        let complete = spec
            .args
            .iter()
            .all(|(name, a)| !a.required || values.contains_key(name));
        (complete && !values.is_empty()).then_some(values)
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

    /// Put the questions of a bare command to the user, one by one, and build the
    /// completed sentence. `None` when the user dismisses a question or gives no
    /// usable answer twice.
    fn ask_clarify(&mut self, c: &crate::clarify::Clarify, p: &mut dyn Prompter) -> Option<String> {
        use crate::clarify::{fill, Step};
        let mut answers: Vec<String> = Vec::new();
        for step in &c.steps {
            let answer = match step {
                Step::Choice {
                    question,
                    options,
                    other_question,
                    other_hint,
                } => {
                    let choices: Vec<Choice> = options
                        .iter()
                        .enumerate()
                        .map(|(i, (label, _))| Choice {
                            id: i.to_string(),
                            label: (*label).into(),
                        })
                        .collect();
                    let picked: usize = p.choose(question, &choices)?.parse().ok()?;
                    match options.get(picked)?.1 {
                        Some(value) => value.to_string(),
                        None => ask_text_twice(p, other_question, other_hint)?,
                    }
                }
                Step::Text { question, hint } => ask_text_twice(p, question, hint)?,
            };
            answers.push(answer);
        }
        Some(fill(c, &answers))
    }

    /// Ask how long a timer should run: quick choices, or a custom length.
    /// `None` when the user dismissed the question.
    fn ask_timer_length(&mut self, p: &mut dyn Prompter) -> Option<String> {
        let presets = [
            ("5", "5 minutes"),
            ("10", "10 minutes"),
            ("15", "15 minutes"),
            ("30", "30 minutes"),
            ("60", "1 hour"),
        ];
        let mut choices: Vec<Choice> = presets
            .iter()
            .map(|(id, label)| Choice {
                id: (*id).into(),
                label: (*label).into(),
            })
            .collect();
        choices.push(Choice {
            id: "custom".into(),
            label: "Custom...".into(),
        });
        let picked = p.choose("How long should the timer run?", &choices)?;
        if let Some((_, label)) = presets.iter().find(|(id, _)| *id == picked) {
            return Some((*label).to_string());
        }
        // Custom: a free-text answer, asked once more if it is blank or has no number.
        let mut hint = "For example 7 minutes, or 1h 30m";
        for _ in 0..2 {
            let answer = p.ask_text("How long?", hint)?;
            let answer = answer.trim();
            if answer.chars().any(|c| c.is_ascii_digit()) {
                // A bare number means minutes.
                return Some(if answer.chars().all(|c| c.is_ascii_digit()) {
                    format!("{answer} minutes")
                } else {
                    answer.to_string()
                });
            }
            hint = "Type a number, like 7 or 1h 30m";
        }
        None
    }

    /// List or stop timers and reminders. Stopping is permanent (a stopped timer
    /// is quick to set again), so there is no Undo.
    fn run_timer_command(
        &mut self,
        id: RequestId,
        text: &str,
        cmd: TimerCommand,
        p: &mut dyn Prompter,
    ) -> CoreEvent {
        let done = |flow: &Self, message: String| {
            flow.log(text, None, Decision::Ran, &message, None);
            CoreEvent::Done {
                id,
                message,
                undoable: false,
            }
        };
        let now = chrono::DateTime::parse_from_rfc3339(&(self.clock)())
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        let pending = match self.store.pending_reminders() {
            Ok(list) => list,
            Err(e) => {
                return self.fail(id, text, None, &format!("I couldn't read your timers: {e}"))
            }
        };
        let wanted: Vec<_> = pending
            .into_iter()
            .filter(|r| match cmd.kind {
                TimerKind::Timers => r.text == "Timer",
                TimerKind::Reminders => r.text != "Timer",
                TimerKind::Both => true,
            })
            .collect();
        let noun = cmd.kind.noun();
        if wanted.is_empty() {
            return done(
                self,
                if cmd.cancel {
                    format!("You have no {noun} to stop.")
                } else {
                    format!("You have no {noun} set.")
                },
            );
        }
        let describe = |r: &jitmo_store::Reminder| -> String {
            let due = chrono::DateTime::parse_from_rfc3339(&r.due_at)
                .map(|t| t.with_timezone(&chrono::Utc));
            let Ok(due) = due else {
                return r.text.clone();
            };
            if r.text == "Timer" {
                return format!("Timer: {} left", time_left(due - now));
            }
            let local = due.with_timezone(&chrono::Local);
            format!("{} at {}", r.text, local.format("%-I:%M %p"))
        };
        if !cmd.cancel {
            let lines: Vec<String> = wanted.iter().map(&describe).collect();
            return done(self, lines.join(". ") + ".");
        }
        let targets: Vec<_> = if cmd.all || wanted.len() == 1 {
            wanted
        } else {
            let mut choices: Vec<Choice> = wanted
                .iter()
                .map(|r| Choice {
                    id: r.id.to_string(),
                    label: describe(r),
                })
                .collect();
            choices.push(Choice {
                id: "all".into(),
                label: format!("All of them ({})", wanted.len()),
            });
            match p.choose("Which one?", &choices) {
                Some(c) if c == "all" => wanted,
                Some(c) => wanted
                    .into_iter()
                    .filter(|r| r.id.to_string() == c)
                    .collect(),
                None => return self.cancelled(id, text, None),
            }
        };
        let mut stopped = Vec::new();
        for r in &targets {
            if self.store.delete_reminder(r.id).is_ok() {
                stopped.push(describe(r));
            }
        }
        if stopped.is_empty() {
            return self.fail(id, text, None, "I couldn't stop that.");
        }
        let message = if stopped.len() == 1 {
            format!("Stopped. ({})", stopped[0])
        } else {
            format!("Stopped {} {noun}.", stopped.len())
        };
        done(self, message)
    }

    /// The user cancelled the model's call (Esc): the request ends as cancelled, with nothing
    /// run and no answer. Reading clears it.
    fn llm_cancelled(&mut self) -> bool {
        self.llm.as_mut().is_some_and(|m| m.take_cancelled())
    }

    fn cancelled(&self, id: RequestId, request: &str, skill_id: Option<&str>) -> CoreEvent {
        self.log(request, skill_id, Decision::Cancelled, "Cancelled.", None);
        CoreEvent::Done {
            id,
            message: "Cancelled.".into(),
            undoable: false,
        }
    }

    /// A plan made by routing each part of a chained request on its own: every part must
    /// be one the router is sure of, and the whole plan must pass the plan checks.
    fn plan_by_parts(&self, text: &str, known: &Known) -> Option<crate::plan::Plan> {
        let parts = crate::plan::parts(text);
        if !(2..=crate::plan::MAX_STEPS).contains(&parts.len()) {
            return None;
        }
        let mut steps = Vec::new();
        for part in &parts {
            let u = self.router.route_with(part, &known.note());
            let RouteDecision::Run { skill_id } = decide(&u) else {
                return None;
            };
            steps.push(crate::plan::PlanStep::request(&skill_id, part));
        }
        let plan = crate::plan::Plan { steps };
        crate::plan::check(&plan, &|s| {
            self.router.skill(s).map(crate::plan::SkillShape::of)
        })
        .ok()
        .map(|()| plan)
    }

    /// [`Flow::run_each`] for a saved plan: each step runs as the skill the plan named.
    fn run_plan(
        &mut self,
        id: RequestId,
        steps: &[String],
        skills: &[String],
        p: &mut dyn Prompter,
    ) -> CoreEvent {
        let mut said: Vec<String> = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            self.forced_skill = skills.get(i).cloned();
            let event = self.run(id, step, p);
            self.forced_skill = None;
            match event {
                CoreEvent::Done { message, .. } => said.push(message),
                CoreEvent::Failed { message, .. } => {
                    said.push(message);
                    if i + 1 < steps.len() {
                        said.push("I stopped there.".into());
                    }
                    return CoreEvent::Failed {
                        id,
                        message: said.join(" "),
                    };
                }
                other => return other,
            }
        }
        CoreEvent::Done {
            id,
            message: said.join(" "),
            undoable: false,
        }
    }

    /// Run several requests in order and report them as one. A step that fails or is
    /// cancelled stops the rest, so "then" never runs on top of something that did not happen.
    fn run_each(&mut self, id: RequestId, steps: &[String], p: &mut dyn Prompter) -> CoreEvent {
        let mut said: Vec<String> = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            match self.run(id, step, p) {
                CoreEvent::Done { message, .. } => said.push(message),
                CoreEvent::Failed { message, .. } => {
                    said.push(message);
                    if i + 1 < steps.len() {
                        said.push("I stopped there.".into());
                    }
                    return CoreEvent::Failed {
                        id,
                        message: said.join(" "),
                    };
                }
                other => return other,
            }
        }
        CoreEvent::Done {
            id,
            message: said.join(" "),
            // Undo covers only the last step, which would mislead here.
            undoable: false,
        }
    }

    fn run(&mut self, id: RequestId, said: &str, p: &mut dyn Prompter) -> CoreEvent {
        // Greetings and fillers are dropped and a mistyped first verb is repaired
        // ("plya arijit singh" -> "play arijit singh") before anything looks at it.
        let english = crate::hinglish::to_english(said);
        let said = english.as_deref().unwrap_or(said);
        let normalized = crate::normalize::normalize(said);
        let text = normalized.as_str();
        // 0. "What can you do?" is answered here, without the router.
        if is_help_request(text) {
            self.log(text, None, Decision::Ran, HELP_MESSAGE, None);
            return CoreEvent::Done {
                id,
                message: HELP_MESSAGE.into(),
                undoable: false,
            };
        }
        // 0. Greetings, thanks, and plain "I can't do that" for things that sound like a
        // skill but are not one (closing an app, shutting down).
        if let Some(message) = crate::normalize::plain_reply(text) {
            // A refusal is a failure (it shakes and stays); a greeting is a result.
            if message.starts_with("I can't") {
                return self.fail(id, text, None, message);
            }
            self.log(text, None, Decision::Ran, message, None);
            return CoreEvent::Done {
                id,
                message: message.into(),
                undoable: false,
            };
        }
        if crate::normalize::place_with_symbols(text) {
            return self.fail(
                id,
                text,
                Some("settings.place"),
                "That doesn't look like a place name. Try “set my city to Pune”.",
            );
        }
        // 0a. Several requests in one sentence: "pause the music and then open notepad",
        // "open zoom and spotify" (each runs), "open edge or chrome" (asks which).
        if let Some((apps, either)) = crate::normalize::several_apps(text) {
            if either {
                let choices: Vec<Choice> = apps
                    .iter()
                    .enumerate()
                    .map(|(i, a)| Choice {
                        id: i.to_string(),
                        label: capitalize(a),
                    })
                    .collect();
                return match p
                    .choose("Which one should I open?", &choices)
                    .and_then(|c| c.parse::<usize>().ok())
                    .and_then(|i| apps.get(i))
                {
                    Some(app) => self.run(id, &format!("open {app}"), p),
                    None => self.cancelled(id, text, None),
                };
            }
            let steps: Vec<String> = apps.iter().map(|a| format!("open {a}")).collect();
            return self.run_each(id, &steps, p);
        }
        // 0a'. A saved plan for a known chain ("focus mode", "mute notifications, dim the
        // screen and play my focus playlist"): checked like any plan, then run in order, and
        // it stops at the first step that fails. No model is involved.
        if let Some(plan) = crate::plan::template(text) {
            let ok = crate::plan::check(&plan, &|skill| {
                self.router.skill(skill).map(crate::plan::SkillShape::of)
            });
            return match (ok, crate::plan::sentences(&plan)) {
                (Ok(()), Some(steps)) => {
                    let skills: Vec<String> = plan.steps.iter().map(|s| s.skill.clone()).collect();
                    self.run_plan(id, &steps, &skills, p)
                }
                (Err(e), _) => {
                    eprintln!("a saved plan was rejected: {e}");
                    self.fail(id, text, None, "I can't do that yet.")
                }
                (Ok(()), None) => self.fail(id, text, None, "I can't do that yet."),
            };
        }
        if let Some(steps) = crate::normalize::sequence(text) {
            return self.run_each(id, &steps, p);
        }
        // 0b. "What timers do I have?" / "stop the timer": answered from the saved
        // reminders, since the router has no label for them.
        if let Some(cmd) = timer_command(text) {
            return self.run_timer_command(id, text, cmd, p);
        }
        // 0c. "start a timer" with no length: offer 5, 10, 15, 30 minutes, an hour or a
        // custom length, then run it as a normal timer request.
        if is_timer_without_length(text) {
            return match self.ask_timer_length(p) {
                Some(length) => self.run(id, &format!("set a timer for {length}"), p),
                None => self.cancelled(id, text, None),
            };
        }
        // 0d. A bare command ("volume", "wifi", "remind me", "search"): ask what is
        // missing, then run the completed sentence.
        if let Some(c) = crate::clarify::clarify(text) {
            return match self.ask_clarify(&c, p) {
                Some(sentence) => self.run(id, &sentence, p),
                None => self.cancelled(id, text, None),
            };
        }
        // 0z. A request a model worked out before, and that worked: no router, no model.
        // (It still goes through the guards, slots, policy, confirm and checks.)
        if self.forced_skill.is_none() {
            let lkey = crate::learned::key(text);
            if let Some((skill, sentence)) = self.learned_mut().get(&lkey) {
                if self.router.skill(&skill).is_some() {
                    self.cache_hit = Some(lkey);
                    self.forced_skill = Some(skill);
                    if let Some(s) = sentence {
                        return self.run(id, &s, p);
                    }
                }
            }
        }
        // 1. Route.
        let known = self.known();
        let understanding = self.router.route_with(text, &known.note());
        // A sum typed with no words ("2+2") or a percentage of a number ("18% of 2450") is
        // the calculator, not a guess between skills.
        let forced = self.forced_skill.take();
        let direct = match forced {
            Some(id) if self.router.skill(&id).is_some() => Some(id),
            _ => crate::normalize::direct_skill(said, text)
                .filter(|id| self.router.skill(id).is_some())
                .map(str::to_string),
        };
        let mut decision = match direct {
            Some(id) => RouteDecision::Run { skill_id: id },
            None => decide(&understanding),
        };
        // The router is unsure: a language model gets the router's best three tools (a short
        // prompt is what keeps it fast) and may pick one. Sure routes (and chains) never wait
        // for it. Nor does a decline: when the tool encoder says no tool fits, a small model
        // overruling it was mostly wrong on the held-out set ("write an essay" became an email,
        // "make flashcards" a calendar event), and a wrong action is worse than "I can't".
        let unsure = matches!(
            decision,
            RouteDecision::Ask { .. } | RouteDecision::ConfirmTask { .. }
        );
        let declined = matches!(decision, RouteDecision::Decline);
        if self.llm.is_some() && self.llm_picks && (unsure || (declined && self.llm_on_decline)) {
            // The router's best two plus the best word-overlap match: the right tool is among
            // them far more often than among the router's best three.
            let ranked: Vec<String> = understanding
                .tasks
                .iter()
                .filter(|t| self.router.skill(&t.id).is_some())
                .map(|t| t.id.clone())
                .collect();
            if self.retriever.is_none() {
                self.retriever = Some(crate::retrieval::Retriever::new(self.router.skills()));
            }
            let picks = match &self.retriever {
                Some(r) => crate::retrieval::candidates(&ranked, r, text, 3),
                None => ranked.iter().take(3).cloned().collect(),
            };
            let tools: Vec<crate::llm::ToolBrief> = picks
                .iter()
                .filter_map(|id| self.router.skill(id))
                .map(tool_brief)
                .collect();
            let recent = self.last_ok.as_ref().map(|(t, _)| t.clone());
            let pick = self
                .llm
                .as_mut()
                .map(|m| m.route(text, &tools, recent.as_deref()))
                .unwrap_or(crate::llm::Pick::Nothing);
            if self.llm_cancelled() {
                return self.cancelled(id, text, None);
            }
            match pick {
                crate::llm::Pick::Tool(skill_id) if self.router.skill(&skill_id).is_some() => {
                    self.pending_learn = Some((crate::learned::key(text), skill_id.clone(), None));
                    decision = RouteDecision::Run { skill_id };
                }
                // Typed values: the engine builds the skill's own sentence from them (asking for
                // a required one that is missing, from the skill's question, not from an error
                // string) and runs it through the normal pipeline.
                crate::llm::Pick::Call { tool, mut args } if self.router.skill(&tool).is_some() => {
                    // Only values copied from the request; an invented one is asked for instead.
                    args.retain(|_, v| grounded_in(v, text));
                    self.pending_typed = Some((tool.clone(), args.clone()));
                    if let Some(sentence) = self.sentence_from_call(&tool, args, p) {
                        self.pending_learn = Some((
                            crate::learned::key(text),
                            tool.clone(),
                            Some(sentence.clone()),
                        ));
                        self.forced_skill = Some(tool);
                        return self.run(id, &sentence, p);
                    }
                    return self.cancelled(id, text, Some(&tool));
                }
                // An answer from a model's memory is a guess. It is shown only when it is
                // grounded in retrieved text, which the writer will do (low-memory brief,
                // step 6); until then it never reaches the user.
                crate::llm::Pick::Answer(_) => {}
                _ => {}
            }
        }
        let skill_id = match decision {
            RouteDecision::Decline => {
                // No tool fits, but a question may still have an answer in a retrieved passage.
                // Reading changes nothing, and the answer shows only when the passage supports it.
                if self.llm.is_some() && is_question(text) {
                    if let Some(event) = self.grounded_answer(id, text) {
                        return event;
                    }
                }
                return self.fail(id, text, None, "I can't do that yet.");
            }
            RouteDecision::Chained => {
                // No saved plan matched. Second approach: route each part on its own; if
                // every part is a request the router is sure of, that is the plan. (A model
                // planner is the third approach, and plugs in behind the same checks.)
                if let Some(plan) = self.plan_by_parts(text, &known) {
                    let skills: Vec<String> = plan.steps.iter().map(|s| s.skill.clone()).collect();
                    if let Some(steps) = crate::plan::sentences(&plan) {
                        return self.run_plan(id, &steps, &skills, p);
                    }
                }
                // Third approach: a drafter, if one is plugged in. The best five skills go in.
                if self.drafter.is_some() {
                    let top: Vec<crate::plan::SkillBrief> = understanding
                        .tasks
                        .iter()
                        .filter_map(|t| self.router.skill(&t.id))
                        .take(crate::plan::MAX_STEPS)
                        .map(|s| crate::plan::SkillBrief {
                            id: s.id.clone(),
                            title: s.title.clone().unwrap_or_else(|| s.id.clone()),
                        })
                        .collect();
                    let drafted = self.drafter.as_mut().and_then(|d| d.draft(text, &top));
                    if let Some(plan) = drafted {
                        let checked = crate::plan::check(&plan, &|s| {
                            self.router.skill(s).map(crate::plan::SkillShape::of)
                        });
                        if let (Ok(()), Some(steps)) = (checked, crate::plan::sentences(&plan)) {
                            let skills: Vec<String> =
                                plan.steps.iter().map(|s| s.skill.clone()).collect();
                            return self.run_plan(id, &steps, &skills, p);
                        }
                    }
                }
                return self.fail(
                    id,
                    text,
                    None,
                    "I can't plan that right now. Try a simpler request.",
                );
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
        // A question that is not a plain "what is X": with a model plugged in, retrieve passages,
        // let it write from the best one, and show the answer only if the passage supports it.
        if skill_id == "web.fact"
            && self.llm.is_some()
            && !jitmo_connectors::web_lookup::is_lookup_question(text)
        {
            if let Some(event) = self.grounded_answer(id, text) {
                return event;
            }
        }
        // Weather with no place said and none saved: ask which city, then run it with
        // the answer, instead of failing with a message that quotes an example city.
        if skill_id == "web.weather"
            && jitmo_connectors::web_lookup::parse_weather(text)
                .place
                .is_none()
        {
            let saved = self
                .store
                .get_setting("default_place")
                .ok()
                .flatten()
                .is_some_and(|p| !p.trim().is_empty());
            if !saved {
                return match ask_text_twice(p, "Which city?", "For example: Pune") {
                    Some(city) => self.run(id, &format!("{text} in {city}"), p),
                    None => self.cancelled(id, text, Some(&skill_id)),
                };
            }
        }
        // The weekly report goes to the manager and nobody else: "send the report to Ved" is
        // not that request, whatever the router guessed.
        if skill_id == "work.send_report" {
            let l = text.to_lowercase();
            if !(l.contains("manager") || l.contains("boss")) {
                return self.fail(
                    id,
                    text,
                    Some(&skill_id),
                    "Who should get the report? I can send this week's report to your manager: say “send this week's report to my manager”.",
                );
            }
        }
        // "email the team": no file is named, so it is the plain email.
        let skill_id = {
            let l = text.to_lowercase();
            let filey = [
                ".xlsx",
                ".docx",
                ".pptx",
                ".pdf",
                ".csv",
                ".png",
                ".jpg",
                "file",
                "screenshot",
                "attachment",
                "photo",
                "picture",
                "document",
            ]
            .iter()
            .any(|w| l.contains(w));
            if skill_id == "files.email" && !filey {
                "mail.send".to_string()
            } else {
                skill_id
            }
        };
        // Typed values from the model, for skills whose connectors take them.
        let typed = self.typed_args(&skill_id, text);
        if self.llm_cancelled() {
            return self.cancelled(id, text, Some(&skill_id));
        }
        // Other guards of the same kind: a skill runs only when the words really ask for it,
        // so a guess by the router can never archive, move or decline something by accident.
        // With typed values the connector looks for the very meeting or invite the model named
        // and fails when there is none, so the keyword list is the rules' fallback only.
        if typed.is_none() {
            let l = text.to_lowercase();
            let any = |names: &[&str]| names.iter().any(|n| l.contains(n));
            let refuse = match skill_id.as_str() {
                "mail.archive_newsletters" if !l.contains("newsletter") => {
                    Some("I only archive newsletters. Say “archive today's newsletters”.")
                }
                "calendar.move"
                    if !any(&[
                        "move",
                        "reschedule",
                        "push",
                        "shift",
                        "postpone",
                        "change",
                        "bring",
                    ]) =>
                {
                    Some("To move a meeting, say “move my 3pm to tomorrow”.")
                }
                "calendar.decline"
                    if !any(&[
                        "decline",
                        "reject",
                        "turn down",
                        "can't make",
                        "cannot make",
                        "pass on",
                    ]) =>
                {
                    Some("To decline an invite, say “decline the Friday review”.")
                }
                _ => None,
            };
            if let Some(message) = refuse {
                return self.fail(id, text, Some(&skill_id), message);
            }
        }
        let Some(skill) = self.router.skill(&skill_id).cloned() else {
            return self.fail(id, text, Some(&skill_id), "Something went wrong.");
        };

        // 2. Fill slots. When the extractor found no person in the words but a saved contact
        // is named in them ("Email Rahul that ..."), that name is the contact.
        let mut spans = understanding.spans.clone();
        if let Some(values) = &typed {
            seed_spans(&skill, values, &mut spans);
        }
        add_named_contact(&skill, &mut spans, text, &known);
        let (mut slots, learned) = match resolve_slots(
            &skill,
            &spans,
            self.context.as_ref(),
            &known,
            &self.store,
            p,
        ) {
            Ok(s) => s,
            Err(SlotError::Message(msg)) => return self.fail(id, text, Some(&skill.id), &msg),
            Err(SlotError::Cancelled) => return self.cancelled(id, text, Some(&skill.id)),
        };
        slots.insert("request".to_string(), SlotValue::from_text(text));
        // The model's values, by their own names (a slot of the same name, resolved above, wins).
        if let Some(values) = &typed {
            for (name, value) in values {
                slots
                    .entry(name.clone())
                    .or_insert_with(|| SlotValue::from_text(value));
            }
        }

        // A send must show, and send, the words the user meant, never the whole sentence.
        if matches!(
            skill.id.as_str(),
            "mail.send" | "chat.post" | "mail.draft" | "whatsapp.send_message"
        ) {
            let typed_body = typed
                .as_ref()
                .and_then(|v| v.get("body"))
                .map(|b| tidy_body(b))
                .filter(|b| !b.is_empty());
            // The rules' reading first; the model's words only when the rules found none.
            let ruled = if skill.id == "chat.post" {
                crate::normalize::post_body(text)
            } else {
                crate::normalize::message_body(text)
            };
            let body = ruled.or(typed_body);
            match body {
                Some(body) => {
                    slots.insert("message".to_string(), SlotValue::from_text(&body));
                }
                None => {
                    return match ask_text_twice(
                        p,
                        "What should it say?",
                        "The exact words to send, e.g. Friday works",
                    ) {
                        Some(what) => self.run(id, &format!("{text} saying {what}"), p),
                        None => self.cancelled(id, text, Some(&skill.id)),
                    };
                }
            }
        }

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
        // Per-file slot sets that worked in the last multi-file step, and an
        // honest summary when only some of the files were done.
        let mut verified_slots: Vec<Slots> = Vec::new();
        let mut partial: Option<String> = None;
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
            // Several files selected: the same step runs once per file.
            let per_file = expand_multi(&step.args, &slots);
            let multi = per_file.is_some();
            let variants = per_file.unwrap_or_else(|| vec![slots.clone()]);
            let total = variants.len();
            let mut done_slots: Vec<Slots> = Vec::new();
            let mut first_err: Option<ConnError> = None;
            for one in variants {
                let args = match fill_args(&step.args, &one) {
                    Ok(a) => a,
                    Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
                };
                match conn.execute(&step.action, &args) {
                    Ok(out) => {
                        executed.push((step.connector.clone(), step.action.clone(), out));
                        done_slots.push(one);
                    }
                    // The app's screen looks different than expected, but an earlier
                    // step already got the user most of the way: hand over to them.
                    Err(ConnError::UiChanged(_))
                        if skill.fallback.as_deref() == Some("open_chat_and_wait")
                            && !executed.is_empty() =>
                    {
                        let who = slots
                            .get("contact")
                            .map_or("the".to_string(), |c| format!("{}'s", c.text));
                        let message = match skill.id.as_str() {
                            "whatsapp.send_file" => format!(
                                "I opened {who} chat and copied the file. Press Ctrl+V, then Send."
                            ),
                            "whatsapp.send_message" => {
                                format!("I opened {who} chat with your message. Press Send.")
                            }
                            _ => format!("I opened {who} chat. Press Send to finish."),
                        };
                        self.log(text, Some(&skill.id), Decision::Ran, &message, None);
                        return CoreEvent::Done {
                            id,
                            message,
                            undoable: false,
                        };
                    }
                    // One file failing must not undo or hide the others.
                    Err(e) if multi => {
                        first_err.get_or_insert(e);
                    }
                    Err(e) => {
                        let message = e.to_string();
                        // A connector that could not tell what was meant fails with a question
                        // ("Do you want it on or off?"): offer the answers as buttons instead of
                        // a dead end, then run the completed sentence.
                        if let Some((question, options)) =
                            crate::clarify::question_for(&message, text)
                        {
                            let choices: Vec<Choice> = options
                                .iter()
                                .enumerate()
                                .map(|(i, (label, _))| Choice {
                                    id: i.to_string(),
                                    label: (*label).into(),
                                })
                                .collect();
                            return match p
                                .choose(question, &choices)
                                .and_then(|c| c.parse::<usize>().ok())
                                .and_then(|i| options.get(i))
                            {
                                Some((_, sentence)) => self.run(id, sentence, p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        // Reminders and timers that are missing one part: ask for it.
                        if message.starts_with("How long should the timer run") {
                            return match self.ask_timer_length(p) {
                                Some(length) => {
                                    self.run(id, &format!("set a timer for {length}"), p)
                                }
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        if message.starts_with("What should I remind you about") {
                            return match ask_text_twice(
                                p,
                                "What should I remind you about?",
                                "For example: call mom",
                            ) {
                                Some(what) => self.run(id, &format!("{text} to {what}"), p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        // A mail or event that is missing one part: ask for it, then run the whole
                        // sentence again.
                        if message.starts_with("What should the email say") {
                            return match ask_text_twice(
                                p,
                                "What should it say?",
                                "For example: Friday works",
                            ) {
                                Some(what) => self.run(id, &format!("{text} saying {what}"), p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        if message.starts_with("For how long?") {
                            return match ask_text_twice(p, "For how long?", "For example: 2 hours")
                            {
                                Some(len) => self.run(id, &format!("{text} for {len}"), p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        if message.starts_with("Which day?") {
                            return match ask_text_twice(p, "Which day?", "For example: Thursday") {
                                Some(day) => self.run(id, &format!("{text} on {day}"), p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        if message.starts_with("Whose latest video") {
                            return match ask_text_twice(
                                p,
                                "Whose latest video?",
                                "A channel, e.g. MKBHD",
                            ) {
                                Some(who) => self.run(id, &format!("open {who}'s latest video"), p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        if message.starts_with("What should I search for") {
                            return match ask_text_twice(
                                p,
                                "What should I search for?",
                                "Anything you would type into a search box",
                            ) {
                                Some(q) => self.run(id, &format!("search the web for {q}"), p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        // "remind me at 3": morning or afternoon? Offered in words, with only
                        // the times a person would really mean (nobody sets 3 in the morning).
                        if message.starts_with("Which time did you mean") {
                            if let Some(options) = time_options(text, &message) {
                                let choices: Vec<Choice> = options
                                    .iter()
                                    .enumerate()
                                    .map(|(i, (label, _))| Choice {
                                        id: i.to_string(),
                                        label: label.clone(),
                                    })
                                    .collect();
                                return match p
                                    .choose("What time did you mean?", &choices)
                                    .and_then(|c| c.parse::<usize>().ok())
                                    .and_then(|i| options.get(i))
                                {
                                    Some((_, sentence)) => self.run(id, sentence, p),
                                    None => self.cancelled(id, text, Some(&skill.id)),
                                };
                            }
                        }
                        if message.starts_with("When should I remind you") {
                            let presets = [
                                ("In 1 hour", "in 1 hour"),
                                ("This evening (6 pm)", "at 6pm"),
                                ("Tomorrow, 9 am", "tomorrow at 9am"),
                            ];
                            let choices: Vec<Choice> = presets
                                .iter()
                                .enumerate()
                                .map(|(i, (label, _))| Choice {
                                    id: i.to_string(),
                                    label: (*label).into(),
                                })
                                .collect();
                            return match p
                                .choose("When should I remind you?", &choices)
                                .and_then(|c| c.parse::<usize>().ok())
                                .and_then(|i| presets.get(i))
                            {
                                Some((_, when)) => self.run(id, &format!("{text} {when}"), p),
                                None => self.cancelled(id, text, Some(&skill.id)),
                            };
                        }
                        return self.fail(id, text, Some(&skill.id), &message);
                    }
                }
            }
            if multi {
                if done_slots.is_empty() {
                    let e =
                        first_err.map_or("Something went wrong.".to_string(), |e| e.to_string());
                    return self.fail(id, text, Some(&skill.id), &e);
                }
                if done_slots.len() < total {
                    let verb = if step.action == "move" {
                        "Moved"
                    } else {
                        "Did"
                    };
                    partial = Some(format!("{verb} {} of {total} files.", done_slots.len()));
                }
                verified_slots = done_slots;
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
            // After a multi-file step, each file that was done is checked.
            let to_check = if verified_slots.is_empty() {
                vec![slots.clone()]
            } else {
                std::mem::take(&mut verified_slots)
            };
            for one in &to_check {
                let args = match fill_args(&check.args, one) {
                    Ok(a) => a,
                    Err(msg) => return self.fail(id, text, Some(&skill.id), &msg),
                };
                // No internal connector name in what the user reads.
                let msg =
                    "I couldn't confirm that it worked. Please check it yourself.".to_string();
                if let Verdict::NotVerified { message } =
                    verify(conn.as_ref(), &check.action, &args, &msg)
                {
                    return self.fail(id, text, Some(&skill.id), &message);
                }
            }
        }

        // 6. Log and report. The action already happened and was verified, so a
        //    log failure is a warning on a successful result, never an error.
        // What the user picked by words is remembered now that it worked (FR10).
        let at = (self.clock)();
        for l in &learned {
            if let Err(e) = self
                .store
                .record_default(&l.slot_type, &l.phrase, &l.value, &at)
            {
                eprintln!("could not save a learned default: {e}");
            }
        }
        // A person who was used successfully joins the contact list (a test skill's
        // pretend send does not count). Groups ("ved and rahul") are not one person.
        if skill.category != "testing" {
            for l in learned.iter().filter(|l| l.slot_type == "contact") {
                if people_in(&l.value) == 1 {
                    if let Err(e) = self.store.remember_contact(&display_name(&l.value), &at) {
                        eprintln!("could not save a contact: {e}");
                    }
                }
            }
        }
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
        let base = partial.or(answer).unwrap_or_else(|| "Done.".to_string());
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
        // The skill's own title, never an example phrase: an example names a
        // person or a song the user never typed.
        self.router
            .skill(skill_id)
            .and_then(|s| s.title.clone())
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

/// A saved contact whose first or full name appears in the request, as a `contact` span,
/// when the skill wants a contact and the extractor found none.
fn add_named_contact(skill: &Skill, spans: &mut Vec<Span>, text: &str, known: &Known) {
    let wants = skill
        .slots
        .values()
        .any(|s| matches!(s, Slot::Param(sp) if sp.kind == "contact"));
    if !wants || spans.iter().any(|s| s.kind == "contact") {
        return;
    }
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    for c in &known.contacts {
        let Some(first) = c.name.split_whitespace().next() else {
            continue;
        };
        if words.iter().any(|w| *w == first.to_lowercase()) {
            spans.push(Span {
                text: first.to_string(),
                kind: "contact".into(),
            });
            return;
        }
    }
}

/// Fill each slot from the spans the router found (first unused span of the
/// slot's type), or from the skill's fixed value. Real things (files) come from
/// the resolver; if several fit equally the user is asked one question (FR4).
fn resolve_slots(
    skill: &Skill,
    spans: &[Span],
    ctx: &dyn ContextSource,
    known: &Known,
    store: &Store,
    p: &mut dyn Prompter,
) -> Result<(Slots, Vec<Learned>), SlotError> {
    let mut used = vec![false; spans.len()];
    let mut out = Slots::new();
    let mut learned = Vec::new();
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
                let mut said = found.map(|(i, s)| {
                    used[i] = true;
                    s.text.as_str()
                });
                // A setting wins; otherwise the value chosen most often before.
                let usual = store
                    .get_setting(&format!("usual.{}", sp.kind))
                    .ok()
                    .flatten()
                    .or_else(|| store.usual_for(&sp.kind).ok().flatten());
                let mut resolution =
                    resolver::resolve_with(&sp.resolve, said, ctx, usual.as_deref(), known);
                // A required free-text slot with no words at all: ask once instead of
                // failing. The answer is then handled as if it had been said.
                let answer;
                if sp.required
                    && sp.is_free_text()
                    && matches!(resolution, Resolution::NotFound(None))
                {
                    answer = ask_for_text(p, &sp.question(name))?;
                    said = Some(answer.as_str());
                    resolution =
                        resolver::resolve_with(&sp.resolve, said, ctx, usual.as_deref(), known);
                }
                match resolution {
                    Resolution::Found(v) => {
                        // A set of files is not one thing to remember.
                        if let (Some(words), false) = (said, v.fields.contains_key("paths")) {
                            learned.push(Learned::new(&sp.kind, words, &v));
                        }
                        out.insert(name.clone(), v);
                    }
                    Resolution::Choose { question, options } => {
                        // The same words were settled before: do not ask again.
                        let remembered = said
                            .and_then(|w| {
                                store
                                    .default_for_phrase(&sp.kind, &w.to_lowercase())
                                    .ok()
                                    .flatten()
                            })
                            .and_then(|val| {
                                options
                                    .iter()
                                    .find(|(id, _, v)| *id == val || v.text == val)
                                    .map(|o| o.2.clone())
                            });
                        if let Some(v) = remembered {
                            out.insert(name.clone(), v);
                            continue;
                        }
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
                        if let Some(words) = said {
                            learned.push(Learned::new(&sp.kind, words, &v));
                        }
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
    Ok((out, learned))
}

/// Ask the question; an empty answer is asked once more, then the task is cancelled.
/// Dismissing (`None`) cancels at once. Returns the trimmed, non-empty words.
fn ask_for_text(p: &mut dyn Prompter, question: &str) -> Result<String, SlotError> {
    let hints = [
        "Type your answer, or Esc to cancel",
        "Type something, or Esc to cancel",
    ];
    for hint in hints {
        let said = p.ask_text(question, hint).ok_or(SlotError::Cancelled)?;
        let said = said.trim();
        if !said.is_empty() {
            return Ok(said.to_string());
        }
    }
    Err(SlotError::Cancelled)
}

/// What the assistant can do today (only things that are built).
const HELP_MESSAGE: &str = "I can open apps, play and pause music, set timers and reminders, \
do quick math, find and move your files, change volume, brightness and Wi-Fi, take a screenshot, \
lock the PC, check the weather and answer quick facts. Say 'what timers do I have' or 'stop the \
timer' to see or stop them. Try: 'open Spotify', '10-minute timer', '18% of 2,450'.";

/// A typed answer; a blank one is asked for once more, then given up on.
/// "chrome" -> "Chrome", for a button label.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

fn ask_text_twice(p: &mut dyn Prompter, question: &str, hint: &str) -> Option<String> {
    for _ in 0..2 {
        let a = p.ask_text(question, hint)?;
        if !a.trim().is_empty() {
            return Some(a.trim().to_string());
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerKind {
    Timers,
    Reminders,
    Both,
}

impl TimerKind {
    fn noun(self) -> &'static str {
        match self {
            TimerKind::Timers => "timers",
            TimerKind::Reminders => "reminders",
            TimerKind::Both => "timers or reminders",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimerCommand {
    /// Stop them (else just list).
    cancel: bool,
    /// "all" was said: no question when several match.
    all: bool,
    kind: TimerKind,
}

/// "what timers do I have", "how much time is left", "stop the timer",
/// "cancel all reminders". Anything naming a time or a task ("cancel my 6pm
/// meeting") is left to the router.
fn timer_command(text: &str) -> Option<TimerCommand> {
    let t = text.trim().trim_end_matches(['?', '!', '.']).to_lowercase();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |names: &[&str]| words.iter().any(|w| names.contains(w));
    let timers = has(&["timer", "timers", "alarm", "alarms", "stopwatch"]);
    let reminders = has(&["reminder", "reminders"]);
    let time_left = t.contains("time left") || t.contains("time is left");
    if !timers && !reminders && !time_left {
        return None;
    }
    let first = words.first().copied().unwrap_or("");
    let cancel = matches!(
        first,
        "stop" | "cancel" | "clear" | "delete" | "remove" | "end" | "kill" | "dismiss" | "forget"
    ) || t.starts_with("turn off")
        || t.starts_with("get rid of");
    let list = matches!(
        first,
        "what" | "which" | "show" | "list" | "any" | "check" | "do" | "how" | "my" | "whats"
    ) || time_left
        || matches!(words.as_slice(), ["timers"] | ["reminders"] | ["alarms"]);
    if !cancel && !list {
        return None;
    }
    // A clock time or a task in the sentence means a different request.
    let other = words.iter().any(|w| {
        w.chars().next().is_some_and(|c| c.is_ascii_digit())
            || matches!(
                *w,
                "at" | "to" | "for" | "set" | "make" | "create" | "add" | "new" | "remind"
            )
    });
    if other && !time_left {
        return None;
    }
    let kind = match (timers || time_left, reminders) {
        (true, false) => TimerKind::Timers,
        (false, true) => TimerKind::Reminders,
        _ => TimerKind::Both,
    };
    Some(TimerCommand {
        cancel,
        all: has(&["all", "every", "everything"]),
        kind,
    })
}

/// "start a timer", "set timer", "new timer", "timer", "begin a timer please": a
/// request to set a timer that names no length.
fn is_timer_without_length(text: &str) -> bool {
    let t = text.trim().trim_end_matches(['?', '!', '.']).to_lowercase();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if !words.contains(&"timer") {
        return false;
    }
    let has_length = words.iter().any(|w| {
        w.chars().any(|c| c.is_ascii_digit())
            || matches!(
                *w,
                "hour"
                    | "hours"
                    | "hr"
                    | "hrs"
                    | "minute"
                    | "minutes"
                    | "min"
                    | "mins"
                    | "second"
                    | "seconds"
                    | "sec"
                    | "secs"
                    | "half"
                    | "quarter"
                    | "one"
                    | "two"
                    | "three"
                    | "four"
                    | "five"
                    | "six"
                    | "seven"
                    | "eight"
                    | "nine"
                    | "ten"
                    | "fifteen"
                    | "twenty"
                    | "thirty"
                    | "forty"
                    | "sixty"
                    | "ninety"
            )
    });
    if has_length {
        return false;
    }
    // Only the plain "start/set/new timer" family; "stop the timer" and the like
    // are handled earlier, and anything else with extra words goes to the router.
    let fillers = [
        "start", "set", "begin", "create", "make", "add", "new", "a", "an", "the", "my", "please",
        "pls", "timer", "can", "you", "could", "i", "want", "need", "to", "me", "a", "run", "put",
        "on", "launch", "kick", "off", "up",
    ];
    words.iter().all(|w| fillers.contains(w))
}

/// "7 min 30 s", "2 h 5 min".
fn time_left(d: chrono::Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        return format!("{secs} s");
    }
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h} h {m} min")
    } else if m < 10 && s > 0 {
        format!("{m} min {s} s")
    } else {
        format!("{m} min")
    }
}

/// The ways to read a bare hour ("remind me at 3"), as (button, complete sentence) pairs.
/// `message` ends with the hour the reminder code found, "(3:00)". Hours 1 to 4 can only
/// mean the afternoon; 5 to 11 can be morning or later, named in words ("6 in the evening")
/// and never "am" or "pm".
fn time_options(said: &str, message: &str) -> Option<Vec<(String, String)>> {
    let inner = message.get(message.rfind('(')? + 1..message.rfind(')')?)?;
    // "3:00" or "6:00 pm only" (the morning one has already gone by today).
    let pm_only = inner.ends_with("pm only");
    let inner = inner.trim_end_matches(" pm only");
    let (h, m) = inner.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    if !(1..=11).contains(&h) {
        return None;
    }
    let shown = if m == 0 {
        h.to_string()
    } else {
        format!("{h}:{m:02}")
    };
    // The token in the sentence that holds the time, preferring one after "at".
    let tokens: Vec<&str> = said.split_whitespace().collect();
    let is_time = |t: &str| {
        let t = t.trim_matches(|c: char| !c.is_alphanumeric() && c != ':');
        t == shown || (m == 0 && t == format!("{h}:00"))
    };
    let at = tokens
        .iter()
        .position(|t| is_time(t))
        .and_then(|i| {
            let after_at = i > 0
                && matches!(
                    tokens[i - 1].to_lowercase().as_str(),
                    "at" | "until" | "till" | "for" | "by"
                );
            after_at.then_some(i)
        })
        .or_else(|| tokens.iter().position(|t| is_time(t)))?;
    let with = |suffix: &str| {
        let mut t: Vec<String> = tokens.iter().map(|s| s.to_string()).collect();
        t[at] = format!("{shown}{suffix}");
        t.join(" ")
    };
    let part = |h24: u32| match h24 {
        0..=11 => "in the morning",
        12..=16 => "in the afternoon",
        17..=20 => "in the evening",
        _ => "at night",
    };
    let mut out = Vec::new();
    if h >= 5 && !pm_only {
        out.push((format!("{shown} {}", part(h)), with("am")));
    }
    out.push((format!("{shown} {}", part(h + 12)), with("pm")));
    Some(out)
}

/// "help", "what can you do", "what can jitmo do" and close variants.
fn is_help_request(text: &str) -> bool {
    let t: String = text
        .trim()
        .trim_end_matches(['?', '!', '.'])
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    matches!(
        t.as_str(),
        "help"
            | "help me"
            | "what can you do"
            | "what can u do"
            | "what can i do"
            | "what can i ask"
            | "what can i ask you"
            | "what can jitmo do"
            | "what do you do"
            | "what are you capable of"
            | "what are your abilities"
            | "how does this work"
            | "what can you even do"
            | "what can it do"
    )
}

/// A choice made by words, to remember if the task succeeds (FR10).
struct Learned {
    slot_type: String,
    phrase: String,
    value: String,
}

impl Learned {
    fn new(slot_type: &str, phrase: &str, v: &SlotValue) -> Self {
        Learned {
            slot_type: slot_type.to_string(),
            phrase: phrase.to_lowercase(),
            // Files are remembered by path, so the same words find the same file.
            value: v.fields.get("path").unwrap_or(&v.text).clone(),
        }
    }
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

/// When a step's arguments use `{slot.path}` of a slot holding several files,
/// one copy of the slots per file, with that slot narrowed to the one file.
/// `None` for an ordinary step.
fn expand_multi(args: &BTreeMap<String, String>, slots: &Slots) -> Option<Vec<Slots>> {
    let (name, value) = slots.iter().find(|(name, v)| {
        v.fields.contains_key("paths") && {
            let key = format!("{{{name}.path}}");
            args.values().any(|a| a.contains(&key))
        }
    })?;
    let variants: Vec<Slots> = value
        .paths()
        .into_iter()
        .map(|path| {
            let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
            let mut one = slots.clone();
            one.insert(
                name.clone(),
                SlotValue {
                    text: file.to_string(),
                    fields: BTreeMap::from([
                        ("name".to_string(), file.to_string()),
                        ("path".to_string(), path.to_string()),
                    ]),
                },
            );
            one
        })
        .collect();
    (!variants.is_empty()).then_some(variants)
}

/// A step's arguments with the slots filled in. An optional typed value (`{name?}`) that
/// nothing filled is left out, so the connector reads the request as before.
fn fill_args(args: &BTreeMap<String, String>, slots: &Slots) -> Result<Args, String> {
    let mut out = Args::new();
    for (k, v) in args {
        if let Some(name) = jitmo_skills::optional_placeholder(v) {
            if let Some(value) = slots.get(name) {
                out.insert(k.clone(), value.text.clone());
            }
            continue;
        }
        out.insert(k.clone(), fill(v, slots)?);
    }
    Ok(out)
}

/// Is a model's value copied from the request? At least half of its words (and at least one)
/// must be in the request, allowing a typo-sized difference ("emial" for "email") and a
/// number written as digits or word ("10" or "ten" against "10-minute").
fn grounded_in(value: &str, request: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let said = words(request);
    let close = |a: &str, b: &str| {
        a == b
            || (a.len() >= 4 && b.len() >= 4 && (a.starts_with(&b[..4]) || b.starts_with(&a[..4])))
            || number_word(a).is_some_and(|n| n == b)
            || number_word(b).is_some_and(|n| n == a)
    };
    // Little words carry no evidence either way.
    const FILLER: &[&str] = &[
        "the", "a", "an", "to", "of", "my", "for", "in", "on", "at", "me",
    ];
    let mine: Vec<String> = words(value)
        .into_iter()
        .filter(|w| !FILLER.contains(&w.as_str()))
        .collect();
    if mine.is_empty() {
        return !words(value).is_empty() && words(value).iter().all(|w| said.contains(w));
    }
    let found = mine
        .iter()
        .filter(|w| said.iter().any(|s| close(w, s)))
        .count();
    found * 2 >= mine.len() && found > 0
}

/// "ten" as "10", for the words people use for small numbers.
fn number_word(w: &str) -> Option<String> {
    const N: &[&str] = &[
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve", "fifteen", "twenty", "thirty", "forty", "fifty", "sixty",
    ];
    const V: &[u32] = &[
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 15, 20, 30, 40, 50, 60,
    ];
    N.iter().position(|n| *n == w).map(|i| V[i].to_string())
}

/// A request worded as a question ("how do vaccines work", "why is the sky blue?").
fn is_question(text: &str) -> bool {
    const STARTS: &[&str] = &[
        "how ",
        "why ",
        "what ",
        "what's ",
        "whats ",
        "who ",
        "when ",
        "where ",
        "which ",
        "explain ",
        "is ",
        "are ",
        "does ",
        "do ",
        "can ",
        "tell me about ",
    ];
    let t = text.trim().to_lowercase();
    t.ends_with('?') || STARTS.iter().any(|s| t.starts_with(s))
}

/// One skill as the model sees it.
fn tool_brief(s: &Skill) -> crate::llm::ToolBrief {
    crate::llm::ToolBrief {
        id: s.id.clone(),
        title: s.title.clone().unwrap_or_else(|| s.id.clone()),
        examples: s.examples.clone(),
        args: s
            .tool
            .as_ref()
            .map(|t| {
                t.args
                    .iter()
                    .map(|(n, a)| crate::llm::ArgBrief {
                        name: n.clone(),
                        description: a.description.clone(),
                        required: a.required,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Put the model's values for people, apps and files first among the spans, so the resolver
/// turns them into a real contact, app or file (and asks when several fit), exactly as it does
/// for words the extractor found. A tool argument feeds the slot of the same name, or the
/// contact slot when the argument is a person.
fn seed_spans(skill: &Skill, values: &BTreeMap<String, String>, spans: &mut Vec<Span>) {
    let Some(tool) = &skill.tool else { return };
    for (name, value) in values {
        let kind = match skill.slots.get(name) {
            Some(Slot::Param(sp)) => Some(sp.kind.clone()),
            _ => tool
                .args
                .get(name)
                .filter(|a| a.kind == "person")
                .and_then(|_| {
                    skill.slots.values().find_map(|s| match s {
                        Slot::Param(sp) if sp.kind == "contact" => Some(sp.kind.clone()),
                        _ => None,
                    })
                }),
        };
        if let Some(kind) = kind {
            spans.retain(|s| s.kind != kind);
            spans.insert(
                0,
                Span {
                    text: value.clone(),
                    kind,
                },
            );
        }
    }
}

/// The words to send, tidied the way the rules tidy them: a capital first letter and a full
/// stop when the sentence has no ending of its own.
fn tidy_body(body: &str) -> String {
    let t = body.trim().trim_matches(['"', '“', '”']).trim();
    let mut c = t.chars();
    let Some(first) = c.next() else {
        return String::new();
    };
    let mut out = format!("{}{}", first.to_uppercase(), c.as_str());
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
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

/// A name the router lowercased ("ved narvekar") is saved as people write it
/// ("Ved Narvekar"). A name with any capital letter is kept as it is.
fn display_name(name: &str) -> String {
    if name.chars().any(char::is_uppercase) {
        return name.trim().to_string();
    }
    name.split_whitespace()
        .map(|w| {
            let mut cs = w.chars();
            cs.next()
                .map(|f| f.to_uppercase().chain(cs).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    const PAUSE: &str = include_str!("../../../skills/music.control.yaml");
    const SEND: &str = include_str!("../../../skills/stub.send.yaml");

    /// Scripted user: answers confirms from a list and records what it was shown.
    #[derive(Default)]
    struct TestUser {
        confirm_answers: Vec<bool>,
        shown: Vec<String>,
        choice: Option<String>,
        choices_seen: Vec<String>,
        text_answers: Vec<String>,
        questions: Vec<String>,
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
        fn ask_text(&mut self, q: &str, _hint: &str) -> Option<String> {
            self.questions.push(q.to_string());
            if self.text_answers.is_empty() {
                None
            } else {
                Some(self.text_answers.remove(0))
            }
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

    // --- asking for a missing free-text slot ------------------------------------

    /// The stub send skill with an extra way to say it that leaves the text out.
    fn without_ask(yaml: &str) -> String {
        let kept: Vec<&str> = yaml
            .lines()
            .filter(|l| !l.trim_start().starts_with("ask:"))
            .collect();
        kept.join(
            "
",
        ) + "
"
    }

    fn send_without_text() -> String {
        SEND.replace("examples:\n", "examples:\n  - \"message {contact}\"\n")
    }

    #[test]
    fn a_missing_text_is_asked_for_and_used_as_if_said() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[&send_without_text()], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        u.text_answers = vec!["  see you at six ".into()];
        let ev = flow.handle(60, "message ved", &mut u);
        assert_eq!(u.questions, vec!["What do you want to say?"]);
        assert_eq!(u.shown, vec!["Send \"see you at six\" to ved?"]);
        assert_eq!(done(&ev).0, "Done.");
        assert_eq!(m.execute_count(), 1);
    }

    #[test]
    fn the_default_question_is_used_without_an_ask_line() {
        let yaml = without_ask(&send_without_text());
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[&yaml], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        u.text_answers = vec!["hi".into()];
        flow.handle(61, "message ved", &mut u);
        assert_eq!(u.questions, vec!["What should it say?"]);
    }

    #[test]
    fn an_empty_answer_is_asked_once_more_then_the_second_is_used() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[&send_without_text()], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        u.text_answers = vec!["   ".into(), "hello".into()];
        let ev = flow.handle(62, "message ved", &mut u);
        assert_eq!(u.questions.len(), 2);
        assert_eq!(u.shown, vec!["Send \"hello\" to ved?"]);
        assert_eq!(done(&ev).0, "Done.");
    }

    #[test]
    fn two_empty_answers_cancel_and_nothing_runs() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[&send_without_text()], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        u.text_answers = vec!["".into(), " ".into(), "late".into()];
        let ev = flow.handle(63, "message ved", &mut u);
        assert_eq!(done(&ev).0, "Cancelled.");
        assert_eq!(u.questions.len(), 2, "asks once more, not a third time");
        assert!(u.shown.is_empty() && m.calls().is_empty());
        assert_eq!(log_decisions(&flow), vec!["cancelled"]);
    }

    #[test]
    fn dismissing_the_question_cancels() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[&send_without_text()], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        let ev = flow.handle(64, "message ved", &mut u);
        assert_eq!(done(&ev).0, "Cancelled.");
        assert_eq!(u.questions.len(), 1);
        assert!(m.calls().is_empty());
    }

    #[test]
    fn a_slot_resolved_from_a_list_is_never_asked_in_words() {
        // The same skill with its text slot turned into a file slot: it still fails
        // with the old message and never shows a question.
        let yaml = without_ask(&send_without_text()).replace("type: text", "type: file_ref");
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[&yaml], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        u.text_answers = vec!["x".into()];
        let ev = flow.handle(65, "message ved", &mut u);
        assert!(u.questions.is_empty());
        assert_eq!(failed(&ev), "I'm missing the text.");
    }

    #[test]
    fn a_text_said_in_the_request_is_not_asked_again() {
        let m = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[&send_without_text()], vec![("stub_messenger", m.clone())]);
        let mut u = user(&[true]);
        flow.handle(66, "send hello to ved", &mut u);
        assert!(u.questions.is_empty());
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
    fn timer_commands_are_recognised_narrowly() {
        let c = |t| timer_command(t);
        let stop = c("stop the timer").unwrap();
        assert!(stop.cancel && stop.kind == TimerKind::Timers && !stop.all);
        assert!(c("cancel all my reminders").unwrap().all);
        let list = c("what timers do I have?").unwrap();
        assert!(!list.cancel && list.kind == TimerKind::Timers);
        assert!(c("show me my reminders").is_some());
        assert!(c("how much time is left on my timer").is_some());
        assert!(c("timers").is_some());
        // Other requests stay with the router.
        assert_eq!(c("set a 10 minute timer"), None);
        assert_eq!(c("remind me to stop the timer at 6"), None);
        assert_eq!(c("cancel my 6pm reminder"), None);
        assert_eq!(c("open spotify"), None);
        assert_eq!(c("stop the music"), None);
    }

    #[test]
    fn stopping_and_listing_timers_works_from_the_saved_reminders() {
        let mut flow = flow_with(&[OPEN], vec![]);
        let base = chrono::DateTime::parse_from_rfc3339(&(flow.clock)()).unwrap();
        let in_ten = (base + chrono::Duration::minutes(10))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let in_hour = (base + chrono::Duration::hours(1))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let say = |flow: &mut Flow, id, q: &str, pick: Option<&str>| match flow.handle(
            id,
            q,
            &mut TestUser {
                choice: pick.map(String::from),
                ..TestUser::default()
            },
        ) {
            CoreEvent::Done { message, .. } => message,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            say(&mut flow, 1, "what timers do I have", None),
            "You have no timers set."
        );
        flow.store.add_reminder(&in_ten, "Timer").unwrap();
        let msg = say(&mut flow, 2, "what timers do I have", None);
        assert!(
            msg.starts_with("Timer: 9 min") || msg.starts_with("Timer: 10 min"),
            "{msg}"
        );
        flow.store.add_reminder(&in_hour, "Timer").unwrap();
        flow.store.add_reminder(&in_hour, "call mom").unwrap();
        // Two timers and no "all": it asks which one.
        let msg = say(&mut flow, 3, "stop the timer", Some("all"));
        assert_eq!(msg, "Stopped 2 timers.");
        // The reminder is untouched by "stop the timer".
        let left = flow.store.pending_reminders().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].text, "call mom");
        assert!(say(&mut flow, 4, "cancel the reminder", None).starts_with("Stopped."));
        assert!(flow.store.pending_reminders().unwrap().is_empty());
    }

    #[test]
    fn weather_with_no_place_asks_which_city_then_uses_the_answer() {
        const WEATHER: &str = include_str!("../../../skills/web.weather.yaml");
        let web = Arc::new(FakeConnector::new("web_lookup"));
        let mut flow = flow_with(&[WEATHER], vec![("web_lookup", web.clone())]);
        let mut u = TestUser {
            text_answers: vec!["Pune".into()],
            // The rule-based router in this test may ask which skill once the city is added.
            choice: Some("web.weather".into()),
            ..TestUser::default()
        };
        let ev = flow.handle(40, "will it rain tomorrow", &mut u);
        assert_eq!(u.questions, ["Which city?"]);
        assert!(matches!(ev, CoreEvent::Done { .. }), "{ev:?}");
        let Some(Call::Execute { args, .. }) = web
            .calls()
            .into_iter()
            .find(|c| matches!(c, Call::Execute { .. }))
        else {
            panic!("the lookup must run with the answer")
        };
        assert!(args.values().any(|v| v.contains("in Pune")), "{args:?}");

        // A place already in the sentence: no question.
        let mut u = TestUser::default();
        let _ = flow.handle(41, "what's the weather in Delhi this weekend", &mut u);
        assert!(u.questions.is_empty());

        // A saved city: no question either.
        flow.store.set_setting("default_place", "Mumbai").unwrap();
        let mut u = TestUser::default();
        let _ = flow.handle(42, "will it rain tomorrow", &mut u);
        assert!(u.questions.is_empty());
    }

    #[test]
    fn a_bare_command_asks_what_is_missing_then_runs_the_completed_sentence() {
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", sys.clone())]);
        // "open" alone: "Which app?" -> spotify -> "open spotify" runs.
        let mut u = TestUser {
            text_answers: vec!["spotify".into()],
            ..TestUser::default()
        };
        let ev = flow.handle(30, "open", &mut u);
        assert_eq!(u.questions, ["Which app?"]);
        assert!(matches!(ev, CoreEvent::Done { .. }), "{ev:?}");
        let ran = sys.calls().len();
        assert!(ran >= 1, "the completed sentence must run");

        // Dismissing the question runs nothing.
        let mut u = TestUser::default();
        let ev = flow.handle(31, "open", &mut u);
        assert!(
            matches!(ev, CoreEvent::Done { ref message, .. } if message == "Cancelled."),
            "{ev:?}"
        );
        assert_eq!(sys.calls().len(), ran, "nothing more may run");

        // A choice question: the options are offered by position.
        let mut u = TestUser::default();
        let _ = flow.handle(32, "wifi", &mut u);
        assert_eq!(u.choices_seen, ["0", "1"]);
    }

    #[test]
    fn a_timer_without_a_length_asks_how_long() {
        assert!(is_timer_without_length("start a timer"));
        assert!(is_timer_without_length("Set timer"));
        assert!(is_timer_without_length("timer please"));
        assert!(!is_timer_without_length("set a 10 minute timer"));
        assert!(!is_timer_without_length("timer for an hour"));
        assert!(!is_timer_without_length("stop the timer"));
        assert!(!is_timer_without_length("open spotify"));

        // A preset is turned into a normal timer request (no timer connector in this
        // flow, so the request itself is what we look at: it must reach the router).
        let mut flow = flow_with(&[OPEN], vec![]);
        let mut u = TestUser {
            choice: Some("10".into()),
            ..TestUser::default()
        };
        let ev = flow.handle(5, "start a timer", &mut u);
        assert_eq!(u.choices_seen, ["5", "10", "15", "30", "60", "custom"]);
        // No timer skill is loaded here, so it is declined after asking.
        assert!(matches!(ev, CoreEvent::Failed { .. }), "{ev:?}");

        // Dismissing the question cancels.
        let mut u = TestUser::default();
        let ev = flow.handle(6, "start a timer", &mut u);
        assert!(
            matches!(ev, CoreEvent::Done { ref message, .. } if message == "Cancelled."),
            "{ev:?}"
        );
    }

    #[test]
    fn what_can_you_do_lists_the_abilities() {
        let mut flow = flow_with(&[OPEN], vec![]);
        for q in ["what can you do?", "  Help ", "What can you even do"] {
            match flow.handle(7, q, &mut user(&[])) {
                CoreEvent::Done {
                    message, undoable, ..
                } => {
                    assert!(message.contains("open apps"), "{message}");
                    assert!(!undoable);
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(!is_help_request("help me find my invoice"));
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
        assert!(sys.calls().is_empty());
        // "A and then B" runs A, then B, in order.
        flow.handle(10, "open spotify and then pause the music", &mut user(&[]));
        // Spotify is opened (execute + check); "pause the music" is not taken for an app.
        assert_eq!(sys.calls().len(), 2, "{:?}", sys.calls());
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
            choice: Some("music.control".into()),
            ..TestUser::default()
        };
        assert_eq!(done(&flow.handle(13, "pause music", &mut u)).0, "Done.");
        assert_eq!(u.choices_seen, vec!["music.control"]);
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

    /// Three PDFs selected in Explorer, plus an Invoices folder.
    struct Selected3(Vec<crate::resolver::FileRef>);

    impl ContextSource for Selected3 {
        fn selected_file(&self) -> Option<crate::resolver::FileRef> {
            self.0.first().cloned()
        }
        fn selected_files(&self) -> Vec<crate::resolver::FileRef> {
            self.0.clone()
        }
        fn recent_downloads(&self, _: usize) -> Vec<crate::resolver::FileRef> {
            Vec::new()
        }
        fn searchable_files(&self) -> Vec<crate::resolver::FileRef> {
            Vec::new()
        }
        fn searchable_folders(&self) -> Vec<crate::resolver::FileRef> {
            vec![file_ref("Invoices")]
        }
    }

    fn multi_move_flow(files: Arc<FakeConnector>) -> Flow {
        let selected = ["a.pdf", "b.pdf", "c.pdf"].map(file_ref).to_vec();
        flow_with(&[MOVE], vec![("files", files)]).with_context(Selected3(selected))
    }

    fn moved_paths(files: &FakeConnector) -> Vec<String> {
        files
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Execute { action, args } if action == "move" => args.get("path").cloned(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn t26_several_selected_files_move_one_by_one_under_one_request() {
        let files = Arc::new(FakeConnector::new("files").undoable());
        let mut flow = multi_move_flow(files.clone());
        let mut u = user(&[]);
        let ev = flow.handle(60, "Move these PDFs to the Invoices folder.", &mut u);
        assert!(u.shown.is_empty());
        assert_eq!(done(&ev), ("Done.", true));
        assert_eq!(files.execute_count(), 3);
        assert_eq!(
            moved_paths(&files),
            ["C:/Docs/a.pdf", "C:/Docs/b.pdf", "C:/Docs/c.pdf"]
        );
        assert_eq!(files.check_count(), 3, "each file is verified");
        assert_eq!(
            log_decisions(&flow),
            vec!["ran"],
            "one request, one log row"
        );
        assert_eq!(flow.undo(), "Undone.");
        let undone = files
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::Undo { .. }))
            .count();
        assert_eq!(undone, 3);
    }

    #[test]
    fn a_failed_file_gives_an_honest_partial_result_and_keeps_undo() {
        let files = Arc::new(
            FakeConnector::new("files")
                .undoable()
                .fail_nth_execute(1, ConnError::Failed("Couldn't move b.pdf.".into())),
        );
        let mut flow = multi_move_flow(files.clone());
        let ev = flow.handle(61, "move these pdfs to the Invoices folder", &mut user(&[]));
        assert_eq!(done(&ev), ("Moved 2 of 3 files.", true));
        assert_eq!(files.execute_count(), 3, "the rest still run");
        assert_eq!(files.check_count(), 2, "only moved files are verified");
        assert_eq!(flow.undo(), "Undone.");
        let undone = files
            .calls()
            .iter()
            .filter(|c| matches!(c, Call::Undo { .. }))
            .count();
        assert_eq!(undone, 2);
    }

    #[test]
    fn if_every_file_fails_the_request_fails_with_the_first_reason() {
        let files = Arc::new(
            FakeConnector::new("files").fail_execute_with(ConnError::Failed("No room.".into())),
        );
        let mut flow = multi_move_flow(files.clone());
        let ev = flow.handle(62, "move these pdfs to the Invoices folder", &mut user(&[]));
        assert_eq!(failed(&ev), "No room.");
        assert_eq!(flow.undo(), "Nothing to undo.");
    }

    #[test]
    fn a_single_selected_file_still_runs_once_and_says_done() {
        let files = Arc::new(FakeConnector::new("files").undoable());
        let mut flow = flow_with(&[MOVE], vec![("files", files.clone())])
            .with_context(Selected3(vec![file_ref("a.pdf")]));
        let ev = flow.handle(63, "move these to the Invoices folder", &mut user(&[]));
        assert_eq!(done(&ev), ("Done.", true));
        assert_eq!(moved_paths(&files), ["C:/Docs/a.pdf"]);
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
    fn a_choice_is_remembered_so_the_same_words_are_not_asked_twice() {
        let (mut flow, wa) = whatsapp_flow(Files {
            selected: None,
            files: vec![file_ref("Invoice March.pdf"), file_ref("Invoice April.pdf")],
        });
        let mut first = user(&[true]);
        first.choice = Some("C:/Docs/Invoice April.pdf".into());
        flow.handle(50, "send invoice to ved on whatsapp", &mut first);
        assert_eq!(first.choices_seen.len(), 2, "the first time it asks");
        // Same words again: the answer is remembered, no question.
        let mut second = user(&[true]);
        let ev = flow.handle(51, "send invoice to ved on whatsapp", &mut second);
        assert!(second.choices_seen.is_empty(), "it must not ask again");
        assert_eq!(
            second.shown,
            vec!["Send Invoice April.pdf to Ved on WhatsApp?"]
        );
        assert_eq!(done(&ev).0, "Done.");
        assert_eq!(wa.execute_count(), 4);
    }

    #[test]
    fn a_choice_is_not_remembered_when_the_task_did_not_finish() {
        let (mut flow, _wa) = whatsapp_flow(Files {
            selected: None,
            files: vec![file_ref("Invoice March.pdf"), file_ref("Invoice April.pdf")],
        });
        let mut first = user(&[false]); // picks a file, then cancels the send
        first.choice = Some("C:/Docs/Invoice April.pdf".into());
        flow.handle(52, "send invoice to ved on whatsapp", &mut first);
        let mut second = user(&[false]);
        second.choice = Some("C:/Docs/Invoice March.pdf".into());
        flow.handle(53, "send invoice to ved on whatsapp", &mut second);
        assert_eq!(
            second.choices_seen.len(),
            2,
            "a cancelled send teaches nothing"
        );
    }

    #[test]
    fn a_changed_app_screen_hands_over_to_the_user_after_the_chat_opened() {
        let wa = Arc::new(
            FakeConnector::new("whatsapp_desktop")
                .fail_action_with("attach_and_send", ConnError::UiChanged("WhatsApp".into())),
        );
        let mut flow =
            flow_with(&[WHATSAPP], vec![("whatsapp_desktop", wa.clone())]).with_context(Files {
                selected: None,
                files: vec![file_ref("report.pdf")],
            });
        let mut u = user(&[true]);
        let ev = flow.handle(54, "send report.pdf to ved on whatsapp", &mut u);
        assert_eq!(
            done(&ev),
            (
                "I opened ved's chat and copied the file. Press Ctrl+V, then Send.",
                false
            )
        );
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

    // --- closed lists: installed apps and contacts ---------------------------

    fn open_flow(apps: &[&str]) -> (Flow, Arc<FakeConnector>) {
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let names: Vec<String> = apps.iter().map(|a| a.to_string()).collect();
        let flow = flow_with(&[OPEN], vec![("windows_system", sys.clone())])
            .with_app_names(move || names.clone());
        (flow, sys)
    }

    fn opened_app(c: &FakeConnector) -> Option<String> {
        c.calls().into_iter().find_map(|c| match c {
            Call::Execute { action, args } if action == "open_app" => args.get("app").cloned(),
            _ => None,
        })
    }

    #[test]
    fn an_installed_app_is_opened_by_its_real_name() {
        let (mut flow, sys) = open_flow(&["Spotify", "Visual Studio Code"]);
        flow.handle(60, "open visual studio", &mut user(&[]));
        assert_eq!(opened_app(&sys).as_deref(), Some("Visual Studio Code"));
    }

    #[test]
    fn an_app_that_is_not_on_the_list_still_goes_to_the_connector_as_said() {
        // The list can be a few minutes old, and the connector refreshes on a miss.
        let (mut flow, sys) = open_flow(&["Spotify"]);
        flow.handle(61, "open brandnewapp", &mut user(&[]));
        assert_eq!(opened_app(&sys).as_deref(), Some("brandnewapp"));
    }

    #[test]
    fn two_apps_that_fit_make_one_question_and_the_answer_is_used() {
        let (mut flow, sys) = open_flow(&["Office Hub", "Office Lens", "Spotify"]);
        let mut u = user(&[]);
        u.choice = Some("Office Lens".into());
        flow.handle(62, "open office", &mut u);
        assert_eq!(u.choices_seen, ["Office Hub", "Office Lens"]);
        assert_eq!(opened_app(&sys).as_deref(), Some("Office Lens"));
        // Dismissing the question opens nothing.
        let (mut flow, sys) = open_flow(&["Office Hub", "Office Lens"]);
        flow.handle(63, "open office", &mut user(&[]));
        assert_eq!(opened_app(&sys), None);
    }

    fn sent_contact(c: &FakeConnector) -> Option<String> {
        c.calls().into_iter().find_map(|c| match c {
            Call::Execute { action, args } if action == "open_chat" => args.get("contact").cloned(),
            _ => None,
        })
    }

    fn report_context() -> Files {
        Files {
            selected: None,
            files: vec![file_ref("report.pdf")],
        }
    }

    #[test]
    fn two_contacts_called_ved_make_a_question_and_the_confirm_shows_name_and_number() {
        let (mut flow, wa) = whatsapp_flow(report_context());
        flow.store()
            .upsert_contact("Ved Narvekar", Some("+91 98765 43210"), "vcard")
            .unwrap();
        flow.store()
            .upsert_contact("Ved Patel", Some("+91 91234 56789"), "vcard")
            .unwrap();
        let mut u = user(&[true]);
        u.choice = Some("Ved Patel".into());
        let ev = flow.handle(64, "send report.pdf to ved on whatsapp", &mut u);
        assert_eq!(u.choices_seen, ["Ved Narvekar", "Ved Patel"]);
        assert_eq!(
            u.shown,
            ["Send report.pdf to Ved Patel (+91 91234 56789) on WhatsApp?"]
        );
        assert_eq!(sent_contact(&wa).as_deref(), Some("Ved Patel"));
        assert_eq!(done(&ev).0, "Done.");
    }

    #[test]
    fn dismissing_the_which_one_question_sends_nothing() {
        let (mut flow, wa) = whatsapp_flow(report_context());
        for n in ["Ved Narvekar", "Ved Patel"] {
            flow.store().upsert_contact(n, None, "vcard").unwrap();
        }
        let mut u = user(&[true]);
        flow.handle(65, "send report.pdf to ved on whatsapp", &mut u);
        assert!(u.shown.is_empty());
        assert_eq!(sent_contact(&wa), None);
    }

    #[test]
    fn a_unique_first_name_resolves_to_the_saved_person_and_a_send_is_still_confirmed() {
        let (mut flow, wa) = whatsapp_flow(report_context());
        flow.store()
            .upsert_contact("Rahul Shah", None, "vcard")
            .unwrap();
        // Declining the confirm: nothing is sent, and the contact is not counted as used.
        let mut u = user(&[false]);
        flow.handle(66, "send report.pdf to rahul on whatsapp", &mut u);
        assert_eq!(u.shown, ["Send report.pdf to Rahul Shah on WhatsApp?"]);
        assert_eq!(sent_contact(&wa), None);
        assert_eq!(flow.store().contacts().unwrap()[0].uses, 0);
    }

    #[test]
    fn a_new_name_is_sent_as_said_then_learned_after_it_works() {
        let (mut flow, wa) = whatsapp_flow(report_context());
        let mut u = user(&[true]);
        flow.handle(67, "send report.pdf to zainab on whatsapp", &mut u);
        assert_eq!(u.shown, ["Send report.pdf to zainab on WhatsApp?"]);
        assert_eq!(sent_contact(&wa).as_deref(), Some("zainab"));
        let saved = flow.store().contacts().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "Zainab");
        assert_eq!(saved[0].source, "learned");
        // Next time the name is known, and the card shows it as saved.
        let mut u = user(&[false]);
        flow.handle(68, "send report.pdf to zainab on whatsapp", &mut u);
        assert_eq!(u.shown, ["Send report.pdf to Zainab on WhatsApp?"]);
    }

    #[test]
    fn a_cancelled_or_pretend_send_teaches_nothing() {
        let (mut flow, _) = whatsapp_flow(report_context());
        flow.handle(
            69,
            "send report.pdf to zainab on whatsapp",
            &mut user(&[false]),
        );
        assert!(flow.store().contacts().unwrap().is_empty());
        // The test send skill only pretends, so it must not fill the list either.
        let stub = Arc::new(FakeConnector::new("stub_messenger"));
        let mut flow = flow_with(&[SEND], vec![("stub_messenger", stub)]);
        flow.handle(70, "send hi to ved", &mut user(&[true]));
        assert!(flow.store().contacts().unwrap().is_empty());
    }

    #[test]
    fn a_group_is_never_learned_as_a_person() {
        let (mut flow, _) = whatsapp_flow(report_context());
        flow.handle(
            71,
            "send report.pdf to ved and rahul on whatsapp",
            &mut user(&[true]),
        );
        assert!(flow.store().contacts().unwrap().is_empty());
    }

    #[test]
    fn display_names_are_capitalised_only_when_all_lowercase() {
        assert_eq!(display_name("ved narvekar"), "Ved Narvekar");
        assert_eq!(display_name("Ved narvekar"), "Ved narvekar");
        assert_eq!(display_name("  zainab "), "Zainab");
    }

    #[test]
    fn every_shipped_skill_can_be_filled_from_its_own_example() {
        // Catches a template that references something the flow can never supply.
        // Skills may still need fields only the Phase 2 resolver provides; those
        // are listed here so the gap is visible and cannot grow unnoticed.
        let needs_resolver = ["whatsapp.send_file"];
        for yaml in [OPEN, PAUSE, SEND, WHATSAPP] {
            let skill = Skill::parse(yaml).unwrap();
            let mut slots: Slots = skill
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
            // The flow always supplies the built-in `request` slot.
            slots.insert("request".into(), SlotValue::from_text("sample request"));
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

    #[test]
    fn set_my_city_saves_the_place_and_undo_restores_it() {
        let dir = std::env::temp_dir().join(format!("jitmo-flow-place-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("jitmo.db");
        let skills =
            vec![Skill::parse(include_str!("../../../skills/settings.place.yaml")).unwrap()];
        let map: HashMap<String, Arc<dyn Connector>> = HashMap::from([(
            "settings".to_string(),
            Arc::new(jitmo_connectors::settings::Settings::new(db.clone())) as Arc<dyn Connector>,
        )]);
        let mut flow = Flow::new(Router::new(skills), Store::open_in_memory().unwrap(), map)
            .with_clock(|| "2026-11-08T12:00:00Z".into());
        let saved = || {
            jitmo_store::Store::open(&db, None)
                .unwrap()
                .get_setting("default_place")
                .unwrap()
        };
        for (said, want) in [
            ("set my city to Pune", "Pune"),
            ("My location is Mumbai", "Mumbai"),
            ("I live in Delhi", "Delhi"),
        ] {
            let ev = flow.handle(1, said, &mut user(&[]));
            assert_eq!(
                done(&ev),
                (format!("Saved {want} as your place.").as_str(), true)
            );
            assert_eq!(saved().as_deref(), Some(want));
        }
        flow.undo();
        assert_eq!(
            saved().as_deref(),
            Some("Mumbai"),
            "undo restores the previous place"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- the local language model behind the engine ------------------------------------

    struct Scripted(Vec<crate::llm::Pick>);
    impl crate::llm::Llm for Scripted {
        fn route(
            &mut self,
            _request: &str,
            _tools: &[crate::llm::ToolBrief],
            _recent: Option<&str>,
        ) -> crate::llm::Pick {
            if self.0.is_empty() {
                crate::llm::Pick::Nothing
            } else {
                self.0.remove(0)
            }
        }
    }

    #[test]
    fn an_ungrounded_model_answer_never_reaches_the_user() {
        let c = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", c)])
            .with_llm_picks(true)
            .with_llm_on_decline(true)
            .with_llm(Box::new(Scripted(vec![crate::llm::Pick::Answer(
                "Boil the pasta for ten minutes.".into(),
            )])));
        let mut u = TestUser::default();
        let ev = flow.handle(1, "blorp the zibble pasta recipe please", &mut u);
        assert!(
            !matches!(&ev, CoreEvent::Done { message, .. } if message.contains("Boil the pasta")),
            "{ev:?}"
        );
    }

    #[test]
    fn a_picked_tool_still_runs_through_the_normal_pipeline() {
        let c = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", c.clone())]).with_llm(Box::new(
            Scripted(vec![crate::llm::Pick::Tool("apps.open".into())]),
        ));
        flow = flow
            .with_app_names(|| vec!["Spotify".to_string()])
            .with_llm_picks(true)
            .with_llm_on_decline(true);
        let mut u = TestUser::default();
        let ev = flow.handle(2, "blorp spotify zibble", &mut u);
        // The picked skill went through slot resolution like any other: with no app named it
        // says so instead of running.
        assert!(
            matches!(&ev, CoreEvent::Failed { message, .. } if message.contains("missing the app")),
            "{ev:?}"
        );
        assert!(c.calls().is_empty());
    }

    #[test]
    fn typed_values_from_the_model_become_the_skills_own_sentence() {
        const TIMER: &str = include_str!("../../../skills/reminders.timer.yaml");
        let c = Arc::new(FakeConnector::new("reminders"));
        let mut args = BTreeMap::new();
        args.insert("length".to_string(), "10 minutes".to_string());
        args.insert("extra".to_string(), "dropped".to_string());
        let mut flow = flow_with(&[TIMER], vec![("reminders", c.clone())])
            .with_llm_picks(true)
            .with_llm_on_decline(true)
            .with_llm(Box::new(Scripted(vec![crate::llm::Pick::Call {
                tool: "reminders.timer".into(),
                args,
            }])));
        let mut u = TestUser::default();
        let ev = flow.handle(1, "blorp zibble wonk 10 minutes", &mut u);
        assert!(matches!(ev, CoreEvent::Done { .. }), "{ev:?}");
        let sentence = c
            .calls()
            .into_iter()
            .find_map(|call| match call {
                Call::Execute { args, .. } => args.get("text").cloned(),
                _ => None,
            })
            .expect("the timer must have run");
        assert_eq!(sentence, "set a timer for 10 minutes");
    }

    /// A model that never picks a tool but fills typed values.
    struct Filler(Vec<(&'static str, &'static str)>);
    impl crate::llm::Llm for Filler {
        fn route(
            &mut self,
            _r: &str,
            _t: &[crate::llm::ToolBrief],
            _recent: Option<&str>,
        ) -> crate::llm::Pick {
            crate::llm::Pick::Nothing
        }
        fn fill(
            &mut self,
            _r: &str,
            _t: &crate::llm::ToolBrief,
        ) -> Option<BTreeMap<String, String>> {
            Some(
                self.0
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )
        }
    }

    /// A model whose every call the user cancels (Esc while it runs).
    struct Cancelled(bool);
    impl crate::llm::Llm for Cancelled {
        fn route(
            &mut self,
            _r: &str,
            _t: &[crate::llm::ToolBrief],
            _recent: Option<&str>,
        ) -> crate::llm::Pick {
            self.0 = true;
            crate::llm::Pick::Nothing
        }
        fn fill(
            &mut self,
            _r: &str,
            _t: &crate::llm::ToolBrief,
        ) -> Option<BTreeMap<String, String>> {
            self.0 = true;
            None
        }
        fn take_cancelled(&mut self) -> bool {
            std::mem::take(&mut self.0)
        }
    }

    #[test]
    fn a_cancelled_pick_ends_the_request_without_falling_back() {
        let c = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", c.clone())])
            .with_app_names(|| vec!["Spotify".to_string()])
            .with_llm_picks(true)
            .with_llm_on_decline(true)
            .with_llm(Box::new(Cancelled(false)));
        let ev = flow.handle(2, "blorp spotify zibble", &mut TestUser::default());
        assert!(
            matches!(&ev, CoreEvent::Done { message, .. } if message == "Cancelled."),
            "{ev:?}"
        );
        assert!(c.calls().is_empty());
    }

    #[test]
    fn a_cancelled_fill_runs_nothing() {
        let c = Arc::new(FakeConnector::new("browser"));
        let mut flow =
            flow_with(&[SEARCH], vec![("browser", c.clone())]).with_llm(Box::new(Cancelled(false)));
        let ev = flow.handle(
            1,
            "search the web for cheap flights to goa",
            &mut TestUser::default(),
        );
        assert!(
            matches!(&ev, CoreEvent::Done { message, .. } if message == "Cancelled."),
            "{ev:?}"
        );
        assert!(!c
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Execute { .. })));
    }

    fn executed_args(c: &FakeConnector) -> Args {
        c.calls()
            .into_iter()
            .find_map(|call| match call {
                Call::Execute { args, .. } => Some(args),
                _ => None,
            })
            .expect("the connector must have run")
    }

    const SEARCH: &str = include_str!("../../../skills/browser.search.yaml");

    #[test]
    fn typed_values_go_straight_to_the_connector() {
        let c = Arc::new(FakeConnector::new("browser"));
        let mut flow =
            flow_with(&[SEARCH], vec![("browser", c.clone())]).with_llm(Box::new(Filler(vec![
                ("query", "cheap flights to goa"),
                ("extra", "dropped"),
            ])));
        let ev = flow.handle(
            1,
            "search the web for cheap flights to goa",
            &mut TestUser::default(),
        );
        assert!(matches!(ev, CoreEvent::Done { .. }), "{ev:?}");
        let args = executed_args(&c);
        assert_eq!(
            args.get("query").map(String::as_str),
            Some("cheap flights to goa")
        );
        assert!(args.contains_key("text"), "the sentence still goes along");
        assert!(!args.contains_key("extra"), "a model cannot add arguments");
    }

    #[test]
    fn without_a_model_the_typed_values_are_left_out() {
        let c = Arc::new(FakeConnector::new("browser"));
        let mut flow = flow_with(&[SEARCH], vec![("browser", c.clone())]);
        let ev = flow.handle(
            1,
            "search the web for cheap flights to goa",
            &mut TestUser::default(),
        );
        assert!(matches!(ev, CoreEvent::Done { .. }), "{ev:?}");
        let args = executed_args(&c);
        assert!(!args.contains_key("query"));
        assert_eq!(
            args.get("text").map(String::as_str),
            Some("search the web for cheap flights to goa")
        );
    }

    #[test]
    fn a_model_that_leaves_a_required_value_empty_falls_back_to_the_rules() {
        let c = Arc::new(FakeConnector::new("browser"));
        let mut flow = flow_with(&[SEARCH], vec![("browser", c.clone())])
            .with_llm(Box::new(Filler(vec![("query", "  ")])));
        let ev = flow.handle(
            1,
            "search the web for cheap flights to goa",
            &mut TestUser::default(),
        );
        assert!(matches!(ev, CoreEvent::Done { .. }), "{ev:?}");
        assert!(!executed_args(&c).contains_key("query"));
    }

    #[test]
    fn a_post_shows_and_sends_the_same_words_and_invented_values_are_dropped() {
        const POST: &str = include_str!("../../../skills/chat.post.yaml");
        let run = |values: Vec<(&'static str, &'static str)>| {
            let c = Arc::new(FakeConnector::new("teams"));
            let mut flow =
                flow_with(&[POST], vec![("teams", c.clone())]).with_llm(Box::new(Filler(values)));
            let mut u = TestUser::default();
            let ev = flow.handle(
                1,
                "post in the project channel that the build is ready",
                &mut u,
            );
            assert!(matches!(ev, CoreEvent::Done { .. }), "{ev:?}");
            (u.shown, executed_args(&c))
        };
        // Values copied from the request go along; the words come from the sentence.
        let (shown, args) = run(vec![("channel", "project"), ("body", "the build is ready")]);
        assert_eq!(shown, ["Post this in Teams? “The build is ready.”"]);
        assert_eq!(
            args.get("body").map(String::as_str),
            Some("The build is ready.")
        );
        assert_eq!(args.get("channel").map(String::as_str), Some("project"));
        // Words the request does not contain are the model's invention: never shown or sent.
        let (shown, args) = run(vec![("channel", "design"), ("body", "the file is updated")]);
        assert_eq!(shown, ["Post this in Teams? “The build is ready.”"]);
        assert!(!args.contains_key("channel"), "{args:?}");
    }

    #[test]
    fn grounding_keeps_copied_values_and_drops_invented_ones() {
        assert!(grounded_in("despacito", "play despacito"));
        assert!(!grounded_in("the latest album", "play despacito"));
        assert!(grounded_in(
            "the contract",
            "find the emial about the contrat"
        ));
        assert!(grounded_in("10 minutes", "set a ten minutes timer"));
        assert!(!grounded_in("model", "open notepad and type hello world"));
    }

    #[test]
    fn a_missing_required_value_is_asked_from_the_skills_own_question() {
        const TIMER: &str = include_str!("../../../skills/reminders.timer.yaml");
        let c = Arc::new(FakeConnector::new("reminders"));
        let mut flow = flow_with(&[TIMER], vec![("reminders", c)])
            .with_llm_picks(true)
            .with_llm_on_decline(true)
            .with_llm(Box::new(Scripted(vec![crate::llm::Pick::Call {
                tool: "reminders.timer".into(),
                args: BTreeMap::new(),
            }])));
        let mut u = TestUser {
            text_answers: vec!["5 minutes".into()],
            ..TestUser::default()
        };
        let _ = flow.handle(2, "blorp zibble wonk 10 minutes", &mut u);
        assert_eq!(u.questions, ["For how long?"]);
    }

    #[test]
    fn a_request_the_model_worked_out_is_remembered_and_runs_without_it() {
        const TIMER: &str = include_str!("../../../skills/reminders.timer.yaml");
        let c = Arc::new(FakeConnector::new("reminders"));
        let mut args = BTreeMap::new();
        args.insert("length".to_string(), "10 minutes".to_string());
        // The script holds ONE pick: the second time the model has nothing to say.
        let mut flow = flow_with(&[TIMER], vec![("reminders", c.clone())])
            .with_llm_picks(true)
            .with_llm_on_decline(true)
            .with_llm(Box::new(Scripted(vec![crate::llm::Pick::Call {
                tool: "reminders.timer".into(),
                args,
            }])));
        let runs = |c: &Arc<FakeConnector>| {
            c.calls()
                .iter()
                .filter(|x| matches!(x, Call::Execute { .. }))
                .count()
        };
        let mut u = TestUser::default();
        let first = flow.handle(1, "blorp zibble wonk 10 minutes", &mut u);
        assert!(matches!(first, CoreEvent::Done { .. }), "{first:?}");
        assert_eq!(runs(&c), 1);
        let again = flow.handle(2, "Blorp  zibble wonk 10 MINUTES!", &mut u);
        assert!(matches!(again, CoreEvent::Done { .. }), "{again:?}");
        assert_eq!(runs(&c), 2, "the remembered request must run on its own");
    }

    #[test]
    fn a_remembered_request_that_fails_is_forgotten() {
        const TIMER: &str = include_str!("../../../skills/reminders.timer.yaml");
        let c = Arc::new(FakeConnector::new("reminders"));
        let mut args = BTreeMap::new();
        args.insert("length".to_string(), "10 minutes".to_string());
        let mut flow = flow_with(&[TIMER], vec![("reminders", c.clone())])
            .with_llm_picks(true)
            .with_llm_on_decline(true)
            .with_llm(Box::new(Scripted(vec![crate::llm::Pick::Call {
                tool: "reminders.timer".into(),
                args,
            }])));
        let mut u = TestUser::default();
        let _ = flow.handle(1, "blorp zibble wonk 10 minutes", &mut u);
        assert!(flow
            .learned_mut()
            .get("blorp zibble wonk 10 minutes")
            .is_some());
        // The connector now fails: the remembered request is dropped.
        flow.connectors.insert(
            "reminders".into(),
            Arc::new(
                FakeConnector::new("reminders").fail_execute_with(ConnError::Failed("nope".into())),
            ),
        );
        let ev = flow.handle(2, "blorp zibble wonk 10 minutes", &mut u);
        assert!(matches!(ev, CoreEvent::Failed { .. }), "{ev:?}");
        assert!(flow
            .learned_mut()
            .get("blorp zibble wonk 10 minutes")
            .is_none());
    }

    // ---- grounded answers -------------------------------------------------------------

    struct Pages;
    impl Connector for Pages {
        fn id(&self) -> &str {
            "web_lookup"
        }
        fn actions(&self) -> Vec<jitmo_connectors::ActionSpec> {
            Vec::new()
        }
        fn health(&self) -> jitmo_connectors::Health {
            jitmo_connectors::Health::Ready
        }
        fn execute(&self, action: &str, _args: &Args) -> Result<Output, ConnError> {
            if action != "passages" {
                return Err(ConnError::Failed("I'm not sure about that one.".into()));
            }
            let mut m = BTreeMap::new();
            m.insert("n".to_string(), "2".to_string());
            m.insert("title0".to_string(), "The Vaccines".to_string());
            m.insert("url0".to_string(), "https://x/band".to_string());
            m.insert(
                "text0".to_string(),
                "The Vaccines are an English indie rock band formed in London in 2010.".to_string(),
            );
            m.insert("title1".to_string(), "Vaccine".to_string());
            m.insert("url1".to_string(), "https://x/vaccine".to_string());
            m.insert(
                "text1".to_string(),
                "A vaccine trains the immune system to recognise a germ. It contains a weakened part of the germ, so the body makes antibodies.".to_string(),
            );
            Ok(Output(m))
        }
        fn check(&self, _a: &str, _g: &Args) -> Result<bool, ConnError> {
            Ok(true)
        }
        fn undo(&self, _a: &str, _o: &Output) -> Option<Result<(), ConnError>> {
            None
        }
    }

    struct Writer(&'static str);
    impl crate::llm::Llm for Writer {
        fn route(
            &mut self,
            _r: &str,
            _t: &[crate::llm::ToolBrief],
            _recent: Option<&str>,
        ) -> crate::llm::Pick {
            crate::llm::Pick::Tool("web.fact".into())
        }
        fn answer_from(&mut self, _q: &str, _p: &str) -> Option<String> {
            Some(self.0.to_string())
        }
    }

    fn fact_flow(writer: &'static str) -> Flow {
        const FACT: &str = include_str!("../../../skills/web.fact.yaml");
        let skills = vec![Skill::parse(FACT).unwrap()];
        let mut map: HashMap<String, Arc<dyn Connector>> = HashMap::new();
        map.insert("web_lookup".into(), Arc::new(Pages));
        Flow::new(Router::new(skills), Store::open_in_memory().unwrap(), map)
            .with_clock(|| "2026-11-08T12:00:00Z".into())
            .with_llm(Box::new(Writer(writer)))
    }

    #[test]
    fn a_how_question_is_answered_from_the_matching_page_with_its_source() {
        let mut flow = fact_flow(
            "A vaccine trains your immune system by showing it a weakened part of the germ.",
        );
        let mut u = TestUser::default();
        let ev = flow.handle(1, "how do vaccines work", &mut u);
        match ev {
            CoreEvent::Done { message, .. } => {
                assert!(message.contains("weakened part of the germ"), "{message}");
                // The band's page was skipped; the real one is cited.
                assert!(message.contains("https://x/vaccine"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_answer_the_passage_does_not_support_is_not_shown() {
        let mut flow = fact_flow("Vaccines were invented in 1850 by a famous king.");
        let mut u = TestUser::default();
        let ev = flow.handle(2, "how do vaccines work", &mut u);
        assert!(
            !matches!(&ev, CoreEvent::Done { message, .. } if message.contains("1850")),
            "{ev:?}"
        );
    }

    #[test]
    fn a_declined_request_is_not_handed_to_the_model_by_default() {
        const TIMER: &str = include_str!("../../../skills/reminders.timer.yaml");
        let c = Arc::new(FakeConnector::new("reminders"));
        let mut args = BTreeMap::new();
        args.insert("length".to_string(), "10 minutes".to_string());
        let mut flow =
            flow_with(&[TIMER], vec![("reminders", c.clone())]).with_llm(Box::new(Scripted(vec![
                crate::llm::Pick::Call {
                    tool: "reminders.timer".into(),
                    args,
                },
            ])));
        let ev = flow.handle(1, "blorp zibble wonk 10 minutes", &mut TestUser::default());
        assert!(
            matches!(&ev, CoreEvent::Failed { message, .. } if message == "I can't do that yet."),
            "{ev:?}"
        );
        assert!(
            c.calls().is_empty(),
            "nothing may run on a declined request"
        );
    }

    #[test]
    fn nothing_from_the_model_keeps_the_old_answer() {
        let c = Arc::new(FakeConnector::new("windows_system"));
        let mut flow = flow_with(&[OPEN], vec![("windows_system", c)])
            .with_llm_picks(true)
            .with_llm_on_decline(true)
            .with_llm(Box::new(Scripted(vec![])));
        let mut u = TestUser::default();
        let ev = flow.handle(3, "blorp zibble wonk 10 minutes", &mut u);
        assert!(matches!(ev, CoreEvent::Failed { .. }), "{ev:?}");
    }
}

#[cfg(test)]
mod time_option_tests {
    use super::time_options;

    #[test]
    fn a_bare_three_only_offers_the_afternoon() {
        let o = time_options(
            "remind me to stretch at 3",
            "Which time did you mean? (3:00)",
        )
        .unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].0, "3 in the afternoon");
        assert_eq!(o[0].1, "remind me to stretch at 3pm");
    }

    #[test]
    fn a_bare_ten_offers_morning_and_night_in_words() {
        let o = time_options(
            "remind me at 10:30 to call",
            "Which time did you mean? (10:30)",
        )
        .unwrap();
        assert_eq!(
            o[0],
            (
                "10:30 in the morning".into(),
                "remind me at 10:30am to call".into()
            )
        );
        assert_eq!(
            o[1],
            (
                "10:30 at night".into(),
                "remind me at 10:30pm to call".into()
            )
        );
        let o = time_options("remind me at 6 to eat", "Which time did you mean? (6:00)").unwrap();
        assert_eq!(o[1].0, "6 in the evening");
        assert!(time_options("remind me at 15", "Which time did you mean? (15:00)").is_none());
        // The morning one is dropped once it has gone by today.
        let o = time_options("remind me at 6", "Which time did you mean? (6:00 pm only)").unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].0, "6 in the evening");
        // "timer until 6" keeps its "until".
        let o = time_options("timer until 6", "Which time did you mean? (6:00)").unwrap();
        assert_eq!(o[1].1, "timer until 6pm");
    }
}
