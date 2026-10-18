//! Interactive REPL (Read-Eval-Print Loop)
//!
//! The dialogue core of Terra. Handles:
//! - User command dispatch (/goal, /done, /status, etc.)
//! - Conversation turns (user → LLM → response)
//! - Multi-turn context accumulation
//! - Streaming output via event callback
//!
//! The REPL is UI-agnostic — it takes input strings and emits
//! ReplOutput events. The CLI/TUI/Web layer renders them.

use std::sync::Arc;

use serde_json::Value;
use tracing::{info, warn};

use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_providers::adapter::{
    CompletionRequest, ToolCall, ToolDef as ProviderToolDef, ToolResult,
};
use terra_types::command::{Command, CommandOp};
use terra_types::event::{StreamEvent, TokenTracker};
use terra_types::text::{prefix_chars, truncate_ellipsis};

use terra_compile::ExecutionPlan;

use crate::orchestrator::Orchestrator;
use crate::skill::{SkillRegistry, SkillContext, code_editor_skill, test_runner_skill, memory_inspector_skill};
use crate::session::SessionManager;
use crate::session_store::{SessionRecord, SessionStore};
use crate::tools::{ToolLevel, ToolRegistry};

/// Callback for L2 tool confirmation.
/// Receives a human-readable prompt string. Returns true if approved, false if declined.
pub type ConfirmCallback = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Output from the REPL — UI-agnostic, renderable by any frontend.
#[derive(Debug, Clone)]
pub enum ReplOutput {
    /// Text to display to the user.
    Text(String),
    /// System notification (e.g., auto-record, mode switch).
    Notification(String),
    /// Streaming token (for incremental display).
    Token(String),
    /// Structured event (for event-driven UIs).
    Event(StreamEvent),
    /// Prompt the user for next input.
    Prompt(String),
}

/// The interactive REPL core.
pub struct Repl {
    executor: Arc<CommandExecutor>,
    #[allow(dead_code)]
    core: Arc<IntelligenceCore>,
    orchestrator: Orchestrator,
    pub session: SessionManager,
    context: Vec<ConversationTurn>,
    token_tracker: TokenTracker,
    session_record: SessionRecord,
    session_store: Option<SessionStore>,
    /// Approximate max context tokens (from provider capability).
    max_context_tokens: u32,
    /// Most recently proposed plan (set by /plan, consumed by /execute).
    pending_plan: Option<ExecutionPlan>,
    /// Skill registry for /skill command.
    skill_registry: SkillRegistry,
    /// Last sleep consolidation report (for /accept and /reject).
    last_consolidation: Option<crate::session::SleepConsolidationReport>,
    /// Optional callback for L2 dangerous tool confirmation.
    /// If None, L2 tools execute without confirmation (non-interactive / test mode).
    confirm_callback: Option<ConfirmCallback>,
    /// Skip confirmation for ShellExec when true.
    pub auto_approve_shell: bool,
    /// Skip confirmation for FileDelete when true.
    pub auto_approve_delete: bool,
    /// Turn count for system prompt strategy (first turn = full, subsequent = minimal).
    turn_count: u32,
}

/// Approximate token count from text (chars / 4 is a rough heuristic).
fn approx_tokens(text: &str) -> u32 {
    (text.len() / 4) as u32
}

/// Check if an L2 (dangerous) tool execution is approved.
///
/// Returns true if the operation should proceed, false if declined.
/// - Auto-approve flags bypass the callback for their respective tool.
/// - If no callback is set, the operation is allowed (non-interactive mode).
fn check_l2_confirmation(
    op_name: &str,
    parsed_args: &Value,
    auto_approve_shell: bool,
    auto_approve_delete: bool,
    confirm_callback: &Option<ConfirmCallback>,
) -> bool {
    match op_name {
        "ShellExec" if auto_approve_shell => true,
        "FileDelete" if auto_approve_delete => true,
        "ShellExec" => {
            let cmd = parsed_args.get("command").and_then(|c| c.as_str()).unwrap_or("?");
            let prompt = format!("[Terra] Execute: `{}`? (y/N)", cmd);
            confirm_callback.as_ref().map(|cb| cb(&prompt)).unwrap_or(true)
        }
        "FileDelete" => {
            let path = parsed_args.get("path").and_then(|p| p.as_str()).unwrap_or("?");
            let prompt = format!("[Terra] Delete {}? (y/N)", path);
            confirm_callback.as_ref().map(|cb| cb(&prompt)).unwrap_or(true)
        }
        _ => {
            let prompt = format!("[Terra] Execute {} (dangerous)? (y/N)", op_name);
            confirm_callback.as_ref().map(|cb| cb(&prompt)).unwrap_or(true)
        }
    }
}

/// Compaction threshold: 70% of max.
const COMPACTION_THRESHOLD: f32 = 0.70;

/// Maximum iterations for the tool-call agent loop.
const MAX_AGENT_LOOP_ITERATIONS: u32 = 20;

/// A single turn in the conversation.
#[derive(Debug, Clone)]
struct ConversationTurn {
    role: String,    // "user" or "assistant"
    content: String,
}

impl Repl {
    pub fn new(executor: Arc<CommandExecutor>, core: Arc<IntelligenceCore>) -> Self {
        let orchestrator = Orchestrator::new(executor.clone(), core.clone());
        let mut skill_registry = SkillRegistry::new();
        skill_registry.register(code_editor_skill());
        skill_registry.register(test_runner_skill());
        skill_registry.register(memory_inspector_skill());
        Self {
            session: SessionManager::new(executor.clone()),
            orchestrator,
            executor,
            core,
            context: Vec::new(),
            token_tracker: TokenTracker::new(),
            session_record: SessionRecord::new(),
            session_store: None,
            max_context_tokens: 128_000, // default, overridden by provider capability
            pending_plan: None,
            skill_registry,
            last_consolidation: None,
            confirm_callback: None,
            auto_approve_shell: false,
            auto_approve_delete: false,
            turn_count: 0,
        }
    }

    /// Set max context tokens (from provider capability).
    pub fn set_max_context(&mut self, max_tokens: u32) {
        self.max_context_tokens = max_tokens;
    }

    /// Set the session store for persistence.
    pub fn with_store(mut self, store: SessionStore) -> Self {
        self.session_store = Some(store);
        self
    }

    /// Set the confirmation callback for L2 dangerous tool operations.
    /// When set, the callback is invoked before executing ShellExec or FileDelete.
    /// If the callback returns false, the tool call is declined.
    pub fn with_confirm_callback(mut self, cb: ConfirmCallback) -> Self {
        self.confirm_callback = Some(cb);
        self
    }

