//! Scores the shipped extractor (rules + GLiNER) on the two halves of the SP2 blind set
//! `spikes/sp2-extractor/data/fresh.txt`: DEV = odd-numbered data lines (1st, 3rd, ...),
//! TEST = even-numbered. DEV failures are listed (expected vs got) so they can be studied;
//! TEST prints only the aggregate score and never an individual sentence, so the number
//! stays honest. Run:
//! `cargo test -p jitmo-model-worker --release --test split_eval -- --ignored --nocapture`
//! (`SKIP_TEST=1` leaves the TEST half out, so it is scored only at the end of an iteration).
#![cfg(feature = "models")]

use jitmo_model_worker::extractor::LazyExtractor;
use jitmo_model_worker::rules::{norm, Gazetteer};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Bag = BTreeMap<(String, String), i64>;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bag(items: impl IntoIterator<Item = (String, String)>) -> Bag {
    let mut b = Bag::new();
    for (k, t) in items {
        *b.entry((k, norm(&t))).or_default() += 1;
    }
    b
}

/// Reviewed corrections to wrong gold labels (`tools/extractor/fresh_gold_fixes.txt`, same
/// line format, matched by id). `NO_FIXES=1` scores against the original labels.
fn fixes() -> BTreeMap<String, String> {
    if std::env::var_os("NO_FIXES").is_some() {
        return BTreeMap::new();
    }
    std::fs::read_to_string(repo().join("tools/extractor/fresh_gold_fixes.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| Some((l.split_once('|')?.0.to_string(), l.to_string())))
        .collect()
}

/// (sentence, gold spans) for the odd (`dev = true`) or even data lines.
fn load(dev: bool) -> Vec<(String, Vec<(String, String)>)> {
    let ann = regex::Regex::new(r"\[(\w+)\|(.*?)\]").unwrap();
    let fixes = fixes();
    // OWN_SET=dev3_set.txt scores one of the self-written sets in tools/extractor instead
    // (all of it, failures listed); the fresh.txt halves are not touched then.
    let own = std::env::var("OWN_SET").ok();
    let path = match &own {
        Some(f) => repo().join("tools/extractor").join(f),
        None => repo().join("spikes/sp2-extractor/data/fresh.txt"),
    };
    std::fs::read_to_string(path)
        .expect("data file")
        .lines()
        .filter(|l| l.contains('|'))
        .enumerate()
        .filter(|(i, _)| own.is_some() || (i % 2 == 0) == dev)
        .map(|(_, line)| {
            let id = line.split_once('|').unwrap().0;
            let line = fixes.get(id).map_or(line, String::as_str);
            let (_, a) = line.split_once('|').unwrap();
            let gold = ann
                .captures_iter(a)
                .map(|c| (c[1].to_string(), c[2].to_string()))
                .collect();
            (ann.replace_all(a, "$2").to_string(), gold)
        })
        .collect()
}

fn fmt(v: &[(String, String)]) -> String {
    v.iter()
        .map(|(k, t)| format!("{k}={t:?}"))
        .collect::<Vec<_>>()
        .join("  ")
}

#[test]
#[ignore]
fn split_eval() {
    let dir = std::env::var_os("JITMO_EXTRACTOR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("models/extractor"));
    let env = ort::init().with_telemetry(false).build().expect("ort");
    let mut ex = LazyExtractor::new(Some(dir));
    let gaz = Gazetteer::default();
    let skip_test = std::env::var_os("SKIP_TEST").is_some();
    for (name, dev) in [("DEV", true), ("TEST", false)] {
        if !dev && (skip_test || std::env::var_os("OWN_SET").is_some()) {
            continue;
        }
        let rows = load(dev);
        let (mut perfect, mut tp, mut fp, mut fn_) = (0, 0i64, 0i64, 0i64);
        for (text, gold) in &rows {
            let got: Vec<(String, String)> = ex
                .extract(&env, text, &gaz)
                .into_iter()
                .map(|e| (e.kind.to_string(), e.text))
                .collect();
            let (p, w) = (bag(got.clone()), bag(gold.clone()));
            let hit: i64 = p
                .iter()
                .map(|(k, &c)| c.min(w.get(k).copied().unwrap_or(0)))
                .sum();
            let (np, nw): (i64, i64) = (p.values().sum(), w.values().sum());
            tp += hit;
            fp += np - hit;
            fn_ += nw - hit;
            if hit == np && hit == nw {
                perfect += 1;
            } else if dev {
                println!(
                    "FAIL {text:?}\n   want {}\n   got  {}",
                    fmt(gold),
                    fmt(&got)
                );
            }
        }
        let (pr, rc) = (
            tp as f64 / (tp + fp).max(1) as f64,
            tp as f64 / (tp + fn_).max(1) as f64,
        );
        println!(
            "{name}: sentence-exact {perfect}/{} = {:.1}%  span F1 {:.1}%",
            rows.len(),
            100.0 * perfect as f64 / rows.len() as f64,
            100.0 * 2.0 * pr * rc / (pr + rc).max(1e-9)
        );
    }
}
