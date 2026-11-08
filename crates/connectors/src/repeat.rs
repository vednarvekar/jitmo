//! Repeat rules for reminders ("every day at 9am", "every 2 hours").
//!
//! Clock-based rules follow the wall clock in the user's own time zone, so
//! "9:00 every day" stays 9:00 across daylight saving. The rule is kept as
//! text in the reminders table (`repeat` column), for example `daily 09:00`,
//! `weekdays 09:00`, `weekly mon 10:00` or `every 120m`.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc, Weekday};
use regex::Regex;

/// Shortest allowed gap for "every N minutes".
pub const MIN_REPEAT_MINUTES: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Daily(u32, u32),
    Weekdays(u32, u32),
    Weekly(Weekday, u32, u32),
    /// Every N minutes, counted from the first time.
    Every(u32),
}

pub fn weekday(word: &str) -> Option<Weekday> {
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

fn weekday_key(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "mon",
        Weekday::Tue => "tue",
        Weekday::Wed => "wed",
        Weekday::Thu => "thu",
        Weekday::Fri => "fri",
        Weekday::Sat => "sat",
        Weekday::Sun => "sun",
    }
}

fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

fn clock_text(h: u32, m: u32) -> String {
    let (h12, suffix) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {suffix}")
}

impl Repeat {
    /// The text kept in the database `repeat` column.
    pub fn to_rule(self) -> String {
        match self {
            Repeat::Daily(h, m) => format!("daily {h:02}:{m:02}"),
            Repeat::Weekdays(h, m) => format!("weekdays {h:02}:{m:02}"),
            Repeat::Weekly(w, h, m) => format!("weekly {} {h:02}:{m:02}", weekday_key(w)),
            Repeat::Every(n) => format!("every {n}m"),
        }
    }

    /// Read a stored rule; `None` for anything unknown.
    pub fn from_rule(rule: &str) -> Option<Repeat> {
        let parts: Vec<&str> = rule.split_whitespace().collect();
        let clock = |s: &str| -> Option<(u32, u32)> {
            let (h, m) = s.split_once(':')?;
            let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
            (h < 24 && m < 60).then_some((h, m))
        };
        match parts[..] {
            ["daily", c] => clock(c).map(|(h, m)| Repeat::Daily(h, m)),
            ["weekdays", c] => clock(c).map(|(h, m)| Repeat::Weekdays(h, m)),
            ["weekly", d, c] => {
                let (h, m) = clock(c)?;
                Some(Repeat::Weekly(weekday(d)?, h, m))
            }
            ["every", n] => {
                let n: u32 = n.strip_suffix('m')?.parse().ok()?;
                (n >= MIN_REPEAT_MINUTES).then_some(Repeat::Every(n))
            }
            _ => None,
        }
    }

    /// "every day at 9:00 AM", "every 2 hours".
    pub fn describe(self) -> String {
        match self {
            Repeat::Daily(h, m) => format!("every day at {}", clock_text(h, m)),
            Repeat::Weekdays(h, m) => format!("every weekday at {}", clock_text(h, m)),
            Repeat::Weekly(w, h, m) => {
                format!("every {} at {}", weekday_name(w), clock_text(h, m))
            }
            Repeat::Every(60) => "every hour".to_string(),
            Repeat::Every(n) if n % 60 == 0 => format!("every {} hours", n / 60),
            Repeat::Every(n) => format!("every {n} minutes"),
        }
    }
}

/// A local wall-clock time on `date`. A time that does not exist (clocks
/// jumping forward) moves one hour later; a repeated time uses the first.
fn wall_clock<Tz: TimeZone>(
    tz: &Tz,
    date: chrono::NaiveDate,
    h: u32,
    m: u32,
) -> Option<DateTime<Utc>> {
    let naive = date.and_time(NaiveTime::from_hms_opt(h, m, 0)?);
    let t = tz.from_local_datetime(&naive).earliest().or_else(|| {
        tz.from_local_datetime(&(naive + Duration::hours(1)))
            .earliest()
    })?;
    Some(t.with_timezone(&Utc))
}

