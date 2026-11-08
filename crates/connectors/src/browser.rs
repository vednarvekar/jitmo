//! `browser`: open a creator's latest YouTube video (A4), search YouTube or the
//! web (A5) and open a website (A6).
//!
//! The connector turns the user's sentence into one https URL that this file
//! builds itself (a fixed host plus a percent-encoded query), then opens it with
//! `Start-Process $env:JITMO_URL` through a [`ScriptRunner`]. The URL travels in
//! an environment variable and is never pasted into a script. Nothing is ever
//! opened that came from the user as a ready-made link, so there is no way to
//! make this connector open `file:`, `javascript:` or a look-alike site.
//!
//! Parsing is pure and unit-tested; no test starts a browser.

use crate::system_controls::ScriptRunner;
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use std::sync::Arc;

/// Opens the page in the default browser. Env: `JITMO_URL`.
pub const OPEN_SCRIPT: &str = "Start-Process $env:JITMO_URL";

// ---------------------------------------------------------------------------
// URL building
// ---------------------------------------------------------------------------

/// Percent-encodes a query for a URL. Spaces become `+`; everything except
/// letters, digits and `-_.~` is written as `%XX` (UTF-8 bytes).
pub fn encode_query(q: &str) -> String {
    let mut out = String::new();
    for b in q.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// True when `url` is a plain https URL on a lowercase ASCII host with no
/// spaces or control characters. Every URL this connector builds must pass.
pub fn is_valid_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    if !rest.bytes().all(|b| b.is_ascii_graphic()) {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host.contains('.')
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// YouTube results for `q`, newest uploads first (`sp=CAI%253D` is the
/// "upload date" sort, already encoded the way YouTube writes it).
fn youtube_latest_url(q: &str) -> String {
    format!(
        "https://www.youtube.com/results?search_query={}&sp=CAI%253D",
        encode_query(q)
    )
}

fn youtube_search_url(q: &str) -> String {
    format!(
        "https://www.youtube.com/results?search_query={}",
        encode_query(q)
    )
}

fn web_search_url(q: &str) -> String {
    format!("https://www.google.com/search?q={}", encode_query(q))
}

// ---------------------------------------------------------------------------
// Understanding the request
// ---------------------------------------------------------------------------

/// Words with the edge punctuation trimmed. Curly apostrophes become straight.
fn words(text: &str) -> Vec<String> {
    text.replace(['\u{2019}', '\u{2018}'], "'")
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn lower(ws: &[String]) -> Vec<String> {
    ws.iter().map(|w| w.to_lowercase()).collect()
}

const LATEST_MARKERS: &[&str] = &["latest", "newest", "new", "recent"];
const VIDEO_WORDS: &[&str] = &["vid", "vids", "video", "videos", "upload", "uploads"];
const CREATOR_FILLER: &[&str] = &[
    "open", "play", "watch", "show", "me", "the", "that", "this", "a", "an", "please", "go", "to",
    "up", "pull", "can", "you", "could", "on", "youtube", "channel", "from", "by", "of", "my", "i",
    "want", "see", "get", "check", "out", "uploaded", "in", "at", "and",
];

/// "Open MKBHD's latest video on YouTube", "open that new mkbhd vid".
/// Returns the creator's name as the user wrote it, without "'s".
pub fn parse_latest(text: &str) -> Option<String> {
    let toks = words(text);
    let lw = lower(&toks);
    let m = lw
        .iter()
        .position(|w| LATEST_MARKERS.contains(&w.as_str()))?;
    let v = lw.iter().position(|w| VIDEO_WORDS.contains(&w.as_str()))?;
    let keep = |range: &[String]| -> Vec<String> {
        range
            .iter()
            .filter(|w| !CREATOR_FILLER.contains(&w.to_lowercase().as_str()))
            .cloned()
            .collect()
    };
    let mut name = keep(&toks[..m]);
    if name.is_empty() && v > m {
        name = keep(&toks[m + 1..v]);
    }
    if name.is_empty() && v + 1 < toks.len() {
        // "the latest video from mkbhd on youtube"
        let after = &toks[v + 1..];
        if matches!(lw[v + 1].as_str(), "from" | "by" | "of") {
            let cut = after
                .iter()
                .skip(1)
                .position(|w| matches!(w.to_lowercase().as_str(), "on" | "in" | "at"))
                .map_or(after.len(), |p| p + 1);
            name = keep(&after[..cut]);
        }
    }
    if let Some(last) = name.last_mut() {
        if let Some(s) = last.strip_suffix("'s").or_else(|| last.strip_suffix('\'')) {
            *last = s.to_string();
        }
    }
    name.retain(|w| !w.is_empty());
    if name.is_empty() || name.len() > 4 {
        return None;
    }
    Some(name.join(" "))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    YouTube,
    Web,
}

const ENGINE_WEB: &[&str] = &["web", "internet", "online", "google", "bing"];

/// "Search YouTube for pasta recipes", "google X", "search the web for X",
/// "look up X", "youtube X", "search X on youtube".
pub fn parse_search(text: &str) -> Option<(Engine, String)> {
    let toks = words(text);
    let lw = lower(&toks);
    let mut i = 0;
    while i < lw.len()
        && matches!(
            lw[i].as_str(),
            "please" | "hey" | "can" | "could" | "you" | "would" | "kindly"
        )
    {
        i += 1;
    }
    let mut engine = Engine::Web;
    match lw.get(i).map(String::as_str)? {
        "google" | "search" | "searching" | "lookup" => i += 1,
        "look" if lw.get(i + 1).map(String::as_str) == Some("up") => i += 2,
        "youtube" => {
            engine = Engine::YouTube;
            i += 1;
        }
        _ => return None,
    }
    let mut named_engine = false;
    while let Some(w) = lw.get(i).map(String::as_str) {
        let next = lw.get(i + 1).map(String::as_str).unwrap_or("");
        let next_is_engine = next == "youtube" || ENGINE_WEB.contains(&next);
        if matches!(w, "for" | "about") || (matches!(w, "the" | "on" | "in") && next_is_engine) {
            i += 1;
        } else if w == "youtube" && !named_engine {
            engine = Engine::YouTube;
            named_engine = true;
            i += 1;
        } else if ENGINE_WEB.contains(&w) && !named_engine {
            named_engine = true;
            i += 1;
        } else {
            break;
        }
    }
    let mut end = toks.len();
    let tail = &lw[i..];
    let n = tail.len();
    if n >= 2 && matches!(tail[n - 2].as_str(), "on" | "in") && tail[n - 1] == "youtube" {
        engine = Engine::YouTube;
        end -= 2;
    } else if n >= 2 && tail[n - 2] == "on" && tail[n - 1] == "google" {
        end -= 2;
    } else if n >= 3 && tail[n - 3] == "on" && tail[n - 2] == "the" && tail[n - 1] == "web" {
        end -= 3;
    } else if n >= 2 && tail[n - 1] == "online" {
        end -= 1;
    }
    if end <= i {
        return None;
    }
    let query = toks[i..end].join(" ");
    Some((engine, query))
}

/// Well-known sites: (names people say, label, https URL).
pub const SITES: &[(&[&str], &str, &str)] = &[
    (&["amazon"], "Amazon", "https://www.amazon.com"),
    (&["youtube", "yt"], "YouTube", "https://www.youtube.com"),
    (
        &["gmail", "google mail"],
        "Gmail",
        "https://mail.google.com",
    ),
    (&["google"], "Google", "https://www.google.com"),
    (&["github"], "GitHub", "https://github.com"),
    (&["netflix"], "Netflix", "https://www.netflix.com"),
    (&["facebook", "fb"], "Facebook", "https://www.facebook.com"),
    (
        &["instagram", "insta"],
        "Instagram",
        "https://www.instagram.com",
    ),
    (&["twitter", "x"], "X", "https://x.com"),
    (&["reddit"], "Reddit", "https://www.reddit.com"),
    (&["linkedin"], "LinkedIn", "https://www.linkedin.com"),
    (
        &["wikipedia", "wiki"],
        "Wikipedia",
        "https://www.wikipedia.org",
    ),
    (
        &["google maps", "maps"],
        "Google Maps",
        "https://maps.google.com",
    ),
    (
        &["google drive", "drive"],
        "Google Drive",
        "https://drive.google.com",
    ),
    (
        &["google docs", "docs"],
        "Google Docs",
        "https://docs.google.com",
    ),
    (
        &["google calendar", "calendar"],
        "Google Calendar",
        "https://calendar.google.com",
    ),
    (
        &["google photos"],
        "Google Photos",
        "https://photos.google.com",
    ),
    (
        &["google translate", "translate"],
        "Google Translate",
        "https://translate.google.com",
    ),
    (&["outlook"], "Outlook", "https://outlook.live.com"),
    (
        &["stackoverflow", "stack overflow"],
        "Stack Overflow",
        "https://stackoverflow.com",
    ),
    (&["twitch"], "Twitch", "https://www.twitch.tv"),
    (&["ebay"], "eBay", "https://www.ebay.com"),
    (
        &["whatsapp web"],
        "WhatsApp Web",
        "https://web.whatsapp.com",
    ),
    (&["chatgpt"], "ChatGPT", "https://chatgpt.com"),
    (&["claude"], "Claude", "https://claude.ai"),
    (&["pinterest"], "Pinterest", "https://www.pinterest.com"),
    (&["paypal"], "PayPal", "https://www.paypal.com"),
    (&["flipkart"], "Flipkart", "https://www.flipkart.com"),
    (&["zomato"], "Zomato", "https://www.zomato.com"),
    (&["swiggy"], "Swiggy", "https://www.swiggy.com"),
    (&["hotstar"], "Hotstar", "https://www.hotstar.com"),
    (
        &["prime video", "amazon prime"],
        "Prime Video",
        "https://www.primevideo.com",
    ),
    (
        &["disney plus", "disneyplus"],
        "Disney+",
        "https://www.disneyplus.com",
    ),
    (&["imdb"], "IMDb", "https://www.imdb.com"),
    (&["bing"], "Bing", "https://www.bing.com"),
    (
        &["duckduckgo", "duck duck go"],
        "DuckDuckGo",
        "https://duckduckgo.com",
    ),
    (&["yahoo"], "Yahoo", "https://www.yahoo.com"),
    (&["notion"], "Notion", "https://www.notion.so"),
    (&["zoom"], "Zoom", "https://zoom.us"),
    (&["dropbox"], "Dropbox", "https://www.dropbox.com"),
    (&["tiktok"], "TikTok", "https://www.tiktok.com"),
    (&["discord"], "Discord", "https://discord.com"),
    (&["bbc"], "BBC", "https://www.bbc.com"),
    (&["cnn"], "CNN", "https://www.cnn.com"),
    (&["apple"], "Apple", "https://www.apple.com"),
    (&["microsoft"], "Microsoft", "https://www.microsoft.com"),
];

/// Top-level domains accepted when someone says a domain such as "github.com".
/// Short on purpose: "report.pdf" or "notes.txt" must never look like a website.
const TLDS: &[&str] = &[
    "com", "org", "net", "io", "co", "in", "edu", "gov", "dev", "app", "ai", "tv", "me", "uk",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    /// A site we know (or a clean domain the user typed).
    Known { label: String, url: String },
    /// A name we do not know: search for it instead of guessing a domain.
    Unknown(String),
}

const SITE_VERBS: &[&str] = &["open", "launch", "visit", "load", "goto"];
const SITE_FILLER: &[&str] = &[
    "the", "website", "site", "page", "homepage", "please", "up", "to", "my",
];
const BROWSER_WORDS: &[&str] = &["the", "my", "browser", "chrome", "edge", "firefox", "brave"];

fn as_domain(name: &str) -> Option<String> {
    let n = name
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = n.split(['/', '?', '#']).next()?.trim_start_matches("www.");
    let (_, tld) = host.rsplit_once('.')?;
    let ok = TLDS.contains(&tld)
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    let url = format!("https://{host}");
    (ok && is_valid_url(&url)).then_some(url)
}

fn known(label: &str, url: &str) -> Site {
    Site::Known {
        label: label.to_string(),
        url: url.to_string(),
    }
}

/// "Open Amazon", "go to github", "visit netflix.com", "open the wikipedia site".
/// A verb is required, so a bare "amazon" is not taken as a request.
pub fn parse_site(text: &str) -> Option<Site> {
    let toks = words(text);
    let lw = lower(&toks);
    let mut i = 0;
    while i < lw.len() && matches!(lw[i].as_str(), "please" | "can" | "could" | "you") {
        i += 1;
    }
    let w = |k: usize| lw.get(k).map(String::as_str).unwrap_or("");
    match (w(i), w(i + 1)) {
        (v, _) if SITE_VERBS.contains(&v) => i += 1,
        ("go", "to") | ("pull", "up") | ("browse", "to") | ("navigate", "to") => i += 2,
        ("take", "me") if w(i + 2) == "to" => i += 3,
        _ => return None,
    }
    let mut rest: Vec<String> = lw[i..].to_vec();
    if let Some(p) = (0..rest.len()).find(|&j| {
        matches!(rest[j].as_str(), "in" | "on")
            && rest.len() > j + 1
            && rest[j + 1..]
                .iter()
                .all(|x| BROWSER_WORDS.contains(&x.as_str()))
    }) {
        rest.truncate(p);
    }
    rest.retain(|x| !SITE_FILLER.contains(&x.as_str()));
    if rest.is_empty() || rest.len() > 4 {
        return None;
    }
    let key = rest.join(" ");
    if let Some((_, label, url)) = SITES
        .iter()
        .find(|(names, _, _)| names.contains(&key.as_str()))
    {
        return Some(known(label, url));
    }
    let squashed = key.replace(' ', "");
    if let Some((_, label, url)) = SITES
        .iter()
        .find(|(names, _, _)| names.iter().any(|n| n.replace(' ', "") == squashed))
    {
        return Some(known(label, url));
    }
    if let Some(url) = as_domain(&squashed) {
        let label = url.trim_start_matches("https://").to_string();
        return Some(Site::Known { label, url });
    }
    Some(Site::Unknown(key))
}

// ---------------------------------------------------------------------------
// Connector
// ---------------------------------------------------------------------------

/// What an action would open, and what to tell the user.
struct Plan {
    url: String,
    message: String,
}

fn arg<'a>(args: &'a Args, name: &str) -> Result<&'a String, ConnError> {
    args.get(name)
        .ok_or_else(|| ConnError::MissingArg(name.to_string()))
}

fn plan(action: &str, args: &Args) -> Result<Plan, ConnError> {
    let text = arg(args, "text")?;
    let plan = match action {
        "youtube_latest" => {
            let creator = parse_latest(text).ok_or_else(|| {
                ConnError::Failed("Whose latest video? Try “open MKBHD's latest video”.".into())
            })?;
            Plan {
                url: youtube_latest_url(&creator),
                message: format!(
                    "Opened YouTube's newest uploads for {creator}. I can't tell which one is the very latest from here, so check the top result."
                ),
            }
        }
        "search" => {
            let (engine, q) = parse_search(text).ok_or_else(|| {
                ConnError::Failed(
                    "What should I search for? Try “search YouTube for pasta”.".into(),
                )
            })?;
            match engine {
                Engine::YouTube => Plan {
                    url: youtube_search_url(&q),
                    message: format!("Opened YouTube results for “{q}”."),
                },
                Engine::Web => Plan {
                    url: web_search_url(&q),
                    message: format!("Opened a web search for “{q}”."),
                },
            }
        }
        "site" => match parse_site(text)
            .ok_or_else(|| ConnError::Failed("Which website? Try “open Amazon”.".into()))?
        {
            Site::Known { label, url } => Plan {
                url,
                message: format!("Opened {label}."),
            },
            Site::Unknown(name) => Plan {
                url: web_search_url(&name),
                message: format!(
                    "I don't know a site called “{name}”, so I searched the web for it."
                ),
            },
        },
        other => return Err(ConnError::UnknownAction(other.to_string())),
    };
    if is_valid_url(&plan.url) {
        Ok(plan)
    } else {
        Err(ConnError::Failed(
            "I couldn't build a safe link for that.".into(),
        ))
    }
}

pub struct Browser {
    runner: Arc<dyn ScriptRunner>,
}

impl Browser {
    pub fn new(runner: Arc<dyn ScriptRunner>) -> Self {
        Browser { runner }
    }
}

impl Connector for Browser {
    fn id(&self) -> &str {
        "browser"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        ["youtube_latest", "search", "site"]
            .iter()
            .map(|n| ActionSpec {
                name: n.to_string(),
                required_args: vec!["text".into()],
            })
            .collect()
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let p = plan(action, args)?;
        self.runner
            .run(OPEN_SCRIPT, &[("JITMO_URL", p.url.as_str())])
            .map_err(|_| ConnError::Failed("I couldn't open your browser.".into()))?;
        Ok(Output(
            [
                ("message".to_string(), p.message),
                ("url".to_string(), p.url),
            ]
            .into(),
        ))
    }

    /// A page that was opened cannot be read back from here, so this only
    /// re-parses the request and reports whether a valid https URL would be
    /// built. It proves the request was understood, not that the page loaded.
    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        Ok(is_valid_url(&plan(action, args)?.url))
    }

    /// Opening a page cannot be undone.
    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    type Call = (String, Vec<(String, String)>);

    struct Recorder {
        calls: Mutex<Vec<Call>>,
        fail: bool,
    }

    impl ScriptRunner for Recorder {
        fn run(&self, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
            self.calls.lock().unwrap().push((
                script.to_string(),
                env.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            if self.fail {
                Err("boom".into())
            } else {
                Ok(String::new())
            }
        }
    }

    fn browser(fail: bool) -> (Browser, Arc<Recorder>) {
        let r = Arc::new(Recorder {
            calls: Mutex::new(vec![]),
            fail,
        });
        (Browser::new(r.clone()), r)
    }

    fn args(t: &str) -> Args {
        [("text".to_string(), t.to_string())].into()
    }

    fn url_of(b: &Browser, action: &str, t: &str) -> String {
        b.execute(action, &args(t)).unwrap().0["url"].clone()
    }

    #[test]
    fn encodes_queries() {
        assert_eq!(encode_query("pasta recipes"), "pasta+recipes");
        assert_eq!(encode_query("a&b=c#d"), "a%26b%3Dc%23d");
        assert_eq!(encode_query("café"), "caf%C3%A9");
        assert_eq!(encode_query("x/../y"), "x%2F..%2Fy");
    }

    #[test]
    fn validates_urls() {
        assert!(is_valid_url("https://www.amazon.com"));
        assert!(!is_valid_url("http://www.amazon.com"));
        assert!(!is_valid_url("file:///c:/x"));
        assert!(!is_valid_url("javascript:alert(1)"));
        assert!(!is_valid_url("https://a b.com"));
        assert!(!is_valid_url("https://localhost"));
        assert!(!is_valid_url("https://Evil.com"));
    }

    #[test]
    fn latest_creators() {
        let l = parse_latest;
        assert_eq!(
            l("Open MKBHD's latest video on YouTube.").as_deref(),
            Some("MKBHD")
        );
        assert_eq!(
            l("Open MKBHD\u{2019}s latest video on YouTube").as_deref(),
            Some("MKBHD")
        );
        assert_eq!(l("open that new mkbhd vid").as_deref(), Some("mkbhd"));
        assert_eq!(
            l("play the newest video from Veritasium").as_deref(),
            Some("Veritasium")
        );
        assert_eq!(
            l("watch the latest video from Marques Brownlee on youtube").as_deref(),
            Some("Marques Brownlee")
        );
        assert_eq!(l("open the latest video"), None);
        assert_eq!(l("open that new vid"), None);
        assert_eq!(l("search youtube for pasta"), None);
    }

    #[test]
    fn latest_url_is_sorted_by_date() {
        let (b, r) = browser(false);
        let out = b
            .execute(
                "youtube_latest",
                &args("Open MKBHD's latest video on YouTube."),
            )
            .unwrap();
        assert_eq!(
            out.0["url"],
            "https://www.youtube.com/results?search_query=MKBHD&sp=CAI%253D"
        );
        assert!(out.0["message"].contains("can't tell which one is the very latest"));
        let calls = r.calls.lock().unwrap();
        assert_eq!(calls[0].0, "Start-Process $env:JITMO_URL");
        assert_eq!(
            calls[0].1,
            vec![("JITMO_URL".to_string(), out.0["url"].clone())]
        );
    }

    #[test]
    fn search_parsing() {
        let s = parse_search;
        let yt = |q: &str| Some((Engine::YouTube, q.to_string()));
        let web = |q: &str| Some((Engine::Web, q.to_string()));
        assert_eq!(s("Search YouTube for pasta recipes."), yt("pasta recipes"));
        assert_eq!(
            s("search on youtube for pasta recipes"),
            yt("pasta recipes")
        );
        assert_eq!(s("youtube pasta recipes"), yt("pasta recipes"));
        assert_eq!(s("search pasta recipes on youtube"), yt("pasta recipes"));
        assert_eq!(s("google best budget phones"), web("best budget phones"));
        assert_eq!(
            s("Search the web for rust lifetimes"),
            web("rust lifetimes")
        );
        assert_eq!(s("look up the capital of peru"), web("the capital of peru"));
        assert_eq!(s("please search for cheap flights"), web("cheap flights"));
        assert_eq!(s("search"), None);
        assert_eq!(s("search youtube for"), None);
        assert_eq!(s("youtube"), None);
        assert_eq!(s("open youtube"), None);
    }

    #[test]
    fn search_urls_encode_the_query() {
        let (b, _) = browser(false);
        assert_eq!(
            url_of(&b, "search", "Search YouTube for pasta recipes"),
            "https://www.youtube.com/results?search_query=pasta+recipes"
        );
        assert_eq!(
            url_of(&b, "search", "google cats&dogs"),
            "https://www.google.com/search?q=cats%26dogs"
        );
    }

    #[test]
    fn injection_stays_in_the_query() {
        let (b, r) = browser(false);
        let u = url_of(&b, "search", "google x\"; calc.exe; \"&evil=1");
        assert!(u.starts_with("https://www.google.com/search?q="));
        let q = &u["https://www.google.com/search?q=".len()..];
        assert!(!q.contains('"') && !q.contains(';') && !q.contains('&'));
        // The script text is always the fixed one.
        assert!(r.calls.lock().unwrap().iter().all(|c| c.0 == OPEN_SCRIPT));
    }

    #[test]
    fn site_parsing() {
        let s = parse_site;
        assert_eq!(
            s("Open Amazon."),
            Some(known("Amazon", "https://www.amazon.com"))
        );
        assert_eq!(
            s("go to github"),
            Some(known("GitHub", "https://github.com"))
        );
        assert_eq!(
            s("take me to gmail"),
            Some(known("Gmail", "https://mail.google.com"))
        );
        assert_eq!(
            s("open the netflix website"),
            Some(known("Netflix", "https://www.netflix.com"))
        );
        assert_eq!(
            s("open google maps in chrome"),
            Some(known("Google Maps", "https://maps.google.com"))
        );
        assert_eq!(
            s("open stack overflow"),
            Some(known("Stack Overflow", "https://stackoverflow.com"))
        );
        assert_eq!(
            s("visit github.com"),
            Some(known("github.com", "https://github.com"))
        );
        assert_eq!(
            s("open https://www.example.org/a/b"),
            Some(known("example.org", "https://example.org"))
        );
        assert_eq!(s("amazon"), None);
        assert_eq!(s("open"), None);
    }

    #[test]
    fn unknown_names_search_instead_of_guessing() {
        assert_eq!(
            parse_site("open Blorptastic"),
            Some(Site::Unknown("blorptastic".into()))
        );
        // Not domains: file names must not become websites.
        assert!(matches!(
            parse_site("open report.pdf"),
            Some(Site::Unknown(_))
        ));
        let (b, _) = browser(false);
        let out = b.execute("site", &args("open Blorptastic")).unwrap();
        assert_eq!(out.0["url"], "https://www.google.com/search?q=blorptastic");
        assert!(out.0["message"].contains("don't know a site"));
    }

    #[test]
    fn site_table_is_sound() {
        assert!(SITES.len() >= 40);
        let mut seen = std::collections::HashSet::new();
        for (names, label, url) in SITES {
            assert!(is_valid_url(url), "{label}");
            assert!(url.matches('/').count() == 2, "{label}: bare host only");
            for n in *names {
                assert!(seen.insert(*n), "duplicate name {n}");
                assert_eq!(
                    parse_site(&format!("open {n}")),
                    Some(known(label, url)),
                    "{n}"
                );
            }
        }
    }

    #[test]
    fn check_reparses_without_running_anything() {
        let (b, r) = browser(false);
        assert_eq!(b.check("site", &args("Open Amazon.")), Ok(true));
        assert_eq!(b.check("search", &args("google rust")), Ok(true));
        assert_eq!(
            b.check("youtube_latest", &args("open that new mkbhd vid")),
            Ok(true)
        );
        assert!(b.check("site", &args("hello")).is_err());
        assert!(b.check("nope", &args("x")).is_err());
        assert!(r.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn errors_are_plain() {
        let (b, _) = browser(true);
        assert_eq!(
            b.execute("site", &args("open amazon")).unwrap_err(),
            ConnError::Failed("I couldn't open your browser.".into())
        );
        assert!(matches!(
            b.execute("site", &Args::new()),
            Err(ConnError::MissingArg(_))
        ));
        assert!(matches!(
            b.execute("youtube_latest", &args("open the latest video")),
            Err(ConnError::Failed(_))
        ));
        assert!(matches!(
            b.execute("zap", &args("x")),
            Err(ConnError::UnknownAction(_))
        ));
        assert!(b.undo("site", &Output::default()).is_none());
        assert_eq!(b.id(), "browser");
        assert_eq!(b.actions().len(), 3);
    }
}
