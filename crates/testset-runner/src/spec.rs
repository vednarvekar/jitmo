//! Expectations for the tests that can be automated today (`testset/automated.yaml`).
//! A test with no entry here is reported as "not built", never as a pass.

use serde::Deserialize;
use std::collections::BTreeMap;

/// What a passing run looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expect {
    /// The right skill ran, its steps were called, the check passed.
    Ran,
    /// The engine said no, clearly, and touched nothing.
    Declined,
    /// A confirm was shown, the user cancelled, and nothing ran.
    Cancelled,
    /// One question was asked and nothing ran before the answer.
    AsksQuestion,
}

/// A saved contact that exists for one test.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactSpec {
    pub name: String,
    pub phone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallSpec {
    pub connector: String,
    pub action: String,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// The exact words typed into the bar.
    pub request: String,
    pub expect: Expect,
    /// For `ran`: the skill that must have run.
    pub skill: Option<String>,
    /// For `ran`: connector calls that must have happened, in order.
    #[serde(default)]
    pub calls: Vec<CallSpec>,
    /// Answers to confirm cards, in order. `true` approves. Default: approve.
    #[serde(default)]
    pub confirm_answers: Vec<bool>,
    /// Only meaningful with the model worker (the rule-based router cannot route
    /// loose wording). Without `--worker` the test is left as not built.
    #[serde(default)]
    pub needs_model: bool,
    /// Files that exist for this test (names; they live in `C:/Docs`).
    #[serde(default)]
    pub files: Vec<String>,
    /// Which of `files` are selected in Explorer when the bar opens (names, in order).
    #[serde(default)]
    pub selected: Vec<String>,
    /// Folders that exist for this test (names; they live in `C:/Docs`).
    #[serde(default)]
    pub folders: Vec<String>,
    /// Contacts saved before the request is typed (as if learned or imported earlier).
    #[serde(default)]
    pub contacts: Vec<ContactSpec>,
    /// Installed apps (display names) known to the engine for this test.
    #[serde(default)]
    pub apps: Vec<String>,
    /// For `ran`: a part of the answer shown to the user (case ignored).
    pub message_contains: Option<String>,
    /// A part of the confirm text that must appear.
    pub confirm_contains: Option<String>,
    /// With `--live`, use the real connector for these ids (they act on the real PC).
    #[serde(default)]
    pub live: Vec<String>,
    /// The test is expected to fail today, for this reason. It is reported but
    /// does not fail the run. Remove the line when the gap is fixed.
    pub known_failing: Option<String>,
}

pub fn load(yaml: &str) -> Result<BTreeMap<String, Spec>, String> {
    serde_yaml_ng::from_str(yaml).map_err(|e| format!("automated.yaml: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_repo_file_and_every_entry_names_a_real_test() {
        let specs = load(include_str!("../../../testset/automated.yaml")).unwrap();
        assert!(!specs.is_empty());
        for id in specs.keys() {
            let n: u32 = id.strip_prefix('T').unwrap().parse().unwrap();
            assert!((1..=100).contains(&n), "{id} is not one of T01..T100");
        }
    }

    #[test]
    fn unknown_fields_are_refused() {
        assert!(load("T01: { request: x, expect: ran, typo: 1 }").is_err());
    }

    #[test]
    fn ran_without_a_skill_is_allowed_to_parse() {
        // The runner reports the missing skill as a failure with a reason.
        assert!(load("T01: { request: x, expect: ran }").is_ok());
    }
}