/// The first time of a rule strictly after `after`. `anchor` is when it was
/// last due; interval rules count from it, so they stay on their own grid.
/// Only ever returns a time in the future: a long gap gives one next time, not a burst.
pub fn next_after<Tz: TimeZone>(
    rule: Repeat,
    anchor: DateTime<Utc>,
    after: DateTime<Utc>,
    tz: &Tz,
) -> Option<DateTime<Utc>> {
    let (h, m, days): (u32, u32, Box<dyn Fn(Weekday) -> bool>) = match rule {
        Repeat::Every(n) => {
            let step = i64::from(n.max(MIN_REPEAT_MINUTES)) * 60;
            let elapsed = (after - anchor).num_seconds();
            let k = if elapsed < 0 { 1 } else { elapsed / step + 1 };
            return Some(anchor + Duration::seconds(k * step));
        }
        Repeat::Daily(h, m) => (h, m, Box::new(|_| true)),
        Repeat::Weekdays(h, m) => (
            h,
            m,
            Box::new(|d| !matches!(d, Weekday::Sat | Weekday::Sun)),
        ),
        Repeat::Weekly(w, h, m) => (h, m, Box::new(move |d| d == w)),
    };
    let first = after.with_timezone(tz).date_naive();
    (0..=8)
        .filter_map(|i| first.checked_add_signed(Duration::days(i)))
        .filter(|d| days(d.weekday()))
        .filter_map(|d| wall_clock(tz, d, h, m))
        .find(|t| *t > after)
}

const UTC_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

fn parse_utc_text(s: &str) -> Option<DateTime<Utc>> {
    chrono::NaiveDateTime::parse_from_str(s, UTC_FORMAT)
        .ok()
        .map(|n| n.and_utc())
}

/// Next time of a stored rule after `now_utc`, as UTC text, using the named
/// IANA zone. `due_at` is when the reminder last came due.
pub fn next_due_in_zone(rule: &str, due_at: &str, now_utc: &str, zone: &str) -> Option<String> {
    let tz: chrono_tz::Tz = zone.parse().ok()?;
    let next = next_after(
        Repeat::from_rule(rule)?,
        parse_utc_text(due_at)?,
        parse_utc_text(now_utc)?,
        &tz,
    )?;
    Some(next.format(UTC_FORMAT).to_string())
}

/// Like [`next_due_in_zone`], in this computer's own time zone.
pub fn next_due_local(rule: &str, due_at: &str, now_utc: &str) -> Option<String> {
    let next = next_after(
        Repeat::from_rule(rule)?,
        parse_utc_text(due_at)?,
        parse_utc_text(now_utc)?,
        &Local,
    )?;
    Some(next.format(UTC_FORMAT).to_string())
}

/// A repeat found in a request: the rule (clock possibly still default), a
/// preset time of day ("every morning"), and the span of text it came from.
pub struct Found {
    pub rule: Repeat,
    pub preset: Option<(u32, u32)>,
    pub span: (usize, usize),
}

