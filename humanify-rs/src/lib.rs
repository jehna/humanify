//! # humanify-rs
//!
//! A JavaScript deobfuscator that renames minified variables and functions
//! using a callback mechanism to determine new names based on context.
//!
//! ## Features
//!
//! - Parse JavaScript to AST and rename identifiers
//! - Local LLM inference for intelligent renaming (via llama.cpp)
//! - GBNF grammar support for constraining LLM output
//!
//! ## Example (with local LLM)
//!
//! ```ignore
//! use humanify_rs::llm::{local_rename, ModelConfig, Prompt};
//!
//! // Load the model
//! let config = ModelConfig::new("2b");
//! let prompt = Prompt::new(config)?;
//!
//! // Rename variables in minified code
//! let code = "function a(b,c){return b+c}";
//! let result = local_rename(&prompt, code, 1000, None::<fn(f64)>)?;
//! ```

mod error;
mod identifier;
pub mod llm;
mod renamer;
mod visitor;

pub use error::HumanifyError;
pub use renamer::{FnRenamer, NoOpRenamer, RenameContext, Renamer};
pub use visitor::visit_all_identifiers;

/// Re-export Result type with HumanifyError
pub type Result<T> = std::result::Result<T, HumanifyError>;
