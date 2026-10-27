//! The command executor: the one place where an agent's intent becomes an
//! effect. Audit → cages → execute → audit, for every command.

pub mod executor;
pub mod isolation;
pub mod router;
pub mod tools;

pub use executor::{CommandExecutor, SessionMode};
pub use isolation::Isolation;
pub use tools::{builtin_tools, is_dangerous, parse_tool_call, ToolSpec};
