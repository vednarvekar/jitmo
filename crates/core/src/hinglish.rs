//! Hinglish (Hindi in Latin letters mixed with English) to the English the router and the
//! skills understand: "mujhe notepad open karo", "next gaana", "ek ghante ka timer",
//! "yaad dila do 6 baje mom ko call karna", "wifi band karo".
//!
//! Deliberately small and rule-based: a handful of very common words and sentence shapes
//! (verb last, "X ko Y karna", "N baje", times of day), and only when the sentence carries a
//! word that is clearly Hindi. Plain English is never touched.

/// Words that are clearly Hindi, never English: their presence says "translate this".
const STRONG: &[&str] = &[
    "mujhe", "karo", "kijiye", "kijiyega", "kholo", "bajao", "baja", "chalao", "chalu", "gaana",
    "gana", "ghante", "ghanta", "baje", "subah", "shaam", "raat", "yaad", "dilana", "dilao",
    "dila", "awaaz", "aawaz", "roko", "agla", "pichla", "karna", "batao", "dikhao", "dikha",
    "lagao", "laga", "kal", "parso", "aaj", "shuru", "mausam", "barish", "baarish", "kahan",
    "kaha", "naya", "nayi", "daalo", "dalo", "rakho", "rajdhani", "chand", "kab", "kaisa",
    "hogi", "mein",
];

/// Whole-sentence shapes that word-by-word translation gets wrong.
fn shape(tokens: &[String]) -> Option<String> {
    let t: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let pos = |w: &[&str]| t.iter().position(|x| w.contains(x));
    let join = |s: &[&str]| s.join(" ");
    // "pune mein barish hogi kya" / "mausam kaisa hai" / "delhi ka mausam"
    if let Some(m) = pos(&["mausam", "barish", "baarish"]) {
        let rain = t[m] != "mausam";
        let place: Vec<&str> = t[..m]
            .iter()
            .copied()
            .filter(|w| !["mein", "me", "ka", "ki", "ke", "aaj", "kal"].contains(w))
            .collect();
        let when = if t.contains(&"kal") { " tomorrow" } else { "" };
        let what = if rain { "will it rain" } else { "weather" };
        return Some(if place.is_empty() {
            format!("{what}{when}")
        } else {
            format!("{what} in {}{when}", join(&place))
        });
    }
    // "resume kahan hai" -> "find resume"
    if let Some(k) = pos(&["kahan", "kaha"]) {
        if k > 0 {
            return Some(format!("find {}", join(&t[..k])));
        }
    }
    // "mkbhd ka naya video" -> "open mkbhd's latest video"
    if let (Some(n), Some(v)) = (pos(&["naya", "nayi"]), pos(&["video", "vid", "videos"])) {
        let who: Vec<&str> = t[..n]
            .iter()
            .copied()
            .filter(|w| !["ka", "ki", "ke"].contains(w))
            .collect();
        if v > n && !who.is_empty() {
            return Some(format!("open {}'s latest video", join(&who)));
        }
    }
    // "report.pdf ko invoices mein daalo" -> "move report.pdf to invoices"
    if let (Some(ko), Some(mein), Some(_)) =
        (pos(&["ko"]), pos(&["mein", "me"]), pos(&["daalo", "dalo", "rakho", "daal", "rakh"]))
    {
        if ko > 0 && mein > ko + 1 {
            return Some(format!("move {} to {}", join(&t[..ko]), join(&t[ko + 1..mein])));
        }
    }
    // "bharat ki rajdhani kya hai" -> "what is the capital of bharat"
    if let Some(r) = pos(&["rajdhani"]) {
        let of: Vec<&str> = t[..r]
            .iter()
            .copied()
            .filter(|w| !["ki", "ka", "ke"].contains(w))
            .collect();
        if !of.is_empty() {
            let place = match join(&of).as_str() {
                "bharat" | "hindustan" => "india".to_string(),
                p => p.to_string(),
            };
            return Some(format!("what is the capital of {place}"));
        }
    }
    // "chand kab nikalega" -> "when is moonrise"
    if t.contains(&"chand") {
        return Some("when is moonrise today".to_string());
    }
    None
}

/// Words dropped because they only make the sentence polite or grammatical.
const FILLER: &[&str] = &[
    "mujhe", "karo", "kar", "kijiye", "kijiyega", "dena", "zara", "jaldi", "na", "ji", "hai",
    "hain", "ki", "ke", "liye", "lagao", "laga", "karna", "batao", "dikhao", "dikha", "ek", "ka",
];

