//! Constitution Loading
//!
//! Loads cage rules from `.terra/constitution.toml` at startup.
//! Once loaded, the constitution is immutable — any attempt to
//! modify it at runtime is blocked by the Meta-Cage.
//!
//! Constitution format:
//! ```toml
//! [meta]
//! version = 1
//! description = "Project constitution"
//!
//! [[protected_paths]]
//! path = ".terra/constitution.toml"
//!
//! [[protected_paths]]
//! path = ".terra/cage_rules/"
//!
//! [sandbox]
//! root = "."
//!
//! [rules]
//! allow_external_writes = false
//! require_spec_complete = true
//! require_verification = true
//! ```

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A loaded constitution — immutable at runtime.
#[derive(Debug, Clone)]
pub struct Constitution {
    config: ConstitutionConfig,
    /// The file path this was loaded from (for audit trail)
    source_path: PathBuf,
}

/// TOML structure for constitution file.
#[derive(Debug, Clone, Deserialize)]
pub struct ConstitutionConfig {
    pub meta: ConstitutionMeta,
    #[serde(default)]
    pub protected_paths: Vec<ProtectedPath>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub rules: RulesConfig,
    /// External MCP tool servers the agent may use. Their tools go through
    /// the same confirmation and audit as Terra's own.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
}

/// An MCP server started over stdio.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConstitutionMeta {
    pub version: u32,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtectedPath {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SandboxConfig {
    #[serde(default = "default_root")]
    pub root: String,
}

fn default_root() -> String {
    ".".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct RulesConfig {
    #[serde(default)]
    pub allow_external_writes: bool,
    #[serde(default = "default_true")]
    pub require_spec_complete: bool,
    #[serde(default = "default_true")]
    pub require_verification: bool,
    /// How shell commands are confined: "auto" (sandbox-exec on macOS,
    /// bwrap on Linux when installed) or "none".
    #[serde(default = "default_isolation")]
    pub shell_isolation: String,
}

fn default_isolation() -> String {
    "auto".into()
}

fn default_true() -> bool {
    true
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            allow_external_writes: false,
            require_spec_complete: true,
            require_verification: true,
            shell_isolation: default_isolation(),
        }
    }
}

/// Errors from constitution loading.
#[derive(thiserror::Error, Debug)]
pub enum ConstitutionError {
    #[error("constitution file not found: {0}")]
    NotFound(PathBuf),
    #[error("failed to read constitution: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("failed to parse constitution: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("constitution version {found} unsupported (expected {expected})")]
    UnsupportedVersion { found: u32, expected: u32 },
}

const CURRENT_VERSION: u32 = 1;

impl Constitution {
    /// Load a constitution from a TOML file.
    ///
    /// Returns an immutable Constitution. The only way to get one
    /// is through this constructor — no setters exist.
    pub fn load(path: &Path) -> Result<Self, ConstitutionError> {
        if !path.exists() {
            return Err(ConstitutionError::NotFound(path.to_path_buf()));
        }

        let content = std::fs::read_to_string(path)?;
        let config: ConstitutionConfig = toml::from_str(&content)?;

        if config.meta.version != CURRENT_VERSION {
            return Err(ConstitutionError::UnsupportedVersion {
                found: config.meta.version,
                expected: CURRENT_VERSION,
            });
        }

        Ok(Self {
            config,
            source_path: path.to_path_buf(),
        })
    }

    /// Load from a TOML string (for testing).
    pub fn from_str(toml_content: &str, source: PathBuf) -> Result<Self, ConstitutionError> {
        let config: ConstitutionConfig = toml::from_str(toml_content)?;

        if config.meta.version != CURRENT_VERSION {
            return Err(ConstitutionError::UnsupportedVersion {
                found: config.meta.version,
                expected: CURRENT_VERSION,
            });
        }

        Ok(Self {
            config,
            source_path: source,
        })
    }

    // --- Read-only accessors (no setters — immutability by API design) ---

    pub fn version(&self) -> u32 {
        self.config.meta.version
    }

    pub fn description(&self) -> &str {
        &self.config.meta.description
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// Get protected paths, resolved relative to the given project root.
    pub fn protected_paths(&self, project_root: &Path) -> Vec<PathBuf> {
        self.config
            .protected_paths
            .iter()
            .map(|p| project_root.join(&p.path))
            .collect()
    }

    /// Get sandbox root, resolved relative to the given project root.
    pub fn sandbox_root(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.config.sandbox.root)
    }

    pub fn require_spec_complete(&self) -> bool {
        self.config.rules.require_spec_complete
    }

    pub fn require_verification(&self) -> bool {
        self.config.rules.require_verification
    }

    pub fn allow_external_writes(&self) -> bool {
        self.config.rules.allow_external_writes
    }

