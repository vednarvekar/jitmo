//! Tidy a typed request before it is routed: drop greetings and fillers, and repair a
//! mistyped or glued leading verb ("plya", "pley", "opne", "openspotify").
//!
//! The audit found that one letter wrong in the first word was enough to turn
//! "plya arijit singh" into a Spotify search for "plya arijit singh". This is the
//! cheap, predictable fix: only the first word and the edges of the sentence are
//! touched, only for a short list of verbs, and only for the kinds of slip typing
//! really produces (a swapped pair, a missing or extra letter, a wrong vowel).
//! Everything else reaches the router exactly as typed.

/// Words that start a sentence without asking for anything.
const LEADING: &[&str] = &[
    "hey", "hi", "hello", "yo", "uh", "um", "umm", "ok", "okay", "so", "jitmo", "please", "pls",
    "plz",
];

/// Words that end a sentence without asking for anything.
const TRAILING: &[&str] = &["please", "pls", "plz", "thanks", "thx"];

/// Verbs whose spelling is repaired when it is the first word.
const VERBS: &[&str] = &[
    "play", "open", "pause", "resume", "mute", "unmute", "search", "google", "remind", "lock",
    "find", "show", "send", "launch", "start", "stop", "cancel", "move", "turn", "set",
];

/// Verbs that may be glued to what follows: "openspotify".
const GLUE_VERBS: &[&str] = &["open", "launch", "start"];

/// Real words that look like a glued verb or a mistyped one.
const NOT_TYPOS: &[&str] = &[
    "plan", "plant", "plane", "clay", "pray", "slay", "stat", "shot", "fund", "mine", "mind",
    "sent", "send", "lick", "lock", "look", "luck", "fine", "find", "fond", "show", "shop",
    "shows", "stop", "step", "star", "stay", "move", "mode", "more", "moved", "moves", "turn",
    "torn", "tune", "sat", "set", "sit", "pose", "poet", "mute", "mate", "mere", "restart",
    "opens", "opened", "opener", "opening", "starts", "started", "launcher", "startup", "block",
    "blocks", "blocked",
];

fn is_adjacent_swap(a: &[char], b: &[char]) -> bool {
    a.len() == b.len()
        && (0..a.len().saturating_sub(1)).any(|i| {
            a[i] == b[i + 1] && a[i + 1] == b[i] && a[..i] == b[..i] && a[i + 2..] == b[i + 2..]
        })
}

/// `typed` is one dropped or added letter away from `verb`.
fn is_one_letter_off(typed: &[char], verb: &[char]) -> bool {
    let (short, long) = if typed.len() < verb.len() {
        (typed, verb)
    } else {
        (verb, typed)
    };
    if long.len() != short.len() + 1 {
        return false;
    }
    (0..long.len()).any(|skip| {
        long.iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .map(|(_, c)| *c)
            .eq(short.iter().copied())
    })
}

/// A vowel swapped for another vowel ("pley" for "play"); the first letter must match.
fn is_vowel_slip(typed: &[char], verb: &[char]) -> bool {
    typed.len() == verb.len()
        && typed.len() >= 4
        && typed[0] == verb[0]
        && typed.iter().zip(verb).filter(|(a, b)| a != b).count() == 1
        && typed
            .iter()
            .zip(verb)
            .filter(|(a, b)| a != b)
            .all(|(a, b)| "aeiou".contains(*a) && "aeiou".contains(*b))
}

/// The verb the first word was meant to be, if it is a plausible slip of one.
fn repair_verb(word: &str) -> Option<&'static str> {
    let w = word.to_lowercase();
    if w.len() < 4 || VERBS.contains(&w.as_str()) || NOT_TYPOS.contains(&w.as_str()) {
        return None;
    }
    let typed: Vec<char> = w.chars().collect();
    let mut found = None;
    for verb in VERBS {
        let v: Vec<char> = verb.chars().collect();
        if is_adjacent_swap(&typed, &v)
            || is_one_letter_off(&typed, &v)
            || is_vowel_slip(&typed, &v)
        {
            // Two different verbs fit: do not guess.
            if found.is_some() {
                return None;
            }
            found = Some(*verb);
        }
    }
    found
}

/// "openspotify" -> ("open", "spotify").
fn split_glued(word: &str) -> Option<(&'static str, String)> {
    let w = word.to_lowercase();
    if NOT_TYPOS.contains(&w.as_str()) {
        return None;
    }
    GLUE_VERBS.iter().find_map(|verb| {
        let rest = w.strip_prefix(verb)?;
        (rest.chars().count() >= 3 && rest.chars().all(|c| c.is_alphanumeric()))
            .then(|| (*verb, word[verb.len()..].to_string()))
    })
}

/// Only digits, operators, brackets, commas and points; at least one number and one
/// operator. A minus counts only with spaces around it ("2 - 1"), so "555-1234" and a
/// date such as "10/11/2026" are not sums.
pub fn is_bare_expression(s: &str) -> bool {
    let allowed = |c: char| c.is_ascii_digit() || " .,+*/x^()%-".contains(c);
    if s.is_empty() || !s.chars().all(allowed) || !s.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if s.matches('/').count() >= 2 {
        return false;
    }
    let has_op = s.chars().any(|c| "+*/x^".contains(c)) || s.contains(" - ");
    // A lone "x" or percent sign is not an expression: "x" needs digits around it.
    has_op && !s.trim_start().starts_with(['x', '*', '/', '^'])
}

