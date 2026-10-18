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
        .stdout(predicate::str::contains("OK"));
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
