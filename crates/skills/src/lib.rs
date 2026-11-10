//! YAML skill loader and validator (LLD 07, "Skill file format").
//!
//! A skill file is loaded in two steps:
//! 1. [`load_signed`] checks the Ed25519 signature over the raw bytes. An
//!    unsigned or altered file is refused before it is parsed.
//! 2. The YAML is parsed and [`Skill::validate`] enforces the LLD rules.
//!
//! This crate has no OS code.

use ed25519_compact::{PublicKey, Signature};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("skill file is not signed correctly")]
    BadSignature,
    #[error("skill file could not be read: {0}")]
    Parse(String),
    #[error("skill `{id}` is invalid: {}", problems.join("; "))]
    Invalid { id: String, problems: Vec<String> },
}

/// How risky a skill is. Drives the policy engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Safe,
    ReadOnly,
    ChangesData,
    SendsOrShares,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub cpu_ms: u32,
    pub timeout_s: u32,
}

/// A slot is either a full definition or a fixed value (like `app: whatsapp`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Slot {
    Fixed(String),
    Param(SlotParam),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotParam {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    /// Sources to try, in order (selected, named, recent, usual, fuzzy...).
    #[serde(default)]
    pub resolve: Vec<String>,
    /// The short question to ask when this slot is required, free text, and the user
    /// said nothing for it ("Which song?"). Optional: a default is made from the kind.
    #[serde(default)]
    pub ask: Option<String>,
}

/// Slot kinds whose value is whatever the user types. Only these can be asked for
/// in words when missing; files, folders, contacts and apps must come from lists.
pub const FREE_TEXT_KINDS: &[&str] = &[
    "query", "media", "message", "event", "note", "text", "place",
];

impl SlotParam {
    /// True when a missing value can be asked for as plain text.
    pub fn is_free_text(&self) -> bool {
        FREE_TEXT_KINDS.contains(&self.kind.as_str())
    }

    /// The question to show when this slot is missing: the skill's `ask:` text, or a
    /// default made from the kind.
    pub fn question(&self, slot_name: &str) -> String {
        if let Some(q) = self.ask.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            return q.to_string();
        }
        match self.kind.as_str() {
            "query" => "What should I search for?".to_string(),
            "media" => "Which one should I play?".to_string(),
            "message" => "What do you want to say?".to_string(),
            "event" => "What should I remind you about?".to_string(),
            "note" => "What should I note down?".to_string(),
            "place" => "Which place?".to_string(),
            "text" => "What should it say?".to_string(),
            _ => format!("What is the {}?", slot_name.replace(['_', '.'], " ")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub connector: String,
    pub action: String,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

/// `undo: none`, or the steps that reverse the skill.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Undo {
    Keyword(String),
    Steps(Vec<Step>),
}

/// One typed argument of a skill, for a model that fills arguments instead of the engine
/// parsing English.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolArg {
    /// person, text, time, duration, app, file, place, number...
    #[serde(rename = "type", default = "default_arg_kind")]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    /// What it is, for the model ("who to email").
    pub description: String,
    /// The question to ask when it is missing ("Who should I email?").
    #[serde(default)]
    pub ask: Option<String>,
}

fn default_arg_kind() -> String {
    "text".into()
}

/// The typed shape of a skill: its arguments and the sentence they make. The engine turns the
/// arguments into that sentence and runs it through the normal pipeline, so every guard,
/// slot, policy, confirm and check still applies; nothing about safety moves into a model.
///
/// `phrase` uses `{arg}` and optional groups `[ in {place}]`, which appear only when every
/// argument inside them has a value.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub args: BTreeMap<String, ToolArg>,
    pub phrase: String,
}

impl Tool {
    /// The sentence for these argument values, or the name of the required one that is missing.
    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<String, String> {
        for (name, a) in &self.args {
            if a.required && values.get(name).is_none_or(|v| v.trim().is_empty()) {
                return Err(name.clone());
            }
        }
        let has = |n: &str| values.get(n).is_some_and(|v| !v.trim().is_empty());
        let mut out = String::new();
        let mut chars = self.phrase.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '[' {
                let mut group = String::new();
                for g in chars.by_ref() {
                    if g == ']' {
                        break;
                    }
                    group.push(g);
                }
                let names = placeholders(&group).unwrap_or_default();
                if names.iter().all(|n| has(n)) {
                    out.push_str(&fill_values(&group, values));
                }
            } else if c == '{' {
                let mut name = String::new();
                for g in chars.by_ref() {
                    if g == '}' {
                        break;
                    }
                    name.push(g);
                }
                out.push_str(values.get(&name).map_or("", |v| v.trim()));
            } else {
                out.push(c);
            }
        }
        Ok(out.split_whitespace().collect::<Vec<_>>().join(" "))
    }
}

