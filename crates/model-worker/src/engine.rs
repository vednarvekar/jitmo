//! What the worker computes. Step 3 adds the real router and extractor.

use jitmo_ipc::{ContextNote, Understanding};

/// Turns a request into an [`Understanding`].
pub trait Engine {
    fn understand(&mut self, text: &str, context: &ContextNote) -> Result<Understanding, String>;
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
