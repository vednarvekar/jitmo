//! `reminders`: set a reminder or a timer (tasks R1 and R2).
//!
//! The connector only records the reminder in the database. The app's scheduler
//! shows a Windows notification when it is due, so a reminder still fires
//! after the command bar is closed. Undo deletes the reminder.
//!
//! The action reads the user's whole request (`text`) and works out the time
//! and what to be reminded of with [`parse`].

use crate::repeat::{find_clock, find_repeat, next_after, with_clock, Repeat};
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc, Weekday};
use jitmo_store::Store;
use regex::Regex;
use std::path::PathBuf;

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub due: DateTime<Local>,
    /// What to say when it fires.
    pub what: String,
    pub timer: bool,
    pub repeat: Option<Repeat>,
}

const ASK_WHEN: &str = "When should I remind you? Try “remind me to call mom at 6pm”.";

fn dur_unit(unit: &str) -> Duration {
    match unit.chars().next() {
        Some('h') => Duration::hours(1),
        Some('s') => Duration::seconds(1),
        _ => Duration::minutes(1),
    }
}

/// "10 minutes", "an hour", "half an hour", "1 hour 30 minutes" -> total.
fn durations(text: &str) -> Option<(Duration, Vec<(usize, usize)>)> {
    let re = Regex::new(
        r"(?i)\b(half an|half a|an|a|one|\d+(?:\.\d+)?)\s*[- ]?\s*(hours?|hrs?|h|minutes?|mins?|m|seconds?|secs?)\b",
    )
    .ok()?;
    let mut total = Duration::zero();
    let mut spans = Vec::new();
    for c in re.captures_iter(text) {
        let n = c.get(1)?.as_str().to_lowercase();
        let unit = dur_unit(&c.get(2)?.as_str().to_lowercase());
        let count = match n.as_str() {
            "half an" | "half a" => 0.5,
            "an" | "a" | "one" => 1.0,
            x => x.parse::<f64>().ok()?,
        };
        total += Duration::seconds((count * unit.num_seconds() as f64).round() as i64);
        let m = c.get(0)?;
        spans.push((m.start(), m.end()));
    }
    (total > Duration::zero()).then_some((total, spans))
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" | "thur" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

fn local_at(date: chrono::NaiveDate, h: u32, m: u32) -> Option<DateTime<Local>> {
    let t = NaiveTime::from_hms_opt(h, m, 0)?;
    Local.from_local_datetime(&date.and_time(t)).earliest()
}

/// Work out when and what from a request such as "remind me to call mom at 6pm".
pub fn parse(text: &str, now: DateTime<Local>) -> Result<Parsed, String> {
    let lower = text.to_lowercase();
    let is_timer = lower.contains("timer");
    let mut cut: Vec<(usize, usize)> = Vec::new();

    if !is_timer {
        if let Some(p) = parse_repeating(text, now)? {
            return Ok(p);
        }
    }

    // Relative: "timer for 10 minutes", "in 20 minutes", "a 10-minute timer".
    if let Some((total, spans)) = durations(text) {
        let relative = is_timer || lower.contains(" in ") || lower.starts_with("in ");
        if relative {
            cut.extend(spans);
            if let Ok(re) = Regex::new(r"(?i)\b(?:in|for)\b") {
                cut.extend(re.find_iter(text).map(|m| (m.start(), m.end())));
            }
            return Ok(Parsed {
                due: now + total,
                what: if is_timer {
                    "Timer".to_string()
                } else {
                    clean(text, &cut)
                },
                timer: is_timer,
                repeat: None,
            });
        }
    }
    if is_timer {
        return Err("How long should the timer run? Try “set a 10-minute timer”.".into());
    }

    // Day words.
    let day_re = Regex::new(
        r"(?i)\b(today|tonight|tomorrow|this (?:morning|afternoon|evening)|(?:next |on )?(monday|tuesday|wednesday|thursday|friday|saturday|sunday|mon|tue|tues|wed|thu|thur|thurs|fri|sat|sun))\b",
    )
    .map_err(|e| e.to_string())?;
    let mut date = now.date_naive();
    let mut explicit_day = false;
    let mut day_default_hour: Option<u32> = None;
    if let Some(c) = day_re.captures(text) {
        let whole = c
            .get(0)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();
        let m = c.get(0).ok_or("no day")?;
        cut.push((m.start(), m.end()));
        explicit_day = true;
        if whole == "tomorrow" {
            date = date.succ_opt().ok_or("date")?;
        } else if whole == "tonight" {
            day_default_hour = Some(20);
        } else if whole.starts_with("this morning") {
            day_default_hour = Some(9);
        } else if whole.starts_with("this afternoon") {
            day_default_hour = Some(15);
        } else if whole.starts_with("this evening") {
            day_default_hour = Some(18);
        } else if let Some(wd) = c.get(2).and_then(|w| weekday(&w.as_str().to_lowercase())) {
            let mut d = date;
            loop {
                d = d.succ_opt().ok_or("date")?;
                if d.weekday() == wd {
                    break;
                }
            }
            date = d;
        }
    }
    let morning = Regex::new(r"(?i)\b(?:in the )?morning\b")
        .map_err(|e| e.to_string())?
        .find(text);
    let evening = Regex::new(r"(?i)\b(?:in the )?evening\b")
        .map_err(|e| e.to_string())?
        .find(text);

    // Clock time: "at 6pm", "6:30 pm", "at 6", "noon", "midnight".
    let clock_re = Regex::new(
        r"(?i)\b(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(a\.?m\.?|p\.?m\.?)?\b|\b(noon|midnight)\b",
    )
    .map_err(|e| e.to_string())?;
    let mut found_clock: Option<(u32, u32, Option<bool>)> = None; // hour, minute, is_pm
    for c in clock_re.captures_iter(text) {
        let whole = c.get(0).ok_or("no match")?;
        let ampm = c.get(3).map(|a| a.as_str().to_lowercase().starts_with('p'));
        let has_at = whole.as_str().to_lowercase().starts_with("at");
        if let Some(w) = c.get(4) {
            let noon = w.as_str().eq_ignore_ascii_case("noon");
            found_clock = Some((12, 0, Some(noon)));
            cut.push((whole.start(), whole.end()));
            break;
        }
        let h: u32 = c.get(1).and_then(|x| x.as_str().parse().ok()).unwrap_or(99);
        let m: u32 = c.get(2).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
        // A bare number counts only after "at" or with am/pm or a colon, so
        // "call mom 2 times" is not a time.
        if h <= 23 && m < 60 && (has_at || ampm.is_some() || c.get(2).is_some()) {
            found_clock = Some((h, m, ampm));
            cut.push((whole.start(), whole.end()));
            break;
        }
    }

    let (hour, minute, ampm) = match (found_clock, day_default_hour) {
        (Some(t), _) => t,
        (None, Some(h)) => (h, 0, Some(h >= 12)),
        (None, None) if explicit_day || morning.is_some() || evening.is_some() => {
            // "tomorrow", "tomorrow morning": a sensible default hour.
            let h = if evening.is_some() { 18 } else { 9 };
            if let Some(m) = morning.or(evening) {
                cut.push((m.start(), m.end()));
            }
            (h, 0, Some(h >= 12))
        }
        _ => return Err(ASK_WHEN.into()),
    };
    if let Some(m) = morning.or(evening) {
        cut.push((m.start(), m.end()));
    }

    // Resolve am/pm. With none given, use the next time the clock shows that hour.
    let due = match ampm {
        _ if hour > 12 || hour == 0 => local_at(date, hour % 24, minute),
        Some(pm) => {
            let h24 = match (hour % 12, pm) {
                (h, true) => h + 12,
                (h, false) => h,
            };
            local_at(date, h24, minute)
        }
        None => {
            let (am, pm) = (hour % 12, hour % 12 + 12);
            let candidates = [local_at(date, am, minute), local_at(date, pm, minute)];
            let mut best = candidates
                .iter()
                .flatten()
                .filter(|t| **t > now)
                .min()
                .copied();
            if best.is_none() {
                best = local_at(date.succ_opt().ok_or("date")?, am, minute);
            }
            best
        }
    };
    let mut due = due.ok_or("That time doesn't exist (clocks change).")?;
    if due <= now {
        if explicit_day {
            return Err("That time has already passed.".into());
        }
        due += Duration::days(1);
    }
    Ok(Parsed {
        due,
        what: clean(text, &cut),
        timer: false,
        repeat: None,
    })
}

/// "every day at 9am", "every Monday at 10", "every 2 hours": `Ok(None)` if the request does not repeat.
fn parse_repeating(text: &str, now: DateTime<Local>) -> Result<Option<Parsed>, String> {
    let Some(found) = find_repeat(text)? else {
        return Ok(None);
    };
    // Blank out the repeat words (same length, so offsets stay valid).
    let (start, end) = found.span;
    let mut masked = text.to_string();
    masked.replace_range(start..end, &" ".repeat(end - start));
    let mut cut = vec![found.span];
    let rule = match find_clock(&masked, &mut cut).or(found.preset) {
        Some((h, m)) => with_clock(found.rule, h, m),
        None => found.rule,
    };
    let now_utc = now.with_timezone(&Utc);
    let due = match rule {
        Repeat::Every(n) => now_utc + Duration::minutes(i64::from(n)),
        _ => next_after(rule, now_utc, now_utc, &Local).ok_or("That time doesn't exist.")?,
    };
    Ok(Some(Parsed {
        due: due.with_timezone(&Local),
        what: clean(text, &cut),
        timer: false,
        repeat: Some(rule),
    }))
}

/// Remove the time words and "remind me to", leaving what to remember.
fn clean(text: &str, cut: &[(usize, usize)]) -> String {
    let mut cuts = cut.to_vec();
    cuts.sort_unstable();
    let mut out = String::new();
    let mut at = 0;
    for (s, e) in cuts {
        if s >= at {
            out.push_str(&text[at..s]);
            out.push(' ');
            at = e;
        }
    }
    out.push_str(&text[at..]);
    let lead = Regex::new(r"(?i)^\s*(?:please\s+)?(?:can you\s+)?(?:remind me|reminder|set a reminder)\s*(?:to|about|abt|that|of)?\s*")
        .expect("regex");
    let s = lead.replace(&out, "").to_string();
    // "every Monday at 10 to send the report" leaves a dangling "to".
    let s = Regex::new(r"(?i)^\s*to\s+")
        .expect("regex")
        .replace(&s, "")
        .to_string();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    if s.is_empty() {
        "Reminder".to_string()
    } else {
        // Sentence case for the notification.
        let mut ch = s.chars();
        ch.next()
            .map(|f| f.to_uppercase().collect::<String>() + ch.as_str())
            .unwrap_or_default()
    }
}

fn describe_when(due: DateTime<Local>, now: DateTime<Local>) -> String {
    let day = if due.date_naive() == now.date_naive() {
        "today".to_string()
    } else if Some(due.date_naive()) == now.date_naive().succ_opt() {
        "tomorrow".to_string()
    } else {
        due.format("%A %-d %B").to_string()
    };
    format!("{} {}", due.format("%-I:%M %p"), day)
}

fn describe_length(d: Duration) -> String {
    let secs = d.num_seconds();
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h} hour{}", if h == 1 { "" } else { "s" }));
    }
    if m > 0 {
        parts.push(format!("{m} minute{}", if m == 1 { "" } else { "s" }));
    }
    if s > 0 {
        parts.push(format!("{s} second{}", if s == 1 { "" } else { "s" }));
    }
    parts.join(" ")
}

