//! What the worker computes. Step 3 adds the real router and extractor.

use crate::rules::{self, Gazetteer};
use jitmo_ipc::{ContextNote, Span, Understanding};

/// Turns a request into an [`Understanding`].
pub trait Engine {
    fn understand(&mut self, text: &str, context: &ContextNote) -> Result<Understanding, String>;

    /// Slot spans for a request whose task is already chosen. The default uses the
    /// rules alone (no model); the real engine uses the NER model, loaded on first use.
    fn extract(
        &mut self,
        text: &str,
        _task: Option<&str>,
        context: &ContextNote,
    ) -> Result<Vec<Span>, String> {
        let gaz = Gazetteer::from_context(context);
        Ok(rules::extract_rules(text, &gaz)
            .iter()
            .map(rules::to_slot_span)
            .collect())
    }
}

fn nothing() -> Understanding {
    Understanding {
        tasks: Vec::new(),
        out_of_scope: 1.0,
        chained: false,
        spans: Vec::new(),
    }
}

/// Placeholder until the real models are wired in: understands nothing.
pub struct UnbuiltEngine;

impl Engine for UnbuiltEngine {
    fn understand(&mut self, _: &str, _: &ContextNote) -> Result<Understanding, String> {
        Ok(nothing())
    }
}

/// Engine for tests of the process plumbing. Special request texts:
/// `__crash__` exits the process, `__hang__` never answers, `__error__` returns an error.
pub struct TestEngine;

impl Engine for TestEngine {
    fn understand(&mut self, text: &str, _: &ContextNote) -> Result<Understanding, String> {
        match text {
            "__crash__" => std::process::exit(101),
            "__hang__" => loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            },
            "__error__" => Err("test error".into()),
            _ => Ok(nothing()),
        }
    }
}
