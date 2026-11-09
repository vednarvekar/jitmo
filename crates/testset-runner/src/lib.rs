//! Test-set runner: runs the tests from doc 04 in a clean setup and records
//! pass or fail, time and peak memory for each (doc 04, "Metrics").
//!
//! - `doc`: reads the 100 tests from the document itself.
//! - `spec`: the expectations for tests that are automated today.
//! - `run`: resets, runs one test, judges it.
//! - `report`: the metrics table and the text report.

pub mod audit;
pub mod doc;
pub mod report;
pub mod run;
pub mod spec;

/// Highest memory this process has reached so far, in bytes (Windows only).
pub fn peak_memory_bytes() -> Option<u64> {
    #[cfg(windows)]
    return jitmo_adapter_windows::memory::peak_working_set_bytes();
    #[cfg(not(windows))]
    None
}
