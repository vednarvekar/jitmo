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
    "Jitmo/0.1 (+https://github.com/vednarvekar/jitmo; local assistant bot) ureq/3";

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
// Providers and fallback chains
// ---------------------------------------------------------------------------
//
// No single outside service may break an answer. Each need (geocoding,
// forecast, fact) has an ordered list of providers; the first that answers
// wins and a failure moves on to the next. The order can be changed (or a
// dead provider dropped) by the `lookup_providers` setting, e.g.
// `geocode=wikipedia;weather=metno;fact=wikipedia`, without a new app
// release. The same setting is where a downloaded remote config would land.

/// A place with coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Finds coordinates for a place name.
pub trait Geocoder: Send + Sync {
    fn id(&self) -> &'static str;
    fn find(&self, http: &dyn Http, place: &str) -> Result<Place, HttpError>;
}

/// Gives daily forecasts for coordinates.
pub trait Forecaster: Send + Sync {
    fn id(&self) -> &'static str;
    /// The credit line the licence asks for.
    fn credit(&self) -> &'static str;
    fn days(
        &self,
        http: &dyn Http,
        at: &Place,
        offset_secs: i64,
    ) -> Result<Vec<DayForecast>, HttpError>;
}

/// Answers a quick-fact question with a short text, credit included.
pub trait FactSource: Send + Sync {
    fn id(&self) -> &'static str;
    fn fact(&self, http: &dyn Http, subject: &str) -> Result<String, HttpError>;
}

/// GET and parse JSON. Bad JSON counts as "nothing found", never a panic.
fn get_json(http: &dyn Http, url: &str) -> Result<Value, HttpError> {
    let body = http.get(url)?;
    serde_json::from_str(&body).map_err(|_| HttpError::NotFound)
}

/// Geocoding by the coordinates of the best English Wikipedia article.
pub struct WikipediaGeocoder;

impl Geocoder for WikipediaGeocoder {
    fn id(&self) -> &'static str {
        "wikipedia"
    }
    fn find(&self, http: &dyn Http, place: &str) -> Result<Place, HttpError> {
        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch={}\
             &gsrlimit=3&prop=coordinates&format=json&formatversion=2",
            encode(place)
        );
        let (name, lat, lon) = parse_geocode(&get_json(http, &url)?).ok_or(HttpError::NotFound)?;
        Ok(Place { name, lat, lon })
    }
}

/// MET Norway locationforecast 2.0.
pub struct MetNoForecaster;

impl Forecaster for MetNoForecaster {
    fn id(&self) -> &'static str {
        "metno"
    }
    fn credit(&self) -> &'static str {
        WEATHER_CREDIT
    }
    fn days(
        &self,
        http: &dyn Http,
        at: &Place,
        offset_secs: i64,
    ) -> Result<Vec<DayForecast>, HttpError> {
        // MET Norway rejects more than 4 decimals.
        let url = format!(
            "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={:.4}&lon={:.4}",
            at.lat, at.lon
        );
        daily_from_met(&get_json(http, &url)?, offset_secs).ok_or(HttpError::NotFound)
    }
}

/// Facts from the English Wikipedia Action API (search plus intro in one call).
pub struct WikipediaFacts;

impl FactSource for WikipediaFacts {
    fn id(&self) -> &'static str {
        "wikipedia"
    }
    fn fact(&self, http: &dyn Http, subject: &str) -> Result<String, HttpError> {
        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch={}\
             &gsrlimit=1&prop=extracts%7Cinfo&exintro=1&explaintext=1&exchars=700&inprop=url\
             &format=json&formatversion=2",
            encode(subject)
        );
        render_fact(&get_json(http, &url)?).ok_or(HttpError::NotFound)
    }
}

/// Backup geocoder: the public Photon server (OpenStreetMap data, ODbL). Fair
/// use only, which is fine because it is asked only when Wikipedia fails.
pub struct PhotonGeocoder;

impl Geocoder for PhotonGeocoder {
    fn id(&self) -> &'static str {
        "photon"
    }
    fn find(&self, http: &dyn Http, place: &str) -> Result<Place, HttpError> {
        let url = format!(
            "https://photon.komoot.io/api/?q={}&limit=1&lang=en",
            encode(place)
        );
        let v = get_json(http, &url)?;
        let f = v.pointer("/features/0").ok_or(HttpError::NotFound)?;
        let lon = f.pointer("/geometry/coordinates/0").and_then(Value::as_f64);
        let lat = f.pointer("/geometry/coordinates/1").and_then(Value::as_f64);
        let name = f.pointer("/properties/name").and_then(Value::as_str);
        match (name, lat, lon) {
            (Some(name), Some(lat), Some(lon)) => Ok(Place {
                name: name.to_string(),
                lat,
                lon,
            }),
            _ => Err(HttpError::NotFound),
        }
    }
}

