//! Tests for `serve_connection` over an in-memory duplex stream.

use super::*;
use crate::router::Router;
use jitmo_connectors::fake::FakeConnector;
use jitmo_connectors::Connector;
use jitmo_skills::Skill;
use jitmo_store::Store;
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::sync::Arc;

/// One direction of an in-memory pipe.
struct ChanReader(Receiver<Vec<u8>>, Vec<u8>);
impl Read for ChanReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1.is_empty() {
            match self.0.recv() {
                Ok(b) => self.1 = b,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.1.len());
        buf[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}
struct ChanWriter(Sender<Vec<u8>>);
impl Write for ChanWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(buf.to_vec())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn pipe() -> (ChanWriter, BufReader<ChanReader>) {
    let (tx, rx) = mpsc::channel();
    (ChanWriter(tx), BufReader::new(ChanReader(rx, Vec::new())))
}

struct Client {
    to_core: ChanWriter,
    from_core: BufReader<ChanReader>,
}
impl Client {
    fn send(&mut self, req: UiRequest) {
        write_message(&mut self.to_core, &req).unwrap();
    }
    fn next(&mut self) -> CoreEvent {
        read_message(&mut self.from_core)
            .unwrap()
            .expect("expected an event")
    }
}

fn flow() -> (Flow, Arc<FakeConnector>) {
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
    conns.insert("windows_system".into(), sys);
    conns.insert("stub_messenger".into(), msg.clone());
    (
        Flow::new(Router::new(skills), Store::open_in_memory().unwrap(), conns),
        msg,
    )
}

/// Run `serve_connection` on a thread while `body` plays the client; the
/// client disconnects when `body` returns. Returns the messenger connector.
fn with_server(body: impl FnOnce(&mut Client)) -> Arc<FakeConnector> {
    let (mut flow, msg) = flow();
    let (to_core, core_in) = pipe();
    let (core_out, from_core) = pipe();
    let server = std::thread::spawn(move || {
        serve_connection(&mut flow, core_in, core_out);
    });
    let mut client = Client { to_core, from_core };
    body(&mut client);
    drop(client);
    server.join().expect("server must not panic");
    msg
}

#[test]
fn submit_emits_started_then_done() {
    with_server(|c| {
        c.send(UiRequest::Submit {
            id: 1,
            text: "open spotify".into(),
        });
        assert_eq!(c.next(), CoreEvent::Started { id: 1 });
        assert!(matches!(c.next(), CoreEvent::Done { id: 1, .. }));
    });
}

#[test]
fn confirm_round_trip() {
    let msg = with_server(|c| {
        c.send(UiRequest::Submit {
            id: 2,
            text: "send hi to ved".into(),
        });
        assert_eq!(c.next(), CoreEvent::Started { id: 2 });
        assert!(matches!(c.next(), CoreEvent::ConfirmNeeded { id: 2, .. }));
        c.send(UiRequest::Confirm {
            id: 2,
            approved: true,
        });
        assert!(matches!(c.next(), CoreEvent::Done { id: 2, .. }));
    });
    assert_eq!(msg.execute_count(), 1);
}

#[test]
fn cancel_and_undo_are_answered() {
    with_server(|c| {
        c.send(UiRequest::Submit {
            id: 3,
            text: "send hi to ved".into(),
        });
        c.next();
        c.next();
        c.send(UiRequest::Cancel { id: 3 });
        match c.next() {
            CoreEvent::Done { id: 3, message, .. } => assert_eq!(message, "Cancelled."),
            other => panic!("unexpected {other:?}"),
        }
        c.send(UiRequest::Undo { id: 4 });
        assert!(matches!(c.next(), CoreEvent::Done { id: 4, .. }));
    });
}

#[test]
fn choice_answer_travels_over_the_stream() {
    // The resolver only asks with real ambiguous data, so drive the prompter
    // against the same stream plumbing directly.
    let (to_core, core_in) = pipe();
    let (core_out, mut from_core) = pipe();
    let (tx, rx) = mpsc::channel::<UiRequest>();
    let out = Mutex::new(core_out);
    let mut reader = core_in;
    let mut writer = to_core;
    let answer = std::thread::scope(|s| {
        s.spawn(move || {
            // client -> core bridge, as serve_connection does it
            while let Ok(Some(req)) = read_message::<_, UiRequest>(&mut reader) {
                let _ = tx.send(req);
            }
        });
        let out = &out;
        let h = s.spawn(move || {
            let emit = |e: CoreEvent| {
                write_message(&mut *out.lock().unwrap(), &e).unwrap();
            };
            let mut p = ChannelPrompter {
                id: 9,
                rx: &rx,
                emit: &emit,
            };
            p.choose(
                "Which?",
                &[Choice {
                    id: "a".into(),
                    label: "A".into(),
                }],
            )
        });
        let ev: CoreEvent = read_message(&mut from_core).unwrap().unwrap();
        assert!(matches!(ev, CoreEvent::ChoiceNeeded { id: 9, .. }));
        write_message(
            &mut writer,
            &UiRequest::Choose {
                id: 9,
                choice_id: "a".into(),
            },
        )
        .unwrap();
        let r = h.join().unwrap();
        drop(writer); // lets the bridge thread end
        r
    });
    assert_eq!(answer, Some("a".into()));
}

#[test]
fn disconnect_mid_request_never_approves_and_does_not_hang() {
    let msg = with_server(|c| {
        c.send(UiRequest::Submit {
            id: 5,
            text: "send hi to ved".into(),
        });
        c.next();
        assert!(matches!(c.next(), CoreEvent::ConfirmNeeded { .. }));
        // The client goes away here, with the flow still waiting.
    });
    assert_eq!(msg.execute_count(), 0);
}

#[test]
fn garbage_line_is_skipped_and_server_keeps_working() {
    with_server(|c| {
        c.to_core
            .write_all(b"this is not json\n{\"type\":\"nope\"}\n")
            .unwrap();
        c.send(UiRequest::Submit {
            id: 6,
            text: "open spotify".into(),
        });
        assert_eq!(c.next(), CoreEvent::Started { id: 6 });
        assert!(matches!(c.next(), CoreEvent::Done { id: 6, .. }));
    });
}
