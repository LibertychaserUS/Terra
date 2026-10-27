mod config;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use terra_agents::{Agent, Output, SessionStore};
use terra_caging::engine::CagingEngine;
use terra_caging::sandbox::SandboxGuard;
use terra_caging::Constitution;
use terra_executor::{CommandExecutor, Isolation};
use terra_memory::MemoryDatabase;
use terra_providers::adapter::{DryRunAdapter, ProviderAdapter};
use terra_providers::OpenAICompatibleAdapter;
use terra_trace::{AuditLog, ChainVerdict};

use config::{detect_environment, resolve_config, to_toml_string};

/// `println!` that drops color codes when stdout is not a color terminal.
macro_rules! say {
    () => { println!() };
    ($($arg:tt)*) => { println!("{}", ui::paint(&format!($($arg)*))) };
}

#[derive(Parser)]
#[command(name = "terra", version, about = "Terra — a caged coding agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Initialize Terra in the current directory (.terra/).
    Init,
    /// Interactive session (the default).
    Chat {
        /// Run shell commands and deletes without asking.
        #[arg(long, default_value_t = false)]
        yes: bool,
    },
    /// Check environment readiness.
    Doctor {
        #[arg(long, default_value_t = false)]
        as_json: bool,
    },
    /// Run one task through the same gates as chat and exit.
    Run {
        /// The task in natural language. It has to name what it changes.
        #[arg(short, long)]
        task: String,
        /// Output as JSON.
        #[arg(long, default_value_t = false)]
        as_json: bool,
        /// Run shell commands and deletes without asking.
        #[arg(long, default_value_t = false)]
        yes: bool,
    },
    /// Show the audit trail of this project (.terra/audit.db).
    Trace {
        /// Number of most recent entries to show (0 = all).
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = false)]
        as_json: bool,
        /// Check the hash chain and print its head.
        #[arg(long, default_value_t = false)]
        verify: bool,
    },
    /// Configuration management.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Serve Terra's tools over MCP on stdio.
    Mcp,
}

#[derive(clap::Subcommand)]
enum ConfigAction {
    /// Show the effective provider configuration.
    Show,
    /// Set provider.base_url, provider.model or provider.api_key.
    Set {
        key: String,
        value: String,
        /// Write to this project's .terra/config.toml instead of ~/.terra/config.toml.
        #[arg(long, default_value_t = false)]
        project: bool,
    },
}

/// Keys `terra config set` accepts — the ones the runtime reads.
const SETTABLE_KEYS: &[&str] = &["provider.base_url", "provider.model", "provider.api_key"];

struct Runtime {
    executor: Arc<CommandExecutor>,
    provider_name: String,
    constitution: Option<Constitution>,
}

fn build_provider(project_root: &Path) -> (Arc<dyn ProviderAdapter>, String) {
    let cfg = resolve_config(Some(project_root));
    let Some(api_key) = cfg.provider.api_key else {
        return (Arc::new(DryRunAdapter::new()), "dry_run".into());
    };
    match OpenAICompatibleAdapter::new(api_key, cfg.provider.base_url, cfg.provider.model.clone(), terra_providers::openai::DEFAULT_TIMEOUT_SECS) {
        Ok(adapter) => (Arc::new(adapter), cfg.provider.model),
        Err(e) => {
            tracing::warn!("provider setup failed ({e}), using dry_run");
            (Arc::new(DryRunAdapter::new()), "dry_run".into())
        }
    }
}

