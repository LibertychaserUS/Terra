use std::ffi::OsStr;
use std::path::Path;

use crate::cage::{Cage, CageAction, CageCheckResult, CageContext};
use crate::sandbox::{resolve_path, SandboxGuard};

/// Specification Cage: nothing changes while the task is not specified.
///
/// In Goal Mode, every write, delete or shell command is blocked until the
/// goal's specification has no open questions. Starting a task
/// (`ExecuteTask`) with an incomplete spec is blocked in the same way.
pub struct SpecificationCage;

impl Cage for SpecificationCage {
    fn name(&self) -> &str {
        "specification"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        match &context.action {
            CageAction::ExecuteTask { spec_complete } if !spec_complete => CageCheckResult::Block(
                "task specification is incomplete (has open questions)".into(),
            ),
            action if action.mutates() && !context.spec_complete => CageCheckResult::Block(
                "the goal's specification still has open questions; answer them before changing anything".into(),
            ),
            _ => CageCheckResult::Pass,
        }
    }

    fn goal_only(&self) -> bool {
        true
    }
}

/// Memory Cage: ownership plus level-based write permissions.
///
/// - L0 is read-only for everyone.
/// - L1 accepts only user commands (`/remember`, `/accept`).
/// - L3 is written in Goal Mode only.
/// - L2, L4, L5, L6 are open to the agent.
pub struct MemoryCage;

impl Cage for MemoryCage {
    fn name(&self) -> &str {
        "memory"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::MemoryWrite { owner, level, is_user_command, .. } = &context.action {
            if *owner != context.agent && !is_user_command {
                return CageCheckResult::Block(format!("{} is not the owner (owner: {})", context.agent, owner));
            }
            match level.as_str() {
                "L0" => return CageCheckResult::Block("L0 (authoritative) is read-only".into()),
                "L1" if !is_user_command => {
                    return CageCheckResult::Block("L1 (project) requires user /remember or /accept".into())
                }
                "L3" if !context.goal_mode => {
                    return CageCheckResult::Block("L3 (accepted paths) only writable in Goal Mode".into())
                }
                _ => {}
            }
        }
        CageCheckResult::Pass
    }
}

/// Action Cage: file writes and deletes stay inside the sandbox; a short
/// list of destructive shell patterns is refused outright.
///
/// The pattern list is a convenience, not the mechanism: the OS-level
/// isolation the executor applies to shell commands is what keeps them
/// inside the project.
pub struct ActionCage {
    sandbox: SandboxGuard,
}

impl ActionCage {
    pub fn new(sandbox: SandboxGuard) -> Self {
        Self { sandbox }
    }
}

const DESTRUCTIVE_SHELL: &[&str] = &["rm -rf /", "rm -fr /", "mkfs", "> /dev/", "dd if=", ":(){"];

impl Cage for ActionCage {
    fn name(&self) -> &str {
        "action"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        match &context.action {
            CageAction::FileWrite { path } => match self.sandbox.check_write(path) {
                Ok(()) => CageCheckResult::Pass,
                Err(e) => CageCheckResult::Block(e.to_string()),
            },
            CageAction::FileDelete { path } => match self.sandbox.check_delete(path) {
                Ok(()) => CageCheckResult::Pass,
                Err(e) => CageCheckResult::Block(e.to_string()),
            },
            CageAction::FileRead { path } => match self.sandbox.check_read(path) {
                Ok(()) => CageCheckResult::Pass,
                Err(e) => CageCheckResult::Block(e.to_string()),
            },
            CageAction::ShellExec { command } => {
                let lower = command.to_lowercase();
                if DESTRUCTIVE_SHELL.iter().any(|p| lower.contains(p)) {
                    CageCheckResult::Block(format!("destructive shell command blocked: {}", command))
                } else {
                    CageCheckResult::Pass
                }
            }
            _ => CageCheckResult::Pass,
        }
    }
}

/// Verification Cage: a completion claim needs verification the agent did
/// not produce itself.
pub struct VerificationCage;

