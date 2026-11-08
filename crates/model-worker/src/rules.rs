//! Tier 0 extractor, rules half (pure Rust, no model): dates, times, durations,
//! numbers, settings, and closed lists of apps, sites, contacts and files, plus
//! sentence frames ("tell X <message>", "play X", "search for X", ...).
//!
//! This is a port of the SP2 spike rules (`spikes/sp2-extractor/extract2.py`),
//! kept deliberately close to the Python so the two can be compared sentence by
//! sentence (`tools/extractor_parity.json`). The rules generalise well for
//! date/time/duration/number/setting/app and for contacts and apps from closed
//! lists; open-ended spans (query, media, message, event, file) are weak, which
//! is why the GLiNER model is consulted for them (see `gliner.rs`).
//!
//! Spans use the spike's type names (`file`, `message`, ...); [`to_slot_span`]
//! turns them into the slot types skills use (`file_ref`, ...).

use fancy_regex::{Captures, Match, Regex};
use jitmo_ipc::{ContextNote, Span};
use std::collections::HashSet;
use std::sync::LazyLock;

/// A span found in the text. `start`/`end` are byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ent {
    pub kind: &'static str,
    pub text: String,
    pub start: usize,
    pub end: usize,
}

impl Ent {
    fn new(kind: &'static str, text: &str, start: usize) -> Self {
        Ent {
            kind,
            text: text.to_string(),
            start,
            end: start + text.len(),
        }
    }
}

/// A span the GLiNER model proposed (already mapped from its label to a type name).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelEnt {
    pub kind: &'static str,
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub score: f32,
}

/// Closed lists supplied at run time (Core's app list, contact list, selected file).
#[derive(Debug, Clone, Default)]
pub struct Gazetteer {
    pub apps: Vec<String>,
    pub contacts: Vec<String>,
    pub files: Vec<String>,
}

impl Gazetteer {
    pub fn from_context(c: &ContextNote) -> Self {
        Gazetteer {
            apps: c.apps.clone(),
            contacts: c.contacts.clone(),
            files: c.selected_item.iter().cloned().collect(),
        }
    }
}

/// Slot type used by skills for a span type of the extractor.
pub fn to_slot_span(e: &Ent) -> Span {
    Span {
        text: e.text.clone(),
        kind: match e.kind {
            "file" => "file_ref",
            k => k,
        }
        .to_string(),
    }
}

// ---------------------------------------------------------------------------
// regex helpers (fancy-regex: look-around like Python's `re`)
// ---------------------------------------------------------------------------

fn rx(p: &str) -> Regex {
    Regex::new(&format!("(?i){p}")).unwrap_or_else(|e| panic!("bad pattern {p}: {e}"))
}

/// Case-sensitive variant.
fn rx_cs(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("bad pattern {p}: {e}"))
}

fn find<'t>(r: &Regex, s: &'t str) -> Option<Match<'t>> {
    r.find(s).ok().flatten()
}

fn caps<'t>(r: &Regex, s: &'t str) -> Option<Captures<'t, str>> {
    r.captures(s).ok().flatten()
}

fn finds<'t>(r: &Regex, s: &'t str) -> Vec<Match<'t>> {
    r.find_iter(s).filter_map(Result::ok).collect()
}

fn capss<'t>(r: &Regex, s: &'t str) -> Vec<Captures<'t, str>> {
    r.captures_iter(s).filter_map(Result::ok).collect()
}

fn is(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

/// Python `re.match`: a match that starts at position 0.
fn match_start<'t>(r: &Regex, s: &'t str) -> Option<Match<'t>> {
    find(r, s).filter(|m| m.start() == 0)
}

