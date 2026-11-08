//! Live test: Core served over a real named pipe, end to end with a fake connector.
//! Run with `cargo test -p jitmo-core --test pipe_live -- --ignored`.
#![cfg(windows)]

use jitmo_adapter_windows::pipe::{connect, PipeServer};
use jitmo_connectors::fake::FakeConnector;
use jitmo_connectors::Connector;
use jitmo_core::flow::Flow;
use jitmo_core::router::Router;
use jitmo_core::service::serve_connection;
use jitmo_ipc::{read_message, write_message, CoreEvent, UiRequest};
use jitmo_skills::Skill;
use jitmo_store::Store;
use std::collections::HashMap;
use std::io::BufReader;
use std::sync::Arc;

#[test]
#[ignore = "uses a real named pipe"]
fn submit_and_confirm_over_a_real_pipe() {
    let name = format!(r"\\.\pipe\jitmo-core-live-{}", std::process::id());
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
    let mut flow = Flow::new(Router::new(skills), Store::open_in_memory().unwrap(), conns);

    let mut server = PipeServer::bind(&name).unwrap();
    let t = std::thread::spawn(move || {
        let stream = server.accept().unwrap();
        let (r, w) = stream.split();
        serve_connection(&mut flow, BufReader::new(r), w);
    });

    let stream = connect(&name).unwrap();
    let (r, mut w) = stream.split();
    let mut r = BufReader::new(r);
    let mut ev = || read_message::<_, CoreEvent>(&mut r).unwrap().unwrap();

    write_message(
        &mut w,
        &UiRequest::Submit {
            id: 1,
            text: "open spotify".into(),
        },
    )
    .unwrap();
    assert_eq!(ev(), CoreEvent::Started { id: 1 });
    assert!(matches!(ev(), CoreEvent::Done { id: 1, .. }));
    assert_eq!(sys.execute_count(), 1);

    write_message(
        &mut w,
        &UiRequest::Submit {
            id: 2,
            text: "send hi to ved".into(),
        },
    )
    .unwrap();
    assert_eq!(ev(), CoreEvent::Started { id: 2 });
    assert!(matches!(ev(), CoreEvent::ConfirmNeeded { id: 2, .. }));
    write_message(
        &mut w,
        &UiRequest::Confirm {
            id: 2,
            approved: true,
        },
    )
    .unwrap();
    assert!(matches!(ev(), CoreEvent::Done { id: 2, .. }));
    assert_eq!(msg.execute_count(), 1);

    drop(w);
    drop(ev);
    drop(r);
    t.join().unwrap();
}
