mod config;
mod ui;
mod tui;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use terra_agents::Orchestrator;
use terra_caging::engine::*;
use terra_caging::sandbox::SandboxGuard;
use terra_core::domain::{AlgorithmModule, CodingModule};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::MemoryDatabase;
use terra_memory::ArtifactStore;
use terra_providers::adapter::{DryRunAdapter, ProviderAdapter};
use terra_providers::OpenAICompatibleAdapter;
use terra_trace::AuditLog;

use config::{detect_environment, resolve_config, to_toml_string};

/// `println!` that drops color codes when stdout is not a color terminal.
macro_rules! say {
    () => { println!() };
    ($($arg:tt)*) => { println!("{}", ui::paint(&format!($($arg)*))) };
}

#[derive(Parser)]
#[command(name = "terra", version, about = "Terra — Agent OS")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Initialize Terra in the current directory.
    Init,

    /// Interactive dialogue mode (REPL).
    Chat {
        /// Use ratatui TUI instead of readline
        #[arg(long, default_value_t = false)]
        tui: bool,
    },

    /// Check environment readiness.
    Doctor {
        #[arg(long, default_value_t = false)]
        as_json: bool,
    },

    /// Run a task end-to-end through the canonical pipeline.
    Run {
        /// The task description in natural language.
        #[arg(short, long)]
        task: String,

        /// Output as JSON for machine consumption.
        #[arg(long, default_value_t = false)]
        as_json: bool,
    },

    /// Show the audit trail of this project (.terra/audit.db).
    Trace {
        /// Number of most recent entries to show (0 = all).
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,

        /// Output as JSON for machine consumption.
        #[arg(long, default_value_t = false)]
        as_json: bool,
    },

    /// Configuration management.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Start MCP (Model Context Protocol) server on stdio.
    Mcp,
}

#[derive(clap::Subcommand)]
enum ConfigAction {
    /// Show the effective provider configuration.
    Show,
    /// Set a provider setting: provider.base_url, provider.model or provider.api_key.
    Set {
        key: String,
        value: String,
        /// Write to this project's .terra/config.toml instead of ~/.terra/config.toml.
        #[arg(long, default_value_t = false)]
        project: bool,
    },
}

/// Keys `terra config set` accepts — the ones the runtime actually reads.
const SETTABLE_KEYS: &[&str] = &["provider.base_url", "provider.model", "provider.api_key"];

/// Everything a command needs, built from the project directory.
struct Runtime {
    executor: Arc<CommandExecutor>,
    core: Arc<IntelligenceCore>,
    provider_name: String,
}

/// Provider from config files and environment (env > project > global > defaults).
/// Without an API key Terra runs against a dry-run stub that echoes the prompt.
fn build_provider(project_root: &Path) -> (Arc<dyn ProviderAdapter>, String) {
    let cfg = resolve_config(Some(project_root));
    let Some(api_key) = cfg.provider.api_key else {
        return (Arc::new(DryRunAdapter::new()), "dry_run".into());
    };
    match OpenAICompatibleAdapter::new(
        api_key,
        cfg.provider.base_url,
        cfg.provider.model.clone(),
        terra_providers::openai::DEFAULT_TIMEOUT_SECS,
    ) {
        Ok(adapter) => (Arc::new(adapter), cfg.provider.model),
        Err(e) => {
            tracing::warn!("provider setup failed ({e}), using dry_run");
            (Arc::new(DryRunAdapter::new()), "dry_run".into())
        }
    }
}

