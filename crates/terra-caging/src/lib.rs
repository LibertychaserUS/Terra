//! Terra's cages: predicates that decide whether a proposed action may
//! happen. They do not choose actions and do not change them.
//!
//! - `cage`: the `Cage` trait, `CageContext`, `CageAction`
//! - `engine`: the five runtime cages and the engine that runs them in order
//! - `sandbox`: path resolution and the project write boundary
//! - `constitution`: `.terra/constitution.toml`, the project's rules
//! - `output`: phrase filter over model answers

pub mod cage;
pub mod constitution;
pub mod engine;
pub mod output;
pub mod sandbox;

pub use cage::{Cage, CageAction, CageCheckResult, CageContext};
pub use constitution::{Constitution, McpServerConfig};
pub use engine::{ActionCage, CagingEngine, CagingResult, MemoryCage, MetaCage, SpecificationCage, VerificationCage};
pub use output::{check_output, OutputCheckResult};
pub use sandbox::{SandboxGuard, SandboxViolation};
