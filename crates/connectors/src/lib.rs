//! The Connector trait and the app connectors (LLD 07, "Connector interface").
//!
//! Every app connector implements [`Connector`]. Skills call connectors only
//! through it, so adding an app never changes the engine. This crate has no
//! Windows-only code; connectors that need UI Automation call into
//! `adapter-windows`.

pub mod browser;
pub mod dnd;
pub mod fake;
pub mod files;
pub mod local_calc;
pub mod reminders;
pub mod repeat;
pub mod settings;
pub mod spotify;
pub mod sun;
pub mod system_controls;
pub mod web_lookup;
#[cfg(windows)]
pub mod windows_system;

use std::collections::BTreeMap;

/// Named arguments for an action, already filled in from the skill's slots.
pub type Args = BTreeMap<String, String>;

/// What an action returned, such as a file path or a message id.
/// Passed back to [`Connector::undo`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output(pub BTreeMap<String, String>);

/// One action a connector supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSpec {
    pub name: String,
    /// Names of the arguments it needs.
    pub required_args: Vec<String>,
}

/// Is the app installed, signed in and reachable?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ready,
    NotInstalled,
    NotSignedIn,
    /// Installed, but cannot be reached right now (offline, not running, UI changed).
    Unreachable,
}

/// Why an action or check failed. Each variant maps to a one-line user message
/// (LLD, "Error handling").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnError {
    #[error("{0} isn't installed.")]
    NotInstalled(String),
    #[error("Please sign in to {0} again.")]
    NotSignedIn(String),
    #[error("{0} needs an internet connection.")]
    Offline(String),
    /// The app's UI no longer matches what the connector expects. The flow
    /// uses the skill's fallback.
    #[error("{0} looks different than expected.")]
    UiChanged(String),
    #[error("The action isn't supported: {0}")]
    UnknownAction(String),
    #[error("A required argument is missing: {0}")]
    MissingArg(String),
    #[error("{0}")]
    Failed(String),
}

/// Every app connector implements this.
pub trait Connector: Send + Sync {
    /// For example `whatsapp_desktop`, `gmail` or `windows_system`.
    fn id(&self) -> &str;

    /// Actions this connector supports, with their arguments.
    fn actions(&self) -> Vec<ActionSpec>;

    /// Is the app installed, signed in and reachable?
    fn health(&self) -> Health;

    /// Run one action. Must not retry on its own: the engine decides about
    /// retries, because repeating a send could send twice.
    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError>;

    /// Check that an action really happened (used by the verifier).
    /// Must not change anything.
    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError>;

    /// Reverse an action if possible. `None` means it cannot be undone.
    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>>;
}
