//! `web_lookup`: short answers that need the internet (tasks Q2, Q3).
//!
//! - `weather` the forecast for a day (rain, temperature or both) (Q2)
//! - `fact`    a quick fact from Wikipedia (Q3)
//!
//! Privacy (PS3): only search terms leave the device. The user's sentence is
//! parsed here and only the place name (weather) or the subject phrase (fact)
//! is put in a request URL. Never the whole sentence, files or contacts.
//!
//! Services (pending the owner's decision): Open-Meteo for weather (no key) and
//! the Wikipedia search and REST summary APIs for facts.
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

/// The real HTTP client: short timeout, built-in Windows TLS.
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
            .timeout_global(Some(Duration::from_secs(6)))
            .user_agent("jitmo/0.1 (local assistant)")
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
}

impl WebLookup {
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
        }
    }

    fn weather(&self, text: &str) -> Result<String, ConnError> {
        let q = parse_weather(text);
        let place = q
            .place
            .clone()
            .or_else(|| self.default_place.clone())
            .ok_or_else(|| ConnError::Failed(NO_PLACE_MESSAGE.into()))?;

        let geo_url = format!(
            "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
            encode(&place)
        );
        let geo = self.fetch_json(&geo_url)?;
        let hit = geo
            .pointer("/results/0")
            .ok_or_else(|| ConnError::Failed("I couldn't find that place.".into()))?;
        let (lat, lon) = match (
            hit.get("latitude").and_then(Value::as_f64),
            hit.get("longitude").and_then(Value::as_f64),
        ) {
            (Some(a), Some(b)) => (a, b),
            _ => return Err(not_found()),
        };
        let name = hit
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&place)
            .to_string();

        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
             &daily=precipitation_probability_max,temperature_2m_max,temperature_2m_min\
             &timezone=auto&forecast_days=7"
        );
        let forecast = self.fetch_json(&url)?;
        render_weather(&q, &name, &forecast).ok_or_else(not_found)
    }

    fn fact(&self, text: &str) -> Result<String, ConnError> {
        let subject = parse_fact_subject(text).ok_or_else(not_found)?;
        let search_url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&srlimit=1&format=json",
            encode(&subject)
        );
        let found = self.fetch_json(&search_url)?;
        let title = found
            .pointer("/query/search/0/title")
            .and_then(Value::as_str)
            .ok_or_else(not_found)?;
        let summary_url = format!(
            "https://en.wikipedia.org/api/rest_v1/page/summary/{}",
            encode(&title.replace(' ', "_"))
        );
        let summary = self.fetch_json(&summary_url)?;
        render_fact(&summary).ok_or_else(not_found)
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

fn num_list(daily: &Value, key: &str) -> Vec<Option<f64>> {
    daily
        .get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().map(Value::as_f64).collect())
        .unwrap_or_default()
}

fn render_weather(q: &WeatherQuery, place: &str, forecast: &Value) -> Option<String> {
    let daily = forecast.get("daily")?;
    let dates: Vec<NaiveDate> = daily
        .get("time")?
        .as_array()?
        .iter()
        .filter_map(|v| NaiveDate::parse_from_str(v.as_str()?, "%Y-%m-%d").ok())
        .collect();
    let rain = num_list(daily, "precipitation_probability_max");
    let hi = num_list(daily, "temperature_2m_max");
    let lo = num_list(daily, "temperature_2m_min");

    let (idx, label): (Vec<usize>, &str) = match q.day {
        Day::Today => (vec![0], "today"),
        Day::Tomorrow => (vec![1], "tomorrow"),
        Day::DayAfterTomorrow => (vec![2], "the day after tomorrow"),
        Day::Weekend => (
            dates
                .iter()
                .enumerate()
                .filter(|(_, d)| matches!(d.weekday(), Weekday::Sat | Weekday::Sun))
                .map(|(i, _)| i)
                .take(2)
                .collect(),
            "this weekend",
        ),
    };
    if idx.is_empty() || idx.iter().any(|&i| i >= dates.len()) {
        return None;
    }
    let pick = |v: &[Option<f64>]| -> Vec<f64> {
        idx.iter()
            .filter_map(|&i| v.get(i).copied().flatten())
            .collect()
    };
    let max = |v: Vec<f64>| {
        v.into_iter()
            .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
    };
    let min = |v: Vec<f64>| {
        v.into_iter()
            .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))))
    };
    let chance = max(pick(&rain));
    let high = max(pick(&hi));
    let low = min(pick(&lo));

    let temps = match (high, low) {
        (Some(h), Some(l)) => Some(format!("high {h:.0}\u{b0}C, low {l:.0}\u{b0}C")),
        _ => None,
    };
    let rain_part = chance.map(|c| format!("{c:.0}% chance of rain"));

    match q.kind {
        Kind::Rain => {
            let c = chance?;
            let word = if c < 20.0 {
                "unlikely"
            } else if c < 50.0 {
                "possible"
            } else {
                "likely"
            };
            Some(format!(
                "Rain is {word} in {place} {label}: {c:.0}% chance{}.",
                temps.map(|t| format!(", {t}")).unwrap_or_default()
            ))
        }
        Kind::Temperature => Some(format!("{place} {label}: {}.", temps?)),
        Kind::General => {
            let parts: Vec<String> = [temps, rain_part].into_iter().flatten().collect();
            if parts.is_empty() {
                None
            } else {
                Some(format!("{place} {label}: {}.", parts.join(", ")))
            }
        }
    }
}