/// "set my city to <script>", "i live in ../../x": a place sentence whose place has code or
/// path characters in it. The extractor would quietly drop them and save "Script"; refuse.
pub fn place_with_symbols(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "set my city",
        "set my location",
        "set my place",
        "i live in",
        "my city is",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
        && text.chars().any(|c| "<>/\\{}$;|&*\"`=".contains(c))
}

/// "2 power 10", "3 to the power of 4", "2 raised to 8".
fn is_power(s: &str) -> bool {
    let w: Vec<String> = s
        .to_lowercase()
        .split_whitespace()
        .map(String::from)
        .collect();
    let num = |x: &str| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit() || c == '.');
    w.len() >= 3
        && num(&w[0])
        && num(&w[w.len() - 1])
        && w[1..w.len() - 1]
            .iter()
            .all(|x| ["power", "to", "the", "of", "raised", "pow"].contains(&x.as_str()))
        && w.iter()
            .any(|x| x == "power" || x == "raised" || x == "pow")
}

/// "18% of 2450", "what is 15 percent of 1,200", "15 percent tip on 80": a percentage of a
/// number. Unmistakably a calculation, so the calculator takes it without a router guess.
pub fn is_percent_of(s: &str) -> bool {
    let lower = s.to_lowercase();
    let w: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == '?')
        .filter(|x| !x.is_empty())
        .collect();
    let is_number = |x: &str| {
        let x = x.trim_matches(|c: char| c == '$' || c == ',');
        !x.is_empty()
            && x.chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    };
    (0..w.len()).any(|i| {
        let (num_ok, mut j) = if let Some(n) = w[i].strip_suffix('%') {
            (is_number(n), i + 1)
        } else if is_number(w[i]) && matches!(w.get(i + 1), Some(&"percent") | Some(&"pct")) {
            (true, i + 2)
        } else {
            (false, i)
        };
        if num_ok && matches!(w.get(j), Some(&"tip")) {
            j += 1;
        }
        num_ok
            && matches!(w.get(j), Some(&"of") | Some(&"on"))
            && w.get(j + 1).is_some_and(|n| is_number(n))
    })
}

/// The request as it should be routed.
pub fn normalize(text: &str) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    // Greetings and fillers at the start (never the whole sentence).
    while words.len() > 1
        && LEADING.contains(
            &words[0]
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
                .as_str(),
        )
    {
        words.remove(0);
    }
    // Politeness at the end.
    loop {
        let strip_two = words.len() > 2
            && words[words.len() - 2].eq_ignore_ascii_case("thank")
            && words[words.len() - 1]
                .trim_matches(|c: char| !c.is_alphanumeric())
                .eq_ignore_ascii_case("you");
        if strip_two {
            words.truncate(words.len() - 2);
            continue;
        }
        let last = words
            .last()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .unwrap_or_default();
        // "draft a reply saying thanks": here "thanks" is the message, not politeness.
        let is_message = words.len() > 1
            && matches!(
                words[words.len() - 2]
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
                    .as_str(),
                "saying" | "say" | "that" | "with" | "is" | "was" | "and" | "to"
            );
        if words.len() > 1 && TRAILING.contains(&last.as_str()) && !is_message {
            words.pop();
        } else {
            break;
        }
    }
    // Arithmetic with no words ("2+2", "1,000 + 500", "(3+4)*2"): say what it is, so it
    // reaches the calculator and not whichever skill the digits happen to resemble.
    let joined = words.join(" ");
    if is_bare_expression(&joined) {
        return format!("calculate {joined}");
    }
    let mut out: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    if let Some(first) = out.first().cloned() {
        if let Some((verb, rest)) = split_glued(&first) {
            out[0] = verb.to_string();
            out.insert(1, rest);
        } else if let Some(verb) = repair_verb(&first) {
            out[0] = verb.to_string();
        }
    }
    // Texting spellings the router was not trained on.
    for (i, w) in out.iter_mut().enumerate() {
        if let Some(full) = slang(w) {
            // Never the first word (that is the verb), except a mistyped "timer" ("tiemr 5 min").
            if i > 0 || ["timer", "next", "skip", "previous"].contains(&full) {
                *w = full.to_string();
            }
        }
    }
    let joined = out.join(" ");
    let joined = stop_music(&joined).unwrap_or(joined);
    // "tell me about who wrote X" (a typed answer to "What would you like to know?").
    let joined = match joined.to_lowercase().strip_prefix("tell me about ") {
        Some(rest)
            if ["who ", "what ", "when ", "where ", "which ", "how ", "why "]
                .iter()
                .any(|q| rest.starts_with(q)) =>
        {
            joined
                .get("tell me about ".len()..)
                .map_or_else(|| joined.clone(), str::to_string)
        }
        _ => joined,
    };
    let joined = file_search(&joined).unwrap_or(joined);
    youtube_search(&joined).unwrap_or(joined)
}

