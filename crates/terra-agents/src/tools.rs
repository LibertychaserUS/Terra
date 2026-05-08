//! Tool definitions for LLM function calling.
//!
//! Maps Terra's 12 CommandOps + PointerQuery to JSON Schema tool definitions.
//! Supports two modes:
//! - Native function calling (tools field in API request)
//! - Prompt-based fallback (tool descriptions injected into system prompt)
//!
//! Tools are organized by security level for context-efficient injection.

use serde::{Serialize, Deserialize};
use serde_json::{json, Value};

/// A tool definition for function calling (mirrors provider's ToolDef).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Security level for tool access control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolLevel {
    /// Read-only operations. Available in Conversation mode.
    L0ReadOnly,
    /// Write operations. Available in Goal mode.
    L1Write,
    /// Dangerous operations. Require user confirmation.
    L2Dangerous,
    /// Internal only. Never exposed to LLM.
    L3Internal,
}

/// Trait for pluggable tool providers.
/// New tools implement this trait and register with ToolRegistry.
pub trait ToolProvider: Send + Sync {
    /// Tool name (matches JSON Schema function name).
    fn name(&self) -> &str;
    /// Short description for the LLM.
    fn description(&self) -> &str;
    /// JSON Schema for tool parameters.
    fn parameters_schema(&self) -> Value;
    /// Security level.
    fn level(&self) -> ToolLevel;
    /// Convert LLM arguments to a CommandOp JSON representation.
    /// Returns the op name and args for the executor to process.
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String>;
}

/// Registry of available tools. Supports dynamic registration.
pub struct ToolRegistry {
    tools: Vec<Box<dyn ToolProvider>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Create a registry with all built-in tools.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(Box::new(FileReadTool));
        reg.register(Box::new(FileWriteTool));
        reg.register(Box::new(FileDeleteTool));
        reg.register(Box::new(ShellExecTool));
        reg.register(Box::new(MemReadTool));
        reg.register(Box::new(MemWriteTool));
        reg.register(Box::new(MemSearchTool));
        reg.register(Box::new(PointerQueryTool));
        reg.register(Box::new(ArtifactSaveTool));
        reg.register(Box::new(ArtifactLoadTool));
        reg
    }

    /// Register a new tool.
    pub fn register(&mut self, tool: Box<dyn ToolProvider>) {
        self.tools.push(tool);
    }

    /// Get tool definitions for a given security level (includes all levels up to max_level).
    pub fn tool_defs(&self, max_level: ToolLevel) -> Vec<ToolDef> {
        self.tools.iter()
            .filter(|t| t.level() <= max_level)
            .map(|t| ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect()
    }

    /// Find a tool by name.
    pub fn find(&self, name: &str) -> Option<&dyn ToolProvider> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    /// Generate prompt-based tool descriptions for non-FC providers.
    /// Compact format to minimize context usage.
    pub fn prompt_description(&self, max_level: ToolLevel) -> String {
        let mut out = String::from("Available tools (call by outputting JSON {\"tool\": \"name\", \"args\": {...}}):\n\n");
        for tool in &self.tools {
            if tool.level() <= max_level {
                out.push_str(&format!("- **{}**: {}\n", tool.name(), tool.description()));
                // Compact parameter list (not full schema)
                if let Some(props) = tool.parameters_schema().get("properties") {
                    if let Some(obj) = props.as_object() {
                        let params: Vec<String> = obj.iter().map(|(k, v)| {
                            let desc = v.get("description").and_then(|d| d.as_str()).unwrap_or("");
                            format!("  - `{}`: {}", k, desc)
                        }).collect();
                        out.push_str(&params.join("\n"));
                        out.push('\n');
                    }
                }
                out.push('\n');
            }
        }
        out
    }

    /// DSL pointer syntax documentation for system prompt.
    pub fn pointer_dsl_docs() -> &'static str {
        r#"You have access to a pointer memory index via the PointerQuery tool. DSL syntax:
- @mem["key"]          — lookup a specific memory entry by key
- @L1[*]               — list all project-level (L1) memories
- @search("query", N)  — search memories by relevance, return top N
- @top(N)              — top N memories by importance score
- @scope["project"]    — filter by scope (global/project/session/task)
Each result shows: key, level, scope, annotation, salience score."#
    }
}

// ─── Built-in Tool Implementations ────────────────────────────────

struct FileReadTool;
impl ToolProvider for FileReadTool {
    fn name(&self) -> &str { "FileRead" }
    fn description(&self) -> &str { "Read the contents of a file at the given path" }
    fn level(&self) -> ToolLevel { ToolLevel::L0ReadOnly }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to read" }
            },
            "required": ["path"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let path = args.get("path").and_then(|p| p.as_str())
            .ok_or("FileRead requires 'path' argument")?;
        Ok(("FileRead".into(), json!({"path": path})))
    }
}

