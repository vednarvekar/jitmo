//! Slot resolver: turns the words the router found ("this file", "the invoice",
//! "ved") into real things (a file with a path), using the skill's `resolve`
//! order for each slot (LLD 07, FR4).
//!
//! Strategies, tried in the order the skill lists them:
//! - `selected_file`  the file selected when the bar was opened; used when the
//!   user said "this" or nothing at all
//! - `named_file`     fuzzy match of the words against files in the usual folders
//! - `recent_download` the newest download; used for "the latest download" or
//!   when nothing was said and nothing is selected
//! - `named_*`        (app, contact, ...) the words as said, until those lookups exist
//! - `usual_*`        the user's usual choice from the settings (`usual.<type>`),
//!   used when nothing was said
//!
//! If several files fit about equally well the result is [`Resolution::Choose`],
//! and Core asks one short question with the options (FR4). A tie is never
//! settled by guessing.

use std::collections::BTreeMap;

/// A slot's value. `text` is what to show and substitute for `{slot}`; `fields`
/// hold parts of it, like `{file.name}` and `{file.path}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotValue {
    pub text: String,
    pub fields: BTreeMap<String, String>,
}

impl SlotValue {
    /// A value made only from the words the user said.
    pub fn from_text(text: &str) -> Self {
        SlotValue {
            text: text.to_string(),
            fields: BTreeMap::from([("name".to_string(), text.to_string())]),
        }
    }

    pub fn from_file(f: &FileRef) -> Self {
        SlotValue {
            text: f.name.clone(),
            fields: BTreeMap::from([
                ("name".to_string(), f.name.clone()),
                ("path".to_string(), f.path.clone()),
            ]),
        }
    }
}

/// A file the context service knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub name: String,
    pub path: String,
    /// Seconds since the Unix epoch; newest first when sorting.
    pub modified: u64,
}

/// Where context comes from. The real one reads the file system and what was
/// selected in Explorer; tests use a fixed list.
pub trait ContextSource: Send {
    /// The file selected when the bar was opened, if any.
    fn selected_file(&self) -> Option<FileRef>;
    /// Newest downloads first.
    fn recent_downloads(&self, limit: usize) -> Vec<FileRef>;
    /// Every file that `named_file` may match against.
    fn searchable_files(&self) -> Vec<FileRef>;
}

/// No context at all (default until the app supplies one).
pub struct NoContext;

impl ContextSource for NoContext {
    fn selected_file(&self) -> Option<FileRef> {
        None
    }
    fn recent_downloads(&self, _: usize) -> Vec<FileRef> {
        Vec::new()
    }
    fn searchable_files(&self) -> Vec<FileRef> {
        Vec::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Found(SlotValue),
    /// Several fit about equally: ask one question. Each option is (id, label, value).
    Choose {
        question: String,
        options: Vec<(String, String, SlotValue)>,
    },
    /// Nothing fit. Holds the words the user said, if any.
    NotFound(Option<String>),
}

/// Two matches this close in score count as a tie.
const TIE_MARGIN: f32 = 0.08;
/// A file name must match at least this well to be considered at all.
const MIN_SCORE: f32 = 0.5;
/// Most options shown in one question.
const MAX_OPTIONS: usize = 3;

/// Resolve one slot. `said` is the span text from the router, if any; `usual`
/// is the user's usual value for this slot type.
pub fn resolve(
    strategies: &[String],
    said: Option<&str>,
    ctx: &dyn ContextSource,
    usual: Option<&str>,
) -> Resolution {
    let said = said.map(str::trim).filter(|s| !s.is_empty());
    let deictic = said.is_none_or(is_deictic);
    let recency = said.is_some_and(is_recency);
    for strategy in strategies {
        match strategy.as_str() {
            "selected_file" if deictic => {
                if let Some(f) = ctx.selected_file() {
                    return Resolution::Found(SlotValue::from_file(&f));
                }
            }
            "named_file" => {
                if let (Some(words), false, false) = (said, deictic, recency) {
                    if let Some(r) = match_named_file(words, &ctx.searchable_files()) {
                        return r;
                    }
                }
            }
            "recent_download" if deictic || recency => {
                if let Some(f) = ctx.recent_downloads(1).first() {
                    return Resolution::Found(SlotValue::from_file(f));
                }
            }
            s if s.starts_with("named_") && s != "named_file" => {
                if let Some(words) = said {
                    return Resolution::Found(SlotValue::from_text(words));
                }
            }
            s if s.starts_with("usual_") && said.is_none() => {
                if let Some(v) = usual {
                    return Resolution::Found(SlotValue::from_text(v));
                }
            }
            _ => {}
        }
    }
    Resolution::NotFound(said.map(str::to_string))
}

fn match_named_file(words: &str, files: &[FileRef]) -> Option<Resolution> {
    let mut scored: Vec<(f32, &FileRef)> = files
        .iter()
        .map(|f| (score(words, &f.name), f))
        .filter(|(s, _)| *s >= MIN_SCORE)
        .collect();
    // Best score first; among equals the newest file.
    scored.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.modified.cmp(&a.1.modified))
    });
    let best = scored.first()?.0;
    let close: Vec<&(f32, &FileRef)> = scored
        .iter()
        .filter(|(s, _)| best - *s <= TIE_MARGIN)
        .take(MAX_OPTIONS)
        .collect();
    if close.len() == 1 {
        return Some(Resolution::Found(SlotValue::from_file(close[0].1)));
    }
    Some(Resolution::Choose {
        question: "Which one?".to_string(),
        options: close
            .iter()
            .map(|(_, f)| (f.path.clone(), f.name.clone(), SlotValue::from_file(f)))
            .collect(),
    })
}

