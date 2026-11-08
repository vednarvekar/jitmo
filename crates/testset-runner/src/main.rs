//! Usage: jitmo-testset-runner [--live] [--strict] [--only T01,T03] [--json out.json]
//!
//! Run from anywhere inside the repo. Default mode uses fake connectors, so it
//! is safe and repeatable. `--live` uses real connectors for tests that allow
//! it (today: T01 opens the real Spotify).

use jitmo_testset_runner::report::{self, Row, Status, Verdict};
use jitmo_testset_runner::{doc, peak_memory_bytes, run, spec};
use std::path::PathBuf;
use std::process::ExitCode;

fn find_root() -> Result<PathBuf, String> {
    let mut dir = std::env::current_dir().map_err(|e| e.to_string())?;
    loop {
        if dir.join("testset").is_dir() && dir.join("docs").is_dir() && dir.join("skills").is_dir() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err("run this from inside the jitmo repo".into());
        }
    }
}

fn go() -> Result<bool, String> {
    let mut live = false;
    let mut only: Option<Vec<String>> = None;
    let mut json: Option<PathBuf> = None;
    let mut strict = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--live" => live = true,
            "--strict" => strict = true,
            "--only" => only = Some(args.next().ok_or("--only needs a list")?.split(',').map(|s| s.trim().to_uppercase()).collect()),
            "--json" => json = Some(args.next().ok_or("--json needs a path")?.into()),
            other => return Err(format!("unknown option {other}")),
        }
    }

    let root = find_root()?;
    let read = |p: PathBuf| std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()));
    let cases = doc::parse(&read(root.join("docs/01-v1-spec/4 - Success Metrics and Test Set.md"))?);
    let specs = spec::load(&read(root.join("testset/automated.yaml"))?)?;
    let skills = run::load_skills(&root.join("skills"))?;
    if cases.len() != 100 {
        return Err(format!("doc 04 has {} tests, expected 100", cases.len()));
    }
    for id in specs.keys() {
        if !cases.iter().any(|c| &c.id == id) {
            return Err(format!("automated.yaml has {id}, which is not in doc 04"));
        }
    }

    let mut rows = Vec::new();
    for case in &cases {
        let selected = only.as_ref().is_none_or(|o| o.contains(&case.id));
        let mut row = Row {
            id: case.id.clone(),
            number: case.number,
            section: case.section.clone(),
            request: case.request.clone(),
            status: Status::NotBuilt,
            reasons: vec![],
            message: String::new(),
            elapsed_ms: None,
            peak_mb: None,
            sent_without_confirm: false,
            crashed: false,
        };
        if let (true, Some(spec)) = (selected, specs.get(&case.id)) {
            row.request = spec.request.clone();
            let r = run::run_case(&skills, spec, live);
            row.elapsed_ms = Some(r.elapsed_ms);
            row.peak_mb = peak_memory_bytes().map(|b| b as f64 / 1_048_576.0);
            row.message = r.message;
            row.sent_without_confirm = r.sent_without_confirm;
            row.crashed = r.crashed;
            match (&spec.known_failing, r.passed) {
                (None, true) => row.status = Status::Pass,
                (None, false) => {
                    row.status = Status::Fail;
                    row.reasons = r.reasons;
                }
                (Some(why), false) => {
                    row.status = Status::KnownFail;
                    row.reasons = std::iter::once(format!("known gap: {why}")).chain(r.reasons).collect();
                }
                (Some(_), true) => {
                    row.status = Status::Fail;
                    row.reasons = vec!["this test now passes: remove `known_failing` from automated.yaml".into()];
                }
            }
        }
        rows.push(row);
    }

    let metrics = report::metrics(&rows, peak_memory_bytes());
    print!("{}", report::render(&rows, &metrics));
    if live {
        println!("\n(live mode: real connectors were used where a test allows it)");
    }
    if let Some(path) = json {
        let body = serde_json::json!({ "live": live, "tests": rows, "metrics": metrics });
        std::fs::write(&path, serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }

    // Exit code: a failing test, a send without a confirm or a crash always fails
    // the run. Rate targets (like 90% success) only fail it with --strict, because
    // most tests are not built yet and known gaps are expected during development.
    let failed = rows.iter().any(|r| r.status == Status::Fail);
    let hard_miss = rows.iter().any(|r| r.sent_without_confirm || r.crashed);
    let any_miss = metrics.iter().any(|m| m.verdict == Verdict::Missed);
    Ok(!failed && !hard_miss && !(strict && any_miss))
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