fn anchored(p: &str) -> Regex {
    rx(&format!("^(?:{p})$"))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Occurrences of `word` as a whole word (not inside another word or hyphenated
/// word), ignoring case: Python `(?<![\w-])word(?![\w-])`. Returns (start, end).
fn word_hits(text: &str, lower: &str, word: &str) -> Vec<(usize, usize)> {
    let w = word.to_ascii_lowercase();
    if w.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, _) in lower.match_indices(&w) {
        let j = i + w.len();
        let before_ok = text[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c) && c != '-');
        let after_ok = text[j..]
            .chars()
            .next()
            .is_none_or(|c| !is_word_char(c) && c != '-');
        if before_ok && after_ok {
            out.push((i, j));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// word lists and patterns
// ---------------------------------------------------------------------------

const APPS: &[&str] = &[
    "spotify",
    "youtube",
    "whatsapp",
    "teams",
    "chrome",
    "excel",
    "word",
    "powerpoint",
    "notepad",
    "outlook",
    "calculator",
    "vs code",
    "edge",
    "gmail",
    "drive",
    "onedrive",
    "slack",
    "zoom",
    "telegram",
    "discord",
    "skype",
    "firefox",
    "brave",
    "paint",
    "vlc",
    "steam",
    "onenote",
    "photoshop",
    "figma",
    "notion",
    "terminal",
    "file explorer",
    "task manager",
    "google meet",
    "microsoft teams",
    "visual studio code",
];
const SITES: &[&str] = &[
    "amazon",
    "netflix",
    "github",
    "linkedin",
    "wikipedia",
    "flipkart",
    "reddit",
    "google",
    "facebook",
    "twitter",
    "myntra",
    "zomato",
    "swiggy",
    "instagram",
    "stack overflow",
    "hacker news",
    "irctc",
    "bookmyshow",
    "quora",
    "ebay",
    "imdb",
    "hotstar",
    "prime video",
    "makemytrip",
    "paytm",
    "x.com",
];
const SETTINGS: &[&str] = &[
    "do not disturb",
    "volume",
    "brightness",
    "bluetooth",
    "wi-fi",
    "wifi",
    "do not disturb",
    "don't disturb",
    "dnd",
    "notifications",
    "sound",
    "airplane mode",
    "quiet mode",
    "silent mode",
    "dark mode",
    "night light",
    "battery saver",
    "focus assist",
];
const FAMILY: &[&str] = &[
    "mom", "mum", "mother", "dad", "father", "bhai", "didi", "boss", "manager", "hr", "wife",
    "husband",
];
const NOT_NAME: &[&str] = &[
    "me",
    "my",
    "it",
    "this",
    "that",
    "these",
    "those",
    "the",
    "a",
    "an",
    "all",
    "everyone",
    "us",
    "him",
    "her",
    "them",
    "everything",
    "something",
    "anything",
    "new",
    "latest",
];
const NOT_APP: &[&str] = &[
    "it",
    "that",
    "this",
    "my",
    "the",
    "something",
    "anything",
    "everything",
    "downloads",
    "documents",
    "desktop",
    "pictures",
    "music",
    "videos",
    "settings",
    "browser",
    "camera",
    "mail",
    "email",
    "emails",
    "inbox",
    "calendar",
    "notes",
    "up",
    "again",
    "now",
    "please",
    "pls",
    "folder",
    "file",
    "files",
    "tab",
    "window",
    "screenshot",
    "photos",
    "recycle",
    "bin",
    "home",
    "news",
    "weather",
    "maps",
    "sleep",
    "work",
    "focus",
    "timer",
    "alarm",
    "meeting",
    "call",
    "a",
    "an",
    "all",
    "everyone",
];
const NOT_MEDIA: &[&str] = &[
    "the music",
    "music",
    "something",
    "it",
    "songs",
    "a song",
    "my focus playlist",
];

const WD: &str = r"(?:mon|tues?|wed(?:nes)?|thu(?:rs?)?|fri|sat(?:ur)?|sun)(?:day)?";
const PART: &str = r"(?:morning|afternoon|evening|night)";
const TIME: &str = r"(?:\d{1,2}(?::\d{2})?\s?(?:am|pm)|noon|midnight|\d{1,2}:\d{2})";
const MONTH: &str = r"(?:jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sept?(?:ember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)";
const AGO: &str = r"(?:(?:\d+|an?|one|two|three|four|five|six|a couple of|a few)\s+(?:days?|weeks?|months?|years?)\s+ago)";
const NUMW: &str = r"(?:\d+(?:\.\d+)?|an?|one|two|three|four|five|six|seven|eight|nine|ten|fifteen|twenty|thirty|forty|forty[- ]five|sixty)";
const NOT_TIME: &str = r"(?![\d:]|\s?(?:am|pm|st|nd|rd|th)\b)";
const EXT: &str = r"(?:pdf|docx?|xlsx?|pptx?|txt|csv|png|jpe?g|zip)";
const FILE_NOUN: &str = r"(?:sheet|deck|report|resume|cv|contract|proposal|invoices?|documents?|docs?|pdfs?|photos?|pictures?|pics?|images?|screenshots?|receipts?|letter|thing|ticket|agreement|attachments?|notes|slides|spreadsheet|file|files|slip|lease|scan|return|pass|policy|bills?|statements?|forms?|certificates?|presentation|plan|itinerary|manual|brochure|transcript|payslip|licen[sc]e|summary|minutes|draft|ppt|essay|assignment|timetable|budget|memo)";
const EVENT_WORDS: &str = r"(?:review|standup|stand-up|sync|lunch|dinner|appointment|interview|class|session|demo|retro|workshop|gym|yoga|1:1)";
const LEAD: &str = r"^(?:(?:pls|please|yo|hey|can you|can u|could you|lemme|gimme)\s+)*";

struct P {
    date: Regex,
    date_full: Regex,
    dur: Regex,
    num: Regex,
    file_ext: Regex,
    file_np: Regex,
    file_np_start: Regex,
    file_poss: Regex,
    file_noun_full: Regex,
    file_noun_start: Regex,
    file_noun_end: Regex,
    folder_cap: Regex,
    folder_np: Regex,
    msg_after: Regex,
    msg_after_app: Regex,
    msg_verb: Regex,
    creator_lead: Regex,
    talk: Regex,
    talk_chain: Regex,
    mail: Regex,
    post: Regex,
    group: Regex,
    free: Regex,
    send2: Regex,
    play: Regex,
    creators: Vec<Regex>,
    search: Regex,
    about: Regex,
    remind: Regex,
    remind_skip: Regex,
    add_ev: Regex,
    block_for: Regex,
    chg_ev: Regex,
    num_to: Regex,
    appish_full: Regex,
    hr_dur_before: Regex,
    family_before: Regex,
    family_after_s: Regex,
    family_after_on: Regex,
    put_on: Regex,
    cal_or_sched: Regex,
    cal_tail: Regex,
    ev_lead: Regex,
    ev_bad_start: Regex,
    ev_words_end: Regex,
    ev_generic_full: Regex,
    ev_generic_start: Regex,
    app_ctx: Regex,
    open_last: Regex,
    folder_after: Regex,
    folder_verb: Regex,
    place_prefix: Regex,
    leading_that_to: Regex,
    norm_s: Regex,
    norm_art: Regex,
    mail_skip_g3: Regex,
}

static P: LazyLock<P> = LazyLock::new(|| {
    let day = format!(
        r"(?:today|tonight|tomorrow|tmrw|tmr|yesterday|(?:this|next|last)\s+(?:week|month|weekend|year|morning|afternoon|evening|night|{WD})|{WD})"
    );
    let ord = format!(r"(?:the\s+)?\d{{1,2}}(?:st|nd|rd|th)(?:\s+of\s+{MONTH})?");
    let date_p = format!(
        r"\b{AGO}\b|\b{ord}\b|\b{MONTH}\s+\d{{1,2}}(?:st|nd|rd|th)?\b|\b\d{{1,2}}\s+{MONTH}\b|\b\d{{1,2}}\s+(?:tomorrow|tmrw|today|tonight)\b|\b{day}(?:\s+{PART})?(?:\s+(?:at\s+)?{TIME})?\b|\b(?:at\s+)?{TIME}\b|\bat\s+\d{{1,2}}\b"
    );
    let dur_p =
        format!(r"\b(?:half an hour|{NUMW}[ -]?(?:minutes?|mins?|hours?|hrs?|seconds?|secs?))\b");
    let nt = NOT_TIME;
    let num_p = format!(
        r"\b\d+(?:\.\d+)?\s?(?:%|percent)|\b\d{{1,3}}(?:,\d{{3}})+\b|\b\d{{3,}}\b|(?<=between )\d+{nt}|(?<=among )\d+{nt}|(?<=times )\d+{nt}|(?<=plus )\d+{nt}|(?<=minus )\d+{nt}|(?<=divided by )\d+{nt}|\b\d+(?=\s+(?:between|among|ways|times|plus|minus|divided)\b)"
    );
    let file_ext_p = format!(r"\b(?:(?:my|the|a|an|this|that)\s+(?:[\w-]+\s+)?)?[\w-]+\.{EXT}\b");
    let file_np_p = format!(
        r"\b(?:the|my|that|these|this|those|our|his|her)\s+(?:[\w-]+\s+){{0,2}}?{FILE_NOUN}\b(?:\s+{FILE_NOUN}\b)*(?!\s+(?:folder|directory))"
    );
    let file_poss_p = format!(
        r"\b(?:today|yesterday|tomorrow|tonight|(?:this|last|next)\s+\w+)'s\s+((?:[\w-]+\s+)?{FILE_NOUN}\b(?:\s+{FILE_NOUN}\b)*)"
    );
    let appsite: Vec<String> = APPS
        .iter()
        .chain(SITES)
        .map(|w| fancy_regex::escape(w).to_string())
        .collect();
    let appsite_rx = format!("(?:{})", appsite.join("|"));
    let setting_rx: Vec<String> = SETTINGS
        .iter()
        .map(|w| fancy_regex::escape(w).to_string())
        .collect();
    let talk_verbs = "tell|message|msg|ping|text|ask|dm|notify";
    let talk_p = format!(
        r"{LEAD}({talk_verbs})\s+(?:(?:the|my)\s+)?([\w'.-]+(?:\s+(?:team|channel|group|chat))?)(?:\s+on\s+({appsite_rx}))?\s+(.+?)\s*[.?!]*$"
    );
    let mail_p = format!(
        r"{LEAD}(email|mail|forward)\s+([\w'.-]+)\s+(?:(?:that|saying|:)\s+)?(.+?)\s*[.?!]*$"
    );
    let post_p = format!(
        r"{LEAD}post\s+(.+?)\s+(?:in|to|on)\s+((?:the\s+)?[\w-]+\s+(?:channel|group|chat))\s*[.?!]*$"
    );
    let play_p = format!(
        r"{LEAD}(?:play|put on)\s+(?:(?:some|me)\s+)?(.+?)(?:\s+on\s+{appsite_rx}|'s\b.*|\s+songs?\b.*|\s+music\b.*|\s+(?:please|pls|plz|for me|now|thanks)\b.*)?\s*[.?!]*$"
    );
    let search_p = format!(r"\bsearch(?:\s+{appsite_rx})?\s+for\s+(.+?)\s*[.?!]*$");
    let about_p = format!(
        r"\b(?:emails?|mails?|messages?)\b.*?\babout\s+(.+?)(?=\s+(?:from|since|of|on)\s+(?:{day}|last|this|next|\d)|\s*[.?!]*$)"
    );
    let remind_p = format!(
        r"\bremind\s+me\s+(?:to|about|abt)\s+(.+?)(?=\s+(?:at|in|on|by|{day})\b|\s*[.?!]*$)"
    );
    let add_ev_p = r"\b(?:add|put|schedule)\s+(?:(?:an?|my)\s+)?(.+?)(?=\s+(?:on|for|in|at|to)\s+(?:my\s+)?(?:calendar|\w+day|\d)|\s+(?:on|for|at|in|tomorrow|today|tmrw|next|this)\b|\s*[.?!]*$)";
    let chg_ev_p = r"\b(?:decline|cancel|skip|move|push|reschedule)\s+(?:the|my)\s+(.+?)(?=\s+(?:on|to|for|from|at)\b|\s+(?:today|tomorrow|tmrw)\b|\s*[.?!]*$)";
    let open_last_p = format!(
        r"{LEAD}(?:open|launch|start|close|quit|run|visit|go to|take me to|fire up)\s+(?:up\s+)?(?:the\s+)?([A-Za-z][\w.+-]*(?:\s[A-Z][\w.+-]*)?)\s*[.?!]*$"
    );
    P {
        date: rx(&date_p),
        date_full: anchored(&date_p),
        dur: rx(&dur_p),
        num: rx(&num_p),
        file_ext: rx(&file_ext_p),
        file_np: rx(&file_np_p),
        file_np_start: rx(&format!("^(?:{file_np_p})")),
        file_poss: rx(&file_poss_p),
        file_noun_full: anchored(FILE_NOUN),
        file_noun_start: rx(&format!(r"^(?:{FILE_NOUN})\b")),
        file_noun_end: rx(&format!("{FILE_NOUN}$")),
        folder_cap: rx_cs(r"\b(?:to|into)\s+(?:the\s+)?([A-Z][\w-]+)(?:\s+folder)?\b"),
        folder_np: rx(r"\bthe\s+([\w-]+)\s+folder\b"),
        msg_after: rx(r"\b(?:saying|say|says|that|:)\s+(.+?)[.?!]*$"),
        msg_after_app: rx(r"\bon\s+(?:whatsapp|teams)\s+(.+?)[.?!]*$"),
        msg_verb: rx(r"\b(?:tell|message|ping|text|whatsapp|mail|email|reply|say|let)\b"),
        talk: rx(&talk_p),
        talk_chain: rx(&format!(
            r"\b(?:and|then|and then)\s+(?=(?:{talk_verbs})\b)"
        )),
        mail: rx(&mail_p),
        post: rx(&post_p),
        group: rx(r"\b(?:the\s+)?(?!the\b)([\w-]+\s+(?:team|channel|group|chat))\b"),
        free: rx(r"\b(?:is|are|does)\s+([\w'-]+)(?:\s+and\s+i)?\s+(?:get\s+)?free\b"),
        send2: rx(
            r"\b(?:send|give|forward|share|text|mail|email|show)\s+(?!(?:to|from|in|on|at|for|with|about|by|of|into|all|any)\b)([A-Za-z][\w'.-]*)\s+(?:the|that|my|this|a|an|these|those)\s+",
        ),
        play: rx(&play_p),
        creators: vec![
            rx(r"\b([\w.]+(?:\s[\w.]+)?)'s\s+(?:latest|newest|new)\s+(?:\w+\s+)?vid"),
            rx(r"\bnew\s+(\w+)\s+vid"),
            rx(r"\bvideos?\s+(?:from|by)\s+(\w+)"),
        ],
        search: rx(&search_p),
        about: rx(&about_p),
        remind: rx(&remind_p),
        remind_skip: rx(&format!(r"^(?:{day}|\d)")),
        add_ev: rx(add_ev_p),
        block_for: rx(r"\bblock\b.*?\bfor\s+(.+?)\s*[.?!]*$"),
        chg_ev: rx(chg_ev_p),
        num_to: rx(&format!(
            r"\b(?:{})\s+to\s+(\d{{1,3}})\b",
            setting_rx.join("|")
        )),
        appish_full: anchored(&appsite_rx),
        hr_dur_before: rx(&format!(r"(?:\d|\b{NUMW})\s*$")),
        family_before: rx(
            r"\b(?:to|with|from|for|tell|ask|text|message|msg|ping|email|mail|dm|notify|whatsapp|forward|call|invite|share|cc)\s*(?:my)?$",
        ),
        family_after_s: rx(r"^'s\b"),
        family_after_on: rx(r"^\s+(?:on|via|over)\s+\w+\s*$"),
        put_on: rx(r"\bput\s+on\b"),
        cal_or_sched: rx(r"calendar|schedule"),
        cal_tail: rx(r"\s+(?:in|on|to)\s+my\s+calendar$"),
        ev_lead: rx(&format!(
            r"^(?:(?:the|my|a|an)\s+)?(?:(?:{TIME}|{day})\s+)*"
        )),
        ev_bad_start: rx(&format!(r"^(?:this|that|these|it|one|{NUMW}|\d)\b")),
        ev_words_end: rx(&format!("{EVENT_WORDS}$")),
        ev_generic_full: anchored(
            r"(?:meeting|call|event|invite|reminder|alarm|timer|appointment|meetings|\w+\s+(?:meeting|call|invite))s?",
        ),
        ev_generic_start: rx(r"^(?:meeting|call|event)\b"),
        app_ctx: rx(
            r"\b(?:open|launch|start|close|quit|run|go to|visit|take me to|browse|load|switch to|pull up|fire up|on|in|via|using|use)\s+(?:up\s+)?(?:the\s+)?$",
        ),
        open_last: rx(&open_last_p),
        folder_after: rx(r"^\s*(?:folder|file|tab|window)"),
        folder_verb: rx(r"\b(?:move|save|copy|put|drop|store)\b"),
        place_prefix: rx(
            r"^(?:the\s+)?(?:capital|population|weather|time|temperature|flights?|hotels?|currency)\s+(?:of|in|at|to|for)\s+",
        ),
        leading_that_to: rx(r"^(?:that|to)\s+"),
        norm_s: rx(r"'s$"),
        norm_art: rx(r"^(?:at|on|the|my|this|these|that|a|an|some|for)\s+"),
        mail_skip_g3: rx(r"^(?:to|from|with)\b"),
        creator_lead: rx(r"^(?:open|play|show|watch|get|see|find|pull up|put on)\s+"),
    }
});

/// The scoring normaliser: lower case, no end punctuation, no "'s", no leading article.
pub fn norm(s: &str) -> String {
    let p = &*P;
    let s = s.to_lowercase();
    let s = s.trim_matches(|c| " .?!,".contains(c)).to_string();
    let s = p.norm_s.replace(&s, "").to_string();
    let s = p.norm_art.replace(&s, "").to_string();
    let s = p.norm_art.replace(&s, "").to_string();
    s.trim().to_string()
}

fn lower_in(list: &[&str], s: &str) -> bool {
    let l = s.to_lowercase();
    list.contains(&l.as_str())
}

// ---------------------------------------------------------------------------
// closed-class spans, dates, numbers
// ---------------------------------------------------------------------------

fn gazetteer(text: &str, lower: &str, words: &[String], kind: &'static str, out: &mut Vec<Ent>) {
    for w in words {
        for (s, e) in word_hits(text, lower, w) {
            out.push(Ent::new(kind, &text[s..e], s));
        }
    }
}

fn closed_class_spans(text: &str, gaz: &Gazetteer) -> Vec<Ent> {
    let p = &*P;
    let lower = text.to_ascii_lowercase();
    let own = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut out = Vec::new();
    gazetteer(text, &lower, &own(APPS), "app", &mut out);
    gazetteer(text, &lower, &gaz.apps, "app", &mut out);
    gazetteer(text, &lower, &own(SITES), "website", &mut out);
    gazetteer(text, &lower, &own(SETTINGS), "setting", &mut out);
    gazetteer(text, &lower, &gaz.contacts, "contact", &mut out);
    gazetteer(text, &lower, &gaz.files, "file", &mut out);
    let ext = finds(&p.file_ext, text);
    for m in &ext {
        out.push(Ent::new("file", m.as_str(), m.start()));
    }
    if ext.is_empty() {
        for m in finds(&p.file_np, text) {
            out.push(Ent::new("file", m.as_str(), m.start()));
        }
        for c in capss(&p.file_poss, text) {
            if let Some(g) = c.get(1) {
                out.push(Ent::new("file", g.as_str(), g.start()));
            }
        }
    }
    for c in capss(&p.folder_np, text) {
        if let Some(g) = c.get(1) {
            out.push(Ent::new("folder", g.as_str(), g.start()));
        }
    }
    let m = caps(&p.msg_after, text).or_else(|| caps(&p.msg_after_app, text));
    if let Some(m) = m {
        if is(&p.msg_verb, text) {
            if let Some(g) = m.get(1) {
                out.push(Ent::new("message", g.as_str(), g.start()));
            }
        }
    }
    out
}

fn rule_spans(text: &str) -> Vec<Ent> {
    let p = &*P;
    let mut out = Vec::new();
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for (kind, r) in [
        ("duration", &p.dur),
        ("date_time", &p.date),
        ("number", &p.num),
    ] {
        for m in finds(r, text) {
            if taken.iter().any(|&(s, e)| m.start() < e && s < m.end()) {
                continue;
            }
            taken.push((m.start(), m.end()));
            out.push(Ent::new(kind, m.as_str(), m.start()));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// sentence frames
// ---------------------------------------------------------------------------

fn add(out: &mut Vec<Ent>, kind: &'static str, c: &Captures<'_, str>, g: usize, off: usize) {
    if let Some(m) = c.get(g) {
        if !m.as_str().trim().is_empty() {
            out.push(Ent {
                kind,
                text: m.as_str().to_string(),
                start: m.start() + off,
                end: m.end() + off,
            });
        }
    }
}

fn group_str<'t>(c: &Captures<'t, str>, g: usize) -> &'t str {
    c.get(g).map_or("", |m| m.as_str())
}

fn frame_spans(text: &str) -> Vec<Ent> {
    let p = &*P;
    let mut out: Vec<Ent> = Vec::new();

    let mut off = 0;
    let mut m = caps(&p.talk, text);
    if m.is_none() {
        for c in finds(&p.talk_chain, text) {
            let e = c.end();
            if let Some(mm) = caps(&p.talk, &text[e..]) {
                m = Some(mm);
                off = e;
                break;
            }
        }
    }
    let talk_ok = m
        .as_ref()
        .is_some_and(|m| !lower_in(NOT_NAME, group_str(m, 2)));
    if talk_ok {
        let m = m.expect("checked");
        add(&mut out, "contact", &m, 2, off);
        add(&mut out, "message", &m, 4, off);
        let g4 = m.get(4).expect("group 4 is not optional");
        let rest = p.leading_that_to.replace(g4.as_str(), "").to_string();
        if rest.len() != g4.as_str().len() {
            out.pop();
            let end = g4.end() + off;
            out.push(Ent {
                kind: "message",
                text: rest.clone(),
                start: end - rest.len(),
                end,
            });
        }
    } else if let Some(m) = caps(&p.mail, text) {
        let g1 = group_str(&m, 1);
        let g2 = group_str(&m, 2);
        let g3 = group_str(&m, 3);
        if !lower_in(NOT_NAME, g2)
            && !is(&p.file_noun_full, g2)
            && !g2.contains('.')
            && g1.to_lowercase() != "forward"
            && !is(&p.mail_skip_g3, g3)
        {
            add(&mut out, "contact", &m, 2, 0);
            let (e2, s3) = (m.get(2).expect("g2").end(), m.get(3).expect("g3").start());
            let explicit = !text[e2..s3].trim().is_empty();
            if explicit
                || (!is(&p.date_full, g3)
                    && match_start(&p.file_np_start, g3).is_none()
                    && !is(&p.file_ext, g3)
                    && match_start(&p.file_noun_start, g3).is_none())
            {
                add(&mut out, "message", &m, 3, 0);
            }
        }
    }

    if let Some(m) = caps(&p.send2, text) {
        let g1 = group_str(&m, 1);
        if !lower_in(NOT_NAME, g1)
            && !is(&p.file_noun_full, g1)
            && !out.iter().any(|o| o.kind == "contact")
        {
            add(&mut out, "contact", &m, 1, 0);
        }
    }
    if let Some(m) = caps(&p.post, text) {
        add(&mut out, "message", &m, 1, 0);
        add(&mut out, "contact", &m, 2, 0);
    }
    for m in capss(&p.group, text) {
        add(&mut out, "contact", &m, 1, 0);
    }
    for m in capss(&p.free, text) {
        let g1 = group_str(&m, 1).to_lowercase();
        if !NOT_NAME.contains(&g1.as_str())
            && !["i", "there", "anyone", "somebody"].contains(&g1.as_str())
        {
            add(&mut out, "contact", &m, 1, 0);
        }
    }
    let lower = text.to_ascii_lowercase();
    for w in FAMILY {
        for (s, e) in word_hits(text, &lower, w) {
            if *w == "hr" && is(&p.hr_dur_before, &text[..s]) {
                continue;
            }
            let before = text[..s].trim_end();
            if !before.is_empty()
                && !is(&p.family_before, before)
                && !is(&p.family_after_s, &text[e..])
                && !is(&p.family_after_on, &text[e..])
            {
                continue;
            }
            if !out
                .iter()
                .any(|o| o.kind == "contact" && o.start <= s && s < o.end)
            {
                out.push(Ent::new("contact", &text[s..e], s));
            }
        }
    }
    if let Some(m) = caps(&p.play, text) {
        let g1 = group_str(&m, 1);
        if !lower_in(NOT_MEDIA, g1) && !is(&p.appish_full, g1) {
            add(&mut out, "media", &m, 1, 0);
        }
    }
    for creator in &p.creators {
        for m in capss(creator, text) {
            let g1 = group_str(&m, 1);
            if !out.iter().any(|o| o.kind == "media")
                && !["the", "my", "this", "that", "his", "her"]
                    .contains(&g1.to_lowercase().as_str())
            {
                if let Some(g) = find(&p.creator_lead, g1) {
                    let g1m = m.get(1).expect("g1");
                    out.push(Ent::new("media", &g1[g.end()..], g1m.start() + g.end()));
                } else {
                    add(&mut out, "media", &m, 1, 0);
                }
            }
        }
    }
    let q = caps(&p.search, text).or_else(|| caps(&p.about, text));
    if let Some(m) = q {
        add(&mut out, "query", &m, 1, 0);
    }
    if let Some(m) = caps(&p.remind, text) {
        if !is(&p.remind_skip, group_str(&m, 1)) {
            add(&mut out, "message", &m, 1, 0);
        }
    }
    // events
    for (which, r) in [(0, &p.add_ev), (1, &p.block_for), (2, &p.chg_ev)] {
        let Some(m) = caps(r, text) else { continue };
        let g1m = m.get(1).expect("g1");
        let g1 = g1m.as_str();
        let g1l = g1.trim().to_lowercase();
        if NOT_NAME.contains(&g1l.as_str()) || g1l == "me" || g1l == "us" {
            continue;
        }
        if which == 0 && is(&p.put_on, text) && !is(&p.cal_or_sched, text) {
            continue; // "put on <artist>" is music, not an event
        }
        let a0 = g1m.start();
        let mut x = g1;
        if let Some(t) = find(&p.cal_tail, x) {
            x = &x[..t.start()];
        }
        let lead_end = find(&p.ev_lead, x).map_or(0, |l| l.end());
        let a = a0 + lead_end;
        x = &x[lead_end..];
        if x.is_empty() || is(&p.ev_bad_start, x) {
            continue;
        }
        if which == 2 && !is(&p.ev_words_end, x) {
            continue;
        }
        if is(&p.ev_generic_full, x) || is(&p.ev_generic_start, x) {
            continue;
        }
        out.push(Ent::new("event", x, a));
        break;
    }
    if let Some(m) = caps(&p.num_to, text) {
        add(&mut out, "number", &m, 1, 0);
    }
    out
}

// ---------------------------------------------------------------------------
// unknown apps and sites
// ---------------------------------------------------------------------------

/// Apps/sites that are not in the list. The model's app/website guess is used only
/// right after an opening or switching verb or "on/in/via"; a bare "open <Word>"
/// falls back to app (website if it has a dot).
fn unknown_apps(text: &str, raw: &[ModelEnt], threshold: f32, existing: &[Ent]) -> Vec<Ent> {
    let p = &*P;
    let mut res: Vec<Ent> = Vec::new();
    let taken = |s: usize, e: usize, res: &[Ent]| {
        existing.iter().chain(res).any(|o| s < o.end && o.start < e)
    };
    for e in raw {
        if matches!(e.kind, "app" | "website")
            && e.score >= threshold.max(0.6)
            && is(&p.app_ctx, &text[..e.start])
            && !lower_in(NOT_APP, &e.text)
            && !is(&p.file_noun_end, &e.text)
            && !taken(e.start, e.end, &res)
            && !is(&p.folder_after, &text[e.end..])
        {
            res.push(Ent {
                kind: e.kind,
                text: e.text.clone(),
                start: e.start,
                end: e.end,
            });
        }
    }
    if res.is_empty()
        && !existing
            .iter()
            .any(|o| matches!(o.kind, "app" | "website" | "file" | "folder" | "media"))
    {
        if let Some(m) = caps(&p.open_last, text) {
            let g1m = m.get(1).expect("g1");
            let g1 = g1m.as_str();
            if !lower_in(NOT_APP, g1) && !is(&p.file_noun_full, g1) {
                let guess = raw
                    .iter()
                    .find(|e| {
                        matches!(e.kind, "app" | "website")
                            && e.text.to_lowercase() == g1.to_lowercase()
                    })
                    .map_or(if g1.contains('.') { "website" } else { "app" }, |e| e.kind);
                res.push(Ent::new(guess, g1, g1m.start()));
            }
        }
    }
    res
}

// ---------------------------------------------------------------------------
// the whole extractor
// ---------------------------------------------------------------------------

/// Kinds the rules own: when they are found by a rule, a model span of that kind is
/// ignored. (The model is only used for contact, file, media, query, message, place,
/// event and, via `unknown_apps`, app/website.)
const RULE_OWNED: &[&str] = &[
    "date_time",
    "duration",
    "number",
    "app",
    "website",
    "setting",
    "folder",
];

/// Combine rule spans, frames and (optionally) model spans into the final list,
/// without duplicates, in text order.
pub fn extract(text: &str, raw: &[ModelEnt], threshold: f32, gaz: &Gazetteer) -> Vec<Ent> {
    let p = &*P;
    let mut out: Vec<Ent> = Vec::new();
    for e in raw.iter().filter(|e| e.score >= threshold) {
        if RULE_OWNED.contains(&e.kind) {
            continue;
        }
        let (mut start, mut etext) = (e.start, e.text.as_str());
        if e.kind == "place" {
            if let Some(m) = match_start(&p.place_prefix, etext) {
                start += m.end();
                etext = &etext[m.end()..];
            }
        }
        out.push(Ent::new(e.kind, etext, start));
    }

    let mut fixed: Vec<Ent> = closed_class_spans(text, gaz);
    fixed.extend(rule_spans(text));
    let frames = frame_spans(text);
    let prot: Vec<&Ent> = frames
        .iter()
        .filter(|f| matches!(f.kind, "message" | "query" | "event"))
        .collect();
    fixed.retain(|f| !prot.iter().any(|q| q.start <= f.start && f.end <= q.end));
    // a file span is dropped when it sits inside a folder span ("the Invoices folder")
    let folders: Vec<Ent> = fixed
        .iter()
        .filter(|g| g.kind == "folder")
        .cloned()
        .collect();
    fixed.retain(|f| {
        !(f.kind == "file"
            && folders.iter().any(|g| {
                g.start <= f.start && f.end <= g.end + 7 && g.start.saturating_sub(8) <= f.start
            }))
    });
    fixed.retain(|f| {
        !frames
            .iter()
            .any(|g| f.start < g.end && g.start < f.end && g.kind != f.kind)
    });
    fixed.extend(frames);
    let all: Vec<Ent> = fixed.iter().chain(out.iter()).cloned().collect();
    fixed.extend(unknown_apps(text, raw, threshold, &all));
    out.retain(|o| !fixed.iter().any(|f| o.start < f.end && f.start < o.end));
    out.extend(fixed);

    // nothing inside a message is an entity of its own ("tell Neha ask mom to call")
    let msgs: Vec<Ent> = out
        .iter()
        .filter(|o| o.kind == "message")
        .cloned()
        .collect();
    out.retain(|o| {
        o.kind == "message" || !msgs.iter().any(|g| g.start <= o.start && o.end <= g.end)
    });

    // a capitalised word after to/into is a folder only if nothing else claimed it
    if is(&p.folder_verb, text) {
        for c in capss(&p.folder_cap, text) {
            let g = c.get(1).expect("g1");
            if !out.iter().any(|o| g.start() < o.end && o.start < g.end()) {
                out.push(Ent::new("folder", g.as_str(), g.start()));
            }
        }
    }

    let mut seen: HashSet<(&'static str, String)> = HashSet::new();
    let mut res = Vec::new();
    for o in out {
        if o.text.trim().chars().count() < 2 && !o.text.trim().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if o.text.trim().is_empty() {
            continue;
        }
        if seen.insert((o.kind, norm(&o.text))) {
            res.push(o);
        }
    }
    res.sort_by_key(|e| (e.start, e.end));
    res
}

/// Rules only (no model): what is needed to fill slots without loading GLiNER.
pub fn extract_rules(text: &str, gaz: &Gazetteer) -> Vec<Ent> {
    extract(text, &[], 0.3, gaz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(text: &str) -> Vec<(String, String)> {
        extract_rules(text, &Gazetteer::default())
            .into_iter()
            .map(|e| (e.kind.to_string(), e.text))
            .collect()
    }

    fn has(text: &str, kind: &str, span: &str) -> bool {
        ex(text)
            .iter()
            .any(|(k, t)| k == kind && norm(t) == norm(span))
    }

    #[test]
    fn dates_times_durations_numbers() {
        assert!(has("will it rain tomorrow", "date_time", "tomorrow"));
        assert!(has("remind me at 5pm to call", "date_time", "5pm"));
        assert!(has("set a timer for 10 minutes", "duration", "10 minutes"));
        assert!(has("what is 15% of 2,000", "number", "15%"));
        assert!(has("emails from 3 weeks ago", "date_time", "3 weeks ago"));
        assert!(has("what's 9 times 47", "number", "9"));
    }

    #[test]
    fn settings_and_closed_lists() {
        assert!(has("turn on do not disturb", "setting", "do not disturb"));
        assert!(has("set the volume to 30", "setting", "volume"));
        assert!(has("set the volume to 30", "number", "30"));
        assert!(has("open Spotify", "app", "spotify"));
        assert!(has("go to amazon", "website", "amazon"));
    }

    #[test]
    fn app_names_are_whole_words() {
        // "word" must not be found inside "password"
        assert!(!has("reset my password", "app", "word"));
        assert!(has("open word", "app", "word"));
    }

    #[test]
    fn frames_find_contact_and_message() {
        let r = ex("Tell Ved on WhatsApp I'm on my way.");
        assert!(r.contains(&("contact".into(), "Ved".into())), "{r:?}");
        assert!(r.contains(&("app".into(), "WhatsApp".into())), "{r:?}");
        assert!(
            r.contains(&("message".into(), "I'm on my way".into())),
            "{r:?}"
        );
    }

    #[test]
    fn files_search_and_play() {
        assert!(has("send report.pdf to Ved", "file", "report.pdf"));
        assert!(has("search for pasta recipes", "query", "pasta recipes"));
        assert!(has("play Arijit Singh on Spotify", "media", "Arijit Singh"));
    }

    #[test]
    fn nothing_inside_a_message_is_an_entity() {
        let r = ex("tell Neha to ask mom to call me tomorrow");
        assert!(!r.iter().any(|(k, _)| k == "date_time"), "{r:?}");
        assert!(
            !r.iter().any(|(k, t)| k == "contact" && t == "mom"),
            "{r:?}"
        );
    }

    #[test]
    fn unknown_app_after_open_is_an_app() {
        assert!(has("open Obsidian", "app", "Obsidian"));
        assert!(has("open example.com", "website", "example.com"));
    }

    #[test]
    fn context_lists_are_closed_lists() {
        let gaz = Gazetteer {
            contacts: vec!["Zoya Khan".into()],
            apps: vec!["Obsidian".into()],
            files: vec![],
        };
        let r = extract_rules("send it to zoya khan and open obsidian", &gaz);
        assert!(r
            .iter()
            .any(|e| e.kind == "contact" && norm(&e.text) == "zoya khan"));
        assert!(r
            .iter()
            .any(|e| e.kind == "app" && norm(&e.text) == "obsidian"));
    }

    #[test]
    fn slot_span_names_follow_skill_types() {
        let e = Ent::new("file", "report.pdf", 0);
        assert_eq!(to_slot_span(&e).kind, "file_ref");
        let e = Ent::new("contact", "Ved", 0);
        assert_eq!(to_slot_span(&e).kind, "contact");
    }
}
