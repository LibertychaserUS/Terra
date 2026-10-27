//! The tools Terra offers a model, and how a call becomes a `CommandOp`.
//!
//! One catalog serves both the agent loop (as function-calling tools) and
//! the MCP server (as MCP tools), so a model sees the same seven tools
//! whichever way it reaches Terra.

use serde_json::{json, Value};

use terra_types::command::CommandOp;

/// A tool as the model sees it.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub parameters: Value,
    /// Whether the user is asked before it runs (shell, delete).
    pub dangerous: bool,
}

/// Terra's built-in tools.
pub fn builtin_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "FileRead",
            description: "Read a text file. Relative paths are resolved from the project root.",
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "File path to read" } },
                "required": ["path"]
            }),
            dangerous: false,
        },
        ToolSpec {
            name: "FileWrite",
            description: "Create or overwrite a file inside the project. Read it first when it exists.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "Full new content of the file" }
                },
                "required": ["path", "content"]
            }),
            dangerous: false,
        },
        ToolSpec {
            name: "FileDelete",
            description: "Delete a file inside the project. The user is asked first.",
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string", "description": "File path to delete" } },
                "required": ["path"]
            }),
            dangerous: true,
        },
        ToolSpec {
            name: "ShellExec",
            description: "Run a shell command in the project directory (tests, build, git). The user is asked first; writes outside the project are refused by the OS sandbox.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute" },
                    "workdir": { "type": "string", "description": "Working directory relative to the project (optional)" },
                    "timeout_ms": { "type": "integer", "description": "Time limit in milliseconds (default 30000)" }
                },
                "required": ["command"]
            }),
            dangerous: true,
        },
        ToolSpec {
            name: "MemRead",
            description: "Read a memory entry by exact key.",
            parameters: json!({
                "type": "object",
                "properties": { "key": { "type": "string", "description": "Memory key" } },
                "required": ["key"]
            }),
            dangerous: false,
        },
        ToolSpec {
            name: "MemWrite",
            description: "Save a note to project memory. Levels: L2 task notes, L4 failed attempts, L5 observations, L6 candidates for the user to promote. L1 is written only by the user.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Short unique key" },
                    "content": { "type": "string", "description": "What to remember" },
                    "level": { "type": "string", "enum": ["L2", "L3", "L4", "L5", "L6"], "description": "Memory level (default L2)" }
                },
                "required": ["key", "content"]
            }),
            dangerous: false,
        },
        ToolSpec {
            name: "MemSearch",
            description: "Full-text search over project memory.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search words" },
                    "top_k": { "type": "integer", "description": "Max results (default 5)" }
                },
                "required": ["query"]
            }),
            dangerous: false,
        },
    ]
}

/// Whether a built-in tool needs the user's confirmation.
pub fn is_dangerous(name: &str) -> bool {
    builtin_tools().iter().any(|t| t.name == name && t.dangerous)
}

fn required_str(args: &Value, field: &str, tool: &str) -> Result<String, String> {
    args.get(field)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| format!("{} requires '{}'", tool, field))
}

/// Turn a model's tool call into a command, or say what is wrong with it.
pub fn parse_tool_call(name: &str, args: &Value) -> Result<CommandOp, String> {
    match name {
        "FileRead" => Ok(CommandOp::FileRead { path: required_str(args, "path", name)? }),
        "FileWrite" => Ok(CommandOp::FileWrite {
            path: required_str(args, "path", name)?,
            content: required_str(args, "content", name)?,
        }),
        "FileDelete" => Ok(CommandOp::FileDelete { path: required_str(args, "path", name)? }),
        "ShellExec" => Ok(CommandOp::ShellExec {
            command: required_str(args, "command", name)?,
            workdir: args.get("workdir").and_then(|w| w.as_str()).filter(|w| !w.is_empty()).map(String::from),
            timeout_ms: args.get("timeout_ms").and_then(|t| t.as_u64()),
        }),
        "MemRead" => Ok(CommandOp::MemRead { key: required_str(args, "key", name)? }),
        "MemWrite" => Ok(CommandOp::MemWrite {
            key: required_str(args, "key", name)?,
            content: required_str(args, "content", name)?,
            scope: "project".into(),
            level: args.get("level").and_then(|l| l.as_str()).unwrap_or("L2").to_string(),
        }),
        "MemSearch" => Ok(CommandOp::MemSearch {
            query: required_str(args, "query", name)?,
            top_k: args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(5) as u32,
        }),
        other => Err(format!("unknown tool: {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_seven_tools_with_object_schemas() {
        let tools = builtin_tools();
        assert_eq!(tools.len(), 7);
        for t in &tools {
            assert_eq!(t.parameters["type"], "object", "{}", t.name);
        }
        assert!(is_dangerous("ShellExec") && is_dangerous("FileDelete"));
        assert!(!is_dangerous("FileWrite"));
    }

    #[test]
    fn parse_known_and_unknown() {
        let op = parse_tool_call("ShellExec", &json!({"command": "ls", "workdir": ""})).unwrap();
        assert!(matches!(op, CommandOp::ShellExec { workdir: None, .. }));
        let op = parse_tool_call("MemWrite", &json!({"key": "k", "content": "c"})).unwrap();
        assert!(matches!(op, CommandOp::MemWrite { ref level, .. } if level == "L2"));
        assert_eq!(parse_tool_call("FileWrite", &json!({"path": "a"})).unwrap_err(), "FileWrite requires 'content'");
        assert!(parse_tool_call("Nope", &json!({})).unwrap_err().contains("unknown tool"));
    }
}
