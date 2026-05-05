//! Terra Multi-Agent Runtime
//!
//! Orchestrator that follows the canonical pipeline:
//! User Input → Compilation → SpecIR → IC consultation
//! → Caging gate → Command Executor → Result

pub mod orchestrator;

pub use orchestrator::Orchestrator;
