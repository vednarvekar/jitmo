//! Rule-based stub router (Phase 1). No models.
//!
//! It stands in for the Tier 0 encoder and extractor so the whole pipeline can
//! run end to end. It produces the same [`Understanding`] the real model worker
//! will, so the flow does not change when the models arrive in Phase 2.
//!
//! How it scores a request against each skill's `examples`:
//! - An example may hold placeholders like `open {app}`. A placeholder matches
//!   one or more words and becomes a span of that slot's type.
//! - A full match scores 1.0 (0.9 if it used a placeholder).
//! - Anything else scores by word overlap, capped at 0.7. So the stub never
//!   reaches the 0.8 "run directly" line on a fuzzy match, which keeps it safe.

use jitmo_ipc::{ContextNote, Span, TaskScore, Understanding};
use jitmo_model_worker::{rules, WorkerClient};
use jitmo_skills::{Skill, Slot};
use std::collections::BTreeSet;
use std::sync::Mutex;

/// Thresholds from LLD 07, tuned in SP1.
pub const RUN_DIRECTLY: f32 = 0.80;
pub const ASK_TO_CONFIRM: f32 = 0.50;
pub const DECLINE_AT: f32 = 0.60;
/// Below this the best guess is too weak to even ask about (V1 has no planner to
/// hand it to), so the request is declined instead of showing three random options.
pub const TOO_WEAK_TO_ASK: f32 = 0.30;

/// What the flow should do after routing (LLD 07, "Confidence rules").
#[derive(Debug, Clone, PartialEq)]
pub enum RouteDecision {
    /// Top task 0.80 or higher, not chained: run it.
    Run { skill_id: String },
    /// Top task 0.50 to 0.80: ask the user to confirm it.
    ConfirmTask { skill_id: String },
    /// Out of scope 0.60 or higher: decline and explain.
    Decline,
    /// Below 0.50: ask one short question with the top 2 or 3 tasks.
    Ask { skill_ids: Vec<String> },
    /// Looks like a chain. Needs a saved template or the planner (not built yet).
    Chained,
}

/// Apply the confidence rules to a router result.
pub fn decide(u: &Understanding) -> RouteDecision {
    let Some(top) = u.tasks.first() else {
        return RouteDecision::Decline;
    };
    if u.out_of_scope >= DECLINE_AT {
        return RouteDecision::Decline;
    }
    if top.score < TOO_WEAK_TO_ASK && !u.chained {
        return RouteDecision::Decline;
    }
    if u.chained {
        return RouteDecision::Chained;
    }
    if top.score >= RUN_DIRECTLY {
        RouteDecision::Run {
            skill_id: top.id.clone(),
        }
    } else if top.score >= ASK_TO_CONFIRM {
        RouteDecision::ConfirmTask {
            skill_id: top.id.clone(),
        }
    } else {
        RouteDecision::Ask {
            skill_ids: u.tasks.iter().take(3).map(|t| t.id.clone()).collect(),
        }
    }
}

/// Router over a fixed set of skills. With a model worker attached it scores
/// tasks with the Tier 0 encoder; without one (or when the worker is down) it
/// falls back to the rule-based example matching.
pub struct Router {
    skills: Vec<Skill>,
    worker: Option<Mutex<WorkerClient>>,
}

/// Asks the extractor for slot spans: (text, task label, context). `None` means it
/// could not answer (no worker, or the worker is down).
type ExtractFn<'a> = dyn FnMut(&str, Option<&str>, &ContextNote) -> Option<Vec<Span>> + 'a;

/// One example, split into words. A word like `{app}` is a placeholder.
struct Example<'a> {
    words: Vec<String>,
    skill: &'a Skill,
}

impl Router {
    pub fn new(skills: Vec<Skill>) -> Self {
        Router {
            skills,
            worker: None,
        }
    }

