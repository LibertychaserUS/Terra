use std::sync::Arc;

use clap::Parser;
use terra_agents::Orchestrator;
use terra_providers::adapter::{DryRunAdapter, ProviderAdapter};
use terra_providers::OpenAICompatibleAdapter;
use terra_trace::AuditLog;

#[derive(Parser)]
#[command(name = "terra", version, about = "Terra — Agent OS")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Check environment readiness.
    Doctor,

    /// Run a task end-to-end.
    Run {
        /// The task description in natural language.
        #[arg(short, long)]
        task: String,

        /// Force dry-run mode (no real API calls).
        #[arg(long, default_value_t = false)]
        dry_run: bool,

        /// Output as JSON for machine consumption.
        #[arg(long, default_value_t = false)]
        as_json: bool,
    },

    /// Show the audit trail.
    Trace,
}

fn resolve_provider(force_dry_run: bool) -> Arc<dyn ProviderAdapter> {
    if force_dry_run {
        return Arc::new(DryRunAdapter::new());
    }

    match OpenAICompatibleAdapter::from_env() {
        Ok(adapter) => Arc::new(adapter),
        Err(_) => {
            eprintln!("No provider API key found. Using dry_run mode.");
            eprintln!("Set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY for live mode.");
            Arc::new(DryRunAdapter::new())
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("terra=info")
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => {
            let has_key = std::env::var("TERRA_PROVIDER_API_KEY").is_ok()
                || std::env::var("DASHSCOPE_API_KEY").is_ok();
            let provider_status = if has_key { "configured" } else { "not configured (dry_run only)" };

            println!("Terra doctor: environment check");
            println!("  Version:  {}", env!("CARGO_PKG_VERSION"));
            println!("  OS:       {} {}", std::env::consts::OS, std::env::consts::ARCH);
            println!("  Provider: {}", provider_status);
            println!("  Status:   OK");
        }

        Some(Commands::Run { task, dry_run, as_json }) => {
            let provider = resolve_provider(dry_run);
            let mode_name = provider.capabilities().provider_id.clone();
            let audit_log = Arc::new(AuditLog::new());
            let orchestrator = Orchestrator::new(provider, audit_log.clone());

            let result = orchestrator.run(&task).await?;

            if as_json {
                let output = serde_json::json!({
                    "spec_id": result.spec.spec_id,
                    "intent": result.spec.intent,
                    "dispatch_mode": format!("{:?}", result.dispatch_mode),
                    "provider": mode_name,
                    "worker_response": result.worker_response,
                    "prompt_tokens": result.prompt_tokens,
                    "completion_tokens": result.completion_tokens,
                    "audit_entries": result.audit_entry_count,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Terra run complete");
                println!("  Spec ID:    {}", result.spec.spec_id);
                println!("  Intent:     {}", result.spec.intent);
                println!("  Provider:   {}", mode_name);
                println!("  Mode:       {:?}", result.dispatch_mode);
                println!("  Response:   {}", result.worker_response);
                println!("  Tokens:     {} prompt + {} completion", result.prompt_tokens, result.completion_tokens);
                println!("  Audit:      {} entries", result.audit_entry_count);
            }
        }

        Some(Commands::Trace) => {
            println!("(No persistent trace in MVP1 — trace is session-only)");
        }

        None => {
            println!("Terra v{}", env!("CARGO_PKG_VERSION"));
            println!("Agent OS — Caging Engineering inspired");
            println!();
            println!("Run `terra --help` for available commands.");
        }
    }

    Ok(())
}
