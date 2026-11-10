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
            // About ten seconds ahead, build the bar (hidden) so it is ready the moment the
            // reminder is due.
            let soon = jitmo_connectors::reminders::utc_text(
                chrono::Local::now() + chrono::Duration::seconds(12),
            );
            if store.due_reminders(&soon).is_ok_and(|d| !d.is_empty()) {
                crate::bar::warm_up(&app);
            }
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

/// Once a minute, look at the calendar (Microsoft or Google, whichever is signed in) and tell
/// the user about a meeting that starts within ten minutes, once per meeting. Off with the
/// setting `meeting_headsup` = `off`. A calendar that is not set up or signed in is skipped
/// quietly: this must never nag.
pub(crate) fn start_meeting_headsup(
    db: std::path::PathBuf,
    key: Option<jitmo_store::Key>,
    app: AppHandle,
) {
    std::thread::spawn(move || {
        let calendar = crate::registry::calendar_connector(&db, key.as_ref());
        let mut announced: std::collections::HashSet<String> = std::collections::HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(60));
            let off = open_store(&db, key.as_ref())
                .ok()
                .and_then(|s| s.get_setting("meeting_headsup").ok().flatten())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("off"));
            if off {
                continue;
            }
            let args: jitmo_connectors::Args = [("text".to_string(), "10".to_string())].into();
            let Ok(out) = calendar.execute("heads_up", &args) else {
                continue;
            };
            let message = out.0.get("message").cloned().unwrap_or_default();
            let Some(id) = out.0.get("id").cloned().filter(|_| !message.is_empty()) else {
                continue;
            };
            if announced.insert(id) {
                crate::bar::show_notice(&app, &message);
            }
        }
    });
}
