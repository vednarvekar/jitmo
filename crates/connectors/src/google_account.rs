//! The Google account connector: setting the OAuth client, signing in and out.
//!
//! Same loopback + PKCE sign-in as Microsoft ([`crate::oauth`], tenant `google`). Google's
//! desktop clients come with a client id (`…apps.googleusercontent.com`) and a client
//! secret (`GOCSPX-…`) that the token endpoint wants; Google itself says that secret is not
//! confidential for an installed app, so it is kept as a plain setting.

use crate::ms365::{arg, out};
use crate::oauth::{listen_for_code, new_state, OAuthConfig, Pkce, Session};
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

type Opener = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
/// Saves `(client_id, client_secret)`.
type Saver = Box<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

pub struct GoogleAccount {
    cfg: Option<OAuthConfig>,
    session: Option<Arc<Session>>,
    open_url: Opener,
    save_client: Saver,
    wait: Duration,
    now: Box<dyn Fn() -> i64 + Send + Sync>,
}

pub fn not_set_up() -> ConnError {
    ConnError::Failed(
        "Google is not set up yet. Say “set Google client id to … and secret …” with the client id and secret from your Google Cloud project, then restart Jitmo and say “sign in to Google”."
            .into(),
    )
}

impl GoogleAccount {
    pub fn new(
        cfg: Option<OAuthConfig>,
        session: Option<Arc<Session>>,
        open_url: Opener,
        save_client: Saver,
    ) -> GoogleAccount {
        GoogleAccount {
            cfg,
            session,
            open_url,
            save_client,
            wait: Duration::from_secs(120),
            now: Box::new(|| chrono::Utc::now().timestamp()),
        }
    }

    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }
}

/// The client id and secret in a sentence: the id ends in `.apps.googleusercontent.com`,
/// the secret starts with `GOCSPX-`.
pub fn parse_client(text: &str) -> Option<(String, String)> {
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
                .trim_end_matches('.')
        })
        .collect();
    let id = words
        .iter()
        .find(|w| w.ends_with(".apps.googleusercontent.com"))?;
    let secret = words.iter().find(|w| w.starts_with("GOCSPX-"))?;
    Some((id.to_string(), secret.to_string()))
}

impl Connector for GoogleAccount {
    fn id(&self) -> &str {
        "google_account"
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
                let (Some(cfg), Some(session)) = (&self.cfg, &self.session) else {
                    return Err(not_set_up());
                };
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
                Ok(out(&[("message", "Signed in to Google.".to_string())]))
            }
            "sign_out" => {
                if let Some(s) = &self.session {
                    s.sign_out();
                }
                Ok(out(&[("message", "Signed out of Google.".to_string())]))
            }
            "set_client_id" => {
                let text = arg(args, "text")?;
                let (id, secret) = parse_client(text).ok_or_else(|| {
                    ConnError::Failed(
                        "I need both: the client id (ends in .apps.googleusercontent.com) and the secret (starts with GOCSPX-)."
                            .into(),
                    )
                })?;
                (self.save_client)(&id, &secret).map_err(ConnError::Failed)?;
                Ok(out(&[(
                    "message",
                    "Saved. Restart Jitmo, then say “sign in to Google”.".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_id_and_secret_in_a_sentence() {
        let (id, secret) = parse_client(
            "set google client id to 123-abc.apps.googleusercontent.com and secret GOCSPX-abc_DEF-1.",
        )
        .unwrap();
        assert_eq!(id, "123-abc.apps.googleusercontent.com");
        assert_eq!(secret, "GOCSPX-abc_DEF-1");
        assert!(
            parse_client("set google client id to 123-abc.apps.googleusercontent.com").is_none()
        );
    }
}
