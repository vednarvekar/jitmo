//! Extra fact sources for `web_lookup` (task Q3): better answers to quick
//! questions such as "capital of Australia", "who wrote Hamlet", "how tall is
//! Mount Everest".
//!
//! Privacy (PS3): only the subject phrase (or the entity name cut out of it)
//! ever appears in a request URL. All access goes through [`Http`], so the
//! tests use canned JSON and never touch the network.
//!
//! Sources (all free, keyless, usable commercially):
//! - [`WikidataFacts`]: structured answers for common relation questions
//!   (capital, author, population, height, built/founded date, birthplace,
//!   director, inventor ...). Wikidata structured data is CC0. Says "nothing
//!   found" for any question it does not recognise.
//! - [`WikipediaSmartFacts`]: English Wikipedia with the query rewritten
//!   (title lookup through redirects, so "capital of Australia" finds
//!   Canberra), hits re-ranked by title overlap, and for attribute questions
//!   the sentence of the intro that answers it (CC BY-SA 4.0, linked).
//! - [`BestFacts`]: Wikidata first, then smart Wikipedia. The recommended
//!   first fact source (measured in `live_benchmark`).
//!
//! DuckDuckGo's Instant Answer API is deliberately not a source: its terms
//! allow non-commercial use only, unless DuckDuckGo approves by email.

use crate::web_lookup::{first_sentences, FactSource, Http, HttpError};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Helpers (the ones in web_lookup are private)
// ---------------------------------------------------------------------------

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

/// GET and parse JSON. Bad JSON counts as "nothing found", never a panic.
fn get_json(http: &dyn Http, url: &str) -> Result<Value, HttpError> {
    let body = http.get(url)?;
    serde_json::from_str(&body).map_err(|_| HttpError::NotFound)
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
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

/// Lowercase words of a phrase, letters and digits only.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Words without the little ones that carry no meaning in a title match.
fn content_words(s: &str) -> Vec<String> {
    const STOP: &[&str] = &["the", "a", "an", "of", "in", "on", "and", "to", "for"];
    words(s)
        .into_iter()
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (words(a), words(b));
    let trim = |w: &[String]| -> Vec<String> {
        match w.first().map(String::as_str) {
            Some("the") => w[1..].to_vec(),
            _ => w.to_vec(),
        }
    };
    !a.is_empty() && trim(&a) == trim(&b)
}

// ---------------------------------------------------------------------------
// Understanding the question
// ---------------------------------------------------------------------------

/// The kinds of relation question Wikidata can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Capital,
    Author,
    Creator,
    Composer,
    Population,
    Height,
    Length,
    Area,
    Inception,
    Birthplace,
    Birthdate,
    Director,
    Inventor,
    Founder,
    Ceo,
    Currency,
    Language,
}

impl Relation {
    /// Wikidata properties to try, in order.
    fn properties(self) -> &'static [&'static str] {
        match self {
            Relation::Capital => &["P36"],
            Relation::Author => &["P50"],
            Relation::Creator => &["P170", "P50"],
            Relation::Composer => &["P86"],
            Relation::Population => &["P1082"],
            Relation::Height => &["P2048", "P2044"],
            Relation::Length => &["P2043"],
            Relation::Area => &["P2046"],
            Relation::Inception => &["P571", "P1619"],
            Relation::Birthplace => &["P19"],
            Relation::Birthdate => &["P569"],
            Relation::Director => &["P57"],
            Relation::Inventor => &["P61"],
            Relation::Founder => &["P112"],
            Relation::Ceo => &["P169"],
            Relation::Currency => &["P38"],
            Relation::Language => &["P37"],
        }
    }

    /// Words that mark the sentence of an article intro answering the question.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Relation::Capital => &["capital"],
            Relation::Author => &["written", "wrote", "author", "novel", "play by", " by "],
            Relation::Creator => &["painted", "created", "painting", " by "],
            Relation::Composer => &["composed", "composer", " by "],
            Relation::Population => &["population", "inhabitants", "residents", "people"],
            Relation::Height => &["tall", "height", "elevation", "high", "metres", "feet"],
            Relation::Length => &["long", "length", "kilometres", "miles"],
            Relation::Area => &["area", "square", "km2", "km²"],
            Relation::Inception => &[
                "built",
                "founded",
                "established",
                "constructed",
                "completed",
                "opened",
                "created",
                "incorporated",
                "began",
                "launched",
            ],
            Relation::Birthplace | Relation::Birthdate => &["born"],
            Relation::Director => &["directed", "director", "film"],
            Relation::Inventor => &["invented", "inventor", "discovered", "discovery"],
            Relation::Founder => &["founded", "founder", "established"],
            Relation::Ceo => &["ceo", "chief executive"],
            Relation::Currency => &["currency"],
            Relation::Language => &["language"],
        }
    }

    /// Answers to these questions contain a number or a date.
    fn numeric(self) -> bool {
        matches!(
            self,
            Relation::Population
                | Relation::Height
                | Relation::Length
                | Relation::Area
                | Relation::Inception
                | Relation::Birthdate
        )
    }
}

/// A fact question taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub relation: Option<Relation>,
    /// The thing asked about ("australia", "mount everest"). The whole
    /// subject when no relation was recognised.
    pub entity: String,
    /// "built", "founded" ... for [`Relation::Inception`].
    pub verb: String,
    /// "largest", "tallest" ... for "what is the largest ocean".
    pub superlative: Option<String>,
}

fn patterns() -> &'static Vec<(Regex, Relation)> {
    static P: OnceLock<Vec<(Regex, Relation)>> = OnceLock::new();
    P.get_or_init(|| {
        let defs: &[(&str, Relation)] = &[
            (r"^capital(?: city)? of (.+)$", Relation::Capital),
            (r"^who (?:wrote|authored) (.+)$", Relation::Author),
            (r"^author of (.+)$", Relation::Author),
            (r"^who (?:painted|drew|sculpted|created) (.+)$", Relation::Creator),
            (r"^who composed (.+)$", Relation::Composer),
            (r"^population of (.+)$", Relation::Population),
            (r"^how many people (?:live|lives|are there) in (.+)$", Relation::Population),
            (r"^how (?:tall|high) (?:is|are|was) (.+)$", Relation::Height),
            (r"^(?:height|elevation) of (.+)$", Relation::Height),
            (r"^how long (?:is|are|was) (.+)$", Relation::Length),
            (r"^length of (.+)$", Relation::Length),
            (r"^(?:area|size) of (.+)$", Relation::Area),
            (
                r"^when (?:was|were|is) (.+?) (built|founded|established|constructed|started|opened|launched|formed|created)$",
                Relation::Inception,
            ),
            (r"^where (?:was|is) (.+?) born$", Relation::Birthplace),
            (r"^when (?:was|is) (.+?) born$", Relation::Birthdate),
            (r"^who directed (.+)$", Relation::Director),
            (r"^director of (.+)$", Relation::Director),
            (r"^who (?:invented|discovered) (.+)$", Relation::Inventor),
            (r"^who (?:founded|started|established) (.+)$", Relation::Founder),
            (r"^(?:ceo|chief executive(?: officer)?) of (.+)$", Relation::Ceo),
            (r"^currency of (.+)$", Relation::Currency),
            (r"^official languages? of (.+)$", Relation::Language),
        ];
        defs.iter()
            .filter_map(|(p, r)| Regex::new(p).ok().map(|re| (re, *r)))
            .collect()
    })
}

fn superlative_pattern() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| {
        Regex::new(r"^(largest|biggest|tallest|longest|highest|smallest|deepest|oldest|fastest|heaviest) (.+?)(?: in the world| on earth| in the universe| ever)?$").unwrap()
    })
}

