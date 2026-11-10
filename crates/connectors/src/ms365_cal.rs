//! Microsoft 365 calendar (Phase 3, Track A): the `outlook_calendar` connector.
//!
//! Same shape as [`crate::ms365`]: a [`CalendarBackend`] trait, a seeded [`FakeCalendar`]
//! for the tests, and Graph later. Reading the agenda and finding a free time are plain code
//! (no model); sending an invite or declining one is a skill with a confirm (SR4), and every
//! change can be undone except an answer already sent.

use crate::ms365::{arg, out, person_in, ready};
use crate::reminders::{durations, weekday};
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use std::sync::{Arc, Mutex};

const FMT: &str = "%Y-%m-%d %H:%M";
/// Working hours for finding a free time.
const DAY_START: u32 = 9;
const DAY_END: u32 = 18;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub attendees: Vec<String>,
    /// An invitation the user has not answered yet.
    pub invite_pending: bool,
    pub declined: bool,
}

pub trait CalendarBackend: Send + Sync {
    fn all(&self) -> Result<Vec<Event>, ConnError>;
    fn add(
        &self,
        title: &str,
        start: NaiveDateTime,
        end: NaiveDateTime,
        attendees: &[String],
    ) -> Result<String, ConnError>;
    fn remove(&self, id: &str) -> Result<(), ConnError>;
    fn reschedule(
        &self,
        id: &str,
        start: NaiveDateTime,
        end: NaiveDateTime,
    ) -> Result<(), ConnError>;
    fn set_declined(&self, id: &str, declined: bool) -> Result<(), ConnError>;
    /// Add a person to an existing event (and send them the invite).
    fn add_attendee(&self, id: &str, who: &str) -> Result<(), ConnError>;
    fn remove_attendee(&self, id: &str, who: &str) -> Result<(), ConnError>;
    /// When somebody else is busy (their free/busy, which Graph shares for colleagues).
    fn busy_of(&self, person: &str) -> Result<Vec<(NaiveDateTime, NaiveDateTime)>, ConnError>;
    /// The people whose calendars can be compared.
    fn people(&self) -> Result<Vec<String>, ConnError>;
}

fn dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, FMT).unwrap_or_default()
}

pub struct FakeCalendar {
    events: Mutex<Vec<Event>>,
    next: Mutex<u32>,
}

fn ev(id: &str, title: &str, start: &str, end: &str, attendees: &[&str], pending: bool) -> Event {
    Event {
        id: id.into(),
        title: title.into(),
        start: dt(start),
        end: dt(end),
        attendees: attendees.iter().map(|s| s.to_string()).collect(),
        invite_pending: pending,
        declined: false,
    }
}

/// The week of Monday 9 November 2026. Today holds three meetings.
fn seeded_events() -> Vec<Event> {
    vec![
        ev(
            "e1",
            "Standup",
            "2026-11-09 09:30",
            "2026-11-09 10:00",
            &[],
            false,
        ),
        ev(
            "e2",
            "Project review",
            "2026-11-09 15:00",
            "2026-11-09 16:00",
            &["Rahul Sharma"],
            false,
        ),
        ev(
            "e3",
            "Team sync",
            "2026-11-09 17:00",
            "2026-11-09 17:30",
            &[],
            false,
        ),
        ev(
            "e4",
            "Dentist checkup",
            "2026-11-10 11:00",
            "2026-11-10 12:00",
            &[],
            false,
        ),
        ev(
            "e5",
            "Budget sync",
            "2026-11-12 11:00",
            "2026-11-12 12:00",
            &["Priya Nair"],
            true,
        ),
        ev(
            "e6",
            "Friday review",
            "2026-11-13 14:00",
            "2026-11-13 15:00",
            &["Priya Nair"],
            true,
        ),
    ]
}

impl FakeCalendar {
    pub fn seeded() -> FakeCalendar {
        FakeCalendar {
            events: Mutex::new(seeded_events()),
            next: Mutex::new(100),
        }
    }

