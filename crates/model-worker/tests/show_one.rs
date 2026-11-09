//! Prints what the rules and the shipped extractor (NER model when `models/ner` exists) find in a
//! few sentences:
//! `TEXT="a sentence|another" cargo test -p jitmo-model-worker --release --test show_one -- --ignored --nocapture`
#![cfg(feature = "models")]
use jitmo_model_worker::extractor::LazyExtractor;
use jitmo_model_worker::rules::{extract_rules, Ent, Gazetteer};

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
    let mut ex = LazyExtractor::new(None);
    for t in std::env::var("TEXT").unwrap_or_default().split('|') {
        println!("{t:?}");
        println!("  rules only  {}", show(extract_rules(t, &gaz)));
        println!("  shipped     {}", show(ex.extract(&env, t, &gaz)));
    }
}
