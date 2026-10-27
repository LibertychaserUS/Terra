use std::path::Path;

use terra_memory::types::{Level, Scope};
use terra_providers::adapter::CompletionRequest;
use terra_types::command::{CommandOp, CommandResult, CommandStatus};

use crate::executor::CommandExecutor;

/// Shell output kept per stream (the rest is dropped, and the result says so).
const MAX_OUTPUT: usize = 1024 * 1024;
const DEFAULT_SHELL_TIMEOUT_MS: u64 = 30_000;

/// Perform the operation. Called after audit and cage checks.
pub async fn route(op: &CommandOp, agent: &str, exec: &CommandExecutor) -> CommandResult {
    let memory = exec.memory();
    match op {
        CommandOp::MemWrite { key, content, scope, level } => {
            let Some(scope) = Scope::parse(scope) else {
                return CommandResult::err("", format!("invalid scope: {}", scope));
            };
            let Some(level) = Level::parse(level) else {
                return CommandResult::err("", format!("invalid level: {}", level));
            };
            match memory.write(key.clone(), scope, level.clone(), agent, content.clone()) {
                Ok(()) => {
                    let mut result = CommandResult::ok("", Some(serde_json::json!({"written": key})));
                    if matches!(level, Level::L5 | Level::L6) {
                        let summary = terra_types::text::truncate_ellipsis(content, 60);
                        result.notification = Some(format!("[Terra] Recorded → {}: \"{}\"", level.as_str(), summary));
                    }
                    result
                }
                Err(e) => CommandResult::err("", e.to_string()),
            }
        }

        CommandOp::MemRead { key } => match memory.read(key) {
            Ok(Some(entry)) => CommandResult::ok(
                "",
                Some(serde_json::json!({
                    "key": entry.key,
                    "content": entry.content,
                    "scope": entry.scope.as_str(),
                    "level": entry.level.as_str(),
                    "owner": entry.owner,
                    "version": entry.version,
                })),
            ),
            Ok(None) => CommandResult {
                command_id: String::new(),
                status: CommandStatus::NotFound,
                data: None,
                error: Some(format!("key not found: {}", key)),
                notification: None,
            },
            Err(e) => CommandResult::err("", e.to_string()),
        },

        CommandOp::MemSearch { query, top_k } => match memory.search(query, *top_k as usize) {
            Ok(hits) => {
                let results: Vec<serde_json::Value> = hits
                    .iter()
                    .map(|h| serde_json::json!({"key": h.key, "score": h.score, "snippet": h.snippet}))
                    .collect();
                CommandResult::ok("", Some(serde_json::json!({"query": query, "count": results.len(), "results": results})))
            }
            Err(e) => CommandResult::err("", e.to_string()),
        },

        CommandOp::TraceEvent { event_type, .. } => {
            CommandResult::ok("", Some(serde_json::json!({"event": event_type, "recorded": true})))
        }

        // Gates: reaching the router means every cage let them through.
        CommandOp::ExecuteTask { spec_id, .. } => {
            CommandResult::ok("", Some(serde_json::json!({"spec_id": spec_id, "allowed": true})))
        }
        CommandOp::ClaimComplete { goal, .. } => {
            CommandResult::ok("", Some(serde_json::json!({"goal": goal, "allowed": true})))
        }

        CommandOp::ProviderCall { system_prompt, user_prompt, max_tokens, temperature, json } => {
            let mut request = CompletionRequest::simple(system_prompt.clone(), user_prompt.clone());
            request.max_tokens = *max_tokens;
            request.temperature = *temperature;
            request.json = *json;
            match exec.provider().complete(request).await {
                Ok(response) => CommandResult::ok(
                    "",
                    Some(serde_json::json!({
                        "content": response.content,
                        "model": response.model,
                        "prompt_tokens": response.usage_prompt_tokens,
                        "completion_tokens": response.usage_completion_tokens,
                    })),
                ),
                Err(e) => CommandResult::err("", format!("{e:#}")),
            }
        }

        CommandOp::FileRead { path } => {
            let full = exec.absolute(path);
            match std::fs::read_to_string(&full) {
                Ok(content) => {
                    let bytes = content.len();
                    CommandResult::ok("", Some(serde_json::json!({"path": path, "content": content, "bytes": bytes})))
                }
                Err(e) => CommandResult::err("", format!("FileRead {}: {}", path, e)),
            }
        }

        CommandOp::FileWrite { path, content } => {
            let full = exec.absolute(path);
            if let Some(parent) = full.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return CommandResult::err("", format!("mkdir {}: {}", parent.display(), e));
                }
            }
            match std::fs::write(&full, content) {
                Ok(()) => CommandResult::ok("", Some(serde_json::json!({"path": path, "bytes_written": content.len()}))),
                Err(e) => CommandResult::err("", format!("FileWrite {}: {}", path, e)),
            }
        }

        CommandOp::FileDelete { path } => match std::fs::remove_file(exec.absolute(path)) {
            Ok(()) => CommandResult::ok("", Some(serde_json::json!({"path": path, "deleted": true}))),
            Err(e) => CommandResult::err("", format!("FileDelete {}: {}", path, e)),
        },

        CommandOp::ShellExec { command, workdir, timeout_ms } => {
            let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_SHELL_TIMEOUT_MS));
            let workdir = workdir.as_deref().map(|w| exec.absolute(w)).unwrap_or_else(|| exec.project_root().to_path_buf());
            let isolation = exec.isolation();
            let mut child_cmd = isolation.wrap(command, &workdir);
            child_cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            let child = match child_cmd.kill_on_drop(true).spawn() {
                Ok(c) => c,
                Err(e) => return CommandResult::err("", format!("ShellExec failed to start ({}): {}", isolation.name(), e)),
            };
            match tokio::time::timeout(timeout, child.wait_with_output()).await {
                Ok(Ok(output)) => {
                    let (stdout, out_cut) = cap(&output.stdout);
                    let (stderr, err_cut) = cap(&output.stderr);
                    let exit_code = output.status.code().unwrap_or(-1);
                    let data = serde_json::json!({
                        "command": command,
                        "workdir": relative_display(&workdir, exec.project_root()),
                        "isolation": isolation.name(),
                        "exit_code": exit_code,
                        "stdout": stdout,
                        "stderr": stderr,
                    });
                    if output.status.success() {
                        let mut result = CommandResult::ok("", Some(data));
                        if out_cut || err_cut {
                            result.notification = Some(format!("[Terra] Output truncated at {} bytes per stream", MAX_OUTPUT));
                        }
                        result
                    } else {
                        let mut result = CommandResult::err(
                            "",
                            format!("ShellExec exit {}: {}", exit_code, if stderr.is_empty() { &stdout } else { &stderr }),
                        );
                        result.data = Some(data);
                        result
                    }
                }
                Ok(Err(e)) => CommandResult::err("", format!("ShellExec failed: {}", e)),
                Err(_) => CommandResult::err("", format!("ShellExec timed out after {}ms: {}", timeout.as_millis(), command)),
            }
        }
    }
}

fn cap(bytes: &[u8]) -> (String, bool) {
    if bytes.len() > MAX_OUTPUT {
        (String::from_utf8_lossy(&bytes[..MAX_OUTPUT]).into_owned(), true)
    } else {
        (String::from_utf8_lossy(bytes).into_owned(), false)
    }
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).map(|p| if p.as_os_str().is_empty() { ".".into() } else { p.display().to_string() }).unwrap_or_else(|_| path.display().to_string())
}
