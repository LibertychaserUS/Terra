use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::MemoryError;

/// Supported artifact types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactType {
    Plan,
    Spec,
    Test,
    Review,
}

impl ArtifactType {
    pub fn as_str(&self) -> &str {
        match self {
            ArtifactType::Plan => "plan",
            ArtifactType::Spec => "spec",
            ArtifactType::Test => "test",
            ArtifactType::Review => "review",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(ArtifactType::Plan),
            "spec" => Some(ArtifactType::Spec),
            "test" => Some(ArtifactType::Test),
            "review" => Some(ArtifactType::Review),
            _ => None,
        }
    }
}

/// Metadata for a stored artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMeta {
    pub id: String,
    pub artifact_type: ArtifactType,
    pub version: u32,
    pub path: PathBuf,
    pub created_at: String,
}

/// Artifact Store — versioned MessagePack file storage.
///
/// Each save creates a new immutable version. Files stored at:
///   {root}/artifacts/{type}/{type}-{version:04}.msgpack
///
/// Design: similar to Git's object store — artifacts are immutable,
/// pointers (in Memory Database) reference the current version.
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    /// Create a store at the given root directory.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Save a new version of an artifact. Returns metadata.
    pub fn save(
        &self,
        artifact_type: ArtifactType,
        content: &[u8],
    ) -> Result<ArtifactMeta, MemoryError> {
        let type_dir = self.root.join("artifacts").join(artifact_type.as_str());
        std::fs::create_dir_all(&type_dir)
            .map_err(|e| MemoryError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1), Some(e.to_string())
            )))?;

        let next_version = self.next_version(&artifact_type)?;
        let filename = format!("{}-{:04}.msgpack", artifact_type.as_str(), next_version);
        let file_path = type_dir.join(&filename);

        std::fs::write(&file_path, content)
            .map_err(|e| MemoryError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1), Some(e.to_string())
            )))?;

        Ok(ArtifactMeta {
            id: Uuid::new_v4().to_string(),
            artifact_type,
            version: next_version,
            path: file_path,
            created_at: Utc::now().to_rfc3339(),
        })
    }

    /// Load an artifact by type and optional version.
    /// If version is None, loads the latest.
    pub fn load(
        &self,
        artifact_type: &ArtifactType,
        version: Option<u32>,
    ) -> Result<(ArtifactMeta, Vec<u8>), MemoryError> {
        let ver = match version {
            Some(v) => v,
            None => {
                let latest = self.latest_version(artifact_type)?;
                match latest {
                    Some(v) => v,
                    None => return Err(MemoryError::NotFound(
                        format!("no {} artifacts", artifact_type.as_str())
                    )),
                }
            }
        };

        let type_dir = self.root.join("artifacts").join(artifact_type.as_str());
        let filename = format!("{}-{:04}.msgpack", artifact_type.as_str(), ver);
        let file_path = type_dir.join(&filename);

        if !file_path.exists() {
            return Err(MemoryError::NotFound(
                format!("{} version {}", artifact_type.as_str(), ver)
            ));
        }

        let content = std::fs::read(&file_path)
            .map_err(|e| MemoryError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1), Some(e.to_string())
            )))?;

        let meta = ArtifactMeta {
            id: Uuid::new_v4().to_string(),
            artifact_type: artifact_type.clone(),
            version: ver,
            path: file_path,
            created_at: String::new(), // not tracked on load
        };

        Ok((meta, content))
    }

    /// List all versions of an artifact type.
    pub fn list(&self, artifact_type: &ArtifactType) -> Result<Vec<ArtifactMeta>, MemoryError> {
        let type_dir = self.root.join("artifacts").join(artifact_type.as_str());
        if !type_dir.exists() {
            return Ok(vec![]);
        }

        let prefix = format!("{}-", artifact_type.as_str());
        let mut metas = Vec::new();

        let entries = std::fs::read_dir(&type_dir)
            .map_err(|e| MemoryError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1), Some(e.to_string())
            )))?;

        for entry in entries {
            let entry = entry.map_err(|e| MemoryError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1), Some(e.to_string())
            )))?;

            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && name.ends_with(".msgpack") {
                let ver_str = name
                    .strip_prefix(&prefix)
                    .and_then(|s| s.strip_suffix(".msgpack"))
                    .unwrap_or("0");
                let version: u32 = ver_str.parse().unwrap_or(0);

                metas.push(ArtifactMeta {
                    id: String::new(),
                    artifact_type: artifact_type.clone(),
                    version,
                    path: entry.path(),
                    created_at: String::new(),
                });
            }
        }

        metas.sort_by_key(|m| m.version);
        Ok(metas)
    }

    /// Get the next version number for an artifact type.
    fn next_version(&self, artifact_type: &ArtifactType) -> Result<u32, MemoryError> {
        match self.latest_version(artifact_type)? {
            Some(v) => Ok(v + 1),
            None => Ok(1),
        }
    }

    /// Get the latest version number, or None if no artifacts exist.
    fn latest_version(&self, artifact_type: &ArtifactType) -> Result<Option<u32>, MemoryError> {
        let metas = self.list(artifact_type)?;
        Ok(metas.last().map(|m| m.version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> (ArtifactStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path().to_path_buf());
        (store, dir)
    }

    // T-AS-001: save creates version
    #[test]
    fn save_creates_version() {
        let (store, _dir) = test_store();
        let content = rmp_serde::to_vec_named(&serde_json::json!({"step": "plan v1"})).unwrap();
        let meta = store.save(ArtifactType::Plan, &content).unwrap();
        assert_eq!(meta.version, 1);
        assert_eq!(meta.artifact_type, ArtifactType::Plan);
        assert!(meta.path.exists());
    }

    // T-AS-002: save increments version
    #[test]
    fn save_increments_version() {
        let (store, _dir) = test_store();
        let c1 = rmp_serde::to_vec_named(&serde_json::json!({"v": 1})).unwrap();
        let c2 = rmp_serde::to_vec_named(&serde_json::json!({"v": 2})).unwrap();

        let m1 = store.save(ArtifactType::Plan, &c1).unwrap();
        let m2 = store.save(ArtifactType::Plan, &c2).unwrap();

        assert_eq!(m1.version, 1);
        assert_eq!(m2.version, 2);
    }

    // T-AS-003: load latest
    #[test]
    fn load_latest() {
        let (store, _dir) = test_store();
        let c1 = rmp_serde::to_vec_named(&serde_json::json!({"v": 1})).unwrap();
        let c2 = rmp_serde::to_vec_named(&serde_json::json!({"v": 2})).unwrap();

        store.save(ArtifactType::Plan, &c1).unwrap();
        store.save(ArtifactType::Plan, &c2).unwrap();

        let (meta, content) = store.load(&ArtifactType::Plan, None).unwrap();
        assert_eq!(meta.version, 2);
        let decoded: serde_json::Value = rmp_serde::from_slice(&content).unwrap();
        assert_eq!(decoded["v"], 2);
    }

    // T-AS-004: load specific version
    #[test]
    fn load_specific_version() {
        let (store, _dir) = test_store();
        let c1 = rmp_serde::to_vec_named(&serde_json::json!({"data": "first"})).unwrap();
        let c2 = rmp_serde::to_vec_named(&serde_json::json!({"data": "second"})).unwrap();

        store.save(ArtifactType::Spec, &c1).unwrap();
        store.save(ArtifactType::Spec, &c2).unwrap();

        let (meta, content) = store.load(&ArtifactType::Spec, Some(1)).unwrap();
        assert_eq!(meta.version, 1);
        let decoded: serde_json::Value = rmp_serde::from_slice(&content).unwrap();
        assert_eq!(decoded["data"], "first");
    }

    // T-AS-005: list artifacts
    #[test]
    fn list_artifacts() {
        let (store, _dir) = test_store();
        for i in 0..3 {
            let c = rmp_serde::to_vec_named(&serde_json::json!({"i": i})).unwrap();
            store.save(ArtifactType::Test, &c).unwrap();
        }

        let list = store.list(&ArtifactType::Test).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].version, 1);
        assert_eq!(list[2].version, 3);
    }

    // T-AS-006: content is msgpack
    #[test]
    fn content_is_msgpack() {
        let (store, _dir) = test_store();
        let original = serde_json::json!({"key": "value", "num": 42});
        let packed = rmp_serde::to_vec_named(&original).unwrap();
        store.save(ArtifactType::Review, &packed).unwrap();

        let (_, content) = store.load(&ArtifactType::Review, None).unwrap();
        let decoded: serde_json::Value = rmp_serde::from_slice(&content).unwrap();
        assert_eq!(decoded["num"], 42);
    }

    // T-AS-007: artifact types all supported
    #[test]
    fn all_artifact_types() {
        let (store, _dir) = test_store();
        let c = rmp_serde::to_vec_named(&serde_json::json!({"t": true})).unwrap();

        for t in [ArtifactType::Plan, ArtifactType::Spec, ArtifactType::Test, ArtifactType::Review] {
            store.save(t.clone(), &c).unwrap();
            let (meta, _) = store.load(&t, None).unwrap();
            assert_eq!(meta.artifact_type, t);
        }
    }

    // T-AS-008: load nonexistent returns error
    #[test]
    fn load_nonexistent() {
        let (store, _dir) = test_store();
        let result = store.load(&ArtifactType::Plan, None);
        assert!(result.is_err());
    }

    #[test]
    fn list_empty_type() {
        let (store, _dir) = test_store();
        let list = store.list(&ArtifactType::Plan).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn load_specific_nonexistent_version() {
        let (store, _dir) = test_store();
        let c = rmp_serde::to_vec_named(&serde_json::json!({"v": 1})).unwrap();
        store.save(ArtifactType::Plan, &c).unwrap();

        let result = store.load(&ArtifactType::Plan, Some(99));
        assert!(result.is_err());
    }
}
