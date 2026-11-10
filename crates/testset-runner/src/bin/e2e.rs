//! `jitmo-e2e`: the engine side of the end-to-end UI tests (see `app/e2e/`).
//!
//! It is the real engine (the real `Flow`, the real router when a worker is given, the real
//! connectors) running against a SIMULATED PC: no app is launched, no setting is changed, no
//! file moves, nothing is sent, nothing leaves the machine. Reminders and settings live in a
//! throw-away database.
//!
//! The UI test harness talks to it over two local TCP ports (127.0.0.1, chosen at random):
//!
//! - the **engine port** speaks exactly what the desktop app speaks over its named pipe:
//!   one JSON `UiRequest` per line in, one JSON `CoreEvent` per line out;
//! - the **control port** answers one line per connection: `calls` returns the connector calls
//!   and scripts the engine made so far (one per line), `clear` forgets them.
//!
//! On start it prints `LISTENING <engine port> <control port>` and serves one engine
//! connection after another until it is killed.
//!
//! Usage: `jitmo-e2e [--worker <exe>] [--models <dir>] [--extractor <dir>] [--skills <dir>]
//! [--flags a,b]`. Without a worker it routes by the skills' example phrases only.

use jitmo_core::service::serve_connection;
use jitmo_model_worker::WorkerLauncher;
use jitmo_testset_runner::audit::{build_world, load_skills, CallRec, World};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;

/// Skills the desktop app does not ship (`stub.send` is a test skill; messaging is Phase 3).
const NOT_IN_THE_APP: &[&str] = &["stub.send", "whatsapp.send_file"];

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn describe(c: &CallRec) -> String {
    let args: Vec<String> = c.args.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
    let kind = if c.script { "script" } else { "call" };
    let result = match &c.result {
        Some(Ok(m)) => format!(" => {m:?}"),
        Some(Err(e)) => format!(" => ERR {e:?}"),
        None => String::new(),
    };
    format!(
        "{kind} {}.{}({}){result}",
        c.connector,
        c.action,
        args.join(", ")
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let skills_dir = PathBuf::from(flag(&args, "--skills").unwrap_or_else(|| "skills".into()));
    let skills: Vec<_> = load_skills(&skills_dir)
        .unwrap_or_else(|e| panic!("skills: {e}"))
        .into_iter()
        .filter(|s| !NOT_IN_THE_APP.contains(&s.id.as_str()))
        .collect();
    let worker = flag(&args, "--worker")
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .map(|program| {
            let mut a = Vec::new();
            for (name, key) in [("--models", "--models"), ("--extractor", "--extractor")] {
                if let Some(dir) = flag(&args, name) {
                    a.push(key.to_string());
                    a.push(dir);
                }
            }
            WorkerLauncher { program, args: a }
        });
    let flags: HashSet<String> = flag(&args, "--flags")
        .map(|f| f.split(',').map(str::to_string).collect())
        .unwrap_or_default();

    let dir = std::env::temp_dir().join(format!("jitmo-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");

    let World { mut flow, log, .. } = build_world(&skills, &worker, &flags, &dir);

    let engine = TcpListener::bind("127.0.0.1:0").expect("engine port");
    let control = TcpListener::bind("127.0.0.1:0").expect("control port");
    println!(
        "LISTENING {} {}",
        engine.local_addr().map(|a| a.port()).unwrap_or(0),
        control.local_addr().map(|a| a.port()).unwrap_or(0)
    );
    let _ = std::io::stdout().flush();

    let control_log = log.clone();
    std::thread::spawn(move || {
        for stream in control.incoming().flatten() {
            let mut stream = stream;
            let mut line = String::new();
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            if BufReader::new(clone).read_line(&mut line).is_err() {
                continue;
            }
            let mut guard = control_log.lock().unwrap_or_else(|e| e.into_inner());
            match line.trim() {
                "calls" => {
                    for c in guard.iter() {
                        let _ = writeln!(stream, "{}", describe(c));
                    }
                }
                "clear" => guard.clear(),
                _ => {}
            }
        }
    });

    for stream in engine.incoming().flatten() {
        let Ok(reader) = stream.try_clone() else {
            continue;
        };
        serve_connection(&mut flow, BufReader::new(reader), stream);
    }
}
