//! The skills audit runner.
//!
//! ```text
//! cargo run --release -p jitmo-testset-runner --bin jitmo-audit -- \
//!     [--worker target/release/jitmo-model-worker.exe] [--models models/router] [--ner models/ner] \
//!     [--rules-only] [--threads 4] [--only music.play,reminders.timer] [--kind typo] \
//!     [--corpus testset/audit_corpus.txt] [--verbose] [--md out.md] [--strict] \
//!     [--llm-model model.gguf --llm-server llama-server.exe [--llm-threads 3] [--llm-pick]]
//!     [--llm-runtime http://127.0.0.1:8090 | --llm-runtime jitmo-runtime.exe [--llm-runtime-config t.toml]]
//! ```
//!
//! With `--llm-runtime` the same model calls go to jitmo-runtime instead of a llama-server this
//! tool starts: a URL connects to a running runtime, a program path starts one (with
//! `config/jitmo-runtime.toml` unless `--llm-runtime-config` is given; `--llm-model` and
//! `--llm-server` are passed on as overrides).
//!
//! With `--llm-model` the engine gets the local language model the app would plug in (it fills
//! typed values for skills that take them; with `--llm-pick` also a tool pick when the router is
//! unsure), as the app does with the same settings.
//!
//! Needs no network. Skips cleanly (exit 0) when the model worker or the model folders are
//! missing, unless `--rules-only` is given (then only the rule-based router is exercised).

use jitmo_model_worker::WorkerLauncher;
use jitmo_testset_runner::audit;
use std::path::PathBuf;
use std::process::ExitCode;

fn find_root() -> Result<PathBuf, String> {
    let mut dir = std::env::current_dir().map_err(|e| e.to_string())?;
    loop {
        if dir.join("testset").is_dir() && dir.join("skills").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err("run this from inside the jitmo repo (or pass --root)".into());
        }
    }
}

