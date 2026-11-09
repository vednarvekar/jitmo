//! Tier 0 extractor, second rule pass (the "v2" rules).
//!
//! The first pass (`rules.rs`, a port of the SP2 spike) is good at closed lists and at
//! dates, durations, numbers and settings, and weak at open-ended spans (query, media,
//! message, event, file) because it only knows a few sentence frames. This pass runs on
//! its output and adds what generalised on a separate dev set (`tools/extractor/dev_set.txt`):
//!
//! - more frames per slot type (search / lookup / google, play / queue / listen to,
//!   drop X a text / reply / let X know, remind me <time> to, cancel / book / schedule),
//! - one trimming routine that peels the closed-class tail (a time, "on Spotify",
//!   "please", "online") off an open-ended span instead of leaving it in,
//! - noun-phrase files after a transfer verb ("send the design mockup to Aarav"),
//! - small repairs: "Wednesday at 3" as one date, "every morning at 6am", "Desktop folder",
//!   a website after "go to", roles and capitalised names after to / with / from.
//!
//! Every new span replaces what the first pass found in the same place, and anything
//! inside a query or a message is not an entity of its own. Where no frame matches, the
//! first pass result is returned unchanged. The Python-parity path (`rules::extract`) does
//! not go through here.

use crate::rules::{
    closed_spans, Ent, Gazetteer, ModelEnt, APPS, FAMILY, NOT_NAME, SETTINGS, SITES, WD,
};
use fancy_regex::{Captures, Regex};
use std::collections::HashSet;
use std::sync::LazyLock;

fn rx(p: &str) -> Regex {
    Regex::new(&format!("(?i){p}")).unwrap_or_else(|e| panic!("bad pattern {p}: {e}"))
}

fn caps<'t>(r: &Regex, s: &'t str) -> Option<Captures<'t, str>> {
    r.captures(s).ok().flatten()
}

