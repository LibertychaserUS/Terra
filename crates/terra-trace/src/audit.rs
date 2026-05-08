use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use terra_types::command::{Command, CommandResult};

use crate::persistent::PersistentAuditLog;

/// A single entry in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub timestamp: DateTime<Utc>,
    pub command: Command,
    pub result: Option<CommandResult>,
    pub duration_us: Option<u64>,
}

/// Audit log with optional write-through SQLite persistence.
///
/// Thread-safe via Mutex. When a persistent backend is attached,
/// every write goes to both in-memory and SQLite. On startup with
/// a path, entries are restored from disk.
pub struct AuditLog {
    entries: Mutex<Vec<AuditEntry>>,
    next_seq: Mutex<u64>,
    persistent: Option<Mutex<PersistentAuditLog>>,
}

impl AuditLog {
    /// Create an in-memory-only audit log (no persistence).
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            next_seq: Mutex::new(1),
            persistent: None,
        }
    }

    /// Create an audit log with SQLite write-through persistence.
    /// Restores existing entries from disk on open.
    pub fn with_persistence(path: &Path) -> anyhow::Result<Self> {
        let persistent = PersistentAuditLog::open(path)?;
        let existing = persistent.entries()?;
        let next_seq = existing.last().map(|e| e.seq + 1).unwrap_or(1);

        Ok(Self {
            entries: Mutex::new(existing),
            next_seq: Mutex::new(next_seq),
            persistent: Some(Mutex::new(persistent)),
        })
    }

    /// Record a command before execution.
    pub fn record_command(&self, command: Command) -> u64 {
        let mut seq = self.next_seq.lock().unwrap();
        let current_seq = *seq;
        *seq += 1;

        // Write-through to SQLite if persistent backend attached
        if let Some(ref persistent) = self.persistent {
            if let Ok(p) = persistent.lock() {
                let _ = p.record_command(&command);
            }
        }

        let entry = AuditEntry {
            seq: current_seq,
            timestamp: Utc::now(),
            command,
            result: None,
            duration_us: None,
        };

        self.entries.lock().unwrap().push(entry);
        current_seq
    }

    /// Record the result of a command after execution.
    pub fn record_result(&self, seq: u64, result: CommandResult, duration_us: u64) {
        // Write-through to SQLite
        if let Some(ref persistent) = self.persistent {
            if let Ok(p) = persistent.lock() {
                let _ = p.record_result(seq, &result, duration_us);
            }
        }

        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.iter_mut().find(|e| e.seq == seq) {
            entry.result = Some(result);
            entry.duration_us = Some(duration_us);
        }
    }

    /// Get all entries (for replay or inspection).
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.lock().unwrap().clone()
    }

    /// Get the number of recorded entries.
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether this log has a persistent backend attached.
    pub fn is_persistent(&self) -> bool {
        self.persistent.is_some()
    }

    /// Render the audit log to Markdown for human review.
    pub fn render_markdown(&self) -> String {
        let entries = self.entries.lock().unwrap();
        let mut lines = vec!["# Terra Audit Log".to_string(), String::new()];

        for entry in entries.iter() {
            lines.push(format!("## #{} — {}", entry.seq, entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC")));
            lines.push(format!("- **Agent**: {}", entry.command.agent));
            lines.push(terra_protocol::render::render_command(&entry.command));
            if let Some(ref result) = entry.result {
                lines.push(format!("- **Status**: {:?}", result.status));
                if let Some(dur) = entry.duration_us {
                    lines.push(format!("- **Duration**: {}μs", dur));
                }
            }
            lines.push(String::new());
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_types::command::{CommandOp, CommandResult};

    #[test]
    fn record_and_retrieve() {
        let log = AuditLog::new();

        let cmd = Command::new("main", CommandOp::MemRead { key: "test".into() });
        let seq = log.record_command(cmd);
        assert_eq!(seq, 1);
        assert_eq!(log.len(), 1);

        log.record_result(seq, CommandResult::ok("cmd-1", None), 150);

        let entries = log.entries();
        assert_eq!(entries[0].seq, 1);
        assert!(entries[0].result.is_some());
        assert_eq!(entries[0].duration_us, Some(150));
    }

    #[test]
    fn sequential_numbering() {
        let log = AuditLog::new();

        let cmd1 = Command::new("main", CommandOp::MemRead { key: "a".into() });
        let cmd2 = Command::new("worker", CommandOp::MemRead { key: "b".into() });

        let seq1 = log.record_command(cmd1);
        let seq2 = log.record_command(cmd2);

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn render_to_markdown() {
        let log = AuditLog::new();
        let cmd = Command::new(
            "main",
            CommandOp::TraceCheckpoint {
                title: "MVP1 complete".into(),
                summary: "End-to-end path works".into(),
            },
        );
        log.record_command(cmd);

        let md = log.render_markdown();
        assert!(md.contains("Terra Audit Log"));
        assert!(md.contains("MVP1 complete"));
    }

    // T-AT-006: empty log renders valid markdown
    #[test]
    fn empty_log_renders() {
        let log = AuditLog::new();
        let md = log.render_markdown();
        assert!(md.contains("Terra Audit Log"));
    }

    // T-AT-007: result for unknown seq doesn't panic
    #[test]
    fn unknown_seq_ignored() {
        let log = AuditLog::new();
        log.record_result(999, CommandResult::ok("x", None), 0);
        assert_eq!(log.len(), 0);
    }

    // T-AT-008: concurrent writes from 10 threads
    #[test]
    fn concurrent_recording_safe() {
        use std::sync::Arc;
        use std::thread;

        let log = Arc::new(AuditLog::new());
        let mut handles = Vec::new();

        for thread_id in 0..10 {
            let log = Arc::clone(&log);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    let cmd = Command::new(
                        &format!("agent-{}", thread_id),
                        CommandOp::MemRead {
                            key: format!("key-{}-{}", thread_id, i),
                        },
                    );
                    log.record_command(cmd);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // 10 threads × 100 commands = 1000 entries
        assert_eq!(log.len(), 1000);

        // Verify no gaps in sequence numbers
        let entries = log.entries();
        let mut seqs: Vec<u64> = entries.iter().map(|e| e.seq).collect();
        seqs.sort();
        for (i, seq) in seqs.iter().enumerate() {
            assert_eq!(*seq, (i + 1) as u64, "gap at position {}", i);
        }
    }

    // T-AT-009: large log performance
    #[test]
    fn large_log_performance() {
        let log = AuditLog::new();
        let start = std::time::Instant::now();

        for i in 0..10_000 {
            let cmd = Command::new(
                "main",
                CommandOp::MemRead { key: format!("key-{}", i) },
            );
            log.record_command(cmd);
        }

        let elapsed = start.elapsed();
        assert_eq!(log.len(), 10_000);
        // Should complete well under 1 second
        assert!(
            elapsed.as_secs() < 2,
            "10K entries took {:?}, expected < 2s",
            elapsed
        );
    }

    // T-FIX-003: audit persists to disk and survives reopen
    #[test]
    fn audit_persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("audit.db");

        // Write entries with persistence
        {
            let log = AuditLog::with_persistence(&db_path).unwrap();
            assert!(log.is_persistent());

            let cmd = Command::new("main", CommandOp::MemRead { key: "persist_test".into() });
            let seq = log.record_command(cmd);
            log.record_result(seq, CommandResult::ok("cmd-1", None), 100);

            let cmd2 = Command::new("worker", CommandOp::MemRead { key: "key2".into() });
            log.record_command(cmd2);

            assert_eq!(log.len(), 2);
        }
        // log dropped here — in-memory gone

        // Reopen from disk — entries should be restored
        {
            let log = AuditLog::with_persistence(&db_path).unwrap();
            assert_eq!(log.len(), 2, "entries should survive reopen");

            let entries = log.entries();
            assert_eq!(entries[0].command.agent, "main");
            assert_eq!(entries[1].command.agent, "worker");
            assert!(entries[0].result.is_some(), "result should persist");
            assert_eq!(entries[0].duration_us, Some(100));

            // New writes continue with correct seq
            let cmd3 = Command::new("explorer", CommandOp::MemRead { key: "key3".into() });
            let seq3 = log.record_command(cmd3);
            assert_eq!(seq3, 3);
        }
    }

    #[test]
    fn in_memory_not_persistent() {
        let log = AuditLog::new();
        assert!(!log.is_persistent());
    }
}
