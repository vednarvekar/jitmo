//! `settings`: save the user's place ("set my city to Pune").
//!
//! Writes the `default_place` setting, which the weather and sunrise/sunset
//! answers use when no city is said. Undo puts the previous value back. The
//! place is checked first (letters, spaces and hyphens, up to 60 characters)
//! and only ever goes to the database as a bound parameter.

use crate::{ActionSpec, Args, ConnError, Connector, Health, Output};
use jitmo_store::Store;
use std::path::PathBuf;

pub const PLACE_KEY: &str = "default_place";
const MAX_PLACE_CHARS: usize = 60;

const BAD_PLACE: &str =
    "I can only save a place name made of letters, spaces and hyphens (up to 60 characters).";

/// Check and tidy a place name. Lower-case names get capitals ("new york" -> "New York").
pub fn clean_place(raw: &str) -> Result<String, String> {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let name = words.join(" ");
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_PLACE_CHARS
        && name.chars().any(char::is_alphabetic)
        && name
            .chars()
            .all(|c| c.is_alphabetic() || c == ' ' || c == '-');
    // "i live in a house", "my city is my home": not a place name.
    let vague = matches!(
        words.first().map(|w| w.to_lowercase()).as_deref(),
        Some("a" | "an" | "my" | "this" | "that" | "here" | "there")
    );
    if !ok || vague {
        return Err(BAD_PLACE.to_string());
    }
    if name.chars().any(char::is_uppercase) {
        return Ok(name);
    }
    let title = |w: &str| {
        let mut c = w.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    };
    Ok(name.split(' ').map(title).collect::<Vec<_>>().join(" "))
}

pub struct Settings {
    db: PathBuf,
    key: Option<jitmo_store::Key>,
}

impl Settings {
    /// `db` is the same database file the app uses.
    pub fn new(db: PathBuf) -> Self {
        Settings { db, key: None }
    }

    /// Database key, when the file is encrypted.
    pub fn with_key(mut self, key: jitmo_store::Key) -> Self {
        self.key = Some(key);
        self
    }

    fn store(&self) -> Result<Store, ConnError> {
        Store::open(&self.db, self.key.as_ref())
            .map_err(|e| ConnError::Failed(format!("I couldn't open the settings: {e}")))
    }
}

impl Connector for Settings {
    fn id(&self) -> &str {
        "settings"
    }

    fn actions(&self) -> Vec<ActionSpec> {
        vec![ActionSpec {
            name: "set_place".into(),
            required_args: vec!["place".into()],
        }]
    }

    fn health(&self) -> Health {
        Health::Ready
    }

    fn execute(&self, action: &str, args: &Args) -> Result<Output, ConnError> {
        if action != "set_place" {
            return Err(ConnError::UnknownAction(action.to_string()));
        }
        let raw = args
            .get("place")
            .ok_or_else(|| ConnError::MissingArg("place".into()))?;
        let place = clean_place(raw).map_err(ConnError::Failed)?;
        let store = self.store()?;
        let previous = store
            .get_setting(PLACE_KEY)
            .map_err(|e| ConnError::Failed(e.to_string()))?;
        store
            .set_setting(PLACE_KEY, &place)
            .map_err(|e| ConnError::Failed(format!("I couldn't save your place: {e}")))?;
        let mut out = vec![
            (
                "message".to_string(),
                format!("Saved {place} as your place."),
            ),
            ("place".to_string(), place),
        ];
        if let Some(p) = previous {
            out.push(("previous".to_string(), p));
        }
        Ok(Output(out.into_iter().collect()))
    }

    fn check(&self, action: &str, args: &Args) -> Result<bool, ConnError> {
        if action != "set_place" {
            return Err(ConnError::UnknownAction(action.to_string()));
        }
        let raw = args
            .get("place")
            .ok_or_else(|| ConnError::MissingArg("place".into()))?;
        let place = clean_place(raw).map_err(ConnError::Failed)?;
        let saved = self
            .store()?
            .get_setting(PLACE_KEY)
            .map_err(|e| ConnError::Failed(e.to_string()))?;
        Ok(saved.as_deref() == Some(place.as_str()))
    }

    fn undo(&self, action: &str, output: &Output) -> Option<Result<(), ConnError>> {
        if action != "set_place" {
            return None;
        }
        Some(self.store().and_then(|s| {
            match output.0.get("previous") {
                Some(p) => s.set_setting(PLACE_KEY, p),
                None => s.delete_setting(PLACE_KEY),
            }
            .map_err(|e| ConnError::Failed(e.to_string()))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jitmo-set-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("jitmo.db")
    }

    fn args(place: &str) -> Args {
        [("place".to_string(), place.to_string())].into()
    }

    #[test]
    fn saves_confirms_checks_and_undo_removes_when_there_was_nothing() {
        let db = temp_db("save");
        let c = Settings::new(db.clone());
        let out = c.execute("set_place", &args("Pune")).unwrap();
        assert_eq!(out.0["message"], "Saved Pune as your place.");
        assert!(c.check("set_place", &args("Pune")).unwrap());
        let s = Store::open(&db, None).unwrap();
        assert_eq!(
            s.get_setting("default_place").unwrap().as_deref(),
            Some("Pune")
        );
        c.undo("set_place", &out).unwrap().unwrap();
        assert_eq!(s.get_setting("default_place").unwrap(), None);
        assert!(!c.check("set_place", &args("Pune")).unwrap());
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn undo_restores_the_previous_place() {
        let db = temp_db("undo");
        let c = Settings::new(db.clone());
        c.execute("set_place", &args("Mumbai")).unwrap();
        let out = c.execute("set_place", &args("delhi")).unwrap();
        assert_eq!(out.0["message"], "Saved Delhi as your place.");
        let s = Store::open(&db, None).unwrap();
        assert_eq!(
            s.get_setting("default_place").unwrap().as_deref(),
            Some("Delhi")
        );
        c.undo("set_place", &out).unwrap().unwrap();
        assert_eq!(
            s.get_setting("default_place").unwrap().as_deref(),
            Some("Mumbai")
        );
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn names_are_tidied_and_bad_names_refused_without_saving() {
        assert_eq!(clean_place("  new   york ").unwrap(), "New York");
        assert_eq!(
            clean_place("Stratford-upon-Avon").unwrap(),
            "Stratford-upon-Avon"
        );
        assert_eq!(clean_place("São Paulo").unwrap(), "São Paulo");
        for bad in [
            "",
            "   ",
            "-",
            "Pune; DROP TABLE settings",
            "Pune'--",
            "Pune 411001",
            "a\u{0}b",
            &"x".repeat(61),
        ] {
            assert!(clean_place(bad).is_err(), "{bad:?}");
        }
        assert!(clean_place(&"x".repeat(60)).is_ok());

        let db = temp_db("bad");
        let c = Settings::new(db.clone());
        let e = c
            .execute("set_place", &args("Pune'; DROP TABLE settings;--"))
            .unwrap_err();
        assert!(e.to_string().contains("letters, spaces and hyphens"), "{e}");
        let s = Store::open(&db, None).unwrap();
        assert_eq!(s.get_setting("default_place").unwrap(), None);
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn missing_argument_and_unknown_action() {
        let c = Settings::new(temp_db("misc"));
        assert!(matches!(
            c.execute("set_place", &Args::new()),
            Err(ConnError::MissingArg(_))
        ));
        assert!(matches!(
            c.execute("nope", &args("Pune")),
            Err(ConnError::UnknownAction(_))
        ));
    }
}