fn is(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

/// Closed-class mention: any app or site name, as a regex alternation.
fn appsite() -> String {
    let all: Vec<String> = APPS
        .iter()
        .chain(SITES)
        .map(|w| fancy_regex::escape(w).to_string())
        .collect();
    format!("(?:{})", all.join("|"))
}

const LEAD: &str = r"^(?:(?:pls|please|yo|hey|can you|can u|could you|could u|lemme|gimme|i wanna|i want to|i'd like to|id like to|go ahead and)\s+)*";
/// A person: optional "my/the", one word, optionally "team/channel/group/chat" after it.
const NAME: &str = r"(?:(?:my|the)\s+)?[\w'.-]+(?:\s+(?:team|channel|group|chat))?";
const DET: &str = r"(?:the|my|that|this|these|those|our|his|her)";
/// Words that end a noun phrase.
const STOP: &str = r"(?:to|with|from|on|in|into|at|for|by|and|before|after|via|as|over|using|folder|please|pls|plz|tomorrow|today|yesterday|tonight|tmrw|that|which|i|we)";
const ROLES: &str = r"(?:vendor|client|customer|supplier|contractor|accountant|manager|boss|landlord|lawyer|doctor|teacher|sister|brother|wife|husband|friend|colleague|assistant|secretary|recruiter|roommate|neighbou?r|cousin|uncle|aunt|grandma|grandpa|principal|professor|supervisor|dad|mom|mum|team)";
/// Extra file nouns beyond the first pass list.
const FILE_NOUN2: &str = r"(?:sheet|deck|report|resume|cv|contract|proposal|invoices?|documents?|docs?|pdfs?|photos?|pictures?|pics?|images?|screenshots?|receipts?|letter|ticket|agreement|attachments?|notes|slides|spreadsheet|files?|slip|lease|scan|return|pass|policy|bills?|statements?|forms?|certificates?|presentation|plan|itinerary|manual|brochure|transcript|payslip|licen[sc]e|summary|minutes|draft|ppt|essay|assignment|timetable|budget|memo|mock-?ups?|albums?|backups?|wireframes?|logos?|roadmap|portfolio|templates?|scripts?|recordings?|spreadsheets?|worksheets?|brief|invoice|checklist|manuscript|thesis|paper|ebook|catalog|catalogue|screenshot)";
const NOT_FILE_HEAD: &[&str] = &[
    "it",
    "app",
    "link",
    "message",
    "text",
    "email",
    "mail",
    "location",
    "folder",
    "reminder",
    "event",
    "meeting",
    "call",
    "alarm",
    "timer",
    "music",
    "song",
    "songs",
    "playlist",
    "screen",
    "window",
    "tab",
    "volume",
    "brightness",
    "wifi",
    "bluetooth",
    "one",
    "ones",
    "stuff",
    "number",
    "address",
    "video",
    "videos",
    "camera",
    "mic",
    "microphone",
    "page",
    "website",
    "site",
    "browser",
    "calendar",
    "inbox",
    "news",
    "weather",
    "time",
    "date",
    "day",
    "week",
    "month",
    "year",
    "morning",
    "evening",
    "night",
    "afternoon",
];
const CONTROL: &[&str] = &[
    "music",
    "the music",
    "song",
    "songs",
    "the song",
    "a song",
    "something",
    "anything",
    "it",
    "this",
    "that",
    "next",
    "previous",
    "video",
    "videos",
    "again",
    "something else",
    "my music",
    "some music",
    "the video",
    "the playlist",
    "my focus playlist",
    "the next one",
    "one",
    "the next song",
    "the previous song",
    "a video",
    "me something",
];
const GENERIC_EVENT: &[&str] = &[
    "meeting",
    "call",
    "event",
    "invite",
    "reminder",
    "alarm",
    "timer",
    "appointment",
    "meetings",
    "calls",
    "events",
    "reminders",
    "alarms",
    "timers",
    "appointments",
    "it",
    "this",
    "that",
    "something",
    "me",
    "us",
    "one",
    "everything",
    "all",
    "all my meetings",
];

struct R {
    trail_filler: Regex,
    trail_prep: Regex,
    lead_fill: Regex,
    det_start: Regex,
    q_google: Regex,
    q_lookup: Regex,
    q_search: Regex,
    q_search_on: Regex,
    q_site: Regex,
    q_about: Regex,
    q_online: Regex,
    q_find: Regex,
    q_showme: Regex,
    m_play: Regex,
    m_hear: Regex,
    m_latest: Regex,
    m_from: Regex,
    m_creator: Regex,
    m_something_by: Regex,
    m_new: Regex,
    m_by: Regex,
    tip_on: Regex,
    unknown_site: Regex,
    m_poss: Regex,
    ms_verb_app: Regex,
    ms_drop: Regex,
    ms_reply_to: Regex,
    ms_reply_to_end: Regex,
    ms_know: Regex,
    ms_say_to: Regex,
    ms_saying: Regex,
    rm_to: Regex,
    rm_mid: Regex,
    ev_cancel: Regex,
    ev_no_to: Regex,
    ev_add: Regex,
    ev_block: Regex,
    ev_cal_tail: Regex,
    ev_calendar_ctx: Regex,
    ev_with: Regex,
    ev_bad_start: Regex,
    ev_words_end: Regex,
    ev_bad_head: Regex,
    file_np: Regex,
    file_head_ok: Regex,
    transfer_verb: Regex,
    folder: Regex,
    role: Regex,
    title: Regex,
    names_and: Regex,
    from_name: Regex,
    generic_to: Regex,
    generic_with: Regex,
    send_ctx: Regex,
    with_ctx: Regex,
    place1: Regex,
    place2: Regex,
    app_running: Regex,
    web_verb: Regex,
    every: Regex,
    every_alone: Regex,
    appsite_full: Regex,
    namewords_not: HashSet<&'static str>,
}

static R: LazyLock<R> = LazyLock::new(|| {
    let a = appsite();
    let name_cap = r"(?-i:[A-Z])[\w'-]*";
    let fverb = r"(?:send|share|email|mail|attach|print|forward|move|copy|rename|zip|upload|delete|open|find|show(?:\s+me)?|pull\s+up|where(?:'s|s|\s+is|\s+are)|where\s+did\s+i\s+(?:put|save|leave|store)|get(?:\s+me)?|fetch|look\s+for|bring\s+up|give)";
    R {
        trail_filler: rx(
            r"\s+(?:please|pls|plz|for me|thanks|thank you|online|right now|asap|real quick)$",
        ),
        trail_prep: rx(
            r"\s+(?:on|in|via|using|with|at|by|for|until|till|from|around|before|after|every|each|this|next|last|to|of|the|and)$",
        ),
        lead_fill: rx(r"^(?:me|some|a|an|the|my|up|for|that|to)\s+"),
        det_start: rx(&format!(r"^{DET}\b")),
        q_google: rx(&format!(
            r"{LEAD}(?:google|bing|duckduckgo|ddg)\s+(?:for\s+)?(.+)$"
        )),
        q_lookup: rx(r"\b(?:look\s?up|look\s+for|research|lookup)\s+(?:up\s+)?(.+)$"),
        q_search: rx(&format!(
            r"\bsearch(?:\s+(?:the\s+)?(?:web|internet|net|online)|\s+{a})?\s+for\s+(.+)$"
        )),
        q_search_on: rx(&format!(
            r"{LEAD}search\s+(?!for\b|the\b)(.+?)\s+(?:on|in|using)\s+{a}\s*[.?!]*$"
        )),
        q_site: rx(&format!(
            r"{LEAD}(?:youtube|yt|amazon|flipkart|wikipedia|wiki|reddit)\s+(?!for\b|and\b|app\b)(.+)$"
        )),
        q_about: rx(r"\b(?:emails?|mails?|messages?|texts?)\b.*?\babout\s+(.+)$"),
        q_online: rx(r"\bfind\s+me\s+(.+?)\s+(?:online|on the web|on the internet)\s*[.?!]*$"),
        q_find: rx(
            r"\bfind\s+(?:me\s+)?(?:some\s+)?(?:videos?|articles?|pages?|tutorials?|results?|posts?|blogs?|info(?:rmation)?|recipes?|reviews?|news|images?|pictures?|photos?)\s+(?:about|on|of|for|regarding)\s+(.+)$",
        ),
        q_showme: rx(
            r"\b(?:show|get|find)\s+me\s+(.+?)\s+(?:on|in)\s+(?:youtube|google|amazon|flipkart|wikipedia|reddit)\b",
        ),
        m_play: rx(&format!(
            r"{LEAD}(?:play|put\s+on|throw\s+on|queue(?:\s+up)?|cue\s+up|shuffle|blast|stream|spin)\s+(?:me\s+)?(.+)$"
        )),
        m_hear: rx(
            r"\b(?:wanna|want\s+to|like\s+to|let\s+me|gonna|going\s+to)\s+(?:hear|listen\s+to)\s+(.+)$",
        ),
        m_latest: rx(r"^(?:the\s+)?(?:latest|newest|new|recent)\s+(?:\w+\s+)?(?:from|by)\s+(.+)$"),
        m_from: rx(
            r"\b(?:videos?|uploads?|songs?|tracks?|releases?|episodes?|vids?)\s+(?:from|by)\s+(.+)$",
        ),
        m_creator: rx(&format!(
            r"{LEAD}(?:open|show|play|watch|get|see|pull up)\s+(?:me\s+)?(.+?)'s\s+(?:latest|newest|new|recent)\b"
        )),
        m_something_by: rx(
            r"^(?:something|anything|some\s+music|some\s+songs|songs|music|a\s+song|tracks?|some\s+tracks?)\s+(?:by|from)\s+(.+)$",
        ),
        m_new: rx(r"^(?:the\s+)?(?:new|latest|newest|old|best of)\s+"),
        tip_on: rx(r"\b(?:tip|tax|discount|gst|vat|commission)\s+(?:on|of)\s+(\d[\d,.]*)"),
        unknown_site: rx(r"\s+on\s+((?-i:[A-Z][A-Z0-9]+|[\w-]+\.[a-z]{2,}))\s*[.?!]*$"),
        m_by: rx(r"^(.+?)\s+by\s+(.+)$"),
        m_poss: rx(r"'s\s+(?:songs?|music|playlist|latest|newest|new|hits|tracks?)\b.*$"),
        ms_verb_app: rx(&format!(
            r"{LEAD}(?:whatsapp|telegram|signal|slack|dm)\s+({NAME})(?:\s+(?:on|via|over|in)\s+{a})?\s+(.+?)\s*[.?!]*$"
        )),
        ms_drop: rx(&format!(
            r"\b(?:drop|send|shoot|give)\s+({NAME})(?:\s+(?:on|via|over|in)\s+{a})?\s+(?:a\s+)?(?:text|message|msg|note|dm|line|ping)\b(?:\s*,\s*|\s+(?:saying|that|which\s+says|to\s+say)\s+|\s*:\s*|\s+)(.+?)\s*[.?!]*$"
        )),
        ms_reply_to: rx(&format!(
            r"\breply(?:\s+to\s+({NAME}))?\s+(?:with|saying|that|:)\s*(.+?)\s*[.?!]*$"
        )),
        ms_reply_to_end: rx(&format!(r"\breply\s+(.+?)\s+to\s+({NAME})\s*[.?!]*$")),
        ms_know: rx(&format!(
            r"\blet\s+({NAME})\s+know\s+(?:that\s+)?(.+?)\s*[.?!]*$"
        )),
        ms_say_to: rx(&format!(r"\bsay\s+(.+?)\s+to\s+({NAME})\s*[.?!]*$")),
        ms_saying: rx(&format!(
            r"\b(?:text|message|msg|dm|ping|email|mail|tell)\s+({NAME})\s+(?:saying|that|to say)\s+(.+?)\s*[.?!]*$"
        )),
        rm_to: rx(
            r"\b(?:remind\s+(?:me|us)|ping\s+me|nudge\s+me|set\s+(?:a\s+)?(?:reminder|alarm)(?:\s+for\s+.+?)?)\s+(?:to|about|abt|that)\s+(.+)$",
        ),
        rm_mid: rx(
            r"\b(?:remind\s+(?:me|us)|ping\s+me|nudge\s+me)\s+(.+?)\s+(?:to|about|abt)\s+(.+)$",
        ),
        ev_cancel: rx(
            r"\b(cancel|skip|decline|reject|drop|postpone|reschedule|move|push)\s+(?:the\s+|my\s+|our\s+)?(.+)$",
        ),
        ev_no_to: rx(r"\bsay\s+no\s+to\s+(?:the\s+|my\s+)?(.+)$"),
        ev_add: rx(
            r"\b(book|schedule|add|put|set\s+up|create|plan|arrange)\s+(?:(?:an?|my|the|our)\s+)?(.+)$",
        ),
        ev_block: rx(r"\bblock\b.*?\bfor\s+(.+)$"),
        ev_cal_tail: rx(
            r"\s+(?:in|on|to|onto|into|at|for)\s+(?:my\s+|the\s+)?(?:calendar|schedule)\b.*$",
        ),
        ev_calendar_ctx: rx(r"\b(?:calendar|schedule|appointment|agenda)\b"),
        ev_with: rx(
            r"^(?:lunch|dinner|coffee|chat|catch[- ]?up|sync|1:1|breakfast|session|interview|demo)\s+with\s+\S+",
        ),
        ev_bad_start: rx(
            r"^(?:this|that|these|it|one|me|us|\d(?!:\d)|an?\s+(?:hour|minute|half)|(?:one|two|three|four|five|six|seven|eight|nine|ten|fifteen|twenty|thirty|forty|sixty)\b|half\b)",
        ),
        ev_words_end: rx(
            r"(?:review|standup|stand-up|sync|lunch|dinner|appointment|interview|class|session|demo|retro|workshop|gym|yoga|1:1|planning|offsite|meeting|call|all hands|all-hands|town hall)$",
        ),
        ev_bad_head: rx(
            r"(?:subscription|order|account|flight|booking|ticket|card|alarm|timer|reminder|download|file|folder|email|mail|message|playlist|song|music|app|notification|notifications|process|task|list|todo|note|notes|number|contact|payment|plan|membership|trial)s?\b",
        ),
        file_np: rx(&format!(
            r"\b{fverb}\s+(?:all\s+)?({DET}|a|an|all)\s+((?:[\w'-]+\s+){{0,3}}?[\w'-]+?)(?=\s+{STOP}\b|\s*[.?!,]*$)"
        )),
        file_head_ok: rx(&format!(r"(?:^|\s){FILE_NOUN2}(?:\s+of\s+.+)?$")),
        transfer_verb: rx(
            r"^(?:send|share|email|mail|attach|print|forward|move|copy|rename|zip|upload|delete|give)$",
        ),
        folder: rx(r"(?:\bthe\s+|\bmy\s+|\b(?:open|show|go\s+to|browse)\s+)([\w-]+)\s+folder\b"),
        role: rx(&format!(
            r"\b(?:to|with|from|for|tell|ask|text|email|mail|message|msg|ping|dm|call|cc|notify|invite)\s+(?:my\s+|the\s+|our\s+)?({ROLES})\b"
        )),
        title: rx(r"\b((?:dr|mr|mrs|ms|prof|professor)\.?\s+(?-i:[A-Z])[\w'-]*)"),
        names_and: rx(&format!(
            r"\b(?:to|with|from|cc|for)\s+({name_cap})((?:\s*(?:,|and)\s*{name_cap})+)"
        )),
        from_name: rx(&format!(
            r"\b(?:emails?|mails?|messages?|invoices?|texts?|notes?|replies|reply|files?|attachments?)\s+from\s+({name_cap}(?:\s+{name_cap})?)"
        )),
        generic_to: rx(&format!(
            r"\bto\s+({name_cap}(?:\s+{name_cap})?)\b(?!\s+folder)"
        )),
        generic_with: rx(&format!(
            r"\b(?:with|cc)\s+({name_cap}(?:\s+{name_cap})?)\b(?!\s+folder)"
        )),
        send_ctx: rx(r"\b(?:send|share|forward|email|mail|give|show|text|message|attach)\b"),
        with_ctx: rx(r"\b(?:share|meet|sync|set\s+up|schedule|book|call|chat|talk)\b"),
        place1: rx(&format!(
            r"\b(?:capital|population|weather|time|temperature|currency|flights?|hotels?|forecast|distance|area)\s+(?:of|in|at|to|for)\s+({name_cap}(?:\s+{name_cap})?)"
        )),
        place2: rx(&format!(
            r"\b(?:raining|rain|snowing|hot|cold|humid|sunny|windy|weather|time)\s+in\s+({name_cap}(?:\s+{name_cap})?)"
        )),
        app_running: rx(&format!(
            r"\b(?:get|bring\s+up|boot\s+up|spin\s+up|fire\s+up|start\s+up)\s+({name_cap})\s+(?:running|up|started|going|open)\b"
        )),
        web_verb: rx(
            r"\b(?:go\s+to|visit|take\s+me\s+to|head\s+to|browse\s+to|navigate\s+to)\s+(?:the\s+)?$",
        ),
        every: rx(&format!(
            r"(?:every|each)\s+(?:other\s+)?(?:day|morning|evening|night|afternoon|week|weekday|weekend|month|{WD})s?\s+(?:at\s+)?$"
        )),
        every_alone: rx(&format!(
            r"\b(?:every|each)\s+(?:other\s+)?(?:day|morning|evening|night|afternoon|week|weekday|weekend|month|{WD})s?\b"
        )),
        appsite_full: rx(&format!("^{a}$")),
        namewords_not: [
            "i",
            "me",
            "my",
            "the",
            "a",
            "an",
            "it",
            "this",
            "that",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
            "january",
            "february",
            "march",
            "april",
            "may",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
            "today",
            "tomorrow",
            "tonight",
            "yesterday",
            "all",
            "everyone",
            "please",
            "whatsapp",
            "email",
            "mail",
            "pdf",
            "documents",
            "desktop",
            "downloads",
            "pictures",
            "music",
            "videos",
            "home",
            "work",
            "later",
            "now",
        ]
        .into_iter()
        .collect(),
    }
});

/// An optional byte range `(start, end)` into the request.
type Range = Option<(usize, usize)>;

/// A span proposed by a frame (byte offsets into the request).
#[derive(Debug, Clone)]
struct New {
    kind: &'static str,
    start: usize,
    end: usize,
}

struct Ctx<'a> {
    text: &'a str,
    ents: Vec<Ent>,
    closed: Vec<Ent>,
    /// Kinds whose first-pass spans were already replaced by a new frame.
    replaced: HashSet<&'static str>,
}

