//! Clarifying questions for bare commands.
//!
//! "volume", "wifi", "remind me", "search" name a task but not what to do. Instead
//! of failing with an error that is really a question, the flow asks it (quick
//! choices where there are natural ones, a typed answer otherwise) and then runs
//! the completed sentence as if the user had said it in full.
//!
//! This is deliberately narrow: only a sentence made of nothing but the task word
//! and polite filler is "bare". Anything with more content goes to the router as
//! before. Pure functions, no I/O, so every rule is a plain test.

/// One question to put to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Quick choices. A `None` value means "something else": a typed answer follows.
    Choice {
        question: &'static str,
        options: Vec<(&'static str, Option<&'static str>)>,
        /// Asked when the user picks the `None` option.
        other_question: &'static str,
        other_hint: &'static str,
    },
    /// A typed answer.
    Text {
        question: &'static str,
        hint: &'static str,
    },
}

/// The questions to ask, in order, and how to build the sentence from the answers
/// (`{0}`, `{1}`, ... are the answers in order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clarify {
    pub steps: Vec<Step>,
    pub template: &'static str,
    /// Used instead of `template` when the first answer is a bare number ("40", "40%").
    pub number_template: Option<&'static str>,
    /// Answers already known from the sentence, placed before the asked ones.
    pub known: Vec<String>,
}