    /// Resume a previous session by loading its record.
    pub fn resume_session(&mut self, record: SessionRecord) {
        // Restore context from turns
        self.context = record.turns.iter().map(|t| ConversationTurn {
            role: t.role.clone(),
            content: t.content.clone(),
        }).collect();
        self.token_tracker = TokenTracker::new();
        self.token_tracker.record(
            record.total_prompt_tokens as u32,
            record.total_completion_tokens as u32,
        );
        self.session_record = record;
    }

    /// Get the current session ID.
    pub fn session_id(&self) -> &str {
        &self.session_record.id
    }

    /// Save current session to disk (if store attached).
    fn auto_save(&self) {
        if let Some(ref store) = self.session_store {
            let _ = store.save(&self.session_record);
        }
    }

    /// Process one line of user input. Returns outputs to display.
    pub async fn process_input(&mut self, input: &str) -> Vec<ReplOutput> {
        let trimmed = input.trim();

        if trimmed.is_empty() {
            return vec![self.prompt()];
        }

        // Command dispatch
        if trimmed.starts_with('/') {
            return self.handle_command(trimmed).await;
        }

        // Conversation turn
        self.handle_conversation(trimmed).await
    }

    /// Handle a /command.
    async fn handle_command(&mut self, input: &str) -> Vec<ReplOutput> {
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let raw_cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        // Command aliases
        let cmd = match raw_cmd {
            "/q" => "/quit",
            "/s" => "/status",
            "/h" => "/help",
            "/g" => "/goal",
            "/d" => "/done",
            "/p" => "/plan",
            "/c" => "/commit",
            other => other,
        };

        match cmd {
            "/goal" => {
                if arg.is_empty() {
                    return vec![
                        ReplOutput::Text("Usage: /goal \"task description\"".into()),
                        self.prompt(),
                    ];
                }
                match self.session.enter_goal(arg).await {
                    Ok(()) => vec![
                        ReplOutput::Notification(format!("Goal Mode activated: {}", arg)),
                        self.prompt(),
                    ],
                    Err(e) => vec![
                        ReplOutput::Text(format!("Error: {}", e)),
                        self.prompt(),
                    ],
                }
            }
            "/done" => {
                let report = self.session.exit_goal().await;
                self.pending_plan = None;
                let mut out = vec![
                    ReplOutput::Notification("Goal Mode ended. Back to Conversation.".into()),
                ];
                if !report.candidates.is_empty() {
                    out.push(ReplOutput::Text(format!(
                        "Sleep consolidation: {} candidates from {} actions",
                        report.candidates.len(), report.action_count
                    )));
                    out.push(ReplOutput::Text("─────────────────────────────────".into()));
                    for (i, c) in report.candidates.iter().enumerate() {
                        out.push(ReplOutput::Text(format!(
                            "  [{}] {}", i, c.principle,
                        )));
                        out.push(ReplOutput::Text(format!(
                            "      from {} actions | IC rule: {}",
                            c.derived_from.len(),
                            c.ic_rule_id.as_deref().unwrap_or("none"),
                        )));
                    }
                    out.push(ReplOutput::Text("─────────────────────────────────".into()));
                    out.push(ReplOutput::Text("Use /accept N or /reject N to decide.".into()));
                    self.last_consolidation = Some(report);
                }
                out.push(self.prompt());
                out
            }
            "/status" => {
                let status = self.session.status();
                let mut lines = vec![
                    format!("Mode: {}", status.mode_name),
                ];
                if let Some(ref intent) = status.goal_intent {
                    lines.push(format!("Goal: {}", intent));
                }
                if let Some(drift) = status.drift_score {
                    lines.push(format!("Drift: {:.2}", drift));
                }
                lines.push(format!("Tokens: {} (prompt: {}, completion: {})",
                    self.token_tracker.total_tokens(),
                    self.token_tracker.prompt_tokens,
                    self.token_tracker.completion_tokens,
                ));
                lines.push(format!("Context turns: {}", self.context.len()));

                let mut out: Vec<ReplOutput> = lines.into_iter()
                    .map(ReplOutput::Text)
                    .collect();
                out.push(self.prompt());
                out
            }
            "/remember" => {
                if arg.is_empty() {
                    return vec![
                        ReplOutput::Text("Usage: /remember \"content to remember\"".into()),
                        self.prompt(),
                    ];
                }
                let mem_key = format!("user:{}", uuid::Uuid::new_v4().to_string()[..8].to_string());
                let cmd = Command::new("user", CommandOp::MemWrite {
                    key: mem_key.clone(),
                    content: arg.to_string(),
                    scope: "project".into(),
                    level: "L1".into(),
                });
                let result = self.executor.execute(cmd).await;
                let msg = if result.status == terra_types::CommandStatus::Ok {
                    // Register pointer for the newly written memory entry
                    let annotation = truncate_ellipsis(arg, 60);
                    self.orchestrator.register_pointer(
                        &mem_key,
                        terra_memory::types::Level::L1,
                        terra_memory::types::Scope::Project,
                        &annotation,
                    );
                    format!("Remembered: {}", arg)
                } else {
                    format!("Failed: {:?}", result.error)
                };
                vec![ReplOutput::Notification(msg), self.prompt()]
            }
            "/sleep" => {
                let report = self.session.consolidate_manual().await;
                let mut out = vec![
                    ReplOutput::Text(format!(
                        "Manual consolidation: {} actions, {} candidates",
                        report.action_count, report.candidates.len()
                    )),
                ];
                for (i, c) in report.candidates.iter().enumerate() {
                    out.push(ReplOutput::Text(format!("  [{}] {}", i, c.principle)));
                }
                out.push(self.prompt());
                out
            }
            "/sessions" => {
                match &self.session_store {
                    Some(store) => {
                        let list = store.list();
                        if list.is_empty() {
                            vec![
                                ReplOutput::Text("No previous sessions.".into()),
                                self.prompt(),
                            ]
                        } else {
                            let mut out = vec![ReplOutput::Text("Sessions:".into())];
                            for s in &list {
                                out.push(ReplOutput::Text(format!(
                                    "  {} — {} turns, {} tokens{}",
                                    s.id, s.turn_count, s.total_tokens,
                                    s.goal.as_ref().map(|g| format!(" ({})", g)).unwrap_or_default(),
                                )));
                            }
                            out.push(self.prompt());
                            out
                        }
                    }
                    None => vec![
                        ReplOutput::Text("No session store (run terra init first).".into()),
                        self.prompt(),
                    ],
                }
            }
            "/trace" => {
                vec![
                    ReplOutput::Text("Trace: use `terra trace` for full audit log".into()),
                    self.prompt(),
                ]
            }
            "/commit" => {
                let project_root = std::env::current_dir().unwrap_or_default();
                let git_info = crate::codebase::detect_git(&project_root);
                if !git_info.is_repo {
                    return vec![
                        ReplOutput::Text("Not a git repository.".into()),
                        self.prompt(),
                    ];
                }
                if !git_info.has_changes {
                    return vec![
                        ReplOutput::Text("No changes to commit.".into()),
                        self.prompt(),
                    ];
                }
                // Ask LLM for commit message
                let diff_cmd = Command::new("main", CommandOp::ShellExec {
                    command: "git diff --stat".into(),
                    workdir: Some(project_root.to_string_lossy().to_string()),
                    timeout_ms: Some(5000),
                });
                let diff_result = self.executor.execute(diff_cmd).await;
                let diff_text = diff_result.data.as_ref()
                    .and_then(|d| d.get("stdout"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("(no diff)");

                let msg_cmd = Command::new("main", CommandOp::ProviderCall {
                    system_prompt: "Generate a concise git commit message (1-2 lines) for these changes. Output ONLY the message.".into(),
                    user_prompt: format!("Changes:\n{}", diff_text),
                    max_tokens: Some(100),
                    temperature: Some(0.0),
                    output_contract: None,
                });
                let msg_result = self.executor.execute(msg_cmd).await;
                let commit_msg = msg_result.data.as_ref()
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("Terra: auto-commit");

                // Stage all + commit
                match crate::codebase::git_commit(&project_root, &["."], commit_msg) {
                    Ok(output) => vec![
                        ReplOutput::Notification(format!("  ✓ Committed: {}", commit_msg.lines().next().unwrap_or(commit_msg))),
                        ReplOutput::Text(output),
                        self.prompt(),
                    ],
                    Err(e) => vec![
                        ReplOutput::Text(format!("Commit failed: {}", e)),
                        self.prompt(),
                    ],
                }
            }
            "/plan" => {
                // Show current plan if in Goal Mode
                if !self.session.mode().is_goal() {
                    return vec![
                        ReplOutput::Text("No active plan. Use /goal first.".into()),
                        self.prompt(),
                    ];
                }
                // Try to propose a plan for the current goal
                let goal = match self.session.mode() {
                    crate::session::SessionMode::Goal { spec, .. } => spec.intent.clone(),
                    _ => String::new(),
                };
                match self.orchestrator.propose_plan(&goal).await {
                    Ok(plan) => {
                        let display = plan.display();
                        self.pending_plan = Some(plan);
                        vec![
                            ReplOutput::Text(display),
                            ReplOutput::Notification("Plan stored. Use /execute to run it.".into()),
                            self.prompt(),
                        ]
                    }
                    Err(e) => vec![
                        ReplOutput::Text(format!("Plan error: {}", e)),
                        self.prompt(),
                    ],
                }
            }
            "/execute" => {
                let plan = match self.pending_plan.take() {
                    Some(p) => p,
                    None => return vec![
                        ReplOutput::Text("No pending plan. Use /plan first to propose one.".into()),
                        self.prompt(),
                    ],
                };
                let mut out = vec![
                    ReplOutput::Notification(format!("Executing plan ({} steps)...", plan.steps.len())),
                ];
                let mut current_plan = plan;
                while let Some(step) = current_plan.next_step().map(|s| s.name.clone()) {
                    current_plan.start_step(&step);
                    out.push(ReplOutput::Text(format!("  Running: {}...", step)));

                    // Execute the step via a ProviderCall (generic execution)
                    let step_desc = current_plan.steps.iter()
                        .find(|s| s.name == step)
                        .map(|s| s.description.clone())
                        .unwrap_or_default();
                    let cmd = Command::new("main", CommandOp::ProviderCall {
                        system_prompt: "Execute this plan step precisely. Output only the result.".into(),
                        user_prompt: format!("Step: {}\nGoal: {}", step_desc, current_plan.spec.intent),
                        max_tokens: Some(2000),
                        temperature: Some(0.0),
                        output_contract: None,
                    });
                    let result = self.executor.execute(cmd).await;

                    if result.status == terra_types::CommandStatus::Ok {
                        let step_output = result.data.as_ref()
                            .and_then(|d| d.get("content"))
                            .cloned()
                            .unwrap_or(serde_json::json!("done"));
                        current_plan.complete_step(&step, step_output);
                        out.push(ReplOutput::Text(format!("  Done: {}", step)));
                    } else {
                        current_plan.fail_step(&step);
                        out.push(ReplOutput::Text(format!("  Failed: {} — {:?}", step, result.error)));
                        break;
                    }
                }
                if current_plan.is_complete() {
                    out.push(ReplOutput::Notification("Plan completed successfully.".into()));
                } else if current_plan.has_failure() {
                    out.push(ReplOutput::Notification("Plan stopped due to failure.".into()));
                }
                out.push(self.prompt());
                out
            }
            "/skill" => {
                if arg.is_empty() {
                    // List available skills
                    let names = self.skill_registry.list();
                    let mut out = vec![ReplOutput::Text("Available skills:".into())];
                    for name in &names {
                        out.push(ReplOutput::Text(format!("  - {}", name)));
                    }
                    out.push(ReplOutput::Text("Usage: /skill <name> [input JSON]".into()));
                    out.push(self.prompt());
                    return out;
                }
                let parts: Vec<&str> = arg.splitn(2, ' ').collect();
                let skill_name = parts[0];
                let input_str = parts.get(1).unwrap_or(&"{}");
                let input: serde_json::Value = serde_json::from_str(input_str)
                    .unwrap_or(serde_json::json!({"instruction": input_str}));

                let skill = match self.skill_registry.lookup(skill_name) {
                    Some(s) => s,
                    None => return vec![
                        ReplOutput::Text(format!("Skill '{}' not found. Use /skill to list.", skill_name)),
                        self.prompt(),
                    ],
                };

                let ctx = SkillContext {
                    agent_id: "main".into(),
                    agent_allowed_ops: skill.required_ops.clone(),
                    input,
                };
                let plan = (skill.execute_fn)(&ctx);
                let result = self.skill_registry.execute_plan(&plan, "main", &self.executor).await;

                let mut out = Vec::new();
                if result.success {
                    out.push(ReplOutput::Notification(format!(
                        "Skill '{}' completed ({}/{} steps)",
                        skill_name, result.steps_completed, result.steps_total
                    )));
                } else {
                    out.push(ReplOutput::Text(format!(
                        "Skill '{}' failed at step {}/{}: {}",
                        skill_name, result.steps_completed, result.steps_total,
                        result.error.as_deref().unwrap_or("unknown error"),
                    )));
                }
                out.push(self.prompt());
                out
            }
            "/accept" => {
                let idx: usize = match arg.parse() {
                    Ok(n) => n,
                    Err(_) => return vec![
                        ReplOutput::Text("Usage: /accept N (candidate index)".into()),
                        self.prompt(),
                    ],
                };
                // Get actual candidate content from the last consolidation report
                let principle = match &self.last_consolidation {
                    Some(report) => match report.candidates.get(idx) {
                        Some(c) => c.principle.clone(),
                        None => return vec![
                            ReplOutput::Text(format!("Invalid index {}. Only {} candidates.", idx, report.candidates.len())),
                            self.prompt(),
                        ],
                    },
                    None => return vec![
                        ReplOutput::Text("No consolidation report. Use /done first.".into()),
                        self.prompt(),
                    ],
                };
                // Write the actual principle to L1 memory
                let key = format!("promoted:{}", uuid::Uuid::new_v4().to_string()[..8].to_string());
                let cmd = Command::new("user", CommandOp::MemWrite {
                    key: key.clone(),
                    content: principle.clone(),
                    scope: "project".into(),
                    level: "L1".into(),
                });
                self.executor.execute(cmd).await;
                self.session.record_verdict(idx, true).await;
                // Register pointer
                let annotation = truncate_ellipsis(&principle, 60);
                self.orchestrator.register_pointer(
                    &key,
                    terra_memory::types::Level::L1,
                    terra_memory::types::Scope::Project,
                    &annotation,
                );
                vec![
                    ReplOutput::Notification(format!("Candidate #{} accepted → L1: \"{}\"", idx, annotation)),
                    self.prompt(),
                ]
            }
            "/reject" => {
                let idx: usize = match arg.parse() {
                    Ok(n) => n,
                    Err(_) => return vec![
                        ReplOutput::Text("Usage: /reject N (candidate index)".into()),
                        self.prompt(),
                    ],
                };
                self.session.record_verdict(idx, false).await;
                vec![
                    ReplOutput::Notification(format!("Candidate #{} rejected → discarded", idx)),
                    self.prompt(),
                ]
            }
            "/clear" => {
                self.context.clear();
                vec![
                    ReplOutput::Text("\x1b[2J\x1b[H".into()), // ANSI clear screen
                    ReplOutput::Notification("Context cleared.".into()),
                    self.prompt(),
                ]
            }
            "/compact" => {
                let (before, after) = self.compact_context().await;
                vec![
                    ReplOutput::Notification(format!("Compacted: {} turns → {} turns", before, after)),
                    self.prompt(),
                ]
            }
            "/help" => {
                vec![
                    ReplOutput::Text("Commands:".into()),
                    ReplOutput::Text("  /goal \"desc\"   — Enter Goal Mode (/g)".into()),
                    ReplOutput::Text("  /done           — Exit Goal Mode, trigger sleep (/d)".into()),
                    ReplOutput::Text("  /status         — Show mode, drift, tokens (/s)".into()),
                    ReplOutput::Text("  /plan           — Show/propose execution plan (/p)".into()),
                    ReplOutput::Text("  /execute        — Run the most recently proposed plan".into()),
                    ReplOutput::Text("  /skill [name]   — List or run a registered Skill".into()),
                    ReplOutput::Text("  /accept N       — Accept sleep promotion candidate".into()),
                    ReplOutput::Text("  /reject N       — Reject sleep promotion candidate".into()),
                    ReplOutput::Text("  /commit         — Stage + commit with LLM message (/c)".into()),
                    ReplOutput::Text("  /undo           — Revert all uncommitted changes".into()),
                    ReplOutput::Text("  /remember \"x\"  — Save to L1 memory".into()),
                    ReplOutput::Text("  /sessions       — List past sessions".into()),
                    ReplOutput::Text("  /compact        — Compact context (free memory)".into()),
                    ReplOutput::Text("  /sleep          — Manual sleep consolidation".into()),
                    ReplOutput::Text("  /clear          — Clear screen and context".into()),
                    ReplOutput::Text("  /help           — This message (/h)".into()),
                    ReplOutput::Text("  /quit           — Exit Terra (/q)".into()),
                    ReplOutput::Text("".into()),
                    ReplOutput::Text("  Ctrl+C interrupt │ Ctrl+D exit │ ↑↓ history".into()),
                    self.prompt(),
                ]
            }
            "/quit" | "/exit" => {
                vec![ReplOutput::Text("Goodbye.".into())]
            }
            "/undo" => {
                let project_root = std::env::current_dir().unwrap_or_default();
                let output = std::process::Command::new("git")
                    .args(["checkout", "."])
                    .current_dir(&project_root)
                    .output();
                match output {
                    Ok(o) if o.status.success() => vec![
                        ReplOutput::Notification("  ✓ Reverted all uncommitted changes".into()),
                        self.prompt(),
                    ],
                    Ok(o) => vec![
                        ReplOutput::Text(format!("Undo failed: {}", String::from_utf8_lossy(&o.stderr))),
                        self.prompt(),
                    ],
                    Err(e) => vec![
                        ReplOutput::Text(format!("Undo failed: {}", e)),
                        self.prompt(),
                    ],
                }
            }
            _ => {
                vec![
                    ReplOutput::Text(format!("Unknown command: {}. Type /help.", cmd)),
                    self.prompt(),
                ]
            }
        }
    }

    /// Handle a conversation turn with agent loop (tool calling).
    ///
    /// System prompt strategy:
    /// - Turn 1: full context (identity + tools docs + project memory + pointer DSL)
    /// - Turn 2+: minimal (identity only; model retains context from turn 1)
    /// - /goal entry: adds goal intent + relevant IC rules (once)
    /// Tool descriptions are NOT in the prompt — they're in the API tools field.
    async fn handle_conversation(&mut self, input: &str) -> Vec<ReplOutput> {
        // Add user turn to context
        self.context.push(ConversationTurn {
            role: "user".into(),
            content: input.to_string(),
        });
        self.session_record.add_turn("user", input);
        self.turn_count += 1;

        let mut out = Vec::new();

        // --- 1. Tool level + definitions ---
        let is_goal = self.session.mode().is_goal();
        let tool_level = ToolLevel::L2Dangerous;

        let tool_registry = ToolRegistry::with_builtins();
        let tool_defs: Vec<ProviderToolDef> = tool_registry
            .tool_defs(tool_level)
            .into_iter()
            .map(|d| ProviderToolDef {
                name: d.name,
                description: d.description,
                parameters: d.parameters,
            })
            .collect();

        // --- 2. System prompt: first turn full, subsequent minimal ---
        let system_prompt = if self.turn_count == 1 {
            // First turn: load prompt from .terra/prompt.md (user-editable) or fallback to embedded
            let project_root = std::env::current_dir().unwrap_or_default();
            let prompt_path = project_root.join(".terra").join("prompt.md");
            let mut prompt = if prompt_path.exists() {
                std::fs::read_to_string(&prompt_path)
                    .unwrap_or_else(|_| include_str!("prompt.md").to_string())
            } else {
                include_str!("prompt.md").to_string()
            };

            // Inject L1 project memory summary (compact)
            let pointer_ctx = {
                let pt = self.orchestrator.pointer_table().lock().unwrap();
                pt.serialize_compact()
            };
            if !pointer_ctx.is_empty() {
                prompt.push_str("\n\n## Project Memory\n");
                prompt.push_str(&pointer_ctx);
            }

            // If already in goal mode at turn 1
            if is_goal {
                if let crate::session::SessionMode::Goal { spec, .. } = self.session.mode() {
                    prompt.push_str(&format!("\n\n## Active Goal\n{}", spec.intent));
                }
            }

            prompt
        } else if is_goal {
            // Goal mode, turn 2+: just identity + goal
            let goal = match self.session.mode() {
                crate::session::SessionMode::Goal { spec, .. } => spec.intent.clone(),
                _ => String::new(),
            };
            format!("You are Terra. Active goal: {}", goal)
        } else {
            // Conversation mode, turn 2+: minimal
            "You are Terra.".to_string()
        };

        // --- 2. Build conversation history for the user_prompt ---
        let history: String = self.context.iter()
            .map(|t| format!("{}: {}", t.role, t.content))
            .collect::<Vec<_>>()
            .join("\n");

        // --- 3. Agent loop ---
        let mut prompt_tokens: u32 = 0;
        let mut completion_tokens: u32 = 0;
        let mut response = String::new();

        let mut pending_tool_calls: Option<Vec<ToolCall>> = None;
        let mut tool_results: Option<Vec<ToolResult>> = None;

        let provider = self.executor.provider().clone();

        for iteration in 1..=MAX_AGENT_LOOP_ITERATIONS {
            // Build request: first iteration sends history, subsequent iterations
            // send empty user_prompt (tool results carry the conversation forward).
            let request = CompletionRequest {
                system_prompt: system_prompt.clone(),
                user_prompt: if iteration == 1 { history.clone() } else { String::new() },
                max_tokens: Some(2000),
                temperature: Some(0.3),
                tools: Some(tool_defs.clone()),
                pending_tool_calls: pending_tool_calls.take(),
                tool_results: tool_results.take(),
            };

            let completion = match provider.complete(request).await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!(error = %e, "provider call failed in agent loop");
                    response = format!("Error calling provider: {}", e);
                    break;
                }
            };

            // Track tokens from this iteration
            prompt_tokens += completion.usage_prompt_tokens;
            completion_tokens += completion.usage_completion_tokens;

            // Audit the LLM call
            self.executor.audit_llm_call(
                "main",
                completion.usage_prompt_tokens,
                completion.usage_completion_tokens,
                completion.tool_calls.len(),
                iteration,
            );

            info!(
                iteration,
                tool_calls = completion.tool_calls.len(),
                content_len = completion.content.len(),
                "agent loop iteration"
            );

            // --- Check for tool calls ---
            if !completion.tool_calls.is_empty() {
                let mut results = Vec::new();
                let calls = completion.tool_calls.clone();

                for call in &calls {
                    out.push(ReplOutput::Notification(
                        format!("  >> Tool: {} ...", call.name)
                    ));

                    // Execute the tool call
                    let result_content = self.execute_tool_call(
                        &tool_registry, &call.name, &call.arguments,
                    ).await;

                    out.push(ReplOutput::Notification(
                        format!("  << {} complete", call.name)
                    ));

                    results.push(ToolResult {
                        tool_call_id: call.id.clone(),
                        content: result_content,
                    });
                }

                // Store for next iteration
                pending_tool_calls = Some(calls);
                tool_results = Some(results);
                continue; // Loop: call LLM again with results
            }

            // --- No tool calls: final text response ---
            response = if completion.content.is_empty() {
                "I couldn't generate a response. Try rephrasing or check your API key with /status.".to_string()
            } else {
                completion.content
            };
            break;
        }

        // Safety: if loop exhausted, notify user
        if response.is_empty() {
            response = "(Agent loop reached maximum iterations without a final response.)".to_string();
            out.push(ReplOutput::Notification(
                format!("Warning: agent loop reached {} iterations limit.", MAX_AGENT_LOOP_ITERATIONS)
            ));
        }

        // --- Post-loop: same as before ---

        // Track accumulated tokens
        self.token_tracker.record(prompt_tokens, completion_tokens);

        // Add assistant turn to context + session record
        self.context.push(ConversationTurn {
            role: "assistant".into(),
            content: response.clone(),
        });
        self.session_record.add_turn("assistant", &response);
        self.session_record.update_tokens(
            self.token_tracker.prompt_tokens,
            self.token_tracker.completion_tokens,
        );

        // Auto-save session to disk
        self.auto_save();

        // Output: emit response as Token chunks for streaming display
        // Split on word boundaries so the UI can render incrementally
        for word in response.split_inclusive(|c: char| c.is_whitespace()) {
            out.push(ReplOutput::Token(word.to_string()));
        }
        // Signal end of streaming with empty text
        out.push(ReplOutput::Text(String::new()));

        // Drift detection: feed response to detector (only fires in Goal Mode)
        if let Some(drift_result) = self.session.check_drift(&response) {
            if drift_result.should_pause {
                out.push(ReplOutput::Notification(format!(
                    "\u{26a0} Goal drift detected (score: {:.2}). Your conversation may be drifting from the goal.",
                    drift_result.score
                )));
            }
        }

        let mode_label = if is_goal { "goal" } else { "conversation" };
        out.push(ReplOutput::Notification(
            crate::status::turn_summary(mode_label, prompt_tokens, completion_tokens)
        ));

        if let Some(compaction_msg) = self.check_compaction().await {
            out.push(ReplOutput::Notification(compaction_msg));
        }
        out.push(self.prompt());
        out
    }

