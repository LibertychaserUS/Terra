use std::path::Path;

use rusqlite::{params, Connection};

use crate::audit::{AuditEvent, EventKind};

/// SQLite storage for the audit log: one insert per event, no updates.
pub struct PersistentAuditLog {
    conn: Connection,
}

impl PersistentAuditLog {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS audit_events (
                 id         INTEGER PRIMARY KEY,
                 seq        INTEGER NOT NULL,
                 kind       TEXT NOT NULL,
                 timestamp  TEXT NOT NULL,
                 payload    TEXT NOT NULL,
                 prev_hash  TEXT NOT NULL,
                 hash       TEXT NOT NULL
             );",
        )?;
        Ok(Self { conn })
    }

    /// Append one event. The caller has already chained its hash.
    pub fn append(&self, event: &AuditEvent) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO audit_events (id, seq, kind, timestamp, payload, prev_hash, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.id,
                event.seq,
                event.kind.as_str(),
                event.timestamp.to_rfc3339(),
                event.payload.to_string(),
                event.prev_hash,
                event.hash,
            ],
        )?;
        Ok(())
    }

    /// Every event, in insertion order.
    pub fn events(&self) -> anyhow::Result<Vec<AuditEvent>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, seq, kind, timestamp, payload, prev_hash, hash FROM audit_events ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (id, seq, kind, timestamp, payload, prev_hash, hash) = row?;
            let kind = EventKind::parse(&kind).ok_or_else(|| anyhow::anyhow!("unknown event kind {kind}"))?;
            let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp)?.with_timezone(&chrono::Utc);
            let payload = serde_json::from_str(&payload)?;
            events.push(AuditEvent { id, seq, kind, timestamp, payload, prev_hash, hash });
        }
        Ok(events)
    }
}
