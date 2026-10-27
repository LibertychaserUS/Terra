//! The `terra` binary in an isolated project and HOME, without an API key.

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

/// Temp project dir, temp HOME (the developer's ~/.terra is never touched)
/// and no provider keys (no real LLM calls).
struct TestEnv {
    project: tempfile::TempDir,
    home: tempfile::TempDir,
}

impl TestEnv {
    fn new() -> Self {
        Self { project: tempfile::tempdir().unwrap(), home: tempfile::tempdir().unwrap() }
    }

    fn terra(&self) -> Command {
        let mut cmd = Command::cargo_bin("terra").unwrap();
        cmd.current_dir(self.project.path())
            .env("HOME", self.home.path())
            .env_remove("TERRA_PROVIDER_API_KEY")
            .env_remove("DASHSCOPE_API_KEY")
            .env_remove("TERRA_PROVIDER_BASE_URL")
            .env_remove("TERRA_PROVIDER_MODEL");
        cmd
    }

    fn git(&self, args: &[&str]) -> String {
        let out = std::process::Command::new("git").args(args).current_dir(self.project.path()).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }
}

#[test]
fn chat_without_input_prints_banner_and_exits() {
    TestEnv::new().terra().write_stdin("").assert().success().stdout(predicate::str::contains("Terra"));
}

#[test]
fn doctor_shows_status() {
    TestEnv::new()
        .terra()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("Status:       dry-run only"))
        .stdout(predicate::str::contains("not initialized"))
        .stdout(predicate::str::contains("Shell:        isolation"));
}

#[test]
fn doctor_as_json() {
    let output = TestEnv::new().terra().args(["doctor", "--as-json"]).output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert!(json.get("os").is_some());
    assert!(json.get("terra_version").is_some());
    assert!(json.get("shell_isolation").is_some());
}

#[test]
fn run_with_task() {
    TestEnv::new()
        .terra()
        .args(["run", "--task", "add a test to src/lib.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Terra run complete"))
        .stdout(predicate::str::contains("nothing was written"));
}

#[test]
fn run_json_has_required_fields() {
    let output = TestEnv::new().terra().args(["run", "--task", "fix src/lib.rs", "--as-json"]).output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for field in ["status", "spec_id", "intent", "response", "provider", "verification", "audit_entries"] {
        assert!(json.get(field).is_some(), "missing {field}: {json}");
    }
    assert_eq!(json["status"], "complete");
    assert_eq!(json["provider"], "dry_run");
}

// The specification cage stops a task with open questions before the model is called.
#[test]
fn run_with_open_questions_is_blocked() {
    let env = TestEnv::new();
    env.terra()
        .args(["run", "--task", "test task"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("cage 'specification' blocked"))
        .stderr(predicate::str::contains("Target file not specified"));

    let out = env.terra().args(["run", "--task", "test task", "--as-json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["status"], "blocked");
    assert_eq!(json["open_questions"][0], "Target file not specified");
}

// The constitution's require_spec_complete switch turns the gate off.
#[test]
fn constitution_can_turn_off_the_spec_gate() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    let path = env.project.path().join(".terra/constitution.toml");
    let rules = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, rules.replace("require_spec_complete = true", "require_spec_complete = false")).unwrap();
    env.terra().args(["run", "--task", "test task"]).assert().success().stdout(predicate::str::contains("Terra run complete"));
}

/// Drive `terra mcp` over stdio: initialize, then one `FileWrite` call.
/// Returns the JSON-RPC response to the call.
fn mcp_file_write(env: &TestEnv, path: &str, content: &str) -> serde_json::Value {
    let init = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }
    });
    let initialized = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "FileWrite", "arguments": { "path": path, "content": content } },
    });
    let stdin = format!("{init}\n{initialized}\n{call}\n");
    let out = env.terra().arg("mcp").write_stdin(stdin).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["id"] == 2)
        .unwrap_or_else(|| panic!("no response to the call; stdout: {stdout}; stderr: {}", String::from_utf8_lossy(&out.stderr)))
}

