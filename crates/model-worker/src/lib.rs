//! Model worker: the server loop that runs inside the worker process, and the
//! [`client::WorkerClient`] that Core uses to start, call and restart it.
//!
//! The worker is a separate process so that a model crash cannot take Core
//! down, and so its memory goes away when it exits after idle (D27). The worker
//! never opens a network connection; it only reads requests on stdin and writes
//! replies on stdout (one JSON message per line, see `jitmo-ipc`).

pub mod client;
pub mod engine;
pub mod server;

pub use client::{WorkerClient, WorkerError, WorkerLauncher};
pub use engine::{Engine, TestEngine, UnbuiltEngine};
pub use server::{serve, Exit};

#[cfg(feature = "models")]
pub mod router;
