//! Terra Caging Engine
//!
//! Implementation of Caging Engineering's 8 cages.
//! Each cage is a constraint checker that shapes the agent's
//! reachable state space.
//!
//! Architecture:
//! - `cage`: core Cage trait, CageCheckResult, CageAction, KnowledgeSource
//! - `sandbox`: file system permission enforcement (Action Cage)
//! - `constitution`: TOML-based project constitution (immutable at runtime)
//! - `engine`: orchestrates all cages in sequence (7 cages active)

pub mod cage;
pub mod sandbox;
pub mod constitution;
pub mod engine;

pub use cage::{Cage, CageAction, CageCheckResult, CageContext, KnowledgeSource};
pub use constitution::Constitution;
pub use sandbox::{SandboxGuard, SandboxViolation};
pub use engine::{CagingEngine, ConvergenceCage, KnowledgeCage};
