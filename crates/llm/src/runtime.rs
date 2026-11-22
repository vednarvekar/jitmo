//! jitmo-runtime as the model engine (`V:\Code\jitmo-runtime`, its docs/06-runtime-api.md).
//!
//! jitmo either connects to a runtime that is already running (`url`) or starts one as its
//! child: `--port 0 --parent-pid <jitmo>` and reads the `JITMO_RUNTIME_LISTENING port=<n>`
//! line from its stdout. The runtime owns everything jitmo's own launcher did (lazy load, idle
//! unload, the memory cap, the free-RAM check, low priority); jitmo only sends requests.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How to reach the runtime.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// A runtime that is already running, e.g. `http://127.0.0.1:8090`. Wins over `exe`.
    pub url: Option<String>,
    /// `jitmo-runtime.exe`, started on first use.
    pub exe: Option<PathBuf>,
    /// Its TOML config (jitmo's is `config/jitmo-runtime.toml`).
    pub config: Option<PathBuf>,
    /// Passed as `--model` (overrides the config's model path).
    pub model_file: Option<PathBuf>,
    /// Passed as `--llama-server`.
    pub llama_server: Option<PathBuf>,
    /// Passed as `--log-file`.
    pub log_file: Option<PathBuf>,
    /// The model name sent in requests (the config's `model.name`).
    pub model: String,
}

impl RuntimeConfig {
    pub fn connect(url: impl Into<String>, model: impl Into<String>) -> RuntimeConfig {
        RuntimeConfig {
            url: Some(url.into().trim_end_matches('/').to_string()),
            exe: None,
            config: None,
            model_file: None,
            llama_server: None,
            log_file: None,
            model: model.into(),
        }
    }

    pub fn spawn(exe: PathBuf, config: Option<PathBuf>, model: impl Into<String>) -> RuntimeConfig {
        RuntimeConfig {
            url: None,
            exe: Some(exe),
            config,
            model_file: None,
            llama_server: None,
            log_file: None,
            model: model.into(),
        }
    }
}

pub(crate) struct Runtime {
    pub(crate) cfg: RuntimeConfig,
    base: Option<String>,
    child: Option<Child>,
}

impl Runtime {
    pub(crate) fn new(cfg: RuntimeConfig) -> Runtime {
        Runtime {
            cfg,
            base: None,
            child: None,
        }
    }

    fn child_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// The runtime's base URL, starting it first when jitmo owns it.
    pub(crate) fn ensure(&mut self) -> Result<String, String> {
        if let Some(url) = &self.cfg.url {
            return Ok(url.clone());
        }
        if self.base.is_some() && self.child_alive() {
            return Ok(self.base.clone().unwrap_or_default());
        }
        self.stop();
        let exe = self
            .cfg
            .exe
            .clone()
            .ok_or("no runtime url or program set")?;
        let mut cmd = Command::new(&exe);
        if let Some(c) = &self.cfg.config {
            cmd.arg("--config").arg(c);
        }
        if let Some(m) = &self.cfg.model_file {
            cmd.arg("--model").arg(m);
        }
        if let Some(s) = &self.cfg.llama_server {
            cmd.arg("--llama-server").arg(s);
        }
        if let Some(l) = &self.cfg.log_file {
            cmd.arg("--log-file").arg(l);
        }
        cmd.args(["--port", "0", "--parent-pid"])
            .arg(std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start jitmo-runtime: {e}"))?;
        let out = child.stdout.take().ok_or("no runtime stdout")?;
        let (tx, rx) = mpsc::channel();
        // Read the port line, then keep draining stdout so the runtime never blocks on it.
        std::thread::spawn(move || {
            let mut tx = Some(tx);
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(p) = parse_listening(&line) {
                    if let Some(t) = tx.take() {
                        let _ = t.send(p);
                    }
                }
            }
        });
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(port) => {
                let base = format!("http://127.0.0.1:{port}");
                self.base = Some(base.clone());
                self.child = Some(child);
                Ok(base)
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                Err("jitmo-runtime did not report its port".into())
            }
        }
    }

    /// Forget a runtime that stopped answering, so the next request starts a new one.
    pub(crate) fn lost(&mut self) {
        if self.cfg.url.is_none() && !self.child_alive() {
            self.stop();
        }
    }

    pub(crate) fn stop(&mut self) {
        self.base = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `JITMO_RUNTIME_LISTENING port=51234` → 51234.
pub(crate) fn parse_listening(line: &str) -> Option<u16> {
    line.trim()
        .strip_prefix("JITMO_RUNTIME_LISTENING")?
        .split_whitespace()
        .find_map(|w| w.strip_prefix("port="))?
        .parse()
        .ok()
}

/// The runtime's error message, e.g. `503 insufficient_memory: not enough free RAM ...`.
pub(crate) fn error_text(status: u16, body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v["error"]["code"].as_str().unwrap_or("error");
    let msg = v["error"]["message"].as_str().unwrap_or(body);
    format!("jitmo-runtime {status} {code}: {msg}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_port_line() {
        assert_eq!(
            parse_listening("JITMO_RUNTIME_LISTENING port=51234"),
            Some(51234)
        );
        assert_eq!(parse_listening("something else port=1"), None);
    }

    #[test]
    fn error_text_names_the_code() {
        let t = error_text(
            503,
            r#"{"error":{"message":"not enough free RAM","code":"insufficient_memory"}}"#,
        );
        assert_eq!(
            t,
            "jitmo-runtime 503 insufficient_memory: not enough free RAM"
        );
    }
}
