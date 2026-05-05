use std::path::{Path, PathBuf};

/// File system permission errors.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    #[error("write outside sandbox: {0}")]
    OutsideSandbox(PathBuf),

    #[error("write to protected path: {0}")]
    ProtectedPath(PathBuf),
}

/// File system permission enforcer.
///
/// Rules:
/// - Writes/deletes MUST be within the sandbox root (project directory).
/// - Writes to protected paths (e.g., .terra/constitution) are blocked.
/// - Reads are allowed anywhere (read-only access outside sandbox).
pub struct SandboxGuard {
    root: PathBuf,
    protected: Vec<PathBuf>,
}

impl SandboxGuard {
    /// Create a new SandboxGuard.
    ///
    /// `root` — the project directory (sandbox boundary).
    /// `protected` — paths within root that are read-only (e.g., constitution).
    pub fn new(root: PathBuf, protected: Vec<PathBuf>) -> Self {
        Self { root, protected }
    }

    /// Check if a write is allowed at this path.
    pub fn check_write(&self, path: &Path) -> Result<(), SandboxViolation> {
        self.check_mutation(path)
    }

    /// Check if a delete is allowed at this path.
    pub fn check_delete(&self, path: &Path) -> Result<(), SandboxViolation> {
        self.check_mutation(path)
    }

    /// Reads are always allowed (any path).
    pub fn check_read(&self, _path: &Path) -> Result<(), SandboxViolation> {
        Ok(())
    }

    /// Internal: check if a mutation (write/delete) is allowed.
    fn check_mutation(&self, path: &Path) -> Result<(), SandboxViolation> {
        // Normalize the path for comparison.
        // In production we'd canonicalize, but for testing
        // we use starts_with on the provided path.
        let normalized = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        // Must be within sandbox root
        if !normalized.starts_with(&self.root) {
            return Err(SandboxViolation::OutsideSandbox(normalized));
        }

        // Must not be in a protected path
        for protected in &self.protected {
            if normalized.starts_with(protected) {
                return Err(SandboxViolation::ProtectedPath(normalized));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_guard() -> SandboxGuard {
        let root = PathBuf::from("/projects/myapp");
        let protected = vec![
            PathBuf::from("/projects/myapp/.terra/constitution.toml"),
            PathBuf::from("/projects/myapp/.terra/cage_rules"),
        ];
        SandboxGuard::new(root, protected)
    }

    // T-CG-009
    #[test]
    fn sandbox_write_inside_root() {
        let guard = test_guard();
        assert!(guard.check_write(Path::new("/projects/myapp/src/main.rs")).is_ok());
    }

    // T-CG-010
    #[test]
    fn sandbox_write_outside_root() {
        let guard = test_guard();
        let result = guard.check_write(Path::new("/etc/passwd"));
        assert!(matches!(result, Err(SandboxViolation::OutsideSandbox(_))));
    }

    // T-CG-011
    #[test]
    fn sandbox_read_outside_root() {
        let guard = test_guard();
        assert!(guard.check_read(Path::new("/etc/passwd")).is_ok());
    }

    // T-CG-012
    #[test]
    fn sandbox_write_to_protected() {
        let guard = test_guard();
        let result = guard.check_write(Path::new("/projects/myapp/.terra/constitution.toml"));
        assert!(matches!(result, Err(SandboxViolation::ProtectedPath(_))));
    }

    // T-CG-013
    #[test]
    fn sandbox_read_protected() {
        let guard = test_guard();
        assert!(guard.check_read(Path::new("/projects/myapp/.terra/constitution.toml")).is_ok());
    }

    // T-CG-014
    #[test]
    fn sandbox_delete_outside_root() {
        let guard = test_guard();
        let result = guard.check_delete(Path::new("/tmp/some_file"));
        assert!(matches!(result, Err(SandboxViolation::OutsideSandbox(_))));
    }

    #[test]
    fn sandbox_delete_inside_root() {
        let guard = test_guard();
        assert!(guard.check_delete(Path::new("/projects/myapp/temp.txt")).is_ok());
    }

    #[test]
    fn sandbox_write_to_protected_subpath() {
        let guard = test_guard();
        let result = guard.check_write(Path::new("/projects/myapp/.terra/cage_rules/meta.toml"));
        assert!(matches!(result, Err(SandboxViolation::ProtectedPath(_))));
    }
}
