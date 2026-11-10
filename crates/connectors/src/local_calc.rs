//! `local_calc`: answers that need no internet and no other app (tasks Q1, Q4, Q5).
//!
//! - `math`    percentages, tips, arithmetic and unit conversions (Q4)
//! - `almanac` Q1: reads the sentence and answers `moon` or `sun`
//! - `moon`    the next full or new moon, from the Meeus formulas (Q1)
//! - `sun`     sunrise and sunset for a city or the saved place (Q1)
//! - `time_in` the time in a city, or here (Q5)
//!
//! Each action reads the user's whole request from the `text` argument and
//! returns the answer in `message`. `check` runs the same parser again and
//! passes when it still produces an answer. Nothing is changed.

use crate::sun;
use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;

pub struct LocalCalc {
    /// The current time; replaced in tests.
    now: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    /// The saved place (setting `default_place`) for "when does the sun set" with no city.
    place: Box<dyn Fn() -> Option<String> + Send + Sync>,
}

impl Default for LocalCalc {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalCalc {
    pub fn new() -> Self {
        LocalCalc {
            now: Box::new(Utc::now),
            place: Box::new(|| None),
        }
    }

    pub fn with_now(now: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        LocalCalc {
            now: Box::new(now),
            place: Box::new(|| None),
        }
    }

    /// The saved place, fixed when the connector is built (like `WebLookup`).
    pub fn with_default_place(mut self, place: Option<String>) -> Self {
        let place = place.filter(|p| !p.trim().is_empty());
        self.place = Box::new(move || place.clone());
        self
    }

    /// The saved place, read each time it is needed (so "set my city" takes
    /// effect without a restart).
    pub fn with_place_source(
        mut self,
        place: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.place = Box::new(place);
        self
    }

    fn answer(&self, action: &str, text: &str) -> Result<Option<String>, ConnError> {
        Ok(match action {
            "math" => math(text),
            "moon" => Some(moon_answer(text, (self.now)())),
            "almanac" => almanac(text, (self.now)(), (self.place)().as_deref()),
            "sun" => Some(sun_answer(text, (self.now)(), (self.place)().as_deref())),
            "time_in" => time_in(text, (self.now)()),
            other => return Err(ConnError::UnknownAction(other.to_string())),
        })
    }
}

impl Connector for LocalCalc {
    fn id(&self) -> &str {
        "local_calc"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        ["math", "moon", "sun", "almanac", "time_in"]
            .iter()
            .map(|a| ActionSpec {
                name: a.to_string(),
                required_args: vec!["text".to_string()],
            })
            .collect()
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        let text = args
            .get("text")
            .ok_or_else(|| ConnError::MissingArg("text".into()))?;
        match self.answer(action, text)? {
            Some(message) => Ok(Output([("message".to_string(), message)].into())),
            None => Err(ConnError::Failed(
                "I couldn't work that out. Try rephrasing it.".into(),
            )),
        }
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        let text = args
            .get("text")
            .ok_or_else(|| ConnError::MissingArg("text".into()))?;
        Ok(self.answer(action, text)?.is_some())
    }

    fn undo(&self, _action: &str, _output: &Output) -> Option<Result<(), ConnError>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Math
// ---------------------------------------------------------------------------

/// Number text for people: up to 6 decimals, no trailing zeros, thousands commas.
pub fn format_number(x: f64) -> String {
    let rounded = (x * 1e6).round() / 1e6;
    let mut s = format!("{rounded:.6}");
    while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
        s.pop();
    }
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b.to_string()),
        None => (false, s),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (body, None),
    };
    let mut grouped = String::new();
    for (n, ch) in int.chars().enumerate() {
        if n > 0 && (int.len() - n) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!(
        "{}{}{}",
        if neg { "-" } else { "" },
        grouped,
        frac.map(|f| format!(".{f}")).unwrap_or_default()
    )
}

/// "five plus twenty five" -> "5 plus 25": spelled-out numbers up to 999,999 become digits.
pub(crate) fn number_words_to_digits(text: &str) -> String {
    fn unit(w: &str) -> Option<u64> {
        Some(match w {
            "zero" => 0,
            "one" => 1,
            "two" => 2,
            "three" => 3,
            "four" => 4,
            "five" => 5,
            "six" => 6,
            "seven" => 7,
            "eight" => 8,
            "nine" => 9,
            "ten" => 10,
            "eleven" => 11,
            "twelve" => 12,
            "thirteen" => 13,
            "fourteen" => 14,
            "fifteen" => 15,
            "sixteen" => 16,
            "seventeen" => 17,
            "eighteen" => 18,
            "nineteen" => 19,
            "twenty" => 20,
            "thirty" => 30,
            "forty" => 40,
            "fifty" => 50,
            "sixty" => 60,
            "seventy" => 70,
            "eighty" => 80,
            "ninety" => 90,
            _ => return None,
        })
    }
    let mut out: Vec<String> = Vec::new();
    // A run of number words being added up: (total so far, current group).
    let mut run: Option<(u64, u64)> = None;
    let flush = |run: &mut Option<(u64, u64)>, out: &mut Vec<String>| {
        if let Some((total, group)) = run.take() {
            out.push((total + group).to_string());
        }
    };
    for w in text.split_whitespace() {
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric());
        if let Some(n) = unit(bare) {
            let (total, group) = run.unwrap_or((0, 0));
            if group == 0 {
                run = Some((total, n));
            } else if group >= 20 && group % 10 == 0 && n < 10 {
                // "twenty five": a unit after tens joins them.
                run = Some((total, group + n));
            } else {
                // "one two": two separate numbers.
                flush(&mut run, &mut out);
                run = Some((0, n));
            }
        } else if bare == "hundred" && run.is_some() {
            let (total, group) = run.unwrap_or((0, 0));
            run = Some((total, group.max(1) * 100));
        } else if bare == "thousand" && run.is_some() {
            let (total, group) = run.unwrap_or((0, 0));
            run = Some((total + group.max(1) * 1000, 0));
        } else {
            flush(&mut run, &mut out);
            out.push(w.to_string());
        }
    }
    flush(&mut run, &mut out);
    out.join(" ")
}