fn words(text: &str) -> Vec<String> {
    text.trim()
        .trim_end_matches(['?', '!', '.'])
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Words that add nothing: politeness and "please do the thing" framing.
const FILLER: &[&str] = &[
    "please", "pls", "plz", "can", "could", "you", "will", "would", "i", "want", "to", "need",
    "like", "the", "my", "a", "an", "me", "it", "now", "just", "hey", "jitmo", "set", "change",
    "adjust", "turn", "switch", "toggle", "do", "make", "new",
];

/// True when every word is filler or one of `core`, and at least one is in `core`.
fn is_bare(w: &[String], core: &[&str]) -> bool {
    let mut seen_core = false;
    for word in w {
        if core.contains(&word.as_str()) {
            seen_core = true;
        } else if !FILLER.contains(&word.as_str()) {
            return false;
        }
    }
    seen_core
}

fn text(question: &'static str, hint: &'static str, template: &'static str) -> Clarify {
    Clarify {
        steps: vec![Step::Text { question, hint }],
        template,
        number_template: None,
        known: Vec::new(),
    }
}

fn pick(question: &'static str, options: Vec<(&'static str, Option<&'static str>)>) -> Clarify {
    Clarify {
        steps: vec![Step::Choice {
            question,
            options,
            other_question: "What level?",
            other_hint: "A number from 0 to 100",
        }],
        template: "{0}",
        number_template: None,
        known: Vec::new(),
    }
}

fn on_off(question: &'static str, on: &'static str, off: &'static str) -> Clarify {
    pick(
        question,
        vec![("Turn on", Some(on)), ("Turn off", Some(off))],
    )
}

/// The clarifying questions for a bare command, if `text` is one.
pub fn clarify(said: &str) -> Option<Clarify> {
    let w = words(said);
    if w.is_empty() || w.len() > 6 {
        return None;
    }
    let has = |names: &[&str]| w.iter().any(|x| names.contains(&x.as_str()));

    // "turn on" with nothing after it: which setting?
    if matches!(
        w.iter().map(String::as_str).collect::<Vec<_>>().as_slice(),
        ["turn" | "switch", "on" | "off"] | ["enable" | "disable"]
    ) {
        let on = w.iter().any(|x| x == "on" || x == "enable");
        let (q, a, b, c) = if on {
            (
                "Turn on what?",
                "turn on wifi",
                "turn on bluetooth",
                "turn on do not disturb",
            )
        } else {
            (
                "Turn off what?",
                "turn off wifi",
                "turn off bluetooth",
                "turn off do not disturb",
            )
        };
        return Some(pick(
            q,
            vec![
                ("Wi-Fi", Some(a)),
                ("Bluetooth", Some(b)),
                ("Do Not Disturb", Some(c)),
            ],
        ));
    }

    // Volume and brightness: up, down, mute, or a level.
    if is_bare(&w, &["volume", "sound", "audio", "loudness"]) {
        let mut c = pick(
            "What should I do with the volume?",
            vec![
                ("Louder", Some("turn the volume up")),
                ("Quieter", Some("turn the volume down")),
                ("Mute", Some("mute the volume")),
                ("Set a level...", None),
            ],
        );
        c.number_template = Some("set the volume to {0} percent");
        return Some(c);
    }
    if is_bare(&w, &["brightness"]) {
        let mut c = pick(
            "What should I do with the brightness?",
            vec![
                ("Brighter", Some("turn the brightness up")),
                ("Dimmer", Some("turn the brightness down")),
                ("Set a level...", None),
            ],
        );
        c.number_template = Some("set the brightness to {0} percent");
        return Some(c);
    }

    // Radios and Do Not Disturb: on or off.
    if is_bare(&w, &["wifi", "wi-fi", "wi", "fi"]) {
        return Some(on_off("Wi-Fi: on or off?", "turn on wifi", "turn off wifi"));
    }
    if is_bare(&w, &["bluetooth", "bt"]) {
        return Some(on_off(
            "Bluetooth: on or off?",
            "turn on bluetooth",
            "turn off bluetooth",
        ));
    }
    if is_bare(&w, &["dnd", "disturb", "notifications", "not", "don't"])
        && (has(&["dnd", "disturb"]) || w == ["notifications"])
    {
        return Some(on_off(
            "Do Not Disturb: on or off?",
            "turn on do not disturb",
            "turn off do not disturb",
        ));
    }

    // Work commands that name no target: ask for the missing part, then run the full sentence.
    match w.join(" ").trim_end_matches(['.', '?', '!']) {
        "move my meeting"
        | "move the meeting"
        | "move a meeting"
        | "reschedule my meeting"
        | "reschedule the meeting"
        | "move meeting" => {
            return Some(text(
                "Which meeting, and to when?",
                "For example: 3pm to tomorrow",
                "move my {0}",
            ));
        }
        "email the report" | "send the report" | "email report" | "send report"
        | "email a report" | "send a report" => {
            return Some(text(
                "Which report, and to whom?",
                "For example: the weekly report to Rahul",
                "email {0}",
            ));
        }
        "decline it"
        | "decline this"
        | "decline"
        | "decline the invite"
        | "decline invite"
        | "decline the meeting" => {
            return Some(text(
                "Which invite?",
                "For example: the Friday review",
                "decline {0}",
            ));
        }
        "share the folder" | "share a folder" | "share folder" | "share my folder" => {
            return Some(text(
                "Which folder, and with whom?",
                "For example: the Project folder with Priya",
                "share {0}",
            ));
        }
        _ => {}
    }

    // Music: what to do, or what to play.
    if has(&["play"]) && is_bare(&w, &["play", "music", "song", "songs"]) {
        return Some(text(
            "What should I play?",
            "A song, an artist or a mood",
            "play {0}",
        ));
    }
    if is_bare(&w, &["music", "song", "songs", "playback", "track"]) {
        return Some(pick(
            "What should I do with the music?",
            vec![
                ("Pause", Some("pause the music")),
                ("Resume", Some("resume the music")),
                ("Next song", Some("skip to the next song")),
                ("Previous song", Some("go back to the previous song")),
            ],
        ));
    }

    // Reminders: what, then when.
    if has(&["remind", "reminder"])
        && is_bare(
            &w,
            &[
                "remind",
                "reminder",
                "later",
                "something",
                "sometime",
                "soon",
            ],
        )
    {
        return Some(Clarify {
            steps: vec![
                Step::Text {
                    question: "What should I remind you about?",
                    hint: "For example: call mom",
                },
                Step::Choice {
                    question: "When?",
                    options: vec![
                        ("In 1 hour", Some("in 1 hour")),
                        ("This evening (6 pm)", Some("at 6pm")),
                        ("Tomorrow, 9 am", Some("tomorrow at 9am")),
                        ("Another time...", None),
                    ],
                    other_question: "When?",
                    other_hint: "For example: at 5:30pm, in 20 minutes, tomorrow 8am",
                },
            ],
            template: "remind me {1} to {0}",
            number_template: None,
            known: Vec::new(),
        });
    }

    // "remind me to call mom": the subject is known, only the time is missing.
    if let Some(what) = reminder_without_time(said) {
        return Some(Clarify {
            steps: vec![Step::Choice {
                question: "When should I remind you?",
                options: vec![
                    ("In 1 hour", Some("in 1 hour")),
                    ("This evening (6 pm)", Some("at 6pm")),
                    ("Tomorrow, 9 am", Some("tomorrow at 9am")),
                    ("Another time...", None),
                ],
                other_question: "When?",
                other_hint: "For example: at 5:30pm, in 20 minutes, tomorrow 8am",
            }],
            template: "remind me {1} to {0}",
            number_template: None,
            known: vec![what],
        });
    }

    // A calculation with nothing to calculate.
    if is_bare(&w, &["calculate", "calc", "compute", "math", "maths"]) {
        return Some(text(
            "What should I calculate?",
            "For example: 18% of 2450, or 12 * 7",
            "calculate {0}",
        ));
    }

    // Search, websites and apps: one typed answer.
    if has(&["search", "google", "lookup"])
        && is_bare(
            &w,
            &["search", "google", "lookup", "web", "internet", "for"],
        )
    {
        return Some(text(
            "What should I search for?",
            "Anything you would type into a search box",
            "search the web for {0}",
        ));
    }
    if is_bare(&w, &["youtube", "yt"]) || (has(&["search"]) && has(&["youtube"]) && w.len() <= 3) {
        return Some(text(
            "What should I search YouTube for?",
            "For example: pasta recipe",
            "search youtube for {0}",
        ));
    }
    let bare_go = w == ["go"] || w == ["go", "to"];
    if bare_go
        || (has(&["website", "site", "webpage", "visit"])
            && is_bare(
                &w,
                &["go", "website", "site", "webpage", "page", "open", "visit"],
            ))
    {
        return Some(text(
            "Which website?",
            "For example: amazon.in",
            "open the website {0}",
        ));
    }
    // "open", "open the", "launch my": the verb with only an article after it.
    let bare_open = matches!(w.first().map(String::as_str), Some("open" | "launch"))
        && w[1..]
            .iter()
            .all(|x| ["the", "a", "an", "my", "some", "up"].contains(&x.as_str()));
    if bare_open
        || (has(&["app", "application", "program"])
            && is_bare(
                &w,
                &["open", "app", "launch", "start", "application", "program"],
            ))
    {
        return Some(text("Which app?", "For example: Spotify", "open {0}"));
    }

    // A question with no subject: "what is", "tell me about", "who is".
    let joined = w.join(" ");
    if [
        "what is",
        "whats",
        "what's",
        "tell me about",
        "tell me",
        "who is",
        "fact",
        "facts",
        "define",
        "what",
    ]
    .contains(&joined.as_str())
    {
        return Some(text(
            "What would you like to know?",
            "For example: who wrote Pride and Prejudice",
            "tell me about {0}",
        ));
    }
    // The time somewhere, with no place.
    if [
        "time in",
        "what time is it in",
        "whats the time in",
        "what's the time in",
    ]
    .contains(&joined.as_str())
    {
        return Some(text("Which city?", "For example: Tokyo", "time in {0}"));
    }
    if ["sun", "the sun"].contains(&joined.as_str()) {
        return Some(pick(
            "Sunrise or sunset?",
            vec![
                ("Sunrise", Some("when is sunrise today")),
                ("Sunset", Some("when is sunset today")),
            ],
        ));
    }
    // The saved place, with no place.
    if [
        "set my city",
        "set my city to",
        "set my location",
        "set my location to",
        "i live in",
        "my location",
        "my city",
        "set city",
        "set location",
        "change my city",
    ]
    .contains(&joined.as_str())
    {
        return Some(text(
            "Which city do you live in?",
            "For example: Pune",
            "set my city to {0}",
        ));
    }
    // Files with nothing named.
    if is_bare(&w, &["find", "file", "files", "search"]) && has(&["find"]) {
        return Some(text(
            "Which file should I look for?",
            "A name or part of it, e.g. invoice",
            "find {0}",
        ));
    }
    if is_bare(&w, &["move", "file", "files"]) && has(&["move"]) {
        return Some(Clarify {
            steps: vec![
                Step::Text {
                    question: "Which file should I move?",
                    hint: "For example: report.pdf",
                },
                Step::Text {
                    question: "To which folder?",
                    hint: "For example: Documents",
                },
            ],
            template: "move {0} to {1}",
            number_template: None,
            known: Vec::new(),
        });
    }
    None
}

/// The subject of "remind me to call mom" when the sentence names no time at all.
fn reminder_without_time(said: &str) -> Option<String> {
    let t = said.trim().trim_end_matches(['?', '!', '.']);
    let lower = t.to_lowercase();
    let rest = lower.strip_prefix("remind me to ")?;
    let what = t.get(t.len() - rest.len()..)?.trim();
    if what.is_empty() {
        return None;
    }
    const CUES: &[&str] = &[
        "am",
        "pm",
        "today",
        "tomorrow",
        "tmrw",
        "tonight",
        "morning",
        "afternoon",
        "evening",
        "noon",
        "midnight",
        "in",
        "at",
        "on",
        "every",
        "next",
        "later",
        "week",
        "weekend",
        "day",
        "days",
        "hour",
        "hours",
        "minute",
        "minutes",
        "min",
        "mins",
        "mon",
        "monday",
        "tue",
        "tuesday",
        "wed",
        "wednesday",
        "thu",
        "thursday",
        "fri",
        "friday",
        "sat",
        "saturday",
        "sun",
        "sunday",
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "jun",
        "jul",
        "aug",
        "sep",
        "oct",
        "nov",
        "dec",
        "soon",
        "after",
        "before",
        "when",
        "once",
    ];
    let timed = rest
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()) || CUES.contains(&w));
    (!timed).then(|| what.to_string())
}