fn overlaps(a: &Ent, s: usize, e: usize) -> bool {
    a.start < e && s < a.end
}

impl<'a> Ctx<'a> {
    /// Peel closed-class tails off `[s, e)`: end punctuation, "please", "online", a time
    /// or duration (when `dates`), and "on <app or site>".
    fn trim(&self, s: usize, mut e: usize, dates: bool) -> usize {
        let r = &*R;
        loop {
            let t = &self.text[s..e];
            let t2 = t.trim_end_matches(|c: char| " .?!,;:".contains(c));
            e = s + t2.len();
            if let Some(m) = r.trail_filler.find(&self.text[s..e]).ok().flatten() {
                e = s + m.start();
                continue;
            }
            let closed = self.closed.iter().find(|x| {
                x.end == e
                    && x.start > s
                    && (matches!(x.kind, "app" | "website")
                        || (dates && matches!(x.kind, "date_time" | "duration")))
            });
            if let Some(x) = closed {
                e = x.start;
                loop {
                    let t = &self.text[s..e];
                    let t2 = t.trim_end();
                    e = s + t2.len();
                    match r.trail_prep.find(&self.text[s..e]).ok().flatten() {
                        Some(m) => e = s + m.start(),
                        None => break,
                    }
                }
                continue;
            }
            break;
        }
        e
    }

