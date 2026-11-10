//! The language model's budget in the app: it is warmed when the bar opens (the load hides
//! behind the typing) and dropped at once when Windows says memory is low.

use jitmo_llm::LocalLlm;
use std::sync::OnceLock;
use std::time::Duration;

static LLM: OnceLock<LocalLlm> = OnceLock::new();

/// Keep a handle to the model and start watching memory pressure.
pub(crate) fn install(llm: LocalLlm) {
    if LLM.set(llm).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let Some(watch) = jitmo_adapter_windows::budget::LowMemory::new() else {
            return;
        };
        loop {
            std::thread::sleep(Duration::from_secs(3));
            if watch.is_low() {
                if let Some(l) = LLM.get() {
                    l.unload();
                }
            }
        }
    });
}

/// The bar opened: start loading the model if there is one and memory allows.
pub(crate) fn warm() {
    if let Some(l) = LLM.get() {
        l.warm();
    }
}