/// Build the final sentence from the answers.
pub fn fill(c: &Clarify, asked: &[String]) -> String {
    let answers: Vec<String> = c.known.iter().chain(asked).cloned().collect();
    let first = answers.first().map(|s| s.trim()).unwrap_or("");
    let is_number = !first.is_empty()
        && first
            .trim_end_matches('%')
            .chars()
            .all(|ch| ch.is_ascii_digit());
    let (template, numeric) = match (is_number, c.number_template) {
        (true, Some(t)) => (t, true),
        _ => (c.template, false),
    };
    let mut out = template.to_string();
    for (i, a) in answers.iter().enumerate() {
        let a = a.trim();
        let a = if numeric { a.trim_end_matches('%') } else { a };
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

/// A question a connector asked by failing ("Do you want it on or off?"), turned into
/// buttons: the question and, for each button, the complete sentence to run when it is
/// clicked. `None` when the message is not one of those questions, or the sentence does
/// not say what it is about (which radio, which setting).
pub fn question_for(
    message: &str,
    said: &str,
) -> Option<(&'static str, Vec<(&'static str, String)>)> {
    let lower = said.to_lowercase();
    let squashed: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
    let mentions = |names: &[&str]| names.iter().any(|n| squashed.contains(n));
    let opts = |items: &[(&'static str, &str)]| -> Vec<(&'static str, String)> {
        items.iter().map(|(l, s)| (*l, (*s).to_string())).collect()
    };
    if message == "Do you want it on or off?" {
        // Which radio? Two at once ("wifi and bluetooth") is not a question one button answers.
        let bluetooth = mentions(&["bluetooth", "blutooth", "bluetoth", "bluetoot"]);
        let wifi = mentions(&["wifi", "wlan", "wireless"]);
        return match (wifi, bluetooth) {
            (true, false) => Some((
                "Wi-Fi: on or off?",
                opts(&[("Turn on", "turn on wifi"), ("Turn off", "turn off wifi")]),
            )),
            (false, true) => Some((
                "Bluetooth: on or off?",
                opts(&[
                    ("Turn on", "turn on bluetooth"),
                    ("Turn off", "turn off bluetooth"),
                ]),
            )),
            // Neither radio was recognised ("turn on blue", "turn it off"): ask which, and
            // keep the on or off they already said.
            (false, false) => {
                let words: Vec<&str> = lower
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| !w.is_empty())
                    .collect();
                let on = words
                    .iter()
                    .any(|w| matches!(*w, "on" | "enable" | "activate" | "chalu" | "start"));
                let off = words
                    .iter()
                    .any(|w| matches!(*w, "off" | "of" | "disable" | "band" | "kill" | "stop"));
                match (on, off) {
                    (true, false) => Some((
                        "Turn on which one?",
                        opts(&[
                            ("Wi-Fi", "turn on wifi"),
                            ("Bluetooth", "turn on bluetooth"),
                        ]),
                    )),
                    (false, true) => Some((
                        "Turn off which one?",
                        opts(&[
                            ("Wi-Fi", "turn off wifi"),
                            ("Bluetooth", "turn off bluetooth"),
                        ]),
                    )),
                    _ => Some((
                        "Which one?",
                        opts(&[
                            ("Wi-Fi on", "turn on wifi"),
                            ("Wi-Fi off", "turn off wifi"),
                            ("Bluetooth on", "turn on bluetooth"),
                            ("Bluetooth off", "turn off bluetooth"),
                        ]),
                    )),
                }
            }
            _ => None,
        };
    }
    if message == "Do you want Do Not Disturb on or off?" {
        return Some((
            "Do Not Disturb: on or off?",
            opts(&[
                ("Turn on", "turn on do not disturb"),
                ("Turn off", "turn off do not disturb"),
            ]),
        ));
    }
    if message == "Should I pause, resume or skip?" {
        return Some((
            "What should I do with the music?",
            opts(&[
                ("Pause", "pause the music"),
                ("Resume", "resume the music"),
                ("Next song", "skip to the next song"),
                ("Previous song", "go back to the previous song"),
            ]),
        ));
    }
    if message.starts_with("I'm not sure what to change") {
        return Some(if mentions(&["bright", "screen", "dim"]) {
            (
                "What should I do with the brightness?",
                opts(&[
                    ("Brighter", "turn the brightness up"),
                    ("Dimmer", "turn the brightness down"),
                ]),
            )
        } else {
            (
                "What should I do with the volume?",
                opts(&[
                    ("Louder", "turn the volume up"),
                    ("Quieter", "turn the volume down"),
                    ("Mute", "mute the volume"),
                ]),
            )
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_questions_become_buttons_with_complete_sentences() {
        let (q, opts) = question_for("Do you want it on or off?", "turn of blutooth").unwrap();
        assert_eq!(q, "Bluetooth: on or off?");
        assert_eq!(opts[1], ("Turn off", "turn off bluetooth".to_string()));
        let (q, _) = question_for("Do you want it on or off?", "wi-fi chalu karo").unwrap();
        assert_eq!(q, "Wi-Fi: on or off?");
        // Two radios named: nothing sensible to offer.
        assert!(question_for(
            "Do you want it on or off?",
            "turn on wifi and off bluetooth"
        )
        .is_none());
        // No radio recognised ("turn on blue" fails to parse a name): ask which, keeping the
        // on or off that was said.
        let (q, opts) = question_for("Do you want it on or off?", "turn it off").unwrap();
        assert_eq!(q, "Turn off which one?");
        assert_eq!(opts[1], ("Bluetooth", "turn off bluetooth".to_string()));
        let (q, opts) = question_for("Do you want it on or off?", "turn on xyz").unwrap();
        assert_eq!(q, "Turn on which one?");
        assert_eq!(opts[0].1, "turn on wifi");
        let (_, opts) = question_for("Do you want it on or off?", "toggle it").unwrap();
        assert_eq!(opts.len(), 4);
        let (_, opts) = question_for("Should I pause, resume or skip?", "music?").unwrap();
        assert_eq!(opts.len(), 4);
        let (q, _) = question_for(
            "I'm not sure what to change. Try “turn the volume down”.",
            "dim it",
        )
        .unwrap();
        assert!(q.contains("brightness"));
        assert!(question_for("I couldn't find that.", "x").is_none());
        // Every offered sentence is complete: not bare, so it can never loop.
        for (msg, said) in [
            ("Do you want it on or off?", "turn of wifi"),
            ("Do you want Do Not Disturb on or off?", "dnd"),
            ("Should I pause, resume or skip?", "music"),
            (
                "I'm not sure what to change. Try “turn the volume down”.",
                "volume",
            ),
        ] {
            for (_, sentence) in question_for(msg, said).unwrap().1 {
                assert_eq!(clarify(&sentence), None, "{sentence}");
            }
        }
    }

    fn first_question(said: &str) -> (&'static str, bool) {
        let c = clarify(said).unwrap_or_else(|| panic!("{said:?} should be clarified"));
        match &c.steps[0] {
            Step::Choice { question, .. } => (question, true),
            Step::Text { question, .. } => (question, false),
        }
    }

    #[test]
    fn bare_commands_get_a_question() {
        for (said, question, is_choice) in [
            ("volume", "What should I do with the volume?", true),
            (
                "change the volume",
                "What should I do with the volume?",
                true,
            ),
            ("wifi", "Wi-Fi: on or off?", true),
            ("turn wifi", "Wi-Fi: on or off?", true),
            ("bluetooth", "Bluetooth: on or off?", true),
            ("dnd", "Do Not Disturb: on or off?", true),
            ("do not disturb", "Do Not Disturb: on or off?", true),
            ("music", "What should I do with the music?", true),
            ("brightness", "What should I do with the brightness?", true),
            ("play", "What should I play?", false),
            ("calculate", "What should I calculate?", false),
            ("search", "What should I search for?", false),
            ("google", "What should I search for?", false),
            ("youtube", "What should I search YouTube for?", false),
            ("go to", "Which website?", false),
            ("open website", "Which website?", false),
            ("open app", "Which app?", false),
            ("open", "Which app?", false),
            ("open the", "Which app?", false),
            ("launch my", "Which app?", false),
            ("remind me later", "What should I remind you about?", false),
            ("what is", "What would you like to know?", false),
            ("tell me about", "What would you like to know?", false),
            ("time in", "Which city?", false),
            ("sun", "Sunrise or sunset?", true),
            ("set my city", "Which city do you live in?", false),
            ("i live in", "Which city do you live in?", false),
            ("find", "Which file should I look for?", false),
            ("find file", "Which file should I look for?", false),
            ("move", "Which file should I move?", false),
            ("notifications", "Do Not Disturb: on or off?", true),
            ("remind me", "What should I remind you about?", false),
            ("set a reminder", "What should I remind you about?", false),
        ] {
            assert_eq!(first_question(said), (question, is_choice), "{said}");
        }
    }

    #[test]
    fn a_reminder_with_a_subject_but_no_time_asks_only_when() {
        let c = clarify("remind me to call mom").unwrap();
        assert_eq!(c.known, ["call mom"]);
        assert_eq!(c.steps.len(), 1);
        assert_eq!(fill(&c, &["at 6pm".into()]), "remind me at 6pm to call mom");
        let Step::Choice { options, .. } = &c.steps[0] else {
            panic!("choice")
        };
        for (_, value) in options {
            if let Some(v) = value {
                assert_eq!(clarify(&fill(&c, &[(*v).into()])), None, "{v}");
            }
        }
        // The original capitalisation of the subject is kept.
        assert_eq!(
            clarify("Remind me to Pay Rent?").unwrap().known,
            ["Pay Rent"]
        );
        // Any hint of a time leaves it to the reminder parser.
        for timed in [
            "remind me to call mom at 6pm",
            "remind me to call mom tomorrow",
            "remind me to call mom in 10 minutes",
            "remind me to call mom on friday",
            "remind me to call mom later",
            "remind me to take 2 tablets",
        ] {
            assert_eq!(clarify(timed), None, "{timed}");
        }
    }

    #[test]
    fn complete_requests_are_left_to_the_router() {
        for said in [
            "turn the volume up",
            "volume 50",
            "set volume to 30",
            "turn off wifi",
            "turn on bluetooth",
            "dnd for an hour",
            "pause the music",
            "play kesariya",
            "search for pasta recipes",
            "open spotify",
            "open the calculator",
            "remind me to call mom at 6pm",
            "go to amazon",
            "what is the weather",
            "set a 10 minute timer",
            "",
        ] {
            assert_eq!(clarify(said), None, "{said:?}");
        }
    }

    #[test]
    fn answers_become_complete_sentences_that_are_not_asked_about_again() {
        // Every choice value must itself be a complete request, or it would loop.
        for said in ["volume", "brightness", "wifi", "bluetooth", "dnd", "music"] {
            let c = clarify(said).unwrap();
            let Step::Choice { options, .. } = &c.steps[0] else {
                panic!("{said}")
            };
            for (_, value) in options {
                if let Some(v) = value {
                    assert_eq!(clarify(v), None, "{v}");
                }
            }
        }
        let rem = clarify("remind me").unwrap();
        let Step::Choice { options, .. } = &rem.steps[1] else {
            panic!("when?")
        };
        for (_, value) in options {
            if let Some(v) = value {
                let s = fill(&rem, &["call mom".into(), (*v).into()]);
                assert_eq!(clarify(&s), None, "{s}");
            }
        }

        let vol = clarify("volume").unwrap();
        assert_eq!(
            fill(&vol, &["turn the volume up".into()]),
            "turn the volume up"
        );
        assert_eq!(fill(&vol, &["40".into()]), "set the volume to 40 percent");
        assert_eq!(fill(&vol, &["40%".into()]), "set the volume to 40 percent");
        assert_eq!(
            fill(&rem, &["call mom".into(), "at 6pm".into()]),
            "remind me at 6pm to call mom"
        );
        assert_eq!(
            fill(&clarify("search").unwrap(), &["pasta recipe".into()]),
            "search the web for pasta recipe"
        );
        assert_eq!(
            fill(&clarify("open website").unwrap(), &["amazon.in".into()]),
            "open the website amazon.in"
        );
    }
}