/// Several apps in one sentence: "open zoom and spotify" -> (["zoom", "spotify"], false),
/// "open edge or chrome" -> (["edge", "chrome"], true: the user must pick one).
pub fn several_apps(text: &str) -> Option<(Vec<String>, bool)> {
    let lower = text.trim().trim_end_matches(['.', '!', '?']).to_lowercase();
    let rest = ["open ", "launch ", "start "]
        .iter()
        .find_map(|v| lower.strip_prefix(v))?;
    // Websites, files and searches have their own skills.
    // "open spotify and then pause the music" is a sequence of different requests.
    if [
        "website", "site", "file", "folder", ".com", "http", "search", "timer", "then",
    ]
    .iter()
    .any(|w| rest.contains(w))
    {
        return None;
    }
    let either = rest.contains(" or ");
    let parts: Vec<String> = rest
        .split(',')
        .flat_map(|p| p.split(" and "))
        .flat_map(|p| p.split(" or "))
        .map(|p| {
            p.trim()
                .trim_start_matches("the ")
                .trim_start_matches("my ")
                .trim()
                .to_string()
        })
        .filter(|p| !p.is_empty())
        .collect();
    // "open spotify and play kesariya": the second part is a request, not an app.
    const VERBS: &[&str] = &[
        "play", "pause", "turn", "set", "remind", "search", "mute", "skip", "close", "lock",
        "take", "send", "go",
    ];
    let short = parts.iter().all(|p| {
        p.split_whitespace().count() <= 3
            && !p
                .split_whitespace()
                .next()
                .is_some_and(|w| VERBS.contains(&w))
    });
    (parts.len() >= 2 && parts.len() <= 4 && short).then_some((parts, either))
}

/// "pause the music and then open notepad": two requests run one after the other.
pub fn sequence(text: &str) -> Option<Vec<String>> {
    let lower = text.to_lowercase();
    // A reminder's subject may itself say "and then" ("remind me to eat and then sleep").
    if lower.contains("remind") || lower.contains("search") {
        return None;
    }
    let parts: Vec<String> = lower
        .split(" and then ")
        .flat_map(|p| p.split(", then "))
        .flat_map(|p| p.split(" then "))
        .map(|p| p.trim().trim_end_matches(',').to_string())
        .filter(|p| !p.is_empty())
        .collect();
    // "open spotify and play kesariya": "and" before a new verb also starts a new request.
    const VERBS: &[&str] = &[
        "open", "launch", "play", "pause", "resume", "skip", "turn", "set", "mute", "unmute",
        "lock", "take", "close", "start",
    ];
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        let mut current = String::new();
        for (i, chunk) in part.split(" and ").enumerate() {
            let starts_verb = chunk
                .split_whitespace()
                .next()
                .is_some_and(|w| VERBS.contains(&w));
            if i > 0 && starts_verb {
                out.push(std::mem::take(&mut current));
            } else if i > 0 {
                current.push_str(" and ");
            }
            current.push_str(chunk);
        }
        out.push(current);
    }
    (out.len() >= 2 && out.len() <= 4).then_some(out)
}

/// "kill the music", "shut up the music", "shut the song off": pausing, said bluntly.
fn stop_music(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let blunt = matches!(
        words.first(),
        Some(&"kill") | Some(&"shut") | Some(&"silence")
    );
    let music = words
        .iter()
        .any(|w| ["music", "song", "spotify", "songs"].contains(w));
    (blunt && music).then(|| "pause the music".to_string())
}

/// "play jazz on youtube", "watch fireship on youtube": a YouTube search.
fn youtube_search(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    let rest = lower.strip_suffix(" on youtube")?;
    for verb in ["play", "watch", "search", "find", "show", "open"] {
        if let Some(what) = rest.strip_prefix(&format!("{verb} ")) {
            let what = what.trim();
            if !what.is_empty() {
                return Some(format!("search youtube for {what}"));
            }
        }
    }
    None
}

/// A sentence whose wording leaves no doubt which skill it is, so the router is not asked:
/// a sum, a percentage, "remind me ...", a timer. (The router is trained on the usual
/// phrasings; "every Monday at 10 remind me to ..." and "set a timr for ..." fall outside them.)
pub fn direct_skill(said: &str, normalized: &str) -> Option<&'static str> {
    if is_bare_expression(said.trim()) || is_percent_of(said) || is_power(said) {
        return Some("calc.math");
    }
    let words: Vec<String> = normalized
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    let has = |names: &[&str]| words.iter().any(|w| names.contains(&w.as_str()));
    if let Some(id) = work_skill(&normalized.to_lowercase()) {
        return Some(id);
    }
    // Playing or stopping music is not a timer or a reminder, whatever else the sentence says.
    if has(&["pause", "resume", "play", "skip", "song", "music"]) {
        return None;
    }
    if has(&["timer", "countdown"]) {
        return Some("reminders.timer");
    }
    if has(&["remind", "reminder", "reminders"]) {
        return Some("reminders.remind");
    }
    let lower = normalized.to_lowercase();
    let starts = |p: &[&str]| p.iter().any(|x| lower.starts_with(x));
    // "open the weather app", "turn on dnd at sunset": a command, not a question.
    if starts(&[
        "open ", "launch ", "start ", "turn ", "set ", "close ", "move ", "send ",
    ]) {
        return None;
    }
    if starts(&["what is the time", "what's the time", "whats the time"]) {
        return Some("calc.time_in");
    }
    // A web search said outright ("google X", "search the web for X", "find X online").
    if starts(&[
        "google ",
        "bing ",
        "search the web",
        "search online",
        "search the internet",
    ]) || (starts(&["find ", "search ", "look up "]) && lower.ends_with(" online"))
    {
        return Some("browser.search");
    }
    if has(&[
        "weather",
        "weathr",
        "wether",
        "temperature",
        "forecast",
        "rain",
        "raining",
        "rainy",
        "umbrella",
        "humid",
        "humidity",
    ]) {
        return Some("web.weather");
    }
    if starts(&[
        "time in ",
        "what time is it in ",
        "whats the time in ",
        "what's the time in ",
    ]) || (has(&["time"]) && has(&["tokyo", "london", "york", "dubai", "ist", "pst", "est"]))
    {
        return Some("calc.time_in");
    }
    if has(&[
        "sunrise", "sunset", "sunrize", "sunst", "sunet", "moonrise", "moonset", "moon", "eclipse",
        "dusk", "dawn",
    ]) {
        return Some("calc.moon");
    }
    // A question about the world: who wrote, what is the population of, how tall is ...
    let question = starts(&[
        "who ",
        "whos ",
        "who's ",
        "what is the ",
        "what was the ",
        "what's the ",
        "when was ",
        "when did ",
        "when is ",
        "where is ",
        "where was ",
        "which ",
        "how tall ",
        "how old ",
        "how far ",
        "how big ",
        "how many ",
        "how much does ",
        "how long is ",
        "how deep ",
        "what is a ",
        "what is an ",
        "tell me about ",
        "define ",
    ]);
    let personal = has(&[
        "my", "me", "i", "timer", "timers", "file", "files", "volume",
    ]);
    if question && !personal {
        return Some("web.fact");
    }
    // "search ved", "search for best pizza in pune": on its own that means the web. Only a
    // word about the PC (file, folder, downloads...) keeps it a file search.
    if starts(&["search ", "searching "]) && !has(PC_WORDS) {
        return Some("browser.search");
    }
    None
}

