use crate::storage::open_store;
use std::time::Duration;
use tauri::AppHandle;

/// Every second, ring the reminders and timers that have come due, in the bar itself (it
/// pops up with an alarm sound; Windows notifications are hidden by Focus assist and Do Not
/// Disturb while still reporting success, so they cannot be relied on). Runs on its own
/// thread with its own database connection.
pub(crate) fn start_reminder_scheduler(
    db: std::path::PathBuf,
    key: Option<jitmo_store::Key>,
    app: AppHandle,
) {
    std::thread::spawn(move || {
        let store = match open_store(&db, key.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("reminders: could not open the database: {e}");
                return;
            }
        };
        loop {
            let now = chrono_utc_now();
            // The alarm is the bar itself: it pops up, rings, and stays until Esc, Enter or
            // Backspace. (The Windows notification, which came from "Windows PowerShell" and
            // was easy to miss, is no longer used.)
            let _ = jitmo_core::scheduler::fire_due(&store, &now, &mut |n| {
                let text = if n.title == "Reminder" {
                    format!("Reminder: {}", n.body)
                } else {
                    n.body.clone()
                };
                crate::bar::show_alarm(&app, &text);
                true
            });
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Now as `YYYY-MM-DDTHH:MM:SSZ`, the format reminders are stored in.
pub(crate) fn chrono_utc_now() -> String {
    jitmo_connectors::reminders::utc_text(chrono::Local::now())
}