pub fn math(text: &str) -> Option<String> {
    let t = text.to_lowercase().replace(['$', '₹', '€', '£'], " ");
    // 1,234 -> 1234
    let digits_comma = Regex::new(r"(\d),(\d{3})").ok()?;
    let mut t = t;
    while digits_comma.is_match(&t) {
        t = digits_comma.replace_all(&t, "$1$2").into_owned();
    }
    let t = number_words_to_digits(&t);
    let num = r"(\d+(?:\.\d+)?)";

    // "sqrt of 144", "square root of 2"
    let root = Regex::new(&format!(r"(?:sqrt|square\s*root)(?:\s*of)?\s*{num}")).ok()?;
    if let Some(c) = root.captures(&t) {
        let v: f64 = c[1].parse().ok()?;
        return Some(format!(
            "\u{221a}{} = {}",
            format_number(v),
            format_number(v.sqrt())
        ));
    }

    // "18% of 2450", "15 percent tip on 1200" (and the usual misspellings of percent)
    let pct = Regex::new(&format!(
        r"{num}\s*(?:%|percent|per cent|percnt|perecent|persent|precent|pct)\s*(tip\s*)?(?:of|on|for)\s*{num}"
    ))
    .ok()?;
    if let Some(c) = pct.captures(&t) {
        let p: f64 = c[1].parse().ok()?;
        let amount: f64 = c[3].parse().ok()?;
        let part = amount * p / 100.0;
        return Some(if c.get(2).is_some() {
            format!(
                "A {}% tip on {} is {} (total {}).",
                format_number(p),
                format_number(amount),
                format_number(part),
                format_number(amount + part)
            )
        } else {
            format!(
                "{}% of {} is {}.",
                format_number(p),
                format_number(amount),
                format_number(part)
            )
        });
    }

    // "5 km to miles", "100 f in c"
    let conv = Regex::new(&format!(r"{num}\s*([a-z°]+)\s+(?:to|in|into)\s+([a-z°]+)")).ok()?;
    if let Some(c) = conv.captures(&t) {
        let v: f64 = c[1].parse().ok()?;
        if let Some((out, from, to)) = convert(v, &c[2], &c[3]) {
            return Some(format!(
                "{} {} is {} {}.",
                format_number(v),
                from,
                format_number(out),
                to
            ));
        }
    }

    // Plain arithmetic: words for operators, then the longest sum-looking part.
    let mut e = t;
    // "split 1200 by 3" is a division.
    if let Ok(split) = Regex::new(&format!(r"split\s*{num}\s*by\s*{num}")) {
        e = split.replace_all(&e, "$1 / $2").into_owned();
    }
    // "20 into 3" means 20 times 3 (the usual Indian-English reading).
    if let Ok(into) = Regex::new(&format!(r"{num}\s*into\s*{num}")) {
        e = into.replace_all(&e, "$1 * $2").into_owned();
    }
    for (word, sym) in [
        ("multiplied by", "*"),
        ("divided by", "/"),
        ("times", "*"),
        ("plus", "+"),
        ("minus", "-"),
        ("over", "/"),
        ("to the power of", "^"),
        ("power", "^"),
    ] {
        e = e.replace(word, sym);
    }
    let expr = Regex::new(r"[\d.(][\d.\s+\-*/x^()]*[\d)]").ok()?;
    let best = expr
        .find_iter(&e)
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| s.chars().any(|c| "+-*/x^".contains(c)))
        .max_by_key(String::len)?;
    let value = eval(&best)?;
    Some(format!(
        "{} = {}",
        best.replace('x', "×"),
        format_number(value)
    ))
}