fn fill_values(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        out.push_str(values.get(&after[..close]).map_or("", |v| v.trim()));
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub id: String,
    /// A short human name ("Play music"), used in "Did you mean: ...?" and choices.
    #[serde(default)]
    pub title: Option<String>,
    /// The task label from the task list (A1, M2, ...) that the model router predicts.
    #[serde(default)]
    pub task: Option<String>,
    pub version: u32,
    pub category: String,
    pub capabilities: Vec<String>,
    pub network: String,
    pub limits: Limits,
    pub risk: Risk,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub slots: BTreeMap<String, Slot>,
    pub confirm: Option<Confirm>,
    pub steps: Vec<Step>,
    /// The verifier rule: proves the action really happened.
    pub check: Option<Step>,
    pub undo: Undo,
    pub fallback: Option<String>,
    /// Typed arguments a model can fill, and the sentence they make (see [`Tool`]).
    #[serde(default)]
    pub tool: Option<Tool>,
}

/// True for capabilities that send or share, like `whatsapp.send` or `gmail.share:link`.
fn is_send_capability(cap: &str) -> bool {
    let name = cap.split(':').next().unwrap_or(cap);
    matches!(name.rsplit('.').next(), Some("send" | "share"))
}

impl Skill {
    /// Does this skill send or share something (so a plan must keep its confirm)?
    pub fn sends(&self) -> bool {
        self.capabilities.iter().any(|c| is_send_capability(c))
    }

    /// Parse YAML without checking signatures. Use [`load_signed`] for real files.
    pub fn parse(yaml: &str) -> Result<Skill, SkillError> {
        let skill: Skill =
            serde_yaml_ng::from_str(yaml).map_err(|e| SkillError::Parse(e.to_string()))?;
        skill.validate()?;
        Ok(skill)
    }

    /// Enforce the LLD rules. Reports every problem at once.
    pub fn validate(&self) -> Result<(), SkillError> {
        let mut p: Vec<String> = Vec::new();

        if self.id.trim().is_empty() || !self.id.contains('.') {
            p.push("id must look like `category.name`".into());
        }
        if self.category.trim().is_empty() {
            p.push("category is empty".into());
        }
        if self.capabilities.is_empty() {
            p.push("capabilities must list what the skill needs (use a narrow scope)".into());
        }
        if self.limits.cpu_ms == 0 || self.limits.timeout_s == 0 {
            p.push("limits must be above zero".into());
        }
        if self.steps.is_empty() {
            p.push("steps is empty".into());
        }
        if self.check.is_none() {
            p.push("every skill needs a check (verifier rule)".into());
        }
        if self.examples.is_empty() {
            p.push("examples are needed to train the router".into());
        }

        for (name, slot) in &self.slots {
            if let Slot::Param(sp) = slot {
                if sp.resolve.is_empty() {
                    p.push(format!("slot `{name}` must list its resolve order"));
                }
                if sp.ask.is_some() && !sp.is_free_text() {
                    p.push(format!(
                        "slot `{name}` has `ask:` but type `{}` comes from a list, not typed words",
                        sp.kind
                    ));
                }
            }
        }

        if let Some(t) = &self.tool {
            let names: BTreeSet<&str> = t.args.keys().map(String::as_str).collect();
            let flat = t.phrase.replace(['[', ']'], "");
            match placeholders(&flat) {
                Err(e) => p.push(format!("tool phrase {e}")),
                Ok(used) => {
                    for u in &used {
                        if !names.contains(u.as_str()) {
                            p.push(format!(
                                "tool phrase uses `{{{u}}}`, which is not a tool arg"
                            ));
                        }
                    }
                    for (n, a) in &t.args {
                        if a.required && !used.iter().any(|u| u == n) {
                            p.push(format!("required tool arg `{n}` is not in the phrase"));
                        }
                    }
                }
            }
        }

        let sends = self.risk == Risk::SendsOrShares;
        if sends && self.confirm.is_none() {
            p.push("a sends_or_shares skill must have a confirm".into());
        }
        // Risk follows capabilities: a send capability always means a confirm.
        if self.capabilities.iter().any(|c| is_send_capability(c)) && !sends {
            p.push("a send/share capability requires risk: sends_or_shares".into());
        }

        match &self.undo {
            Undo::Keyword(k) if k != "none" => {
                p.push(format!("undo must be `none` or a list of steps, not `{k}`"))
            }
            Undo::Steps(s) if s.is_empty() => {
                p.push("undo steps are empty; write `undo: none` instead".into())
            }
            _ => {}
        }

        // Every {placeholder} must name a declared slot, so a skill can never
        // reach the user with a template the engine cannot fill.
        // `{request}` is built in: the user's whole request, for connectors that read it themselves.
        let mut declared: BTreeSet<&str> = self.slots.keys().map(String::as_str).collect();
        declared.insert("request");
        // The words to send, worked out by the engine for send skills.
        declared.insert("message");
        if let Some(c) = &self.confirm {
            check_template("confirm text", &c.text, &declared, true, &mut p);
        }
        let undo_steps: &[Step] = match &self.undo {
            Undo::Steps(s) => s,
            Undo::Keyword(_) => &[],
        };
        for (i, step) in self
            .steps
            .iter()
            .chain(self.check.iter())
            .chain(undo_steps)
            .enumerate()
        {
            for (arg, value) in &step.args {
                let place = format!("step {} arg `{arg}`", i + 1);
                check_template(&place, value, &declared, true, &mut p);
            }
        }
        for example in &self.examples {
            // Examples match words, so a slot there stands for the whole value.
            check_template("example", example, &declared, false, &mut p);
        }

        if p.is_empty() {
            Ok(())
        } else {
            Err(SkillError::Invalid {
                id: self.id.clone(),
                problems: p,
            })
        }
    }
}

