use std::collections::HashMap;

/// A simple vector search index using TF-IDF cosine similarity.
///
/// This is the fallback implementation when no ONNX embedding model
/// is available. It operates on text content extracted from MessagePack
/// entries. Future: replace with FAISS/usearch + sentence-transformers.
///
/// Architecture:
/// - Embedding trait defines the interface (swappable)
/// - TfIdfIndex is the default implementation
/// - Multi-head search: multiple queries, merge + deduplicate
pub struct VectorIndex {
    entries: Vec<IndexEntry>,
}

struct IndexEntry {
    key: String,
    tokens: HashMap<String, f32>, // TF-IDF weights
}

impl VectorIndex {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Add or update an entry in the index.
    pub fn upsert(&mut self, key: &str, text: &str) {
        // Remove existing entry if present
        self.entries.retain(|e| e.key != key);

        let tokens = tokenize_and_weight(text);
        self.entries.push(IndexEntry {
            key: key.to_string(),
            tokens,
        });
    }

    /// Remove an entry from the index.
    pub fn remove(&mut self, key: &str) {
        self.entries.retain(|e| e.key != key);
    }

    /// Search for entries similar to the query. Returns (key, score) pairs
    /// sorted by descending similarity.
    pub fn search(&self, query: &str, top_k: usize) -> Vec<(String, f32)> {
        let query_tokens = tokenize_and_weight(query);

        let mut scored: Vec<(String, f32)> = self.entries
            .iter()
            .map(|entry| {
                let score = cosine_similarity(&query_tokens, &entry.tokens);
                (entry.key.clone(), score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// Multi-head search: run multiple queries (one per perspective),
    /// merge results, deduplicate by key, keep highest score.
    pub fn multi_head_search(
        &self,
        queries: &[String],
        top_k: usize,
    ) -> Vec<(String, f32)> {
        let mut combined: HashMap<String, f32> = HashMap::new();

        for query in queries {
            let results = self.search(query, top_k);
            for (key, score) in results {
                let entry = combined.entry(key).or_insert(0.0);
                if score > *entry {
                    *entry = score;
                }
            }
        }

        let mut sorted: Vec<(String, f32)> = combined.into_iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(top_k);
        sorted
    }

    /// Number of entries in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Tokenize text and compute TF weights.
fn tokenize_and_weight(text: &str) -> HashMap<String, f32> {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(String::from)
        .collect();

    let total = words.len() as f32;
    if total == 0.0 {
        return HashMap::new();
    }

    let mut freq: HashMap<String, f32> = HashMap::new();
    for word in &words {
        *freq.entry(word.clone()).or_insert(0.0) += 1.0;
    }

    // Normalize to TF
    for val in freq.values_mut() {
        *val /= total;
    }

    freq
}

/// Cosine similarity between two TF-IDF vectors.
fn cosine_similarity(a: &HashMap<String, f32>, b: &HashMap<String, f32>) -> f32 {
    let dot: f32 = a.iter()
        .filter_map(|(k, v)| b.get(k).map(|bv| v * bv))
        .sum();

    let mag_a: f32 = a.values().map(|v| v * v).sum::<f32>().sqrt();
    let mag_b: f32 = b.values().map(|v| v * v).sum::<f32>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }

    dot / (mag_a * mag_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated_index() -> VectorIndex {
        let mut idx = VectorIndex::new();
        idx.upsert("auth_pattern", "authentication login password validation security");
        idx.upsert("db_schema", "database schema migration table column index");
        idx.upsert("api_design", "REST API endpoint request response JSON");
        idx.upsert("auth_bug", "authentication bug empty password login failure");
        idx.upsert("cache_strategy", "cache redis memory TTL eviction performance");
        idx.upsert("security_audit", "security vulnerability XSS injection authentication");
        idx.upsert("ui_layout", "frontend layout CSS grid flexbox responsive");
        idx.upsert("test_coverage", "test unit integration coverage assertion");
        idx.upsert("deploy_config", "deployment docker container kubernetes config");
        idx.upsert("perf_optimize", "performance optimization query database index");
        idx
    }

    // T-MD-027: vector search returns relevant results
    #[test]
    fn search_returns_relevant() {
        let idx = populated_index();
        let results = idx.search("authentication password security", 3);

        assert!(!results.is_empty(), "should find results");
        // Top results should be auth-related
        let top_key = &results[0].0;
        assert!(
            top_key.contains("auth") || top_key.contains("security"),
            "top result '{}' should be auth/security related", top_key
        );
        assert!(results[0].1 > 0.0, "score should be positive");
    }

    // T-MD-028: multi-head search merges results from multiple perspectives
    #[test]
    fn multihead_search_merges() {
        let idx = populated_index();

        let queries = vec![
            "authentication security vulnerability".to_string(),  // security perspective
            "performance optimization query".to_string(),          // perf perspective
        ];

        let results = idx.multi_head_search(&queries, 5);

        // Should have results from BOTH perspectives
        let keys: Vec<&str> = results.iter().map(|r| r.0.as_str()).collect();

        let has_auth = keys.iter().any(|k| k.contains("auth") || k.contains("security"));
        let has_perf = keys.iter().any(|k| k.contains("perf") || k.contains("db"));

        assert!(has_auth, "should have auth/security results: {:?}", keys);
        assert!(has_perf, "should have perf/db results: {:?}", keys);
        assert!(results.len() >= 2, "should have at least 2 merged results");
    }

    #[test]
    fn search_no_results_for_unrelated() {
        let idx = populated_index();
        let results = idx.search("quantum physics particle accelerator", 3);
        // Should have very low or no matches
        assert!(
            results.is_empty() || results[0].1 < 0.1,
            "unrelated query should have no/low matches"
        );
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut idx = VectorIndex::new();
        idx.upsert("key1", "old content about databases");
        idx.upsert("key1", "new content about authentication security");

        assert_eq!(idx.len(), 1);

        let results = idx.search("authentication", 1);
        assert_eq!(results[0].0, "key1");
    }

    #[test]
    fn remove_entry() {
        let mut idx = VectorIndex::new();
        idx.upsert("key1", "test content");
        assert_eq!(idx.len(), 1);

        idx.remove("key1");
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn empty_query() {
        let idx = populated_index();
        let results = idx.search("", 3);
        assert!(results.is_empty());
    }

    #[test]
    fn empty_index() {
        let idx = VectorIndex::new();
        let results = idx.search("anything", 3);
        assert!(results.is_empty());
    }

    #[test]
    fn top_k_limits_results() {
        let idx = populated_index();
        let results = idx.search("database", 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn scores_are_sorted_descending() {
        let idx = populated_index();
        let results = idx.search("authentication security", 5);
        for window in results.windows(2) {
            assert!(window[0].1 >= window[1].1, "scores should be descending");
        }
    }
}
