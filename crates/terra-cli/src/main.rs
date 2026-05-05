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

    // L2: Memory Database
    let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());

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
        Some(Commands::Chat) => {
            use std::io::BufRead;
            use terra_agents::repl::{Repl, ReplOutput};
            #[allow(unused_imports)]
            use ui::{auto_renderer, OutputRenderer};

            // Detect provider
            let (provider, provider_name): (Arc<dyn ProviderAdapter>, String) =
                match OpenAICompatibleAdapter::from_env() {
                    Ok(adapter) => {
                        let name = adapter.capabilities().model_id.clone();
                        (Arc::new(adapter), name)
                    }
                    Err(_) => (Arc::new(terra_providers::adapter::DryRunAdapter::new()), "dry_run".into()),
                };

            // Build runtime (constitution-driven)
            let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let terra_dir = project_root.join(".terra");
            let constitution_path = terra_dir.join("constitution.toml");
            let constitution = terra_caging::Constitution::load(&constitution_path).ok();

            let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
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
                None => SandboxGuard::new(project_root, vec![]),
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

            let mut repl = Repl::new(executor, core);
            let renderer = auto_renderer(&provider_name);

            // Banner
            renderer.render_banner();

            // Initial prompt
            renderer.render(&ReplOutput::Prompt("terra > ".into()));

            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let input = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };

                let outputs = repl.process_input(&input).await;

                if Repl::should_quit(&outputs) {
                    for o in &outputs {
                        renderer.render(o);
                    }
                    break;
                }

                for output in &outputs {
                    renderer.render(output);
                }
            }
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

        None => {
            println!("Terra v{}", env!("CARGO_PKG_VERSION"));
            println!("Agent OS — Caging Engineering inspired");
            println!();
            println!("Run `terra --help` for available commands.");
        }
    }

    Ok(())
}
