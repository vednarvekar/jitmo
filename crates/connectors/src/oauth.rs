//! Microsoft sign-in (Phase 3, Track A): OAuth 2.0 authorization code flow with PKCE, for a
//! desktop app. No client secret is involved; the app is a public client.
//!
//! The pieces, each testable without a network or a Microsoft account:
//! - [`OAuthConfig`] builds the sign-in URL; [`Pkce`] makes the code verifier and challenge;
//! - [`listen_for_code`] receives the redirect on `127.0.0.1` and checks the `state`;
//! - [`Session`] swaps the code for tokens, keeps the access token in memory, refreshes it
//!   when it is about to expire, and answers "please sign in again" (never a crash) when the
//!   refresh token is gone or was revoked;
//! - [`TokenStore`] keeps the refresh token: in Windows Credential Manager in the app
//!   ([`CredentialTokenStore`], PS5), in memory in tests.
//!
//! Every connector asks only for the scopes it needs ([`scopes`]). The tenant, client id and
//! redirect port come from the app registration the owner creates; until then nothing here
//! talks to Microsoft.

use crate::ConnError;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Least-privilege scopes, by what a connector does.
pub mod scopes {
    /// Lets the app get a refresh token, so sign-in lasts.
    pub const OFFLINE: &str = "offline_access";
    pub const MAIL_READ: &str = "Mail.Read";
    /// Archive, move and drafts (never sending: that needs [`MAIL_SEND`]).
    pub const MAIL_WRITE: &str = "Mail.ReadWrite";
    pub const MAIL_SEND: &str = "Mail.Send";
    pub const CALENDAR_READ: &str = "Calendars.Read";
    pub const CALENDAR_WRITE: &str = "Calendars.ReadWrite";
    pub const FILES_READ: &str = "Files.Read";
    pub const FILES_WRITE: &str = "Files.ReadWrite";
    pub const CHAT_SEND: &str = "ChatMessage.Send";
}

/// The app registration and what one connector asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthConfig {
    pub client_id: String,
    /// `common`, `organizations`, `consumers`, or a tenant id.
    pub tenant: String,
    /// Always a loopback address: `http://127.0.0.1:PORT/`.
    pub redirect_uri: String,
    pub scopes: Vec<String>,
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A redirect to this PC only.
pub fn is_loopback(uri: &str) -> bool {
    uri.starts_with("http://127.0.0.1:") || uri.starts_with("http://localhost:")
}

impl OAuthConfig {
    /// The same registration with the redirect of one sign-in (its own free port).
    pub fn with_redirect(&self, redirect_uri: &str) -> OAuthConfig {
        OAuthConfig {
            redirect_uri: redirect_uri.to_string(),
            ..self.clone()
        }
    }

    fn base(&self) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0",
            self.tenant
        )
    }

    pub fn token_url(&self) -> String {
        format!("{}/token", self.base())
    }

    /// The page the user signs in on (opened in the default browser).
    pub fn authorize_url(&self, state: &str, challenge: &str) -> String {
        format!(
            "{}/authorize?client_id={}&response_type=code&redirect_uri={}&response_mode=query\
             &scope={}&state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
            self.base(),
            encode(&self.client_id),
            encode(&self.redirect_uri),
            encode(&self.scopes.join(" ")),
            encode(state),
            encode(challenge),
        )
    }

    /// Only loopback redirects are allowed: the code must never leave this PC.
    pub fn redirect_is_loopback(&self) -> bool {
        is_loopback(&self.redirect_uri)
    }
}

/// The PKCE pair: a secret the app keeps (`verifier`) and its hash (`challenge`) that goes in
/// the sign-in URL, so a stolen redirect cannot be swapped for tokens by anyone else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// From 32 random bytes (the caller supplies randomness, so tests are exact).
    pub fn from_bytes(bytes: &[u8; 32]) -> Pkce {
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let verifier = b64.encode(bytes);
        let challenge = b64.encode(Sha256::digest(verifier.as_bytes()));
        Pkce {
            verifier,
            challenge,
        }
    }

    /// With fresh random bytes from the operating system.
    pub fn generate() -> Result<Pkce, String> {
        Ok(Pkce::from_bytes(&random32()?))
    }
}

