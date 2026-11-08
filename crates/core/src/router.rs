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

use jitmo_ipc::{Span, TaskScore, Understanding};
use jitmo_skills::{Skill, Slot};
use std::collections::BTreeSet;

/// Thresholds from LLD 07, tuned in SP1.
pub const RUN_DIRECTLY: f32 = 0.80;
pub const ASK_TO_CONFIRM: f32 = 0.50;
pub const DECLINE_AT: f32 = 0.60;

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
    if u.chained {
        return RouteDecision::Chained;
    }
    if top.score >= RUN_DIRECTLY {
        RouteDecision::Run { skill_id: top.id.clone() }
    } else if top.score >= ASK_TO_CONFIRM {
        RouteDecision::ConfirmTask { skill_id: top.id.clone() }
    } else {
        RouteDecision::Ask {
            skill_ids: u.tasks.iter().take(3).map(|t| t.id.clone()).collect(),
        }
    }
}

/// Router over a fixed set of skills.
pub struct StubRouter {
    skills: Vec<Skill>,
}

/// One example, split into words. A word like `{app}` is a placeholder.
struct Example<'a> {
    words: Vec<String>,
    skill: &'a Skill,
}

impl StubRouter {
    pub fn new(skills: Vec<Skill>) -> Self {
        StubRouter { skills }
    }

    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    pub fn skill(&self, id: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.id == id)
    }

    /// Route one request.
    pub fn route(&self, text: &str) -> Understanding {
        let request = words(text);
        let chained = looks_chained(&request);

        let mut scored: Vec<(f32, &Skill, Vec<Span>)> = Vec::new();
        for skill in &self.skills {
            let mut best = (0.0f32, Vec::new());
            for ex in skill.examples.iter().map(|e| Example { words: words(e), skill }) {
                let (score, spans) = score_example(&ex, &request);
                if score > best.0 {
                    best = (score, spans);
                }
            }
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
                .map(|s| TaskScore { id: s.1.id.clone(), score: s.0 })
                .collect(),
            out_of_scope: (1.0 - best_score).clamp(0.0, 1.0),
            chained,
            spans,
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

fn is_placeholder(w: &str) -> bool {
    w.starts_with('{') && w.ends_with('}') && w.len() > 2
}

fn looks_chained(request: &[String]) -> bool {
    request.windows(2).any(|p| p[0] == "and" && p[1] == "then") || request.iter().any(|w| w == "then")
}

fn score_example(ex: &Example, request: &[String]) -> (f32, Vec<Span>) {
    // Full match, with placeholders.
    let mut captured: Vec<(String, String)> = Vec::new();
    if match_words(&ex.words, request, &mut captured) {
        let has_placeholder = ex.words.iter().any(|w| is_placeholder(w));
        let spans = captured
            .into_iter()
            .map(|(name, text)| Span { kind: slot_type(ex.skill, &name), text })
            .collect();
        return (if has_placeholder { 0.9 } else { 1.0 }, spans);
    }
    // Fuzzy: word overlap, capped at 0.7.
    let a: BTreeSet<&str> = ex.words.iter().filter(|w| !is_placeholder(w)).map(String::as_str).collect();
    let b: BTreeSet<&str> = request.iter().map(String::as_str).collect();
    let both = a.intersection(&b).count() as f32;
    let either = a.union(&b).count() as f32;
    if either == 0.0 {
        return (0.0, Vec::new());
    }
    ((both / either) * 0.7, Vec::new())
}

/// Match `pattern` against `request` from the start to the end. A placeholder
/// takes one or more words. Captures are pushed as (slot name, text).
fn match_words(pattern: &[String], request: &[String], out: &mut Vec<(String, String)>) -> bool {
    match pattern.split_first() {
        None => request.is_empty(),
        Some((p, rest)) if is_placeholder(p) => {
            let name = p[1..p.len() - 1].to_string();
            for take in 1..=request.len() {
                let mark = out.len();
                out.push((name.clone(), request[..take].join(" ")));
                if match_words(rest, &request[take..], out) {
                    return true;
                }
                out.truncate(mark);
            }
            false
        }
        Some((p, rest)) => match request.split_first() {
            Some((r, request_rest)) if r == p => match_words(rest, request_rest, out),
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

    fn router() -> StubRouter {
        let skills = [
            include_str!("../../../skills/apps.open.yaml"),
            include_str!("../../../skills/music.pause.yaml"),
            include_str!("../../../skills/whatsapp.send_file.yaml"),
        ]
        .iter()
        .map(|y| Skill::parse(y).unwrap())
        .collect();
        StubRouter::new(skills)
    }

    #[test]
    fn t01_open_spotify_runs_apps_open_with_an_app_span() {
        let u = router().route("Open Spotify.");
        assert_eq!(u.tasks[0].id, "apps.open");
        assert!(u.tasks[0].score >= RUN_DIRECTLY);
        assert_eq!(u.spans, vec![Span { text: "spotify".into(), kind: "app".into() }]);
        assert_eq!(decide(&u), RouteDecision::Run { skill_id: "apps.open".into() });
    }

    #[test]
    fn placeholder_takes_several_words() {
        let u = router().route("launch Visual Studio Code");
        assert_eq!(u.spans[0].text, "visual studio code");
    }

    #[test]
    fn exact_literal_example_scores_one() {
        let u = router().route("pause the music");
        assert_eq!(u.tasks[0].id, "music.pause");
        assert_eq!(u.tasks[0].score, 1.0);
    }

    #[test]
    fn fuzzy_match_never_runs_directly() {
        let u = router().route("please could you pause the music now");
        assert_eq!(u.tasks[0].id, "music.pause");
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
                TaskScore { id: "a".into(), score },
                TaskScore { id: "b".into(), score: 0.1 },
            ],
            out_of_scope: oos,
            chained: false,
            spans: vec![],
        };
        assert_eq!(decide(&mk(0.80, 0.0)), RouteDecision::Run { skill_id: "a".into() });
        assert_eq!(decide(&mk(0.79, 0.0)), RouteDecision::ConfirmTask { skill_id: "a".into() });
        assert_eq!(decide(&mk(0.50, 0.0)), RouteDecision::ConfirmTask { skill_id: "a".into() });
        assert_eq!(
            decide(&mk(0.49, 0.0)),
            RouteDecision::Ask { skill_ids: vec!["a".into(), "b".into()] }
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
