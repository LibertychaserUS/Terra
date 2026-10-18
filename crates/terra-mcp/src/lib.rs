//! MCP (Model Context Protocol) Server for Terra Agent OS.
//!
//! Exposes Terra's syscall interface as MCP tools,
//! enabling Claude Code, Cursor, VS Code, and other MCP clients
//! to invoke Terra capabilities.
//!
//! Transport: JSON-RPC 2.0 over stdio (stdin/stdout).
//! Logging goes to stderr so it never contaminates the protocol stream.

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

use terra_executor::CommandExecutor;
use terra_types::command::{Command, CommandOp, CommandStatus};

// ─── JSON-RPC 2.0 Structures ────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// ─── MCP Server ──────────────────────────────────────────────────

/// MCP Server running over stdio transport.
///
/// All tool calls are routed through CommandExecutor, which enforces
/// the full audit -> cage -> execute -> record pipeline.
pub struct McpServer {
    executor: Arc<CommandExecutor>,
}

impl McpServer {
    /// Create a new MCP server backed by the given CommandExecutor.
    pub fn new(executor: Arc<CommandExecutor>) -> Self {
        Self { executor }
    }

    /// Dispatch a single JSON-RPC request and return the response.
    ///
    /// This is the core routing method. Tests call it directly;
    /// `run_stdio` wraps it in a line-oriented IO loop.
    pub async fn dispatch(&self, request: &JsonRpcRequest) -> Option<JsonRpcResponse> {
        match request.method.as_str() {
            "initialize" => Some(self.handle_initialize(request)),
            "notifications/initialized" => {
                // Fire-and-forget notification. No response.
                None
            }
            "tools/list" => Some(self.handle_tools_list(request)),
            "tools/call" => Some(self.handle_tools_call(request).await),
            "resources/list" => Some(self.handle_resources_list(request)),
            _ => Some(self.method_not_found(request)),
        }
    }