// The meta cage keeps the agent from writing the rules that cage it, also in a
// project with no constitution yet (where no path is listed as protected).
#[test]
fn agent_cannot_write_the_constitution_over_mcp() {
    let env = TestEnv::new();
    let wider_sandbox = "[meta]\nversion = 1\n[sandbox]\nroot = \"/\"\n";
    let constitution = env.project.path().join(".terra/constitution.toml");

    let resp = mcp_file_write(&env, ".terra/constitution.toml", wider_sandbox);
    assert!(resp["error"]["message"].as_str().unwrap().contains("cage 'meta' blocked"), "{}", resp);
    assert!(!constitution.exists());

    env.terra().arg("init").assert().success();
    let original = std::fs::read_to_string(&constitution).unwrap();
    let resp = mcp_file_write(&env, ".terra/../.terra/constitution.toml", wider_sandbox);
    assert!(resp["error"]["message"].as_str().unwrap().contains("cage 'meta' blocked"), "{}", resp);
    assert_eq!(std::fs::read_to_string(&constitution).unwrap(), original);

    let resp = mcp_file_write(&env, "notes.md", "hello");
    assert!(resp["error"].is_null(), "{}", resp);
    assert_eq!(std::fs::read_to_string(env.project.path().join("notes.md")).unwrap(), "hello");
}

#[test]
fn help_flag() {
    TestEnv::new()
        .terra()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("config"));
}

#[test]
fn unknown_command_error() {
    TestEnv::new().terra().arg("nonexistent-command").assert().failure();
}

#[test]
fn config_show_displays() {
    TestEnv::new()
        .terra()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("provider"))
        .stdout(predicate::str::contains("model"))
        .stdout(predicate::str::contains("base_url"));
}

/// Files under .terra/ that git would pick up (untracked or modified).
fn terra_files_visible_to_git(env: &TestEnv) -> Vec<String> {
    env.git(&["status", "--porcelain", "--untracked-files=all"])
        .lines()
        .map(|l| l[3..].to_string())
        .filter(|p| p.starts_with(".terra/"))
        .collect()
}

fn assert_only_shareable_terra_files(visible: &[String]) {
    for path in visible {
        let name = Path::new(path).file_name().unwrap().to_str().unwrap();
        assert!(
            matches!(name, ".gitignore" | "constitution.toml" | "prompt.md"),
            "runtime file {} would be committed; visible: {:?}",
            path,
            visible
        );
    }
}

#[test]
fn init_keeps_runtime_data_out_of_git() {
    let env = TestEnv::new();
    env.git(&["init", "-q"]);
    env.terra().arg("init").assert().success();
    env.terra().args(["run", "--task", "touch notes.md"]).assert().success();
    assert!(env.project.path().join(".terra/memory.db").exists());
    assert!(env.project.path().join(".terra/audit.db").exists());
    assert_only_shareable_terra_files(&terra_files_visible_to_git(&env));
}

#[test]
fn chat_auto_init_keeps_runtime_data_out_of_git() {
    let env = TestEnv::new();
    env.git(&["init", "-q"]);
    env.terra().arg("chat").write_stdin("/remember note\n/quit\n").assert().success();
    assert!(env.project.path().join(".terra/history.txt").exists());
    let visible = terra_files_visible_to_git(&env);
    assert!(visible.iter().any(|p| p.ends_with(".gitignore")), "{:?}", visible);
    assert_only_shareable_terra_files(&visible);
}

/// Serve canned OpenAI-style chat completions on a local port.
fn spawn_stub_llm(reply: &'static str) -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).ok();
            let json = serde_json::json!({
                "id": "stub", "object": "chat.completion", "created": 0, "model": "stub-model",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": reply}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            })
            .to_string();
            let mut stream = reader.into_inner();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", json.len(), json).ok();
        }
    });
    format!("http://{}", addr)
}