    /// Drop leading filler ("me", "some", articles) from the start of `[s, e)`.
    fn lead(&self, mut s: usize, e: usize) -> usize {
        let r = &*R;
        while let Some(m) = r.lead_fill.find(&self.text[s..e]).ok().flatten() {
            if m.start() != 0 {
                break;
            }
            s += m.end();
        }
        s
    }

    fn clean(&self, s: usize, e: usize, dates: bool) -> Option<(usize, usize)> {
        let s = self.lead(s, e);
        let e = self.trim(s, e, dates);
        (e > s && !self.text[s..e].trim().is_empty()).then_some((s, e))
    }

    /// Put a new span in, replacing whatever overlaps it. For the open-ended kinds the
    /// first-pass spans of that kind elsewhere in the request go too (once per run).
    fn put(&mut self, n: New) {
        let (s, e) = (n.start, n.end);
        let open = matches!(n.kind, "query" | "media" | "message" | "event");
        if open && self.replaced.insert(n.kind) {
            self.ents.retain(|x| x.kind != n.kind);
        }
        self.ents.retain(|x| !overlaps(x, s, e));
        self.ents.push(Ent::new(n.kind, &self.text[s..e], s));
    }

    /// Like `put`, but only if nothing else is there.
    fn put_if_free(&mut self, n: New) {
        if !self.ents.iter().any(|x| overlaps(x, n.start, n.end)) {
            self.put(n);
        }
    }

