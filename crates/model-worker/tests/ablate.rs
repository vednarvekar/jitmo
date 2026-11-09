//! Scores the extractor configurations side by side on the sets named in `SETS`
//! (default: the blind `fresh.txt`), with latency and process memory. Used for the
//! comparison table in `tools/extractor/RESULTS.md`:
//! `cargo test -p jitmo-model-worker --release --test ablate -- --ignored --nocapture`
#![cfg(feature = "models")]

use jitmo_model_worker::gliner::{select, Cand, Gliner, LABELS};
use jitmo_model_worker::rules::{self, norm, Gazetteer, ModelEnt};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn working_set_mb() -> f64 {
    let pid = std::process::id();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).WorkingSet64"),
        ])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or(0.0, |b| b / 1048576.0)
}

type Row = (String, Vec<(String, String)>);

fn load(name: &str) -> Vec<Row> {
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
            let (_, a) = line.split_once('|').unwrap();
            let gold = ann
                .captures_iter(a)
                .map(|c| (c[1].to_string(), c[2].to_string()))
                .collect();
            (ann.replace_all(a, "$2").to_string(), gold)
        })
        .collect()
}

fn bag(items: impl IntoIterator<Item = (String, String)>) -> BTreeMap<(String, String), i64> {
    let mut b = BTreeMap::new();
    for (k, t) in items {
        *b.entry((k, norm(&t))).or_default() += 1;
    }
    b
}

fn media_artist_labels() -> Vec<(&'static str, &'static str)> {
    LABELS
        .iter()
        .map(|&(k, l)| match k {
            "media" => (k, "artist or song name"),
            "event" => (k, "calendar event"),
            _ => (k, l),
        })
        .collect()
}

#[test]
#[ignore]
fn ablate() {
    let env = ort::init().with_telemetry(false).build().expect("ort");
    let dir = repo().join("models/extractor");
    let rss0 = working_set_mb();
    let mut g = Gliner::load(&env, &dir, 1).expect("model");
    let rss1 = working_set_mb();
    let gaz = Gazetteer::default();
    let sets: Vec<String> = std::env::var("SETS")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_else(|_| vec!["fresh.txt".to_string()]);
    let tuned: BTreeMap<&str, f32> = [("contact", 0.2), ("media", 0.7), ("place", 0.2)]
        .into_iter()
        .collect();
    let tuned_th = |k: &str| tuned.get(k).copied().unwrap_or(0.3);
    for set in &sets {
        let rows = load(set);
        let n = rows.len();
        // model outputs once per label variant
        let t0 = Instant::now();
        let base: Vec<Vec<Cand>> = rows
            .iter()
            .map(|r| g.candidates(&r.0, LABELS, 0.1).expect("base"))
            .collect();
        let model_ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let alt_labels = media_artist_labels();
        let alt: Vec<Vec<Cand>> = rows
            .iter()
            .map(|r| g.candidates(&r.0, &alt_labels, 0.1).expect("alt"))
            .collect();
        let rss2 = working_set_mb();

        type Pipe<'a> = Box<dyn Fn(usize, &str) -> Vec<(String, String)> + 'a>;
        let to_pairs = |v: Vec<rules::Ent>| -> Vec<(String, String)> {
            v.into_iter()
                .map(|e| (e.kind.to_string(), e.text))
                .collect()
        };
        let raw =
            |c: &[Cand], th: &dyn Fn(&str) -> f32| -> Vec<ModelEnt> { select(c.to_vec(), th) };
        // Optional: spans from a fine-tuned PyTorch GLiNER, dumped to JSON by a script
        // (`FT_RAW=path`), keyed by set name and sentence text.
        let ft_raw: Option<serde_json::Value> = std::env::var_os("FT_RAW")
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok());
        let ft_spans = |t: &str| -> Vec<ModelEnt> {
            let Some(list) = ft_raw
                .as_ref()
                .and_then(|v| v.get(set.as_str())?.get(t)?.as_array())
            else {
                return Vec::new();
            };
            list.iter()
                .filter_map(|e| {
                    let kind = LABELS
                        .iter()
                        .find(|(k, _)| Some(*k) == e["kind"].as_str())?
                        .0;
                    Some(ModelEnt {
                        kind,
                        text: e["text"].as_str()?.to_string(),
                        start: e["start"].as_u64()? as usize,
                        end: e["end"].as_u64()? as usize,
                        score: e["score"].as_f64()? as f32,
                    })
                })
                .collect()
        };
        let mut configs: Vec<(&str, Pipe)> = vec![
            (
                "classic rules only",
                Box::new(|_, t| to_pairs(rules::extract_rules_classic(t, &gaz))),
            ),
            (
                "classic rules + GLiNER (shipped before)",
                Box::new(|i, t| to_pairs(rules::extract(t, &raw(&base[i], &|_| 0.3), 0.3, &gaz))),
            ),
            (
                "classic + GLiNER, tuned labels/thresholds",
                Box::new(|i, t| to_pairs(rules::extract(t, &raw(&alt[i], &tuned_th), 0.0, &gaz))),
            ),
            (
                "v2 rules only",
                Box::new(|_, t| to_pairs(rules::extract_rules(t, &gaz))),
            ),
            (
                "v2 rules + GLiNER",
                Box::new(|i, t| {
                    to_pairs(rules::extract_v2(t, &raw(&base[i], &|_| 0.3), 0.3, &gaz))
                }),
            ),
            (
                "v2 + GLiNER, tuned labels/thresholds",
                Box::new(|i, t| {
                    to_pairs(rules::extract_v2(t, &raw(&alt[i], &tuned_th), 0.0, &gaz))
                }),
            ),
        ];
        if ft_raw.is_some() {
            configs.push((
                "classic + fine-tuned GLiNER (PyTorch spans)",
                Box::new(|_, t| to_pairs(rules::extract(t, &ft_spans(t), 0.3, &gaz))),
            ));
            configs.push((
                "v2 + fine-tuned GLiNER (PyTorch spans)",
                Box::new(|_, t| to_pairs(rules::extract_v2(t, &ft_spans(t), 0.3, &gaz))),
            ));
        }
        println!(
            "== {set} ({n} sentences). model alone {model_ms:.0} ms/sentence; working set {rss0:.0} MB before load, {rss1:.0} MB after load, {rss2:.0} MB after {n} sentences"
        );
        for (name, f) in &configs {
            let t0 = Instant::now();
            let mut perfect = 0;
            let (mut tp, mut fp, mut fn_) = (0i64, 0i64, 0i64);
            for (i, (text, gold)) in rows.iter().enumerate() {
                let pred = bag(f(i, text));
                let want = bag(gold.clone());
                let hit: i64 = pred
                    .iter()
                    .map(|(k, &c)| c.min(want.get(k).copied().unwrap_or(0)))
                    .sum();
                let (p, w): (i64, i64) = (pred.values().sum(), want.values().sum());
                tp += hit;
                fp += p - hit;
                fn_ += w - hit;
                perfect += (hit == p && hit == w) as usize;
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
            let pr = tp as f64 / (tp + fp).max(1) as f64;
            let rc = tp as f64 / (tp + fn_).max(1) as f64;
            println!(
                "{name:44} exact {perfect:3}/{n} = {:5.1}%  F1 {:5.1}%  (rules/pipeline time {ms:.2} ms/sentence)",
                100.0 * perfect as f64 / n as f64,
                100.0 * 2.0 * pr * rc / (pr + rc).max(1e-9)
            );
        }
    }
}
