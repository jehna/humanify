//! Local LLM inference module using llama.cpp
//!
//! This module provides:
//! - GBNF grammar builder for constraining LLM output
//! - Model loading and configuration
//! - Inference with grammar-constrained sampling

mod grammar;
mod model;
mod prompt;

pub use grammar::{Grammar, GrammarBuilder};
pub use model::{LlamaModel, ModelConfig, ModelError};
pub use prompt::{Prompt, PromptError};