    fn has(&self, kind: &str) -> bool {
        self.ents.iter().any(|x| x.kind == kind)
    }
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

fn lower_in(list: &[&str], s: &str) -> bool {
    let l = s.trim().to_lowercase();
    list.contains(&l.as_str())
}

// ---------------------------------------------------------------------------
// dates
// ---------------------------------------------------------------------------

fn fix_dates(c: &mut Ctx) {
    let text = c.text;
    merge_dates(text, &mut c.ents);
    merge_dates(text, &mut c.closed);
}

/// "every day", "every morning at 6am", and "Wednesday" + "at 3" as one date.
fn merge_dates(text: &str, v: &mut Vec<Ent>) {
    let r = &*R;
    // "every day" alone (when nothing else is there)
    let found: Vec<(usize, usize)> = r
        .every_alone
        .find_iter(text)
        .filter_map(Result::ok)
        .map(|m| (m.start(), m.end()))
        .collect();
    for (s, e) in found {
        if !v
            .iter()
            .any(|x| s < x.end && x.start < e && x.kind != "date_time")
        {
            v.retain(|x| !(s <= x.start && x.end <= e && x.kind == "date_time"));
            v.push(Ent::new("date_time", &text[s..e], s));
        }
    }
    // neighbouring dates with only blank or "at" between
    v.sort_by_key(|e| (e.start, e.end));
    let mut i = 0;
    while i + 1 < v.len() {
        let (a, b) = (&v[i], &v[i + 1]);
        if a.kind == "date_time" && b.kind == "date_time" {
            let gap = text[a.end.min(b.start)..b.start].trim().to_lowercase();
            if gap.is_empty() || gap == "at" {
                let merged = Ent::new("date_time", &text[a.start..b.end.max(a.end)], a.start);
                v[i] = merged;
                v.remove(i + 1);
                continue;
            }
        }
        i += 1;
    }
    // "every morning at 6am"
    for k in 0..v.len() {
        if v[k].kind != "date_time" {
            continue;
        }
        let before = &text[..v[k].start];
        if let Some(m) = r.every.find(before).ok().flatten() {
            let ns = m.start();
            let e = v[k].end;
            v[k] = Ent::new("date_time", &text[ns..e], ns);
        }
    }
}

// ---------------------------------------------------------------------------
// frames
// ---------------------------------------------------------------------------

fn query_frames(c: &mut Ctx) {
    let r = &*R;
    let text = c.text;
    let mut hit: Option<(usize, usize, Range)> = None;
    // (start, end, the verb word to drop when it is an app/site name)
    if let Some(m) = caps(&r.q_google, text) {
        let g = m.get(1).expect("g1");
        let before = text[..g.start()].trim_end();
        let before = before.strip_suffix(" for").unwrap_or(before);
        let vs = before.rfind(' ').map_or(0, |i| i + 1);
        hit = Some((g.start(), g.end(), Some((vs, before.len()))));
    }
    if hit.is_none() {
        if let Some(m) = caps(&r.q_search, text).or_else(|| caps(&r.q_search_on, text)) {
            let g = m.get(1).expect("g1");
            hit = Some((g.start(), g.end(), None));
        }
    }
    if hit.is_none() {
        if let Some(m) = caps(&r.q_about, text) {
            let g = m.get(1).expect("g1");
            hit = Some((g.start(), g.end(), None));
        }
    }
    if hit.is_none() {
        if let Some(m) = caps(&r.q_online, text) {
            let g = m.get(1).expect("g1");
            hit = Some((g.start(), g.end(), None));
        }
    }
    if hit.is_none() {
        if let Some(m) = caps(&r.q_find, text) {
            let g = m.get(1).expect("g1");
            hit = Some((g.start(), g.end(), None));
        }
    }
    if hit.is_none() {
        if let Some(m) = caps(&r.q_site, text) {
            let g = m.get(1).expect("g1");
            let verb_end = text[..g.start()].trim_end().len();
            let verb_start = text[..verb_end].rfind(' ').map_or(0, |i| i + 1);
            hit = Some((g.start(), g.end(), Some((verb_start, verb_end))));
        }
    }
    if hit.is_none() {
        if let Some(m) = caps(&r.q_lookup, text) {
            let g = m.get(1).expect("g1");
            // "look for the invoice" is a file search, not a web query
            let rest = &text[g.start()..g.end()];
            if !is(&r.det_start, rest) && !c.has("file") {
                hit = Some((g.start(), g.end(), None));
            }
        }
    }
    if hit.is_none() {
        if let Some(m) = caps(&r.q_showme, text) {
            let g = m.get(1).expect("g1");
            let rest = &text[g.start()..g.end()];
            if !is(&r.det_start, rest) && !is(&r.file_head_ok, rest) && !c.has("file") {
                hit = Some((g.start(), g.end(), None));
            }
        }
    }
    let Some((s, mut e, verb)) = hit else { return };
    // "... on OLX": a site that is not in the list
    let mut site: Option<(usize, usize)> = None;
    if let Some(m) = caps(&r.unknown_site, &text[s..e]) {
        let g = m.get(1).expect("g1");
        site = Some((s + g.start(), s + g.end()));
        e = s + m.get(0).expect("m0").start();
    }
    let Some((s, e)) = c.clean(s, e, true) else {
        return;
    };
    if let Some((vs, ve)) = verb {
        // "google ..." / "youtube ..." used as the verb is not an app or site mention
        let keep = |x: &Ent| !(matches!(x.kind, "app" | "website") && x.start >= vs && x.end <= ve);
        c.ents.retain(keep);
        c.closed.retain(keep);
    }
    c.put(New {
        kind: "query",
        start: s,
        end: e,
    });
    if let Some((ss, se)) = site {
        c.put_if_free(New {
            kind: "website",
            start: ss,
            end: se,
        });
    }
}

fn media_frames(c: &mut Ctx) {
    let r = &*R;
    let text = c.text;
    let lower = text.to_lowercase();
    let mut rest: Option<(usize, usize)> = None;
    if let Some(m) = caps(&r.m_play, text) {
        let g = m.get(1).expect("g1");
        let is_put_on = lower.contains("put on") && !lower.contains("put on the");
        let cal = lower.contains("calendar") || lower.contains("schedule");
        let is_put = m
            .get(0)
            .is_some_and(|x| x.as_str().to_lowercase().contains("put"));
        if !(is_put && (cal || !is_put_on)) {
            rest = Some((g.start(), g.end()));
        }
    }
    if rest.is_none() {
        if let Some(m) = caps(&r.m_hear, text) {
            let g = m.get(1).expect("g1");
            rest = Some((g.start(), g.end()));
        }
    }
    if rest.is_none() {
        if let Some(m) = caps(&r.m_creator, text) {
            let g = m.get(1).expect("g1");
            let name = g.as_str();
            if !lower_in(&["the", "my", "this", "that", "his", "her"], name) && !c.has("file") {
                if let Some((s, e)) = c.clean(g.start(), g.end(), false) {
                    c.put(New {
                        kind: "media",
                        start: s,
                        end: e,
                    });
                }
            }
            return;
        }
    }
    if rest.is_none() {
        if let Some(m) = caps(&r.m_from, text) {
            if is(&rx(r"\b(?:open|show|play|watch|get|see|pull up)\b"), text) {
                let g = m.get(1).expect("g1");
                rest = Some((g.start(), g.end()));
            }
        }
    }
    let Some((s, e)) = rest else { return };
    let (s, e) = (c.lead(s, e), e);
    let e = c.trim(s, e, true);
    if e <= s {
        return;
    }
    let mut seg = &text[s..e];
    let (mut s, mut e) = (s, e);
    if lower_in(CONTROL, seg) || is(&r.appsite_full, seg.trim()) {
        return;
    }
    if let Some(m) = caps(&r.m_latest, seg).or_else(|| caps(&r.m_something_by, seg)) {
        let g = m.get(1).expect("g1");
        s += g.start();
        e = s + g.as_str().len();
        seg = &text[s..e];
    } else if let Some(m) = r.m_new.find(seg).ok().flatten() {
        s += m.end();
        seg = &text[s..e];
    } else if let Some(m) = r.m_poss.find(seg).ok().flatten() {
        e = s + m.start();
        seg = &text[s..e];
    }
    // "<song> songs" / "<artist> music": the genre word is not part of the name
    let seg_l = seg.to_lowercase();
    for tail in [" songs", " music", " tracks", " album", " single"] {
        if seg_l.ends_with(tail) {
            e -= tail.len();
            seg = &text[s..e];
            break;
        }
    }
    if seg.trim().is_empty() || word_count(seg) > 7 {
        return;
    }
    if let Some(m) = caps(&r.m_by, seg) {
        let (a, b) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
        c.put(New {
            kind: "media",
            start: s + a.start(),
            end: s + a.end(),
        });
        c.put(New {
            kind: "media",
            start: s + b.start(),
            end: s + b.end(),
        });
    } else {
        c.put(New {
            kind: "media",
            start: s,
            end: e,
        });
    }
}

fn message_frames(c: &mut Ctx) {
    let r = &*R;
    let text = c.text;
    let name_ok = |s: &str| {
        let l = s.trim().to_lowercase();
        !NOT_NAME.contains(&l.as_str()) && !l.is_empty()
    };
    // (contact range, message range)
    let mut found: Option<(Range, (usize, usize))> = None;
    let rng = |m: &fancy_regex::Match| (m.start(), m.end());
    if let Some(m) = caps(&r.ms_verb_app, text) {
        let (n, b) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
        if name_ok(n.as_str()) {
            found = Some((Some(rng(&n)), rng(&b)));
            // "whatsapp dad ...": the app name is the verb here, not a mention
            let before = text[..n.start()].trim_end();
            let vs = before.rfind(' ').map_or(0, |i| i + 1);
            let keep = |x: &Ent| !(x.kind == "app" && x.start == vs && x.end == before.len());
            c.ents.retain(keep);
            c.closed.retain(keep);
        }
    }
    if found.is_none() {
        if let Some(m) = caps(&r.ms_drop, text) {
            let (n, b) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
            if name_ok(n.as_str()) {
                found = Some((Some(rng(&n)), rng(&b)));
            }
        }
    }
    if found.is_none() {
        if let Some(m) = caps(&r.ms_reply_to, text) {
            let b = m.get(2).expect("g2");
            found = Some((m.get(1).map(|n| rng(&n)), rng(&b)));
        } else if let Some(m) = caps(&r.ms_reply_to_end, text) {
            let (b, n) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
            found = Some((Some(rng(&n)), rng(&b)));
        }
    }
    if found.is_none() {
        if let Some(m) = caps(&r.ms_know, text) {
            let (n, b) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
            if name_ok(n.as_str()) {
                found = Some((Some(rng(&n)), rng(&b)));
            }
        }
    }
    if found.is_none() {
        if let Some(m) = caps(&r.ms_say_to, text) {
            let (b, n) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
            if name_ok(n.as_str()) {
                found = Some((Some(rng(&n)), rng(&b)));
            }
        }
    }
    if found.is_none() {
        if let Some(m) = caps(&r.ms_saying, text) {
            let (n, b) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
            if name_ok(n.as_str()) {
                found = Some((Some(rng(&n)), rng(&b)));
            }
        }
    }
    if let Some((contact, (bs, be))) = found {
        let be = text[bs..be]
            .trim_end_matches(|ch: char| " .?!".contains(ch))
            .len()
            + bs;
        if be > bs {
            c.put(New {
                kind: "message",
                start: bs,
                end: be,
            });
            if let Some((cs, ce)) = contact {
                let cs = c.lead_name(cs, ce);
                c.put_if_free(New {
                    kind: "contact",
                    start: cs,
                    end: ce,
                });
            }
        }
        return;
    }
    // reminders: "remind me to X", "remind me tomorrow to X", "ping me to X"
    let mut rest: Option<(usize, usize)> = None;
    if let Some(m) = caps(&r.rm_to, text) {
        let g = m.get(1).expect("g1");
        rest = Some((g.start(), g.end()));
    } else if let Some(m) = caps(&r.rm_mid, text) {
        let (mid, g) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
        let covered = c.ents.iter().any(|x| {
            matches!(x.kind, "date_time" | "duration")
                && x.start >= mid.start()
                && x.end <= mid.end()
        });
        if covered && word_count(mid.as_str()) <= 4 {
            rest = Some((g.start(), g.end()));
        }
    }
    if let Some((s, e)) = rest {
        if let Some((s, e)) = c.clean(s, e, true) {
            c.put(New {
                kind: "message",
                start: s,
                end: e,
            });
        }
    }
}

impl Ctx<'_> {
    /// A contact range without a leading "my" / "the" is not needed (the scorer ignores
    /// them), so this only trims blanks.
    fn lead_name(&self, s: usize, e: usize) -> usize {
        s + (self.text[s..e].len() - self.text[s..e].trim_start().len())
    }
}