    /// Whether shell commands run without OS-level isolation.
    pub fn shell_isolation_off(&self) -> bool {
        self.config.rules.shell_isolation.eq_ignore_ascii_case("none")
    }

    /// External MCP servers declared by the project.
    pub fn mcp_servers(&self) -> &[McpServerConfig] {
        &self.config.mcp_servers
    }

    /// Build a SandboxGuard from this constitution.
    pub fn to_sandbox_guard(&self, project_root: &Path) -> crate::sandbox::SandboxGuard {
        crate::sandbox::SandboxGuard::new(
            self.sandbox_root(project_root),
            self.protected_paths(project_root),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CONSTITUTION: &str = r#"
[meta]
version = 1
description = "Test project constitution"

[[protected_paths]]
path = ".terra/constitution.toml"

[[protected_paths]]
path = ".terra/cage_rules/"

[sandbox]
root = "."

[rules]
allow_external_writes = false
require_spec_complete = true
require_verification = true
"#;

    #[test]
    fn load_from_string() {
        let c = Constitution::from_str(TEST_CONSTITUTION, PathBuf::from("test.toml")).unwrap();
        assert_eq!(c.version(), 1);
        assert_eq!(c.description(), "Test project constitution");
    }

    #[test]
    fn protected_paths_resolved() {
        let c = Constitution::from_str(TEST_CONSTITUTION, PathBuf::from("test.toml")).unwrap();
        let root = PathBuf::from("/projects/myapp");
        let paths = c.protected_paths(&root);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/projects/myapp/.terra/constitution.toml"));
        assert_eq!(paths[1], PathBuf::from("/projects/myapp/.terra/cage_rules/"));
    }

    #[test]
    fn sandbox_root_resolved() {
        let c = Constitution::from_str(TEST_CONSTITUTION, PathBuf::from("test.toml")).unwrap();
        let root = PathBuf::from("/projects/myapp");
        assert_eq!(c.sandbox_root(&root), PathBuf::from("/projects/myapp/."));
    }

    #[test]
    fn rules_defaults() {
        let c = Constitution::from_str(TEST_CONSTITUTION, PathBuf::from("test.toml")).unwrap();
        assert!(c.require_spec_complete());
        assert!(c.require_verification());
        assert!(!c.allow_external_writes());
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("constitution.toml");
        std::fs::write(&path, TEST_CONSTITUTION).unwrap();

        let c = Constitution::load(&path).unwrap();
        assert_eq!(c.version(), 1);
        assert_eq!(c.source_path(), path);
    }

    #[test]
    fn load_not_found() {
        let result = Constitution::load(Path::new("/nonexistent/constitution.toml"));
        assert!(matches!(result, Err(ConstitutionError::NotFound(_))));
    }

    #[test]
    fn load_bad_version() {
        let toml = r#"
[meta]
version = 99
description = "future"

[sandbox]
root = "."
"#;
        let result = Constitution::from_str(toml, PathBuf::from("test.toml"));
        assert!(matches!(
            result,
            Err(ConstitutionError::UnsupportedVersion { found: 99, expected: 1 })
        ));
    }

    #[test]
    fn load_invalid_toml() {
        let result = Constitution::from_str("not valid toml {{{{", PathBuf::from("test.toml"));
        assert!(matches!(result, Err(ConstitutionError::ParseError(_))));
    }

    #[test]
    fn constitution_immutable_by_design() {
        let c = Constitution::from_str(TEST_CONSTITUTION, PathBuf::from("test.toml")).unwrap();
        // No setters exist on Constitution — this test verifies
        // that the only way to get data is through read-only accessors.
        // The compiler enforces this: c.config is private.
        let _ = c.version();
        let _ = c.description();
        let _ = c.require_spec_complete();
        // If anyone adds a setter, this test should be updated to fail.
    }

    #[test]
    fn constitution_to_sandbox_guard() {
        let c = Constitution::from_str(TEST_CONSTITUTION, PathBuf::from("test.toml")).unwrap();
        let root = PathBuf::from("/projects/myapp");
        let guard = c.to_sandbox_guard(&root);

        // Verify the guard works correctly
        use std::path::Path;
        assert!(guard.check_write(Path::new("/projects/myapp/src/main.rs")).is_ok());
        assert!(guard.check_write(Path::new("/projects/myapp/.terra/constitution.toml")).is_err());
        assert!(guard.check_write(Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn minimal_constitution() {
        let toml = r#"
[meta]
version = 1
"#;
        let c = Constitution::from_str(toml, PathBuf::from("test.toml")).unwrap();
        assert_eq!(c.version(), 1);
        assert!(c.description().is_empty());
        assert!(c.require_spec_complete()); // default true
        assert!(c.require_verification());   // default true
        assert!(!c.allow_external_writes()); // default false
    }
}
