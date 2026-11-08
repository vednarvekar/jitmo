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
///
/// A repeating reminder is moved to its next time instead (same row, so Undo
/// still removes the whole series). The next time is always after `now_utc`,
/// so a computer that was off for several periods shows one notice, not a burst.
pub fn fire_due(
    store: &Store,
    now_utc: &str,
    notify: &mut dyn FnMut(&Notice) -> bool,
) -> Result<usize, StoreError> {
    fire_due_with(
        store,
        now_utc,
        notify,
        &jitmo_connectors::repeat::next_due_local,
    )
}

/// [`fire_due`] with the next-time rule given: `next(rule, due_at, now_utc)`.
pub fn fire_due_with(
    store: &Store,
    now_utc: &str,
    notify: &mut dyn FnMut(&Notice) -> bool,
    next: &dyn Fn(&str, &str, &str) -> Option<String>,
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
        let following = r
            .repeat
            .as_deref()
            .and_then(|rule| next(rule, &r.due_at, now_utc))
            // A rule that cannot move forward ends the reminder, never loops it.
            .filter(|t| t.as_str() > now_utc);
        match following {
            Some(t) => store.reschedule_reminder(r.id, &t)?,
            None => store.mark_reminder_done(r.id)?,
        }
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

    fn london(rule: &str, due: &str, now: &str) -> Option<String> {
        jitmo_connectors::repeat::next_due_in_zone(rule, due, now, "Europe/London")
    }

    fn new_york(rule: &str, due: &str, now: &str) -> Option<String> {
        jitmo_connectors::repeat::next_due_in_zone(rule, due, now, "America/New_York")
    }

    #[test]
    fn a_repeating_reminder_is_rearmed_not_finished() {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .add_reminder_repeating("2027-01-10T09:00:00Z", "Take vitamins", Some("daily 09:00"))
            .unwrap();
        let mut n = 0;
        fire_due_with(
            &s,
            "2027-01-10T09:00:03Z",
            &mut |_| {
                n += 1;
                true
            },
            &london,
        )
        .unwrap();
        assert_eq!(n, 1);
        let pending = s.pending_reminders().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        assert_eq!(pending[0].due_at, "2027-01-11T09:00:00Z");
        // Not due again until tomorrow.
        assert_eq!(
            fire_due_with(&s, "2027-01-10T09:00:10Z", &mut |_| true, &london).unwrap(),
            0
        );
    }

    #[test]
    fn closed_for_many_periods_fires_once_and_schedules_a_future_time() {
        let s = Store::open_in_memory().unwrap();
        s.add_reminder_repeating("2027-01-10T09:00:00Z", "Water", Some("every 120m"))
            .unwrap();
        let mut shown = 0;
        let now = "2027-01-13T09:30:00Z";
        let fired = fire_due_with(
            &s,
            now,
            &mut |_| {
                shown += 1;
                true
            },
            &london,
        )
        .unwrap();
        assert_eq!((fired, shown), (1, 1));
        let next = s.pending_reminders().unwrap()[0].due_at.clone();
        assert_eq!(next, "2027-01-13T11:00:00Z");
        assert!(next.as_str() > now);
        assert_eq!(fire_due_with(&s, now, &mut |_| true, &london).unwrap(), 0);

        let d = Store::open_in_memory().unwrap();
        d.add_reminder_repeating("2027-01-01T09:00:00Z", "Vitamins", Some("daily 09:00"))
            .unwrap();
        fire_due_with(&d, "2027-01-20T15:00:00Z", &mut |_| true, &london).unwrap();
        assert_eq!(
            d.pending_reminders().unwrap()[0].due_at,
            "2027-01-21T09:00:00Z"
        );
    }

    #[test]
    fn daily_rearm_keeps_wall_clock_across_daylight_saving() {
        let s = Store::open_in_memory().unwrap();
        // 9:00 EST on Saturday 13 March 2027; clocks go forward the next day.
        s.add_reminder_repeating("2027-03-13T14:00:00Z", "Vitamins", Some("daily 09:00"))
            .unwrap();
        fire_due_with(&s, "2027-03-13T14:00:02Z", &mut |_| true, &new_york).unwrap();
        assert_eq!(
            s.pending_reminders().unwrap()[0].due_at,
            "2027-03-14T13:00:00Z"
        );
    }

    #[test]
    fn a_broken_rule_ends_the_reminder_instead_of_looping() {
        let s = Store::open_in_memory().unwrap();
        s.add_reminder_repeating("2027-01-10T09:00:00Z", "x", Some("garbage"))
            .unwrap();
        assert_eq!(
            fire_due_with(&s, "2027-01-10T09:00:03Z", &mut |_| true, &london).unwrap(),
            1
        );
        assert!(s.pending_reminders().unwrap().is_empty());
    }

    #[test]
    fn utc_text_parses_for_differences() {
        let a = parse_utc("2026-11-08T12:00:00Z").unwrap();
        let b = parse_utc("2026-11-09T12:00:30Z").unwrap();
        assert_eq!(b - a, 86_400 + 30);
    }
}