/// "this", "this file", "that one", "selected", "it".
fn is_deictic(words: &str) -> bool {
    let w = words.to_lowercase();
    let first = w.split_whitespace().next().unwrap_or("");
    matches!(first, "this" | "that" | "it" | "selected" | "current")
}

/// "the latest download", "my newest file", "the last one I downloaded".
fn is_recency(words: &str) -> bool {
    let w = words.to_lowercase();
    [
        "latest",
        "newest",
        "recent",
        "last download",
        "just downloaded",
    ]
    .iter()
    .any(|k| w.contains(k))
}

const STOPWORDS: [&str; 6] = ["the", "my", "a", "an", "of", "file"];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// How well the user's words match a file name, from 0 to 1.
///
/// Each wanted word scores 1 for an exact word, 0.9 if it starts or contains
/// the word, 0.8 if it is one typo away (words of 5+ letters), else 0. The
/// average is trimmed slightly when the name has many more words than were asked for.
pub fn score(wanted: &str, file_name: &str) -> f32 {
    let want: Vec<String> = tokens(wanted)
        .into_iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect();
    let have = tokens(file_name);
    if want.is_empty() || have.is_empty() {
        return 0.0;
    }
    let total: f32 = want
        .iter()
        .map(|w| have.iter().map(|h| word_score(w, h)).fold(0.0f32, f32::max))
        .sum();
    let base = total / want.len() as f32;
    let extra = (want.len() as f32 / have.len() as f32).min(1.0);
    base * (0.9 + 0.1 * extra)
}