struct FileWriteTool;
impl ToolProvider for FileWriteTool {
    fn name(&self) -> &str { "FileWrite" }
    fn description(&self) -> &str { "Write content to a file at the given path" }
    fn level(&self) -> ToolLevel { ToolLevel::L1Write }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let path = args.get("path").and_then(|p| p.as_str())
            .ok_or("FileWrite requires 'path'")?;
        let content = args.get("content").and_then(|c| c.as_str())
            .ok_or("FileWrite requires 'content'")?;
        Ok(("FileWrite".into(), json!({"path": path, "content": content})))
    }
}

struct FileDeleteTool;
impl ToolProvider for FileDeleteTool {
    fn name(&self) -> &str { "FileDelete" }
    fn description(&self) -> &str { "Delete a file at the given path (destructive)" }
    fn level(&self) -> ToolLevel { ToolLevel::L2Dangerous }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to delete" }
            },
            "required": ["path"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let path = args.get("path").and_then(|p| p.as_str())
            .ok_or("FileDelete requires 'path'")?;
        Ok(("FileDelete".into(), json!({"path": path})))
    }
}

struct ShellExecTool;
impl ToolProvider for ShellExecTool {
    fn name(&self) -> &str { "ShellExec" }
    fn description(&self) -> &str { "Execute a shell command (requires user confirmation)" }
    fn level(&self) -> ToolLevel { ToolLevel::L2Dangerous }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "workdir": { "type": "string", "description": "Working directory (optional)" }
            },
            "required": ["command"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let cmd = args.get("command").and_then(|c| c.as_str())
            .ok_or("ShellExec requires 'command'")?;
        Ok(("ShellExec".into(), json!({
            "command": cmd,
            "workdir": args.get("workdir").and_then(|w| w.as_str()),
            "timeout_ms": 30000
        })))
    }
}

struct MemReadTool;
impl ToolProvider for MemReadTool {
    fn name(&self) -> &str { "MemRead" }
    fn description(&self) -> &str { "Read a memory entry by key" }
    fn level(&self) -> ToolLevel { ToolLevel::L0ReadOnly }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Memory key to read" }
            },
            "required": ["key"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let key = args.get("key").and_then(|k| k.as_str())
            .ok_or("MemRead requires 'key'")?;
        Ok(("MemRead".into(), json!({"key": key})))
    }
}

struct MemWriteTool;
impl ToolProvider for MemWriteTool {
    fn name(&self) -> &str { "MemWrite" }
    fn description(&self) -> &str { "Write content to project memory at a specified level" }
    fn level(&self) -> ToolLevel { ToolLevel::L1Write }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Memory key" },
                "content": { "type": "string", "description": "Content to store" },
                "level": { "type": "string", "enum": ["L1","L2","L3","L4","L5","L6"], "description": "Memory level" }
            },
            "required": ["key", "content", "level"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let key = args.get("key").and_then(|k| k.as_str()).ok_or("requires 'key'")?;
        let content = args.get("content").and_then(|c| c.as_str()).ok_or("requires 'content'")?;
        let level = args.get("level").and_then(|l| l.as_str()).unwrap_or("L2");
        Ok(("MemWrite".into(), json!({"key": key, "content": content, "scope": "project", "level": level})))
    }
}

struct MemSearchTool;
impl ToolProvider for MemSearchTool {
    fn name(&self) -> &str { "MemSearch" }
    fn description(&self) -> &str { "Search project memory by query, returns top matches" }
    fn level(&self) -> ToolLevel { ToolLevel::L0ReadOnly }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "top_k": { "type": "integer", "description": "Max results (default 5)" }
            },
            "required": ["query"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let query = args.get("query").and_then(|q| q.as_str()).ok_or("requires 'query'")?;
        let top_k = args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(5);
        Ok(("MemSearch".into(), json!({"query": query, "top_k": top_k})))
    }
}

struct PointerQueryTool;
impl ToolProvider for PointerQueryTool {
    fn name(&self) -> &str { "PointerQuery" }
    fn description(&self) -> &str { "Query the pointer memory index using DSL syntax (@mem[\"key\"], @L1[*], @search(\"query\", N), @top(N))" }
    fn level(&self) -> ToolLevel { ToolLevel::L0ReadOnly }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "dsl": { "type": "string", "description": "DSL query (e.g. @mem[\"auth\"], @L1[*], @search(\"bug\", 5), @top(10))" }
            },
            "required": ["dsl"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let dsl = args.get("dsl").and_then(|d| d.as_str()).ok_or("requires 'dsl'")?;
        Ok(("PointerQuery".into(), json!({"dsl": dsl})))
    }
}

struct ArtifactSaveTool;
impl ToolProvider for ArtifactSaveTool {
    fn name(&self) -> &str { "ArtifactSave" }
    fn description(&self) -> &str { "Save a versioned artifact (plan, spec, test, review)" }
    fn level(&self) -> ToolLevel { ToolLevel::L1Write }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "artifact_type": { "type": "string", "enum": ["plan","spec","test","review"], "description": "Artifact type" },
                "content": { "type": "object", "description": "Artifact content (JSON)" }
            },
            "required": ["artifact_type", "content"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let at = args.get("artifact_type").and_then(|a| a.as_str()).ok_or("requires 'artifact_type'")?;
        let content = args.get("content").cloned().unwrap_or(json!({}));
        Ok(("ArtifactSave".into(), json!({"artifact_type": at, "content": content})))
    }
}

