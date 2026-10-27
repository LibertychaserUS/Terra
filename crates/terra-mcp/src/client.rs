//! External MCP servers as extra tools for the agent loop.
//!
//! Each server from the constitution's `[[mcp_servers]]` is started over
//! stdio. Its tools are offered to the model as `<server>__<tool>`; a call
//! is forwarded as-is. The executor does not see these calls (they do not
//! touch Terra's file or memory commands), so the loop records each one in
//! the audit log itself and asks the user first, like a shell command.

use std::collections::HashMap;

use rmcp::model::{CallToolRequestParams, Tool};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::TokioChildProcess;
use serde_json::Value;
use tracing::warn;

/// Separator between server name and tool name in the model-facing name.
pub const TOOL_SEPARATOR: &str = "__";

/// A tool offered by a connected server.
#[derive(Debug, Clone)]
pub struct ExternalTool {
    pub server: String,
    pub tool: Tool,
}

impl ExternalTool {
    /// The name the model calls.
    pub fn qualified_name(&self) -> String {
        format!("{}{}{}", self.server, TOOL_SEPARATOR, self.tool.name)
    }
}

struct Connected {
    service: RunningService<RoleClient, ()>,
    tools: Vec<Tool>,
}

/// Connections to the project's external MCP servers.
#[derive(Default)]
pub struct McpClients {
    servers: HashMap<String, Connected>,
}

/// How to start one server.
#[derive(Debug, Clone)]
pub struct ServerSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

impl McpClients {
    /// Start every server. One that fails to start is skipped with a warning;
    /// the others still work.
    pub async fn connect(specs: &[ServerSpec], cwd: &std::path::Path) -> Self {
        let mut clients = Self::default();
        for spec in specs {
            match Self::connect_one(spec, cwd).await {
                Ok(connected) => {
                    clients.servers.insert(spec.name.clone(), connected);
                }
                Err(e) => warn!(server = %spec.name, error = %e, "MCP server not started"),
            }
        }
        clients
    }

    async fn connect_one(spec: &ServerSpec, cwd: &std::path::Path) -> anyhow::Result<Connected> {
        let mut command = tokio::process::Command::new(&spec.command);
        command.args(&spec.args).current_dir(cwd);
        let transport = TokioChildProcess::new(command)?;
        let service = ().serve(transport).await?;
        let tools = service.peer().list_all_tools().await?;
        Ok(Connected { service, tools })
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    pub fn server_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.servers.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// Every external tool, grouped by server name.
    pub fn tools(&self) -> Vec<ExternalTool> {
        let mut out = Vec::new();
        for name in self.server_names() {
            for tool in &self.servers[name].tools {
                out.push(ExternalTool { server: name.to_string(), tool: tool.clone() });
            }
        }
        out
    }

    /// Split a qualified name into (server, tool) if it names a known server.
    pub fn resolve<'a>(&self, qualified: &'a str) -> Option<(&'a str, &'a str)> {
        let (server, tool) = qualified.split_once(TOOL_SEPARATOR)?;
        self.servers.contains_key(server).then_some((server, tool))
    }

    /// Call a tool and return its text content (or an error string).
    pub async fn call(&self, server: &str, tool: &str, arguments: Value) -> anyhow::Result<String> {
        let connected = self.servers.get(server).ok_or_else(|| anyhow::anyhow!("unknown MCP server {server}"))?;
        let params = CallToolRequestParams::new(tool.to_string()).with_arguments(arguments.as_object().cloned().unwrap_or_default());
        let result = connected.service.peer().call_tool(params).await?;
        let text: Vec<String> = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect();
        let text = if text.is_empty() {
            result.structured_content.map(|v| v.to_string()).unwrap_or_default()
        } else {
            text.join("\n")
        };
        if result.is_error.unwrap_or(false) {
            anyhow::bail!("{}", if text.is_empty() { "tool reported an error".to_string() } else { text });
        }
        Ok(text)
    }

    /// Shut every server down.
    pub async fn shutdown(self) {
        for (_, connected) in self.servers {
            let _ = connected.service.cancel().await;
        }
    }
}