impl Cage for VerificationCage {
    fn name(&self) -> &str {
        "verification"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::ClaimComplete { verified: false } = &context.action {
            return CageCheckResult::Block("completion claimed without external verification".into());
        }
        CageCheckResult::Pass
    }

    fn goal_only(&self) -> bool {
        true
    }
}

/// Meta-Cage: the agent cannot change the rules that cage it.
///
/// Blocks `ModifyRule`, and any write or delete that resolves to the project
/// constitution (`.terra/constitution.toml`) or a path beneath it, whether or
/// not the constitution lists itself as protected.
pub struct MetaCage;

impl MetaCage {
    /// Whether `path`, once `..` and symlinks are resolved, is the constitution
    /// or lies beneath it. Case is ignored: macOS file systems usually do.
    fn is_rule_file(path: &Path) -> bool {
        let resolved = resolve_path(path);
        let names: Vec<&OsStr> = resolved.components().map(|c| c.as_os_str()).collect();
        names.windows(2).any(|pair| {
            pair[0].eq_ignore_ascii_case(".terra") && pair[1].eq_ignore_ascii_case("constitution.toml")
        })
    }
}

impl Cage for MetaCage {
    fn name(&self) -> &str {
        "meta"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        match &context.action {
            CageAction::ModifyRule { rule_name } => {
                CageCheckResult::Block(format!("cannot modify protected rule: {}", rule_name))
            }
            CageAction::FileWrite { path } | CageAction::FileDelete { path } if Self::is_rule_file(path) => {
                CageCheckResult::Block(format!("cannot modify protected rule file: {}", path.display()))
            }
            _ => CageCheckResult::Pass,
        }
    }
}

/// Runs the cages in sequence. The first `Block` stops the action; warnings
/// are collected and do not stop it.
pub struct CagingEngine {
    cages: Vec<Box<dyn Cage>>,
}

/// Result of running all cages.
#[derive(Debug)]
pub struct CagingResult {
    pub allowed: bool,
    pub warnings: Vec<(String, String)>,
    pub block: Option<(String, String)>,
}

impl CagingEngine {
    pub fn new(cages: Vec<Box<dyn Cage>>) -> Self {
        Self { cages }
    }

    /// The standard cage set, in the order they run: Specification, Memory,
    /// Meta, Action, Verification. Meta runs before Action so a write to the
    /// constitution is reported as a rule change.
    pub fn standard(sandbox: SandboxGuard, require_spec_complete: bool, require_verification: bool) -> Self {
        let mut cages: Vec<Box<dyn Cage>> = Vec::new();
        if require_spec_complete {
            cages.push(Box::new(SpecificationCage));
        }
        cages.push(Box::new(MemoryCage));
        cages.push(Box::new(MetaCage));
        cages.push(Box::new(ActionCage::new(sandbox)));
        if require_verification {
            cages.push(Box::new(VerificationCage));
        }
        Self::new(cages)
    }

    /// Names of the registered cages, in order.
    pub fn names(&self) -> Vec<&str> {
        self.cages.iter().map(|c| c.name()).collect()
    }

