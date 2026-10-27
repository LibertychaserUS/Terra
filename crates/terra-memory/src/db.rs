use std::path::Path;
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::error::MemoryError;
use crate::types::{Level, MemoryEntry, Scope};

/// The memory database: SQLite in WAL mode with an FTS5 index over content.
///
/// The connection sits behind a `Mutex` so the database is `Send + Sync`;
/// a CLI session issues memory commands one at a time, so contention is nil.
pub struct MemoryDatabase {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct GcReport {
    pub entries_removed: usize,
}

/// One search result: the entry key and its FTS5 rank (lower is better,
/// reported as a positive score where higher is better).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub key: String,
    pub score: f64,
    pub snippet: String,
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA user_version = 2;

CREATE TABLE IF NOT EXISTS memories (
    id          TEXT PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    scope       TEXT NOT NULL CHECK (scope IN ('global','project','session','task')),
    level       TEXT NOT NULL CHECK (level IN ('L0','L1','L2','L3','L4','L5','L6')),
    owner       TEXT NOT NULL,
    content     TEXT NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1,
    alive       INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope);
CREATE INDEX IF NOT EXISTS idx_memories_level ON memories(level);

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    key UNINDEXED, content, content='memories', content_rowid='rowid'
);
CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, key, content) VALUES (new.rowid, new.key, new.content);
END;
CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content) VALUES ('delete', old.rowid, old.key, old.content);
END;
CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE OF content ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, key, content) VALUES ('delete', old.rowid, old.key, old.content);
    INSERT INTO memories_fts(rowid, key, content) VALUES (new.rowid, new.key, new.content);
END;
";

const SELECT_ENTRY: &str =
    "SELECT id, key, scope, level, owner, content, version, alive, created_at, updated_at FROM memories";

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    let parse_time = |s: String| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_default()
    };
    Ok(MemoryEntry {
        id: row.get(0)?,
        key: row.get(1)?,
        scope: Scope::parse(&row.get::<_, String>(2)?).unwrap_or(Scope::Task),
        level: Level::parse(&row.get::<_, String>(3)?).unwrap_or(Level::L2),
        owner: row.get(4)?,
        content: row.get(5)?,
        version: row.get(6)?,
        alive: row.get(7)?,
        created_at: parse_time(row.get(8)?),
        updated_at: parse_time(row.get(9)?),
    })
}

