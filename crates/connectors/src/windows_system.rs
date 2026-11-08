//! `windows_system` connector (Windows only). Today: A1, open an app.
//! The other system controls (S1-S5) come in Phase 2.

use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use jitmo_adapter_windows::apps::{self, Lookup};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long `app_is_running` waits for a freshly launched app to show up.
const START_WAIT: Duration = Duration::from_secs(8);
const POLL_EVERY: Duration = Duration::from_millis(500);

pub struct WindowsSystem;

fn arg<'a>(args: &'a Args, name: &str) -> Result<&'a str, ConnError> {
    args.get(name)
        .map(String::as_str)
        .ok_or_else(|| ConnError::MissingArg(name.into()))
}

impl Connector for WindowsSystem {
    fn id(&self) -> &str {
        "windows_system"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec {
            name: "open_app".into(),
            required_args: vec!["app".into()],
        }]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        match action {
            "open_app" => {
                let wanted = arg(args, "app")?;
                match apps::lookup_app(wanted).map_err(ConnError::Failed)? {
                    Lookup::Found(app) => {
                        apps::launch(&app).map_err(ConnError::Failed)?;
                        Ok(Output(BTreeMap::from([("app".to_string(), app.name)])))
                    }
                    Lookup::NotFound => Err(ConnError::NotInstalled(wanted.into())),
                    Lookup::Ambiguous(names) => Err(ConnError::Failed(format!(
                        "I found several apps called {wanted}: {}.",
                        names.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                    ))),
                }
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        match action {
            "app_is_running" => {
                let wanted = arg(args, "app")?;
                // A launched app takes a moment to appear, so wait for it.
                let start = Instant::now();
                loop {
                    if apps::is_app_running(wanted, false) {
                        return Ok(true);
                    }
                    if start.elapsed() >= START_WAIT {
                        // Last try: some apps run under a different process name,
                        // so also compare window titles.
                        return Ok(apps::is_app_running(wanted, true));
                    }
                    std::thread::sleep(POLL_EVERY);
                }
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    /// Opening an app is not undone.
    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(app: &str) -> Args {
        Args::from([("app".to_string(), app.to_string())])
    }

    #[test]
    fn missing_argument_and_unknown_action_are_errors() {
        let c = WindowsSystem;
        assert_eq!(
            c.execute("open_app", &Args::new()),
            Err(ConnError::MissingArg("app".into()))
        );
        assert_eq!(
            c.execute("fly", &args("x")),
            Err(ConnError::UnknownAction("fly".into()))
        );
        assert_eq!(
            c.check("fly", &args("x")),
            Err(ConnError::UnknownAction("fly".into()))
        );
        assert!(c.undo("open_app", &Output::default()).is_none());
    }

    /// Opens a real app (default Calculator; set JITMO_LIVE_APP to try another).
    /// Run by hand: cargo test -p jitmo-connectors -- --ignored --nocapture
    #[test]
    #[ignore = "launches a real app"]
    fn live_open_app_and_verify() {
        let app = std::env::var("JITMO_LIVE_APP").unwrap_or_else(|_| "calculator".into());
        let c = WindowsSystem;
        // JITMO_WARM_SECS=n: warm the index first and wait n seconds (what the
        // app does at startup); unset: a cold lookup that has to build it.
        if let Some(secs) = std::env::var("JITMO_WARM_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        {
            apps::warm_up();
            std::thread::sleep(Duration::from_secs(secs));
        }
        let t = Instant::now();
        let looked = apps::lookup_app(&app).unwrap();
        println!("TIMING first lookup_app: {:?} -> {looked:?}", t.elapsed());
        let t = Instant::now();
        let _ = apps::lookup_app(&app).unwrap();
        println!("TIMING second lookup_app: {:?}", t.elapsed());
        let t = Instant::now();
        let out = c.execute("open_app", &args(&app)).unwrap();
        println!(
            "TIMING execute(open_app), lookup + launch: {:?}",
            t.elapsed()
        );
        println!("opened: {out:?}");
        let t = Instant::now();
        let running = c.check("app_is_running", &args(&app));
        println!("TIMING check(app_is_running): {:?}", t.elapsed());
        assert_eq!(running, Ok(true));
    }

    #[test]
    fn an_app_that_does_not_exist_is_reported_as_not_installed() {
        let r = WindowsSystem.execute("open_app", &args("definitely-not-an-app-xyz"));
        assert_eq!(
            r,
            Err(ConnError::NotInstalled("definitely-not-an-app-xyz".into()))
        );
    }
}