    /// Execute a single tool call. Returns the result as a string for the LLM.
    ///
    /// - PointerQuery: resolved locally via the orchestrator's PointerTable.
    /// - All other tools: converted to CommandOp and executed through the executor
    ///   (with full cage checking and audit).
    async fn execute_tool_call(
        &self,
        registry: &ToolRegistry,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> String {
        // Find the tool in the registry
        let tool = match registry.find(tool_name) {
            Some(t) => t,
            None => {
                warn!(tool_name, "unknown tool called by LLM");
                return format!("Error: unknown tool '{}'", tool_name);
            }
        };

        // Parse args via the tool provider
        let (op_name, parsed_args) = match tool.parse_args(arguments) {
            Ok(result) => result,
            Err(e) => {
                warn!(tool_name, error = %e, "tool argument parse error");
                return format!("Error: invalid arguments for '{}': {}", tool_name, e);
            }
        };

        // L2 confirmation gate: check before executing dangerous tools
        if tool.level() == ToolLevel::L2Dangerous {
            let approved = check_l2_confirmation(
                &op_name,
                &parsed_args,
                self.auto_approve_shell,
                self.auto_approve_delete,
                &self.confirm_callback,
            );
            if !approved {
                info!(tool_name, "L2 tool declined by user");
                return "UserDeclined: operation cancelled by user".to_string();
            }
        }

        // Special case: PointerQuery is resolved locally, not through executor
        if op_name == "PointerQuery" {
            return self.resolve_pointer_query(&parsed_args);
        }

        // Convert to CommandOp and execute through executor (cage-checked, audited)
        let cmd_op = match op_name.as_str() {
            "FileRead" => CommandOp::FileRead {
                path: parsed_args["path"].as_str().unwrap_or("").to_string(),
            },
            "FileWrite" => CommandOp::FileWrite {
                path: parsed_args["path"].as_str().unwrap_or("").to_string(),
                content: parsed_args["content"].as_str().unwrap_or("").to_string(),
            },
            "FileDelete" => CommandOp::FileDelete {
                path: parsed_args["path"].as_str().unwrap_or("").to_string(),
            },
            "ShellExec" => CommandOp::ShellExec {
                command: parsed_args["command"].as_str().unwrap_or("").to_string(),
                workdir: parsed_args.get("workdir").and_then(|w| w.as_str()).map(String::from),
                timeout_ms: parsed_args.get("timeout_ms").and_then(|t| t.as_u64()),
            },
            "MemRead" => CommandOp::MemRead {
                key: parsed_args["key"].as_str().unwrap_or("").to_string(),
            },
            "MemWrite" => CommandOp::MemWrite {
                key: parsed_args["key"].as_str().unwrap_or("").to_string(),
                content: parsed_args["content"].as_str().unwrap_or("").to_string(),
                scope: parsed_args.get("scope").and_then(|s| s.as_str()).unwrap_or("project").to_string(),
                level: parsed_args.get("level").and_then(|l| l.as_str()).unwrap_or("L2").to_string(),
            },
            "MemSearch" => CommandOp::MemSearch {
                query: parsed_args["query"].as_str().unwrap_or("").to_string(),
                top_k: parsed_args.get("top_k").and_then(|t| t.as_u64()).unwrap_or(5) as u32,
            },
            "ArtifactSave" => CommandOp::ArtifactSave {
                artifact_type: parsed_args["artifact_type"].as_str().unwrap_or("").to_string(),
                content: parsed_args.get("content").cloned().unwrap_or(serde_json::json!({})),
            },
            "ArtifactLoad" => CommandOp::ArtifactLoad {
                artifact_type: parsed_args["artifact_type"].as_str().unwrap_or("").to_string(),
                version: parsed_args.get("version").and_then(|v| v.as_u64()).map(|v| v as u32),
            },
            unknown => {
                warn!(op = unknown, "unhandled tool op in execute_tool_call");
                return format!("Error: unhandled operation '{}'", unknown);
            }
        };

        let cmd = Command::new("main", cmd_op);
        let result = self.executor.execute(cmd).await;

        // Format result for LLM consumption
        match result.status {
            terra_types::CommandStatus::Ok => {
                match result.data {
                    Some(data) => serde_json::to_string(&data).unwrap_or_else(|_| "{}".into()),
                    None => "OK".to_string(),
                }
            }
            terra_types::CommandStatus::NotFound => {
                format!("Not found: {}", result.error.unwrap_or_default())
            }
            terra_types::CommandStatus::PermissionDenied => {
                format!("Permission denied: {}", result.error.unwrap_or_default())
            }
            terra_types::CommandStatus::Error => {
                format!("Error: {}", result.error.unwrap_or_default())
            }
        }
    }

    /// Resolve a PointerQuery DSL expression against the orchestrator's PointerTable.
    fn resolve_pointer_query(&self, args: &serde_json::Value) -> String {
        let dsl = args.get("dsl").and_then(|d| d.as_str()).unwrap_or("");
        if dsl.is_empty() {
            return "Error: empty DSL query".to_string();
        }

        let pt = self.orchestrator.pointer_table().lock().unwrap();
        let results = pt.resolve(dsl);

        if results.is_empty() {
            return format!("No results for: {}", dsl);
        }

        // Format results for LLM
        let formatted: Vec<String> = results.iter().map(|p| {
            format!(
                "- {} [{:?}/{:?}] (salience: {:.2}): {}",
                p.key, p.level, p.scope, p.salience, p.annotation
            )
        }).collect();

        format!("Pointer results ({}):\n{}", results.len(), formatted.join("\n"))
    }

    /// Check if context needs compaction. Returns notification if compacted.
    async fn check_compaction(&mut self) -> Option<String> {
        let context_size: u32 = self.context.iter()
            .map(|t| approx_tokens(&t.content))
            .sum();

        let threshold = (self.max_context_tokens as f32 * COMPACTION_THRESHOLD) as u32;

        if context_size > threshold {
            let compacted = self.compact_context().await;
            Some(format!(
                "Context compacted: {} turns → {} turns ({} → {} approx tokens)",
                compacted.0, compacted.1, context_size,
                self.context.iter().map(|t| approx_tokens(&t.content)).sum::<u32>()
            ))
        } else {
            None
        }
    }

    /// Compact the conversation context.
    /// Keeps last 5 turns verbatim, summarizes the rest into one turn.
    /// Externalizes key content to memory (L5).
    async fn compact_context(&mut self) -> (usize, usize) {
        let original_count = self.context.len();

        if original_count <= 6 {
            return (original_count, original_count); // nothing to compact
        }

        let keep_recent = 5;
        let old_turns = &self.context[..original_count - keep_recent];
        let recent_turns = self.context[original_count - keep_recent..].to_vec();

        // Build summary of old turns
        let summary: String = old_turns.iter()
            .map(|t| format!("{}: {}", t.role, truncate_ellipsis(&t.content, 100)))
            .collect::<Vec<_>>()
            .join("\n");

        // Externalize summary to memory L5
        let compaction_key = format!("compaction:{}", uuid::Uuid::new_v4().to_string()[..8].to_string());
        let cmd = Command::new("session", CommandOp::MemWrite {
            key: compaction_key.clone(),
            content: summary.clone(),
            scope: "task".into(),
            level: "L5".into(),
        });
        self.executor.execute(cmd).await;

        // Register pointer for the compaction memory entry
        let annotation = truncate_ellipsis(&summary, 60);
        self.orchestrator.register_pointer(
            &compaction_key,
            terra_memory::types::Level::L5,
            terra_memory::types::Scope::Task,
            &annotation,
        );

        // Replace context: one summary turn + recent turns
        let mut new_context = vec![ConversationTurn {
            role: "system".into(),
            content: format!("[Compacted {} earlier turns. Key topics: {}]",
                old_turns.len(),
                prefix_chars(&summary, 200)
            ),
        }];
        new_context.extend(recent_turns);

        let new_count = new_context.len();
        self.context = new_context;

        (original_count, new_count)
    }

    /// Generate the prompt string based on current mode.
    fn prompt(&self) -> ReplOutput {
        let mode = if self.session.mode().is_goal() { "goal" } else { "terra" };
        ReplOutput::Prompt(format!("{} > ", mode))
    }

    /// Get token tracker (for display).
    pub fn tokens(&self) -> &TokenTracker {
        &self.token_tracker
    }

    /// Check if user requested quit.
    pub fn should_quit(outputs: &[ReplOutput]) -> bool {
        outputs.iter().any(|o| matches!(o, ReplOutput::Text(t) if t == "Goodbye."))
    }

    /// Clean up session-scoped memory entries on exit.
    pub fn cleanup_session(&self) {
        let cleaned = self.executor.cleanup_memory_scope(terra_memory::types::Scope::Session);
        if cleaned > 0 {
            tracing::info!(cleaned, "session-scoped memory entries cleaned on exit");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_core::domain::{AlgorithmModule, CodingModule};
    use terra_memory::{ArtifactStore, MemoryDatabase};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    fn test_repl() -> (Repl, tempfile::TempDir) {
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

        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        core.register(Box::new(AlgorithmModule));
        let core = Arc::new(core);

        (Repl::new(exec, core), dir)
    }

    // T-STR-003: conversation accepts multi-turn
    #[tokio::test]
    async fn multi_turn_conversation() {
        let (mut repl, _dir) = test_repl();

        let out1 = repl.process_input("Hello").await;
        assert!(out1.iter().any(|o| matches!(o, ReplOutput::Token(_))));

        let out2 = repl.process_input("What did I just say?").await;
        assert!(out2.iter().any(|o| matches!(o, ReplOutput::Token(_))));

        // Context should accumulate
        assert_eq!(repl.context.len(), 4); // 2 user + 2 assistant
    }

    // T-STR-004: goal mode allows conversation
    #[tokio::test]
    async fn goal_mode_conversation() {
        let (mut repl, _dir) = test_repl();

        // Start session first
        repl.session.start("test", "test").await.unwrap();

        // Enter goal mode
        let out = repl.process_input("/goal fix the auth bug").await;
        assert!(out.iter().any(|o| matches!(o, ReplOutput::Notification(_))));

        // Can still converse in goal mode
        let out = repl.process_input("What files should I look at?").await;
        assert!(out.iter().any(|o| matches!(o, ReplOutput::Token(_))));
    }

    // Command dispatch: /help
    #[tokio::test]
    async fn help_command() {
        let (mut repl, _dir) = test_repl();
        let out = repl.process_input("/help").await;
        let has_help = out.iter().any(|o| match o {
            ReplOutput::Text(t) => t.contains("/goal"),
            _ => false,
        });
        assert!(has_help, "should list commands");
    }

    // Command dispatch: /status
    #[tokio::test]
    async fn status_command() {
        let (mut repl, _dir) = test_repl();
        let out = repl.process_input("/status").await;
        let has_mode = out.iter().any(|o| match o {
            ReplOutput::Text(t) => t.contains("Mode:"),
            _ => false,
        });
        assert!(has_mode);
    }

    // T-CMD-008: commands no LLM cost
    #[tokio::test]
    async fn commands_no_llm_cost() {
        let (mut repl, _dir) = test_repl();

        repl.process_input("/help").await;
        repl.process_input("/status").await;

        // No tokens consumed
        assert_eq!(repl.tokens().total_tokens(), 0, "commands should not use LLM tokens");
    }

    // /quit
    #[tokio::test]
    async fn quit_command() {
        let (mut repl, _dir) = test_repl();
        let out = repl.process_input("/quit").await;
        assert!(Repl::should_quit(&out));
    }

    // Unknown command
    #[tokio::test]
    async fn unknown_command() {
        let (mut repl, _dir) = test_repl();
        let out = repl.process_input("/nonexistent").await;
        let has_error = out.iter().any(|o| match o {
            ReplOutput::Text(t) => t.contains("Unknown command"),
            _ => false,
        });
        assert!(has_error);
    }

    // Empty input returns prompt
    #[tokio::test]
    async fn empty_input_prompt() {
        let (mut repl, _dir) = test_repl();
        let out = repl.process_input("").await;
        assert!(out.iter().any(|o| matches!(o, ReplOutput::Prompt(_))));
    }

    // Prompt changes with mode
    #[tokio::test]
    async fn prompt_reflects_mode() {
        let (mut repl, _dir) = test_repl();

        let out = repl.process_input("").await;
        let prompt = out.iter().find_map(|o| match o {
            ReplOutput::Prompt(p) => Some(p.clone()),
            _ => None,
        }).unwrap();
        assert!(prompt.contains("terra"), "should show 'terra' in conversation mode");
    }

    // ─── L2 Confirmation Tests ──────────────────────────────────

    // T-CONF-001: ShellExec declined when callback returns false
    #[test]
    fn l2_shell_exec_declined() {
        let args = serde_json::json!({"command": "rm -rf /", "timeout_ms": 30000});
        let cb: ConfirmCallback = Arc::new(|_| false);
        let approved = check_l2_confirmation("ShellExec", &args, false, false, &Some(cb));
        assert!(!approved, "ShellExec should be declined when callback returns false");
    }

    // T-CONF-002: ShellExec approved when callback returns true
    #[test]
    fn l2_shell_exec_approved() {
        let args = serde_json::json!({"command": "ls", "timeout_ms": 30000});
        let cb: ConfirmCallback = Arc::new(|_| true);
        let approved = check_l2_confirmation("ShellExec", &args, false, false, &Some(cb));
        assert!(approved, "ShellExec should be approved when callback returns true");
    }

    // T-CONF-003: FileDelete declined when callback returns false
    #[test]
    fn l2_file_delete_declined() {
        let args = serde_json::json!({"path": "/important/file.txt"});
        let cb: ConfirmCallback = Arc::new(|_| false);
        let approved = check_l2_confirmation("FileDelete", &args, false, false, &Some(cb));
        assert!(!approved, "FileDelete should be declined when callback returns false");
    }

    // T-CONF-004: No callback = allowed (non-interactive mode)
    #[test]
    fn l2_no_callback_allows_execution() {
        let args = serde_json::json!({"command": "echo hello"});
        let approved = check_l2_confirmation("ShellExec", &args, false, false, &None);
        assert!(approved, "should allow execution when no callback is set");
    }

    // T-CONF-005: Prompt includes the command for ShellExec
    #[test]
    fn l2_shell_exec_prompt_contains_command() {
        let args = serde_json::json!({"command": "git status", "timeout_ms": 30000});
        let received_prompt = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let prompt_capture = received_prompt.clone();
        let cb: ConfirmCallback = Arc::new(move |prompt: &str| {
            *prompt_capture.lock().unwrap() = prompt.to_string();
            true
        });
        check_l2_confirmation("ShellExec", &args, false, false, &Some(cb));
        let prompt = received_prompt.lock().unwrap();
        assert!(prompt.contains("git status"), "prompt should contain the command");
        assert!(prompt.contains("[Terra]"), "prompt should have [Terra] prefix");
    }

    // T-CONF-006: Prompt includes the path for FileDelete
    #[test]
    fn l2_file_delete_prompt_contains_path() {
        let args = serde_json::json!({"path": "/tmp/to_delete.txt"});
        let received_prompt = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let prompt_capture = received_prompt.clone();
        let cb: ConfirmCallback = Arc::new(move |prompt: &str| {
            *prompt_capture.lock().unwrap() = prompt.to_string();
            false
        });
        check_l2_confirmation("FileDelete", &args, false, false, &Some(cb));
        let prompt = received_prompt.lock().unwrap();
        assert!(prompt.contains("/tmp/to_delete.txt"), "prompt should contain the file path");
    }

    // T-CONF-007: auto_approve_shell skips callback
    #[test]
    fn l2_auto_approve_shell_skips_callback() {
        let args = serde_json::json!({"command": "ls"});
        // Callback that would decline -- but auto_approve should skip it
        let cb: ConfirmCallback = Arc::new(|_| false);
        let approved = check_l2_confirmation("ShellExec", &args, true, false, &Some(cb));
        assert!(approved, "auto_approve_shell should skip the callback");
    }

    // T-CONF-008: auto_approve_delete skips callback
    #[test]
    fn l2_auto_approve_delete_skips_callback() {
        let args = serde_json::json!({"path": "/tmp/file.txt"});
        let cb: ConfirmCallback = Arc::new(|_| false);
        let approved = check_l2_confirmation("FileDelete", &args, false, true, &Some(cb));
        assert!(approved, "auto_approve_delete should skip the callback");
    }

    // T-CONF-009: auto_approve_shell does NOT affect FileDelete
    #[test]
    fn l2_auto_approve_shell_does_not_affect_delete() {
        let args = serde_json::json!({"path": "/tmp/file.txt"});
        let cb: ConfirmCallback = Arc::new(|_| false);
        let approved = check_l2_confirmation("FileDelete", &args, true, false, &Some(cb));
        assert!(!approved, "auto_approve_shell should not affect FileDelete");
    }

    // T-CONF-010: unknown L2 tool falls through to generic prompt
    #[test]
    fn l2_unknown_dangerous_tool_prompts() {
        let args = serde_json::json!({});
        let received_prompt = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let prompt_capture = received_prompt.clone();
        let cb: ConfirmCallback = Arc::new(move |prompt: &str| {
            *prompt_capture.lock().unwrap() = prompt.to_string();
            false
        });
        let approved = check_l2_confirmation("SomeNewDangerousTool", &args, false, false, &Some(cb));
        assert!(!approved);
        let prompt = received_prompt.lock().unwrap();
        assert!(prompt.contains("SomeNewDangerousTool"), "prompt should name the tool");
        assert!(prompt.contains("dangerous"), "prompt should indicate danger");
    }

    // T-CONF-011: with_confirm_callback builder pattern works
    #[test]
    fn with_confirm_callback_sets_callback() {
        let (repl, _dir) = test_repl();
        assert!(repl.confirm_callback.is_none(), "default should have no callback");

        let cb: ConfirmCallback = Arc::new(|_| true);
        let repl = repl.with_confirm_callback(cb);
        assert!(repl.confirm_callback.is_some(), "should have callback after with_confirm_callback");
    }

    // T-CONF-012: auto_approve defaults to false
    #[test]
    fn auto_approve_defaults_false() {
        let (repl, _dir) = test_repl();
        assert!(!repl.auto_approve_shell, "auto_approve_shell should default to false");
        assert!(!repl.auto_approve_delete, "auto_approve_delete should default to false");
    }

    // /accept without /done returns helpful error
    #[tokio::test]
    async fn accept_without_done_returns_error() {
        let (mut repl, _dir) = test_repl();
        let out = repl.process_input("/accept 0").await;
        let has_error = out.iter().any(|o| match o {
            ReplOutput::Text(t) => t.contains("No consolidation report"),
            _ => false,
        });
        assert!(has_error, "should tell user to /done first");
    }

    // /accept with invalid index returns error
    #[tokio::test]
    async fn accept_invalid_index_returns_error() {
        let (mut repl, _dir) = test_repl();
        let out = repl.process_input("/accept abc").await;
        let has_error = out.iter().any(|o| match o {
            ReplOutput::Text(t) => t.contains("Usage"),
            _ => false,
        });
        assert!(has_error, "should show usage for non-numeric input");
    }
    // Regression: long non-ASCII input used to panic on byte slicing.
    #[tokio::test]
    async fn remember_long_cjk_does_not_panic() {
        let (mut repl, _dir) = test_repl();
        let note = "这个项目使用Rust编写，内存系统有七层，每一层都有不同的写权限和垃圾回收策略";
        let out = repl.process_input(&format!("/remember {}", note)).await;
        assert!(out.iter().any(|o| matches!(o, ReplOutput::Notification(n) if n.starts_with("Remembered"))));
    }

    #[tokio::test]
    async fn long_cjk_conversation_and_compaction_do_not_panic() {
        let (mut repl, _dir) = test_repl();
        let line = "请解释一下这个项目的架构，包括内存系统、行为笼子、多智能体运行时以及语言编译管线的设计。".repeat(3);
        for _ in 0..4 {
            let out = repl.process_input(&line).await;
            assert!(out.iter().any(|o| matches!(o, ReplOutput::Token(_))));
        }
        let out = repl.process_input("/compact").await;
        assert!(out.iter().any(|o| matches!(o, ReplOutput::Notification(n) if n.starts_with("Compacted"))));
    }
}
