//! The audit log: an append-only, hash-chained record of every command the
//! executor saw and every result it produced.
//!
//! Each command and each result is one event. An event's hash covers its
//! own fields and the hash of the event before it, so a row that is edited
//! or removed breaks the chain from that point on (`verify_chain`). The log
//! never updates or deletes a row; the `AuditEntry` view joins a command
//! event with its result event.

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use terra_types::command::{Command, CommandOp, CommandResult};

use crate::persistent::PersistentAuditLog;

/// A command and (once known) its result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub timestamp: DateTime<Utc>,
    pub command: Command,
    pub result: Option<CommandResult>,
    pub duration_us: Option<u64>,
}

impl AuditEntry {
    /// One-line, human-readable summary used by `terra trace`.
    pub fn summary_line(&self) -> String {
        let status = match &self.result {
            Some(r) => format!("{:?}", r.status),
            None => "pending".to_string(),
        };
        let duration = match self.duration_us {
            Some(us) if us >= 1_000 => format!("{:.1}ms", us as f64 / 1_000.0),
            Some(us) => format!("{}µs", us),
            None => "-".to_string(),
        };
        format!(
            "#{:<5} {}  {:<8} {:<60} {:<16} {}",
            self.seq,
            self.timestamp.format("%Y-%m-%d %H:%M:%S"),
            self.command.agent,
            op_summary(&self.command.op),
            status,
            duration,
        )
    }
}

/// Short description of an operation: its name plus the most telling argument.
pub fn op_summary(op: &CommandOp) -> String {
    use terra_types::text::truncate_ellipsis;
    let detail = match op {
        CommandOp::MemWrite { key, level, .. } => format!("{} ({})", key, level),
        CommandOp::MemRead { key } => key.clone(),
        CommandOp::MemSearch { query, top_k } => format!("{:?} top {}", query, top_k),
        CommandOp::TraceEvent { event_type, .. } => event_type.clone(),
        CommandOp::ProviderCall { user_prompt, .. } => format!("{:?}", user_prompt),
        CommandOp::FileRead { path } => path.clone(),
        CommandOp::FileWrite { path, content } => format!("{} ({} bytes)", path, content.len()),
        CommandOp::FileDelete { path } => path.clone(),
        CommandOp::ShellExec { command, .. } => format!("{:?}", command),
        CommandOp::ExecuteTask { intent, open_questions, .. } => {
            format!("{:?} ({} open questions)", intent, open_questions.len())
        }
        CommandOp::ClaimComplete { evidence, .. } => evidence.clone(),
    };
    truncate_ellipsis(&format!("{} {}", op.name(), detail.replace('\n', " ")), 60)
}

/// One row of the append-only log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEvent {
    /// Position in the log, starting at 1.
    pub id: u64,
    /// The command this event belongs to.
    pub seq: u64,
    pub kind: EventKind,
    pub timestamp: DateTime<Utc>,
    /// The command or the result, as JSON.
    pub payload: serde_json::Value,
    pub prev_hash: String,
    pub hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Command,
    Result,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Command => "command",
            EventKind::Result => "result",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "command" => Some(EventKind::Command),
            "result" => Some(EventKind::Result),
            _ => None,
        }
    }
}

/// The hash of the empty chain.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// SHA-256 over the fields that make an event, plus the previous hash.
pub fn event_hash(prev_hash: &str, id: u64, seq: u64, kind: EventKind, timestamp: &str, payload: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [prev_hash, &id.to_string(), &seq.to_string(), kind.as_str(), timestamp, payload] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    format!("{:x}", hasher.finalize())
}

/// Outcome of walking the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainVerdict {
    /// Every hash matches; `events` rows were checked.
    Intact { events: usize, head: String },
    /// The first event whose hash does not match what its fields produce.
    Broken { at_id: u64, expected: String, found: String },
}

/// Recompute every hash; the first mismatch breaks the chain.
pub fn verify_events(events: &[AuditEvent]) -> ChainVerdict {
    let mut prev = GENESIS_HASH.to_string();
    for (i, event) in events.iter().enumerate() {
        let expected_id = i as u64 + 1;
        let payload = event.payload.to_string();
        let expected = event_hash(&prev, event.id, event.seq, event.kind, &event.timestamp.to_rfc3339(), &payload);
        if event.id != expected_id || event.prev_hash != prev || event.hash != expected {
            return ChainVerdict::Broken { at_id: event.id, expected, found: event.hash.clone() };
        }
        prev = event.hash.clone();
    }
    ChainVerdict::Intact { events: events.len(), head: prev }
}

struct State {
    events: Vec<AuditEvent>,
    entries: Vec<AuditEntry>,
    next_seq: u64,
    head: String,
}

