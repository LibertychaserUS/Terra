//! Terra Intelligence Core
//!
//! Agent's built-in first principles. From "one", not "zero".
//!
//! Architecture:
//! - `types`: Rule, Perspective, Severity, RuleCategory
//! - `framework`: Universal reasoning/behavioral/diagnostic rules (L0, read-only)
//! - `domain`: DomainModule trait and built-in modules (coding, algorithm)
//! - `core`: IntelligenceCore composing universal + domain modules

pub mod types;
pub mod framework;
pub mod domain;
pub mod core;

pub use self::core::IntelligenceCore;
pub use domain::DomainModule;
pub use types::{Perspective, Rule, RuleCategory, Severity};
