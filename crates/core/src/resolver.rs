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
//! - `named_app`, `named_contact`  the words matched against the closed lists in
//!   [`Known`] (installed apps, saved contacts): the real entry, or a question when
//!   two fit; a name on no list passes through as said
//! - `named_*`        (other types) the words as said
//! - `usual_*`        the user's usual choice from the settings (`usual.<type>`),
//!   used when nothing was said
//!
//! If several files fit about equally well the result is [`Resolution::Choose`],
//! and Core asks one short question with the options (FR4). A tie is never
//! settled by guessing.

use crate::known::{self, Known, Match};
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

    /// A person. `name` is the full name; `label` is what a confirm card shows: the
    /// name, with the number after it when it is known ("Ved Narvekar (+91 98765 43210)").
    pub fn from_person(name: &str, phone: Option<&str>) -> Self {
        let label = match phone {
            Some(p) if !p.trim().is_empty() => format!("{name} ({})", p.trim()),
            _ => name.to_string(),
        };
        let mut fields = BTreeMap::from([
            ("name".to_string(), name.to_string()),
            ("label".to_string(), label),
        ]);
        if let Some(p) = phone.filter(|p| !p.trim().is_empty()) {
            fields.insert("phone".to_string(), p.trim().to_string());
        }
        SlotValue {
            text: name.to_string(),
            fields,
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

    /// Several files at once ("these PDFs"). `text` and `name` read "3 files",
    /// `path` is the first file and `paths` holds all of them, one per line (a
    /// newline cannot appear in a Windows path). Use [`SlotValue::paths`] to read them.
    pub fn from_files(files: &[FileRef]) -> Self {
        let text = format!("{} files", files.len());
        let joined: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        SlotValue {
            text: text.clone(),
            fields: BTreeMap::from([
                ("name".to_string(), text),
                (
                    "path".to_string(),
                    files.first().map(|f| f.path.clone()).unwrap_or_default(),
                ),
                ("paths".to_string(), joined.join("\n")),
            ]),
        }
    }

    /// The files of a multi value; empty for any other value.
    pub fn paths(&self) -> Vec<&str> {
        self.fields
            .get("paths")
            .map(|p| p.split('\n').filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
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
    /// Every file selected when the bar was opened (default: the one file, if any).
    fn selected_files(&self) -> Vec<FileRef> {
        self.selected_file().into_iter().collect()
    }
    /// Newest downloads first.
    fn recent_downloads(&self, limit: usize) -> Vec<FileRef>;
    /// Every file that `named_file` may match against.
    fn searchable_files(&self) -> Vec<FileRef>;
    /// Every folder that `named_folder` may match against (default: none).
    fn searchable_folders(&self) -> Vec<FileRef> {
        Vec::new()
    }
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
    resolve_with(strategies, said, ctx, usual, &Known::default())
}

/// Like [`resolve`], with the closed lists of installed apps and saved contacts.
pub fn resolve_with(
    strategies: &[String],
    said: Option<&str>,
    ctx: &dyn ContextSource,
    usual: Option<&str>,
    known: &Known,
) -> Resolution {
    let said = said.map(str::trim).filter(|s| !s.is_empty());
    let deictic = said.is_none_or(is_deictic);
    let recency = said.is_some_and(is_recency);
    let plural = said.and_then(plural_selection);
    for strategy in strategies {
        match strategy.as_str() {
            "selected_file" if plural.is_some() => {
                let exts = plural.as_deref().unwrap_or_default();
                let mut files = ctx.selected_files();
                if !exts.is_empty() {
                    files.retain(|f| {
                        let ext = f.name.rsplit_once('.').map_or("", |(_, e)| e);
                        exts.iter().any(|x| x.eq_ignore_ascii_case(ext))
                    });
                }
                match files.len() {
                    0 => {}
                    1 => return Resolution::Found(SlotValue::from_file(&files[0])),
                    _ => return Resolution::Found(SlotValue::from_files(&files)),
                }
            }
            "selected_file" if deictic => {
                if let Some(f) = ctx.selected_file() {
                    return Resolution::Found(SlotValue::from_file(&f));
                }
            }
            "named_folder" => {
                if let (Some(words), false) = (said, deictic) {
                    if let Some(r) = match_named_file(words, &ctx.searchable_folders()) {
                        return r;
                    }
                }
            }
            "named_file" => {
                // "these pdfs" points at the selection; it never names a file.
                if let (Some(words), false, false, false) =
                    (said, deictic, recency, plural.is_some())
                {
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
            s if s.starts_with("named_") && s != "named_file" && s != "named_folder" => {
                if let Some(words) = said {
                    return closed_list(s, words, known);
                }
            }
            s if s.starts_with("usual_") && said.is_none() => {
                if let Some(v) = usual {
                    return match s {
                        // A usual contact gets their number too, when it is known.
                        "usual_contact" => match known::match_contact(v, &known.contacts) {
                            Match::One(c) => Resolution::Found(contact_value(&c)),
                            _ => Resolution::Found(SlotValue::from_person(v, None)),
                        },
                        _ => Resolution::Found(SlotValue::from_text(v)),
                    };
                }
            }
            _ => {}
        }
    }
    Resolution::NotFound(said.map(str::to_string))
}

fn contact_value(c: &jitmo_store::Contact) -> SlotValue {
    SlotValue::from_person(&c.name, c.phone.as_deref())
}

/// `named_app` and `named_contact`: match the words against the closed list. A name
/// on the list becomes the real entry; several that fit make one question (T94); a
/// name that is on no list passes through as said, because the lists are learned and
/// the connector still checks it (and every send is confirmed).
fn closed_list(strategy: &str, words: &str, known: &Known) -> Resolution {
    match strategy {
        "named_contact" => match known::match_contact(words, &known.contacts) {
            Match::One(c) => Resolution::Found(contact_value(&c)),
            Match::Many(cs) => Resolution::Choose {
                question: "Which one?".to_string(),
                options: cs
                    .iter()
                    .take(MAX_OPTIONS)
                    .map(|c| {
                        (
                            c.name.clone(),
                            contact_value(c).fields["label"].clone(),
                            contact_value(c),
                        )
                    })
                    .collect(),
            },
            Match::None => Resolution::Found(SlotValue::from_person(words, None)),
        },
        "named_app" => match known::match_app(words, &known.apps) {
            Match::One(a) => Resolution::Found(SlotValue::from_text(&a)),
            Match::Many(apps) => Resolution::Choose {
                question: "Which one?".to_string(),
                options: apps
                    .iter()
                    .take(MAX_OPTIONS)
                    .map(|a| (a.clone(), a.clone(), SlotValue::from_text(a)))
                    .collect(),
            },
            Match::None => Resolution::Found(SlotValue::from_text(words)),
        },
        _ => Resolution::Found(SlotValue::from_text(words)),
    }
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

/// "this", "this file", "that one", "selected", "it": only pointing words and
/// generic nouns, nothing that names the file. "that budget thing" names it.
fn is_deictic(words: &str) -> bool {
    let w = words.to_lowercase();
    let mut any_pointer = false;
    for t in tokens(&w) {
        match t.as_str() {
            "this" | "that" | "it" | "these" | "those" | "selected" | "current" => {
                any_pointer = true
            }
            "file" | "files" | "document" | "doc" | "one" | "ones" | "thing" | "the" => {}
            _ => return false,
        }
    }
    any_pointer
}

/// "these", "these pdfs", "the selected files", "them": pointing words (at least
/// one plural) plus generic nouns and file-type words. Gives the extensions the
/// type words allow (empty = any type), or `None` if the words are anything else
/// or do not point at several files.
fn plural_selection(words: &str) -> Option<Vec<&'static str>> {
    let mut pointer = false;
    let mut plural = false;
    let mut exts: Vec<&'static str> = Vec::new();
    for t in tokens(words) {
        match t.as_str() {
            "these" | "those" | "them" => {
                pointer = true;
                plural = true;
            }
            "selected" | "current" | "this" | "that" => pointer = true,
            "files" | "documents" | "docs" | "ones" => plural = true,
            "file" | "document" | "doc" | "one" | "thing" | "the" | "all" | "of" => {}
            w => {
                let (stem, s) = match w.strip_suffix('s') {
                    Some(stem) if w.len() > 3 => (stem, true),
                    _ => (w, false),
                };
                exts.extend(type_extensions(stem)?);
                plural |= s;
            }
        }
    }
    (pointer && plural).then_some(exts)
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

/// Words that say nothing about which file is meant.
const STOPWORDS: [&str; 20] = [
    "the",
    "my",
    "a",
    "an",
    "of",
    "file",
    "files",
    "from",
    "last",
    "week",
    "yesterday",
    "today",
    "that",
    "thing",
    "one",
    "new",
    "old",
    "latest",
    "wheres",
    "where",
];

/// "sheet" means a spreadsheet, "deck" means slides, and so on: they match the
/// file type instead of the name.
fn type_extensions(word: &str) -> Option<&'static [&'static str]> {
    Some(match word {
        "sheet" | "spreadsheet" | "excel" => &["xlsx", "xls", "csv", "ods"],
        "deck" | "slides" | "presentation" | "powerpoint" => &["pptx", "ppt", "odp", "key"],
        "doc" | "document" | "word" => &["docx", "doc", "odt", "rtf"],
        "pdf" => &["pdf"],
        "photo" | "picture" | "pic" | "image" | "screenshot" => {
            &["jpg", "jpeg", "png", "heic", "gif", "webp"]
        }
        "video" | "vid" | "movie" => &["mp4", "mov", "mkv", "avi", "webm"],
        "song" | "music" | "audio" => &["mp3", "wav", "flac", "m4a"],
        _ => return None,
    })
}

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
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default();
    let total: f32 = want
        .iter()
        .map(|w| {
            // "pdfs" is "pdf"; a file-type word matches the extension.
            let w = if w.len() > 3 && w.ends_with('s') {
                &w[..w.len() - 1]
            } else {
                w.as_str()
            };
            if let Some(exts) = type_extensions(w) {
                return if exts.contains(&ext.as_str()) {
                    1.0
                } else {
                    0.0
                };
            }
            have.iter().map(|h| word_score(w, h)).fold(0.0f32, f32::max)
        })
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
        /// Several selected files; when set, these are the selection.
        many: Vec<FileRef>,
        files: Vec<FileRef>,
    }

    impl ContextSource for Fixed {
        fn selected_file(&self) -> Option<FileRef> {
            self.many.first().cloned().or_else(|| self.selected.clone())
        }
        fn selected_files(&self) -> Vec<FileRef> {
            if self.many.is_empty() {
                self.selected.iter().cloned().collect()
            } else {
                self.many.clone()
            }
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
            many: Vec::new(),
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

    fn three_selected() -> Fixed {
        let mut c = ctx();
        c.many = vec![file("a.pdf", 1), file("b.PDF", 2), file("notes.docx", 3)];
        c
    }

    #[test]
    fn these_with_several_selected_is_one_multi_value() {
        for words in ["these", "the selected files", "them", "these files"] {
            let v = found(resolve(&file_slot(), Some(words), &three_selected(), None));
            assert_eq!(v.text, "3 files", "{words}");
            assert_eq!(v.fields["name"], "3 files");
            assert_eq!(v.fields["path"], "C:\\Users\\me\\Documents\\a.pdf");
            assert_eq!(
                v.paths(),
                [
                    "C:\\Users\\me\\Documents\\a.pdf",
                    "C:\\Users\\me\\Documents\\b.PDF",
                    "C:\\Users\\me\\Documents\\notes.docx"
                ]
            );
        }
    }

    #[test]
    fn these_pdfs_keeps_only_the_selected_pdfs() {
        let v = found(resolve(
            &file_slot(),
            Some("these pdfs"),
            &three_selected(),
            None,
        ));
        assert_eq!(v.text, "2 files");
        assert_eq!(v.paths().len(), 2);
        // Only one pdf left: it is just that file.
        let mut c = three_selected();
        c.many.remove(0);
        let v = found(resolve(&file_slot(), Some("these pdfs"), &c, None));
        assert_eq!(v.text, "b.PDF");
        assert!(!v.fields.contains_key("paths"));
        // No pdf at all: not found, and never a fuzzy name match.
        let mut c = three_selected();
        c.many = vec![file("notes.docx", 3)];
        assert!(matches!(
            resolve(&file_slot(), Some("these pdfs"), &c, None),
            Resolution::NotFound(_)
        ));
    }

    #[test]
    fn single_selection_and_singular_words_are_unchanged() {
        let mut c = ctx();
        c.selected = Some(file("Report.pdf", 1));
        for words in ["this", "this file", "these", "them"] {
            let v = found(resolve(&file_slot(), Some(words), &c, None));
            assert_eq!(v.text, "Report.pdf", "{words}");
            assert!(!v.fields.contains_key("paths"));
        }
        // Singular words with several selected: the first, as before.
        let v = found(resolve(
            &file_slot(),
            Some("this file"),
            &three_selected(),
            None,
        ));
        assert_eq!(v.text, "a.pdf");
    }

    #[test]
    fn plural_selection_words() {
        assert_eq!(plural_selection("these pdfs"), Some(vec!["pdf"]));
        assert_eq!(plural_selection("these"), Some(vec![]));
        assert_eq!(plural_selection("this file"), None);
        assert_eq!(plural_selection("the invoices"), None);
        assert_eq!(plural_selection("my budget files"), None);
        assert_eq!(plural_selection("pdfs"), None, "no pointing word");
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
    fn file_type_words_match_the_extension_and_time_words_are_ignored() {
        let mut c = ctx();
        c.files.push(file("Budget 2026.xlsx", 400));
        c.files.push(file("Budget notes.docx", 10));
        let v = found(resolve(
            &file_slot(),
            Some("the budget sheet from last week"),
            &c,
            None,
        ));
        assert_eq!(v.text, "Budget 2026.xlsx");
        // Two budget files and no type word: a question, not a guess.
        assert!(matches!(
            resolve(&file_slot(), Some("that budget thing"), &c, None),
            Resolution::Choose { .. }
        ));
    }

    #[test]
    fn that_thing_alone_is_a_pointer_but_a_named_thing_is_not() {
        assert!(is_deictic("this file"));
        assert!(is_deictic("that one"));
        assert!(is_deictic("these"));
        assert!(!is_deictic("that budget thing"));
        assert!(!is_deictic("the invoice"));
    }

    #[test]
    fn plural_type_words_still_match() {
        assert!(score("pdfs", "Invoice March.pdf") >= 0.9);
    }

    #[test]
    fn a_folder_is_found_by_name() {
        struct Dirs;
        impl ContextSource for Dirs {
            fn selected_file(&self) -> Option<FileRef> {
                None
            }
            fn recent_downloads(&self, _: usize) -> Vec<FileRef> {
                Vec::new()
            }
            fn searchable_files(&self) -> Vec<FileRef> {
                Vec::new()
            }
            fn searchable_folders(&self) -> Vec<FileRef> {
                vec![file("Invoices", 0), file("Receipts", 0)]
            }
        }
        let s = strategies(&["named_folder"]);
        assert_eq!(
            found(resolve(&s, Some("invoices"), &Dirs, None)).text,
            "Invoices"
        );
        assert!(matches!(
            resolve(&s, Some("taxes"), &Dirs, None),
            Resolution::NotFound(_)
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