/// Words after which a number word is a count ("ek ghante", "do minute").
const UNITS: &[&str] = &[
    "ghante", "ghanta", "minute", "min", "mint", "second", "sec", "din", "baar",
];

const VERBS: &[&str] = &[
    "open", "play", "pause", "resume", "skip", "search", "google", "lock", "mute", "unmute",
    "call", "send", "text", "message",
];

fn number(word: &str) -> Option<&'static str> {
    Some(match word {
        "ek" => "1",
        "do" => "2",
        "teen" => "3",
        "char" => "4",
        "paanch" | "panch" => "5",
        "chhe" | "che" => "6",
        "saat" => "7",
        "aath" => "8",
        "nau" => "9",
        "das" => "10",
        "pandrah" => "15",
        "bees" => "20",
        "tees" => "30",
        _ => return None,
    })
}

fn clean(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// "kal subah 9 baje" -> "tomorrow at 9am".
fn time_phrase(tokens: &[String]) -> String {
    let mut day = "";
    let mut period = "";
    let mut hour = String::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        match t {
            "kal" => day = "tomorrow",
            "aaj" => day = "today",
            "parso" => day = "day after tomorrow",
            "subah" => period = "am",
            "shaam" | "raat" | "dopahar" => period = "pm",
            "baje" => {}
            _ => {
                let next_is_baje = tokens.get(i + 1).is_some_and(|n| n == "baje");
                if next_is_baje || t.chars().all(|c| c.is_ascii_digit()) {
                    hour = number(t).map_or_else(|| t.to_string(), str::to_string);
                }
            }
        }
        i += 1;
    }
    let mut out = String::new();
    if !day.is_empty() {
        out.push_str(day);
    }
    if !hour.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("at {hour}{period}"));
    } else if !period.is_empty() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(if period == "am" { "morning" } else { "evening" });
    }
    out
}

/// The words that only say when ("9 baje", "kal", "subah"), with their numbers, removed.
fn without_time(tokens: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let before_baje = tokens.get(i + 1).is_some_and(|n| n == "baje");
        if before_baje
            || [
                "baje", "kal", "aaj", "parso", "subah", "shaam", "raat", "dopahar",
            ]
            .contains(&t.as_str())
        {
            continue;
        }
        out.push(t.clone());
    }
    out
}

/// "mom ko call karna" -> "call mom"; otherwise the words as they are, minus filler.
fn subject(tokens: &[String]) -> String {
    let tokens = without_time(tokens);
    let kept: Vec<&str> = tokens
        .iter()
        .map(String::as_str)
        .filter(|t| !FILLER.contains(t))
        .collect();
    if let Some(i) = kept.iter().position(|t| *t == "ko") {
        let (who, what) = (&kept[..i], &kept[i + 1..]);
        if !who.is_empty() && what.len() == 1 && VERBS.contains(&what[0]) {
            return format!("{} {}", what[0], who.join(" "));
        }
    }
    kept.into_iter()
        .filter(|t| *t != "ko")
        .collect::<Vec<_>>()
        .join(" ")
}

