use std::path::{Path, PathBuf};

/// File system permission errors.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    #[error("write outside sandbox: {0}")]
    OutsideSandbox(PathBuf),

    #[error("write to protected path: {0}")]
    ProtectedPath(PathBuf),

    #[error("symlink escape: {link} resolves to {target} outside sandbox")]
    SymlinkEscape { link: PathBuf, target: PathBuf },
}

/// File system permission enforcer.
///
/// Rules:
/// - Writes/deletes MUST be within the sandbox root (project directory).
/// - Writes to protected paths (e.g., .terra/constitution) are blocked.
/// - Symlinks that resolve outside the sandbox are blocked for writes.
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

    /// Check a path for symlink escape: if it's a symlink,
    /// resolve it and verify the target is within sandbox.
    pub fn check_symlink(&self, path: &Path) -> Result<(), SandboxViolation> {
        // Try to read symlink target. If not a symlink, this returns Err,
        // which means no escape — that's fine.
        if let Ok(target) = std::fs::read_link(path) {
            // Resolve relative symlink targets relative to the symlink's parent
            let resolved = if target.is_relative() {
                path.parent()
                    .unwrap_or(Path::new("."))
                    .join(&target)
            } else {
                target.clone()
            };

            if !resolved.starts_with(&self.root) {
                return Err(SandboxViolation::SymlinkEscape {
                    link: path.to_path_buf(),
                    target: resolved,
                });
            }
        }
        Ok(())
    }

    /// Internal: check if a mutation (write/delete) is allowed.
    fn check_mutation(&self, path: &Path) -> Result<(), SandboxViolation> {
        let normalized = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        // Check symlink escape before other checks
        self.check_symlink(&normalized)?;

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

    // T-CG-015: symlink escape detection
    #[test]
    fn sandbox_symlink_escape_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create a symlink inside the sandbox pointing outside
        let link_path = root.join("escape_link");
        std::os::unix::fs::symlink("/etc/passwd", &link_path).unwrap();

        let guard = SandboxGuard::new(root, vec![]);

        // Writing through the symlink should be blocked
        let result = guard.check_write(&link_path);
        assert!(
            matches!(result, Err(SandboxViolation::SymlinkEscape { .. })),
            "symlink pointing outside sandbox should be blocked: {:?}",
            result
        );
    }

    #[test]
    fn sandbox_symlink_inside_passes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create a target file inside sandbox
        let target = root.join("real_file.txt");
        std::fs::write(&target, "data").unwrap();

        // Create a symlink inside sandbox pointing to another file inside
        let link_path = root.join("safe_link");
        std::os::unix::fs::symlink(&target, &link_path).unwrap();

        let guard = SandboxGuard::new(root, vec![]);

        // Writing through an internal symlink should pass
        assert!(guard.check_write(&link_path).is_ok());
    }
}