fn event_frames(c: &mut Ctx) {
    let r = &*R;
    let text = c.text;
    let mut cand: Option<(usize, usize)> = None;
    let mut verb = String::new();
    if let Some(m) = caps(&r.ev_no_to, text) {
        let g = m.get(1).expect("g1");
        cand = Some((g.start(), g.end()));
        verb = "cancel".into();
    }
    if cand.is_none() {
        if let Some(m) = caps(&r.ev_cancel, text) {
            let (v, g) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
            verb = v.as_str().to_lowercase();
            cand = Some((g.start(), g.end()));
        }
    }
    if cand.is_none() {
        if let Some(m) = caps(&r.ev_add, text) {
            let (v, g) = (m.get(1).expect("g1"), m.get(2).expect("g2"));
            verb = v.as_str().to_lowercase();
            cand = Some((g.start(), g.end()));
        }
    }
    let block = cand.is_none();
    if block {
        if let Some(m) = caps(&r.ev_block, text) {
            let g = m.get(1).expect("g1");
            cand = Some((g.start(), g.end()));
            verb = "block".into();
        }
    }
    let Some((mut s, mut e)) = cand else { return };
    // cut at a calendar tail or at the first date after the start
    if let Some(m) = r.ev_cal_tail.find(&text[s..e]).ok().flatten() {
        e = s + m.start();
    }
    // leading dates ("the Friday review")
    loop {
        let lead = c
            .closed
            .iter()
            .find(|x| x.kind == "date_time" && x.start == s && x.end < e)
            .map(|x| x.end);
        match lead {
            Some(end) => {
                s = end;
                while text[s..e].starts_with(char::is_whitespace) {
                    s += 1;
                }
            }
            None => break,
        }
    }
    let first_date = c
        .closed
        .iter()
        .filter(|x| matches!(x.kind, "date_time") && x.start > s && x.start < e)
        .map(|x| x.start)
        .min();
    if let Some(d) = first_date {
        e = d;
    }
    let tail_dur: Vec<usize> = c
        .closed
        .iter()
        .filter(|x| x.kind == "duration" && x.start > s && x.start < e && x.end == e)
        .map(|x| x.start)
        .collect();
    if let Some(d) = tail_dur.first() {
        e = *d;
    }
    // trailing preps/app
    let Some((s, mut e)) = c.clean(s, e, false) else {
        return;
    };
    while let Some(m) = r.trail_prep.find(&text[s..e]).ok().flatten() {
        e = s + m.start();
    }
    if e <= s {
        return;
    }
    let x = &text[s..e];
    let xl = x.to_lowercase();
    if word_count(x) > 6 || is(&r.ev_bad_start, &xl) {
        return;
    }
    let with_person = is(&r.ev_with, &xl);
    if lower_in(GENERIC_EVENT, x) {
        return;
    }
    if xl.starts_with("meeting ") || xl.starts_with("call ") {
        // "meeting notes" is a file; "call with Imran" is a person and a time, not an event
        return;
    }
    if is(&r.ev_bad_head, &xl) && !is(&r.ev_words_end, &xl) {
        return;
    }
    if c.ents
        .iter()
        .any(|x| overlaps(x, s, e) && matches!(x.kind, "app" | "website" | "setting"))
    {
        return;
    }
    let after_date = c
        .closed
        .iter()
        .any(|x| matches!(x.kind, "date_time") && x.start >= e);
    let cal = is(&r.ev_calendar_ctx, text);
    let strong_verb = matches!(
        verb.as_str(),
        "book" | "schedule" | "cancel" | "skip" | "decline" | "reject"
    ) || block;
    let ok = match verb.as_str() {
        "reschedule" | "postpone" => !is(&r.ev_bad_start, &xl) && word_count(x) <= 4,
        "move" | "push" => is(&r.ev_words_end, &xl) || with_person,
        "add" | "put" | "set up" | "create" | "plan" | "arrange" => {
            (cal || after_date || is(&r.ev_words_end, &xl)) && !is(&rx(r"^on\b"), &xl)
        }
        "drop" => false,
        _ => strong_verb && (cal || after_date || is(&r.ev_words_end, &xl) || word_count(x) <= 3),
    };
    if !ok {
        return;
    }
    // an event with a person keeps the person inside it
    c.put(New {
        kind: "event",
        start: s,
        end: e,
    });
}

fn file_frames(c: &mut Ctx) {
    let r = &*R;
    let text = c.text;
    let Some(m) = caps(&r.file_np, text) else {
        return;
    };
    let verb_end = m.get(1).map_or(0, |g| g.start());
    let verb = text[m.get(0).expect("m0").start()..verb_end]
        .trim()
        .to_lowercase();
    let first = verb.split_whitespace().next().unwrap_or("");
    let g = m.get(2).expect("np");
    // "the thing I downloaded": a short relative clause belongs to the file phrase
    let rel = rx(
        r"^\s+(?:i|we|you|he|she|they)\s+(?:just\s+|recently\s+)?(?:downloaded|got|saved|received|sent|made|wrote|created|edited|opened|printed|scanned|took|shared)\b",
    );
    let rel_end = rel
        .find(&text[g.end()..])
        .ok()
        .flatten()
        .map(|x| g.end() + x.end());
    let g_end = rel_end.unwrap_or_else(|| g.end());
    let np = &text[g.start()..g_end];
    let np_base = g.as_str();
    let head_np = np_base.split(" of ").next().unwrap_or(np_base);
    let head = head_np
        .split_whitespace()
        .last()
        .unwrap_or("")
        .to_lowercase();
    if head.is_empty() || NOT_FILE_HEAD.contains(&head.as_str()) {
        return;
    }
    // an app, site, setting, person or media name is not a file
    if is(&r.appsite_full, np.trim())
        || SETTINGS.iter().any(|s| s.eq_ignore_ascii_case(np.trim()))
        || FAMILY.iter().any(|s| s.eq_ignore_ascii_case(np.trim()))
    {
        return;
    }
    let transfer = is(&r.transfer_verb, first);
    let noun = is(&r.file_head_ok, np_base) || rel_end.is_some();
    if !(noun || transfer) {
        return;
    }
    // "the Invoices folder" is a folder
    if text[g_end..]
        .trim_start()
        .to_lowercase()
        .starts_with("folder")
    {
        return;
    }
    // a date or app inside the phrase (possessive dates are handled by the first pass)
    if c.closed
        .iter()
        .any(|x| overlaps(x, g.start(), g_end) && matches!(x.kind, "date_time" | "app" | "website"))
    {
        return;
    }
    if word_count(np) > 6 {
        return;
    }
    // the phrase sits inside a message or query ("remind me to send the invoice")
    if c.ents
        .iter()
        .any(|x| matches!(x.kind, "message" | "query") && x.start <= g.start() && g_end <= x.end)
    {
        return;
    }
    c.put(New {
        kind: "file",
        start: g.start(),
        end: g_end,
    });
}

