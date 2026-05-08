//! Long Conversation Test — 30+ turns with real LLM
//!
//! Simulates a real user session: conversation, mode switching,
//! memory operations, multi-topic dialogue. Checks for:
//! - Context accumulation correctness
//! - Token tracking accuracy
//! - Mode switching stability
//! - Command handling mid-conversation
//! - Response quality over long context
//! - Memory operations
//! - Error recovery

use std::sync::Arc;

use terra_agents::repl::{Repl, ReplOutput};
use terra_caging::engine::*;
use terra_caging::sandbox::SandboxGuard;
use terra_core::domain::{AlgorithmModule, CodingModule};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::{ArtifactStore, MemoryDatabase};
use terra_providers::adapter::ProviderAdapter;
use terra_providers::OpenAICompatibleAdapter;
use terra_trace::AuditLog;

fn has_api_key() -> bool {
    std::env::var("DASHSCOPE_API_KEY").is_ok()
}

fn live_repl() -> Option<(Repl, tempfile::TempDir)> {
    let provider: Arc<dyn ProviderAdapter> = match OpenAICompatibleAdapter::from_env() {
        Ok(a) => Arc::new(a),
        Err(_) => return None,
    };
    let dir = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
    let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
    let audit = Arc::new(AuditLog::new());
    let sandbox = SandboxGuard::new(dir.path().to_path_buf(), vec![]);
    let caging = Arc::new(CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]));
    let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));

    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));
    let core = Arc::new(core);

    Some((Repl::new(exec, core), dir))
}

