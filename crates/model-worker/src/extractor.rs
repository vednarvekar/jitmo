//! The Tier 0 extractor: a small token-classification model (`ner.rs`, MiniLM-L6 BIO tagger,
//! int8 ONNX, about 65 MB in memory and about 3 ms per request) loaded on first use, with the
//! closed lists the app passes (installed apps, contacts) used to correct what it tags.
//! If the model folder is missing or does not load, the rules alone answer (`rules.rs` /
//! `refine.rs`, under 1 ms, no model memory). The model goes away when the worker exits
//! after idle.

use crate::ner::{ner_dir, NerModel};
use crate::rules::{self, Ent, Gazetteer, ModelEnt};
use ort::environment::Environment;
use std::path::PathBuf;

/// Minimum mean tag probability of a span (int8 lowers probabilities a little).
pub const THRESHOLD: f32 = 0.3;

/// How the model's spans are turned into the final list. [`Mode::shipped`] is what ships;
/// the others exist so `tests/split_eval.rs` can compare them on the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    /// Kinds that come from the rules instead of the model (their model spans are dropped).
    pub rules_for: Vec<&'static str>,
    /// Apply the closed-list correction ([`correct_with_lists`]).
    pub lists: bool,
}

impl Mode {
    /// The model's spans as they are.
    pub fn ner() -> Mode {
        Mode {
            rules_for: Vec::new(),
            lists: false,
        }
    }

    /// What ships: the model's spans plus the closed-list correction. Rules for the closed classes
    /// (dates, durations, numbers, settings) won on DEV (94 to 96%) and lost on TEST (86 to 78%),
    /// so they are not used; see D33.
    pub fn shipped() -> Mode {
        Mode {
            rules_for: Vec::new(),
            lists: true,
        }
    }
}

pub struct LazyExtractor {
    dir: Option<PathBuf>,
    threads: usize,
    model: Option<NerModel>,
    /// Loading failed once; do not try again on every request.
    failed: bool,
    pub mode: Mode,
}

impl LazyExtractor {
    /// `dir` overrides the model folder (else `JITMO_NER_DIR`, else `models/ner`).
    pub fn new(dir: Option<PathBuf>) -> Self {
        let threads = std::env::var("JITMO_EXTRACTOR_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        Self {
            dir,
            threads,
            model: None,
            failed: false,
            mode: Mode::shipped(),
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    /// True when the model could not be found or loaded (rules only).
    pub fn is_rules_only(&self) -> bool {
        self.failed
    }

    fn model(&mut self, env: &Environment) -> Option<&mut NerModel> {
        if self.model.is_none() && !self.failed {
            let loaded = ner_dir(self.dir.as_deref())
                .ok_or_else(|| "no extractor model folder found (models/ner)".to_string())
                .and_then(|d| NerModel::load(env, &d, self.threads));
            match loaded {
                Ok(m) => self.model = Some(m),
                Err(e) => {
                    tracing::warn!("extractor model not available, using rules only: {e}");
                    self.failed = true;
                }
            }
        }
        self.model.as_mut()
    }

    /// Spans for `text`. If the model is missing or fails, the rules alone answer (never an error).
    pub fn extract(&mut self, env: &Environment, text: &str, gaz: &Gazetteer) -> Vec<Ent> {
        let mode = self.mode.clone();
        let Some(m) = self.model(env) else {
            return rules::extract_v2(text, &[], THRESHOLD, gaz);
        };
        let raw = match m.predict(text, THRESHOLD) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("extractor model failed, using rules only: {e}");
                return rules::extract_v2(text, &[], THRESHOLD, gaz);
            }
        };
        combine(text, raw, gaz, &mode)
    }
}

/// Turn the model's spans (and, depending on `mode`, the rules and closed lists) into the final list.
pub fn combine(text: &str, raw: Vec<ModelEnt>, gaz: &Gazetteer, mode: &Mode) -> Vec<Ent> {
    let mut out: Vec<Ent> = raw
        .into_iter()
        .filter(|e| !mode.rules_for.contains(&e.kind))
        .map(|e| Ent {
            kind: e.kind,
            text: e.text,
            start: e.start,
            end: e.end,
        })
        .collect();
    if !mode.rules_for.is_empty() {
        // A rule span inside what the model took for a message or query is part of that
        // text ("tell Neha to call me tomorrow"), not a slot of its own.
        let free: Vec<(usize, usize)> = out
            .iter()
            .filter(|e| matches!(e.kind, "message" | "query"))
            .map(|e| (e.start, e.end))
            .collect();
        out.extend(
            rules::extract_v2(text, &[], THRESHOLD, gaz)
                .into_iter()
                .filter(|e| mode.rules_for.contains(&e.kind))
                .filter(|e| !free.iter().any(|&(a, b)| a <= e.start && e.end <= b)),
        );
        out.sort_by_key(|e| (e.start, e.end));
    }
    if mode.lists {
        out = correct_with_lists(out, gaz);
    }
    out
}

