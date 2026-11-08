//! Tier 0 extractor, model half: GLiNER-small (int8 ONNX) run with ONNX Runtime and
//! the `tokenizers` crate, no Python and no torch. A port of the `gliner` package's
//! span mode (version 0.2.30), checked against its output in the tests.
//!
//! Input format (span mode): the prompt is `<<ENT>> label` for every label, then
//! `<<SEP>>`, then the words of the text. Each word is tokenised on its own; the
//! first sub-token of every text word is marked in `words_mask` (1-based). The model
//! scores every span of up to `max_width` words for every label; scores go through a
//! sigmoid, spans above the threshold are kept, and overlapping spans are removed
//! greedily by score.
//!
//! Files in the model folder: `model_int8.onnx` (or `onnx/model_int8.onnx`),
//! `tokenizer.json`, and optionally `gliner_config.json` (for `max_width`, else 12).

use crate::rules::ModelEnt;
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use regex::Regex;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

/// Span type name and the label text the model sees for it. The wording was chosen
/// in SP2 (label set "a") and is part of the measured accuracy: do not reword it
/// without re-scoring.
pub const LABELS: &[(&str, &str)] = &[
    ("app", "application"),
    ("contact", "person or group"),
    ("file", "file or document"),
    ("folder", "folder"),
    ("media", "song, artist or video creator"),
    ("website", "website"),
    ("query", "search query"),
    ("message", "message text"),
    ("place", "place"),
    ("event", "meeting or event"),
    ("setting", "system setting"),
];

const ENT_TOKEN: &str = "<<ENT>>";
const SEP_TOKEN: &str = "<<SEP>>";
/// GLiNER's own cap on the number of words per text.
const MAX_WORDS: usize = 384;

pub struct Gliner {
    session: Session,
    tokenizer: Tokenizer,
    max_width: usize,
    words: Regex,
    cls: i64,
    sep: i64,
}

/// Where the model files are looked for: `JITMO_EXTRACTOR_DIR`, else `models/extractor`
/// next to the program, else `models/extractor` in the working directory.
pub fn extractor_dir(arg: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(arg.map(Path::to_path_buf));
    candidates.extend(std::env::var_os("JITMO_EXTRACTOR_DIR").map(PathBuf::from));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("models").join("extractor"));
        }
    }
    candidates.push(PathBuf::from("models/extractor"));
    candidates.into_iter().find(|d| model_file(d).is_some())
}

fn model_file(dir: &Path) -> Option<PathBuf> {
    ["model_int8.onnx", "onnx/model_int8.onnx"]
        .iter()
        .map(|f| dir.join(f))
        .find(|p| p.exists())
}