/// Backup forecaster for US points only: the National Weather Service
/// (public domain, no key). Gives a chance of rain, not an amount. Anywhere
/// outside the US it answers 404, which counts as "nothing found".
pub struct NwsForecaster;

impl Forecaster for NwsForecaster {
    fn id(&self) -> &'static str {
        "nws"
    }
    fn credit(&self) -> &'static str {
        "(Data from the US National Weather Service)"
    }
    fn days(
        &self,
        http: &dyn Http,
        at: &Place,
        _offset_secs: i64,
    ) -> Result<Vec<DayForecast>, HttpError> {
        let points = get_json(
            http,
            &format!("https://api.weather.gov/points/{:.4},{:.4}", at.lat, at.lon),
        )?;
        let hourly = points
            .pointer("/properties/forecastHourly")
            .and_then(Value::as_str)
            .filter(|u| u.starts_with("https://api.weather.gov/"))
            .ok_or(HttpError::NotFound)?;
        daily_from_nws(&get_json(http, hourly)?).ok_or(HttpError::NotFound)
    }
}

/// Group the NWS hourly periods into local days (each period carries its own
/// UTC offset, so its date is already the local one).
fn daily_from_nws(forecast: &Value) -> Option<Vec<DayForecast>> {
    let periods = forecast.pointer("/properties/periods")?.as_array()?;
    let mut days: Vec<DayForecast> = Vec::new();
    for p in periods {
        let Some(start) = p
            .get("startTime")
            .and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        else {
            continue;
        };
        let Some(mut temp) = p.get("temperature").and_then(Value::as_f64) else {
            continue;
        };
        if p.get("temperatureUnit").and_then(Value::as_str) != Some("C") {
            temp = (temp - 32.0) * 5.0 / 9.0;
        }
        let chance = p
            .pointer("/probabilityOfPrecipitation/value")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let date = start.date_naive();
        if days.last().map(|d| d.date) != Some(date) {
            days.push(DayForecast {
                date,
                high: f64::NEG_INFINITY,
                low: f64::INFINITY,
                rain_mm: 0.0,
                rain_chance: Some(0.0),
            });
        }
        let d = days.last_mut()?;
        d.high = d.high.max(temp);
        d.low = d.low.min(temp);
        d.rain_chance = Some(d.rain_chance.unwrap_or(0.0).max(chance));
    }
    days.retain(|d| d.high.is_finite() && d.low.is_finite());
    (!days.is_empty()).then_some(days)
}

/// Backup facts: Wikipedia's REST search plus page summary. The same site as
/// the primary but different endpoints, so it covers an Action API fault.
pub struct WikipediaRestFacts;

impl FactSource for WikipediaRestFacts {
    fn id(&self) -> &'static str {
        "wikipedia_rest"
    }
    fn fact(&self, http: &dyn Http, subject: &str) -> Result<String, HttpError> {
        let search = get_json(
            http,
            &format!(
                "https://en.wikipedia.org/w/rest.php/v1/search/page?q={}&limit=1",
                encode(subject)
            ),
        )?;
        let key = search
            .pointer("/pages/0/key")
            .and_then(Value::as_str)
            .ok_or(HttpError::NotFound)?;
        let s = get_json(
            http,
            &format!(
                "https://en.wikipedia.org/api/rest_v1/page/summary/{}",
                encode(key)
            ),
        )?;
        if s.get("type").and_then(Value::as_str) == Some("disambiguation") {
            return Err(HttpError::NotFound);
        }
        // Reuse the Action API renderer by giving it the same shape.
        let shaped = serde_json::json!({"query": {"pages": [{
            "index": 1,
            "title": s.get("title").cloned().unwrap_or(Value::Null),
            "extract": s.get("extract").cloned().unwrap_or(Value::Null),
            "fullurl": s.pointer("/content_urls/desktop/page").cloned().unwrap_or(Value::Null),
        }]}});
        render_fact(&shaped).ok_or(HttpError::NotFound)
    }
}

/// Combine the failures of a whole chain into the one that is told to the user.
/// All offline: offline. Any real failure: a general failure. Otherwise: not found.
fn combine(errors: Vec<HttpError>) -> HttpError {
    if errors.is_empty() || errors.iter().all(|e| *e == HttpError::NotFound) {
        return HttpError::NotFound;
    }
    if errors.iter().all(|e| *e == HttpError::Offline) {
        return HttpError::Offline;
    }
    if errors.iter().any(|e| matches!(e, HttpError::Other(_))) {
        return HttpError::Other("every provider failed".into());
    }
    HttpError::NotFound
}

