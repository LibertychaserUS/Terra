//! Terra Multi-Agent Runtime
//!
//! Orchestrator that follows the canonical pipeline:
//! User Input → Compilation → SpecIR → IC consultation
//! → Caging gate → Command Executor → Result
//!
//! Agent roles:
//! - Main (Orchestrator): full pipeline, strongest model
//! - Worker: coding tasks, filtered rules, balanced model
//! - Explorer: search/research, fast model
//! - CageMonitor: diagnostic checks only, no mutations

pub mod orchestrator;
pub mod roles;
pub mod dispatch;
pub mod session;
pub mod drift;
pub mod profile;
pub mod subagent;
pub mod skill;
pub mod repl;
pub mod session_store;
pub mod status;
pub mod codebase;

pub use orchestrator::Orchestrator;
pub use roles::{AgentRole, CageMonitorAgent, ExplorerAgent, WorkerAgent};
pub use dispatch::{ModelDispatcher, ModelCapability, DispatchParams};
pub use session::{SessionManager, SessionMode, SessionStatus, SleepSettings};
pub use drift::{DriftDetector, DriftConfig};
pub use profile::{AgentProfile, MemoryView};
pub use subagent::SubAgentRunner;
pub use skill::{SkillRegistry, SkillDef, TrustLevel};
pub use repl::Repl;
