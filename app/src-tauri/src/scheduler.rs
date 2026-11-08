use crate::storage::open_store;

/// Every 10 seconds, show the reminders and timers that have come due as
/// Windows notifications. Runs on its own thread with its own database connection.
pub(crate) fn start_reminder_scheduler(db: std::path::PathBuf, key: Option<jitmo_store::Key>) {
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
            let _ = jitmo_core::scheduler::fire_due(&store, &now, &mut |n| {
                #[cfg(windows)]
                {
                    jitmo_adapter_windows::toast::show_toast(&n.title, &n.body).is_ok()
                }
                #[cfg(not(windows))]
                {
                    let _ = n;
                    false
                }
            });
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    });
}

/// Now as `YYYY-MM-DDTHH:MM:SSZ`, the format reminders are stored in.
pub(crate) fn chrono_utc_now() -> String {
    jitmo_connectors::reminders::utc_text(chrono::Local::now())
}