    /// Use the model worker for task scoring and, when the example patterns and the
    /// rules cannot fill a slot, for the extractor.
    pub fn with_worker(mut self, worker: WorkerClient) -> Self {
        self.worker = Some(Mutex::new(worker));
        self
    }

    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    pub fn skill(&self, id: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.id == id)
    }

    /// Route one request.
    pub fn route(&self, text: &str) -> Understanding {
        self.route_with(text, &ContextNote::default())
    }

    /// Route one request, with what the user is looking at (`selected_item`, ...).
    pub fn route_with(&self, text: &str, context: &ContextNote) -> Understanding {
        let stub = self.route_by_examples(text);
        let Some(worker) = &self.worker else {
            return stub;
        };
        let ctx = context.clone();
        let reply = worker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .understand(text, &ctx);
        match reply {
            Ok(u) => {
                let mut extract = |t: &str, task: Option<&str>, c: &ContextNote| {
                    let r = worker
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extract(t, task, c);
                    r.map_err(|e| tracing::warn!("extractor unavailable: {e}"))
                        .ok()
                };
                self.map_worker_result_with(u, text, &ctx, &mut extract)
            }
            Err(e) => {
                tracing::warn!("model worker unavailable, using the rule-based router: {e}");
                stub
            }
        }
    }

    /// Turn task labels (A1, M2, ...) into skill ids.
    #[cfg(test)]
    fn map_worker_result(&self, u: Understanding, text: &str) -> Understanding {
        self.map_worker_result_with(u, text, &ContextNote::default(), &mut |_, _, _| None)
    }

    fn map_worker_result_with(
        &self,
        u: Understanding,
        text: &str,
        ctx: &ContextNote,
        extract: &mut ExtractFn,
    ) -> Understanding {
        let stub = self.route_by_examples(text);
        // A skill with no task label (a dev or test skill) cannot be predicted by the
        // model, so an exact example match on one still wins.
        if let Some(top) = stub.tasks.first() {
            let unlabeled = self.skill(&top.id).is_some_and(|s| s.task.is_none());
            if unlabeled && top.score >= 0.9 {
                return stub;
            }
        }
        // A label is a skill id (the fine-tuned tool encoder, `models/tools`) or a spec task
        // id (the older head over a frozen encoder, `models/router`).
        let skill_for = |label: &str| {
            self.skills.iter().find(|s| s.id == label).or_else(|| {
                self.skills
                    .iter()
                    .find(|s| s.task.as_deref() == Some(label))
            })
        };
        // The best task has no skill built yet: say "can't do that" rather than
        // running a lesser match.
        if let Some(top) = u.tasks.first() {
            if !u.chained && skill_for(&top.id).is_none() && top.score > u.out_of_scope {
                return Understanding {
                    tasks: Vec::new(),
                    out_of_scope: 1.0,
                    chained: false,
                    spans: Vec::new(),
                };
            }
        }
        let tasks: Vec<TaskScore> = u
            .tasks
            .iter()
            .filter_map(|t| {
                skill_for(&t.id).map(|s| TaskScore {
                    id: s.id.clone(),
                    score: t.score,
                })
            })
            .collect();
        // Slot spans: the chosen skill's own example patterns first; what they leave
        // unfilled comes from the rules, then from the extractor model.
        let spans = tasks
            .first()
            .and_then(|t| self.skill(&t.id))
            .map(|s| {
                let pattern = best_for(s, &tokens(text)).1;
                complete_spans(s, pattern, text, ctx, extract)
            })
            .unwrap_or_default();
        Understanding {
            tasks,
            out_of_scope: u.out_of_scope,
            chained: u.chained,
            spans,
        }
    }

    fn route_by_examples(&self, text: &str) -> Understanding {
        let request = tokens(text);
        let chained = looks_chained(&request);

        let mut scored: Vec<(f32, &Skill, Vec<Span>)> = Vec::new();
        for skill in &self.skills {
            let best = best_for(skill, &request);
            scored.push((best.0, skill, best.1));
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));

        let best_score = scored.first().map_or(0.0, |s| s.0);
        let spans = scored.first().map(|s| s.2.clone()).unwrap_or_default();
        Understanding {
            tasks: scored
                .iter()
                .filter(|s| s.0 > 0.0)
                .take(5)
                .map(|s| TaskScore {
                    id: s.1.id.clone(),
                    score: s.0,
                })
                .collect(),
            out_of_scope: (1.0 - best_score).clamp(0.0, 1.0),
            chained,
            spans,
        }
    }
}

