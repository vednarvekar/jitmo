//! Runs the real worker binary and checks start, crash, hang and idle exit.

use jitmo_ipc::ContextNote;
use jitmo_model_worker::{WorkerClient, WorkerError, WorkerLauncher};
use std::time::Duration;

fn client(extra: &[&str], timeout_ms: u64) -> WorkerClient {
    let mut args = vec!["--engine".to_string(), "test".to_string()];
    args.extend(extra.iter().map(|s| s.to_string()));
    WorkerClient::new(
        WorkerLauncher {
            program: env!("CARGO_BIN_EXE_jitmo-model-worker").into(),
            args,
        },
        Duration::from_millis(timeout_ms),
    )
}

fn ctx() -> ContextNote {
    ContextNote::default()
}

#[test]
fn starts_on_first_call_and_answers() {
    let mut c = client(&[], 5000);
    assert!(!c.is_running());
    c.ping().unwrap();
    assert!(c.is_running());
    let u = c.understand("open spotify", &ctx()).unwrap();
    assert_eq!(u.out_of_scope, 1.0);
    assert_eq!(c.restarts(), 0);
}

#[test]
fn restarts_after_a_crash_and_keeps_working() {
    let mut c = client(&[], 5000);
    c.ping().unwrap();
    // The crash request fails on both tries (the test engine crashes every time).
    assert!(matches!(
        c.understand("__crash__", &ctx()),
        Err(WorkerError::Unavailable)
    ));
    assert_eq!(c.restarts(), 2);
    // A later normal request starts a fresh worker.
    assert!(c.understand("hello", &ctx()).is_ok());
}

#[test]
fn a_hung_worker_times_out_and_is_replaced() {
    let mut c = client(&[], 300);
    assert!(matches!(
        c.understand("__hang__", &ctx()),
        Err(WorkerError::Unavailable)
    ));
    assert!(c.understand("hello", &ctx()).is_ok());
}

#[test]
fn engine_errors_do_not_restart_the_worker() {
    let mut c = client(&[], 5000);
    assert!(matches!(
        c.understand("__error__", &ctx()),
        Err(WorkerError::Reported(_))
    ));
    assert_eq!(c.restarts(), 0);
    assert!(c.ping().is_ok());
}

#[test]
fn exits_after_idle_and_starts_again_on_demand() {
    let mut c = client(&["--idle-secs", "1"], 5000);
    c.ping().unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!c.is_running(), "worker should have exited after idle");
    c.ping().unwrap();
    assert_eq!(c.restarts(), 0, "an idle exit is not a crash");
}

#[test]
fn missing_program_is_a_clear_error() {
    let mut c = WorkerClient::new(
        WorkerLauncher {
            program: "definitely-not-here.exe".into(),
            args: vec![],
        },
        Duration::from_millis(500),
    );
    assert!(matches!(c.ping(), Err(WorkerError::Start(_))));
}
