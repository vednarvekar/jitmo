//! Policy engine (LLD 07, "Policy engine"; requirements SR1-SR7, PS9).
//!
//! Plain code, no models. Every step is checked before it runs. The six rules
//! are checked top to bottom and the first match wins:
//!
//! | # | If the step...                                | Then                              |
//! |---|-----------------------------------------------|-----------------------------------|
//! | 1 | deletes anything permanently                  | Block (SR6)                       |
//! | 2 | was triggered by text inside content          | Block and flag (SR5)              |
//! | 3 | sends to more than 10 recipients              | Block in V1                       |
//! | 4 | sends or shares                               | Confirm with recipient + preview  |
//! | 5 | changes data                                  | Run, then show Undo for 10 s      |
//! | 6 | is safe or read only                          | Run right away                    |
//!
//! Blocks are written to the action log right away ([`evaluate_and_log`]).
//! Confirm and Run outcomes are logged by the flow once the step finishes,
//! because their final decision (confirmed, cancelled, ran, failed) is not
//! known yet.

use jitmo_skills::{Risk, Skill};
use jitmo_store::{Decision, Store, StoreError};

/// Most recipients a single send may have in V1 (rule 3).
pub const MAX_RECIPIENTS: u32 = 10;
/// How long the Undo button stays up after a data change (rule 5).
pub const UNDO_WINDOW_SECS: u32 = 10;

/// Facts about one step that the rules look at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFacts {
    pub skill_id: String,
    pub risk: Risk,
    /// The step would delete something permanently.
    pub permanent_delete: bool,
    /// The step was triggered by text found inside an email, message, file or
    /// web page rather than by the user's own request.
    pub triggered_by_content: bool,
    /// How many recipients the step sends or shares to (0 if it sends nothing).
    pub recipients: u32,
}

impl StepFacts {
    /// Build facts from a skill. `permanent_delete` is read from the skill's
    /// capabilities (any ending in `delete:permanent`). The flow supplies the
    /// two facts that only it can know.
    pub fn from_skill(skill: &Skill, triggered_by_content: bool, recipients: u32) -> Self {
        StepFacts {
            skill_id: skill.id.clone(),
            risk: skill.risk,
            permanent_delete: skill.capabilities.iter().any(|c| c.ends_with("delete:permanent")),
            triggered_by_content,
            recipients,
        }
    }
}

/// Why a step was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Rule 1 (SR6).
    PermanentDelete,
    /// Rule 2 (SR5). The flow should also flag the content as suspicious.
    TriggeredByContent,
    /// Rule 3.
    TooManyRecipients,
}

impl BlockReason {
    /// One-line message for the user (LLD, "Error handling": never silent).
    pub fn message(self) -> &'static str {
        match self {
            BlockReason::PermanentDelete => "I can't permanently delete things.",
            BlockReason::TriggeredByContent => {
                "That request came from inside a message or file, not from you, so I didn't act on it."
            }
            BlockReason::TooManyRecipients => "I can't send to more than 10 people at once.",
        }
    }
}

/// What the flow must do with the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Do not run. `flag` is true when the content should be marked suspicious.
    Block { reason: BlockReason, flag: bool },
    /// Show the confirm card (recipient and preview) and wait. Cancel does nothing.
    Confirm,
    /// Run, then offer Undo for this many seconds.
    RunThenUndo { seconds: u32 },
    /// Run right away.
    Run,
}

/// The six rules, in order. First match wins.
pub fn evaluate(f: &StepFacts) -> Outcome {
    // 1. Permanent delete.
    if f.permanent_delete {
        return Outcome::Block { reason: BlockReason::PermanentDelete, flag: false };
    }
    // 2. Triggered by content, not by the user.
    if f.triggered_by_content {
        return Outcome::Block { reason: BlockReason::TriggeredByContent, flag: true };
    }
    // 3. Too many recipients.
    if f.recipients > MAX_RECIPIENTS {
        return Outcome::Block { reason: BlockReason::TooManyRecipients, flag: false };
    }
    match f.risk {
        // 4. Sends or shares.
        Risk::SendsOrShares => Outcome::Confirm,
        // 5. Changes data.
        Risk::ChangesData => Outcome::RunThenUndo { seconds: UNDO_WINDOW_SECS },
        // 6. Safe or read only.
        Risk::Safe | Risk::ReadOnly => Outcome::Run,
    }
}

/// Evaluate a step and write any block to the action log.
///
/// `at` is an ISO-8601 timestamp and `request` is the user's text.
pub fn evaluate_and_log(
    store: &Store,
    at: &str,
    request: &str,
    facts: &StepFacts,
) -> Result<Outcome, StoreError> {
    let outcome = evaluate(facts);
    if let Outcome::Block { reason, .. } = outcome {
        store.log_action(
            at,
            request,
            Some(&facts.skill_id),
            Decision::Blocked,
            Some(reason.message()),
            None,
        )?;
    }
    Ok(outcome)
}

