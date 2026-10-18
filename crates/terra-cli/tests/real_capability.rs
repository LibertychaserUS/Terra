//! Real Capability Tests
//!
//! Tests Terra's actual output quality through real LLM calls.
//! Categories: LeetCode, Web Development, Architecture Design, Research.
//! Ignored by default; run with an API key set: cargo test -- --ignored

use std::sync::Arc;

use terra_caging::engine::*;
use terra_caging::sandbox::SandboxGuard;
use terra_core::domain::{AlgorithmModule, CodingModule};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::{ArtifactStore, MemoryDatabase};
use terra_providers::adapter::ProviderAdapter;
use terra_providers::OpenAICompatibleAdapter;
use terra_trace::AuditLog;
use terra_types::command::{Command, CommandOp, CommandStatus};

fn safe_truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn has_api_key() -> bool {
    std::env::var("DASHSCOPE_API_KEY").is_ok()
}

fn live_setup() -> Option<(Arc<CommandExecutor>, tempfile::TempDir)> {
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
    Some((exec, dir))
}

async fn ask(exec: &CommandExecutor, system: &str, prompt: &str) -> String {
    let cmd = Command::new("test", CommandOp::ProviderCall {
        system_prompt: system.into(),
        user_prompt: prompt.into(),
        max_tokens: Some(2000),
        temperature: Some(0.0),
        output_contract: None,
    });
    let result = exec.execute(cmd).await;
    result.data
        .and_then(|d| d.get("content").and_then(|c| c.as_str().map(String::from)))
        .unwrap_or_default()
}

// ====================================================================
// 1. LeetCode — code generation quality
// ====================================================================

/// LeetCode #1: Two Sum (Easy)
/// Optimal: HashMap O(n), General: brute force O(n²)
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn leetcode_two_sum() {
    assert!(has_api_key(), "live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY");
    let (exec, _dir) = live_setup().unwrap();

    let response = ask(&exec,
        "You are an expert algorithm engineer. Write clean, optimal code.",
        "Solve LeetCode #1 Two Sum in Rust. Given nums: Vec<i32> and target: i32, return indices of two numbers that add up to target. Write ONLY the function, no explanation.",
    ).await;

    eprintln!("=== Two Sum ===\n{}", response);

    // Quality checks
    assert!(response.contains("fn") || response.contains("pub fn"), "should contain function definition");
    // Optimal solution uses HashMap
    let is_optimal = response.contains("HashMap") || response.contains("hash_map");
    eprintln!("Optimal (HashMap): {}", is_optimal);
}

/// LeetCode #3: Longest Substring Without Repeating Characters (Medium)
/// Optimal: sliding window O(n)
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn leetcode_longest_substring() {
    assert!(has_api_key(), "live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY");
    let (exec, _dir) = live_setup().unwrap();

    let response = ask(&exec,
        "You are an expert algorithm engineer. Write optimal solutions.",
        "Solve LeetCode #3: Longest Substring Without Repeating Characters in Rust. fn length_of_longest_substring(s: String) -> i32. Write ONLY the function.",
    ).await;

    eprintln!("=== Longest Substring ===\n{}", response);

    assert!(response.contains("fn"), "should have function");
    // Optimal: sliding window with HashSet/HashMap
    let is_optimal = response.contains("HashSet") || response.contains("HashMap") || response.contains("window");
    eprintln!("Optimal (sliding window): {}", is_optimal);
}

/// LeetCode #146: LRU Cache (Medium)
/// Tests data structure design ability
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn leetcode_lru_cache() {
    assert!(has_api_key(), "live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY");
    let (exec, _dir) = live_setup().unwrap();

    let response = ask(&exec,
        "You are an expert systems programmer. Write production-quality Rust.",
        "Implement LRU Cache in Rust (LeetCode #146). Support get(key) and put(key, value) in O(1). Write the complete struct and impl.",
    ).await;

    eprintln!("=== LRU Cache ===\n{}", response);

    assert!(response.contains("struct"), "should have struct definition");
    assert!(response.contains("get") && response.contains("put"), "should have get/put methods");
    // O(1) requires HashMap + LinkedList
    let is_optimal = response.contains("HashMap") && (response.contains("LinkedList") || response.contains("list") || response.contains("Node"));
    eprintln!("Optimal (HashMap+LinkedList): {}", is_optimal);
}

// ====================================================================
// 2. Web Development — practical generation
// ====================================================================