/// Recursive-descent arithmetic: + - * / x ^ and parentheses.
fn eval(src: &str) -> Option<f64> {
    struct P<'a> {
        s: Vec<char>,
        i: usize,
        _src: &'a str,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.s.get(self.i).is_some_and(|c| c.is_whitespace()) {
                self.i += 1;
            }
        }
        fn peek(&mut self) -> Option<char> {
            self.ws();
            self.s.get(self.i).copied()
        }
        fn expr(&mut self) -> Option<f64> {
            let mut v = self.term()?;
            while let Some(c) = self.peek() {
                if c == '+' || c == '-' {
                    self.i += 1;
                    let r = self.term()?;
                    v = if c == '+' { v + r } else { v - r };
                } else {
                    break;
                }
            }
            Some(v)
        }
        fn term(&mut self) -> Option<f64> {
            let mut v = self.power()?;
            while let Some(c) = self.peek() {
                if c == '*' || c == '/' || c == 'x' {
                    self.i += 1;
                    let r = self.power()?;
                    if c == '/' {
                        if r == 0.0 {
                            return None;
                        }
                        v /= r;
                    } else {
                        v *= r;
                    }
                } else {
                    break;
                }
            }
            Some(v)
        }
        fn power(&mut self) -> Option<f64> {
            let base = self.atom()?;
            if self.peek() == Some('^') {
                self.i += 1;
                let exp = self.power()?;
                return Some(base.powf(exp));
            }
            Some(base)
        }
        fn atom(&mut self) -> Option<f64> {
            match self.peek()? {
                '(' => {
                    self.i += 1;
                    let v = self.expr()?;
                    if self.peek()? != ')' {
                        return None;
                    }
                    self.i += 1;
                    Some(v)
                }
                '-' => {
                    self.i += 1;
                    Some(-self.atom()?)
                }
                c if c.is_ascii_digit() || c == '.' => {
                    let start = self.i;
                    while self
                        .s
                        .get(self.i)
                        .is_some_and(|c| c.is_ascii_digit() || *c == '.')
                    {
                        self.i += 1;
                    }
                    self.s[start..self.i]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .ok()
                }
                _ => None,
            }
        }
    }
    let mut p = P {
        s: src.chars().collect(),
        i: 0,
        _src: src,
    };
    let v = p.expr()?;
    p.ws();
    (p.i == p.s.len() && v.is_finite()).then_some(v)
}

