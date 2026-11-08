//! UI side of the Core pipe: sends [`UiRequest`]s, forwards [`CoreEvent`]s.
//!
//! A supervisor thread connects to the per-user pipe, reads events until the
//! connection drops, then reconnects with a short backoff. Anything still in
//! flight when the connection drops gets a `Failed` event, so the bar never
//! waits for an answer that cannot come.

use jitmo_adapter_windows::pipe::{connect, default_pipe_name, PipeWriter};
use jitmo_ipc::{read_message, write_message, CoreEvent, FrameError, RequestId, UiRequest};
use std::collections::HashSet;
use std::io::BufReader;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const LOST: &str = "Lost the connection to the assistant. Try again.";
const BACKOFF_START: Duration = Duration::from_millis(200);
const BACKOFF_MAX: Duration = Duration::from_secs(3);

struct Shared {
    writer: Mutex<Option<PipeWriter>>,
    in_flight: Mutex<HashSet<RequestId>>,
    emit: Box<dyn Fn(CoreEvent) + Send + Sync>,
}

impl Shared {
    /// Emit `Failed` for every request that has not finished.
    fn fail_in_flight(&self, message: &str) {
        let ids: Vec<RequestId> = self.in_flight.lock().unwrap().drain().collect();
        for id in ids {
            (self.emit)(CoreEvent::Failed {
                id,
                message: message.into(),
            });
        }
    }
}

pub struct CoreClient {
    shared: Arc<Shared>,
}

impl CoreClient {
    /// Start connecting in the background. `emit` is called for every event.
    pub fn start(emit: impl Fn(CoreEvent) + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            writer: Mutex::new(None),
            in_flight: Mutex::new(HashSet::new()),
            emit: Box::new(emit),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("jitmo-core-client".into())
            .spawn(move || supervise(s))
            .expect("failed to start the core client thread");
        CoreClient { shared }
    }

    /// Send a UI message to Core. Never blocks on Core's work. If Core cannot
    /// be reached, a request that expects a final event gets `Failed` at once.
    pub fn send(&self, req: UiRequest) {
        let expects_final = match &req {
            UiRequest::Submit { id, .. } | UiRequest::Undo { id } => Some(*id),
            _ => None,
        };
        if let Some(id) = expects_final {
            self.shared.in_flight.lock().unwrap().insert(id);
        }
        let ok = {
            let mut guard = self.shared.writer.lock().unwrap();
            match guard.as_mut() {
                Some(w) => {
                    let ok = write_message(w, &req).is_ok();
                    if !ok {
                        *guard = None; // the reader thread notices the break and reconnects
                    }
                    ok
                }
                None => false,
            }
        };
        if !ok {
            self.shared.fail_in_flight(LOST);
        }
    }
}

fn supervise(shared: Arc<Shared>) {
    let name = default_pipe_name();
    let mut backoff = BACKOFF_START;
    loop {
        match connect(&name) {
            Ok(stream) => {
                backoff = BACKOFF_START;
                let (reader, writer) = stream.split();
                *shared.writer.lock().unwrap() = Some(writer);
                let mut reader = BufReader::new(reader);
                loop {
                    match read_message::<_, CoreEvent>(&mut reader) {
                        Ok(Some(event)) => {
                            if let CoreEvent::Done { id, .. } | CoreEvent::Failed { id, .. } =
                                &event
                            {
                                shared.in_flight.lock().unwrap().remove(id);
                            }
                            (shared.emit)(event);
                        }
                        Ok(None) | Err(FrameError::Io(_)) => break,
                        Err(FrameError::Json(e)) => eprintln!("bad event from Core: {e}"),
                    }
                }
                *shared.writer.lock().unwrap() = None;
                shared.fail_in_flight(LOST);
            }
            Err(_) => {
                // Core is not there (yet). Anything sent meanwhile was already failed in `send`.
            }
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}
