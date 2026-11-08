//! Core's side: starts the worker process on demand, calls it with a timeout and
//! restarts it when it crashes or stops answering.

use jitmo_ipc::{
    read_message, write_message, ContextNote, Span, Understanding, WorkerReply, WorkerRequest,
};
use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// Could not start the process (missing file, blocked by policy).
    #[error("could not start the model worker: {0}")]
    Start(String),
    /// It crashed or hung twice in a row; the caller should fall back or tell the user.
    #[error("the model worker is not responding")]
    Unavailable,
    /// The worker answered with an error of its own (no restart needed).
    #[error("the model worker reported: {0}")]
    Reported(String),
}

/// How to start the worker.
#[derive(Debug, Clone)]
pub struct WorkerLauncher {
    pub program: PathBuf,
    pub args: Vec<String>,
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<WorkerReply>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct WorkerClient {
    launcher: WorkerLauncher,
    timeout: Duration,
    running: Option<Running>,
    next_id: u64,
    restarts: u32,
}

impl WorkerClient {
    /// `timeout` is how long to wait for one reply before treating the worker as hung.
    pub fn new(launcher: WorkerLauncher, timeout: Duration) -> Self {
        Self {
            launcher,
            timeout,
            running: None,
            next_id: 1,
            restarts: 0,
        }
    }

    /// Times a request failed (crash or hang) and the worker was restarted.
    /// Starting again after an idle exit does not count.
    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    /// True while a worker process is alive.
    pub fn is_running(&mut self) -> bool {
        self.alive()
    }

    /// Stop the worker now (frees its memory).
    pub fn stop(&mut self) {
        self.running = None;
    }

    pub fn ping(&mut self) -> Result<(), WorkerError> {
        self.call(|id| WorkerRequest::Ping { id }).map(|_| ())
    }

    pub fn understand(
        &mut self,
        text: &str,
        context: &ContextNote,
    ) -> Result<Understanding, WorkerError> {
        let reply = self.call(|id| WorkerRequest::Understand {
            id,
            text: text.to_string(),
            context: context.clone(),
        })?;
        match reply {
            WorkerReply::Understood { result, .. } => Ok(result),
            other => Err(WorkerError::Reported(format!("unexpected reply {other:?}"))),
        }
    }

    /// Slot spans for a request whose task is already chosen. The first call loads the
    /// extractor model inside the worker, so it can take a moment.
    pub fn extract(
        &mut self,
        text: &str,
        task: Option<&str>,
        context: &ContextNote,
    ) -> Result<Vec<Span>, WorkerError> {
        let reply = self.call(|id| WorkerRequest::Extract {
            id,
            text: text.to_string(),
            task: task.map(str::to_string),
            context: context.clone(),
        })?;
        match reply {
            WorkerReply::Extracted { spans, .. } => Ok(spans),
            other => Err(WorkerError::Reported(format!("unexpected reply {other:?}"))),
        }
    }

    fn alive(&mut self) -> bool {
        match &mut self.running {
            Some(r) => matches!(r.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// One try, and one more after a restart if the transport failed.
    fn call(&mut self, make: impl Fn(u64) -> WorkerRequest) -> Result<WorkerReply, WorkerError> {
        for attempt in 0..2 {
            if !self.alive() {
                // Not running: first start, or it exited after idle.
                self.running = None;
                self.running = Some(self.spawn()?);
            }
            let id = self.next_id;
            self.next_id += 1;
            match self.exchange(id, &make(id)) {
                Some(WorkerReply::Error { message, .. }) => {
                    return Err(WorkerError::Reported(message))
                }
                Some(reply) => return Ok(reply),
                None => {
                    // Crashed or hung: kill it and start a fresh one.
                    self.running = None;
                    self.restarts += 1;
                    tracing::warn!("model worker failed (attempt {})", attempt + 1);
                }
            }
        }
        Err(WorkerError::Unavailable)
    }

    /// Send one request and wait for its reply. `None` means the worker is gone or hung.
    fn exchange(&mut self, id: u64, req: &WorkerRequest) -> Option<WorkerReply> {
        let timeout = self.timeout;
        let running = self.running.as_mut()?;
        write_message(&mut running.stdin, req).ok()?;
        loop {
            match running.replies.recv_timeout(timeout) {
                Ok(WorkerReply::Understood { id: got, .. }) if got != id => continue,
                Ok(WorkerReply::Extracted { id: got, .. }) if got != id => continue,
                Ok(WorkerReply::Pong { id: got }) if got != id => continue,
                Ok(r) => return Some(r),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    fn spawn(&self) -> Result<Running, WorkerError> {
        let mut cmd = Command::new(&self.launcher.program);
        cmd.args(&self.launcher.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| WorkerError::Start(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerError::Start("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::Start("no stdout".into()))?;
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            let mut out = BufReader::new(stdout);
            while let Ok(Some(reply)) = read_message::<_, WorkerReply>(&mut out) {
                if tx.send(reply).is_err() {
                    break;
                }
            }
        });
        Ok(Running {
            child,
            stdin,
            replies,
        })
    }
}
