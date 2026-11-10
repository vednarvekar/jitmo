//! Learned requests (low-memory brief, step 7): a request that a language model had to
//! work out, and that then worked (and was confirmed where it needed a confirm), is remembered
//! as "this sentence means this skill". The next time the same sentence comes in it skips the
//! router and the model entirely: instant, and still through the same guards, slot resolution,
//! policy, confirm and checks. A remembered request that later fails is forgotten.

const MAX_ENTRIES: usize = 300;

/// A sentence in one canonical form: lower case, single spaces, no trailing punctuation.
pub fn key(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| ".!?,; ".contains(c))
        .to_string()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Learned {
    /// (key, skill id, the sentence to run instead, when the model had typed values).
    entries: Vec<(String, String, Option<String>)>,
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

impl Learned {
    /// Read the saved form (`key \t skill \t sentence` per line).
    pub fn load(saved: &str) -> Learned {
        let entries = saved
            .lines()
            .filter_map(|l| {
                let mut p = l.split('\t');
                let (k, s) = (p.next()?, p.next()?);
                let sentence = p.next().filter(|x| !x.is_empty()).map(str::to_string);
                (!k.is_empty() && !s.is_empty()).then(|| (k.to_string(), s.to_string(), sentence))
            })
            .collect();
        Learned { entries }
    }

    pub fn dump(&self) -> String {
        self.entries
            .iter()
            .map(|(k, s, sent)| format!("{k}\t{s}\t{}", sent.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn get(&self, key: &str) -> Option<(String, Option<String>)> {
        self.entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, s, sent)| (s.clone(), sent.clone()))
    }

    pub fn put(&mut self, key: &str, skill: &str, sentence: Option<&str>) {
        let key = clean(key);
        self.entries.retain(|(k, _, _)| *k != key);
        self.entries
            .push((key, skill.to_string(), sentence.map(clean)));
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.retain(|(k, _, _)| k != key);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_case_spacing_and_trailing_punctuation() {
        assert_eq!(key("  Blorp  the   Spotify!! "), "blorp the spotify");
    }

    #[test]
    fn entries_survive_a_save_and_load() {
        let mut l = Learned::default();
        l.put("pull up spotify", "apps.open", None);
        l.put("buzz rahul", "mail.send", Some("email rahul that call me"));
        let back = Learned::load(&l.dump());
        assert_eq!(back, l);
        assert_eq!(
            back.get("pull up spotify"),
            Some(("apps.open".into(), None))
        );
        assert_eq!(
            back.get("buzz rahul"),
            Some(("mail.send".into(), Some("email rahul that call me".into())))
        );
    }

    #[test]
    fn a_new_answer_replaces_the_old_and_old_entries_fall_off() {
        let mut l = Learned::default();
        l.put("x", "a.b", None);
        l.put("x", "c.d", None);
        assert_eq!(l.len(), 1);
        assert_eq!(l.get("x").unwrap().0, "c.d");
        for i in 0..400 {
            l.put(&format!("req {i}"), "a.b", None);
        }
        assert_eq!(l.len(), MAX_ENTRIES);
        assert!(l.get("x").is_none());
        l.remove("req 399");
        assert!(l.get("req 399").is_none());
    }
}
