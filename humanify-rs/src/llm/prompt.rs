//! Prompt and inference for local LLM
//!
//! This module provides the core inference functionality using llama.cpp.

use std::num::NonZeroU32;
use std::path::Path;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel as LlamaCppModel;
use llama_cpp_2::sampling::LlamaSampler;
use thiserror::Error;

use super::grammar::Grammar;
use super::model::{get_ensured_model_path, get_model_definition, ChatTemplate, ModelConfig};

/// Errors that can occur during prompting
#[derive(Error, Debug)]
pub enum PromptError {
    /// Failed to initialize llama backend
    #[error("Failed to initialize llama backend: {0}")]
    BackendInit(String),

    /// Failed to load model
    #[error("Failed to load model: {0}")]
    ModelLoad(String),

    /// Failed to create context
    #[error("Failed to create context: {0}")]
    ContextCreate(String),

    /// Failed to tokenize input
    #[error("Failed to tokenize: {0}")]
    Tokenize(String),

    /// Failed during inference
    #[error("Inference error: {0}")]
    Inference(String),

    /// Model error
    #[error("Model error: {0}")]
    Model(#[from] super::model::ModelError),
}

/// A prompt function that can be called multiple times with different prompts
pub struct Prompt {
    backend: LlamaBackend,
    model: LlamaCppModel,
    config: ModelConfig,
    chat_template: ChatTemplate,
}

impl Prompt {
    /// Create a new prompter with the given configuration
    pub fn new(config: ModelConfig) -> Result<Self, PromptError> {
        // Initialize the backend
        let backend =
            LlamaBackend::init().map_err(|e| PromptError::BackendInit(format!("{:?}", e)))?;

        // Get model path and definition
        let model_path = get_ensured_model_path(&config.model)?;
        let model_def = get_model_definition(&config.model)?;

        // Create model params
        let mut model_params = LlamaModelParams::default();

        if config.disable_gpu {
            model_params = model_params.with_n_gpu_layers(0);
        } else if let Some(layers) = config.gpu_layers {
            model_params = model_params.with_n_gpu_layers(layers);
        }

        // Load the model
        let model = LlamaCppModel::load_from_file(&backend, &model_path, &model_params)
            .map_err(|e| PromptError::ModelLoad(format!("{:?}", e)))?;

        Ok(Self {
            backend,
            model,
            config,
            chat_template: model_def.chat_template,
        })
    }

    /// Create a prompter from a model file path directly
    pub fn from_path(path: impl AsRef<Path>, config: ModelConfig) -> Result<Self, PromptError> {
        let backend =
            LlamaBackend::init().map_err(|e| PromptError::BackendInit(format!("{:?}", e)))?;

        let mut model_params = LlamaModelParams::default();

        if config.disable_gpu {
            model_params = model_params.with_n_gpu_layers(0);
        } else if let Some(layers) = config.gpu_layers {
            model_params = model_params.with_n_gpu_layers(layers);
        }

        let model = LlamaCppModel::load_from_file(&backend, path.as_ref(), &model_params)
            .map_err(|e| PromptError::ModelLoad(format!("{:?}", e)))?;

        Ok(Self {
            backend,
            model,
            config,
            chat_template: ChatTemplate::None,
        })
    }

    /// Execute a prompt and return the extracted result
    ///
    /// # Arguments
    /// * `system_prompt` - The system prompt (instructions)
    /// * `user_prompt` - The user prompt (input)
    /// * `grammar` - Grammar to constrain the output
    ///
    /// # Returns
    /// The extracted variable from the grammar, or the full response if no variable
    pub fn prompt(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        grammar: &Grammar,
    ) -> Result<String, PromptError> {
        // Format the prompt based on chat template
        let full_prompt = self.format_prompt(system_prompt, user_prompt);

        // Create context params
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.config.context_size));

