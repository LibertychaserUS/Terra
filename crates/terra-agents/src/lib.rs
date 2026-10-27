//! The agent: a tool loop over the executor, Goal Mode with a specified
//! goal and a verified completion claim, and the user's slash commands.

pub mod agent;
pub mod codebase;
pub mod commands;
pub mod consolidate;
pub mod run;
pub mod session;
pub mod session_store;
pub mod spec;

pub use agent::{Agent, ConfirmCallback, Output};
pub use run::{run_task, TaskReport};
pub use session::{Session, SessionMode, Verification, VERIFIER_AGENT};
pub use session_store::{SessionRecord, SessionStore};
pub use spec::Spec;
