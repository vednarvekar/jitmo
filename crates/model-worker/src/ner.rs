//! Tier 0 extractor model: a small fine-tuned encoder (BIO tags over word pieces, int8 ONNX)
//! run with ONNX Runtime and the `tokenizers` crate. Its spans ([`ModelEnt`]) are the extractor's
//! answer (see `extractor.rs`); the rules in `rules.rs` / `refine.rs` are only the fallback.
//! Span kind names are the extractor's (`file`, `message`, ...); `rules::to_slot_span` maps them to
//! the slot types skills use.
//!
//! Files in the model folder (written by `tools/extractor_ner/export_onnx.py`):
//! `model_int8.onnx` (inputs `input_ids`, `attention_mask`, `token_type_ids`, int64 `[1, n]`;
//! output `logits` `[1, n, labels]`), `tokenizer.json`, `labels.json` (`["O", "B-app", "I-app", ...]`).
//!
//! Decoding: argmax per word piece, contiguous `B-x I-x ...` runs become one span, special
//! tokens are skipped, spans are snapped outwards to whole words, and the score of a span is the
//! mean softmax probability of its tags.

use crate::rules::ModelEnt;
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use tokenizers::{Tokenizer, TruncationParams};

/// Span types the model can tag (must match `ner_common.TYPES` in the Python tools).
pub const KINDS: &[&str] = &[
    "app",
    "contact",
    "date_time",
    "duration",
    "event",
    "file",
    "folder",
    "media",
    "message",
    "number",
    "place",
    "query",
    "setting",
    "website",
];

const MAX_TOKENS: usize = 64;

/// One word-piece tag: `None` is "O"; `Some((begins, kind))` otherwise.
type Tag = Option<(bool, &'static str)>;

pub struct NerModel {
    session: Session,
    tokenizer: Tokenizer,
    tags: Vec<Tag>,
}

/// Where the model files are looked for: `JITMO_NER_DIR`, else `models/ner` next to the
/// program, else `models/ner` in the working directory.
pub fn ner_dir(arg: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(arg.map(Path::to_path_buf));
    candidates.extend(std::env::var_os("JITMO_NER_DIR").map(PathBuf::from));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("models").join("ner"));
        }
    }
    candidates.push(PathBuf::from("models/ner"));
    candidates
        .into_iter()
        .find(|d| d.join("model_int8.onnx").exists())
}

/// Turn the label names from `labels.json` into tags.
pub fn parse_labels(names: &[String]) -> Result<Vec<Tag>, String> {
    names
        .iter()
        .map(|n| {
            if n == "O" {
                return Ok(None);
            }
            let (prefix, kind) = n
                .split_once('-')
                .ok_or_else(|| format!("bad label {n:?}"))?;
            let kind = KINDS
                .iter()
                .find(|k| **k == kind)
                .ok_or_else(|| format!("unknown span type in label {n:?}"))?;
            match prefix {
                "B" => Ok(Some((true, *kind))),
                "I" => Ok(Some((false, *kind))),
                _ => Err(format!("bad label {n:?}")),
            }
        })
        .collect()
}

/// A token's tag and its probability.
#[derive(Debug, Clone, Copy)]
pub struct TokenTag {
    pub tag: Tag,
    pub prob: f32,
}

/// Softmax over `logits` (`[n, labels]`, row-major) and argmax per row.
pub fn tag_tokens(logits: &[f32], labels: usize, tags: &[Tag]) -> Vec<TokenTag> {
    logits
        .chunks_exact(labels)
        .map(|row| {
            let max = row.iter().cloned().fold(f32::MIN, f32::max);
            let sum: f32 = row.iter().map(|x| (x - max).exp()).sum();
            let (best, _) = row
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .expect("row has labels");
            TokenTag {
                tag: tags[best],
                prob: 1.0 / sum,
            }
        })
        .collect()
}