/// Work requests the router tends to confuse with each other (email, calendar, files and
/// chat share many words): the phrasing that settles them, so a send is never taken for a
/// summary and a reply list is never taken for a draft.
fn work_skill(lower: &str) -> Option<&'static str> {
    let has = |s: &str| lower.contains(s);
    let starts = |s: &str| lower.starts_with(s);
    const FILE_EXT: &[&str] = &[".xlsx", ".docx", ".pptx", ".pdf", ".csv", ".png", ".jpg"];
    // A file sent on WhatsApp is not an attachment posted to a chat.
    if has("whatsapp")
        && [
            "pdf", "file", "photo", "picture", "doc", "image", "document",
        ]
        .iter()
        .any(|w| lower.split(|c: char| !c.is_alphanumeric()).any(|x| x == *w))
    {
        return Some("whatsapp.send_file");
    }
    // The Google account: set the client, sign in, sign out.
    let google = has("google") || has("gmail") || has("g suite") || has("workspace");
    if google && (has("client id") || has("client secret")) && (starts("set") || starts("save")) {
        return Some("account.google_client");
    }
    if google && (has("sign out") || has("log out") || has("logout") || has("disconnect")) {
        return Some("account.google_sign_out");
    }
    if google
        && (has("sign in") || has("log in") || has("login") || has("connect"))
        && !has("search")
    {
        return Some("account.google_sign_in");
    }
    // The Microsoft account: set the app id, sign in, sign out.
    let ms = has("microsoft") || has("outlook") || has("office 365") || has("m365");
    if ms && (has("client id") || has("app id")) && (starts("set") || starts("save")) {
        return Some("account.ms365_client");
    }
    if ms && (has("sign out") || has("log out") || has("logout") || has("disconnect")) {
        return Some("account.ms365_sign_out");
    }
    if ms && (has("sign in") || has("log in") || has("login") || has("connect")) {
        return Some("account.ms365_sign_in");
    }
    // The inbox and the calendar in general, said many ways, so by shape.
    let mailish = has("email") || has("mail") || has("inbox");
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    // Verbs that mean something other than reading: never take these for a look at the inbox.
    let other_verb = [
        "send",
        "draft",
        "archive",
        "reply",
        "forward",
        "delete",
        "remove",
        "trash",
        "mark",
        "write",
        "compose",
        "find",
        "search",
        "save",
        "attach",
        "unsubscribe",
    ]
    .iter()
    .any(|v| words.contains(v));
    if mailish && (has("urgent") || has("important")) && words.len() <= 6 && !other_verb {
        return Some("mail.urgent");
    }
    // "summarize my inbox", "summarize today's emails": the inbox, not one email.
    if mailish
        && (has("summarize") || has("summary") || has("sum up") || has("tldr"))
        && (has("inbox") || has("emails") || has("my mail"))
        && !has(" from ")
    {
        return Some("mail.inbox");
    }
    if mailish
        && (has("latest email")
            || has("last email")
            || has("newest email")
            || has("read my latest")
            || has("latest mail")
            || has("last mail"))
        && !has("from ")
        && !other_verb
    {
        return Some("mail.summarize");
    }
    // "search my email for the invoice", "look in my mail for tickets".
    if mailish
        && words.len() >= 3
        && (starts("search")
            || starts("look")
            || starts("find")
            || starts("any mail")
            || starts("anything"))
        && !has("file")
    {
        return Some("mail.find");
    }
    // "what did Priya send me", "mail from Rahul".
    if starts("what did ") && (has(" send ") || has(" sent ") || has(" say ")) && has(" me") {
        return Some("mail.find");
    }
    // The first word says it is a look at the mail ("email" and "mail" only when alone).
    let reads_mail = words.first().is_some_and(|w| {
        [
            "emails", "inbox", "my", "check", "show", "read", "any", "new", "unread", "how", "do",
            "what", "whats", "latest", "recent", "view", "see", "get", "have", "i", "is", "are",
        ]
        .contains(w)
    }) || (words.len() == 1 && words[0] == "mail");
    if mailish
        && reads_mail
        && words.len() <= 7
        && !other_verb
        && !has(" from ")
        && !has(" about ")
        && !has(" with ")
        && !has("replied")
        && !has("attachment")
    {
        return Some("mail.inbox");
    }
    if has("next meeting") || has("next event") || has("next appointment") {
        return Some("calendar.agenda");
    }
    // Mail that has attachments (a look, not saving them).
    if mailish && has("attachment") && !starts("save") && !has("folder") && !starts("send") {
        return Some("mail.find");
    }
    if (starts("open my calendar")
        || starts("open calendar")
        || starts("show my calendar")
        || starts("show calendar"))
        && !has("app")
    {
        return Some("calendar.agenda");
    }
    // Invitations waiting, and answering them.
    if (starts("any invites")
        || starts("pending invites")
        || starts("my invites")
        || starts("new invites")
        || starts("do i have any invites")
        || starts("show my invites")
        || starts("any invitations")
        || starts("pending invitations")
        || starts("check my invites"))
        && !has("send")
    {
        return Some("calendar.agenda");
    }
    if (starts("accept ") || starts("rsvp ") || starts("say yes to "))
        && (has("invite")
            || has("meeting")
            || has("review")
            || has("sync")
            || has("call")
            || has("event"))
    {
        return Some("calendar.accept");
    }
    // Questions about meetings with a person are a look at the calendar, never an invite.
    if starts("do i have a meeting with")
        || starts("do i have meetings with")
        || starts("when am i meeting")
        || starts("when is my meeting with")
        || starts("am i meeting")
        || starts("when do i meet")
    {
        return Some("calendar.agenda");
    }
    if starts("how's my week")
        || starts("hows my week")
        || starts("how is my week")
        || starts("my week")
    {
        return Some("calendar.agenda");
    }
    if has("morning brief") {
        return Some("work.morning_brief");
    }
    if has("meeting request") && (has("calendar") || has("event")) {
        return Some("work.event_from_email");
    }
    if (starts("save ") || starts("put ")) && (has("attachment") || has("email")) {
        return Some("work.save_attachments");
    }
    if (starts("send the attachment")
        || starts("post the attachment")
        || starts("share the attachment"))
        && (has("chat") || has("channel"))
    {
        return Some("work.forward_attachment");
    }
    if has("report") && has("manager") && (starts("send") || starts("email")) {
        return Some("work.send_report");
    }
    if (has("replied to") && (starts("what") || starts("which") || starts("who")))
        || has("unanswered")
        || has("need a reply")
        || has("need to reply")
        || has("waiting for a reply")
    {
        return Some("mail.unanswered");
    }
    if starts("email ") && FILE_EXT.iter().any(|e| has(e)) {
        return Some("files.email");
    }
    if starts("email ") && (has(" that ") || has(" saying ")) {
        return Some("mail.send");
    }
    None
}

