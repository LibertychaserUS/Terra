//! Terra Intelligence Core
//!
//! Agent's built-in first principles. From "one", not "zero".
//!
//! Architecture:
//! - `types`: Rule, Perspective, Severity, RuleCategory, DiagnosticThreshold
//! - `framework`: Universal reasoning/behavioral/diagnostic rules (L0, read-only)
//! - `domain`: DomainModule trait and built-in modules (coding, algorithm)
//! - `core`: IntelligenceCore composing universal + domain modules
//! - `diagnostic`: Measurable self-diagnostic detection (D-001/D-002/D-003)

pub mod types;
pub mod framework;
pub mod domain;
pub mod core;
pub mod diagnostic;
pub mod soul;

pub use self::core::IntelligenceCore;
pub use diagnostic::DiagnosticSignal;
pub use domain::DomainModule;
pub use soul::{SoulKind, SoulTemplate, soul_template};
pub use types::{DiagnosticThreshold, Perspective, Rule, RuleCategory, Severity, TaskType};