/// Find "every day", "every Monday", "every 2 hours", ... in a request.
/// `Ok(None)` when the request does not repeat; `Err` when it is not allowed.
pub fn find_repeat(text: &str) -> Result<Option<Found>, String> {
    let lower = text.to_lowercase();
    let span = |m: regex::Match| (m.start(), m.end());
    let re = |p: &str| Regex::new(p).map_err(|e| e.to_string());

    // Intervals first: "every 2 hours", "every hour", "every half hour".
    let interval =
        re(r"\bevery\s+(?:(half an?|an?|one|\d+(?:\.\d+)?)\s*)?(hours?|hrs?|minutes?|mins?)\b")?;
    if let Some(c) = interval.captures(&lower) {
        let n = c.get(1).map_or("1", |g| g.as_str());
        let count = match n {
            "half a" | "half an" => 0.5,
            "a" | "an" | "one" => 1.0,
            x => x.parse::<f64>().map_err(|_| "That repeat is unclear.")?,
        };
        let per = if c[2].starts_with('h') { 60.0 } else { 1.0 };
        let minutes = (count * per).round();
        if minutes < f64::from(MIN_REPEAT_MINUTES) {
            return Err(format!(
                "Repeats need at least {MIN_REPEAT_MINUTES} minutes between them."
            ));
        }
        if minutes > 100_000.0 {
            return Err("That repeat is too long. Try every day or every week.".into());
        }
        let whole = c.get(0).ok_or("no match")?;
        return Ok(Some(Found {
            rule: Repeat::Every(minutes as u32),
            preset: None,
            span: span(whole),
        }));
    }
    let weekdays = re(r"\bevery\s+week\s?days?\b|\bon\s+week\s?days\b|\bweekdays\b")?;
    if let Some(m) = weekdays.find(&lower) {
        return Ok(Some(Found {
            rule: Repeat::Weekdays(9, 0),
            preset: None,
            span: span(m),
        }));
    }
    let names = "monday|tuesday|wednesday|thursday|friday|saturday|sunday|mon|tues|tue|wed|thurs|thur|thu|fri|sat|sun";
    let weekly = re(&format!(r"\b(?:every|each)\s+({names})\b"))?;
    if let Some(c) = weekly.captures(&lower) {
        let wd = weekday(&c[1]).ok_or("That repeat is unclear.")?;
        let whole = c.get(0).ok_or("no match")?;
        return Ok(Some(Found {
            rule: Repeat::Weekly(wd, 9, 0),
            preset: None,
            span: span(whole),
        }));
    }
    let part = re(r"\bevery\s+(morning|afternoon|evening|night)\b")?;
    if let Some(c) = part.captures(&lower) {
        let h = match &c[1] {
            "morning" => 9,
            "afternoon" => 15,
            "evening" => 18,
            _ => 21,
        };
        let whole = c.get(0).ok_or("no match")?;
        return Ok(Some(Found {
            rule: Repeat::Daily(h, 0),
            preset: Some((h, 0)),
            span: span(whole),
        }));
    }
    let daily = re(r"\b(?:every\s*day|each\s+day|daily)\b")?;
    if let Some(m) = daily.find(&lower) {
        return Ok(Some(Found {
            rule: Repeat::Daily(9, 0),
            preset: None,
            span: span(m),
        }));
    }
    Ok(None)
}

/// Put a time of day into a rule.
pub fn with_clock(rule: Repeat, h: u32, m: u32) -> Repeat {
    match rule {
        Repeat::Daily(..) => Repeat::Daily(h, m),
        Repeat::Weekdays(..) => Repeat::Weekdays(h, m),
        Repeat::Weekly(w, ..) => Repeat::Weekly(w, h, m),
        every => every,
    }
}