struct ArtifactLoadTool;
impl ToolProvider for ArtifactLoadTool {
    fn name(&self) -> &str { "ArtifactLoad" }
    fn description(&self) -> &str { "Load a versioned artifact by type and optional version" }
    fn level(&self) -> ToolLevel { ToolLevel::L0ReadOnly }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "artifact_type": { "type": "string", "enum": ["plan","spec","test","review"], "description": "Artifact type" },
                "version": { "type": "integer", "description": "Version number (optional, latest if omitted)" }
            },
            "required": ["artifact_type"]
        })
    }
    fn parse_args(&self, args: &Value) -> Result<(String, Value), String> {
        let at = args.get("artifact_type").and_then(|a| a.as_str()).ok_or("requires 'artifact_type'")?;
        Ok(("ArtifactLoad".into(), json!({"artifact_type": at, "version": args.get("version")})))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_with_builtins() {
        let reg = ToolRegistry::with_builtins();
        assert_eq!(reg.tools.len(), 10);
    }

    #[test]
    fn l0_tools_in_conversation() {
        let reg = ToolRegistry::with_builtins();
        let defs = reg.tool_defs(ToolLevel::L0ReadOnly);
        assert!(defs.iter().all(|d| ["FileRead", "MemRead", "MemSearch", "PointerQuery", "ArtifactLoad"].contains(&d.name.as_str())));
        assert_eq!(defs.len(), 5);
    }

    #[test]
    fn l1_tools_in_goal() {
        let reg = ToolRegistry::with_builtins();
        let defs = reg.tool_defs(ToolLevel::L1Write);
        assert!(defs.len() > 5); // L0 + L1 tools
        assert!(defs.iter().any(|d| d.name == "FileWrite"));
        assert!(defs.iter().any(|d| d.name == "MemWrite"));
    }

    #[test]
    fn l2_includes_dangerous() {
        let reg = ToolRegistry::with_builtins();
        let defs = reg.tool_defs(ToolLevel::L2Dangerous);
        assert!(defs.iter().any(|d| d.name == "ShellExec"));
        assert!(defs.iter().any(|d| d.name == "FileDelete"));
    }

    #[test]
    fn find_tool_by_name() {
        let reg = ToolRegistry::with_builtins();
        assert!(reg.find("FileRead").is_some());
        assert!(reg.find("PointerQuery").is_some());
        assert!(reg.find("NonExistent").is_none());
    }

    #[test]
    fn tool_def_has_valid_schema() {
        let reg = ToolRegistry::with_builtins();
        for def in reg.tool_defs(ToolLevel::L2Dangerous) {
            assert!(!def.name.is_empty());
            assert!(!def.description.is_empty());
            assert!(def.parameters.get("type").is_some());
            assert!(def.parameters.get("properties").is_some());
        }
    }

    #[test]
    fn parse_file_read_args() {
        let tool = FileReadTool;
        let args = json!({"path": "/tmp/test.txt"});
        let (op, parsed) = tool.parse_args(&args).unwrap();
        assert_eq!(op, "FileRead");
        assert_eq!(parsed["path"], "/tmp/test.txt");
    }

    #[test]
    fn parse_pointer_query_args() {
        let tool = PointerQueryTool;
        let args = json!({"dsl": "@search(\"auth\", 5)"});
        let (op, parsed) = tool.parse_args(&args).unwrap();
        assert_eq!(op, "PointerQuery");
        assert_eq!(parsed["dsl"], "@search(\"auth\", 5)");
    }

    #[test]
    fn prompt_description_compact() {
        let reg = ToolRegistry::with_builtins();
        let desc = reg.prompt_description(ToolLevel::L0ReadOnly);
        assert!(desc.contains("FileRead"));
        assert!(desc.contains("PointerQuery"));
        assert!(!desc.contains("FileWrite")); // L1, not included at L0
    }

    #[test]
    fn pointer_dsl_docs_present() {
        let docs = ToolRegistry::pointer_dsl_docs();
        assert!(docs.contains("@mem[\"key\"]"));
        assert!(docs.contains("@search"));
        assert!(docs.contains("@top"));
    }

    #[test]
    fn register_custom_tool() {
        struct CustomTool;
        impl ToolProvider for CustomTool {
            fn name(&self) -> &str { "CustomTool" }
            fn description(&self) -> &str { "A custom tool" }
            fn parameters_schema(&self) -> Value { json!({"type": "object", "properties": {}}) }
            fn level(&self) -> ToolLevel { ToolLevel::L0ReadOnly }
            fn parse_args(&self, _args: &Value) -> Result<(String, Value), String> {
                Ok(("Custom".into(), json!({})))
            }
        }

        let mut reg = ToolRegistry::with_builtins();
        let before = reg.tools.len();
        reg.register(Box::new(CustomTool));
        assert_eq!(reg.tools.len(), before + 1);
        assert!(reg.find("CustomTool").is_some());
    }
}