/// 32 random bytes: also used for the `state` value.
pub fn random32() -> Result<[u8; 32], String> {
    #[cfg(windows)]
    {
        jitmo_adapter_windows::credentials::random_bytes32()
    }
    #[cfg(not(windows))]
    {
        let mut b = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut b))
            .map_err(|e| e.to_string())?;
        Ok(b)
    }
}

/// A `state` value that ties the redirect to the sign-in this app started.
pub fn new_state() -> Result<String, String> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random32()?[..16]))
}

// ---------------------------------------------------------------------------
// The redirect
// ---------------------------------------------------------------------------

/// The authorization code in a redirect request line (`GET /?code=...&state=... HTTP/1.1`),
/// after checking the `state` and that Microsoft did not answer with an error.
pub fn parse_redirect(request: &str, expected_state: &str) -> Result<String, AuthError> {
    let line = request.lines().next().unwrap_or("");
    let target = line.split_whitespace().nth(1).unwrap_or("");
    let query = target.split_once('?').map_or("", |(_, q)| q);
    let get = |name: &str| {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| decode(v))
    };
    if let Some(err) = get("error") {
        // "access_denied" is the user pressing Cancel.
        return Err(if err == "access_denied" {
            AuthError::Cancelled
        } else {
            AuthError::Failed(get("error_description").unwrap_or(err))
        });
    }
    if get("state").as_deref() != Some(expected_state) {
        return Err(AuthError::Failed(
            "The sign-in did not match the one that was started.".into(),
        ));
    }
    get("code").ok_or_else(|| AuthError::Failed("No sign-in code came back.".into()))
}

/// Wait for the browser to be sent back to `127.0.0.1`, answer it with a short page, and
/// return the code. Gives up after `wait`.
pub fn listen_for_code(
    listener: &TcpListener,
    expected_state: &str,
    wait: Duration,
) -> Result<String, AuthError> {
    listener
        .set_nonblocking(true)
        .map_err(|e| AuthError::Failed(e.to_string()))?;
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                // A favicon or a probe is not the redirect: keep waiting for the real one.
                if !request.contains("code=") && !request.contains("error=") {
                    let _ =
                        stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let result = parse_redirect(&request, expected_state);
                let body = match &result {
                    Ok(_) => "You are signed in. You can close this tab and go back to Jitmo.",
                    Err(_) => {
                        "Sign-in did not finish. You can close this tab and try again in Jitmo."
                    }
                };
                let page = format!(
                    "<!doctype html><meta charset=utf-8><title>Jitmo</title>\
                     <body style=\"font:16px system-ui;margin:3em\">{body}"
                );
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
                        page.len()
                    )
                    .as_bytes(),
                );
                return result;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() >= wait {
                    return Err(AuthError::TimedOut);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(AuthError::Failed(e.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// No usable sign-in: ask the user to sign in again.
    SignInAgain,
    /// The user pressed Cancel on the Microsoft page.
    Cancelled,
    TimedOut,
    Offline,
    Failed(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::SignInAgain => write!(f, "Please sign in to Microsoft 365 again."),
            AuthError::Cancelled => write!(f, "Sign-in was cancelled."),
            AuthError::TimedOut => write!(f, "Sign-in took too long. Try again."),
            AuthError::Offline => write!(f, "Microsoft 365 needs an internet connection."),
            AuthError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl From<AuthError> for ConnError {
    fn from(e: AuthError) -> ConnError {
        match e {
            AuthError::SignInAgain => ConnError::NotSignedIn("Microsoft 365".into()),
            AuthError::Offline => ConnError::Offline("Microsoft 365".into()),
            other => ConnError::Failed(other.to_string()),
        }
    }
}

/// Sends the token requests. The real one uses HTTPS; tests use a recorded answer.
pub trait Transport: Send + Sync {
    /// POST a form; returns the status code and body. `Err` means no answer at all.
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, String), String>;
}

pub struct UreqTransport;

impl Transport for UreqTransport {
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, String), String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        let mut resp = agent
            .post(url)
            .send_form(form.iter().copied())
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok((status, body))
    }
}

/// Where the refresh token lives between runs.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, refresh_token: &str) -> Result<(), String>;
    fn clear(&self);
}