/// Capability check on every call (PS9). Denied unless the skill declared the
/// exact capability, scope included: `files.read:selected` does not allow
/// `files.read:all`.
pub fn capability_allowed(skill: &Skill, requested: &str) -> bool {
    skill.capabilities.iter().any(|c| c == requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(risk: Risk) -> StepFacts {
        StepFacts {
            skill_id: "test.skill".into(),
            risk,
            permanent_delete: false,
            triggered_by_content: false,
            recipients: 0,
        }
    }

    fn block(reason: BlockReason, flag: bool) -> Outcome {
        Outcome::Block { reason, flag }
    }

    // --- one test per rule ------------------------------------------------

    #[test]
    fn rule1_permanent_delete_is_blocked() {
        let f = StepFacts { permanent_delete: true, ..facts(Risk::ChangesData) };
        assert_eq!(evaluate(&f), block(BlockReason::PermanentDelete, false));
    }

    #[test]
    fn rule2_content_triggered_is_blocked_and_flagged() {
        let f = StepFacts { triggered_by_content: true, ..facts(Risk::ReadOnly) };
        assert_eq!(evaluate(&f), block(BlockReason::TriggeredByContent, true));
    }

    #[test]
    fn rule3_more_than_ten_recipients_is_blocked() {
        let f = StepFacts { recipients: 11, ..facts(Risk::SendsOrShares) };
        assert_eq!(evaluate(&f), block(BlockReason::TooManyRecipients, false));
    }

    #[test]
    fn rule3_exactly_ten_recipients_still_needs_confirm() {
        let f = StepFacts { recipients: 10, ..facts(Risk::SendsOrShares) };
        assert_eq!(evaluate(&f), Outcome::Confirm);
    }

    #[test]
    fn rule4_send_or_share_needs_confirm() {
        let f = StepFacts { recipients: 1, ..facts(Risk::SendsOrShares) };
        assert_eq!(evaluate(&f), Outcome::Confirm);
    }

    #[test]
    fn rule5_changes_data_runs_then_offers_undo_for_10_seconds() {
        assert_eq!(evaluate(&facts(Risk::ChangesData)), Outcome::RunThenUndo { seconds: 10 });
    }

    #[test]
    fn rule6_safe_and_read_only_run_right_away() {
        assert_eq!(evaluate(&facts(Risk::Safe)), Outcome::Run);
        assert_eq!(evaluate(&facts(Risk::ReadOnly)), Outcome::Run);
    }

    // --- order: an earlier rule wins over a later one ------------------------

    #[test]
    fn rule1_beats_rule2() {
        let f = StepFacts {
            permanent_delete: true,
            triggered_by_content: true,
            ..facts(Risk::ChangesData)
        };
        assert_eq!(evaluate(&f), block(BlockReason::PermanentDelete, false));
    }

    #[test]
    fn rule2_beats_rule3() {
        let f = StepFacts {
            triggered_by_content: true,
            recipients: 50,
            ..facts(Risk::SendsOrShares)
        };
        assert_eq!(evaluate(&f), block(BlockReason::TriggeredByContent, true));
    }

    #[test]
    fn rule2_beats_rule4_a_content_triggered_send_is_blocked_not_confirmed() {
        let f = StepFacts {
            triggered_by_content: true,
            recipients: 1,
            ..facts(Risk::SendsOrShares)
        };
        assert!(matches!(evaluate(&f), Outcome::Block { .. }));
    }

    #[test]
    fn rule3_beats_rule4() {
        let f = StepFacts { recipients: 11, ..facts(Risk::SendsOrShares) };
        assert!(matches!(evaluate(&f), Outcome::Block { reason: BlockReason::TooManyRecipients, .. }));
    }

    // --- action log ----------------------------------------------------------

    #[test]
    fn blocks_are_written_to_the_action_log() {
        let store = Store::open_in_memory().unwrap();
        let f = StepFacts { permanent_delete: true, ..facts(Risk::ChangesData) };
        let out = evaluate_and_log(&store, "2026-11-08T12:00:00Z", "delete it forever", &f).unwrap();
        assert!(matches!(out, Outcome::Block { .. }));
        let rows = store.recent_actions(5).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].decision, "blocked");
        assert_eq!(rows[0].skill_id.as_deref(), Some("test.skill"));
        assert_eq!(rows[0].result.as_deref(), Some(BlockReason::PermanentDelete.message()));
    }

    #[test]
    fn non_blocks_are_not_logged_here() {
        let store = Store::open_in_memory().unwrap();
        evaluate_and_log(&store, "2026-11-08T12:00:00Z", "mute", &facts(Risk::Safe)).unwrap();
        assert!(store.recent_actions(5).unwrap().is_empty());
    }

    // --- capabilities (PS9) --------------------------------------------------

    fn sample_skill() -> Skill {
        Skill::parse(include_str!("../../../skills/whatsapp.send_file.yaml")).unwrap()
    }

    #[test]
    fn declared_capability_is_allowed_and_everything_else_is_denied() {
        let s = sample_skill();
        assert!(capability_allowed(&s, "files.read:selected"));
        assert!(!capability_allowed(&s, "files.read:all"));
        assert!(!capability_allowed(&s, "gmail.send"));
    }

    #[test]
    fn facts_from_the_sample_skill_need_a_confirm() {
        let f = StepFacts::from_skill(&sample_skill(), false, 1);
        assert!(!f.permanent_delete);
        assert_eq!(evaluate(&f), Outcome::Confirm);
    }

    #[test]
    fn permanent_delete_is_read_from_capabilities() {
        let mut s = sample_skill();
        s.capabilities.push("files.delete:permanent".into());
        assert!(StepFacts::from_skill(&s, false, 0).permanent_delete);
    }
}
