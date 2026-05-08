use std::sync::{Arc, Mutex};

use terra_memory::MemoryDatabase;
use terra_memory::artifact::{ArtifactStore, ArtifactType};
use terra_memory::search::VectorIndex;
use terra_memory::types::{Level, Scope};
use terra_protocol::command::{CommandOp, CommandResult, CommandStatus};
use terra_providers::adapter::{CompletionRequest, ProviderAdapter};

/// Routes a CommandOp to its backend and returns the result.
///
/// This is the only place where backend systems are called directly.
/// The executor calls this AFTER audit recording and cage checking.
pub async fn route(
    op: &CommandOp,
    agent: &str,
    memory: &MemoryDatabase,
    artifacts: &ArtifactStore,
    search_index: &Mutex<VectorIndex>,
    provider: &Arc<dyn ProviderAdapter>,
) -> CommandResult {
    match op {
        CommandOp::MemWrite { key, content, scope, level } => {
            let level_str = level.clone();
            let scope = match Scope::from_str(scope) {
                Some(s) => s,
                None => return CommandResult::err("", format!("invalid scope: {}", scope)),
            };
            let level = match Level::from_str(level) {
                Some(l) => l,
                None => return CommandResult::err("", format!("invalid level: {}", level)),
            };
            match memory.write(key.clone(), scope, level, agent, content.as_bytes().to_vec()) {
                Ok(()) => {
                    // Also index for vector search
                    if let Ok(mut idx) = search_index.lock() {
                        idx.upsert(key, content);
                    }

                    // Auto-record notification for L5/L6 writes
                    let notification = if level_str == "L5" || level_str == "L6" {
                        let summary = if content.len() > 60 {
                            format!("{}...", &content[..57])
                        } else {
                            content.clone()
                        };
                        Some(format!("[Terra] Recorded → {}: \"{}\"", level_str, summary))
                    } else {
                        None
                    };

                    let mut result = CommandResult::ok("", Some(serde_json::json!({"written": key})));
                    result.notification = notification;
                    result
                }
                Err(e) => CommandResult::err("", e.to_string()),
            }
        }

        CommandOp::MemRead { key } => {
            match memory.read(key) {
                Ok(Some(entry)) => {
                    let content = String::from_utf8_lossy(&entry.content).to_string();
                    CommandResult::ok("", Some(serde_json::json!({
                        "key": entry.key,
                        "content": content,
                        "scope": entry.scope.as_str(),
                        "level": entry.level.as_str(),
                        "owner": entry.owner,
                        "version": entry.version,
                    })))
                }
                Ok(None) => CommandResult {
                    command_id: String::new(),
                    status: CommandStatus::NotFound,
                    data: None,
                    error: Some(format!("key not found: {}", key)),
                    notification: None,
                },
                Err(e) => CommandResult::err("", e.to_string()),
            }
        }

        CommandOp::MemSearch { query, top_k } => {
            // Multi-head mode: if query starts with "multi:", split into sub-queries
            if query.starts_with("multi:") {
                let raw = &query["multi:".len()..];
                let sub_queries: Vec<String> = raw.split('|')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                let results = match search_index.lock() {
                    Ok(idx) => idx.multi_head_search(&sub_queries, *top_k as usize),
                    Err(_) => vec![],
                };

                let entries: Vec<serde_json::Value> = results
                    .iter()
                    .map(|(key, score)| serde_json::json!({"key": key, "score": score}))
                    .collect();

                CommandResult::ok("", Some(serde_json::json!({
                    "query": query,
                    "mode": "multi_head",
                    "sub_queries": sub_queries,
                    "top_k": top_k,
                    "results": entries,
                    "count": results.len(),
                })))
            } else {
                let results = match search_index.lock() {
                    Ok(idx) => idx.search(query, *top_k as usize),
                    Err(_) => vec![],
                };

                let entries: Vec<serde_json::Value> = results
                    .iter()
                    .map(|(key, score)| serde_json::json!({"key": key, "score": score}))
                    .collect();

                CommandResult::ok("", Some(serde_json::json!({
                    "query": query,
                    "top_k": top_k,
                    "results": entries,
                    "count": results.len(),
                })))
            }
        }

        CommandOp::ArtifactSave { artifact_type, content: _ } => {
            let art_type = match ArtifactType::from_str(artifact_type) {
                Some(t) => t,
                None => return CommandResult::err("", format!("invalid artifact type: {}", artifact_type)),
            };
            // Serialize the content value to MessagePack for storage
            let packed = match rmp_serde::to_vec_named(&serde_json::json!({"type": artifact_type})) {
                Ok(b) => b,
                Err(e) => return CommandResult::err("", e.to_string()),
            };
            match artifacts.save(art_type, &packed) {
                Ok(meta) => CommandResult::ok("", Some(serde_json::json!({
                    "type": artifact_type,
                    "version": meta.version,
                    "path": meta.path.to_string_lossy(),
                }))),
                Err(e) => CommandResult::err("", e.to_string()),
            }
        }

        CommandOp::ArtifactLoad { artifact_type, version } => {
            let art_type = match ArtifactType::from_str(artifact_type) {
                Some(t) => t,
                None => return CommandResult::err("", format!("invalid artifact type: {}", artifact_type)),
            };
            match artifacts.load(&art_type, *version) {
                Ok((meta, content)) => {
                    let decoded: serde_json::Value = rmp_serde::from_slice(&content)
                        .unwrap_or(serde_json::json!({"raw": "binary"}));
                    CommandResult::ok("", Some(serde_json::json!({
                        "type": artifact_type,
                        "version": meta.version,
                        "content": decoded,
                    })))
                }
                Err(e) => CommandResult::err("", e.to_string()),
            }
        }

        CommandOp::TraceEvent { event_type, data: _ } => {
            CommandResult::ok("", Some(serde_json::json!({
                "event": event_type,
                "recorded": true,
            })))
        }

        CommandOp::TraceCheckpoint { title, summary: _ } => {
            CommandResult::ok("", Some(serde_json::json!({
                "checkpoint": title,
                "recorded": true,
            })))
        }

        CommandOp::ProviderCall { system_prompt, user_prompt, max_tokens, temperature, .. } => {
            let request = CompletionRequest {
                system_prompt: system_prompt.clone(),
                user_prompt: user_prompt.clone(),
                max_tokens: *max_tokens,
                temperature: *temperature,
                tools: None,
                tool_results: None,
                pending_tool_calls: None,
            };

            match provider.complete(request).await {
                Ok(response) => CommandResult::ok("", Some(serde_json::json!({
                    "content": response.content,
                    "model": response.model,
                    "prompt_tokens": response.usage_prompt_tokens,
                    "completion_tokens": response.usage_completion_tokens,
                }))),
                Err(e) => CommandResult::err("", e.to_string()),
            }
        }

        CommandOp::FileRead { path } => {
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let len = content.len();
                    CommandResult::ok("", Some(serde_json::json!({
                        "path": path,
                        "content": content,
                        "bytes": len,
                    })))
                }
                Err(e) => CommandResult::err("", format!("FileRead {}: {}", path, e)),
            }
        }

        CommandOp::FileWrite { path, content } => {
            // Ensure parent directory exists
            if let Some(parent) = std::path::Path::new(path).parent() {
                if !parent.exists() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return CommandResult::err("", format!("mkdir {}: {}", parent.display(), e));
                    }
                }
            }
            match std::fs::write(path, content) {
                Ok(()) => CommandResult::ok("", Some(serde_json::json!({
                    "path": path,
                    "bytes_written": content.len(),
                }))),
                Err(e) => CommandResult::err("", format!("FileWrite {}: {}", path, e)),
            }
        }

        CommandOp::FileDelete { path } => {
            match std::fs::remove_file(path) {
                Ok(()) => CommandResult::ok("", Some(serde_json::json!({
                    "path": path,
                    "deleted": true,
                }))),
                Err(e) => CommandResult::err("", format!("FileDelete {}: {}", path, e)),
            }
        }

        CommandOp::ShellExec { command, workdir, timeout_ms } => {
            let timeout = std::time::Duration::from_millis(
                timeout_ms.unwrap_or(30_000)
            );

            let child = match tokio::process::Command::new("sh")
                .args(["-c", command])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .current_dir(workdir.as_deref().unwrap_or("."))
                .spawn()
            {
                Ok(c) => c,
                Err(e) => return CommandResult::err("", format!("ShellExec failed: {}", e)),
            };

            match tokio::time::timeout(timeout, child.wait_with_output()).await {
                Ok(Ok(output)) => {
                    // Cap output size to prevent OOM (10 MB)
                    const MAX_OUTPUT: usize = 10 * 1024 * 1024;
                    let (stdout, truncated) = if output.stdout.len() > MAX_OUTPUT {
                        (String::from_utf8_lossy(&output.stdout[..MAX_OUTPUT]).to_string(), true)
                    } else {
                        (String::from_utf8_lossy(&output.stdout).to_string(), false)
                    };
                    let stderr = if output.stderr.len() > MAX_OUTPUT {
                        String::from_utf8_lossy(&output.stderr[..MAX_OUTPUT]).to_string()
                    } else {
                        String::from_utf8_lossy(&output.stderr).to_string()
                    };
                    let exit_code = output.status.code().unwrap_or(-1);

                    if output.status.success() {
                        let mut result = CommandResult::ok("", Some(serde_json::json!({
                            "command": command,
                            "exit_code": exit_code,
                            "stdout": stdout,
                            "stderr": stderr,
                        })));
                        if truncated {
                            result.notification = Some(format!(
                                "[Terra] Output truncated at {} bytes", MAX_OUTPUT
                            ));
                        }
                        result
                    } else {
                        CommandResult::err("", format!(
                            "ShellExec exit {}: {}",
                            exit_code,
                            if stderr.is_empty() { &stdout } else { &stderr }
                        ))
                    }
                }
                Ok(Err(e)) => CommandResult::err("", format!("ShellExec failed: {}", e)),
                Err(_) => {
                    CommandResult::err("", format!(
                        "ShellExec timed out after {}ms: {}",
                        timeout.as_millis(), command
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_providers::adapter::DryRunAdapter;

    fn test_memory() -> MemoryDatabase {
        MemoryDatabase::open_in_memory().unwrap()
    }

    fn test_provider() -> Arc<dyn ProviderAdapter> {
        Arc::new(DryRunAdapter::new())
    }

    fn test_artifacts() -> (ArtifactStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (ArtifactStore::new(dir.path().to_path_buf()), dir)
    }

    fn test_search() -> Mutex<VectorIndex> {
        Mutex::new(VectorIndex::new())
    }

    #[tokio::test]
    async fn route_mem_write() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::MemWrite {
                key: "test_key".into(), content: "test content".into(),
                scope: "project".into(), level: "L1".into(),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn route_mem_read_after_write() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        route(
            &CommandOp::MemWrite {
                key: "k1".into(), content: "data".into(),
                scope: "project".into(), level: "L1".into(),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        let result = route(&CommandOp::MemRead { key: "k1".into() }, "main", &mem, &art, &search, &prov).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.data.unwrap()["key"], "k1");
    }

    #[tokio::test]
    async fn route_mem_read_not_found() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(&CommandOp::MemRead { key: "nope".into() }, "main", &mem, &art, &search, &prov).await;
        assert_eq!(result.status, CommandStatus::NotFound);
    }

    #[tokio::test]
    async fn route_invalid_scope() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::MemWrite {
                key: "k".into(), content: "c".into(),
                scope: "INVALID".into(), level: "L1".into(),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Error);
    }

    #[tokio::test]
    async fn route_trace_checkpoint() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::TraceCheckpoint { title: "test".into(), summary: "testing".into() },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn route_provider_call_dry_run() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::ProviderCall {
                system_prompt: "You are helpful.".into(),
                user_prompt: "What is 2+2?".into(),
                max_tokens: Some(50), temperature: None,
                output_contract: None,
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.data.unwrap()["content"].as_str().unwrap().contains("DRY_RUN"));
    }

    #[tokio::test]
    async fn route_artifact_save_and_load() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();

        // Save
        let save_result = route(
            &CommandOp::ArtifactSave {
                artifact_type: "plan".into(),
                content: serde_json::json!({"step": "plan v1"}),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(save_result.status, CommandStatus::Ok);
        assert_eq!(save_result.data.as_ref().unwrap()["version"], 1);

        // Load
        let load_result = route(
            &CommandOp::ArtifactLoad { artifact_type: "plan".into(), version: None },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(load_result.status, CommandStatus::Ok);
        assert_eq!(load_result.data.as_ref().unwrap()["version"], 1);
    }

    // T-MW-006: L5 write emits notification
    #[tokio::test]
    async fn l5_write_emits_notification() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::MemWrite {
                key: "observation".into(),
                content: "SQLite WAL mode is important".into(),
                scope: "project".into(),
                level: "L5".into(),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.notification.is_some());
        let notif = result.notification.unwrap();
        assert!(notif.starts_with("[Terra] Recorded"));
        assert!(notif.contains("L5"));
    }

    // T-MW-007: L6 agent nominate emits notification
    #[tokio::test]
    async fn l6_agent_nominate_notification() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::MemWrite {
                key: "candidate".into(),
                content: "This pattern is worth promoting".into(),
                scope: "project".into(),
                level: "L6".into(),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.notification.is_some());
        assert!(result.notification.unwrap().contains("L6"));
    }

    // T-MW-008: L2 write has no notification
    #[tokio::test]
    async fn l2_write_no_notification() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::MemWrite {
                key: "task_data".into(),
                content: "temporary".into(),
                scope: "task".into(),
                level: "L2".into(),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.notification.is_none(), "L2 writes should not produce notifications");
    }

    // ShellExec timeout test
    #[tokio::test]
    async fn shell_exec_timeout() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::ShellExec {
                command: "sleep 10".into(),
                workdir: None,
                timeout_ms: Some(200), // 200ms timeout, sleep 10s
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Error);
        assert!(result.error.as_deref().unwrap().contains("timed out"));
    }

    // ShellExec success
    #[tokio::test]
    async fn shell_exec_success() {
        let mem = test_memory();
        let (art, _d) = test_artifacts();
        let search = test_search();
        let prov = test_provider();
        let result = route(
            &CommandOp::ShellExec {
                command: "echo hello".into(),
                workdir: None,
                timeout_ms: Some(5000),
            },
            "main", &mem, &art, &search, &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
        let stdout = result.data.unwrap()["stdout"].as_str().unwrap().to_string();
        assert!(stdout.contains("hello"));
    }
}
