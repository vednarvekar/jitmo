//! Prints what the first rule pass and the shipped extractor find in a few sentences
//! (with the model when `models/extractor` exists):
//! `TEXT="a sentence|another" cargo test -p jitmo-model-worker --release --test show_one -- --ignored --nocapture`
#![cfg(feature = "models")]
use jitmo_model_worker::extractor::{LazyExtractor, THRESHOLD};
use jitmo_model_worker::gliner::Gliner;
use jitmo_model_worker::rules::{extract_rules, extract_rules_classic, Ent, Gazetteer};
use std::path::Path;

#[test]
#[ignore]
fn show_one() {
    let gaz = Gazetteer::default();
    let show = |v: Vec<Ent>| {
        v.into_iter()
            .map(|e| format!("{}={:?}", e.kind, e.text))
            .collect::<Vec<_>>()
            .join("  ")
    };
    let env = ort::init().with_telemetry(false).build().expect("ort");
    let mut ex = LazyExtractor::new(Some(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/extractor"),
    ));
    let mut g = Gliner::load(
        &env,
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/extractor"),
        1,
    )
    .ok();
    for t in std::env::var("TEXT").unwrap_or_default().split('|') {
        println!("{t:?}");
        if let Some(g) = g.as_mut() {
            let raw: Vec<String> = g
                .predict(t, 0.1)
                .unwrap_or_default()
                .iter()
                .map(|e| format!("{}={:?}@{:.2}", e.kind, e.text, e.score))
                .collect();
            println!(
                "  model raw (>=0.1; shipped cut {THRESHOLD}) {}",
                raw.join("  ")
            );
        }
        println!("  classic rules   {}", show(extract_rules_classic(t, &gaz)));
        println!("  v2 rules        {}", show(extract_rules(t, &gaz)));
        println!(
            "  classic + model {}",
            show(ex.extract_classic(&env, t, &gaz))
        );
        println!("  v2 + model      {}", show(ex.extract(&env, t, &gaz)));
    }
}
