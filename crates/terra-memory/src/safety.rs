use crate::error::MemoryError;
use crate::store::SqliteStore;
use crate::types::Level;

/// Safety enforcer — decoupled from storage.
///
/// Checks ownership, lifetime, and promotion rules before
/// allowing mutations. This is the "Rust-style borrow checker"
/// for the memory system.
pub struct SafetyEnforcer;

impl SafetyEnforcer {
    /// Check if agent is allowed to write/update/delete a key.
    pub fn check_write_permission(
        store: &SqliteStore,
        key: &str,
        agent: &str,
    ) -> Result<(), MemoryError> {
        if let Some(owner) = store.get_owner(key)? {
            if owner != agent {
                return Err(MemoryError::PermissionDenied {
                    agent: agent.to_string(),
                    key: key.to_string(),
                });
            }
        }
        // Key doesn't exist yet → write is allowed (will be owned by agent)
        Ok(())
    }

    /// Check if promotion to target level is allowed.
    pub fn check_promotion(
        store: &SqliteStore,
        key: &str,
        target: &Level,
        reviewed: bool,
    ) -> Result<(), MemoryError> {
        let entry = store
            .read(key)?
            .ok_or_else(|| MemoryError::NotFound(key.to_string()))?;

        if !entry.alive {
            return Err(MemoryError::Dangling(key.to_string()));
        }

        // L6 promotion requires human review
        if entry.level == Level::L6 && !reviewed {
            return Err(MemoryError::PromotionBlocked {
                reason: "L6 model memory candidate requires human review".into(),
            });
        }

        // Cannot promote to a lower (more authoritative) level than current
        // unless going from task (L2) to project (L1) which is normal
        // The valid promotions: L2→L1, L3→L1, L4→L1, L5→L1
        if *target >= entry.level {
            return Err(MemoryError::PromotionBlocked {
                reason: format!(
                    "cannot promote from {} to {} (target must be more authoritative)",
                    entry.level.as_str(),
                    target.as_str()
                ),
            });
        }

        // Cannot promote directly to L0 from non-L1
        if *target == Level::L0 && entry.level != Level::L1 {
            return Err(MemoryError::PromotionBlocked {
                reason: "only L1 entries can be promoted to L0".into(),
            });
        }

        Ok(())
    }

    /// Check that a read target is alive (not dangling).
    pub fn check_alive(
        store: &SqliteStore,
        key: &str,
    ) -> Result<(), MemoryError> {
        if let Some(entry) = store.read(key)? {
            if !entry.alive {
                return Err(MemoryError::Dangling(key.to_string()));
            }
            Ok(())
        } else {
            Err(MemoryError::NotFound(key.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{new_entry, SqliteStore};
    use crate::types::{Level, Scope};

    fn setup() -> SqliteStore {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .insert(&new_entry("owned_by_main", Scope::Project, Level::L1, "main", b"x".to_vec()))
            .unwrap();
        store
    }

    // T-M-011
    #[test]
    fn non_owner_cannot_update() {
        let store = setup();
        let result = SafetyEnforcer::check_write_permission(&store, "owned_by_main", "worker");
        assert!(matches!(result, Err(MemoryError::PermissionDenied { .. })));
    }

    // T-M-012
    #[test]
    fn non_owner_can_read() {
        let store = setup();
        // Read doesn't go through SafetyEnforcer — anyone can read
        let entry = store.read("owned_by_main").unwrap();
        assert!(entry.is_some());
    }

    // T-M-025
    #[test]
    fn promote_l6_requires_review() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .insert(&new_entry("candidate", Scope::Project, Level::L6, "main", b"x".to_vec()))
            .unwrap();
        let result = SafetyEnforcer::check_promotion(&store, "candidate", &Level::L1, false);
        assert!(matches!(result, Err(MemoryError::PromotionBlocked { .. })));
    }

    // T-M-024
    #[test]
    fn promote_l2_to_l1_allowed() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .insert(&new_entry("task_mem", Scope::Task, Level::L2, "main", b"x".to_vec()))
            .unwrap();
        let result = SafetyEnforcer::check_promotion(&store, "task_mem", &Level::L1, false);
        assert!(result.is_ok());
    }

    #[test]
    fn promote_backwards_blocked() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .insert(&new_entry("already_l1", Scope::Project, Level::L1, "main", b"x".to_vec()))
            .unwrap();
        let result = SafetyEnforcer::check_promotion(&store, "already_l1", &Level::L2, false);
        assert!(matches!(result, Err(MemoryError::PromotionBlocked { .. })));
    }

    #[test]
    fn check_alive_on_deleted() {
        let store = setup();
        store.soft_delete("owned_by_main").unwrap();
        let result = SafetyEnforcer::check_alive(&store, "owned_by_main");
        assert!(matches!(result, Err(MemoryError::Dangling(_))));
    }
}
