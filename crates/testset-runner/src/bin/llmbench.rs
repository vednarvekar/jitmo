//! Benchmark of a small local language model as jitmo's tool picker (docs/03-delivery/15).
//!
//! It starts `llama-server` itself (nothing is downloaded here), feeds it each request with only
//! the three best candidate tools, and measures what matters for a background model: accuracy,
//! wrong actions, speed split into loading / reading the prompt / writing, and memory.
//!
//! ```text
//! jitmo-llmbench --exe V:\jitmo-sandbox\llama\llama-server.exe --model <file.gguf>
//!     [--corpus testset/heldout.txt] [--skills skills] [--threads 3] [--port 8792]
//!     [--limit 300] [--kinds natural,student] [--show 25] [--candidates 3]
//! ```
//!
//! Candidates come from a plain word-overlap retrieval over each skill's title and examples
//! (a stand-in for the shared encoder, step 4); its recall@3 is reported so a miss that the
//! model could never have fixed is not blamed on the model.

use jitmo_core::router::Router;
use jitmo_model_worker::{WorkerClient, WorkerLauncher};
use jitmo_skills::Skill;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::process::{Child, Command, Stdio};
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

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_string)
        .collect()
}

struct Retriever {
    docs: Vec<(String, HashSet<String>)>,
    idf: HashMap<String, f64>,
}

impl Retriever {
    fn new(skills: &[Skill]) -> Retriever {
        let docs: Vec<(String, HashSet<String>)> = skills
            .iter()
            .map(|s| {
                let mut text = s.title.clone().unwrap_or_default();
                for e in &s.examples {
                    text.push(' ');
                    text.push_str(e);
                }
                // The id itself is a strong hint ("mail.find" -> mail, find).
                text.push(' ');
                text.push_str(&s.id.replace('.', " ").replace('_', " "));
                (s.id.clone(), words(&text).into_iter().collect())
            })
            .collect();
        let n = docs.len() as f64;
        let mut df: HashMap<String, f64> = HashMap::new();
        for (_, w) in &docs {
            for t in w {
                *df.entry(t.clone()).or_default() += 1.0;
            }
        }
        let idf = df
            .into_iter()
            .map(|(t, d)| (t, ((n + 1.0) / (d + 0.5)).ln()))
            .collect();
        Retriever { docs, idf }
    }

    fn top(&self, request: &str, k: usize) -> Vec<String> {
        let q: HashSet<String> = words(request).into_iter().collect();
        let mut scored: Vec<(f64, &String)> = self
            .docs
            .iter()
            .map(|(id, w)| {
                let s: f64 = q
                    .iter()
                    .filter(|t| w.contains(*t))
                    .map(|t| self.idf.get(t).copied().unwrap_or(0.0))
                    .sum();
                (s, id)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(k)
            .map(|(_, id)| id.clone())
            .collect()
    }
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(secs)))
        .build()
        .into()
}