/// Try each provider in order; the first success wins.
fn chain<P: ?Sized, T>(
    providers: &[&P],
    mut call: impl FnMut(&P) -> Result<T, HttpError>,
) -> Result<T, HttpError> {
    let mut errors = Vec::new();
    for p in providers {
        match call(p) {
            Ok(v) => return Ok(v),
            Err(e) => errors.push(e),
        }
    }
    Err(combine(errors))
}

/// The providers named for `key` in a `key=a,b;key2=c` config string, if any.
fn parse_order(config: &str, key: &str) -> Option<Vec<String>> {
    config.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let ids: Vec<String> = v
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        (!ids.is_empty()).then_some(ids)
    })
}

/// `all` ordered as the config says. Unknown ids are ignored; when nothing
/// usable is named, the built-in order is used.
fn ordered<'a, P: ?Sized>(
    all: &'a [Box<P>],
    order: Option<&[String]>,
    id: impl Fn(&P) -> &'static str,
) -> Vec<&'a P> {
    let picked: Vec<&P> = match order {
        Some(ids) => ids
            .iter()
            .filter_map(|want| all.iter().find(|p| id(p) == want).map(|p| &**p))
            .collect(),
        None => Vec::new(),
    };
    if picked.is_empty() {
        all.iter().map(|p| &**p).collect()
    } else {
        picked
    }
}

// ---------------------------------------------------------------------------
// Cache (last good answers, shown when the network or every provider is down)
// ---------------------------------------------------------------------------

const CACHE_LIMIT: usize = 60;
const WEATHER_FRESH_SECS: i64 = 20 * 60;
const WEATHER_STALE_SECS: i64 = 12 * 3600;
const FACT_FRESH_SECS: i64 = 24 * 3600;
const FACT_STALE_SECS: i64 = 30 * 24 * 3600;

/// Loads / saves the whole cache as one text value (the app keeps it in a setting).
type CacheLoad = Box<dyn Fn() -> Option<String> + Send + Sync>;
type CacheSave = Box<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct Cache {
    /// key -> (unix seconds, answer)
    entries: std::sync::Mutex<Option<std::collections::HashMap<String, (i64, String)>>>,
    load: Option<CacheLoad>,
    save: Option<CacheSave>,
}

impl Cache {
    fn with_entries<R>(
        &self,
        f: impl FnOnce(&mut std::collections::HashMap<String, (i64, String)>) -> R,
    ) -> R {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(|| {
            self.load
                .as_ref()
                .and_then(|l| l())
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default()
        });
        f(map)
    }

    /// An answer younger than `max_age` seconds.
    fn get(&self, key: &str, now: i64, max_age: i64) -> Option<(i64, String)> {
        self.with_entries(|m| {
            m.get(key)
                .filter(|(at, _)| now - at <= max_age && now >= *at)
                .cloned()
        })
    }

    fn put(&self, key: &str, now: i64, answer: &str) {
        let text = self.with_entries(|m| {
            m.insert(key.to_string(), (now, answer.to_string()));
            while m.len() > CACHE_LIMIT {
                let Some(oldest) = m
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                m.remove(&oldest);
            }
            serde_json::to_string(m).ok()
        });
        if let (Some(save), Some(text)) = (&self.save, text) {
            save(&text);
        }
    }
}

/// "5 min ago", "3 h ago", "2 days ago".
fn ago(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 3600 {
        format!("{} min ago", (secs / 60).max(1))
    } else if secs < 48 * 3600 {
        format!("{} h ago", secs / 3600)
    } else {
        format!("{} days ago", secs / 86400)
    }
}

// ---------------------------------------------------------------------------
// Connector
// ---------------------------------------------------------------------------

type StringSource = Box<dyn Fn() -> Option<String> + Send + Sync>;

pub struct WebLookup {
    http: Box<dyn Http>,
    /// The place saved in the app settings, used when the request names none.
    default_place: Option<String>,
    /// Reads the saved place each time, so "set my city" works without a restart.
    place_source: Option<StringSource>,
    /// Reads the `lookup_providers` config each time (order / switch off providers).
    config_source: Option<StringSource>,
    /// The clock, replaceable in tests.
    now: Box<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync>,
    cache: Cache,
    geocoders: Vec<Box<dyn Geocoder>>,
    forecasters: Vec<Box<dyn Forecaster>>,
    facts: Vec<Box<dyn FactSource>>,
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

