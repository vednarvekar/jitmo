//! The Tier 0 extractor: rules plus the GLiNER model, with the model loaded on first
//! use. The model (about 180 MB on disk, roughly 200 MB of memory once loaded) is
//! only needed when the rules and the skill example patterns cannot fill a slot, so
//! most requests never load it. It goes away when the worker exits after idle.

use crate::gliner::{extractor_dir, Gliner};
use crate::rules::{self, Ent, Gazetteer};
use ort::environment::Environment;
use std::path::PathBuf;

/// GLiNER score cut-off. int8 lowers scores, so 0.3 rather than 0.5 (SP2).
pub const THRESHOLD: f32 = 0.3;

pub struct LazyExtractor {
    dir: Option<PathBuf>,
    threads: usize,
    model: Option<Gliner>,
    /// Loading failed once; do not try again on every request.
    failed: bool,
}

impl LazyExtractor {
    /// `dir` overrides the model folder (else `JITMO_EXTRACTOR_DIR`, else `models/extractor`).
    pub fn new(dir: Option<PathBuf>) -> Self {
        let threads = std::env::var("JITMO_EXTRACTOR_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        Self {
            dir,
            threads,
            model: None,
            failed: false,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    fn model(&mut self, env: &Environment) -> Option<&mut Gliner> {
        if self.model.is_none() && !self.failed {
            let loaded = extractor_dir(self.dir.as_deref())
                .ok_or_else(|| "no extractor model folder found".to_string())
                .and_then(|d| Gliner::load(env, &d, self.threads));
            match loaded {
                Ok(m) => self.model = Some(m),
                Err(e) => {
                    tracing::warn!("extractor model not available, using rules only: {e}");
                    self.failed = true;
                }
            }
        }
        self.model.as_mut()
    }

    /// Spans for `text`: rules plus model. If the model is missing or fails, the
    /// rules alone answer (never an error).
    pub fn extract(&mut self, env: &Environment, text: &str, gaz: &Gazetteer) -> Vec<Ent> {
        let raw = match self.model(env) {
            Some(m) => m.predict(text, THRESHOLD).unwrap_or_else(|e| {
                tracing::warn!("extractor model failed, using rules only: {e}");
                Vec::new()
            }),
            None => Vec::new(),
        };
        rules::extract(text, &raw, THRESHOLD, gaz)
    }
}
