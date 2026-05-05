//! Terra Language Compilation Layer
//!
//! JSP-inspired compilation of user natural language into
//! standardized Spec IR with terminology normalization.
//!
//! Passes:
//! 1. Lexical — extract file paths, line numbers, entities (program)
//! 2. Terminology — normalize informal terms to canonical forms (program)
//! 3. Semantic Resolution — map entities to project objects (LLM-assisted, optional)
//! 4. Completeness Check — identify missing info (LLM-assisted, optional)
//! 5. Ambiguity Detection — flag vague references (LLM-assisted, optional)

pub mod spec_ir;

pub use spec_ir::{SpecIR, Target, TermMapping, TermTable};