fn contact_fixes(c: &mut Ctx) {
    let r = &*R;
    let text = c.text;
    // first-pass contacts that start with a verb or preposition ("add team")
    let bad_first = [
        "add", "in", "to", "for", "from", "of", "with", "on", "at", "by", "tell", "ask", "send",
        "get", "make", "set", "start", "open", "a", "an", "our", "your",
    ];
    let mut fixed: Vec<Ent> = Vec::new();
    for x in std::mem::take(&mut c.ents) {
        if x.kind == "contact" {
            let t = x.text.as_str();
            if let Some((first, rest)) = t.split_once(' ') {
                if bad_first.contains(&first.to_lowercase().as_str()) {
                    if first.eq_ignore_ascii_case("add") || first.eq_ignore_ascii_case("in") {
                        // "add team offsite": not a person at all
                        if !rest.to_lowercase().starts_with("team ") && word_count(rest) <= 1 {
                            continue;
                        }
                    }
                    let ns = x.start + first.len() + 1;
                    fixed.push(Ent::new("contact", rest, ns));
                    continue;
                }
            }
        }
        fixed.push(x);
    }
    // a name that runs into a date ("Priya Friday") stops before the date
    for x in &mut fixed {
        if x.kind == "contact" {
            if let Some(d) = c
                .closed
                .iter()
                .find(|d| d.kind == "date_time" && d.start > x.start && d.start < x.end)
            {
                let cut = text[x.start..d.start].trim_end().len();
                *x = Ent::new("contact", &text[x.start..x.start + cut], x.start);
            }
        }
    }
    c.ents = fixed;

    let mut add: Vec<New> = Vec::new();
    for m in r.role.captures_iter(text).filter_map(Result::ok) {
        let g = m.get(1).expect("g1");
        add.push(New {
            kind: "contact",
            start: g.start(),
            end: g.end(),
        });
    }
    for m in r.title.captures_iter(text).filter_map(Result::ok) {
        let g = m.get(1).expect("g1");
        add.push(New {
            kind: "contact",
            start: g.start(),
            end: g.end(),
        });
    }
    let nm = |s: &str| !R.namewords_not.contains(s.to_lowercase().as_str());
    if let Some(m) = caps(&r.names_and, text) {
        let g1 = m.get(1).expect("g1");
        let g2 = m.get(2).expect("g2");
        if nm(g1.as_str()) {
            add.push(New {
                kind: "contact",
                start: g1.start(),
                end: g1.end(),
            });
        }
        let name = rx_cs_word();
        for w in name.find_iter(g2.as_str()).filter_map(Result::ok) {
            if nm(w.as_str()) {
                add.push(New {
                    kind: "contact",
                    start: g2.start() + w.start(),
                    end: g2.start() + w.end(),
                });
            }
        }
    }
    if let Some(m) = caps(&r.from_name, text) {
        let g = m.get(1).expect("g1");
        if nm(g.as_str()) {
            // a sender: wins over an app or site of the same name
            c.ents.retain(|x| {
                !(matches!(x.kind, "app" | "website") && overlaps(x, g.start(), g.end()))
            });
            add.push(New {
                kind: "contact",
                start: g.start(),
                end: g.end(),
            });
        }
    }
    if is(&r.send_ctx, text) {
        if let Some(m) = caps(&r.generic_to, text) {
            let g = m.get(1).expect("g1");
            if nm(g.as_str()) && !is(&r.appsite_full, g.as_str()) {
                add.push(New {
                    kind: "contact",
                    start: g.start(),
                    end: g.end(),
                });
            }
        }
    }
    if is(&r.with_ctx, text) {
        if let Some(m) = caps(&r.generic_with, text) {
            let g = m.get(1).expect("g1");
            if nm(g.as_str()) && !is(&r.appsite_full, g.as_str()) {
                add.push(New {
                    kind: "contact",
                    start: g.start(),
                    end: g.end(),
                });
            }
        }
    }
    for mut n in add {
        // a name does not run on into a date word ("Priya Friday")
        while let Some(i) = c.text[n.start..n.end].rfind(' ') {
            let last = &c.text[n.start + i + 1..n.end];
            if R.namewords_not.contains(last.to_lowercase().as_str()) {
                n.end = n.start + i;
            } else {
                break;
            }
        }
        if c.ents
            .iter()
            .any(|x| x.kind == "contact" && overlaps(x, n.start, n.end))
        {
            continue;
        }
        // not inside, or crossing, an open-ended span the frames already claimed
        if c.ents.iter().any(|x| {
            matches!(x.kind, "message" | "query" | "media" | "event" | "file")
                && overlaps(x, n.start, n.end)
        }) {
            continue;
        }
        c.ents.retain(|x| {
            !(overlaps(x, n.start, n.end)
                && x.kind != "contact"
                && matches!(
                    x.kind,
                    "place" | "media" | "event" | "app" | "website" | "file"
                ))
        });
        c.ents
            .push(Ent::new("contact", &c.text[n.start..n.end], n.start));
    }
}

fn rx_cs_word() -> &'static Regex {
    static W: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[A-Z][\w'-]*").expect("word pattern"));
    &W
}

fn misc_fixes(c: &mut Ctx) {
    let r = &*R;
    let text = c.text;
    // places
    for re in [&r.place1, &r.place2] {
        if let Some(m) = caps(re, text) {
            let g = m.get(1).expect("g1");
            if !R.namewords_not.contains(g.as_str().to_lowercase().as_str()) {
                c.put_if_free(New {
                    kind: "place",
                    start: g.start(),
                    end: g.end(),
                });
            }
        }
    }
    // "Desktop folder", "open Downloads folder"
    for m in r.folder.captures_iter(text).filter_map(Result::ok) {
        let g = m.get(1).expect("g1");
        let l = g.as_str().to_lowercase();
        if [
            "a", "this", "that", "new", "same", "other", "shared", "the", "my", "empty", "whole",
            "entire", "to", "in",
        ]
        .contains(&l.as_str())
        {
            continue;
        }
        if c.ents
            .iter()
            .any(|x| x.kind == "folder" && overlaps(x, g.start(), g.end()))
        {
            continue;
        }
        c.ents.retain(|x| {
            !(overlaps(x, g.start(), g.end() + 7)
                && matches!(x.kind, "app" | "file" | "website")
                && x.start <= g.end() + 7)
        });
        c.ents.push(Ent::new("folder", g.as_str(), g.start()));
    }
    // "15% tip on 80"
    if let Some(m) = caps(&r.tip_on, text) {
        let g = m.get(1).expect("g1");
        c.put_if_free(New {
            kind: "number",
            start: g.start(),
            end: g.end(),
        });
    }
    // "get Obsidian running"
    if let Some(m) = caps(&r.app_running, text) {
        let g = m.get(1).expect("g1");
        if !R.namewords_not.contains(g.as_str().to_lowercase().as_str()) {
            c.put_if_free(New {
                kind: "app",
                start: g.start(),
                end: g.end(),
            });
        }
    }
    // an unknown name after "go to / visit / take me to" is a site
    for x in &mut c.ents {
        if x.kind == "app"
            && !APPS.iter().any(|a| a.eq_ignore_ascii_case(&x.text))
            && is(&r.web_verb, &text[..x.start])
        {
            x.kind = "website";
        }
    }
}

