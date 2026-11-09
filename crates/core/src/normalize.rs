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
    "opens", "opened", "opener", "opening", "starts", "started", "launcher", "startup",
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
fn is_bare_expression(s: &str) -> bool {
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
        if words.len() > 1 && TRAILING.contains(&last.as_str()) {
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
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_verb_glued_to_its_object_is_split() {
        assert_eq!(normalize("openspotify"), "open spotify");
        assert_eq!(normalize("launchchrome"), "launch chrome");
        assert_eq!(normalize("opener"), "opener");
        assert_eq!(normalize("startup"), "startup");
        assert_eq!(normalize("open"), "open");
    }
}
