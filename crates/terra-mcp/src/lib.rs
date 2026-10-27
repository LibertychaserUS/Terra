//! Model Context Protocol, both ways: Terra as a tool server for other
//! clients, and external MCP servers as tools for Terra's own loop.

pub mod client;
pub mod server;

pub use client::{ExternalTool, McpClients, ServerSpec, TOOL_SEPARATOR};
pub use server::{TerraServer, MCP_AGENT};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use rmcp::model::ErrorCode;
    use terra_caging::engine::CagingEngine;
    use terra_caging::sandbox::SandboxGuard;
    use terra_executor::CommandExecutor;
    use terra_memory::MemoryDatabase;
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    fn server() -> (TerraServer, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".terra")).unwrap();
        std::fs::write(root.join(".terra/constitution.toml"), "original").unwrap();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let sandbox = SandboxGuard::new(root.clone(), vec![root.join(".terra/constitution.toml")]);
        let caging = Arc::new(CagingEngine::standard(sandbox, true, true));
        let exec = CommandExecutor::new(memory, Arc::new(AuditLog::new()), caging, Arc::new(DryRunAdapter::new()))
            .with_project_root(root);
        (TerraServer::new(Arc::new(exec)), dir)
    }

    #[test]
    fn tool_list_matches_the_catalog() {
        let names: Vec<String> = TerraServer::tools().iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names, ["FileRead", "FileWrite", "FileDelete", "ShellExec", "MemRead", "MemWrite", "MemSearch"]);
    }

    #[tokio::test]
    async fn memory_roundtrip_through_executor() {
        let (server, _dir) = server();
        server.call("MemWrite", serde_json::json!({"key": "k", "content": "hello", "level": "L2"})).await.unwrap();
        let result = server.call("MemRead", serde_json::json!({"key": "k"})).await.unwrap();
        let text = result.content[0].as_text().unwrap().text.clone();
        assert!(text.contains("hello"));
        assert!(text.contains("mcp_client"));
    }

    #[tokio::test]
    async fn unknown_tool_and_missing_args_are_invalid_params() {
        let (server, _dir) = server();
        let err = server.call("NonExistentTool", serde_json::json!({})).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_PARAMS);
        let err = server.call("FileRead", serde_json::json!({})).await.unwrap_err();
        assert!(err.message.contains("requires 'path'"));
    }

    #[tokio::test]
    async fn sandbox_escapes_through_file_write_are_cage_blocks() {
        let (server, dir) = server();
        let root = dir.path();
        let escape = format!("{}/../terra_mcp_escape_test.txt", root.display());
        let sneaky = format!("{}/.terra/../.terra/constitution.toml", root.display());
        for path in [escape.as_str(), sneaky.as_str()] {
            let err = server.call("FileWrite", serde_json::json!({"path": path, "content": "pwned"})).await.unwrap_err();
            assert_eq!(err.data.unwrap()["cage_block"], true, "{path}");
        }
        assert!(!root.parent().unwrap().join("terra_mcp_escape_test.txt").exists());
        assert_eq!(std::fs::read_to_string(root.join(".terra/constitution.toml")).unwrap(), "original");
    }

    #[tokio::test]
    async fn tool_errors_are_results_not_protocol_errors() {
        let (server, _dir) = server();
        let result = server.call("FileRead", serde_json::json!({"path": "missing.txt"})).await.unwrap();
        assert_eq!(result.is_error, Some(true));
    }
}