fn word_score(want: &str, have: &str) -> f32 {
    if want == have {
        1.0
    } else if want.len() >= 3 && (have.starts_with(want) || have.contains(want)) {
        0.9
    } else if want.len() >= 5 && have.len() >= 5 && edit_distance(want, have) <= 1 {
        0.8
    } else {
        0.0
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed {
        selected: Option<FileRef>,
        files: Vec<FileRef>,
    }

    impl ContextSource for Fixed {
        fn selected_file(&self) -> Option<FileRef> {
            self.selected.clone()
        }
        fn recent_downloads(&self, limit: usize) -> Vec<FileRef> {
            let mut f = self.files.clone();
            f.sort_by_key(|x| std::cmp::Reverse(x.modified));
            f.truncate(limit);
            f
        }
        fn searchable_files(&self) -> Vec<FileRef> {
            self.files.clone()
        }
    }

    fn file(name: &str, modified: u64) -> FileRef {
        FileRef {
            name: name.into(),
            path: format!("C:\\Users\\me\\Documents\\{name}"),
            modified,
        }
    }

    fn strategies(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn file_slot() -> Vec<String> {
        strategies(&["selected_file", "named_file", "recent_download"])
    }

    fn ctx() -> Fixed {
        Fixed {
            selected: None,
            files: vec![
                file("Invoice March.pdf", 100),
                file("Invoice April.pdf", 200),
                file("Resume 2026.docx", 50),
                file("holiday-photo.jpg", 300),
            ],
        }
    }

    fn found(r: Resolution) -> SlotValue {
        match r {
            Resolution::Found(v) => v,
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn this_file_uses_the_selection() {
        let mut c = ctx();
        c.selected = Some(file("Report.pdf", 1));
        let v = found(resolve(&file_slot(), Some("this file"), &c, None));
        assert_eq!(v.text, "Report.pdf");
        assert_eq!(v.fields["path"], "C:\\Users\\me\\Documents\\Report.pdf");
    }

    #[test]
    fn nothing_said_and_nothing_selected_takes_the_newest_download() {
        let v = found(resolve(&file_slot(), None, &ctx(), None));
        assert_eq!(v.text, "holiday-photo.jpg");
    }

    #[test]
    fn the_selection_wins_over_downloads_when_nothing_is_said() {
        let mut c = ctx();
        c.selected = Some(file("Report.pdf", 1));
        assert_eq!(
            found(resolve(&file_slot(), None, &c, None)).text,
            "Report.pdf"
        );
    }

    #[test]
    fn a_named_file_is_matched_loosely() {
        let v = found(resolve(&file_slot(), Some("my resume"), &ctx(), None));
        assert_eq!(v.text, "Resume 2026.docx");
        let v = found(resolve(
            &file_slot(),
            Some("the holiday photo"),
            &ctx(),
            None,
        ));
        assert_eq!(v.text, "holiday-photo.jpg");
    }

    #[test]
    fn one_typo_still_matches() {
        let v = found(resolve(&file_slot(), Some("resumee"), &ctx(), None));
        assert_eq!(v.text, "Resume 2026.docx");
    }

    #[test]
    fn two_equally_good_files_make_a_question_not_a_guess() {
        match resolve(&file_slot(), Some("the invoice"), &ctx(), None) {
            Resolution::Choose { options, .. } => {
                let labels: Vec<&str> = options.iter().map(|o| o.1.as_str()).collect();
                assert_eq!(labels.len(), 2);
                assert!(labels.contains(&"Invoice March.pdf"));
                assert!(labels.contains(&"Invoice April.pdf"));
            }
            other => panic!("expected a question, got {other:?}"),
        }
    }

    #[test]
    fn adding_a_word_breaks_the_tie() {
        let v = found(resolve(
            &file_slot(),
            Some("the april invoice"),
            &ctx(),
            None,
        ));
        assert_eq!(v.text, "Invoice April.pdf");
    }

    #[test]
    fn the_latest_download_phrase_uses_recency() {
        let v = found(resolve(
            &file_slot(),
            Some("the latest download"),
            &ctx(),
            None,
        ));
        assert_eq!(v.text, "holiday-photo.jpg");
    }

    #[test]
    fn an_unknown_file_is_not_found_not_guessed() {
        assert_eq!(
            resolve(&file_slot(), Some("tax return"), &ctx(), None),
            Resolution::NotFound(Some("tax return".into()))
        );
    }

    #[test]
    fn this_with_no_selection_and_no_downloads_is_not_found() {
        let r = resolve(&file_slot(), Some("this file"), &NoContext, None);
        assert!(matches!(r, Resolution::NotFound(_)));
    }

    #[test]
    fn named_text_slots_pass_the_words_through() {
        let v = found(resolve(
            &strategies(&["named_app"]),
            Some("Spotify"),
            &NoContext,
            None,
        ));
        assert_eq!(v.text, "Spotify");
        assert_eq!(v.fields["name"], "Spotify");
    }

    #[test]
    fn usual_choice_fills_a_slot_only_when_nothing_was_said() {
        let s = strategies(&["named_contact", "usual_contact"]);
        assert_eq!(
            found(resolve(&s, None, &NoContext, Some("Ved"))).text,
            "Ved"
        );
        assert_eq!(
            found(resolve(&s, Some("Neha"), &NoContext, Some("Ved"))).text,
            "Neha"
        );
        assert!(matches!(
            resolve(&s, None, &NoContext, None),
            Resolution::NotFound(None)
        ));
    }

    #[test]
    fn scores_rank_sensibly() {
        assert!(score("invoice", "Invoice March.pdf") > score("invoice", "Resume.docx"));
        assert_eq!(score("zzz", "Invoice March.pdf"), 0.0);
        assert!(
            score("april invoice", "Invoice April.pdf")
                > score("april invoice", "Invoice March.pdf")
        );
    }
}
