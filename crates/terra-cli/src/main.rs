mod config;
mod ui;

use std::path::PathBuf;
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
    Chat,

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

    /// Show the audit trail.
    Trace,

    /// Configuration management.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(clap::Subcommand)]
enum ConfigAction {
    /// Show current configuration.
    Show,
    /// Set a configuration value.
    Set {
        key: String,
        value: String,
    },
}

/// Bootstrap the Terra runtime: Memory, Caging, IC, Executor, Orchestrator.
///
/// Loads .terra/constitution.toml if present to configure sandbox and cage rules.
/// Falls back to sensible defaults when no constitution file exists.
fn bootstrap() -> Orchestrator {
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let terra_dir = project_root.join(".terra");

    // Try loading constitution
    let constitution_path = terra_dir.join("constitution.toml");
    let constitution = terra_caging::Constitution::load(&constitution_path).ok();

    if constitution.is_some() {
        tracing::info!("Constitution loaded from {}", constitution_path.display());
    }

    // L2: Memory Database (per-project persistent storage)
    let memory = if terra_dir.exists() {
        let db_path = terra_dir.join("memory.db");
        match MemoryDatabase::open(&db_path) {
            Ok(db) => {
                tracing::info!("Memory database: {}", db_path.display());
                Arc::new(db)
            }
            Err(e) => {
                tracing::warn!("Failed to open {}: {}, falling back to in-memory", db_path.display(), e);
                Arc::new(MemoryDatabase::open_in_memory().unwrap())
            }
        }
    } else {
        Arc::new(MemoryDatabase::open_in_memory().unwrap())
    };

    // L2: Audit Trail (with persistence if .terra dir exists)
    let audit = if terra_dir.exists() {
        let audit_path = terra_dir.join("audit.db");
        match AuditLog::with_persistence(&audit_path) {
            Ok(log) => {
                tracing::info!("Audit log: persistent ({})", audit_path.display());
                Arc::new(log)
            }
            Err(_) => Arc::new(AuditLog::new()),
        }
    } else {
        Arc::new(AuditLog::new())
    };

    // L4: Caging Engine — driven by constitution or defaults
    let sandbox = match &constitution {
        Some(c) => c.to_sandbox_guard(&project_root),
        None => SandboxGuard::new(project_root.clone(), vec![]),
    };

    let caging = Arc::new(CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]));

    // L2: Artifact Store
    let artifacts = Arc::new(ArtifactStore::new(terra_dir));

    // L1: Provider
    let provider: Arc<dyn ProviderAdapter> = match OpenAICompatibleAdapter::from_env() {
        Ok(adapter) => Arc::new(adapter),
        Err(_) => Arc::new(DryRunAdapter::new()),
    };

    // L3+: Command Executor
    let executor = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));

    // L4: Intelligence Core
    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));
    let core = Arc::new(core);

    // L5: Orchestrator
    Orchestrator::new(executor, core)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("terra=info")
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init) => {
            use std::io::{BufRead, Write};

            let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let terra_dir = project_root.join(".terra");

            println!("\x1b[1m\x1b[35m");
            println!("  Terra Agent OS — Project Setup");
            println!("\x1b[0m");

            if terra_dir.exists() {
                println!("  \x1b[33m.terra/ already exists.\x1b[0m Overwrite? (y/N)");
                print!("  > ");
                std::io::stdout().flush().ok();
                let mut answer = String::new();
                std::io::stdin().lock().read_line(&mut answer).ok();
                if !answer.trim().eq_ignore_ascii_case("y") {
                    println!("  Aborted.");
                    return Ok(());
                }
            }

            // Create .terra/
            std::fs::create_dir_all(&terra_dir)?;

            // Generate constitution.toml
            let constitution_content = format!(
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
            std::fs::write(terra_dir.join("constitution.toml"), &constitution_content)?;
            println!("  \x1b[32m✓\x1b[0m Constitution written");

            // Generate config.toml
            let config_content = r#"[provider]
base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
model = "qwen-plus"

[memory]
cache_capacity = 1000

[sleep]
trigger = "session_end"
interval_hours = 24
max_candidates = 10
"#;
            std::fs::write(terra_dir.join("config.toml"), config_content)?;
            println!("  \x1b[32m✓\x1b[0m Config written");

            // Check API key
            let has_key = std::env::var("DASHSCOPE_API_KEY").is_ok()
                || std::env::var("TERRA_PROVIDER_API_KEY").is_ok();
            if has_key {
                println!("  \x1b[32m✓\x1b[0m API key detected");
            } else {
                println!("  \x1b[33m!\x1b[0m No API key found (will use dry_run mode)");
                println!("    Set DASHSCOPE_API_KEY or TERRA_PROVIDER_API_KEY in your environment.");
            }

            // Summary
            println!();
            println!("  \x1b[1mProject initialized:\x1b[0m");
            println!("    \x1b[2m{}\x1b[0m", terra_dir.display());
            println!("    ├── constitution.toml  (safety rules)");
            println!("    ├── config.toml        (provider + sleep)");
            println!("    ├── memory.db          (created on first run)");
            println!("    └── audit.db           (created on first run)");
            println!();
            println!("  Next steps:");
            println!("    \x1b[32mterra chat\x1b[0m     — Start interactive dialogue");
            println!("    \x1b[32mterra doctor\x1b[0m   — Check environment");
            println!("    \x1b[32mterra run\x1b[0m      — Run a one-shot task");
            println!();
        }

        Some(Commands::Chat) | None => {
            use terra_agents::repl::{Repl, ReplOutput};
            use terra_agents::codebase::{ProjectContext, detect_git};
            use terra_agents::session_store::SessionStore;
            #[allow(unused_imports)]
            use ui::{auto_renderer, OutputRenderer};
            use rustyline::error::ReadlineError;
            use rustyline::{CompletionType, Config, EditMode, Editor};

            let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let terra_dir = project_root.join(".terra");

            // Auto-init if needed
            if !terra_dir.exists() {
                std::fs::create_dir_all(&terra_dir).ok();
                std::fs::write(terra_dir.join("constitution.toml"),
                    "[meta]\nversion = 1\ndescription = \"Terra auto-initialized\"\n\n[[protected_paths]]\npath = \".terra/constitution.toml\"\n\n[sandbox]\nroot = \".\"\n\n[rules]\nallow_external_writes = false\nrequire_spec_complete = true\nrequire_verification = true\n"
                ).ok();
                std::fs::write(terra_dir.join("config.toml"),
                    "[provider]\nbase_url = \"https://dashscope.aliyuncs.com/compatible-mode/v1\"\nmodel = \"qwen-plus\"\n\n[memory]\ncache_capacity = 1000\n\n[sleep]\ntrigger = \"session_end\"\ninterval_hours = 24\nmax_candidates = 10\n"
                ).ok();
            }

            // First launch declaration
            let ack_path = terra_dir.join(".acknowledged");
            if !ack_path.exists() {
                println!();
                println!("\x1b[1m  Terra Agent OS — First Launch\x1b[0m");
                println!();
                println!("  Terra can:");
                println!("    \x1b[36m◆\x1b[0m Read and modify files in this directory");
                println!("    \x1b[36m◆\x1b[0m Execute shell commands");
                println!("    \x1b[36m◆\x1b[0m Call LLM APIs (token costs apply)");
                println!("    \x1b[36m◆\x1b[0m Store conversation history and project memory");
                println!();
                println!("  All operations are sandboxed to this directory.");
                println!("  Destructive operations require your confirmation.");
                println!();
                std::fs::write(&ack_path, "acknowledged").ok();
            }

            // Detect provider
            let (provider, provider_name): (Arc<dyn ProviderAdapter>, String) =
                match OpenAICompatibleAdapter::from_env() {
                    Ok(adapter) => {
                        let name = adapter.capabilities().model_id.clone();
                        (Arc::new(adapter), name)
                    }
                    Err(_) => {
                        println!("  \x1b[33m⚠\x1b[0m No API key — dry_run mode");
                        (Arc::new(terra_providers::adapter::DryRunAdapter::new()), "dry_run".into())
                    }
                };

            // Project awareness
            let project_ctx = ProjectContext::scan(&project_root);
            let git_info = detect_git(&project_root);

            // Display project info (oh-my-zsh style)
            println!("  \x1b[2m📂 {}\x1b[0m", project_root.display());
            println!("  \x1b[2m🔧 {} │ {} files │ {}\x1b[0m",
                project_ctx.project_type.as_str(),
                project_ctx.total_files,
                provider_name,
            );
            if git_info.is_repo {
                let changes = if git_info.has_changes { format!(" │ {} changed", git_info.changed_files) } else { String::new() };
                println!("  \x1b[2m🌿 {}{}\x1b[0m", git_info.branch, changes);
            }
            println!();

            // Build runtime
            let constitution = terra_caging::Constitution::load(&terra_dir.join("constitution.toml")).ok();

            let memory = if terra_dir.exists() {
                MemoryDatabase::open(&terra_dir.join("memory.db"))
                    .map(Arc::new)
                    .unwrap_or_else(|_| Arc::new(MemoryDatabase::open_in_memory().unwrap()))
            } else {
                Arc::new(MemoryDatabase::open_in_memory().unwrap())
            };
            let artifacts = Arc::new(ArtifactStore::new(terra_dir.clone()));
            let audit = if terra_dir.exists() {
                AuditLog::with_persistence(&terra_dir.join("audit.db"))
                    .map(Arc::new)
                    .unwrap_or_else(|_| Arc::new(AuditLog::new()))
            } else {
                Arc::new(AuditLog::new())
            };
            let sandbox = match &constitution {
                Some(c) => c.to_sandbox_guard(&project_root),
                None => SandboxGuard::new(project_root.clone(), vec![]),
            };
            let caging = Arc::new(CagingEngine::new(vec![
                Box::new(SpecificationCage),
                Box::new(MemoryCage),
                Box::new(ActionCage::new(sandbox)),
                Box::new(VerificationCage),
                Box::new(MetaCage),
            ]));
            let executor = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));

            let mut core = IntelligenceCore::new();
            core.register(Box::new(CodingModule));
            core.register(Box::new(AlgorithmModule));
            let core = Arc::new(core);

            let mut repl = Repl::new(executor, core)
                .with_store(SessionStore::new(&terra_dir));
            let renderer = auto_renderer(&provider_name);

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
                    println!("{}", greeter_output);
                    println!();
                }
            }

            loop {
                let prompt = if repl.session.mode().is_goal() { "goal > " } else { "terra > " };

                match rl.readline(prompt) {
                    Ok(line) => {
                        let _ = rl.add_history_entry(&line);

                        // Start spinner for non-command input (conversation)
                        let is_command = line.trim().starts_with('/');
                        let spinner_msg = if !is_command && !line.trim().is_empty() {
                            let short = if line.len() > 30 { format!("{}...", &line.trim()[..27]) } else { line.trim().to_string() };
                            Some(short)
                        } else {
                            None
                        };
                        let spinner = spinner_msg.as_ref().map(|msg| ui::Spinner::start(msg));

                        let outputs = repl.process_input(&line).await;

                        // Stop spinner
                        if let Some(s) = &spinner {
                            s.stop();
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }

                        if Repl::should_quit(&outputs) {
                            // Exit summary
                            let tokens = repl.tokens();
                            println!();
                            println!("  \x1b[2m─────────────────────────────────\x1b[0m");
                            println!("  \x1b[2mSession: {} tokens ({} prompt, {} completion)\x1b[0m",
                                tokens.total_tokens(), tokens.prompt_tokens, tokens.completion_tokens);
                            println!("  \x1b[2mSession ID: {}\x1b[0m", repl.session_id());
                            println!();
                            println!("  Goodbye. 👋");
                            break;
                        }

                        for output in &outputs {
                            match output {
                                ReplOutput::Prompt(_) => {}
                                // Skip "Thinking" notifications — spinner already showed it
                                ReplOutput::Notification(msg) if (msg.contains("Thinking") || msg.contains("Analyzing") || msg.contains("Processing")) && spinner.is_some() => {}
                                other => renderer.render(other),
                            }
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        // Ctrl+C: cancel current, show new prompt
                        println!("\n\x1b[33m[interrupted]\x1b[0m");
                        continue;
                    }
                    Err(ReadlineError::Eof) => {
                        let tokens = repl.tokens();
                        println!("\n  \x1b[2mSession: {} tokens\x1b[0m", tokens.total_tokens());
                        println!("  Goodbye. 👋");
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
        }

        Some(Commands::Doctor { as_json }) => {
            let report = detect_environment(None);
            if as_json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Terra doctor: environment check");
                println!("  Version:    {}", report.terra_version);
                println!("  OS:         {} {}", report.os, report.arch);
                println!("  Provider:   {}", if report.provider_configured {
                    format!("configured ({})", report.provider_source)
                } else {
                    "not configured (dry_run only)".into()
                });
                println!("  Global cfg: {}", if report.global_config_exists { "found" } else { "not found" });
                println!("  Project cfg:{}", if report.project_config_exists { "found" } else { "not found" });
                println!("  Status:     OK");
            }
        }

        Some(Commands::Run { task, as_json }) => {
            let orchestrator = bootstrap();
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

        Some(Commands::Trace) => {
            println!("(Trace is session-scoped — view via terra trace show)");
        }

        Some(Commands::Config { action }) => match action {
            ConfigAction::Show => {
                let config = resolve_config(None);
                print!("{}", to_toml_string(&config)?);
            }
            ConfigAction::Set { key, value } => {
                let config_path = config::global_config_path();
                config::config_set(&config_path, &key, &value)?;
                println!("Set {} = {} in {}", key, value, config_path.display());
            }
        },
    }

    Ok(())
}