/// Words that all say the same superlative.
fn superlative_group(adj: &str) -> &'static [&'static str] {
    match adj {
        "largest" | "biggest" => &["largest", "biggest"],
        "tallest" | "highest" => &["tallest", "highest"],
        "longest" => &["longest"],
        "smallest" => &["smallest"],
        "deepest" => &["deepest"],
        "oldest" => &["oldest"],
        "fastest" => &["fastest"],
        _ => &["heaviest"],
    }
}

fn clean_entity(s: &str) -> String {
    let s = s.trim().trim_matches(|c: char| "?!.,\"'".contains(c));
    let s = s.trim();
    for lead in ["the ", "a ", "an "] {
        if let Some(rest) = s.strip_prefix(lead) {
            return rest.trim().to_string();
        }
    }
    s.to_string()
}

/// Take a subject phrase (as made by `parse_fact_subject`) apart.
pub fn parse_question(subject: &str) -> Question {
    let mut t = subject.trim().to_lowercase();
    // "tell me about X" arrives as "about X".
    for lead in ["about ", "the "] {
        if let Some(rest) = t.strip_prefix(lead) {
            t = rest.to_string();
        }
    }
    if let Some(c) = superlative_pattern().captures(&t) {
        let adj = c[1].to_string();
        let thing = clean_entity(&c[2]);
        return Question {
            relation: None,
            entity: format!("{adj} {thing}"),
            verb: String::new(),
            superlative: Some(adj),
        };
    }
    for (re, rel) in patterns() {
        if let Some(c) = re.captures(&t) {
            let entity = clean_entity(c.get(1).map_or("", |m| m.as_str()));
            if entity.is_empty() {
                continue;
            }
            let verb = if *rel == Relation::Inception {
                match c.get(2).map_or("", |m| m.as_str()) {
                    "started" | "formed" => "founded",
                    "constructed" => "built",
                    v => v,
                }
                .to_string()
            } else {
                String::new()
            };
            return Question {
                relation: Some(*rel),
                entity,
                verb,
                superlative: None,
            };
        }
    }
    Question {
        relation: None,
        entity: clean_entity(&t),
        verb: String::new(),
        superlative: None,
    }
}

// ---------------------------------------------------------------------------
// Wikidata
// ---------------------------------------------------------------------------

const WIKIDATA_API: &str = "https://www.wikidata.org/w/api.php";
const WIKIDATA_CREDIT: &str = "(Wikidata, CC0)";
/// How many entity candidates are looked at, and how many get their claims read.
const SEARCH_LIMIT: usize = 12;
const CLAIM_CANDIDATES: usize = 7;

struct Candidate {
    id: String,
    label: String,
}

/// Structured answers from Wikidata for relation questions.
///
/// Entity resolution: the entity name is searched with `wbsearchentities`,
/// candidates without an English Wikipedia article are dropped (a cheap
/// notability test), exact label matches go first, and the first candidate
/// that actually has the asked-for property wins. So "Hamlet" resolves to the
/// play for "who wrote", "Titanic" to the film for "who directed".
pub struct WikidataFacts;

impl FactSource for WikidataFacts {
    fn id(&self) -> &'static str {
        "wikidata"
    }

    fn fact(&self, http: &dyn Http, subject: &str) -> Result<String, HttpError> {
        let q = parse_question(subject);
        let relation = q.relation.ok_or(HttpError::NotFound)?;
        let cands: Vec<Candidate> = self
            .search(http, &q.entity)?
            .into_iter()
            .take(CLAIM_CANDIDATES)
            .collect();
        // In one parallel round: which candidates have an English article (a
        // cheap notability test), and each candidate's claims for the
        // asked-for property. If the article check fails, every candidate is
        // accepted.
        let ids: Vec<&str> = cands.iter().map(|c| c.id.as_str()).collect();
        let (linked, claim_sets) = std::thread::scope(|s| {
            let links = s.spawn(|| linked_to_enwiki(http, &ids));
            let handles: Vec<_> = cands
                .iter()
                .map(|c| s.spawn(move || claims_for(http, &c.id, relation)))
                .collect();
            let links = links.join().unwrap_or(Err(HttpError::NotFound));
            let sets: Vec<Result<Vec<Value>, HttpError>> = handles
                .into_iter()
                .map(|h| h.join().unwrap_or(Err(HttpError::NotFound)))
                .collect();
            (links.ok(), sets)
        });
        let mut first_err: Option<HttpError> = None;
        for (cand, set) in cands.iter().zip(claim_sets) {
            if linked.as_ref().is_some_and(|l| !l.contains(&cand.id)) {
                continue;
            }
            let list = match set {
                Ok(l) => l,
                Err(HttpError::NotFound) => continue,
                Err(e) => {
                    first_err.get_or_insert(e);
                    continue;
                }
            };
            let picked = pick_claims(&list, relation);
            if picked.is_empty() {
                continue;
            }
            if let Some(text) = self.describe(http, &q, cand, &picked)? {
                return Ok(format!("{text} {WIKIDATA_CREDIT}"));
            }
        }
        Err(first_err.unwrap_or(HttpError::NotFound))
    }
}