enum Unit {
    /// Kind and size in the kind's base unit.
    Linear(&'static str, f64),
    Temp(char),
}

fn unit(name: &str) -> Option<(Unit, &'static str)> {
    use Unit::*;
    Some(match name {
        "m" | "meter" | "meters" | "metre" | "metres" => (Linear("length", 1.0), "m"),
        "km" | "kilometer" | "kilometers" | "kilometre" | "kilometres" => {
            (Linear("length", 1000.0), "km")
        }
        "cm" | "centimeter" | "centimeters" => (Linear("length", 0.01), "cm"),
        "mm" | "millimeter" | "millimeters" => (Linear("length", 0.001), "mm"),
        "mi" | "mile" | "miles" => (Linear("length", 1609.344), "miles"),
        "ft" | "foot" | "feet" => (Linear("length", 0.3048), "feet"),
        "in" | "inch" | "inches" => (Linear("length", 0.0254), "inches"),
        "yd" | "yard" | "yards" => (Linear("length", 0.9144), "yards"),
        "g" | "gram" | "grams" => (Linear("mass", 1.0), "g"),
        "kg" | "kilogram" | "kilograms" | "kilo" | "kilos" => (Linear("mass", 1000.0), "kg"),
        "lb" | "lbs" | "pound" | "pounds" => (Linear("mass", 453.592_37), "lb"),
        "oz" | "ounce" | "ounces" => (Linear("mass", 28.349_523), "oz"),
        "ml" | "milliliter" | "milliliters" => (Linear("volume", 1.0), "ml"),
        "l" | "liter" | "liters" | "litre" | "litres" => (Linear("volume", 1000.0), "L"),
        "gal" | "gallon" | "gallons" => (Linear("volume", 3_785.411_784), "gallons"),
        "cup" | "cups" => (Linear("volume", 236.588_236), "cups"),
        "c" | "celsius" | "°c" => (Temp('c'), "°C"),
        "f" | "fahrenheit" | "°f" => (Temp('f'), "°F"),
        "k" | "kelvin" => (Temp('k'), "K"),
        _ => return None,
    })
}

fn convert(v: f64, from: &str, to: &str) -> Option<(f64, &'static str, &'static str)> {
    let (a, an) = unit(from)?;
    let (b, bn) = unit(to)?;
    let out = match (a, b) {
        (Unit::Linear(k1, f1), Unit::Linear(k2, f2)) if k1 == k2 => v * f1 / f2,
        (Unit::Temp(x), Unit::Temp(y)) => {
            let celsius = match x {
                'c' => v,
                'f' => (v - 32.0) * 5.0 / 9.0,
                _ => v - 273.15,
            };
            match y {
                'c' => celsius,
                'f' => celsius * 9.0 / 5.0 + 32.0,
                _ => celsius + 273.15,
            }
        }
        _ => return None,
    };
    Some((out, an, bn))
}

// ---------------------------------------------------------------------------
// Moon (Meeus, "Astronomical Algorithms", chapter 49, main terms)
// ---------------------------------------------------------------------------

fn julian_to_utc(jd: f64) -> DateTime<Utc> {
    let secs = ((jd - 2_440_587.5) * 86_400.0).round() as i64;
    Utc.timestamp_opt(secs, 0).single().unwrap_or_default()
}

/// Time of the lunation `k` (whole numbers are new moons, .5 are full moons).
fn phase_time(k: f64) -> DateTime<Utc> {
    let t = k / 1236.85;
    let jde = 2_451_550.097_66 + 29.530_588_861 * k + 0.000_154_37 * t * t
        - 0.000_000_150 * t * t * t
        + 0.000_000_000_73 * t.powi(4);
    let rad = |d: f64| d.to_radians();
    let e = 1.0 - 0.002_516 * t - 0.000_007_4 * t * t;
    let m = rad(2.5534 + 29.105_356_70 * k - 0.000_001_4 * t * t - 0.000_000_11 * t.powi(3));
    let mp = rad(
        201.5643 + 385.816_935_28 * k + 0.010_758_2 * t * t + 0.000_012_38 * t.powi(3)
            - 0.000_000_058 * t.powi(4),
    );
    let f = rad(
        160.7108 + 390.670_502_84 * k - 0.001_611_8 * t * t - 0.000_002_27 * t.powi(3)
            + 0.000_000_011 * t.powi(4),
    );
    let om = rad(124.7746 - 1.563_755_88 * k + 0.002_067_2 * t * t + 0.000_002_15 * t.powi(3));
    let full = (k - k.floor() - 0.5).abs() < 1e-9;
    let c = if full {
        -0.40614 * mp.sin()
            + 0.17302 * e * m.sin()
            + 0.01614 * (2.0 * mp).sin()
            + 0.01043 * (2.0 * f).sin()
            + 0.00734 * e * (mp - m).sin()
            - 0.00515 * e * (mp + m).sin()
            + 0.00209 * e * e * (2.0 * m).sin()
            - 0.00111 * (mp - 2.0 * f).sin()
            - 0.00057 * (mp + 2.0 * f).sin()
            + 0.00056 * e * (2.0 * mp + m).sin()
            - 0.00042 * (3.0 * mp).sin()
            + 0.00042 * e * (m + 2.0 * f).sin()
            + 0.00038 * e * (m - 2.0 * f).sin()
            - 0.00024 * e * (2.0 * mp - m).sin()
            - 0.00017 * om.sin()
    } else {
        -0.40720 * mp.sin()
            + 0.17241 * e * m.sin()
            + 0.01608 * (2.0 * mp).sin()
            + 0.01039 * (2.0 * f).sin()
            + 0.00739 * e * (mp - m).sin()
            - 0.00514 * e * (mp + m).sin()
            + 0.00208 * e * e * (2.0 * m).sin()
            - 0.00111 * (mp - 2.0 * f).sin()
            - 0.00057 * (mp + 2.0 * f).sin()
            + 0.00056 * e * (2.0 * mp + m).sin()
            - 0.00042 * (3.0 * mp).sin()
            + 0.00042 * e * (m + 2.0 * f).sin()
            + 0.00038 * e * (m - 2.0 * f).sin()
            - 0.00024 * e * (2.0 * mp - m).sin()
            - 0.00017 * om.sin()
    };
    julian_to_utc(jde + c)
}

/// The next full (or new) moon strictly after `from`.
pub fn next_phase(from: DateTime<Utc>, full: bool) -> DateTime<Utc> {
    let years = f64::from(from.year()) + f64::from(from.ordinal0()) / 365.25;
    let mut k = ((years - 2000.0) * 12.368_5).floor() - 2.0;
    if full {
        k += 0.5;
    }
    loop {
        let t = phase_time(k);
        if t > from {
            return t;
        }
        k += 1.0;
    }
}

fn moon_answer(text: &str, now: DateTime<Utc>) -> String {
    let t = text.to_lowercase();
    let full = !t.contains("new moon");
    let when = next_phase(now, full).with_timezone(&Local);
    format!(
        "The next {} moon is on {}.",
        if full { "full" } else { "new" },
        when.format("%A %-d %B at %-I:%M %p")
    )
}

// ---------------------------------------------------------------------------
// Almanac: moon or sun, decided from the sentence (task Q1)
// ---------------------------------------------------------------------------

/// True when the sentence is about the sun (sunrise, sunset, "the sun sets").
fn mentions_sun(t: &str) -> bool {
    Regex::new(r"\b(?:sun\s?rises?|sun\s?sets?|sundown|sunup|sun)\b").is_ok_and(|r| r.is_match(t))
}

pub fn almanac(text: &str, now: DateTime<Utc>, default_place: Option<&str>) -> Option<String> {
    let t = text.to_lowercase();
    if t.contains("moon") {
        Some(moon_answer(text, now))
    } else if mentions_sun(&t) {
        Some(sun_answer(text, now, default_place))
    } else {
        None
    }
}

/// The longest known place name in `t`.
fn find_place(t: &str) -> Option<&'static (&'static str, f64, f64, &'static str)> {
    sun::PLACES
        .iter()
        .filter(|p| {
            Regex::new(&format!(r"\b{}\b", regex::escape(p.0))).is_ok_and(|r| r.is_match(t))
        })
        .max_by_key(|p| p.0.len())
}

