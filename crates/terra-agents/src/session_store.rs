//! Session Persistence
//!
//! Persists conversation sessions to .terra/sessions/{id}.json.
//! Supports: save, load, list, resume last.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A persisted session record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub turns: Vec<TurnRecord>,
    pub mode_history: Vec<ModeEvent>,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub goal: Option<String>,
}

/// A single conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRecord {
    pub role: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

/// A mode change event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeEvent {
    pub from: String,
    pub to: String,
    pub timestamp: DateTime<Utc>,
}

/// Summary of a session for listing.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub turn_count: usize,
    pub total_tokens: u64,
    pub goal: Option<String>,
}

impl SessionRecord {
    /// Create a new empty session.
    pub fn new() -> Self {
        let id = format!("{}", Utc::now().format("%Y%m%d_%H%M%S"));
        Self {
            id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turns: Vec::new(),
            mode_history: Vec::new(),
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            goal: None,
        }
    }

    /// Add a turn.
    pub fn add_turn(&mut self, role: &str, content: &str) {
        self.turns.push(TurnRecord {
            role: role.into(),
            content: content.into(),
            timestamp: Utc::now(),
        });
        self.updated_at = Utc::now();
    }

    /// Record a mode change.
    pub fn record_mode_change(&mut self, from: &str, to: &str) {
        self.mode_history.push(ModeEvent {
            from: from.into(),
            to: to.into(),
            timestamp: Utc::now(),
        });
    }

    /// Update token counts.
    pub fn update_tokens(&mut self, prompt: u64, completion: u64) {
        self.total_prompt_tokens = prompt;
        self.total_completion_tokens = completion;
    }

    /// Total tokens.
    pub fn total_tokens(&self) -> u64 {
        self.total_prompt_tokens + self.total_completion_tokens
    }
}

/// Manages session files on disk.
pub struct SessionStore {
    sessions_dir: PathBuf,
}

impl SessionStore {
    /// Create a store for the given .terra/sessions/ directory.
    pub fn new(terra_dir: &Path) -> Self {
        let sessions_dir = terra_dir.join("sessions");
        Self { sessions_dir }
    }

    /// Ensure the sessions directory exists.
    fn ensure_dir(&self) -> std::io::Result<()> {
        if !self.sessions_dir.exists() {
            std::fs::create_dir_all(&self.sessions_dir)?;
        }
        Ok(())
    }

    /// Save a session to disk.
    pub fn save(&self, record: &SessionRecord) -> anyhow::Result<()> {
        self.ensure_dir()?;
        let path = self.sessions_dir.join(format!("{}.json", record.id));
        let json = serde_json::to_string_pretty(record)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    /// Load a session by ID.
    pub fn load(&self, id: &str) -> anyhow::Result<SessionRecord> {
        let path = self.sessions_dir.join(format!("{}.json", id));
        let json = std::fs::read_to_string(&path)?;
        let record: SessionRecord = serde_json::from_str(&json)?;
        Ok(record)
    }

    /// List all sessions, newest first.
    pub fn list(&self) -> Vec<SessionSummary> {
        let mut summaries = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&self.sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(json) = std::fs::read_to_string(&path) {
                        if let Ok(record) = serde_json::from_str::<SessionRecord>(&json) {
                            summaries.push(SessionSummary {
                                id: record.id.clone(),
                                created_at: record.created_at,
                                turn_count: record.turns.len(),
                                total_tokens: record.total_tokens(),
                                goal: record.goal.clone(),
                            });
                        }
                    }
                }
            }
        }

        summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        summaries
    }

    /// Get the most recent session ID.
    pub fn last_session_id(&self) -> Option<String> {
        self.list().first().map(|s| s.id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-SS-001: session saved to disk
    #[test]
    fn session_saved_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        let mut record = SessionRecord::new();
        record.add_turn("user", "hello");
        record.add_turn("assistant", "hi there");
        store.save(&record).unwrap();

        let path = dir.path().join("sessions").join(format!("{}.json", record.id));
        assert!(path.exists(), "session file should exist");
    }

    // T-SS-002: session contains turns
    #[test]
    fn session_contains_turns() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        let mut record = SessionRecord::new();
        record.add_turn("user", "what is rust?");
        record.add_turn("assistant", "a systems language");
        record.update_tokens(100, 50);
        store.save(&record).unwrap();

        let loaded = store.load(&record.id).unwrap();
        assert_eq!(loaded.turns.len(), 2);
        assert_eq!(loaded.turns[0].role, "user");
        assert_eq!(loaded.turns[0].content, "what is rust?");
        assert_eq!(loaded.turns[1].role, "assistant");
        assert_eq!(loaded.total_prompt_tokens, 100);
    }

    // T-SS-004: session list shows history
    #[test]
    fn session_list_shows_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        let mut r1 = SessionRecord::new();
        r1.id = "session_001".into();
        r1.add_turn("user", "first");
        store.save(&r1).unwrap();

        let mut r2 = SessionRecord::new();
        r2.id = "session_002".into();
        r2.add_turn("user", "second");
        r2.add_turn("assistant", "reply");
        store.save(&r2).unwrap();

        let list = store.list();
        assert_eq!(list.len(), 2);
        // Newest first
        assert!(list.iter().any(|s| s.id == "session_002" && s.turn_count == 2));
    }

    // T-SS-005: new session starts fresh
    #[test]
    fn new_session_fresh() {
        let record = SessionRecord::new();
        assert!(record.turns.is_empty());
        assert!(record.goal.is_none());
        assert!(!record.id.is_empty());
    }

    // T-SS-006: session survives restart (save + load)
    #[test]
    fn session_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        // First "run"
        let mut record = SessionRecord::new();
        let session_id = record.id.clone();
        record.add_turn("user", "remember this");
        record.add_turn("assistant", "I will");
        store.save(&record).unwrap();

        // Second "run" — reload
        let restored = store.load(&session_id).unwrap();
        assert_eq!(restored.turns.len(), 2);
        assert_eq!(restored.turns[0].content, "remember this");
    }

    // T-SS-007: mode changes recorded
    #[test]
    fn session_records_mode_changes() {
        let mut record = SessionRecord::new();
        record.record_mode_change("conversation", "goal");
        record.record_mode_change("goal", "conversation");
        assert_eq!(record.mode_history.len(), 2);
        assert_eq!(record.mode_history[0].to, "goal");
    }

    // T-SS-008: token usage recorded
    #[test]
    fn session_records_token_usage() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        let mut record = SessionRecord::new();
        record.update_tokens(500, 200);
        store.save(&record).unwrap();

        let loaded = store.load(&record.id).unwrap();
        assert_eq!(loaded.total_tokens(), 700);
    }

    // Last session ID
    #[test]
    fn last_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        assert!(store.last_session_id().is_none());

        let mut r = SessionRecord::new();
        r.id = "latest".into();
        store.save(&r).unwrap();

        assert_eq!(store.last_session_id(), Some("latest".into()));
    }
}