/// Audit log with optional write-through SQLite persistence.
pub struct AuditLog {
    state: Mutex<State>,
    persistent: Option<Mutex<PersistentAuditLog>>,
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditLog {
    /// An in-memory log (tests, projects without `.terra/`).
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State { events: Vec::new(), entries: Vec::new(), next_seq: 1, head: GENESIS_HASH.into() }),
            persistent: None,
        }
    }

    /// A log persisted at `path`; existing events are loaded and the chain
    /// continues from their head. A broken chain on disk is an error.
    pub fn with_persistence(path: &Path) -> anyhow::Result<Self> {
        let persistent = PersistentAuditLog::open(path)?;
        let events = persistent.events()?;
        if let ChainVerdict::Broken { at_id, .. } = verify_events(&events) {
            anyhow::bail!("audit log {} is not intact: hash chain breaks at event {}", path.display(), at_id);
        }
        let mut state = State { events: Vec::new(), entries: Vec::new(), next_seq: 1, head: GENESIS_HASH.into() };
        for event in events {
            state.apply(event);
        }
        Ok(Self { state: Mutex::new(state), persistent: Some(Mutex::new(persistent)) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Append one event. `seq` is allocated here for commands so that
    /// entries are in sequence order even under concurrent recording.
    fn append(&self, seq: Option<u64>, kind: EventKind, payload: serde_json::Value) -> u64 {
        let mut state = self.lock();
        let seq = seq.unwrap_or_else(|| {
            let seq = state.next_seq;
            state.next_seq += 1;
            seq
        });
        let id = state.events.len() as u64 + 1;
        let timestamp = Utc::now();
        let hash = event_hash(&state.head, id, seq, kind, &timestamp.to_rfc3339(), &payload.to_string());
        let event = AuditEvent { id, seq, kind, timestamp, payload, prev_hash: state.head.clone(), hash };
        if let Some(persistent) = &self.persistent {
            if let Ok(p) = persistent.lock() {
                if let Err(e) = p.append(&event) {
                    eprintln!("[terra] audit write failed: {e}");
                }
            }
        }
        state.apply(event);
        seq
    }

    /// Record a command before it runs. Returns its sequence number.
    pub fn record_command(&self, command: Command) -> u64 {
        let payload = serde_json::to_value(&command).unwrap_or(serde_json::Value::Null);
        self.append(None, EventKind::Command, payload)
    }

    /// Record the result of command `seq`.
    pub fn record_result(&self, seq: u64, result: CommandResult, duration_us: u64) {
        let payload = serde_json::json!({ "result": result, "duration_us": duration_us });
        self.append(Some(seq), EventKind::Result, payload);
    }

    /// All entries, oldest first.
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.lock().entries.clone()
    }

    /// Entries recorded after sequence number `seq`, oldest first.
    pub fn entries_after(&self, seq: u64) -> Vec<AuditEntry> {
        let state = self.lock();
        let start = state.entries.partition_point(|e| e.seq <= seq);
        state.entries[start..].to_vec()
    }

    /// Sequence number of the latest command (0 when empty).
    pub fn last_seq(&self) -> u64 {
        self.lock().next_seq - 1
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Raw events, oldest first.
    pub fn events(&self) -> Vec<AuditEvent> {
        self.lock().events.clone()
    }

    /// Hash of the latest event: a fingerprint of the whole log so far.
    pub fn head_hash(&self) -> String {
        self.lock().head.clone()
    }

    /// Walk the in-memory chain.
    pub fn verify_chain(&self) -> ChainVerdict {
        verify_events(&self.lock().events)
    }

    pub fn is_persistent(&self) -> bool {
        self.persistent.is_some()
    }
}

impl State {
    fn apply(&mut self, event: AuditEvent) {
        match event.kind {
            EventKind::Command => {
                if let Ok(command) = serde_json::from_value::<Command>(event.payload.clone()) {
                    self.entries.push(AuditEntry {
                        seq: event.seq,
                        timestamp: event.timestamp,
                        command,
                        result: None,
                        duration_us: None,
                    });
                    self.next_seq = self.next_seq.max(event.seq + 1);
                }
            }
            EventKind::Result => {
                if let Some(entry) = self.entries.iter_mut().rev().find(|e| e.seq == event.seq) {
                    entry.result = serde_json::from_value(event.payload["result"].clone()).ok();
                    entry.duration_us = event.payload["duration_us"].as_u64();
                }
            }
        }
        self.head = event.hash.clone();
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_types::command::{CommandOp, CommandResult};

    fn read(agent: &str, key: &str) -> Command {
        Command::new(agent, CommandOp::MemRead { key: key.into() })
    }

    #[test]
    fn record_and_retrieve() {
        let log = AuditLog::new();
        let seq = log.record_command(read("main", "test"));
        assert_eq!(seq, 1);
        assert_eq!(log.len(), 1);
        log.record_result(seq, CommandResult::ok("cmd-1", None), 150);

        let entries = log.entries();
        assert_eq!(entries[0].seq, 1);
        assert!(entries[0].result.is_some());
        assert_eq!(entries[0].duration_us, Some(150));
        assert_eq!(log.events().len(), 2, "one command event and one result event");
    }

    #[test]
    fn sequential_numbering_and_entries_after() {
        let log = AuditLog::new();
        assert_eq!(log.record_command(read("main", "a")), 1);
        assert_eq!(log.record_command(read("worker", "b")), 2);
        assert_eq!(log.last_seq(), 2);
        let after = log.entries_after(1);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].command.agent, "worker");
    }

    #[test]
    fn unknown_seq_result_is_kept_as_event_but_no_entry() {
        let log = AuditLog::new();
        log.record_result(99, CommandResult::ok("x", None), 1);
        assert_eq!(log.len(), 0);
        assert_eq!(log.events().len(), 1);
    }

    #[test]
    fn chain_is_intact_and_detects_tampering() {
        let log = AuditLog::new();
        for i in 0..5 {
            let seq = log.record_command(read("main", &format!("k{i}")));
            log.record_result(seq, CommandResult::ok("", None), i);
        }
        assert!(matches!(log.verify_chain(), ChainVerdict::Intact { events: 10, .. }));
        assert_eq!(log.head_hash(), log.events().last().unwrap().hash);

        let mut events = log.events();
        events[3].payload["result"]["status"] = serde_json::json!("permission_denied");
        assert!(matches!(verify_events(&events), ChainVerdict::Broken { at_id: 4, .. }));

        let mut events = log.events();
        events.remove(2);
        assert!(matches!(verify_events(&events), ChainVerdict::Broken { at_id: 4, .. }));

        let mut events = log.events();
        events.pop();
        assert!(matches!(verify_events(&events), ChainVerdict::Intact { events: 9, .. }), "truncation at the tail is only visible against a head hash kept elsewhere");
    }

    #[test]
    fn concurrent_recording_safe() {
        use std::sync::Arc;
        let log = Arc::new(AuditLog::new());
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let log = log.clone();
                std::thread::spawn(move || {
                    for i in 0..100 {
                        let seq = log.record_command(read("main", &format!("{t}-{i}")));
                        log.record_result(seq, CommandResult::ok("", None), 1);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(log.len(), 800);
        let seqs: Vec<u64> = log.entries().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (1..=800).collect::<Vec<_>>(), "entries are in sequence order");
        assert!(matches!(log.verify_chain(), ChainVerdict::Intact { .. }));
    }

    #[test]
    fn audit_persists_to_disk_and_chain_continues() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        let head = {
            let log = AuditLog::with_persistence(&path).unwrap();
            assert!(log.is_persistent());
            let seq = log.record_command(read("main", "a"));
            log.record_result(seq, CommandResult::ok("", None), 100);
            log.record_command(read("worker", "b"));
            log.head_hash()
        };
        let log = AuditLog::with_persistence(&path).unwrap();
        assert_eq!(log.len(), 2, "entries survive reopen");
        let entries = log.entries();
        assert_eq!(entries[0].command.agent, "main");
        assert_eq!(entries[0].duration_us, Some(100));
        assert_eq!(entries[1].command.agent, "worker");
        assert_eq!(log.head_hash(), head);
        assert_eq!(log.record_command(read("main", "c")), 3);
        assert!(matches!(log.verify_chain(), ChainVerdict::Intact { events: 4, .. }));
    }

    #[test]
    fn tampered_disk_log_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.db");
        {
            let log = AuditLog::with_persistence(&path).unwrap();
            let seq = log.record_command(read("main", "a"));
            log.record_result(seq, CommandResult::ok("", None), 1);
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("UPDATE audit_events SET payload = replace(payload, '\"ok\"', '\"error\"') WHERE id = 2", [])
            .unwrap();
        let err = match AuditLog::with_persistence(&path) {
            Ok(_) => panic!("a tampered log must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("breaks at event 2"), "{err}");
    }

    #[test]
    fn summary_line_is_single_line_and_names_the_op() {
        let log = AuditLog::new();
        let seq = log.record_command(Command::new(
            "main",
            CommandOp::ShellExec { command: "cargo test\n--all".into(), workdir: None, timeout_ms: None },
        ));
        log.record_result(seq, CommandResult::ok("", None), 1_500);
        let line = log.entries()[0].summary_line();
        assert!(!line.contains('\n'));
        assert!(line.contains("ShellExec"));
        assert!(line.contains("Ok"));
        assert!(line.contains("1.5ms"));
    }

    #[test]
    fn op_summary_truncates_long_prompts() {
        let op = CommandOp::ProviderCall {
            system_prompt: "s".into(),
            user_prompt: "x".repeat(500),
            max_tokens: None,
            temperature: None,
            json: false,
        };
        let s = op_summary(&op);
        assert!(s.starts_with("ProviderCall"));
        assert_eq!(s.chars().count(), 60);
    }
}