impl MemoryDatabase {
    /// Open or create the database file.
    pub fn open(path: &Path) -> Result<Self, MemoryError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// An in-memory database (tests, and projects without `.terra/`).
    pub fn open_in_memory() -> Result<Self, MemoryError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, MemoryError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned lock means another thread panicked mid-query; the
        // connection itself is still usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Insert a new entry owned by `owner`. Keys are unique.
    pub fn write(
        &self,
        key: impl Into<String>,
        scope: Scope,
        level: Level,
        owner: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<(), MemoryError> {
        let key = key.into();
        let now = Utc::now().to_rfc3339();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO memories (id, key, scope, level, owner, content, version, alive, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 1, ?7, ?7)",
            params![
                Uuid::new_v4().to_string(),
                key,
                scope.as_str(),
                level.as_str(),
                owner.into(),
                content.into(),
                now,
            ],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(ref err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                MemoryError::DuplicateKey(key.clone())
            }
            other => MemoryError::Database(other),
        })?;
        Ok(())
    }

    /// Read a live entry. A soft-deleted key is reported as dangling.
    pub fn read(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let entry = self.read_any(key)?;
        match entry {
            Some(e) if !e.alive => Err(MemoryError::Dangling(key.to_string())),
            other => Ok(other),
        }
    }

    fn read_any(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let conn = self.lock();
        Ok(conn
            .query_row(&format!("{SELECT_ENTRY} WHERE key = ?1"), params![key], row_to_entry)
            .optional()?)
    }

    /// Who owns `key`, if it exists and is alive.
    pub fn owner_of(&self, key: &str) -> Result<Option<String>, MemoryError> {
        Ok(self.read_any(key)?.filter(|e| e.alive).map(|e| e.owner))
    }

    fn check_owner(&self, key: &str, agent: &str) -> Result<MemoryEntry, MemoryError> {
        let entry = self.read_any(key)?.ok_or_else(|| MemoryError::NotFound(key.to_string()))?;
        if !entry.alive {
            return Err(MemoryError::Dangling(key.to_string()));
        }
        if entry.owner != agent {
            return Err(MemoryError::PermissionDenied { agent: agent.to_string(), key: key.to_string() });
        }
        Ok(entry)
    }

    /// Replace the content of an entry you own. Returns the new version.
    pub fn update(&self, key: &str, content: impl Into<String>, agent: &str) -> Result<u64, MemoryError> {
        let entry = self.check_owner(key, agent)?;
        let new_version = entry.version + 1;
        let conn = self.lock();
        let rows = conn.execute(
            "UPDATE memories SET content = ?1, version = ?2, updated_at = ?3 WHERE key = ?4 AND version = ?5 AND alive = 1",
            params![content.into(), new_version, Utc::now().to_rfc3339(), key, entry.version],
        )?;
        if rows == 0 {
            return Err(MemoryError::Stale { expected: entry.version, found: new_version });
        }
        Ok(new_version)
    }

    /// Soft-delete an entry you own.
    pub fn delete(&self, key: &str, agent: &str) -> Result<(), MemoryError> {
        self.check_owner(key, agent)?;
        let conn = self.lock();
        conn.execute(
            "UPDATE memories SET alive = 0, updated_at = ?1 WHERE key = ?2 AND alive = 1",
            params![Utc::now().to_rfc3339(), key],
        )?;
        Ok(())
    }

    /// Move an entry to a more authoritative level.
    ///
    /// L6 candidates need `reviewed = true` (a human verdict). Only L1
    /// entries may become L0.
    pub fn promote(&self, key: &str, target: Level, reviewed: bool) -> Result<(), MemoryError> {
        let entry = self.read_any(key)?.ok_or_else(|| MemoryError::NotFound(key.to_string()))?;
        if !entry.alive {
            return Err(MemoryError::Dangling(key.to_string()));
        }
        if entry.level == Level::L6 && !reviewed {
            return Err(MemoryError::PromotionBlocked {
                reason: "L6 candidates need a human review before promotion".into(),
            });
        }
        if target >= entry.level {
            return Err(MemoryError::PromotionBlocked {
                reason: format!(
                    "cannot promote from {} to {} (target must be more authoritative)",
                    entry.level.as_str(),
                    target.as_str()
                ),
            });
        }
        if target == Level::L0 && entry.level != Level::L1 {
            return Err(MemoryError::PromotionBlocked { reason: "only L1 entries can be promoted to L0".into() });
        }
        let conn = self.lock();
        conn.execute(
            "UPDATE memories SET level = ?1, updated_at = ?2 WHERE key = ?3",
            params![target.as_str(), Utc::now().to_rfc3339(), key],
        )?;
        Ok(())
    }

    /// Full-text search over live entries, best matches first.
    pub fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchHit>, MemoryError> {
        let fts_query = fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT m.key, bm25(memories_fts) AS rank, snippet(memories_fts, 1, '', '', '...', 12)
             FROM memories_fts JOIN memories m ON m.rowid = memories_fts.rowid
             WHERE memories_fts MATCH ?1 AND m.alive = 1
             ORDER BY rank LIMIT ?2",
        )?;
        let hits = stmt
            .query_map(params![fts_query, top_k as i64], |row| {
                Ok(SearchHit {
                    key: row.get(0)?,
                    score: -row.get::<_, f64>(1)?,
                    snippet: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(hits)
    }

    /// Live entries at a level, newest first.
    pub fn list_level(&self, level: Level) -> Result<Vec<MemoryEntry>, MemoryError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "{SELECT_ENTRY} WHERE level = ?1 AND alive = 1 ORDER BY updated_at DESC"
        ))?;
        let entries = stmt
            .query_map(params![level.as_str()], row_to_entry)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    /// Soft-delete every live entry in a scope (task cleanup at goal end).
    pub fn cleanup_scope(&self, scope: Scope) -> Result<usize, MemoryError> {
        let conn = self.lock();
        Ok(conn.execute(
            "UPDATE memories SET alive = 0, updated_at = ?1 WHERE scope = ?2 AND alive = 1",
            params![Utc::now().to_rfc3339(), scope.as_str()],
        )?)
    }

    /// Remove soft-deleted entries outside the protected levels (L0, L1).
    pub fn gc(&self) -> Result<GcReport, MemoryError> {
        let conn = self.lock();
        let removed = conn.execute("DELETE FROM memories WHERE alive = 0 AND level NOT IN ('L0', 'L1')", [])?;
        Ok(GcReport { entries_removed: removed })
    }

    /// Number of live entries.
    pub fn count(&self) -> Result<usize, MemoryError> {
        let conn = self.lock();
        Ok(conn.query_row("SELECT COUNT(*) FROM memories WHERE alive = 1", [], |r| r.get(0))?)
    }
}

