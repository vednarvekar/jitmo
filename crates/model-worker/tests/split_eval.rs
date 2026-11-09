//! Scores the shipped extractor on the two halves of the SP2 blind set
//! `spikes/sp2-extractor/data/fresh.txt`: DEV = odd-numbered data lines (1st, 3rd, ...),
//! TEST = even-numbered. DEV failures are listed (expected vs got) so they can be studied;
//! TEST prints only the aggregate score and never an individual sentence, so the number
//! stays honest. Run:
//! `cargo test -p jitmo-model-worker --release --test split_eval -- --ignored --nocapture`
//!
//! Environment:
//! - `SKIP_TEST=1` leaves the TEST half out (score it only at the end of an iteration).
//! - `MODES=rules,ner,ner+lists,ner+rules:contact/event` which combinations to score, in one
//!   pass (default: the shipped one). `rules` is the fallback without the model.
//! - `GAZ=gold` (or a `@gold` suffix on one mode, e.g. `ner+lists@gold`) passes the closed lists the app would pass: every app and contact named in the
//!   gold of the rows being scored (an address book that has all of them; default: empty lists).
//! - `OWN_SET=dev3_set.txt` scores one of the self-written sets in `tools/extractor` (all of it).
//! - `SHOW=0` hides the DEV failure list.
//! - `JITMO_NER_DIR` points at the model folder (default `models/ner`); the test skips with a
//!   message when it is absent.
#![cfg(feature = "models")]

use jitmo_model_worker::extractor::{LazyExtractor, Mode};
use jitmo_model_worker::rules::{extract_rules, norm, Ent, Gazetteer};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Bag = BTreeMap<(String, String), i64>;
type Row = (String, Vec<(String, String)>);

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
fn load(dev: bool) -> Vec<Row> {
    let ann = regex::Regex::new(r"\[(\w+)\|(.*?)\]").unwrap();
    let fixes = fixes();
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

/// `None` is the rules-only fallback.
fn parse_mode(s: &str) -> Option<Mode> {
    if s == "rules" {
        return None;
    }
    let mut mode = Mode::ner();
    let all = jitmo_model_worker::ner::KINDS;
    for part in s.split('+') {
        match part {
            "ner" => {}
            "lists" => mode.lists = true,
            _ => {
                mode.rules_for = part
                    .strip_prefix("rules:")
                    .expect("mode name")
                    .split('/')
                    .map(|k| *all.iter().find(|a| **a == k).expect("kind"))
                    .collect()
            }
        }
    }
    Some(mode)
}

fn model_dir() -> PathBuf {
    std::env::var_os("JITMO_NER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("models/ner"))
}

#[test]
#[ignore]
fn split_eval() {
    let dir = model_dir();
    if !dir.join("model_int8.onnx").exists() {
        eprintln!("skipping: models/ner not present (see tools/extractor_ner/README.md)");
        return;
    }
    let env = ort::init().with_telemetry(false).build().expect("ort");
    let mut ex = LazyExtractor::new(Some(dir));
    let skip_test = std::env::var_os("SKIP_TEST").is_some();
    let show = std::env::var("SHOW").map_or(true, |v| v != "0");
    let gold_env = std::env::var("GAZ").is_ok_and(|v| v == "gold");
    let modes = std::env::var("MODES").unwrap_or_else(|_| "ner".into());
    for full_name in modes.split(',') {
        // a trailing "@gold" passes the gold lists for that entry only
        let (mode_name, gold_lists) = match full_name.strip_suffix("@gold") {
            Some(n) => (n, true),
            None => (full_name, gold_env),
        };
        let mode = parse_mode(mode_name);
        for (name, dev) in [("DEV", true), ("TEST", false)] {
            if !dev && (skip_test || std::env::var_os("OWN_SET").is_some()) {
                continue;
            }
            let rows = load(dev);
            let gaz = if gold_lists {
                let names = |kind: &str| {
                    let mut v: Vec<String> = rows
                        .iter()
                        .flat_map(|(_, g)| g.iter())
                        .filter(|(k, _)| k == kind)
                        .map(|(_, t)| t.clone())
                        .collect();
                    v.sort();
                    v.dedup();
                    v
                };
                Gazetteer {
                    apps: names("app"),
                    contacts: names("contact"),
                    files: vec![],
                }
            } else {
                Gazetteer::default()
            };
            let (mut perfect, mut tp, mut fp, mut fn_) = (0, 0i64, 0i64, 0i64);
            let mut per_kind: BTreeMap<String, [i64; 3]> = BTreeMap::new();
            for (text, gold) in &rows {
                let ents: Vec<Ent> = match &mode {
                    Some(m) => {
                        ex.mode = m.clone();
                        ex.extract(&env, text, &gaz)
                    }
                    None => extract_rules(text, &gaz),
                };
                let got: Vec<(String, String)> = ents
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
                for (k, &c) in &p {
                    let h = c.min(w.get(k).copied().unwrap_or(0));
                    per_kind.entry(k.0.clone()).or_default()[1] += c - h;
                }
                for (k, &c) in &w {
                    let h = c.min(p.get(k).copied().unwrap_or(0));
                    per_kind.entry(k.0.clone()).or_default()[2] += c - h;
                }
                if hit == np && hit == nw {
                    perfect += 1;
                } else if dev && show {
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
                "{name} [{mode_name}{}]: sentence-exact {perfect}/{} = {:.1}%  span F1 {:.1}%",
                if gold_lists { ", gold lists" } else { "" },
                rows.len(),
                100.0 * perfect as f64 / rows.len() as f64,
                100.0 * 2.0 * pr * rc / (pr + rc).max(1e-9)
            );
            if dev {
                let kinds: Vec<String> = per_kind
                    .iter()
                    .map(|(k, c)| format!("{k} fp{} fn{}", c[1], c[2]))
                    .collect();
                println!("   DEV per kind (span errors): {}", kinds.join(", "));
            }
        }
    }
}