/// Run the second rule pass over the first-pass spans `base`. `raw` is the model's spans
/// (unused for now: the model only fills what the rules leave empty, which the first
/// pass already did).
pub fn refine(text: &str, base: Vec<Ent>, _raw: &[ModelEnt], gaz: &Gazetteer) -> Vec<Ent> {
    let mut c = Ctx {
        text,
        ents: base,
        closed: closed_spans(text, gaz),
        replaced: HashSet::new(),
    };
    c.closed.sort_by_key(|e| (e.start, e.end));
    fix_dates(&mut c);
    query_frames(&mut c);
    media_frames(&mut c);
    message_frames(&mut c);
    file_frames(&mut c);
    event_frames(&mut c);
    contact_fixes(&mut c);
    misc_fixes(&mut c);

    // put back the dates, durations, apps and sites a wrong first-pass frame swallowed
    // (for instance "no to the all hands on Wednesday" read as a message)
    for x in c.closed.clone() {
        if matches!(x.kind, "date_time" | "duration" | "app" | "website")
            && !c.ents.iter().any(|o| overlaps(o, x.start, x.end))
        {
            c.ents.push(x);
        }
    }
    // nothing inside a query or message is an entity of its own
    let outer: Vec<(usize, usize)> = c
        .ents
        .iter()
        .filter(|x| matches!(x.kind, "query" | "message"))
        .map(|x| (x.start, x.end))
        .collect();
    c.ents.retain(|x| {
        matches!(x.kind, "query" | "message")
            || !outer.iter().any(|&(s, e)| s <= x.start && x.end <= e)
    });
    // a person who is part of an event title stays inside the event
    let events: Vec<(usize, usize)> = c
        .ents
        .iter()
        .filter(|x| x.kind == "event")
        .map(|x| (x.start, x.end))
        .collect();
    c.ents.retain(|x| {
        x.kind == "event"
            || !(matches!(x.kind, "contact" | "place" | "media" | "file")
                && events.iter().any(|&(s, e)| s <= x.start && x.end <= e))
    });
    let mut seen: HashSet<(&'static str, usize, usize)> = HashSet::new();
    let mut out: Vec<Ent> = c
        .ents
        .into_iter()
        .filter(|x| seen.insert((x.kind, x.start, x.end)))
        .collect();
    out.sort_by_key(|e| (e.start, e.end));
    out
}

#[cfg(test)]
mod tests {
    use crate::rules::{extract_rules, norm, Gazetteer};

    fn has(text: &str, kind: &str, span: &str) -> bool {
        let r = extract_rules(text, &Gazetteer::default());
        r.iter()
            .any(|e| e.kind == kind && norm(&e.text) == norm(span))
    }

    fn count(text: &str, kind: &str) -> usize {
        extract_rules(text, &Gazetteer::default())
            .iter()
            .filter(|e| e.kind == kind)
            .count()
    }

    #[test]
    fn query_frames_trim_the_tail() {
        assert!(has(
            "google best laptops under 50000",
            "query",
            "best laptops under 50000"
        ));
        assert_eq!(count("google best laptops under 50000", "website"), 0);
        assert!(has(
            "look up how to change a flat tire",
            "query",
            "how to change a flat tire"
        ));
        assert!(has(
            "search for used cars in Pune on OLX",
            "query",
            "used cars in Pune"
        ));
        assert!(has("search for used cars in Pune on OLX", "website", "OLX"));
        // a file search is not a web query
        assert!(!has("look for the invoice", "query", "the invoice"));
    }

    #[test]
    fn media_frames() {
        assert!(has("queue up Kesariya", "media", "Kesariya"));
        assert!(has(
            "play Blinding Lights by The Weeknd",
            "media",
            "Blinding Lights"
        ));
        assert!(has(
            "play Blinding Lights by The Weeknd",
            "media",
            "The Weeknd"
        ));
        assert!(has("play something by Coldplay", "media", "Coldplay"));
        assert_eq!(count("pause the music", "media"), 0);
    }

    #[test]
    fn message_and_reminder_frames() {
        assert!(has(
            "reply to Kabir with sounds good, see you then",
            "message",
            "sounds good, see you then"
        ));
        assert!(has(
            "let Mehul know the meeting is cancelled",
            "message",
            "the meeting is cancelled"
        ));
        assert!(has("whatsapp dad i'll be home by nine", "contact", "dad"));
        assert_eq!(count("whatsapp dad i'll be home by nine", "app"), 0);
        assert!(has(
            "remind me tomorrow morning to send the invoice",
            "message",
            "send the invoice"
        ));
        assert!(has(
            "remind me tomorrow morning to send the invoice",
            "date_time",
            "tomorrow morning"
        ));
        assert_eq!(
            count("remind me tomorrow morning to send the invoice", "file"),
            0
        );
        assert!(has(
            "remind me to stretch every two hours",
            "message",
            "stretch"
        ));
        assert!(has(
            "remind me to stretch every two hours",
            "duration",
            "two hours"
        ));
    }

    #[test]
    fn events_files_and_dates() {
        assert!(has("cancel yoga tomorrow", "event", "yoga"));
        assert!(has(
            "decline the quarterly planning on Tuesday",
            "event",
            "quarterly planning"
        ));
        assert!(has(
            "say no to the all hands on Wednesday",
            "event",
            "all hands"
        ));
        assert!(has(
            "say no to the all hands on Wednesday",
            "date_time",
            "Wednesday"
        ));
        // "call with X" is a person and a time, not an event
        assert!(!has(
            "set up a call with Imran on Tuesday at 3pm",
            "event",
            "call with Imran"
        ));
        assert!(has(
            "send the design mockup to Aarav",
            "file",
            "design mockup"
        ));
        assert!(has(
            "show me the photo of the sunset",
            "file",
            "photo of the sunset"
        ));
        assert!(has(
            "am i free Wednesday at 3",
            "date_time",
            "Wednesday at 3"
        ));
        assert!(has(
            "add gym session every morning at 6am",
            "date_time",
            "every morning at 6am"
        ));
        assert!(has(
            "add gym session every morning at 6am",
            "event",
            "gym session"
        ));
    }

    #[test]
    fn small_repairs() {
        assert!(has("open Desktop folder", "folder", "Desktop"));
        assert!(has("take me to Goodreads", "website", "Goodreads"));
        assert!(has("get Obsidian running", "app", "Obsidian"));
        assert!(has("what's the population of Brazil", "place", "Brazil"));
        assert!(has(
            "find emails from Amazon about my refund",
            "contact",
            "Amazon"
        ));
        assert!(has(
            "share the pitch deck with Kunal and Rhea",
            "contact",
            "Rhea"
        ));
    }

    #[test]
    fn untouched_where_no_frame_applies() {
        // sentences the first pass already handles keep their result
        assert!(has(
            "Tell Ved on WhatsApp I'm on my way.",
            "message",
            "I'm on my way"
        ));
        assert!(has("set the volume to 30", "number", "30"));
        assert!(has("open Spotify", "app", "Spotify"));
    }
}
