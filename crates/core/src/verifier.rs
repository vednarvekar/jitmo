//! Verifier: checks that a step really worked (HLD: "Checks every step really
//! worked"; LLD "Error handling": "Retries once, then reports").
//!
//! What is retried is the *check*, never the action. An action can have side
//! effects (re-running a send could send twice), while a check changes
//! nothing and a UI-driven app is often only a moment slow.

use jitmo_connectors::{Args, Connector};

/// Result of verifying one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The check passed (on the first try or the retry).
    Verified,
    /// The check ran but said the action did not happen, or it could not run.
    /// Carries the one-line message for the user.
    NotVerified { message: String },
}

/// Run `check_action` on the connector. If it fails or errors, run it once more.
///
/// `unverified_message` is the skill-specific line shown on failure, such as
/// "The file may not have sent. Please check WhatsApp."
pub fn verify(
    connector: &dyn Connector,
    check_action: &str,
    args: &Args,
    unverified_message: &str,
) -> Verdict {
    for _attempt in 0..2 {
        if let Ok(true) = connector.check(check_action, args) {
            return Verdict::Verified;
        }
    }
    Verdict::NotVerified { message: unverified_message.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jitmo_connectors::{fake::FakeConnector, ConnError};

    const MSG: &str = "The file may not have sent. Please check WhatsApp.";

    #[test]
    fn passes_first_time_with_one_check() {
        let c = FakeConnector::new("wa");
        assert_eq!(verify(&c, "last_message_is_file", &Args::new(), MSG), Verdict::Verified);
        assert_eq!(c.check_count(), 1);
    }

    #[test]
    fn retries_once_and_passes() {
        let c = FakeConnector::new("wa").script_checks(vec![Ok(false), Ok(true)]);
        assert_eq!(verify(&c, "x", &Args::new(), MSG), Verdict::Verified);
        assert_eq!(c.check_count(), 2);
    }

    #[test]
    fn fails_after_one_retry_with_the_skill_message() {
        let c = FakeConnector::new("wa").default_check(false);
        assert_eq!(
            verify(&c, "x", &Args::new(), MSG),
            Verdict::NotVerified { message: MSG.into() }
        );
        assert_eq!(c.check_count(), 2); // first try + one retry, no more
    }

    #[test]
    fn a_check_error_counts_as_a_failed_check() {
        let c = FakeConnector::new("wa")
            .script_checks(vec![Err(ConnError::UiChanged("WhatsApp".into())), Ok(true)]);
        assert_eq!(verify(&c, "x", &Args::new(), MSG), Verdict::Verified);
    }

    #[test]
    fn the_action_is_never_re_run() {
        let c = FakeConnector::new("wa").default_check(false);
        verify(&c, "x", &Args::new(), MSG);
        assert_eq!(c.execute_count(), 0);
    }
}
