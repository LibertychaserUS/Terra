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

use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_types::command::{Command, CommandOp};
use terra_types::event::{StreamEvent, TokenTracker};

use crate::session::SessionManager;

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
    core: Arc<IntelligenceCore>,
    session: SessionManager,
    context: Vec<ConversationTurn>,
    token_tracker: TokenTracker,
}

/// A single turn in the conversation.
#[derive(Debug, Clone)]
struct ConversationTurn {
    role: String,    // "user" or "assistant"
    content: String,
}

impl Repl {
    pub fn new(executor: Arc<CommandExecutor>, core: Arc<IntelligenceCore>) -> Self {
        Self {
            session: SessionManager::new(executor.clone()),
            executor,
            core,
            context: Vec::new(),
            token_tracker: TokenTracker::new(),
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
        let cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

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
                let mut out = vec![
                    ReplOutput::Notification("Goal Mode ended. Back to Conversation.".into()),
                ];
                if !report.candidates.is_empty() {
                    out.push(ReplOutput::Text(format!(
                        "Sleep consolidation: {} candidates from {} actions",
                        report.candidates.len(), report.action_count
                    )));
                    for (i, c) in report.candidates.iter().enumerate() {
                        out.push(ReplOutput::Text(format!(
                            "  [{}] {} (from {} actions, IC rule: {})",
                            i, c.principle,
                            c.derived_from.len(),
                            c.ic_rule_id.as_deref().unwrap_or("none"),
                        )));
                    }
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
                let cmd = Command::new("user", CommandOp::MemWrite {
                    key: format!("user:{}", uuid::Uuid::new_v4().to_string()[..8].to_string()),
                    content: arg.to_string(),
                    scope: "project".into(),
                    level: "L1".into(),
                });
                let result = self.executor.execute(cmd).await;
                let msg = if result.status == terra_types::CommandStatus::Ok {
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
            "/trace" => {
                // Executor holds the audit log — we can't directly access it here.
                // In a full implementation, we'd have a reference to AuditLog.
                vec![
                    ReplOutput::Text("Trace: use `terra trace` for full audit log".into()),
                    self.prompt(),
                ]
            }
            "/help" => {
                vec![
                    ReplOutput::Text("Commands:".into()),
                    ReplOutput::Text("  /goal \"desc\"  — Enter Goal Mode".into()),
                    ReplOutput::Text("  /done          — Exit Goal Mode, trigger sleep".into()),
                    ReplOutput::Text("  /status        — Show mode, drift, tokens".into()),
                    ReplOutput::Text("  /remember \"x\" — Save to L1 memory".into()),
                    ReplOutput::Text("  /sleep         — Manual sleep consolidation".into()),
                    ReplOutput::Text("  /trace         — View audit trail".into()),
                    ReplOutput::Text("  /help          — This message".into()),
                    ReplOutput::Text("  /quit          — Exit Terra".into()),
                    self.prompt(),
                ]
            }
            "/quit" | "/exit" => {
                vec![ReplOutput::Text("Goodbye.".into())]
            }
            _ => {
                vec![
                    ReplOutput::Text(format!("Unknown command: {}. Type /help.", cmd)),
                    self.prompt(),
                ]
            }
        }
    }

    /// Handle a conversation turn (not a command).
    async fn handle_conversation(&mut self, input: &str) -> Vec<ReplOutput> {
        // Add user turn to context
        self.context.push(ConversationTurn {
            role: "user".into(),
            content: input.to_string(),
        });

        // Build system prompt from IC + mode
        let mode_context = if self.session.mode().is_goal() {
            let task_type = terra_core::TaskType::BugFix; // default for Goal Mode
            self.core.build_system_prompt(&task_type)
        } else {
            "You are Terra, an AI Agent OS assistant. Be helpful and concise.".into()
        };

        // Build conversation history for the prompt
        let history: String = self.context.iter()
            .map(|t| format!("{}: {}", t.role, t.content))
            .collect::<Vec<_>>()
            .join("\n");

        let cmd = Command::new("main", CommandOp::ProviderCall {
            system_prompt: mode_context,
            user_prompt: history,
            max_tokens: Some(2000),
            temperature: Some(0.3),
            output_contract: None,
        });

        let result = self.executor.execute(cmd).await;

        let response = result.data
            .as_ref()
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("(no response)")
            .to_string();

        // Track tokens
        let prompt_tokens = result.data.as_ref()
            .and_then(|d| d.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        let completion_tokens = result.data.as_ref()
            .and_then(|d| d.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        self.token_tracker.record(prompt_tokens, completion_tokens);

        // Add assistant turn to context
        self.context.push(ConversationTurn {
            role: "assistant".into(),
            content: response.clone(),
        });

        // Check for notification from auto-record
        let mut out = Vec::new();
        if let Some(ref notif) = result.notification {
            out.push(ReplOutput::Notification(notif.clone()));
        }
        out.push(ReplOutput::Text(response));
        out.push(self.prompt());
        out
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
        assert!(out1.iter().any(|o| matches!(o, ReplOutput::Text(_))));

        let out2 = repl.process_input("What did I just say?").await;
        assert!(out2.iter().any(|o| matches!(o, ReplOutput::Text(_))));

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
        assert!(out.iter().any(|o| matches!(o, ReplOutput::Text(_))));
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
}
