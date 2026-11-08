//! Windows-only code: UI Automation, named pipes, Credential Manager, system controls.
//!
//! Anything that needs a Windows API lives behind `#[cfg(windows)]`, so the
//! rest of the workspace still builds and tests on other systems. The pure
//! logic (matching names, parsing output) is not gated, so it is unit-tested
//! everywhere.

pub mod apps;
