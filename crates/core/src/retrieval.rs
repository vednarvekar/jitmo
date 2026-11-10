//! A plain word-overlap retriever over each skill's title, examples and id. It complements the
//! router's 50-class head when a language model is asked to pick a tool: the router's best two
//! plus the best word-overlap match that is not among them contain the right tool far more
//! often than the router's best three (measured on the held-out set: 91.6% against 84.9%).

use jitmo_skills::Skill;
use std::collections::{HashMap, HashSet};

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_string)
        .collect()
}

pub struct Retriever {
    docs: Vec<(String, HashSet<String>)>,
    idf: HashMap<String, f64>,
}

impl Retriever {
    pub fn new(skills: &[Skill]) -> Retriever {
        let docs: Vec<(String, HashSet<String>)> = skills
            .iter()
            .map(|s| {
                let mut text = s.title.clone().unwrap_or_default();
                for e in &s.examples {
                    text.push(' ');
                    text.push_str(e);
                }
                text.push(' ');
                text.push_str(&s.id.replace(['.', '_'], " "));
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

    /// The `k` skill ids whose words overlap the request most (rarer words count more).
    pub fn top(&self, request: &str, k: usize) -> Vec<String> {
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

/// The candidates for a model: the router's best two, then the best word-overlap match not
/// already there, up to `n` in all.
pub fn candidates(
    router_ranked: &[String],
    retriever: &Retriever,
    request: &str,
    n: usize,
) -> Vec<String> {
    let mut out: Vec<String> = router_ranked
        .iter()
        .take(n.saturating_sub(1))
        .cloned()
        .collect();
    for id in retriever.top(request, 12) {
        if out.len() >= n {
            break;
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(id: &str, title: &str, examples: &[&str]) -> Skill {
        let ex: String = examples.iter().map(|e| format!("  - \"{e}\"\n")).collect();
        Skill::parse(&format!(
            "id: {id}\ntitle: \"{title}\"\nversion: 1\ncategory: test\ncapabilities:\n  - files.read:selected\nnetwork: none\nlimits: {{ cpu_ms: 10, timeout_s: 1 }}\nrisk: read_only\nexamples:\n{ex}steps:\n  - connector: x\n    action: y\n    args: {{}}\ncheck:\n  connector: x\n  action: y\n  args: {{}}\nundo: none\n"
        ))
        .unwrap()
    }

    #[test]
    fn the_best_word_overlap_comes_first_and_the_mix_adds_it() {
        let skills = vec![
            skill("a.one", "Open an app", &["launch notepad"]),
            skill("b.two", "Check the weather", &["will it rain"]),
            skill("c.three", "Find a file", &["where is my resume"]),
        ];
        let r = Retriever::new(&skills);
        assert_eq!(r.top("will it rain tomorrow", 1), vec!["b.two".to_string()]);
        let c = candidates(
            &[
                "a.one".to_string(),
                "c.three".to_string(),
                "b.two".to_string(),
            ],
            &r,
            "will it rain tomorrow",
            3,
        );
        assert_eq!(c, vec!["a.one", "c.three", "b.two"]);
    }
}
