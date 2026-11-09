//! The closed lists the engine knows: installed apps and contacts.
//!
//! They serve two jobs:
//! - the router/extractor treats them as closed lists, so a name in the request is
//!   recognised even when the sentence is loose (`ContextNote.apps` / `.contacts`);
//! - the resolver (`named_app`, `named_contact`) turns the words the user said into
//!   the real entry, or asks which one when several fit.
//!
//! The lists are advisory, not a wall: a name that is on neither list still passes
//! through as said (the connector has the final word, and a person who has never been
//! messaged cannot be on a list learned from use). What the lists add is canonical
//! names, a number to show on the confirm card, and a question instead of a guess when
//! two entries fit (SR7, T94).
//!
//! Where contacts come from, without any Microsoft or Google sign-in:
//! 1. learned: a task that used a name and worked saves it (`Store::remember_contact`);
//! 2. imported: vCard (`.vcf`) files in the user's `Contacts` folder, which is what
//!    "Export" in Windows Contacts, Outlook, Google Contacts and phones produce.

use jitmo_ipc::ContextNote;
use jitmo_store::{Contact, Store};
use std::path::Path;

/// A snapshot of the lists, taken once per request.
#[derive(Debug, Clone, Default)]
pub struct Known {
    /// Display names of installed apps.
    pub apps: Vec<String>,
    /// Saved contacts, most used first.
    pub contacts: Vec<Contact>,
}

impl Known {
    /// The lists as the router wants them: app names, and contact names plus each first
    /// name (people say "ved", not "Ved Narvekar").
    pub fn note(&self) -> ContextNote {
        let mut contacts: Vec<String> = Vec::new();
        for c in &self.contacts {
            push_unique(&mut contacts, &c.name);
        }
        for c in &self.contacts {
            if let Some(first) = c.name.split_whitespace().next() {
                if first.chars().count() >= 2 && first != c.name {
                    push_unique(&mut contacts, first);
                }
            }
        }
        ContextNote {
            apps: self.apps.clone(),
            contacts,
            ..ContextNote::default()
        }
    }
}

fn push_unique(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        list.push(name.to_string());
    }
}

/// Lowercase letters and digits only, so "Visual Studio Code" and "visual-studio-code"
/// compare equal. Same rule as the app lookup in the Windows adapter.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// What a name in the request matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match<T> {
    One(T),
    /// Several fit and none is exact: the user must say which.
    Many(Vec<T>),
    None,
}

/// Match the words the user said against saved contacts.
///
/// 1. the full name, ignoring case and punctuation, wins outright;
/// 2. otherwise every word said must be a whole word of the contact's name ("ved" fits
///    "Ved Narvekar", "ved nar" fits nothing): one fit is the person, several is a
///    question. No guessing from typos or sounds, because a send goes to a real person.
///
/// Lists of people ("ved and rahul", "all my contacts") match nobody; they are counted
/// and limited by the policy, not resolved here.
pub fn match_contact(said: &str, contacts: &[Contact]) -> Match<Contact> {
    let wanted = words(said);
    if wanted.is_empty() {
        return Match::None;
    }
    if let Some(c) = contacts.iter().find(|c| words(&c.name) == wanted) {
        return Match::One(c.clone());
    }
    let fits: Vec<Contact> = contacts
        .iter()
        .filter(|c| {
            let have = words(&c.name);
            wanted.iter().all(|w| have.contains(w))
        })
        .cloned()
        .collect();
    match fits.len() {
        0 => Match::None,
        1 => Match::One(fits.into_iter().next().unwrap_or_else(|| unreachable!())),
        _ => Match::Many(fits),
    }
}

/// Match the words the user said against installed app names: an exact name first,
/// then apps whose name contains it (as the launcher does). Uninstallers never count.
pub fn match_app(said: &str, apps: &[String]) -> Match<String> {
    let wanted = squash(said);
    if wanted.is_empty() {
        return Match::None;
    }
    let usable = || apps.iter().filter(|a| !squash(a).contains("uninstall"));
    if let Some(a) = usable().find(|a| squash(a) == wanted) {
        return Match::One(a.clone());
    }
    if wanted.chars().count() < 3 {
        return Match::None;
    }
    let mut hits: Vec<String> = Vec::new();
    for a in usable().filter(|a| squash(a).contains(&wanted)) {
        if !hits.iter().any(|h| squash(h) == squash(a)) {
            hits.push(a.clone());
        }
    }
    match hits.len() {
        0 => Match::None,
        1 => Match::One(hits.remove(0)),
        _ => Match::Many(hits),
    }
}

