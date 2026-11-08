//! The loop that runs inside the worker process.

use crate::engine::Engine;
use jitmo_ipc::{read_message, write_message, FrameError, WorkerReply, WorkerRequest};
use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

/// Why the loop ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    /// Core closed the pipe.
    Closed,
    /// Nothing arrived for the idle time; the process exits to free its memory.
    Idle,
}

/// Serve requests until Core closes the pipe or `idle` passes without a request.
pub fn serve<R, W, E>(
    input: R,
    mut output: W,
    engine: &mut E,
    idle: Option<Duration>,
) -> Result<Exit, FrameError>
where
    R: BufRead + Send + 'static,
    W: Write,
    E: Engine,
{
    // A reader thread lets the loop wait with a timeout.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = input;
        loop {
            let msg = read_message::<_, WorkerRequest>(&mut input);
            let stop = matches!(msg, Ok(None) | Err(FrameError::Io(_)));
            if tx.send(msg).is_err() || stop {
                break;
            }
        }
    });
    loop {
        let next = match idle {
            Some(d) => match rx.recv_timeout(d) {
                Ok(m) => m,
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(Exit::Idle),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Exit::Closed),
            },
            None => match rx.recv() {
                Ok(m) => m,
                Err(_) => return Ok(Exit::Closed),
            },
        };
        match next {
            Ok(None) => return Ok(Exit::Closed),
            Ok(Some(WorkerRequest::Ping { id })) => {
                write_message(&mut output, &WorkerReply::Pong { id })?
            }
            Ok(Some(WorkerRequest::Understand { id, text, context })) => {
                let reply = match engine.understand(&text, &context) {
                    Ok(result) => WorkerReply::Understood { id, result },
                    Err(message) => WorkerReply::Error { id, message },
                };
                write_message(&mut output, &reply)?;
            }
            Ok(Some(WorkerRequest::Extract {
                id,
                text,
                task,
                context,
            })) => {
                let reply = match engine.extract(&text, task.as_deref(), &context) {
                    Ok(spans) => WorkerReply::Extracted { id, spans },
                    Err(message) => WorkerReply::Error { id, message },
                };
                write_message(&mut output, &reply)?;
            }
            // A garbled line is answered, not fatal: Core can retry.
            Err(FrameError::Json(e)) => write_message(
                &mut output,
                &WorkerReply::Error {
                    id: 0,
                    message: format!("bad request: {e}"),
                },
            )?,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{TestEngine, UnbuiltEngine};
    use jitmo_ipc::ContextNote;
    use std::io::Cursor;

    fn run(input: Vec<u8>, engine: &mut impl Engine) -> Vec<WorkerReply> {
        let mut out = Vec::new();
        let exit = serve(Cursor::new(input), &mut out, engine, None).unwrap();
        assert_eq!(exit, Exit::Closed);
        let mut r = Cursor::new(out);
        let mut replies = Vec::new();
        while let Some(m) = read_message::<_, WorkerReply>(&mut r).unwrap() {
            replies.push(m);
        }
        replies
    }

    #[test]
    fn answers_ping_and_understand_in_order() {
        let mut input = Vec::new();
        write_message(&mut input, &WorkerRequest::Ping { id: 1 }).unwrap();
        write_message(
            &mut input,
            &WorkerRequest::Understand {
                id: 2,
                text: "open spotify".into(),
                context: ContextNote::default(),
            },
        )
        .unwrap();
        let replies = run(input, &mut UnbuiltEngine);
        assert_eq!(replies[0], WorkerReply::Pong { id: 1 });
        assert!(matches!(replies[1], WorkerReply::Understood { id: 2, .. }));
    }

    #[test]
    fn engine_error_and_garbage_become_error_replies() {
        let mut input = Vec::new();
        write_message(
            &mut input,
            &WorkerRequest::Understand {
                id: 3,
                text: "__error__".into(),
                context: ContextNote::default(),
            },
        )
        .unwrap();
        input.extend_from_slice(b"not json\n");
        write_message(&mut input, &WorkerRequest::Ping { id: 4 }).unwrap();
        let replies = run(input, &mut TestEngine);
        assert!(matches!(replies[0], WorkerReply::Error { id: 3, .. }));
        assert!(matches!(replies[1], WorkerReply::Error { id: 0, .. }));
        assert_eq!(replies[2], WorkerReply::Pong { id: 4 });
    }

    #[test]
    fn exits_after_idle() {
        // A pipe that never delivers a line and never closes.
        let (keep, reader) = std::sync::mpsc::channel::<u8>();
        struct Blocked(std::sync::mpsc::Receiver<u8>);
        impl std::io::Read for Blocked {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.0.recv();
                Ok(0)
            }
        }
        let input = std::io::BufReader::new(Blocked(reader));
        let mut out = Vec::new();
        let exit = serve(
            input,
            &mut out,
            &mut UnbuiltEngine,
            Some(Duration::from_millis(50)),
        )
        .unwrap();
        assert_eq!(exit, Exit::Idle);
        drop(keep);
    }
}
