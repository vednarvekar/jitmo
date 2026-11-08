//! Message types shared by the UI, Core and the model worker (LLD 07, "Processes").
//!
//! Transport: Windows named pipes, one JSON message per line (newline-delimited).
//! This crate has no OS code, so it builds and tests anywhere.
//!
//! Two links exist:
//! - UI <-> Core: [`UiRequest`] goes to Core, [`CoreEvent`] comes back.
//! - Core <-> Model worker: [`WorkerRequest`] goes to the worker, [`WorkerReply`] comes back.
//!   Only Core talks to the worker, and the worker has no network access.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

/// Identifies one user request so events can be matched to it.
pub type RequestId = u64;

// ---------------------------------------------------------------------------
// Shared pieces
// ---------------------------------------------------------------------------

/// Context note sent with a request: what the user is looking at (LLD, "Router and extractor").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextNote {
    /// Name of the foreground app, if known.
    pub open_app: Option<String>,
    /// Display name of the selected item (file, text), if any.
    pub selected_item: Option<String>,
    /// App names Core knows about (installed or running). The extractor uses them as a
    /// closed list. Absent in older messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<String>,
    /// Contact names Core knows about. The extractor uses them as a closed list.
    /// Absent in older messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<String>,
}

/// One entry the user can pick when Core needs a choice (ties, FR4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// UI -> Core
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiRequest {
    /// The user typed a request in the command bar.
    Submit { id: RequestId, text: String },
    /// Answer to a [`CoreEvent::ConfirmNeeded`] card.
    Confirm { id: RequestId, approved: bool },
    /// Answer to a [`CoreEvent::ChoiceNeeded`] question.
    Choose { id: RequestId, choice_id: String },
    /// Cancel a request that is still running.
    Cancel { id: RequestId },
    /// Undo the last undoable action of a finished request (SR3).
    Undo { id: RequestId },
}

// ---------------------------------------------------------------------------
// Core -> UI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreEvent {
    /// Request accepted; Core is working on it.
    Started { id: RequestId },
    /// A send/share/change needs approval before it runs (policy engine).
    ConfirmNeeded { id: RequestId, text: String },
    /// Several matches tie; ask one question (FR4).
    ChoiceNeeded {
        id: RequestId,
        question: String,
        choices: Vec<Choice>,
    },
    /// Final one-line result. Every request ends with exactly one of `Done` or `Failed`.
    Done {
        id: RequestId,
        message: String,
        /// True if the UI should offer an Undo button.
        undoable: bool,
    },
    /// Final one-line failure message (LLD, "Error handling"). Never silent.
    Failed { id: RequestId, message: String },
}

// ---------------------------------------------------------------------------
// Core <-> Model worker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerRequest {
    /// Route the text and extract spans in one call (Tier 0).
    Understand {
        id: RequestId,
        text: String,
        context: ContextNote,
    },
    /// Liveness check; Core restarts the worker if it does not answer.
    Ping { id: RequestId },
    /// Pull slot spans out of the text for a task the router already chose. Loads the
    /// extractor model on first use; it stays loaded until the worker exits.
    Extract {
        id: RequestId,
        text: String,
        /// Task label the router chose (A1, M2, ...), if known.
        #[serde(default)]
        task: Option<String>,
        #[serde(default)]
        context: ContextNote,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerReply {
    Understood {
        id: RequestId,
        result: Understanding,
    },
    Pong {
        id: RequestId,
    },
    Extracted {
        id: RequestId,
        spans: Vec<Span>,
    },
    Error {
        id: RequestId,
        message: String,
    },
}

/// Output of the router and extractor. Matches the JSON shown in LLD 07.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Understanding {
    /// Candidate tasks, best first.
    pub tasks: Vec<TaskScore>,
    /// Score that the request is outside what the assistant can do.
    pub out_of_scope: f32,
    /// True if the request looks like a chain of tasks.
    pub chained: bool,
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskScore {
    pub id: String,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    /// Slot type, such as `file_ref`, `contact` or `app`.
    #[serde(rename = "type")]
    pub kind: String,
}

// ---------------------------------------------------------------------------
// Framing: one JSON message per line
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("pipe error: {0}")]
    Io(#[from] io::Error),
    #[error("bad message: {0}")]
    Json(#[from] serde_json::Error),
}

/// Write one message followed by a newline, then flush.
pub fn write_message<W: Write, T: Serialize>(out: &mut W, msg: &T) -> Result<(), FrameError> {
    serde_json::to_writer(&mut *out, msg)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// Read one message. Returns `Ok(None)` when the other side closed the pipe.
pub fn read_message<R: BufRead, T: for<'de> Deserialize<'de>>(
    input: &mut R,
) -> Result<Option<T>, FrameError> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(line.trim_end())?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn ui_request_round_trips() {
        let msg = UiRequest::Submit {
            id: 1,
            text: "send this file to ved".into(),
        };
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();
        assert!(buf.ends_with(b"\n"));
        let back: UiRequest = read_message(&mut Cursor::new(buf)).unwrap().unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn events_use_snake_case_tags() {
        let json = serde_json::to_string(&CoreEvent::Failed {
            id: 2,
            message: "x".into(),
        })
        .unwrap();
        assert!(json.contains(r#""type":"failed""#));
    }

    #[test]
    fn understanding_matches_lld_json() {
        let json = r#"{
            "tasks": [ { "id": "whatsapp.send_file", "score": 0.94 } ],
            "out_of_scope": 0.01,
            "chained": false,
            "spans": [ { "text": "this file", "type": "file_ref" } ]
        }"#;
        let u: Understanding = serde_json::from_str(json).unwrap();
        assert_eq!(u.tasks[0].id, "whatsapp.send_file");
        assert_eq!(u.spans[0].kind, "file_ref");
    }

    #[test]
    fn several_messages_read_in_order_then_eof() {
        let mut buf = Vec::new();
        write_message(&mut buf, &WorkerRequest::Ping { id: 1 }).unwrap();
        write_message(&mut buf, &WorkerRequest::Ping { id: 2 }).unwrap();
        let mut r = Cursor::new(buf);
        let a: WorkerRequest = read_message(&mut r).unwrap().unwrap();
        let b: WorkerRequest = read_message(&mut r).unwrap().unwrap();
        assert_eq!(
            (a, b),
            (WorkerRequest::Ping { id: 1 }, WorkerRequest::Ping { id: 2 })
        );
        assert!(read_message::<_, WorkerRequest>(&mut r).unwrap().is_none());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let r: Result<Option<UiRequest>, _> =
            read_message(&mut Cursor::new(b"not json\n".to_vec()));
        assert!(matches!(r, Err(FrameError::Json(_))));
    }
}
