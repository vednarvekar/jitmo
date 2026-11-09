//! Checks the Rust GLiNER port and the rules against the Python reference
//! (`tools/extractor/dump_reference.py`), and scores the extractor on the SP2 sets.
//!
//! The model files are not in git. Put them in `models/extractor/` at the repo root,
//! or point `JITMO_EXTRACTOR_DIR` at them (see the README). Without them these tests
//! print a message and pass.
#![cfg(feature = "models")]

use jitmo_model_worker::extractor::LazyExtractor;
use jitmo_model_worker::gliner::Gliner;
use jitmo_model_worker::rules::{norm, Gazetteer};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn model_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("JITMO_EXTRACTOR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("models/extractor"));
    if dir.join("model_int8.onnx").exists() || dir.join("onnx/model_int8.onnx").exists() {
        Some(dir)
    } else {
        eprintln!(
            "skipping: extractor model not found in {} (set JITMO_EXTRACTOR_DIR, see README)",
            dir.display()
        );
        None
    }
}

fn env() -> ort::environment::Environment {
    ort::init().with_telemetry(false).build().expect("ort")
}

#[derive(Deserialize)]
struct Reference {
    threshold: f32,
    sentences: Vec<RefSentence>,
}
#[derive(Deserialize)]
struct RefSentence {
    text: String,
    entities: Vec<RefEnt>,
}
#[derive(Deserialize)]
struct RefEnt {
    label: String,
    text: String,
    score: f32,
}

#[test]
fn gliner_matches_python() {
    let Some(dir) = model_dir() else { return };
    let reference: Reference = serde_json::from_slice(
        &std::fs::read(repo().join("tools/extractor/extractor_reference.json")).expect("fixture"),
    )
    .expect("fixture json");
    assert!(reference.sentences.len() >= 30);
    let env = env();
    let mut gliner = Gliner::load(&env, &dir, 1).expect("load");
    let mut bad = Vec::new();
    for s in &reference.sentences {
        let got = gliner
            .predict(&s.text, reference.threshold)
            .expect("predict");
        let label_of = |kind: &str| {
            jitmo_model_worker::gliner::LABELS
                .iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, l)| l.to_string())
                .unwrap()
        };
        let got: Vec<(String, String, f32)> = got
            .into_iter()
            .map(|e| (label_of(e.kind), e.text, e.score))
            .collect();
        let want: Vec<(String, String, f32)> = s
            .entities
            .iter()
            .map(|e| (e.label.clone(), e.text.clone(), e.score))
            .collect();
        // The int8 graph is not bit-identical across ONNX Runtime builds (Python's
        // wheel vs the `ort` crate): scores move by a few hundredths, so spans that
        // sit right at the threshold may appear on one side only. Ignore those.
        let sure = |v: Vec<(String, String, f32)>| {
            v.into_iter()
                .filter(|e| e.2 >= reference.threshold + 0.05)
                .collect::<Vec<_>>()
        };
        let (got, want) = (sure(got), sure(want));
        let same = got.len() == want.len()
            && got
                .iter()
                .zip(&want)
                .all(|(a, b)| a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).abs() < 0.05);
        if !same {
            bad.push(format!("{:?}\n  rust   {got:?}\n  python {want:?}", s.text));
        }
    }
    assert!(bad.is_empty(), "{} differ:\n{}", bad.len(), bad.join("\n"));
}

// ---------------------------------------------------------------------------
// scoring on the SP2 sets (same convention as spikes/sp2-extractor/extract2.py)
// ---------------------------------------------------------------------------

type Bag = BTreeMap<(String, String), i64>;

fn bag(items: impl IntoIterator<Item = (String, String)>) -> Bag {
    let mut b = Bag::new();
    for (k, t) in items {
        *b.entry((k, norm(&t))).or_default() += 1;
    }
    b
}

fn minus(a: &Bag, b: &Bag) -> Bag {
    a.iter()
        .filter_map(|(k, &n)| {
            let left = n - b.get(k).copied().unwrap_or(0);
            (left > 0).then(|| (k.clone(), left))
        })
        .collect()
}

