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
use std::collections::BTreeMap;

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

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub id: String,
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
}

/// True for capabilities that send or share, like `whatsapp.send` or `gmail.share:link`.
fn is_send_capability(cap: &str) -> bool {
    let name = cap.split(':').next().unwrap_or(cap);
    matches!(name.rsplit('.').next(), Some("send" | "share"))
}

impl Skill {
    /// Parse YAML without checking signatures. Use [`load_signed`] for real files.
    pub fn parse(yaml: &str) -> Result<Skill, SkillError> {
        let skill: Skill = serde_yaml_ng::from_str(yaml).map_err(|e| SkillError::Parse(e.to_string()))?;
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
            Undo::Keyword(k) if k != "none" => p.push(format!("undo must be `none` or a list of steps, not `{k}`")),
            Undo::Steps(s) if s.is_empty() => p.push("undo steps are empty; write `undo: none` instead".into()),
            _ => {}
        }

        if p.is_empty() {
            Ok(())
        } else {
            Err(SkillError::Invalid { id: self.id.clone(), problems: p })
        }
    }
}

/// Verify the detached Ed25519 signature over the raw file bytes, then parse and validate.
///
/// `signature` is the 64 raw bytes of the `.sig` file. Anything that is not
/// signed by `trusted` is refused.
pub fn load_signed(yaml: &[u8], signature: &[u8], trusted: &PublicKey) -> Result<Skill, SkillError> {
    let sig = Signature::from_slice(signature).map_err(|_| SkillError::BadSignature)?;
    trusted.verify(yaml, &sig).map_err(|_| SkillError::BadSignature)?;
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
        let yaml = SAMPLE.replace("confirm:\n  text: \"Send {file.name} to {contact.name} on WhatsApp?\"\n", "");
        assert!(problems(&yaml).iter().any(|m| m.contains("must have a confirm")));
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
        assert!(problems(&yaml).iter().any(|m| m.contains("slot `contact` must list its resolve order")));
    }

    #[test]
    fn send_capability_needs_send_risk() {
        let yaml = SAMPLE.replace("risk: sends_or_shares", "risk: read_only");
        assert!(problems(&yaml).iter().any(|m| m.contains("requires risk: sends_or_shares")));
    }

    #[test]
    fn bad_undo_keyword_is_rejected() {
        let yaml = SAMPLE.replace("undo: none", "undo: maybe");
        assert!(problems(&yaml).iter().any(|m| m.contains("undo must be")));
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