/// Memory, audit, cages, isolation, provider: everything a command needs.
/// Memory and audit persist under `.terra/` when it exists.
fn build_runtime(project_root: &Path, event_bus: Option<Arc<dyn terra_types::EventBus>>) -> anyhow::Result<Runtime> {
    let terra_dir = project_root.join(".terra");
    let persistent = terra_dir.exists();
    let constitution = Constitution::load(&terra_dir.join("constitution.toml")).ok();

    let memory = match persistent.then(|| MemoryDatabase::open(&terra_dir.join("memory.db"))) {
        Some(Ok(db)) => db,
        Some(Err(e)) => {
            tracing::warn!("cannot open .terra/memory.db ({e}); memory is in-memory for this run");
            MemoryDatabase::open_in_memory()?
        }
        None => MemoryDatabase::open_in_memory()?,
    };
    let audit = match persistent.then(|| AuditLog::with_persistence(&terra_dir.join("audit.db"))) {
        Some(Ok(log)) => log,
        Some(Err(e)) => anyhow::bail!("{e}\nThe audit log is append-only and hash-chained; a break means it was edited outside Terra. Move it aside to start a new one."),
        None => AuditLog::new(),
    };

    let (sandbox, protected, require_spec, require_verify, isolation_off) = match &constitution {
        Some(c) => (
            c.to_sandbox_guard(project_root),
            c.protected_paths(project_root),
            c.require_spec_complete(),
            c.require_verification(),
            c.shell_isolation_off(),
        ),
        None => (SandboxGuard::new(project_root.to_path_buf(), vec![]), vec![], true, true, false),
    };
    let caging = CagingEngine::standard(sandbox, require_spec, require_verify);
    let isolation = if isolation_off { Isolation::None } else { Isolation::auto(project_root, &protected) };
    let (provider, provider_name) = build_provider(project_root);

    let mut executor = CommandExecutor::new(Arc::new(memory), Arc::new(audit), Arc::new(caging), provider)
        .with_project_root(project_root)
        .with_isolation(isolation);
    if let Some(bus) = event_bus {
        executor = executor.with_event_bus(bus);
    }
    Ok(Runtime { executor: Arc::new(executor), provider_name, constitution })
}

async fn connect_mcp(constitution: Option<&Constitution>, project_root: &Path) -> Option<terra_mcp::McpClients> {
    let specs: Vec<terra_mcp::ServerSpec> = constitution?
        .mcp_servers()
        .iter()
        .map(|s| terra_mcp::ServerSpec { name: s.name.clone(), command: s.command.clone(), args: s.args.clone() })
        .collect();
    if specs.is_empty() {
        return None;
    }
    Some(terra_mcp::McpClients::connect(&specs, project_root).await)
}

/// `.terra/.gitignore`: only the shareable project files are tracked.
const TERRA_GITIGNORE: &str = "# Terra runtime data (databases, history, sessions, local config).\n# Only the project rules and the system prompt are meant to be shared.\n*\n!.gitignore\n!constitution.toml\n!prompt.md\n";

const DEFAULT_PROJECT_CONFIG: &str = r#"# Provider settings for this project; unset keys fall back to ~/.terra/config.toml.
# Environment variables (TERRA_PROVIDER_API_KEY, TERRA_PROVIDER_BASE_URL,
# TERRA_PROVIDER_MODEL) take precedence. This file is git-ignored.
[provider]
# base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1"
# model = "qwen-plus"
# api_key = "..."
"#;

fn write_terra_gitignore(terra_dir: &Path) -> std::io::Result<()> {
    std::fs::write(terra_dir.join(".gitignore"), TERRA_GITIGNORE)
}

fn write_project_files(project_root: &Path) -> std::io::Result<()> {
    let terra_dir = project_root.join(".terra");
    std::fs::create_dir_all(&terra_dir)?;
    let constitution = format!(
        r#"[meta]
version = 1
description = "Terra project constitution for {}"

# Files the agent may not write or delete, even inside the project.
[[protected_paths]]
path = ".terra/constitution.toml"

[[protected_paths]]
path = ".terra/audit.db"

[sandbox]
root = "."

[rules]
require_spec_complete = true   # Goal Mode: no change before the spec has no open questions
require_verification = true    # /done needs a passing /verify after the last change
shell_isolation = "auto"       # "auto": sandbox-exec (macOS) / bwrap (Linux); "none" to turn off

# External MCP tool servers (started over stdio). Their tools appear as
# <name>__<tool> and are confirmed and audited like shell commands.
# [[mcp_servers]]
# name = "fs"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
"#,
        project_root.file_name().and_then(|n| n.to_str()).unwrap_or("project")
    );
    std::fs::write(terra_dir.join("constitution.toml"), constitution)?;
    std::fs::write(terra_dir.join("config.toml"), DEFAULT_PROJECT_CONFIG)?;
    std::fs::write(terra_dir.join("prompt.md"), include_str!("../../terra-agents/src/prompt.md"))?;
    write_terra_gitignore(&terra_dir)
}

