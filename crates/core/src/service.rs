//! Core service: owns the [`Flow`] on its own thread and speaks the UI
//! protocol ([`UiRequest`] in, [`CoreEvent`] out).
//!
//! This is the seam between the UI and the engine. Two transports share one
//! request loop: an in-process channel ([`CoreService`]) and any byte stream
//! such as the per-user named pipe ([`serve_connection`]).
//!
//! A request that needs the user (confirm card, choice) pauses the flow thread
//! until the matching answer arrives. Cancel while waiting counts as "no" (and as
//! dismissed for a typed question). There is no timeout: like the confirm card, a
//! question waits until it is answered, cancelled or the UI goes away.

use crate::flow::{Flow, Prompter};
use jitmo_ipc::{read_message, write_message, Choice, CoreEvent, FrameError, RequestId, UiRequest};
use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::thread::JoinHandle;

pub struct CoreService {
    tx: Sender<UiRequest>,
    thread: Option<JoinHandle<()>>,
}

impl CoreService {
    /// Start the service thread. `emit` is called for every event, from that thread.
    pub fn spawn(flow: Flow, emit: impl Fn(CoreEvent) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<UiRequest>();
        let thread = std::thread::Builder::new()
            .name("jitmo-core".into())
            .spawn(move || {
                let mut flow = flow;
                run(&mut flow, rx, emit)
            })
            .expect("failed to start the core thread");
        CoreService {
            tx,
            thread: Some(thread),
        }
    }

    /// Send a UI message to Core. Never blocks.
    pub fn send(&self, req: UiRequest) {
        // The thread only ends when this struct is dropped.
        let _ = self.tx.send(req);
    }
}

impl Drop for CoreService {
    fn drop(&mut self) {
        // Closing the channel ends the loop; then wait for it.
        let (dead, _) = mpsc::channel();
        self.tx = dead;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Serve one client over a byte stream: newline-delimited [`UiRequest`]s in,
/// [`CoreEvent`]s out. Returns when the client closes its end (or the stream
/// breaks). A request still waiting for an answer at that point is never
/// approved. A line that is not valid JSON is logged and skipped.
pub fn serve_connection<R, W>(flow: &mut Flow, reader: R, writer: W)
where
    R: BufRead + Send,
    W: Write,
{
    let (tx, rx) = mpsc::channel::<UiRequest>();
    let writer = Mutex::new(writer);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut reader = reader;
            loop {
                match read_message::<_, UiRequest>(&mut reader) {
                    Ok(Some(req)) => {
                        if tx.send(req).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(FrameError::Json(e)) => tracing::warn!("ignoring a bad message: {e}"),
                    Err(FrameError::Io(_)) => break,
                }
            }
            // `tx` drops here, and the request loop sees the disconnect.
        });
        run(flow, rx, |event| {
            if let Ok(mut w) = writer.lock() {
                let _ = write_message(&mut *w, &event);
            }
        });
    });
}

fn run(flow: &mut Flow, rx: Receiver<UiRequest>, emit: impl Fn(CoreEvent)) {
    while let Ok(req) = rx.recv() {
        match req {
            UiRequest::Submit { id, text } => {
                emit(CoreEvent::Started { id });
                let mut prompter = ChannelPrompter {
                    id,
                    rx: &rx,
                    emit: &emit,
                };
                // A panic in any connector must not kill the engine: answer this request
                // with a failure and keep serving the next one.
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    flow.handle(id, &text, &mut prompter)
                }))
                .unwrap_or_else(|_| CoreEvent::Failed {
                    id,
                    message: "Something went wrong with that request.".into(),
                });
                emit(done);
            }
            UiRequest::Undo { id } => {
                let message = flow.undo();
                emit(CoreEvent::Done {
                    id,
                    message,
                    undoable: false,
                });
            }
            // Answers or cancels with nothing waiting for them: ignore.
            UiRequest::Confirm { .. }
            | UiRequest::Choose { .. }
            | UiRequest::Answer { .. }
            | UiRequest::Cancel { .. } => {}
        }
    }
}

/// Asks the UI and blocks until it answers.
struct ChannelPrompter<'a, E: Fn(CoreEvent)> {
    id: RequestId,
    rx: &'a Receiver<UiRequest>,
    emit: &'a E,
}