/// Time of day for a repeating reminder from the rest of the sentence.
/// Without am/pm, 1 to 6 means the afternoon and 7 to 11 the morning.
/// Pushes the span it used onto `cut`.
pub fn find_clock(rest: &str, cut: &mut Vec<(usize, usize)>) -> Option<(u32, u32)> {
    let re = Regex::new(
        r"(?i)\b(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(a\.?m\.?|p\.?m\.?)?\b|\b(noon|midnight)\b",
    )
    .ok()?;
    for c in re.captures_iter(rest) {
        let whole = c.get(0)?;
        if let Some(w) = c.get(4) {
            cut.push((whole.start(), whole.end()));
            let noon = w.as_str().eq_ignore_ascii_case("noon");
            return Some((if noon { 12 } else { 0 }, 0));
        }
        let h: u32 = c.get(1)?.as_str().parse().ok()?;
        let m: u32 = c.get(2).map_or(Some(0), |x| x.as_str().parse().ok())?;
        let ampm = c.get(3).map(|a| a.as_str().to_lowercase().starts_with('p'));
        let has_at = whole.as_str().to_lowercase().starts_with("at");
        if h > 23 || m > 59 || !(has_at || ampm.is_some() || c.get(2).is_some()) {
            continue;
        }
        let h24 = match ampm {
            _ if h > 12 || h == 0 => h % 24,
            Some(pm) => h % 12 + if pm { 12 } else { 0 },
            None if (1..=6).contains(&h) => h + 12,
            None => h,
        };
        cut.push((whole.start(), whole.end()));
        return Some((h24, m));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Tz;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    const LONDON: Tz = chrono_tz::Europe::London;
    const NY: Tz = chrono_tz::America::New_York;

    #[test]
    fn rules_round_trip_and_describe() {
        for r in [
            Repeat::Daily(9, 0),
            Repeat::Weekdays(18, 30),
            Repeat::Weekly(Weekday::Mon, 10, 0),
            Repeat::Every(120),
        ] {
            assert_eq!(Repeat::from_rule(&r.to_rule()), Some(r));
        }
        assert_eq!(Repeat::Daily(9, 0).describe(), "every day at 9:00 AM");
        assert_eq!(
            Repeat::Weekly(Weekday::Mon, 10, 0).describe(),
            "every Monday at 10:00 AM"
        );
        assert_eq!(Repeat::Every(120).describe(), "every 2 hours");
        assert_eq!(Repeat::Every(60).describe(), "every hour");
        assert_eq!(Repeat::Every(45).describe(), "every 45 minutes");
        assert_eq!(Repeat::from_rule("every 3m"), None);
        assert_eq!(Repeat::from_rule("daily 25:00"), None);
        assert_eq!(Repeat::from_rule("junk"), None);
    }

    #[test]
    fn daily_next_is_the_next_nine() {
        let rule = Repeat::Daily(9, 0);
        // January: London is UTC.
        let a = utc(2027, 1, 10, 8, 0);
        assert_eq!(
            next_after(rule, a, a, &LONDON),
            Some(utc(2027, 1, 10, 9, 0))
        );
        let b = utc(2027, 1, 10, 9, 0);
        assert_eq!(
            next_after(rule, b, b, &LONDON),
            Some(utc(2027, 1, 11, 9, 0))
        );
    }

    #[test]
    fn daily_nine_stays_nine_across_spring_forward() {
        // US clocks go forward on Sunday 14 March 2027, 2:00 to 3:00.
        let rule = Repeat::Daily(9, 0);
        let before = utc(2027, 3, 13, 14, 0); // 9:00 EST on Saturday
        let next = next_after(rule, before, before, &NY).unwrap();
        assert_eq!(next, utc(2027, 3, 14, 13, 0), "9:00 EDT is 13:00 UTC");
    }

    #[test]
    fn daily_nine_stays_nine_across_fall_back() {
        // US clocks go back on Sunday 7 November 2027.
        let rule = Repeat::Daily(9, 0);
        let before = utc(2027, 11, 6, 13, 0); // 9:00 EDT
        let next = next_after(rule, before, before, &NY).unwrap();
        assert_eq!(next, utc(2027, 11, 7, 14, 0), "9:00 EST is 14:00 UTC");
    }

    #[test]
    fn a_time_inside_the_spring_gap_moves_an_hour_later() {
        // 2:30 does not exist in New York on 14 March 2027; use 3:30 EDT.
        let rule = Repeat::Daily(2, 30);
        let after = utc(2027, 3, 14, 5, 0);
        assert_eq!(
            next_after(rule, after, after, &NY),
            Some(utc(2027, 3, 14, 7, 30))
        );
    }

    #[test]
    fn a_repeated_hour_in_the_fall_uses_the_first_one() {
        // 1:30 happens twice on 7 Nov 2027 in New York; first is EDT (05:30 UTC).
        let rule = Repeat::Daily(1, 30);
        let after = utc(2027, 11, 7, 0, 0);
        assert_eq!(
            next_after(rule, after, after, &NY),
            Some(utc(2027, 11, 7, 5, 30))
        );
    }

    #[test]
    fn weekly_picks_the_named_weekday() {
        // 2027-01-10 is a Sunday; the next Monday is the 11th.
        let rule = Repeat::Weekly(Weekday::Mon, 10, 0);
        let a = utc(2027, 1, 10, 12, 0);
        assert_eq!(
            next_after(rule, a, a, &LONDON),
            Some(utc(2027, 1, 11, 10, 0))
        );
        // Right after firing on Monday it is the following Monday.
        let m = utc(2027, 1, 11, 10, 0);
        assert_eq!(
            next_after(rule, m, m, &LONDON),
            Some(utc(2027, 1, 18, 10, 0))
        );
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let rule = Repeat::Weekdays(9, 0);
        // Friday 2027-01-08 at 9:00 -> Monday the 11th.
        let fri = utc(2027, 1, 8, 9, 0);
        assert_eq!(
            next_after(rule, fri, fri, &LONDON),
            Some(utc(2027, 1, 11, 9, 0))
        );
    }

    #[test]
    fn intervals_stay_on_their_grid_and_never_burst() {
        let rule = Repeat::Every(120);
        let anchor = utc(2027, 1, 10, 8, 0);
        // Three days later: one next time on the 2-hour grid, in the future.
        let now = utc(2027, 1, 13, 9, 30);
        let next = next_after(rule, anchor, now, &LONDON).unwrap();
        assert_eq!(next, utc(2027, 1, 13, 10, 0));
        assert!(next > now);
    }

    #[test]
    fn closed_for_many_days_gives_one_future_time() {
        let rule = Repeat::Daily(9, 0);
        let anchor = utc(2027, 1, 1, 9, 0);
        let now = utc(2027, 1, 20, 15, 0);
        assert_eq!(
            next_after(rule, anchor, now, &LONDON),
            Some(utc(2027, 1, 21, 9, 0))
        );
    }

    #[test]
    fn text_api_uses_the_named_zone() {
        assert_eq!(
            next_due_in_zone(
                "daily 09:00",
                "2027-03-13T14:00:00Z",
                "2027-03-13T14:00:05Z",
                "America/New_York"
            )
            .as_deref(),
            Some("2027-03-14T13:00:00Z")
        );
        assert_eq!(next_due_in_zone("junk", "x", "y", "UTC"), None);
    }

    #[test]
    fn finds_the_repeat_words() {
        let f = find_repeat("remind me every day at 9am to take vitamins")
            .unwrap()
            .unwrap();
        assert_eq!(f.rule, Repeat::Daily(9, 0));
        let f = find_repeat("every Monday at 10 to send the report")
            .unwrap()
            .unwrap();
        assert!(matches!(f.rule, Repeat::Weekly(Weekday::Mon, ..)));
        let f = find_repeat("remind me every 2 hours to drink water")
            .unwrap()
            .unwrap();
        assert_eq!(f.rule, Repeat::Every(120));
        let f = find_repeat("remind me on weekdays at 8 to stand up")
            .unwrap()
            .unwrap();
        assert!(matches!(f.rule, Repeat::Weekdays(..)));
        assert!(find_repeat("remind me at 6pm to call mom")
            .unwrap()
            .is_none());
    }

    #[test]
    fn under_five_minutes_is_refused() {
        let e = find_repeat("remind me every 2 minutes to blink")
            .err()
            .unwrap();
        assert!(e.contains("at least 5 minutes"), "{e}");
        assert!(find_repeat("remind me every 5 minutes to blink").is_ok());
    }

    #[test]
    fn clock_guesses() {
        let mut cut = Vec::new();
        assert_eq!(find_clock(" at 9am ", &mut cut), Some((9, 0)));
        assert_eq!(find_clock(" at 10 ", &mut cut), Some((10, 0)));
        assert_eq!(find_clock(" at 6 ", &mut cut), Some((18, 0)));
        assert_eq!(find_clock(" at 7:30 pm ", &mut cut), Some((19, 30)));
        assert_eq!(find_clock(" noon ", &mut cut), Some((12, 0)));
        assert_eq!(find_clock(" call mom 2 times ", &mut cut), None);
    }
}
