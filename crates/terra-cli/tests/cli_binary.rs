//! CLI binary tests — T-CLI-001 to T-CLI-008.
//!
//! These tests run the `terra` binary as a subprocess and
//! verify its stdout, stderr, and exit code.

use assert_cmd::Command;
use predicates::prelude::*;

fn terra() -> Command {
    Command::cargo_bin("terra").unwrap()
}

// T-CLI-001: no args shows banner
#[test]
fn no_args_shows_banner() {
    terra()
        .assert()
        .success()
        .stdout(predicate::str::contains("Terra"))
        .stdout(predicate::str::contains("Agent OS"));
}

// T-CLI-002: doctor shows status
#[test]
fn doctor_shows_status() {
    terra()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("OK"));
}

// T-CLI-003: doctor --as-json outputs valid JSON
#[test]
fn doctor_as_json() {
    let output = terra()
        .args(["doctor", "--as-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor --as-json should output valid JSON");
    assert!(json.get("os").is_some());
    assert!(json.get("terra_version").is_some());
}

// T-CLI-004: run with task completes
#[test]
fn run_with_task() {
    terra()
        .args(["run", "--task", "test task", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete"));
}

// T-CLI-005: run --as-json outputs valid JSON
#[test]
fn run_as_json() {
    let output = terra()
        .args(["run", "--task", "test task", "--dry-run", "--as-json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("run --as-json should output valid JSON");
    assert!(json.get("spec_id").is_some());
}

// T-CLI-006: run JSON has required fields
#[test]
fn run_json_has_required_fields() {
    let output = terra()
        .args(["run", "--task", "test", "--dry-run", "--as-json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("spec_id").is_some(), "missing spec_id");
    assert!(json.get("intent").is_some(), "missing intent");
    assert!(json.get("worker_response").is_some(), "missing worker_response");
    assert!(json.get("dispatch_mode").is_some(), "missing dispatch_mode");
    assert!(json.get("prompt_tokens").is_some(), "missing prompt_tokens");
    assert!(json.get("completion_tokens").is_some(), "missing completion_tokens");
    assert!(json.get("audit_entries").is_some(), "missing audit_entries");
}

// T-CLI-007: help flag
#[test]
fn help_flag() {
    terra()
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
    terra()
        .arg("nonexistent-command")
        .assert()
        .failure();
}