    /// Read the provider order live (the `lookup_providers` setting).
    pub fn with_config_source(
        mut self,
        source: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.config_source = Some(Box::new(source));
        self
    }

    /// Keep the answer cache across restarts: `load` returns the saved text,
    /// `save` stores it.
    pub fn with_cache_store(
        mut self,
        load: impl Fn() -> Option<String> + Send + Sync + 'static,
        save: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        self.cache.load = Some(Box::new(load));
        self.cache.save = Some(Box::new(save));
        self
    }

    /// Add providers after the built-in ones (tests and later providers).
    pub fn with_geocoder(mut self, g: Box<dyn Geocoder>) -> Self {
        self.geocoders.push(g);
        self
    }
    pub fn with_forecaster(mut self, f: Box<dyn Forecaster>) -> Self {
        self.forecasters.push(f);
        self
    }
    pub fn with_fact_source(mut self, f: Box<dyn FactSource>) -> Self {
        self.facts.push(f);
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

    fn config(&self) -> String {
        self.config_source
            .as_ref()
            .and_then(|f| f())
            .unwrap_or_default()
    }

    pub fn new(default_place: Option<String>) -> Self {
        Self::with_http(Box::new(UreqHttp::new()), default_place)
    }

    pub fn with_http(http: Box<dyn Http>, default_place: Option<String>) -> Self {
        WebLookup {
            http,
            default_place: default_place.filter(|p| !p.trim().is_empty()),
            place_source: None,
            config_source: None,
            now: Box::new(chrono::Utc::now),
            cache: Cache::default(),
            geocoders: vec![Box::new(WikipediaGeocoder), Box::new(PhotonGeocoder)],
            forecasters: vec![
                Box::new(MetNoForecaster),
                Box::new(crate::web_weather::FmiEcmwfForecaster),
                Box::new(NwsForecaster),
            ],
            // Wikidata for relation questions, then smart Wikipedia; the two
            // plain Wikipedia sources stay as outage fallbacks.
            facts: vec![
                Box::new(crate::web_facts::BestFacts),
                Box::new(WikipediaFacts),
                Box::new(WikipediaRestFacts),
            ],
        }
    }

    fn weather(&self, text: &str) -> Result<String, ConnError> {
        let q = parse_weather(text);
        let place = q
            .place
            .clone()
            .or_else(|| self.saved_place())
            .ok_or_else(|| ConnError::Failed(NO_PLACE_MESSAGE.into()))?;
        let key = format!("weather|{}|{:?}|{:?}", place.to_lowercase(), q.day, q.kind);
        self.cached(&key, WEATHER_FRESH_SECS, WEATHER_STALE_SECS, || {
            self.fetch_weather(&q, &place)
        })
    }

    /// Up to three page intros for the question: `n`, then `title0`, `url0`, `text0`, ...
    fn passages(&self, question: &str) -> Result<Output, ConnError> {
        let q = question.trim().trim_end_matches(['?', '.', '!']);
        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch={}\n             &gsrlimit=3&prop=extracts%7Cinfo&exintro=1&explaintext=1&exchars=1200&inprop=url\n             &format=json&formatversion=2",
            encode(q)
        );
        let found = get_json(&*self.http, &url).map_err(|e| match e {
            HttpError::Offline => ConnError::Failed(OFFLINE_MESSAGE.into()),
            _ => not_found(),
        })?;
        let mut pages: Vec<&Value> = found
            .pointer("/query/pages")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        pages.sort_by_key(|p| p.get("index").and_then(Value::as_i64).unwrap_or(i64::MAX));
        let mut out = std::collections::BTreeMap::new();
        let mut n = 0;
        for p in pages.iter().take(3) {
            let text = p
                .get("extract")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let lower = text.to_lowercase();
            if text.is_empty() || lower.contains("may refer to") || lower.contains("can refer to") {
                continue;
            }
            let title = p.get("title").and_then(Value::as_str).unwrap_or("");
            let url = p.get("fullurl").and_then(Value::as_str).unwrap_or("");
            out.insert(format!("title{n}"), title.to_string());
            out.insert(format!("url{n}"), url.to_string());
            out.insert(format!("text{n}"), text.to_string());
            n += 1;
        }
        if n == 0 {
            return Err(not_found());
        }
        out.insert("n".to_string(), n.to_string());
        Ok(Output(out))
    }

