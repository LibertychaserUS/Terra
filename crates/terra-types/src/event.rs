//! Streaming Events
//!
//! Structured events emitted during Terra operations.
//! Consumable by terminal, TUI, or web UI.
//! Every event has: type, timestamp, data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A structured event emitted during operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event_type: EventType,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}

/// Types of events the system can emit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// LLM token arrived (streaming).
    Token,
    /// A step in a Skill started.
    StepStart,
    /// A step completed.
    StepComplete,
    /// A step failed.
    StepFailed,
    /// Mode changed (Conversation ↔ Goal).
    ModeSwitch,
    /// Memory auto-record notification.
    MemoryRecord,
    /// Drift detection result.
    DriftCheck,
    /// Audit entry created.
    AuditEntry,
    /// Stream finished.
    Done,
}

impl StreamEvent {
    pub fn token(text: &str) -> Self {
        Self {
            event_type: EventType::Token,
            timestamp: Utc::now(),
            data: serde_json::json!({"text": text}),
        }
    }

    pub fn step_start(step_name: &str, description: &str) -> Self {
        Self {
            event_type: EventType::StepStart,
            timestamp: Utc::now(),
            data: serde_json::json!({
                "step": step_name,
                "description": description,
            }),
        }
    }

    pub fn step_complete(step_name: &str) -> Self {
        Self {
            event_type: EventType::StepComplete,
            timestamp: Utc::now(),
            data: serde_json::json!({"step": step_name}),
        }
    }

    pub fn step_failed(step_name: &str, error: &str) -> Self {
        Self {
            event_type: EventType::StepFailed,
            timestamp: Utc::now(),
            data: serde_json::json!({"step": step_name, "error": error}),
        }
    }

    pub fn done() -> Self {
        Self {
            event_type: EventType::Done,
            timestamp: Utc::now(),
            data: serde_json::json!({}),
        }
    }
}

/// Tracks cumulative token usage across a session.
#[derive(Debug, Clone, Default)]
pub struct TokenTracker {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_calls: u64,
}

impl TokenTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, prompt: u32, completion: u32) {
        self.prompt_tokens += prompt as u64;
        self.completion_tokens += completion as u64;
        self.total_calls += 1;
    }

    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

/// Event Bus — channel for streaming events from Executor to UI.
///
/// Implementations:
/// - ChannelBus: async mpsc channel (for TUI)
/// - CollectorBus: collects events in Vec (for testing)
/// - NullBus: discards events (for batch mode)
pub trait EventBus: Send + Sync {
    fn emit(&self, event: StreamEvent);
}

/// Null bus — discards all events. Default when no UI attached.
pub struct NullBus;
impl EventBus for NullBus {
    fn emit(&self, _event: StreamEvent) {}
}

/// Collector bus — stores events for testing/inspection.
pub struct CollectorBus {
    events: std::sync::Mutex<Vec<StreamEvent>>,
}

impl CollectorBus {
    pub fn new() -> Self {
        Self { events: std::sync::Mutex::new(Vec::new()) }
    }

    pub fn events(&self) -> Vec<StreamEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }
}

impl EventBus for CollectorBus {
    fn emit(&self, event: StreamEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-STR-006: event stream structured
    #[test]
    fn events_have_consistent_structure() {
        let events = vec![
            StreamEvent::token("hello"),
            StreamEvent::step_start("read", "Reading file"),
            StreamEvent::step_complete("read"),
            StreamEvent::step_failed("write", "permission denied"),
            StreamEvent::done(),
        ];

        for event in &events {
            // Every event has type + timestamp + data
            assert!(!event.timestamp.to_rfc3339().is_empty());
            assert!(event.data.is_object() || event.data.is_null());
        }

        assert_eq!(events[0].event_type, EventType::Token);
        assert_eq!(events[0].data["text"], "hello");
        assert_eq!(events[1].event_type, EventType::StepStart);
        assert_eq!(events[4].event_type, EventType::Done);
    }

    // T-STR-007: token count tracked
    #[test]
    fn token_count_tracked() {
        let mut tracker = TokenTracker::new();
        assert_eq!(tracker.total_tokens(), 0);

        tracker.record(100, 50);
        assert_eq!(tracker.prompt_tokens, 100);
        assert_eq!(tracker.completion_tokens, 50);
        assert_eq!(tracker.total_tokens(), 150);
        assert_eq!(tracker.total_calls, 1);

        tracker.record(200, 80);
        assert_eq!(tracker.total_tokens(), 430);
        assert_eq!(tracker.total_calls, 2);
    }
}