    /// Run the MCP server over stdio. Reads JSON-RPC requests line by line
    /// from stdin and writes responses to stdout. Logging goes to stderr.
    pub async fn run_stdio(&self) {
        eprintln!("[terra-mcp] server starting on stdio");
        let stdin = io::stdin();
        let stdout = io::stdout();

        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[terra-mcp] stdin read error: {}", e);
                    break;
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let request: JsonRpcRequest = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    let err_resp = JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id: Value::Null,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32700,
                            message: format!("Parse error: {}", e),
                            data: None,
                        }),
                    };
                    let mut out = stdout.lock();
                    let _ = serde_json::to_writer(&mut out, &err_resp);
                    let _ = out.write_all(b"\n");
                    let _ = out.flush();
                    continue;
                }
            };

            if let Some(response) = self.dispatch(&request).await {
                let mut out = stdout.lock();
                let _ = serde_json::to_writer(&mut out, &response);
                let _ = out.write_all(b"\n");
                let _ = out.flush();
            }
        }

        eprintln!("[terra-mcp] server shutting down");
    }

    // ─── Method Handlers ─────────────────────────────────────────

    fn handle_initialize(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        info!("MCP initialize");
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone().unwrap_or(Value::Null),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {}
                },
                "serverInfo": {
                    "name": "terra",
                    "version": "0.1.0"
                }
            })),
            error: None,
        }
    }

    fn handle_tools_list(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let tools = self.build_tool_list();
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone().unwrap_or(Value::Null),
            result: Some(json!({ "tools": tools })),
            error: None,
        }
    }

    async fn handle_tools_call(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);

        let tool_name = request.params.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("");
        let arguments = request.params.get("arguments")
            .cloned()
            .unwrap_or(json!({}));

        info!(tool = %tool_name, "MCP tools/call");

        // Map tool name + arguments -> CommandOp
        let op = match self.parse_tool_call(tool_name, &arguments) {
            Ok(op) => op,
            Err(msg) => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32602,
                        message: msg,
                        data: None,
                    }),
                };
            }
        };

        // Execute through the full pipeline (audit -> cage -> execute -> record)
        let cmd = Command::new("mcp_client", op);
        let result = self.executor.execute(cmd).await;

        match result.status {
            CommandStatus::Ok => {
                let content = result.data
                    .map(|d| serde_json::to_string_pretty(&d).unwrap_or_default())
                    .unwrap_or_else(|| "ok".into());
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: Some(json!({
                        "content": [{
                            "type": "text",
                            "text": content
                        }]
                    })),
                    error: None,
                }
            }
            CommandStatus::PermissionDenied => {
                let reason = result.error.unwrap_or_else(|| "permission denied".into());
                warn!(tool = %tool_name, reason = %reason, "cage blocked MCP tool call");
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32000,
                        message: reason,
                        data: Some(json!({ "cage_block": true })),
                    }),
                }
            }
            CommandStatus::NotFound => {
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32601,
                        message: result.error.unwrap_or_else(|| "not found".into()),
                        data: None,
                    }),
                }
            }
            CommandStatus::Error => {
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: result.error.unwrap_or_else(|| "internal error".into()),
                        data: None,
                    }),
                }
            }
        }
    }

    fn handle_resources_list(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        // Placeholder for future file tree + memory index resources
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone().unwrap_or(Value::Null),
            result: Some(json!({ "resources": [] })),
            error: None,
        }
    }

    fn method_not_found(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: request.id.clone().unwrap_or(Value::Null),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("method not found: {}", request.method),
                data: None,
            }),
        }
    }

    // ─── Tool List ───────────────────────────────────────────────

    /// Build the full list of MCP tools (10 builtins + 2 trace + PointerQuery = 13).
    fn build_tool_list(&self) -> Vec<Value> {
        vec![
            // File operations
            json!({
                "name": "FileRead",
                "description": "Read the contents of a file at the given path",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to read" }
                    },
                    "required": ["path"]
                }
            }),
            json!({
                "name": "FileWrite",
                "description": "Write content to a file at the given path",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to write" },
                        "content": { "type": "string", "description": "Content to write" }
                    },
                    "required": ["path", "content"]
                }
            }),
            json!({
                "name": "FileDelete",
                "description": "Delete a file at the given path (destructive)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path to delete" }
                    },
                    "required": ["path"]
                }
            }),
            // Shell
            json!({
                "name": "ShellExec",
                "description": "Execute a shell command (requires user confirmation)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to execute" },
                        "workdir": { "type": "string", "description": "Working directory (optional)" }
                    },
                    "required": ["command"]
                }
            }),
            // Memory
            json!({
                "name": "MemRead",
                "description": "Read a memory entry by key",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string", "description": "Memory key to read" }
                    },
                    "required": ["key"]
                }
            }),
            json!({
                "name": "MemWrite",
                "description": "Write content to project memory at a specified level",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string", "description": "Memory key" },
                        "content": { "type": "string", "description": "Content to store" },
                        "level": { "type": "string", "enum": ["L1","L2","L3","L4","L5","L6"], "description": "Memory level" }
                    },
                    "required": ["key", "content", "level"]
                }
            }),
            json!({
                "name": "MemSearch",
                "description": "Search project memory by query, returns top matches",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "top_k": { "type": "integer", "description": "Max results (default 5)" }
                    },
                    "required": ["query"]
                }
            }),
            // Artifacts
            json!({
                "name": "ArtifactSave",
                "description": "Save a versioned artifact (plan, spec, test, review)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "artifact_type": { "type": "string", "enum": ["plan","spec","test","review"], "description": "Artifact type" },
                        "content": { "type": "object", "description": "Artifact content (JSON)" }
                    },
                    "required": ["artifact_type", "content"]
                }
            }),
            json!({
                "name": "ArtifactLoad",
                "description": "Load a versioned artifact by type and optional version",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "artifact_type": { "type": "string", "enum": ["plan","spec","test","review"], "description": "Artifact type" },
                        "version": { "type": "integer", "description": "Version number (optional, latest if omitted)" }
                    },
                    "required": ["artifact_type"]
                }
            }),
            // Trace (MCP-exposed, not in ToolRegistry builtins)
            json!({
                "name": "TraceEvent",
                "description": "Emit a trace event for observability",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "event_type": { "type": "string", "description": "Event type identifier" },
                        "data": { "type": "object", "description": "Event payload (JSON)" }
                    },
                    "required": ["event_type", "data"]
                }
            }),
            json!({
                "name": "TraceCheckpoint",
                "description": "Record a named checkpoint in the trace log",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Checkpoint title" },
                        "summary": { "type": "string", "description": "Checkpoint summary" }
                    },
                    "required": ["title", "summary"]
                }
            }),
            // Pointer DSL
            json!({
                "name": "PointerQuery",
                "description": "Query the pointer memory index using DSL syntax (@mem[\"key\"], @L1[*], @search(\"query\", N), @top(N))",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "dsl": { "type": "string", "description": "DSL query (e.g. @mem[\"auth\"], @L1[*], @search(\"bug\", 5), @top(10))" }
                    },
                    "required": ["dsl"]
                }
            }),
            // ProviderCall
            json!({
                "name": "ProviderCall",
                "description": "Send a prompt to the configured LLM provider",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "system_prompt": { "type": "string", "description": "System prompt" },
                        "user_prompt": { "type": "string", "description": "User prompt" },
                        "max_tokens": { "type": "integer", "description": "Maximum tokens (optional)" },
                        "temperature": { "type": "number", "description": "Temperature (optional)" }
                    },
                    "required": ["system_prompt", "user_prompt"]
                }
            }),
        ]
    }

    // ─── Tool Call Parsing ───────────────────────────────────────

    /// Parse an MCP tool call into a CommandOp.
    fn parse_tool_call(&self, name: &str, args: &Value) -> Result<CommandOp, String> {
        match name {
            "FileRead" => {
                let path = get_str(args, "path")?;
                Ok(CommandOp::FileRead { path })
            }
            "FileWrite" => {
                let path = get_str(args, "path")?;
                let content = get_str(args, "content")?;
                Ok(CommandOp::FileWrite { path, content })
            }
            "FileDelete" => {
                let path = get_str(args, "path")?;
                Ok(CommandOp::FileDelete { path })
            }
            "ShellExec" => {
                let command = get_str(args, "command")?;
                let workdir = args.get("workdir").and_then(|w| w.as_str()).map(String::from);
                let timeout_ms = args.get("timeout_ms").and_then(|t| t.as_u64());
                Ok(CommandOp::ShellExec { command, workdir, timeout_ms })
            }
            "MemRead" => {
                let key = get_str(args, "key")?;
                Ok(CommandOp::MemRead { key })
            }
            "MemWrite" => {
                let key = get_str(args, "key")?;
                let content = get_str(args, "content")?;
                let level = args.get("level").and_then(|l| l.as_str())
                    .unwrap_or("L2").to_string();
                Ok(CommandOp::MemWrite {
                    key,
                    content,
                    scope: "project".into(),
                    level,
                })
            }
            "MemSearch" => {
                let query = get_str(args, "query")?;
                let top_k = args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(5) as u32;
                Ok(CommandOp::MemSearch { query, top_k })
            }
            "ArtifactSave" => {
                let artifact_type = get_str(args, "artifact_type")?;
                let content = args.get("content").cloned().unwrap_or(json!({}));
                Ok(CommandOp::ArtifactSave { artifact_type, content })
            }
            "ArtifactLoad" => {
                let artifact_type = get_str(args, "artifact_type")?;
                let version = args.get("version").and_then(|v| v.as_u64()).map(|v| v as u32);
                Ok(CommandOp::ArtifactLoad { artifact_type, version })
            }
            "TraceEvent" => {
                let event_type = get_str(args, "event_type")?;
                let data = args.get("data").cloned().unwrap_or(json!({}));
                Ok(CommandOp::TraceEvent { event_type, data })
            }
            "TraceCheckpoint" => {
                let title = get_str(args, "title")?;
                let summary = get_str(args, "summary")?;
                Ok(CommandOp::TraceCheckpoint { title, summary })
            }
            "PointerQuery" => {
                // MCP PointerQuery falls back to MemSearch because the MCP server
                // doesn't hold a PointerTable. Full DSL resolution is only available
                // in the interactive REPL. Here we extract the search intent from
                // common DSL patterns and route to MemSearch.
                let dsl = get_str(args, "dsl")?;
                let query = if dsl.starts_with("@search(") {
                    // @search("query", N) → extract query
                    dsl.trim_start_matches("@search(")
                        .split(',').next()
                        .unwrap_or(&dsl)
                        .trim_matches(|c| c == '"' || c == '\'' || c == ')')
                        .to_string()
                } else if dsl.starts_with("@mem[") {
                    // @mem["key"] → exact key lookup via MemRead
                    let key = dsl.trim_start_matches("@mem[")
                        .trim_end_matches(']')
                        .trim_matches('"')
                        .to_string();
                    return Ok(CommandOp::MemRead { key });
                } else {
                    // @L1[*], @top(N), etc. → search with the raw DSL as query
                    dsl.clone()
                };
                Ok(CommandOp::MemSearch { query, top_k: 10 })
            }
            "ProviderCall" => {
                let system_prompt = get_str(args, "system_prompt")?;
                let user_prompt = get_str(args, "user_prompt")?;
                let max_tokens = args.get("max_tokens").and_then(|t| t.as_u64()).map(|t| t as u32);
                let temperature = args.get("temperature").and_then(|t| t.as_f64()).map(|t| t as f32);
                Ok(CommandOp::ProviderCall {
                    system_prompt,
                    user_prompt,
                    max_tokens,
                    temperature,
                    output_contract: None,
                })
            }
            _ => Err(format!("unknown tool: {}", name)),
        }
    }
}

