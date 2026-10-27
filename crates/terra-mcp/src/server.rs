//! Terra as an MCP server: the seven built-in tools over stdio, every call
//! routed through the executor (audit → cages → execute → audit).

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use serde_json::{json, Value};

use terra_executor::{builtin_tools, parse_tool_call, CommandExecutor};
use terra_types::command::{Command, CommandStatus};

/// Name the executor records for calls that arrive over MCP.
pub const MCP_AGENT: &str = "mcp_client";

#[derive(Clone)]
pub struct TerraServer {
    executor: Arc<CommandExecutor>,
}

impl TerraServer {
    pub fn new(executor: Arc<CommandExecutor>) -> Self {
        Self { executor }
    }

    /// The tool list as MCP sees it.
    pub fn tools() -> Vec<Tool> {
        builtin_tools()
            .into_iter()
            .map(|t| {
                let schema = t.parameters.as_object().cloned().unwrap_or_default();
                Tool::new(t.name, t.description, Arc::new(schema))
            })
            .collect()
    }

    /// Run one tool call through the executor and shape the MCP result.
    pub async fn call(&self, name: &str, arguments: Value) -> Result<CallToolResult, ErrorData> {
        let op = parse_tool_call(name, &arguments).map_err(|msg| ErrorData::invalid_params(msg, None))?;
        let result = self.executor.execute(Command::new(MCP_AGENT, op)).await;
        match result.status {
            CommandStatus::Ok => {
                let text = result.data.map(|d| serde_json::to_string_pretty(&d).unwrap_or_default()).unwrap_or_else(|| "ok".into());
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
            }
            CommandStatus::PermissionDenied => {
                let reason = result.error.unwrap_or_else(|| "permission denied".into());
                Err(ErrorData::new(rmcp::model::ErrorCode(-32000), reason, Some(json!({ "cage_block": true }))))
            }
            CommandStatus::NotFound => Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                result.error.unwrap_or_else(|| "not found".into()),
                None,
            )),
            CommandStatus::Error => Ok(CallToolResult::error(vec![ContentBlock::text(
                result.error.unwrap_or_else(|| "internal error".into()),
            )])),
        }
    }

    /// Serve on stdin/stdout until the client disconnects.
    pub async fn run_stdio(self) -> anyhow::Result<()> {
        let service = self.serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        Ok(())
    }
}

impl ServerHandler for TerraServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("terra", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Terra's tools act on the project directory it was started in. Writes stay inside the project; \
                 shell commands run in an OS sandbox; every call is audited and checked by the cages first.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = request.arguments.map(Value::Object).unwrap_or_else(|| json!({}));
        self.call(&request.name, arguments).await.map(CallToolResponse::from)
    }
}