/// Words that make "search ..." a search of the PC's files and not of the web.
const PC_WORDS: &[&str] = &[
    "file",
    "files",
    "folder",
    "folders",
    "document",
    "documents",
    "doc",
    "docs",
    "pdf",
    "docx",
    "xlsx",
    "pptx",
    "jpg",
    "png",
    "txt",
    "photo",
    "photos",
    "picture",
    "pictures",
    "image",
    "images",
    "screenshot",
    "screenshots",
    "downloads",
    "desktop",
    "pc",
    "computer",
    "laptop",
    "drive",
    "my",
    "me",
    "recent",
    "latest",
    "newest",
    "invoice",
    "invoices",
    "resume",
    "report",
];

/// "search my files for tax", "search invoice" -> "find tax", "find invoice": the file
/// search says "find".
fn file_search(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '-')
        .filter(|w| !w.is_empty())
        .collect();
    if words.first() != Some(&"search") || !words.iter().any(|w| PC_WORDS.contains(w)) {
        return None;
    }
    let rest: Vec<&str> = words[1..]
        .iter()
        .copied()
        .filter(|w| {
            !matches!(
                *w,
                "for"
                    | "my"
                    | "the"
                    | "all"
                    | "file"
                    | "files"
                    | "me"
                    | "in"
                    | "on"
                    | "downloads"
                    | "desktop"
                    | "folder"
                    | "pc"
                    | "computer"
                    | "laptop"
            )
        })
        .collect();
    (!rest.is_empty()).then(|| format!("find {}", rest.join(" ")))
}

/// The words that will actually be sent: the part after "saying", "that", "about"…
/// Capitalised, with a full stop. `None` when the sentence names no message.
pub fn message_body(request: &str) -> Option<String> {
    let lower = request.to_lowercase();
    for marker in [
        " saying ",
        " say ",
        " that ",
        " telling him ",
        " telling her ",
        " with the message ",
        " message ",
        " about ",
        ": ",
    ] {
        if let Some(i) = lower.find(marker) {
            let rest = request[i + marker.len()..].trim().trim_end_matches('.');
            if !rest.is_empty() {
                let mut c = rest.chars();
                let first = c.next()?.to_uppercase().collect::<String>();
                return Some(format!("{first}{}.", c.as_str()));
            }
        }
    }
    None
}

/// The words to post in a Teams channel: "tell the team channel standup moves to 11" ->
/// "Standup moves to 11."; otherwise the same as [`message_body`].
pub fn post_body(request: &str) -> Option<String> {
    let lower = request.to_lowercase();
    for marker in [" channel ", " chat "] {
        if let Some(i) = lower.find(marker) {
            let rest = request[i + marker.len()..].trim().trim_end_matches('.');
            let rest = rest
                .strip_prefix("that ")
                .or_else(|| rest.strip_prefix("saying "))
                .unwrap_or(rest);
            if !rest.is_empty() {
                let mut c = rest.chars();
                let first = c.next()?.to_uppercase().collect::<String>();
                return Some(format!("{first}{}.", c.as_str()));
            }
        }
    }
    message_body(request)
}

