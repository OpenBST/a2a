// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! SQLite credential + model-alias store: plaintext API keys + simple
//! metadata + project-shared model alias definitions.
//!
//! Schema is intentionally minimal (SPEC §4.1):
//!   - `profiles(name, api_key, created_at, last_used_at, note)`
//!   - `meta(key, value)` — currently only `default_profile`
//!   - `model_aliases(alias, cursor_model, default_mode, thinking_hint,
//!     description, created_at)` — user-global, shared across all
//!     a2a projects on this machine
//!
//! No encryption, no `disabled_until` state machine, no `api_key_hash`
//! dedup index, no `[fallback]` chain table. Per SPEC §2: trust the
//! user's machine; don't reinvent `chmod 0600`. Per SPEC §11
//! simplification (post-config refactor): all configuration that used
//! to live in TOML now lives here or is hardcoded in `crate::defaults`.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub api_key: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub note: Option<String>,
}

/// Outcome of `try_insert_model_alias`: `Inserted` on success,
/// `AlreadyExists` when a row with the same primary key already
/// existed (the SQLite `ConstraintViolation` was caught and the
/// table is unchanged).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryInsertOutcome {
    Inserted,
    AlreadyExists,
}

/// User-global model alias row. SPEC §11.
#[derive(Debug, Clone)]
pub struct ModelAlias {
    pub alias: String,
    /// Value passed to `cursor-agent --model`.
    pub cursor_model: String,
    /// `"agent"` (default) or `"plan"`. SPEC §8.0.
    pub default_mode: String,
    /// Optional human-facing note ("Opus 4.7 1M Thinking — best for
    /// architecture review"). Display-only; no runtime effect.
    pub thinking_hint: Option<String>,
    /// Optional human-friendly label for `a2a models list`.
    pub description: Option<String>,
    /// Wall-clock seconds at insert time. The "first added alias"
    /// semantics for `default_models` resolution (`a2a ask` without
    /// `--models`) sorts by this column.
    pub created_at: i64,
}

pub struct CredStore {
    conn: Connection,
    #[allow(dead_code)]
    path: PathBuf,
}

