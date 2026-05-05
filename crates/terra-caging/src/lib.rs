//! Terra Caging Engine
//!
//! Implementation of Caging Engineering's 8 cages.
//! Each cage is a constraint checker that shapes the agent's
//! reachable state space.
//!
//! Architecture:
//! - `cage`: core Cage trait and CageCheckResult
//! - `sandbox`: file system permission enforcement (Action Cage)
//! - `engine`: orchestrates all cages in sequence

pub mod cage;
pub mod sandbox;
pub mod engine;

pub use cage::{Cage, CageCheckResult};
pub use sandbox::{SandboxGuard, SandboxViolation};
pub use engine::CagingEngine;