    /// Run all cages against the given context.
    pub fn check(&self, context: &CageContext) -> CagingResult {
        let mut warnings = Vec::new();
        for cage in &self.cages {
            if cage.goal_only() && !context.goal_mode {
                continue;
            }
            match cage.check(context) {
                CageCheckResult::Pass => {}
                CageCheckResult::Warn(msg) => warnings.push((cage.name().to_string(), msg)),
                CageCheckResult::Block(msg) => {
                    return CagingResult { allowed: false, warnings, block: Some((cage.name().to_string(), msg)) };
                }
            }
        }
        CagingResult { allowed: true, warnings, block: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_engine() -> CagingEngine {
        let sandbox = SandboxGuard::new(
            PathBuf::from("/projects/myapp"),
            vec![PathBuf::from("/projects/myapp/.terra/constitution.toml")],
        );
        CagingEngine::standard(sandbox, true, true)
    }

    fn goal(action: CageAction) -> CageContext {
        CageContext::goal("main", action)
    }

    fn blocked_by(result: &CagingResult) -> &str {
        &result.block.as_ref().expect("should be blocked").0
    }

    #[test]
    fn standard_order() {
        assert_eq!(test_engine().names(), ["specification", "memory", "meta", "action", "verification"]);
        let sandbox = SandboxGuard::new(PathBuf::from("/p"), vec![]);
        assert_eq!(CagingEngine::standard(sandbox, false, false).names(), ["memory", "meta", "action"]);
    }

    #[test]
    fn spec_cage_gates_task_start() {
        let engine = test_engine();
        assert!(engine.check(&goal(CageAction::ExecuteTask { spec_complete: true })).allowed);
        let result = engine.check(&goal(CageAction::ExecuteTask { spec_complete: false }));
        assert_eq!(blocked_by(&result), "specification");
    }

    #[test]
    fn spec_cage_blocks_changes_while_goal_has_open_questions() {
        let engine = test_engine();
        let write = CageAction::FileWrite { path: PathBuf::from("/projects/myapp/src/main.rs") };
        let mut ctx = goal(write.clone());
        ctx.spec_complete = false;
        assert_eq!(blocked_by(&engine.check(&ctx)), "specification");

        ctx.action = CageAction::ShellExec { command: "cargo test".into() };
        assert_eq!(blocked_by(&engine.check(&ctx)), "specification");

        // Reads and memory notes are still fine: the agent may investigate.
        ctx.action = CageAction::FileRead { path: PathBuf::from("/projects/myapp/src/main.rs") };
        assert!(engine.check(&ctx).allowed);

        // A complete spec lifts the gate; conversation mode never has it.
        assert!(engine.check(&goal(write.clone())).allowed);
        let mut conv = CageContext::conversation("main", write);
        conv.spec_complete = false;
        assert!(engine.check(&conv).allowed);
    }

    #[test]
    fn memory_cage_ownership_and_levels() {
        let engine = test_engine();
        let write = |agent: &str, owner: &str, level: &str, user: bool, goal_mode: bool| CageContext {
            agent: agent.into(),
            action: CageAction::MemoryWrite {
                key: "k".into(),
                owner: owner.into(),
                level: level.into(),
                is_user_command: user,
            },
            goal_mode,
            spec_complete: true,
        };
        assert!(engine.check(&write("main", "main", "L2", false, true)).allowed);
        assert_eq!(blocked_by(&engine.check(&write("worker", "main", "L2", false, true))), "memory");
        assert!(!engine.check(&write("main", "main", "L0", false, true)).allowed);
        assert!(!engine.check(&write("user", "user", "L0", true, true)).allowed, "even the user cannot write L0");
        assert!(!engine.check(&write("worker", "worker", "L1", false, true)).allowed);
        assert!(engine.check(&write("user", "user", "L1", true, false)).allowed);
        assert!(!engine.check(&write("main", "main", "L3", false, false)).allowed);
        assert!(engine.check(&write("main", "main", "L3", false, true)).allowed);
        for level in ["L2", "L4", "L5", "L6"] {
            assert!(engine.check(&write("main", "main", level, false, false)).allowed, "{level}");
        }
    }

    #[test]
    fn action_cage_sandbox_and_shell_patterns() {
        let engine = test_engine();
        let outside = engine.check(&goal(CageAction::FileWrite { path: PathBuf::from("/etc/passwd") }));
        assert_eq!(blocked_by(&outside), "action");
        assert!(engine.check(&goal(CageAction::FileRead { path: PathBuf::from("/etc/hosts") })).allowed);
        assert!(engine.check(&goal(CageAction::ShellExec { command: "cargo test".into() })).allowed);
        for cmd in ["rm -rf /", "sudo mkfs.ext4 /dev/sda", "echo x > /dev/sda", "dd if=/dev/zero of=/dev/sda"] {
            let result = engine.check(&goal(CageAction::ShellExec { command: cmd.into() }));
            assert_eq!(blocked_by(&result), "action", "{cmd}");
        }
    }

    #[test]
    fn verification_cage_blocks_unverified_claims_in_goal_mode_only() {
        let engine = test_engine();
        assert_eq!(blocked_by(&engine.check(&goal(CageAction::ClaimComplete { verified: false }))), "verification");
        assert!(engine.check(&goal(CageAction::ClaimComplete { verified: true })).allowed);
        assert!(engine.check(&CageContext::conversation("main", CageAction::ClaimComplete { verified: false })).allowed);
    }

    #[test]
    fn meta_cage_blocks_rule_changes_in_every_mode() {
        let engine = test_engine();
        let rule = CageAction::ModifyRule { rule_name: "sandbox.root".into() };
        assert_eq!(blocked_by(&engine.check(&goal(rule.clone()))), "meta");
        assert_eq!(blocked_by(&engine.check(&CageContext::conversation("main", rule))), "meta");
        let write = CageAction::FileWrite { path: PathBuf::from("/projects/myapp/.terra/constitution.toml") };
        assert_eq!(blocked_by(&engine.check(&goal(write))), "meta", "meta runs before action");
    }

    fn meta_check(action: CageAction) -> CageCheckResult {
        MetaCage.check(&CageContext::conversation("main", action))
    }

    #[test]
    fn meta_cage_resolves_dots_and_case() {
        for path in [
            "/projects/myapp/.terra/constitution.toml",
            "/projects/myapp/.terra/../.terra/constitution.toml",
            "/projects/myapp/src/../.terra/./constitution.toml",
            "/projects/myapp/.TERRA/Constitution.toml",
            "/projects/myapp/.terra/constitution.toml/shadow",
            ".terra/constitution.toml",
        ] {
            let path = PathBuf::from(path);
            assert!(meta_check(CageAction::FileWrite { path: path.clone() }).is_blocked(), "{}", path.display());
            assert!(meta_check(CageAction::FileDelete { path }).is_blocked());
        }
        for path in [
            "/projects/myapp/.terra/prompt.md",
            "/projects/myapp/.terra/constitution.toml.bak",
            "/projects/myapp/src/constitution.toml",
        ] {
            assert!(meta_check(CageAction::FileWrite { path: PathBuf::from(path) }).is_pass(), "{path}");
        }
        assert!(meta_check(CageAction::FileRead { path: PathBuf::from("/projects/myapp/.terra/constitution.toml") }).is_pass());
    }

    #[cfg(unix)]
    #[test]
    fn meta_cage_follows_symlinks_to_the_constitution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".terra")).unwrap();
        std::fs::write(dir.path().join(".terra/constitution.toml"), "[meta]\nversion = 1\n").unwrap();
        let link = dir.path().join("rules.toml");
        std::os::unix::fs::symlink(dir.path().join(".terra/constitution.toml"), &link).unwrap();
        assert!(meta_check(CageAction::FileWrite { path: link }).is_blocked());
    }

    #[test]
    fn warnings_do_not_block_and_first_block_wins() {
        struct WarnCage;
        impl Cage for WarnCage {
            fn name(&self) -> &str {
                "warn_test"
            }
            fn check(&self, _ctx: &CageContext) -> CageCheckResult {
                CageCheckResult::Warn("proceed with caution".into())
            }
        }
        let engine = CagingEngine::new(vec![Box::new(WarnCage), Box::new(SpecificationCage), Box::new(VerificationCage)]);
        let ok = engine.check(&goal(CageAction::ExecuteTask { spec_complete: true }));
        assert!(ok.allowed);
        assert_eq!(ok.warnings, vec![("warn_test".to_string(), "proceed with caution".to_string())]);

        let mut ctx = goal(CageAction::ExecuteTask { spec_complete: false });
        ctx.spec_complete = false;
        assert_eq!(blocked_by(&engine.check(&ctx)), "specification");
    }
}