// --- vCard import ----------------------------------------------------------

/// One person read from a vCard file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VCard {
    pub name: String,
    pub phone: Option<String>,
}

/// Read the people out of vCard text (versions 2.1 to 4.0; one file may hold many
/// cards). Takes the display name (`FN`, else `N`) and the first phone number (`TEL`).
/// Cards without a name are skipped. Folded lines are joined.
pub fn parse_vcards(text: &str) -> Vec<VCard> {
    // Unfold: a line starting with a space or tab continues the previous one.
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) && !lines.is_empty() {
            if let Some(last) = lines.last_mut() {
                last.push_str(&line[1..]);
            }
        } else {
            lines.push(line.to_string());
        }
    }
    let mut out = Vec::new();
    let mut current: Option<(Option<String>, Option<String>, Option<String>)> = None;
    for line in lines {
        let upper = line.to_ascii_uppercase();
        if upper == "BEGIN:VCARD" {
            current = Some((None, None, None));
        } else if upper == "END:VCARD" {
            if let Some((full, structured, phone)) = current.take() {
                if let Some(name) = full.or(structured).filter(|n| !n.trim().is_empty()) {
                    out.push(VCard { name, phone });
                }
            }
        } else if let (Some(card), Some((prop, value))) = (current.as_mut(), line.split_once(':')) {
            let prop = prop.split(';').next().unwrap_or("").to_ascii_uppercase();
            let value = value.trim();
            match prop.as_str() {
                "FN" if card.0.is_none() => card.0 = Some(value.to_string()),
                "N" if card.1.is_none() => {
                    // Family;Given;Additional;Prefix;Suffix  ->  "Given Additional Family"
                    let p: Vec<&str> = value.split(';').collect();
                    let part = |i: usize| p.get(i).copied().unwrap_or("").trim();
                    let name = [part(1), part(2), part(0)]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" ");
                    card.1 = Some(name);
                }
                "TEL" if card.2.is_none() && !value.is_empty() => {
                    card.2 = Some(value.trim_start_matches("tel:").to_string())
                }
                _ => {}
            }
        }
    }
    out
}

