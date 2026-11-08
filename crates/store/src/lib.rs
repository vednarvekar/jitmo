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

    #[cfg(not(feature = "sqlcipher"))]
    #[test]
    fn key_is_refused_without_sqlcipher() {
        let r = Store::init(Connection::open_in_memory().unwrap(), Some(&[7u8; 32]));
        assert!(matches!(r, Err(StoreError::EncryptionUnavailable)));
    }
}