/// Answers that need no skill: greetings, thanks, and things Jitmo cannot do (said plainly
/// instead of guessing at the nearest skill, which once opened Chrome for "close chrome").
pub fn plain_reply(said: &str) -> Option<&'static str> {
    let t: String = said
        .trim()
        .trim_end_matches(['?', '!', '.'])
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let words: Vec<&str> = t.split(' ').collect();
    let first = *words.first()?;
    let only = |allowed: &[&str]| words[1..].iter().all(|w| allowed.contains(w));
    match t.as_str() {
        "hello" | "hi" | "hii" | "hey" | "heya" | "yo" | "sup" | "hola" | "namaste"
        | "hello jitmo" | "hi jitmo" | "hey jitmo" | "good morning" | "good afternoon"
        | "good evening" | "good night" => {
            return Some(
                "Hi! Ask me to open an app, set a timer, check the weather or play music. \
                 Say “help” to see everything.",
            )
        }
        "thanks" | "thank you" | "thank u" | "thanks a lot" | "thx" | "ty" | "thanks jitmo"
        | "shukriya" | "dhanyavad" => return Some("Anytime."),
        "how are you" | "how r u" | "how are you doing" | "whats up" | "what's up" => {
            return Some("All good, thanks. What can I do for you?")
        }
        "who are you" | "what are you" | "what is your name" | "whats your name"
        | "what's your name" => return Some(
            "I'm Jitmo, a small assistant that lives on your PC. Say “help” to see what I can do.",
        ),
        "tell me a joke" | "joke" | "tell a joke" | "say something funny" => {
            return Some(
                "I don't tell jokes yet. I can open apps, set timers, check the weather and more. \
                 Say “help” to see everything.",
            )
        }
        _ => {}
    }
    // Power: shutting down, restarting, sleeping.
    let pc = [
        "the", "my", "pc", "computer", "laptop", "system", "windows", "now", "it", "please",
    ];
    let power = match first {
        "shutdown" | "reboot" | "restart" | "hibernate" | "logout" => only(&pc),
        "shut" => words.get(1) == Some(&"down") && words[2..].iter().all(|w| pc.contains(w)),
        "sleep" => only(&pc),
        "log" | "sign" => words.get(1).is_some_and(|w| *w == "off" || *w == "out"),
        "go" => words.get(1..3) == Some(&["to", "sleep"][..]),
        "turn" | "switch" => {
            words.get(1) == Some(&"off")
                && words.len() > 2
                && words[2..].iter().all(|w| pc.contains(w))
                && words[2..]
                    .iter()
                    .any(|w| ["pc", "computer", "laptop", "system"].contains(w))
        }
        _ => false,
    };
    if power {
        return Some("I can't shut down, restart or sleep the PC yet.");
    }
    // Mail and calendar things that are not supported: say so, never the nearest skill.
    let mail = t.contains("email") || t.contains("mail") || t.contains("inbox");
    if first == "forward" && mail {
        return Some("I can't do that yet. I can send a new email: say “email Rahul that …”.");
    }
    if matches!(first, "delete" | "remove" | "trash") && (mail || t.contains("message")) {
        return Some(
            "I can't do that yet. I can archive today's newsletters, but not delete email.",
        );
    }
    if first == "mark" && t.contains("read") && mail {
        return Some("I can't do that yet. I can't mark email as read.");
    }
    // Teams: only posting is supported; a status or joining a call is not a post.
    if t.contains("teams") && !t.contains("post") {
        if t.contains("status") || t.contains("available") || t.contains("away") {
            return Some("I can't change your Teams status yet. I can post a message in Teams.");
        }
        if first == "join" || t.contains("join ") || t.contains("start a call") {
            return Some("I can't join a Teams meeting yet. I can show you your next meeting.");
        }
    }
    let clock = words.iter().any(|w| {
        w.chars().next().is_some_and(|c| c.is_ascii_digit())
            && (w.ends_with("pm") || w.ends_with("am"))
    });
    if matches!(first, "cancel" | "delete" | "remove")
        && (t.contains("meeting") || t.contains("event") || t.contains("appointment") || clock)
        && !t.contains("timer")
        && !t.contains("remind")
        && !t.contains("alarm")
    {
        return Some("I can't do that yet. I can move or decline a meeting, but not cancel it.");
    }
    // Closing an app: only opening is supported.
    if matches!(first, "close" | "quit" | "exit") && words.len() > 1 {
        let radio_or_other = words
            .iter()
            .any(|w| ["bluetooth", "wifi", "tab", "tabs", "timer", "music", "bar"].contains(w));
        if !radio_or_other {
            return Some("I can't close apps yet. I can open them, though.");
        }
    }
    None
}