// Project config drives the provider; with a real provider the spec is
// written by the model (here: a stub that answers prose, so the heuristic
// takes over) and the answer comes from the configured endpoint.
#[test]
fn project_config_drives_the_provider() {
    let env = TestEnv::new();
    let url = spawn_stub_llm("reply from the configured provider");
    env.terra().arg("init").assert().success();
    for (key, value) in [("provider.base_url", url.as_str()), ("provider.model", "stub-model"), ("provider.api_key", "test-key")] {
        env.terra().args(["config", "set", key, value, "--project"]).assert().success();
    }

    let report: serde_json::Value = serde_json::from_slice(&env.terra().args(["doctor", "--as-json"]).output().unwrap().stdout).unwrap();
    assert_eq!(report["provider_configured"], true);
    assert_eq!(report["provider_source"], ".terra/config.toml");
    assert_eq!(report["model"], "stub-model");
    assert_eq!(report["project_initialized"], true);

    let out = env.terra().args(["run", "--task", "say hi in hello.txt", "--as-json"]).output().unwrap();
    let run: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(run["provider"], "stub-model", "dry-run would report dry_run");
    assert!(run["response"].as_str().unwrap().contains("reply from the configured provider"), "{run}");
}

#[test]
fn config_set_rejects_keys_the_runtime_ignores() {
    TestEnv::new()
        .terra()
        .args(["config", "set", "memory.cache_capacity", "5"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Supported: provider.base_url"));
}

#[test]
fn config_set_does_not_echo_api_key() {
    TestEnv::new()
        .terra()
        .args(["config", "set", "provider.api_key", "sk-very-secret"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sk-very-secret").not());
}

#[test]
fn trace_reports_missing_log_then_lists_entries_and_verifies_the_chain() {
    let env = TestEnv::new();
    env.terra().arg("trace").assert().success().stdout(predicate::str::contains("No audit log here yet"));
    env.terra().arg("init").assert().success();
    env.terra().args(["run", "--task", "list files in notes.md"]).assert().success();
    env.terra()
        .arg("trace")
        .assert()
        .success()
        .stdout(predicate::str::contains("entries, showing"))
        .stdout(predicate::str::contains("ProviderCall"))
        .stdout(predicate::str::contains("ClaimComplete"));
    let out = env.terra().args(["trace", "--as-json", "-n", "2"]).output().unwrap();
    let entries: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 2);
    env.terra().args(["trace", "--verify"]).assert().success().stdout(predicate::str::contains("Audit chain intact"));
}

// An audit log edited behind Terra's back is refused, not silently reused.
#[test]
fn tampered_audit_log_is_refused() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    env.terra().args(["run", "--task", "note it in notes.md"]).assert().success();
    let db = env.project.path().join(".terra/audit.db");
    let status = std::process::Command::new("sqlite3")
        .arg(&db)
        .arg("UPDATE audit_events SET payload = replace(payload, '\"ok\"', '\"error\"') WHERE kind = 'result';")
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        eprintln!("sqlite3 not available; skipping");
        return;
    }
    env.terra().args(["trace", "--verify"]).assert().code(1).stderr(predicate::str::contains("hash chain breaks"));
    env.terra().args(["run", "--task", "touch notes.md"]).assert().failure().stderr(predicate::str::contains("append-only"));
}

#[test]
fn piped_output_has_no_color_codes() {
    let env = TestEnv::new();
    let out = env.terra().arg("init").output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains('\x1b'));
    let out = env.terra().arg("chat").write_stdin("/help\nhello\n/quit\n").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains('\x1b'), "escape codes in piped chat output: {text}");
    assert!(!text.contains("Thinking"), "spinner in piped chat output");
    assert!(text.contains("DRY_RUN response to: hello"), "{text}");
}

#[test]
fn global_settings_apply_after_init() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    env.terra().args(["config", "set", "provider.model", "gpt-4o"]).assert().success();
    env.terra().args(["config", "show"]).assert().success().stdout(predicate::str::contains("model = \"gpt-4o\""));
}

#[test]
fn empty_api_key_means_dry_run() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    env.terra().args(["config", "set", "provider.api_key", "", "--project"]).assert().success();
    env.terra().arg("doctor").env("TERRA_PROVIDER_API_KEY", "").assert().success().stdout(predicate::str::contains("dry-run only"));
}

// `terra trace | head -1` used to panic with "failed printing to stdout: Broken pipe".
#[cfg(unix)]
#[test]
fn closed_stdout_does_not_panic() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    env.terra().args(["run", "--task", "fill the audit log for notes.md"]).assert().success();
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("terra"))
        .arg("trace")
        .current_dir(env.project.path())
        .env("HOME", env.home.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{}", stderr);
}
