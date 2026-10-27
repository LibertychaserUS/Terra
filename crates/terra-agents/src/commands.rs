//! Slash commands: the user's side of the session. They cost no tokens
//! except where they say so (`/goal` specifies with the model, `/done` asks
//! it for consolidation candidates, `/commit` asks it for a message).

use terra_types::command::{Command, CommandOp, CommandStatus};
use terra_types::text::truncate_ellipsis;

use crate::agent::{Agent, Output};
use crate::codebase;
use crate::consolidate;
use crate::session::VERIFIER_AGENT;

/// Time limit for a `/verify` command (test suites can be slow).
const VERIFY_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const VERIFY_OUTPUT_LINES: usize = 8;
const MAX_CANDIDATES: usize = 3;

const HELP: &str = "\
Commands:
  /goal \"task\"      Enter Goal Mode: the task is specified first; while it has open
                    questions nothing in the project may change. /goal again adds detail.
  /done             Claim the goal is complete (needs a passing /verify after the last change),
                    then review consolidation candidates with /accept N or /reject N
  /abandon          Leave Goal Mode without a completion claim
  /verify [cmd]     Run the project's test command (or cmd) as the verifier
  /status           Mode, goal, verification state, tokens
  /memory           List L1 project memory
  /remember \"text\"  Save text to L1 project memory
  /trace [n]        Show the last n audit entries (default 10)
  /sessions         List saved sessions
  /compact          Summarize older messages to free context
  /commit           Stage all changes and commit with a model-written message (asks first)
  /undo             Discard unstaged edits to tracked files (asks first)
  /clear            Clear the conversation context
  /help             This list
  /quit             Exit";

