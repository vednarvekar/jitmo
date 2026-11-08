//! Local store: SQLite tables and migrations (LLD 07, "Tables").
//!
//! The key comes from the caller (the Windows adapter reads it from Credential
//! Manager), so this crate has no OS code.
//!
//! Encryption needs the `sqlcipher` feature. Without it this is plain SQLite,
//! and [`Store::open`] returns [`StoreError::EncryptionUnavailable`] if a key is
//! passed. That keeps a build from silently storing data unencrypted when the
//! caller expects encryption.

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("a key was given but this build has no SQLCipher support")]
    EncryptionUnavailable,
    #[error("wrong key or not a database")]
    BadKey,
    #[error("database is from a newer version (schema {found}, this build knows {known})")]
    SchemaTooNew { found: u32, known: u32 },
}

/// 32-byte database key.
pub type Key = [u8; 32];

/// One migration per entry. Never edit an old one; add a new one.
const MIGRATIONS: &[&str] = &[
    // 1: the four V1 tables
    "
    CREATE TABLE defaults (
        id INTEGER PRIMARY KEY,
        slot_type TEXT NOT NULL,
        phrase TEXT NOT NULL,
        context TEXT,
        value TEXT NOT NULL,
        uses INTEGER DEFAULT 1,
        last_used TEXT NOT NULL
    );
    CREATE TABLE action_log (
        id INTEGER PRIMARY KEY,
        at TEXT NOT NULL,
        request TEXT NOT NULL,
        skill_id TEXT,
        decision TEXT NOT NULL,
        result TEXT,
        undo_token TEXT
    );
    CREATE TABLE reminders (
        id INTEGER PRIMARY KEY,
        due_at TEXT NOT NULL,
        text TEXT NOT NULL,
        repeat TEXT,
        done INTEGER DEFAULT 0
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    ",
];

/// Whether this build can encrypt the database.
pub const ENCRYPTION_AVAILABLE: bool = cfg!(feature = "sqlcipher");

/// Allowed values of `action_log.decision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Ran,
    Confirmed,
    Cancelled,
    Blocked,
    Failed,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Decision::Ran => "ran",
            Decision::Confirmed => "confirmed",
            Decision::Cancelled => "cancelled",
            Decision::Blocked => "blocked",
            Decision::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub id: i64,
    pub at: String,
    pub request: String,
    pub skill_id: Option<String>,
    pub decision: String,
    pub result: Option<String>,
    pub undo_token: Option<String>,
}

/// A reminder or timer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: i64,
    pub due_at: String,
    pub text: String,
}

/// True if `path` reads as an ordinary (unencrypted) SQLite database.
fn is_plain_database(path: &Path) -> bool {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .and_then(|c| {
            c.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })
        })
        .is_ok()
}

const COUNTED_TABLES: [&str; 4] = ["defaults", "action_log", "reminders", "settings"];

