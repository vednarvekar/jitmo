//! Microsoft 365 files, Teams and the work flows (Phase 3, Track A): the `onedrive`, `teams`
//! and `ms365_workflows` connectors.
//!
//! Backends are traits with seeded fakes, as in [`crate::ms365`]. The workflows connector
//! joins mail, calendar, files and chat for W1 to W5 (morning brief, email to event, save
//! attachments, forward an attachment to a chat, send the weekly report). Each step that
//! sends or shares runs under a skill with a confirm; what is saved or created can be
//! undone.

use crate::ms365::{arg, message_text, out, person_in, ready, MailBackend};
use crate::ms365_cal::{clock_times, day_in, day_name, short, CalendarBackend};
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use chrono::{DateTime, Duration, Local};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// OneDrive
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub folder: String,
}

pub trait DriveBackend: Send + Sync {
    fn files(&self) -> Result<Vec<DriveFile>, ConnError>;
    fn folders(&self) -> Result<Vec<String>, ConnError>;
    fn save(&self, name: &str, folder: &str) -> Result<String, ConnError>;
    fn delete(&self, id: &str) -> Result<(), ConnError>;
    /// Share a folder with someone; returns the link.
    fn share(&self, folder: &str, with: &str) -> Result<String, ConnError>;
    fn unshare(&self, folder: &str, with: &str) -> Result<(), ConnError>;
    fn shares(&self) -> Result<Vec<(String, String)>, ConnError>;
}

pub struct FakeDrive {
    files: Mutex<Vec<DriveFile>>,
    shares: Mutex<Vec<(String, String)>>,
    next: Mutex<u32>,
}

fn seeded_files() -> Vec<DriveFile> {
    let f = |id: &str, name: &str, folder: &str| DriveFile {
        id: id.into(),
        name: name.into(),
        folder: folder.into(),
    };
    vec![
        f("f1", "weekly_report.pdf", "Reports"),
        f("f2", "budget.xlsx", "Documents"),
        f("f3", "Project plan.docx", "Project"),
    ]
}

impl FakeDrive {
    pub fn seeded() -> FakeDrive {
        FakeDrive {
            files: Mutex::new(seeded_files()),
            shares: Mutex::new(Vec::new()),
            next: Mutex::new(100),
        }
    }

    pub fn reset(&self) {
        if let Ok(mut g) = self.files.lock() {
            *g = seeded_files();
        }
        if let Ok(mut g) = self.shares.lock() {
            g.clear();
        }
    }

    fn broken() -> ConnError {
        ConnError::Failed("The test drive is broken.".into())
    }
}

impl DriveBackend for FakeDrive {
    fn files(&self) -> Result<Vec<DriveFile>, ConnError> {
        Ok(self.files.lock().map_err(|_| Self::broken())?.clone())
    }

    fn folders(&self) -> Result<Vec<String>, ConnError> {
        Ok(vec![
            "Documents".into(),
            "Finance".into(),
            "Project".into(),
            "Reports".into(),
        ])
    }

    fn save(&self, name: &str, folder: &str) -> Result<String, ConnError> {
        let mut n = self.next.lock().map_err(|_| Self::broken())?;
        *n += 1;
        let id = format!("f{n}");
        self.files
            .lock()
            .map_err(|_| Self::broken())?
            .push(DriveFile {
                id: id.clone(),
                name: name.into(),
                folder: folder.into(),
            });
        Ok(id)
    }

    fn delete(&self, id: &str) -> Result<(), ConnError> {
        self.files
            .lock()
            .map_err(|_| Self::broken())?
            .retain(|f| f.id != id);
        Ok(())
    }

    fn share(&self, folder: &str, with: &str) -> Result<String, ConnError> {
        self.shares
            .lock()
            .map_err(|_| Self::broken())?
            .push((folder.into(), with.into()));
        Ok(format!(
            "https://onedrive.test/share/{}",
            folder.to_lowercase()
        ))
    }

    fn unshare(&self, folder: &str, with: &str) -> Result<(), ConnError> {
        self.shares
            .lock()
            .map_err(|_| Self::broken())?
            .retain(|(f, w)| !(f == folder && w == with));
        Ok(())
    }

    fn shares(&self) -> Result<Vec<(String, String)>, ConnError> {
        Ok(self.shares.lock().map_err(|_| Self::broken())?.clone())
    }
}

/// The folder a request names: "the Project folder", "to the Finance folder", "in Finance".
fn folder_in(text: &str, folders: &[String]) -> Option<String> {
    let lower = text.to_lowercase();
    folders
        .iter()
        .find(|f| {
            lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == f.to_lowercase())
        })
        .cloned()
}

