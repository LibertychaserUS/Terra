use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::error::MemoryError;
use crate::types::{Level, MemoryEntry, Scope};

/// SQLite storage layer — pure data access, no business logic.
///
/// Business rules (ownership, lifetime, promotion gates) are
/// enforced by the `safety` module, not here.
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Open or create a database at the given path.
    pub fn open(path: &Path) -> Result<Self, MemoryError> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.initialize()?;
        Ok(store)
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, MemoryError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<(), MemoryError> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA user_version = 1;

             CREATE TABLE IF NOT EXISTS memories (
                 id          TEXT PRIMARY KEY,
                 key         TEXT NOT NULL UNIQUE,
                 scope       TEXT NOT NULL CHECK (scope IN ('global','project','session','task')),
                 level       TEXT NOT NULL CHECK (level IN ('L0','L1','L2','L3','L4','L5','L6')),
                 owner       TEXT NOT NULL,
                 content     BLOB NOT NULL,
                 version     INTEGER NOT NULL DEFAULT 1,
                 ref_count   INTEGER NOT NULL DEFAULT 0,
                 alive       INTEGER NOT NULL DEFAULT 1,
                 created_at  TEXT NOT NULL,
                 updated_at  TEXT NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope);
             CREATE INDEX IF NOT EXISTS idx_memories_level ON memories(level);
             CREATE INDEX IF NOT EXISTS idx_memories_alive ON memories(alive);",
        )?;
        Ok(())
    }

    /// Insert a new memory entry.
    pub fn insert(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        self.conn
            .execute(
                "INSERT INTO memories (id, key, scope, level, owner, content, version, ref_count, alive, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    entry.id,
                    entry.key,
                    entry.scope.as_str(),
                    entry.level.as_str(),
                    entry.owner,
                    entry.content,
                    entry.version,
                    entry.ref_count,
                    entry.alive,
                    entry.created_at.to_rfc3339(),
                    entry.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| {
                if let rusqlite::Error::SqliteFailure(ref err, _) = e {
                    if err.code == rusqlite::ErrorCode::ConstraintViolation {
                        return MemoryError::DuplicateKey(entry.key.clone());
                    }
                }
                MemoryError::Database(e)
            })?;
        Ok(())
    }

    /// Read a memory entry by key.
    pub fn read(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let result = self
            .conn
            .query_row(
                "SELECT id, key, scope, level, owner, content, version, ref_count, alive, created_at, updated_at
                 FROM memories WHERE key = ?1",
                params![key],
                |row| {
                    Ok(MemoryEntry {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        scope: Scope::from_str(&row.get::<_, String>(2)?).unwrap_or(Scope::Task),
                        level: Level::from_str(&row.get::<_, String>(3)?).unwrap_or(Level::L2),
                        owner: row.get(4)?,
                        content: row.get(5)?,
                        version: row.get(6)?,
                        ref_count: row.get(7)?,
                        alive: row.get(8)?,
                        created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(9)?)
                            .unwrap_or_default()
                            .with_timezone(&Utc),
                        updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(10)?)
                            .unwrap_or_default()
                            .with_timezone(&Utc),
                    })
                },
            )
            .optional()?;
        Ok(result)
    }

    /// Update content and increment version. Returns new version.
    pub fn update_content(
        &self,
        key: &str,
        content: &[u8],
        expected_version: u64,
    ) -> Result<u64, MemoryError> {
        let now = Utc::now().to_rfc3339();
        let new_version = expected_version + 1;
        let rows = self.conn.execute(
            "UPDATE memories SET content = ?1, version = ?2, updated_at = ?3
             WHERE key = ?4 AND version = ?5 AND alive = 1",
            params![content, new_version, now, key, expected_version],
        )?;
        if rows == 0 {
            // Either key doesn't exist, or version mismatch
            if let Some(entry) = self.read(key)? {
                if !entry.alive {
                    return Err(MemoryError::Dangling(key.to_string()));
                }
                return Err(MemoryError::Stale {
                    expected: expected_version,
                    found: entry.version,
                });
            }
            return Err(MemoryError::NotFound(key.to_string()));
        }
        Ok(new_version)
    }

    /// Soft-delete: set alive=false.
    pub fn soft_delete(&self, key: &str) -> Result<(), MemoryError> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE memories SET alive = 0, updated_at = ?1 WHERE key = ?2 AND alive = 1",
            params![now, key],
        )?;
        if rows == 0 {
            return Err(MemoryError::NotFound(key.to_string()));
        }
        Ok(())
    }

    /// Update the level of a memory entry (promotion).
    pub fn update_level(&self, key: &str, new_level: &Level) -> Result<(), MemoryError> {
        let now = Utc::now().to_rfc3339();
        let rows = self.conn.execute(
            "UPDATE memories SET level = ?1, updated_at = ?2 WHERE key = ?3 AND alive = 1",
            params![new_level.as_str(), now, key],
        )?;
        if rows == 0 {
            return Err(MemoryError::NotFound(key.to_string()));
        }
        Ok(())
    }

    /// Get the owner of a memory entry.
    pub fn get_owner(&self, key: &str) -> Result<Option<String>, MemoryError> {
        let result = self
            .conn
            .query_row(
                "SELECT owner FROM memories WHERE key = ?1 AND alive = 1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(result)
    }

    /// Remove entries with ref_count=0 and non-protected level.
    /// Returns number of entries removed.
    pub fn gc(&self) -> Result<usize, MemoryError> {
        let rows = self.conn.execute(
            "DELETE FROM memories
             WHERE alive = 0 AND ref_count = 0
             AND level NOT IN ('L0', 'L1')",
            [],
        )?;
        Ok(rows)
    }

    /// Count all alive entries.
    pub fn count_alive(&self) -> Result<usize, MemoryError> {
        let count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE alive = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }
}

