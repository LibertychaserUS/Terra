use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use terra_protocol::command::{Command, CommandResult};

/// A single entry in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub timestamp: DateTime<Utc>,
    pub command: Command,
    pub result: Option<CommandResult>,
    pub duration_us: Option<u64>,
}

/// In-memory audit log. All commands pass through here.
///
/// Thread-safe via Mutex. In production, this would also
/// flush to disk (MessagePack files in .terra/audit/).
pub struct AuditLog {
    entries: Mutex<Vec<AuditEntry>>,
    next_seq: Mutex<u64>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            next_seq: Mutex::new(1),
        }
    }

    /// Record a command before execution.
    pub fn record_command(&self, command: Command) -> u64 {
        let mut seq = self.next_seq.lock().unwrap();
        let current_seq = *seq;
        *seq += 1;

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
    use terra_protocol::command::{CommandOp, CommandResult};

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
}