pub struct OneDrive {
    backend: Arc<dyn DriveBackend>,
    health: Box<dyn Fn() -> Health + Send + Sync>,
}

impl OneDrive {
    pub fn new(
        backend: Arc<dyn DriveBackend>,
        health: Box<dyn Fn() -> Health + Send + Sync>,
    ) -> OneDrive {
        OneDrive { backend, health }
    }

    pub fn fake(drive: Arc<FakeDrive>) -> OneDrive {
        OneDrive::new(drive, Box::new(|| Health::Ready))
    }
}

impl Connector for OneDrive {
    fn id(&self) -> &str {
        "onedrive"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec {
            name: "share".into(),
            required_args: vec!["text".into(), "with".into()],
        }]
    }

    fn health(&self) -> Health {
        (self.health)()
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        ready(&*self.health, "OneDrive")?;
        match action {
            "share" => {
                let text = arg(args, "text")?;
                let with = arg(args, "with")?;
                let folder = folder_in(text, &self.backend.folders()?).ok_or_else(|| {
                    ConnError::Failed(
                        "Which folder, and with whom? Try “share the Project folder with Priya”."
                            .into(),
                    )
                })?;
                let link = self.backend.share(&folder, with)?;
                Ok(out(&[
                    (
                        "message",
                        format!("Shared the {folder} folder with {with}."),
                    ),
                    ("folder", folder),
                    ("with", with.to_string()),
                    ("link", link),
                ]))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        ready(&*self.health, "OneDrive")?;
        match action {
            "share" => {
                let with = arg(args, "with")?;
                let folder = folder_in(arg(args, "text")?, &self.backend.folders()?);
                Ok(folder.is_some_and(|f| {
                    self.backend
                        .shares()
                        .is_ok_and(|s| s.contains(&(f, with.to_string())))
                }))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        if action != "share" {
            return None;
        }
        Some(
            self.backend
                .unshare(output.0.get("folder")?, output.0.get("with")?),
        )
    }
}

// ---------------------------------------------------------------------------
// Teams
// ---------------------------------------------------------------------------

pub trait ChatBackend: Send + Sync {
    fn channels(&self) -> Result<Vec<String>, ConnError>;
    fn post(&self, channel: &str, text: &str) -> Result<String, ConnError>;
    fn messages(&self, channel: &str) -> Result<Vec<(String, String)>, ConnError>;
    fn delete(&self, id: &str) -> Result<(), ConnError>;
}

#[derive(Default)]
pub struct FakeChat {
    posts: Mutex<Vec<(String, String, String)>>, // id, channel, text
}

impl FakeChat {
    pub fn seeded() -> FakeChat {
        FakeChat::default()
    }

    pub fn reset(&self) {
        if let Ok(mut g) = self.posts.lock() {
            g.clear();
        }
    }
}

impl ChatBackend for FakeChat {
    fn channels(&self) -> Result<Vec<String>, ConnError> {
        Ok(vec!["team".into(), "project".into()])
    }

    fn post(&self, channel: &str, text: &str) -> Result<String, ConnError> {
        let mut g = self
            .posts
            .lock()
            .map_err(|_| ConnError::Failed("The test chat is broken.".into()))?;
        let id = format!("p{}", g.len() + 1);
        g.push((id.clone(), channel.into(), text.into()));
        Ok(id)
    }

    fn messages(&self, channel: &str) -> Result<Vec<(String, String)>, ConnError> {
        Ok(self
            .posts
            .lock()
            .map_err(|_| ConnError::Failed("The test chat is broken.".into()))?
            .iter()
            .filter(|(_, c, _)| c == channel)
            .map(|(id, _, t)| (id.clone(), t.clone()))
            .collect())
    }

    fn delete(&self, id: &str) -> Result<(), ConnError> {
        self.posts
            .lock()
            .map_err(|_| ConnError::Failed("The test chat is broken.".into()))?
            .retain(|(i, _, _)| i != id);
        Ok(())
    }
}

/// "Tell the team channel standup moves to 11" -> ("team", "Standup moves to 11.").
fn chat_request(text: &str, channels: &[String]) -> Option<(String, String)> {
    let channel = channels
        .iter()
        .find(|c| {
            text.to_lowercase()
                .split(|ch: char| !ch.is_alphanumeric())
                .any(|w| w == c.as_str())
        })?
        .clone();
    let lower = text.to_lowercase();
    // The message follows "channel" / "chat" ("tell the team channel ..."), else "that".
    for marker in [" channel ", " chat "] {
        if let Some(i) = lower.find(marker) {
            let rest = text[i + marker.len()..].trim().trim_end_matches('.');
            let rest = rest
                .strip_prefix("that ")
                .or_else(|| rest.strip_prefix("saying "))
                .unwrap_or(rest);
            if !rest.is_empty() {
                let mut c = rest.chars();
                let first = c.next()?.to_uppercase().collect::<String>();
                return Some((channel, format!("{first}{}.", c.as_str())));
            }
        }
    }
    message_text(text).map(|m| (channel, m))
}

pub struct Teams {
    backend: Arc<dyn ChatBackend>,
    health: Box<dyn Fn() -> Health + Send + Sync>,
}

impl Teams {
    pub fn new(
        backend: Arc<dyn ChatBackend>,
        health: Box<dyn Fn() -> Health + Send + Sync>,
    ) -> Teams {
        Teams { backend, health }
    }

    pub fn fake(chat: Arc<FakeChat>) -> Teams {
        Teams::new(chat, Box::new(|| Health::Ready))
    }
}

impl Connector for Teams {
    fn id(&self) -> &str {
        "teams"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec {
            name: "post".into(),
            required_args: vec!["text".into()],
        }]
    }

    fn health(&self) -> Health {
        (self.health)()
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        ready(&*self.health, "Teams")?;
        match action {
            "post" => {
                let (channel, message) = chat_request(arg(args, "text")?, &self.backend.channels()?)
                    .ok_or_else(|| {
                        ConnError::Failed(
                            "Which channel, and what should I say? Try “tell the team channel standup moves to 11”.".into(),
                        )
                    })?;
                let id = self.backend.post(&channel, &message)?;
                Ok(out(&[
                    ("message", format!("Posted in {channel}: “{message}”")),
                    ("id", id),
                    ("channel", channel),
                ]))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        ready(&*self.health, "Teams")?;
        match action {
            "post" => {
                let Some((channel, message)) =
                    chat_request(arg(args, "text")?, &self.backend.channels()?)
                else {
                    return Ok(false);
                };
                Ok(self
                    .backend
                    .messages(&channel)?
                    .iter()
                    .any(|(_, t)| *t == message))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    // A posted message cannot be taken back by the assistant: people have already seen it.
    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Work flows W1 to W5
// ---------------------------------------------------------------------------

pub struct Workflows {
    mail: Arc<dyn MailBackend>,
    calendar: Arc<dyn CalendarBackend>,
    drive: Arc<dyn DriveBackend>,
    chat: Arc<dyn ChatBackend>,
    outlook: Arc<dyn Connector>,
    agenda: Arc<dyn Connector>,
    health: Box<dyn Fn() -> Health + Send + Sync>,
    now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
}

/// The backends and connectors the flows join.
pub struct WorkflowParts {
    pub mail: Arc<dyn MailBackend>,
    pub calendar: Arc<dyn CalendarBackend>,
    pub drive: Arc<dyn DriveBackend>,
    pub chat: Arc<dyn ChatBackend>,
    pub outlook: Arc<dyn Connector>,
    pub agenda: Arc<dyn Connector>,
}

impl Workflows {
    pub fn new(
        parts: WorkflowParts,
        health: Box<dyn Fn() -> Health + Send + Sync>,
        now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
    ) -> Workflows {
        Workflows {
            mail: parts.mail,
            calendar: parts.calendar,
            drive: parts.drive,
            chat: parts.chat,
            outlook: parts.outlook,
            agenda: parts.agenda,
            health,
            now,
        }
    }

    /// The newest message from the person named in `text` (or from anyone) that has an
    /// attachment.
    fn mail_with_attachment(&self, text: &str) -> Result<crate::ms365::Mail, ConnError> {
        let all = self.mail.all()?;
        let names: Vec<String> = all.iter().map(|m| m.from_name.clone()).collect();
        let who = person_in(text, &names);
        let mut hits: Vec<crate::ms365::Mail> = all
            .into_iter()
            .filter(|m| !m.attachments.is_empty())
            .filter(|m| {
                who.as_ref().is_none_or(|w| {
                    m.from_name
                        .split_whitespace()
                        .next()
                        .is_some_and(|f| f.eq_ignore_ascii_case(w))
                })
            })
            .collect();
        hits.sort_by(|a, b| b.received.cmp(&a.received));
        hits.into_iter().next().ok_or_else(|| {
            ConnError::Failed(match who {
                Some(w) => format!("I found no email from {w} with an attachment."),
                None => "I found no email with an attachment.".into(),
            })
        })
    }
}

impl Connector for Workflows {
    fn id(&self) -> &str {
        "ms365_workflows"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        let a = |n: &str, r: &[&str]| ActionSpec {
            name: n.into(),
            required_args: r.iter().map(|s| s.to_string()).collect(),
        };
        vec![
            a("morning_brief", &[]),
            a("event_from_email", &["text"]),
            a("save_attachments", &["text"]),
            a("forward_attachment", &["text"]),
            a("send_report", &["text"]),
        ]
    }

    fn health(&self) -> Health {
        (self.health)()
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        ready(&*self.health, "Microsoft 365")?;
        match action {
            // W1: urgent mail, then today's meetings. Read only.
            "morning_brief" => {
                let urgent = self.outlook.execute("urgent", &Args::new())?;
                let today = self.agenda.execute(
                    "agenda",
                    &Args::from([("text".to_string(), "today".to_string())]),
                )?;
                Ok(out(&[(
                    "message",
                    format!("{} {}", urgent.0["message"], today.0["message"]),
                )]))
            }
            // W2: the open meeting request becomes a calendar event.
            "event_from_email" => {
                let today = (self.now)().date_naive();
                let mut requests: Vec<crate::ms365::Mail> = self
                    .mail
                    .all()?
                    .into_iter()
                    .filter(|m| {
                        m.folder == "Inbox" && m.subject.to_lowercase().contains("meeting request")
                    })
                    .collect();
                requests.sort_by(|a, b| b.received.cmp(&a.received));
                let m = requests
                    .first()
                    .ok_or_else(|| ConnError::Failed("I found no meeting request.".into()))?;
                let day = day_in(&m.body, today).ok_or_else(|| {
                    ConnError::Failed("That email does not say which day.".into())
                })?;
                let &(h, min) = clock_times(&m.body).first().ok_or_else(|| {
                    ConnError::Failed("That email does not say what time.".into())
                })?;
                let start = day.and_hms_opt(h, min, 0).unwrap_or_default();
                let title = m
                    .subject
                    .split_once(':')
                    .map_or(m.subject.clone(), |(_, t)| t.trim().to_string());
                let title = {
                    let mut c = title.chars();
                    c.next().map_or(title.clone(), |f| {
                        format!("{}{}", f.to_uppercase(), c.as_str())
                    })
                };
                let id = self.calendar.add(
                    &title,
                    start,
                    start + Duration::hours(1),
                    std::slice::from_ref(&m.from_name),
                )?;
                Ok(out(&[
                    (
                        "message",
                        format!("Added “{title}” on {} at {}.", day_name(day), short(start)),
                    ),
                    ("id", id),
                    ("title", title),
                    ("attendee", m.from_name.clone()),
                ]))
            }
            // W3: the attachments of one email go to a folder.
            "save_attachments" => {
                let text = arg(args, "text")?;
                let folder = folder_in(text, &self.drive.folders()?).ok_or_else(|| {
                    ConnError::Failed(
                        "Which folder? Try “save the invoices to the Finance folder”.".into(),
                    )
                })?;
                let m = self.mail_with_attachment(text)?;
                let ids: Vec<String> = m
                    .attachments
                    .iter()
                    .map(|a| self.drive.save(a, &folder))
                    .collect::<Result<_, _>>()?;
                Ok(out(&[
                    (
                        "message",
                        format!("Saved {} to the {folder} folder.", m.attachments.join(", ")),
                    ),
                    ("ids", ids.join(",")),
                    ("folder", folder),
                ]))
            }
            // W4: an attachment goes to a chat (the skill confirms first).
            "forward_attachment" => {
                let text = arg(args, "text")?;
                let channel = {
                    let channels = self.chat.channels()?;
                    channels
                        .iter()
                        .find(|c| text.to_lowercase().contains(c.as_str()))
                        .cloned()
                        .ok_or_else(|| ConnError::Failed("Which chat?".into()))?
                };
                let m = self.mail_with_attachment(text)?;
                let file = m.attachments.first().cloned().unwrap_or_default();
                let id = self
                    .chat
                    .post(&channel, &format!("{file} (from {}'s email)", m.from_name))?;
                Ok(out(&[
                    ("message", format!("Posted {file} in {channel}.")),
                    ("id", id),
                    ("channel", channel),
                ]))
            }
            // W5: the latest weekly report goes to the manager (the skill confirms first).
            "send_report" => {
                let files = self.drive.files()?;
                let report = files
                    .iter()
                    .rev()
                    .find(|f| f.folder == "Reports" || f.name.to_lowercase().contains("report"))
                    .ok_or_else(|| ConnError::Failed("I found no report to send.".into()))?;
                let sent = self.mail.add_outgoing(
                    "your manager",
                    "Here is this week's report.",
                    Some(&report.name),
                    true,
                )?;
                Ok(out(&[
                    ("message", format!("Sent {} to your manager.", report.name)),
                    ("id", sent),
                ]))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, _args: &Args) -> Result<bool, ConnError> {
        ready(&*self.health, "Microsoft 365")?;
        match action {
            "morning_brief" => Ok(true),
            "event_from_email" => Ok(self
                .calendar
                .all()?
                .iter()
                .any(|e| e.title.to_lowercase().contains("project review"))),
            "save_attachments" => Ok(self.drive.files()?.iter().any(|f| f.folder == "Finance")),
            "forward_attachment" => Ok(!self.chat.messages("project")?.is_empty()),
            "send_report" => Ok(self
                .mail
                .outgoing()?
                .iter()
                .any(|o| o.sent && o.attachment.is_some())),
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        match action {
            "event_from_email" => Some(self.calendar.remove(output.0.get("id")?)),
            "save_attachments" => Some(
                output
                    .0
                    .get("ids")?
                    .split(',')
                    .try_for_each(|id| self.drive.delete(id)),
            ),
            // A post and a sent email are seen by other people: no undo.
            _ => None,
        }
    }
}

/// All five Microsoft 365 connectors over one set of seeded fakes, signed in, on the test
/// clock (Monday 9 November 2026, 10:00). What the audit and the test set run against.
pub struct FakeSuite {
    pub mail: Arc<crate::ms365::FakeMailbox>,
    pub calendar: Arc<crate::ms365_cal::FakeCalendar>,
    pub drive: Arc<FakeDrive>,
    pub chat: Arc<FakeChat>,
    pub connectors: Vec<Arc<dyn Connector>>,
}

pub fn fake_suite() -> FakeSuite {
    use crate::ms365::{fixed_now, FakeMailbox, Outlook};
    use crate::ms365_cal::{FakeCalendar, OutlookCalendar};
    let mail = Arc::new(FakeMailbox::seeded());
    let calendar = Arc::new(FakeCalendar::seeded());
    let drive = Arc::new(FakeDrive::seeded());
    let chat = Arc::new(FakeChat::seeded());
    let outlook: Arc<dyn Connector> = Arc::new(Outlook::fake(mail.clone()));
    let agenda: Arc<dyn Connector> = Arc::new(OutlookCalendar::fake(calendar.clone()));
    let flows: Arc<dyn Connector> = Arc::new(Workflows::new(
        WorkflowParts {
            mail: mail.clone(),
            calendar: calendar.clone(),
            drive: drive.clone(),
            chat: chat.clone(),
            outlook: outlook.clone(),
            agenda: agenda.clone(),
        },
        Box::new(|| Health::Ready),
        Box::new(fixed_now),
    ));
    let connectors = vec![
        outlook,
        agenda,
        Arc::new(OneDrive::fake(drive.clone())) as Arc<dyn Connector>,
        Arc::new(Teams::fake(chat.clone())),
        flows,
    ];
    FakeSuite {
        mail,
        calendar,
        drive,
        chat,
        connectors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ms365::{fixed_now, FakeMailbox, Outlook};
    use crate::ms365_cal::{FakeCalendar, OutlookCalendar};

    fn args(pairs: &[(&str, &str)]) -> Args {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    struct World {
        mail: Arc<FakeMailbox>,
        cal: Arc<FakeCalendar>,
        drive: Arc<FakeDrive>,
        chat: Arc<FakeChat>,
        flows: Workflows,
    }

    fn world() -> World {
        let mail = Arc::new(FakeMailbox::seeded());
        let cal = Arc::new(FakeCalendar::seeded());
        let drive = Arc::new(FakeDrive::seeded());
        let chat = Arc::new(FakeChat::seeded());
        let flows = Workflows::new(
            WorkflowParts {
                mail: mail.clone(),
                calendar: cal.clone(),
                drive: drive.clone(),
                chat: chat.clone(),
                outlook: Arc::new(Outlook::fake(mail.clone())),
                agenda: Arc::new(OutlookCalendar::fake(cal.clone())),
            },
            Box::new(|| Health::Ready),
            Box::new(fixed_now),
        );
        World {
            mail,
            cal,
            drive,
            chat,
            flows,
        }
    }

    #[test]
    fn teams_posts_to_the_named_channel() {
        let w = world();
        let t = Teams::fake(w.chat.clone());
        let r = t
            .execute(
                "post",
                &args(&[("text", "Tell the team channel standup moves to 11")]),
            )
            .unwrap();
        assert_eq!(r.0["message"], "Posted in team: “Standup moves to 11.”");
        assert_eq!(
            t.check(
                "post",
                &args(&[("text", "Tell the team channel standup moves to 11")])
            ),
            Ok(true)
        );
        assert!(t.undo("post", &r).is_none());
        assert!(t
            .execute("post", &args(&[("text", "tell everyone hi")]))
            .is_err());
    }

    #[test]
    fn a_folder_is_shared_with_a_person_and_unshared_by_undo() {
        let w = world();
        let d = OneDrive::fake(w.drive.clone());
        let a = args(&[
            ("text", "Share the Project folder with Priya"),
            ("with", "Priya Nair"),
        ]);
        let r = d.execute("share", &a).unwrap();
        assert_eq!(r.0["message"], "Shared the Project folder with Priya Nair.");
        assert_eq!(d.check("share", &a), Ok(true));
        assert_eq!(d.undo("share", &r), Some(Ok(())));
        assert_eq!(d.check("share", &a), Ok(false));
        let e = d
            .execute(
                "share",
                &args(&[("text", "share the folder"), ("with", "Priya")]),
            )
            .unwrap_err();
        assert!(e.to_string().starts_with("Which folder"));
    }

    #[test]
    fn the_morning_brief_joins_urgent_mail_and_todays_meetings() {
        let w = world();
        let r = w.flows.execute("morning_brief", &args(&[])).unwrap();
        let m = &r.0["message"];
        assert!(m.contains("2 urgent") && m.contains("09:30 Standup"), "{m}");
    }

    #[test]
    fn a_meeting_request_email_becomes_an_event_and_undo_removes_it() {
        let w = world();
        let r = w
            .flows
            .execute(
                "event_from_email",
                &args(&[("text", "Add this meeting request to my calendar")]),
            )
            .unwrap();
        assert_eq!(
            r.0["message"],
            "Added “Project review” on Thu 12 Nov at 15:00."
        );
        assert_eq!(r.0["attendee"], "Rahul Sharma");
        assert_eq!(w.cal.all().unwrap().len(), 7);
        assert_eq!(w.flows.undo("event_from_email", &r), Some(Ok(())));
        assert_eq!(w.cal.all().unwrap().len(), 6);
    }

    #[test]
    fn attachments_are_saved_in_the_named_folder_and_undo_deletes_them() {
        let w = world();
        let r = w
            .flows
            .execute(
                "save_attachments",
                &args(&[(
                    "text",
                    "Save the invoices from Rahul's email to the Finance folder",
                )]),
            )
            .unwrap();
        assert_eq!(
            r.0["message"],
            "Saved invoice_october.pdf to the Finance folder."
        );
        assert!(w
            .drive
            .files()
            .unwrap()
            .iter()
            .any(|f| f.name == "invoice_october.pdf" && f.folder == "Finance"));
        assert_eq!(w.flows.undo("save_attachments", &r), Some(Ok(())));
        assert!(!w
            .drive
            .files()
            .unwrap()
            .iter()
            .any(|f| f.folder == "Finance"));
        assert!(w
            .flows
            .execute("save_attachments", &args(&[("text", "save the invoices")]))
            .is_err());
    }

    #[test]
    fn an_attachment_is_forwarded_to_the_project_chat_and_the_report_is_sent() {
        let w = world();
        let r = w
            .flows
            .execute(
                "forward_attachment",
                &args(&[(
                    "text",
                    "Send the attachment from Rahul's email to the project chat",
                )]),
            )
            .unwrap();
        assert_eq!(r.0["message"], "Posted invoice_october.pdf in project.");
        assert!(w.flows.undo("forward_attachment", &r).is_none());
        let s = w
            .flows
            .execute(
                "send_report",
                &args(&[("text", "Send this week's report to my manager")]),
            )
            .unwrap();
        assert_eq!(s.0["message"], "Sent weekly_report.pdf to your manager.");
        assert!(w.mail.outgoing().unwrap()[0].sent);
        assert!(w.flows.undo("send_report", &s).is_none());
    }
}