fn inter(a: &Bag, b: &Bag) -> i64 {
    a.iter()
        .map(|(k, &n)| n.min(b.get(k).copied().unwrap_or(0)))
        .sum()
}

struct Row {
    id: String,
    text: String,
    gold: Vec<(String, String)>,
}

fn load_set(name: &str) -> Vec<Row> {
    let path = if name.starts_with("dev") {
        repo().join("tools/extractor").join(name)
    } else {
        repo().join("spikes/sp2-extractor/data").join(name)
    };
    let ann = regex::Regex::new(r"\[(\w+)\|(.*?)\]").unwrap();
    std::fs::read_to_string(path)
        .expect("data")
        .lines()
        .filter(|l| l.contains('|'))
        .map(|line| {
            let (id, a) = line.split_once('|').unwrap();
            let gold = ann
                .captures_iter(a)
                .map(|c| (c[1].to_string(), c[2].to_string()))
                .collect();
            Row {
                id: id.to_string(),
                text: ann.replace_all(a, "$2").to_string(),
                gold,
            }
        })
        .collect()
}

#[derive(Default)]
struct Score {
    tp: i64,
    fp: i64,
    fn_: i64,
    perfect: usize,
    n: usize,
    per: BTreeMap<String, [i64; 3]>,
}

impl Score {
    fn line(&self, name: &str) -> String {
        let p = self.tp as f64 / (self.tp + self.fp).max(1) as f64;
        let r = self.tp as f64 / (self.tp + self.fn_).max(1) as f64;
        let f1 = 2.0 * p * r / (p + r).max(1e-9);
        format!(
            "{name:28} sentence-exact {:3}/{:3} = {:5.1}%   span F1 {:5.1}% (P {:.1}% / R {:.1}%)",
            self.perfect,
            self.n,
            100.0 * self.perfect as f64 / self.n.max(1) as f64,
            100.0 * f1,
            100.0 * p,
            100.0 * r
        )
    }
}

fn score(rows: &[Row], mut run: impl FnMut(&str) -> Vec<(String, String)>, show: bool) -> Score {
    let mut s = Score::default();
    for row in rows {
        let pred = bag(run(&row.text));
        let want = bag(row.gold.clone());
        let hit = inter(&pred, &want);
        let extra = minus(&pred, &want);
        let missed = minus(&want, &pred);
        let (n_fp, n_fn): (i64, i64) = (extra.values().sum(), missed.values().sum());
        s.tp += hit;
        s.fp += n_fp;
        s.fn_ += n_fn;
        s.n += 1;
        s.perfect += (n_fp == 0 && n_fn == 0) as usize;
        for (k, c) in &want {
            let got = pred.get(k).copied().unwrap_or(0);
            s.per.entry(k.0.clone()).or_default()[0] += c.min(&got);
            s.per.entry(k.0.clone()).or_default()[2] += (c - got).max(0);
        }
        for (k, c) in &pred {
            let w = want.get(k).copied().unwrap_or(0);
            s.per.entry(k.0.clone()).or_default()[1] += (c - w).max(0);
        }
        if show && (n_fp > 0 || n_fn > 0) {
            println!(
                "  {} {:?}\n      missed {:?}  extra {:?}",
                row.id,
                row.text,
                missed.keys().collect::<Vec<_>>(),
                extra.keys().collect::<Vec<_>>()
            );
        }
    }
    s
}

#[derive(Deserialize)]
struct Parity {
    set: String,
    id: String,
    text: String,
    spans: Vec<(String, String)>,
}