fn current_dir() -> PathBuf {
    std::env::current_dir().expect("current directory")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // SIGPIPE: `terra trace | head` should end quietly, not panic.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "terra=warn".into()))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Init) => init(),
        Some(Commands::Chat { yes }) => chat(yes).await,
        None => chat(false).await,
        Some(Commands::Doctor { as_json }) => doctor(as_json),
        Some(Commands::Run { task, as_json, yes }) => run(&task, as_json, yes).await,
        Some(Commands::Trace { limit, as_json, verify }) => trace(limit, as_json, verify),
        Some(Commands::Config { action }) => config_cmd(action),
        Some(Commands::Mcp) => {
            let Runtime { executor, .. } = build_runtime(&current_dir(), None)?;
            terra_mcp::TerraServer::new(executor).run_stdio().await
        }
    }
}

fn init() -> anyhow::Result<()> {
    use std::io::{BufRead, Write};
    let project_root = current_dir();
    let terra_dir = project_root.join(".terra");
    say!();
    say!("\x1b[1m\x1b[35m  Terra — project setup\x1b[0m");
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
    say!("  \x1b[32m✓\x1b[0m .terra/constitution.toml  (rules: sandbox, protected paths, gates, MCP servers)");
    say!("  \x1b[32m✓\x1b[0m .terra/config.toml        (provider settings, git-ignored)");
    say!("  \x1b[32m✓\x1b[0m .terra/prompt.md          (system prompt, editable)");
    say!("  \x1b[32m✓\x1b[0m .terra/.gitignore         (runtime data stays out of git)");
    if resolve_config(Some(&project_root)).provider.api_key.is_some() {
        say!("  \x1b[32m✓\x1b[0m API key detected");
    } else {
        say!("  \x1b[33m!\x1b[0m No API key found (chat and run use a dry-run stub)");
        say!("    Set TERRA_PROVIDER_API_KEY (or DASHSCOPE_API_KEY), or run `terra config set provider.api_key <key>`.");
    }
    say!();
    say!("  Next: \x1b[32mterra chat\x1b[0m, \x1b[32mterra doctor\x1b[0m, or \x1b[32mterra run --task \"...\"\x1b[0m");
    say!();
    Ok(())
}