const NO_PLACE: &str = "Which city? Try “sunset in Pune”, or save one with “set my city to Pune”.";

pub fn sun_answer(text: &str, now: DateTime<Utc>, default_place: Option<&str>) -> String {
    let t = text.to_lowercase();
    let wants_set = Regex::new(r"sun\s?sets?|sundown|sun goes down").is_ok_and(|r| r.is_match(&t));
    let wants_rise = Regex::new(r"sun\s?rises?|sunup|sun comes up").is_ok_and(|r| r.is_match(&t));
    let (rise, set) = match (wants_rise, wants_set) {
        (true, false) => (true, false),
        (false, true) => (false, true),
        _ => (true, true),
    };
    let (offset, day_word) = if t.contains("day after tomorrow") {
        (2, "the day after tomorrow")
    } else if t.contains("tomorrow") {
        (1, "tomorrow")
    } else if t.contains("yesterday") {
        (-1, "yesterday")
    } else {
        (0, "today")
    };

    // A place said in the sentence wins, then the saved place.
    let said_unknown = Regex::new(
        r"\b(?:in|at|for)\s+(?:the\s+)?([a-z][a-z .'-]*?)(?:\s+(?:today|tomorrow|tonight|yesterday|please)\b|[?.!,]|$)",
    )
    .ok()
    .and_then(|r| r.captures(&t).map(|c| c[1].trim().to_string()))
    .filter(|w| {
        !w.is_empty()
            && !matches!(
                w.as_str(),
                "the evening" | "evening" | "morning" | "the morning" | "my city" | "my place" | "here"
            )
    });
    let spot = match find_place(&t) {
        Some(p) => p,
        None => match said_unknown {
            Some(name) if default_place.is_none_or(|d| !d.to_lowercase().contains(&name)) => {
                return format!(
                    "I don't know where {} is yet. Try a bigger city nearby.",
                    title(&name)
                );
            }
            _ => {
                let Some(d) = default_place else {
                    return NO_PLACE.to_string();
                };
                let key = d.split(',').next().unwrap_or(d).trim().to_lowercase();
                match sun::place(&key) {
                    Some(p) => p,
                    None => {
                        return format!(
                            "I don't know where {} is yet. Say a bigger city nearby, like “sunset in Pune”.",
                            d.trim()
                        )
                    }
                }
            }
        },
    };
    let (name, lat, lon, zone) = *spot;
    let Ok(tz) = zone.parse::<Tz>() else {
        return NO_PLACE.to_string();
    };
    let Some(date) = now
        .with_timezone(&tz)
        .date_naive()
        .checked_add_signed(Duration::days(offset))
    else {
        return NO_PLACE.to_string();
    };

    let mut parts: Vec<String> = Vec::new();
    for (wanted, is_rise, noun) in [(rise, true, "Sunrise"), (set, false, "Sunset")] {
        if !wanted {
            continue;
        }
        let part = match sun::sun_event(date, lat, lon, tz, is_rise) {
            sun::SunEvent::At(when) => {
                // Round to the nearest minute, as published tables do.
                let rounded = when + Duration::seconds(30);
                let verb = if when <= now { "was" } else { "is" };
                format!(
                    "{} {} at {}",
                    noun.to_lowercase(),
                    verb,
                    rounded.with_timezone(&tz).format("%-I:%M %p")
                )
            }
            sun::SunEvent::PolarNight => {
                return format!(
                    "There is no sunrise or sunset in {} {day_word}: the sun stays below the horizon all day (polar night).",
                    title(name)
                )
            }
            sun::SunEvent::PolarDay => {
                return format!(
                    "There is no sunrise or sunset in {} {day_word}: the sun stays up all day (midnight sun).",
                    title(name)
                )
            }
            sun::SunEvent::NoneToday => {
                format!("there is no {} {day_word}", noun.to_lowercase())
            }
        };
        parts.push(part);
    }
    let body = parts.join("; ");
    format!("In {} {day_word}: {body}.", title(name))
}

// ---------------------------------------------------------------------------
// Time zones
// ---------------------------------------------------------------------------

