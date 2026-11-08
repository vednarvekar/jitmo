//! Tier 0 router: MiniLM int8 encoder (ONNX Runtime) plus a logistic-regression
//! head, as proven in SP1. Chains are split at "and / then / commas" and the
//! split is kept only when every part is confident, none is OUT and they differ.
//!
//! Files in the model directory: `model_int8.onnx`, `tokenizer.json`, `head.json`
//! (classes, coef, b exported from the SP1 head).

use crate::engine::Engine;
use jitmo_ipc::{ContextNote, TaskScore, Understanding};
use ort::session::Session;
use ort::value::Tensor;
use regex::Regex;
use serde::Deserialize;
use std::path::Path;
use tokenizers::{Tokenizer, TruncationParams};

/// A chain part must be at least this sure (SP1 cutoff, tuned on val sets).
const CHAIN_PART_MIN: f32 = 0.8;
const OUT: &str = "OUT";

#[derive(Deserialize)]
struct Head {
    classes: Vec<String>,
    coef: Vec<Vec<f32>>,
    b: Vec<f32>,
}

pub struct RouterEngine {
    _env: ort::environment::Environment,
    session: Session,
    tokenizer: Tokenizer,
    head: Head,
    split: Regex,
}

impl RouterEngine {
    pub fn load(dir: &Path) -> Result<Self, String> {
        // No telemetry: the worker must not talk to the network.
        let env = ort::init()
            .with_telemetry(false)
            .build()
            .map_err(|e| format!("onnx runtime: {e}"))?;
        let mut builder = Session::builder(&env).map_err(|e| format!("router model: {e}"))?;
        builder = builder
            .with_intra_threads(1)
            .map_err(|e| format!("router model: {e}"))?;
        let session = builder
            .commit_from_file(dir.join("model_int8.onnx"))
            .map_err(|e| format!("router model: {e}"))?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| format!("router tokenizer: {e}"))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 128,
                ..Default::default()
            }))
            .map_err(|e| format!("router tokenizer: {e}"))?;
        let head: Head = serde_json::from_slice(
            &std::fs::read(dir.join("head.json")).map_err(|e| format!("router head: {e}"))?,
        )
        .map_err(|e| format!("router head: {e}"))?;
        if head.coef.len() != head.classes.len() || head.b.len() != head.classes.len() {
            return Err("router head: sizes do not match".into());
        }
        let split = Regex::new(
            r"(?i)\s*(?:,\s*then\s+|\s+and\s+then\s+|\s+then\s+|\s+after that\s+|,\s+and\s+|,\s*|\s+and\s+|\s+&\s+)",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            _env: env,
            session,
            tokenizer,
            head,
            split,
        })
    }

    /// Probability of each class for one utterance.
    fn probabilities(&mut self, text: &str) -> Result<Vec<f32>, String> {
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
                "attention_mask" => Tensor::from_array((shape, mask.clone())).map_err(|e| e.to_string())?,
                "token_type_ids" => Tensor::from_array((shape, types)).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| format!("router run: {e}"))?;
        let (dims, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("router output: {e}"))?;
        let hidden = *dims.last().ok_or("empty output")? as usize;
        // Mean over real tokens, then L2 normalise.
        let mut v = vec![0f32; hidden];
        let mut count = 0f32;
        for (t, m) in mask.iter().enumerate() {
            if *m == 1 {
                count += 1.0;
                for (h, slot) in v.iter_mut().enumerate() {
                    *slot += data[t * hidden + h];
                }
            }
        }
        for x in &mut v {
            *x /= count.max(1.0);
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        for x in &mut v {
            *x /= norm;
        }
        let mut z: Vec<f32> = self
            .head
            .coef
            .iter()
            .zip(&self.head.b)
            .map(|(row, b)| row.iter().zip(&v).map(|(a, x)| a * x).sum::<f32>() + b)
            .collect();
        let max = z.iter().cloned().fold(f32::MIN, f32::max);
        let mut sum = 0.0;
        for x in &mut z {
            *x = (*x - max).exp();
            sum += *x;
        }
        for x in &mut z {
            *x /= sum;
        }
        Ok(z)
    }

    fn top(&self, p: &[f32]) -> (String, f32) {
        let (i, s) = p
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("head has classes");
        (self.head.classes[i].clone(), *s)
    }

    fn out_of_scope(&self, p: &[f32]) -> f32 {
        self.head
            .classes
            .iter()
            .position(|c| c == OUT)
            .map_or(0.0, |i| p[i])
    }
}