fn table_counts(conn: &Connection, schema: &str) -> Result<Vec<i64>, StoreError> {
    COUNTED_TABLES
        .iter()
        .map(|t| {
            conn.query_row(&format!("SELECT count(*) FROM {schema}.{t}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(StoreError::from)
        })
        .collect()
}

fn side_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Plain -> encrypted. The original is never removed: it is backed up first and
/// only replaced after the encrypted copy has been opened and its counts match.
fn encrypt_plain_in_place(path: &Path, key: &Key) -> Result<(), StoreError> {
    let io =
        |e: std::io::Error| StoreError::Db(rusqlite::Error::ToSqlConversionFailure(Box::new(e)));
    let tmp = side_path(path, ".enc.tmp");
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(side_path(&tmp, "-journal"));

    let plain = Connection::open(path)?;
    plain.busy_timeout(std::time::Duration::from_secs(5))?;
    // Fold any write-ahead log into the main file so the backup is complete.
    let busy: i64 = plain.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        return Err(StoreError::Db(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("database is in use; could not prepare it for encryption".into()),
        )));
    }

    // Backup, never overwriting an earlier one.
    let mut bak = side_path(path, ".plain.bak");
    let mut n = 0;
    while bak.exists() {
        n += 1;
        bak = side_path(path, &format!(".plain.bak.{n}"));
    }
    std::fs::copy(path, &bak).map_err(io)?;

    // Export into a new encrypted file.
    let version: u32 = plain.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let exported = (|| -> Result<Vec<i64>, StoreError> {
        plain.execute(
            "ATTACH DATABASE ?1 AS encrypted KEY ?2",
            params![tmp.to_string_lossy(), format!("x'{hex}'")],
        )?;
        plain.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))?;
        plain.execute_batch(&format!("PRAGMA encrypted.user_version = {version}"))?;
        let new_counts = table_counts(&plain, "encrypted")?;
        let old_counts = table_counts(&plain, "main")?;
        if new_counts != old_counts {
            return Err(StoreError::BadKey);
        }
        plain.execute_batch("DETACH DATABASE encrypted")?;
        Ok(old_counts)
    })();
    drop(plain);
    let counts = match exported {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };

    // Verify by opening the new file the way the app will.
    let check = Store::open(&tmp, Some(key)).and_then(|s| table_counts(&s.conn, "main"));
    match check {
        Ok(c) if c == counts => {}
        other => {
            let _ = std::fs::remove_file(&tmp);
            return Err(other.err().unwrap_or(StoreError::BadKey));
        }
    }

    // Swap. Leftover log files belong to the old plain file.
    let _ = std::fs::remove_file(side_path(path, "-wal"));
    let _ = std::fs::remove_file(side_path(path, "-shm"));
    let _ = std::fs::remove_file(side_path(path, "-journal"));
    std::fs::rename(&tmp, path).map_err(io)?;
    Ok(())
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) the database at `path`, then run any missing migrations.
    pub fn open(path: &Path, key: Option<&Key>) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        // The app, the reminder scheduler and the reminders connector each hold
        // their own connection to this file; wait briefly instead of failing.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init(conn, key)
    }

    /// Open `path` encrypted with `key`, converting an existing plain database.
    ///
    /// * No file yet: a new encrypted database is created.
    /// * File opens with the key: used as is.
    /// * File is a plain SQLite database: it is first copied to
    ///   `<name>.plain.bak` (a numbered suffix is used if that name is taken),
    ///   exported into a new encrypted file with `sqlcipher_export`, the row
    ///   counts of the four tables are compared, and only then does the new file
    ///   replace the old one. The backup is kept.
    /// * File opens with neither: [`StoreError::BadKey`]; nothing is touched.
    ///
    /// Without the `sqlcipher` feature this returns
    /// [`StoreError::EncryptionUnavailable`] and does not touch the file.
    pub fn open_encrypted_migrating(path: &Path, key: &Key) -> Result<Self, StoreError> {
        if !ENCRYPTION_AVAILABLE {
            return Err(StoreError::EncryptionUnavailable);
        }
        let empty = std::fs::metadata(path)
            .map(|m| m.len() == 0)
            .unwrap_or(true);
        if empty {
            return Self::open(path, Some(key));
        }
        match Self::open(path, Some(key)) {
            Ok(store) => return Ok(store),
            Err(StoreError::BadKey) => {}
            Err(e) => return Err(e),
        }
        if !is_plain_database(path) {
            return Err(StoreError::BadKey);
        }
        encrypt_plain_in_place(path, key)?;
        Self::open(path, Some(key))
    }

    /// In-memory database for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, key: Option<&Key>) -> Result<Self, StoreError> {
        if let Some(key) = key {
            if !ENCRYPTION_AVAILABLE {
                return Err(StoreError::EncryptionUnavailable);
            }
            let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
            // Raw-key form: x'<64 hex digits>'. Avoids key stretching on a random key.
            conn.pragma_update(None, "key", format!("x'{hex}'"))?;
            // Reading anything fails if the key is wrong.
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(|_| StoreError::BadKey)?;
        }
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let found: u32 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let known = MIGRATIONS.len() as u32;
        if found > known {
            return Err(StoreError::SchemaTooNew { found, known });
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as u32)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32, StoreError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    // --- settings -------------------------------------------------------

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    // --- action log -----------------------------------------------------

    /// Append an entry. `at` is an ISO-8601 timestamp supplied by the caller.
    pub fn log_action(
        &self,
        at: &str,
        request: &str,
        skill_id: Option<&str>,
        decision: Decision,
        result: Option<&str>,
        undo_token: Option<&str>,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO action_log(at, request, skill_id, decision, result, undo_token)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![at, request, skill_id, decision.as_str(), result, undo_token],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Most recent entries first.
    pub fn recent_actions(&self, limit: u32) -> Result<Vec<LogEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, at, request, skill_id, decision, result, undo_token
             FROM action_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok(LogEntry {
                id: r.get(0)?,
                at: r.get(1)?,
                request: r.get(2)?,
                skill_id: r.get(3)?,
                decision: r.get(4)?,
                result: r.get(5)?,
                undo_token: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Makes every later log write fail, so tests can check that a broken log
    /// never turns a finished action into an error. Not for production code.
    #[doc(hidden)]
    pub fn break_action_log_for_tests(&self) {
        let _ = self.conn.execute("DROP TABLE action_log", []);
    }

    // --- learned defaults (FR10) -----------------------------------------

    /// Remember that, for slots of this type, the words `phrase` ended in `value`.
    pub fn record_default(
        &self,
        slot_type: &str,
        phrase: &str,
        value: &str,
        at: &str,
    ) -> Result<(), StoreError> {
        let changed = self.conn.execute(
            "UPDATE defaults SET uses = uses + 1, last_used = ?4
             WHERE slot_type = ?1 AND phrase = ?2 AND value = ?3",
            params![slot_type, phrase, value, at],
        )?;
        if changed == 0 {
            self.conn.execute(
                "INSERT INTO defaults(slot_type, phrase, value, uses, last_used)
                 VALUES(?1, ?2, ?3, 1, ?4)",
                params![slot_type, phrase, value, at],
            )?;
        }
        Ok(())
    }

    /// What these exact words ended in before (most used, then most recent).
    pub fn default_for_phrase(
        &self,
        slot_type: &str,
        phrase: &str,
    ) -> Result<Option<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT value FROM defaults WHERE slot_type = ?1 AND phrase = ?2
             ORDER BY uses DESC, last_used DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![slot_type, phrase])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// The user's usual value for this slot type: the one chosen at least twice, most used first.
    pub fn usual_for(&self, slot_type: &str) -> Result<Option<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT value FROM defaults WHERE slot_type = ?1
             GROUP BY value HAVING SUM(uses) >= 2
             ORDER BY SUM(uses) DESC, MAX(last_used) DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([slot_type])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    // --- reminders ------------------------------------------------------

    /// Add a reminder. `due_at` is UTC as `YYYY-MM-DDTHH:MM:SSZ`, so text order is time order.
    pub fn add_reminder(&self, due_at: &str, text: &str) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO reminders(due_at, text) VALUES(?1, ?2)",
            params![due_at, text],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Reminders not yet shown whose time has come, oldest first.
    pub fn due_reminders(&self, now: &str) -> Result<Vec<Reminder>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, due_at, text FROM reminders
             WHERE done = 0 AND due_at <= ?1 ORDER BY due_at, id",
        )?;
        let rows = stmt.query_map([now], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                due_at: r.get(1)?,
                text: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Reminders still waiting (not shown, not deleted), soonest first.
    pub fn pending_reminders(&self) -> Result<Vec<Reminder>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, due_at, text FROM reminders WHERE done = 0 ORDER BY due_at, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                due_at: r.get(1)?,
                text: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn mark_reminder_done(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("UPDATE reminders SET done = 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Remove a reminder (Undo of "remind me").
    pub fn delete_reminder(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM reminders WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Clear the undo token once Undo is used or no longer possible.
    pub fn clear_undo_token(&self, id: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE action_log SET undo_token = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminders_come_due_in_time_order_and_only_once() {
        let s = Store::open_in_memory().unwrap();
        let late = s.add_reminder("2026-11-08T18:00:00Z", "call mom").unwrap();
        let early = s.add_reminder("2026-11-08T12:10:00Z", "timer").unwrap();
        assert!(s.due_reminders("2026-11-08T12:00:00Z").unwrap().is_empty());
        let due = s.due_reminders("2026-11-08T12:10:00Z").unwrap();
        assert_eq!(due.iter().map(|r| r.id).collect::<Vec<_>>(), [early]);
        s.mark_reminder_done(early).unwrap();
        assert!(s.due_reminders("2026-11-08T12:10:00Z").unwrap().is_empty());
        assert_eq!(s.pending_reminders().unwrap().len(), 1);
        assert_eq!(s.due_reminders("2026-11-09T00:00:00Z").unwrap()[0].id, late);
    }

    #[test]
    fn defaults_are_learned_per_phrase_and_a_usual_needs_two_uses() {
        let s = Store::open_in_memory().unwrap();
        s.record_default("contact", "ved", "Ved Narvekar", "2026-11-08T10:00:00Z")
            .unwrap();
        assert_eq!(
            s.default_for_phrase("contact", "ved").unwrap().as_deref(),
            Some("Ved Narvekar")
        );
        assert_eq!(
            s.usual_for("contact").unwrap(),
            None,
            "one use is not a habit"
        );
        s.record_default("contact", "ved", "Ved Narvekar", "2026-11-08T11:00:00Z")
            .unwrap();
        assert_eq!(
            s.usual_for("contact").unwrap().as_deref(),
            Some("Ved Narvekar")
        );
        assert_eq!(s.default_for_phrase("contact", "mom").unwrap(), None);
        assert_eq!(
            s.usual_for("app").unwrap(),
            None,
            "other slot types are separate"
        );
    }

    #[test]
    fn a_deleted_reminder_never_fires() {
        let s = Store::open_in_memory().unwrap();
        let id = s.add_reminder("2026-11-08T12:10:00Z", "x").unwrap();
        s.delete_reminder(id).unwrap();
        assert!(s.due_reminders("2027-01-01T00:00:00Z").unwrap().is_empty());
    }

    #[test]
    fn migrations_create_all_four_tables() {
        let s = Store::open_in_memory().unwrap();
        let n: i64 = s
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'
                 AND name IN ('defaults','action_log','reminders','settings')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 4);
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as u32);
    }

    #[test]
    fn reopening_does_not_rerun_migrations() {
        let dir = std::env::temp_dir().join(format!("jitmo-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.db");
        {
            let s = Store::open(&path, None).unwrap();
            s.set_setting("k", "v").unwrap();
        }
        let s = Store::open(&path, None).unwrap();
        assert_eq!(s.get_setting("k").unwrap().as_deref(), Some("v"));
        drop(s);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn newer_schema_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99u32).unwrap();
        assert!(matches!(
            Store::init(conn, None),
            Err(StoreError::SchemaTooNew { found: 99, .. })
        ));
    }

    #[test]
    fn settings_upsert() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.get_setting("hotkey").unwrap(), None);
        s.set_setting("hotkey", "Ctrl+Alt+J").unwrap();
        s.set_setting("hotkey", "Ctrl+Shift+J").unwrap();
        assert_eq!(
            s.get_setting("hotkey").unwrap().as_deref(),
            Some("Ctrl+Shift+J")
        );
    }

    #[test]
    fn action_log_newest_first_and_undo_token_clears() {
        let s = Store::open_in_memory().unwrap();
        s.log_action(
            "2026-11-08T10:00:00Z",
            "mute",
            Some("system.mute"),
            Decision::Ran,
            Some("Muted."),
            None,
        )
        .unwrap();
        let id = s
            .log_action(
                "2026-11-08T10:01:00Z",
                "delete x",
                Some("files.delete"),
                Decision::Confirmed,
                None,
                Some("tok"),
            )
            .unwrap();
        let rows = s.recent_actions(10).unwrap();
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].decision, "confirmed");
        assert_eq!(rows[0].undo_token.as_deref(), Some("tok"));
        s.clear_undo_token(id).unwrap();
        assert_eq!(s.recent_actions(1).unwrap()[0].undo_token, None);
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn encrypted_file_is_unreadable_without_the_right_key() {
        let dir = std::env::temp_dir().join(format!("jitmo-enc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("enc.db");
        let key = [9u8; 32];
        {
            let s = Store::open(&path, Some(&key)).unwrap();
            s.set_setting("secret", "hunter2").unwrap();
        }
        // Plain SQLite files start with this header; an encrypted one must not.
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3"));
        assert!(!bytes.windows(7).any(|w| w == b"hunter2"));
        // Right key works, wrong key and no key are refused.
        let s = Store::open(&path, Some(&key)).unwrap();
        assert_eq!(s.get_setting("secret").unwrap().as_deref(), Some("hunter2"));
        drop(s);
        assert!(matches!(
            Store::open(&path, Some(&[1u8; 32])),
            Err(StoreError::BadKey)
        ));
        assert!(Store::open(&path, None).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("jitmo-mig-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(not(feature = "sqlcipher"))]
    #[test]
    fn migrating_open_refuses_and_leaves_the_file_alone_without_sqlcipher() {
        let dir = temp_dir("nofeat");
        let path = dir.join("p.db");
        Store::open(&path, None)
            .unwrap()
            .set_setting("a", "1")
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        let r = Store::open_encrypted_migrating(&path, &[3u8; 32]);
        assert!(matches!(r, Err(StoreError::EncryptionUnavailable)));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.join("p.db.plain.bak").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn migrating_open_creates_a_new_encrypted_file() {
        let dir = temp_dir("new");
        let path = dir.join("n.db");
        let key = [5u8; 32];
        Store::open_encrypted_migrating(&path, &key)
            .unwrap()
            .set_setting("k", "v")
            .unwrap();
        assert!(!std::fs::read(&path)
            .unwrap()
            .starts_with(b"SQLite format 3"));
        assert!(!dir.join("n.db.plain.bak").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn plain_database_is_converted_keeping_rows_and_a_backup() {
        let dir = temp_dir("plain");
        let path = dir.join("p.db");
        let key = [6u8; 32];
        {
            let s = Store::open(&path, None).unwrap();
            s.set_setting("hotkey", "Ctrl+J").unwrap();
            s.add_reminder("2026-11-08T12:10:00Z", "call mom").unwrap();
            s.log_action(
                "2026-11-08T10:00:00Z",
                "mute",
                None,
                Decision::Ran,
                None,
                None,
            )
            .unwrap();
            s.record_default("contact", "ved", "Ved", "2026-11-08T10:00:00Z")
                .unwrap();
        }
        let s = Store::open_encrypted_migrating(&path, &key).unwrap();
        assert_eq!(s.get_setting("hotkey").unwrap().as_deref(), Some("Ctrl+J"));
        assert_eq!(s.pending_reminders().unwrap().len(), 1);
        assert_eq!(s.recent_actions(10).unwrap().len(), 1);
        assert_eq!(
            s.default_for_phrase("contact", "ved").unwrap().as_deref(),
            Some("Ved")
        );
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len() as u32);
        drop(s);
        // The live file is now encrypted; the backup is still plain and complete.
        assert!(!std::fs::read(&path)
            .unwrap()
            .starts_with(b"SQLite format 3"));
        assert!(Store::open(&path, None).is_err());
        let bak = dir.join("p.db.plain.bak");
        assert!(std::fs::read(&bak).unwrap().starts_with(b"SQLite format 3"));
        let old = Store::open(&bak, None).unwrap();
        assert_eq!(
            old.get_setting("hotkey").unwrap().as_deref(),
            Some("Ctrl+J")
        );
        assert!(!dir.join("p.db.enc.tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn encrypted_database_reopens_and_makes_no_backup() {
        let dir = temp_dir("reopen");
        let path = dir.join("e.db");
        let key = [7u8; 32];
        Store::open_encrypted_migrating(&path, &key)
            .unwrap()
            .set_setting("k", "v")
            .unwrap();
        let s = Store::open_encrypted_migrating(&path, &key).unwrap();
        assert_eq!(s.get_setting("k").unwrap().as_deref(), Some("v"));
        assert!(!dir.join("e.db.plain.bak").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn wrong_key_is_refused_and_the_file_is_untouched() {
        let dir = temp_dir("wrong");
        let path = dir.join("w.db");
        Store::open_encrypted_migrating(&path, &[8u8; 32])
            .unwrap()
            .set_setting("k", "v")
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        let r = Store::open_encrypted_migrating(&path, &[9u8; 32]);
        assert!(matches!(r, Err(StoreError::BadKey)));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.join("w.db.plain.bak").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(feature = "sqlcipher")]
    #[test]
    fn a_file_that_is_not_a_database_is_refused_and_kept() {
        let dir = temp_dir("junk");
        let path = dir.join("j.db");
        std::fs::write(&path, vec![0xABu8; 4096]).unwrap();
        let r = Store::open_encrypted_migrating(&path, &[1u8; 32]);
        assert!(matches!(r, Err(StoreError::BadKey)));
        assert_eq!(std::fs::read(&path).unwrap(), vec![0xABu8; 4096]);
        assert!(!dir.join("j.db.plain.bak").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(not(feature = "sqlcipher"))]
    #[test]
    fn key_is_refused_without_sqlcipher() {
        let r = Store::init(Connection::open_in_memory().unwrap(), Some(&[7u8; 32]));
        assert!(matches!(r, Err(StoreError::EncryptionUnavailable)));
    }
}
