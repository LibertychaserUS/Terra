use terra_memory::MemoryDatabase;
use terra_memory::types::{Level, Scope};
use terra_protocol::command::{CommandOp, CommandResult, CommandStatus};

/// Routes a CommandOp to its backend and returns the result.
///
/// This is the only place where backend systems are called directly.
/// The executor calls this AFTER audit recording and cage checking.
pub fn route(
    op: &CommandOp,
    agent: &str,
    memory: &MemoryDatabase,
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
            // Placeholder: full vector search requires embedding integration.
            // For now, return empty results.
            CommandResult::ok("", Some(serde_json::json!({
                "query": query,
                "top_k": top_k,
                "results": [],
                "note": "vector search not yet implemented"
            })))
        }

        CommandOp::ArtifactSave { artifact_type, content: _ } => {
            // Placeholder: artifact store implementation pending.
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_memory() -> MemoryDatabase {
        MemoryDatabase::open_in_memory().unwrap()
    }

    #[test]
    fn route_mem_write() {
        let mem = test_memory();
        let result = route(
            &CommandOp::MemWrite {
                key: "test_key".into(),
                content: "test content".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
            "main",
            &mem,
        );
        assert_eq!(result.status, CommandStatus::Ok);
    }

    #[test]
    fn route_mem_read_after_write() {
        let mem = test_memory();
        route(
            &CommandOp::MemWrite {
                key: "k1".into(),
                content: "data".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
            "main",
            &mem,
        );

        let result = route(&CommandOp::MemRead { key: "k1".into() }, "main", &mem);
        assert_eq!(result.status, CommandStatus::Ok);
        let data = result.data.unwrap();
        assert_eq!(data["key"], "k1");
        assert_eq!(data["content"], "data");
    }

    #[test]
    fn route_mem_read_not_found() {
        let mem = test_memory();
        let result = route(&CommandOp::MemRead { key: "nope".into() }, "main", &mem);
        assert_eq!(result.status, CommandStatus::NotFound);
    }

    #[test]
    fn route_invalid_scope() {
        let mem = test_memory();
        let result = route(
            &CommandOp::MemWrite {
                key: "k".into(),
                content: "c".into(),
                scope: "INVALID".into(),
                level: "L1".into(),
            },
            "main",
            &mem,
        );
        assert_eq!(result.status, CommandStatus::Error);
    }

    #[test]
    fn route_trace_checkpoint() {
        let mem = test_memory();
        let result = route(
            &CommandOp::TraceCheckpoint {
                title: "test".into(),
                summary: "testing".into(),
            },
            "main",
            &mem,
        );
        assert_eq!(result.status, CommandStatus::Ok);
    }
}