    pub fn reset(&self) {
        if let Ok(mut g) = self.events.lock() {
            *g = seeded_events();
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<Event>>, ConnError> {
        self.events
            .lock()
            .map_err(|_| ConnError::Failed("The test calendar is broken.".into()))
    }
}

impl CalendarBackend for FakeCalendar {
    fn all(&self) -> Result<Vec<Event>, ConnError> {
        Ok(self.lock()?.clone())
    }

    fn add(
        &self,
        title: &str,
        start: NaiveDateTime,
        end: NaiveDateTime,
        attendees: &[String],
    ) -> Result<String, ConnError> {
        let mut n = self
            .next
            .lock()
            .map_err(|_| ConnError::Failed("The test calendar is broken.".into()))?;
        *n += 1;
        let id = format!("e{n}");
        self.lock()?.push(Event {
            id: id.clone(),
            title: title.into(),
            start,
            end,
            attendees: attendees.to_vec(),
            invite_pending: false,
            declined: false,
        });
        Ok(id)
    }

    fn remove(&self, id: &str) -> Result<(), ConnError> {
        self.lock()?.retain(|e| e.id != id);
        Ok(())
    }

    fn reschedule(
        &self,
        id: &str,
        start: NaiveDateTime,
        end: NaiveDateTime,
    ) -> Result<(), ConnError> {
        let mut g = self.lock()?;
        let e = g
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| ConnError::Failed("I can't find that meeting.".into()))?;
        e.start = start;
        e.end = end;
        Ok(())
    }

    fn set_declined(&self, id: &str, declined: bool) -> Result<(), ConnError> {
        let mut g = self.lock()?;
        let e = g
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| ConnError::Failed("I can't find that invite.".into()))?;
        e.declined = declined;
        e.invite_pending = !declined;
        Ok(())
    }

    fn add_attendee(&self, id: &str, who: &str) -> Result<(), ConnError> {
        let mut g = self.lock()?;
        let e = g
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| ConnError::Failed("I can't find that meeting.".into()))?;
        if !e.attendees.iter().any(|a| a == who) {
            e.attendees.push(who.to_string());
        }
        Ok(())
    }

    fn remove_attendee(&self, id: &str, who: &str) -> Result<(), ConnError> {
        let mut g = self.lock()?;
        if let Some(e) = g.iter_mut().find(|e| e.id == id) {
            e.attendees.retain(|a| a != who);
        }
        Ok(())
    }

    fn busy_of(&self, person: &str) -> Result<Vec<(NaiveDateTime, NaiveDateTime)>, ConnError> {
        let p = person.to_lowercase();
        Ok(match p.as_str() {
            // Rahul: out Monday afternoon (the project review), busy Tuesday afternoon and
            // Thursday morning.
            "rahul" | "rahul sharma" => vec![
                (dt("2026-11-10 13:00"), dt("2026-11-10 17:00")),
                (dt("2026-11-12 09:00"), dt("2026-11-12 12:00")),
            ],
            "priya" | "priya nair" => vec![(dt("2026-11-11 09:00"), dt("2026-11-11 18:00"))],
            _ => Vec::new(),
        })
    }

    fn people(&self) -> Result<Vec<String>, ConnError> {
        Ok(vec![
            "Rahul Sharma".into(),
            "Priya Nair".into(),
            "Neha Joshi".into(),
        ])
    }
}

// ---------------------------------------------------------------------------
// Reading dates out of a request
// ---------------------------------------------------------------------------

/// The day a request names: "today", "tomorrow", a weekday ("on Thursday", "thurs"). The
/// next such weekday after today, since a past day is never what is meant.
pub(crate) fn day_in(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    for w in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        match w {
            "today" | "tonight" => return Some(today),
            "tomorrow" | "tmrw" => return today.succ_opt(),
            _ => {
                if let Some(wd) = weekday(w) {
                    let mut d = today;
                    for _ in 0..7 {
                        d = d.succ_opt()?;
                        if d.weekday() == wd {
                            return Some(d);
                        }
                    }
                }
            }
        }
    }
    None
}

/// "morning", "afternoon", "evening" as an hour range.
fn part_of_day(text: &str) -> Option<(u32, u32)> {
    let l = text.to_lowercase();
    if l.contains("morning") {
        Some((6, 12))
    } else if l.contains("afternoon") {
        Some((12, 18))
    } else if l.contains("evening") {
        Some((17, 23))
    } else {
        None
    }
}