/// Extract text responses from ReplOutput vec.
/// Collects both Text and Token outputs (Token is now used for streamed LLM responses).
fn extract_text(outputs: &[ReplOutput]) -> String {
    outputs.iter()
        .filter_map(|o| match o {
            ReplOutput::Text(t) if !t.is_empty() => Some(t.as_str()),
            ReplOutput::Token(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn has_notification(outputs: &[ReplOutput]) -> bool {
    outputs.iter().any(|o| matches!(o, ReplOutput::Notification(_)))
}

/// 30+ turn long conversation with real LLM.
/// Tests the full dialogue lifecycle.
#[tokio::test]
async fn long_conversation_30_turns() {
    if !has_api_key() {
        eprintln!("skipping long_conversation_30_turns (no API key)");
        return;
    }

    let (mut repl, _dir) = match live_repl() {
        Some(r) => r,
        None => { eprintln!("skipping (provider init failed)"); return; }
    };

    let mut turn = 0;
    let mut issues: Vec<String> = Vec::new();

    // Helper to run a turn and check
    macro_rules! turn {
        ($input:expr, $check:expr) => {
            turn += 1;
            let outputs = repl.process_input($input).await;
            let text = extract_text(&outputs);
            eprintln!("--- Turn {} ---", turn);
            eprintln!("User: {}", $input);
            let display_end = text.char_indices().nth(150).map(|(i, _)| i).unwrap_or(text.len());
            eprintln!("Terra: {}", &text[..display_end]);
            let check_fn: &dyn Fn(&str, &[ReplOutput]) -> Option<String> = &$check;
            if let Some(issue) = check_fn(&text, &outputs) {
                issues.push(format!("Turn {}: {}", turn, issue));
                eprintln!("  ISSUE: {}", issues.last().unwrap());
            }
        };
    }

    // === Phase 1: Basic conversation (turns 1-5) ===
    eprintln!("\n=== Phase 1: Basic Conversation ===");

    turn!("Hello! I'm starting a new project in Rust.", |text: &str, _| {
        if text.is_empty() { Some("empty response".into()) } else { None }
    });

    turn!("What are the best practices for error handling in Rust?", |text: &str, _| {
        if !text.to_lowercase().contains("result") && !text.to_lowercase().contains("error") {
            Some("should mention Result/Error types".into())
        } else { None }
    });

    turn!("Can you explain the difference between String and &str?", |text: &str, _| {
        if !text.to_lowercase().contains("heap") && !text.to_lowercase().contains("owned") && !text.to_lowercase().contains("borrow") {
            Some("should mention ownership/heap/borrowing".into())
        } else { None }
    });

    turn!("/status", |text: &str, _| {
        if !text.contains("conversation") { Some("should show conversation mode".into()) }
        else if !text.contains("Tokens:") { Some("should show token count".into()) }
        else { None }
    });

    turn!("What did we discuss so far?", |text: &str, _| {
        // Tests context memory — should reference earlier topics
        if !text.to_lowercase().contains("rust") && !text.to_lowercase().contains("error") {
            Some("should recall earlier conversation context".into())
        } else { None }
    });

    // === Phase 2: Goal Mode (turns 6-12) ===
    eprintln!("\n=== Phase 2: Goal Mode ===");

    turn!("/goal Implement a binary search function in Rust", |_text: &str, outputs: &[ReplOutput]| {
        if !has_notification(outputs) { Some("should emit notification for mode switch".into()) }
        else { None }
    });

    turn!("/status", |text: &str, _| {
        if !text.contains("goal") { Some("should show goal mode".into()) }
        else { None }
    });

    turn!("What should the function signature look like?", |text: &str, _| {
        if text.is_empty() { Some("empty response in goal mode".into()) } else { None }
    });

    turn!("Write the implementation", |text: &str, _| {
        if !text.contains("fn") && !text.contains("binary") {
            Some("should write code related to binary search".into())
        } else { None }
    });

    turn!("Now add unit tests for it", |text: &str, _| {
        if !text.to_lowercase().contains("test") {
            Some("should write tests".into())
        } else { None }
    });

    turn!("What edge cases should we handle?", |text: &str, _| {
        if !text.to_lowercase().contains("empty") && !text.to_lowercase().contains("edge") {
            Some("should mention edge cases".into())
        } else { None }
    });

    turn!("/done", |_text: &str, outputs: &[ReplOutput]| {
        if !has_notification(outputs) { Some("should emit notification for mode exit".into()) }
        else { None }
    });

    // === Phase 3: Topic switching (turns 13-18) ===
    eprintln!("\n=== Phase 3: Topic Switching ===");

    turn!("Let's switch topics. Explain microservices vs monolith.", |text: &str, _| {
        if text.is_empty() { Some("empty response".into()) } else { None }
    });

    turn!("What are the trade-offs of event-driven architecture?", |text: &str, _| {
        if text.is_empty() { Some("empty response".into()) } else { None }
    });

    turn!("How does Kafka compare to RabbitMQ?", |text: &str, _| {
        if !text.to_lowercase().contains("kafka") && !text.to_lowercase().contains("rabbit") {
            Some("should compare the two".into())
        } else { None }
    });

    turn!("/remember Always use structured logging in production", |_text: &str, outputs: &[ReplOutput]| {
        if !has_notification(outputs) { Some("should confirm remember".into()) }
        else { None }
    });

    turn!("Back to Rust — what's the best HTTP framework?", |text: &str, _| {
        if !text.to_lowercase().contains("axum") && !text.to_lowercase().contains("actix") && !text.to_lowercase().contains("rocket") {
            Some("should mention at least one Rust HTTP framework".into())
        } else { None }
    });

    turn!("/status", |text: &str, _| {
        if !text.contains("Tokens:") { Some("should show accumulated tokens".into()) }
        else { None }
    });

    // === Phase 4: Error handling + edge cases (turns 19-24) ===
    eprintln!("\n=== Phase 4: Error Handling ===");

    turn!("/nonexistent_command", |text: &str, _| {
        if !text.contains("Unknown") { Some("should report unknown command".into()) }
        else { None }
    });

    turn!("", |_text: &str, outputs: &[ReplOutput]| {
        if !outputs.iter().any(|o| matches!(o, ReplOutput::Prompt(_))) {
            Some("empty input should return prompt".into())
        } else { None }
    });

    turn!("/goal", |text: &str, _| {
        if !text.contains("Usage") { Some("should show usage for empty /goal".into()) }
        else { None }
    });

    turn!("/goal Design a caching layer", |_text: &str, _| { None });

    // Nested goal should fail
    turn!("/goal Another task", |text: &str, _| {
        if !text.to_lowercase().contains("error") && !text.to_lowercase().contains("already") && !text.to_lowercase().contains("/done") {
            Some("nested goal should be rejected".into())
        } else { None }
    });

    turn!("/done", |_text: &str, _| { None });

    // === Phase 5: Deep technical (turns 25-30+) ===
    eprintln!("\n=== Phase 5: Deep Technical ===");

    turn!("Explain how Linux's CFS scheduler works at a technical level", |text: &str, _| {
        if !text.to_lowercase().contains("vruntime") && !text.to_lowercase().contains("red-black") && !text.to_lowercase().contains("fair") {
            Some("should mention vruntime or red-black tree or fairness".into())
        } else { None }
    });

    turn!("How does Rust's borrow checker work internally?", |text: &str, _| {
        if text.is_empty() { Some("empty response".into()) } else { None }
    });

    turn!("Compare B-trees and LSM-trees for database storage engines", |text: &str, _| {
        if !text.to_lowercase().contains("b-tree") && !text.to_lowercase().contains("lsm") {
            Some("should discuss both data structures".into())
        } else { None }
    });

    turn!("What's the CAP theorem and how does Spanner circumvent it?", |text: &str, _| {
        if !text.to_lowercase().contains("cap") {
            Some("should discuss CAP".into())
        } else { None }
    });

    turn!("Explain the Raft consensus algorithm step by step", |text: &str, _| {
        if !text.to_lowercase().contains("leader") && !text.to_lowercase().contains("election") {
            Some("should explain leader election".into())
        } else { None }
    });

    turn!("What was our first topic today?", |text: &str, _| {
        // Critical: after 30 turns, can it still recall the beginning?
        if !text.to_lowercase().contains("rust") {
            Some("CONTEXT LOSS: cannot recall first topic (Rust)".into())
        } else { None }
    });

    // === Final status ===
    let final_outputs = repl.process_input("/status").await;
    let final_text = extract_text(&final_outputs);
    eprintln!("\n=== Final Status ===\n{}", final_text);

    let total_tokens = repl.tokens().total_tokens();
    eprintln!("\nTotal tokens consumed: {}", total_tokens);
    eprintln!("Total turns: {}", turn);

    // === Report ===
    eprintln!("\n=== Issue Report ===");
    if issues.is_empty() {
        eprintln!("No issues found across {} turns!", turn);
    } else {
        eprintln!("{} issues found across {} turns:", issues.len(), turn);
        for issue in &issues {
            eprintln!("  - {}", issue);
        }
    }

    // Assertions
    assert!(turn >= 30, "should have at least 30 turns, got {}", turn);
    assert!(total_tokens > 0, "should have consumed tokens");

    // Allow up to 5 minor issues (topic recall after 30 turns may fail)
    let critical_issues: Vec<_> = issues.iter()
        .filter(|i| i.contains("CONTEXT LOSS") || i.contains("empty response") || i.contains("nested goal"))
        .collect();
    assert!(
        critical_issues.len() <= 2,
        "too many critical issues: {:?}", critical_issues
    );
}
