//! Separate model process (no network). Speaks `WorkerRequest`/`WorkerReply` on stdin/stdout.
//!
//! Options: `--idle-secs N` exit after N seconds without a request (default 300,
//! 0 = never); `--engine test` use the crash/hang test engine.

use jitmo_model_worker::{serve, Exit, TestEngine, UnbuiltEngine};
use std::io::{stdin, stdout, BufReader};
use std::time::Duration;

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
        serve(input, stdout(), &mut UnbuiltEngine, idle)
    };
    match result {
        Ok(Exit::Closed | Exit::Idle) => {}
        Err(e) => {
            eprintln!("model worker stopped: {e}");
            std::process::exit(1);
        }
    }
}