/// Group token tags into spans. `offsets` are byte offsets of each token in `text`
/// (`(0, 0)` for special tokens). Spans are snapped outwards to whole words (a word is a
/// run of alphanumerics) and filtered by mean probability >= `min_score`.
pub fn decode(
    text: &str,
    offsets: &[(usize, usize)],
    tags: &[TokenTag],
    min_score: f32,
) -> Vec<ModelEnt> {
    struct Cur {
        kind: &'static str,
        start: usize,
        end: usize,
        probs: Vec<f32>,
    }
    let mut spans: Vec<Cur> = Vec::new();
    let mut cur: Option<Cur> = None;
    for (&(a, b), t) in offsets.iter().zip(tags) {
        if a == b {
            continue;
        }
        match t.tag {
            None => spans.extend(cur.take()),
            Some((begins, kind)) => match cur.as_mut() {
                Some(c) if !begins && c.kind == kind => {
                    c.end = b;
                    c.probs.push(t.prob);
                }
                _ => {
                    spans.extend(cur.take());
                    cur = Some(Cur {
                        kind,
                        start: a,
                        end: b,
                        probs: vec![t.prob],
                    });
                }
            },
        }
    }
    spans.extend(cur.take());
    let alnum = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    spans
        .into_iter()
        .filter_map(|mut s| {
            while alnum(text[..s.start].chars().next_back())
                && alnum(text[s.start..].chars().next())
            {
                s.start -= text[..s.start]
                    .chars()
                    .next_back()
                    .map_or(1, char::len_utf8);
            }
            while alnum(text[s.end..].chars().next()) && alnum(text[..s.end].chars().next_back()) {
                s.end += text[s.end..].chars().next().map_or(1, char::len_utf8);
            }
            let score = s.probs.iter().sum::<f32>() / s.probs.len() as f32;
            (score >= min_score).then(|| ModelEnt {
                kind: s.kind,
                text: text[s.start..s.end].to_string(),
                start: s.start,
                end: s.end,
                score,
            })
        })
        .collect()
}

