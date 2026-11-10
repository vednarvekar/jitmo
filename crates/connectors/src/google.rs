//! Google Workspace backends: Gmail, Google Calendar and Google Drive behind the same
//! traits as Microsoft 365 ([`MailBackend`], [`CalendarBackend`], [`DriveBackend`]).
//!
//! The mail and calendar skills talk to the connectors `outlook`, `outlook_calendar` and
//! `onedrive`; those connectors do not care which service is behind them. With a Google
//! client id saved (and no Microsoft one), the app plugs these backends in instead, so every
//! mail, calendar and file skill works on a Google account without a second set of skills.
//!
//! Sign-in is the same loopback + PKCE flow as Microsoft ([`crate::oauth`], tenant `google`).
//! One request client, [`Graph`], is reused with the label "Google".

use crate::graph::Graph;
use crate::ms365::{Mail, MailBackend, Outgoing};
use crate::ms365_cal::{CalendarBackend, Event};
use crate::ms365_work::{DriveBackend, DriveFile};
use crate::ConnError;
use base64::Engine;
use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Least-privilege scopes for what the Google backends do. Gmail's are "restricted" and
/// Calendar/Drive's "sensitive" in Google's terms; an app used by its owner needs no review.
pub fn scopes() -> Vec<String> {
    [
        "https://www.googleapis.com/auth/gmail.modify",
        "https://www.googleapis.com/auth/gmail.send",
        "https://www.googleapis.com/auth/calendar",
        "https://www.googleapis.com/auth/drive.metadata.readonly",
        "https://www.googleapis.com/auth/drive.file",
        "https://www.googleapis.com/auth/contacts.readonly",
        "https://www.googleapis.com/auth/contacts.other.readonly",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

const GMAIL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const CAL: &str = "https://www.googleapis.com/calendar/v3/calendars/primary";
const DRIVE: &str = "https://www.googleapis.com/drive/v3";
const PEOPLE: &str = "https://people.googleapis.com/v1";

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The address of a person, by exact name, from the user's own contacts (never a fuzzy
/// match, SR7). A name that already is an address is used as it is.
pub fn address_of(g: &Graph, name: &str) -> Result<String, ConnError> {
    if name.contains('@') {
        return Ok(name.to_string());
    }
    let wanted = name.trim().to_lowercase();
    for path in [
        format!(
            "{PEOPLE}/people:searchContacts?query={}&readMask=names,emailAddresses&pageSize=10",
            enc(name)
        ),
        format!(
            "{PEOPLE}/otherContacts:search?query={}&readMask=names,emailAddresses&pageSize=10",
            enc(name)
        ),
    ] {
        // The first call to a search endpoint warms its index and may come back empty.
        let v = match g.call("GET", &path, None, None) {
            Ok(v) => v,
            Err(ConnError::Failed(_)) => continue,
            Err(e) => return Err(e),
        };
        for r in v["results"].as_array().into_iter().flatten() {
            let p = &r["person"];
            let named = p["names"].as_array().into_iter().flatten().any(|n| {
                n["displayName"]
                    .as_str()
                    .is_some_and(|d| d.to_lowercase() == wanted)
            });
            if named {
                if let Some(a) = p["emailAddresses"][0]["value"].as_str() {
                    return Ok(a.to_string());
                }
            }
        }
    }
    Err(ConnError::Failed(format!(
        "I can't find an email address for {name}."
    )))
}

// ---------------------------------------------------------------------------
// Gmail
// ---------------------------------------------------------------------------

pub struct GoogleMail {
    g: Arc<Graph>,
}

impl GoogleMail {
    pub fn new(g: Arc<Graph>) -> GoogleMail {
        GoogleMail { g }
    }

    fn mail_from(v: &Value, replied: &HashSet<String>) -> Mail {
        let labels: Vec<&str> = v["labelIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let has = |l: &str| labels.contains(&l);
        let header = |name: &str| -> String {
            v["payload"]["headers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|h| {
                    h["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .and_then(|h| h["value"].as_str())
                .unwrap_or("")
                .to_string()
        };
        let (from_name, from_address) = split_address(&header("From"));
        let subject = header("Subject");
        let unread = has("UNREAD");
        let newsletter = has("CATEGORY_PROMOTIONS")
            || has("CATEGORY_UPDATES")
            || has("CATEGORY_FORUMS")
            || !header("List-Unsubscribe").is_empty()
            || subject.to_lowercase().contains("newsletter");
        let attachments: Vec<String> = v["payload"]["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["filename"].as_str())
            .filter(|f| !f.is_empty())
            .map(str::to_string)
            .collect();
        let received = v["internalDate"]
            .as_str()
            .and_then(|ms| ms.parse::<i64>().ok())
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        Mail {
            id: v["id"].as_str().unwrap_or("").to_string(),
            from_name,
            from_address,
            subject,
            body: v["snippet"].as_str().unwrap_or("").to_string(),
            received,
            unread,
            // Gmail's own importance guess, only for mail still waiting to be read.
            urgent: unread && (has("IMPORTANT") || has("STARRED")) && !newsletter,
            newsletter,
            replied: replied.contains(v["threadId"].as_str().unwrap_or("~")),
            folder: if has("INBOX") { "Inbox" } else { "Archive" }.to_string(),
            attachments,
        }
    }

    fn fetch_many(&self, ids: &[String]) -> Result<Vec<Value>, ConnError> {
        let results: Vec<Result<Value, ConnError>> = std::thread::scope(|s| {
            let handles: Vec<_> = ids
                .iter()
                .map(|id| {
                    let url = format!(
                        "{GMAIL}/messages/{}?format=full&fields=id,threadId,labelIds,snippet,internalDate,payload(headers,parts(filename))",
                        enc(id)
                    );
                    s.spawn(move || self.g.call("GET", &url, None, None))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(ConnError::Failed("A mail request failed.".into())))
                })
                .collect()
        });
        results.into_iter().collect()
    }
}

/// `"Rahul Sharma <rahul@x.com>"` -> ("Rahul Sharma", "rahul@x.com").
pub(crate) fn split_address(from: &str) -> (String, String) {
    if let (Some(a), Some(b)) = (from.find('<'), from.rfind('>')) {
        if a < b {
            let name = from[..a].trim().trim_matches('"').to_string();
            let addr = from[a + 1..b].trim().to_string();
            let name = if name.is_empty() { addr.clone() } else { name };
            return (name, addr);
        }
    }
    (from.trim().to_string(), from.trim().to_string())
}

/// A plain-text RFC 822 message, base64url encoded for the Gmail API.
fn raw_message(to_name: &str, to_address: &str, subject: &str, body: &str) -> String {
    let safe = |s: &str| s.replace(['\r', '\n'], " ");
    let subject = safe(subject);
    let encoded_subject = if subject.is_ascii() {
        subject
    } else {
        format!(
            "=?UTF-8?B?{}?=",
            base64::engine::general_purpose::STANDARD.encode(subject)
        )
    };
    let who = if to_name.contains('@') || to_name.is_empty() {
        safe(to_address)
    } else {
        format!("{} <{}>", safe(to_name).replace('"', ""), safe(to_address))
    };
    let text = format!(
        "To: {who}\r\nSubject: {encoded_subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}",
        base64::engine::general_purpose::STANDARD.encode(body)
    );
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text)
}

/// The subject for a mail with only a body: its first sentence, shortened.
fn subject_of(body: &str) -> String {
    let first = body
        .split(['.', '!', '?', '\n'])
        .find(|s| !s.trim().is_empty())
        .unwrap_or(body)
        .trim();
    let cut: String = first.chars().take(60).collect();
    cut
}

impl MailBackend for GoogleMail {
    fn all(&self) -> Result<Vec<Mail>, ConnError> {
        // Conversations the user has already answered: threads with a message in Sent.
        let sent = self.g.call(
            "GET",
            &format!("{GMAIL}/messages?labelIds=SENT&maxResults=100&fields=messages(threadId)"),
            None,
            None,
        )?;
        let replied: HashSet<String> = sent["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["threadId"].as_str().map(str::to_string))
            .collect();
        let mut out = Vec::new();
        for (label, _folder) in [("INBOX", "Inbox")] {
            let list = self.g.call(
                "GET",
                &format!("{GMAIL}/messages?labelIds={label}&maxResults=40&fields=messages(id)"),
                None,
                None,
            )?;
            let ids: Vec<String> = list["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect();
            for v in self.fetch_many(&ids)? {
                out.push(Self::mail_from(&v, &replied));
            }
        }
        // A few recent archived messages, so "find" can see mail that is no longer in the inbox.
        let list = self.g.call(
            "GET",
            &format!(
                "{GMAIL}/messages?q={}&maxResults=15&fields=messages(id)",
                enc("-in:inbox -in:sent -in:drafts -in:spam -in:trash")
            ),
            None,
            None,
        )?;
        let ids: Vec<String> = list["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().map(str::to_string))
            .collect();
        for v in self.fetch_many(&ids)? {
            out.push(Self::mail_from(&v, &replied));
        }
        Ok(out)
    }

    fn get(&self, id: &str) -> Result<Option<Mail>, ConnError> {
        match self.fetch_many(&[id.to_string()]) {
            Ok(mut v) => Ok(v.pop().map(|m| Self::mail_from(&m, &HashSet::new()))),
            Err(ConnError::Failed(m)) if m.contains("could not find") => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn move_to(&self, id: &str, folder: &str) -> Result<(), ConnError> {
        let body = match folder.to_lowercase().as_str() {
            "archive" => json!({ "removeLabelIds": ["INBOX"] }),
            "inbox" => json!({ "addLabelIds": ["INBOX"] }),
            "trash" | "deleted" => {
                self.g.call(
                    "POST",
                    &format!("{GMAIL}/messages/{}/trash", enc(id)),
                    None,
                    Some(&json!({})),
                )?;
                return Ok(());
            }
            other => {
                return Err(ConnError::Failed(format!(
                    "I can only move mail to Archive or Inbox on Google, not “{other}”."
                )))
            }
        };
        self.g.call(
            "POST",
            &format!("{GMAIL}/messages/{}/modify", enc(id)),
            None,
            Some(&body),
        )?;
        Ok(())
    }

    fn add_outgoing(
        &self,
        to: &str,
        body: &str,
        attachment: Option<&str>,
        sent: bool,
    ) -> Result<String, ConnError> {
        if attachment.is_some() {
            return Err(ConnError::Failed(
                "Attaching files to email is not connected yet.".into(),
            ));
        }
        let address = address_of(&self.g, to)?;
        let raw = raw_message(to, &address, &subject_of(body), body);
        if sent {
            let v = self.g.call(
                "POST",
                &format!("{GMAIL}/messages/send"),
                None,
                Some(&json!({ "raw": raw })),
            )?;
            Ok(v["id"].as_str().unwrap_or("sent").to_string())
        } else {
            let v = self.g.call(
                "POST",
                &format!("{GMAIL}/drafts"),
                None,
                Some(&json!({ "message": { "raw": raw } })),
            )?;
            Ok(v["id"].as_str().unwrap_or("").to_string())
        }
    }

    fn outgoing(&self) -> Result<Vec<Outgoing>, ConnError> {
        let mut out = Vec::new();
        for (label, sent) in [("SENT", true), ("DRAFT", false)] {
            let list = self.g.call(
                "GET",
                &format!("{GMAIL}/messages?labelIds={label}&maxResults=20&fields=messages(id)"),
                None,
                None,
            )?;
            let ids: Vec<String> = list["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect();
            for v in self.fetch_many_to(&ids)? {
                let to = v["payload"]["headers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|h| {
                        h["name"]
                            .as_str()
                            .is_some_and(|n| n.eq_ignore_ascii_case("To"))
                    })
                    .and_then(|h| h["value"].as_str())
                    .map(|t| split_address(t).0)
                    .unwrap_or_default();
                out.push(Outgoing {
                    id: v["id"].as_str().unwrap_or("").to_string(),
                    to,
                    body: v["snippet"].as_str().unwrap_or("").to_string(),
                    attachment: None,
                    sent,
                });
            }
        }
        Ok(out)
    }

    fn delete_outgoing(&self, id: &str) -> Result<(), ConnError> {
        // A draft id deletes the draft; a message id moves the message to the bin.
        if self
            .g
            .call("DELETE", &format!("{GMAIL}/drafts/{}", enc(id)), None, None)
            .is_ok()
        {
            return Ok(());
        }
        self.g.call(
            "POST",
            &format!("{GMAIL}/messages/{}/trash", enc(id)),
            None,
            Some(&json!({})),
        )?;
        Ok(())
    }
}

impl GoogleMail {
    /// Headers and snippet only (for the sent and drafts lists).
    fn fetch_many_to(&self, ids: &[String]) -> Result<Vec<Value>, ConnError> {
        let results: Vec<Result<Value, ConnError>> = std::thread::scope(|s| {
            let handles: Vec<_> = ids
                .iter()
                .map(|id| {
                    let url = format!(
                        "{GMAIL}/messages/{}?format=metadata&metadataHeaders=To&fields=id,snippet,payload(headers)",
                        enc(id)
                    );
                    s.spawn(move || self.g.call("GET", &url, None, None))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(ConnError::Failed("A mail request failed.".into())))
                })
                .collect()
        });
        results.into_iter().collect()
    }
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

pub struct GoogleCalendar {
    g: Arc<Graph>,
    now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
}

impl GoogleCalendar {
    pub fn new(g: Arc<Graph>) -> GoogleCalendar {
        GoogleCalendar {
            g,
            now: Box::new(Local::now),
        }
    }

    pub fn with_now(mut self, now: DateTime<Local>) -> GoogleCalendar {
        self.now = Box::new(move || now);
        self
    }

    fn get_event(&self, id: &str) -> Result<Value, ConnError> {
        self.g
            .call("GET", &format!("{CAL}/events/{}", enc(id)), None, None)
    }

    fn patch(&self, id: &str, body: &Value) -> Result<(), ConnError> {
        self.g.call(
            "PATCH",
            &format!("{CAL}/events/{}?sendUpdates=all", enc(id)),
            None,
            Some(body),
        )?;
        Ok(())
    }

    /// Answer an invitation as the user ("accepted", "declined").
    fn respond(&self, id: &str, answer: &str) -> Result<(), ConnError> {
        let ev = self.get_event(id)?;
        let mut people: Vec<Value> = ev["attendees"].as_array().cloned().unwrap_or_default();
        let mut found = false;
        for p in &mut people {
            if p["self"].as_bool().unwrap_or(false) {
                p["responseStatus"] = json!(answer);
                found = true;
            }
        }
        if !found {
            return Err(ConnError::Failed(
                "That event has no invitation for you to answer.".into(),
            ));
        }
        self.patch(id, &json!({ "attendees": people }))
    }
}

fn rfc3339_local(t: NaiveDateTime) -> String {
    Local
        .from_local_datetime(&t)
        .earliest()
        .map_or_else(|| t.and_utc().to_rfc3339(), |l| l.to_rfc3339())
}

fn parse_when(v: &Value) -> Option<NaiveDateTime> {
    if let Some(dt) = v["dateTime"].as_str() {
        return DateTime::parse_from_rfc3339(dt)
            .ok()
            .map(|t| t.with_timezone(&Local).naive_local());
    }
    // An all-day event: midnight.
    v["date"].as_str().and_then(|d| {
        NaiveDateTime::parse_from_str(&format!("{d} 00:00:00"), "%Y-%m-%d %H:%M:%S").ok()
    })
}

impl CalendarBackend for GoogleCalendar {
    fn all(&self) -> Result<Vec<Event>, ConnError> {
        let now = (self.now)();
        let from = (now - Duration::days(1)).with_timezone(&Utc).to_rfc3339();
        let to = (now + Duration::days(21)).with_timezone(&Utc).to_rfc3339();
        let v = self.g.call(
            "GET",
            &format!(
                "{CAL}/events?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=100&fields=items(id,summary,start,end,attendees,organizer,status)",
                enc(&from),
                enc(&to)
            ),
            None,
            None,
        )?;
        Ok(v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["status"].as_str() != Some("cancelled"))
            .filter_map(|e| {
                let me = e["attendees"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|a| a["self"].as_bool().unwrap_or(false));
                let response = me
                    .and_then(|a| a["responseStatus"].as_str())
                    .unwrap_or("accepted");
                Some(Event {
                    id: e["id"].as_str()?.to_string(),
                    title: e["summary"].as_str().unwrap_or("(no title)").to_string(),
                    start: parse_when(&e["start"])?,
                    end: parse_when(&e["end"])?,
                    attendees: e["attendees"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|a| !a["self"].as_bool().unwrap_or(false))
                        .filter_map(|a| {
                            a["displayName"]
                                .as_str()
                                .or_else(|| a["email"].as_str())
                                .map(str::to_string)
                        })
                        .collect(),
                    invite_pending: response == "needsAction"
                        && !e["organizer"]["self"].as_bool().unwrap_or(false),
                    declined: response == "declined",
                })
            })
            .collect())
    }

    fn add(
        &self,
        title: &str,
        start: NaiveDateTime,
        end: NaiveDateTime,
        attendees: &[String],
    ) -> Result<String, ConnError> {
        let mut people = Vec::new();
        for a in attendees {
            people.push(json!({ "email": address_of(&self.g, a)?, "displayName": a }));
        }
        let v = self.g.call(
            "POST",
            &format!("{CAL}/events?sendUpdates=all"),
            None,
            Some(&json!({
                "summary": title,
                "start": { "dateTime": rfc3339_local(start) },
                "end": { "dateTime": rfc3339_local(end) },
                "attendees": people,
            })),
        )?;
        Ok(v["id"].as_str().unwrap_or("").to_string())
    }

    fn remove(&self, id: &str) -> Result<(), ConnError> {
        self.g.call(
            "DELETE",
            &format!("{CAL}/events/{}?sendUpdates=all", enc(id)),
            None,
            None,
        )?;
        Ok(())
    }

    fn reschedule(
        &self,
        id: &str,
        start: NaiveDateTime,
        end: NaiveDateTime,
    ) -> Result<(), ConnError> {
        self.patch(
            id,
            &json!({
                "start": { "dateTime": rfc3339_local(start) },
                "end": { "dateTime": rfc3339_local(end) },
            }),
        )
    }

    fn set_declined(&self, id: &str, declined: bool) -> Result<(), ConnError> {
        self.respond(id, if declined { "declined" } else { "needsAction" })
    }

    fn accept(&self, id: &str) -> Result<(), ConnError> {
        self.respond(id, "accepted")
    }

    fn add_attendee(&self, id: &str, who: &str) -> Result<(), ConnError> {
        let email = address_of(&self.g, who)?;
        let ev = self.get_event(id)?;
        let mut people: Vec<Value> = ev["attendees"].as_array().cloned().unwrap_or_default();
        if !people.iter().any(|p| {
            p["email"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(&email))
        }) {
            people.push(json!({ "email": email, "displayName": who }));
        }
        self.patch(id, &json!({ "attendees": people }))
    }

    fn remove_attendee(&self, id: &str, who: &str) -> Result<(), ConnError> {
        let ev = self.get_event(id)?;
        let wanted = who.to_lowercase();
        let people: Vec<Value> = ev["attendees"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| {
                p["displayName"].as_str().map(str::to_lowercase).as_deref() != Some(&wanted)
                    && p["email"].as_str().map(str::to_lowercase).as_deref() != Some(&wanted)
            })
            .collect();
        self.patch(id, &json!({ "attendees": people }))
    }

    fn busy_of(&self, person: &str) -> Result<Vec<(NaiveDateTime, NaiveDateTime)>, ConnError> {
        let email = address_of(&self.g, person)?;
        let now = (self.now)();
        let v = self.g.call(
            "POST",
            "https://www.googleapis.com/calendar/v3/freeBusy",
            None,
            Some(&json!({
                "timeMin": (now - Duration::days(1)).with_timezone(&Utc).to_rfc3339(),
                "timeMax": (now + Duration::days(14)).with_timezone(&Utc).to_rfc3339(),
                "items": [{ "id": email }],
            })),
        )?;
        Ok(v["calendars"][&email]["busy"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| {
                Some((
                    parse_when(&json!({"dateTime": b["start"]}))?,
                    parse_when(&json!({"dateTime": b["end"]}))?,
                ))
            })
            .collect())
    }

    fn people(&self) -> Result<Vec<String>, ConnError> {
        let v = self.g.call(
            "GET",
            &format!("{PEOPLE}/people/me/connections?pageSize=100&personFields=names"),
            None,
            None,
        )?;
        Ok(v["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["names"][0]["displayName"].as_str().map(str::to_string))
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Drive
// ---------------------------------------------------------------------------

pub struct GoogleDrive {
    g: Arc<Graph>,
}

impl GoogleDrive {
    pub fn new(g: Arc<Graph>) -> GoogleDrive {
        GoogleDrive { g }
    }

    fn folder_id(&self, name: &str) -> Result<String, ConnError> {
        let q = format!(
            "mimeType='application/vnd.google-apps.folder' and name='{}' and trashed=false",
            name.replace('\'', "\\'")
        );
        let v = self.g.call(
            "GET",
            &format!(
                "{DRIVE}/files?q={}&fields=files(id,name)&pageSize=5",
                enc(&q)
            ),
            None,
            None,
        )?;
        v["files"][0]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| ConnError::Failed(format!("I can't find a folder called {name}.")))
    }
}

impl DriveBackend for GoogleDrive {
    fn files(&self) -> Result<Vec<DriveFile>, ConnError> {
        let v = self.g.call(
            "GET",
            &format!(
                "{DRIVE}/files?q={}&orderBy=modifiedTime%20desc&pageSize=50&fields=files(id,name)",
                enc("mimeType!='application/vnd.google-apps.folder' and trashed=false")
            ),
            None,
            None,
        )?;
        Ok(v["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                Some(DriveFile {
                    id: f["id"].as_str()?.to_string(),
                    name: f["name"].as_str()?.to_string(),
                    folder: "My Drive".to_string(),
                })
            })
            .collect())
    }

    fn folders(&self) -> Result<Vec<String>, ConnError> {
        let v = self.g.call(
            "GET",
            &format!(
                "{DRIVE}/files?q={}&pageSize=100&fields=files(name)",
                enc("mimeType='application/vnd.google-apps.folder' and trashed=false")
            ),
            None,
            None,
        )?;
        Ok(v["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["name"].as_str().map(str::to_string))
            .collect())
    }

    fn save(&self, _name: &str, _folder: &str) -> Result<String, ConnError> {
        Err(ConnError::Failed(
            "Saving attachments to Google Drive is not connected yet.".into(),
        ))
    }

    fn delete(&self, id: &str) -> Result<(), ConnError> {
        self.g.call(
            "PATCH",
            &format!("{DRIVE}/files/{}", enc(id)),
            None,
            Some(&json!({ "trashed": true })),
        )?;
        Ok(())
    }

    fn share(&self, folder: &str, with: &str) -> Result<String, ConnError> {
        let id = self.folder_id(folder)?;
        let email = address_of(&self.g, with)?;
        self.g.call(
            "POST",
            &format!(
                "{DRIVE}/files/{}/permissions?sendNotificationEmail=true",
                enc(&id)
            ),
            None,
            Some(&json!({ "role": "reader", "type": "user", "emailAddress": email })),
        )?;
        let v = self.g.call(
            "GET",
            &format!("{DRIVE}/files/{}?fields=webViewLink", enc(&id)),
            None,
            None,
        )?;
        Ok(v["webViewLink"].as_str().unwrap_or("").to_string())
    }

    fn unshare(&self, folder: &str, with: &str) -> Result<(), ConnError> {
        let id = self.folder_id(folder)?;
        let email = address_of(&self.g, with)?;
        let v = self.g.call(
            "GET",
            &format!(
                "{DRIVE}/files/{}/permissions?fields=permissions(id,emailAddress)",
                enc(&id)
            ),
            None,
            None,
        )?;
        for p in v["permissions"].as_array().into_iter().flatten() {
            if p["emailAddress"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(&email))
            {
                if let Some(pid) = p["id"].as_str() {
                    self.g.call(
                        "DELETE",
                        &format!("{DRIVE}/files/{}/permissions/{}", enc(&id), enc(pid)),
                        None,
                        None,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn shares(&self) -> Result<Vec<(String, String)>, ConnError> {
        Ok(Vec::new())
    }
}

/// The text of a Google Doc, for "explain this document" and study help.
pub fn doc_text(g: &Graph, file_id: &str) -> Result<String, ConnError> {
    let url = format!(
        "{DRIVE}/files/{}/export?mimeType=text%2Fplain",
        enc(file_id)
    );
    // The export endpoint answers plain text, not JSON; ask through the file metadata
    // instead when the body is not JSON.
    match g.call("GET", &url, None, None) {
        Ok(v) => Ok(v.as_str().unwrap_or("").to_string()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Http;
    use crate::oauth::{MemoryTokenStore, OAuthConfig, Session, TokenStore, Transport};
    use std::sync::Mutex;

    struct Stub {
        routes: Vec<(&'static str, u16, String)>,
        seen: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl Http for Stub {
        fn send(
            &self,
            method: &str,
            url: &str,
            _bearer: &str,
            _prefer: Option<&str>,
            body: Option<&str>,
        ) -> Result<(u16, String), String> {
            self.seen
                .lock()
                .unwrap()
                .push((method.into(), url.into(), body.map(str::to_string)));
            for (needle, status, text) in &self.routes {
                if url.contains(needle) {
                    return Ok((*status, text.clone()));
                }
            }
            Ok((404, "{}".into()))
        }
    }

    struct NoNet;
    impl Transport for NoNet {
        fn post_form(&self, _: &str, _: &[(&str, &str)]) -> Result<(u16, String), String> {
            Err("offline".into())
        }
    }

    fn graph(routes: Vec<(&'static str, u16, String)>) -> (Arc<Graph>, Arc<Stub>) {
        let store = MemoryTokenStore::default();
        store.save("refresh").unwrap();
        // A token that is already valid: the session never needs the network.
        let session = Arc::new(Session::new(
            OAuthConfig {
                client_id: "id".into(),
                tenant: crate::oauth::GOOGLE.into(),
                redirect_uri: String::new(),
                scopes: scopes(),
                client_secret: Some("secret".into()),
            },
            Box::new(OkToken),
            Box::new(store),
        ));
        let stub = Arc::new(Stub {
            routes,
            seen: Mutex::new(Vec::new()),
        });
        (
            Arc::new(Graph::new(stub.clone(), session).labelled("Google")),
            stub,
        )
    }

    struct OkToken;
    impl Transport for OkToken {
        fn post_form(&self, _: &str, _: &[(&str, &str)]) -> Result<(u16, String), String> {
            Ok((
                200,
                r#"{"access_token":"tok","expires_in":3600}"#.to_string(),
            ))
        }
    }

    fn message(id: &str, labels: &str, from: &str, subject: &str) -> String {
        format!(
            r#"{{"id":"{id}","threadId":"t{id}","labelIds":{labels},"snippet":"hello there","internalDate":"1762680000000","payload":{{"headers":[{{"name":"From","value":"{from}"}},{{"name":"Subject","value":"{subject}"}}],"parts":[{{"filename":"budget.xlsx"}},{{"filename":""}}]}}}}"#
        )
    }

    #[test]
    fn splits_addresses() {
        assert_eq!(
            split_address("Rahul Sharma <rahul@x.com>"),
            ("Rahul Sharma".to_string(), "rahul@x.com".to_string())
        );
        assert_eq!(
            split_address("plain@x.com"),
            ("plain@x.com".to_string(), "plain@x.com".to_string())
        );
    }

    #[test]
    fn a_message_becomes_a_mail() {
        let v: Value = serde_json::from_str(&message(
            "m1",
            r#"["INBOX","UNREAD","IMPORTANT"]"#,
            "Rahul Sharma <rahul@x.com>",
            "Budget",
        ))
        .unwrap();
        let m = GoogleMail::mail_from(&v, &HashSet::new());
        assert_eq!(m.from_name, "Rahul Sharma");
        assert!(m.unread && m.urgent && !m.newsletter);
        assert_eq!(m.folder, "Inbox");
        assert_eq!(m.attachments, vec!["budget.xlsx".to_string()]);
        assert!(!m.received.is_empty());
    }

    #[test]
    fn promotions_are_newsletters_and_never_urgent() {
        let v: Value = serde_json::from_str(&message(
            "m2",
            r#"["INBOX","UNREAD","IMPORTANT","CATEGORY_PROMOTIONS"]"#,
            "Tech Weekly <hi@tw.com>",
            "This week",
        ))
        .unwrap();
        let m = GoogleMail::mail_from(&v, &HashSet::new());
        assert!(m.newsletter && !m.urgent);
    }

    #[test]
    fn the_inbox_is_read_with_replies_marked() {
        let (g, _) = graph(vec![
            (
                "labelIds=SENT&maxResults=100",
                200,
                r#"{"messages":[{"threadId":"tm1"}]}"#.into(),
            ),
            (
                "labelIds=INBOX",
                200,
                r#"{"messages":[{"id":"m1"}]}"#.into(),
            ),
            ("/messages?q=", 200, "{}".into()),
            (
                "/messages/m1",
                200,
                message("m1", r#"["INBOX"]"#, "Priya <p@x.com>", "Hi"),
            ),
        ]);
        let mail = GoogleMail::new(g).all().unwrap();
        assert_eq!(mail.len(), 1);
        assert!(mail[0].replied);
    }

    #[test]
    fn a_send_needs_an_exact_contact_and_uses_a_short_subject() {
        let (g, stub) = graph(vec![
            (
                "people:searchContacts",
                200,
                r#"{"results":[{"person":{"names":[{"displayName":"Rahul Sharma"}],"emailAddresses":[{"value":"rahul@x.com"}]}}]}"#.into(),
            ),
            ("/messages/send", 200, r#"{"id":"sent1"}"#.into()),
        ]);
        let id = GoogleMail::new(g.clone())
            .add_outgoing("Rahul Sharma", "The report is ready. See you.", None, true)
            .unwrap();
        assert_eq!(id, "sent1");
        let seen = stub.seen.lock().unwrap();
        let send = seen
            .iter()
            .find(|(_, u, _)| u.ends_with("/messages/send"))
            .unwrap();
        let raw = serde_json::from_str::<Value>(send.2.as_ref().unwrap()).unwrap()["raw"]
            .as_str()
            .unwrap()
            .to_string();
        let text = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(raw)
                .unwrap(),
        )
        .unwrap();
        assert!(text.contains("To: Rahul Sharma <rahul@x.com>"));
        assert!(text.contains("Subject: The report is ready"));
        drop(seen);
        // A name that is not in the contacts is refused, never guessed.
        let (g2, _) = graph(vec![(
            "people:searchContacts",
            200,
            r#"{"results":[]}"#.into(),
        )]);
        assert!(GoogleMail::new(g2)
            .add_outgoing("Nobody", "hi", None, true)
            .is_err());
    }

    #[test]
    fn archiving_removes_the_inbox_label() {
        let (g, stub) = graph(vec![("/modify", 200, "{}".into())]);
        GoogleMail::new(g).move_to("m1", "Archive").unwrap();
        let seen = stub.seen.lock().unwrap();
        assert!(seen[0].2.as_ref().unwrap().contains("removeLabelIds"));
    }

    #[test]
    fn events_are_read_with_invites_and_declines() {
        let (g, _) = graph(vec![(
            "/events?timeMin",
            200,
            r#"{"items":[
              {"id":"e1","summary":"Review","start":{"dateTime":"2026-11-10T15:00:00+05:30"},"end":{"dateTime":"2026-11-10T16:00:00+05:30"},
               "organizer":{"self":false},"attendees":[{"email":"a@x.com","displayName":"Asha"},{"email":"me@x.com","self":true,"responseStatus":"needsAction"}]},
              {"id":"e2","summary":"Gym","start":{"date":"2026-11-11"},"end":{"date":"2026-11-12"},
               "organizer":{"self":true}},
              {"id":"e3","summary":"Old","status":"cancelled","start":{"date":"2026-11-11"},"end":{"date":"2026-11-12"}}
            ]}"#
                .into(),
        )]);
        let cal = GoogleCalendar::new(g);
        let ev = cal.all().unwrap();
        assert_eq!(ev.len(), 2);
        assert!(ev[0].invite_pending && !ev[0].declined);
        assert_eq!(ev[0].attendees, vec!["Asha".to_string()]);
        assert!(!ev[1].invite_pending);
    }

    #[test]
    fn accepting_changes_only_my_answer() {
        let (g, stub) = graph(vec![
            (
                "/events/e1",
                200,
                r#"{"id":"e1","attendees":[{"email":"a@x.com"},{"email":"me@x.com","self":true,"responseStatus":"needsAction"}]}"#.into(),
            ),
        ]);
        GoogleCalendar::new(g).accept("e1").unwrap();
        let seen = stub.seen.lock().unwrap();
        let patch = seen.iter().find(|(m, _, _)| m == "PATCH").unwrap();
        assert!(patch.1.contains("sendUpdates=all"));
        let body: Value = serde_json::from_str(patch.2.as_ref().unwrap()).unwrap();
        assert_eq!(body["attendees"][1]["responseStatus"], "accepted");
        assert!(body["attendees"][0].get("responseStatus").is_none());
    }

    #[test]
    fn a_google_401_asks_to_sign_in_to_google() {
        let (g, _) = graph(vec![("labelIds=SENT", 401, "{}".into())]);
        let e = GoogleMail::new(g).all().unwrap_err();
        assert!(e.to_string().contains("Google"), "{e}");
    }

    #[test]
    fn the_sign_in_urls_are_googles() {
        let cfg = OAuthConfig {
            client_id: "abc".into(),
            tenant: crate::oauth::GOOGLE.into(),
            redirect_uri: "http://127.0.0.1:5000/".into(),
            scopes: scopes(),
            client_secret: Some("s".into()),
        };
        let url = cfg.authorize_url("st", "ch");
        assert!(url.starts_with("https://accounts.google.com/"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("code_challenge_method=S256"));
        assert_eq!(cfg.token_url(), "https://oauth2.googleapis.com/token");
        let _ = NoNet;
    }
}