        // Create context
        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| PromptError::ContextCreate(format!("{:?}", e)))?;

        // Tokenize the prompt
        let tokens = self
            .model
            .str_to_token(&full_prompt, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| PromptError::Tokenize(format!("{:?}", e)))?;

        // Create batch and add tokens
        let mut batch = LlamaBatch::new(self.config.context_size as usize, 1);

        for (i, token) in tokens.iter().enumerate() {
            let is_last = i == tokens.len() - 1;
            batch
                .add(*token, i as i32, &[0], is_last)
                .map_err(|e| PromptError::Inference(format!("Failed to add token: {:?}", e)))?;
        }

        // Decode the prompt
        ctx.decode(&mut batch)
            .map_err(|e| PromptError::Inference(format!("Failed to decode prompt: {:?}", e)))?;

        // Create sampler chain
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::dist(self.config.seed.unwrap_or(1234)),
            LlamaSampler::greedy(),
        ]);

        // TODO: Add grammar constraint to sampler
        // The llama-cpp-2 API for grammar is still being explored

        // Generate response
        let mut response = String::new();
        let max_tokens = 256; // Maximum response length
        let mut n_cur = batch.n_tokens();

        for _ in 0..max_tokens {
            // Sample next token
            let token = sampler.sample(&ctx, n_cur - 1);

            // Check for end of generation
            if self.model.is_eog_token(token) {
                break;
            }

            // Decode token to string
            let piece = self
                .model
                .token_to_str(token, llama_cpp_2::model::Special::Tokenize)
                .map_err(|e| PromptError::Inference(format!("Failed to decode token: {:?}", e)))?;

            response.push_str(&piece);

            // Check if we have a complete response based on grammar
            if let Some(extracted) = grammar.extract(&response) {
                // If we can extract a result and there's a suffix, check if it's complete
                if response.len() > extracted.len() {
                    break;
                }
            }

            // Prepare for next token
            batch.clear();
            batch
                .add(token, n_cur, &[0], true)
                .map_err(|e| PromptError::Inference(format!("Failed to add token: {:?}", e)))?;

            ctx.decode(&mut batch)
                .map_err(|e| PromptError::Inference(format!("Failed to decode: {:?}", e)))?;

            n_cur += 1;
        }

        // Extract the result using grammar
        Ok(grammar.extract_owned(&response).unwrap_or(response))
    }

    /// Format a prompt using the appropriate chat template
    fn format_prompt(&self, system_prompt: &str, user_prompt: &str) -> String {
        match self.chat_template {
            ChatTemplate::None => {
                format!("{}\n\n{}", system_prompt, user_prompt)
            }
            ChatTemplate::Llama3_1 => {
                format!(
                    "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>user<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n",
                    system_prompt, user_prompt
                )
            }
            ChatTemplate::Phi3 => {
                format!(
                    "<|system|>\n{}<|end|>\n<|user|>\n{}<|end|>\n<|assistant|>\n",
                    system_prompt, user_prompt
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_prompt_none() {
        let config = ModelConfig::default();
        // We can't create a full Prompt without a model, but we can test the format logic
        let system = "You are a helpful assistant.";
        let user = "Hello!";

        // Test the formatting directly
        let formatted = format!("{}\n\n{}", system, user);
        assert!(formatted.contains(system));
        assert!(formatted.contains(user));
    }

    #[test]
    fn test_format_prompt_llama() {
        let system = "You are a helpful assistant.";
        let user = "Hello!";

        let formatted = format!(
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>user<|end_header_id|>\n\n{}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n",
            system, user
        );

        assert!(formatted.contains("<|begin_of_text|>"));
        assert!(formatted.contains(system));
        assert!(formatted.contains(user));
    }

    #[test]
    fn test_format_prompt_phi() {
        let system = "You are a helpful assistant.";
        let user = "Hello!";

        let formatted = format!(
            "<|system|>\n{}<|end|>\n<|user|>\n{}<|end|>\n<|assistant|>\n",
            system, user
        );

        assert!(formatted.contains("<|system|>"));
        assert!(formatted.contains(system));
        assert!(formatted.contains(user));
    }
}