const CITIES: &[(&str, &str)] = &[
    ("new york", "America/New_York"),
    ("nyc", "America/New_York"),
    ("boston", "America/New_York"),
    ("washington", "America/New_York"),
    ("chicago", "America/Chicago"),
    ("dallas", "America/Chicago"),
    ("houston", "America/Chicago"),
    ("denver", "America/Denver"),
    ("phoenix", "America/Phoenix"),
    ("los angeles", "America/Los_Angeles"),
    ("san francisco", "America/Los_Angeles"),
    ("seattle", "America/Los_Angeles"),
    ("vancouver", "America/Vancouver"),
    ("toronto", "America/Toronto"),
    ("mexico city", "America/Mexico_City"),
    ("sao paulo", "America/Sao_Paulo"),
    ("buenos aires", "America/Argentina/Buenos_Aires"),
    ("london", "Europe/London"),
    ("dublin", "Europe/Dublin"),
    ("paris", "Europe/Paris"),
    ("berlin", "Europe/Berlin"),
    ("madrid", "Europe/Madrid"),
    ("rome", "Europe/Rome"),
    ("amsterdam", "Europe/Amsterdam"),
    ("zurich", "Europe/Zurich"),
    ("moscow", "Europe/Moscow"),
    ("istanbul", "Europe/Istanbul"),
    ("cairo", "Africa/Cairo"),
    ("lagos", "Africa/Lagos"),
    ("nairobi", "Africa/Nairobi"),
    ("johannesburg", "Africa/Johannesburg"),
    ("dubai", "Asia/Dubai"),
    ("abu dhabi", "Asia/Dubai"),
    ("riyadh", "Asia/Riyadh"),
    ("tehran", "Asia/Tehran"),
    ("karachi", "Asia/Karachi"),
    ("delhi", "Asia/Kolkata"),
    ("mumbai", "Asia/Kolkata"),
    ("bangalore", "Asia/Kolkata"),
    ("bengaluru", "Asia/Kolkata"),
    ("kolkata", "Asia/Kolkata"),
    ("chennai", "Asia/Kolkata"),
    ("india", "Asia/Kolkata"),
    ("dhaka", "Asia/Dhaka"),
    ("bangkok", "Asia/Bangkok"),
    ("jakarta", "Asia/Jakarta"),
    ("singapore", "Asia/Singapore"),
    ("kuala lumpur", "Asia/Kuala_Lumpur"),
    ("hong kong", "Asia/Hong_Kong"),
    ("shanghai", "Asia/Shanghai"),
    ("beijing", "Asia/Shanghai"),
    ("taipei", "Asia/Taipei"),
    ("manila", "Asia/Manila"),
    ("seoul", "Asia/Seoul"),
    ("tokyo", "Asia/Tokyo"),
    ("japan", "Asia/Tokyo"),
    ("sydney", "Australia/Sydney"),
    ("melbourne", "Australia/Melbourne"),
    ("perth", "Australia/Perth"),
    ("auckland", "Pacific/Auckland"),
    ("honolulu", "Pacific/Honolulu"),
    ("utc", "UTC"),
    ("gmt", "UTC"),
];

fn time_in(text: &str, now: DateTime<Utc>) -> Option<String> {
    let t = text.to_lowercase();
    // The longest city name found in the request wins ("new york" over "york").
    let hit = CITIES
        .iter()
        .copied()
        .chain(sun::PLACES.iter().map(|p| (p.0, p.3)))
        .filter(|(name, _)| {
            let re = Regex::new(&format!(r"\b{}\b", regex::escape(name))).ok();
            re.is_some_and(|r| r.is_match(&t))
        })
        .max_by_key(|(name, _)| name.len());
    match hit {
        Some((name, zone)) => {
            let tz: Tz = zone.parse().ok()?;
            let local = now.with_timezone(&tz);
            Some(format!(
                "It's {} in {} ({}).",
                local.format("%-I:%M %p"),
                title(name),
                local.format("%A %-d %B")
            ))
        }
        // "in <something>" that we do not know: say so rather than show the wrong time.
        None if Regex::new(r"\b(?:in|at)\s+[a-z]{3,}").ok()?.is_match(&t) => None,
        None => {
            let local = now.with_timezone(&Local);
            Some(format!(
                "It's {} ({}).",
                local.format("%-I:%M %p"),
                local.format("%A %-d %B")
            ))
        }
    }
}