fn is_facility(label: &str) -> bool {
    let l = label.to_lowercase();
    [
        "hospital",
        "medical cent",
        "clinic",
        "infirmary",
        "maternity",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// The English name of the place an item is located in (property P131).
fn located_in(http: &dyn Http, id: &str) -> Result<Option<String>, HttpError> {
    let v = get_json(
        http,
        &format!("{WIKIDATA_API}?action=wbgetclaims&entity={id}&property=P131&format=json"),
    )?;
    let Some(town) = v
        .pointer("/claims/P131/0/mainsnak/datavalue/value/id")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let l = get_json(
        http,
        &format!(
            "{WIKIDATA_API}?action=wbgetentities&ids={town}&props=labels&languages=en&languagefallback=1&format=json"
        ),
    )?;
    Ok(str_at(&l, &format!("/entities/{town}/labels/en/value")).map(String::from))
}

/// Which of these items have an English Wikipedia article.
fn linked_to_enwiki(http: &dyn Http, ids: &[&str]) -> Result<HashSet<String>, HttpError> {
    let v = get_json(
        http,
        &format!(
            "{WIKIDATA_API}?action=wbgetentities&ids={}&props=sitelinks&sitefilter=enwiki&format=json",
            encode(&ids.join("|"))
        ),
    )?;
    Ok(ids
        .iter()
        .filter(|id| {
            v.pointer(&format!("/entities/{id}/sitelinks/enwiki"))
                .is_some()
        })
        .map(|id| id.to_string())
        .collect())
}

/// The claims of the first of the relation's properties the item has.
fn claims_for(http: &dyn Http, id: &str, relation: Relation) -> Result<Vec<Value>, HttpError> {
    for prop in relation.properties() {
        let v = get_json(
            http,
            &format!("{WIKIDATA_API}?action=wbgetclaims&entity={id}&property={prop}&format=json"),
        )?;
        if let Some(list) = v
            .pointer(&format!("/claims/{prop}"))
            .and_then(Value::as_array)
            .filter(|l| !pick_claims(l, relation).is_empty())
        {
            return Ok(list.clone());
        }
    }
    Err(HttpError::NotFound)
}

impl WikidataFacts {
    /// Candidate entities for a name: exact label matches first, then the
    /// search order (the search already favours well-known items).
    fn search(&self, http: &dyn Http, name: &str) -> Result<Vec<Candidate>, HttpError> {
        let found = get_json(
            http,
            &format!(
                "{WIKIDATA_API}?action=wbsearchentities&search={}&language=en&uselang=en\
                 &type=item&limit={SEARCH_LIMIT}&format=json",
                encode(name)
            ),
        )?;
        let mut hits: Vec<Candidate> = found
            .get("search")
            .and_then(Value::as_array)
            .ok_or(HttpError::NotFound)?
            .iter()
            .filter_map(|h| {
                Some(Candidate {
                    id: h.get("id")?.as_str()?.to_string(),
                    label: h.get("label")?.as_str()?.to_string(),
                })
            })
            .filter(|c| c.id.starts_with('Q') && c.id[1..].chars().all(|d| d.is_ascii_digit()))
            .collect();
        hits.sort_by_key(|c| !same_name(&c.label, name));
        if hits.is_empty() {
            return Err(HttpError::NotFound);
        }
        Ok(hits)
    }

    /// Turn the chosen claims into one sentence.
    fn describe(
        &self,
        http: &dyn Http,
        q: &Question,
        cand: &Candidate,
        claims: &[&Value],
    ) -> Result<Option<String>, HttpError> {
        let mut values: Vec<RawValue> = claims.iter().filter_map(|c| raw_value(c)).collect();
        if values.is_empty() {
            return Ok(None);
        }
        // Names for the items and units that were referenced.
        let mut ids: Vec<String> = Vec::new();
        for v in &values {
            match v {
                RawValue::Item(id) => ids.push(id.clone()),
                RawValue::Quantity { unit: Some(u), .. } => ids.push(u.clone()),
                _ => {}
            }
        }
        ids.sort();
        ids.dedup();
        let labels = if ids.is_empty() {
            Value::Null
        } else {
            get_json(
                http,
                &format!(
                    "{WIKIDATA_API}?action=wbgetentities&ids={}&props=labels&languages=en&languagefallback=1&format=json",
                    encode(&ids.join("|"))
                ),
            )?
        };
        let label_of = |id: &str| -> Option<String> {
            labels
                .pointer(&format!("/entities/{id}/labels/en/value"))
                .and_then(Value::as_str)
                .map(String::from)
        };
        let rel = q.relation.ok_or(HttpError::NotFound)?;
        values.truncate(3);
        let mut shown: Vec<String> = Vec::new();
        let mut as_of: Option<String> = None;
        for v in &values {
            match v {
                RawValue::Item(id) => shown.extend(label_of(id)),
                RawValue::Text(t) => shown.push(t.clone()),
                RawValue::Time { text } => shown.push(text.clone()),
                RawValue::Quantity { amount, unit, year } => {
                    let unit_text = unit.as_deref().and_then(label_of).map(|l| short_unit(&l));
                    shown.push(format_quantity(*amount, unit_text.as_deref(), rel));
                    as_of = year.clone();
                }
            }
        }
        if shown.is_empty() {
            return Ok(None);
        }
        // Born in a hospital: say which town it is in.
        if rel == Relation::Birthplace && is_facility(&shown[0]) {
            if let Some(RawValue::Item(id)) = values.first() {
                if let Ok(Some(town)) = located_in(http, id) {
                    shown[0] = format!("{}, {town}", shown[0]);
                }
            }
        }
        let name = &cand.label;
        let list = join_list(&shown);
        let used_elevation = rel == Relation::Height
            && claims
                .first()
                .and_then(|c| str_at(c, "/mainsnak/property"))
                .is_some_and(|p| p == "P2044");
        let sentence = match rel {
            Relation::Capital => format!("The capital of {name} is {list}."),
            Relation::Author => format!("{name} was written by {list}."),
            Relation::Creator => format!("{name} was created by {list}."),
            Relation::Composer => format!("{name} was composed by {list}."),
            Relation::Population => match as_of {
                Some(y) => format!("The population of {name} is about {list} ({y})."),
                None => format!("The population of {name} is about {list}."),
            },
            Relation::Height if used_elevation => format!("{name} is {list} above sea level."),
            Relation::Height => format!("{name} is {list} tall."),
            Relation::Length => format!("{name} is {list} long."),
            Relation::Area => format!("The area of {name} is {list}."),
            Relation::Inception => {
                let verb = if q.verb.is_empty() {
                    "founded"
                } else {
                    &q.verb
                };
                format!("{name} was {verb} in {list}.")
            }
            Relation::Birthplace => format!("{name} was born in {list}."),
            Relation::Birthdate => format!("{name} was born on {list}."),
            Relation::Director => format!("{name} was directed by {list}."),
            Relation::Inventor => format!("{name} was invented or discovered by {list}."),
            Relation::Founder => format!("{name} was founded by {list}."),
            Relation::Ceo => format!("The CEO of {name} is {list}."),
            Relation::Currency => format!("The currency of {name} is the {list}."),
            Relation::Language => format!("The official language of {name} is {list}."),
        };
        Ok(Some(sentence))
    }
}

enum RawValue {
    Item(String),
    Text(String),
    Time {
        text: String,
    },
    Quantity {
        amount: f64,
        unit: Option<String>,
        /// Year of the "point in time" qualifier, if the claim has one.
        year: Option<String>,
    },
}

fn raw_value(claim: &Value) -> Option<RawValue> {
    let snak = claim.get("mainsnak")?;
    if snak.get("snaktype")?.as_str()? != "value" {
        return None;
    }
    let dv = snak.get("datavalue")?;
    let value = dv.get("value")?;
    match dv.get("type")?.as_str()? {
        "wikibase-entityid" => Some(RawValue::Item(value.get("id")?.as_str()?.to_string())),
        "string" => Some(RawValue::Text(value.as_str()?.to_string())),
        "monolingualtext" => Some(RawValue::Text(value.get("text")?.as_str()?.to_string())),
        "time" => {
            let text = format_time(
                value.get("time")?.as_str()?,
                value.get("precision").and_then(Value::as_i64).unwrap_or(9),
            )?;
            Some(RawValue::Time { text })
        }
        "quantity" => {
            let amount = value.get("amount")?.as_str()?.parse::<f64>().ok()?;
            let unit = value
                .get("unit")
                .and_then(Value::as_str)
                .and_then(|u| u.rsplit_once("/entity/"))
                .map(|(_, id)| id.to_string());
            let year = claim_time(claim, "P585").and_then(|t| {
                let digits = t.trim_start_matches(['+', '-']);
                digits
                    .split('-')
                    .next()
                    .map(|y| y.trim_start_matches('0').to_string())
            });
            Some(RawValue::Quantity { amount, unit, year })
        }
        _ => None,
    }
}

/// The time string of a time qualifier, such as "+2023-01-01T00:00:00Z".
fn claim_time(claim: &Value, prop: &str) -> Option<String> {
    claim
        .pointer(&format!("/qualifiers/{prop}/0/datavalue/value/time"))
        .and_then(Value::as_str)
        .map(String::from)
}

/// The claims that should be shown: not deprecated, preferred rank first,
/// ended ones (end time qualifier) dropped when others remain, and for
/// dated values (population) only the latest.
fn pick_claims(all: &[Value], relation: Relation) -> Vec<&Value> {
    let mut v: Vec<&Value> = all
        .iter()
        .filter(|c| str_at(c, "/rank") != Some("deprecated"))
        .collect();
    if v.iter().any(|c| str_at(c, "/rank") == Some("preferred")) {
        v.retain(|c| str_at(c, "/rank") == Some("preferred"));
    }
    if v.iter().any(|c| c.pointer("/qualifiers/P582").is_none()) {
        v.retain(|c| c.pointer("/qualifiers/P582").is_none());
    }
    if v.iter().any(|c| claim_time(c, "P585").is_some()) {
        // Latest point in time first (the strings sort the same way as dates).
        v.sort_by_key(|c| std::cmp::Reverse(claim_time(c, "P585").unwrap_or_default()));
        v.truncate(1);
    }
    if relation.numeric() {
        v.truncate(1);
    }
    v
}

fn join_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// "+1889-03-31T00:00:00Z" at precision 11 -> "31 March 1889".
fn format_time(time: &str, precision: i64) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let neg = time.starts_with('-');
    let body = time.trim_start_matches(['+', '-']);
    let (date, _) = body.split_once('T')?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: usize = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let y = if neg {
        format!("{year} BC")
    } else {
        year.to_string()
    };
    let month_name = (1..=12).contains(&month).then(|| MONTHS[month - 1]);
    Some(match (precision, month_name) {
        (p, Some(m)) if p >= 11 && day >= 1 => format!("{day} {m} {y}"),
        (10, Some(m)) => format!("{m} {y}"),
        _ => y,
    })
}

/// 8848.86 -> "8,848.86"; 12345678.0 -> "12,345,678".
fn group_digits(x: f64) -> String {
    let rounded = if x.abs() >= 10_000.0 {
        format!("{:.0}", x)
    } else {
        let s = format!("{:.2}", x);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    let (sign, rest) = match rounded.strip_prefix('-') {
        Some(r) => ("-", r.to_string()),
        None => ("", rounded),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((i, f)) => (i.to_string(), format!(".{f}")),
        None => (rest, String::new()),
    };
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}{frac}")
}

fn short_unit(label: &str) -> String {
    match label {
        "metre" | "meter" => "m".into(),
        "kilometre" | "kilometer" => "km".into(),
        "centimetre" | "centimeter" => "cm".into(),
        "square kilometre" | "square kilometer" => "km\u{b2}".into(),
        "square metre" | "square meter" => "m\u{b2}".into(),
        "mile" => "mi".into(),
        "foot" => "ft".into(),
        other => other.to_string(),
    }
}

fn format_quantity(amount: f64, unit: Option<&str>, relation: Relation) -> String {
    let number = if relation == Relation::Population {
        if amount.abs() >= 1e9 {
            format!("{:.2} billion", amount / 1e9)
        } else if amount.abs() >= 1e6 {
            format!("{:.1} million", amount / 1e6)
        } else {
            group_digits(amount)
        }
    } else {
        group_digits(amount)
    };
    match unit {
        Some(u) if u.chars().count() <= 3 || u.contains('\u{b2}') => format!("{number} {u}"),
        Some(u) => format!("{number} {u}"),
        None => number,
    }
}

// ---------------------------------------------------------------------------
// Smart Wikipedia
// ---------------------------------------------------------------------------

const WIKI_API: &str = "https://en.wikipedia.org/w/api.php";

struct TitleHit {
    title: String,
    matched: Option<String>,
}

struct Article {
    title: String,
    url: String,
    extract: String,
    index: i64,
}

/// English Wikipedia with a rewritten query.
///
/// 1. The subject, then the entity cut out of it, are looked up as page
///    titles through the REST title search (it follows redirects such as
///    "Capital of Australia" -> Canberra and ignores case).
/// 2. Failing that, a full-text search whose hits are re-ranked by how well
///    the title overlaps the entity (so the article about the thing beats the
///    article that merely mentions it, and "List of ..." pages lose).
/// 3. For attribute questions ("how tall", "when was ... built") the sentence
///    of the intro that carries the answer is shown, not just the first two.
pub struct WikipediaSmartFacts;

impl FactSource for WikipediaSmartFacts {
    fn id(&self) -> &'static str {
        "wikipedia_smart"
    }

    fn fact(&self, http: &dyn Http, subject: &str) -> Result<String, HttpError> {
        let q = parse_question(subject);
        let subject_clean = clean_entity(&subject.trim().to_lowercase());
        let mut tried: HashSet<String> = HashSet::new();
        let mut last_err = HttpError::NotFound;

        // Stage 1: title lookups, strict for the whole subject, looser for the entity.
        let mut stages: Vec<(String, bool)> = Vec::new();
        if !subject_clean.is_empty() {
            stages.push((subject_clean.clone(), true));
        }
        if q.relation.is_some() && !q.entity.is_empty() {
            stages.push((q.entity.clone(), false));
        }
        for (phrase, strict) in stages {
            let hits = match title_search(http, &phrase) {
                Ok(h) => h,
                Err(e) => {
                    last_err = e;
                    continue;
                }
            };
            let Some(title) = best_title(&hits, &phrase, strict) else {
                continue;
            };
            if !tried.insert(title.clone()) {
                continue;
            }
            match fetch_article(http, &title) {
                Ok(a) => {
                    if let Some(text) = render(&a, &q) {
                        return Ok(text);
                    }
                }
                Err(e) => last_err = e,
            }
        }

        // Stage 2: full-text search, re-ranked.
        let query = if q.entity.is_empty() {
            subject_clean.clone()
        } else {
            q.entity.clone()
        };
        let mut articles = match search_articles(http, &query) {
            Ok(a) => a,
            Err(e) => {
                return Err(if last_err == HttpError::NotFound {
                    e
                } else {
                    last_err
                })
            }
        };
        let focus = content_words(&query);
        articles.sort_by(|a, b| {
            rank_score(b, &focus, q.superlative.as_deref())
                .partial_cmp(&rank_score(a, &focus, q.superlative.as_deref()))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.index.cmp(&b.index))
        });
        for a in &articles {
            if let Some(text) = render(a, &q) {
                return Ok(text);
            }
        }
        Err(HttpError::NotFound)
    }
}

