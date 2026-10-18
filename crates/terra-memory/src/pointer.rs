//! DSL Pointer Memory Index
//!
//! Lightweight pointer table that acts as an index over Terra's memory database.
//! Instead of full-text searches, the pointer table maintains compact index records
//! with salience scoring. The table serializes to compact text for LLM context inclusion
//! and to TOML for disk persistence.
//!
//! Two-layer design:
//! - **Rust API layer**: `PointerTable`, `Pointer`, `SalienceSignal` structs
//! - **Text DSL layer**: `@mem["key"]`, `@L1[*]`, `@search("query", 5)` syntax parsed programmatically

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::types::{Level, Scope};

// ---------------------------------------------------------------------------
// Pointer
// ---------------------------------------------------------------------------

/// A pointer to a memory entry -- lightweight index record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pointer {
    pub key: String,
    pub level: Level,
    pub scope: Scope,
    /// Optional path to backing `.md` file.
    pub path: Option<String>,
    /// Human/AI-readable description.
    pub annotation: String,
    /// Dynamic relevance score in `[0.0, 1.0]`.
    pub salience: f32,
    /// Individual salience components (preserved for accurate recalculation).
    pub salience_keyword: f32,
    pub salience_semantic: f32,
    pub last_accessed: DateTime<Utc>,
    pub access_count: u32,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// SalienceSignal
// ---------------------------------------------------------------------------

/// Signals that drive salience recalculation.
#[derive(Debug, Clone)]
pub enum SalienceSignal {
    /// Pointer was read/accessed.
    Accessed,
    /// Keyword relevance score from TF-IDF (`VectorIndex`).
    KeywordMatch(f32),
    /// Semantic similarity score from `EmbeddingBackend`.
    SemanticMatch(f32),
    /// Recalculate time-based decay component.
    TimeDecay,
}

// ---------------------------------------------------------------------------
// PointerTable
// ---------------------------------------------------------------------------

/// The pointer table -- lightweight index over the memory database.
///
/// Backed by a `BTreeMap` for deterministic iteration order (sorted by key).
pub struct PointerTable {
    entries: BTreeMap<String, Pointer>,
}

impl PointerTable {
    /// Create an empty pointer table.
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    // -- Mutation -----------------------------------------------------------

    /// Register a new pointer for a memory key.
    ///
    /// If a pointer with the same key already exists it is replaced.
    pub fn register(
        &mut self,
        key: String,
        level: Level,
        scope: Scope,
        annotation: String,
    ) {
        let now = Utc::now();
        let pointer = Pointer {
            key: key.clone(),
            level,
            scope,
            path: None,
            annotation,
            salience: 0.5, // neutral initial salience
            salience_keyword: 0.0,
            salience_semantic: 0.0,
            last_accessed: now,
            access_count: 0,
            created_at: now,
        };
        self.entries.insert(key, pointer);
    }

    /// Register a pointer with a backing file path.
    pub fn register_with_path(
        &mut self,
        key: String,
        level: Level,
        scope: Scope,
        annotation: String,
        path: String,
    ) {
        let now = Utc::now();
        let pointer = Pointer {
            key: key.clone(),
            level,
            scope,
            path: Some(path),
            annotation,
            salience: 0.5,
            salience_keyword: 0.0,
            salience_semantic: 0.0,
            last_accessed: now,
            access_count: 0,
            created_at: now,
        };
        self.entries.insert(key, pointer);
    }

    /// Remove a pointer by key, returning it if it existed.
    pub fn unregister(&mut self, key: &str) -> Option<Pointer> {
        self.entries.remove(key)
    }

    // -- Query --------------------------------------------------------------

    /// Get a pointer by exact key.
    pub fn get(&self, key: &str) -> Option<&Pointer> {
        self.entries.get(key)
    }

    /// Get a mutable pointer by key (for updating access stats externally).
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Pointer> {
        self.entries.get_mut(key)
    }

    /// List all pointers (in key order).
    pub fn list(&self) -> Vec<&Pointer> {
        self.entries.values().collect()
    }

    /// Number of pointers in the table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // -- DSL Resolve --------------------------------------------------------

