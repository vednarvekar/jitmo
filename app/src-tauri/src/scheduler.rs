use crate::storage::open_store;
use std::time::{Duration, Instant};
use tauri::AppHandle;

/// Every second, show the reminders and timers that have come due: as a Windows
/// notification, and in the bar itself (Windows hides notifications under Focus assist
/// or Do Not Disturb and still reports success, so the bar is the alert that cannot be
/// missed). Runs on its own thread with its own database connection.
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
        // A refused notification is tried again on later checks; the bar must not pop up
        // again for the same one.
        let mut last_in_bar: Option<(String, Instant)> = None;
        loop {
            let now = chrono_utc_now();
            let _ = jitmo_core::scheduler::fire_due(&store, &now, &mut |n| {
                let first_time = !matches!(
                    &last_in_bar,
                    Some((text, at)) if *text == n.body && at.elapsed() < Duration::from_secs(150)
                );
                if first_time {
                    last_in_bar = Some((n.body.clone(), Instant::now()));
                    let text = if n.title == "Reminder" {
                        format!("Reminder: {}", n.body)
                    } else {
                        n.body.clone()
                    };
                    crate::bar::show_notice(&app, &text);
                }
                #[cfg(windows)]
                {
                    jitmo_adapter_windows::toast::show_toast(&n.title, &n.body).is_ok()
                }
                #[cfg(not(windows))]
                {
                    false
                }
            });
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Now as `YYYY-MM-DDTHH:MM:SSZ`, the format reminders are stored in.
pub(crate) fn chrono_utc_now() -> String {
    jitmo_connectors::reminders::utc_text(chrono::Local::now())
}