/// Working set and private bytes of a process (MB), from PowerShell.
fn memory_of(pid: u32) -> Option<(f64, f64, f64)> {
    let script = format!(
        "$p=Get-Process -Id {pid}; \"$($p.WorkingSet64) $($p.PeakWorkingSet64) $($p.PrivateMemorySize64)\""
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let v: Vec<f64> = text
        .split_whitespace()
        .filter_map(|x| x.parse::<f64>().ok())
        .collect();
    (v.len() == 3).then(|| (v[0] / 1048576.0, v[1] / 1048576.0, v[2] / 1048576.0))
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() as f64 - 1.0) * p) as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exe = flag(&args, "--exe");
    let model = flag(&args, "--model");
    let port = flag(&args, "--port").unwrap_or_else(|| "8792".into());
    let threads = flag(&args, "--threads").unwrap_or_else(|| "3".into());
    let ctx = flag(&args, "--ctx").unwrap_or_else(|| "2048".into());
    let batch = flag(&args, "--batch").unwrap_or_else(|| "512".into());
    let compact = args.iter().any(|a| a == "--compact");
    let corpus = flag(&args, "--corpus").unwrap_or_else(|| "testset/heldout.txt".into());
    let skills_dir = flag(&args, "--skills").unwrap_or_else(|| "skills".into());
    let limit: usize = flag(&args, "--limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let show: usize = flag(&args, "--show")
        .and_then(|s| s.parse().ok())
        .unwrap_or(25);
    let k: usize = flag(&args, "--candidates")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let kinds: Vec<String> = flag(&args, "--kinds")
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    let (Some(exe), Some(model)) = (exe, model) else {
        eprintln!("usage: jitmo-llmbench --exe <llama-server.exe> --model <file.gguf> [options]");
        std::process::exit(2);
    };

    let mut skills: Vec<Skill> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&skills_dir) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) == Some("yaml") {
                if let Ok(y) = std::fs::read_to_string(e.path()) {
                    if let Ok(s) = Skill::parse(&y) {
                        // Account set-up and the test stub are not requests a user makes.
                        if !s.id.starts_with("account.") && s.id != "stub.send" {
                            skills.push(s);
                        }
                    }
                }
            }
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<String> = skills.iter().map(|s| s.id.clone()).collect();
    let by_id: HashMap<&str, &Skill> = skills.iter().map(|s| (s.id.as_str(), s)).collect();
    let retriever = Retriever::new(&skills);
    // With `--worker`, candidates come from jitmo's real router (the shared encoder stand-in);
    // otherwise from the plain word-overlap retrieval.
    let router: Option<Router> = flag(&args, "--worker").map(|w| {
        let models = flag(&args, "--models").unwrap_or_else(|| "models/router".into());
        let ner = flag(&args, "--ner").unwrap_or_else(|| "models/ner".into());
        let launcher = WorkerLauncher {
            program: w.into(),
            args: vec![
                "--models".into(),
                models,
                "--idle-secs".into(),
                "0".into(),
                "--extractor".into(),
                ner,
            ],
        };
        Router::new(skills.clone())
            .with_worker(WorkerClient::new(launcher, Duration::from_secs(20)))
    });
    let cands = |text: &str| -> Vec<String> {
        match &router {
            Some(r) => r
                .route(text)
                .tasks
                .iter()
                .filter(|t| ids.contains(&t.id))
                .map(|t| t.id.clone())
                .take(k)
                .collect(),
            None => retriever.top(text, k),
        }
    };

    let mut cases = load_cases(&corpus, &ids, &kinds);
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
    let recall = cases
        .iter()
        .filter(|c| c.expected != "none")
        .filter(|c| cands(&c.text).contains(&c.expected))
        .count();
    let tool_cases = cases.iter().filter(|c| c.expected != "none").count();
    println!(
        "{} skills, {} cases ({} need a tool), retrieval recall@{k} = {}/{} = {:.1}%",
        ids.len(),
        cases.len(),
        tool_cases,
        recall,
        tool_cases,
        100.0 * recall as f64 / tool_cases.max(1) as f64
    );

    // Start the server (below normal priority, no window) and time the cold load.
    let mut cmd = Command::new(&exe);
    cmd.args([
        "-m",
        &model,
        "--host",
        "127.0.0.1",
        "--port",
        &port,
        "-c",
        &ctx,
        "-t",
        &threads,
        "--no-webui",
        "--parallel",
        "1",
        "-b",
        &batch,
        "-ub",
        &batch,
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_4000);
    }
    let t0 = Instant::now();
    let mut child: Child = cmd.spawn().expect("could not start llama-server");
    let pid = child.id();
    let base = format!("http://127.0.0.1:{port}");
    let mut ready = false;
    while t0.elapsed() < Duration::from_secs(180) {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if let Ok(r) = agent(3).get(&format!("{base}/health")).call() {
            if r.status().as_u16() == 200 {
                ready = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if !ready {
        eprintln!("the server did not become ready");
        let _ = child.kill();
        std::process::exit(1);
    }
    let cold = t0.elapsed().as_secs_f64();
    let (ws_idle, _, _) = memory_of(pid).unwrap_or((0.0, 0.0, 0.0));
    println!("cold load {cold:.2}s, working set after load {ws_idle:.0} MB");

    let url = format!("{base}/v1/chat/completions");
    let ag = agent(120);
    let run = |case: &Case, cache: bool| -> (String, f64, f64, f64, Vec<String>) {
        let cand = cands(&case.text);
        let catalog: String = cand
            .iter()
            .enumerate()
            .filter_map(|(n, id)| by_id.get(id.as_str()).map(|s| (n, *s)))
            .map(|(n, s)| {
                if compact {
                    return format!("{}: {}", n + 1, s.title.clone().unwrap_or_default());
                }
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
             user asks, or \"none\" when no tool fits, when it is general chat or a question the \
             tools cannot answer, or when it is unsafe or unsupported.\n\nTools:\n{catalog}"
        );
        let mut allowed: Vec<Value> = if compact {
            (1..=cand.len()).map(|n| json!(n.to_string())).collect()
        } else {
            cand.iter().map(|i| json!(i)).collect()
        };
        allowed.push(json!(if compact { "0" } else { "none" }));
        let schema = json!({
            "type": "object",
            "properties": { "tool": { "type": "string", "enum": allowed } },
            "required": ["tool"],
            "additionalProperties": false
        });
        let system = if compact {
            // The short prompt and the one-token answer a fine-tuned picker uses.
            let briefs: Vec<jitmo_core::llm::ToolBrief> = cand
                .iter()
                .filter_map(|id| by_id.get(id.as_str()))
                .map(|s| jitmo_core::llm::ToolBrief {
                    id: s.id.clone(),
                    title: s.title.clone().unwrap_or_default(),
                    examples: Vec::new(),
                    args: Vec::new(),
                })
                .collect();
            jitmo_llm::pick_compact_system(&briefs)
        } else {
            system
        };
        let mut body = json!({
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": case.text }
            ],
            "temperature": 0,
            "max_tokens": 40,
            "cache_prompt": cache,
            "chat_template_kwargs": { "enable_thinking": false },
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "pick", "strict": true, "schema": schema }
            }
        });
        if compact {
            body.as_object_mut().map(|o| o.remove("response_format"));
            body["max_tokens"] = json!(1);
            body["grammar"] = json!(format!("root ::= [0-{}]", cand.len().min(9)));
        }
        let t = Instant::now();
        let resp = ag
            .post(&url)
            .header("Content-Type", "application/json")
            .send(body.to_string());
        let total = t.elapsed().as_secs_f64() * 1000.0;
        let (picked, p_ms, g_ms) = match resp {
            Ok(mut r) => {
                let text = r.body_mut().read_to_string().unwrap_or_default();
                let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let content = v["choices"][0]["message"]["content"].as_str();
                let picked = if compact {
                    content.map(|c| c.trim().to_string())
                } else {
                    content
                        .and_then(|c| serde_json::from_str::<Value>(c).ok())
                        .and_then(|v| v["tool"].as_str().map(str::to_string))
                }
                .unwrap_or_else(|| "?".into());
                let picked = if compact {
                    match picked.parse::<usize>() {
                        Ok(0) => "none".to_string(),
                        Ok(n) => cand.get(n - 1).cloned().unwrap_or(picked),
                        Err(_) => picked,
                    }
                } else {
                    picked
                };
                (
                    picked,
                    v["timings"]["prompt_ms"].as_f64().unwrap_or(0.0),
                    v["timings"]["predicted_ms"].as_f64().unwrap_or(0.0),
                )
            }
            Err(e) => (format!("error: {e}"), 0.0, 0.0),
        };
        (picked, total, p_ms, g_ms, cand)
    };

    let (mut right, mut wrong_action, mut asked) = (0, 0, 0);
    let (mut in_cand, mut in_cand_right) = (0, 0);
    let (mut totals, mut prompts, mut writes) = (Vec::new(), Vec::new(), Vec::new());
    let mut misses: Vec<String> = Vec::new();
    let mut peak_ws: f64 = ws_idle;
    for (n, c) in cases.iter().enumerate() {
        let (picked, total, p_ms, g_ms, cand) = run(c, true);
        totals.push(total);
        prompts.push(p_ms);
        writes.push(g_ms);
        if c.expected != "none" && cand.contains(&c.expected) {
            in_cand += 1;
            if picked == c.expected {
                in_cand_right += 1;
            }
        }
        if picked == c.expected {
            right += 1;
        } else {
            if picked == "none" {
                asked += 1;
            } else {
                wrong_action += 1;
            }
            misses.push(format!(
                "  [{}] “{}”: expected {}, picked {} (candidates {})",
                c.kind,
                c.text,
                c.expected,
                picked,
                cand.join(", ")
            ));
        }
        if (n + 1) % 50 == 0 {
            if let Some((ws, _, _)) = memory_of(pid) {
                peak_ws = peak_ws.max(ws);
            }
            println!("  {}/{} done, {} right", n + 1, cases.len(), right);
        }
    }
    // Without the prompt cache, on a sample.
    let mut nocache = Vec::new();
    for c in cases.iter().take(25) {
        nocache.push(run(c, false).1);
    }
    let (ws, peak, private) = memory_of(pid).unwrap_or((0.0, 0.0, 0.0));
    peak_ws = peak_ws.max(ws).max(peak);
    let _ = child.kill();
    let _ = child.wait();

    for v in [&mut totals, &mut prompts, &mut writes, &mut nocache] {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    }
    let n = cases.len().max(1) as f64;
    println!("\nmodel {model}");
    println!(
        "tool pick: {} right of {} = {:.1}%   wrong action {} ({:.1}%)   said none when a tool was right or on a decline case {}",
        right,
        cases.len(),
        100.0 * right as f64 / n,
        wrong_action,
        100.0 * wrong_action as f64 / n,
        asked
    );
    println!(
        "when the right tool was among the candidates: {} of {} = {:.1}%",
        in_cand_right,
        in_cand,
        100.0 * in_cand_right as f64 / in_cand.max(1) as f64
    );
    println!(
        "speed (ms): total p50 {:.0} p95 {:.0}   reading the prompt p50 {:.0}   writing p50 {:.0}   total p50 without prompt cache {:.0}",
        pct(&totals, 0.5),
        pct(&totals, 0.95),
        pct(&prompts, 0.5),
        pct(&writes, 0.5),
        pct(&nocache, 0.5)
    );
    println!(
        "memory of the model process: working set {ws:.0} MB, peak working set {peak_ws:.0} MB, private bytes {private:.0} MB (threads {threads})"
    );
    for m in misses.iter().take(show) {
        println!("{m}");
    }
}
