//! Fires reminders and timers that have come due.
//!
//! The app calls [`fire_due`] every few seconds from its own thread. A reminder
//! is marked done whether or not the notification worked, so a broken
//! notification setting can never make the same reminder fire in a loop.

use jitmo_store::{Reminder, Store, StoreError};

/// How late a reminder may be and still count as on time.
const ON_TIME_SECONDS: i64 = 300;

/// What to show for a reminder. `late` is true if the app was closed when it was due.
pub struct Notice {
    pub title: String,
    pub body: String,
}

pub fn notice_for(r: &Reminder, late: bool) -> Notice {
    let timer = r.text == "Timer";
    let title = if timer { "Timer finished" } else { "Reminder" };
    let body = if timer {
        "Your timer is up.".to_string()
    } else {
        r.text.clone()
    };
    Notice {
        title: title.to_string(),
        body: if late {
            format!("{body} (missed while jitmo was closed)")
        } else {
            body
        },
    }
}

fn parse_utc(s: &str) -> Option<i64> {
    // YYYY-MM-DDTHH:MM:SSZ -> seconds since an arbitrary epoch, good for differences.
    let digits: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect();
    let [y, mo, d, h, mi, se] = digits[..] else {
        return None;
    };
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

/// Show every due reminder with `notify` (which returns whether it worked) and
/// mark it done. Returns how many were handled.
pub fn fire_due(
    store: &Store,
    now_utc: &str,
    notify: &mut dyn FnMut(&Notice) -> bool,
) -> Result<usize, StoreError> {
    let now = parse_utc(now_utc);
    let due = store.due_reminders(now_utc)?;
    for r in &due {
        let late = match (now, parse_utc(&r.due_at)) {
            (Some(n), Some(d)) => n - d > ON_TIME_SECONDS,
            _ => false,
        };
        if !notify(&notice_for(r, late)) {
            eprintln!("could not show the notification for reminder {}", r.id);
        }
        store.mark_reminder_done(r.id)?;
    }
    Ok(due.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_reminders_fire_once_each() {
        let s = Store::open_in_memory().unwrap();
        s.add_reminder("2026-11-08T12:10:00Z", "Call mom").unwrap();
        s.add_reminder("2026-11-08T18:00:00Z", "Later").unwrap();
        let mut shown = Vec::new();
        let n = fire_due(&s, "2026-11-08T12:10:05Z", &mut |x| {
            shown.push((x.title.clone(), x.body.clone()));
            true
        })
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(shown, [("Reminder".to_string(), "Call mom".to_string())]);
        // Same moment again: nothing left to fire.
        assert_eq!(
            fire_due(&s, "2026-11-08T12:10:06Z", &mut |_| true).unwrap(),
            0
        );
    }

    #[test]
    fn a_timer_has_its_own_wording() {
        let s = Store::open_in_memory().unwrap();
        s.add_reminder("2026-11-08T12:10:00Z", "Timer").unwrap();
        let mut shown = Vec::new();
        fire_due(&s, "2026-11-08T12:10:00Z", &mut |x| {
            shown.push((x.title.clone(), x.body.clone()));
            true
        })
        .unwrap();
        assert_eq!(
            shown[0],
            ("Timer finished".into(), "Your timer is up.".into())
        );
    }

    #[test]
    fn a_reminder_missed_while_closed_says_so() {
        let s = Store::open_in_memory().unwrap();
        s.add_reminder("2026-11-08T12:00:00Z", "Call mom").unwrap();
        let mut body = String::new();
        fire_due(&s, "2026-11-08T14:00:00Z", &mut |x| {
            body = x.body.clone();
            true
        })
        .unwrap();
        assert_eq!(body, "Call mom (missed while jitmo was closed)");
    }

    #[test]
    fn a_failed_notification_still_marks_it_done_so_it_cannot_loop() {
        let s = Store::open_in_memory().unwrap();
        s.add_reminder("2026-11-08T12:00:00Z", "x").unwrap();
        assert_eq!(
            fire_due(&s, "2026-11-08T12:00:01Z", &mut |_| false).unwrap(),
            1
        );
        assert_eq!(
            fire_due(&s, "2026-11-08T12:00:02Z", &mut |_| false).unwrap(),
            0
        );
    }

    #[test]
    fn utc_text_parses_for_differences() {
        let a = parse_utc("2026-11-08T12:00:00Z").unwrap();
        let b = parse_utc("2026-11-09T12:00:30Z").unwrap();
        assert_eq!(b - a, 86_400 + 30);
    }
}
