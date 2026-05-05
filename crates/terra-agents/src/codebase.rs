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
}
