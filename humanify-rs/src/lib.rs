//! # humanify-rs
//!
//! A JavaScript deobfuscator that renames minified variables and functions
//! using a callback mechanism to determine new names based on context.

mod error;
mod identifier;
mod renamer;
mod visitor;

pub use error::HumanifyError;
pub use renamer::{FnRenamer, NoOpRenamer, RenameContext, Renamer};
pub use visitor::visit_all_identifiers;

/// Re-export Result type with HumanifyError
pub type Result<T> = std::result::Result<T, HumanifyError>;
