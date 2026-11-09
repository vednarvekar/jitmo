//! Checks the token-classification extractor (`ner.rs`) and scores the rules plus NER hybrid on the
//! SP2 `fresh.txt` blind set, split into DEV (odd-numbered data lines) and TEST (even-numbered).
//!
//! Only aggregates are printed (sentence-exact count and span F1); no sentence of TEST is ever shown.
//! The model files are not in git: put them in `models/ner/` (or set `JITMO_NER_DIR`), see
//! `tools/extractor_ner/README.md`. Without them the tests print a message and pass.
#![cfg(feature = "models")]

use jitmo_model_worker::ner::{ner_dir, NerModel};
use jitmo_model_worker::rules::{self, norm, Gazetteer, ModelEnt};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn model_dir() -> Option<PathBuf> {
    let found = ner_dir(None).or_else(|| {
        let d = repo().join("models/ner");
        d.join("model_int8.onnx").exists().then_some(d)
    });
    if found.is_none() {
        eprintln!("skipping: models/ner not present (see tools/extractor_ner/README.md)");
    }
    found
}

type Bag = BTreeMap<(String, String), i64>;

fn bag(items: impl IntoIterator<Item = (String, String)>) -> Bag {
    let mut b = Bag::new();
    for (k, t) in items {
        *b.entry((k, norm(&t))).or_default() += 1;
    }
    b
}

fn count_minus(a: &Bag, b: &Bag) -> i64 {
    a.iter()
        .map(|(k, &n)| (n - b.get(k).copied().unwrap_or(0)).max(0))
        .sum()
}

/// (text, gold spans) for the data lines of `fresh.txt`, in order.
fn fresh_rows() -> Vec<(String, Vec<(String, String)>)> {
    let ann = regex::Regex::new(r"\[(\w+)\|(.*?)\]").unwrap();
    std::fs::read_to_string(repo().join("spikes/sp2-extractor/data/fresh.txt"))
        .or_else(|_| std::fs::read_to_string("V:/Code/jitmo/spikes/sp2-extractor/data/fresh.txt"))
        .expect("fresh.txt")
        .lines()
        .filter(|l| l.contains('|'))
        .map(|line| {
            let (_, a) = line.split_once('|').unwrap();
            let gold = ann
                .captures_iter(a)
                .map(|c| (c[1].to_string(), c[2].to_string()))
                .collect();
            (ann.replace_all(a, "$2").to_string(), gold)
        })
        .collect()
}

/// Sentence-exact count and span (tp, fp, fn) for the rows at `indices`.
fn score_split(
    rows: &[(String, Vec<(String, String)>)],
    indices: impl Iterator<Item = usize>,
    mut run: impl FnMut(&str) -> Vec<(String, String)>,
) -> (usize, usize, [i64; 3]) {
    let (mut perfect, mut n, mut c) = (0, 0, [0i64; 3]);
    for i in indices {
        let (text, gold) = &rows[i];
        let pred = bag(run(text));
        let want = bag(gold.clone());
        let fp = count_minus(&pred, &want);
        let fn_ = count_minus(&want, &pred);
        let tp = pred.values().sum::<i64>() - fp;
        c[0] += tp;
        c[1] += fp;
        c[2] += fn_;
        perfect += (fp == 0 && fn_ == 0) as usize;
        n += 1;
    }
    (perfect, n, c)
}

fn line(name: &str, (perfect, n, c): (usize, usize, [i64; 3])) -> String {
    let p = c[0] as f64 / (c[0] + c[1]).max(1) as f64;
    let r = c[0] as f64 / (c[0] + c[2]).max(1) as f64;
    format!(
        "{name:34} sentence-exact {perfect:3}/{n:3} = {:5.1}%  span F1 {:5.1}% (P {:.1} / R {:.1})",
        100.0 * perfect as f64 / n.max(1) as f64,
        100.0 * 2.0 * p * r / (p + r).max(1e-9),
        100.0 * p,
        100.0 * r
    )
}

fn report(
    name: &str,
    rows: &[(String, Vec<(String, String)>)],
    mut run: impl FnMut(&str) -> Vec<(String, String)>,
) {
    let dev = score_split(rows, (0..rows.len()).step_by(2), &mut run);
    let test = score_split(rows, (1..rows.len()).step_by(2), &mut run);
    println!("{}", line(&format!("DEV  {name}"), dev));
    println!("{}", line(&format!("TEST {name}"), test));
}

#[derive(serde::Deserialize)]
struct JsonEnt {
    kind: String,
    start: usize,
    end: usize,
    text: String,
    score: f32,
}

fn kind_static(k: &str) -> &'static str {
    jitmo_model_worker::ner::KINDS
        .iter()
        .find(|x| **x == k)
        .copied()
        .unwrap_or("query")
}