/// The clock times written like "3pm" or "10:30am" in a request, in order, as (hour, minute).
pub(crate) fn clock_times(text: &str) -> Vec<(u32, u32)> {
    let lower = text.to_lowercase();
    let bytes: Vec<char> = lower.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_alphanumeric()) {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let h: u32 = bytes[i..j].iter().collect::<String>().parse().unwrap_or(99);
            let mut m = 0;
            let mut k = j;
            if k < bytes.len() && bytes[k] == ':' {
                let s = k + 1;
                let mut e = s;
                while e < bytes.len() && bytes[e].is_ascii_digit() {
                    e += 1;
                }
                m = bytes[s..e].iter().collect::<String>().parse().unwrap_or(99);
                k = e;
            }
            while k < bytes.len() && bytes[k] == ' ' {
                k += 1;
            }
            let rest: String = bytes[k..].iter().take(2).collect();
            if (rest == "am" || rest == "pm") && (1..=12).contains(&h) && m < 60 {
                let h24 = match (h % 12, rest == "pm") {
                    (x, true) => x + 12,
                    (x, false) => x,
                };
                out.push((h24, m));
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

fn at(date: NaiveDate, h: u32, m: u32) -> NaiveDateTime {
    date.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default())
}

pub(crate) fn short(t: NaiveDateTime) -> String {
    t.format("%H:%M").to_string()
}

pub(crate) fn day_name(d: NaiveDate) -> String {
    d.format("%a %-d %b").to_string()
}

/// The free stretches of one day (within working hours) that nobody is busy in.
fn free_between(
    day: NaiveDate,
    busy: &[(NaiveDateTime, NaiveDateTime)],
    min: Duration,
) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let (start, end) = (at(day, DAY_START, 0), at(day, DAY_END, 0));
    let mut spans: Vec<_> = busy
        .iter()
        .filter(|(s, e)| *e > start && *s < end)
        .map(|(s, e)| ((*s).max(start), (*e).min(end)))
        .collect();
    spans.sort();
    let mut free = Vec::new();
    let mut cursor = start;
    for (s, e) in spans {
        if s - cursor >= min {
            free.push((cursor, s));
        }
        cursor = cursor.max(e);
    }
    if end - cursor >= min {
        free.push((cursor, end));
    }
    free
}

pub struct OutlookCalendar {
    backend: Arc<dyn CalendarBackend>,
    health: Box<dyn Fn() -> Health + Send + Sync>,
    now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
}

impl OutlookCalendar {
    pub fn new(
        backend: Arc<dyn CalendarBackend>,
        health: Box<dyn Fn() -> Health + Send + Sync>,
        now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
    ) -> OutlookCalendar {
        OutlookCalendar {
            backend,
            health,
            now,
        }
    }

    pub fn fake(calendar: Arc<FakeCalendar>) -> OutlookCalendar {
        OutlookCalendar::new(
            calendar,
            Box::new(|| Health::Ready),
            Box::new(crate::ms365::fixed_now),
        )
    }

    fn today(&self) -> NaiveDate {
        (self.now)().date_naive()
    }

    fn my_busy(&self) -> Result<Vec<(NaiveDateTime, NaiveDateTime)>, ConnError> {
        Ok(self
            .backend
            .all()?
            .into_iter()
            .filter(|e| !e.declined)
            .map(|e| (e.start, e.end))
            .collect())
    }

    /// The first stretch of `len` on `day` that I and `with` are both free in.
    fn first_free(
        &self,
        day: NaiveDate,
        len: Duration,
        with: Option<&str>,
        window: Option<(u32, u32)>,
    ) -> Result<Option<NaiveDateTime>, ConnError> {
        let mut busy = self.my_busy()?;
        if let Some(p) = with {
            busy.extend(self.backend.busy_of(p)?);
        }
        let (wf, wt) = window.unwrap_or((DAY_START, DAY_END));
        for (s, e) in free_between(day, &busy, len) {
            let from = s.max(at(day, wf, 0));
            if e - from >= len && from + len <= at(day, wt.max(wf), 0).max(from + len) {
                return Ok(Some(from));
            }
        }
        Ok(None)
    }

