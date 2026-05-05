//! Skill System
//!
//! Skills are composable sequences of syscalls (CommandOps).
//! Registered, versioned, permission-gated by trust level.
//!
//! Trust hierarchy: System > Official > Community > User
//! A Skill declares its required_ops; the registry validates
//! that the executing agent has permission for all of them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use terra_executor::CommandExecutor;
use terra_types::command::{Command, CommandOp, CommandStatus};

/// Trust level for Skills. Higher trust = more ops allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TrustLevel {
    User = 0,
    Community = 1,
    Official = 2,
    System = 3,
}

/// Skill definition — metadata + execution logic.
pub struct SkillDef {
    pub name: String,
    pub version: String,
    pub trust_level: TrustLevel,
    pub required_ops: Vec<String>,
    pub description: String,
    /// The execution function: takes executor + context, returns result
    pub execute_fn: Box<dyn Fn(&SkillContext) -> SkillPlan + Send + Sync>,
}

/// Context passed to a Skill during execution.
pub struct SkillContext {
    pub agent_id: String,
    pub agent_allowed_ops: Vec<String>,
    pub input: serde_json::Value,
}

/// A plan of CommandOps that the Skill wants to execute in sequence.
#[derive(Debug, Clone)]
pub struct SkillPlan {
    pub steps: Vec<SkillStep>,
}

/// A single step in a Skill execution plan.
#[derive(Debug, Clone)]
pub struct SkillStep {
    pub op_name: String,
    pub command: CommandOp,
    pub description: String,
}

/// Result of executing a Skill.
#[derive(Debug, Clone)]
pub struct SkillResult {
    pub skill_name: String,
    pub success: bool,
    pub steps_completed: usize,
    pub steps_total: usize,
    pub output: serde_json::Value,
    pub error: Option<String>,
}

/// Skill Registry — manages registration and lookup.
pub struct SkillRegistry {
    skills: HashMap<String, SkillDef>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self { skills: HashMap::new() }
    }

    /// Register a new Skill.
    pub fn register(&mut self, skill: SkillDef) {
        self.skills.insert(skill.name.clone(), skill);
    }

    /// Look up a Skill by name.
    pub fn lookup(&self, name: &str) -> Option<&SkillDef> {
        self.skills.get(name)
    }

    /// List all registered Skill names.
    pub fn list(&self) -> Vec<&str> {
        self.skills.keys().map(|s| s.as_str()).collect()
    }

    /// Validate that an agent can execute a Skill.
    pub fn validate_agent(&self, skill_name: &str, agent_ops: &[String]) -> Result<(), String> {
        let skill = self.skills.get(skill_name)
            .ok_or_else(|| format!("skill '{}' not found", skill_name))?;

        let missing: Vec<&String> = skill.required_ops
            .iter()
            .filter(|op| !agent_ops.contains(op))
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "agent missing ops for skill '{}': {:?}",
                skill_name, missing
            ))
        }
    }

    /// Execute a Skill plan through the Executor.
    pub async fn execute_plan(
        &self,
        plan: &SkillPlan,
        agent_id: &str,
        executor: &CommandExecutor,
    ) -> SkillResult {
        let mut output = serde_json::json!({});
        let total = plan.steps.len();

        for (i, step) in plan.steps.iter().enumerate() {
            let cmd = Command::new(agent_id, step.command.clone());
            let result = executor.execute(cmd).await;

            if result.status != CommandStatus::Ok {
                return SkillResult {
                    skill_name: String::new(),
                    success: false,
                    steps_completed: i,
                    steps_total: total,
                    output,
                    error: result.error,
                };
            }

            if let Some(data) = result.data {
                output[&step.op_name] = data;
            }
        }

        SkillResult {
            skill_name: String::new(),
            success: true,
            steps_completed: total,
            steps_total: total,
            output,
            error: None,
        }
    }
}

// --- Built-in Skills ---

/// CodeEditor: FileRead → ProviderCall → FileWrite
pub fn code_editor_skill() -> SkillDef {
    SkillDef {
        name: "code_editor".into(),
        version: "1.0.0".into(),
        trust_level: TrustLevel::System,
        required_ops: vec!["FileRead".into(), "ProviderCall".into(), "FileWrite".into()],
        description: "Read a file, modify via LLM, write back.".into(),
        execute_fn: Box::new(|ctx| {
            let file = ctx.input.get("file").and_then(|f| f.as_str()).unwrap_or("unknown");
            let instruction = ctx.input.get("instruction").and_then(|i| i.as_str()).unwrap_or("");

            SkillPlan {
                steps: vec![
                    SkillStep {
                        op_name: "read".into(),
                        command: CommandOp::FileRead { path: file.into() },
                        description: format!("Read {}", file),
                    },
                    SkillStep {
                        op_name: "modify".into(),
                        command: CommandOp::ProviderCall {
                            system_prompt: "You are a precise code editor. Output ONLY the modified file content.".into(),
                            user_prompt: format!("File: {}\nInstruction: {}", file, instruction),
                            max_tokens: Some(4000),
                            temperature: Some(0.0),
                            output_contract: Some("file_write".into()),
                        },
                        description: format!("Modify {} via LLM", file),
                    },
                    SkillStep {
                        op_name: "write".into(),
                        command: CommandOp::FileWrite {
                            path: file.into(),
                            content: String::new(), // filled from LLM response at runtime
                        },
                        description: format!("Write back {}", file),
                    },
                ],
            }
        }),
    }
}