/// Hybrid score from raw spans dumped by a Python tool (`eval_onnx.py --dump`, or any other
/// candidate model): `NER_PREDS=file.json cargo test -p jitmo-model-worker --release --test ner_eval
/// eval_hybrid_from_json -- --ignored --nocapture`.
#[test]
#[ignore]
fn eval_hybrid_from_json() {
    let Some(path) = std::env::var_os("NER_PREDS") else {
        eprintln!("set NER_PREDS to a JSON file {{text: [{{kind,start,end,text,score}}]}}");
        return;
    };
    let preds: BTreeMap<String, Vec<JsonEnt>> =
        serde_json::from_slice(&std::fs::read(path).expect("preds")).expect("json");
    let rows = fresh_rows();
    let gaz = Gazetteer::default();
    let th: f32 = std::env::var("NER_TH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.3);
    let raw_of = |t: &str| -> Vec<ModelEnt> {
        preds
            .get(t)
            .map(|v| {
                v.iter()
                    .map(|e| ModelEnt {
                        kind: kind_static(&e.kind),
                        text: e.text.clone(),
                        start: e.start,
                        end: e.end,
                        score: e.score,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    report("rules only (extract_v2, no model)", &rows, |t| {
        rules::extract_v2(t, &[], th, &gaz)
            .into_iter()
            .map(|e| (e.kind.to_string(), e.text))
            .collect()
    });
    report("rules + model spans", &rows, |t| {
        rules::extract_v2(t, &raw_of(t), th, &gaz)
            .into_iter()
            .map(|e| (e.kind.to_string(), e.text))
            .collect()
    });
    report("model spans alone (all types)", &rows, |t| {
        raw_of(t)
            .into_iter()
            .filter(|e| e.score >= th)
            .map(|e| (e.kind.to_string(), e.text))
            .collect()
    });
    // Variants: the rules own only the closed/regular kinds, the model owns the rest.
    let closed = ["date_time", "duration", "number", "setting"];
    report("rules(date/dur/num/setting) + model rest", &rows, |t| {
        let mut out: Vec<(String, String)> = rules::extract_v2(t, &[], th, &gaz)
            .into_iter()
            .filter(|e| closed.contains(&e.kind))
            .map(|e| (e.kind.to_string(), e.text))
            .collect();
        out.extend(
            raw_of(t)
                .into_iter()
                .filter(|e| e.score >= th && !closed.contains(&e.kind))
                .map(|e| (e.kind.to_string(), e.text)),
        );
        out
    });
    report("model, rules fill kinds it left empty", &rows, |t| {
        let mut out: Vec<(String, String)> = raw_of(t)
            .into_iter()
            .filter(|e| e.score >= th)
            .map(|e| (e.kind.to_string(), e.text))
            .collect();
        let have: Vec<String> = out.iter().map(|(k, _)| k.clone()).collect();
        out.extend(
            rules::extract_v2(t, &[], th, &gaz)
                .into_iter()
                .filter(|e| !have.contains(&e.kind.to_string()))
                .map(|e| (e.kind.to_string(), e.text)),
        );
        out
    });
}

/// The real Rust runtime on `models/ner`: model alone and rules + model, with latency.
/// `cargo test -p jitmo-model-worker --release --test ner_eval eval_live -- --ignored --nocapture`
#[test]
#[ignore]
fn eval_live() {
    let Some(dir) = model_dir() else { return };
    let env = ort::init().with_telemetry(false).build().expect("ort");
    let mut m = NerModel::load(&env, &dir, 1).expect("load");
    let rows = fresh_rows();
    let gaz = Gazetteer::default();
    let th = 0.3;
    let mut ms = Vec::new();
    report("Rust NER alone", &rows, |t| {
        let t0 = std::time::Instant::now();
        let r = m.predict(t, th).unwrap();
        ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        r.into_iter()
            .map(|e| (e.kind.to_string(), e.text))
            .collect()
    });
    ms.sort_by(|a, b| a.total_cmp(b));
    println!(
        "NER latency (1 thread): mean {:.1} ms, p95 {:.1} ms",
        ms.iter().sum::<f64>() / ms.len() as f64,
        ms[ms.len() * 95 / 100]
    );
    report("Rust rules + NER", &rows, |t| {
        let raw = m.predict(t, 0.0).unwrap();
        rules::extract_v2(t, &raw, th, &gaz)
            .into_iter()
            .map(|e| (e.kind.to_string(), e.text))
            .collect()
    });
    // Hybrid D: the model runs only when the rules leave a model-owned slot kind empty.
    let mut calls = 0usize;
    report("Rust rules, NER only if needed", &rows, |t| {
        let first = rules::extract_v2(t, &[], th, &gaz);
        let needs = !first.iter().any(|e| {
            matches!(
                e.kind,
                "contact" | "message" | "file" | "media" | "query" | "event" | "place"
            )
        });
        if !needs {
            return first
                .into_iter()
                .map(|e| (e.kind.to_string(), e.text))
                .collect();
        }
        calls += 1;
        let raw = m.predict(t, 0.0).unwrap();
        rules::extract_v2(t, &raw, th, &gaz)
            .into_iter()
            .map(|e| (e.kind.to_string(), e.text))
            .collect()
    });
    println!(
        "(model calls in the on-demand run: {calls} of {} sentence passes)",
        rows.len()
    );
}

/// With the model files present, the Rust runtime must tag obvious sentences. Skipped otherwise.
#[test]
fn tags_obvious_sentences_with_the_real_model() {
    let Some(dir) = model_dir() else { return };
    let env = ort::init().with_telemetry(false).build().expect("ort");
    let mut m = NerModel::load(&env, &dir, 1).expect("load");
    let got = m.predict("tell Priya I reached home", 0.0).unwrap();
    let kinds: Vec<(&str, &str)> = got.iter().map(|e| (e.kind, e.text.as_str())).collect();
    assert!(kinds.contains(&("contact", "Priya")), "{kinds:?}");
    assert!(kinds.iter().any(|(k, _)| *k == "message"), "{kinds:?}");
    assert!(m.predict("", 0.0).unwrap().is_empty());
}
