//! Microsoft Graph backends (Phase 3, Track A): the real mail and calendar behind
//! [`MailBackend`] and [`CalendarBackend`], so the `outlook` and `outlook_calendar`
//! connectors run against a signed-in account instead of the seeded fakes.
//!
//! [`Graph`] adds the bearer token from a [`Session`] (refreshing it when it is about to
//! expire), sends the request over an [`Http`] (real HTTPS in the app, a recorded answer in
//! the tests) and turns the failures people see into the usual connector errors: 401 is
//! "please sign in again", no answer is offline, anything else is a short message.
//!
//! Scope of this file: mail (read, move, draft, send) and calendar (read, create, move,
//! respond, free/busy). OneDrive and Teams are not connected yet: [`NoDrive`] and [`NoChat`]
//! say so plainly, and the three flows that need them (save attachments, forward to a chat,
//! send the weekly report) answer with that message.

use crate::ms365::{Mail, MailBackend, Outgoing};
use crate::ms365_cal::{CalendarBackend, Event};
use crate::ms365_work::{ChatBackend, DriveBackend, DriveFile};
use crate::oauth::Session;
use crate::ConnError;
use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

const BASE: &str = "https://graph.microsoft.com/v1.0";

/// Sends one HTTPS request. The real one is [`UreqHttp`].
pub trait Http: Send + Sync {
    /// Returns the status and body; `Err` means there was no answer at all.
    fn send(
        &self,
        method: &str,
        url: &str,
        bearer: &str,
        prefer: Option<&str>,
        body: Option<&str>,
    ) -> Result<(u16, String), String>;
}

pub struct UreqHttp;

impl Http for UreqHttp {
    fn send(
        &self,
        method: &str,
        url: &str,
        bearer: &str,
        prefer: Option<&str>,
        body: Option<&str>,
    ) -> Result<(u16, String), String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .provider(ureq::tls::TlsProvider::NativeTls)
                    .build(),
            )
            .timeout_global(Some(std::time::Duration::from_secs(20)))
            .build()
            .into();
        let mut b = ureq::http::Request::builder()
            .method(method)
            .uri(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Accept", "application/json");
        if let Some(p) = prefer {
            b = b.header("Prefer", p);
        }
        let req = if let Some(body) = body {
            b.header("Content-Type", "application/json")
                .body(body.to_string())
        } else {
            b.body(String::new())
        }
        .map_err(|e| e.to_string())?;
        let mut resp = agent.run(req).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok((status, text))
    }
}

/// An authorised Graph client.
pub struct Graph {
    http: Arc<dyn Http>,
    session: Arc<Session>,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
    base: String,
}

fn epoch_now() -> i64 {
    Utc::now().timestamp()
}

impl Graph {
    pub fn new(http: Arc<dyn Http>, session: Arc<Session>) -> Graph {
        Graph {
            http,
            session,
            now: Box::new(epoch_now),
            base: BASE.into(),
        }
    }

    #[cfg(test)]
    fn with_clock(mut self, now: i64) -> Graph {
        self.now = Box::new(move || now);
        self
    }

    /// Call Graph. `path` starts with `/me/...` (or is a full URL for a paged link).
    pub fn call(
        &self,
        method: &str,
        path: &str,
        prefer: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, ConnError> {
        let token = self.session.access_token((self.now)())?;
        let url = if path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{path}", self.base)
        };
        let body = body.map(Value::to_string);
        let (status, text) = self
            .http
            .send(method, &url, &token, prefer, body.as_deref())
            .map_err(|_| ConnError::Offline("Microsoft 365".into()))?;
        match status {
            200..=299 => {
                if text.trim().is_empty() {
                    Ok(Value::Null)
                } else {
                    serde_json::from_str(&text).map_err(|_| {
                        ConnError::Failed("Microsoft 365 sent an answer I can't read.".into())
                    })
                }
            }
            401 => {
                self.session.sign_out();
                Err(ConnError::NotSignedIn("Microsoft 365".into()))
            }
            403 => Err(ConnError::Failed(
                "Microsoft 365 did not allow that. The app may need another permission.".into(),
            )),
            404 => Err(ConnError::Failed(
                "Microsoft 365 could not find that.".into(),
            )),
            429 | 500..=599 => Err(ConnError::Offline("Microsoft 365".into())),
            other => {
                let msg = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                    .unwrap_or_default();
                Err(ConnError::Failed(format!(
                    "Microsoft 365 refused that ({other}). {msg}"
                )))
            }
        }
    }

