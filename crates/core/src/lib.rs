//! Core engine: flow, resolver, policy, verifier and scheduler.
//!
//! No Windows-only code lives here, so it builds and tests anywhere.

pub mod flow;
pub mod policy;
pub mod router;
pub mod service;
pub mod verifier;
