//! Codebase Awareness
//!
//! Scans the project directory to build a file tree.
//! Detects project type. Provides condensed context for LLM prompts.

use std::path::{Path, PathBuf};

/// Project type detected from manifest files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectType {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    Java,
    Unknown,
}

impl ProjectType {
    pub fn as_str(&self) -> &str {
        match self {
            ProjectType::Rust => "Rust",
            ProjectType::JavaScript => "JavaScript",
            ProjectType::TypeScript => "TypeScript",
            ProjectType::Python => "Python",
            ProjectType::Go => "Go",
            ProjectType::Java => "Java",
            ProjectType::Unknown => "Unknown",
        }
    }
}

/// A file entry in the project tree.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Project awareness — file tree + type detection.
pub struct ProjectContext {
    pub root: PathBuf,
    pub project_type: ProjectType,
    pub files: Vec<FileEntry>,
    pub total_files: usize,
}

impl ProjectContext {
    /// Scan a project directory. Respects .gitignore patterns (basic).
    pub fn scan(root: &Path) -> Self {
        let project_type = detect_project_type(root);
        let mut files = Vec::new();
        let ignore_dirs = [".terra", ".git", "target", "node_modules", "__pycache__", ".venv", "dist", "build"];

        walk_dir(root, root, &ignore_dirs, &mut files, 0, 3); // max depth 3 for overview

        let total_files = files.len();

        Self { root: root.to_path_buf(), project_type, files, total_files }
    }

    /// Generate a condensed context string for LLM system prompt.
    pub fn to_prompt_context(&self) -> String {
        let mut lines = vec![
            format!("Project: {} ({})", self.root.file_name().and_then(|n| n.to_str()).unwrap_or("project"), self.project_type.as_str()),
            format!("Files: {} (showing key structure)", self.total_files),
        ];

        // Show directories and important files
        for entry in &self.files {
            let indent = entry.path.matches('/').count();
            let prefix = "  ".repeat(indent);
            if entry.is_dir {
                lines.push(format!("{}📁 {}/", prefix, entry.path.rsplit('/').next().unwrap_or(&entry.path)));
            } else {
                let name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
                if is_important_file(name) {
                    lines.push(format!("{}  {}", prefix, name));
                }
            }
        }

        lines.join("\n")
    }

    /// Get key files (manifests, configs, entry points).
    pub fn key_files(&self) -> Vec<&str> {
        self.files.iter()
            .filter(|f| !f.is_dir && is_important_file(f.path.rsplit('/').next().unwrap_or(&f.path)))
            .map(|f| f.path.as_str())
            .collect()
    }
}

/// Detect project type from manifest files.
fn detect_project_type(root: &Path) -> ProjectType {
    if root.join("Cargo.toml").exists() { return ProjectType::Rust; }
    if root.join("package.json").exists() {
        if root.join("tsconfig.json").exists() { return ProjectType::TypeScript; }
        return ProjectType::JavaScript;
    }
    if root.join("pyproject.toml").exists() || root.join("setup.py").exists() { return ProjectType::Python; }
    if root.join("go.mod").exists() { return ProjectType::Go; }
    if root.join("pom.xml").exists() || root.join("build.gradle").exists() { return ProjectType::Java; }
    ProjectType::Unknown
}

/// Check if a file is important enough to show in condensed view.
fn is_important_file(name: &str) -> bool {
    let important = [
        "Cargo.toml", "package.json", "tsconfig.json", "pyproject.toml",
        "go.mod", "pom.xml", "build.gradle", "Makefile", "Dockerfile",
        "README.md", "CLAUDE.md", "AGENTS.md", ".env.example",
        "main.rs", "lib.rs", "mod.rs", "index.ts", "index.js",
        "main.py", "app.py", "main.go", "Main.java",
    ];
    important.contains(&name)
}

/// Walk directory recursively (limited depth).
fn walk_dir(root: &Path, dir: &Path, ignore: &[&str], entries: &mut Vec<FileEntry>, depth: usize, max_depth: usize) {
    if depth > max_depth { return; }

    let mut items: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    items.sort_by_key(|e| e.file_name());

    for item in items {
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();

        // Skip ignored directories
        if ignore.iter().any(|i| name == *i) { continue; }
        // Skip hidden files
        if name.starts_with('.') && name != ".env.example" { continue; }

        let rel_path = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_string();
        let metadata = std::fs::metadata(&path);
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);

        entries.push(FileEntry { path: rel_path.clone(), is_dir, size });

        if is_dir {
            walk_dir(root, &path, ignore, entries, depth + 1, max_depth);
        }
    }
}

// --- Git Integration ---

/// Git status information.
#[derive(Debug, Clone)]
pub struct GitInfo {
    pub is_repo: bool,
    pub branch: String,
    pub has_changes: bool,
    pub changed_files: usize,
}