/// TestRunner: ShellExec → TraceEvent
pub fn test_runner_skill() -> SkillDef {
    SkillDef {
        name: "test_runner".into(),
        version: "1.0.0".into(),
        trust_level: TrustLevel::System,
        required_ops: vec!["ShellExec".into(), "TraceEvent".into()],
        description: "Run tests and record results.".into(),
        execute_fn: Box::new(|ctx| {
            let command = ctx.input.get("command").and_then(|c| c.as_str()).unwrap_or("cargo test");
            let workdir = ctx.input.get("workdir").and_then(|w| w.as_str()).map(String::from);

            SkillPlan {
                steps: vec![
                    SkillStep {
                        op_name: "test".into(),
                        command: CommandOp::ShellExec {
                            command: command.into(),
                            workdir,
                            timeout_ms: Some(60_000),
                        },
                        description: format!("Run: {}", command),
                    },
                    SkillStep {
                        op_name: "record".into(),
                        command: CommandOp::TraceEvent {
                            event_type: "test_run".into(),
                            data: serde_json::json!({"command": command}),
                        },
                        description: "Record test result".into(),
                    },
                ],
            }
        }),
    }
}

/// MemoryInspector: MemSearch → MemRead
pub fn memory_inspector_skill() -> SkillDef {
    SkillDef {
        name: "memory_inspector".into(),
        version: "1.0.0".into(),
        trust_level: TrustLevel::System,
        required_ops: vec!["MemSearch".into(), "MemRead".into()],
        description: "Search and read memory entries.".into(),
        execute_fn: Box::new(|ctx| {
            let query = ctx.input.get("query").and_then(|q| q.as_str()).unwrap_or("");

            SkillPlan {
                steps: vec![
                    SkillStep {
                        op_name: "search".into(),
                        command: CommandOp::MemSearch {
                            query: query.into(),
                            top_k: 5,
                        },
                        description: format!("Search memory: {}", query),
                    },
                ],
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_memory::{ArtifactStore, MemoryDatabase};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    fn test_executor() -> (Arc<CommandExecutor>, tempfile::TempDir) {
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
        (exec, dir)
    }

    fn test_registry() -> SkillRegistry {
        let mut reg = SkillRegistry::new();
        reg.register(code_editor_skill());
        reg.register(test_runner_skill());
        reg.register(memory_inspector_skill());
        reg
    }

    #[test]
    fn registry_list() {
        let reg = test_registry();
        let names = reg.list();
        assert!(names.contains(&"code_editor"));
        assert!(names.contains(&"test_runner"));
        assert!(names.contains(&"memory_inspector"));
    }

    #[test]
    fn registry_lookup() {
        let reg = test_registry();
        let skill = reg.lookup("code_editor").unwrap();
        assert_eq!(skill.trust_level, TrustLevel::System);
        assert!(skill.required_ops.contains(&"FileWrite".to_string()));
    }

    // Skill permission: agent has required ops → ok
    #[test]
    fn skill_permission_allowed() {
        let reg = test_registry();
        let agent_ops = vec!["FileRead".into(), "ProviderCall".into(), "FileWrite".into()];
        assert!(reg.validate_agent("code_editor", &agent_ops).is_ok());
    }

    // Skill permission: agent missing ops → blocked
    #[test]
    fn skill_permission_blocked() {
        let reg = test_registry();
        let agent_ops = vec!["FileRead".into()]; // missing ProviderCall, FileWrite
        let result = reg.validate_agent("code_editor", &agent_ops);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing ops"));
    }

    // Trust levels ordered correctly
    #[test]
    fn trust_level_ordering() {
        assert!(TrustLevel::User < TrustLevel::Community);
        assert!(TrustLevel::Community < TrustLevel::Official);
        assert!(TrustLevel::Official < TrustLevel::System);
    }

    // TestRunner skill executes shell command
    #[tokio::test]
    async fn test_runner_executes() {
        let (exec, _dir) = test_executor();
        let reg = test_registry();
        let skill = reg.lookup("test_runner").unwrap();

        let ctx = SkillContext {
            agent_id: "worker-1".into(),
            agent_allowed_ops: vec!["ShellExec".into(), "TraceEvent".into()],
            input: serde_json::json!({"command": "echo test_passed"}),
        };

        let plan = (skill.execute_fn)(&ctx);
        assert_eq!(plan.steps.len(), 2);

        let result = reg.execute_plan(&plan, "worker-1", &exec).await;
        assert!(result.success, "test runner should succeed: {:?}", result.error);
    }

    // MemoryInspector skill searches memory
    #[tokio::test]
    async fn memory_inspector_executes() {
        let (exec, _dir) = test_executor();
        let reg = test_registry();
        let skill = reg.lookup("memory_inspector").unwrap();

        let ctx = SkillContext {
            agent_id: "explorer-1".into(),
            agent_allowed_ops: vec!["MemSearch".into(), "MemRead".into()],
            input: serde_json::json!({"query": "authentication"}),
        };

        let plan = (skill.execute_fn)(&ctx);
        let result = reg.execute_plan(&plan, "explorer-1", &exec).await;
        assert!(result.success);
    }

    // CodeEditor skill plan has 3 steps
    #[test]
    fn code_editor_plan_structure() {
        let skill = code_editor_skill();
        let ctx = SkillContext {
            agent_id: "worker-1".into(),
            agent_allowed_ops: vec![],
            input: serde_json::json!({"file": "src/main.rs", "instruction": "add logging"}),
        };

        let plan = (skill.execute_fn)(&ctx);
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].op_name, "read");
        assert_eq!(plan.steps[1].op_name, "modify");
        assert_eq!(plan.steps[2].op_name, "write");
    }

    // Unknown skill → error
    #[test]
    fn unknown_skill_error() {
        let reg = test_registry();
        let result = reg.validate_agent("nonexistent", &[]);
        assert!(result.is_err());
    }
}