async fn chat(yes: bool) -> anyhow::Result<()> {
    use rustyline::error::ReadlineError;
    use rustyline::{CompletionType, Config, EditMode, Editor};

    let project_root = current_dir();
    let terra_dir = project_root.join(".terra");
    if !terra_dir.exists() {
        write_project_files(&project_root)?;
    }
    if !terra_dir.join(".gitignore").exists() {
        write_terra_gitignore(&terra_dir).ok();
    }

    let (bus, mut event_rx) = terra_types::ChannelBus::new();
    let Runtime { executor, provider_name, constitution } = build_runtime(&project_root, Some(Arc::new(bus)))?;

    ui::banner(&provider_name);
    if provider_name == "dry_run" {
        say!("  \x1b[33m⚠\x1b[0m No API key: dry_run mode (replies echo your input). See `terra doctor`.");
    }
    let git = terra_agents::codebase::detect_git(&project_root);
    let project = terra_agents::codebase::detect_project_type(&project_root);
    let branch = if git.is_repo { format!(" │ {}{}", git.branch, if git.has_changes { format!(" ({} changed)", git.changed_files) } else { String::new() }) } else { String::new() };
    say!("  \x1b[2m{} │ {}{} │ shell isolation: {}\x1b[0m", project_root.display(), project.as_str(), branch, executor.isolation().name());
    if executor.isolation().is_none() {
        say!("  \x1b[33m⚠\x1b[0m Shell commands run with your full permissions on this system.");
    }

    // Live operation lines while a turn runs.
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if event.event_type == terra_types::EventType::StepStart {
                if let Some(desc) = event.data.get("description").and_then(|d| d.as_str()) {
                    if !desc.contains("Thinking") && !desc.starts_with("Event:") {
                        eprintln!("{}", ui::paint(&format!("\x1b[2m  ● {}\x1b[0m", desc)));
                    }
                }
            }
        }
    });

    let confirm: terra_agents::ConfirmCallback = Arc::new(|prompt: &str| {
        use std::io::{BufRead, Write};
        print!("\n  {} ", ui::paint(&format!("\x1b[33m{}\x1b[0m", prompt)));
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer).ok();
        matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
    });

    let mut agent = Agent::new(executor.clone(), &project_root).with_store(SessionStore::new(&terra_dir)).with_confirm_callback(confirm);
    agent.auto_approve_shell = yes;
    agent.auto_approve_delete = yes;
    if let Some(clients) = connect_mcp(constitution.as_ref(), &project_root).await {
        say!("  \x1b[2mMCP servers: {}\x1b[0m", clients.server_names().join(", "));
        agent = agent.with_mcp(clients);
    }
    say!();

    let rl_config = Config::builder().completion_type(CompletionType::List).edit_mode(EditMode::Emacs).build();
    let mut rl: Editor<(), _> = Editor::with_config(rl_config)?;
    let history_path = terra_dir.join("history.txt");
    let _ = rl.load_history(&history_path);

    loop {
        let prompt = if agent.session.mode().is_goal() { "goal > " } else { "terra > " };
        match rl.readline(prompt) {
            Ok(line) => {
                let _ = rl.add_history_entry(&line);
                let is_command = line.trim().starts_with('/');
                let spinner = (!is_command && !line.trim().is_empty()).then(ui::Spinner::start);
                let outputs = agent.process_input(&line).await;
                if let Some(s) = &spinner {
                    s.stop();
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                if outputs.contains(&Output::Quit) {
                    break;
                }
                for output in &outputs {
                    ui::render(output);
                }
            }
            Err(ReadlineError::Interrupted) => {
                say!("\n\x1b[33m[interrupted]\x1b[0m");
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    let tokens = agent.tokens();
    say!();
    say!("  \x1b[2mSession {}: {} tokens ({} prompt, {} completion)\x1b[0m", agent.session_id(), tokens.total_tokens(), tokens.prompt_tokens, tokens.completion_tokens);
    let _ = std::fs::create_dir_all(&terra_dir);
    let _ = rl.save_history(&history_path);
    agent.cleanup_session();
    agent.shutdown().await;
    Ok(())
}

fn doctor(as_json: bool) -> anyhow::Result<()> {
    let project_root = current_dir();
    let report = detect_environment(Some(&project_root));
    let isolation = Isolation::auto(&project_root, &[]);
    if as_json {
        let mut json = serde_json::to_value(&report)?;
        json["shell_isolation"] = serde_json::json!(isolation.name());
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }
    let yes_no = |found: bool| if found { "found" } else { "not found" };
    println!("Terra doctor: environment check");
    println!("  Version:      {} ({} {})", report.terra_version, report.os, report.arch);
    println!("  Project:      {} ({})", project_root.display(), if report.project_initialized { ".terra/ found" } else { "not initialized — run `terra init`" });
    println!("  Provider:     {} at {}", report.model, report.base_url);
    println!("  API key:      {}", if report.provider_configured { format!("from {}", report.provider_source) } else { "not set".into() });
    println!("  Config files: global {}, project {}", yes_no(report.global_config_exists), yes_no(report.project_config_exists));
    println!("  Shell:        isolation {}", isolation.name());
    if report.provider_configured {
        println!("  Status:       ready");
    } else {
        println!("  Status:       dry-run only — set TERRA_PROVIDER_API_KEY or `terra config set provider.api_key <key>`");
    }
    Ok(())
}

async fn run(task: &str, as_json: bool, yes: bool) -> anyhow::Result<()> {
    let project_root = current_dir();
    let Runtime { executor, constitution, .. } = build_runtime(&project_root, None)?;
    let mut agent = Agent::new(executor, &project_root);
    if let Some(clients) = connect_mcp(constitution.as_ref(), &project_root).await {
        agent = agent.with_mcp(clients);
    }
    let report = terra_agents::run_task(&mut agent, task, yes).await;
    agent.shutdown().await;

    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if report.status == "blocked" {
        eprintln!("Terra run did not start: {}", report.response);
        eprintln!("  Intent:     {}", report.intent);
        for q in &report.open_questions {
            eprintln!("  Open:       {}", q);
        }
        eprintln!("Name what the task changes (for example a file such as src/auth.rs),");
        eprintln!("or set require_spec_complete = false in .terra/constitution.toml.");
    } else {
        println!("Terra run {}", report.status);
        println!("  Intent:        {}", report.intent);
        println!("  Provider:      {}", report.provider);
        println!("  Verification:  {}", report.verification);
        println!("  Audit entries: {}", report.audit_entries);
        println!("  Response:      {}", report.response);
    }
    if report.status == "blocked" || report.status == "error" {
        std::process::exit(1);
    }
    Ok(())
}

fn trace(limit: usize, as_json: bool, verify: bool) -> anyhow::Result<()> {
    let audit_path = current_dir().join(".terra").join("audit.db");
    if !audit_path.exists() {
        println!("No audit log here yet ({}).", audit_path.display());
        println!("It is created the first time `terra chat`, `terra run` or `terra mcp` runs in this directory.");
        return Ok(());
    }
    let log = match AuditLog::with_persistence(&audit_path) {
        Ok(log) => log,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if verify {
        match log.verify_chain() {
            ChainVerdict::Intact { events, head } => println!("Audit chain intact: {} events, head {}", events, head),
            ChainVerdict::Broken { at_id, .. } => {
                println!("Audit chain BROKEN at event {}", at_id);
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let entries = log.entries();
    let skip = if limit == 0 { 0 } else { entries.len().saturating_sub(limit) };
    let shown = &entries[skip..];
    if as_json {
        println!("{}", serde_json::to_string_pretty(shown)?);
    } else {
        println!("Audit log {} — {} entries, showing {} (chain head {})", audit_path.display(), entries.len(), shown.len(), &log.head_hash()[..12]);
        for entry in shown {
            println!("{}", entry.summary_line());
        }
    }
    Ok(())
}

fn config_cmd(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Show => {
            let config = resolve_config(Some(&current_dir()));
            println!("# Effective settings (env vars > .terra/config.toml > ~/.terra/config.toml > defaults)");
            print!("{}", to_toml_string(&config)?);
        }
        ConfigAction::Set { key, value, project } => {
            if !SETTABLE_KEYS.contains(&key.as_str()) {
                anyhow::bail!("unknown key '{}'. Supported: {}", key, SETTABLE_KEYS.join(", "));
            }
            let config_path = if project { config::project_config_path(&current_dir()) } else { config::global_config_path() };
            config::config_set(&config_path, &key, &value)?;
            let shown = if key == "provider.api_key" { "***" } else { value.as_str() };
            println!("Set {} = {} in {}", key, shown, config_path.display());
            if let Some(var) = config::env_override_for(&key) {
                println!("Note: {} is set in the environment and takes precedence.", var);
            }
        }
    }
    Ok(())
}