fn render_fact(summary: &Value) -> Option<String> {
    if summary.get("type").and_then(Value::as_str) == Some("disambiguation") {
        return None;
    }
    let extract = summary.get("extract")?.as_str()?.trim();
    if extract.is_empty() {
        return None;
    }
    Some(first_sentences(extract, 2, 320))
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

    const GEO: &str = r#"{"results":[{"name":"Pune","latitude":18.5196,"longitude":73.8553}]}"#;
    // 2026-11-07 is a Saturday.
    const FORECAST: &str = r#"{"daily":{
        "time":["2026-11-04","2026-11-05","2026-11-06","2026-11-07","2026-11-08","2026-11-09","2026-11-10"],
        "precipitation_probability_max":[10,70,5,0,15,null,40],
        "temperature_2m_max":[30.2,28.4,31.0,32.1,31.5,30.0,29.0],
        "temperature_2m_min":[20.0,21.4,19.0,18.5,19.2,20.0,21.0]}}"#;

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
        assert!(first_sentences(&"word ".repeat(100), 2, 50).ends_with("..."));
    }

    #[test]
    fn rain_tomorrow_in_named_place() {
        let (c, seen) = connector(
            vec![
                ("geocoding-api", Ok(GEO.into())),
                ("api.open-meteo", Ok(FORECAST.into())),
            ],
            None,
        );
        let out = c
            .execute("weather", &args("Will it rain in Pune tomorrow?"))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Rain is likely in Pune tomorrow: 70% chance, high 28\u{b0}C, low 21\u{b0}C."
        );
        // PS3: the sentence never leaves the device.
        for url in seen.lock().unwrap().iter() {
            assert!(!url.to_lowercase().contains("will"), "{url}");
            assert!(!url.contains("rain%20in"), "{url}");
        }
    }

    #[test]
    fn uses_saved_place_and_asks_when_none() {
        let routes = || {
            vec![
                ("geocoding-api", Ok(GEO.to_string())),
                ("api.open-meteo", Ok(FORECAST.to_string())),
            ]
        };
        let (c, seen) = connector(routes(), Some("Pune"));
        let out = c.execute("weather", &args("gonna rain tmrw?")).unwrap();
        assert!(out.0["message"].starts_with("Rain is likely in Pune tomorrow"));
        assert!(seen.lock().unwrap()[0].contains("name=Pune"));
        assert_eq!(c.check("weather", &args("gonna rain tmrw?")), Ok(true));

        let (c, seen) = connector(routes(), None);
        let err = c.execute("weather", &args("gonna rain tmrw?")).unwrap_err();
        assert_eq!(err, ConnError::Failed(NO_PLACE_MESSAGE.into()));
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(c.check("weather", &args("gonna rain tmrw?")), Ok(false));
    }

    #[test]
    fn weekend_and_temperature_answers() {
        let (c, _) = connector(
            vec![
                ("geocoding-api", Ok(GEO.into())),
                ("api.open-meteo", Ok(FORECAST.into())),
            ],
            Some("Pune"),
        );
        let out = c.execute("weather", &args("weather this weekend")).unwrap();
        assert_eq!(
            out.0["message"],
            "Pune this weekend: high 32\u{b0}C, low 18\u{b0}C, 15% chance of rain."
        );
        let out = c.execute("weather", &args("how hot is it today")).unwrap();
        assert_eq!(
            out.0["message"],
            "Pune today: high 30\u{b0}C, low 20\u{b0}C."
        );
    }

    #[test]
    fn capital_of_australia() {
        let (c, seen) = connector(
            vec![
                (
                    "list=search",
                    Ok(r#"{"query":{"search":[{"title":"Canberra"}]}}"#.into()),
                ),
                (
                    "page/summary/Canberra",
                    Ok(r#"{"type":"standard","extract":"Canberra is the capital city of Australia. It is located at the northern end of the Australian Capital Territory. More text."}"#.into()),
                ),
            ],
            None,
        );
        let out = c
            .execute("fact", &args("What's the capital of Australia?"))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Canberra is the capital city of Australia. It is located at the northern end of the Australian Capital Territory."
        );
        assert!(seen.lock().unwrap()[0].contains("srsearch=capital%20of%20australia"));
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
        for body in ["", "not json", "{}", "[]", "null", r#"{"results":[{}]}"#] {
            let (c, _) = connector(vec![("", Ok(body.into()))], Some("Pune"));
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
        let summary = serde_json::json!({"type": "disambiguation", "extract": "X may refer to:"});
        assert_eq!(render_fact(&summary), None);
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