/// The full Rust pipeline (rules + GLiNER int8, threshold 0.3) against the Python one
/// on every sentence of the five SP2 sets. A small number of differences is allowed
/// (floating point and regex-engine edge cases); the list is printed.
#[test]
fn parity_with_python_pipeline() {
    let Some(dir) = model_dir() else { return };
    let rows: Vec<Parity> = serde_json::from_slice(
        &std::fs::read(repo().join("tools/extractor/extractor_parity.json")).expect("fixture"),
    )
    .expect("fixture json");
    let env = env();
    let mut ex = LazyExtractor::new(Some(dir));
    let gaz = Gazetteer::default();
    let mut diffs = Vec::new();
    for r in &rows {
        let got = bag(ex
            .extract_classic(&env, &r.text, &gaz)
            .into_iter()
            .map(|e| (e.kind.to_string(), e.text)));
        let want = bag(r.spans.clone());
        if got != want {
            diffs.push(format!(
                "{}/{} {:?}\n    rust-only {:?}\n    python-only {:?}",
                r.set,
                r.id,
                r.text,
                minus(&got, &want).keys().collect::<Vec<_>>(),
                minus(&want, &got).keys().collect::<Vec<_>>()
            ));
        }
    }
    println!(
        "{} of {} sentences differ from Python",
        diffs.len(),
        rows.len()
    );
    for d in &diffs {
        println!("{d}");
    }
    assert!(
        diffs.len() * 100 <= rows.len() * 2,
        "more than 2% of sentences differ from the Python pipeline"
    );
}

/// Scores the extractor on the SP2 sets. Run with:
/// `cargo test -p jitmo-model-worker --release --test extractor_model eval_sp2 -- --ignored --nocapture`
/// `fresh` and `test` are burned for tuning (SP2 RESULT.md); they are scored for reporting only.
#[test]
#[ignore]
fn eval_sp2() {
    let Some(dir) = model_dir() else { return };
    let env = env();
    let mut ex = LazyExtractor::new(Some(dir));
    let gaz = Gazetteer::default();
    let show = std::env::var_os("SHOW").is_some();
    let mut all: Vec<Row> = Vec::new();
    let sets: Vec<String> = std::env::var("SETS")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_else(|_| {
            ["doc4.txt", "val.txt", "val2.txt", "test.txt", "fresh.txt"]
                .map(str::to_string)
                .to_vec()
        });
    for set in sets.iter().map(String::as_str) {
        let rows = load_set(set);
        let mut lat = Vec::new();
        let s = score(
            &rows,
            |t| {
                let t0 = std::time::Instant::now();
                let r = ex.extract(&env, t, &gaz);
                lat.push(t0.elapsed().as_secs_f64() * 1000.0);
                r.into_iter()
                    .map(|e| (e.kind.to_string(), e.text))
                    .collect()
            },
            show,
        );
        lat.sort_by(|a, b| a.total_cmp(b));
        println!(
            "{}   mean {:.0} ms p95 {:.0} ms",
            s.line(set),
            lat.iter().sum::<f64>() / lat.len() as f64,
            lat[lat.len() * 95 / 100]
        );
        if set == "fresh.txt" {
            for (k, [tp, fp, fn_]) in &s.per {
                println!("    {k:10} TP {tp:3} FP {fp:3} FN {fn_:3}");
            }
        }
        // Rules only (no model), for comparison.
        let r = score(
            &rows,
            |t| {
                jitmo_model_worker::rules::extract_rules(t, &gaz)
                    .into_iter()
                    .map(|e| (e.kind.to_string(), e.text))
                    .collect()
            },
            false,
        );
        println!("{}", r.line(&format!("{set} (rules only)")));
        let r = score(
            &rows,
            |t| {
                jitmo_model_worker::rules::extract_rules_classic(t, &gaz)
                    .into_iter()
                    .map(|e| (e.kind.to_string(), e.text))
                    .collect()
            },
            false,
        );
        println!("{}", r.line(&format!("{set} (classic rules only)")));
        let r = score(
            &rows,
            |t| {
                ex.extract_classic(&env, t, &gaz)
                    .into_iter()
                    .map(|e| (e.kind.to_string(), e.text))
                    .collect()
            },
            false,
        );
        println!("{}", r.line(&format!("{set} (classic + model)")));
        all.extend(rows);
    }
    let _ = all;
}

// ---------------------------------------------------------------------------
// GLiNER label wording and per-type threshold sweep (tuning pool only, never `fresh`)
// ---------------------------------------------------------------------------

type Labels = Vec<(&'static str, &'static str)>;