pub fn utc_text(t: DateTime<Local>) -> String {
    t.with_timezone(&Utc)
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

pub struct Reminders {
    db: PathBuf,
    key: Option<jitmo_store::Key>,
    now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
}

impl Reminders {
    /// `db` is the same database file the app uses.
    pub fn new(db: PathBuf) -> Self {
        Reminders {
            db,
            key: None,
            now: Box::new(Local::now),
        }
    }

    /// Database key, when the file is encrypted.
    pub fn with_key(mut self, key: jitmo_store::Key) -> Self {
        self.key = Some(key);
        self
    }

    pub fn with_now(mut self, now: impl Fn() -> DateTime<Local> + Send + Sync + 'static) -> Self {
        self.now = Box::new(now);
        self
    }

    fn store(&self) -> Result<Store, ConnError> {
        Store::open(&self.db, self.key.as_ref())
            .map_err(|e| ConnError::Failed(format!("I couldn't open the reminders: {e}")))
    }
}

impl Connector for Reminders {
    fn id(&self) -> &str {
        "reminders"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec {
            name: "set".into(),
            required_args: vec!["text".into()],
        }]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        if action != "set" {
            return Err(ConnError::UnknownAction(action.to_string()));
        }
        let text = args
            .get("text")
            .ok_or_else(|| ConnError::MissingArg("text".into()))?;
        let now = (self.now)();
        let p = parse(text, now).map_err(ConnError::Failed)?;
        let id = self
            .store()?
            .add_reminder_repeating(
                &utc_text(p.due),
                &p.what,
                p.repeat.map(Repeat::to_rule).as_deref(),
            )
            .map_err(|e| ConnError::Failed(format!("I couldn't save the reminder: {e}")))?;
        let message = if let Some(r) = p.repeat {
            format!(
                "Reminder set for {}: {}. Repeats {}.",
                describe_when(p.due, now),
                p.what,
                r.describe()
            )
        } else if p.timer {
            format!("Timer set for {}.", describe_length(p.due - now))
        } else {
            format!(
                "Reminder set for {}: {}.",
                describe_when(p.due, now),
                p.what
            )
        };
        Ok(Output(
            [
                ("message".to_string(), message),
                ("id".to_string(), id.to_string()),
            ]
            .into(),
        ))
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        if action != "set" {
            return Err(ConnError::UnknownAction(action.to_string()));
        }
        let text = args
            .get("text")
            .ok_or_else(|| ConnError::MissingArg("text".into()))?;
        let p = parse(text, (self.now)()).map_err(ConnError::Failed)?;
        let pending = self
            .store()?
            .pending_reminders()
            .map_err(|e| ConnError::Failed(e.to_string()))?;
        Ok(pending.iter().any(|r| r.text == p.what))
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        if action != "set" {
            return None;
        }
        let id: i64 = output.0.get("id")?.parse().ok()?;
        Some(self.store().and_then(|s| {
            s.delete_reminder(id)
                .map_err(|e| ConnError::Failed(e.to_string()))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sunday 8 Nov 2026, 14:00 local.
    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 8, 14, 0, 0).unwrap()
    }

    fn p(text: &str) -> Parsed {
        parse(text, now()).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn hm(t: DateTime<Local>) -> (u32, u32, u32) {
        use chrono::Timelike;
        (t.day(), t.hour(), t.minute())
    }

    #[test]
    fn t37_remind_me_at_6pm() {
        let r = p("Remind me to call mom at 6pm.");
        assert_eq!(hm(r.due), (8, 18, 0));
        assert_eq!(r.what, "Call mom");
        assert!(!r.timer);
    }

    #[test]
    fn t59_casual_bare_six_means_the_next_six() {
        let r = p("remind me abt mom call at 6");
        assert_eq!(hm(r.due), (8, 18, 0), "it is 2pm, so the next 6 is 6pm");
        assert_eq!(r.what, "Mom call");
    }

    #[test]
    fn a_bare_hour_already_passed_today_rolls_to_tomorrows_next() {
        // 2pm: "at 1" -> next time the clock reads 1 is 1am tomorrow.
        let r = p("remind me to stretch at 1");
        assert_eq!(hm(r.due), (9, 1, 0));
    }

    #[test]
    fn t38_ten_minute_timer() {
        let r = p("Set a 10-minute timer.");
        assert!(r.timer);
        assert_eq!(r.due, now() + Duration::minutes(10));
        assert_eq!(r.what, "Timer");
    }

    #[test]
    fn timer_forms() {
        assert_eq!(
            p("timer for 1 hour 30 minutes").due,
            now() + Duration::minutes(90)
        );
        assert_eq!(p("set a timer for an hour").due, now() + Duration::hours(1));
        assert_eq!(
            p("start a half an hour timer").due,
            now() + Duration::minutes(30)
        );
        assert_eq!(p("45 second timer").due, now() + Duration::seconds(45));
    }

    #[test]
    fn relative_reminders() {
        let r = p("remind me in 20 minutes to check the oven");
        assert_eq!(r.due, now() + Duration::minutes(20));
        assert_eq!(r.what, "Check the oven");
    }

    #[test]
    fn day_words_and_weekdays() {
        let r = p("remind me tomorrow at 9am to send the report");
        assert_eq!(hm(r.due), (9, 9, 0));
        assert_eq!(r.what, "Send the report");
        let r = p("remind me on friday at 3:30pm to book the table");
        assert_eq!(hm(r.due), (13, 15, 30));
        let r = p("remind me tomorrow morning to water the plants");
        assert_eq!(hm(r.due), (9, 9, 0));
        let r = p("remind me tonight to lock up");
        assert_eq!(hm(r.due), (8, 20, 0));
    }

    #[test]
    fn noon_and_twenty_four_hour_times() {
        assert_eq!(hm(p("remind me tomorrow at noon to eat").due), (9, 12, 0));
        assert_eq!(hm(p("remind me at 17:45 to leave").due), (8, 17, 45));
    }

    #[test]
    fn t73_no_time_asks_when() {
        let e = parse("Remind me later.", now()).unwrap_err();
        assert!(e.starts_with("When should I remind you?"), "{e}");
    }

    #[test]
    fn numbers_that_are_not_times_are_not_times() {
        assert!(parse("remind me to call mom 2 times", now()).is_err());
    }

    #[test]
    fn a_timer_with_no_length_asks_how_long() {
        assert!(parse("set a timer", now())
            .unwrap_err()
            .starts_with("How long"));
    }

    #[test]
    fn a_time_earlier_today_with_a_named_day_is_refused() {
        let e = parse("remind me today at 9am to jog", now()).unwrap_err();
        assert_eq!(e, "That time has already passed.");
    }

    // --- connector ---

    fn temp_db(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jitmo-rem-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("jitmo.db")
    }

    fn args(text: &str) -> Args {
        [("text".to_string(), text.to_string())].into()
    }

    #[test]
    fn set_saves_check_passes_and_undo_deletes() {
        let db = temp_db("set");
        let c = Reminders::new(db.clone()).with_now(now);
        let out = c
            .execute("set", &args("Remind me to call mom at 6pm."))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Reminder set for 6:00 PM today: Call mom."
        );
        assert!(c
            .check("set", &args("Remind me to call mom at 6pm."))
            .unwrap());
        let s = Store::open(&db, None).unwrap();
        assert_eq!(s.pending_reminders().unwrap().len(), 1);
        c.undo("set", &out).unwrap().unwrap();
        assert!(s.pending_reminders().unwrap().is_empty());
        assert!(!c
            .check("set", &args("Remind me to call mom at 6pm."))
            .unwrap());
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn timer_message_names_the_length() {
        let db = temp_db("timer");
        let c = Reminders::new(db.clone()).with_now(now);
        let out = c.execute("set", &args("Set a 10-minute timer.")).unwrap();
        assert_eq!(out.0["message"], "Timer set for 10 minutes.");
        let due = Store::open(&db, None).unwrap().pending_reminders().unwrap();
        assert_eq!(due[0].due_at, utc_text(now() + Duration::minutes(10)));
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn repeating_reminders_parse_with_their_rule() {
        // now(): Sunday 8 Nov 2026, 14:00 local.
        let r = p("remind me every day at 9am to take vitamins");
        assert_eq!(r.repeat, Some(Repeat::Daily(9, 0)));
        assert_eq!(r.what, "Take vitamins");
        assert_eq!(hm(r.due), (9, 9, 0), "9am today has passed: tomorrow");

        let r = p("every Monday at 10 to send the report");
        assert_eq!(r.repeat, Some(Repeat::Weekly(Weekday::Mon, 10, 0)));
        assert_eq!(r.what, "Send the report");
        assert_eq!(hm(r.due), (9, 10, 0), "Monday 9 Nov");

        let r = p("remind me every 2 hours to drink water");
        assert_eq!(r.repeat, Some(Repeat::Every(120)));
        assert_eq!(r.due, now() + Duration::hours(2));
        assert_eq!(r.what, "Drink water");

        let r = p("remind me to stand up every weekday at 8:30am");
        assert_eq!(r.repeat, Some(Repeat::Weekdays(8, 30)));
        assert_eq!(r.what, "Stand up");

        let r = p("remind me to stretch every evening");
        assert_eq!(r.repeat, Some(Repeat::Daily(18, 0)));
    }

    #[test]
    fn a_repeat_under_five_minutes_is_refused() {
        let e = parse("remind me every 3 minutes to blink", now()).unwrap_err();
        assert!(e.contains("at least 5 minutes"), "{e}");
    }

    #[test]
    fn plain_reminders_do_not_repeat() {
        assert_eq!(p("Remind me to call mom at 6pm.").repeat, None);
        assert_eq!(p("Set a 10-minute timer.").repeat, None);
    }

    #[test]
    fn repeating_set_stores_the_rule_says_so_and_undo_deletes_the_series() {
        let db = temp_db("repeat");
        let c = Reminders::new(db.clone()).with_now(now);
        let out = c
            .execute("set", &args("remind me every day at 9am to take vitamins"))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Reminder set for 9:00 AM tomorrow: Take vitamins. Repeats every day at 9:00 AM."
        );
        let s = Store::open(&db, None).unwrap();
        let pending = s.pending_reminders().unwrap();
        assert_eq!(pending[0].repeat.as_deref(), Some("daily 09:00"));
        // The scheduler re-arms by keeping the same id, so Undo removes the series.
        s.reschedule_reminder(pending[0].id, "2099-01-01T09:00:00Z")
            .unwrap();
        c.undo("set", &out).unwrap().unwrap();
        assert!(s.pending_reminders().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn nothing_is_saved_when_the_time_is_unclear() {
        let db = temp_db("unclear");
        let c = Reminders::new(db.clone()).with_now(now);
        assert!(c.execute("set", &args("Remind me later.")).is_err());
        assert!(Store::open(&db, None)
            .unwrap()
            .pending_reminders()
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }
}
