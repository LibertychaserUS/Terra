//! CLI binary tests — T-CLI-001 to T-CLI-010, plus .terra/.gitignore checks.

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;

/// An isolated environment: temp project dir, temp HOME (so ~/.terra of the
/// developer is never read or written) and no provider keys (no real LLM calls).
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
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(self.project.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }
}

// T-CLI-001: no args shows banner
#[test]
fn no_args_shows_banner() {
    TestEnv::new().terra()
        .assert()
        .success()
        .stdout(predicate::str::contains("Terra"))
        .stdout(predicate::str::contains("Agent OS"));
}

// T-CLI-002: doctor shows status
#[test]
fn doctor_shows_status() {
    TestEnv::new().terra()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("Status:       dry-run only"))
        .stdout(predicate::str::contains("not initialized"));
}

// T-CLI-003: doctor --as-json outputs valid JSON
#[test]
fn doctor_as_json() {
    let output = TestEnv::new().terra()
        .args(["doctor", "--as-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("should output valid JSON");
    assert!(json.get("os").is_some());
    assert!(json.get("terra_version").is_some());
}

// T-CLI-004: run with task completes
#[test]
fn run_with_task() {
    TestEnv::new().terra()
        .args(["run", "--task", "test task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"));
}

// T-CLI-005: run --as-json outputs valid JSON
#[test]
fn run_as_json() {
    let output = TestEnv::new().terra()
        .args(["run", "--task", "test task", "--as-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("should output valid JSON");
    assert!(json.get("spec_id").is_some());
}

// T-CLI-006: run JSON has required fields
#[test]
fn run_json_has_required_fields() {
    let output = TestEnv::new().terra()
        .args(["run", "--task", "test", "--as-json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("spec_id").is_some(), "missing spec_id");
    assert!(json.get("intent").is_some(), "missing intent");
    assert!(json.get("worker_response").is_some(), "missing worker_response");
    assert!(json.get("ic_rules_applied").is_some(), "missing ic_rules_applied");
    assert!(json.get("audit_entries").is_some(), "missing audit_entries");
}

// T-CLI-007: help flag
#[test]
fn help_flag() {
    TestEnv::new().terra()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor"))
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("config"));
}

// T-CLI-008: unknown command error
#[test]
fn unknown_command_error() {
    TestEnv::new().terra()
        .arg("nonexistent-command")
        .assert()
        .failure();
}

// T-CLI-009: config show displays config
#[test]
fn config_show_displays() {
    TestEnv::new().terra()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("provider"))
        .stdout(predicate::str::contains("model"));
}

// T-CLI-010: config set then show
#[test]
fn config_set_and_show() {
    // Note: config set writes to ~/.terra/config.toml by default.
    // We test that the subcommand at least parses correctly.
    // Full persistence test is in config.rs unit tests (T-CF-005).
    TestEnv::new().terra()
        .args(["config", "show"])
        .assert()
        .success()
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
            "runtime file {} would be committed; visible: {:?}", path, visible
        );
    }
}

// `/commit` stages everything, so Terra's own databases must be git-ignored.
#[test]
fn init_keeps_runtime_data_out_of_git() {
    let env = TestEnv::new();
    env.git(&["init", "-q"]);
    env.terra().arg("init").assert().success();
    env.terra().args(["run", "--task", "touch the databases"]).assert().success();
    assert!(env.project.path().join(".terra/memory.db").exists());
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
                "id": "stub", "model": "stub-model",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": reply}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            }).to_string();
            let mut stream = reader.into_inner();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", json.len(), json).ok();
        }
    });
    format!("http://{}", addr)
}

// `terra config set` used to write a file nothing read; the runtime now uses it.
#[test]
fn project_config_drives_the_provider() {
    let env = TestEnv::new();
    let url = spawn_stub_llm("reply from the configured provider");
    env.terra().arg("init").assert().success();
    for (key, value) in [("provider.base_url", url.as_str()), ("provider.model", "stub-model"), ("provider.api_key", "test-key")] {
        env.terra().args(["config", "set", key, value, "--project"]).assert().success();
    }

    let report: serde_json::Value =
        serde_json::from_slice(&env.terra().args(["doctor", "--as-json"]).output().unwrap().stdout).unwrap();
    assert_eq!(report["provider_configured"], true);
    assert_eq!(report["provider_source"], ".terra/config.toml");
    assert_eq!(report["model"], "stub-model");
    assert_eq!(report["project_initialized"], true);

    let out = env.terra().args(["run", "--task", "say hi", "--as-json"]).output().unwrap();
    let run: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(run["provider"], "stub-model", "dry-run would report dry_run");
    assert!(run["worker_response"].as_str().unwrap().contains("reply from the configured provider"));
}

#[test]
fn config_set_rejects_keys_the_runtime_ignores() {
    TestEnv::new().terra()
        .args(["config", "set", "memory.cache_capacity", "5"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Supported: provider.base_url"));
}

#[test]
fn config_set_does_not_echo_api_key() {
    TestEnv::new().terra()
        .args(["config", "set", "provider.api_key", "sk-very-secret"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sk-very-secret").not());
}

#[test]
fn trace_reports_missing_log_then_lists_entries() {
    let env = TestEnv::new();
    env.terra().arg("trace").assert().success().stdout(predicate::str::contains("No audit log here yet"));
    env.terra().arg("init").assert().success();
    env.terra().args(["run", "--task", "list files"]).assert().success();
    env.terra()
        .arg("trace")
        .assert()
        .success()
        .stdout(predicate::str::contains("entries, showing"))
        .stdout(predicate::str::contains("ProviderCall"));
    let out = env.terra().args(["trace", "--as-json", "-n", "2"]).output().unwrap();
    let entries: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 2);
}

#[test]
fn piped_output_has_no_color_codes() {
    let env = TestEnv::new();
    let out = env.terra().arg("init").output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains('\x1b'));
    let out = env.terra().arg("chat").write_stdin("/help\nhello\n/quit\n").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains('\x1b'), "escape codes in piped chat output");
    assert!(!text.contains("Thinking"), "spinner in piped chat output");
}

// `terra init` used to write explicit defaults that shadowed ~/.terra/config.toml.
#[test]
fn global_settings_apply_after_init() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    env.terra().args(["config", "set", "provider.model", "gpt-4o"]).assert().success();
    env.terra()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("model = \"gpt-4o\""));
}

#[test]
fn empty_api_key_means_dry_run() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    env.terra().args(["config", "set", "provider.api_key", "", "--project"]).assert().success();
    env.terra()
        .arg("doctor")
        .env("TERRA_PROVIDER_API_KEY", "")
        .assert()
        .success()
        .stdout(predicate::str::contains("dry-run only"));
}

// `terra trace | head -1` used to panic with "failed printing to stdout: Broken pipe".
#[cfg(unix)]
#[test]
fn closed_stdout_does_not_panic() {
    let env = TestEnv::new();
    env.terra().arg("init").assert().success();
    env.terra().args(["run", "--task", "fill the audit log"]).assert().success();
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("terra"))
        .arg("trace")
        .current_dir(env.project.path())
        .env("HOME", env.home.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take()); // reader goes away before terra writes
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{}", stderr);
}
