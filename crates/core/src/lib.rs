//! Core engine: flow, resolver, policy, verifier and scheduler.
//!
//! No Windows-only code lives here, so it builds and tests anywhere.

pub mod clarify;
pub mod context;
pub mod flow;
pub mod followup;
pub mod hinglish;
pub mod known;
pub mod normalize;
pub mod plan;
pub mod policy;
pub mod resolver;
pub mod router;
pub mod scheduler;
pub mod service;
pub mod verifier;