impl NerModel {
    pub fn load(env: &Environment, dir: &Path, threads: usize) -> Result<Self, String> {
        let mut builder = Session::builder(env).map_err(|e| format!("ner model: {e}"))?;
        builder = builder
            .with_intra_threads(threads.max(1))
            .map_err(|e| format!("ner model: {e}"))?;
        let session = builder
            .commit_from_file(dir.join("model_int8.onnx"))
            .map_err(|e| format!("ner model: {e}"))?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| format!("ner tokenizer: {e}"))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| format!("ner tokenizer: {e}"))?;
        let names: Vec<String> = serde_json::from_slice(
            &std::fs::read(dir.join("labels.json")).map_err(|e| format!("ner labels: {e}"))?,
        )
        .map_err(|e| format!("ner labels: {e}"))?;
        Ok(Self {
            session,
            tokenizer,
            tags: parse_labels(&names)?,
        })
    }

    /// Spans the model tags in `text` whose mean tag probability is at least `min_score`,
    /// in text order. Offsets are byte offsets into `text`. The tags are already
    /// a decision, so there is no per-type threshold to tune.
    pub fn predict(&mut self, text: &str, min_score: f32) -> Result<Vec<ModelEnt>, String> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("tokenize: {e}"))?;
        let n = enc.get_ids().len();
        let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
        let mask: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
        let types = vec![0i64; n];
        let shape = [1usize, n];
        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => Tensor::from_array((shape, ids)).map_err(|e| e.to_string())?,
                "attention_mask" => Tensor::from_array((shape, mask)).map_err(|e| e.to_string())?,
                "token_type_ids" => Tensor::from_array((shape, types)).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("ner run: {e}"))?;
        let (dims, logits) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("ner output: {e}"))?;
        let labels = *dims.last().ok_or("empty output")? as usize;
        if labels != self.tags.len() || logits.len() != n * labels {
            return Err("ner output does not match labels.json".into());
        }
        let tagged = tag_tokens(logits, labels, &self.tags);
        Ok(decode(text, enc.get_offsets(), &tagged, min_score))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Vec<Tag> {
        let mut names = vec!["O".to_string()];
        for k in KINDS {
            names.push(format!("B-{k}"));
            names.push(format!("I-{k}"));
        }
        parse_labels(&names).unwrap()
    }

    fn tt(tag: &str, p: f32) -> TokenTag {
        let names = [tag.to_string()];
        TokenTag {
            tag: parse_labels(&names).unwrap()[0],
            prob: p,
        }
    }

    #[test]
    fn parses_label_names() {
        let tags = labels();
        assert_eq!(tags.len(), 1 + 2 * KINDS.len());
        assert_eq!(tags[0], None);
        assert_eq!(tags[1], Some((true, "app")));
        assert_eq!(tags[2], Some((false, "app")));
        assert!(parse_labels(&["B-nope".to_string()]).is_err());
        assert!(parse_labels(&["X-app".to_string()]).is_err());
    }

    #[test]
    fn softmax_argmax_picks_the_largest_logit() {
        let tags = labels();
        let l = tags.len();
        let mut logits = vec![0.0f32; 2 * l];
        logits[3] = 5.0; // token 0 -> B-contact? index 3 is B-contact
        logits[l] = 4.0; // token 1 -> O
        let t = tag_tokens(&logits, l, &tags);
        assert_eq!(t[0].tag, Some((true, "contact")));
        assert!(t[0].prob > 0.8 && t[0].prob < 1.0);
        assert_eq!(t[1].tag, None);
    }

    #[test]
    fn decode_joins_pieces_and_snaps_to_words() {
        // "tell priya hello": tokens [CLS] tell priya hello [SEP]; the model tags only "pri" of "priya"
        let text = "tell priya hello";
        let offsets = [(0, 0), (0, 4), (5, 8), (8, 10), (11, 16), (0, 0)];
        let tags = [
            tt("O", 1.0),
            tt("O", 1.0),
            tt("B-contact", 0.9),
            tt("I-contact", 0.7),
            tt("B-message", 0.8),
            tt("O", 1.0),
        ];
        let got = decode(text, &offsets, &tags, 0.0);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].kind, got[0].text.as_str()), ("contact", "priya"));
        assert_eq!((got[0].start, got[0].end), (5, 10));
        assert!((got[0].score - 0.8).abs() < 1e-6);
        assert_eq!((got[1].kind, got[1].text.as_str()), ("message", "hello"));
    }

    #[test]
    fn decode_splits_on_a_new_begin_and_on_a_type_change() {
        let text = "play a by b";
        let offsets = [(0, 0), (5, 6), (7, 9), (10, 11), (0, 0)];
        let tags = [
            tt("O", 1.0),
            tt("B-media", 0.9),
            tt("O", 1.0),
            tt("B-media", 0.9),
            tt("O", 1.0),
        ];
        let got = decode(text, &offsets, &tags, 0.0);
        assert_eq!(got.len(), 2);
        // I- of another type starts a new span
        let tags = [
            tt("O", 1.0),
            tt("B-media", 0.9),
            tt("I-contact", 0.9),
            tt("O", 1.0),
            tt("O", 1.0),
        ];
        let got = decode(text, &offsets, &tags, 0.0);
        assert_eq!(
            got.iter().map(|e| e.kind).collect::<Vec<_>>(),
            ["media", "contact"]
        );
    }

    #[test]
    fn decode_drops_low_scores_and_handles_multibyte_text() {
        let text = "ping José now";
        // tokens: ping | Jos | ##é | now
        let offsets = [(0, 0), (0, 4), (5, 8), (8, 10), (11, 14), (0, 0)];
        let tags = [
            tt("O", 1.0),
            tt("O", 1.0),
            tt("B-contact", 0.4),
            tt("I-contact", 0.4),
            tt("O", 1.0),
            tt("O", 1.0),
        ];
        assert!(decode(text, &offsets, &tags, 0.5).is_empty());
        let got = decode(text, &offsets, &tags, 0.3);
        assert_eq!(got[0].text, "José");
    }
}
