//! Smoke test for the skills audit harness: the rule-based router only (no models, no network),
//! a handful of corpus lines. It proves the harness builds a Flow, runs it and judges it; the full
//! audit is `cargo run --release -p jitmo-testset-runner --bin jitmo-audit` (see the binary's header).

use jitmo_testset_runner::audit;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_corpus_parses_and_has_at_least_300_cases() {
    let text = std::fs::read_to_string(root().join("testset/audit_corpus.txt")).unwrap();
    let cases = audit::load_corpus(&text);
    assert!(cases.len() >= 300, "{} cases", cases.len());
    for c in &cases {
        assert!(
            !c.skill.is_empty() && !c.kind.is_empty() && !c.expect.is_empty(),
            "line {}",
            c.line
        );
    }
}

#[test]
fn a_few_cases_run_end_to_end_without_models() {
    let skills = audit::load_skills(&root().join("skills")).unwrap();
    let text = "apps.open|clean|open spotify|run:apps.open;call=windows_system.open_app;arg=app~spotify|\n\
                reminders.timer|clean|set a 10-minute timer|run:reminders.timer;msg~10 minutes;pending=1|\n\
                system.volume|clean|turn the volume down|run:system.volume;msg~30%|\n";
    let cases = audit::load_corpus(text);
    let cfg = audit::Config {
        root: root(),
        worker: None,
        threads: 1,
        llm: None,
        llm_picks: true,
    };
    let results = audit::run_all(&cfg, &skills, &cases);
    assert_eq!(results.len(), 3);
    for o in &results {
        assert!(o.passed(), "{}", o.summary());
    }
}