/// Helper to create a new MemoryEntry.
pub fn new_entry(
    key: impl Into<String>,
    scope: Scope,
    level: Level,
    owner: impl Into<String>,
    content: Vec<u8>,
) -> MemoryEntry {
    let now = Utc::now();
    MemoryEntry {
        id: Uuid::new_v4().to_string(),
        key: key.into(),
        scope,
        level,
        owner: owner.into(),
        content,
        version: 1,
        ref_count: 0,
        alive: true,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SqliteStore {
        SqliteStore::open_in_memory().unwrap()
    }

    fn test_entry(key: &str) -> MemoryEntry {
        new_entry(key, Scope::Project, Level::L1, "main", b"test content".to_vec())
    }

    // T-M-001
    #[test]
    fn create_database_and_tables() {
        let store = test_store();
        assert_eq!(store.count_alive().unwrap(), 0);
    }

    // T-M-002
    #[test]
    fn write_and_read() {
        let store = test_store();
        let entry = test_entry("auth_pattern");
        store.insert(&entry).unwrap();

        let read = store.read("auth_pattern").unwrap().unwrap();
        assert_eq!(read.key, "auth_pattern");
        assert_eq!(read.content, b"test content");
        assert_eq!(read.scope, Scope::Project);
        assert_eq!(read.level, Level::L1);
        assert_eq!(read.owner, "main");
        assert_eq!(read.version, 1);
        assert!(read.alive);
    }

    // T-M-003
    #[test]
    fn write_duplicate_key() {
        let store = test_store();
        store.insert(&test_entry("dup")).unwrap();
        let result = store.insert(&test_entry("dup"));
        assert!(matches!(result, Err(MemoryError::DuplicateKey(_))));
    }

    // T-M-004
    #[test]
    fn read_missing_key() {
        let store = test_store();
        let result = store.read("nonexistent").unwrap();
        assert!(result.is_none());
    }

    // T-M-005
    #[test]
    fn update_increments_version() {
        let store = test_store();
        store.insert(&test_entry("versioned")).unwrap();
        let new_ver = store.update_content("versioned", b"updated", 1).unwrap();
        assert_eq!(new_ver, 2);

        let read = store.read("versioned").unwrap().unwrap();
        assert_eq!(read.version, 2);
        assert_eq!(read.content, b"updated");
    }

    // T-M-006
    #[test]
    fn delete_sets_alive_false() {
        let store = test_store();
        store.insert(&test_entry("deleteme")).unwrap();
        store.soft_delete("deleteme").unwrap();

        let read = store.read("deleteme").unwrap().unwrap();
        assert!(!read.alive);
    }

    // T-M-007
    #[test]
    fn content_stored_as_msgpack() {
        let store = test_store();
        let data = serde_json::json!({"key": "value", "num": 42});
        let packed = rmp_serde::to_vec_named(&data).unwrap();
        let entry = new_entry("packed", Scope::Project, Level::L2, "main", packed.clone());
        store.insert(&entry).unwrap();

        let read = store.read("packed").unwrap().unwrap();
        let decoded: serde_json::Value = rmp_serde::from_slice(&read.content).unwrap();
        assert_eq!(decoded["num"], 42);
    }

    // T-M-008 (scope constraint tested via SQL CHECK)
    #[test]
    fn scope_constraint() {
        let store = test_store();
        let result = store.conn.execute(
            "INSERT INTO memories VALUES ('id','key','INVALID','L0','owner',X'00',1,0,1,'now','now')",
            [],
        );
        assert!(result.is_err());
    }

    // T-M-009
    #[test]
    fn level_constraint() {
        let store = test_store();
        let result = store.conn.execute(
            "INSERT INTO memories VALUES ('id','key','project','INVALID','owner',X'00',1,0,1,'now','now')",
            [],
        );
        assert!(result.is_err());
    }

    // T-M-010
    #[test]
    fn owner_set() {
        let store = test_store();
        let entry = new_entry("owned", Scope::Task, Level::L2, "worker-1", b"data".to_vec());
        store.insert(&entry).unwrap();
        assert_eq!(store.get_owner("owned").unwrap(), Some("worker-1".into()));
    }

    // T-M-015
    #[test]
    fn dangling_pointer_on_update() {
        let store = test_store();
        store.insert(&test_entry("deleted")).unwrap();
        store.soft_delete("deleted").unwrap();
        let result = store.update_content("deleted", b"new", 1);
        assert!(matches!(result, Err(MemoryError::Dangling(_))));
    }

    // T-M-016
    #[test]
    fn version_mismatch() {
        let store = test_store();
        store.insert(&test_entry("stale")).unwrap();
        store.update_content("stale", b"v2", 1).unwrap();
        let result = store.update_content("stale", b"v3", 1); // expected=1 but current=2
        assert!(matches!(result, Err(MemoryError::Stale { expected: 1, found: 2 })));
    }

    // T-M-017
    #[test]
    fn gc_removes_dead_weak() {
        let store = test_store();
        let entry = new_entry("gc_target", Scope::Task, Level::L5, "main", b"x".to_vec());
        store.insert(&entry).unwrap();
        store.soft_delete("gc_target").unwrap();
        let removed = store.gc().unwrap();
        assert_eq!(removed, 1);
    }

    // T-M-018
    #[test]
    fn gc_preserves_authoritative() {
        let store = test_store();
        let entry = new_entry("constitution", Scope::Global, Level::L0, "system", b"law".to_vec());
        store.insert(&entry).unwrap();
        store.soft_delete("constitution").unwrap();
        let removed = store.gc().unwrap();
        assert_eq!(removed, 0); // L0 is gc-protected
    }

    // T-M-023
    #[test]
    fn promote_level() {
        let store = test_store();
        store.insert(&new_entry("promoted", Scope::Task, Level::L2, "main", b"x".to_vec())).unwrap();
        store.update_level("promoted", &Level::L1).unwrap();
        let read = store.read("promoted").unwrap().unwrap();
        assert_eq!(read.level, Level::L1);
    }
}