    fn fact(&self, text: &str) -> Result<String, ConnError> {
        // "What does ephemeral mean": a dictionary, never an encyclopedia intro.
        if let Some(word) = definition_word(text) {
            let key = format!("define|{word}");
            return self.cached(&key, FACT_FRESH_SECS, FACT_STALE_SECS, || {
                define(&*self.http, &word)
            });
        }
        // How/why questions, sums and conversions are not "look up a title": the first
        // Wikipedia hit answers some other question with confidence. Say so instead.
        if !is_lookup_question(text) {
            return Err(ConnError::Failed(
                "I'm not sure about that one, and I'd rather not guess. \
                 Say “search for …” to look it up on the web."
                    .into(),
            ));
        }
        let subject = parse_fact_subject(text).ok_or_else(not_found)?;
        let key = format!("fact|{subject}");
        self.cached(&key, FACT_FRESH_SECS, FACT_STALE_SECS, || {
            let config = self.config();
            let order = parse_order(&config, "fact");
            let sources = ordered(&self.facts, order.as_deref(), |p| p.id());
            chain(&sources, |s| s.fact(&*self.http, &subject))
        })
    }

    /// A fresh cached answer if there is one; else ask the providers; when
    /// they all fail for a reason other than "not found", fall back to an
    /// older answer and say how old it is.
    fn cached(
        &self,
        key: &str,
        fresh_secs: i64,
        stale_secs: i64,
        fetch: impl FnOnce() -> Result<String, HttpError>,
    ) -> Result<String, ConnError> {
        let now = (self.now)().timestamp();
        if let Some((_, answer)) = self.cache.get(key, now, fresh_secs) {
            return Ok(answer);
        }
        match fetch() {
            Ok(answer) => {
                self.cache.put(key, now, &answer);
                Ok(answer)
            }
            Err(HttpError::NotFound) => Err(not_found()),
            Err(e) => match self.cache.get(key, now, stale_secs) {
                Some((at, answer)) => Ok(format!(
                    "{answer} (last checked {}, I can't reach the internet for a fresh one)",
                    ago(now - at)
                )),
                None => Err(match e {
                    HttpError::Offline => ConnError::Failed(OFFLINE_MESSAGE.into()),
                    _ => ConnError::Failed(
                        "I couldn't get an answer from the internet just now.".into(),
                    ),
                }),
            },
        }
    }

