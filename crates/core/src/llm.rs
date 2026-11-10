//! The seam for a local language model. The engine stays what it was (rules and the router
//! first, then policy, confirms and checks); a model plugs in behind it for what rules cannot
//! do:
//!
//! 1. **Pick a tool** for a request the router is unsure about or declined, from the router's
//!    best few (open set: a new skill needs no retraining).
//! 2. **Fill typed arguments** for that tool (who, what to say, when), as schema-constrained
//!    values. The engine turns them into the skill's canonical sentence and runs it through the
//!    normal pipeline, so guards, slot resolution, policy, confirms and checks all still apply.
//!
//! The model never runs anything, and text it reads (a mail, a page) is never a command.
//! Free-text answers from a model's memory are not shown: they are guesses (the grounded
//! writer, step 6 of the low-memory brief, will answer only from retrieved text).

use std::collections::BTreeMap;

/// One typed argument as the model sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgBrief {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// One tool as the model sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolBrief {
    pub id: String,
    pub title: String,
    pub examples: Vec<String>,
    /// Empty for skills that have no typed arguments yet.
    pub args: Vec<ArgBrief>,
}

/// What the model decided for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Run this skill on the request as worded (the engine still checks and confirms).
    Tool(String),
    /// Run this skill with these typed values (the engine builds the sentence and checks).
    Call {
        tool: String,
        args: BTreeMap<String, String>,
    },
    /// An answer from the model's own memory. Never shown until it is grounded.
    Answer(String),
    /// Nothing to do: fall back to "I can't do that yet".
    Nothing,
}

pub trait Llm: Send {
    /// Pick a tool (and its arguments when it has them). `recent` is the last request that
    /// worked, for follow-ups.
    fn route(&mut self, request: &str, tools: &[ToolBrief], recent: Option<&str>) -> Pick;

    /// Write an answer to `question` using ONLY `passage` (retrieved text), or `None`. The
    /// engine shows it only if [`crate::grounding::supported`] agrees.
    fn answer_from(&mut self, _question: &str, _passage: &str) -> Option<String> {
        None
    }
}
