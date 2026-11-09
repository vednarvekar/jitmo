//! `web_lookup`: short answers that need the internet (tasks Q2, Q3).
//!
//! - `weather` the forecast for a day (rain, temperature or both) (Q2)
//! - `fact`    a quick fact from Wikipedia (Q3)
//!
//! Privacy (PS3): only search terms leave the device. The user's sentence is
//! parsed here and only the place name (weather) or the subject phrase (fact)
//! is put in a request URL. Never the whole sentence, files or contacts.
//!
//! Services (decision D18, terms checked 2026-11-09, all usable commercially
//! with no API key):
//! - weather:   MET Norway locationforecast 2.0 (CC BY 4.0 + NLOD 2.0, needs an
//!   identifying User-Agent, coordinates truncated to 4 decimals).
//! - geocoding: the English Wikipedia Action API (article coordinates).
//! - facts:     the English Wikipedia Action API (intro extract, CC BY-SA 4.0,
//!   credited with a link to the article).
//!
//! Open-Meteo is NOT used: its free API is non-commercial only.
//!
//! All network access goes through the [`Http`] trait, so tests use canned
//! JSON and never touch the network.

use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use chrono::{Datelike, NaiveDate, Weekday};
use regex::Regex;
use serde_json::Value;
use std::time::Duration;

/// The one-line answer when there is no internet.
pub const OFFLINE_MESSAGE: &str =
    "That needs an internet connection, and I can't reach one right now.";
const NOT_FOUND_MESSAGE: &str = "I couldn't find that.";
const NO_PLACE_MESSAGE: &str = "Which place? Say, for example, 'will it rain in Pune tomorrow'";

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// No connection could be made (no network, DNS failure, timeout).
    Offline,
    /// The server answered 404.
    NotFound,
    /// Anything else.
    Other(String),
}

/// A GET that returns the response body as text.
pub trait Http: Send + Sync {
    fn get(&self, url: &str) -> Result<String, HttpError>;
}

/// Identifies the app and a contact, as MET Norway and Wikimedia require.
pub const USER_AGENT: &str =
    "Jitmo/0.1 (+https://github.com/vednarvekar/jitmo; local assistant) ureq/3";

/// The real HTTP client: short timeouts, built-in Windows TLS.
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqHttp {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(4)))
            .timeout_global(Some(Duration::from_secs(8)))
            .user_agent(USER_AGENT)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .provider(ureq::tls::TlsProvider::NativeTls)
                    .build(),
            )
            .build();
        UreqHttp {
            agent: config.into(),
        }
    }
}

