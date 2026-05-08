use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection};

use crate::audit::AuditEntry;
use terra_protocol::codec;
use terra_types::command::{Command, CommandResult};

/// SQLite-backed persistent audit log.
///
/// Provides the same interface as AuditLog but persists entries
/// to disk. Used for production; AuditLog (in-memory) for tests.
pub struct PersistentAuditLog {
    conn: Connection,
}

impl PersistentAuditLog {
    /// Open or create a persistent audit log.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        let log = Self { conn };
        log.initialize()?;
        Ok(log)
    }

    /// Open an in-memory instance (for testing).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let log = Self { conn };
        log.initialize()?;
        Ok(log)
    }

    fn initialize(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;

             CREATE TABLE IF NOT EXISTS audit_log (
                 seq         INTEGER PRIMARY KEY AUTOINCREMENT,
                 timestamp   TEXT NOT NULL,
                 agent       TEXT NOT NULL,
                 command     BLOB NOT NULL,
                 result      BLOB,
                 duration_us INTEGER
             );",
        )?;
        Ok(())
    }

    /// Record a command. Returns the sequence number.
    pub fn record_command(&self, command: &Command) -> anyhow::Result<u64> {
        let cmd_bytes = codec::encode(command)?;
        self.conn.execute(
            "INSERT INTO audit_log (timestamp, agent, command) VALUES (?1, ?2, ?3)",
            params![
                Utc::now().to_rfc3339(),
                command.agent,
                cmd_bytes,
            ],
        )?;
        Ok(self.conn.last_insert_rowid() as u64)
    }

    /// Record the result of a command.
    pub fn record_result(
        &self,
        seq: u64,
        result: &CommandResult,
        duration_us: u64,
    ) -> anyhow::Result<()> {
        let result_bytes = codec::encode(result)?;
        self.conn.execute(
            "UPDATE audit_log SET result = ?1, duration_us = ?2 WHERE seq = ?3",
            params![result_bytes, duration_us, seq],
        )?;
        Ok(())
    }

    /// Get all entries.
    pub fn entries(&self) -> anyhow::Result<Vec<AuditEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, timestamp, agent, command, result, duration_us FROM audit_log ORDER BY seq",
        )?;

        let entries = stmt.query_map([], |row| {
            let cmd_bytes: Vec<u8> = row.get(3)?;
            let result_bytes: Option<Vec<u8>> = row.get(4)?;
            let timestamp_str: String = row.get(1)?;

            Ok((
                row.get::<_, u64>(0)?,
                timestamp_str,
                cmd_bytes,
                result_bytes,
                row.get::<_, Option<u64>>(5)?,
            ))
        })?;

        let mut result = Vec::new();
        for entry in entries {
            let (seq, ts, cmd_bytes, result_bytes, duration_us) = entry?;
            let command: Command = codec::decode(&cmd_bytes)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
            let cmd_result: Option<CommandResult> = result_bytes
                .map(|b| codec::decode(&b))
                .transpose()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
            let timestamp = chrono::DateTime::parse_from_rfc3339(&ts)
                .unwrap_or_default()
                .with_timezone(&Utc);

            result.push(AuditEntry {
                seq,
                timestamp,
                command,
                result: cmd_result,
                duration_us,
            });
        }
        Ok(result)
    }

    /// Get entry count.
    pub fn len(&self) -> anyhow::Result<usize> {
        let count: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_log",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn is_empty(&self) -> anyhow::Result<bool> {
        Ok(self.len()? == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_types::command::{CommandOp, CommandResult as CmdResult};

    fn test_log() -> PersistentAuditLog {
        PersistentAuditLog::open_in_memory().unwrap()
    }

    #[test]
    fn persistent_record_and_retrieve() {
        let log = test_log();
        let cmd = Command::new("main", CommandOp::MemRead { key: "test".into() });
        let seq = log.record_command(&cmd).unwrap();
        assert_eq!(seq, 1);

        log.record_result(seq, &CmdResult::ok("cmd-1", None), 200).unwrap();

        let entries = log.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].seq, 1);
        assert!(entries[0].result.is_some());
        assert_eq!(entries[0].duration_us, Some(200));
    }

    #[test]
    fn persistent_sequential_numbering() {
        let log = test_log();
        let cmd1 = Command::new("main", CommandOp::MemRead { key: "a".into() });
        let cmd2 = Command::new("worker", CommandOp::MemRead { key: "b".into() });

        let seq1 = log.record_command(&cmd1).unwrap();
        let seq2 = log.record_command(&cmd2).unwrap();

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(log.len().unwrap(), 2);
    }

    #[test]
    fn persistent_entries_ordered() {
        let log = test_log();
        for i in 0..5 {
            let cmd = Command::new("main", CommandOp::MemRead { key: format!("key-{}", i) });
            log.record_command(&cmd).unwrap();
        }
        let entries = log.entries().unwrap();
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.seq, (i + 1) as u64);
        }
    }

    #[test]
    fn persistent_msgpack_roundtrip() {
        let log = test_log();
        let cmd = Command::new(
            "main",
            CommandOp::TraceCheckpoint {
                title: "test checkpoint".into(),
                summary: "testing persistence".into(),
            },
        );
        log.record_command(&cmd).unwrap();

        let entries = log.entries().unwrap();
        assert_eq!(entries[0].command.agent, "main");
    }

    #[test]
    fn persistent_empty_log() {
        let log = test_log();
        assert!(log.is_empty().unwrap());
        assert_eq!(log.entries().unwrap().len(), 0);
    }
}
