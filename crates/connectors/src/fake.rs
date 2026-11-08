//! A scripted connector for tests. It touches no real app.
//!
//! It records every call, and its answers can be set up in advance, so tests
//! can drive confirms, failures and verifier retries without any app.

use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

/// One recorded call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Execute { action: String, args: Args },
    Check { action: String, args: Args },
    Undo { action: String },
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    /// Answers for `check`, used front to back. When empty, `default_check` is used.
    check_script: VecDeque<Result<bool, ConnError>>,
    default_check: bool,
    execute_error: Option<ConnError>,
    /// Text shown to the user as the answer (a fake that must say it is a fake).
    answer: Option<String>,
    /// Errors for particular actions only.
    action_errors: Vec<(String, ConnError)>,
    undoable: bool,
    health: Option<Health>,
    /// Errors for particular executes only, by position (0 = the first call).
    nth_errors: Vec<(usize, ConnError)>,
}

pub struct FakeConnector {
    id: String,
    state: Mutex<State>,
}

impl FakeConnector {
    /// A connector whose actions succeed, whose checks pass, and which cannot undo.
    pub fn new(id: &str) -> Self {
        let state = State {
            default_check: true,
            ..State::default()
        };
        FakeConnector {
            id: id.into(),
            state: Mutex::new(state),
        }
    }

    /// Make every successful `execute` answer with this text instead of plain "Done.".
    pub fn with_answer(self, text: &str) -> Self {
        self.state.lock().unwrap().answer = Some(text.to_string());
        self
    }

    /// Make `execute` fail with this error.
    pub fn fail_execute_with(self, err: ConnError) -> Self {
        self.state.lock().unwrap().execute_error = Some(err);
        self
    }

    /// Make `execute` fail with this error for this one action only.
    pub fn fail_action_with(self, action: &str, err: ConnError) -> Self {
        self.state
            .lock()
            .unwrap()
            .action_errors
            .push((action.to_string(), err));
        self
    }

    /// Make only the `n`th `execute` call (0 = the first) fail with this error.
    pub fn fail_nth_execute(self, n: usize, err: ConnError) -> Self {
        self.state.lock().unwrap().nth_errors.push((n, err));
        self
    }

    /// Script the next `check` answers, in order.
    pub fn script_checks(self, answers: Vec<Result<bool, ConnError>>) -> Self {
        self.state.lock().unwrap().check_script = answers.into();
        self
    }

    /// Answer for any `check` once the script runs out.
    pub fn default_check(self, answer: bool) -> Self {
        self.state.lock().unwrap().default_check = answer;
        self
    }

    /// Let `undo` succeed.
    pub fn undoable(self) -> Self {
        self.state.lock().unwrap().undoable = true;
        self
    }

    pub fn with_health(self, health: Health) -> Self {
        self.state.lock().unwrap().health = Some(health);
        self
    }

    /// Everything that was called, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    pub fn execute_count(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| matches!(c, Call::Execute { .. }))
            .count()
    }

    pub fn check_count(&self) -> usize {
        self.calls()
            .iter()
            .filter(|c| matches!(c, Call::Check { .. }))
            .count()
    }
}

impl Connector for FakeConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn actions(&self) -> Vec<ActionSpec> {
        Vec::new()
    }

    fn health(&self) -> Health {
        self.state.lock().unwrap().health.unwrap_or(Health::Ready)
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let mut s = self.state.lock().unwrap();
        let index = s
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Execute { .. }))
            .count();
        s.calls.push(Call::Execute {
            action: action.into(),
            args: args.clone(),
        });
        if let Some((_, e)) = s.nth_errors.iter().find(|(n, _)| *n == index) {
            return Err(e.clone());
        }
        if let Some((_, e)) = s.action_errors.iter().find(|(a, _)| a == action) {
            return Err(e.clone());
        }
        match &s.execute_error {
            Some(e) => Err(e.clone()),
            None => {
                let mut out = BTreeMap::from([("action".to_string(), action.to_string())]);
                if let Some(a) = &s.answer {
                    out.insert("message".to_string(), a.clone());
                }
                Ok(Output(out))
            }
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(Call::Check {
            action: action.into(),
            args: args.clone(),
        });
        match s.check_script.pop_front() {
            Some(answer) => answer,
            None => Ok(s.default_check),
        }
    }

    fn undo(&self, action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(Call::Undo {
            action: action.into(),
        });
        s.undoable.then_some(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_calls_in_order() {
        let c = FakeConnector::new("fake");
        let args = Args::from([("contact".to_string(), "Ved".to_string())]);
        c.execute("open_chat", &args).unwrap();
        c.check("last_message_is_file", &args).unwrap();
        assert_eq!(c.execute_count(), 1);
        assert_eq!(c.check_count(), 1);
        assert!(matches!(c.calls()[0], Call::Execute { .. }));
    }

    #[test]
    fn scripted_checks_then_default() {
        let c = FakeConnector::new("fake")
            .script_checks(vec![Ok(false)])
            .default_check(true);
        let a = Args::new();
        assert_eq!(c.check("x", &a), Ok(false));
        assert_eq!(c.check("x", &a), Ok(true));
    }

    #[test]
    fn undo_is_none_unless_enabled() {
        let out = Output::default();
        assert!(FakeConnector::new("a").undo("x", &out).is_none());
        assert!(FakeConnector::new("a")
            .undoable()
            .undo("x", &out)
            .unwrap()
            .is_ok());
    }

    #[test]
    fn execute_can_be_made_to_fail() {
        let c =
            FakeConnector::new("a").fail_execute_with(ConnError::NotInstalled("Spotify".into()));
        let err = c.execute("play", &Args::new()).unwrap_err();
        assert_eq!(err.to_string(), "Spotify isn't installed.");
    }
}

#[cfg(test)]
mod answer_tests {
    use super::*;

    #[test]
    fn a_fake_can_say_it_is_a_fake() {
        let c = FakeConnector::new("x").with_answer("Test only.");
        let out = c.execute("send", &Args::new()).unwrap();
        assert_eq!(out.0["message"], "Test only.");
        let plain = FakeConnector::new("y")
            .execute("send", &Args::new())
            .unwrap();
        assert!(!plain.0.contains_key("message"));
    }
}