/// Extract a required string field from a JSON object.
fn get_str(args: &Value, field: &str) -> Result<String, String> {
    args.get(field)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| format!("missing required field '{}'", field))
}

// ─── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_memory::{ArtifactStore, MemoryDatabase};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    /// Build a test McpServer with in-memory dependencies.
    fn test_server() -> (McpServer, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
        let audit = Arc::new(AuditLog::new());
        let provider: Arc<dyn terra_providers::adapter::ProviderAdapter> =
            Arc::new(DryRunAdapter::new());
        let sandbox = SandboxGuard::new(dir.path().to_path_buf(), vec![]);
        let caging = Arc::new(CagingEngine::new(vec![
            Box::new(SpecificationCage),
            Box::new(MemoryCage),
            Box::new(ActionCage::new(sandbox)),
            Box::new(VerificationCage),
            Box::new(MetaCage),
        ]));
        let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));
        (McpServer::new(exec), dir)
    }

    /// Helper to build a JsonRpcRequest.
    fn rpc_request(id: u64, method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params,
        }
    }

    // T-MCP-001: initialize returns protocol version and capabilities
    #[tokio::test]
    async fn mcp_server_initialize() {
        let (server, _dir) = test_server();
        let req = rpc_request(1, "initialize", json!({}));
        let resp = server.dispatch(&req).await.unwrap();

        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "terra");
        assert_eq!(result["serverInfo"]["version"], "0.1.0");
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["capabilities"]["resources"].is_object());
    }

    // T-MCP-002: tools/list returns all 13 syscall tools
    #[tokio::test]
    async fn mcp_tools_list_all_syscalls() {
        let (server, _dir) = test_server();
        let req = rpc_request(2, "tools/list", json!({}));
        let resp = server.dispatch(&req).await.unwrap();

        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 13, "expected 13 tools (10 builtins + TraceEvent + TraceCheckpoint + PointerQuery), got {}", tools.len());

        // Verify all expected tool names are present
        let names: Vec<&str> = tools.iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"FileRead"));
        assert!(names.contains(&"FileWrite"));
        assert!(names.contains(&"FileDelete"));
        assert!(names.contains(&"ShellExec"));
        assert!(names.contains(&"MemRead"));
        assert!(names.contains(&"MemWrite"));
        assert!(names.contains(&"MemSearch"));
        assert!(names.contains(&"ArtifactSave"));
        assert!(names.contains(&"ArtifactLoad"));
        assert!(names.contains(&"TraceEvent"));
        assert!(names.contains(&"TraceCheckpoint"));
        assert!(names.contains(&"PointerQuery"));
        assert!(names.contains(&"ProviderCall"));

        // Each tool should have name, description, inputSchema
        for tool in tools {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    // T-MCP-003: MemWrite through executor writes and is retrievable
    #[tokio::test]
    async fn mcp_mem_write_through_executor() {
        let (server, _dir) = test_server();

        // Write via MCP
        let req = rpc_request(3, "tools/call", json!({
            "name": "MemWrite",
            "arguments": {
                "key": "test_key",
                "content": "hello from MCP",
                "level": "L2"
            }
        }));
        let resp = server.dispatch(&req).await.unwrap();
        assert!(resp.error.is_none(), "MemWrite should succeed, got: {:?}", resp.error);

        // Read back via MCP
        let req = rpc_request(4, "tools/call", json!({
            "name": "MemRead",
            "arguments": { "key": "test_key" }
        }));
        let resp = server.dispatch(&req).await.unwrap();
        assert!(resp.error.is_none(), "MemRead should succeed, got: {:?}", resp.error);

        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("test_key"), "response should contain the key");
        assert!(text.contains("mcp_client"), "owner should be mcp_client");
    }

    // T-MCP-004: MemRead returns data for existing key
    #[tokio::test]
    async fn mcp_mem_read() {
        let (server, _dir) = test_server();

        // First write something
        let write_req = rpc_request(5, "tools/call", json!({
            "name": "MemWrite",
            "arguments": {
                "key": "read_test",
                "content": "data for reading",
                "level": "L2"
            }
        }));
        server.dispatch(&write_req).await.unwrap();

        // Now read it
        let read_req = rpc_request(6, "tools/call", json!({
            "name": "MemRead",
            "arguments": { "key": "read_test" }
        }));
        let resp = server.dispatch(&read_req).await.unwrap();
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("read_test"));
    }

    // T-MCP-010: calling an unknown tool returns an error
    #[tokio::test]
    async fn mcp_invalid_tool_error() {
        let (server, _dir) = test_server();
        let req = rpc_request(10, "tools/call", json!({
            "name": "NonExistentTool",
            "arguments": {}
        }));
        let resp = server.dispatch(&req).await.unwrap();

        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("unknown tool"));
    }

    // Bonus: notifications/initialized returns None (no response)
    #[tokio::test]
    async fn notifications_initialized_is_silent() {
        let (server, _dir) = test_server();
        let req = rpc_request(99, "notifications/initialized", json!({}));
        let resp = server.dispatch(&req).await;
        assert!(resp.is_none(), "notifications/initialized should not produce a response");
    }

    // Bonus: unknown method returns method-not-found error
    #[tokio::test]
    async fn unknown_method_error() {
        let (server, _dir) = test_server();
        let req = rpc_request(100, "some/unknown/method", json!({}));
        let resp = server.dispatch(&req).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    // Bonus: resources/list returns empty
    #[tokio::test]
    async fn resources_list_empty() {
        let (server, _dir) = test_server();
        let req = rpc_request(101, "resources/list", json!({}));
        let resp = server.dispatch(&req).await.unwrap();
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert_eq!(result["resources"].as_array().unwrap().len(), 0);
    }
    /// Build a server whose sandbox protects `.terra/constitution.toml`.
    fn protected_server() -> (McpServer, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".terra")).unwrap();
        std::fs::write(root.join(".terra/constitution.toml"), "original").unwrap();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let artifacts = Arc::new(ArtifactStore::new(root.clone()));
        let audit = Arc::new(AuditLog::new());
        let provider: Arc<dyn terra_providers::adapter::ProviderAdapter> =
            Arc::new(DryRunAdapter::new());
        let sandbox = SandboxGuard::new(root.clone(), vec![root.join(".terra/constitution.toml")]);
        let caging = Arc::new(CagingEngine::new(vec![Box::new(ActionCage::new(sandbox))]));
        let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));
        (McpServer::new(exec), dir)
    }

    // Regression: `..` segments must not escape the sandbox or reach protected files.
    #[tokio::test]
    async fn mcp_file_write_dot_dot_cannot_escape_sandbox() {
        let (server, dir) = protected_server();
        let root = dir.path();
        let escape = format!("{}/../terra_mcp_escape_test.txt", root.display());
        let sneaky = format!("{}/.terra/../.terra/constitution.toml", root.display());

        for (id, path) in [(10, escape.as_str()), (11, sneaky.as_str())] {
            let req = rpc_request(id, "tools/call", json!({
                "name": "FileWrite",
                "arguments": { "path": path, "content": "pwned" }
            }));
            let resp = server.dispatch(&req).await.unwrap();
            let err = resp.error.unwrap_or_else(|| panic!("write to {} should be blocked", path));
            assert_eq!(err.data.unwrap()["cage_block"], true);
        }

        assert!(!root.parent().unwrap().join("terra_mcp_escape_test.txt").exists());
        let constitution = std::fs::read_to_string(root.join(".terra/constitution.toml")).unwrap();
        assert_eq!(constitution, "original");
    }
}