/// Bootstrap the Terra runtime: Memory, Caging, IC, Executor.
///
/// Loads .terra/constitution.toml if present to configure sandbox and cage rules.
/// Memory and audit are persisted in .terra/ when it exists, in memory otherwise.
fn build_runtime(project_root: &Path, event_bus: Option<Arc<dyn terra_types::EventBus>>) -> Runtime {
    let terra_dir = project_root.join(".terra");
    let persistent = terra_dir.exists();

    let constitution_path = terra_dir.join("constitution.toml");
    let constitution = terra_caging::Constitution::load(&constitution_path).ok();
    if constitution.is_some() {
        tracing::info!("Constitution loaded from {}", constitution_path.display());
    }

    // L2: Memory Database (per-project persistent storage)
    let memory = match persistent.then(|| MemoryDatabase::open(&terra_dir.join("memory.db"))) {
        Some(Ok(db)) => Arc::new(db),
        Some(Err(e)) => {
            tracing::warn!("Failed to open .terra/memory.db: {e}, falling back to in-memory");
            Arc::new(MemoryDatabase::open_in_memory().expect("in-memory SQLite"))
        }
        None => Arc::new(MemoryDatabase::open_in_memory().expect("in-memory SQLite")),
    };

    // L2: Audit Trail
    let audit = match persistent.then(|| AuditLog::with_persistence(&terra_dir.join("audit.db"))) {
        Some(Ok(log)) => Arc::new(log),
        Some(Err(e)) => {
            tracing::warn!("Failed to open .terra/audit.db: {e}, audit kept in memory only");
            Arc::new(AuditLog::new())
        }
        None => Arc::new(AuditLog::new()),
    };

    // L3: Caging Engine — driven by constitution or defaults
    let sandbox = match &constitution {
        Some(c) => c.to_sandbox_guard(project_root),
        None => SandboxGuard::new(project_root.to_path_buf(), vec![]),
    };
    let caging = Arc::new(CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]));

    let artifacts = Arc::new(ArtifactStore::new(terra_dir));
    let (provider, provider_name) = build_provider(project_root);

    let mut executor = CommandExecutor::new(memory, artifacts, audit, caging, provider);
    if let Some(bus) = event_bus {
        executor = executor.with_event_bus(bus);
    }

    // L4: Intelligence Core
    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));

    Runtime { executor: Arc::new(executor), core: Arc::new(core), provider_name }
}

/// `.terra/.gitignore`: only the shareable project files are tracked; memory,
/// audit log, history, sessions, artifacts and the local config stay private.
const TERRA_GITIGNORE: &str = "# Terra runtime data (databases, history, sessions, artifacts, local config).\n# Only the project rules and the system prompt are meant to be shared.\n*\n!.gitignore\n!constitution.toml\n!prompt.md\n";

const DEFAULT_PROJECT_CONFIG: &str = r#"# Provider settings for this project.
# Environment variables (TERRA_PROVIDER_API_KEY, TERRA_PROVIDER_BASE_URL,
# TERRA_PROVIDER_MODEL) take precedence. This file is git-ignored.
[provider]
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
model = "qwen-plus"
# api_key = "..."
"#;

fn write_terra_gitignore(terra_dir: &Path) -> std::io::Result<()> {
    std::fs::write(terra_dir.join(".gitignore"), TERRA_GITIGNORE)
}

/// Write constitution, config, prompt and .gitignore into `.terra/`.
fn write_project_files(project_root: &Path) -> std::io::Result<()> {
    let terra_dir = project_root.join(".terra");
    std::fs::create_dir_all(&terra_dir)?;
    let constitution = format!(
        r#"[meta]
version = 1
description = "Terra project constitution for {}"

[[protected_paths]]
path = ".terra/constitution.toml"

[[protected_paths]]
path = ".terra/audit.db"

[sandbox]
root = "."

[rules]
allow_external_writes = false
require_spec_complete = true
require_verification = true
"#,
        project_root.file_name().and_then(|n| n.to_str()).unwrap_or("project")
    );
    std::fs::write(terra_dir.join("constitution.toml"), constitution)?;
    std::fs::write(terra_dir.join("config.toml"), DEFAULT_PROJECT_CONFIG)?;
    std::fs::write(terra_dir.join("prompt.md"), include_str!("../../terra-agents/src/prompt.md"))?;
    write_terra_gitignore(&terra_dir)
}

fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Warnings only by default: the interactive UI already shows progress.
    // Override with RUST_LOG=terra=info for diagnostics.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("terra=warn"))
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Some(Commands::Init) => {
            use std::io::{BufRead, Write};

            let project_root = current_dir();
            let terra_dir = project_root.join(".terra");

            say!();
            say!("\x1b[1m\x1b[35m  Terra Agent OS — Project Setup\x1b[0m");
            say!();

            if terra_dir.exists() {
                say!("  \x1b[33m.terra/ already exists.\x1b[0m Overwrite constitution, config and prompt? (y/N)");
                print!("  > ");
                std::io::stdout().flush().ok();
                let mut answer = String::new();
                std::io::stdin().lock().read_line(&mut answer).ok();
                if !answer.trim().eq_ignore_ascii_case("y") {
                    say!("  Aborted.");
                    return Ok(());
                }
            }

            write_project_files(&project_root)?;
            say!("  \x1b[32m✓\x1b[0m Constitution written");
            say!("  \x1b[32m✓\x1b[0m Config written");
            say!("  \x1b[32m✓\x1b[0m Prompt written (editable: .terra/prompt.md)");
            say!("  \x1b[32m✓\x1b[0m .gitignore written (runtime data stays out of git)");

            if resolve_config(Some(&project_root)).provider.api_key.is_some() {
                say!("  \x1b[32m✓\x1b[0m API key detected");
            } else {
                say!("  \x1b[33m!\x1b[0m No API key found (chat and run will use a dry-run stub)");
                say!("    Set TERRA_PROVIDER_API_KEY (or DASHSCOPE_API_KEY), or run");
                say!("    `terra config set provider.api_key <key>`.");
            }

            say!();
            say!("  \x1b[1mProject initialized:\x1b[0m");
            say!("    \x1b[2m{}\x1b[0m", terra_dir.display());
            say!("    ├── constitution.toml  (sandbox + protected paths)");
            say!("    ├── config.toml        (provider settings, git-ignored)");
            say!("    ├── prompt.md          (system prompt, editable)");
            say!("    ├── memory.db          (created on first run)");
            say!("    └── audit.db           (created on first run)");
            say!();
            say!("  Next steps:");
            say!("    \x1b[32mterra chat\x1b[0m     — Start interactive dialogue");
            say!("    \x1b[32mterra doctor\x1b[0m   — Check environment");
            say!("    \x1b[32mterra run\x1b[0m      — Run a one-shot task");
            say!();
        }

        ref cmd @ (Some(Commands::Chat { .. }) | None) => {
            let use_tui = matches!(cmd, Some(Commands::Chat { tui: true }));
            use terra_agents::repl::{Repl, ReplOutput};
            use terra_agents::codebase::{ProjectContext, detect_git};
            use terra_agents::session_store::SessionStore;
            use ui::auto_renderer;
            use rustyline::error::ReadlineError;
            use rustyline::{CompletionType, Config, EditMode, Editor};

            let project_root = current_dir();
            let terra_dir = project_root.join(".terra");

            // Auto-init if needed
            if !terra_dir.exists() {
                write_project_files(&project_root).ok();
            }
            // Keep databases, history and sessions out of the user's git repo,
            // including for projects initialized before this file existed.
            if !terra_dir.join(".gitignore").exists() {
                write_terra_gitignore(&terra_dir).ok();
            }

            // First launch declaration
            let ack_path = terra_dir.join(".acknowledged");
            if !ack_path.exists() {
                say!();
                say!("\x1b[1m  Terra Agent OS — First Launch\x1b[0m");
                say!();
                say!("  Terra can:");
                say!("    \x1b[36m◆\x1b[0m Read files (anywhere) and write files (only inside this directory)");
                say!("    \x1b[36m◆\x1b[0m Execute shell commands — each one asks for your confirmation");
                say!("    \x1b[36m◆\x1b[0m Call LLM APIs (token costs apply)");
                say!("    \x1b[36m◆\x1b[0m Store conversation history and project memory in .terra/");
                say!();
                say!("  File writes and deletes are sandboxed to this directory.");
                say!("  Shell commands and deletes run only after you answer y.");
                say!();
                std::fs::write(&ack_path, "acknowledged").ok();
            }

            // EventBus: live operation lines in the REPL. The TUI owns the
            // screen, so it gets no bus (events would print over it).
            let (bus, event_rx) = if use_tui {
                (None, None)
            } else {
                let (bus, rx) = terra_types::ChannelBus::new();
                (Some(Arc::new(bus) as Arc<dyn terra_types::EventBus>), Some(rx))
            };
            let Runtime { executor, core, provider_name } = build_runtime(&project_root, bus);

            if provider_name == "dry_run" {
                say!("  \x1b[33m⚠\x1b[0m No API key — dry_run mode (replies echo your input). See `terra doctor`.");
            }

            // Project awareness
            let project_ctx = ProjectContext::scan(&project_root);
            let git_info = detect_git(&project_root);

            // Display project info (oh-my-zsh style)
            say!("  \x1b[2m📂 {}\x1b[0m", project_root.display());
            say!("  \x1b[2m🔧 {} │ {} files │ {}\x1b[0m",
                project_ctx.project_type.as_str(),
                project_ctx.total_files,
                provider_name,
            );
            if git_info.is_repo {
                let changes = if git_info.has_changes { format!(" │ {} changed", git_info.changed_files) } else { String::new() };
                say!("  \x1b[2m🌿 {}{}\x1b[0m", git_info.branch, changes);
            }
            say!();

            // Print each operation as its own line while a turn runs.
            if let Some(mut rx) = event_rx {
                tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        use terra_types::EventType;
                        match event.event_type {
                            EventType::StepStart => {
                                if let Some(desc) = event.data.get("description").and_then(|d| d.as_str()) {
                                    if !desc.contains("Thinking") && !desc.starts_with("Event:") {
                                        eprintln!("{}", ui::paint(&format!("\x1b[2m  ● {}\x1b[0m", desc.trim_start_matches(['●', ' ']))));
                                    }
                                }
                            }
                            EventType::StepFailed => {
                                eprintln!("{}", ui::paint("  \x1b[31m✗ step failed\x1b[0m"));
                            }
                            _ => {}
                        }
                    }
                });
            }

            // TUI mode: launch ratatui TUI instead of rustyline
            if use_tui {
                use terra_agents::greeter::*;
                let session_greeter = SessionGreeter::from_session_dir(&terra_dir);
                let git_greeter = GitGreeter::from_git_info(&git_info);
                let default_greeter = DefaultGreeter;
                let suggestions = collect_suggestions(
                    &[&session_greeter, &git_greeter, &default_greeter],
                    4,
                );
                let greeter_lines: Vec<String> = suggestions.iter()
                    .map(|s| format!("  {} — {}", s.label, s.hint))
                    .collect();
                tui::run_tui_async(executor, core, &provider_name, greeter_lines, &terra_dir).await?;
                return Ok(());
            }

            // L2 confirmation callback: prompts user in terminal for dangerous operations.
            let confirm_cb: terra_agents::repl::ConfirmCallback = Arc::new(|prompt: &str| {
                use std::io::{Write, BufRead};
                print!("\n  {} ", ui::paint(&format!("\x1b[33m{}\x1b[0m", prompt)));
                std::io::stdout().flush().ok();
                let mut answer = String::new();
                std::io::stdin().lock().read_line(&mut answer).ok();
                let answer = answer.trim().to_lowercase();
                answer == "y" || answer == "yes"
            });

            let mut repl = Repl::new(executor, core)
                .with_project_root(&project_root)
                .with_store(SessionStore::new(&terra_dir))
                .with_confirm_callback(confirm_cb);
            let renderer = auto_renderer(&provider_name);
            let animate = ui::color_enabled();

            // Rustyline config
            let rl_config = Config::builder()
                .completion_type(CompletionType::List)
                .edit_mode(EditMode::Emacs)
                .build();
            let mut rl: Editor<(), _> = Editor::with_config(rl_config)?;

            let history_path = terra_dir.join("history.txt");
            let _ = rl.load_history(&history_path);

            // Banner
            renderer.render_banner();

            // Smart Greeter
            {
                use terra_agents::greeter::*;
                let session_greeter = SessionGreeter::from_session_dir(&terra_dir);
                let git_greeter = GitGreeter::from_git_info(&git_info);
                let default_greeter = DefaultGreeter;
                let suggestions = collect_suggestions(
                    &[&session_greeter, &git_greeter, &default_greeter],
                    4,
                );
                let greeter_output = render_greeter(&suggestions);
                if !greeter_output.is_empty() {
                    say!("{}", greeter_output);
                    say!();
                }
            }

            loop {
                let prompt = if repl.session.mode().is_goal() { "goal > " } else { "terra > " };

                match rl.readline(prompt) {
                    Ok(line) => {
                        let _ = rl.add_history_entry(&line);

                        // Start thinking spinner for non-command input
                        let is_command = line.trim().starts_with('/');
                        let spinner = if !is_command && !line.trim().is_empty() {
                            Some(ui::Spinner::start())
                        } else {
                            None
                        };

                        let outputs = repl.process_input(&line).await;

                        // Stop spinner before rendering any output
                        if let Some(s) = &spinner {
                            s.stop();
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }

                        if Repl::should_quit(&outputs) {
                            // Exit summary
                            let tokens = repl.tokens();
                            say!();
                            say!("  \x1b[2m─────────────────────────────────\x1b[0m");
                            say!("  \x1b[2mSession: {} tokens ({} prompt, {} completion)\x1b[0m",
                                tokens.total_tokens(), tokens.prompt_tokens, tokens.completion_tokens);
                            say!("  \x1b[2mSession ID: {}\x1b[0m", repl.session_id());
                            say!();
                            say!("  Goodbye. 👋");
                            break;
                        }

                        for output in &outputs {
                            match output {
                                ReplOutput::Prompt(_) => {}
                                // Token: word-by-word display of the finished response
                                ReplOutput::Token(word) => {
                                    use std::io::Write;
                                    print!("{}", word);
                                    std::io::stdout().flush().ok();
                                    if animate {
                                        tokio::time::sleep(std::time::Duration::from_millis(8)).await;
                                    }
                                }
                                // Empty text = end of streaming response
                                ReplOutput::Text(t) if t.is_empty() => {
                                    println!();
                                }
                                // Non-empty text: render immediately (commands, errors)
                                ReplOutput::Text(text) => {
                                    ui::render_text_block(text);
                                }
                                other => renderer.render(other),
                            }
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        // Ctrl+C: cancel current, show new prompt
                        say!("\n\x1b[33m[interrupted]\x1b[0m");
                        continue;
                    }
                    Err(ReadlineError::Eof) => {
                        let tokens = repl.tokens();
                        say!("\n  \x1b[2mSession: {} tokens\x1b[0m", tokens.total_tokens());
                        say!("  Goodbye. 👋");
                        break;
                    }
                    Err(err) => {
                        eprintln!("Error: {:?}", err);
                        break;
                    }
                }
            }

            // Save history
            let _ = std::fs::create_dir_all(&terra_dir);
            let _ = rl.save_history(&history_path);

            // Lifecycle cleanup: soft-delete session-scoped memory entries
            repl.cleanup_session();
        }

        Some(Commands::Doctor { as_json }) => {
            let project_root = current_dir();
            let report = detect_environment(Some(&project_root));
            if as_json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let yes_no = |found: bool| if found { "found" } else { "not found" };
                println!("Terra doctor: environment check");
                println!("  Version:      {} ({} {})", report.terra_version, report.os, report.arch);
                println!("  Project:      {} ({})", project_root.display(),
                    if report.project_initialized { ".terra/ found" } else { "not initialized — run `terra init`" });
                println!("  Provider:     {} at {}", report.model, report.base_url);
                println!("  API key:      {}", if report.provider_configured {
                    format!("from {}", report.provider_source)
                } else {
                    "not set".into()
                });
                println!("  Config files: global {}, project {}",
                    yes_no(report.global_config_exists), yes_no(report.project_config_exists));
                if report.provider_configured {
                    println!("  Status:       ready");
                } else {
                    println!("  Status:       dry-run only — set TERRA_PROVIDER_API_KEY or `terra config set provider.api_key <key>`");
                }
            }
        }

        Some(Commands::Run { task, as_json }) => {
            let Runtime { executor, core, .. } = build_runtime(&current_dir(), None);
            let orchestrator = Orchestrator::new(executor, core);
            let result = orchestrator.run(&task).await?;

            if as_json {
                let output = serde_json::json!({
                    "spec_id": result.spec.spec_id,
                    "intent": result.spec.intent,
                    "provider": result.provider_id,
                    "worker_response": result.worker_response,
                    "ic_rules_applied": result.ic_rules_applied,
                    "cage_checks_passed": result.cage_checks_passed,
                    "audit_entries": result.audit_entry_count,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Terra run complete");
                println!("  Spec ID:    {}", result.spec.spec_id);
                println!("  Intent:     {}", result.spec.intent);
                println!("  IC Rules:   {} applied", result.ic_rules_applied);
                println!("  Pipeline:   canonical (IC → Caging → Executor)");
                println!("  Response:   {}", result.worker_response);
                println!("  Audit:      {} entries", result.audit_entry_count);
            }
        }

        Some(Commands::Trace { limit, as_json }) => {
            let audit_path = current_dir().join(".terra").join("audit.db");
            if !audit_path.exists() {
                println!("No audit log here yet ({}).", audit_path.display());
                println!("It is created the first time `terra chat`, `terra run` or `terra mcp` runs in this directory.");
                return Ok(());
            }
            let log = AuditLog::with_persistence(&audit_path)?;
            let entries = log.entries();
            let skip = if limit == 0 { 0 } else { entries.len().saturating_sub(limit) };
            let shown = &entries[skip..];
            if as_json {
                println!("{}", serde_json::to_string_pretty(shown)?);
            } else {
                println!("Audit log {} — {} entries, showing {}", audit_path.display(), entries.len(), shown.len());
                for entry in shown {
                    println!("{}", entry.summary_line());
                }
            }
        }

        Some(Commands::Config { action }) => match action {
            ConfigAction::Show => {
                let project_root = current_dir();
                let config = resolve_config(Some(&project_root));
                println!("# Effective settings (env vars > .terra/config.toml > ~/.terra/config.toml > defaults)");
                print!("{}", to_toml_string(&config)?);
            }
            ConfigAction::Set { key, value, project } => {
                if !SETTABLE_KEYS.contains(&key.as_str()) {
                    anyhow::bail!("unknown key '{}'. Supported: {}", key, SETTABLE_KEYS.join(", "));
                }
                let config_path = if project {
                    config::project_config_path(&current_dir())
                } else {
                    config::global_config_path()
                };
                config::config_set(&config_path, &key, &value)?;
                let shown = if key == "provider.api_key" { "***" } else { value.as_str() };
                println!("Set {} = {} in {}", key, shown, config_path.display());
                if let Some(var) = config::env_override_for(&key) {
                    println!("Note: {} is set in the environment and takes precedence.", var);
                }
            }
        },

        Some(Commands::Mcp) => {
            let Runtime { executor, .. } = build_runtime(&current_dir(), None);
            let server = terra_mcp::McpServer::new(executor);
            server.run_stdio().await;
        }
    }

    Ok(())
}