/// The names inside `{...}` in a template. Each is `slot` or `slot.field`.
/// An unclosed brace or an empty or odd name is an error.
pub fn placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err("has a `{` with no closing `}`".into());
        };
        let name = &after[..close];
        let well_formed = !name.is_empty()
            && name.split('.').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '_')
            });
        if !well_formed {
            return Err(format!("has an invalid placeholder `{{{name}}}`"));
        }
        names.push(name.to_string());
        rest = &after[close + 1..];
    }
    Ok(names)
}

fn check_template(
    place: &str,
    template: &str,
    declared: &BTreeSet<&str>,
    allow_fields: bool,
    problems: &mut Vec<String>,
) {
    match placeholders(template) {
        Err(e) => problems.push(format!("{place} {e}")),
        Ok(names) => {
            for name in names {
                let (slot, field) = match name.split_once('.') {
                    Some((s, f)) => (s, Some(f)),
                    None => (name.as_str(), None),
                };
                if !declared.contains(slot) {
                    problems.push(format!("{place}: `{{{name}}}` does not match any slot"));
                } else if field.is_some() && !allow_fields {
                    problems.push(format!("{place}: `{{{name}}}` cannot use a field here"));
                }
            }
        }
    }
}

/// Verify the detached Ed25519 signature over the raw file bytes, then parse and validate.
///
/// `signature` is the 64 raw bytes of the `.sig` file. Anything that is not
/// signed by `trusted` is refused.
pub fn load_signed(
    yaml: &[u8],
    signature: &[u8],
    trusted: &PublicKey,
) -> Result<Skill, SkillError> {
    let sig = Signature::from_slice(signature).map_err(|_| SkillError::BadSignature)?;
    trusted
        .verify(yaml, &sig)
        .map_err(|_| SkillError::BadSignature)?;
    let text = std::str::from_utf8(yaml).map_err(|e| SkillError::Parse(e.to_string()))?;
    Skill::parse(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_compact::{KeyPair, Seed};

    const SAMPLE: &str = include_str!("../../../skills/whatsapp.send_file.yaml");

    fn key() -> KeyPair {
        KeyPair::from_seed(Seed::new([42u8; 32]))
    }

    fn problems(yaml: &str) -> Vec<String> {
        match Skill::parse(yaml) {
            Err(SkillError::Invalid { problems, .. }) => problems,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn sample_skill_loads() {
        let s = Skill::parse(SAMPLE).unwrap();
        assert_eq!(s.id, "whatsapp.send_file");
        assert_eq!(s.risk, Risk::SendsOrShares);
        assert_eq!(s.slots["app"], Slot::Fixed("whatsapp".into()));
        assert_eq!(s.undo, Undo::Keyword("none".into()));
    }

    #[test]
    fn signed_file_loads() {
        let kp = key();
        let sig = kp.sk.sign(SAMPLE.as_bytes(), None);
        let s = load_signed(SAMPLE.as_bytes(), &sig[..], &kp.pk).unwrap();
        assert_eq!(s.id, "whatsapp.send_file");
    }

    #[test]
    fn altered_file_is_refused() {
        let kp = key();
        let sig = kp.sk.sign(SAMPLE.as_bytes(), None);
        let evil = SAMPLE.replace("timeout_s: 15", "timeout_s: 9999");
        let r = load_signed(evil.as_bytes(), &sig[..], &kp.pk);
        assert!(matches!(r, Err(SkillError::BadSignature)));
    }

    #[test]
    fn wrong_signer_and_missing_signature_are_refused() {
        let kp = key();
        let other = KeyPair::from_seed(Seed::new([7u8; 32]));
        let sig = other.sk.sign(SAMPLE.as_bytes(), None);
        assert!(matches!(
            load_signed(SAMPLE.as_bytes(), &sig[..], &kp.pk),
            Err(SkillError::BadSignature)
        ));
        assert!(matches!(
            load_signed(SAMPLE.as_bytes(), &[], &kp.pk),
            Err(SkillError::BadSignature)
        ));
    }

    #[test]
    fn send_without_confirm_is_rejected() {
        let yaml = SAMPLE.replace(
            "confirm:\n  text: \"Send {file.name} to {contact.label} on WhatsApp?\"\n",
            "",
        );
        assert!(problems(&yaml)
            .iter()
            .any(|m| m.contains("must have a confirm")));
    }

    #[test]
    fn missing_check_is_rejected() {
        // Parse as a value, drop `check`, and re-serialize.
        let mut v: serde_yaml_ng::Value = serde_yaml_ng::from_str(SAMPLE).unwrap();
        v.as_mapping_mut().unwrap().remove("check");
        let yaml = serde_yaml_ng::to_string(&v).unwrap();
        assert!(problems(&yaml).iter().any(|m| m.contains("needs a check")));
    }

    #[test]
    fn slot_without_resolve_order_is_rejected() {
        let yaml = SAMPLE.replace("resolve: [named_contact, usual_contact]", "resolve: []");
        assert!(problems(&yaml)
            .iter()
            .any(|m| m.contains("slot `contact` must list its resolve order")));
    }

    #[test]
    fn ask_is_optional_free_text_only_and_has_defaults() {
        let sp = |kind: &str, ask: Option<&str>| SlotParam {
            kind: kind.into(),
            required: true,
            resolve: vec!["named_text".into()],
            ask: ask.map(str::to_string),
        };
        assert_eq!(
            sp("media", None).question("song"),
            "Which one should I play?"
        );
        assert_eq!(
            sp("media", Some("Which song?")).question("song"),
            "Which song?"
        );
        assert_eq!(
            sp("media", Some("  ")).question("song"),
            "Which one should I play?"
        );
        assert_eq!(sp("odd", None).question("my_slot"), "What is the my slot?");
        for k in [
            "query", "media", "message", "event", "note", "text", "place",
        ] {
            assert!(sp(k, None).is_free_text(), "{k}");
        }
        for k in ["file_ref", "folder", "contact", "app"] {
            assert!(!sp(k, None).is_free_text(), "{k}");
        }
        // `ask:` parses on a text slot and is refused on a list-resolved one.
        let ok = SAMPLE.replace(
            "resolve: [named_contact, usual_contact]",
            "resolve: [named_contact, usual_contact]\n    ask: \"Who?\"",
        );
        assert!(problems(&ok)
            .iter()
            .any(|m| m.contains("slot `contact` has `ask:`")));
    }

    #[test]
    fn send_capability_needs_send_risk() {
        let yaml = SAMPLE.replace("risk: sends_or_shares", "risk: read_only");
        assert!(problems(&yaml)
            .iter()
            .any(|m| m.contains("requires risk: sends_or_shares")));
    }

    #[test]
    fn bad_undo_keyword_is_rejected() {
        let yaml = SAMPLE.replace("undo: none", "undo: maybe");
        assert!(problems(&yaml).iter().any(|m| m.contains("undo must be")));
    }

    #[test]
    fn placeholder_names_are_parsed() {
        assert_eq!(
            placeholders("Send {file.name} to {contact}").unwrap(),
            vec!["file.name", "contact"]
        );
        assert!(placeholders("no placeholders").unwrap().is_empty());
        assert!(placeholders("open { app").is_err());
        assert!(placeholders("bad {}").is_err());
        assert!(placeholders("bad {a..b}").is_err());
        assert!(placeholders("bad {a b}").is_err());
    }

    #[test]
    fn a_placeholder_for_a_missing_slot_is_rejected_everywhere() {
        let confirm = SAMPLE.replace("{contact.label}", "{person.name}");
        assert!(problems(&confirm)
            .iter()
            .any(|m| m.contains("confirm text") && m.contains("{person.name}")));

        let arg = SAMPLE.replace("{file.path}", "{doc.path}");
        assert!(problems(&arg)
            .iter()
            .any(|m| m.contains("arg `file`") && m.contains("{doc.path}")));

        let check = SAMPLE.replace("file_name: \"{file.name}\"", "file_name: \"{nope}\"");
        assert!(problems(&check).iter().any(|m| m.contains("{nope}")));
    }

    #[test]
    fn dotted_fields_are_allowed_in_confirms_and_args_but_not_in_examples() {
        assert!(Skill::parse(SAMPLE).is_ok()); // uses {file.name}, {contact.label}, {file.path}
        let example = SAMPLE.replace(
            "send this file to ved on whatsapp",
            "send {file.name} to ved",
        );
        assert!(problems(&example)
            .iter()
            .any(|m| m.contains("cannot use a field")));
    }

    #[test]
    fn an_unclosed_brace_is_rejected() {
        // Only the last placeholder of the confirm loses its closing brace.
        let yaml = SAMPLE.replace("{contact.label} on WhatsApp", "{contact.label on WhatsApp");
        assert!(problems(&yaml)
            .iter()
            .any(|m| m.contains("confirm text") && m.contains("no closing")));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let yaml = format!("{SAMPLE}\nrun_anything: true\n");
        assert!(matches!(Skill::parse(&yaml), Err(SkillError::Parse(_))));
    }

    #[test]
    fn all_problems_are_reported_together() {
        let yaml = SAMPLE
            .replace("risk: sends_or_shares", "risk: read_only")
            .replace("resolve: [named_contact, usual_contact]", "resolve: []");
        assert!(problems(&yaml).len() >= 2);
    }
}

#[cfg(test)]
mod tool_tests {
    use super::*;

    fn tool() -> Tool {
        let arg = |d: &str, required: bool| ToolArg {
            kind: "text".into(),
            required,
            description: d.into(),
            ask: None,
        };
        Tool {
            args: BTreeMap::from([
                ("what".to_string(), arg("what to do", true)),
                ("when".to_string(), arg("when", false)),
                ("place".to_string(), arg("where", false)),
            ]),
            phrase: "remind me to {what}[ {when}][ in {place}]".into(),
        }
    }

    fn vals(p: &[(&str, &str)]) -> BTreeMap<String, String> {
        p.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_phrase_is_built_from_the_arguments() {
        assert_eq!(
            tool()
                .render(&vals(&[("what", "call mom"), ("when", "at 5pm")]))
                .unwrap(),
            "remind me to call mom at 5pm"
        );
    }

    #[test]
    fn optional_groups_appear_only_when_all_their_arguments_are_there() {
        let t = tool();
        assert_eq!(
            t.render(&vals(&[("what", "stretch")])).unwrap(),
            "remind me to stretch"
        );
        assert_eq!(
            t.render(&vals(&[("what", "stretch"), ("place", "the gym")]))
                .unwrap(),
            "remind me to stretch in the gym"
        );
    }

    #[test]
    fn a_missing_required_argument_is_named() {
        assert_eq!(
            tool().render(&vals(&[("when", "at 5pm")])).unwrap_err(),
            "what"
        );
        assert_eq!(tool().render(&vals(&[("what", "  ")])).unwrap_err(), "what");
    }

    #[test]
    fn a_phrase_naming_an_unknown_arg_is_invalid() {
        let yaml = include_str!("../../../skills/apps.open.yaml")
            .replace("phrase: \"open {app}\"", "phrase: \"open {nope}\"");
        let err = Skill::parse(&yaml).unwrap_err().to_string();
        assert!(err.contains("not a tool arg"), "{err}");
    }
}