    fn get(&self, path: &str) -> Result<Value, ConnError> {
        self.call("GET", path, None, None)
    }

    /// The address of a person, by exact name, from the user's own people list (SR7: never a
    /// fuzzy match). `Err` says so when there is none.
    pub fn address_of(&self, name: &str) -> Result<String, ConnError> {
        let v = self.get(&format!(
            "/me/people?$search=%22{}%22&$top=10&$select=displayName,scoredEmailAddresses",
            enc(name)
        ))?;
        let wanted = name.trim().to_lowercase();
        v["value"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| {
                p["displayName"]
                    .as_str()
                    .is_some_and(|d| d.to_lowercase() == wanted)
            })
            .and_then(|p| p["scoredEmailAddresses"][0]["address"].as_str())
            .map(str::to_string)
            .ok_or_else(|| ConnError::Failed(format!("I can't find an email address for {name}.")))
    }
}

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

// ---------------------------------------------------------------------------
// Times
// ---------------------------------------------------------------------------

/// Graph's `2026-11-10T09:30:00.0000000` (UTC, as asked with `Prefer: outlook.timezone`).
fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    let core = s.split('.').next().unwrap_or(s).trim_end_matches('Z');
    NaiveDateTime::parse_from_str(core, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|n| Utc.from_utc_datetime(&n))
}

fn to_local(s: &str) -> Option<NaiveDateTime> {
    parse_utc(s).map(|t| t.with_timezone(&Local).naive_local())
}

