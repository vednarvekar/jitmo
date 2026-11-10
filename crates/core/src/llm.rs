//! The seam for a local language model. The engine stays what it was (rules and the router
//! first, then policy, confirms and checks); a model plugs in behind it for the two things
//! rules cannot do:
//!
//! 1. **Pick a tool** for a request the router is unsure about or declined, from the skill
//!    catalog (open set: a new skill needs no retraining).
//! 2. **Answer** a question or write a piece of text no skill covers (explain, translate,
//!    quiz, rewrite), honestly: "I'm not sure" beats a confident guess.
//!
//! The model never runs anything. A picked tool goes through the same guards, slot resolution,
//! policy, confirm and verification as any other, and the text of a mail or page it reads is
//! never a command (see [`Pick`]).

/// One tool as the model sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolBrief {
    pub id: String,
    pub title: String,
    pub examples: Vec<String>,
}

/// What the model decided for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Run this skill (the engine still checks and confirms).
    Tool(String),
    /// No tool fits; this is the answer to show.
    Answer(String),
    /// Nothing to do and nothing to say: fall back to "I can't do that yet".
    Nothing,
}

pub trait Llm: Send {
    /// Pick a tool or answer. `recent` is the last request and its result, for follow-ups.
    fn route(&mut self, request: &str, tools: &[ToolBrief], recent: Option<&str>) -> Pick;
}
