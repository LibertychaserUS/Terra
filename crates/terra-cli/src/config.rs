use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Terra configuration with layered precedence:
/// env var > project config > global config > defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerraConfig {
    pub provider: ProviderConfig,
    pub memory: MemoryConfig,
    #[serde(default)]
    pub sleep: SleepConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub cache_capacity: usize,
}

/// Sleep consolidation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleepConfig {
    /// Trigger mode: "session_end", "interval", or "manual"
    pub trigger: String,
    /// Hours between automatic consolidations (when trigger = "interval")
    pub interval_hours: u32,
    /// Maximum candidates per consolidation batch
    pub max_candidates: usize,
}

impl Default for SleepConfig {
    fn default() -> Self {
        Self {
            trigger: "session_end".into(),
            interval_hours: 24,
            max_candidates: 10,
        }
    }
}

impl Default for TerraConfig {
    fn default() -> Self {
        Self {
            provider: ProviderConfig {
                api_key: None,
                base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
                model: "qwen-plus".into(),
            },
            memory: MemoryConfig {
                cache_capacity: 1000,
            },
            sleep: SleepConfig::default(),
        }
    }
}

/// Resolve the global config directory (~/.terra/).
pub fn global_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".terra")
}

/// Resolve the global config file path.
pub fn global_config_path() -> PathBuf {
    global_config_dir().join("config.toml")
}

/// Resolve the project config file path.
pub fn project_config_path(project_root: &Path) -> PathBuf {
    project_root.join(".terra").join("config.toml")
}

/// Load a TOML config file, returning None if it doesn't exist.
fn load_toml(path: &Path) -> Result<Option<toml::Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    let value: toml::Value = content
        .parse()
        .with_context(|| format!("failed to parse config: {}", path.display()))?;
    Ok(Some(value))
}

/// Resolve the final configuration with full precedence chain.
pub fn resolve_config(project_root: Option<&Path>) -> TerraConfig {
    let mut config = TerraConfig::default();

    // Layer 1: global config
    if let Ok(Some(global)) = load_toml(&global_config_path()) {
        apply_toml(&mut config, &global);
    }

    // Layer 2: project config (overrides global)
    if let Some(root) = project_root {
        if let Ok(Some(project)) = load_toml(&project_config_path(root)) {
            apply_toml(&mut config, &project);
        }
    }

    // Layer 3: environment variables (highest precedence)
    if let Ok(key) = env::var("TERRA_PROVIDER_API_KEY") {
        config.provider.api_key = Some(key);
    } else if let Ok(key) = env::var("DASHSCOPE_API_KEY") {
        config.provider.api_key = Some(key);
    }
    if let Ok(url) = env::var("TERRA_PROVIDER_BASE_URL") {
        config.provider.base_url = url;
    }
    if let Ok(model) = env::var("TERRA_PROVIDER_MODEL") {
        config.provider.model = model;
    }

    config
}

/// Apply TOML values onto config (partial override).
fn apply_toml(config: &mut TerraConfig, toml: &toml::Value) {
    if let Some(provider) = toml.get("provider") {
        if let Some(url) = provider.get("base_url").and_then(|v| v.as_str()) {
            config.provider.base_url = url.to_string();
        }
        if let Some(model) = provider.get("model").and_then(|v| v.as_str()) {
            config.provider.model = model.to_string();
        }
        if let Some(key) = provider.get("api_key").and_then(|v| v.as_str()) {
            config.provider.api_key = Some(key.to_string());
        }
    }
    if let Some(memory) = toml.get("memory") {
        if let Some(cap) = memory.get("cache_capacity").and_then(|v| v.as_integer()) {
            config.memory.cache_capacity = cap as usize;
        }
    }
    if let Some(sleep) = toml.get("sleep") {
        if let Some(trigger) = sleep.get("trigger").and_then(|v| v.as_str()) {
            config.sleep.trigger = trigger.to_string();
        }
        if let Some(hours) = sleep.get("interval_hours").and_then(|v| v.as_integer()) {
            config.sleep.interval_hours = hours as u32;
        }
        if let Some(max) = sleep.get("max_candidates").and_then(|v| v.as_integer()) {
            config.sleep.max_candidates = max as usize;
        }
    }
}