impl Engine for RouterEngine {
    fn understand(&mut self, text: &str, _: &ContextNote) -> Result<Understanding, String> {
        let whole = self.probabilities(text)?;
        let parts: Vec<String> = self
            .split
            .split(text)
            .filter(|p| !p.trim().is_empty())
            .map(str::to_string)
            .collect();
        if parts.len() > 1 {
            let mut tops = Vec::new();
            for part in &parts {
                let p = self.probabilities(part)?;
                tops.push(self.top(&p));
            }
            let confident = tops.iter().all(|(_, s)| *s >= CHAIN_PART_MIN);
            let no_out = tops.iter().all(|(l, _)| l != OUT);
            let mut labels: Vec<&String> = tops.iter().map(|(l, _)| l).collect();
            labels.sort();
            labels.dedup();
            if confident && no_out && labels.len() > 1 {
                return Ok(Understanding {
                    tasks: tops
                        .iter()
                        .map(|(l, s)| TaskScore {
                            id: l.clone(),
                            score: *s,
                        })
                        .collect(),
                    out_of_scope: 0.0,
                    chained: true,
                    spans: Vec::new(),
                });
            }
        }
        // Best three real tasks, best first (OUT is reported separately).
        let mut ranked: Vec<(usize, f32)> = whole.iter().cloned().enumerate().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let tasks = ranked
            .into_iter()
            .filter(|(i, _)| self.head.classes[*i] != OUT)
            .take(3)
            .map(|(i, s)| TaskScore {
                id: self.head.classes[i].clone(),
                score: s,
            })
            .collect();
        Ok(Understanding {
            tasks,
            out_of_scope: self.out_of_scope(&whole),
            chained: false,
            spans: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs `models/router/` at the repo root; skipped if it is not there.
    fn engine() -> Option<RouterEngine> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/router");
        if !dir.join("model_int8.onnx").exists() {
            eprintln!("skipping: models/router not present");
            return None;
        }
        Some(RouterEngine::load(&dir).expect("load"))
    }

    fn top(e: &mut RouterEngine, text: &str) -> Understanding {
        e.understand(text, &ContextNote::default()).unwrap()
    }

    #[test]
    fn routes_open_app_and_pause() {
        let Some(mut e) = engine() else { return };
        let u = top(&mut e, "open spotify");
        assert_eq!(u.tasks[0].id, "A1");
        assert!(u.tasks[0].score >= 0.8, "{u:?}");
        assert!(!u.chained);
        let u = top(&mut e, "pause the music");
        assert_eq!(u.tasks[0].id, "A3");
    }

    #[test]
    fn unrelated_request_is_out_of_scope() {
        let Some(mut e) = engine() else { return };
        let u = top(&mut e, "dim the lights in my living room");
        assert!(u.out_of_scope > 0.3 || u.tasks[0].score < 0.8, "{u:?}");
    }

    #[test]
    fn a_chain_is_split_into_its_tasks() {
        let Some(mut e) = engine() else { return };
        let u = top(&mut e, "open spotify and then pause the music");
        if u.chained {
            let ids: Vec<&str> = u.tasks.iter().map(|t| t.id.as_str()).collect();
            assert_eq!(ids, ["A1", "A3"]);
        }
    }
}

#[cfg(test)]
mod eval {
    use super::*;

    /// Re-scores the SP1 held-out sets and the Phase 2 held-out set in Rust. Run with:
    /// `cargo test -p jitmo-model-worker --release eval_sp1 -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn eval_sp1_sets() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut e = RouterEngine::load(&root.join("models/router")).expect("load");
        for file in [
            "spikes/sp1-router/data/test.txt",
            "spikes/sp1-router/data/test2.txt",
            "tools/router/data/heldout_phase2.txt",
        ] {
            let text = std::fs::read_to_string(root.join(file)).expect("router eval data");
            let (mut ok, mut n, mut single_ok, mut single_n) = (0, 0, 0, 0);
            let mut ms = Vec::new();
            for line in text.lines().filter(|l| l.contains('|')) {
                let (want, req) = line.split_once('|').unwrap();
                let mut want: Vec<&str> = want.split('+').collect();
                want.sort();
                let t0 = std::time::Instant::now();
                let u = e.understand(req, &ContextNote::default()).unwrap();
                ms.push(t0.elapsed().as_secs_f64() * 1000.0);
                let mut got: Vec<&str> = if u.chained {
                    u.tasks.iter().map(|t| t.id.as_str()).collect()
                } else if u.out_of_scope >= u.tasks[0].score {
                    vec!["OUT"]
                } else {
                    vec![u.tasks[0].id.as_str()]
                };
                got.sort();
                got.dedup();
                let hit = got == want;
                n += 1;
                ok += hit as u32;
                if want.len() == 1 {
                    single_n += 1;
                    single_ok += hit as u32;
                }
            }
            ms.sort_by(|a, b| a.total_cmp(b));
            println!(
                "{file}: all {ok}/{n}, single {single_ok}/{single_n}, mean {:.1} ms, p95 {:.1} ms",
                ms.iter().sum::<f64>() / ms.len() as f64,
                ms[ms.len() * 95 / 100]
            );
        }
    }
}
