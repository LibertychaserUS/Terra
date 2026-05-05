//! Terra Language Compilation Layer
//!
//! JSP-inspired compilation of user natural language into
//! standardized Spec IR with terminology normalization.
//!
//! Passes:
//! 1. Lexical — extract file paths, line numbers, entities
//! 2. Terminology — normalize informal terms to canonical forms
//! 3-5. (Future: Semantic, Completeness, Ambiguity — require LLM)

pub mod spec_ir;

pub use spec_ir::{SpecIR, Target, TermMapping, TermTable};