fn go() -> Result<bool, String> {
    let mut root: Option<PathBuf> = None;
    let mut worker: Option<PathBuf> = None;
    let mut models: Option<PathBuf> = None;
    let mut ner: Option<PathBuf> = None;
    let mut corpus: Option<PathBuf> = None;
    let mut rules_only = false;
    let mut threads = 4usize;
    let mut only: Option<Vec<String>> = None;
    let mut kind: Option<String> = None;
    let mut verbose = false;
    let mut strict = false;
    let mut all_skills = false;
    let mut md: Option<PathBuf> = None;
    let mut llm_model: Option<PathBuf> = None;
    let mut llm_server: Option<PathBuf> = None;
    let mut llm_threads: Option<u32> = None;
    let mut llm_picks = false;
    let mut llm_runtime: Option<String> = None;
    let mut llm_runtime_config: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--root" => root = Some(val("--root")?.into()),
            "--worker" => worker = Some(val("--worker")?.into()),
            "--models" => models = Some(val("--models")?.into()),
            "--ner" => ner = Some(val("--ner")?.into()),
            "--corpus" => corpus = Some(val("--corpus")?.into()),
            "--threads" => threads = val("--threads")?.parse().map_err(|_| "bad --threads")?,
            "--only" => {
                only = Some(
                    val("--only")?
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect(),
                )
            }
            "--kind" => kind = Some(val("--kind")?),
            "--md" => md = Some(val("--md")?.into()),
            "--llm-model" => llm_model = Some(val("--llm-model")?.into()),
            "--llm-server" => llm_server = Some(val("--llm-server")?.into()),
            "--llm-pick" => llm_picks = true,
            "--llm-runtime" => llm_runtime = Some(val("--llm-runtime")?),
            "--llm-runtime-config" => {
                llm_runtime_config = Some(val("--llm-runtime-config")?.into())
            }
            "--llm-threads" => {
                llm_threads = Some(
                    val("--llm-threads")?
                        .parse()
                        .map_err(|_| "bad --llm-threads")?,
                )
            }
            "--rules-only" => rules_only = true,
            "--verbose" => verbose = true,
            "--strict" => strict = true,
            "--all-skills" => all_skills = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let root = match root {
        Some(r) => r,
        None => find_root()?,
    };
    let mut skills = audit::load_skills(&root.join("skills"))?;
    if !all_skills {
        // The app does not ship this one (app/src-tauri/src/registry.rs), so neither does the audit.
        skills.retain(|s| s.id != "whatsapp.send_file");
    }
    let text =
        std::fs::read_to_string(corpus.unwrap_or_else(|| root.join("testset/audit_corpus.txt")))
            .map_err(|e| format!("corpus: {e}"))?;
    let mut cases = audit::load_corpus(&text);
    if let Some(o) = &only {
        cases.retain(|c| o.contains(&c.skill));
    }
    if let Some(k) = &kind {
        cases.retain(|c| &c.kind == k);
    }

    let launcher = if rules_only {
        None
    } else {
        let worker = worker.unwrap_or_else(|| root.join("target/release/jitmo-model-worker.exe"));
        let models = models.unwrap_or_else(|| root.join("models/router"));
        let ner = ner.unwrap_or_else(|| root.join("models/ner"));
        if !worker.exists() || !models.join("model_int8.onnx").exists() {
            println!(
                "SKIPPED: the model worker ({}) or the router model ({}) is missing. \
                 Use --rules-only to audit the rule-based router.",
                worker.display(),
                models.display()
            );
            return Ok(true);
        }
        let mut a = vec![
            "--models".to_string(),
            models.to_string_lossy().into_owned(),
            "--idle-secs".into(),
            "0".into(),
        ];
        if ner.join("model_int8.onnx").exists() {
            a.push("--extractor".into());
            a.push(ner.to_string_lossy().into_owned());
        }
        Some(WorkerLauncher {
            program: worker,
            args: a,
        })
    };

    let llm = if let Some(rt) = llm_runtime {
        let mut c = if rt.starts_with("http://") || rt.starts_with("https://") {
            jitmo_llm::RuntimeConfig::connect(rt, "jitmo-router")
        } else {
            let config = llm_runtime_config
                .or_else(|| Some(root.join("config/jitmo-runtime.toml")))
                .filter(|p| p.exists());
            jitmo_llm::RuntimeConfig::spawn(rt.into(), config, "jitmo-router")
        };
        c.model_file = llm_model;
        c.llama_server = llm_server;
        Some(jitmo_llm::LocalLlm::runtime(c, true, true))
    } else {
        match (llm_model, llm_server) {
            (Some(model), Some(server)) => {
                let mut c = jitmo_llm::LlmConfig::new(server, model);
                if let Some(t) = llm_threads {
                    c.threads = t;
                }
                Some(jitmo_llm::LocalLlm::new(c))
            }
            (None, None) => None,
            _ => return Err("--llm-model and --llm-server go together".into()),
        }
    };
    let cfg = audit::Config {
        root: root.clone(),
        worker: launcher,
        threads,
        llm,
        llm_picks,
    };
    println!(
        "jitmo-audit: {} cases, router = {}, {} threads",
        cases.len(),
        match (cfg.worker.is_some(), cfg.llm.is_some()) {
            (true, true) if cfg.llm.as_ref().is_some_and(|l| l.on_runtime()) => {
                "real model worker + language model on jitmo-runtime"
            }
            (true, true) => "real model worker + language model",
            (true, false) => "real model worker",
            (false, _) => "rule-based only",
        },
        threads
    );
    let started = std::time::Instant::now();
    let results = audit::run_all(&cfg, &skills, &cases);
    println!("finished in {:.1}s\n", started.elapsed().as_secs_f64());

    println!("Static lints of skills/*.yaml");
    for l in audit::lint_skills(&skills) {
        println!("  {l}");
    }
    println!();
    println!("{}", audit::render_table(&results));
    println!("{}", audit::render_rule_hits(&results));
    if verbose {
        for o in &results {
            println!(
                "{} [{}] {:?}\n    {}",
                if o.passed() { "ok  " } else { "FAIL" },
                o.case.skill,
                o.case.input,
                o.summary()
            );
        }
    } else {
        print!("{}", audit::render_failures(&results));
    }
    if let Some(p) = md {
        std::fs::write(&p, audit::render_markdown(&results)).map_err(|e| e.to_string())?;
        println!("\nfull table written to {}", p.display());
    }
    let failed = results.iter().filter(|o| !o.passed()).count();
    Ok(!(strict && failed > 0))
}

fn main() -> ExitCode {
    match go() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