/// Fill the slots the example patterns left empty. Order of trust:
/// 1. the example pattern (a span it captured is never replaced or duplicated),
/// 2. the rules (dates, closed lists, sentence frames; pure Rust, no model),
/// 3. the extractor model in the worker, asked only if a slot is still empty, so
///    requests the first two can handle never load it.
fn complete_spans(
    skill: &Skill,
    pattern: Vec<Span>,
    text: &str,
    ctx: &ContextNote,
    extract: &mut ExtractFn,
) -> Vec<Span> {
    let needed: Vec<&str> = skill
        .slots
        .values()
        .filter_map(|s| match s {
            Slot::Param(p) => Some(p.kind.as_str()),
            Slot::Fixed(_) => None,
        })
        .collect();
    if unfilled(&needed, &pattern) == 0 {
        return pattern;
    }
    let mut spans = pattern;
    let gaz = rules::Gazetteer::from_context(ctx);
    let found: Vec<Span> = rules::extract_rules(text, &gaz)
        .iter()
        .map(rules::to_slot_span)
        .collect();
    add_spans(&mut spans, found, &needed);
    if unfilled(&needed, &spans) > 0 {
        if let Some(found) = extract(text, skill.task.as_deref(), ctx) {
            add_spans(&mut spans, found, &needed);
        }
    }
    spans
}

/// How many slot-kind needs are still without a span.
fn unfilled(needed: &[&str], spans: &[Span]) -> usize {
    let mut have: Vec<&Span> = spans.iter().collect();
    let mut missing = 0;
    for kind in needed {
        match have.iter().position(|s| s.kind == *kind) {
            Some(i) => {
                have.remove(i);
            }
            None => missing += 1,
        }
    }
    missing
}

/// Append `extra` after `spans`. A kind the existing spans already cover (as many as
/// the skill needs, at least one) takes nothing more, so an example-pattern span is
/// never overridden. A skill's free-text slot accepts the extractor's message or
/// query span.
fn add_spans(spans: &mut Vec<Span>, extra: Vec<Span>, needed: &[&str]) {
    for mut s in extra {
        let wants_text = needed.contains(&"text") && !needed.contains(&s.kind.as_str());
        if wants_text && (s.kind == "message" || s.kind == "query") {
            s.kind = "text".to_string();
        }
        let want = needed.iter().filter(|k| **k == s.kind).count().max(1);
        let have = spans.iter().filter(|o| o.kind == s.kind).count();
        let same_words = |o: &Span| o.text.to_lowercase() == s.text.to_lowercase();
        if have < want && !spans.iter().any(|o| o.kind == s.kind && same_words(o)) {
            spans.push(s);
        }
    }
}

/// Lowercase words with punctuation removed. `{app}` placeholders are kept.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '{' && c != '}')
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// One word of the request. `norm` is what matching compares; `raw` keeps the
/// user's punctuation (like the commas in "ved, rahul, priya"), so captured
/// text still shows how many names were listed.
struct Tok {
    norm: String,
    raw: String,
}

fn tokens(text: &str) -> Vec<Tok> {
    text.to_lowercase()
        .split_whitespace()
        .filter_map(|w| {
            let norm = w.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
            (!norm.is_empty()).then(|| Tok {
                norm,
                raw: w.to_string(),
            })
        })
        .collect()
}

