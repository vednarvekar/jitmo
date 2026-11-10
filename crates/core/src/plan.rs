//! Plans: a short list of existing skills to run in order (LLD, "Planner").
//!
//! This is the part of the planner that is plain code, so it can be built and tested before
//! any model exists:
//! - the plan format (at most 5 steps, each a skill and its slots; a slot may point at an
//!   earlier step's result with `$stepN.result`);
//! - the four checks a plan must pass before it runs ([`check`]);
//! - saved templates for the common chains, tried before any model ([`template`]): a
//!   request that matches one never loads the planner (workflows W1 to W6, tests T44, T75,
//!   T79, T81).
//!
//! A plan that fails a check is rejected: the engine says it cannot do this yet instead of
//! guessing. A model that drafts plans later plugs in behind [`check`] unchanged.

use std::collections::BTreeMap;

/// The most steps a plan may have.
pub const MAX_STEPS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub skill: String,
    /// Slot name to value. A value `$step1.result` means "the result of step 1".
    pub slots: BTreeMap<String, String>,
}

impl PlanStep {
    /// A step whose only slot is the sentence to run (what the offline skills take).
    pub fn request(skill: &str, sentence: &str) -> PlanStep {
        PlanStep {
            skill: skill.into(),
            slots: BTreeMap::from([("request".to_string(), sentence.to_string())]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub steps: Vec<PlanStep>,
}

/// What the checks need to know about a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillShape {
    pub required_slots: Vec<String>,
    /// The skill sends or shares something.
    pub sends: bool,
    /// The skill asks the user to confirm before it runs.
    pub confirms: bool,
}

impl SkillShape {
    pub fn of(skill: &jitmo_skills::Skill) -> SkillShape {
        SkillShape {
            required_slots: skill
                .slots
                .iter()
                .filter(|(_, s)| matches!(s, jitmo_skills::Slot::Param(p) if p.required))
                .map(|(name, _)| name.clone())
                .collect(),
            sends: skill.sends(),
            confirms: skill.confirm.is_some(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    Empty,
    /// Check 3: more than [`MAX_STEPS`] steps.
    TooLong(usize),
    /// Check 1: a skill id that does not exist.
    UnknownSkill {
        step: usize,
        skill: String,
    },
    /// Check 2: a required slot is neither filled nor linked to an earlier step.
    MissingSlot {
        step: usize,
        slot: String,
    },
    /// A link to a step that comes later (or itself, or does not exist).
    BadLink {
        step: usize,
        slot: String,
    },
    /// Check 4: a send or share step whose skill has no confirm.
    SendWithoutConfirm {
        step: usize,
        skill: String,
    },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Empty => write!(f, "the plan has no steps"),
            PlanError::TooLong(n) => write!(f, "the plan has {n} steps; the most is {MAX_STEPS}"),
            PlanError::UnknownSkill { step, skill } => {
                write!(f, "step {step} uses a skill that does not exist ({skill})")
            }
            PlanError::MissingSlot { step, slot } => {
                write!(f, "step {step} has no value for the required slot {slot}")
            }
            PlanError::BadLink { step, slot } => {
                write!(
                    f,
                    "step {step} slot {slot} points at a step that has not run yet"
                )
            }
            PlanError::SendWithoutConfirm { step, skill } => {
                write!(f, "step {step} sends or shares but {skill} has no confirm")
            }
        }
    }
}

/// "$step2.result" -> Some(2).
fn link(value: &str) -> Option<usize> {
    let rest = value.strip_prefix("$step")?.strip_suffix(".result")?;
    rest.parse().ok()
}

/// Run the four checks. Steps are numbered from 1, as in the plan format.
pub fn check(plan: &Plan, shape_of: &dyn Fn(&str) -> Option<SkillShape>) -> Result<(), PlanError> {
    if plan.steps.is_empty() {
        return Err(PlanError::Empty);
    }
    if plan.steps.len() > MAX_STEPS {
        return Err(PlanError::TooLong(plan.steps.len()));
    }
    for (i, step) in plan.steps.iter().enumerate() {
        let n = i + 1;
        let shape = shape_of(&step.skill).ok_or_else(|| PlanError::UnknownSkill {
            step: n,
            skill: step.skill.clone(),
        })?;
        for (slot, value) in &step.slots {
            if value.starts_with("$step") && !matches!(link(value), Some(k) if k >= 1 && k < n) {
                return Err(PlanError::BadLink {
                    step: n,
                    slot: slot.clone(),
                });
            }
        }
        // A step that carries its own sentence (`request`) has its slots filled from that
        // sentence when it runs, and asked for if they are missing: the ordinary route.
        let from_sentence = step
            .slots
            .get("request")
            .is_some_and(|r| !r.trim().is_empty());
        for slot in shape.required_slots.iter().filter(|_| !from_sentence) {
            if step.slots.get(slot).is_none_or(|v| v.trim().is_empty()) {
                return Err(PlanError::MissingSlot {
                    step: n,
                    slot: slot.clone(),
                });
            }
        }
        if shape.sends && !shape.confirms {
            return Err(PlanError::SendWithoutConfirm {
                step: n,
                skill: step.skill.clone(),
            });
        }
    }
    Ok(())
}

/// A skill as the planner sees it: its id and a short name. Only the best five are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBrief {
    pub id: String,
    pub title: String,
}

/// Drafts a plan for a request no saved template and no part-wise routing could handle. The
/// model planner (llama.cpp, Track B) is an implementation of this; its draft is free text per
/// step (`PlanStep::request`), shaped and then **checked by [`check`] like any other plan**,
/// so a model can only combine skills that exist, never invent an action.
pub trait PlanDrafter: Send {
    fn draft(&mut self, request: &str, skills: &[SkillBrief]) -> Option<Plan>;
}

/// The sentences to run, in order, for a plan made of [`PlanStep::request`] steps. `None`
/// when a step has no plain sentence (it links to an earlier result): the caller runs such
/// a plan step by step instead.
pub fn sentences(plan: &Plan) -> Option<Vec<String>> {
    plan.steps
        .iter()
        .map(|s| {
            s.slots
                .get("request")
                .filter(|r| !r.starts_with("$step"))
                .cloned()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Saved templates
// ---------------------------------------------------------------------------

/// What one phrase of a chained request means: the skill and the sentence that runs it.
fn phrase(part: &str) -> Option<(&'static str, String)> {
    let (skill, sentence) = phrase_static(part)?;
    Some((skill, sentence))
}

/// The phrases with a fixed meaning, then the ones that carry a name ("email it to Rahul").
fn phrase_static(part: &str) -> Option<(&'static str, String)> {
    if let Some(found) = phrase_fixed(part) {
        return Some((found.0, found.1.to_string()));
    }
    let p = part
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    let p = p
        .trim_start_matches("please ")
        .trim_start_matches("and ")
        .trim_start_matches("then ")
        .trim();
    // "email it to Rahul", "email the screenshot to Rahul": a send of what came before.
    if let Some(who) = p
        .strip_prefix("email it to ")
        .or_else(|| p.strip_prefix("email that to "))
        .or_else(|| p.strip_prefix("email the screenshot to "))
        .or_else(|| p.strip_prefix("send it to "))
    {
        let who = who.trim();
        if !who.is_empty() && !who.contains(char::is_whitespace) {
            return Some(("files.email", format!("email the screenshot to {who}")));
        }
    }
    // "open Chrome": an app, by name.
    if let Some(app) = p.strip_prefix("open ") {
        let app = app.trim();
        if !app.is_empty() && app.split_whitespace().count() <= 3 {
            return Some(("apps.open", format!("open {app}")));
        }
    }
    // "search for flights to Goa": a web search.
    if let Some(q) = p
        .strip_prefix("search for ")
        .or_else(|| p.strip_prefix("search "))
    {
        if !q.trim().is_empty() {
            return Some(("browser.search", format!("search for {}", q.trim())));
        }
    }
    // "invite Priya": add her to the meeting the previous step made.
    if let Some(who) = p.strip_prefix("invite ") {
        let who = who.trim();
        if !who.is_empty() && !who.contains(char::is_whitespace) {
            return Some((
                "calendar.invite",
                format!("invite {who} to the project review"),
            ));
        }
    }
    None
}

fn phrase_fixed(part: &str) -> Option<(&'static str, &'static str)> {
    let p = part
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    let p = p
        .trim_start_matches("please ")
        .trim_start_matches("and ")
        .trim_start_matches("then ")
        .trim();
    let any = |names: &[&str]| names.contains(&p);
    if any(&[
        "mute notifications",
        "mute the notifications",
        "mute my notifications",
        "silence notifications",
        "turn on do not disturb",
        "turn on dnd",
        "do not disturb",
        "dnd on",
        "dnd",
        "pause notifications",
    ]) {
        return Some(("system.dnd", "turn on do not disturb"));
    }
    if any(&[
        "dim the screen",
        "dim my screen",
        "dim the display",
        "dim screen",
        "dim the brightness",
        "lower the brightness",
        "turn the brightness down",
        "turn down the brightness",
    ]) {
        return Some(("system.volume", "turn the brightness down"));
    }
    if any(&[
        "play my focus playlist",
        "play the focus playlist",
        "play deep focus",
        "play focus music",
        "play my focus music",
        "put on my focus playlist",
        "start my focus playlist",
    ]) {
        return Some(("music.play", "play deep focus"));
    }
    if any(&[
        "archive the newsletters",
        "archive today's newsletters",
        "archive newsletters",
        "clear the newsletters",
    ]) {
        return Some(("mail.archive_newsletters", "archive today's newsletters"));
    }
    if any(&[
        "show what's urgent",
        "show me what's urgent",
        "show what is urgent",
        "show urgent emails",
        "show me urgent emails",
        "list what's urgent",
        "check urgent emails",
    ]) {
        return Some(("mail.urgent", "any urgent emails"));
    }
    if any(&["take a screenshot", "take a screen shot", "screenshot"]) {
        return Some(("system.screenshot", "take a screenshot"));
    }
    if any(&[
        "turn rahul's email into a meeting",
        "turn this email into a meeting",
        "turn the email into a meeting",
        "make rahul's email a meeting",
    ]) {
        return Some((
            "work.event_from_email",
            "add this meeting request to my calendar",
        ));
    }
    None
}

/// Split "a, b and c" / "a then b" into its parts.
pub(crate) fn parts(text: &str) -> Vec<String> {
    let mut out = vec![text.to_string()];
    for sep in [" and then ", " then ", ", and ", ",", " and ", " & "] {
        out = out
            .iter()
            .flat_map(|p| p.split(sep).map(str::to_string).collect::<Vec<_>>())
            .collect();
    }
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The request without a closing "for an hour" / "for 30 minutes".
fn without_duration(text: &str) -> &str {
    let lower = text.to_lowercase();
    for tail in [
        " for an hour",
        " for 1 hour",
        " for half an hour",
        " for 30 minutes",
    ] {
        if lower.ends_with(tail) {
            return text[..text.len() - tail.len()].trim();
        }
    }
    text
}

/// A saved plan for a chained request, if `text` matches one: "focus mode" (W6), or a chain
/// of phrases that each mean one known step ("mute notifications, dim the screen and play
/// my focus playlist"). Every phrase must be understood; a chain with a part this does not
/// know is not guessed at (it goes on to the ordinary routes).
pub fn template(text: &str) -> Option<Plan> {
    let clean = text.trim().trim_end_matches(['.', '!', '?']).trim();
    let body = without_duration(clean);
    let lower = body.to_lowercase();
    // W6: "focus mode", "start focus mode", "turn on focus mode", "focus mode for an hour".
    let focus_words = lower
        .trim_start_matches("please ")
        .trim_start_matches("start ")
        .trim_start_matches("turn on ")
        .trim_start_matches("enable ")
        .trim_start_matches("activate ")
        .trim_start_matches("begin ")
        .trim();
    if matches!(focus_words, "focus mode" | "focus" | "focus time") {
        return Some(Plan {
            steps: vec![
                PlanStep::request("music.play", "play deep focus"),
                PlanStep::request("system.dnd", "turn on do not disturb"),
            ],
        });
    }
    let mapped: Option<Vec<(&str, String)>> = parts(body).iter().map(|p| phrase(p)).collect();
    let mapped = mapped?;
    if mapped.len() < 2 {
        return None;
    }
    // The same step twice is a typo, not a plan.
    for (i, (_, sentence)) in mapped.iter().enumerate() {
        if mapped[..i].iter().any(|(_, s)| s == sentence) {
            return None;
        }
    }
    Some(Plan {
        steps: mapped
            .into_iter()
            .map(|(skill, sentence)| PlanStep::request(skill, &sentence))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes(id: &str) -> Option<SkillShape> {
        let s = |req: &[&str], sends, confirms| SkillShape {
            required_slots: req.iter().map(|s| s.to_string()).collect(),
            sends,
            confirms,
        };
        match id {
            "files.find" => Some(s(&["query"], false, false)),
            "whatsapp.send_file" => Some(s(&["file", "contact"], true, true)),
            "naked.send" => Some(s(&[], true, false)),
            "music.play" | "system.dnd" | "system.volume" => Some(s(&[], false, false)),
            _ => None,
        }
    }

    fn step(skill: &str, slots: &[(&str, &str)]) -> PlanStep {
        PlanStep {
            skill: skill.into(),
            slots: slots
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn the_example_plan_from_the_design_passes() {
        let plan = Plan {
            steps: vec![
                step("files.find", &[("query", "budget sheet")]),
                step(
                    "whatsapp.send_file",
                    &[("file", "$step1.result"), ("contact", "Rahul")],
                ),
            ],
        };
        assert_eq!(check(&plan, &shapes), Ok(()));
    }

    #[test]
    fn each_of_the_four_checks_rejects() {
        // 1. unknown skill
        let p = Plan {
            steps: vec![step("moon.land", &[])],
        };
        assert!(matches!(
            check(&p, &shapes),
            Err(PlanError::UnknownSkill { step: 1, .. })
        ));
        // 2. missing required slot, and a blank one
        let p = Plan {
            steps: vec![step("files.find", &[])],
        };
        assert!(matches!(
            check(&p, &shapes),
            Err(PlanError::MissingSlot { step: 1, .. })
        ));
        let p = Plan {
            steps: vec![step("files.find", &[("query", "  ")])],
        };
        assert!(matches!(
            check(&p, &shapes),
            Err(PlanError::MissingSlot { .. })
        ));
        // 3. too many steps
        let p = Plan {
            steps: (0..6).map(|_| step("system.dnd", &[])).collect(),
        };
        assert_eq!(check(&p, &shapes), Err(PlanError::TooLong(6)));
        let p = Plan {
            steps: (0..5).map(|_| step("system.dnd", &[])).collect(),
        };
        assert_eq!(check(&p, &shapes), Ok(()));
        // 4. a send with no confirm
        let p = Plan {
            steps: vec![step("naked.send", &[])],
        };
        assert!(matches!(
            check(&p, &shapes),
            Err(PlanError::SendWithoutConfirm { .. })
        ));
        assert_eq!(check(&Plan::default(), &shapes), Err(PlanError::Empty));
    }

    #[test]
    fn a_step_with_its_own_sentence_fills_its_slots_when_it_runs() {
        // files.find needs a `query`; a template step carries the whole sentence instead.
        let p = Plan {
            steps: vec![PlanStep::request("files.find", "find the budget sheet")],
        };
        assert_eq!(check(&p, &shapes), Ok(()));
        let p = Plan {
            steps: vec![PlanStep::request("files.find", "  ")],
        };
        assert!(matches!(
            check(&p, &shapes),
            Err(PlanError::MissingSlot { .. })
        ));
    }

    #[test]
    fn the_work_chains_become_plans() {
        let p = template("Archive the newsletters and show what's urgent.").unwrap();
        assert_eq!(
            sentences(&p).unwrap(),
            ["archive today's newsletters", "any urgent emails"]
        );
        let p = template("Take a screenshot and email it to Rahul.").unwrap();
        assert_eq!(p.steps[0].skill, "system.screenshot");
        assert_eq!(p.steps[1].skill, "files.email");
        let p = template("Turn Rahul's email into a meeting and invite Priya.").unwrap();
        assert_eq!(p.steps[0].skill, "work.event_from_email");
        assert_eq!(p.steps[1].skill, "calendar.invite");
    }

    #[test]
    fn a_link_may_only_point_backwards() {
        let p = Plan {
            steps: vec![
                step("files.find", &[("query", "$step2.result")]),
                step("files.find", &[("query", "x")]),
            ],
        };
        assert!(matches!(
            check(&p, &shapes),
            Err(PlanError::BadLink { step: 1, .. })
        ));
        let p = Plan {
            steps: vec![step("files.find", &[("query", "$step1.result")])],
        };
        assert!(matches!(check(&p, &shapes), Err(PlanError::BadLink { .. })));
        let p = Plan {
            steps: vec![step("files.find", &[("query", "$stepx.result")])],
        };
        assert!(matches!(check(&p, &shapes), Err(PlanError::BadLink { .. })));
    }

    #[test]
    fn focus_mode_is_one_saved_plan() {
        for said in [
            "Focus mode for an hour.",
            "focus mode",
            "start focus mode",
            "turn on focus mode",
        ] {
            let plan = template(said).unwrap_or_else(|| panic!("{said}"));
            assert_eq!(
                sentences(&plan).unwrap(),
                ["play deep focus", "turn on do not disturb"],
                "{said}"
            );
            assert_eq!(check(&plan, &shapes), Ok(()));
        }
    }

    #[test]
    fn a_chain_of_known_phrases_becomes_a_plan_in_order() {
        let plan =
            template("Mute notifications, dim the screen and play my focus playlist.").unwrap();
        assert_eq!(
            sentences(&plan).unwrap(),
            [
                "turn on do not disturb",
                "turn the brightness down",
                "play deep focus"
            ]
        );
        let plan = template("play my focus playlist and mute notifications for an hour").unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].skill, "music.play");
        assert_eq!(plan.steps[1].skill, "system.dnd");
    }

    #[test]
    fn a_chain_with_a_part_it_does_not_know_is_left_alone() {
        assert!(template("mute notifications and bake a cake").is_none());
        assert!(template("mute notifications").is_none()); // one step is not a chain
        assert!(template("dnd and dnd").is_none());
        let p = template("Open Chrome and search for flights to Goa.").unwrap();
        assert_eq!(
            sentences(&p).unwrap(),
            ["open chrome", "search for flights to goa"]
        );
        assert_eq!(p.steps[1].skill, "browser.search");
        assert!(template("open chrome and bake a cake").is_none());
        assert!(template("").is_none());
    }
}
