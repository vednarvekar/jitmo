//! The extractor through its public entry point (`LazyExtractor`): the rules-only fallback when
//! the NER model folder is missing, and, when `models/ner` exists (git-ignored, see
//! `tools/extractor_ner/README.md`), the shipped path with a model. The model test skips with a
//! message when the files are absent, so CI without models stays green.
//!
//! The ONNX runtime allows one environment per process; both tests share this one.
#![cfg(feature = "models")]

use jitmo_model_worker::extractor::LazyExtractor;
use jitmo_model_worker::rules::{to_slot_span, Gazetteer};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static ENV: LazyLock<ort::environment::Environment> =
    LazyLock::new(|| ort::init().with_telemetry(false).build().expect("ort"));

fn model_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("JITMO_NER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/ner"));
    if dir.join("model_int8.onnx").exists() {
        Some(dir)
    } else {
        eprintln!("skipping: models/ner not present (see tools/extractor_ner/README.md)");
        None
    }
}

#[test]
fn a_missing_model_folder_means_rules_only_and_no_error() {
    // A folder that does not exist; the other places a model is looked for are the
    // program's own folder and `models/ner` below the working directory, neither has one here.
    if std::env::var_os("JITMO_NER_DIR").is_some() {
        return;
    }
    let mut ex = LazyExtractor::new(Some(PathBuf::from("no/such/folder")));
    let got = ex.extract(
        &ENV,
        "remind me tomorrow at 5pm to call mom",
        &Gazetteer::default(),
    );
    assert!(ex.is_rules_only());
    assert!(!ex.is_loaded());
    assert!(
        got.iter()
            .any(|e| e.kind == "date_time" && e.text.contains("tomorrow")),
        "{got:?}"
    );
    // asking again does not retry the load or fail
    assert!(!ex
        .extract(&ENV, "set a timer for 10 minutes", &Gazetteer::default())
        .is_empty());
}

#[test]
fn the_model_tags_slots_and_the_lists_snap_names() {
    let Some(dir) = model_dir() else { return };
    let mut ex = LazyExtractor::new(Some(dir));
    let none = Gazetteer::default();
    let got = ex.extract(&ENV, "tell Priya I reached home", &none);
    assert!(ex.is_loaded() && !ex.is_rules_only());
    let kinds: Vec<(&str, &str)> = got.iter().map(|e| (e.kind, e.text.as_str())).collect();
    assert!(kinds.contains(&("contact", "Priya")), "{kinds:?}");
    assert!(kinds.iter().any(|(k, _)| *k == "message"), "{kinds:?}");

    // Skill slot types: file -> file_ref, everything else keeps its name.
    let spans: Vec<_> = ex
        .extract(&ENV, "send report.pdf to Ved", &none)
        .iter()
        .map(to_slot_span)
        .collect();
    assert!(spans.iter().any(|s| s.kind == "file_ref"), "{spans:?}");
    assert!(spans.iter().any(|s| s.kind == "contact"), "{spans:?}");

    // A closed list types and snaps a name: whatever the model made of "Zoya Khan", the
    // result is exactly that contact.
    let gaz = Gazetteer {
        contacts: vec!["Zoya Khan".into()],
        ..Default::default()
    };
    let got = ex.extract(&ENV, "send the notes to zoya khan", &gaz);
    assert!(
        got.iter()
            .any(|e| e.kind == "contact" && e.text.eq_ignore_ascii_case("zoya khan")),
        "{got:?}"
    );
    assert!(ex.extract(&ENV, "", &none).is_empty());
}