    fn find_by_hour(&self, text: &str, day: NaiveDate) -> Result<Event, ConnError> {
        let times = clock_times(text);
        let todays: Vec<Event> = self
            .backend
            .all()?
            .into_iter()
            .filter(|e| e.start.date() == day && !e.declined)
            .collect();
        let hit: Vec<&Event> = match times.first() {
            Some(&(h, m)) => todays
                .iter()
                .filter(|e| e.start.time() == NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default())
                .collect(),
            None => todays.iter().collect(),
        };
        match hit.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(ConnError::Failed("I can't find that meeting.".into())),
            _ => Err(ConnError::Failed(
                "Which meeting, and to when? Try “move my 3pm to tomorrow”.".into(),
            )),
        }
    }
}

fn title_from(request: &str) -> String {
    let mut words: Vec<&str> = request.split_whitespace().collect();
    while words.first().is_some_and(|w| {
        matches!(
            w.to_lowercase().as_str(),
            "add"
                | "schedule"
                | "book"
                | "put"
                | "create"
                | "set"
                | "up"
                | "a"
                | "an"
                | "my"
                | "to"
                | "the"
                | "calendar"
        )
    }) {
        words.remove(0);
    }
    let t = words.join(" ");
    let mut c = t.chars();
    match c.next() {
        Some(f) => format!("{}{}", f.to_uppercase(), c.as_str()),
        None => "Meeting".into(),
    }
}

impl Connector for OutlookCalendar {
    fn id(&self) -> &str {
        "outlook_calendar"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        let a = |n: &str, r: &[&str]| ActionSpec {
            name: n.into(),
            required_args: r.iter().map(|s| s.to_string()).collect(),
        };
        vec![
            a("agenda", &["text"]),
            a("free", &["text"]),
            a("add", &["text"]),
            a("block", &["text"]),
            a("invite", &["to", "text"]),
            a("move", &["text"]),
            a("decline", &["text"]),
            a("context", &["text"]),
        ]
    }