#[derive(Default)]
pub struct MemoryTokenStore(Mutex<Option<String>>);

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Option<String> {
        self.0.lock().ok()?.clone()
    }
    fn save(&self, refresh_token: &str) -> Result<(), String> {
        *self.0.lock().map_err(|e| e.to_string())? = Some(refresh_token.to_string());
        Ok(())
    }
    fn clear(&self) {
        if let Ok(mut g) = self.0.lock() {
            *g = None;
        }
    }
}

/// The refresh token in Windows Credential Manager, target `jitmo/ms365/refresh-token`.
#[cfg(windows)]
pub struct CredentialTokenStore;

#[cfg(windows)]
const REFRESH_TARGET: &str = "jitmo/ms365/refresh-token";

#[cfg(windows)]
impl TokenStore for CredentialTokenStore {
    fn load(&self) -> Option<String> {
        jitmo_adapter_windows::credentials::read_secret(REFRESH_TARGET)
            .ok()
            .flatten()
            .filter(|t| !t.is_empty())
    }
    fn save(&self, refresh_token: &str) -> Result<(), String> {
        jitmo_adapter_windows::credentials::write_secret(REFRESH_TARGET, "jitmo", refresh_token)
    }
    fn clear(&self) {
        let _ = jitmo_adapter_windows::credentials::delete_secret(REFRESH_TARGET);
    }
}

/// Seconds of slack: a token that expires within this is refreshed first.
const EXPIRY_SLACK: i64 = 60;

struct Access {
    token: String,
    expires_at: i64,
}

/// One signed-in account for one connector.
pub struct Session {
    cfg: OAuthConfig,
    transport: Box<dyn Transport>,
    store: Box<dyn TokenStore>,
    access: Mutex<Option<Access>>,
}

/// A token response: `(access, refresh, expires_in)` or an OAuth error code.
fn parse_token_response(status: u16, body: &str) -> Result<(String, Option<String>, i64), String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    if status == 200 {
        let access = v["access_token"]
            .as_str()
            .ok_or("the answer had no access token")?;
        return Ok((
            access.to_string(),
            v["refresh_token"].as_str().map(str::to_string),
            v["expires_in"].as_i64().unwrap_or(3600),
        ));
    }
    Err(v["error"].as_str().unwrap_or("unknown_error").to_string())
}

impl Session {
    pub fn new(
        cfg: OAuthConfig,
        transport: Box<dyn Transport>,
        store: Box<dyn TokenStore>,
    ) -> Session {
        Session {
            cfg,
            transport,
            store,
            access: Mutex::new(None),
        }
    }

    /// Has the user signed in on this PC (is there a refresh token)?
    pub fn is_signed_in(&self) -> bool {
        self.store.load().is_some()
    }

    fn take(&self, status: u16, body: &str, now: i64) -> Result<(), AuthError> {
        match parse_token_response(status, body) {
            Ok((access, refresh, expires_in)) => {
                if let Some(r) = refresh {
                    self.store.save(&r).map_err(AuthError::Failed)?;
                }
                if let Ok(mut g) = self.access.lock() {
                    *g = Some(Access {
                        token: access,
                        expires_at: now + expires_in,
                    });
                }
                Ok(())
            }
            // The refresh token expired, was revoked, or the password changed.
            Err(code) if code == "invalid_grant" || code == "interaction_required" => {
                self.sign_out();
                Err(AuthError::SignInAgain)
            }
            Err(code) => Err(AuthError::Failed(format!(
                "Microsoft sign-in failed ({code})."
            ))),
        }
    }

