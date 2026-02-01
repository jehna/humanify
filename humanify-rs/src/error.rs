//! Error types for humanify-rs

use thiserror::Error;

/// Errors that can occur during JavaScript processing
#[derive(Error, Debug)]
pub enum HumanifyError {
    /// Failed to parse the JavaScript code
    #[error("Failed to parse JavaScript: {0}")]
    ParseError(String),

    /// Failed to generate code from AST
    #[error("Failed to generate code: {0}")]
    CodegenError(String),

    /// Invalid identifier name
    #[error("Invalid identifier: {0}")]
    InvalidIdentifier(String),
}
