use std::sync::Arc;

use terra_memory::MemoryDatabase;
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
    provider: &Arc<dyn ProviderAdapter>,
) -> CommandResult {
    match op {
        CommandOp::MemWrite { key, content, scope, level } => {
            let scope = match Scope::from_str(scope) {
                Some(s) => s,
                None => return CommandResult::err("", format!("invalid scope: {}", scope)),
            };
            let level = match Level::from_str(level) {
                Some(l) => l,
                None => return CommandResult::err("", format!("invalid level: {}", level)),
            };

            match memory.write(key.clone(), scope, level, agent, content.as_bytes().to_vec()) {
                Ok(()) => CommandResult::ok("", Some(serde_json::json!({"written": key}))),
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
                },
                Err(e) => CommandResult::err("", e.to_string()),
            }
        }

        CommandOp::MemSearch { query, top_k } => {
            CommandResult::ok("", Some(serde_json::json!({
                "query": query,
                "top_k": top_k,
                "results": [],
                "note": "vector search not yet implemented"
            })))
        }

        CommandOp::ArtifactSave { artifact_type, content: _ } => {
            CommandResult::ok("", Some(serde_json::json!({
                "type": artifact_type,
                "status": "saved",
                "note": "artifact store pending"
            })))
        }

        CommandOp::ArtifactLoad { artifact_type, version } => {
            CommandResult::ok("", Some(serde_json::json!({
                "type": artifact_type,
                "version": version,
                "note": "artifact store pending"
            })))
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

        CommandOp::ProviderCall { system_prompt, user_prompt, max_tokens, temperature } => {
            let request = CompletionRequest {
                system_prompt: system_prompt.clone(),
                user_prompt: user_prompt.clone(),
                max_tokens: *max_tokens,
                temperature: *temperature,
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

    #[tokio::test]
    async fn route_mem_write() {
        let mem = test_memory();
        let prov = test_provider();
        let result = route(
            &CommandOp::MemWrite {
                key: "test_key".into(),
                content: "test content".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
            "main",
            &mem,
            &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn route_mem_read_after_write() {
        let mem = test_memory();
        let prov = test_provider();
        route(
            &CommandOp::MemWrite {
                key: "k1".into(),
                content: "data".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
            "main",
            &mem,
            &prov,
        ).await;

        let result = route(&CommandOp::MemRead { key: "k1".into() }, "main", &mem, &prov).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.data.unwrap()["key"], "k1");
    }

    #[tokio::test]
    async fn route_mem_read_not_found() {
        let mem = test_memory();
        let prov = test_provider();
        let result = route(&CommandOp::MemRead { key: "nope".into() }, "main", &mem, &prov).await;
        assert_eq!(result.status, CommandStatus::NotFound);
    }

    #[tokio::test]
    async fn route_invalid_scope() {
        let mem = test_memory();
        let prov = test_provider();
        let result = route(
            &CommandOp::MemWrite {
                key: "k".into(),
                content: "c".into(),
                scope: "INVALID".into(),
                level: "L1".into(),
            },
            "main",
            &mem,
            &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Error);
    }

    #[tokio::test]
    async fn route_trace_checkpoint() {
        let mem = test_memory();
        let prov = test_provider();
        let result = route(
            &CommandOp::TraceCheckpoint {
                title: "test".into(),
                summary: "testing".into(),
            },
            "main",
            &mem,
            &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn route_provider_call_dry_run() {
        let mem = test_memory();
        let prov = test_provider();
        let result = route(
            &CommandOp::ProviderCall {
                system_prompt: "You are helpful.".into(),
                user_prompt: "What is 2+2?".into(),
                max_tokens: Some(50),
                temperature: None,
            },
            "main",
            &mem,
            &prov,
        ).await;
        assert_eq!(result.status, CommandStatus::Ok);
        let data = result.data.unwrap();
        assert!(data["content"].as_str().unwrap().contains("DRY_RUN"));
        assert_eq!(data["prompt_tokens"], 0);
    }
}
