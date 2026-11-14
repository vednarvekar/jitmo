//! Separate model process (no network). Speaks `WorkerRequest`/`WorkerReply` on stdin/stdout.
//!
//! Options:
//! - `--idle-secs N`  exit after N seconds without a request (default 300, 0 = never)
//! - `--models DIR`   folder with the router files (else `JITMO_MODELS`, else
//!   `models/tools` or `models/router` next to the program, else the same under `./`; `tools`
//!   wins when both exist)
//! - `--extractor DIR` folder with the extractor (NER) model (else `JITMO_NER_DIR`, else
//!   `models/ner` next to the program, else `./models/ner`); loaded on first use, and without it
//!   the extractor uses the rules only
//! - `--engine test`  the crash/hang test engine (for process tests)
//!
//! If the models cannot be loaded the process exits with code 2 and Core falls
//! back to its rule-based router.

use jitmo_model_worker::{serve, Engine, Exit, TestEngine};
use std::io::{stdin, stdout, BufReader};
use std::path::PathBuf;
use std::time::Duration;

fn models_dir(arg: Option<String>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(arg.map(PathBuf::from));
    candidates.extend(std::env::var_os("JITMO_MODELS").map(PathBuf::from));
    // `tools` (the fine-tuned tool encoder, classes are skill ids) wins over `router` (the
    // older frozen encoder with a head over the spec's task ids) when both are there.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("models").join("tools"));
            candidates.push(dir.join("models").join("router"));
        }
    }
    candidates.push(PathBuf::from("models/tools"));
    candidates.push(PathBuf::from("models/router"));
    candidates
        .into_iter()
        .find(|d| d.join("model_int8.onnx").exists())
}

fn real_engine(
    models: Option<String>,
    extractor: Option<String>,
) -> Result<Box<dyn Engine + Send>, String> {
    #[cfg(feature = "models")]
    {
        let dir = models_dir(models).ok_or("no model folder found")?;
        Ok(Box::new(
            jitmo_model_worker::router::RouterEngine::load_with(
                &dir,
                extractor.map(PathBuf::from),
            )?,
        ))
    }
    #[cfg(not(feature = "models"))]
    {
        let _ = (models, extractor, models_dir(None));
        Err("built without models".to_string())
    }
}

struct Boxed(Box<dyn Engine + Send>);

impl Engine for Boxed {
    fn understand(
        &mut self,
        text: &str,
        context: &jitmo_ipc::ContextNote,
    ) -> Result<jitmo_ipc::Understanding, String> {
        self.0.understand(text, context)
    }

    fn extract(
        &mut self,
        text: &str,
        task: Option<&str>,
        context: &jitmo_ipc::ContextNote,
    ) -> Result<Vec<jitmo_ipc::Span>, String> {
        self.0.extract(text, task, context)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let idle_secs: u64 = value("--idle-secs")
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let idle = (idle_secs > 0).then(|| Duration::from_secs(idle_secs));
    let input = BufReader::new(stdin());
    let result = if value("--engine").as_deref() == Some("test") {
        serve(input, stdout(), &mut TestEngine, idle)
    } else {
        match real_engine(value("--models"), value("--extractor")) {
            Ok(engine) => serve(input, stdout(), &mut Boxed(engine), idle),
            Err(e) => {
                eprintln!("model worker cannot start: {e}");
                std::process::exit(2);
            }
        }
    };
    match result {
        Ok(Exit::Closed | Exit::Idle) => {}
        Err(e) => {
            eprintln!("model worker stopped: {e}");
            std::process::exit(1);
        }
    }
}