impl Http for UreqHttp {
    fn get(&self, url: &str) -> Result<String, HttpError> {
        match self.agent.get(url).call() {
            Ok(mut resp) => resp
                .body_mut()
                .read_to_string()
                .map_err(|e| HttpError::Other(e.to_string())),
            Err(ureq::Error::StatusCode(404)) => Err(HttpError::NotFound),
            Err(
                ureq::Error::Io(_)
                | ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed
                | ureq::Error::Timeout(_),
            ) => Err(HttpError::Offline),
            Err(e) => Err(HttpError::Other(e.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Connector
// ---------------------------------------------------------------------------

pub struct WebLookup {
    http: Box<dyn Http>,
    /// The place saved in the app settings, used when the request names none.
    default_place: Option<String>,
    /// Reads the saved place each time, so "set my city" works without a restart.
    place_source: Option<Box<dyn Fn() -> Option<String> + Send + Sync>>,
    /// The clock, replaceable in tests.
    now: Box<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>,
}

/// Credit required by the MET Norway licence (CC BY 4.0).
pub const WEATHER_CREDIT: &str = "(Data from MET Norway)";

impl WebLookup {
    /// Replace the clock (tests).
    pub fn with_clock(
        mut self,
        now: impl Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync + 'static,
    ) -> Self {
        self.now = Box::new(now);
        self
    }

    /// Read the saved place live instead of once at startup.
    pub fn with_place_source(
        mut self,
        source: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.place_source = Some(Box::new(source));
        self
    }

    fn saved_place(&self) -> Option<String> {
        let live = self
            .place_source
            .as_ref()
            .and_then(|f| f())
            .filter(|p| !p.trim().is_empty());
        live.or_else(|| self.default_place.clone())
    }

    pub fn new(default_place: Option<String>) -> Self {
        Self::with_http(Box::new(UreqHttp::new()), default_place)
    }

    pub fn with_http(http: Box<dyn Http>, default_place: Option<String>) -> Self {
        WebLookup {
            http,
            default_place: default_place.filter(|p| !p.trim().is_empty()),
            place_source: None,
            now: Box::new(chrono::Utc::now),
        }
    }

    fn weather(&self, text: &str) -> Result<String, ConnError> {
        let q = parse_weather(text);
        let place = q
            .place
            .clone()
            .or_else(|| self.default_place.clone())
            .ok_or_else(|| ConnError::Failed(NO_PLACE_MESSAGE.into()))?;

        // Geocoding: the coordinates of the best-matching Wikipedia article.
        let geo_url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch={}\
             &gsrlimit=3&prop=coordinates&format=json&formatversion=2",
            encode(&place)
        );
        let geo = self.fetch_json(&geo_url)?;
        let (name, lat, lon) = parse_geocode(&geo)
            .ok_or_else(|| ConnError::Failed("I couldn't find that place.".into()))?;

        // MET Norway rejects more than 4 decimals.
        let url = format!(
            "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={lat:.4}&lon={lon:.4}"
        );
        let forecast = self.fetch_json(&url)?;
        let offset_secs = solar_offset_secs(lon);
        let today = (self.now)()
            .checked_add_signed(chrono::Duration::seconds(offset_secs))
            .map(|t| t.date_naive())
            .ok_or_else(not_found)?;
        let days = daily_from_met(&forecast, offset_secs).ok_or_else(not_found)?;
        let text = render_weather(&q, &name, today, &days).ok_or_else(not_found)?;
        Ok(format!("{text} {WEATHER_CREDIT}"))
    }

    fn fact(&self, text: &str) -> Result<String, ConnError> {
        let subject = parse_fact_subject(text).ok_or_else(not_found)?;
        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch={}\
             &gsrlimit=1&prop=extracts%7Cinfo&exintro=1&explaintext=1&exchars=700&inprop=url\
             &format=json&formatversion=2",
            encode(&subject)
        );
        let found = self.fetch_json(&url)?;
        render_fact(&found).ok_or_else(not_found)
    }

    /// GET and parse JSON. Bad JSON becomes "I couldn't find that.", never a panic.
    fn fetch_json(&self, url: &str) -> Result<Value, ConnError> {
        match self.http.get(url) {
            Ok(body) => serde_json::from_str(&body).map_err(|_| not_found()),
            Err(HttpError::Offline) => Err(ConnError::Failed(OFFLINE_MESSAGE.into())),
            Err(HttpError::NotFound) => Err(not_found()),
            Err(HttpError::Other(_)) => Err(ConnError::Failed(
                "I couldn't get an answer from the internet just now.".into(),
            )),
        }
    }
}

fn not_found() -> ConnError {
    ConnError::Failed(NOT_FOUND_MESSAGE.into())
}

impl Connector for WebLookup {
    fn id(&self) -> &str {
        "web_lookup"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        ["weather", "fact"]
            .iter()
            .map(|a| ActionSpec {
                name: a.to_string(),
                required_args: vec!["text".to_string()],
            })
            .collect()
    }

    fn health(&self) -> Health {
        // Reachability is only known by trying; execute reports being offline.
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let text = args
            .get("text")
            .ok_or_else(|| ConnError::MissingArg("text".into()))?;
        let message = match action {
            "weather" => self.weather(text)?,
            "fact" => self.fact(text)?,
            other => return Err(ConnError::UnknownAction(other.to_string())),
        };
        Ok(Output([("message".to_string(), message)].into()))
    }

    /// A network answer cannot be re-verified (and a second request would
    /// send the search terms again), so this only re-parses the request and
    /// passes when it is answerable: a fact has a subject, a forecast has a
    /// place (in the request or the saved one).
    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        let text = args
            .get("text")
            .ok_or_else(|| ConnError::MissingArg("text".into()))?;
        match action {
            "weather" => Ok(parse_weather(text).place.is_some() || self.saved_place().is_some()),
            "fact" => Ok(parse_fact_subject(text).is_some()),
            other => Err(ConnError::UnknownAction(other.to_string())),
        }
    }

    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    Today,
    Tomorrow,
    DayAfterTomorrow,
    Weekend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rain,
    Temperature,
    General,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherQuery {
    pub place: Option<String>,
    pub day: Day,
    pub kind: Kind,
}

pub fn parse_weather(text: &str) -> WeatherQuery {
    let lower = text.to_lowercase();
    let has = |pat: &str| Regex::new(pat).map(|r| r.is_match(&lower)).unwrap_or(false);

    let day = if has(r"\bday after tomorrow\b") {
        Day::DayAfterTomorrow
    } else if has(r"\b(?:tomorrow|tmrw|tmr|2morrow)\b") {
        Day::Tomorrow
    } else if has(r"\bweekend\b") {
        Day::Weekend
    } else {
        Day::Today
    };

    let kind = if has(r"\b(?:rain|rainy|raining|rains|umbrella|drizzle|showers?)\b") {
        Kind::Rain
    } else if has(r"\b(?:temperature|temp|hot|cold|warm|cool|chilly|degrees)\b") {
        Kind::Temperature
    } else {
        Kind::General
    };

    WeatherQuery {
        place: parse_place(text),
        day,
        kind,
    }
}

fn parse_place(text: &str) -> Option<String> {
    let re = Regex::new(
        r"(?i)\b(?:in|at)\s+([\p{L}][\p{L} .'-]*?)\s*(?:\b(?:today|tonight|tomorrow|tmrw|tmr|this|on|over|next|right|now|please|later)\b|[?.!,]|$)",
    )
    .ok()?;
    let raw = re.captures(text)?.get(1)?.as_str().trim();
    let lower = raw.to_lowercase();
    let bad_start = ["the ", "a ", "an ", "my ", "me ", "us ", "this ", "that "];
    if raw.is_empty()
        || raw.len() > 60
        || bad_start.iter().any(|b| lower.starts_with(b))
        || matches!(
            lower.as_str(),
            "the" | "weekend" | "general" | "case" | "fact"
        )
    {
        return None;
    }
    Some(title_case(raw))
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(c) => c.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The phrase to look up for a quick fact, or `None` when nothing is left.
pub fn parse_fact_subject(text: &str) -> Option<String> {
    let mut t = text.trim().to_lowercase();
    t = t
        .trim_matches(|c: char| c.is_whitespace() || "?!.,".contains(c))
        .to_string();
    let fillers = Regex::new(
        r"^(?:hey|hi|ok|okay|please|can you|could you|tell me|do you know|i wanna know|i want to know|quick question)[\s,:]+",
    )
    .ok()?;
    while fillers.is_match(&t) {
        t = fillers.replace(&t, "").into_owned();
    }
    let lead = Regex::new(
        r"^(?:(?:what|who)(?:'s|’s|s| is| are| was| were)|tell me about|define|meaning of|look up|search for)(?:\s+(?:the|a|an))?(?:\s+|$)",
    )
    .ok()?;
    t = lead.replace(&t, "").into_owned();
    let t = t.trim();
    if t.chars().count() < 2 {
        return None;
    }
    Some(t.chars().take(100).collect())
}

/// Percent-encode a query value (only unreserved characters stay as they are).
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// One local day of forecast, built from the hourly MET Norway series.
#[derive(Debug, Clone, PartialEq)]
pub struct DayForecast {
    pub date: NaiveDate,
    pub high: f64,
    pub low: f64,
    pub rain_mm: f64,
}

/// Rough local time offset from longitude (solar time, nearest half hour).
/// The sources give no time zone; this only decides which day an hour is in.
fn solar_offset_secs(lon: f64) -> i64 {
    let half_hours = (lon / 7.5).round().clamp(-24.0, 24.0) as i64;
    half_hours * 1800
}

/// Best Wikipedia search hit that has coordinates: (name, lat, lon).
fn parse_geocode(v: &Value) -> Option<(String, f64, f64)> {
    let mut pages: Vec<&Value> = v.pointer("/query/pages")?.as_array()?.iter().collect();
    pages.sort_by_key(|p| p.get("index").and_then(Value::as_i64).unwrap_or(i64::MAX));
    pages.into_iter().find_map(|p| {
        let c = p.pointer("/coordinates/0")?;
        let lat = c.get("lat")?.as_f64()?;
        let lon = c.get("lon")?.as_f64()?;
        let title = p.get("title")?.as_str()?;
        let name = title.split(" (").next().unwrap_or(title).trim().to_string();
        Some((name, lat, lon))
    })
}

/// Group a MET Norway `compact` forecast into local days.
fn daily_from_met(forecast: &Value, offset_secs: i64) -> Option<Vec<DayForecast>> {
    let series = forecast.pointer("/properties/timeseries")?.as_array()?;
    let mut days: Vec<DayForecast> = Vec::new();
    for entry in series {
        let Some(time) = entry
            .get("time")
            .and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        else {
            continue;
        };
        let Some(local) = time.checked_add_signed(chrono::Duration::seconds(offset_secs)) else {
            continue;
        };
        let date = local.date_naive();
        let data = entry.get("data")?;
        let temp = data
            .pointer("/instant/details/air_temperature")
            .and_then(Value::as_f64);
        // Hourly entries carry next_1_hours; the later 6-hourly ones carry
        // next_6_hours at 00, 06, 12 and 18 UTC. Never count a span twice.
        let rain = data
            .pointer("/next_1_hours/details/precipitation_amount")
            .and_then(Value::as_f64)
            .or_else(|| {
                use chrono::Timelike;
                if time.hour() % 6 == 0 {
                    data.pointer("/next_6_hours/details/precipitation_amount")
                        .and_then(Value::as_f64)
                } else {
                    None
                }
            })
            .unwrap_or(0.0);
        if days.last().map(|d| d.date) != Some(date) {
            days.push(DayForecast {
                date,
                high: f64::NEG_INFINITY,
                low: f64::INFINITY,
                rain_mm: 0.0,
            });
        }
        let d = days.last_mut()?;
        if let Some(t) = temp {
            d.high = d.high.max(t);
            d.low = d.low.min(t);
        }
        d.rain_mm += rain;
    }
    days.retain(|d| d.high.is_finite() && d.low.is_finite());
    if days.is_empty() {
        None
    } else {
        Some(days)
    }
}

fn render_weather(
    q: &WeatherQuery,
    place: &str,
    today: NaiveDate,
    days: &[DayForecast],
) -> Option<String> {
    let start = days.iter().position(|d| d.date >= today)?;
    let days = &days[start..];
    let (picked, label): (Vec<&DayForecast>, &str) = match q.day {
        Day::Today => (days.iter().take(1).collect(), "today"),
        Day::Tomorrow => (days.get(1).into_iter().collect(), "tomorrow"),
        Day::DayAfterTomorrow => (days.get(2).into_iter().collect(), "the day after tomorrow"),
        Day::Weekend => (
            days.iter()
                .filter(|d| matches!(d.date.weekday(), Weekday::Sat | Weekday::Sun))
                .take(2)
                .collect(),
            "this weekend",
        ),
    };
    if picked.is_empty() {
        return None;
    }
    let high = picked.iter().map(|d| d.high).fold(f64::MIN, f64::max);
    let low = picked.iter().map(|d| d.low).fold(f64::MAX, f64::min);
    let mm: f64 = picked.iter().map(|d| d.rain_mm).sum();
    let temps = format!("high {high:.0}\u{b0}C, low {low:.0}\u{b0}C");
    let rain_part = if mm < 0.2 {
        "no rain expected".to_string()
    } else {
        format!("about {mm:.1} mm of rain")
    };

    match q.kind {
        Kind::Rain => {
            let word = if mm < 0.2 {
                "unlikely"
            } else if mm < 1.0 {
                "possible"
            } else {
                "likely"
            };
            Some(format!(
                "Rain is {word} in {place} {label}: {rain_part}, {temps}."
            ))
        }
        Kind::Temperature => Some(format!("{place} {label}: {temps}.")),
        Kind::General => Some(format!("{place} {label}: {temps}, {rain_part}.")),
    }
}

/// The intro of the best Wikipedia hit, two sentences, with the CC BY-SA credit.
fn render_fact(found: &Value) -> Option<String> {
    let mut pages: Vec<&Value> = found.pointer("/query/pages")?.as_array()?.iter().collect();
    pages.sort_by_key(|p| p.get("index").and_then(Value::as_i64).unwrap_or(i64::MAX));
    let page = pages.first()?;
    let extract = page.get("extract")?.as_str()?.trim();
    let lower = extract.to_lowercase();
    if extract.is_empty() || lower.contains("may refer to") || lower.contains("can refer to") {
        return None;
    }
    let title = page.get("title").and_then(Value::as_str).unwrap_or("");
    let url = page
        .get("fullurl")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| {
            format!(
                "https://en.wikipedia.org/wiki/{}",
                encode(&title.replace(' ', "_"))
            )
        });
    Some(format!(
        "{} (Wikipedia, CC BY-SA 4.0: {url})",
        first_sentences(&strip_parentheses(extract), 2, 320)
    ))
}

/// Drop "(...)" groups (pronunciations, native spellings) and tidy spaces.
fn strip_parentheses(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ,", ",")
        .replace(" .", ".")
}

/// The first `n` sentences, cut at `max_chars` on a word edge.
pub fn first_sentences(text: &str, n: usize, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut end = chars.len();
    let mut found = 0;
    for i in 0..chars.len() {
        if !matches!(chars[i], '.' | '!' | '?') {
            continue;
        }
        let space_after = chars.get(i + 1).is_some_and(|c| c.is_whitespace());
        let upper_next = chars[i + 1..]
            .iter()
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
        if !(space_after && upper_next) {
            continue;
        }
        // Skip abbreviations such as "St.", "Dr.", "U.S." and initials.
        let word: String = chars[..i]
            .iter()
            .rev()
            .take_while(|c| !c.is_whitespace())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let abbr = word.len() <= 1
            || word.contains('.')
            || matches!(
                word.as_str(),
                "St" | "Dr" | "Mr" | "Mrs" | "Ms" | "Prof" | "Jr" | "Sr" | "vs" | "c" | "ca"
            );
        if abbr && chars[i] == '.' {
            continue;
        }
        found += 1;
        if found == n {
            end = i + 1;
            break;
        }
    }
    let s: String = chars[..end].iter().collect();
    if s.chars().count() <= max_chars {
        return s;
    }
    let cut: String = s.chars().take(max_chars).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut);
    format!("{}...", cut.trim_end_matches([',', ';', ':']))
}

// ---------------------------------------------------------------------------
// Tests (none touch the network)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeHttp {
        routes: Vec<(&'static str, Result<String, HttpError>)>,
        seen: std::sync::Arc<Mutex<Vec<String>>>,
    }

    impl Http for FakeHttp {
        fn get(&self, url: &str) -> Result<String, HttpError> {
            self.seen.lock().unwrap().push(url.to_string());
            for (needle, reply) in &self.routes {
                if url.contains(needle) {
                    return reply.clone();
                }
            }
            Err(HttpError::Other("no route".into()))
        }
    }

    type Seen = std::sync::Arc<Mutex<Vec<String>>>;

    fn connector(
        routes: Vec<(&'static str, Result<String, HttpError>)>,
        place: Option<&str>,
    ) -> (WebLookup, Seen) {
        let seen: Seen = Default::default();
        let http = FakeHttp {
            routes,
            seen: seen.clone(),
        };
        (
            WebLookup::with_http(Box::new(http), place.map(String::from)),
            seen,
        )
    }

    fn args(text: &str) -> Args {
        [("text".to_string(), text.to_string())].into()
    }

    const GEO: &str = r#"{"query":{"pages":[
        {"pageid":2,"title":"Pune (disambiguation)","index":2},
        {"pageid":1,"title":"Pune","index":1,"coordinates":[{"lat":18.5196,"lon":73.8553,"primary":true}]}]}}"#;

    /// A MET Norway style forecast: four 6-hourly entries per UTC day.
    /// Pune is UTC+5 by the solar offset, so all four fall on the same local day.
    fn met() -> String {
        let days = [
            ("2026-11-04", [20.0, 28.0, 30.0, 24.0], [0.0, 0.0, 0.0, 0.0]),
            ("2026-11-05", [21.0, 26.0, 28.4, 23.0], [0.0, 0.5, 3.0, 1.0]),
            ("2026-11-06", [19.0, 29.0, 31.0, 24.0], [0.0, 0.0, 0.0, 0.0]),
            ("2026-11-07", [18.0, 30.0, 32.1, 25.0], [0.0, 0.0, 0.0, 0.0]),
            ("2026-11-08", [19.2, 28.0, 31.5, 24.0], [0.0, 0.0, 0.4, 0.0]),
        ];
        let mut series = Vec::new();
        for (date, temps, rain) in days {
            for (i, hour) in ["00", "06", "12", "18"].iter().enumerate() {
                series.push(serde_json::json!({
                    "time": format!("{date}T{hour}:00:00Z"),
                    "data": {
                        "instant": {"details": {"air_temperature": temps[i]}},
                        "next_6_hours": {"details": {"precipitation_amount": rain[i]}}
                    }
                }));
            }
        }
        serde_json::json!({"properties": {"timeseries": series}}).to_string()
    }

    fn clock() -> chrono::DateTime<chrono::Utc> {
        "2026-11-04T10:00:00Z".parse().unwrap()
    }

    fn routes_ok() -> Vec<(&'static str, Result<String, HttpError>)> {
        vec![
            ("prop=coordinates", Ok(GEO.into())),
            ("api.met.no", Ok(met())),
        ]
    }

    fn connector_at_clock(
        routes: Vec<(&'static str, Result<String, HttpError>)>,
        place: Option<&str>,
    ) -> (WebLookup, Seen) {
        let (c, seen) = connector(routes, place);
        (c.with_clock(clock), seen)
    }

    #[test]
    fn parses_weather_requests() {
        let q = parse_weather("Will it rain tomorrow?");
        assert_eq!(
            q,
            WeatherQuery {
                place: None,
                day: Day::Tomorrow,
                kind: Kind::Rain
            }
        );
        let q = parse_weather("gonna rain tmrw?");
        assert_eq!((q.place, q.day, q.kind), (None, Day::Tomorrow, Kind::Rain));
        let q = parse_weather("will it rain in pune tomorrow");
        assert_eq!(q.place.as_deref(), Some("Pune"));
        let q = parse_weather("what's the weather in New York this weekend?");
        assert_eq!(q.place.as_deref(), Some("New York"));
        assert_eq!((q.day, q.kind), (Day::Weekend, Kind::General));
        let q = parse_weather("how hot is it in Delhi");
        assert_eq!(q.place.as_deref(), Some("Delhi"));
        assert_eq!((q.day, q.kind), (Day::Today, Kind::Temperature));
        // "in the morning" is not a place.
        assert_eq!(parse_weather("will it rain in the morning").place, None);
    }

    #[test]
    fn parses_fact_subjects() {
        let s = |t| parse_fact_subject(t);
        assert_eq!(
            s("What's the capital of Australia?").as_deref(),
            Some("capital of australia")
        );
        assert_eq!(s("hey, who is Marie Curie").as_deref(), Some("marie curie"));
        assert_eq!(
            s("how tall is mount everest").as_deref(),
            Some("how tall is mount everest")
        );
        assert_eq!(s("what is"), None);
        assert_eq!(s("?"), None);
    }

    #[test]
    fn splits_sentences() {
        let t = "Canberra is the capital of Australia. It is in the ACT. It was founded in 1913.";
        assert_eq!(
            first_sentences(t, 2, 300),
            "Canberra is the capital of Australia. It is in the ACT."
        );
        let t = "Dr. Who is a show. It is old. Really.";
        assert_eq!(first_sentences(t, 1, 300), "Dr. Who is a show.");
        assert_eq!(first_sentences("No end here", 2, 300), "No end here");
        assert_eq!(
            strip_parentheses("The Tower (  EYE-fəl; French: x ) is tall (very)."),
            "The Tower is tall."
        );
        assert!(first_sentences(&"word ".repeat(100), 2, 50).ends_with("..."));
    }

    #[test]
    fn rain_tomorrow_in_named_place() {
        let (c, seen) = connector_at_clock(routes_ok(), None);
        let out = c
            .execute("weather", &args("Will it rain in Pune tomorrow?"))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Rain is likely in Pune tomorrow: about 4.5 mm of rain, high 28\u{b0}C, low 21\u{b0}C. (Data from MET Norway)"
        );
        // PS3: the sentence never leaves the device.
        let seen = seen.lock().unwrap();
        for url in seen.iter() {
            assert!(!url.to_lowercase().contains("will"), "{url}");
            assert!(!url.contains("rain%20in"), "{url}");
        }
        // MET Norway: at most 4 decimals.
        assert!(seen[1].contains("lat=18.5196&lon=73.8553"), "{}", seen[1]);
    }

    #[test]
    fn uses_saved_place_and_asks_when_none() {
        let (c, seen) = connector_at_clock(routes_ok(), Some("Pune"));
        let out = c.execute("weather", &args("gonna rain tmrw?")).unwrap();
        assert!(out.0["message"].starts_with("Rain is likely in Pune tomorrow"));
        assert!(seen.lock().unwrap()[0].contains("gsrsearch=Pune"));
        assert_eq!(c.check("weather", &args("gonna rain tmrw?")), Ok(true));

        let (c, seen) = connector_at_clock(routes_ok(), None);
        let err = c.execute("weather", &args("gonna rain tmrw?")).unwrap_err();
        assert_eq!(err, ConnError::Failed(NO_PLACE_MESSAGE.into()));
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(c.check("weather", &args("gonna rain tmrw?")), Ok(false));
    }

    #[test]
    fn weekend_and_temperature_answers() {
        let (c, _) = connector_at_clock(routes_ok(), Some("Pune"));
        let out = c.execute("weather", &args("weather this weekend")).unwrap();
        assert_eq!(
            out.0["message"],
            "Pune this weekend: high 32\u{b0}C, low 18\u{b0}C, about 0.4 mm of rain. (Data from MET Norway)"
        );
        let out = c.execute("weather", &args("how hot is it today")).unwrap();
        assert_eq!(
            out.0["message"],
            "Pune today: high 30\u{b0}C, low 20\u{b0}C. (Data from MET Norway)"
        );
        let out = c.execute("weather", &args("will it rain today")).unwrap();
        assert!(out.0["message"].starts_with("Rain is unlikely in Pune today: no rain expected"));
    }

    #[test]
    fn capital_of_australia() {
        let (c, seen) = connector(
            vec![(
                "prop=extracts",
                Ok(r#"{"query":{"pages":[{"title":"Canberra","index":1,"fullurl":"https://en.wikipedia.org/wiki/Canberra","extract":"Canberra is the capital city of Australia. It is located at the northern end of the Australian Capital Territory. More text."}]}}"#.into()),
            )],
            None,
        );
        let out = c
            .execute("fact", &args("What's the capital of Australia?"))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Canberra is the capital city of Australia. It is located at the northern end of the Australian Capital Territory. (Wikipedia, CC BY-SA 4.0: https://en.wikipedia.org/wiki/Canberra)"
        );
        assert!(seen.lock().unwrap()[0].contains("gsrsearch=capital%20of%20australia"));
    }

    #[test]
    fn offline_gives_the_exact_message() {
        let (c, _) = connector(vec![("", Err(HttpError::Offline))], Some("Pune"));
        let expected = ConnError::Failed(
            "That needs an internet connection, and I can't reach one right now.".into(),
        );
        assert_eq!(
            c.execute("fact", &args("What's the capital of Australia?"))
                .unwrap_err(),
            expected
        );
        assert_eq!(
            c.execute("weather", &args("will it rain tomorrow"))
                .unwrap_err(),
            expected
        );
    }

    #[test]
    fn bad_replies_never_panic() {
        for body in [
            "",
            "not json",
            "{}",
            "[]",
            "null",
            r#"{"results":[{}]}"#,
            r#"{"query":{"pages":[{}]}}"#,
        ] {
            let (c, _) = connector_at_clock(vec![("", Ok(body.into()))], Some("Pune"));
            for (action, text) in [("fact", "capital of peru"), ("weather", "rain tomorrow")] {
                let err = c.execute(action, &args(text)).unwrap_err();
                let ConnError::Failed(msg) = err else {
                    panic!("wrong error for {body}")
                };
                assert!(msg.starts_with("I couldn't find that"), "{body}: {msg}");
            }
        }
        // Server error and disambiguation pages.
        let (c, _) = connector(
            vec![("", Err(HttpError::Other("500".into())))],
            Some("Pune"),
        );
        assert!(c.execute("fact", &args("capital of peru")).is_err());
        let page = serde_json::json!({"query": {"pages": [
            {"title": "X", "index": 1, "extract": "X may refer to:"}]}});
        assert_eq!(render_fact(&page), None);
    }

    #[test]
    fn user_agent_identifies_the_app() {
        assert!(USER_AGENT.starts_with("Jitmo/"));
        assert!(USER_AGENT.contains("github.com"));
    }

    #[test]
    #[ignore = "uses the network: cargo test -p jitmo-connectors live -- --ignored --nocapture"]
    fn live_lookups() {
        let c = WebLookup::new(None);
        for (action, text) in [
            ("weather", "will it rain in London tomorrow"),
            ("weather", "what's the weather in Pune"),
            ("fact", "Eiffel Tower"),
            ("fact", "what is the capital of Australia"),
        ] {
            let t = std::time::Instant::now();
            let r = c.execute(action, &args(text));
            println!(
                "[{} ms] {action} '{text}' -> {r:?}",
                t.elapsed().as_millis()
            );
        }
    }

    #[test]
    fn unknown_action_and_missing_arg() {
        let (c, _) = connector(vec![], None);
        assert!(matches!(
            c.execute("nope", &args("x")),
            Err(ConnError::UnknownAction(_))
        ));
        assert!(matches!(
            c.execute("fact", &Args::new()),
            Err(ConnError::MissingArg(_))
        ));
        assert_eq!(c.id(), "web_lookup");
        assert_eq!(c.actions().len(), 2);
    }
}
