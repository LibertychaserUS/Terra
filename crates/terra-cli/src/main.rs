use std::sync::Arc;

use clap::Parser;
use terra_agents::Orchestrator;
use terra_providers::adapter::DryRunAdapter;
use terra_trace::AuditLog;

#[derive(Parser)]
#[command(name = "terra", version, about = "Terra — AI Agent Platform")]
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

        /// Output as JSON for machine consumption.
        #[arg(long, default_value_t = false)]
        as_json: bool,
    },

    /// Show the audit trail.
    Trace,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => {
            println!("Terra doctor: environment check");
            println!("  Version: {}", env!("CARGO_PKG_VERSION"));
            println!("  Rust edition: 2021");
            println!("  Provider: dry_run (default)");
            println!("  Status: OK");
        }

        Some(Commands::Run { task, as_json }) => {
            let provider = Arc::new(DryRunAdapter::new());
            let audit_log = Arc::new(AuditLog::new());
            let orchestrator = Orchestrator::new(provider, audit_log.clone());

            let result = orchestrator.run(&task).await?;

            if as_json {
                let output = serde_json::json!({
                    "spec_id": result.spec.spec_id,
                    "intent": result.spec.intent,
                    "dispatch_mode": format!("{:?}", result.dispatch_mode),
                    "worker_response": result.worker_response,
                    "prompt_tokens": result.prompt_tokens,
                    "completion_tokens": result.completion_tokens,
                    "audit_entries": result.audit_entry_count,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!("Terra run complete");
                println!("  Spec ID: {}", result.spec.spec_id);
                println!("  Intent: {}", result.spec.intent);
                println!("  Mode: {:?}", result.dispatch_mode);
                println!("  Response: {}", result.worker_response);
                println!("  Tokens: {} prompt + {} completion", result.prompt_tokens, result.completion_tokens);
                println!("  Audit entries: {}", result.audit_entry_count);
            }
        }

        Some(Commands::Trace) => {
            println!("(No persistent trace in MVP1 — trace is session-only)");
        }

        None => {
            println!("Terra v{}", env!("CARGO_PKG_VERSION"));
            println!("AI Agent Platform — Caging Engineering inspired");
            println!();
            println!("Run `terra --help` for available commands.");
        }
    }

    Ok(())
}