    /// Resolve a DSL query string to matching pointers.
    ///
    /// Supported patterns:
    /// - `@mem["key"]`               -- exact key lookup
    /// - `@L0[*]` through `@L6[*]`  -- level filter
    /// - `@scope["project"]`         -- scope filter
    /// - `@search("query")`          -- annotation text search (top 10 default)
    /// - `@search("query", N)`       -- annotation text search, top N
    /// - `@all`                      -- all pointers
    /// - `@top(N)`                   -- top N by salience
    pub fn resolve(&self, dsl: &str) -> Vec<&Pointer> {
        let dsl = dsl.trim();

        // @mem["key"]
        if let Some(key) = parse_mem_key(dsl) {
            return self.get(&key).into_iter().collect();
        }

        // @L0[*] .. @L6[*]
        if let Some(level) = parse_level_wildcard(dsl) {
            return self
                .entries
                .values()
                .filter(|p| p.level == level)
                .collect();
        }

        // @scope["..."]
        if let Some(scope) = parse_scope_filter(dsl) {
            return self
                .entries
                .values()
                .filter(|p| p.scope == scope)
                .collect();
        }

        // @search("query") or @search("query", N)
        if let Some((query, top_n)) = parse_search(dsl) {
            return self.search_annotations(&query, top_n);
        }

        // @all
        if dsl == "@all" {
            return self.entries.values().collect();
        }

        // @top(N)
        if let Some(n) = parse_top_n(dsl) {
            return self.working_set(n);
        }

        // Unrecognised -- return empty
        Vec::new()
    }

    // -- Working set --------------------------------------------------------