    fn fetch_weather(&self, q: &WeatherQuery, place: &str) -> Result<String, HttpError> {
        let config = self.config();
        let geocoders = ordered(
            &self.geocoders,
            parse_order(&config, "geocode").as_deref(),
            |p| p.id(),
        );
        let forecasters = ordered(
            &self.forecasters,
            parse_order(&config, "weather").as_deref(),
            |p| p.id(),
        );

        // Only the place name goes out. Tries each geocoder until one finds it.
        let geo = chain(&geocoders, |g| g.find(&*self.http, place))?;
        let offset_secs = solar_offset_secs(geo.lon);
        let today = (self.now)()
            .checked_add_signed(chrono::Duration::seconds(offset_secs))
            .map(|t| t.date_naive())
            .ok_or(HttpError::NotFound)?;
        chain(&forecasters, |f| {
            let days = f.days(&*self.http, &geo, offset_secs)?;
            let text = render_weather(q, &geo.name, today, &days).ok_or(HttpError::NotFound)?;
            Ok(format!("{text} {}", f.credit()))
        })
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
        ["weather", "fact", "passages"]
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
        // Passages for a grounded answer: up to three Wikipedia intros that match the words of
        // the question (the caller judges which one really answers it).
        if action == "passages" {
            return self.passages(text);
        }
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

/// "what does ephemeral mean", "define ephemeral", "meaning of ephemeral" -> "ephemeral".
pub fn definition_word(text: &str) -> Option<String> {
    let t = text
        .trim()
        .trim_matches(|c: char| c.is_whitespace() || "?!.,".contains(c))
        .to_lowercase();
    let re = Regex::new(
        r"^(?:what(?:'s|’s| is| does| do)? (?:the )?(?:meaning of |definition of )|(?:what does|what do) |define |definition of |meaning of )([a-z][a-z'-]{1,30})(?: mean| means)?$",
    )
    .ok()?;
    re.captures(&t).map(|c| c[1].to_string())
}

/// True for "who/what/where is X" style lookups; false for how/why questions, explanations,
/// sums and conversions that an encyclopedia intro cannot answer.
pub fn is_lookup_question(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    let t = t.trim_start_matches(|c: char| !c.is_alphanumeric());
    let bad_start = [
        "how ",
        "why ",
        "explain",
        "solve",
        "calculate",
        "convert",
        "difference between",
        "derivative",
        "integral",
        "compare",
        "when should",
        "can i ",
        "should i ",
    ];
    if bad_start.iter().any(|s| t.starts_with(s)) {
        return false;
    }
    let bad_inside = [
        "how many",
        "how much",
        "how do ",
        "how does ",
        "derivative of",
        "integral of",
        " in a ",
        " in an ",
        " per ",
        "explain ",
        "simply",
        "work?",
    ];
    if bad_inside.iter().any(|s| t.contains(s)) {
        return false;
    }
    // Anything with digits and an operator is arithmetic, not a title.
    !(t.chars().any(|c| c.is_ascii_digit()) && t.chars().any(|c| "+-*/^=".contains(c)))
}

/// One dictionary entry from Wiktionary data (api.dictionaryapi.dev), with its credit.
fn define(http: &dyn Http, word: &str) -> Result<String, HttpError> {
    let url = format!(
        "https://api.dictionaryapi.dev/api/v2/entries/en/{}",
        encode(word)
    );
    let v = get_json(http, &url)?;
    let entry = v
        .as_array()
        .and_then(|a| a.first())
        .ok_or(HttpError::NotFound)?;
    let meaning = entry
        .get("meanings")
        .and_then(Value::as_array)
        .and_then(|m| m.first())
        .ok_or(HttpError::NotFound)?;
    let kind = meaning
        .get("partOfSpeech")
        .and_then(Value::as_str)
        .unwrap_or("");
    let def = meaning
        .pointer("/definitions/0/definition")
        .and_then(Value::as_str)
        .ok_or(HttpError::NotFound)?;
    let word = title_case(word);
    let head = if kind.is_empty() {
        word.clone()
    } else {
        format!("{word} ({kind})")
    };
    Ok(format!(
        "{head}: {} (Wiktionary, CC BY-SA 4.0: https://en.wiktionary.org/wiki/{})",
        def.trim_end_matches('.').to_string() + ".",
        encode(&word.to_lowercase())
    ))
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
    /// Highest chance of rain (percent) when the source gives a chance instead
    /// of an amount (US National Weather Service). `None` for amount sources.
    pub rain_chance: Option<f64>,
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
                rain_chance: None,
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
    // A source that gives a chance (percent) instead of an amount.
    let chance = picked
        .iter()
        .map(|d| d.rain_chance)
        .collect::<Option<Vec<f64>>>()
        .map(|c| c.into_iter().fold(0.0, f64::max));
    let rain_part = match chance {
        Some(c) if c < 10.0 => "no rain expected".to_string(),
        Some(c) => format!("{c:.0}% chance of rain"),
        None if mm < 0.2 => "no rain expected".to_string(),
        None => format!("about {mm:.1} mm of rain"),
    };

    match q.kind {
        Kind::Rain => {
            let word = match chance {
                Some(c) if c < 20.0 => "unlikely",
                Some(c) if c < 50.0 => "possible",
                Some(_) => "likely",
                None if mm < 0.2 => "unlikely",
                None if mm < 1.0 => "possible",
                None => "likely",
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
        // Some source asked Wikipedia for an intro extract, and no URL carries
        // the user's whole sentence (PS3).
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|u| u.contains("prop=extracts")));
        assert!(seen.iter().all(|u| !u.to_lowercase().contains("what")));
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

    /// A second fact source that always answers the same text.
    struct FixedFact(&'static str, Result<&'static str, HttpError>);
    impl FactSource for FixedFact {
        fn id(&self) -> &'static str {
            self.0
        }
        fn fact(&self, _: &dyn Http, _: &str) -> Result<String, HttpError> {
            self.1.clone().map(String::from)
        }
    }

    #[test]
    fn a_failing_provider_falls_through_to_the_next() {
        // The primary (Wikipedia) errors; the backup answers.
        let (c, _) = connector(vec![("", Err(HttpError::Other("503".into())))], None);
        let c = c.with_fact_source(Box::new(FixedFact("backup", Ok("Canberra. (Backup)"))));
        let out = c
            .execute("fact", &args("what is the capital of Australia"))
            .unwrap();
        assert_eq!(out.0["message"], "Canberra. (Backup)");
        // "Nothing found" also moves on to the next provider.
        let (c, _) = connector(vec![("", Ok(r#"{"query":{"pages":[]}}"#.into()))], None);
        let c = c.with_fact_source(Box::new(FixedFact("backup", Ok("Found. (Backup)"))));
        assert_eq!(
            c.execute("fact", &args("capital of peru")).unwrap().0["message"],
            "Found. (Backup)"
        );
    }

    #[test]
    fn the_config_setting_orders_and_switches_off_providers() {
        let (c, _) = connector(vec![("", Err(HttpError::Other("503".into())))], None);
        let c = c
            .with_fact_source(Box::new(FixedFact("backup", Ok("From backup."))))
            .with_config_source(|| Some("fact=backup".into()));
        assert_eq!(
            c.execute("fact", &args("capital of peru")).unwrap().0["message"],
            "From backup."
        );
        // An unknown id falls back to the built-in order.
        assert_eq!(
            parse_order("weather=metno;fact=nope", "fact"),
            Some(vec!["nope".to_string()])
        );
        let (c, _) = connector(
            vec![(
                "prop=extracts",
                Ok(r#"{"query":{"pages":[{"title":"Lima","index":1,"extract":"Lima is the capital of Peru."}]}}"#.into()),
            )],
            None,
        );
        let c = c.with_config_source(|| Some("fact=nope".into()));
        assert!(c.execute("fact", &args("capital of peru")).is_ok());
        assert_eq!(parse_order("fact=", "fact"), None);
        assert_eq!(parse_order("", "fact"), None);
    }

    fn nws_hourly() -> String {
        // Two local days; the offset in startTime decides the date.
        let mut periods = Vec::new();
        for (t, f, pop) in [
            ("2026-11-04T20:00:00-05:00", 50.0, 10),
            ("2026-11-05T02:00:00-05:00", 41.0, 20),
            ("2026-11-05T14:00:00-05:00", 59.0, 60),
            ("2026-11-05T22:00:00-05:00", 45.0, 30),
        ] {
            periods.push(serde_json::json!({
                "startTime": t, "temperature": f, "temperatureUnit": "F",
                "probabilityOfPrecipitation": {"value": pop}
            }));
        }
        serde_json::json!({"properties": {"periods": periods}}).to_string()
    }

    #[test]
    fn weather_falls_back_to_nws_when_met_norway_is_down() {
        let (c, seen) = connector_at_clock(
            vec![
                ("prop=coordinates", Ok(GEO.into())),
                ("api.met.no", Err(HttpError::Other("503".into()))),
                (
                    "api.weather.gov/points",
                    Ok(r#"{"properties":{"forecastHourly":"https://api.weather.gov/gridpoints/X/1,2/forecast/hourly"}}"#.into()),
                ),
                ("forecast/hourly", Ok(nws_hourly())),
            ],
            Some("Pune"),
        );
        let out = c
            .execute("weather", &args("will it rain tomorrow"))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Rain is likely in Pune tomorrow: 60% chance of rain, high 15\u{b0}C, low 5\u{b0}C. (Data from the US National Weather Service)"
        );
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("api.met.no")));
        // A forecastHourly link that points anywhere else is never fetched.
        let (c, seen) = connector_at_clock(
            vec![
                ("prop=coordinates", Ok(GEO.into())),
                ("api.met.no", Err(HttpError::Offline)),
                (
                    "api.weather.gov/points",
                    Ok(r#"{"properties":{"forecastHourly":"https://evil.example/x"}}"#.into()),
                ),
            ],
            Some("Pune"),
        );
        assert!(c
            .execute("weather", &args("will it rain tomorrow"))
            .is_err());
        assert!(!seen.lock().unwrap().iter().any(|u| u.contains("evil")));
    }

    #[test]
    fn geocoding_falls_back_to_photon() {
        let (c, _) = connector_at_clock(
            vec![
                ("wikipedia.org", Err(HttpError::Other("503".into()))),
                (
                    "photon.komoot.io",
                    Ok(r#"{"features":[{"geometry":{"coordinates":[73.8553,18.5196]},"properties":{"name":"Pune"}}]}"#.into()),
                ),
                ("api.met.no", Ok(met())),
            ],
            Some("Pune"),
        );
        let out = c
            .execute("weather", &args("will it rain tomorrow"))
            .unwrap();
        assert!(out.0["message"].starts_with("Rain is likely in Pune tomorrow"));
    }

    #[test]
    fn facts_fall_back_to_wikipedia_rest() {
        let (c, _) = connector(
            vec![
                ("w/api.php", Err(HttpError::Other("503".into()))),
                (
                    "rest.php/v1/search/page",
                    Ok(r#"{"pages":[{"key":"Canberra"}]}"#.into()),
                ),
                (
                    "rest_v1/page/summary/Canberra",
                    Ok(r#"{"type":"standard","title":"Canberra","extract":"Canberra is the capital city of Australia.","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Canberra"}}}"#.into()),
                ),
            ],
            None,
        );
        assert_eq!(
            c.execute("fact", &args("capital of australia")).unwrap().0["message"],
            "Canberra is the capital city of Australia. (Wikipedia, CC BY-SA 4.0: https://en.wikipedia.org/wiki/Canberra)"
        );
    }

    #[test]
    fn errors_combine_into_the_right_message() {
        use HttpError::*;
        assert_eq!(combine(vec![Offline, Offline]), Offline);
        assert_eq!(combine(vec![NotFound, NotFound]), NotFound);
        assert_eq!(combine(vec![Offline, NotFound]), NotFound);
        assert!(matches!(
            combine(vec![Offline, Other("x".into())]),
            Other(_)
        ));
        assert_eq!(combine(vec![]), NotFound);
    }

    #[test]
    fn an_older_answer_is_shown_when_the_network_is_down() {
        use std::sync::atomic::{AtomicI64, Ordering};
        let secs = std::sync::Arc::new(AtomicI64::new(0));
        let t = secs.clone();
        let base: chrono::DateTime<chrono::Utc> = "2026-11-04T10:00:00Z".parse().unwrap();
        let clock = move || base + chrono::Duration::seconds(t.load(Ordering::SeqCst));

        let (c, _) = connector(routes_ok(), Some("Pune"));
        let c = c.with_clock(clock);
        let fresh = c
            .execute("weather", &args("will it rain tomorrow"))
            .unwrap();
        let first = fresh.0["message"].clone();

        // Within the fresh window the cache answers (no providers asked).
        secs.store(60, Ordering::SeqCst);
        let again = c
            .execute("weather", &args("will it rain tomorrow"))
            .unwrap();
        assert_eq!(again.0["message"], first);

        // Same cache, but every provider is now offline and the answer is old.
        let cache_text = c.cache.with_entries(|m| serde_json::to_string(m).unwrap());
        let (c2, _) = connector(vec![("", Err(HttpError::Offline))], Some("Pune"));
        let t2 = secs.clone();
        let c2 = c2
            .with_clock(move || base + chrono::Duration::seconds(t2.load(Ordering::SeqCst)))
            .with_cache_store(move || Some(cache_text.clone()), |_| {});
        secs.store(3 * 3600, Ordering::SeqCst);
        let stale = c2
            .execute("weather", &args("will it rain tomorrow"))
            .unwrap();
        let msg = &stale.0["message"];
        assert!(msg.starts_with(first.as_str()), "{msg}");
        assert!(msg.contains("last checked 3 h ago"), "{msg}");

        // Too old: the honest offline message.
        secs.store(30 * 3600, Ordering::SeqCst);
        assert_eq!(
            c2.execute("weather", &args("will it rain tomorrow"))
                .unwrap_err(),
            ConnError::Failed(OFFLINE_MESSAGE.into())
        );
    }

    #[test]
    fn the_cache_is_saved_and_capped() {
        let saved: std::sync::Arc<Mutex<Option<String>>> = Default::default();
        let s = saved.clone();
        let (c, _) = connector(
            vec![(
                "prop=extracts",
                Ok(r#"{"query":{"pages":[{"title":"Lima","index":1,"extract":"Lima is the capital of Peru."}]}}"#.into()),
            )],
            None,
        );
        let c = c.with_cache_store(|| None, move |t| *s.lock().unwrap() = Some(t.to_string()));
        c.execute("fact", &args("capital of peru")).unwrap();
        assert!(saved
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .contains("fact|capital of peru"));
        for i in 0..(CACHE_LIMIT + 10) {
            c.cache.put(&format!("k{i}"), i as i64, "x");
        }
        assert!(c.cache.with_entries(|m| m.len()) <= CACHE_LIMIT);
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

#[cfg(test)]
mod honest_facts {
    use super::*;

    #[test]
    fn definitions_are_found() {
        assert_eq!(
            definition_word("what does ephemeral mean"),
            Some("ephemeral".into())
        );
        assert_eq!(
            definition_word("define ephemeral"),
            Some("ephemeral".into())
        );
        assert_eq!(
            definition_word("what is the meaning of serendipity?"),
            Some("serendipity".into())
        );
        assert_eq!(definition_word("what is the capital of france"), None);
    }

    #[test]
    fn explanations_and_sums_are_not_lookups() {
        for q in [
            "how do vaccines work",
            "how many grams in a pound",
            "what's the derivative of x squared",
            "explain newton's second law simply",
        ] {
            assert!(!is_lookup_question(q), "{q}");
        }
        for q in [
            "what is the capital of australia",
            "who is ada lovelace",
            "what is photosynthesis",
        ] {
            assert!(is_lookup_question(q), "{q}");
        }
    }
}