fn strip_quotes(arg: &str) -> &str {
    let t = arg.trim();
    if t.len() >= 2 && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\''))) {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

fn text(s: impl Into<String>) -> Vec<Output> {
    vec![Output::Text(s.into())]
}

fn notice(s: impl Into<String>) -> Vec<Output> {
    vec![Output::Notice(s.into())]
}

pub async fn handle(agent: &mut Agent, input: &str) -> Vec<Output> {
    let (raw_cmd, rest) = input.split_once(' ').unwrap_or((input, ""));
    let arg = strip_quotes(rest);
    let cmd = match raw_cmd {
        "/q" | "/exit" => "/quit",
        "/s" => "/status",
        "/h" | "/?" => "/help",
        "/g" => "/goal",
        "/d" => "/done",
        "/c" => "/commit",
        other => other,
    };
    match cmd {
        "/help" => text(HELP),
        "/quit" => vec![Output::Quit],
        "/goal" => goal(agent, arg).await,
        "/done" => done(agent).await,
        "/abandon" => match agent.session.abandon_goal().await {
            Ok(()) => notice("Goal abandoned (not claimed complete). Back to Conversation."),
            Err(e) => text(e),
        },
        "/verify" => verify(agent, arg).await,
        "/status" => status(agent),
        "/memory" => memory(agent),
        "/remember" => remember(agent, arg).await,
        "/accept" => verdict(agent, arg, true).await,
        "/reject" => verdict(agent, arg, false).await,
        "/trace" => trace(agent, arg),
        "/sessions" => sessions(agent),
        "/compact" => match agent.compact().await {
            Some((old, summary)) => notice(format!("Compacted {} older messages. Summary: {}", old, truncate_ellipsis(&summary, 200))),
            None => notice("Nothing to compact yet."),
        },
        "/clear" => {
            agent.clear_context();
            notice("Context cleared.")
        }
        "/commit" => commit(agent).await,
        "/undo" => undo(agent),
        other => text(format!("Unknown command: {}. Type /help for the list.", other)),
    }
}

async fn goal(agent: &mut Agent, arg: &str) -> Vec<Output> {
    if arg.is_empty() {
        return text("Usage: /goal \"task description\"");
    }
    let result = if agent.session.mode().is_goal() { agent.session.refine_goal(arg).await } else { agent.session.enter_goal(arg).await };
    match result {
        Ok(spec) => {
            let mut out = vec![Output::Notice(format!("Goal Mode: {}", spec.intent))];
            if spec.is_complete() {
                out.push(Output::Text("Specification complete; file, shell and delete actions are allowed.".into()));
                if !spec.success_criteria.is_empty() {
                    out.push(Output::Text(format!("Done when: {}", spec.success_criteria.join("; "))));
                }
            } else {
                out.push(Output::Text("Open questions (nothing in the project may change until they are answered; answer with /goal \"...\"):".into()));
                out.extend(spec.open_questions.iter().map(|q| Output::Text(format!("  - {}", q))));
            }
            out
        }
        Err(e) => text(format!("Error: {}", e)),
    }
}

async fn done(agent: &mut Agent) -> Vec<Output> {
    let Some(verification) = agent.session.verification() else {
        return text("Not in Goal Mode. Use /goal first.");
    };
    let goal = agent.session.spec().map(|s| s.intent.clone()).unwrap_or_default();
    let entries = agent.session.goal_entries();
    match agent.session.complete_goal().await {
        Ok(_) => {
            let mut out = vec![Output::Notice(format!("Goal complete ({}). Back to Conversation.", verification.describe()))];
            let report = consolidate::consolidate(agent.executor(), &goal, &entries, MAX_CANDIDATES).await;
            if report.candidates.is_empty() {
                out.push(Output::Text(format!("Sleep consolidation: nothing to keep from {} actions.", report.action_count)));
            } else {
                out.push(Output::Text(format!("Sleep consolidation: {} candidates from {} actions", report.candidates.len(), report.action_count)));
                for (i, c) in report.candidates.iter().enumerate() {
                    out.push(Output::Text(format!("  [{}] {}", i, c.principle)));
                    if !c.evidence.is_empty() {
                        out.push(Output::Text(format!("      evidence: {}", c.evidence)));
                    }
                }
                out.push(Output::Text("Use /accept N to keep one as L1 project memory, /reject N to drop it.".into()));
                agent.pending = Some(report);
            }
            out
        }
        Err(reason) => vec![
            Output::Notice(format!("Goal not closed: {}", reason)),
            Output::Text(format!("  {}", verification.describe())),
            Output::Text("  Run /verify [command] to check the work, or /abandon to leave Goal Mode without claiming it is done.".into()),
        ],
    }
}

async fn verify(agent: &mut Agent, arg: &str) -> Vec<Output> {
    let command = if arg.is_empty() {
        match codebase::detect_project_type(agent.project_root()).test_command() {
            Some(c) => c.to_string(),
            None => return text("Usage: /verify <command> (no test command detected for this project)"),
        }
    } else {
        arg.to_string()
    };
    let cmd = Command::new(VERIFIER_AGENT, CommandOp::ShellExec { command: command.clone(), workdir: None, timeout_ms: Some(VERIFY_TIMEOUT_MS) });
    let result = agent.executor().execute(cmd).await;
    let mut out = Vec::new();
    match result.status {
        CommandStatus::Ok => out.push(Output::Notice(format!("/verify passed: `{}`", command))),
        CommandStatus::PermissionDenied => out.push(Output::Notice(format!("/verify blocked: {}", result.error.clone().unwrap_or_default()))),
        _ => out.push(Output::Notice(format!("/verify failed: `{}`", command))),
    }
    if let Some(data) = &result.data {
        let output = [data["stdout"].as_str().unwrap_or(""), data["stderr"].as_str().unwrap_or("")].join("\n");
        let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
        let skip = lines.len().saturating_sub(VERIFY_OUTPUT_LINES);
        out.extend(lines[skip..].iter().map(|l| Output::Text(format!("  {}", l))));
    } else if let Some(err) = result.error {
        out.push(Output::Text(format!("  {}", err)));
    }
    if let Some(v) = agent.session.verification() {
        out.push(Output::Text(format!("  Goal state: {}", v.describe())));
    }
    out
}

fn status(agent: &Agent) -> Vec<Output> {
    let mut out = Vec::new();
    match agent.session.spec() {
        Some(spec) => {
            out.push(Output::Text(format!("Mode: goal — {}", spec.intent)));
            out.push(Output::Text(format!("Spec: {}", if spec.is_complete() { "complete".to_string() } else { format!("{} open questions", spec.open_questions.len()) })));
            if let Some(v) = agent.session.verification() {
                out.push(Output::Text(format!("Verification: {}", v.describe())));
            }
        }
        None => out.push(Output::Text("Mode: conversation".into())),
    }
    let t = agent.tokens();
    out.push(Output::Text(format!("Tokens: {} ({} prompt, {} completion) over {} calls", t.total_tokens(), t.prompt_tokens, t.completion_tokens, t.total_calls)));
    out.push(Output::Text(format!("Context: {} messages", agent.message_count())));
    out.push(Output::Text(format!("Shell isolation: {}", agent.executor().isolation().name())));
    out.push(Output::Text(format!("Tools: {}", agent.tool_names().join(", "))));
    out
}

fn memory(agent: &Agent) -> Vec<Output> {
    match agent.executor().memory().list_level(terra_memory::Level::L1) {
        Ok(entries) if entries.is_empty() => text("No L1 project memory yet. Use /remember or /accept."),
        Ok(entries) => {
            let mut out = vec![Output::Text(format!("L1 project memory ({} entries):", entries.len()))];
            out.extend(entries.iter().map(|e| Output::Text(format!("  {} — {}", e.key, truncate_ellipsis(&e.content.replace('\n', " "), 120)))));
            out
        }
        Err(e) => text(format!("Error: {}", e)),
    }
}

async fn remember(agent: &mut Agent, arg: &str) -> Vec<Output> {
    if arg.is_empty() {
        return text("Usage: /remember \"content to remember\"");
    }
    let key = format!("user:{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let cmd = Command::new("user", CommandOp::MemWrite { key, content: arg.to_string(), scope: "project".into(), level: "L1".into() });
    let result = agent.executor().execute(cmd).await;
    if result.status == CommandStatus::Ok {
        notice(format!("Remembered: {}", arg))
    } else {
        text(format!("Failed: {}", result.error.unwrap_or_default()))
    }
}

async fn verdict(agent: &mut Agent, arg: &str, accepted: bool) -> Vec<Output> {
    let Ok(idx) = arg.parse::<usize>() else {
        return text(format!("Usage: /{} N (candidate index)", if accepted { "accept" } else { "reject" }));
    };
    let Some(candidate) = agent.pending.as_ref().and_then(|r| r.candidates.get(idx)).cloned() else {
        return text("No such candidate. Candidates are listed after /done.");
    };
    agent.session.audit_event("sleep_verdict", serde_json::json!({ "candidate": candidate.principle, "accepted": accepted })).await;
    if !accepted {
        return notice(format!("Candidate #{} rejected.", idx));
    }
    let key = format!("promoted:{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let cmd = Command::new("user", CommandOp::MemWrite { key, content: candidate.principle.clone(), scope: "project".into(), level: "L1".into() });
    let result = agent.executor().execute(cmd).await;
    if result.status == CommandStatus::Ok {
        notice(format!("Candidate #{} accepted → L1: \"{}\"", idx, truncate_ellipsis(&candidate.principle, 80)))
    } else {
        text(format!("Failed: {}", result.error.unwrap_or_default()))
    }
}

fn trace(agent: &Agent, arg: &str) -> Vec<Output> {
    let n = arg.parse().unwrap_or(10);
    let entries = agent.executor().audit().entries();
    if entries.is_empty() {
        return text("Audit log is empty.");
    }
    let skip = entries.len().saturating_sub(n);
    let mut out = vec![Output::Text(format!("Last {} of {} audit entries:", entries.len() - skip, entries.len()))];
    out.extend(entries[skip..].iter().map(|e| Output::Text(e.summary_line())));
    out.push(Output::Text(format!("Chain head: {}", &agent.executor().audit().head_hash()[..16])));
    out
}

fn sessions(agent: &Agent) -> Vec<Output> {
    let Some(store) = agent.store() else {
        return text("No session store (run terra init first).");
    };
    let list = store.list();
    if list.is_empty() {
        return text("No previous sessions.");
    }
    let mut out = vec![Output::Text("Sessions:".into())];
    for s in &list {
        out.push(Output::Text(format!(
            "  {} — {} turns, {} tokens{}",
            s.id,
            s.turn_count,
            s.total_tokens,
            s.goal.as_ref().map(|g| format!(" ({})", g)).unwrap_or_default(),
        )));
    }
    out
}

async fn commit(agent: &mut Agent) -> Vec<Output> {
    let root = agent.project_root().to_path_buf();
    let git = codebase::detect_git(&root);
    if !git.is_repo {
        return text("Not a git repository.");
    }
    if !git.has_changes {
        return text("Nothing to commit.");
    }
    let files = match codebase::git_unstaged_files(&root) {
        Ok(f) => f,
        Err(e) => return text(format!("git error: {}", e)),
    };
    let diff: String = files.iter().filter_map(|f| codebase::git_diff_file(&root, f)).collect::<Vec<_>>().join("\n");
    let cmd = Command::new(
        "main",
        CommandOp::ProviderCall {
            system_prompt: "Write a git commit message for this diff: a short imperative subject line, then an optional short body. Output only the message.".into(),
            user_prompt: truncate_ellipsis(&diff, 12_000),
            max_tokens: Some(300),
            temperature: Some(0.2),
            json: false,
        },
    );
    let result = agent.executor().execute(cmd).await;
    let message = result
        .data
        .and_then(|d| d.get("content").and_then(|c| c.as_str()).map(|s| s.trim().to_string()))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "Update files".into());
    let prompt = format!("Commit {} file(s) with message \"{}\"? [y/N]", files.len(), truncate_ellipsis(&message, 60));
    if !agent.confirm(&prompt) {
        return notice("Commit cancelled.");
    }
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    match codebase::git_commit(&root, &refs, &message) {
        Ok(out) => notice(format!("Committed: {}", out.lines().next().unwrap_or("").trim())),
        Err(e) => text(format!("git commit failed: {}", e)),
    }
}

fn undo(agent: &mut Agent) -> Vec<Output> {
    let root = agent.project_root().to_path_buf();
    let files = match codebase::git_unstaged_files(&root) {
        Ok(f) if !f.is_empty() => f,
        Ok(_) => return text("No unstaged changes."),
        Err(e) => return text(format!("git error: {}", e)),
    };
    let mut out: Vec<Output> = files.iter().map(|f| Output::Text(format!("  {}", f))).collect();
    if !agent.confirm(&format!("Discard unstaged edits to {} tracked file(s)? [y/N]", files.len())) {
        out.push(Output::Notice("Undo cancelled.".into()));
        return out;
    }
    let status = std::process::Command::new("git").args(["checkout", "--", "."]).current_dir(&root).status();
    match status {
        Ok(s) if s.success() => out.push(Output::Notice("Unstaged edits discarded.".into())),
        _ => out.push(Output::Text("git checkout failed.".into())),
    }
    out
}