/// Join captured words and drop punctuation at the two ends only.
fn captured_text(toks: &[Tok]) -> String {
    let joined = toks
        .iter()
        .map(|t| t.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    joined
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// The best-scoring example of one skill, with the spans it captured.
fn best_for(skill: &Skill, request: &[Tok]) -> (f32, Vec<Span>) {
    let mut best = (0.0f32, Vec::new());
    for ex in skill.examples.iter().map(|e| Example {
        words: words(e),
        skill,
    }) {
        let (score, spans) = score_example(&ex, request);
        if score > best.0 {
            best = (score, spans);
        }
    }
    best
}

fn is_placeholder(w: &str) -> bool {
    w.starts_with('{') && w.ends_with('}') && w.len() > 2
}

fn looks_chained(request: &[Tok]) -> bool {
    request
        .windows(2)
        .any(|p| p[0].norm == "and" && p[1].norm == "then")
        || request.iter().any(|t| t.norm == "then")
}

fn score_example(ex: &Example, request: &[Tok]) -> (f32, Vec<Span>) {
    // Full match, with placeholders.
    let mut captured: Vec<(String, String)> = Vec::new();
    if match_words(&ex.words, request, &mut captured) {
        let has_placeholder = ex.words.iter().any(|w| is_placeholder(w));
        let spans = captured
            .into_iter()
            .map(|(name, text)| Span {
                kind: slot_type(ex.skill, &name),
                text,
            })
            .collect();
        return (if has_placeholder { 0.9 } else { 1.0 }, spans);
    }
    // Fuzzy: word overlap, capped at 0.7.
    let a: BTreeSet<&str> = ex
        .words
        .iter()
        .filter(|w| !is_placeholder(w))
        .map(String::as_str)
        .collect();
    let b: BTreeSet<&str> = request.iter().map(|t| t.norm.as_str()).collect();
    let both = a.intersection(&b).count() as f32;
    let either = a.union(&b).count() as f32;
    if either == 0.0 {
        return (0.0, Vec::new());
    }
    ((both / either) * 0.7, Vec::new())
}

/// Match `pattern` against `request` from the start to the end. A placeholder
/// takes one or more words. Captures are pushed as (slot name, text).
fn match_words(pattern: &[String], request: &[Tok], out: &mut Vec<(String, String)>) -> bool {
    match pattern.split_first() {
        None => request.is_empty(),
        Some((p, rest)) if is_placeholder(p) => {
            let name = p[1..p.len() - 1].to_string();
            for take in 1..=request.len() {
                let mark = out.len();
                out.push((name.clone(), captured_text(&request[..take])));
                if match_words(rest, &request[take..], out) {
                    return true;
                }
                out.truncate(mark);
            }
            false
        }
        Some((p, rest)) => match request.split_first() {
            Some((r, request_rest)) if &r.norm == p => match_words(rest, request_rest, out),
            _ => false,
        },
    }
}

/// The type of slot `name` in `skill` (such as `app`), or the name itself.
fn slot_type(skill: &Skill, name: &str) -> String {
    match skill.slots.get(name) {
        Some(Slot::Param(p)) => p.kind.clone(),
        _ => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(tasks: &[(&str, f32)], out: f32) -> Understanding {
        Understanding {
            tasks: tasks
                .iter()
                .map(|(id, score)| TaskScore {
                    id: id.to_string(),
                    score: *score,
                })
                .collect(),
            out_of_scope: out,
            chained: false,
            spans: Vec::new(),
        }
    }

    #[test]
    fn model_labels_become_skill_ids_and_keep_example_spans() {
        let r = router();
        let u = r.map_worker_result(model(&[("A1", 0.95), ("A3", 0.02)], 0.01), "open spotify");
        assert_eq!(u.tasks[0].id, "apps.open");
        assert_eq!(u.tasks[0].score, 0.95);
        assert_eq!(u.spans[0].text.to_lowercase(), "spotify");
    }

    #[test]
    fn a_label_that_is_a_skill_id_is_that_skill() {
        // The fine-tuned tool encoder answers with skill ids, not the spec's task ids.
        let r = router();
        let u = r.map_worker_result(
            model(&[("apps.open", 0.91), ("music.control", 0.04)], 0.01),
            "can you pull up spotify for me",
        );
        assert_eq!(u.tasks[0].id, "apps.open");
        assert_eq!(u.tasks[1].id, "music.control");
        assert_eq!(
            decide(&u),
            RouteDecision::Run {
                skill_id: "apps.open".into()
            }
        );
    }

    fn sp(kind: &str, text: &str) -> Span {
        Span {
            kind: kind.into(),
            text: text.into(),
        }
    }

    /// Runs the mapping with a fake extractor that answers `reply` and counts calls.
    fn with_fake_worker(
        text: &str,
        reply: Option<Vec<Span>>,
        calls: &mut Vec<Option<String>>,
    ) -> Understanding {
        let mut fake = |_: &str, task: Option<&str>, _: &ContextNote| {
            calls.push(task.map(str::to_string));
            reply.clone()
        };
        router().map_worker_result_with(
            model(&[("M2", 0.95)], 0.01),
            text,
            &ContextNote::default(),
            &mut fake,
        )
    }

    #[test]
    fn extractor_is_not_asked_when_the_patterns_fill_every_slot() {
        let mut calls = Vec::new();
        let reply = Some(vec![sp("contact", "Wrong Person")]);
        let u = with_fake_worker("send report.pdf to ved on whatsapp", reply, &mut calls);
        assert!(calls.is_empty(), "model must not be consulted: {calls:?}");
        assert_eq!(
            u.spans,
            vec![sp("file_ref", "report.pdf"), sp("contact", "ved")]
        );
    }

    #[test]
    fn extractor_is_not_asked_when_the_rules_fill_every_slot() {
        // No example matches; the rules find the file by its extension and the
        // contact from the closed list Core supplies.
        let mut calls = Vec::new();
        let r = router();
        let ctx = ContextNote {
            contacts: vec!["Zainab".into()],
            ..ContextNote::default()
        };
        let mut fake = |_: &str, _: Option<&str>, _: &ContextNote| {
            calls.push(());
            None
        };
        let u = r.map_worker_result_with(
            model(&[("M2", 0.95)], 0.01),
            "shoot zainab the budget.xlsx",
            &ctx,
            &mut fake,
        );
        assert!(calls.is_empty());
        assert!(u.spans.contains(&sp("contact", "zainab")), "{:?}", u.spans);
        assert!(
            u.spans
                .iter()
                .any(|s| s.kind == "file_ref" && s.text.ends_with("budget.xlsx")),
            "{:?}",
            u.spans
        );
    }

    #[test]
    fn extractor_fills_only_what_is_missing() {
        let mut calls = Vec::new();
        let reply = Some(vec![
            sp("contact", "Zainab"),
            sp("file_ref", "the quarterly thing"),
        ]);
        let u = with_fake_worker(
            "pass the quarterly thing along to Zainab via whatsapp",
            reply,
            &mut calls,
        );
        assert_eq!(calls, vec![Some("M2".to_string())]);
        assert!(u.spans.contains(&sp("contact", "Zainab")), "{:?}", u.spans);
        assert!(
            u.spans.iter().any(|s| s.kind == "file_ref"),
            "{:?}",
            u.spans
        );
    }

    #[test]
    fn extractor_spans_never_override_a_pattern_span() {
        let mut calls = Vec::new();
        // The pattern captures the file; the contact is missing. The model also
        // proposes a different file, which must be ignored.
        let reply = Some(vec![sp("file_ref", "other.pdf"), sp("contact", "Ved")]);
        let u = with_fake_worker(
            "send report.pdf to ved on whatsapp now please",
            reply,
            &mut calls,
        );
        let files: Vec<&Span> = u.spans.iter().filter(|s| s.kind == "file_ref").collect();
        assert!(files.iter().all(|s| s.text != "other.pdf"), "{:?}", u.spans);
        // Whatever fills the contact, there is exactly one span per needed kind.
        assert_eq!(u.spans.iter().filter(|s| s.kind == "contact").count(), 1);
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn a_dead_extractor_leaves_the_pattern_and_rule_spans() {
        let mut calls = Vec::new();
        let u = with_fake_worker(
            "sort of send the invoice thing to someone",
            None,
            &mut calls,
        );
        assert_eq!(u.tasks[0].id, "whatsapp.send_file");
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn the_free_text_slot_takes_the_extractors_message() {
        let skill = Skill::parse(include_str!("../../../skills/stub.send.yaml")).unwrap();
        let needed: Vec<&str> = skill
            .slots
            .values()
            .filter_map(|s| match s {
                Slot::Param(p) => Some(p.kind.as_str()),
                Slot::Fixed(_) => None,
            })
            .collect();
        assert!(needed.contains(&"text"));
        let mut spans = Vec::new();
        add_spans(&mut spans, vec![sp("message", "running late")], &needed);
        assert_eq!(spans, vec![sp("text", "running late")]);
    }

    #[test]
    fn a_task_with_no_skill_is_declined_not_swapped_for_a_lesser_one() {
        let r = router();
        // A2 (play a song) has no skill yet; A1 is a weak second.
        let u = r.map_worker_result(model(&[("A2", 0.9), ("A1", 0.05)], 0.02), "play jazz");
        assert_eq!(decide(&u), RouteDecision::Decline);
    }

    #[test]
    fn model_out_of_scope_declines() {
        let r = router();
        let u = r.map_worker_result(model(&[("A1", 0.2)], 0.8), "dim the lights");
        assert_eq!(decide(&u), RouteDecision::Decline);
    }

    fn router() -> Router {
        let skills = [
            include_str!("../../../skills/apps.open.yaml"),
            include_str!("../../../skills/music.control.yaml"),
            include_str!("../../../skills/whatsapp.send_file.yaml"),
        ]
        .iter()
        .map(|y| Skill::parse(y).unwrap())
        .collect();
        Router::new(skills)
    }

    #[test]
    fn t01_open_spotify_runs_apps_open_with_an_app_span() {
        let u = router().route("Open Spotify.");
        assert_eq!(u.tasks[0].id, "apps.open");
        assert!(u.tasks[0].score >= RUN_DIRECTLY);
        assert_eq!(
            u.spans,
            vec![Span {
                text: "spotify".into(),
                kind: "app".into()
            }]
        );
        assert_eq!(
            decide(&u),
            RouteDecision::Run {
                skill_id: "apps.open".into()
            }
        );
    }

    #[test]
    fn placeholder_takes_several_words() {
        let u = router().route("launch Visual Studio Code");
        assert_eq!(u.spans[0].text, "visual studio code");
    }

    #[test]
    fn exact_literal_example_scores_one() {
        let u = router().route("pause the music");
        assert_eq!(u.tasks[0].id, "music.control");
        assert_eq!(u.tasks[0].score, 1.0);
    }

    #[test]
    fn fuzzy_match_never_runs_directly() {
        let u = router().route("please could you pause the music now");
        assert_eq!(u.tasks[0].id, "music.control");
        assert!(u.tasks[0].score < RUN_DIRECTLY);
        assert!(!matches!(decide(&u), RouteDecision::Run { .. }));
    }

    #[test]
    fn unrelated_request_is_declined() {
        let u = router().route("what is the capital of france");
        assert!(u.out_of_scope >= DECLINE_AT);
        assert_eq!(decide(&u), RouteDecision::Decline);
    }

    #[test]
    fn chained_request_is_flagged() {
        let u = router().route("open spotify and then pause the music");
        assert!(u.chained);
        assert_eq!(decide(&u), RouteDecision::Chained);
    }

    #[test]
    fn confidence_rules_by_score() {
        let mk = |score: f32, oos: f32| Understanding {
            tasks: vec![
                TaskScore {
                    id: "a".into(),
                    score,
                },
                TaskScore {
                    id: "b".into(),
                    score: 0.1,
                },
            ],
            out_of_scope: oos,
            chained: false,
            spans: vec![],
        };
        assert_eq!(
            decide(&mk(0.80, 0.0)),
            RouteDecision::Run {
                skill_id: "a".into()
            }
        );
        assert_eq!(
            decide(&mk(0.79, 0.0)),
            RouteDecision::ConfirmTask {
                skill_id: "a".into()
            }
        );
        assert_eq!(
            decide(&mk(0.50, 0.0)),
            RouteDecision::ConfirmTask {
                skill_id: "a".into()
            }
        );
        assert_eq!(
            decide(&mk(0.49, 0.0)),
            RouteDecision::Ask {
                skill_ids: vec!["a".into(), "b".into()]
            }
        );
        assert_eq!(decide(&mk(0.95, 0.60)), RouteDecision::Decline);
    }

    #[test]
    fn empty_request_is_declined() {
        let u = router().route("   ");
        assert_eq!(decide(&u), RouteDecision::Decline);
    }

    #[test]
    fn every_skill_file_in_the_repo_loads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        let mut n = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let text = std::fs::read_to_string(&path).unwrap();
                Skill::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                n += 1;
            }
        }
        assert!(n >= 3);
    }
}