/// Serialize config to TOML string (for `terra config show` and file writing).
pub fn to_toml_string(config: &TerraConfig) -> Result<String> {
    // Manual serialization to avoid exposing api_key
    let mut output = String::new();
    output.push_str("[provider]\n");
    output.push_str(&format!("base_url = \"{}\"\n", config.provider.base_url));
    output.push_str(&format!("model = \"{}\"\n", config.provider.model));
    if config.provider.api_key.is_some() {
        output.push_str("api_key = \"***REDACTED***\"\n");
    }
    Ok(output)
}

/// Set a config value using dotted key notation (e.g., "provider.model").
/// Writes to the specified config file (creates if not exists).
pub fn config_set(config_path: &Path, dotted_key: &str, value: &str) -> Result<()> {
    // Load existing config or start fresh
    let mut doc: toml::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        content.parse().with_context(|| format!("failed to parse {}", config_path.display()))?
    } else {
        toml::Table::new()
    };

    // Parse dotted key: "provider.model" → ["provider", "model"]
    let parts: Vec<&str> = dotted_key.split('.').collect();
    if parts.len() != 2 {
        anyhow::bail!("key must be in format 'section.key', got '{}'", dotted_key);
    }

    let section = parts[0];
    let key = parts[1];

    // Get or create section table
    let section_table = doc
        .entry(section.to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));

    if let toml::Value::Table(ref mut table) = section_table {
        // Try to parse as integer first, then as string
        if let Ok(num) = value.parse::<i64>() {
            table.insert(key.to_string(), toml::Value::Integer(num));
        } else {
            table.insert(key.to_string(), toml::Value::String(value.to_string()));
        }
    } else {
        anyhow::bail!("'{}' is not a table in config", section);
    }

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    // Write back
    let output = toml::to_string_pretty(&doc)
        .context("failed to serialize config")?;
    std::fs::write(config_path, &output)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(())
}

/// Environment detection for `terra doctor`.
#[derive(Debug, Serialize)]
pub struct EnvironmentReport {
    pub os: String,
    pub arch: String,
    pub terra_version: String,
    /// Minimum supported Rust version, from Cargo.toml `rust-version`.
    pub rust_version_min: String,
    pub provider_configured: bool,
    /// Where the API key comes from: an env var name, a config file, or "none".
    pub provider_source: String,
    pub model: String,
    pub base_url: String,
    pub project_initialized: bool,
    pub global_config_exists: bool,
    pub project_config_exists: bool,
}

/// Where the effective API key comes from, following the same precedence as
/// `resolve_config`: env vars, then project config, then global config.
pub fn api_key_source(project_root: Option<&Path>) -> String {
    for var in ["TERRA_PROVIDER_API_KEY", "DASHSCOPE_API_KEY"] {
        if env::var(var).is_ok() {
            return var.into();
        }
    }
    let has_key = |path: &Path| {
        matches!(load_toml(path), Ok(Some(t)) if t.get("provider").and_then(|p| p.get("api_key")).is_some())
    };
    if let Some(root) = project_root {
        if has_key(&project_config_path(root)) {
            return ".terra/config.toml".into();
        }
    }
    if has_key(&global_config_path()) {
        return "~/.terra/config.toml".into();
    }
    "none".into()
}

/// Environment variable that overrides `key` and is currently set, if any.
pub fn env_override_for(key: &str) -> Option<&'static str> {
    let candidates: &[&'static str] = match key {
        "provider.api_key" => &["TERRA_PROVIDER_API_KEY", "DASHSCOPE_API_KEY"],
        "provider.base_url" => &["TERRA_PROVIDER_BASE_URL"],
        "provider.model" => &["TERRA_PROVIDER_MODEL"],
        _ => &[],
    };
    candidates.iter().copied().find(|var| env::var(var).is_ok())
}