/// The English for a Hinglish sentence, or `None` when it is not Hinglish.
pub fn to_english(text: &str) -> Option<String> {
    let tokens: Vec<String> = text
        .split_whitespace()
        .map(clean)
        .filter(|t| !t.is_empty())
        .collect();
    if !tokens.iter().any(|t| STRONG.contains(&t.as_str())) {
        return None;
    }

    if let Some(s) = shape(&tokens) {
        return Some(s);
    }

    // "<time> yaad dila do <what>": a reminder.
    if let Some(p) = tokens.iter().position(|t| t == "yaad") {
        let after = &tokens[p + 1..];
        let rest: Vec<String> = after
            .iter()
            .skip_while(|t| {
                [
                    "dila", "dilana", "dilao", "dilado", "dilaye", "do", "dena", "karna",
                ]
                .contains(&t.as_str())
            })
            .cloned()
            .collect();
        let when = time_phrase(&tokens[..p]);
        let what = subject(&rest);
        let when_after = if when.is_empty() {
            time_phrase(&rest)
        } else {
            String::new()
        };
        let when = if when.is_empty() { when_after } else { when };
        return Some(match (when.is_empty(), what.is_empty()) {
            (true, true) => "remind me".to_string(),
            (false, true) => format!("remind me {when}"),
            (true, false) => format!("remind me to {what}"),
            (false, false) => format!("remind me {when} to {what}"),
        });
    }

    // Everything else: translate word by word, then put the verb first.
    let mut words: Vec<String> = Vec::new();
    let mut verb_first: Option<&str> = None;
    for (i, t) in tokens.iter().enumerate() {
        let t = t.as_str();
        let next_is_unit = tokens
            .get(i + 1)
            .is_some_and(|n| UNITS.contains(&n.as_str()));
        match t {
            "kholo" | "khol" => verb_first = Some("open"),
            "bajao" | "baja" | "chalao" | "chala" => verb_first = Some("play"),
            "roko" | "ruko" => verb_first = Some("pause"),
            "dikhao" | "dikha" => verb_first = Some("show"),
            "gaana" | "gana" => words.push("song".into()),
            "ghante" | "ghanta" => words.push("hour".into()),
            "chalu" | "shuru" => words.push("on".into()),
            "band" => words.push("off".into()),
            "awaaz" | "aawaz" => words.push("volume".into()),
            "kam" => words.push("down".into()),
            "badhao" | "badha" | "zyada" | "tez" => words.push("up".into()),
            "agla" => words.push("next".into()),
            "pichla" | "pehle" => words.push("previous".into()),
            "kal" => words.push("tomorrow".into()),
            "aaj" => words.push("today".into()),
            // "ek" and "do" are a count only right before a unit; elsewhere they are
            // filler ("do" also means "give": "timer laga do").
            "ek" | "do" => {
                if next_is_unit {
                    words.push(if t == "ek" { "1" } else { "2" }.into());
                }
            }
            _ => {
                if let Some(n) = number(t) {
                    words.push(n.into());
                } else if !FILLER.contains(&t) && t != "ko" && t != "baje" {
                    words.push(t.to_string());
                }
            }
        }
    }
    if let Some(v) = verb_first {
        words.insert(0, v.to_string());
    } else if let Some(last) = words.last().cloned() {
        // "notepad open" -> "open notepad"
        if VERBS.contains(&last.as_str()) && words.len() > 1 {
            words.pop();
            words.insert(0, last);
        }
    }
    let out = words.join(" ");
    (!out.is_empty() && out != text.trim().to_lowercase()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_hinglish_requests_become_english() {
        for (said, want) in [
            ("mujhe notepad open karo", "open notepad"),
            ("notepad kholo", "open notepad"),
            ("kesariya bajao", "play kesariya"),
            ("next gaana", "next song"),
            ("agla gaana", "next song"),
            ("wifi band karo", "wifi off"),
            ("bluetooth chalu karo", "bluetooth on"),
            ("awaaz kam karo", "volume down"),
            ("awaaz badhao", "volume up"),
            ("ek ghante ka timer", "1 hour timer"),
            ("timer laga do 10 minute ka", "timer 10 minute"),
            (
                "mujhe kal subah 9 baje yaad dilana meeting hai",
                "remind me tomorrow at 9am to meeting",
            ),
            (
                "yaad dila do 6 baje mom ko call karna",
                "remind me at 6 to call mom",
            ),
            (
                "kal shaam 6 baje yaad dila do bijli ka bill",
                "remind me tomorrow at 6pm to bijli bill",
            ),
            ("pune mein barish hogi kya", "will it rain in pune"),
            ("mausam kaisa hai", "weather"),
            ("kal delhi ka mausam", "weather in delhi tomorrow"),
            ("resume kahan hai", "find resume"),
            ("mkbhd ka naya video", "open mkbhd's latest video"),
            (
                "report.pdf ko invoices mein daalo",
                "move report.pdf to invoices",
            ),
            ("bharat ki rajdhani kya hai", "what is the capital of india"),
            ("chand kab nikalega", "when is moonrise today"),
        ] {
            assert_eq!(to_english(said).as_deref(), Some(want), "{said}");
        }
    }

    #[test]
    fn plain_english_is_never_touched() {
        for said in [
            "open spotify",
            "do it now",
            "turn the volume up",
            "play band of horses",
            "set a 10 minute timer",
            "remind me to call mom at 6pm",
            "what is the weather",
            "",
        ] {
            assert_eq!(to_english(said), None, "{said}");
        }
    }
}
