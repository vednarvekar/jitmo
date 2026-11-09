//! Prints what the first rule pass and the shipped extractor find in a few sentences
//! (with the model when `models/extractor` exists):
//! `TEXT="a sentence|another" cargo test -p jitmo-model-worker --release --test show_one -- --ignored --nocapture`
#![cfg(feature = "models")]
use jitmo_model_worker::extractor::LazyExtractor;
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
    for t in std::env::var("TEXT").unwrap_or_default().split('|') {
        println!("{t:?}");
        println!("  classic rules   {}", show(extract_rules_classic(t, &gaz)));
        println!("  v2 rules        {}", show(extract_rules(t, &gaz)));
        println!(
            "  classic + model {}",
            show(ex.extract_classic(&env, t, &gaz))
        );
        println!("  v2 + model      {}", show(ex.extract(&env, t, &gaz)));
    }
}