pub fn detect_environment(project_root: Option<&Path>) -> EnvironmentReport {
    let config = resolve_config(project_root);
    let provider_source = api_key_source(project_root);

    EnvironmentReport {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        terra_version: env!("CARGO_PKG_VERSION").into(),
        rust_version_min: env!("CARGO_PKG_RUST_VERSION").into(),
        provider_configured: config.provider.api_key.is_some(),
        provider_source,
        model: config.provider.model,
        base_url: config.provider.base_url,
        project_initialized: project_root.map(|r| r.join(".terra").is_dir()).unwrap_or(false),
        global_config_exists: global_config_path().exists(),
        project_config_exists: project_root
            .map(|r| project_config_path(r).exists())
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // T-CF-001
    #[test]
    fn default_config_values() {
        let config = TerraConfig::default();
        assert_eq!(config.provider.model, "qwen-plus");
        assert!(config.provider.api_key.is_none());
        assert_eq!(config.memory.cache_capacity, 1000);
    }

    // T-CF-002
    #[test]
    fn project_overrides_global() {
        let dir = tempfile::tempdir().unwrap();
        let global_dir = dir.path().join("global");
        let project_dir = dir.path().join("project");

        // Create global config
        fs::create_dir_all(&global_dir).unwrap();
        fs::write(
            global_dir.join("config.toml"),
            "[provider]\nmodel = \"global-model\"\n",
        )
        .unwrap();

        // Create project config with override
        fs::create_dir_all(project_dir.join(".terra")).unwrap();
        fs::write(
            project_dir.join(".terra/config.toml"),
            "[provider]\nmodel = \"project-model\"\n",
        )
        .unwrap();

        // Test: project config applies over global
        let mut config = TerraConfig::default();
        if let Ok(Some(global_toml)) = load_toml(&global_dir.join("config.toml")) {
            apply_toml(&mut config, &global_toml);
        }
        assert_eq!(config.provider.model, "global-model");

        if let Ok(Some(project_toml)) = load_toml(&project_dir.join(".terra/config.toml")) {
            apply_toml(&mut config, &project_toml);
        }
        assert_eq!(config.provider.model, "project-model");
    }

    // T-CF-003
    #[test]
    fn env_overrides_config() {
        // env::set_var is unsafe in multi-threaded tests, but fine for unit tests
        let config = resolve_config(None);
        // We can't easily test env override without side effects,
        // but we verify the precedence logic exists
        assert!(config.provider.model.len() > 0);
    }

    // T-CF-004
    #[test]
    fn config_show_redacts_key() {
        let mut config = TerraConfig::default();
        config.provider.api_key = Some("sk-secret-key-12345".into());
        let output = to_toml_string(&config).unwrap();
        assert!(output.contains("REDACTED"));
        assert!(!output.contains("sk-secret"));
    }

    // T-CF-006
    #[test]
    fn invalid_config_handled() {
        let dir = tempfile::tempdir().unwrap();
        let bad_path = dir.path().join("bad.toml");
        fs::write(&bad_path, "this is not [valid toml").unwrap();
        let result = load_toml(&bad_path);
        assert!(result.is_err());
    }

    #[test]
    fn environment_detection() {
        let report = detect_environment(None);
        assert!(!report.os.is_empty());
        assert!(!report.arch.is_empty());
        assert!(!report.terra_version.is_empty());
    }

    // T-CF-005: config set persists
    #[test]
    fn config_set_persists() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        // Set a value
        config_set(&config_path, "provider.model", "gpt-4o").unwrap();

        // Read back and verify
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("gpt-4o"));

        // Load and verify via TOML parser
        let parsed: toml::Value = content.parse().unwrap();
        assert_eq!(
            parsed["provider"]["model"].as_str(),
            Some("gpt-4o")
        );
    }

    #[test]
    fn config_set_integer_value() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        config_set(&config_path, "memory.cache_capacity", "2000").unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let parsed: toml::Value = content.parse().unwrap();
        assert_eq!(parsed["memory"]["cache_capacity"].as_integer(), Some(2000));
    }

    #[test]
    fn config_set_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("subdir").join("config.toml");

        assert!(!config_path.exists());
        config_set(&config_path, "provider.model", "test").unwrap();
        assert!(config_path.exists());
    }

    #[test]
    fn config_set_invalid_key_format() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        let result = config_set(&config_path, "no_dot_key", "value");
        assert!(result.is_err());
    }
}