    fn health(&self) -> Health {
        (self.health)()
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        ready(&*self.health, "Outlook calendar")?;
        let text = arg(args, "text").unwrap_or("");
        let today = self.today();
        match action {
            "agenda" => {
                let day = day_in(text, today).unwrap_or(today);
                let part = part_of_day(text);
                let mut events: Vec<Event> = self
                    .backend
                    .all()?
                    .into_iter()
                    .filter(|e| e.start.date() == day && !e.declined)
                    .filter(|e| part.is_none_or(|(f, t)| e.start.time().hour_in(f, t)))
                    .collect();
                events.sort_by_key(|e| e.start);
                let label = if day == today {
                    "today".to_string()
                } else {
                    day_name(day)
                };
                let message = if events.is_empty() {
                    format!("Nothing on your calendar {label}.")
                } else {
                    format!(
                        "{label}: {}.",
                        events
                            .iter()
                            .map(|e| format!("{} {}", short(e.start), e.title))
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                };
                Ok(out(&[
                    ("message", message),
                    ("count", events.len().to_string()),
                ]))
            }
            "free" => {
                let people = self.backend.people()?;
                let with = person_in(text, &people);
                let mut slots = Vec::new();
                // This week: the rest of Monday to Friday, from today on.
                let mut day = today;
                while day.weekday() != Weekday::Sat && slots.len() < 3 {
                    let mut busy = self.my_busy()?;
                    if let Some(p) = &with {
                        busy.extend(self.backend.busy_of(p)?);
                    }
                    let from_now = if day == today {
                        (self.now)().naive_local()
                    } else {
                        at(day, 0, 0)
                    };
                    for (s, e) in free_between(day, &busy, Duration::hours(1)) {
                        let s = s.max(from_now);
                        if e - s >= Duration::hours(1) {
                            slots.push(format!("{} {}–{}", day_name(day), short(s), short(e)));
                        }
                    }
                    day = day.succ_opt().unwrap_or(day);
                    if day.weekday() == Weekday::Sun {
                        break;
                    }
                }
                slots.truncate(3);
                let who = with
                    .as_deref()
                    .map_or("you are".to_string(), |p| format!("you and {p} are both"));
                let message = if slots.is_empty() {
                    "I found no shared free time this week.".to_string()
                } else {
                    format!("When {who} free: {}.", slots.join("; "))
                };
                Ok(out(&[
                    ("message", message),
                    ("count", slots.len().to_string()),
                ]))
            }
            "add" => {
                let due = crate::reminders::parse(&format!("remind me {text}"), (self.now)())
                    .map_err(ConnError::Failed)?;
                let start = due.due.naive_local();
                let title = title_from(&due.what);
                let id = self
                    .backend
                    .add(&title, start, start + Duration::hours(1), &[])?;
                Ok(out(&[
                    (
                        "message",
                        format!(
                            "Added “{title}” on {} at {}.",
                            day_name(start.date()),
                            short(start)
                        ),
                    ),
                    ("id", id),
                    ("title", title),
                    ("start", start.format(FMT).to_string()),
                ]))
            }
            "block" => {
                let digits = crate::local_calc::number_words_to_digits(text);
                let (len, _) = durations(&digits).ok_or_else(|| {
                    ConnError::Failed(
                        "For how long? Try “block two hours tomorrow morning”.".into(),
                    )
                })?;
                let day = day_in(text, today).unwrap_or(today);
                let window = part_of_day(text);
                let len = Duration::seconds(len.num_seconds());
                let start = self.first_free(day, len, None, window)?.ok_or_else(|| {
                    ConnError::Failed("There is no free stretch that long then.".into())
                })?;
                let id = self.backend.add("Focus time", start, start + len, &[])?;
                Ok(out(&[
                    (
                        "message",
                        format!(
                            "Blocked {}–{} on {} for focus.",
                            short(start),
                            short(start + len),
                            day_name(day)
                        ),
                    ),
                    ("id", id),
                    ("title", "Focus time".into()),
                    ("start", start.format(FMT).to_string()),
                ]))
            }
            "invite" => {
                let who = arg(args, "to")?;
                let digits = crate::local_calc::number_words_to_digits(text);
                let (len, _) = durations(&digits).unwrap_or((Duration::minutes(30), vec![]));
                let len = Duration::seconds(len.num_seconds());
                // "invite Priya to the project review": add her to a meeting that exists.
                if day_in(text, today).is_none() {
                    let words: Vec<String> = text
                        .to_lowercase()
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| w.len() > 3)
                        .map(str::to_string)
                        .collect();
                    let mut hits: Vec<Event> = self
                        .backend
                        .all()?
                        .into_iter()
                        .filter(|e| e.start.date() >= today && !e.declined)
                        .filter(|e| {
                            e.title
                                .to_lowercase()
                                .split_whitespace()
                                .any(|t| t.len() > 3 && words.iter().any(|w| w == t))
                        })
                        .collect();
                    // The newest one: the meeting that was just made.
                    // The best title match, and of those the latest (a meeting just made).
                    let score = |e: &Event| {
                        e.title
                            .to_lowercase()
                            .split_whitespace()
                            .filter(|t| t.len() > 3 && words.iter().any(|w| w == t))
                            .count()
                    };
                    hits.sort_by_key(|e| std::cmp::Reverse((score(e), e.start)));
                    if let Some(e) = hits.first() {
                        self.backend.add_attendee(&e.id, who)?;
                        return Ok(out(&[
                            (
                                "message",
                                format!(
                                    "Invited {who} to “{}” on {} at {}.",
                                    e.title,
                                    day_name(e.start.date()),
                                    short(e.start)
                                ),
                            ),
                            ("attendee_of", e.id.clone()),
                            ("who", who.to_string()),
                        ]));
                    }
                }
                let day = day_in(text, today).ok_or_else(|| {
                    ConnError::Failed(
                        "Which day? Try “set up 30 minutes with Rahul on Thursday”.".into(),
                    )
                })?;
                let first = who.split_whitespace().next().unwrap_or(who);
                let start = self
                    .first_free(day, len, Some(first), None)?
                    .ok_or_else(|| {
                        ConnError::Failed(format!(
                            "You and {who} have no shared free time that day."
                        ))
                    })?;
                let id = self.backend.add(
                    &format!("Meeting with {who}"),
                    start,
                    start + len,
                    &[who.to_string()],
                )?;
                Ok(out(&[
                    (
                        "message",
                        format!("Invited {who} for {} at {}.", day_name(day), short(start)),
                    ),
                    ("id", id),
                    ("title", format!("Meeting with {who}")),
                    ("start", start.format(FMT).to_string()),
                ]))
            }
            "move" => {
                let times = clock_times(text);
                let day = today;
                let e = self.find_by_hour(text, day)?;
                // "to tomorrow, same time", or "to Friday at 4pm": the target day, and the new
                // time when one is given after the first.
                let to_day = day_in(&text.to_lowercase().replace("my ", ""), today)
                    .filter(|d| *d != today)
                    .ok_or_else(|| {
                        ConnError::Failed("To when? Try “move my 3pm to tomorrow”.".into())
                    })?;
                let new_time = times.get(1).map_or(e.start.time(), |&(h, m)| {
                    NaiveTime::from_hms_opt(h, m, 0).unwrap_or(e.start.time())
                });
                let len = e.end - e.start;
                let new_start = to_day.and_time(new_time);
                self.backend.reschedule(&e.id, new_start, new_start + len)?;
                Ok(out(&[
                    (
                        "message",
                        format!(
                            "Moved “{}” to {} at {}.",
                            e.title,
                            day_name(to_day),
                            short(new_start)
                        ),
                    ),
                    ("id", e.id.clone()),
                    ("old_start", e.start.format(FMT).to_string()),
                    ("old_end", e.end.format(FMT).to_string()),
                ]))
            }
            "decline" => {
                let day = day_in(text, today);
                let words: Vec<String> = text
                    .to_lowercase()
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| w.len() > 2)
                    .map(str::to_string)
                    .collect();
                let mut pending: Vec<Event> = self
                    .backend
                    .all()?
                    .into_iter()
                    .filter(|e| e.invite_pending && !e.declined)
                    .filter(|e| day.is_none_or(|d| e.start.date() == d))
                    .collect();
                // Narrow by the title words that appear in the request.
                let named: Vec<Event> = pending
                    .iter()
                    .filter(|e| {
                        e.title
                            .to_lowercase()
                            .split_whitespace()
                            .any(|t| words.iter().any(|w| w == t))
                    })
                    .cloned()
                    .collect();
                if !named.is_empty() {
                    pending = named;
                }
                match pending.as_slice() {
                    [one] => {
                        self.backend.set_declined(&one.id, true)?;
                        Ok(out(&[
                            ("message", format!("Declined “{}”.", one.title)),
                            ("id", one.id.clone()),
                        ]))
                    }
                    [] => Err(ConnError::Failed("I can't find that invite.".into())),
                    _ => Err(ConnError::Failed("Which invite should I decline?".into())),
                }
            }
            "context" => {
                // "Show me what I need for my 3pm": the meeting, and its people.
                let e = self.find_by_hour(text, today)?;
                let with = if e.attendees.is_empty() {
                    String::new()
                } else {
                    format!(" with {}", e.attendees.join(", "))
                };
                Ok(out(&[
                    (
                        "message",
                        format!("{} at {}{with}.", e.title, short(e.start)),
                    ),
                    ("title", e.title),
                ]))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        ready(&*self.health, "Outlook calendar")?;
        match action {
            "agenda" | "free" | "context" => Ok(true),
            // A check sees only the arguments, so these prove the calendar is reachable.
            "add" | "block" | "invite" | "move" | "decline" => Ok(self.backend.all().is_ok()),
            other => {
                let _ = args;
                Err(ConnError::UnknownAction(other.into()))
            }
        }
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        match action {
            "invite" if output.0.contains_key("attendee_of") => Some(
                self.backend
                    .remove_attendee(output.0.get("attendee_of")?, output.0.get("who")?),
            ),
            "invite" if output.0.contains_key("attendee_of") => Some(
                self.backend
                    .remove_attendee(output.0.get("attendee_of")?, output.0.get("who")?),
            ),
            "add" | "block" | "invite" => {
                let id = output.0.get("id")?;
                Some(self.backend.remove(id))
            }
            "move" => {
                let id = output.0.get("id")?;
                let s = NaiveDateTime::parse_from_str(output.0.get("old_start")?, FMT).ok()?;
                let e = NaiveDateTime::parse_from_str(output.0.get("old_end")?, FMT).ok()?;
                Some(self.backend.reschedule(id, s, e))
            }
            "decline" => {
                let id = output.0.get("id")?;
                Some(self.backend.set_declined(id, false))
            }
            _ => None,
        }
    }
}

trait HourIn {
    fn hour_in(&self, from: u32, to: u32) -> bool;
}

impl HourIn for NaiveTime {
    fn hour_in(&self, from: u32, to: u32) -> bool {
        use chrono::Timelike;
        self.hour() >= from && self.hour() < to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, &str)]) -> Args {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn setup() -> (OutlookCalendar, Arc<FakeCalendar>) {
        let b = Arc::new(FakeCalendar::seeded());
        (OutlookCalendar::fake(b.clone()), b)
    }

    fn run(c: &OutlookCalendar, action: &str, text: &str) -> Result<Output, ConnError> {
        c.execute(action, &args(&[("text", text)]))
    }

    fn msg(c: &OutlookCalendar, action: &str, text: &str) -> String {
        run(c, action, text).unwrap().0["message"].clone()
    }

    #[test]
    fn the_agenda_for_today_a_day_and_a_part_of_the_day() {
        let (c, _) = setup();
        assert_eq!(
            msg(&c, "agenda", "What's on my calendar today?"),
            "today: 09:30 Standup; 15:00 Project review; 17:00 Team sync."
        );
        assert!(msg(&c, "agenda", "am i free thurs afternoon")
            .starts_with("Nothing on your calendar Thu 12 Nov"));
        assert_eq!(
            msg(&c, "agenda", "what do i have thursday morning"),
            "Thu 12 Nov: 11:00 Budget sync."
        );
    }

    #[test]
    fn shared_free_time_leaves_out_both_calendars() {
        let (c, _) = setup();
        let m = msg(&c, "free", "When are Rahul and I both free this week?");
        assert!(m.starts_with("When you and rahul are both free:"), "{m}");
        // Monday: after 10:00 now and not in the 15:00 review.
        assert!(m.contains("Mon 9 Nov 10:00–15:00"), "{m}");
        let alone = msg(&c, "free", "when am i free this week");
        assert!(alone.starts_with("When you are free:"));
    }

    #[test]
    fn adding_an_event_parses_the_day_and_time_and_can_be_undone() {
        let (c, b) = setup();
        let r = run(&c, "add", "Add dentist on Friday at 4pm").unwrap();
        assert_eq!(r.0["message"], "Added “Dentist” on Fri 13 Nov at 16:00.");
        assert_eq!(b.all().unwrap().len(), 7);
        assert_eq!(c.undo("add", &r), Some(Ok(())));
        assert_eq!(b.all().unwrap().len(), 6);
    }

    #[test]
    fn a_focus_block_lands_in_the_free_part_of_the_day() {
        let (c, b) = setup();
        let r = run(&c, "block", "Block two hours tomorrow morning").unwrap();
        // Tomorrow has the 11:00 dentist: the first free two hours in the morning start at 09:00? No:
        // 09:00–11:00 is free.
        assert_eq!(
            r.0["message"],
            "Blocked 09:00–11:00 on Tue 10 Nov for focus."
        );
        assert_eq!(c.undo("block", &r), Some(Ok(())));
        assert_eq!(b.all().unwrap().len(), 6);
        assert!(run(&c, "block", "block some time").is_err());
    }

    #[test]
    fn an_invite_picks_a_time_when_both_are_free() {
        let (c, b) = setup();
        let r = c
            .execute(
                "invite",
                &args(&[
                    ("to", "Rahul Sharma"),
                    ("text", "Set up 30 minutes with Rahul on Thursday"),
                ]),
            )
            .unwrap();
        // Thursday: Rahul is busy 09:00–12:00, I have the Budget sync invite at 11:00.
        assert_eq!(
            r.0["message"],
            "Invited Rahul Sharma for Thu 12 Nov at 12:00."
        );
        assert_eq!(c.undo("invite", &r), Some(Ok(())));
        assert_eq!(b.all().unwrap().len(), 6);
        assert!(c
            .execute(
                "invite",
                &args(&[
                    ("to", "Rahul Sharma"),
                    ("text", "set up 30 minutes with Rahul")
                ])
            )
            .is_err());
    }

    #[test]
    fn inviting_someone_to_a_named_meeting_adds_them_and_undo_removes_them() {
        let (c, b) = setup();
        let r = c
            .execute(
                "invite",
                &args(&[
                    ("to", "Priya"),
                    ("text", "invite priya to the project review"),
                ]),
            )
            .unwrap();
        assert_eq!(
            r.0["message"],
            "Invited Priya to “Project review” on Mon 9 Nov at 15:00."
        );
        assert!(b
            .all()
            .unwrap()
            .iter()
            .find(|e| e.id == "e2")
            .unwrap()
            .attendees
            .contains(&"Priya".to_string()));
        assert_eq!(c.undo("invite", &r), Some(Ok(())));
        assert!(!b
            .all()
            .unwrap()
            .iter()
            .find(|e| e.id == "e2")
            .unwrap()
            .attendees
            .contains(&"Priya".to_string()));
    }

    #[test]
    fn moving_the_3pm_to_tomorrow_keeps_the_time_and_undoes() {
        let (c, b) = setup();
        let r = run(&c, "move", "Move my 3pm to tomorrow, same time").unwrap();
        assert_eq!(
            r.0["message"],
            "Moved “Project review” to Tue 10 Nov at 15:00."
        );
        assert_eq!(c.undo("move", &r), Some(Ok(())));
        assert_eq!(
            b.all()
                .unwrap()
                .iter()
                .find(|e| e.id == "e2")
                .unwrap()
                .start,
            dt("2026-11-09 15:00")
        );
        let r = run(&c, "move", "move my 3pm to friday at 4pm").unwrap();
        assert!(r.0["message"].ends_with("Fri 13 Nov at 16:00."));
    }

    #[test]
    fn moving_with_three_meetings_and_no_time_asks_which() {
        let (c, _) = setup();
        let e = run(&c, "move", "Move my meeting to tomorrow").unwrap_err();
        assert!(e.to_string().starts_with("Which meeting"));
        assert!(run(&c, "move", "move my 7pm to tomorrow").is_err());
    }

    #[test]
    fn declining_names_one_invite_or_asks() {
        let (c, b) = setup();
        let r = run(&c, "decline", "Decline the Friday review").unwrap();
        assert_eq!(r.0["message"], "Declined “Friday review”.");
        assert!(
            b.all()
                .unwrap()
                .iter()
                .find(|e| e.id == "e6")
                .unwrap()
                .declined
        );
        assert_eq!(c.undo("decline", &r), Some(Ok(())));
        // "Decline it" with two pending invites: ask.
        let e = run(&c, "decline", "Decline it").unwrap_err();
        assert_eq!(e.to_string(), "Which invite should I decline?");
    }

    #[test]
    fn the_3pm_context_names_the_meeting_and_the_people() {
        let (c, _) = setup();
        assert_eq!(
            msg(&c, "context", "Show me what I need for my 3pm"),
            "Project review at 15:00 with Rahul Sharma."
        );
    }

    #[test]
    fn clock_times_and_days_are_read_from_words() {
        assert_eq!(
            clock_times("move my 3pm to 10:30 am"),
            vec![(15, 0), (10, 30)]
        );
        assert_eq!(clock_times("at 12am and 12pm"), vec![(0, 0), (12, 0)]);
        assert!(clock_times("room 3 floor 4").is_empty());
        let today = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(
            day_in("on thurs", today),
            NaiveDate::from_ymd_opt(2026, 11, 12)
        );
        assert_eq!(
            day_in("tomorrow", today),
            NaiveDate::from_ymd_opt(2026, 11, 10)
        );
        assert_eq!(
            day_in("monday", today),
            NaiveDate::from_ymd_opt(2026, 11, 16)
        );
    }

    #[test]
    fn signed_out_changes_nothing() {
        let b = Arc::new(FakeCalendar::seeded());
        let c = OutlookCalendar::new(
            b.clone(),
            Box::new(|| Health::NotSignedIn),
            Box::new(crate::ms365::fixed_now),
        );
        assert_eq!(
            run(&c, "add", "add dentist friday 4pm")
                .unwrap_err()
                .to_string(),
            "Please sign in to Microsoft 365 again."
        );
        assert_eq!(b.all().unwrap().len(), 6);
    }
}