/// Kinds whose span is replaced by a known name it contains.
const SNAP_KINDS: &[&str] = &["contact", "app", "website", "place"];
/// Kinds that are only retyped when the whole span is the known name.
const EXACT_KINDS: &[&str] = &["media", "file", "folder", "event", "query"];

/// Closed-list correction. A span the model typed as contact, app, website or place that
/// contains a known app or contact name (whole words, any case) becomes that name, with that
/// type, snapped to the name. A span of another open kind (media, file, folder, event, query) that
/// is exactly a known name is retyped. Names the model did not tag are not added (measured: it
/// made DEV worse). Nothing is done with an empty list.
pub fn correct_with_lists(ents: Vec<Ent>, gaz: &Gazetteer) -> Vec<Ent> {
    if gaz.apps.is_empty() && gaz.contacts.is_empty() {
        return ents;
    }
    let mut names: Vec<(&'static str, &str)> = gaz
        .contacts
        .iter()
        .map(|n| ("contact", n.as_str()))
        .chain(gaz.apps.iter().map(|n| ("app", n.as_str())))
        .filter(|(_, n)| !n.trim().is_empty())
        .collect();
    // Longest first, so "Zoya Khan" wins over "Zoya".
    names.sort_by_key(|(_, n)| std::cmp::Reverse(n.chars().count()));

    let mut out: Vec<Ent> = Vec::new();
    for e in ents {
        let hit = names
            .iter()
            .find_map(|(kind, n)| find_word(&e.text, n).map(|(a, b)| (*kind, a, b)));
        match hit {
            Some((kind, a, b)) if SNAP_KINDS.contains(&e.kind) => out.push(Ent {
                kind,
                text: e.text[a..b].to_string(),
                start: e.start + a,
                end: e.start + b,
            }),
            Some((kind, a, b)) if EXACT_KINDS.contains(&e.kind) && a == 0 && b == e.text.len() => {
                out.push(Ent { kind, ..e })
            }
            _ => out.push(e),
        }
    }
    out.sort_by_key(|e| (e.start, e.end));
    out
}

/// First whole-word, case-insensitive occurrence of `name` in `text` (byte range).
fn find_word(text: &str, name: &str) -> Option<(usize, usize)> {
    let name: Vec<char> = name.trim().chars().flat_map(char::to_lowercase).collect();
    if name.is_empty() {
        return None;
    }
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    for (i, _) in text.char_indices() {
        if word(text[..i].chars().next_back()) {
            continue;
        }
        let mut k = 0;
        let mut end = i;
        for c in text[i..].chars() {
            if k >= name.len() {
                break;
            }
            let mut low = c.to_lowercase();
            let ok = low.all(|l| {
                let hit = name.get(k) == Some(&l);
                k += 1;
                hit
            });
            if !ok {
                k = usize::MAX;
                break;
            }
            end += c.len_utf8();
        }
        if k == name.len() && !word(text[end..].chars().next()) {
            return Some((i, end));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(kind: &'static str, text: &str, start: usize) -> Ent {
        Ent {
            kind,
            text: text.to_string(),
            start,
            end: start + text.len(),
        }
    }

    fn gaz() -> Gazetteer {
        Gazetteer {
            apps: vec!["Obsidian".into(), "Google Chrome".into()],
            contacts: vec!["Zoya Khan".into(), "Mom".into()],
            files: vec![],
        }
    }

    #[test]
    fn finds_whole_words_in_any_case() {
        assert_eq!(
            find_word("send it to zoya khan now", "Zoya Khan"),
            Some((11, 20))
        );
        assert_eq!(find_word("my password", "word"), None);
        assert_eq!(find_word("open Word", "word"), Some((5, 9)));
        assert_eq!(find_word("", "x"), None);
    }

    #[test]
    fn a_span_containing_a_known_name_is_snapped_and_retyped() {
        let text = "open the obsidian app";
        let got = correct_with_lists(vec![ent("website", "the obsidian app", 5)], &gaz());
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].kind, got[0].text.as_str()), ("app", "obsidian"));
        assert_eq!(&text[got[0].start..got[0].end], "obsidian");
    }

    #[test]
    fn exact_names_retype_other_open_kinds_but_containing_ones_do_not() {
        let got = correct_with_lists(
            vec![ent("file", "the Mom photos", 5), ent("media", "Mom", 23)],
            &gaz(),
        );
        assert_eq!(got[0].kind, "file");
        assert_eq!(got[1].kind, "contact");
    }

    #[test]
    fn a_name_the_model_missed_is_not_added() {
        assert!(correct_with_lists(vec![], &gaz()).is_empty());
    }

    #[test]
    fn empty_lists_change_nothing() {
        let v = vec![ent("contact", "Ved", 0)];
        assert_eq!(correct_with_lists(v.clone(), &Gazetteer::default()), v);
    }
}