impl Gliner {
    pub fn load(env: &Environment, dir: &Path, threads: usize) -> Result<Self, String> {
        let file = model_file(dir).ok_or("extractor model not found")?;
        let mut builder = Session::builder(env).map_err(|e| format!("extractor model: {e}"))?;
        builder = builder
            .with_intra_threads(threads.max(1))
            .map_err(|e| format!("extractor model: {e}"))?;
        let session = builder
            .commit_from_file(file)
            .map_err(|e| format!("extractor model: {e}"))?;
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| format!("extractor tokenizer: {e}"))?;
        let max_width = std::fs::read(dir.join("gliner_config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("max_width").and_then(|w| w.as_u64()))
            .map_or(12, |w| w as usize);
        let cls = tokenizer.token_to_id("[CLS]").ok_or("no [CLS] token")? as i64;
        let sep = tokenizer.token_to_id("[SEP]").ok_or("no [SEP] token")? as i64;
        Ok(Self {
            session,
            tokenizer,
            max_width,
            words: Regex::new(r"\w+(?:[-_]\w+)*|\S").map_err(|e| e.to_string())?,
            cls,
            sep,
        })
    }

    /// Spans the model finds in `text` above `threshold` (scores strictly greater),
    /// non-overlapping, in text order. Offsets are byte offsets into `text`.
    pub fn predict(&mut self, text: &str, threshold: f32) -> Result<Vec<ModelEnt>, String> {
        let words: Vec<(usize, usize)> = self
            .words
            .find_iter(text)
            .map(|m| (m.start(), m.end()))
            .take(MAX_WORDS)
            .collect();
        let n = words.len();
        if n == 0 {
            return Ok(Vec::new());
        }

        // Prompt words, then the text words.
        let mut prompt: Vec<&str> = Vec::new();
        for (_, label) in LABELS {
            prompt.push(ENT_TOKEN);
            prompt.push(label);
        }
        prompt.push(SEP_TOKEN);
        let mut input_ids: Vec<i64> = vec![self.cls];
        let mut words_mask: Vec<i64> = vec![0];
        let text_words = words.iter().map(|&(s, e)| &text[s..e]);
        for (i, w) in prompt.iter().copied().chain(text_words).enumerate() {
            let enc = self
                .tokenizer
                .encode(w, false)
                .map_err(|e| format!("tokenize: {e}"))?;
            for (j, id) in enc.get_ids().iter().enumerate() {
                input_ids.push(*id as i64);
                let first_of_text_word = j == 0 && i >= prompt.len();
                words_mask.push(if first_of_text_word {
                    (i - prompt.len() + 1) as i64
                } else {
                    0
                });
            }
        }
        input_ids.push(self.sep);
        words_mask.push(0);
        let t = input_ids.len();

        let k = self.max_width;
        let mut span_idx: Vec<i64> = Vec::with_capacity(n * k * 2);
        let mut span_mask: Vec<bool> = Vec::with_capacity(n * k);
        for s in 0..n {
            for w in 0..k {
                span_idx.push(s as i64);
                span_idx.push((s + w) as i64);
                span_mask.push(s + w < n);
            }
        }
        let e = |e: ort::Error| format!("extractor run: {e}");
        let outputs = self
            .session
            .run(ort::inputs![
                "input_ids" => Tensor::from_array(([1, t], input_ids)).map_err(e)?,
                "attention_mask" => Tensor::from_array(([1, t], vec![1i64; t])).map_err(e)?,
                "words_mask" => Tensor::from_array(([1, t], words_mask)).map_err(e)?,
                "text_lengths" => Tensor::from_array(([1usize, 1], vec![n as i64])).map_err(e)?,
                "span_idx" => Tensor::from_array(([1, n * k, 2], span_idx)).map_err(e)?,
                "span_mask" => Tensor::from_array(([1, n * k], span_mask)).map_err(e)?,
            ])
            .map_err(e)?;
        let (dims, logits) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|x| format!("extractor output: {x}"))?;
        let classes = *dims.last().ok_or("empty output")? as usize;
        if classes != LABELS.len() || logits.len() != n * k * classes {
            return Err(format!("unexpected extractor output shape {dims:?}"));
        }

        // Candidates in (start, width, class) order, as torch.where would give them.
        struct Cand {
            start: usize,
            end: usize,
            class: usize,
            score: f32,
        }
        let mut cands: Vec<Cand> = Vec::new();
        for s in 0..n {
            for w in 0..k {
                if s + w >= n {
                    continue;
                }
                let base = (s * k + w) * classes;
                for c in 0..classes {
                    let p = 1.0 / (1.0 + (-logits[base + c]).exp());
                    if p > threshold {
                        cands.push(Cand {
                            start: s,
                            end: s + w,
                            class: c,
                            score: p,
                        });
                    }
                }
            }
        }
        // Greedy, highest score first (stable), no overlaps (flat NER).
        let mut order: Vec<usize> = (0..cands.len()).collect();
        order.sort_by(|&a, &b| cands[b].score.total_cmp(&cands[a].score));
        let mut chosen: Vec<&Cand> = Vec::new();
        for i in order {
            let c = &cands[i];
            if !chosen.iter().any(|o| !(c.start > o.end || o.start > c.end)) {
                chosen.push(c);
            }
        }
        chosen.sort_by_key(|c| c.start);
        Ok(chosen
            .into_iter()
            .map(|c| {
                let (s, _) = words[c.start];
                let (_, e) = words[c.end];
                ModelEnt {
                    kind: LABELS[c.class].0,
                    text: text[s..e].to_string(),
                    start: s,
                    end: e,
                    score: c.score,
                }
            })
            .collect())
    }
}