fn label_variants() -> Vec<(&'static str, Labels)> {
    let base: Labels = jitmo_model_worker::gliner::LABELS.to_vec();
    let with = |changes: &[(&'static str, &'static str)]| -> Labels {
        base.iter()
            .map(|&(k, l)| {
                let l = changes.iter().find(|c| c.0 == k).map_or(l, |c| c.1);
                (k, l)
            })
            .collect()
    };
    vec![
        ("base", base.clone()),
        (
            "short",
            with(&[
                ("app", "app"),
                ("contact", "person"),
                ("file", "file"),
                ("media", "music"),
                ("query", "query"),
                ("message", "message"),
                ("event", "event"),
                ("setting", "setting"),
            ]),
        ),
        (
            "media-artist",
            with(&[
                ("media", "artist or song name"),
                ("event", "calendar event"),
            ]),
        ),
        (
            "long",
            with(&[
                ("query", "web search topic"),
                ("message", "text message to send"),
                ("media", "singer, band, song or youtuber"),
                ("event", "calendar event title"),
                ("file", "document or file name"),
            ]),
        ),
        (
            "reminder",
            with(&[
                ("message", "reminder or message content"),
                ("query", "search terms"),
                ("event", "meeting or appointment"),
            ]),
        ),
    ]
}

fn kind_threshold(th: &BTreeMap<&str, f32>, k: &str) -> f32 {
    th.get(k).copied().unwrap_or(0.3)
}

#[test]
#[ignore]
fn sweep_gliner() {
    let Some(dir) = model_dir() else { return };
    let env = env();
    let mut g = Gliner::load(&env, &dir, 1).expect("load");
    let gaz = Gazetteer::default();
    let mut rows: Vec<Row> = Vec::new();
    for set in ["dev_set.txt", "doc4.txt", "val.txt", "val2.txt", "test.txt"] {
        rows.extend(load_set(set));
    }
    let n_dev = load_set("dev_set.txt").len();
    for (name, labels) in label_variants() {
        let cands: Vec<Vec<jitmo_model_worker::gliner::Cand>> = rows
            .iter()
            .map(|r| g.candidates(&r.text, &labels, 0.1).expect("cands"))
            .collect();
        let run = |th: &BTreeMap<&str, f32>, range: std::ops::Range<usize>| -> usize {
            let mut ok = 0;
            for i in range {
                let raw =
                    jitmo_model_worker::gliner::select(cands[i].clone(), |k| kind_threshold(th, k));
                let got: Vec<(String, String)> =
                    jitmo_model_worker::rules::extract(&rows[i].text, &raw, 0.0, &gaz)
                        .into_iter()
                        .map(|e| (e.kind.to_string(), e.text))
                        .collect();
                if bag(got) == bag(rows[i].gold.clone()) {
                    ok += 1;
                }
            }
            ok
        };
        let mut best = BTreeMap::new();
        let global: Vec<(f32, usize)> = [0.2f32, 0.25, 0.3, 0.35, 0.4, 0.5, 0.6]
            .iter()
            .map(|&t| {
                let th: BTreeMap<&str, f32> = jitmo_model_worker::gliner::LABELS
                    .iter()
                    .map(|&(k, _)| (k, t))
                    .collect();
                (t, run(&th, 0..rows.len()))
            })
            .collect();
        println!(
            "{name:14} global thresholds (pool {}): {global:?}",
            rows.len()
        );
        // coordinate descent per type on the whole pool
        let mut cur: BTreeMap<&str, f32> = jitmo_model_worker::gliner::LABELS
            .iter()
            .map(|&(k, _)| (k, 0.3))
            .collect();
        let mut score = run(&cur, 0..rows.len());
        for _ in 0..2 {
            for &(k, _) in jitmo_model_worker::gliner::LABELS {
                for t in [0.15f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.95] {
                    let mut trial = cur.clone();
                    trial.insert(k, t);
                    let s = run(&trial, 0..rows.len());
                    if s > score {
                        score = s;
                        cur = trial;
                    }
                }
            }
        }
        best.clone_from(&cur);
        println!(
            "{name:14} per-type best {score}/{}  (dev_set part {}/{n_dev})  {best:?}",
            rows.len(),
            run(&best, 0..n_dev)
        );
    }
}