impl<E: Fn(CoreEvent)> Prompter for ChannelPrompter<'_, E> {
    fn confirm(&mut self, text: &str) -> bool {
        (self.emit)(CoreEvent::ConfirmNeeded {
            id: self.id,
            text: text.into(),
        });
        loop {
            match self.rx.recv() {
                Ok(UiRequest::Confirm { id, approved }) if id == self.id => return approved,
                Ok(UiRequest::Cancel { id }) if id == self.id => return false,
                Ok(_) => {}             // not for this request; drop it
                Err(_) => return false, // UI gone: never approve
            }
        }
    }

    fn choose(&mut self, question: &str, choices: &[Choice]) -> Option<String> {
        (self.emit)(CoreEvent::ChoiceNeeded {
            id: self.id,
            question: question.into(),
            choices: choices.to_vec(),
        });
        loop {
            match self.rx.recv() {
                Ok(UiRequest::Choose { id, choice_id }) if id == self.id => return Some(choice_id),
                Ok(UiRequest::Cancel { id }) if id == self.id => return None,
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }

    fn ask_text(&mut self, question: &str, hint: &str) -> Option<String> {
        (self.emit)(CoreEvent::TextNeeded {
            id: self.id,
            question: question.into(),
            hint: hint.into(),
        });
        loop {
            match self.rx.recv() {
                Ok(UiRequest::Answer { id, text }) if id == self.id => return Some(text),
                Ok(UiRequest::Cancel { id }) if id == self.id => return None,
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::Router;
    use jitmo_connectors::fake::FakeConnector;
    use jitmo_connectors::Connector;
    use jitmo_skills::Skill;
    use jitmo_store::Store;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    fn service() -> (
        CoreService,
        Receiver<CoreEvent>,
        Arc<FakeConnector>,
        Arc<FakeConnector>,
    ) {
        let skills = [
            include_str!("../../../skills/apps.open.yaml"),
            include_str!("../../../skills/stub.send.yaml"),
        ]
        .iter()
        .map(|y| Skill::parse(y).unwrap())
        .collect();
        let sys = Arc::new(FakeConnector::new("windows_system"));
        let msg = Arc::new(FakeConnector::new("stub_messenger"));
        let mut conns: HashMap<String, Arc<dyn Connector>> = HashMap::new();
        conns.insert("windows_system".into(), sys.clone());
        conns.insert("stub_messenger".into(), msg.clone());
        let flow = Flow::new(Router::new(skills), Store::open_in_memory().unwrap(), conns);
        let (etx, erx) = mpsc::channel();
        let etx = std::sync::Mutex::new(etx);
        let svc = CoreService::spawn(flow, move |e| {
            let _ = etx.lock().unwrap().send(e);
        });
        (svc, erx, sys, msg)
    }

    fn next(rx: &Receiver<CoreEvent>) -> CoreEvent {
        rx.recv_timeout(Duration::from_secs(5))
            .expect("expected an event")
    }

    #[test]
    fn simple_request_emits_started_then_done() {
        let (svc, rx, sys, _) = service();
        svc.send(UiRequest::Submit {
            id: 1,
            text: "open spotify".into(),
        });
        assert_eq!(next(&rx), CoreEvent::Started { id: 1 });
        assert_eq!(
            next(&rx),
            CoreEvent::Done {
                id: 1,
                message: "Done.".into(),
                undoable: false
            }
        );
        assert_eq!(sys.execute_count(), 1);
    }

    #[test]
    fn send_waits_for_confirm_and_runs_after_approval() {
        let (svc, rx, _, msg) = service();
        svc.send(UiRequest::Submit {
            id: 2,
            text: "send hi to ved".into(),
        });
        assert_eq!(next(&rx), CoreEvent::Started { id: 2 });
        assert_eq!(
            next(&rx),
            CoreEvent::ConfirmNeeded {
                id: 2,
                text: "Send \"hi\" to ved?".into()
            }
        );
        assert_eq!(
            msg.execute_count(),
            0,
            "nothing runs before the user answers"
        );
        svc.send(UiRequest::Confirm {
            id: 2,
            approved: true,
        });
        assert!(matches!(next(&rx), CoreEvent::Done { id: 2, .. }));
        assert_eq!(msg.execute_count(), 1);
    }

    #[test]
    fn cancel_while_waiting_means_no() {
        let (svc, rx, _, msg) = service();
        svc.send(UiRequest::Submit {
            id: 3,
            text: "send hi to ved".into(),
        });
        next(&rx); // Started
        next(&rx); // ConfirmNeeded
        svc.send(UiRequest::Cancel { id: 3 });
        assert_eq!(
            next(&rx),
            CoreEvent::Done {
                id: 3,
                message: "Cancelled.".into(),
                undoable: false
            }
        );
        assert_eq!(msg.execute_count(), 0);
    }

    #[test]
    fn an_answer_for_a_different_request_does_not_approve() {
        let (svc, rx, _, msg) = service();
        svc.send(UiRequest::Submit {
            id: 4,
            text: "send hi to ved".into(),
        });
        next(&rx);
        next(&rx);
        svc.send(UiRequest::Confirm {
            id: 99,
            approved: true,
        }); // wrong id
        svc.send(UiRequest::Confirm {
            id: 4,
            approved: false,
        });
        assert_eq!(
            next(&rx),
            CoreEvent::Done {
                id: 4,
                message: "Cancelled.".into(),
                undoable: false
            }
        );
        assert_eq!(msg.execute_count(), 0);
    }

    #[test]
    fn dropping_the_ui_while_waiting_never_approves() {
        let (svc, rx, _, msg) = service();
        svc.send(UiRequest::Submit {
            id: 5,
            text: "send hi to ved".into(),
        });
        next(&rx);
        next(&rx);
        drop(svc); // channel closes while the flow waits
        assert_eq!(msg.execute_count(), 0);
        drop(rx);
    }

    fn asking_service() -> (CoreService, Receiver<CoreEvent>, Arc<FakeConnector>) {
        let yaml = include_str!("../../../skills/stub.send.yaml")
            .replace("examples:\n", "examples:\n  - \"message {contact}\"\n");
        let skills = vec![Skill::parse(&yaml).unwrap()];
        let msg = Arc::new(FakeConnector::new("stub_messenger"));
        let mut conns: HashMap<String, Arc<dyn Connector>> = HashMap::new();
        conns.insert("stub_messenger".into(), msg.clone());
        let flow = Flow::new(Router::new(skills), Store::open_in_memory().unwrap(), conns);
        let (etx, erx) = mpsc::channel();
        let etx = std::sync::Mutex::new(etx);
        let svc = CoreService::spawn(flow, move |e| {
            let _ = etx.lock().unwrap().send(e);
        });
        (svc, erx, msg)
    }

    #[test]
    fn a_missing_text_waits_for_the_typed_answer() {
        let (svc, rx, msg) = asking_service();
        svc.send(UiRequest::Submit {
            id: 10,
            text: "message ved".into(),
        });
        assert_eq!(next(&rx), CoreEvent::Started { id: 10 });
        assert!(matches!(
            next(&rx),
            CoreEvent::TextNeeded { id: 10, ref question, .. } if question == "What do you want to say?"
        ));
        svc.send(UiRequest::Answer {
            id: 99,
            text: "wrong request".into(),
        }); // not for this request: ignored
        svc.send(UiRequest::Answer {
            id: 10,
            text: "hello".into(),
        });
        assert_eq!(
            next(&rx),
            CoreEvent::ConfirmNeeded {
                id: 10,
                text: "Send \"hello\" to ved?".into()
            }
        );
        svc.send(UiRequest::Confirm {
            id: 10,
            approved: true,
        });
        assert!(matches!(next(&rx), CoreEvent::Done { id: 10, .. }));
        assert_eq!(msg.execute_count(), 1);
    }

    #[test]
    fn cancel_while_a_question_is_open_ends_the_request() {
        let (svc, rx, msg) = asking_service();
        svc.send(UiRequest::Submit {
            id: 11,
            text: "message ved".into(),
        });
        next(&rx);
        next(&rx); // TextNeeded
        svc.send(UiRequest::Cancel { id: 11 });
        assert_eq!(
            next(&rx),
            CoreEvent::Done {
                id: 11,
                message: "Cancelled.".into(),
                undoable: false
            }
        );
        assert_eq!(msg.execute_count(), 0);
    }

    #[test]
    fn undo_with_nothing_to_undo_answers() {
        let (svc, rx, _, _) = service();
        svc.send(UiRequest::Undo { id: 6 });
        assert_eq!(
            next(&rx),
            CoreEvent::Done {
                id: 6,
                message: "Nothing to undo.".into(),
                undoable: false
            }
        );
    }

    #[test]
    fn stray_answers_with_nothing_waiting_are_ignored() {
        let (svc, rx, _, _) = service();
        svc.send(UiRequest::Confirm {
            id: 7,
            approved: true,
        });
        svc.send(UiRequest::Submit {
            id: 8,
            text: "open spotify".into(),
        });
        assert_eq!(next(&rx), CoreEvent::Started { id: 8 });
    }
}

#[cfg(test)]
#[path = "service_stream_tests.rs"]
mod stream_tests;
