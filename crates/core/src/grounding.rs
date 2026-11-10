//! A model's written answer is shown only when the passage it was given supports it
//! (low-memory brief: "answers come from retrieved text, otherwise I'm not sure").
//!
//! The check is deliberately plain: every number in the answer must appear in the passage,
//! and most of the answer's longer words must too. A small model that wanders off the passage
//! (or invents a figure) fails this and the user gets "I'm not sure" instead.

fn stem(w: &str) -> String {
    let w = w.to_lowercase();
    for suffix in ["ing", "ed", "es", "s"] {
        if w.len() > suffix.len() + 3 {
            if let Some(s) = w.strip_suffix(suffix) {
                return s.trim_end_matches('e').to_string();
            }
        }
    }
    w.trim_end_matches('e').to_string()
}

fn content_words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4 && !w.chars().all(|c| c.is_ascii_digit()))
        .map(stem)
        .collect()
}

fn numbers(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect()
}

/// Is `answer` supported by `passage`?
pub fn supported(answer: &str, passage: &str) -> bool {
    let a = answer.trim();
    if a.is_empty() || a.to_lowercase().contains("as an ai") {
        return false;
    }
    let passage_lower = passage.to_lowercase();
    // Every figure must come from the passage.
    if !numbers(a)
        .iter()
        .all(|n| passage_lower.contains(n.as_str()))
    {
        return false;
    }
    let passage_stems: std::collections::HashSet<String> =
        content_words(passage).into_iter().collect();
    let words = content_words(a);
    if words.is_empty() {
        return true;
    }
    let found = words.iter().filter(|w| passage_stems.contains(*w)).count();
    found * 10 >= words.len() * 6
}

/// How well a page matches a question: the share (0 to 1) of the question's content words
/// found in the page text. A page about another thing with the same name scores low.
pub fn coverage(question: &str, page: &str) -> f64 {
    const FILLER: &[&str] = &[
        "what", "does", "work", "works", "explain", "simply", "simple", "word", "words", "tell",
        "about", "give", "show", "mean", "means", "meaning", "this", "that", "with", "from",
        "have", "make", "makes", "need", "know", "please", "describe",
    ];
    let page_stems: std::collections::HashSet<String> = content_words(page).into_iter().collect();
    let terms: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4)
        .filter(|w| !FILLER.contains(&w.to_lowercase().as_str()))
        .map(stem)
        .collect();
    if terms.is_empty() {
        return 0.0;
    }
    terms.iter().filter(|t| page_stems.contains(*t)).count() as f64 / terms.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSAGE: &str = "A vaccine trains the immune system to recognise a germ. It contains a weakened or inactive part of the germ, so the body makes antibodies. Smallpox was declared eradicated in 1980.";

    #[test]
    fn an_answer_that_follows_the_passage_is_supported() {
        assert!(supported(
            "A vaccine trains your immune system by showing it a weakened part of the germ, so the body makes antibodies.",
            PASSAGE
        ));
    }

    #[test]
    fn an_invented_figure_or_a_wandering_answer_is_not() {
        assert!(!supported("Smallpox was eradicated in 1975.", PASSAGE));
        assert!(!supported(
            "Quantum computers use qubits that remain entangled across galaxies.",
            PASSAGE
        ));
        assert!(!supported("", PASSAGE));
    }

    #[test]
    fn a_page_about_something_else_covers_little_of_the_question() {
        let band = "The Vaccines are an English indie rock band formed in London in 2010.";
        assert!(coverage("how do vaccines work", PASSAGE) >= 0.99);
        assert!(coverage("how do vaccines work please explain", band) >= 0.99);
        // Only the name matches; the question's other substance words are missing.
        assert!(coverage("how do vaccines protect against infection", band) < 0.6);
        assert!(coverage("how do vaccines protect against infection", PASSAGE) < 0.6);
    }
}