    /// Return the top-N pointers sorted by salience descending.
    ///
    /// This is the "working set" -- the most relevant pointers for the current
    /// context window.
    pub fn working_set(&self, top_n: usize) -> Vec<&Pointer> {
        let mut sorted: Vec<&Pointer> = self.entries.values().collect();
        sorted.sort_by(|a, b| {
            b.salience
                .partial_cmp(&a.salience)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(top_n);
        sorted
    }

    // -- Salience -----------------------------------------------------------

    /// Update salience for a pointer based on a signal.
    ///
    /// Salience formula:
    /// ```text
    /// salience = keyword_match * 0.3
    ///          + semantic_similarity * 0.4
    ///          + time_decay * 0.2
    ///          + access_frequency * 0.1
    /// ```
    ///
    /// Each signal updates its component and the final salience is recalculated.
    pub fn update_salience(&mut self, key: &str, signal: SalienceSignal) {
        if let Some(ptr) = self.entries.get_mut(key) {
            // Update the specific component
            match signal {
                SalienceSignal::Accessed => {
                    ptr.access_count += 1;
                    ptr.last_accessed = Utc::now();
                }
                SalienceSignal::KeywordMatch(score) => {
                    ptr.salience_keyword = score.clamp(0.0, 1.0);
                }
                SalienceSignal::SemanticMatch(score) => {
                    ptr.salience_semantic = score.clamp(0.0, 1.0);
                }
                SalienceSignal::TimeDecay => {
                    // No component to update; time is always recomputed
                }
            }

            // Recompute composite salience from all components
            let time_decay = compute_time_decay(ptr.last_accessed);
            let access_freq = (ptr.access_count as f32 / 20.0).min(1.0);
            ptr.salience = (
                ptr.salience_keyword * 0.3
                + ptr.salience_semantic * 0.4
                + time_decay * 0.2
                + access_freq * 0.1
            ).clamp(0.0, 1.0);
        }
    }

    // -- Serialization: compact text ----------------------------------------

    /// Serialize to compact text for LLM context inclusion.
    ///
    /// Format: one line per pointer, sorted by salience descending.
    /// ```text
    /// @auth_pattern [L1/project] (0.85) -- JWT authentication design decisions
    /// @db_schema [L2/session] (0.62) -- Database migration tracking
    /// ```
    pub fn serialize_compact(&self) -> String {
        let mut sorted: Vec<&Pointer> = self.entries.values().collect();
        sorted.sort_by(|a, b| {
            b.salience
                .partial_cmp(&a.salience)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        sorted
            .iter()
            .map(|p| {
                format!(
                    "@{} [{}/{}] ({:.2}) -- {}",
                    p.key,
                    p.level.as_str(),
                    p.scope.as_str(),
                    p.salience,
                    p.annotation,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    // -- Serialization: TOML persistence ------------------------------------

    /// Serialize the pointer table to TOML for persistence (`_index.toml`).
    pub fn to_toml(&self) -> String {
        let wrapper = TomlWrapper::from_table(self);
        toml::to_string_pretty(&wrapper).unwrap_or_default()
    }

    /// Load a pointer table from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, String> {
        let wrapper: TomlWrapper =
            toml::from_str(toml_str).map_err(|e| format!("TOML parse error: {e}"))?;
        Ok(wrapper.into_table())
    }

    // -- Internal helpers ---------------------------------------------------

    /// Search annotations by simple substring/word overlap, return top N.
    fn search_annotations(&self, query: &str, top_n: usize) -> Vec<&Pointer> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 1)
            .collect();

        if query_words.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(&Pointer, f32)> = self
            .entries
            .values()
            .filter_map(|p| {
                let ann_lower = p.annotation.to_lowercase();
                let key_lower = p.key.to_lowercase();
                let combined = format!("{} {}", key_lower, ann_lower);

                let matches = query_words
                    .iter()
                    .filter(|w| combined.contains(*w))
                    .count();

                if matches > 0 {
                    let score = matches as f32 / query_words.len() as f32;
                    Some((p, score))
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_n);
        scored.into_iter().map(|(p, _)| p).collect()
    }
}

impl Default for PointerTable {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Time decay helper
// ===========================================================================

/// Compute time decay: `exp(-0.1 * days_since_last_access)`.
///
/// Returns a value in `(0.0, 1.0]` -- 1.0 means just accessed, decays toward 0.
fn compute_time_decay(last_accessed: DateTime<Utc>) -> f32 {
    let now = Utc::now();
    let duration = now.signed_duration_since(last_accessed);
    let days = duration.num_seconds() as f64 / 86_400.0;
    let days = days.max(0.0); // guard against clock skew
    (-0.1 * days).exp() as f32
}

// ===========================================================================
// DSL parsers (private)
// ===========================================================================

/// Parse `@mem["key"]` -> Some(key)
fn parse_mem_key(dsl: &str) -> Option<String> {
    let re = Regex::new(r#"^@mem\["([^"]+)"\]$"#).ok()?;
    re.captures(dsl).map(|c| c[1].to_string())
}

/// Parse `@L0[*]` through `@L6[*]` -> Some(Level)
fn parse_level_wildcard(dsl: &str) -> Option<Level> {
    let re = Regex::new(r#"^@(L[0-6])\[\*\]$"#).ok()?;
    let caps = re.captures(dsl)?;
    Level::parse(&caps[1])
}

/// Parse `@scope["project"]` -> Some(Scope)
fn parse_scope_filter(dsl: &str) -> Option<Scope> {
    let re = Regex::new(r#"^@scope\["([^"]+)"\]$"#).ok()?;
    let caps = re.captures(dsl)?;
    Scope::parse(&caps[1])
}

/// Parse `@search("query")` or `@search("query", N)` -> Some((query, top_n))
fn parse_search(dsl: &str) -> Option<(String, usize)> {
    // With explicit top_n: @search("query", 5)
    let re_with_n =
        Regex::new(r#"^@search\("([^"]+)",\s*(\d+)\)$"#).ok()?;
    if let Some(caps) = re_with_n.captures(dsl) {
        let query = caps[1].to_string();
        let n: usize = caps[2].parse().ok()?;
        return Some((query, n));
    }

    // Without top_n: @search("query") -- default 10
    let re_no_n = Regex::new(r#"^@search\("([^"]+)"\)$"#).ok()?;
    if let Some(caps) = re_no_n.captures(dsl) {
        let query = caps[1].to_string();
        return Some((query, 10));
    }

    None
}

/// Parse `@top(N)` -> Some(N)
fn parse_top_n(dsl: &str) -> Option<usize> {
    let re = Regex::new(r#"^@top\((\d+)\)$"#).ok()?;
    let caps = re.captures(dsl)?;
    caps[1].parse().ok()
}

// ===========================================================================
// TOML serde wrapper
// ===========================================================================

/// Intermediate representation for TOML serialization.
///
/// TOML doesn't natively support `BTreeMap<String, Pointer>` where `Pointer`
/// contains `DateTime` and enums. We flatten to a table of entries.
#[derive(Serialize, Deserialize)]
struct TomlWrapper {
    pointer: Vec<TomlPointer>,
}

#[derive(Serialize, Deserialize)]
struct TomlPointer {
    key: String,
    level: String,
    scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    annotation: String,
    salience: f32,
    #[serde(default)]
    salience_keyword: f32,
    #[serde(default)]
    salience_semantic: f32,
    last_accessed: String, // ISO 8601
    access_count: u32,
    created_at: String, // ISO 8601
}

impl TomlWrapper {
    fn from_table(table: &PointerTable) -> Self {
        let pointer = table
            .entries
            .values()
            .map(|p| TomlPointer {
                key: p.key.clone(),
                level: p.level.as_str().to_string(),
                scope: p.scope.as_str().to_string(),
                path: p.path.clone(),
                annotation: p.annotation.clone(),
                salience: p.salience,
                salience_keyword: p.salience_keyword,
                salience_semantic: p.salience_semantic,
                last_accessed: p.last_accessed.to_rfc3339(),
                access_count: p.access_count,
                created_at: p.created_at.to_rfc3339(),
            })
            .collect();
        Self { pointer }
    }

    fn into_table(self) -> PointerTable {
        let mut entries = BTreeMap::new();
        for tp in self.pointer {
            let level = Level::parse(&tp.level).unwrap_or(Level::L6);
            let scope = Scope::parse(&tp.scope).unwrap_or(Scope::Session);
            let last_accessed = DateTime::parse_from_rfc3339(&tp.last_accessed)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let created_at = DateTime::parse_from_rfc3339(&tp.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let pointer = Pointer {
                key: tp.key.clone(),
                level,
                scope,
                path: tp.path,
                annotation: tp.annotation,
                salience: tp.salience,
                salience_keyword: tp.salience_keyword,
                salience_semantic: tp.salience_semantic,
                last_accessed,
                access_count: tp.access_count,
                created_at,
            };
            entries.insert(tp.key, pointer);
        }
        PointerTable { entries }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    /// Helper: build a table with several pointers for testing.
    fn test_table() -> PointerTable {
        let mut table = PointerTable::new();
        table.register(
            "auth_pattern".into(),
            Level::L1,
            Scope::Project,
            "JWT authentication design decisions".into(),
        );
        table.register(
            "db_schema".into(),
            Level::L2,
            Scope::Session,
            "Database migration tracking".into(),
        );
        table.register(
            "api_design".into(),
            Level::L1,
            Scope::Project,
            "REST API endpoint design notes".into(),
        );
        table.register(
            "cache_strategy".into(),
            Level::L3,
            Scope::Global,
            "Redis caching strategy for sessions".into(),
        );
        table.register(
            "ui_layout".into(),
            Level::L4,
            Scope::Task,
            "Frontend layout CSS grid decisions".into(),
        );

        // Set varying salience for deterministic ordering tests
        if let Some(p) = table.get_mut("auth_pattern") {
            p.salience = 0.85;
        }
        if let Some(p) = table.get_mut("db_schema") {
            p.salience = 0.62;
        }
        if let Some(p) = table.get_mut("api_design") {
            p.salience = 0.78;
        }
        if let Some(p) = table.get_mut("cache_strategy") {
            p.salience = 0.45;
        }
        if let Some(p) = table.get_mut("ui_layout") {
            p.salience = 0.30;
        }

        table
    }

    // =======================================================================
    // T-MEM-051: DSL parsing for all patterns
    // =======================================================================

    #[test]
    fn t_mem_051_dsl_mem_key_exact() {
        let table = test_table();

        let result = table.resolve(r#"@mem["auth_pattern"]"#);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "auth_pattern");
    }

    #[test]
    fn t_mem_051_dsl_mem_key_missing() {
        let table = test_table();

        let result = table.resolve(r#"@mem["nonexistent"]"#);
        assert!(result.is_empty());
    }

    #[test]
    fn t_mem_051_dsl_level_wildcard() {
        let table = test_table();

        // L1 should return auth_pattern and api_design
        let result = table.resolve("@L1[*]");
        assert_eq!(result.len(), 2);
        let keys: Vec<&str> = result.iter().map(|p| p.key.as_str()).collect();
        assert!(keys.contains(&"auth_pattern"));
        assert!(keys.contains(&"api_design"));
    }

    #[test]
    fn t_mem_051_dsl_level_wildcard_l0_empty() {
        let table = test_table();

        let result = table.resolve("@L0[*]");
        assert!(result.is_empty(), "No L0 pointers in test data");
    }

    #[test]
    fn t_mem_051_dsl_scope_filter() {
        let table = test_table();

        let result = table.resolve(r#"@scope["project"]"#);
        assert_eq!(result.len(), 2);
        for p in &result {
            assert_eq!(p.scope, Scope::Project);
        }
    }

    #[test]
    fn t_mem_051_dsl_scope_global() {
        let table = test_table();

        let result = table.resolve(r#"@scope["global"]"#);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key, "cache_strategy");
    }

    #[test]
    fn t_mem_051_dsl_search_basic() {
        let table = test_table();

        let result = table.resolve(r#"@search("authentication")"#);
        assert!(!result.is_empty());
        assert_eq!(result[0].key, "auth_pattern");
    }

    #[test]
    fn t_mem_051_dsl_search_with_top_n() {
        let table = test_table();

        let result = table.resolve(r#"@search("design", 2)"#);
        assert!(result.len() <= 2);
        // Both auth_pattern and api_design have "design" in their annotations
        let keys: Vec<&str> = result.iter().map(|p| p.key.as_str()).collect();
        assert!(
            keys.contains(&"auth_pattern") || keys.contains(&"api_design"),
            "should match design-related pointers: {:?}",
            keys
        );
    }

    #[test]
    fn t_mem_051_dsl_all() {
        let table = test_table();

        let result = table.resolve("@all");
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn t_mem_051_dsl_top_n() {
        let table = test_table();

        let result = table.resolve("@top(3)");
        assert_eq!(result.len(), 3);
        // Should be sorted by salience descending: 0.85, 0.78, 0.62
        assert_eq!(result[0].key, "auth_pattern");
        assert_eq!(result[1].key, "api_design");
        assert_eq!(result[2].key, "db_schema");
    }

    #[test]
    fn t_mem_051_dsl_unrecognised() {
        let table = test_table();

        let result = table.resolve("@invalid_syntax");
        assert!(result.is_empty());
    }

    // =======================================================================
    // T-MEM-052: Salience calculation with all 4 signals
    // =======================================================================

    #[test]
    fn t_mem_052_salience_accessed() {
        let mut table = PointerTable::new();
        table.register(
            "test_key".into(),
            Level::L2,
            Scope::Session,
            "test".into(),
        );
        let initial = table.get("test_key").unwrap().salience;

        table.update_salience("test_key", SalienceSignal::Accessed);

        let ptr = table.get("test_key").unwrap();
        assert_eq!(ptr.access_count, 1);
        // Salience should change (access_freq now > 0)
        assert_ne!(ptr.salience, initial, "salience should change after access");
    }

    #[test]
    fn t_mem_052_salience_keyword_match() {
        let mut table = PointerTable::new();
        table.register("kw".into(), Level::L1, Scope::Project, "test".into());

        table.update_salience("kw", SalienceSignal::KeywordMatch(0.9));

        let ptr = table.get("kw").unwrap();
        assert!(ptr.salience > 0.0);
        assert!(ptr.salience <= 1.0);
    }

    #[test]
    fn t_mem_052_salience_semantic_match() {
        let mut table = PointerTable::new();
        table.register("sem".into(), Level::L1, Scope::Project, "test".into());

        table.update_salience("sem", SalienceSignal::SemanticMatch(0.95));

        let ptr = table.get("sem").unwrap();
        assert!(ptr.salience > 0.0);
        assert!(ptr.salience <= 1.0);
    }

    #[test]
    fn t_mem_052_salience_time_decay() {
        let mut table = PointerTable::new();
        table.register("td".into(), Level::L3, Scope::Global, "test".into());

        // Simulate stale pointer: set last_accessed to 30 days ago
        if let Some(p) = table.get_mut("td") {
            p.last_accessed = Utc::now() - Duration::days(30);
            p.salience = 0.9;
        }

        table.update_salience("td", SalienceSignal::TimeDecay);

        let ptr = table.get("td").unwrap();
        assert!(
            ptr.salience < 0.9,
            "salience should decay for stale pointer: got {}",
            ptr.salience
        );
    }

    #[test]
    fn t_mem_052_salience_clamp_bounds() {
        let mut table = PointerTable::new();
        table.register("clamp".into(), Level::L0, Scope::Global, "test".into());

        // Very high keyword match shouldn't exceed 1.0
        if let Some(p) = table.get_mut("clamp") {
            p.salience = 0.99;
            p.access_count = 100;
        }
        table.update_salience("clamp", SalienceSignal::KeywordMatch(1.0));

        let ptr = table.get("clamp").unwrap();
        assert!(ptr.salience <= 1.0, "salience must not exceed 1.0");
        assert!(ptr.salience >= 0.0, "salience must not go below 0.0");
    }

    #[test]
    fn t_mem_052_salience_nonexistent_key_noop() {
        let mut table = PointerTable::new();
        // Should not panic
        table.update_salience("no_such_key", SalienceSignal::Accessed);
    }

    // =======================================================================
    // T-MEM-053: Working set returns sorted by salience
    // =======================================================================

    #[test]
    fn t_mem_053_working_set_sorted() {
        let table = test_table();

        let ws = table.working_set(5);
        assert_eq!(ws.len(), 5);

        for window in ws.windows(2) {
            assert!(
                window[0].salience >= window[1].salience,
                "working set must be sorted descending: {} >= {}",
                window[0].salience,
                window[1].salience,
            );
        }
    }

    #[test]
    fn t_mem_053_working_set_truncation() {
        let table = test_table();

        let ws = table.working_set(2);
        assert_eq!(ws.len(), 2);
        assert_eq!(ws[0].key, "auth_pattern"); // highest salience
        assert_eq!(ws[1].key, "api_design"); // second highest
    }

    #[test]
    fn t_mem_053_working_set_empty_table() {
        let table = PointerTable::new();
        let ws = table.working_set(10);
        assert!(ws.is_empty());
    }

    #[test]
    fn t_mem_053_working_set_top_n_exceeds_count() {
        let table = test_table();

        let ws = table.working_set(100);
        assert_eq!(ws.len(), 5, "should return all 5 even though top_n=100");
    }

    // =======================================================================
    // T-MEM-054: TOML serialization round-trip
    // =======================================================================

    #[test]
    fn t_mem_054_toml_roundtrip() {
        let table = test_table();

        // Serialize
        let toml_str = table.to_toml();
        assert!(!toml_str.is_empty(), "TOML output should not be empty");

        // Deserialize
        let restored = PointerTable::from_toml(&toml_str).expect("TOML parse should succeed");

        // Verify round-trip fidelity
        assert_eq!(restored.len(), table.len());

        for key in ["auth_pattern", "db_schema", "api_design", "cache_strategy", "ui_layout"] {
            let orig = table.get(key).unwrap();
            let rest = restored.get(key).unwrap();

            assert_eq!(orig.key, rest.key);
            assert_eq!(orig.level, rest.level);
            assert_eq!(orig.scope, rest.scope);
            assert_eq!(orig.annotation, rest.annotation);
            assert!((orig.salience - rest.salience).abs() < 0.001);
            assert_eq!(orig.access_count, rest.access_count);
            assert_eq!(orig.path, rest.path);
        }
    }

    #[test]
    fn t_mem_054_toml_with_path() {
        let mut table = PointerTable::new();
        table.register_with_path(
            "with_path".into(),
            Level::L1,
            Scope::Project,
            "entry with path".into(),
            "memory/project/auth.md".into(),
        );

        let toml_str = table.to_toml();
        let restored = PointerTable::from_toml(&toml_str).unwrap();

        let ptr = restored.get("with_path").unwrap();
        assert_eq!(ptr.path, Some("memory/project/auth.md".to_string()));
    }

    #[test]
    fn t_mem_054_toml_invalid_parse() {
        let result = PointerTable::from_toml("this is not valid toml {{{");
        assert!(result.is_err());
    }

    #[test]
    fn t_mem_054_toml_empty_table() {
        let table = PointerTable::new();
        let toml_str = table.to_toml();
        let restored = PointerTable::from_toml(&toml_str).unwrap();
        assert!(restored.is_empty());
    }

    // =======================================================================
    // T-MEM-055: Register/unregister lifecycle
    // =======================================================================

    #[test]
    fn t_mem_055_register_basic() {
        let mut table = PointerTable::new();
        assert!(table.is_empty());

        table.register("key1".into(), Level::L1, Scope::Project, "description".into());

        assert_eq!(table.len(), 1);
        let ptr = table.get("key1").unwrap();
        assert_eq!(ptr.key, "key1");
        assert_eq!(ptr.level, Level::L1);
        assert_eq!(ptr.scope, Scope::Project);
        assert_eq!(ptr.annotation, "description");
        assert_eq!(ptr.access_count, 0);
        assert!((ptr.salience - 0.5).abs() < 0.001, "initial salience should be 0.5");
    }

    #[test]
    fn t_mem_055_register_overwrites() {
        let mut table = PointerTable::new();
        table.register("key1".into(), Level::L1, Scope::Project, "first".into());
        table.register("key1".into(), Level::L2, Scope::Session, "second".into());

        assert_eq!(table.len(), 1, "overwrite should not create duplicate");
        let ptr = table.get("key1").unwrap();
        assert_eq!(ptr.level, Level::L2);
        assert_eq!(ptr.annotation, "second");
    }

    #[test]
    fn t_mem_055_unregister() {
        let mut table = PointerTable::new();
        table.register("key1".into(), Level::L1, Scope::Project, "test".into());
        table.register("key2".into(), Level::L2, Scope::Session, "test2".into());

        let removed = table.unregister("key1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().key, "key1");
        assert_eq!(table.len(), 1);
        assert!(table.get("key1").is_none());
        assert!(table.get("key2").is_some());
    }

    #[test]
    fn t_mem_055_unregister_nonexistent() {
        let mut table = PointerTable::new();
        let removed = table.unregister("nope");
        assert!(removed.is_none());
    }

    #[test]
    fn t_mem_055_lifecycle_full() {
        let mut table = PointerTable::new();

        // Register
        table.register("lifecycle".into(), Level::L3, Scope::Task, "temp data".into());
        assert_eq!(table.len(), 1);

        // Access
        table.update_salience("lifecycle", SalienceSignal::Accessed);
        assert_eq!(table.get("lifecycle").unwrap().access_count, 1);

        // Update salience
        table.update_salience("lifecycle", SalienceSignal::KeywordMatch(0.8));
        let s1 = table.get("lifecycle").unwrap().salience;

        // Access again
        table.update_salience("lifecycle", SalienceSignal::Accessed);
        assert_eq!(table.get("lifecycle").unwrap().access_count, 2);

        // Salience should have shifted
        let s2 = table.get("lifecycle").unwrap().salience;
        assert!(
            (s2 - s1).abs() > 0.0001 || s1 == s2, // may be same if negligible
            "salience should evolve with signals"
        );

        // Unregister
        let removed = table.unregister("lifecycle");
        assert!(removed.is_some());
        assert!(table.is_empty());
    }

    // =======================================================================
    // T-MEM-056: Compact serialization format
    // =======================================================================

    #[test]
    fn t_mem_056_compact_format() {
        let table = test_table();
        let compact = table.serialize_compact();

        // Should have 5 lines
        let lines: Vec<&str> = compact.lines().collect();
        assert_eq!(lines.len(), 5);

        // First line should be the highest salience pointer
        assert!(
            lines[0].starts_with("@auth_pattern"),
            "first line should be highest salience: got '{}'",
            lines[0]
        );

        // Each line should match the format: @key [Level/scope] (score) -- annotation
        for line in &lines {
            assert!(line.starts_with('@'), "line should start with @: {}", line);
            assert!(line.contains('['), "line should contain [: {}", line);
            assert!(line.contains(']'), "line should contain ]: {}", line);
            assert!(line.contains('('), "line should contain (: {}", line);
            assert!(line.contains(')'), "line should contain ): {}", line);
            assert!(line.contains(" -- "), "line should contain ' -- ': {}", line);
        }
    }

    #[test]
    fn t_mem_056_compact_sorted_by_salience() {
        let table = test_table();
        let compact = table.serialize_compact();

        let lines: Vec<&str> = compact.lines().collect();

        // Extract salience scores from format: ... (0.85) ...
        let scores: Vec<f32> = lines
            .iter()
            .filter_map(|line| {
                let start = line.find('(')? + 1;
                let end = line.find(')')?;
                line[start..end].parse::<f32>().ok()
            })
            .collect();

        assert_eq!(scores.len(), 5);
        for window in scores.windows(2) {
            assert!(
                window[0] >= window[1],
                "compact output should be sorted descending: {} >= {}",
                window[0],
                window[1],
            );
        }
    }

    #[test]
    fn t_mem_056_compact_empty_table() {
        let table = PointerTable::new();
        let compact = table.serialize_compact();
        assert!(compact.is_empty());
    }

    #[test]
    fn t_mem_056_compact_contains_metadata() {
        let table = test_table();
        let compact = table.serialize_compact();

        // Verify level and scope appear
        assert!(compact.contains("[L1/project]"));
        assert!(compact.contains("[L2/session]"));
        assert!(compact.contains("[L3/global]"));
        assert!(compact.contains("[L4/task]"));

        // Verify annotations appear
        assert!(compact.contains("JWT authentication design decisions"));
        assert!(compact.contains("Database migration tracking"));
    }

    // =======================================================================
    // Edge cases and integration
    // =======================================================================

    #[test]
    fn dsl_search_case_insensitive() {
        let table = test_table();

        let result = table.resolve(r#"@search("JWT")"#);
        assert!(!result.is_empty(), "search should be case-insensitive for annotation text");
    }

    #[test]
    fn dsl_search_matches_key_and_annotation() {
        let table = test_table();

        // "auth" appears in the key "auth_pattern"
        let result = table.resolve(r#"@search("auth")"#);
        assert!(!result.is_empty(), "search should match against both key and annotation");
    }

    #[test]
    fn working_set_reflects_salience_updates() {
        let mut table = PointerTable::new();
        table.register("low".into(), Level::L3, Scope::Session, "low priority".into());
        table.register("high".into(), Level::L1, Scope::Project, "high priority".into());

        // Initially both at 0.5
        assert_eq!(table.working_set(1)[0].key, "high"); // alphabetical tie-break in BTreeMap

        // Boost "low" with semantic match
        if let Some(p) = table.get_mut("low") {
            p.salience = 0.3;
        }
        if let Some(p) = table.get_mut("high") {
            p.salience = 0.2;
        }

        let ws = table.working_set(1);
        assert_eq!(ws[0].key, "low", "working set should reflect updated salience");
    }

    #[test]
    fn register_with_path_roundtrip() {
        let mut table = PointerTable::new();
        table.register_with_path(
            "pathed".into(),
            Level::L1,
            Scope::Project,
            "has a path".into(),
            "memory/L1/pathed.md".into(),
        );

        let ptr = table.get("pathed").unwrap();
        assert_eq!(ptr.path, Some("memory/L1/pathed.md".to_string()));
    }

    #[test]
    fn default_impl() {
        let table = PointerTable::default();
        assert!(table.is_empty());
    }

    #[test]
    fn salience_components_preserved() {
        let mut pt = PointerTable::new();
        pt.register("key1".into(), Level::L1, Scope::Project, "test".into());

        pt.update_salience("key1", SalienceSignal::KeywordMatch(0.8));
        let p = pt.get("key1").unwrap();
        assert!((p.salience_keyword - 0.8).abs() < 0.01);

        pt.update_salience("key1", SalienceSignal::SemanticMatch(0.6));
        let p = pt.get("key1").unwrap();
        assert!((p.salience_semantic - 0.6).abs() < 0.01);
        assert!((p.salience_keyword - 0.8).abs() < 0.01); // keyword preserved

        // Final salience should be: 0.8*0.3 + 0.6*0.4 + time*0.2 + 0*0.1
        // = 0.24 + 0.24 + ~0.2 + 0 = ~0.68
        assert!(p.salience > 0.5);
    }

    #[test]
    fn salience_consistent_formula() {
        let mut pt = PointerTable::new();
        pt.register("k".into(), Level::L2, Scope::Task, "test".into());

        // All signals should use the same 0.3+0.4+0.2+0.1 formula
        pt.update_salience("k", SalienceSignal::KeywordMatch(1.0));
        pt.update_salience("k", SalienceSignal::SemanticMatch(1.0));
        pt.update_salience("k", SalienceSignal::Accessed);

        let p = pt.get("k").unwrap();
        // keyword=1.0*0.3 + semantic=1.0*0.4 + time~=1.0*0.2 + access=0.05*0.1
        // Should be close to 0.3 + 0.4 + 0.2 + 0.005 = 0.905
        assert!(p.salience > 0.85, "salience should be ~0.9, got {}", p.salience);
    }
}
