//! OS-level confinement for shell commands.
//!
//! The cages decide whether a command may run; this module decides what it
//! can touch once it runs. Without it, `sh -c` runs with the user's full
//! permissions and the cage is a suggestion.
//!
//! - macOS: `sandbox-exec` with a profile that denies file writes outside
//!   the project root (plus temp and toolchain cache directories) and
//!   denies writes to the protected paths inside it.
//! - Linux: `bwrap` (bubblewrap) with the root file system read-only and the
//!   project bound read-write, protected paths bound read-only.
//! - Otherwise: no isolation, and the executor records that in each result.

use std::path::{Path, PathBuf};

use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// No OS-level confinement.
    None,
    /// macOS `sandbox-exec` with a generated profile.
    SandboxExec { root: PathBuf, protected: Vec<PathBuf>, allow: Vec<PathBuf> },
    /// Linux bubblewrap.
    Bubblewrap { root: PathBuf, protected: Vec<PathBuf> },
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

impl Isolation {
    /// The best isolation this machine offers for `root`.
    pub fn auto(root: &Path, protected: &[PathBuf]) -> Self {
        let root = canonical(root);
        let protected: Vec<PathBuf> = protected.iter().map(|p| canonical(p)).collect();
        if cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").exists() {
            return Isolation::SandboxExec { root, protected, allow: Self::default_allow_list() };
        }
        if cfg!(target_os = "linux") && command_exists("bwrap") {
            return Isolation::Bubblewrap { root, protected };
        }
        Isolation::None
    }

    /// Directories a build tool may need to write outside the project:
    /// temp directories and the toolchain caches under `$HOME`.
    fn default_allow_list() -> Vec<PathBuf> {
        let mut allow = vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp"), PathBuf::from("/private/var/folders"), PathBuf::from("/dev")];
        if let Some(tmp) = std::env::var_os("TMPDIR") {
            allow.push(canonical(Path::new(&tmp)));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            for cache in [".cargo", ".rustup", ".npm", ".cache", ".bun", ".yarn", ".pnpm-store", "go/pkg", ".m2", ".gradle", ".pyenv", "Library/Caches"] {
                allow.push(home.join(cache));
            }
        }
        allow
    }

    pub fn name(&self) -> &'static str {
        match self {
            Isolation::None => "none",
            Isolation::SandboxExec { .. } => "sandbox-exec",
            Isolation::Bubblewrap { .. } => "bwrap",
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Isolation::None)
    }

    /// The sandbox-exec profile (SBPL). Later rules win, so the order is:
    /// allow everything, deny writes, allow writes inside the project and
    /// the allow list, deny writes to protected paths.
    pub fn sandbox_profile(root: &Path, protected: &[PathBuf], allow: &[PathBuf]) -> String {
        fn quote(p: &Path) -> String {
            format!("\"{}\"", p.display().to_string().replace('\\', "\\\\").replace('"', "\\\""))
        }
        let mut profile = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");
        profile.push_str(&format!("(allow file-write* (subpath {}))\n", quote(root)));
        for dir in allow {
            profile.push_str(&format!("(allow file-write* (subpath {}))\n", quote(dir)));
        }
        profile.push_str("(allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (regex #\"^/dev/fd/\"))\n");
        for p in protected {
            profile.push_str(&format!("(deny file-write* (subpath {}) (literal {}))\n", quote(p), quote(p)));
        }
        profile
    }

    /// Build the process that runs `command` under this isolation.
    pub fn wrap(&self, command: &str, workdir: &Path) -> Command {
        match self {
            Isolation::None => {
                let mut cmd = Command::new("sh");
                cmd.args(["-c", command]);
                cmd
            }
            Isolation::SandboxExec { root, protected, allow } => {
                let mut cmd = Command::new("/usr/bin/sandbox-exec");
                cmd.arg("-p").arg(Self::sandbox_profile(root, protected, allow)).arg("sh").arg("-c").arg(command);
                cmd
            }
            Isolation::Bubblewrap { root, protected } => {
                let mut cmd = Command::new("bwrap");
                cmd.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]);
                cmd.arg("--bind").arg(root).arg(root);
                if let Some(home) = std::env::var_os("HOME") {
                    for cache in [".cargo", ".rustup", ".npm", ".cache"] {
                        let dir = PathBuf::from(&home).join(cache);
                        if dir.is_dir() {
                            cmd.arg("--bind").arg(&dir).arg(&dir);
                        }
                    }
                }
                for p in protected {
                    if p.exists() {
                        cmd.arg("--ro-bind").arg(p).arg(p);
                    }
                }
                cmd.args(["--die-with-parent", "--", "sh", "-c", command]);
                cmd
            }
        }
        .tap_workdir(workdir)
    }
}

trait TapWorkdir {
    fn tap_workdir(self, workdir: &Path) -> Self;
}

impl TapWorkdir for Command {
    fn tap_workdir(mut self, workdir: &Path) -> Self {
        self.current_dir(workdir);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_denies_outside_and_protected_allows_root() {
        let root = PathBuf::from("/Users/me/proj");
        let protected = vec![root.join(".terra/constitution.toml")];
        let profile = Isolation::sandbox_profile(&root, &protected, &[PathBuf::from("/tmp")]);
        let lines: Vec<&str> = profile.lines().collect();
        assert_eq!(lines[1], "(allow default)");
        assert_eq!(lines[2], "(deny file-write*)");
        assert!(lines[3].contains("(allow file-write* (subpath \"/Users/me/proj\"))"));
        assert!(lines.last().unwrap().starts_with("(deny file-write* (subpath \"/Users/me/proj/.terra/constitution.toml\")"));
    }

    #[test]
    fn none_runs_plain_sh() {
        let cmd = Isolation::None.wrap("echo hi", Path::new("/"));
        assert_eq!(cmd.as_std().get_program(), "sh");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn sandbox_exec_confines_writes_to_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(root.join(".terra")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join(".terra/constitution.toml"), "[meta]\nversion = 1\n").unwrap();
        assert_eq!(Isolation::auto(&root, &[]).name(), "sandbox-exec");
        // No allow list: the temp directory this test lives in is outside too.
        let isolation = Isolation::SandboxExec {
            root: canonical(&root),
            protected: vec![canonical(&root.join(".terra/constitution.toml"))],
            allow: vec![],
        };

        let run = |cmd: &str| {
            let mut c = isolation.wrap(cmd, &root);
            async move { c.output().await.unwrap().status.success() }
        };
        assert!(run("echo inside > inside.txt").await);
        assert!(root.join("inside.txt").exists());
        assert!(!run(&format!("echo escaped > {}/by-shell.txt", outside.display())).await);
        assert!(!outside.join("by-shell.txt").exists());
        assert!(!run("echo pwned >> .terra/constitution.toml").await);
        assert_eq!(std::fs::read_to_string(root.join(".terra/constitution.toml")).unwrap(), "[meta]\nversion = 1\n");
        assert!(run("cat .terra/constitution.toml > /dev/null").await, "reads still work");
    }
}