pub fn open() -> Result<CredStore> {
    let path = crate::paths::credentials_db_path()?;

    // SPEC §15: detect a legacy v0.x schema (`api_key BLOB`,
    // `encrypted` column, `disabled_until`, etc.) that the current
    // code can't read. Back it up and start fresh — the v1
    // simplification removes encryption and the disable state
    // machine, so an in-place ALTER would be a futile dance.
    if path.exists() {
        let probe = Connection::open(&path)
            .with_context(|| format!("probe legacy schema in {}", path.display()))?;
        if needs_legacy_backup(&probe)? {
            drop(probe);
            let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            let backup = path.with_file_name(format!(
                "{}.legacy-bak-{}",
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("credentials.db"),
                stamp
            ));
            std::fs::rename(&path, &backup).with_context(|| {
                format!("back up legacy credentials.db to {}", backup.display())
            })?;
            tracing::warn!(
                "credentials.db: detected legacy v0.x schema (encrypted profiles or BLOB \
                 api_key); backed up as {} and starting a fresh empty store. \
                 Re-run `a2a auth add` to register your API keys.",
                backup.display()
            );
            eprintln!(
                "[a2a] credentials.db schema is from an older a2a version (encrypted / BLOB \
                 keys). The previous file has been preserved at:"
            );
            eprintln!("    {}", backup.display());
            eprintln!("[a2a] Starting fresh. Re-run `a2a auth add` to register your API keys.");
        }
    }

    let conn = Connection::open(&path)
        .with_context(|| format!("open SQLite database {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .context("set SQLite busy_timeout")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }
    let store = CredStore { conn, path };
    store.ensure_schema()?;
    Ok(store)
}

/// Return true if the on-disk `profiles` table has a v0.x shape
/// (`api_key BLOB`, or columns like `encrypted`/`disabled_until`/etc
/// that v1 doesn't carry). New empty / fresh-v1 databases return false.
fn needs_legacy_backup(conn: &Connection) -> Result<bool> {
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='profiles'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !table_exists {
        return Ok(false);
    }
    let mut stmt = conn.prepare("PRAGMA table_info(profiles)")?;
    let columns: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, ty) in &columns {
        if name == "api_key" && ty.eq_ignore_ascii_case("BLOB") {
            return Ok(true);
        }
        if matches!(
            name.as_str(),
            "encrypted" | "salt" | "nonce" | "disabled_until" | "api_key_hash"
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

impl CredStore {
    fn ensure_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS profiles (
                name           TEXT PRIMARY KEY,
                api_key        TEXT NOT NULL,
                created_at     INTEGER NOT NULL,
                last_used_at   INTEGER,
                note           TEXT
            );
            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE IF NOT EXISTS model_aliases (
                alias         TEXT PRIMARY KEY,
                cursor_model  TEXT NOT NULL,
                default_mode  TEXT NOT NULL DEFAULT 'agent',
                thinking_hint TEXT,
                description   TEXT,
                created_at    INTEGER NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    // ---------- model_aliases CRUD ----------

    pub fn model_alias_exists(&self, alias: &str) -> Result<bool> {
        let cnt: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM model_aliases WHERE alias = ?1",
            params![alias],
            |r| r.get(0),
        )?;
        Ok(cnt > 0)
    }

    pub fn insert_model_alias(&mut self, m: &ModelAlias) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO model_aliases
               (alias, cursor_model, default_mode, thinking_hint, description, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                m.alias,
                m.cursor_model,
                m.default_mode,
                m.thinking_hint,
                m.description,
                m.created_at
            ],
        )?;
        Ok(())
    }

    /// Replace an existing alias's row entirely. UPSERT semantics
    /// (`ON CONFLICT DO UPDATE`). Used by `a2a models add --force`
    /// (re-define the alias from scratch) where overwriting an
    /// existing row is the explicit intent.
    pub fn replace_model_alias(&mut self, m: &ModelAlias) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO model_aliases
               (alias, cursor_model, default_mode, thinking_hint, description, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)
               ON CONFLICT(alias) DO UPDATE SET
                 cursor_model  = excluded.cursor_model,
                 default_mode  = excluded.default_mode,
                 thinking_hint = excluded.thinking_hint,
                 description   = excluded.description"#,
            params![
                m.alias,
                m.cursor_model,
                m.default_mode,
                m.thinking_hint,
                m.description,
                m.created_at
            ],
        )?;
        Ok(())
    }

    /// Plain INSERT — fails when a row with the same `alias` exists.
    /// Used by `a2a models add` **without** `--force` so two
    /// concurrent inserts can't both pass an `exists()` pre-check
    /// and silently UPSERT-overwrite each other (TOCTOU race
    /// flagged in r20 review). The caller maps the SQLite
    /// `ConstraintViolation` to a friendly "alias already
    /// registered" UserError.
    pub fn try_insert_model_alias(&mut self, m: &ModelAlias) -> Result<TryInsertOutcome> {
        match self.conn.execute(
            r#"INSERT INTO model_aliases
               (alias, cursor_model, default_mode, thinking_hint, description, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                m.alias,
                m.cursor_model,
                m.default_mode,
                m.thinking_hint,
                m.description,
                m.created_at
            ],
        ) {
            Ok(_) => Ok(TryInsertOutcome::Inserted),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(TryInsertOutcome::AlreadyExists)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_model_alias(&self, alias: &str) -> Result<Option<ModelAlias>> {
        let row = self.conn.query_row(
            "SELECT alias, cursor_model, default_mode, thinking_hint, description, created_at \
             FROM model_aliases WHERE alias = ?1",
            params![alias],
            |r| {
                Ok(ModelAlias {
                    alias: r.get(0)?,
                    cursor_model: r.get(1)?,
                    default_mode: r.get(2)?,
                    thinking_hint: r.get(3)?,
                    description: r.get(4)?,
                    created_at: r.get(5)?,
                })
            },
        );
        match row {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all aliases, ordered by `created_at` ascending so the
    /// **first row** is the earliest-added alias. SPEC §11 says
    /// `a2a ask` without `--models` runs only the first-added alias.
    pub fn list_model_aliases(&self) -> Result<Vec<ModelAlias>> {
        let mut stmt = self.conn.prepare(
            "SELECT alias, cursor_model, default_mode, thinking_hint, description, created_at \
             FROM model_aliases ORDER BY created_at ASC, alias ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ModelAlias {
                alias: r.get(0)?,
                cursor_model: r.get(1)?,
                default_mode: r.get(2)?,
                thinking_hint: r.get(3)?,
                description: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn delete_model_alias(&mut self, alias: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM model_aliases WHERE alias = ?1", params![alias])?;
        Ok(n > 0)
    }

    /// Wipe the alias table (used by `a2a reset models --yes`).
    pub fn delete_all_model_aliases(&mut self) -> Result<usize> {
        let n = self.conn.execute("DELETE FROM model_aliases", [])?;
        Ok(n)
    }

    /// Apply scalar field updates to an existing alias. Returns
    /// `Ok(false)` if the alias does not exist (so callers can surface
    /// a friendlier "alias not defined" message). Each parameter is
    /// `Option<...>`: `None` means "leave the column unchanged",
    /// `Some(v)` means "replace with v" (including empty strings,
    /// which clear the column for nullable fields).
    pub fn update_model_alias_fields(
        &mut self,
        alias: &str,
        cursor_model: Option<&str>,
        default_mode: Option<&str>,
        thinking_hint: Option<Option<&str>>,
        description: Option<Option<&str>>,
    ) -> Result<bool> {
        if !self.model_alias_exists(alias)? {
            return Ok(false);
        }
        if let Some(v) = cursor_model {
            self.conn.execute(
                "UPDATE model_aliases SET cursor_model = ?1 WHERE alias = ?2",
                params![v, alias],
            )?;
        }
        if let Some(v) = default_mode {
            self.conn.execute(
                "UPDATE model_aliases SET default_mode = ?1 WHERE alias = ?2",
                params![v, alias],
            )?;
        }
        if let Some(v) = thinking_hint {
            self.conn.execute(
                "UPDATE model_aliases SET thinking_hint = ?1 WHERE alias = ?2",
                params![v, alias],
            )?;
        }
        if let Some(v) = description {
            self.conn.execute(
                "UPDATE model_aliases SET description = ?1 WHERE alias = ?2",
                params![v, alias],
            )?;
        }
        Ok(true)
    }

    pub fn profile_exists(&self, name: &str) -> Result<bool> {
        let cnt: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM profiles WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )?;
        Ok(cnt > 0)
    }

    pub fn insert_profile(&mut self, p: &Profile) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO profiles (name, api_key, created_at, last_used_at, note)
               VALUES (?1, ?2, ?3, ?4, ?5)"#,
            params![p.name, p.api_key, p.created_at, p.last_used_at, p.note],
        )?;
        Ok(())
    }

    /// Cheap COUNT(*) probe — returns true when the profile table is
    /// empty. Used by the fallback runner after a KeyDead-triggered
    /// delete to short-circuit further alias attempts when there are
    /// no credentials left in the store at all.
    pub fn is_empty(&self) -> Result<bool> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM profiles", [], |r| r.get(0))?;
        Ok(n == 0)
    }

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, api_key, created_at, last_used_at, note \
             FROM profiles ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Profile {
                name: r.get(0)?,
                api_key: r.get(1)?,
                created_at: r.get(2)?,
                last_used_at: r.get(3)?,
                note: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn get_profile(&self, name: &str) -> Result<Option<Profile>> {
        let row = self.conn.query_row(
            "SELECT name, api_key, created_at, last_used_at, note \
             FROM profiles WHERE name = ?1",
            params![name],
            |r| {
                Ok(Profile {
                    name: r.get(0)?,
                    api_key: r.get(1)?,
                    created_at: r.get(2)?,
                    last_used_at: r.get(3)?,
                    note: r.get(4)?,
                })
            },
        );
        match row {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_profile(&mut self, name: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM profiles WHERE name = ?1", params![name])?;
        // Vacuum is unnecessary on every delete; run it explicitly via a
        // separate API if a user requests it. Skipping here keeps
        // `delete_profile` cheap (called from KeyDead path during a
        // running consultation).
        Ok(())
    }

    pub fn update_profile_key(&mut self, name: &str, api_key: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET api_key = ?1 WHERE name = ?2",
            params![api_key, name],
        )?;
        Ok(())
    }

    pub fn record_last_used(&mut self, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE profiles SET last_used_at = ?1 WHERE name = ?2",
            params![Utc::now().timestamp(), name],
        )?;
        Ok(())
    }

    pub fn get_default_profile(&self) -> Result<Option<String>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'default_profile'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.filter(|s| !s.is_empty()))
    }

    pub fn set_default_profile(&mut self, name: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('default_profile', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![name],
        )?;
        Ok(())
    }

    pub fn clear_default_profile(&mut self) -> Result<()> {
        self.conn
            .execute("DELETE FROM meta WHERE key = 'default_profile'", [])?;
        Ok(())
    }
}

#[cfg(test)]
impl CredStore {
    /// In-memory store for unit tests. Skips the on-disk path
    /// resolution + legacy-schema migration; just creates the schema
    /// in a private SQLite connection.
    pub(crate) fn in_memory_for_test() -> Result<CredStore> {
        let conn = Connection::open_in_memory()?;
        let store = CredStore {
            conn,
            path: PathBuf::from(":memory:"),
        };
        store.ensure_schema()?;
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::{CredStore, Profile};

    fn add(db: &mut CredStore, name: &str) {
        db.insert_profile(&Profile {
            name: name.into(),
            api_key: format!("k_{name}"),
            created_at: 1_700_000_000,
            last_used_at: None,
            note: None,
        })
        .unwrap();
    }

    #[test]
    fn empty_store_reports_empty() {
        let db = CredStore::in_memory_for_test().unwrap();
        assert!(db.is_empty().unwrap());
    }

    #[test]
    fn populated_store_reports_not_empty() {
        let mut db = CredStore::in_memory_for_test().unwrap();
        add(&mut db, "default");
        assert!(!db.is_empty().unwrap());
    }

    #[test]
    fn delete_last_profile_makes_store_empty() {
        let mut db = CredStore::in_memory_for_test().unwrap();
        add(&mut db, "only");
        assert!(!db.is_empty().unwrap());
        db.delete_profile("only").unwrap();
        assert!(db.is_empty().unwrap());
    }

    #[test]
    fn delete_one_of_many_keeps_store_populated() {
        let mut db = CredStore::in_memory_for_test().unwrap();
        add(&mut db, "a");
        add(&mut db, "b");
        db.delete_profile("a").unwrap();
        assert!(!db.is_empty().unwrap());
        db.delete_profile("b").unwrap();
        assert!(db.is_empty().unwrap());
    }
}