/// Import every `.vcf` file directly inside `dir` (not subfolders) into the store.
/// Safe to run at every start: a person already saved keeps their uses, and only
/// gains a number they did not have. Returns how many cards were read. A missing or
/// unreadable folder is not an error: most PCs have no contacts folder content.
pub fn import_vcard_folder(store: &Store, dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut read = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("vcf"))
        {
            continue;
        }
        // vCard is UTF-8 in practice; an odd byte must not lose the whole file.
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        for card in parse_vcards(&String::from_utf8_lossy(&bytes)) {
            if store
                .upsert_contact(&card.name, card.phone.as_deref(), "vcard")
                .is_ok()
            {
                read += 1;
            }
        }
    }
    read
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, phone: Option<&str>) -> Contact {
        Contact {
            name: name.into(),
            phone: phone.map(str::to_string),
            source: "learned".into(),
            uses: 0,
        }
    }

    #[test]
    fn a_full_name_wins_even_when_it_is_also_part_of_another() {
        let list = [person("Ved", None), person("Ved Narvekar", None)];
        assert_eq!(match_contact("ved", &list), Match::One(list[0].clone()));
        assert_eq!(
            match_contact("Ved Narvekar", &list),
            Match::One(list[1].clone())
        );
    }

    #[test]
    fn a_first_name_fits_one_person_or_asks_when_two_share_it() {
        let one = [person("Ved Narvekar", None), person("Rahul Shah", None)];
        assert_eq!(
            match_contact("ved", &one),
            Match::One(person("Ved Narvekar", None))
        );
        let two = [person("Ved Narvekar", None), person("Ved Patel", None)];
        assert!(matches!(match_contact("Ved", &two), Match::Many(v) if v.len() == 2));
        assert_eq!(
            match_contact("ved patel", &two),
            Match::One(person("Ved Patel", None))
        );
    }

    #[test]
    fn a_typo_a_stranger_and_a_group_match_nobody() {
        let list = [person("Ved Narvekar", None), person("Rahul", None)];
        assert_eq!(match_contact("vedd", &list), Match::None);
        assert_eq!(match_contact("zainab", &list), Match::None);
        assert_eq!(match_contact("ved and rahul", &list), Match::None);
        assert_eq!(match_contact("all my contacts", &list), Match::None);
        assert_eq!(match_contact("  ", &list), Match::None);
    }

    #[test]
    fn apps_match_exactly_then_by_containing_name() {
        let apps: Vec<String> = [
            "Spotify",
            "Visual Studio Code",
            "Word",
            "Microsoft Word Viewer",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(match_app("spotify", &apps), Match::One("Spotify".into()));
        assert_eq!(match_app("word", &apps), Match::One("Word".into()));
        assert_eq!(
            match_app("visual-studio code", &apps),
            Match::One("Visual Studio Code".into())
        );
        assert_eq!(
            match_app("studio", &apps),
            Match::One("Visual Studio Code".into())
        );
        assert_eq!(match_app("photoshop", &apps), Match::None);
        assert_eq!(match_app("o", &apps), Match::None);
    }

    #[test]
    fn several_apps_that_fit_are_reported_not_guessed() {
        let apps: Vec<String> = ["Office Hub", "Office Lens", "Uninstall Office"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            match_app("office", &apps),
            Match::Many(vec!["Office Hub".into(), "Office Lens".into()])
        );
    }

    #[test]
    fn the_router_note_has_full_and_first_names_once() {
        let k = Known {
            apps: vec!["Spotify".into()],
            contacts: vec![
                person("Ved Narvekar", None),
                person("Ved Patel", None),
                person("Rahul", None),
            ],
        };
        let n = k.note();
        assert_eq!(n.apps, ["Spotify"]);
        assert_eq!(n.contacts, ["Ved Narvekar", "Ved Patel", "Rahul", "Ved"]);
    }

    const CARDS: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Narvekar;Ved;;;\r\nFN:Ved Narvekar\r\n\
TEL;TYPE=CELL:+91 98765 43210\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:2.1\r\nN:Shah;Priya;K.;;\r\nTEL;CELL:+44 7700 900123\r\nTEL;HOME:123\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Rahul S\r\n harma\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:3.0\r\nTEL:999\r\nEND:VCARD\r\n";

    #[test]
    fn vcards_give_names_and_the_first_number() {
        let cards = parse_vcards(CARDS);
        assert_eq!(
            cards,
            vec![
                VCard {
                    name: "Ved Narvekar".into(),
                    phone: Some("+91 98765 43210".into())
                },
                VCard {
                    name: "Priya K. Shah".into(),
                    phone: Some("+44 7700 900123".into())
                },
                VCard {
                    name: "Rahul Sharma".into(),
                    phone: None
                },
            ]
        );
    }

    #[test]
    fn importing_a_folder_is_repeatable_and_ignores_other_files() {
        let dir = std::env::temp_dir().join(format!("jitmo-vcf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("all.vcf"), CARDS).unwrap();
        std::fs::write(dir.join("notes.txt"), CARDS).unwrap();
        let store = Store::open_in_memory().unwrap();
        assert_eq!(import_vcard_folder(&store, &dir), 3);
        store
            .remember_contact("Ved Narvekar", "2026-11-09T10:00:00Z")
            .unwrap();
        assert_eq!(import_vcard_folder(&store, &dir), 3);
        let all = store.contacts().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].name, "Ved Narvekar");
        assert_eq!(all[0].uses, 1);
        assert_eq!(all[0].phone.as_deref(), Some("+91 98765 43210"));
        assert_eq!(import_vcard_folder(&store, &dir.join("missing")), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