/// Detect git repository info.
pub fn detect_git(root: &Path) -> GitInfo {
    if !root.join(".git").exists() {
        return GitInfo { is_repo: false, branch: String::new(), has_changes: false, changed_files: 0 };
    }

    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .and_then(|o| if o.status.success() { Some(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { None })
        .unwrap_or_else(|| "unknown".into());

    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .ok();

    let (has_changes, changed_files) = status
        .map(|o| {
            let lines: Vec<&str> = std::str::from_utf8(&o.stdout).unwrap_or("").lines().filter(|l| !l.is_empty()).collect();
            (!lines.is_empty(), lines.len())
        })
        .unwrap_or((false, 0));

    GitInfo { is_repo: true, branch, has_changes, changed_files }
}

/// Get a diff summary for a specific file.
pub fn git_diff_file(root: &Path, file_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--stat", file_path])
        .current_dir(root)
        .output()
        .ok()?;

    if output.status.success() {
        let diff = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if diff.is_empty() { None } else { Some(diff) }
    } else {
        None
    }
}

/// Get a colored diff for a specific file (ANSI colors: green +, red -).
pub fn git_diff_colored(root: &Path, file_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["diff", file_path])
        .current_dir(root)
        .output()
        .ok()?;

    if !output.status.success() { return None; }
    let raw = String::from_utf8_lossy(&output.stdout);
    if raw.trim().is_empty() { return None; }

    // Colorize diff output
    let colored: String = raw.lines().map(|line| {
        if line.starts_with('+') && !line.starts_with("+++") {
            format!("\x1b[32m{}\x1b[0m", line)  // green for additions
        } else if line.starts_with('-') && !line.starts_with("---") {
            format!("\x1b[31m{}\x1b[0m", line)  // red for deletions
        } else if line.starts_with("@@") {
            format!("\x1b[36m{}\x1b[0m", line)  // cyan for hunk headers
        } else {
            line.to_string()
        }
    }).collect::<Vec<_>>().join("\n");

    Some(colored)
}

/// Stage files and create a commit with the given message.
/// Tracked files under `root` with unstaged changes — exactly what
/// `git checkout -- .` run in `root` would discard. Paths are relative to `root`.
pub fn git_unstaged_files(root: &Path) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", "--relative"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git diff failed: {}", e))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

pub fn git_commit(root: &Path, files: &[&str], message: &str) -> Result<String, String> {
    // Stage files
    for file in files {
        let result = std::process::Command::new("git")
            .args(["add", file])
            .current_dir(root)
            .output()
            .map_err(|e| format!("git add failed: {}", e))?;
        if !result.status.success() {
            return Err(format!("git add {} failed: {}", file, String::from_utf8_lossy(&result.stderr)));
        }
    }

    // Commit
    let result = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git commit failed: {}", e))?;

    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&result.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-CB-001: startup scans file tree
    #[test]
    fn startup_scans_filetree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn lib() {}").unwrap();

        let ctx = ProjectContext::scan(dir.path());
        assert!(ctx.total_files >= 2, "should find files, got {}", ctx.total_files);
    }

    // T-CB-002: file tree in LLM context
    #[test]
    fn filetree_in_prompt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();

        let ctx = ProjectContext::scan(dir.path());
        let prompt = ctx.to_prompt_context();
        assert!(prompt.contains("Rust"), "should detect Rust project: {}", prompt);
        assert!(prompt.contains("Cargo.toml") || prompt.contains("main.rs"),
            "should mention key files: {}", prompt);
    }

    // T-CB-003: project type detected
    #[test]
    fn project_type_detected() {
        let dir = tempfile::tempdir().unwrap();

        // Rust
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        assert_eq!(detect_project_type(dir.path()), ProjectType::Rust);

        // Python
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir2.path().join("pyproject.toml"), "").unwrap();
        assert_eq!(detect_project_type(dir2.path()), ProjectType::Python);

        // Unknown
        let dir3 = tempfile::tempdir().unwrap();
        assert_eq!(detect_project_type(dir3.path()), ProjectType::Unknown);
    }

    #[test]
    fn ignores_terra_and_git() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".terra")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".terra/config.toml"), "").unwrap();
        std::fs::write(dir.path().join("main.rs"), "").unwrap();

        let ctx = ProjectContext::scan(dir.path());
        let has_terra = ctx.files.iter().any(|f| f.path.contains(".terra"));
        assert!(!has_terra, "should not include .terra/");
    }

    // T-GIT-001: git repo detected
    #[test]
    fn git_repo_detected() {
        // Use the actual Terra project root (which is a git repo)
        let terra_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        let info = detect_git(terra_root);
        assert!(info.is_repo, "Terra project should be a git repo");
        assert!(!info.branch.is_empty(), "should have a branch name");
    }

    #[test]
    fn non_git_detected() {
        let dir = tempfile::tempdir().unwrap();
        let info = detect_git(dir.path());
        assert!(!info.is_repo);
    }
    #[test]
    fn git_unstaged_files_lists_only_modified_tracked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(root).output().unwrap()
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("tracked.txt"), "v1").unwrap();
        git(&["add", "."]);
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);

        assert!(git_unstaged_files(root).unwrap().is_empty());

        std::fs::write(root.join("tracked.txt"), "v2").unwrap();
        std::fs::write(root.join("untracked.txt"), "new").unwrap();
        assert_eq!(git_unstaged_files(root).unwrap(), vec!["tracked.txt".to_string()]);
    }
}
