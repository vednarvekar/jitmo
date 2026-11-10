//! Microsoft 365 connectors (Phase 3, Track A): `outlook` and `outlook_calendar`.
//!
//! Each connector talks to a backend trait. Today the only backends are the seeded fakes in
//! this file: a small mailbox and calendar with the people and files the test set uses (two
//! Rahuls, a meeting request, invoices), and a reset step. The real backend, which calls
//! Microsoft Graph with a [`crate::oauth::Session`] token, plugs in behind the same traits
//! when the app registration exists; nothing above this file changes.
//!
//! What is built in from the start, because it is hard to add later:
//! - no send action at all (SR4: a send needs a confirm with recipient and preview, and the
//!   `Mail.Send` scope, which is not asked for yet);
//! - archive and move can be undone (SR3), and `check` reads the state back;
//! - mail text is data (SR5): these actions return sender, subject and counts for the user
//!   to read, never message bodies for another model to act on.

use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::sync::{Arc, Mutex};

const NAME: &str = "Outlook";

fn arg<'a>(args: &'a Args, name: &str) -> Result<&'a str, ConnError> {
    args.get(name)
        .map(String::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ConnError::MissingArg(name.into()))
}

fn out(pairs: &[(&str, String)]) -> Output {
    Output(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Mail
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub id: String,
    pub from_name: String,
    pub from_address: String,
    pub subject: String,
    pub unread: bool,
    pub urgent: bool,
    pub folder: String,
}

pub trait MailBackend: Send + Sync {
    fn unread(&self) -> Result<Vec<Mail>, ConnError>;
    fn search(&self, query: &str) -> Result<Vec<Mail>, ConnError>;
    fn get(&self, id: &str) -> Result<Option<Mail>, ConnError>;
    fn move_to(&self, id: &str, folder: &str) -> Result<(), ConnError>;
}

/// A mailbox in memory, seeded for tests. `reset` puts it back as it started.
pub struct FakeMailbox {
    mails: Mutex<Vec<Mail>>,
}

fn seeded_mail() -> Vec<Mail> {
    let m = |id: &str, name: &str, addr: &str, subject: &str, unread, urgent| Mail {
        id: id.into(),
        from_name: name.into(),
        from_address: addr.into(),
        subject: subject.into(),
        unread,
        urgent,
        folder: "Inbox".into(),
    };
    vec![
        m(
            "m1",
            "Rahul Sharma",
            "rahul.sharma@contoso.test",
            "Meeting request: project review",
            true,
            true,
        ),
        m(
            "m2",
            "Rahul Mehta",
            "rahul.mehta@fabrikam.test",
            "Invoice for October",
            true,
            false,
        ),
        m(
            "m3",
            "Priya Nair",
            "priya@contoso.test",
            "Weekly report",
            true,
            false,
        ),
        m(
            "m4",
            "Finance",
            "finance@contoso.test",
            "Invoice March attached",
            false,
            false,
        ),
        m(
            "m5",
            "Ved Narvekar",
            "ved@contoso.test",
            "Lunch on Friday?",
            true,
            false,
        ),
        m(
            "m6",
            "Ved Kulkarni",
            "ved.k@fabrikam.test",
            "Urgent: contract signature",
            true,
            true,
        ),
    ]
}

impl FakeMailbox {
    pub fn seeded() -> FakeMailbox {
        FakeMailbox {
            mails: Mutex::new(seeded_mail()),
        }
    }

    pub fn reset(&self) {
        if let Ok(mut g) = self.mails.lock() {
            *g = seeded_mail();
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<Mail>>, ConnError> {
        self.mails
            .lock()
            .map_err(|_| ConnError::Failed("The test mailbox is broken.".into()))
    }
}

impl MailBackend for FakeMailbox {
    fn unread(&self) -> Result<Vec<Mail>, ConnError> {
        Ok(self
            .lock()?
            .iter()
            .filter(|m| m.unread && m.folder == "Inbox")
            .cloned()
            .collect())
    }

    fn search(&self, query: &str) -> Result<Vec<Mail>, ConnError> {
        let q = query.to_lowercase();
        Ok(self
            .lock()?
            .iter()
            .filter(|m| {
                m.subject.to_lowercase().contains(&q)
                    || m.from_name.to_lowercase().contains(&q)
                    || m.from_address.to_lowercase().contains(&q)
            })
            .cloned()
            .collect())
    }

    fn get(&self, id: &str) -> Result<Option<Mail>, ConnError> {
        Ok(self.lock()?.iter().find(|m| m.id == id).cloned())
    }

    fn move_to(&self, id: &str, folder: &str) -> Result<(), ConnError> {
        let mut g = self.lock()?;
        let m = g
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| ConnError::Failed("I can't find that email.".into()))?;
        m.folder = folder.into();
        Ok(())
    }
}

fn line(m: &Mail) -> String {
    format!("{}: {}", m.from_name, m.subject)
}

pub struct Outlook {
    backend: Arc<dyn MailBackend>,
    health: Box<dyn Fn() -> Health + Send + Sync>,
}

impl Outlook {
    pub fn new(
        backend: Arc<dyn MailBackend>,
        health: Box<dyn Fn() -> Health + Send + Sync>,
    ) -> Outlook {
        Outlook { backend, health }
    }

    /// With the seeded fake mailbox, always signed in.
    pub fn fake(mailbox: Arc<FakeMailbox>) -> Outlook {
        Outlook::new(mailbox, Box::new(|| Health::Ready))
    }

    fn ready(&self) -> Result<(), ConnError> {
        match (self.health)() {
            Health::Ready => Ok(()),
            Health::NotSignedIn => Err(ConnError::NotSignedIn("Microsoft 365".into())),
            Health::Unreachable => Err(ConnError::Offline("Microsoft 365".into())),
            Health::NotInstalled => Err(ConnError::NotInstalled(NAME.into())),
        }
    }
}

impl Connector for Outlook {
    fn id(&self) -> &str {
        "outlook"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        let a = |n: &str, r: &[&str]| ActionSpec {
            name: n.into(),
            required_args: r.iter().map(|s| s.to_string()).collect(),
        };
        // No "send": see the module notes.
        vec![
            a("unread", &[]),
            a("urgent", &[]),
            a("search", &["query"]),
            a("archive", &["id"]),
            a("move", &["id", "folder"]),
        ]
    }

    fn health(&self) -> Health {
        (self.health)()
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        self.ready()?;
        match action {
            "unread" => {
                let mails = self.backend.unread()?;
                let message = if mails.is_empty() {
                    "No unread email.".to_string()
                } else {
                    format!(
                        "{} unread: {}.",
                        mails.len(),
                        mails
                            .iter()
                            .take(5)
                            .map(line)
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                };
                Ok(out(&[
                    ("message", message),
                    ("count", mails.len().to_string()),
                ]))
            }
            "urgent" => {
                let mails: Vec<Mail> = self
                    .backend
                    .unread()?
                    .into_iter()
                    .filter(|m| m.urgent)
                    .collect();
                let message = if mails.is_empty() {
                    "Nothing urgent.".to_string()
                } else {
                    format!(
                        "{} urgent: {}.",
                        mails.len(),
                        mails.iter().map(line).collect::<Vec<_>>().join("; ")
                    )
                };
                Ok(out(&[
                    ("message", message),
                    ("count", mails.len().to_string()),
                ]))
            }
            "search" => {
                let q = arg(args, "query")?;
                let mails = self.backend.search(q)?;
                let message = if mails.is_empty() {
                    format!("No email matches “{q}”.")
                } else {
                    format!(
                        "{} found: {}.",
                        mails.len(),
                        mails
                            .iter()
                            .take(5)
                            .map(line)
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                };
                // The ids let a later step (archive, move) name one message exactly.
                let ids = mails
                    .iter()
                    .map(|m| m.id.clone())
                    .collect::<Vec<_>>()
                    .join(",");
                Ok(out(&[("message", message), ("ids", ids)]))
            }
            "archive" | "move" => {
                let id = arg(args, "id")?;
                let folder = if action == "archive" {
                    "Archive"
                } else {
                    arg(args, "folder")?
                };
                let before = self
                    .backend
                    .get(id)?
                    .ok_or_else(|| ConnError::Failed("I can't find that email.".into()))?;
                self.backend.move_to(id, folder)?;
                Ok(out(&[
                    (
                        "message",
                        format!("Moved “{}” to {folder}.", before.subject),
                    ),
                    ("id", id.to_string()),
                    ("folder", folder.to_string()),
                    ("was", before.folder),
                ]))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        self.ready()?;
        match action {
            "archive" | "move" => {
                let id = arg(args, "id")?;
                let want = if action == "archive" {
                    "Archive"
                } else {
                    arg(args, "folder")?
                };
                Ok(self.backend.get(id)?.is_some_and(|m| m.folder == want))
            }
            // Reads change nothing, so there is nothing to prove.
            "unread" | "urgent" | "search" => Ok(true),
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        if !matches!(action, "archive" | "move") {
            return None;
        }
        let id = output.0.get("id")?;
        let was = output.0.get("was")?;
        Some(self.backend.move_to(id, was))
    }
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub title: String,
    /// Local start, `YYYY-MM-DD HH:MM`.
    pub start: String,
}

pub trait CalendarBackend: Send + Sync {
    fn on_day(&self, day: &str) -> Result<Vec<Event>, ConnError>;
    fn create(&self, title: &str, start: &str) -> Result<Event, ConnError>;
    fn get(&self, id: &str) -> Result<Option<Event>, ConnError>;
    fn delete(&self, id: &str) -> Result<(), ConnError>;
}

pub struct FakeCalendar {
    events: Mutex<Vec<Event>>,
    next: Mutex<u32>,
}

fn seeded_events() -> Vec<Event> {
    let e = |id: &str, title: &str, start: &str| Event {
        id: id.into(),
        title: title.into(),
        start: start.into(),
    };
    vec![
        e("e1", "Standup", "2026-11-09 09:30"),
        e("e2", "Project review with Rahul", "2026-11-09 15:00"),
        e("e3", "Dentist", "2026-11-10 11:00"),
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
    fn on_day(&self, day: &str) -> Result<Vec<Event>, ConnError> {
        let mut v: Vec<Event> = self
            .lock()?
            .iter()
            .filter(|e| e.start.starts_with(day))
            .cloned()
            .collect();
        v.sort_by(|a, b| a.start.cmp(&b.start));
        Ok(v)
    }

    fn create(&self, title: &str, start: &str) -> Result<Event, ConnError> {
        let mut n = self
            .next
            .lock()
            .map_err(|_| ConnError::Failed("The test calendar is broken.".into()))?;
        *n += 1;
        let e = Event {
            id: format!("e{n}"),
            title: title.into(),
            start: start.into(),
        };
        self.lock()?.push(e.clone());
        Ok(e)
    }

    fn get(&self, id: &str) -> Result<Option<Event>, ConnError> {
        Ok(self.lock()?.iter().find(|e| e.id == id).cloned())
    }

    fn delete(&self, id: &str) -> Result<(), ConnError> {
        self.lock()?.retain(|e| e.id != id);
        Ok(())
    }
}

pub struct OutlookCalendar {
    backend: Arc<dyn CalendarBackend>,
    health: Box<dyn Fn() -> Health + Send + Sync>,
}

impl OutlookCalendar {
    pub fn new(
        backend: Arc<dyn CalendarBackend>,
        health: Box<dyn Fn() -> Health + Send + Sync>,
    ) -> OutlookCalendar {
        OutlookCalendar { backend, health }
    }

    pub fn fake(calendar: Arc<FakeCalendar>) -> OutlookCalendar {
        OutlookCalendar::new(calendar, Box::new(|| Health::Ready))
    }

    fn ready(&self) -> Result<(), ConnError> {
        match (self.health)() {
            Health::Ready => Ok(()),
            Health::NotSignedIn => Err(ConnError::NotSignedIn("Microsoft 365".into())),
            Health::Unreachable => Err(ConnError::Offline("Microsoft 365".into())),
            Health::NotInstalled => Err(ConnError::NotInstalled("Outlook calendar".into())),
        }
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
        vec![a("day", &["date"]), a("create", &["title", "start"])]
    }

    fn health(&self) -> Health {
        (self.health)()
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        self.ready()?;
        match action {
            "day" => {
                let day = arg(args, "date")?;
                let events = self.backend.on_day(day)?;
                let message = if events.is_empty() {
                    "Nothing on the calendar.".to_string()
                } else {
                    events
                        .iter()
                        .map(|e| format!("{} {}", e.start.get(11..).unwrap_or(""), e.title))
                        .collect::<Vec<_>>()
                        .join("; ")
                };
                Ok(out(&[
                    ("message", message),
                    ("count", events.len().to_string()),
                ]))
            }
            "create" => {
                let title = arg(args, "title")?;
                let start = arg(args, "start")?;
                if start.len() != 16 || start.as_bytes()[10] != b' ' {
                    return Err(ConnError::Failed(
                        "I need the start as a date and time.".into(),
                    ));
                }
                let e = self.backend.create(title, start)?;
                Ok(out(&[
                    ("message", format!("Added “{title}” on {start}.")),
                    ("id", e.id),
                ]))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        self.ready()?;
        match action {
            "day" => Ok(true),
            // The new event's id is not known to a check, so look for it by title and time.
            "create" => {
                let (title, start) = (arg(args, "title")?, arg(args, "start")?);
                let day = start.get(..10).unwrap_or(start);
                Ok(self
                    .backend
                    .on_day(day)?
                    .iter()
                    .any(|e| e.title == title && e.start == start))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        if action != "create" {
            return None;
        }
        let id = output.0.get("id")?;
        Some(self.backend.delete(id))
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

    fn mail() -> (Outlook, Arc<FakeMailbox>) {
        let b = Arc::new(FakeMailbox::seeded());
        (Outlook::fake(b.clone()), b)
    }

    #[test]
    fn unread_and_urgent_read_the_seeded_inbox() {
        let (o, _) = mail();
        let r = o.execute("unread", &args(&[])).unwrap();
        assert_eq!(r.0["count"], "5");
        let r = o.execute("urgent", &args(&[])).unwrap();
        assert_eq!(r.0["count"], "2");
        assert!(r.0["message"].contains("Rahul Sharma"));
    }

    #[test]
    fn two_rahuls_are_found_separately_never_merged() {
        let (o, _) = mail();
        let r = o.execute("search", &args(&[("query", "rahul")])).unwrap();
        assert_eq!(r.0["ids"], "m1,m2");
        let r = o.execute("search", &args(&[("query", "ved")])).unwrap();
        assert_eq!(r.0["ids"], "m5,m6");
        let r = o.execute("search", &args(&[("query", "zzz")])).unwrap();
        assert!(r.0["message"].starts_with("No email matches"));
    }

    #[test]
    fn archive_moves_reads_back_and_undoes() {
        let (o, b) = mail();
        let r = o.execute("archive", &args(&[("id", "m2")])).unwrap();
        assert_eq!(o.check("archive", &args(&[("id", "m2")])), Ok(true));
        assert_eq!(b.get("m2").unwrap().unwrap().folder, "Archive");
        assert_eq!(o.undo("archive", &r), Some(Ok(())));
        assert_eq!(b.get("m2").unwrap().unwrap().folder, "Inbox");
        assert_eq!(o.check("archive", &args(&[("id", "m2")])), Ok(false));
        b.reset();
    }

    #[test]
    fn move_needs_a_folder_and_a_real_message() {
        let (o, _) = mail();
        assert!(matches!(
            o.execute("move", &args(&[("id", "m1")])),
            Err(ConnError::MissingArg(_))
        ));
        assert!(o.execute("archive", &args(&[("id", "nope")])).is_err());
        let r = o
            .execute("move", &args(&[("id", "m1"), ("folder", "Projects")]))
            .unwrap();
        assert_eq!(r.0["folder"], "Projects");
    }

    #[test]
    fn there_is_no_send_action_yet() {
        let (o, _) = mail();
        assert!(o.actions().iter().all(|a| !a.name.contains("send")));
        assert!(matches!(
            o.execute("send", &args(&[])),
            Err(ConnError::UnknownAction(_))
        ));
    }

    #[test]
    fn signed_out_says_sign_in_again_and_touches_nothing() {
        let b = Arc::new(FakeMailbox::seeded());
        let o = Outlook::new(b.clone(), Box::new(|| Health::NotSignedIn));
        let e = o.execute("archive", &args(&[("id", "m1")])).unwrap_err();
        assert_eq!(e.to_string(), "Please sign in to Microsoft 365 again.");
        assert_eq!(b.get("m1").unwrap().unwrap().folder, "Inbox");
        assert_eq!(o.health(), Health::NotSignedIn);
    }

    #[test]
    fn the_calendar_lists_a_day_and_creates_with_undo() {
        let cal = Arc::new(FakeCalendar::seeded());
        let c = OutlookCalendar::fake(cal.clone());
        let r = c.execute("day", &args(&[("date", "2026-11-09")])).unwrap();
        assert_eq!(r.0["count"], "2");
        assert!(r.0["message"].starts_with("09:30 Standup"));
        let made = c
            .execute(
                "create",
                &args(&[("title", "Project review"), ("start", "2026-11-12 10:00")]),
            )
            .unwrap();
        assert_eq!(
            c.check(
                "create",
                &args(&[("title", "Project review"), ("start", "2026-11-12 10:00")])
            ),
            Ok(true)
        );
        assert_eq!(c.undo("create", &made), Some(Ok(())));
        assert_eq!(
            c.check(
                "create",
                &args(&[("title", "Project review"), ("start", "2026-11-12 10:00")])
            ),
            Ok(false)
        );
        assert!(c
            .execute("create", &args(&[("title", "x"), ("start", "tomorrow")]))
            .is_err());
        cal.reset();
    }
}