fn from_local(t: NaiveDateTime) -> String {
    Local
        .from_local_datetime(&t)
        .earliest()
        .map_or_else(|| t.and_utc(), |l| l.with_timezone(&Utc))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

const UTC: &str = "outlook.timezone=\"UTC\"";

// ---------------------------------------------------------------------------
// Mail
// ---------------------------------------------------------------------------

pub struct GraphMail {
    g: Arc<Graph>,
}

impl GraphMail {
    pub fn new(g: Arc<Graph>) -> GraphMail {
        GraphMail { g }
    }

    /// Ids of the folders messages are put in, by the name the connector uses.
    fn folder_id(&self, name: &str) -> Result<String, ConnError> {
        let well_known = match name.to_lowercase().as_str() {
            "inbox" => Some("inbox"),
            "archive" => Some("archive"),
            "drafts" => Some("drafts"),
            "sent" | "sent items" => Some("sentitems"),
            "deleted" | "deleted items" | "trash" => Some("deleteditems"),
            _ => None,
        };
        if let Some(w) = well_known {
            return Ok(w.to_string());
        }
        let v = self.g.get(&format!(
            "/me/mailFolders?$filter=displayName%20eq%20%27{}%27&$select=id",
            enc(name)
        ))?;
        v["value"][0]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| ConnError::Failed(format!("I can't find a folder called {name}.")))
    }

    fn mail_from(&self, v: &Value, folder: &str, replied: &HashSet<String>) -> Mail {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let subject = s("subject");
        let from_name = v["from"]["emailAddress"]["name"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let from_address = v["from"]["emailAddress"]["address"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let conversation = s("conversationId");
        let unread = !v["isRead"].as_bool().unwrap_or(true);
        // Newsletters: Microsoft sorts bulk mail into "Other"; the subject often says so.
        let newsletter = subject.to_lowercase().contains("newsletter")
            || v["inferenceClassification"].as_str() == Some("other") && unread;
        Mail {
            id: s("id"),
            from_name,
            from_address,
            subject,
            body: s("bodyPreview"),
            received: s("receivedDateTime").get(..10).unwrap_or("").to_string(),
            unread,
            urgent: v["importance"].as_str() == Some("high"),
            newsletter,
            replied: replied.contains(&conversation),
            folder: folder.to_string(),
            attachments: if v["hasAttachments"].as_bool().unwrap_or(false) {
                vec!["attachment".to_string()]
            } else {
                Vec::new()
            },
        }
    }
}

const MAIL_FIELDS: &str = "id,subject,from,bodyPreview,receivedDateTime,isRead,importance,hasAttachments,conversationId,inferenceClassification";

impl MailBackend for GraphMail {
    fn all(&self) -> Result<Vec<Mail>, ConnError> {
        // The conversations the user has already answered: those with a message in Sent Items.
        let sent = self
            .g
            .get("/me/mailFolders/sentitems/messages?$top=100&$select=conversationId")?;
        let replied: HashSet<String> = sent["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["conversationId"].as_str().map(str::to_string))
            .collect();
        let mut out = Vec::new();
        for (folder, path) in [("Inbox", "inbox"), ("Archive", "archive")] {
            let v = self.g.get(&format!(
                "/me/mailFolders/{path}/messages?$top=50&$orderby=receivedDateTime%20desc&$select={MAIL_FIELDS}"
            ))?;
            for m in v["value"].as_array().into_iter().flatten() {
                out.push(self.mail_from(m, folder, &replied));
            }
        }
        Ok(out)
    }

    fn get(&self, id: &str) -> Result<Option<Mail>, ConnError> {
        let v = match self.g.get(&format!(
            "/me/messages/{}?$select={MAIL_FIELDS},parentFolderId",
            enc(id)
        )) {
            Ok(v) => v,
            Err(ConnError::Failed(m)) if m.contains("could not find") => return Ok(None),
            Err(e) => return Err(e),
        };
        let parent = v["parentFolderId"].as_str().unwrap_or("");
        let folder = if parent == self.folder_id("archive").unwrap_or_default() {
            "Archive"
        } else if parent == self.folder_id("inbox").unwrap_or_default() {
            "Inbox"
        } else {
            "Other"
        };
        // The folder ids come back as opaque strings: compare against the real ones.
        let inbox = self.g.get("/me/mailFolders/inbox?$select=id")?;
        let archive = self.g.get("/me/mailFolders/archive?$select=id")?;
        let folder = if parent == archive["id"].as_str().unwrap_or("~") {
            "Archive"
        } else if parent == inbox["id"].as_str().unwrap_or("~") {
            "Inbox"
        } else {
            folder
        };
        Ok(Some(self.mail_from(&v, folder, &HashSet::new())))
    }

    fn move_to(&self, id: &str, folder: &str) -> Result<(), ConnError> {
        let dest = self.folder_id(folder)?;
        self.g.call(
            "POST",
            &format!("/me/messages/{}/move", enc(id)),
            None,
            Some(&json!({ "destinationId": dest })),
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
        // Exact name only (SR7); a name that is already an address is used as it is.
        let address = if to.contains('@') {
            to.to_string()
        } else {
            self.g.address_of(to)?
        };
        let message = json!({
            "subject": body.chars().take(60).collect::<String>(),
            "body": { "contentType": "Text", "content": body },
            "toRecipients": [{ "emailAddress": { "address": address, "name": to } }],
        });
        if sent {
            self.g.call(
                "POST",
                "/me/sendMail",
                None,
                Some(&json!({ "message": message, "saveToSentItems": true })),
            )?;
            Ok("sent".into())
        } else {
            let v = self.g.call("POST", "/me/messages", None, Some(&message))?;
            Ok(v["id"].as_str().unwrap_or("").to_string())
        }
    }

    fn outgoing(&self) -> Result<Vec<Outgoing>, ConnError> {
        let mut out = Vec::new();
        for (path, sent) in [("sentitems", true), ("drafts", false)] {
            let v = self.g.get(&format!(
                "/me/mailFolders/{path}/messages?$top=20&$select=id,toRecipients,bodyPreview,hasAttachments"
            ))?;
            for m in v["value"].as_array().into_iter().flatten() {
                out.push(Outgoing {
                    id: m["id"].as_str().unwrap_or("").to_string(),
                    to: m["toRecipients"][0]["emailAddress"]["name"]
                        .as_str()
                        .or_else(|| m["toRecipients"][0]["emailAddress"]["address"].as_str())
                        .unwrap_or("")
                        .to_string(),
                    body: m["bodyPreview"].as_str().unwrap_or("").to_string(),
                    attachment: m["hasAttachments"]
                        .as_bool()
                        .unwrap_or(false)
                        .then(|| "attachment".to_string()),
                    sent,
                });
            }
        }
        Ok(out)
    }

    fn delete_outgoing(&self, id: &str) -> Result<(), ConnError> {
        self.g
            .call("DELETE", &format!("/me/messages/{}", enc(id)), None, None)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

pub struct GraphCalendar {
    g: Arc<Graph>,
    now: Box<dyn Fn() -> DateTime<Local> + Send + Sync>,
}

impl GraphCalendar {
    pub fn new(g: Arc<Graph>) -> GraphCalendar {
        GraphCalendar {
            g,
            now: Box::new(Local::now),
        }
    }

    #[cfg(test)]
    fn with_now(mut self, now: DateTime<Local>) -> GraphCalendar {
        self.now = Box::new(move || now);
        self
    }
}

impl CalendarBackend for GraphCalendar {
    fn all(&self) -> Result<Vec<Event>, ConnError> {
        let now = (self.now)();
        let from = (now - Duration::days(1))
            .with_timezone(&Utc)
            .format("%Y-%m-%dT%H:%M:%S");
        let to = (now + Duration::days(21))
            .with_timezone(&Utc)
            .format("%Y-%m-%dT%H:%M:%S");
        let v = self.g.call(
            "GET",
            &format!(
                "/me/calendarView?startDateTime={from}&endDateTime={to}&$top=100&$orderby=start/dateTime&$select=id,subject,start,end,attendees,responseStatus,isOrganizer"
            ),
            Some(UTC),
            None,
        )?;
        Ok(v["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let response = e["responseStatus"]["response"].as_str().unwrap_or("");
                Some(Event {
                    id: e["id"].as_str()?.to_string(),
                    title: e["subject"].as_str().unwrap_or("(no title)").to_string(),
                    start: to_local(e["start"]["dateTime"].as_str()?)?,
                    end: to_local(e["end"]["dateTime"].as_str()?)?,
                    attendees: e["attendees"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|a| a["emailAddress"]["name"].as_str().map(str::to_string))
                        .collect(),
                    invite_pending: response == "notResponded"
                        && !e["isOrganizer"].as_bool().unwrap_or(false),
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
            let address = if a.contains('@') {
                a.clone()
            } else {
                self.g.address_of(a)?
            };
            people.push(
                json!({ "emailAddress": { "address": address, "name": a }, "type": "required" }),
            );
        }
        let v = self.g.call(
            "POST",
            "/me/events",
            None,
            Some(&json!({
                "subject": title,
                "start": { "dateTime": from_local(start), "timeZone": "UTC" },
                "end": { "dateTime": from_local(end), "timeZone": "UTC" },
                "attendees": people,
            })),
        )?;
        Ok(v["id"].as_str().unwrap_or("").to_string())
    }

    fn remove(&self, id: &str) -> Result<(), ConnError> {
        self.g
            .call("DELETE", &format!("/me/events/{}", enc(id)), None, None)?;
        Ok(())
    }

    fn reschedule(
        &self,
        id: &str,
        start: NaiveDateTime,
        end: NaiveDateTime,
    ) -> Result<(), ConnError> {
        self.g.call(
            "PATCH",
            &format!("/me/events/{}", enc(id)),
            None,
            Some(&json!({
                "start": { "dateTime": from_local(start), "timeZone": "UTC" },
                "end": { "dateTime": from_local(end), "timeZone": "UTC" },
            })),
        )?;
        Ok(())
    }

    fn set_declined(&self, id: &str, declined: bool) -> Result<(), ConnError> {
        let action = if declined { "decline" } else { "accept" };
        self.g.call(
            "POST",
            &format!("/me/events/{}/{action}", enc(id)),
            None,
            Some(&json!({ "sendResponse": true })),
        )?;
        Ok(())
    }

    fn accept(&self, id: &str) -> Result<(), ConnError> {
        self.g.call(
            "POST",
            &format!("/me/events/{}/accept", enc(id)),
            None,
            Some(&json!({ "sendResponse": true })),
        )?;
        Ok(())
    }

    fn add_attendee(&self, id: &str, who: &str) -> Result<(), ConnError> {
        let address = if who.contains('@') {
            who.to_string()
        } else {
            self.g.address_of(who)?
        };
        let current = self
            .g
            .get(&format!("/me/events/{}?$select=attendees", enc(id)))?;
        let mut list = current["attendees"].as_array().cloned().unwrap_or_default();
        list.push(
            json!({ "emailAddress": { "address": address, "name": who }, "type": "required" }),
        );
        self.g.call(
            "PATCH",
            &format!("/me/events/{}", enc(id)),
            None,
            Some(&json!({ "attendees": list })),
        )?;
        Ok(())
    }

    fn remove_attendee(&self, id: &str, who: &str) -> Result<(), ConnError> {
        let current = self
            .g
            .get(&format!("/me/events/{}?$select=attendees", enc(id)))?;
        let list: Vec<Value> = current["attendees"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a["emailAddress"]["name"].as_str() != Some(who))
            .collect();
        self.g.call(
            "PATCH",
            &format!("/me/events/{}", enc(id)),
            None,
            Some(&json!({ "attendees": list })),
        )?;
        Ok(())
    }

    fn busy_of(&self, person: &str) -> Result<Vec<(NaiveDateTime, NaiveDateTime)>, ConnError> {
        let address = if person.contains('@') {
            person.to_string()
        } else {
            self.g.address_of(person)?
        };
        let now = (self.now)();
        let from = now.with_timezone(&Utc);
        let to = from + Duration::days(7);
        let fmt = |t: DateTime<Utc>| json!({ "dateTime": t.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC" });
        let v = self.g.call(
            "POST",
            "/me/calendar/getSchedule",
            None,
            Some(&json!({
                "schedules": [address],
                "startTime": fmt(from),
                "endTime": fmt(to),
                "availabilityViewInterval": 30,
            })),
        )?;
        Ok(v["value"][0]["scheduleItems"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| i["status"].as_str() != Some("free"))
            .filter_map(|i| {
                Some((
                    to_local(i["start"]["dateTime"].as_str()?)?,
                    to_local(i["end"]["dateTime"].as_str()?)?,
                ))
            })
            .collect())
    }

    fn people(&self) -> Result<Vec<String>, ConnError> {
        let v = self.g.get("/me/people?$top=50&$select=displayName")?;
        Ok(v["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["displayName"].as_str().map(str::to_string))
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Not connected yet
// ---------------------------------------------------------------------------

fn not_connected(what: &str) -> ConnError {
    ConnError::Failed(format!("{what} is not connected yet."))
}

pub struct NoDrive;

impl DriveBackend for NoDrive {
    fn files(&self) -> Result<Vec<DriveFile>, ConnError> {
        Err(not_connected("OneDrive"))
    }
    fn folders(&self) -> Result<Vec<String>, ConnError> {
        Err(not_connected("OneDrive"))
    }
    fn save(&self, _: &str, _: &str) -> Result<String, ConnError> {
        Err(not_connected("OneDrive"))
    }
    fn delete(&self, _: &str) -> Result<(), ConnError> {
        Err(not_connected("OneDrive"))
    }
    fn share(&self, _: &str, _: &str) -> Result<String, ConnError> {
        Err(not_connected("OneDrive"))
    }
    fn unshare(&self, _: &str, _: &str) -> Result<(), ConnError> {
        Err(not_connected("OneDrive"))
    }
    fn shares(&self) -> Result<Vec<(String, String)>, ConnError> {
        Err(not_connected("OneDrive"))
    }
}

pub struct NoChat;

impl ChatBackend for NoChat {
    fn channels(&self) -> Result<Vec<String>, ConnError> {
        Err(not_connected("Teams"))
    }
    fn post(&self, _: &str, _: &str) -> Result<String, ConnError> {
        Err(not_connected("Teams"))
    }
    fn messages(&self, _: &str) -> Result<Vec<(String, String)>, ConnError> {
        Err(not_connected("Teams"))
    }
    fn delete(&self, _: &str) -> Result<(), ConnError> {
        Err(not_connected("Teams"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{MemoryTokenStore, OAuthConfig, Transport};
    use std::sync::Mutex;

    /// Answers by the first rule whose method and URL fragment match; records every call.
    struct Stub {
        rules: Vec<(&'static str, &'static str, u16, String)>,
        calls: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl Http for Stub {
        fn send(
            &self,
            method: &str,
            url: &str,
            bearer: &str,
            _prefer: Option<&str>,
            body: Option<&str>,
        ) -> Result<(u16, String), String> {
            assert_eq!(bearer, "ACCESS", "every call carries the access token");
            self.calls
                .lock()
                .unwrap()
                .push((method.into(), url.into(), body.map(str::to_string)));
            self.rules
                .iter()
                .find(|(m, frag, _, _)| *m == method && url.contains(frag))
                .map(|(_, _, s, b)| (*s, b.clone()))
                .ok_or_else(|| format!("no rule for {method} {url}"))
        }
    }

    fn graph(rules: Vec<(&'static str, &'static str, u16, String)>) -> (Arc<Graph>, Arc<Stub>) {
        let stub = Arc::new(Stub {
            rules,
            calls: Mutex::new(Vec::new()),
        });
        // Seed the access token through a code exchange that the stub transport answers.
        struct Ok1;
        impl Transport for Ok1 {
            fn post_form(&self, _: &str, _: &[(&str, &str)]) -> Result<(u16, String), String> {
                Ok((
                    200,
                    r#"{"access_token":"ACCESS","refresh_token":"R","expires_in":3600}"#.into(),
                ))
            }
        }
        let session = Arc::new(Session::new(
            OAuthConfig {
                client_id: "c".into(),
                tenant: "common".into(),
                redirect_uri: "http://127.0.0.1:1/".into(),
                scopes: vec![],
            },
            Box::new(Ok1),
            Box::new(MemoryTokenStore::default()),
        ));
        session
            .exchange_code("code", "verifier", "http://127.0.0.1:1/", 1000)
            .unwrap();
        (
            Arc::new(Graph::new(stub.clone(), session).with_clock(1100)),
            stub,
        )
    }

    fn json_of(v: Value) -> String {
        v.to_string()
    }

    #[test]
    fn inbox_mail_is_mapped_with_unread_urgent_and_replied() {
        let (g, stub) = graph(vec![
            (
                "GET",
                "sentitems/messages",
                200,
                json_of(json!({"value":[{"conversationId":"c2"}]})),
            ),
            (
                "GET",
                "mailFolders/inbox/messages",
                200,
                json_of(json!({"value":[
                    {"id":"a1","subject":"Meeting request","from":{"emailAddress":{"name":"Rahul Sharma","address":"r@x.test"}},
                     "bodyPreview":"Thursday at 3pm?","receivedDateTime":"2026-11-09T08:00:00Z","isRead":false,"importance":"high",
                     "hasAttachments":false,"conversationId":"c1","inferenceClassification":"focused"},
                    {"id":"a2","subject":"Weekly newsletter","from":{"emailAddress":{"name":"News","address":"n@x.test"}},
                     "bodyPreview":"news","receivedDateTime":"2026-11-09T07:00:00Z","isRead":false,"importance":"normal",
                     "hasAttachments":true,"conversationId":"c2","inferenceClassification":"other"}
                ]})),
            ),
            (
                "GET",
                "mailFolders/archive/messages",
                200,
                json_of(json!({"value":[]})),
            ),
        ]);
        let mails = GraphMail::new(g).all().unwrap();
        assert_eq!(mails.len(), 2);
        assert!(mails[0].urgent && mails[0].unread && !mails[0].replied && !mails[0].newsletter);
        assert_eq!(mails[0].from_name, "Rahul Sharma");
        assert_eq!(mails[0].received, "2026-11-09");
        assert!(mails[1].newsletter && mails[1].replied && !mails[1].attachments.is_empty());
        assert!(stub
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(m, _, _)| m == "GET"));
    }

    #[test]
    fn archive_moves_to_the_archive_folder() {
        let (g, stub) = graph(vec![("POST", "/move", 200, "{}".into())]);
        GraphMail::new(g).move_to("a1", "Archive").unwrap();
        let calls = stub.calls.lock().unwrap();
        assert!(calls[0].1.ends_with("/me/messages/a1/move"));
        assert_eq!(
            calls[0].2.as_deref(),
            Some(r#"{"destinationId":"archive"}"#)
        );
    }

    #[test]
    fn a_send_needs_an_exact_name_match_and_never_guesses_an_address() {
        let people = json_of(json!({"value":[
            {"displayName":"Rahul Mehta","scoredEmailAddresses":[{"address":"mehta@x.test"}]},
            {"displayName":"Rahul Sharma","scoredEmailAddresses":[{"address":"sharma@x.test"}]}
        ]}));
        let (g, stub) = graph(vec![
            ("GET", "/me/people", 200, people),
            ("POST", "/me/sendMail", 202, String::new()),
        ]);
        let mail = GraphMail::new(g);
        mail.add_outgoing("Rahul Sharma", "The report is ready.", None, true)
            .unwrap();
        let calls = stub.calls.lock().unwrap();
        let body = calls.last().unwrap().2.clone().unwrap();
        assert!(
            body.contains("sharma@x.test") && !body.contains("mehta@x.test"),
            "{body}"
        );
        drop(calls);
        // "Rahul" alone matches nobody exactly: refused, not guessed.
        let e = mail.add_outgoing("Rahul", "hi", None, true).unwrap_err();
        assert_eq!(e.to_string(), "I can't find an email address for Rahul.");
        // Files cannot be attached yet, and the message says so.
        let e = mail
            .add_outgoing("Rahul Sharma", "hi", Some("a.pdf"), true)
            .unwrap_err();
        assert!(e.to_string().contains("not connected yet"));
    }

    #[test]
    fn events_become_local_events_with_their_invite_state() {
        let (g, _) = graph(vec![(
            "GET",
            "/me/calendarView",
            200,
            json_of(json!({"value":[
                {"id":"e1","subject":"Review","start":{"dateTime":"2026-11-13T08:30:00.0000000"},"end":{"dateTime":"2026-11-13T09:30:00.0000000"},
                 "attendees":[{"emailAddress":{"name":"Priya Nair"}}],"responseStatus":{"response":"notResponded"},"isOrganizer":false},
                {"id":"e2","subject":"Mine","start":{"dateTime":"2026-11-13T10:00:00.0000000"},"end":{"dateTime":"2026-11-13T10:30:00.0000000"},
                 "attendees":[],"responseStatus":{"response":"organizer"},"isOrganizer":true}
            ]})),
        )]);
        let cal = GraphCalendar::new(g);
        let events = cal.all().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].invite_pending && !events[1].invite_pending);
        assert_eq!(events[0].attendees, ["Priya Nair"]);
        // The UTC time was converted to this PC's local time, one hour long.
        assert_eq!(events[0].end - events[0].start, Duration::hours(1));
    }

    #[test]
    fn declining_creating_and_free_busy_call_the_right_endpoints() {
        let (g, stub) = graph(vec![
            ("POST", "/decline", 202, String::new()),
            ("POST", "/me/events", 201, json_of(json!({"id":"new1"}))),
            ("DELETE", "/me/events/new1", 204, String::new()),
            (
                "GET",
                "/me/people",
                200,
                json_of(
                    json!({"value":[{"displayName":"Rahul Sharma","scoredEmailAddresses":[{"address":"r@x.test"}]}]}),
                ),
            ),
            (
                "POST",
                "getSchedule",
                200,
                json_of(json!({"value":[{"scheduleItems":[
                    {"status":"busy","start":{"dateTime":"2026-11-12T04:00:00.0000000"},"end":{"dateTime":"2026-11-12T06:30:00.0000000"}},
                    {"status":"free","start":{"dateTime":"2026-11-12T07:00:00.0000000"},"end":{"dateTime":"2026-11-12T08:00:00.0000000"}}
                ]}]})),
            ),
        ]);
        let now = Local.with_ymd_and_hms(2026, 11, 9, 10, 0, 0).unwrap();
        let cal = GraphCalendar::new(g).with_now(now);
        cal.set_declined("e9", true).unwrap();
        let start = NaiveDateTime::parse_from_str("2026-11-12 15:00", "%Y-%m-%d %H:%M").unwrap();
        let id = cal
            .add(
                "Review",
                start,
                start + Duration::hours(1),
                &["Rahul Sharma".into()],
            )
            .unwrap();
        assert_eq!(id, "new1");
        cal.remove("new1").unwrap();
        // Only the busy block is returned, not the free one.
        assert_eq!(cal.busy_of("Rahul Sharma").unwrap().len(), 1);
        let calls = stub.calls.lock().unwrap();
        assert!(calls
            .iter()
            .any(|(m, u, _)| m == "POST" && u.ends_with("/me/events/e9/decline")));
        let created = calls
            .iter()
            .find(|(m, u, _)| m == "POST" && u.ends_with("/me/events"))
            .unwrap();
        assert!(created.2.as_ref().unwrap().contains("r@x.test"));
    }

    #[test]
    fn failures_become_the_usual_errors() {
        let (g, _) = graph(vec![
            ("GET", "/me/people", 401, "{}".into()),
            ("GET", "/me/messages/m", 404, "{}".into()),
            ("GET", "/me/events", 429, "{}".into()),
            ("GET", "/me/mailFolders", 403, "{}".into()),
        ]);
        // 401: signed out, with the standard wording, and the stored sign-in is dropped.
        assert_eq!(
            g.address_of("x").unwrap_err().to_string(),
            "Please sign in to Microsoft 365 again."
        );
        // After that the session has nothing to refresh with.
        assert!(matches!(
            g.call("GET", "/me/events", None, None),
            Err(ConnError::NotSignedIn(_))
        ));
    }

    #[test]
    fn not_connected_backends_say_so() {
        assert_eq!(
            NoDrive.folders().unwrap_err().to_string(),
            "OneDrive is not connected yet."
        );
        assert_eq!(
            NoChat.post("team", "hi").unwrap_err().to_string(),
            "Teams is not connected yet."
        );
        assert_eq!(enc("Rahul S"), "Rahul%20S");
    }
}

#[cfg(all(test, windows))]
mod live {
    use super::*;
    use crate::oauth::{CredentialTokenStore, OAuthConfig, UreqTransport};

    /// Reads the signed-in account through Graph and prints only counts, never subjects,
    /// bodies or names. Run with `JITMO_MS365_CLIENT_ID=<id> cargo test -p jitmo-connectors
    /// live_ -- --ignored --nocapture` after `sign in to Microsoft` in the app.
    #[test]
    #[ignore = "reads the real signed-in Microsoft account"]
    fn live_read_only_smoke() {
        let Ok(client_id) = std::env::var("JITMO_MS365_CLIENT_ID") else {
            println!("set JITMO_MS365_CLIENT_ID to run this");
            return;
        };
        let session = Arc::new(Session::new(
            OAuthConfig {
                client_id,
                tenant: "common".into(),
                redirect_uri: String::new(),
                scopes: crate::ms365_account::default_scopes(),
            },
            Box::new(UreqTransport),
            Box::new(CredentialTokenStore),
        ));
        println!(
            "signed in (a refresh token is stored): {}",
            session.is_signed_in()
        );
        let g = Arc::new(Graph::new(Arc::new(UreqHttp), session));
        match GraphMail::new(g.clone()).all() {
            Ok(m) => println!(
                "mail: {} messages, {} unread, {} urgent, {} newsletters, {} replied, {} in archive",
                m.len(),
                m.iter().filter(|x| x.unread).count(),
                m.iter().filter(|x| x.urgent).count(),
                m.iter().filter(|x| x.newsletter).count(),
                m.iter().filter(|x| x.replied).count(),
                m.iter().filter(|x| x.folder == "Archive").count(),
            ),
            Err(e) => println!("mail FAILED: {e}"),
        }
        match GraphCalendar::new(g.clone()).all() {
            Ok(e) => println!(
                "calendar: {} events in the next 3 weeks, {} pending invites, {} declined",
                e.len(),
                e.iter().filter(|x| x.invite_pending).count(),
                e.iter().filter(|x| x.declined).count(),
            ),
            Err(e) => println!("calendar FAILED: {e}"),
        }
        match GraphCalendar::new(g).people() {
            Ok(p) => println!("people: {} names", p.len()),
            Err(e) => println!("people FAILED: {e}"),
        }
    }
}