    /// Swap the code from the redirect for tokens.
    pub fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
        now: i64,
    ) -> Result<(), AuthError> {
        // The redirect of this sign-in (the loopback port is chosen when it starts).
        if !is_loopback(redirect_uri) {
            return Err(AuthError::Failed(
                "The redirect must be a loopback address.".into(),
            ));
        }
        let scope = self.cfg.scopes.join(" ");
        let (status, body) = self
            .transport
            .post_form(
                &self.cfg.token_url(),
                &[
                    ("client_id", &self.cfg.client_id),
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("redirect_uri", redirect_uri),
                    ("code_verifier", verifier),
                    ("scope", &scope),
                ],
            )
            .map_err(|_| AuthError::Offline)?;
        self.take(status, &body, now)
    }

    /// A valid access token, refreshing it when it is about to expire. `SignInAgain` when
    /// there is nothing to refresh with.
    pub fn access_token(&self, now: i64) -> Result<String, AuthError> {
        if let Ok(g) = self.access.lock() {
            if let Some(a) = g.as_ref() {
                if a.expires_at - EXPIRY_SLACK > now {
                    return Ok(a.token.clone());
                }
            }
        }
        let refresh = self.store.load().ok_or(AuthError::SignInAgain)?;
        let scope = self.cfg.scopes.join(" ");
        let (status, body) = self
            .transport
            .post_form(
                &self.cfg.token_url(),
                &[
                    ("client_id", &self.cfg.client_id),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", &refresh),
                    ("scope", &scope),
                ],
            )
            .map_err(|_| AuthError::Offline)?;
        self.take(status, &body, now)?;
        self.access
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|a| a.token.clone()))
            .ok_or(AuthError::SignInAgain)
    }

    /// Forget the account on this PC.
    pub fn sign_out(&self) {
        self.store.clear();
        if let Ok(mut g) = self.access.lock() {
            *g = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn cfg() -> OAuthConfig {
        OAuthConfig {
            client_id: "client-123".into(),
            tenant: "common".into(),
            redirect_uri: "http://127.0.0.1:53682/".into(),
            scopes: vec![scopes::OFFLINE.into(), scopes::MAIL_READ.into()],
        }
    }

    /// Answers from a script and records what was asked.
    #[derive(Default)]
    struct Script {
        answers: Mutex<Vec<Result<(u16, String), String>>>,
        asked: Mutex<Vec<Vec<(String, String)>>>,
    }
    struct FakeTransport(Arc<Script>);
    impl Transport for FakeTransport {
        fn post_form(&self, _url: &str, form: &[(&str, &str)]) -> Result<(u16, String), String> {
            self.0.asked.lock().unwrap().push(
                form.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            );
            self.0.answers.lock().unwrap().remove(0)
        }
    }
    fn tokens(access: &str, refresh: &str, expires: i64) -> Result<(u16, String), String> {
        Ok((
            200,
            format!(
                r#"{{"access_token":"{access}","refresh_token":"{refresh}","expires_in":{expires}}}"#
            ),
        ))
    }
    fn session(answers: Vec<Result<(u16, String), String>>) -> (Session, Arc<Script>) {
        let script = Arc::new(Script {
            answers: Mutex::new(answers),
            ..Script::default()
        });
        let s = Session::new(
            cfg(),
            Box::new(FakeTransport(script.clone())),
            Box::new(MemoryTokenStore::default()),
        );
        (s, script)
    }

    #[test]
    fn pkce_matches_the_published_example() {
        // RFC 7636 appendix B: this verifier hashes to this challenge.
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = b64.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let p = Pkce::from_bytes(&[7u8; 32]);
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(p.challenge.len(), 43);
        assert!(!p.verifier.contains(['+', '/', '=']));
    }

    #[test]
    fn the_sign_in_url_carries_pkce_state_and_only_the_asked_scopes() {
        let url = cfg().authorize_url("st@te", "chal");
        assert!(url.starts_with("https://login.microsoftonline.com/common/oauth2/v2.0/authorize?"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("code_challenge=chal&code_challenge_method=S256"));
        assert!(url.contains("state=st%40te"));
        assert!(url.contains("scope=offline_access%20Mail.Read"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2F"));
        assert!(!url.contains("Mail.Send") && !url.contains("ReadWrite"));
    }

    #[test]
    fn the_redirect_is_checked_for_state_errors_and_cancel() {
        let ok = "GET /?code=abc%2Fdef&state=S1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(parse_redirect(ok, "S1"), Ok("abc/def".into()));
        assert!(matches!(
            parse_redirect(ok, "other"),
            Err(AuthError::Failed(_))
        ));
        let denied = "GET /?error=access_denied&state=S1 HTTP/1.1\r\n\r\n";
        assert_eq!(parse_redirect(denied, "S1"), Err(AuthError::Cancelled));
        let bad =
            "GET /?error=server_error&error_description=Oops+it+broke&state=S1 HTTP/1.1\r\n\r\n";
        assert_eq!(
            parse_redirect(bad, "S1"),
            Err(AuthError::Failed("Oops it broke".into()))
        );
        assert!(parse_redirect("GET /?state=S1 HTTP/1.1", "S1").is_err());
    }

    #[test]
    fn the_loopback_listener_receives_the_code_and_ignores_probes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            for path in ["/favicon.ico", "/?code=the-code&state=S9"] {
                let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
                    .unwrap();
                let mut reply = String::new();
                let _ = s.read_to_string(&mut reply);
            }
        });
        let code = listen_for_code(&listener, "S9", Duration::from_secs(5)).unwrap();
        assert_eq!(code, "the-code");
        browser.join().unwrap();
    }

    #[test]
    fn the_listener_gives_up_when_nobody_comes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        assert_eq!(
            listen_for_code(&listener, "S", Duration::from_millis(150)),
            Err(AuthError::TimedOut)
        );
    }

    #[test]
    fn a_code_becomes_tokens_and_the_refresh_token_is_stored() {
        let (s, script) = session(vec![tokens("A1", "R1", 3600)]);
        assert!(!s.is_signed_in());
        s.exchange_code("the-code", "the-verifier", "http://127.0.0.1:53682/", 1000)
            .unwrap();
        assert!(s.is_signed_in());
        // Cached: no second request while the token is good.
        assert_eq!(s.access_token(1100).unwrap(), "A1");
        let asked = script.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].contains(&("code_verifier".into(), "the-verifier".into())));
        assert!(asked[0].contains(&("grant_type".into(), "authorization_code".into())));
    }

    #[test]
    fn an_expiring_token_is_refreshed_and_the_new_refresh_token_kept() {
        let (s, script) = session(vec![tokens("A1", "R1", 3600), tokens("A2", "R2", 3600)]);
        s.exchange_code("c", "v", "http://127.0.0.1:53682/", 1000)
            .unwrap();
        // 59 s before expiry is inside the slack.
        assert_eq!(s.access_token(1000 + 3600 - 59).unwrap(), "A2");
        let asked = script.asked.lock().unwrap();
        assert!(asked[1].contains(&("refresh_token".into(), "R1".into())));
        drop(asked);
        assert!(s.is_signed_in());
    }

    #[test]
    fn a_revoked_sign_in_means_please_sign_in_again_and_forgets_it() {
        let (s, _) = session(vec![
            tokens("A1", "R1", 10),
            Ok((400, r#"{"error":"invalid_grant"}"#.into())),
        ]);
        s.exchange_code("c", "v", "http://127.0.0.1:53682/", 0)
            .unwrap();
        assert_eq!(s.access_token(100), Err(AuthError::SignInAgain));
        assert!(!s.is_signed_in());
        // And with nothing stored there is nothing to try.
        assert_eq!(s.access_token(200), Err(AuthError::SignInAgain));
        let e: ConnError = AuthError::SignInAgain.into();
        assert_eq!(e.to_string(), "Please sign in to Microsoft 365 again.");
    }

    #[test]
    fn no_network_is_offline_not_signed_out() {
        let (s, _) = session(vec![tokens("A1", "R1", 10), Err("dns".into())]);
        s.exchange_code("c", "v", "http://127.0.0.1:53682/", 0)
            .unwrap();
        assert_eq!(s.access_token(100), Err(AuthError::Offline));
        assert!(s.is_signed_in(), "being offline must not sign the user out");
    }

    #[test]
    fn a_redirect_that_leaves_this_pc_is_refused() {
        let mut c = cfg();
        c.redirect_uri = "https://evil.example/cb".into();
        let (s, _) = session(vec![]);
        let s2 = Session::new(
            c,
            Box::new(FakeTransport(Arc::new(Script::default()))),
            Box::new(MemoryTokenStore::default()),
        );
        assert!(matches!(
            s2.exchange_code("c", "v", "https://evil.example/cb", 0),
            Err(AuthError::Failed(_))
        ));
        drop(s);
    }

    #[test]
    fn percent_coding_round_trips() {
        for s in ["a b/c?d&e=f", "ünï", "~-_."] {
            assert_eq!(decode(&encode(s)), s);
        }
    }
}