/// Simple webpage: generate complete HTML/CSS/JS
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn build_simple_webpage() {
    assert!(has_api_key(), "live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY");
    let (exec, dir) = live_setup().unwrap();

    let response = ask(&exec,
        "You are a frontend developer. Output ONLY the HTML code, no explanation.",
        "Create a single HTML file with embedded CSS and JS: a landing page with a centered title 'Terra Agent OS', a subtitle 'Efficiency is Intelligence', a dark theme, and a button that shows an alert when clicked.",
    ).await;

    eprintln!("=== Webpage ===\n{}", safe_truncate(&response, 500));

    // Write to file and verify structure
    let html_path = dir.path().join("index.html");
    std::fs::write(&html_path, &response).unwrap();

    assert!(response.contains("<html") || response.contains("<!DOCTYPE"), "should be HTML");
    assert!(response.contains("Terra"), "should contain title");
    assert!(response.contains("<style") || response.contains("style="), "should have CSS");
    assert!(response.contains("<script") || response.contains("onclick"), "should have JS");
    assert!(html_path.exists(), "file should be written");

    let file_size = std::fs::metadata(&html_path).unwrap().len();
    eprintln!("HTML file size: {} bytes", file_size);
}

// ====================================================================
// 3. Architecture Design — knowledge breadth & depth
// ====================================================================

/// Complex web system: architecture + tech stack
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn architecture_design() {
    assert!(has_api_key(), "live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY");
    let (exec, _dir) = live_setup().unwrap();

    let response = ask(&exec,
        "You are a senior systems architect with 15 years experience.",
        "Design the architecture for a real-time collaborative document editing system (like Google Docs) that supports 10,000 concurrent users. Include:\n1. System components and their responsibilities\n2. Technology stack with justification\n3. Data consistency strategy (CRDT vs OT)\n4. Scaling approach\n5. Key trade-offs\nBe specific about technology choices.",
    ).await;

    eprintln!("=== Architecture Design ===\n{}", safe_truncate(&response, 1000));

    // Knowledge depth checks
    let mentions_crdt = response.to_lowercase().contains("crdt");
    let mentions_ot = response.to_lowercase().contains("operational transform") || response.to_lowercase().contains("ot");
    let mentions_websocket = response.to_lowercase().contains("websocket");
    let mentions_redis = response.to_lowercase().contains("redis");
    let mentions_scaling = response.to_lowercase().contains("horizontal") || response.to_lowercase().contains("shard");

    eprintln!("CRDT mentioned: {}", mentions_crdt);
    eprintln!("OT mentioned: {}", mentions_ot);
    eprintln!("WebSocket: {}", mentions_websocket);
    eprintln!("Redis: {}", mentions_redis);
    eprintln!("Scaling strategy: {}", mentions_scaling);

    assert!(mentions_crdt || mentions_ot, "should discuss consistency strategy");
    assert!(mentions_websocket, "should mention WebSocket for real-time");
}

// ====================================================================
// 4. Scientific Research — reasoning & creativity
// ====================================================================

/// Research exploration: novel application of existing concepts
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn research_exploration() {
    assert!(has_api_key(), "live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY");
    let (exec, _dir) = live_setup().unwrap();

    let response = ask(&exec,
        "You are a researcher at the intersection of programming languages and AI safety.",
        "Propose a novel approach to formally verifying AI agent behavior using ideas from type theory and linear logic. Specifically:\n1. How could linear types prevent an agent from using a resource (API call, file access) more than its budget allows?\n2. How could session types model and enforce multi-agent communication protocols?\n3. What are the open problems in making this practical?\nBe technically precise.",
    ).await;

    eprintln!("=== Research Exploration ===\n{}", safe_truncate(&response, 1200));

    // Depth checks
    let mentions_linear = response.to_lowercase().contains("linear");
    let mentions_session_types = response.to_lowercase().contains("session type");
    let mentions_resource = response.to_lowercase().contains("resource") || response.to_lowercase().contains("affine");
    let has_open_problems = response.to_lowercase().contains("open problem") || response.to_lowercase().contains("challenge") || response.to_lowercase().contains("limitation");

    eprintln!("Linear types: {}", mentions_linear);
    eprintln!("Session types: {}", mentions_session_types);
    eprintln!("Resource management: {}", mentions_resource);
    eprintln!("Open problems: {}", has_open_problems);

    assert!(mentions_linear, "should discuss linear types");
    assert!(has_open_problems, "should identify open problems");
}
