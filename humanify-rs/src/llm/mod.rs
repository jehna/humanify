//! Local LLM inference module using llama.cpp
//!
//! This module provides:
//! - GBNF grammar builder for constraining LLM output
//! - Model loading and configuration
//! - Inference with grammar-constrained sampling
//! - Variable unminification using two-stage LLM prompting

mod grammar;
mod model;
mod prompt;
mod unminify;

pub use grammar::{Grammar, GrammarBuilder};
pub use model::{
    ensure_model_directory, get_model_definition, get_model_path, is_model_downloaded,
    LlamaModel, ModelConfig, ModelDefinition, ModelError, MODELS,
};
pub use prompt::{Prompt, PromptError};
pub use unminify::{
    define_filename, local_rename, unminify_variable_name, LocalRenameError, LocalRenamer,
    DEFAULT_CONTEXT_WINDOW_SIZE,
};