/// Turn free text into an FTS5 query: each word becomes a quoted prefix term,
/// joined with OR, so punctuation and FTS operators in user input are inert.
fn fts_query(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> MemoryDatabase {
        MemoryDatabase::open_in_memory().unwrap()
    }

    #[test]
    fn write_read_roundtrip() {
        let db = db();
        db.write("key1", Scope::Project, Level::L1, "main", "hello").unwrap();
        let entry = db.read("key1").unwrap().unwrap();
        assert_eq!(entry.content, "hello");
        assert_eq!(entry.owner, "main");
        assert_eq!(db.count().unwrap(), 1);
    }

    #[test]
    fn read_nonexistent_returns_none() {
        assert!(db().read("nope").unwrap().is_none());
    }

    #[test]
    fn read_deleted_returns_dangling() {
        let db = db();
        db.write("del", Scope::Task, Level::L2, "main", "x").unwrap();
        db.delete("del", "main").unwrap();
        assert!(matches!(db.read("del"), Err(MemoryError::Dangling(_))));
    }

    #[test]
    fn update_by_owner_bumps_version() {
        let db = db();
        db.write("upd", Scope::Project, Level::L1, "main", "v1").unwrap();
        assert_eq!(db.update("upd", "v2", "main").unwrap(), 2);
        assert_eq!(db.read("upd").unwrap().unwrap().content, "v2");
    }

    #[test]
    fn non_owner_cannot_update_or_delete() {
        let db = db();
        db.write("owned", Scope::Project, Level::L1, "main", "x").unwrap();
        assert!(matches!(db.update("owned", "hack", "attacker"), Err(MemoryError::PermissionDenied { .. })));
        assert!(matches!(db.delete("owned", "attacker"), Err(MemoryError::PermissionDenied { .. })));
        assert_eq!(db.owner_of("owned").unwrap().as_deref(), Some("main"));
    }

    #[test]
    fn promotion_rules() {
        let db = db();
        db.write("task_knowledge", Scope::Task, Level::L2, "main", "insight").unwrap();
        db.promote("task_knowledge", Level::L1, false).unwrap();
        assert_eq!(db.read("task_knowledge").unwrap().unwrap().level, Level::L1);

        db.write("candidate", Scope::Project, Level::L6, "main", "pattern").unwrap();
        assert!(matches!(db.promote("candidate", Level::L1, false), Err(MemoryError::PromotionBlocked { .. })));
        db.promote("candidate", Level::L1, true).unwrap();

        assert!(matches!(db.promote("candidate", Level::L2, true), Err(MemoryError::PromotionBlocked { .. })));
        db.write("l2", Scope::Task, Level::L2, "main", "x").unwrap();
        assert!(matches!(db.promote("l2", Level::L0, true), Err(MemoryError::PromotionBlocked { .. })));
    }

    #[test]
    fn duplicate_key_error() {
        let db = db();
        db.write("dup", Scope::Task, Level::L2, "main", "a").unwrap();
        assert!(matches!(db.write("dup", Scope::Task, Level::L2, "main", "b"), Err(MemoryError::DuplicateKey(_))));
    }

    #[test]
    fn gc_removes_dead_but_keeps_protected_levels() {
        let db = db();
        db.write("garbage", Scope::Task, Level::L5, "main", "x").unwrap();
        db.delete("garbage", "main").unwrap();
        db.write("law", Scope::Global, Level::L1, "user", "rule").unwrap();
        db.delete("law", "user").unwrap();
        assert_eq!(db.gc().unwrap().entries_removed, 1);
    }

    #[test]
    fn cleanup_scope_removes_task_entries() {
        let db = db();
        db.write("task1", Scope::Task, Level::L2, "main", "a").unwrap();
        db.write("task2", Scope::Task, Level::L2, "main", "b").unwrap();
        db.write("proj1", Scope::Project, Level::L1, "main", "c").unwrap();
        assert_eq!(db.cleanup_scope(Scope::Task).unwrap(), 2);
        assert!(db.read("proj1").unwrap().is_some());
    }

    #[test]
    fn search_finds_by_content_and_ranks() {
        let db = db();
        db.write("auth", Scope::Project, Level::L1, "main", "The authentication module uses JWT tokens").unwrap();
        db.write("db", Scope::Project, Level::L1, "main", "SQLite is the database; WAL mode is on").unwrap();
        db.write("auth2", Scope::Project, Level::L1, "main", "authentication retries are capped at three").unwrap();

        let hits = db.search("authentication JWT", 5).unwrap();
        assert_eq!(hits[0].key, "auth", "{hits:?}");
        assert!(hits.iter().all(|h| h.key != "db"));
        assert!(hits[0].snippet.contains("JWT"));
    }

    #[test]
    fn search_ignores_dead_entries_and_odd_input() {
        let db = db();
        db.write("gone", Scope::Task, Level::L2, "main", "ephemeral note").unwrap();
        db.delete("gone", "main").unwrap();
        assert!(db.search("ephemeral", 5).unwrap().is_empty());
        assert!(db.search("\"unbalanced (quote OR", 5).unwrap().is_empty());
        assert!(db.search("   ", 5).unwrap().is_empty());
    }

    #[test]
    fn search_survives_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memory.db");
        MemoryDatabase::open(&path).unwrap()
            .write("persist", Scope::Project, Level::L1, "main", "cargo test is the verifier").unwrap();
        let reopened = MemoryDatabase::open(&path).unwrap();
        assert_eq!(reopened.search("verifier", 3).unwrap()[0].key, "persist");
    }

    #[test]
    fn list_level() {
        let db = db();
        db.write("a", Scope::Project, Level::L1, "user", "x").unwrap();
        db.write("b", Scope::Task, Level::L2, "main", "y").unwrap();
        let l1 = db.list_level(Level::L1).unwrap();
        assert_eq!(l1.len(), 1);
        assert_eq!(l1[0].key, "a");
    }
}