fn title(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    // --- math ---

    #[test]
    fn t21_percent_of() {
        assert_eq!(
            math("What's 18% of 2,450?").unwrap(),
            "18% of 2,450 is 441."
        );
    }

    #[test]
    fn t64_tip() {
        let a = math("whats 15 percent tip on 1200").unwrap();
        assert!(a.contains("is 180 "), "{a}");
        assert!(a.contains("total 1,380"), "{a}");
    }

    #[test]
    fn arithmetic_with_precedence_words_and_brackets() {
        assert_eq!(math("what is 12 + 30 / 6").unwrap(), "12 + 30 / 6 = 17");
        assert_eq!(math("calculate (2 + 3) * 4").unwrap(), "(2 + 3) * 4 = 20");
        assert_eq!(math("45 times 12").unwrap(), "45 * 12 = 540");
        assert_eq!(math("2 ^ 10").unwrap(), "2 ^ 10 = 1,024");
    }

    #[test]
    fn words_roots_splits_powers_and_percent_typos_are_understood() {
        assert_eq!(math("five plus five").unwrap(), "5 + 5 = 10");
        assert_eq!(math("twenty five times four").unwrap(), "25 * 4 = 100");
        assert_eq!(math("one hundred plus two").unwrap(), "100 + 2 = 102");
        assert_eq!(math("sqrt of 144").unwrap(), "\u{221a}144 = 12");
        assert_eq!(math("square root of 81").unwrap(), "\u{221a}81 = 9");
        assert_eq!(math("split 1200 by 3").unwrap(), "1200 / 3 = 400");
        assert_eq!(math("20 into 3").unwrap(), "20 * 3 = 60");
        assert_eq!(math("2 power 10").unwrap(), "2 ^ 10 = 1,024");
        assert_eq!(math("1,000 + 500").unwrap(), "1000 + 500 = 1,500");
        for typo in ["18 percnt of 2450", "18 perecent of 2450", "18 pct of 2450"] {
            assert_eq!(math(typo).unwrap(), "18% of 2,450 is 441.", "{typo}");
        }
        assert_eq!(number_words_to_digits("one two"), "1 2");
        assert_eq!(
            number_words_to_digits("a ten rupee note"),
            "a 10 rupee note"
        );
        assert!(math("calculate").is_none());
    }

    #[test]
    fn division_by_zero_is_no_answer() {
        assert!(math("5 / 0").is_none());
    }

    #[test]
    fn unit_conversions() {
        assert_eq!(
            math("convert 5 km to miles").unwrap(),
            "5 km is 3.106856 miles."
        );
        assert_eq!(math("100 f in c").unwrap(), "100 °F is 37.777778 °C.");
        assert!(
            math("5 kg to km").is_none(),
            "different kinds do not convert"
        );
    }

    #[test]
    fn no_numbers_no_answer() {
        assert!(math("hello there").is_none());
    }

    #[test]
    fn numbers_are_formatted_for_people() {
        assert_eq!(format_number(1234567.5), "1,234,567.5");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-1500.0), "-1,500");
    }

    // --- moon ---

    #[test]
    fn full_moons_match_published_dates() {
        // Published full moons (UTC) for 2026; the formula should be within minutes.
        let cases = [
            (at(2026, 1, 1, 0, 0), at(2026, 1, 3, 10, 3)),
            (at(2026, 1, 10, 0, 0), at(2026, 2, 1, 22, 9)),
            (at(2026, 2, 10, 0, 0), at(2026, 3, 3, 11, 38)),
        ];
        for (from, expected) in cases {
            let got = next_phase(from, true);
            let off = (got - expected).num_minutes().abs();
            assert!(
                off <= 15,
                "from {from}: got {got}, expected {expected}, off {off} min"
            );
        }
    }

    #[test]
    fn new_moon_is_about_two_weeks_from_full() {
        let full = next_phase(at(2026, 6, 1, 0, 0), true);
        let new = next_phase(full, false);
        let days = (new - full).num_hours() as f64 / 24.0;
        assert!((13.5..16.0).contains(&days), "{days}");
    }

    #[test]
    fn moon_answer_names_the_phase() {
        assert!(
            moon_answer("when is the next full moon", at(2026, 1, 1, 0, 0)).contains("full moon")
        );
        assert!(moon_answer("next new moon", at(2026, 1, 1, 0, 0)).contains("new moon"));
    }

    // --- sun and almanac ---

    #[test]
    fn sunset_in_a_named_city_uses_that_citys_zone() {
        // 9 Nov 2026 03:00 UTC = 08:30 IST. Published Mumbai sunset 18:01:59, sunrise 06:42:35.
        let a = sun_answer("sunrise tomorrow in Mumbai", at(2026, 11, 9, 3, 0), None);
        assert!(
            a.starts_with("In Mumbai tomorrow: sunrise is at 6:4"),
            "{a}"
        );
        let b = sun_answer(
            "when does the sun set in Mumbai",
            at(2026, 11, 9, 3, 0),
            None,
        );
        assert_eq!(b, "In Mumbai today: sunset is at 6:02 PM.");
    }

    #[test]
    fn past_events_use_was_and_both_are_given_for_sun_times() {
        let a = sun_answer("sun times in Mumbai", at(2026, 11, 9, 14, 0), None);
        assert_eq!(
            a,
            "In Mumbai today: sunrise was at 6:43 AM; sunset was at 6:02 PM."
        );
    }

    #[test]
    fn no_place_uses_the_saved_place_or_asks() {
        let now = at(2026, 11, 9, 3, 0);
        let a = sun_answer("when does the sun set today", now, Some("Pune"));
        assert!(a.starts_with("In Pune today: sunset is at "), "{a}");
        let b = sun_answer("when does the sun set today", now, Some("Pune, India"));
        assert_eq!(a, b);
        let ask = sun_answer("when does the sun set today", now, None);
        assert!(ask.starts_with("Which city?"), "{ask}");
        let unknown = sun_answer("when does the sun set today", now, Some("Atlantis"));
        assert!(
            unknown.contains("don't know where Atlantis is"),
            "{unknown}"
        );
    }

    #[test]
    fn an_unknown_said_place_is_not_swapped_for_the_saved_one() {
        let a = sun_answer("sunset in Atlantis", at(2026, 11, 9, 3, 0), Some("Pune"));
        assert!(a.contains("don't know where Atlantis is"), "{a}");
    }

    #[test]
    fn polar_night_and_midnight_sun_give_a_plain_message() {
        let a = sun_answer("sunrise in Tromso", at(2026, 12, 21, 12, 0), None);
        assert!(a.contains("polar night"), "{a}");
        let b = sun_answer("sunset in Tromso", at(2026, 6, 21, 12, 0), None);
        assert!(b.contains("midnight sun"), "{b}");
    }

    #[test]
    fn almanac_picks_moon_or_sun_from_the_sentence() {
        let now = at(2026, 1, 1, 0, 0);
        assert!(almanac("When is the next full moon?", now, None)
            .unwrap()
            .contains("full moon"));
        assert!(almanac("full moon when", now, None)
            .unwrap()
            .contains("full moon"));
        assert!(almanac("sunrise in Delhi", now, None)
            .unwrap()
            .contains("Delhi"));
        assert!(almanac("what is the capital of France", now, None).is_none());
        assert!(
            almanac("remind me on sunday", now, None).is_none(),
            "sunday is not the sun"
        );
    }

    #[test]
    fn the_old_moon_action_and_the_new_ones_work_through_the_connector() {
        let c =
            LocalCalc::with_now(|| at(2026, 11, 9, 3, 0)).with_default_place(Some("Pune".into()));
        let a: Args = [("text".to_string(), "when is the next full moon".to_string())].into();
        assert!(c.execute("moon", &a).unwrap().0["message"].contains("full moon"));
        assert!(c.execute("almanac", &a).unwrap().0["message"].contains("full moon"));
        let s: Args = [("text".to_string(), "sunset today".to_string())].into();
        assert!(c.execute("almanac", &s).unwrap().0["message"].starts_with("In Pune today"));
        assert!(c.check("almanac", &s).unwrap());
        let bad: Args = [("text".to_string(), "hello".to_string())].into();
        assert!(!c.check("almanac", &bad).unwrap());
    }

    #[test]
    fn the_place_can_be_read_live() {
        use std::sync::{Arc, Mutex};
        let saved = Arc::new(Mutex::new(Some("Delhi".to_string())));
        let s2 = saved.clone();
        let c = LocalCalc::with_now(|| at(2026, 11, 9, 3, 0))
            .with_place_source(move || s2.lock().unwrap().clone());
        let a: Args = [("text".to_string(), "sunset".to_string())].into();
        assert!(c.execute("almanac", &a).unwrap().0["message"].starts_with("In Delhi"));
        *saved.lock().unwrap() = Some("Pune".into());
        assert!(c.execute("almanac", &a).unwrap().0["message"].starts_with("In Pune"));
    }

    #[test]
    fn new_cities_also_work_for_time_in() {
        let a = time_in("time in Pune", at(2026, 1, 1, 0, 0)).unwrap();
        assert!(a.starts_with("It's 5:30 AM in Pune"), "{a}");
    }

    // --- zones ---

    #[test]
    fn time_in_new_york_follows_daylight_saving() {
        // 12:00 UTC in July is 8:00 AM in New York (EDT); in January 7:00 AM (EST).
        let summer = time_in("What time is it in New York?", at(2026, 7, 1, 12, 0)).unwrap();
        assert!(summer.starts_with("It's 8:00 AM in New York"), "{summer}");
        let winter = time_in("what time is it in new york", at(2026, 1, 1, 12, 0)).unwrap();
        assert!(winter.starts_with("It's 7:00 AM in New York"), "{winter}");
    }

    #[test]
    fn longest_city_name_wins_and_india_works() {
        let a = time_in("time in Mumbai", at(2026, 1, 1, 0, 0)).unwrap();
        assert!(a.starts_with("It's 5:30 AM in Mumbai"), "{a}");
    }

    #[test]
    fn unknown_place_is_no_answer_not_the_local_time() {
        assert!(time_in("what time is it in Atlantis", at(2026, 1, 1, 0, 0)).is_none());
    }

    #[test]
    fn no_place_means_here() {
        assert!(time_in("what time is it", at(2026, 1, 1, 0, 0))
            .unwrap()
            .starts_with("It's "));
    }

    // --- connector ---

    #[test]
    fn execute_returns_the_answer_and_check_agrees() {
        let c = LocalCalc::with_now(|| at(2026, 1, 1, 0, 0));
        let args: Args = [("text".to_string(), "what's 18% of 2450".to_string())].into();
        let out = c.execute("math", &args).unwrap();
        assert_eq!(out.0["message"], "18% of 2,450 is 441.");
        assert!(c.check("math", &args).unwrap());
        let bad: Args = [("text".to_string(), "hello".to_string())].into();
        assert!(c.execute("math", &bad).is_err());
        assert!(!c.check("math", &bad).unwrap());
        assert!(matches!(
            c.execute("nope", &args),
            Err(ConnError::UnknownAction(_))
        ));
    }
}
