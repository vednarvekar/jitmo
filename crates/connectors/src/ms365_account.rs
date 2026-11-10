//! The Microsoft 365 account connector (Phase 3, Track A): signing in and out, and the
//! stand-in the other Microsoft connectors use until the app registration exists.
//!
//! Signing in opens the Microsoft page in the default browser and waits (up to two minutes)
//! for the redirect on `127.0.0.1` ([`crate::oauth`]). The client id of the app registration
//! is a setting (`ms365_client_id`): without it every Microsoft connector says plainly that
//! Microsoft 365 is not set up, instead of failing in a way nobody can act on.

use crate::ms365::{arg, out};
use crate::oauth::{listen_for_code, new_state, scopes, OAuthConfig, Pkce, Session};
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

/// Least-privilege scopes for everything the Microsoft connectors do today. Files and
/// chat are added with the connectors that need them.
pub fn default_scopes() -> Vec<String> {
    [
        scopes::OFFLINE,
        scopes::MAIL_WRITE,
        scopes::MAIL_SEND,
        scopes::CALENDAR_WRITE,
        "People.Read",
        "Calendars.Read.Shared",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

type Opener = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
type Saver = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub struct Ms365Account {
    /// `None` until a client id is saved.
    cfg: Option<OAuthConfig>,
    session: Option<Arc<Session>>,
    open_url: Opener,
    save_client_id: Saver,
    wait: Duration,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl Ms365Account {
    pub fn new(
        cfg: Option<OAuthConfig>,
        session: Option<Arc<Session>>,
        open_url: Opener,
        save_client_id: Saver,
    ) -> Ms365Account {
        Ms365Account {
            cfg,
            session,
            open_url,
            save_client_id,
            wait: Duration::from_secs(120),
            now: Box::new(|| chrono::Utc::now().timestamp()),
        }
    }

    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    fn setup(&self) -> Result<(&OAuthConfig, &Arc<Session>), ConnError> {
        match (&self.cfg, &self.session) {
            (Some(c), Some(s)) => Ok((c, s)),
            _ => Err(not_set_up()),
        }
    }
}

/// What every Microsoft connector says before there is a client id.
pub fn not_set_up() -> ConnError {
    ConnError::Failed(
        "Microsoft 365 is not set up yet. Say “set Microsoft client id to …” with the id of your app registration, then restart Jitmo and say “sign in to Microsoft”."
            .into(),
    )
}

fn is_guid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

impl Connector for Ms365Account {
    fn id(&self) -> &str {
        "ms365_account"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        let a = |n: &str, r: &[&str]| ActionSpec {
            name: n.into(),
            required_args: r.iter().map(|s| s.to_string()).collect(),
        };
        vec![
            a("sign_in", &[]),
            a("sign_out", &[]),
            a("set_client_id", &["text"]),
        ]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        match action {
            "sign_in" => {
                let (cfg, session) = self.setup()?;
                let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| {
                    ConnError::Failed(format!("I could not wait for the sign-in: {e}"))
                })?;
                let port = listener
                    .local_addr()
                    .map_err(|e| ConnError::Failed(e.to_string()))?
                    .port();
                let redirect = format!("http://127.0.0.1:{port}/");
                let cfg = cfg.with_redirect(&redirect);
                let pkce = Pkce::generate().map_err(ConnError::Failed)?;
                let state = new_state().map_err(ConnError::Failed)?;
                (self.open_url)(&cfg.authorize_url(&state, &pkce.challenge)).map_err(|_| {
                    ConnError::Failed("I could not open the browser for sign-in.".into())
                })?;
                let code = listen_for_code(&listener, &state, self.wait)?;
                session.exchange_code(&code, &pkce.verifier, &redirect, (self.now)())?;
                Ok(out(&[(
                    "message",
                    "Signed in to Microsoft 365.".to_string(),
                )]))
            }
            "sign_out" => {
                if let Some(s) = &self.session {
                    s.sign_out();
                }
                Ok(out(&[(
                    "message",
                    "Signed out of Microsoft 365.".to_string(),
                )]))
            }
            "set_client_id" => {
                let text = arg(args, "text")?;
                // The id is the first word that looks like a GUID.
                let id = text
                    .split_whitespace()
                    .map(|w| w.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '-'))
                    .find(|w| is_guid(w))
                    .ok_or_else(|| {
                        ConnError::Failed(
                            "That does not look like a client id. It is 36 characters, like 12345678-aaaa-bbbb-cccc-1234567890ab."
                                .into(),
                        )
                    })?;
                (self.save_client_id)(id).map_err(ConnError::Failed)?;
                Ok(out(&[(
                    "message",
                    "Saved. Restart Jitmo, then say “sign in to Microsoft”.".to_string(),
                )]))
            }
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn check(&self, action: &str, _args: &Args) -> Result<bool, ConnError> {
        match action {
            "sign_in" => Ok(self.session.as_ref().is_some_and(|s| s.is_signed_in())),
            "sign_out" => Ok(self.session.as_ref().is_none_or(|s| !s.is_signed_in())),
            "set_client_id" => Ok(true),
            other => Err(ConnError::UnknownAction(other.into())),
        }
    }

    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

/// A Microsoft connector with no client id: every action says Microsoft 365 is not set up.
pub struct NotSetUp {
    id: String,
    actions: Vec<ActionSpec>,
}

impl NotSetUp {
    pub fn new(id: &str, real: &dyn Connector) -> NotSetUp {
        NotSetUp {
            id: id.to_string(),
            actions: real.actions(),
        }
    }
}

impl Connector for NotSetUp {
    fn id(&self) -> &str {
        &self.id
    }

    fn actions(&self) -> Vec<ActionSpec> {
        self.actions.clone()
    }

    fn health(&self) -> Health {
        Health::NotSignedIn
    }

    fn execute(&self, _action: &str, _args: &Args) -> Result<Output, ConnError> {
        Err(not_set_up())
    }

    fn check(&self, _action: &str, _args: &Args) -> Result<bool, ConnError> {
        Err(not_set_up())
    }

    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{MemoryTokenStore, TokenStore, Transport};
    use std::io::{Read, Write};
    use std::sync::Mutex;

    fn args(pairs: &[(&str, &str)]) -> Args {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn cfg() -> OAuthConfig {
        OAuthConfig {
            client_id: "11111111-2222-3333-4444-555555555555".into(),
            tenant: "common".into(),
            redirect_uri: String::new(),
            scopes: default_scopes(),
            client_secret: None,
        }
    }

    struct Tokens;
    impl Transport for Tokens {
        fn post_form(&self, _: &str, form: &[(&str, &str)]) -> Result<(u16, String), String> {
            // The redirect sent with the code must be the loopback one the browser used.
            let redirect = form
                .iter()
                .find(|(k, _)| *k == "redirect_uri")
                .map(|(_, v)| *v)
                .unwrap_or("");
            assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
            Ok((
                200,
                r#"{"access_token":"A","refresh_token":"R","expires_in":3600}"#.into(),
            ))
        }
    }

    struct Shared(Arc<MemoryTokenStore>);
    impl TokenStore for Shared {
        fn load(&self) -> Option<String> {
            self.0.load()
        }
        fn save(&self, t: &str) -> Result<(), String> {
            self.0.save(t)
        }
        fn clear(&self) {
            self.0.clear()
        }
    }

    /// The part of a sign-in URL the "browser" needs: the port it was sent back to.
    fn redirect_port(url: &str) -> u16 {
        let query = url.split_once('?').unwrap().1;
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix("redirect_uri="))
            .unwrap()
            .replace("%3A", ":")
            .replace("%2F", "/")
            .trim_start_matches("http://127.0.0.1:")
            .trim_end_matches('/')
            .parse()
            .unwrap()
    }

    fn query_value(url: &str, key: &str) -> String {
        url.split_once('?')
            .unwrap()
            .1
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
            .unwrap()
            .to_string()
    }

    fn answer_redirect(port: u16, request: String) {
        std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(request.as_bytes()).unwrap();
            let mut r = String::new();
            let _ = s.read_to_string(&mut r);
        });
    }

    #[test]
    fn signing_in_opens_the_browser_waits_for_the_redirect_and_stores_the_refresh_token() {
        let store = Arc::new(MemoryTokenStore::default());
        let session = Arc::new(Session::new(
            cfg(),
            Box::new(Tokens),
            Box::new(Shared(store.clone())),
        ));
        let opened = Arc::new(Mutex::new(String::new()));
        let seen = opened.clone();
        let account = Ms365Account::new(
            Some(cfg()),
            Some(session),
            Box::new(move |url| {
                *seen.lock().unwrap() = url.to_string();
                let state = query_value(url, "state");
                answer_redirect(
                    redirect_port(url),
                    format!("GET /?code=abc&state={state} HTTP/1.1\r\nHost: x\r\n\r\n"),
                );
                Ok(())
            }),
            Box::new(|_| Ok(())),
        )
        .with_wait(Duration::from_secs(10));
        let r = account.execute("sign_in", &args(&[])).unwrap();
        assert_eq!(r.0["message"], "Signed in to Microsoft 365.");
        assert_eq!(store.load().as_deref(), Some("R"));
        assert!(opened
            .lock()
            .unwrap()
            .contains("code_challenge_method=S256"));
        assert_eq!(account.check("sign_in", &args(&[])), Ok(true));
        account.execute("sign_out", &args(&[])).unwrap();
        assert_eq!(store.load(), None);
        assert_eq!(account.check("sign_out", &args(&[])), Ok(true));
    }

    #[test]
    fn without_a_client_id_everything_says_it_is_not_set_up() {
        let account = Ms365Account::new(None, None, Box::new(|_| Ok(())), Box::new(|_| Ok(())));
        let e = account.execute("sign_in", &args(&[])).unwrap_err();
        assert!(e.to_string().starts_with("Microsoft 365 is not set up yet"));
        let real = crate::ms365::Outlook::fake(Arc::new(crate::ms365::FakeMailbox::seeded()));
        let stand_in = NotSetUp::new("outlook", &real);
        assert_eq!(stand_in.actions().len(), real.actions().len());
        assert!(stand_in.execute("urgent", &args(&[])).is_err());
        assert_eq!(stand_in.health(), Health::NotSignedIn);
    }

    #[test]
    fn the_client_id_is_found_in_the_sentence_and_saved() {
        let saved = Arc::new(Mutex::new(String::new()));
        let s = saved.clone();
        let account = Ms365Account::new(
            None,
            None,
            Box::new(|_| Ok(())),
            Box::new(move |id| {
                *s.lock().unwrap() = id.to_string();
                Ok(())
            }),
        );
        account
            .execute(
                "set_client_id",
                &args(&[(
                    "text",
                    "set Microsoft client id to 12345678-abcd-4ef0-9876-1234567890ab",
                )]),
            )
            .unwrap();
        assert_eq!(
            *saved.lock().unwrap(),
            "12345678-abcd-4ef0-9876-1234567890ab"
        );
        assert!(account
            .execute(
                "set_client_id",
                &args(&[("text", "set microsoft client id to banana")])
            )
            .is_err());
    }

    #[test]
    fn a_cancelled_sign_in_is_reported_not_stored() {
        let session = Arc::new(Session::new(
            cfg(),
            Box::new(Tokens),
            Box::new(MemoryTokenStore::default()),
        ));
        let account = Ms365Account::new(
            Some(cfg()),
            Some(session.clone()),
            Box::new(|url| {
                let state = query_value(url, "state");
                answer_redirect(
                    redirect_port(url),
                    format!("GET /?error=access_denied&state={state} HTTP/1.1\r\n\r\n"),
                );
                Ok(())
            }),
            Box::new(|_| Ok(())),
        )
        .with_wait(Duration::from_secs(10));
        let e = account.execute("sign_in", &args(&[])).unwrap_err();
        assert_eq!(e.to_string(), "Sign-in was cancelled.");
        assert!(!session.is_signed_in());
    }
}
