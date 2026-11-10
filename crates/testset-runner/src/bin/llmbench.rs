//! Benchmark of a local language model as the tool picker, against the skills audit corpus.
//!
//! Talks to a `llama-server` already running on this PC (nothing is downloaded or started
//! here):
//!
//! ```text
//! jitmo-llmbench --server http://127.0.0.1:8081 [--corpus testset/audit_corpus.txt]
//!                [--skills skills] [--limit 150] [--kinds clean,typo,slang] [--show 30]
//! ```
//!
//! For each corpus line whose expectation is `run:<skill>` (or `decline`), the model gets the
//! skill catalog and the request and must answer `{"tool": "<skill id>" | "none"}` as
//! schema-constrained JSON. The report gives accuracy, the speed (p50/p95) and the misses.

use jitmo_skills::Skill;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

struct Case {
    expected: String,
    kind: String,
    text: String,
}

fn load_cases(path: &str, ids: &[String], kinds: &[String]) -> Vec<Case> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 4 {
            continue;
        }
        let (kind, input, expect) = (parts[1], parts[2], parts[3]);
        if !kinds.is_empty() && !kinds.iter().any(|k| k == kind) {
            continue;
        }
        // Special inputs (`<empty>`, `&&`) are not plain requests.
        if input.contains("&&") || input.starts_with('<') || input.trim().is_empty() {
            continue;
        }
        let first = expect.split(';').next().unwrap_or("");
        let expected = if let Some(rest) = first.strip_prefix("run:") {
            let id = rest.split('/').next().unwrap_or("");
            if !ids.iter().any(|i| i == id) {
                continue;
            }
            id.to_string()
        } else if first == "decline" {
            "none".to_string()
        } else {
            continue;
        };
        out.push(Case {
            expected,
            kind: kind.to_string(),
            text: input.to_string(),
        });
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let server = flag(&args, "--server").unwrap_or_else(|| "http://127.0.0.1:8081".into());
    let corpus = flag(&args, "--corpus").unwrap_or_else(|| "testset/audit_corpus.txt".into());
    let skills_dir = flag(&args, "--skills").unwrap_or_else(|| "skills".into());
    let limit: usize = flag(&args, "--limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);
    let show: usize = flag(&args, "--show")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let kinds: Vec<String> = flag(&args, "--kinds")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_default();

    // The catalog: id, title and two examples for every skill.
    let mut skills: Vec<Skill> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&skills_dir) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) == Some("yaml") {
                if let Ok(y) = std::fs::read_to_string(e.path()) {
                    if let Ok(s) = Skill::parse(&y) {
                        skills.push(s);
                    }
                }
            }
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<String> = skills.iter().map(|s| s.id.clone()).collect();
    let catalog: String = skills
        .iter()
        .map(|s| {
            let ex: Vec<String> = s
                .examples
                .iter()
                .take(2)
                .map(|e| format!("\"{e}\""))
                .collect();
            format!(
                "- {}: {} (e.g. {})",
                s.id,
                s.title.clone().unwrap_or_default(),
                ex.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let system = format!(
        "You route requests for a Windows assistant. Pick the single tool that does what the \
         user asks. Use \"none\" when no tool fits, when it is general chat or a question the \
         tools cannot answer, or when it asks for something unsafe.\n\nTools:\n{catalog}"
    );
    let mut enum_ids: Vec<Value> = ids.iter().map(|i| json!(i)).collect();
    enum_ids.push(json!("none"));
    let schema = json!({
        "type": "object",
        "properties": { "tool": { "type": "string", "enum": enum_ids } },
        "required": ["tool"],
        "additionalProperties": false
    });

    let mut cases = load_cases(&corpus, &ids, &kinds);
    // A spread across the corpus rather than just its first lines.
    if cases.len() > limit {
        let step = cases.len() as f64 / limit as f64;
        cases = (0..limit)
            .map(|i| {
                let c = &cases[(i as f64 * step) as usize];
                Case {
                    expected: c.expected.clone(),
                    kind: c.kind.clone(),
                    text: c.text.clone(),
                }
            })
            .collect();
    }
    println!(
        "{} skills, {} cases, catalog ~{} chars",
        ids.len(),
        cases.len(),
        catalog.len()
    );

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let url = format!("{}/v1/chat/completions", server.trim_end_matches('/'));

    let mut right = 0;
    let mut times: Vec<f64> = Vec::new();
    let mut misses: Vec<String> = Vec::new();
    for (n, c) in cases.iter().enumerate() {
        let body = json!({
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": c.text }
            ],
            "temperature": 0,
            "max_tokens": 40,
            "cache_prompt": true,
            "chat_template_kwargs": { "enable_thinking": false },
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "pick", "strict": true, "schema": schema }
            }
        });
        let t = Instant::now();
        let resp = agent
            .post(&url)
            .header("Content-Type", "application/json")
            .send(body.to_string());
        let secs = t.elapsed().as_secs_f64();
        let picked = match resp {
            Ok(mut r) => {
                let text = r.body_mut().read_to_string().unwrap_or_default();
                serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v["choices"][0]["message"]["content"]
                            .as_str()
                            .and_then(|c| serde_json::from_str::<Value>(c).ok())
                    })
                    .and_then(|v| v["tool"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "?".into())
            }
            Err(e) => format!("error: {e}"),
        };
        times.push(secs);
        if picked == c.expected {
            right += 1;
        } else {
            misses.push(format!(
                "  [{}] “{}”: expected {}, picked {}",
                c.kind, c.text, c.expected, picked
            ));
        }
        if (n + 1) % 10 == 0 {
            println!("  {}/{} done, {} right so far", n + 1, cases.len(), right);
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |p: f64| times[((times.len() as f64 - 1.0) * p) as usize];
    println!(
        "\naccuracy {}/{} = {:.1}%   time p50 {:.2}s  p95 {:.2}s  max {:.2}s",
        right,
        cases.len(),
        100.0 * right as f64 / cases.len().max(1) as f64,
        pct(0.5),
        pct(0.95),
        times.last().copied().unwrap_or(0.0)
    );
    for m in misses.iter().take(show) {
        println!("{m}");
    }
}