fn title_search(http: &dyn Http, phrase: &str) -> Result<Vec<TitleHit>, HttpError> {
    let v = get_json(
        http,
        &format!(
            "https://en.wikipedia.org/w/rest.php/v1/search/title?q={}&limit=6",
            encode(phrase)
        ),
    )?;
    Ok(v.get("pages")
        .and_then(Value::as_array)
        .map(|pages| {
            pages
                .iter()
                .filter_map(|p| {
                    Some(TitleHit {
                        title: p.get("title")?.as_str()?.to_string(),
                        matched: p
                            .get("matched_title")
                            .and_then(Value::as_str)
                            .map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The title to read for a phrase. Strict: the page title or the redirect
/// that matched must be the phrase itself. Loose: also a title that contains
/// every word of the phrase (the shortest such).
fn best_title(hits: &[TitleHit], phrase: &str, strict: bool) -> Option<String> {
    if let Some(h) = hits.iter().find(|h| {
        same_name(&h.title, phrase) || h.matched.as_deref().is_some_and(|m| same_name(m, phrase))
    }) {
        return Some(h.title.clone());
    }
    if strict {
        return None;
    }
    let need = content_words(phrase);
    if need.is_empty() {
        return None;
    }
    hits.iter()
        .filter(|h| {
            let have = words(&h.title);
            need.iter().all(|w| have.contains(w)) && !h.title.to_lowercase().starts_with("list of")
        })
        .min_by_key(|h| h.title.chars().count())
        .map(|h| h.title.clone())
}

fn parse_articles(v: &Value) -> Vec<Article> {
    let Some(pages) = v.pointer("/query/pages").and_then(Value::as_array) else {
        return Vec::new();
    };
    pages
        .iter()
        .filter_map(|p| {
            let title = p.get("title")?.as_str()?.to_string();
            let extract = p.get("extract")?.as_str()?.trim().to_string();
            let url = p
                .get("fullurl")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| {
                    format!(
                        "https://en.wikipedia.org/wiki/{}",
                        encode(&title.replace(' ', "_"))
                    )
                });
            Some(Article {
                title,
                url,
                extract,
                index: p.get("index").and_then(Value::as_i64).unwrap_or(i64::MAX),
            })
        })
        .collect()
}

fn fetch_article(http: &dyn Http, title: &str) -> Result<Article, HttpError> {
    let v = get_json(
        http,
        &format!(
            "{WIKI_API}?action=query&titles={}&redirects=1&prop=extracts%7Cinfo&exintro=1\
             &explaintext=1&exchars=2500&inprop=url&format=json&formatversion=2",
            encode(title)
        ),
    )?;
    parse_articles(&v)
        .into_iter()
        .next()
        .ok_or(HttpError::NotFound)
}

fn search_articles(http: &dyn Http, query: &str) -> Result<Vec<Article>, HttpError> {
    let v = get_json(
        http,
        &format!(
            "{WIKI_API}?action=query&generator=search&gsrsearch={}&gsrlimit=6\
             &prop=extracts%7Cinfo&exintro=1&explaintext=1&exchars=2500&exlimit=max\
             &inprop=url&format=json&formatversion=2",
            encode(query)
        ),
    )?;
    let a = parse_articles(&v);
    if a.is_empty() {
        Err(HttpError::NotFound)
    } else {
        Ok(a)
    }
}

/// Higher is better: title words that are the focus words, few extra words,
/// no list pages. The search rank breaks ties.
fn rank_score(a: &Article, focus: &[String], superlative: Option<&str>) -> f64 {
    let title_words = content_words(&a.title);
    let overlap = title_words.iter().filter(|w| focus.contains(w)).count() as f64;
    let extra = title_words.len() as f64 - overlap;
    let all_focus_found = focus.iter().all(|w| title_words.contains(w));
    let list = a.title.to_lowercase().starts_with("list of");
    // "the largest ocean": the article whose lead says so.
    let says_it = superlative.is_some_and(|s| {
        let lead = a.extract.to_lowercase();
        let lead: String = lead.chars().take(300).collect();
        superlative_group(s).iter().any(|w| lead.contains(w))
    });
    let bonus = if says_it { 4.0 } else { 0.0 };
    overlap * 2.0 - extra * 0.75 + if all_focus_found { 2.0 } else { 0.0 }
        - if list { 4.0 } else { 0.0 }
        + bonus
}

/// The sentences of a text, abbreviations respected.
fn sentences(text: &str) -> Vec<String> {
    let text = text.trim();
    let total = text.chars().count();
    let mut out = Vec::new();
    let mut done = 0;
    for k in 1..=40 {
        let upto = first_sentences(text, k, usize::MAX);
        let n = upto.chars().count();
        let piece: String = upto.chars().skip(done).collect();
        let piece = piece.trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        done = n;
        if n >= total {
            break;
        }
    }
    out
}

/// The answer text for an article, or `None` for an empty or disambiguation page.
fn render(a: &Article, q: &Question) -> Option<String> {
    let lower = a.extract.to_lowercase();
    if a.extract.is_empty() || lower.contains("may refer to") || lower.contains("can refer to") {
        return None;
    }
    let all = sentences(&strip_parentheses(&a.extract));
    let first = all.first()?.clone();
    let body = match q.relation {
        Some(rel) => {
            let score = |s: &str| -> usize {
                let l = s.to_lowercase();
                let hits = rel.keywords().iter().filter(|k| l.contains(**k)).count();
                if hits == 0 {
                    0
                } else {
                    hits * 2 + usize::from(rel.numeric() && l.chars().any(|c| c.is_ascii_digit()))
                }
            };
            let best = all
                .iter()
                .enumerate()
                .map(|(i, s)| (score(s), std::cmp::Reverse(i)))
                .max();
            match best {
                Some((sc, std::cmp::Reverse(i))) if sc >= 2 && i > 0 => {
                    first_sentences(&format!("{first} {}", all[i]), 2, 420)
                }
                Some((sc, _)) if sc >= 2 => {
                    first_sentences(&all[..all.len().min(2)].join(" "), 2, 320)
                }
                _ => first_sentences(&all[..all.len().min(2)].join(" "), 2, 320),
            }
        }
        None => {
            let lead = first_sentences(&all[..all.len().min(2)].join(" "), 2, 320);
            // "speed of light", "boiling point of water": the intro's number.
            let measure = [
                "speed",
                "boiling point",
                "melting point",
                "density",
                "mass",
                "distance",
            ]
            .iter()
            .any(|m| q.entity.starts_with(m));
            let has_digit = |s: &str| s.chars().any(|c| c.is_ascii_digit());
            let focus = content_words(&q.entity);
            let number_sentence = all.iter().take(8).skip(2).find(|s| {
                let l = s.to_lowercase();
                has_digit(s) && focus.iter().any(|w| l.contains(w.as_str()))
            });
            match number_sentence {
                Some(s) if measure && !has_digit(&lead) => {
                    first_sentences(&format!("{first} {s}"), 2, 420)
                }
                _ => lead,
            }
        }
    };
    Some(format!("{body} (Wikipedia, CC BY-SA 4.0: {})", a.url))
}

// ---------------------------------------------------------------------------
// The combination
// ---------------------------------------------------------------------------

/// Wikidata for relation questions, else smart Wikipedia. The most accurate
/// combination in the live benchmark; a single source id for the chain.
pub struct BestFacts;

impl FactSource for BestFacts {
    fn id(&self) -> &'static str {
        "facts_best"
    }

    fn fact(&self, http: &dyn Http, subject: &str) -> Result<String, HttpError> {
        match WikidataFacts.fact(http, subject) {
            Ok(a) => Ok(a),
            Err(first) => WikipediaSmartFacts.fact(http, subject).map_err(|second| {
                // Offline only when both said so; otherwise the more informative one.
                if first == HttpError::Offline && second == HttpError::Offline {
                    HttpError::Offline
                } else if second == HttpError::NotFound {
                    first
                } else {
                    second
                }
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (none touch the network, except the ignored benchmark)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_lookup::{parse_fact_subject, UreqHttp, WikipediaFacts, WikipediaRestFacts};
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

    fn fake(routes: Vec<(&'static str, Result<String, HttpError>)>) -> FakeHttp {
        FakeHttp {
            routes,
            seen: Default::default(),
        }
    }

    fn ok(s: &str) -> Result<String, HttpError> {
        Ok(s.to_string())
    }

    #[test]
    fn questions_are_taken_apart() {
        let p = |t: &str| parse_question(&parse_fact_subject(t).unwrap());
        let q = p("What is the capital of Australia?");
        assert_eq!(
            (q.relation, q.entity.as_str()),
            (Some(Relation::Capital), "australia")
        );
        let q = p("who wrote Hamlet");
        assert_eq!(
            (q.relation, q.entity.as_str()),
            (Some(Relation::Author), "hamlet")
        );
        let q = p("how tall is Mount Everest");
        assert_eq!(
            (q.relation, q.entity.as_str()),
            (Some(Relation::Height), "mount everest")
        );
        let q = p("when was the Eiffel Tower built");
        assert_eq!(q.relation, Some(Relation::Inception));
        assert_eq!(
            (q.entity.as_str(), q.verb.as_str()),
            ("eiffel tower", "built")
        );
        let q = p("population of Japan");
        assert_eq!(
            (q.relation, q.entity.as_str()),
            (Some(Relation::Population), "japan")
        );
        let q = p("where was Albert Einstein born");
        assert_eq!(
            (q.relation, q.entity.as_str()),
            (Some(Relation::Birthplace), "albert einstein")
        );
        let q = p("tell me about the Roman Empire");
        assert_eq!((q.relation, q.entity.as_str()), (None, "roman empire"));
        let q = p("who is Marie Curie");
        assert_eq!((q.relation, q.entity.as_str()), (None, "marie curie"));
        let q = p("who is the CEO of Tesla");
        assert_eq!(
            (q.relation, q.entity.as_str()),
            (Some(Relation::Ceo), "tesla")
        );
        let q = p("how many people live in Tokyo");
        assert_eq!(
            (q.relation, q.entity.as_str()),
            (Some(Relation::Population), "tokyo")
        );
    }

    #[test]
    fn formats_times_and_numbers() {
        assert_eq!(
            format_time("+1889-03-31T00:00:00Z", 11).unwrap(),
            "31 March 1889"
        );
        assert_eq!(
            format_time("+1889-03-00T00:00:00Z", 10).unwrap(),
            "March 1889"
        );
        assert_eq!(format_time("+1889-00-00T00:00:00Z", 9).unwrap(), "1889");
        assert_eq!(format_time("-0500-01-01T00:00:00Z", 9).unwrap(), "500 BC");
        assert_eq!(format_time("garbage", 9), None);
        assert_eq!(group_digits(8848.86), "8,848.86");
        assert_eq!(group_digits(828.0), "828");
        assert_eq!(group_digits(7_692_024.0), "7,692,024");
        assert_eq!(
            format_quantity(125_116_000.0, None, Relation::Population),
            "125.1 million"
        );
        assert_eq!(
            format_quantity(1_428_627_663.0, None, Relation::Population),
            "1.43 billion"
        );
        assert_eq!(format_quantity(330.0, Some("m"), Relation::Height), "330 m");
        assert_eq!(
            join_list(&["A".into(), "B".into(), "C".into()]),
            "A, B and C"
        );
    }

    fn claim(rank: &str, extra: &str) -> Value {
        serde_json::from_str(&format!(
            r#"{{"rank":"{rank}","mainsnak":{{"snaktype":"value","datavalue":{{"type":"wikibase-entityid","value":{{"id":"Q1"}}}}}}{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn picks_the_current_claims() {
        let ended = claim("normal", r#","qualifiers":{"P582":[{}]}"#);
        let now = claim("normal", "");
        let bad = claim("deprecated", "");
        let all = [ended.clone(), now.clone(), bad];
        let picked = pick_claims(&all, Relation::Capital);
        assert_eq!(picked.len(), 1);
        assert!(picked[0].pointer("/qualifiers/P582").is_none());
        // Preferred beats normal.
        let pref = claim("preferred", r#","qualifiers":{"P1":[]}"#);
        let all = [now, pref];
        assert_eq!(pick_claims(&all, Relation::Capital).len(), 1);
        // Dated values: only the latest.
        let old = claim(
            "normal",
            r#","qualifiers":{"P585":[{"datavalue":{"value":{"time":"+2010-01-01T00:00:00Z"}}}]}"#,
        );
        let new = claim(
            "normal",
            r#","qualifiers":{"P585":[{"datavalue":{"value":{"time":"+2020-01-01T00:00:00Z"}}}]}"#,
        );
        let all = [old, new];
        let picked = pick_claims(&all, Relation::Population);
        assert_eq!(picked.len(), 1);
        assert!(claim_time(picked[0], "P585").unwrap().contains("2020"));
    }

    fn wikidata_routes() -> Vec<(&'static str, Result<String, HttpError>)> {
        vec![
            (
                "action=wbsearchentities",
                ok(r#"{"search":[
                    {"id":"Q999","label":"Australia (film)"},
                    {"id":"Q3960","label":"Australia"},
                    {"id":"Q408","label":"Australia"}]}"#),
            ),
            (
                "props=sitelinks",
                // Q3960 has no English article, Q999 does.
                ok(
                    r#"{"entities":{"Q999":{"sitelinks":{"enwiki":{}}},"Q3960":{"sitelinks":{}},"Q408":{"sitelinks":{"enwiki":{}}}}}"#,
                ),
            ),
            ("entity=Q999&property=P36", ok(r#"{"claims":{}}"#)),
            (
                "entity=Q408&property=P36",
                ok(
                    r#"{"claims":{"P36":[{"rank":"normal","mainsnak":{"snaktype":"value","datavalue":{"type":"wikibase-entityid","value":{"id":"Q3114"}}}}]}}"#,
                ),
            ),
            (
                "props=labels",
                ok(r#"{"entities":{"Q3114":{"labels":{"en":{"value":"Canberra"}}}}}"#),
            ),
        ]
    }

    #[test]
    fn wikidata_answers_capital_questions() {
        let http = fake(wikidata_routes());
        let out = WikidataFacts.fact(&http, "capital of australia").unwrap();
        assert_eq!(out, "The capital of Australia is Canberra. (Wikidata, CC0)");
        // Only the entity name went out; the candidate without an English
        // article was ignored even though its claims were fetched.
        let seen = http.seen.lock().unwrap();
        assert!(seen[0].contains("search=australia&"), "{}", seen[0]);
        assert!(seen.iter().all(|u| !u.contains("capital%20of")));
    }

    #[test]
    fn wikidata_ignores_other_questions_and_bad_replies() {
        let http = fake(vec![]);
        assert_eq!(
            WikidataFacts.fact(&http, "marie curie"),
            Err(HttpError::NotFound)
        );
        assert!(http.seen.lock().unwrap().is_empty());
        for body in [
            "",
            "not json",
            "{}",
            "[]",
            "null",
            r#"{"search":[{}]}"#,
            r#"{"search":[]}"#,
        ] {
            let http = fake(vec![("", ok(body))]);
            assert_eq!(
                WikidataFacts.fact(&http, "capital of peru"),
                Err(HttpError::NotFound),
                "{body}"
            );
        }
        let http = fake(vec![("", Err(HttpError::Offline))]);
        assert_eq!(
            WikidataFacts.fact(&http, "capital of peru"),
            Err(HttpError::Offline)
        );
    }

    #[test]
    fn wikidata_height_uses_the_unit_name() {
        let http = fake(vec![
            (
                "action=wbsearchentities",
                ok(r#"{"search":[{"id":"Q513","label":"Mount Everest"}]}"#),
            ),
            (
                "props=sitelinks",
                ok(r#"{"entities":{"Q513":{"sitelinks":{"enwiki":{}}}}}"#),
            ),
            ("property=P2048", ok(r#"{"claims":{}}"#)),
            (
                "property=P2044",
                ok(
                    r#"{"claims":{"P2044":[{"rank":"normal","mainsnak":{"snaktype":"value","property":"P2044","datavalue":{"type":"quantity","value":{"amount":"+8848.86","unit":"http://www.wikidata.org/entity/Q11573"}}}}]}}"#,
                ),
            ),
            (
                "props=labels",
                ok(r#"{"entities":{"Q11573":{"labels":{"en":{"value":"metre"}}}}}"#),
            ),
        ]);
        assert_eq!(
            WikidataFacts
                .fact(&http, "how tall is mount everest")
                .unwrap(),
            "Mount Everest is 8,848.86 m above sea level. (Wikidata, CC0)"
        );
    }

    #[test]
    fn wikidata_dates() {
        let http = fake(vec![
            (
                "action=wbsearchentities",
                ok(r#"{"search":[{"id":"Q243","label":"Eiffel Tower"}]}"#),
            ),
            (
                "props=sitelinks",
                ok(r#"{"entities":{"Q243":{"sitelinks":{"enwiki":{}}}}}"#),
            ),
            (
                "property=P571",
                ok(
                    r#"{"claims":{"P571":[{"rank":"normal","mainsnak":{"snaktype":"value","datavalue":{"type":"time","value":{"time":"+1889-03-31T00:00:00Z","precision":11}}}}]}}"#,
                ),
            ),
        ]);
        assert_eq!(
            WikidataFacts
                .fact(&http, "when was the eiffel tower built")
                .unwrap(),
            "Eiffel Tower was built in 31 March 1889. (Wikidata, CC0)"
        );
    }

    const TITLE_HIT: &str = r#"{"pages":[{"id":51983,"key":"Canberra","title":"Canberra","matched_title":"Capital of australia"},{"id":55866,"key":"Vienna","title":"Vienna","matched_title":"Capital of Austria"}]}"#;
    const CANBERRA: &str = r#"{"query":{"pages":[{"title":"Canberra","index":1,"fullurl":"https://en.wikipedia.org/wiki/Canberra","extract":"Canberra (KAN-brə) is the capital city of Australia. With an estimated population of 484,630 as of 2025, Canberra is the largest inland city. It was founded in 1913."}]}}"#;

    #[test]
    fn smart_wikipedia_follows_the_redirect_title() {
        let http = fake(vec![
            ("rest.php/v1/search/title", ok(TITLE_HIT)),
            ("titles=Canberra", ok(CANBERRA)),
        ]);
        assert_eq!(
            WikipediaSmartFacts.fact(&http, "capital of australia").unwrap(),
            "Canberra is the capital city of Australia. With an estimated population of 484,630 as of 2025, Canberra is the largest inland city. (Wikipedia, CC BY-SA 4.0: https://en.wikipedia.org/wiki/Canberra)"
        );
    }

    #[test]
    fn smart_wikipedia_picks_the_sentence_that_answers() {
        let tower = r#"{"query":{"pages":[{"title":"Eiffel Tower","index":1,"fullurl":"https://en.wikipedia.org/wiki/Eiffel_Tower","extract":"The Eiffel Tower is a wrought-iron lattice tower in Paris. It is named after the engineer Gustave Eiffel. Locally nicknamed the Iron Lady. It was built from 1887 to 1889 as the entrance to the World's Fair."}]}}"#;
        let http = fake(vec![
            (
                "rest.php/v1/search/title",
                ok(r#"{"pages":[{"title":"Eiffel Tower","matched_title":null}]}"#),
            ),
            ("titles=Eiffel%20Tower", ok(tower)),
        ]);
        let out = WikipediaSmartFacts
            .fact(&http, "when was the eiffel tower built")
            .unwrap();
        assert!(out.starts_with("The Eiffel Tower is a wrought-iron lattice tower in Paris. It was built from 1887 to 1889"), "{out}");
    }

    #[test]
    fn smart_wikipedia_reranks_full_text_hits() {
        // No title match; the full-text search ranks the territory first.
        let found = r#"{"query":{"pages":[
            {"title":"Australian Capital Territory","index":1,"extract":"The Australian Capital Territory is a federal territory."},
            {"title":"List of Australian capital cities","index":2,"extract":"A list."},
            {"title":"Boiling point","index":3,"extract":"The boiling point of a substance is the temperature at which it boils."}]}}"#;
        let http = fake(vec![
            ("rest.php/v1/search/title", ok(r#"{"pages":[]}"#)),
            ("generator=search", ok(found)),
        ]);
        let out = WikipediaSmartFacts
            .fact(&http, "boiling point of water")
            .unwrap();
        assert!(out.starts_with("The boiling point of a substance"), "{out}");
    }

    #[test]
    fn smart_wikipedia_skips_disambiguation_and_survives_junk() {
        let dis = r#"{"query":{"pages":[{"title":"Mercury","index":1,"extract":"Mercury may refer to:"}]}}"#;
        let http = fake(vec![
            (
                "rest.php/v1/search/title",
                ok(r#"{"pages":[{"title":"Mercury"}]}"#),
            ),
            ("titles=Mercury", ok(dis)),
            ("generator=search", ok(dis)),
        ]);
        assert_eq!(
            WikipediaSmartFacts.fact(&http, "mercury"),
            Err(HttpError::NotFound)
        );
        for body in [
            "",
            "not json",
            "{}",
            "[]",
            "null",
            r#"{"pages":[{}]}"#,
            r#"{"query":{"pages":[{}]}}"#,
        ] {
            let http = fake(vec![("", ok(body))]);
            assert_eq!(
                WikipediaSmartFacts.fact(&http, "capital of peru"),
                Err(HttpError::NotFound),
                "{body}"
            );
        }
        let http = fake(vec![("", Err(HttpError::Offline))]);
        assert_eq!(
            WikipediaSmartFacts.fact(&http, "capital of peru"),
            Err(HttpError::Offline)
        );
    }

    #[test]
    fn best_facts_falls_through_to_wikipedia() {
        // Wikidata has nothing; smart Wikipedia answers.
        let routes = vec![
            ("action=wbsearchentities", ok(r#"{"search":[]}"#)),
            ("rest.php/v1/search/title", ok(TITLE_HIT)),
            ("titles=Canberra", ok(CANBERRA)),
        ];
        let http = fake(routes);
        assert!(BestFacts
            .fact(&http, "capital of australia")
            .unwrap()
            .starts_with("Canberra is the capital"));
        // Wikidata answers: Wikipedia is never asked.
        let http = fake(wikidata_routes());
        assert_eq!(
            BestFacts.fact(&http, "capital of australia").unwrap(),
            "The capital of Australia is Canberra. (Wikidata, CC0)"
        );
        assert!(http
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|u| !u.contains("wikipedia.org")));
        // Everything offline.
        let http = fake(vec![("", Err(HttpError::Offline))]);
        assert_eq!(
            BestFacts.fact(&http, "capital of peru"),
            Err(HttpError::Offline)
        );
        assert_eq!(BestFacts.id(), "facts_best");
        assert_eq!(WikidataFacts.id(), "wikidata");
        assert_eq!(WikipediaSmartFacts.id(), "wikipedia_smart");
    }

    // -----------------------------------------------------------------------
    // Live benchmark
    // -----------------------------------------------------------------------

    /// DuckDuckGo Instant Answer API, measured only. Its terms allow
    /// non-commercial use unless approved by email, so it is not shipped.
    struct DdgInstant;
    impl FactSource for DdgInstant {
        fn id(&self) -> &'static str {
            "ddg_instant"
        }
        fn fact(&self, http: &dyn Http, subject: &str) -> Result<String, HttpError> {
            let v = get_json(
                http,
                &format!(
                    "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
                    encode(subject)
                ),
            )?;
            for key in ["Answer", "AbstractText", "Definition"] {
                if let Some(t) = v.get(key).and_then(Value::as_str).filter(|t| !t.is_empty()) {
                    return Ok(t.to_string());
                }
            }
            Err(HttpError::NotFound)
        }
    }

    /// Lowercase, no thousands commas, plain spaces.
    fn norm(s: &str) -> String {
        s.to_lowercase().replace([',', '\u{a0}', '\u{202f}'], "")
    }

    #[derive(Clone)]
    struct Run {
        result: Result<String, HttpError>,
        ms: u128,
    }

    #[test]
    #[ignore = "uses the network: cargo test -p jitmo-connectors live_benchmark -- --ignored --nocapture"]
    fn live_benchmark() {
        let bench: Vec<Value> =
            serde_json::from_str(include_str!("../testdata/fact_benchmark.json"))
                .expect("benchmark json");
        let http = UreqHttp::new();
        let sources: Vec<(&str, Box<dyn FactSource>)> = vec![
            ("wikipedia_current", Box::new(WikipediaFacts)),
            ("wikipedia_rest", Box::new(WikipediaRestFacts)),
            ("wikipedia_smart", Box::new(WikipediaSmartFacts)),
            ("wikidata", Box::new(WikidataFacts)),
            ("ddg_instant", Box::new(DdgInstant)),
        ];
        // runs[source][question]
        let mut runs: Vec<Vec<Run>> = vec![Vec::new(); sources.len()];
        let mut subjects = Vec::new();
        let mut expects: Vec<Vec<String>> = Vec::new();
        for item in &bench {
            let q = item["q"].as_str().unwrap();
            let subject = parse_fact_subject(q).unwrap_or_else(|| q.to_string());
            let any: Vec<String> = item["any"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| norm(k.as_str().unwrap()))
                .collect();
            for (i, (_, s)) in sources.iter().enumerate() {
                let t = std::time::Instant::now();
                let result = s.fact(&http, &subject);
                runs[i].push(Run {
                    result,
                    ms: t.elapsed().as_millis(),
                });
            }
            subjects.push((q.to_string(), subject));
            expects.push(any);
        }

        let correct = |r: &Run, q: usize| -> Option<bool> {
            r.result
                .as_ref()
                .ok()
                .map(|a| expects[q].iter().any(|k| norm(a).contains(k.as_str())))
        };
        let idx = |name: &str| sources.iter().position(|(n, _)| *n == name).unwrap();

        // Chains: first answer wins, time is the sum of the sources tried.
        let chains: Vec<(&str, Vec<&str>)> = vec![
            ("wikidata > smart", vec!["wikidata", "wikipedia_smart"]),
            (
                "wikidata > smart > current",
                vec!["wikidata", "wikipedia_smart", "wikipedia_current"],
            ),
            (
                "wikidata > smart > current > rest",
                vec![
                    "wikidata",
                    "wikipedia_smart",
                    "wikipedia_current",
                    "wikipedia_rest",
                ],
            ),
            ("smart > wikidata", vec!["wikipedia_smart", "wikidata"]),
            (
                "wikidata > current (old Wikipedia)",
                vec!["wikidata", "wikipedia_current"],
            ),
            (
                "ddg > wikidata > smart (not shippable)",
                vec!["ddg_instant", "wikidata", "wikipedia_smart"],
            ),
        ];
        let mut rows: Vec<(String, Vec<Run>)> = sources
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.to_string(), runs[i].clone()))
            .collect();
        for (name, order) in &chains {
            let mut combined = Vec::new();
            #[allow(clippy::needless_range_loop)]
            for q in 0..bench.len() {
                let mut ms = 0;
                let mut last: Option<Result<String, HttpError>> = None;
                for s in order {
                    let r = &runs[idx(s)][q];
                    ms += r.ms;
                    if r.result.is_ok() {
                        last = Some(r.result.clone());
                        break;
                    }
                    last = Some(r.result.clone());
                }
                combined.push(Run {
                    result: last.unwrap_or(Err(HttpError::NotFound)),
                    ms,
                });
            }
            rows.push((name.to_string(), combined));
        }

        println!("\n{} questions", bench.len());
        println!(
            "{:<42} {:>8} {:>7} {:>7} {:>10} {:>8}",
            "approach", "correct", "wrong", "none", "median ms", "p90 ms"
        );
        for (name, rs) in &rows {
            let (mut good, mut bad, mut none) = (0, 0, 0);
            for (q, r) in rs.iter().enumerate() {
                match correct(r, q) {
                    Some(true) => good += 1,
                    Some(false) => bad += 1,
                    None => none += 1,
                }
            }
            let mut ms: Vec<u128> = rs.iter().map(|r| r.ms).collect();
            ms.sort_unstable();
            println!(
                "{:<42} {:>3}/{:<4} {:>7} {:>7} {:>10} {:>8}",
                name,
                good,
                rs.len(),
                bad,
                none,
                ms[ms.len() / 2],
                ms[ms.len() * 9 / 10]
            );
        }

        println!(
            "\nfailures of the first chain row, and of each single source on relation questions"
        );
        for (name, rs) in rows.iter().filter(|(n, _)| {
            n == "wikidata > smart"
                || n == "wikipedia_smart"
                || n == "wikidata"
                || n == "wikipedia_current"
        }) {
            println!("--- {name}");
            for (q, r) in rs.iter().enumerate() {
                if correct(r, q) != Some(true) {
                    let a = match &r.result {
                        Ok(a) => a.chars().take(160).collect::<String>(),
                        Err(e) => format!("{e:?}"),
                    };
                    println!("  [{}] {} -> {a}", subjects[q].0, expects[q].join("|"));
                }
            }
        }
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;
    use std::sync::Mutex;

    struct Routes(Vec<(&'static str, String)>, Mutex<Vec<String>>);
    impl Http for Routes {
        fn get(&self, url: &str) -> Result<String, HttpError> {
            self.1.lock().unwrap().push(url.to_string());
            self.0
                .iter()
                .find(|(n, _)| url.contains(n))
                .map(|(_, r)| r.clone())
                .ok_or(HttpError::Other("no route".into()))
        }
    }

    #[test]
    fn a_hospital_birthplace_names_its_town() {
        let item = |id: &str| {
            format!(
                r#"{{"claims":{{"P19":[{{"rank":"normal","mainsnak":{{"snaktype":"value","datavalue":{{"type":"wikibase-entityid","value":{{"id":"{id}"}}}}}}}}]}}}}"#
            )
        };
        let http = Routes(
            vec![
                (
                    "action=wbsearchentities",
                    r#"{"search":[{"id":"Q76","label":"Barack Obama"}]}"#.into(),
                ),
                (
                    "props=sitelinks",
                    r#"{"entities":{"Q76":{"sitelinks":{"enwiki":{}}}}}"#.into(),
                ),
                ("entity=Q76&property=P19", item("Q1")),
                (
                    "entity=Q1&property=P131",
                    r#"{"claims":{"P131":[{"mainsnak":{"datavalue":{"value":{"id":"Q2"}}}}]}}"#
                        .into(),
                ),
                (
                    "ids=Q1&props=labels",
                    r#"{"entities":{"Q1":{"labels":{"en":{"value":"Kapiolani Medical Center"}}}}}"#
                        .into(),
                ),
                (
                    "ids=Q2&props=labels",
                    r#"{"entities":{"Q2":{"labels":{"en":{"value":"Honolulu"}}}}}"#.into(),
                ),
            ],
            Default::default(),
        );
        assert_eq!(
            WikidataFacts
                .fact(&http, "where was barack obama born")
                .unwrap(),
            "Barack Obama was born in Kapiolani Medical Center, Honolulu. (Wikidata, CC0)"
        );
    }

    #[test]
    fn superlative_questions_prefer_the_article_that_says_so() {
        let q = parse_question("largest ocean in the world");
        assert_eq!(q.entity, "largest ocean");
        assert_eq!(q.superlative.as_deref(), Some("largest"));
        let found = r#"{"query":{"pages":[
            {"title":"Ocean","index":1,"extract":"An ocean is a body of salt water."},
            {"title":"Pacific Ocean","index":2,"extract":"The Pacific Ocean is the largest and deepest of Earth's oceanic divisions."}]}}"#;
        let http = Routes(
            vec![
                ("rest.php/v1/search/title", r#"{"pages":[]}"#.into()),
                ("generator=search", found.into()),
            ],
            Default::default(),
        );
        let out = WikipediaSmartFacts
            .fact(&http, "largest ocean in the world")
            .unwrap();
        assert!(out.starts_with("The Pacific Ocean is the largest"), "{out}");
    }

    #[test]
    fn a_measure_question_shows_the_intro_number() {
        let page = r#"{"query":{"pages":[{"title":"Speed of light","index":1,"fullurl":"https://en.wikipedia.org/wiki/Speed_of_light","extract":"The speed of light is the speed of electromagnetic waves. Light travels slower inside glass. The speed of light in vacuum is exactly 299792458 metres per second. Other text."}]}}"#;
        let http = Routes(
            vec![
                (
                    "rest.php/v1/search/title",
                    r#"{"pages":[{"title":"Speed of light"}]}"#.into(),
                ),
                ("titles=Speed%20of%20light", page.into()),
            ],
            Default::default(),
        );
        let out = WikipediaSmartFacts.fact(&http, "speed of light").unwrap();
        assert!(out.contains("299792458"), "{out}");
    }
}

#[cfg(test)]
mod live_one {
    use super::*;
    use crate::web_lookup::{parse_fact_subject, UreqHttp};

    /// One question through the best chain: `JITMO_Q="who wrote Hamlet" cargo test -p
    /// jitmo-connectors live_one -- --ignored --nocapture`.
    #[test]
    #[ignore = "uses the network"]
    fn live_one() {
        let q =
            std::env::var("JITMO_Q").unwrap_or_else(|_| "what is the capital of Australia".into());
        let subject = parse_fact_subject(&q).unwrap_or(q);
        let http = UreqHttp::new();
        println!("wikidata: {:?}", WikidataFacts.fact(&http, &subject));
        println!("smart:    {:?}", WikipediaSmartFacts.fact(&http, &subject));
    }
}