/// "tmrw" -> "tomorrow" and the like (whole words only).
fn slang(word: &str) -> Option<&'static str> {
    Some(match word.to_lowercase().as_str() {
        "tmrw" | "tmr" | "tomorow" | "tommorow" | "tomorrw" | "2morrow" | "2moro" => "tomorrow",
        "tonite" | "2nite" => "tonight",
        "abt" => "about",
        "timr" | "tiemr" | "timmer" | "timar" | "tymer" | "timeer" => "timer",
        "mintue" | "mintues" | "minuets" | "minuts" | "minutess" => "minutes",
        "nxt" | "nekst" => "next",
        "skp" => "skip",
        "prv" | "previus" | "prevous" => "previous",
        "sng" | "sonng" => "song",
        "muzic" | "musik" | "musc" => "music",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_weather_time_and_search_route_directly() {
        let d = |s: &str| direct_skill(s, &normalize(s));
        assert_eq!(d("who wrote Pride and Prejudice"), Some("web.fact"));
        assert_eq!(d("what is the population of india"), Some("web.fact"));
        assert_eq!(d("temperature in delhi"), Some("web.weather"));
        assert_eq!(d("weathr in pune"), Some("web.weather"));
        assert_eq!(d("time in tokio"), Some("calc.time_in"));
        assert_eq!(d("sunrize in mumbai"), Some("calc.moon"));
        assert_eq!(d("google pune weather"), Some("browser.search"));
        assert_eq!(
            d("find me a recipe for dosa online"),
            Some("browser.search")
        );
        assert_eq!(d("2 power 10"), Some("calc.math"));
        assert_eq!(d("open the weather app"), None);
        assert_eq!(d("who is my favourite"), None);
        assert_eq!(d("play rain sounds"), None);
        assert_eq!(
            d("remind me to check the weather"),
            Some("reminders.remind")
        );
    }

    #[test]
    fn music_typos_apps_and_sequences() {
        assert_eq!(normalize("nxt song"), "next song");
        assert_eq!(normalize("skp"), "skip");
        assert_eq!(normalize("kill the music"), "pause the music");
        assert_eq!(normalize("shut up the music"), "pause the music");
        assert_eq!(
            several_apps("open zoom and spotify"),
            Some((vec!["zoom".into(), "spotify".into()], false))
        );
        assert_eq!(
            several_apps("open edge or chrome"),
            Some((vec!["edge".into(), "chrome".into()], true))
        );
        assert_eq!(
            several_apps("open notepad, calculator and paint"),
            Some((
                vec!["notepad".into(), "calculator".into(), "paint".into()],
                false
            ))
        );
        assert_eq!(several_apps("open spotify"), None);
        assert_eq!(several_apps("open the website amazon and flipkart"), None);
        assert_eq!(
            sequence("pause the music and then open notepad"),
            Some(vec!["pause the music".into(), "open notepad".into()])
        );
        assert_eq!(sequence("remind me to eat and then sleep"), None);
        assert_eq!(
            sequence("open spotify and play kesariya"),
            Some(vec!["open spotify".into(), "play kesariya".into()])
        );
        assert_eq!(sequence("play rock and roll"), None);
        assert_eq!(sequence("turn wifi and bluetooth off"), None);
        assert_eq!(several_apps("open spotify and then pause the music"), None);
        assert_eq!(sequence("play kesariya"), None);
    }

    #[test]
    fn greetings_and_politeness_are_dropped() {
        assert_eq!(normalize("hey jitmo play kesariya"), "play kesariya");
        assert_eq!(normalize("yo play despacito pls"), "play despacito");
        assert_eq!(normalize("uh open spotify"), "open spotify");
        assert_eq!(normalize("please open notepad, thanks"), "open notepad,");
        assert_eq!(normalize("open spotify thank you"), "open spotify");
        // The sentence is never emptied.
        assert_eq!(normalize("please"), "please");
        assert_eq!(normalize("hey"), "hey");
        assert_eq!(normalize("  volume   50 "), "volume 50");
    }

    #[test]
    fn a_mistyped_first_verb_is_repaired() {
        for (typed, want) in [
            ("plya arijit singh", "play arijit singh"),
            ("paly some jazz", "play some jazz"),
            ("pley despacito", "play despacito"),
            ("opne spotify", "open spotify"),
            ("oepn notepad", "open notepad"),
            ("serach for pasta", "search for pasta"),
            ("pasue the music", "pause the music"),
            ("remnd me at 6", "remind me at 6"),
            ("muet", "mute"),
        ] {
            assert_eq!(normalize(typed), want, "{typed}");
        }
    }

    #[test]
    fn real_words_and_correct_verbs_are_left_alone() {
        for same in [
            "plan my day",
            "play kesariya",
            "stop the timer",
            "show recent files",
            "send hi to ved",
            "find invoice",
            "set a timer for 5 minutes",
            "what is the weather",
            "mine craft",
            "opening hours of the bank",
        ] {
            assert_eq!(normalize(same), same, "{same}");
        }
    }

    #[test]
    fn bare_arithmetic_is_labelled_as_a_calculation() {
        assert_eq!(normalize("2+2"), "calculate 2+2");
        assert_eq!(normalize("1,000 + 500"), "calculate 1,000 + 500");
        assert_eq!(normalize("(3+4)*2"), "calculate (3+4)*2");
        assert_eq!(normalize("0.1+0.2"), "calculate 0.1+0.2");
        assert_eq!(normalize("20 x 3"), "calculate 20 x 3");
        assert_eq!(normalize("1/3"), "calculate 1/3");
        assert_eq!(normalize("10 - 4"), "calculate 10 - 4");
        // Not sums.
        for same in ["555-1234", "10/11/2026", "2026", "50%", "x", "+"] {
            assert_eq!(normalize(same), same, "{same}");
        }
    }

    #[test]
    fn timer_typos_youtube_and_direct_routes() {
        assert_eq!(
            normalize("set a timr for 10 minutes"),
            "set a timer for 10 minutes"
        );
        assert_eq!(normalize("tiemr 5 min"), "timer 5 min");
        assert_eq!(normalize("set a 10 mintue timer"), "set a 10 minutes timer");
        assert_eq!(normalize("play jazz on youtube"), "search youtube for jazz");
        assert_eq!(
            normalize("watch fireship on YouTube"),
            "search youtube for fireship"
        );
        assert_eq!(normalize("play despacito"), "play despacito");
        let d = |s: &str| direct_skill(s, &normalize(s));
        assert_eq!(d("2+2"), Some("calc.math"));
        assert_eq!(d("18% of 2450"), Some("calc.math"));
        // The inbox in general, said many ways (and never the newsletter archive).
        for said in [
            "emails",
            "inbox",
            "check my email",
            "any new emails",
            "unread mail",
            "how many unread emails do i have",
            "show my inbox",
        ] {
            assert_eq!(d(said), Some("mail.inbox"), "{said}");
        }
        assert_eq!(d("any urgent emails"), Some("mail.urgent"));
        assert_eq!(d("read my latest email"), Some("mail.summarize"));
        assert_eq!(d("when is my next meeting"), Some("calendar.agenda"));
        // Not the inbox: other verbs, other things.
        assert_ne!(d("send an email to rahul"), Some("mail.inbox"));
        assert_ne!(d("remind me to mail the letter"), Some("mail.inbox"));
        assert_ne!(d("find emails about the invoice"), Some("mail.inbox"));
        // "search X" is the web, unless the words are about files on the PC.
        assert_eq!(d("search ved"), Some("browser.search"));
        assert_eq!(d("search for ved narvekar"), Some("browser.search"));
        assert_eq!(d("search cats on youtube"), Some("browser.search"));
        assert_eq!(normalize("search my files for tax"), "find tax");
        assert_eq!(normalize("search invoice"), "find invoice");
        assert_eq!(normalize("search downloads for invoice"), "find invoice");
        assert_eq!(normalize("search ved"), "search ved");
        // Plain answers, and plain refusals instead of the nearest skill.
        assert!(plain_reply("hello").is_some());
        assert_eq!(plain_reply("thanks!"), Some("Anytime."));
        assert!(plain_reply("close chrome").unwrap().contains("can't close"));
        assert!(plain_reply("shut down the pc")
            .unwrap()
            .contains("shut down"));
        assert!(plain_reply("sleep").is_some());
        assert!(plain_reply("restart").is_some());
        assert_eq!(plain_reply("restart spotify"), None);
        assert_eq!(plain_reply("open spotify"), None);
        assert_eq!(plain_reply("close the timer"), None);
        assert_eq!(plain_reply("turn off bluetooth"), None);
        assert_ne!(d("search my files for tax"), Some("browser.search"));
        assert_ne!(d("search downloads for invoice"), Some("browser.search"));
        assert_eq!(
            d("every Monday at 10 remind me to send the report"),
            Some("reminders.remind")
        );
        assert_eq!(
            d("remind me tmrw at 9 to pay rent"),
            Some("reminders.remind")
        );
        assert_eq!(d("set a timr for 10 minutes"), Some("reminders.timer"));
        assert_eq!(d("timer 5"), Some("reminders.timer"));
        // Music words win: these are not timers or reminders.
        assert_eq!(d("pause the timer and play music"), None);
        assert_eq!(d("play a song to remind me of home"), None);
        assert_eq!(d("open spotify"), None);
    }

    #[test]
    fn a_percentage_of_a_number_is_recognised() {
        for yes in [
            "18% of 2450",
            "what is 15 percent of 1,200?",
            "15 percent tip on 80",
            "what's 20% of $500",
            "7.5 pct of 200",
        ] {
            assert!(is_percent_of(yes), "{yes}");
        }
        for no in [
            "of 2450",
            "18% off",
            "50 percent",
            "percent of what",
            "open spotify",
            "18 of 2450",
        ] {
            assert!(!is_percent_of(no), "{no}");
        }
    }

    #[test]
    fn texting_spellings_of_common_words_are_written_out() {
        assert_eq!(
            normalize("remind me tmrw at 9 to pay rent"),
            "remind me tomorrow at 9 to pay rent"
        );
        assert_eq!(
            normalize("remind me abt the bill tonite"),
            "remind me about the bill tonight"
        );
        // Only whole words, and never the first word (that is the verb).
        assert_eq!(normalize("tmrw at 9"), "tmrw at 9");
        assert_eq!(normalize("set a timer"), "set a timer");
    }

    #[test]
    fn a_verb_glued_to_its_object_is_split() {
        assert_eq!(normalize("openspotify"), "open spotify");
        assert_eq!(normalize("launchchrome"), "launch chrome");
        assert_eq!(normalize("opener"), "opener");
        assert_eq!(normalize("startup"), "startup");
        assert_eq!(normalize("open"), "open");
    }
}

#[cfg(test)]
mod google_routes {
    use super::*;

    #[test]
    fn google_account_sentences_route_to_the_google_skills() {
        assert_eq!(
            work_skill("sign in to google"),
            Some("account.google_sign_in")
        );
        assert_eq!(
            work_skill("connect my gmail"),
            Some("account.google_sign_in")
        );
        assert_eq!(
            work_skill("sign out of google"),
            Some("account.google_sign_out")
        );
        assert_eq!(
            work_skill("set google client id to x.apps.googleusercontent.com and secret GOCSPX-y"),
            Some("account.google_client")
        );
        assert_ne!(
            work_skill("search google for pizza"),
            Some("account.google_sign_in")
        );
    }
}
