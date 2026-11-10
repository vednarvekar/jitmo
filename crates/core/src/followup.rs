//! Follow-ups: "what about tomorrow", "and in Mumbai", "again". The bar remembers the last
//! request that worked and turns a short follow-up into a full sentence that the normal
//! pipeline then handles, so every safety check still runs on the full sentence.

const DAYS: &[&str] = &[
    "today",
    "tomorrow",
    "tonight",
    "this weekend",
    "weekend",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

fn clean(s: &str) -> String {
    s.trim()
        .trim_end_matches(['?', '!', '.'])
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn day_in(s: &str) -> Option<&'static str> {
    DAYS.iter().copied().find(|d| {
        s.split(|c: char| !c.is_alphanumeric())
            .collect::<Vec<_>>()
            .join(" ")
            .contains(d)
    })
}

fn place_in(s: &str) -> Option<String> {
    let i = s.rfind(" in ")?;
    let mut rest = s[i + 4..].to_string();
    for d in DAYS {
        rest = rest.replace(d, "");
    }
    let rest = rest.trim().to_string();
    (!rest.is_empty()).then_some(rest)
}

fn is_weather(last: &str) -> bool {
    ["weather", "rain", "temperature", "forecast", "sunny", "umbrella", "hot", "cold"]
        .iter()
        .any(|w| last.contains(w))
}

fn is_calendar(last: &str) -> bool {
    ["calendar", "agenda", "meeting", "schedule", "free "]
        .iter()
        .any(|w| last.contains(w))
}

/// A full sentence for a short follow-up, or `None` when `said` stands on its own.
pub fn rewrite(last: &str, said: &str) -> Option<String> {
    let last_c = clean(last);
    let s = clean(said);
    if last_c.is_empty() || s.is_empty() {
        return None;
    }
    if matches!(
        s.as_str(),
        "again" | "do that again" | "repeat that" | "once more" | "do it again" | "same again"
    ) {
        return Some(last.trim().to_string());
    }
    // "what about X", "how about X", "and X"
    let rest = ["what about ", "how about ", "and what about ", "and "]
        .iter()
        .find_map(|p| s.strip_prefix(p))
        .map(str::to_string)
        // A lone day word after a weather or calendar answer ("tomorrow?").
        .or_else(|| DAYS.contains(&s.as_str()).then(|| s.clone()))?;
    if rest.is_empty() || rest.split(' ').count() > 6 {
        return None;
    }
    if is_weather(&last_c) {
        let day = day_in(&rest).or_else(|| day_in(&last_c));
        let place = if day_in(&rest).is_some() && !rest.starts_with("in ") {
            place_in(&rest).or_else(|| place_in(&last_c))
        } else {
            Some(rest.trim_start_matches("in ").to_string())
                .filter(|p| day_in(p).is_none())
                .or_else(|| place_in(&last_c))
        };
        let mut out = String::from("weather");
        if let Some(d) = day {
            out.push(' ');
            out.push_str(d);
        }
        if let Some(p) = place {
            out.push_str(" in ");
            out.push_str(&p);
        }
        return Some(out);
    }
    if is_calendar(&last_c) {
        if let Some(d) = day_in(&rest) {
            return Some(format!("what's on my calendar {d}"));
        }
    }
    // A fact about something else: swap the subject ("capital of australia" -> "capital of france").
    if let Some(i) = last_c.rfind(" of ") {
        if !rest.contains(' ') || rest.split(' ').count() <= 3 {
            return Some(format!("{} of {}", &last_c[..i], rest));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_follow_ups() {
        assert_eq!(
            rewrite("weather in pune", "what about tomorrow?").as_deref(),
            Some("weather tomorrow in pune")
        );
        assert_eq!(
            rewrite("will it rain tomorrow in pune", "and in mumbai").as_deref(),
            Some("weather tomorrow in mumbai")
        );
        assert_eq!(
            rewrite("weather in pune", "tomorrow").as_deref(),
            Some("weather tomorrow in pune")
        );
    }

    #[test]
    fn calendar_and_fact_follow_ups() {
        assert_eq!(
            rewrite("what's on my calendar today", "what about friday").as_deref(),
            Some("what's on my calendar friday")
        );
        assert_eq!(
            rewrite("what is the capital of australia", "what about france").as_deref(),
            Some("what is the capital of france")
        );
    }

    #[test]
    fn again_repeats_and_stand_alone_sentences_pass() {
        assert_eq!(rewrite("open spotify", "again").as_deref(), Some("open spotify"));
        assert_eq!(rewrite("weather in pune", "open chrome"), None);
        assert_eq!(rewrite("", "again"), None);
    }
}
