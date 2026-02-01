//! Model loading and configuration for local LLM inference
//!
//! This module provides model management including:
//! - Pre-configured model definitions (2B, 8B)
//! - Model path management
//! - Model loading with llama.cpp

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during model operations
#[derive(Error, Debug)]
pub enum ModelError {
    /// Model not found in registry
    #[error("Model '{0}' not found. Available models: 2b, 8b")]
    UnknownModel(String),

    /// Model file not found on disk
    #[error("Model file not found at {path}. Run download first.")]
    ModelFileNotFound { path: PathBuf },

    /// Failed to load model
    #[error("Failed to load model: {0}")]
    LoadError(String),

    /// Failed to create model directory
    #[error("Failed to create model directory: {0}")]
    DirectoryError(#[from] std::io::Error),
}

/// Chat template format for different model types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatTemplate {
    /// No chat template (direct prompting)
    None,
    /// Llama 3.1 chat format
    Llama3_1,
    /// Phi-3 chat format
    Phi3,
}

/// Definition of a supported model
#[derive(Debug, Clone)]
pub struct ModelDefinition {
    /// Model name/identifier
    pub name: &'static str,
    /// Download URL
    pub url: &'static str,
    /// Filename for the downloaded model
    pub filename: &'static str,
    /// Chat template to use
    pub chat_template: ChatTemplate,
}

/// Pre-configured models
pub static MODELS: &[ModelDefinition] = &[
    ModelDefinition {
        name: "2b",
        url: "https://huggingface.co/bartowski/Phi-3.1-mini-4k-instruct-GGUF/resolve/main/Phi-3.1-mini-4k-instruct-Q4_K_M.gguf?download=true",
        filename: "Phi-3.1-mini-4k-instruct-Q4_K_M.gguf",
        chat_template: ChatTemplate::Phi3,
    },
    ModelDefinition {
        name: "8b",
        url: "https://huggingface.co/lmstudio-community/Meta-Llama-3.1-8B-Instruct-GGUF/resolve/main/Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf?download=true",
        filename: "Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf",
        chat_template: ChatTemplate::Llama3_1,
    },
];

/// Default model to use
pub const DEFAULT_MODEL: &str = "2b";

/// Get model definition by name
pub fn get_model_definition(name: &str) -> Result<&'static ModelDefinition, ModelError> {
    MODELS
        .iter()
        .find(|m| m.name == name)
        .ok_or_else(|| ModelError::UnknownModel(name.to_string()))
}

/// Get the model storage directory
pub fn get_model_directory() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".humanifyjs")
        .join("models")
}

/// Get the full path to a model file
pub fn get_model_path(name: &str) -> Result<PathBuf, ModelError> {
    let definition = get_model_definition(name)?;
    Ok(get_model_directory().join(definition.filename))
}

/// Check if a model is downloaded
pub fn is_model_downloaded(name: &str) -> Result<bool, ModelError> {
    let path = get_model_path(name)?;
    Ok(path.exists())
}

/// Get the path to a model, ensuring it exists
pub fn get_ensured_model_path(name: &str) -> Result<PathBuf, ModelError> {
    let path = get_model_path(name)?;
    if !path.exists() {
        return Err(ModelError::ModelFileNotFound { path });
    }
    Ok(path)
}

/// Ensure the model directory exists
pub fn ensure_model_directory() -> Result<(), ModelError> {
    let dir = get_model_directory();
    std::fs::create_dir_all(&dir)?;
    Ok(())
}

/// Configuration for loading a model
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// Model name (e.g., "2b", "8b")
    pub model: String,
    /// Number of GPU layers to offload (None = auto)
    pub gpu_layers: Option<u32>,
    /// Disable GPU entirely
    pub disable_gpu: bool,
    /// Random seed for reproducibility (None = random)
    pub seed: Option<u32>,
    /// Context size in tokens
    pub context_size: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            model: DEFAULT_MODEL.to_string(),
            gpu_layers: None,
            disable_gpu: false,
            seed: None,
            context_size: 4096,
        }
    }
}

impl ModelConfig {
    /// Create a new config with the specified model
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            ..Default::default()
        }
    }

    /// Set GPU layers
    pub fn with_gpu_layers(mut self, layers: u32) -> Self {
        self.gpu_layers = Some(layers);
        self
    }

    /// Disable GPU
    pub fn with_gpu_disabled(mut self) -> Self {
        self.disable_gpu = true;
        self
    }

    /// Set random seed
    pub fn with_seed(mut self, seed: u32) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set context size
    pub fn with_context_size(mut self, size: u32) -> Self {
        self.context_size = size;
        self
    }
}

/// Loaded LLM model ready for inference
pub struct LlamaModel {
    config: ModelConfig,
    // The actual model will be added when we implement inference
    // For now, we just store the config
    _model_path: PathBuf,
}

impl LlamaModel {
    /// Load a model with the given configuration
    pub fn load(config: ModelConfig) -> Result<Self, ModelError> {
        let model_path = get_ensured_model_path(&config.model)?;

        // TODO: Actually load the model using llama-cpp-2
        // For now, just verify the path exists

        Ok(Self {
            config,
            _model_path: model_path,
        })
    }

    /// Get the model configuration
    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    /// Get the chat template for this model
    pub fn chat_template(&self) -> Result<ChatTemplate, ModelError> {
        let def = get_model_definition(&self.config.model)?;
        Ok(def.chat_template)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_model_definition() {
        let def = get_model_definition("2b").unwrap();
        assert_eq!(def.name, "2b");
        assert_eq!(def.chat_template, ChatTemplate::Phi3);

        let def = get_model_definition("8b").unwrap();
        assert_eq!(def.name, "8b");
        assert_eq!(def.chat_template, ChatTemplate::Llama3_1);
    }

    #[test]
    fn test_unknown_model() {
        let result = get_model_definition("unknown");
        assert!(matches!(result, Err(ModelError::UnknownModel(_))));
    }

    #[test]
    fn test_model_config_builder() {
        let config = ModelConfig::new("8b")
            .with_gpu_disabled()
            .with_seed(42)
            .with_context_size(2048);

        assert_eq!(config.model, "8b");
        assert!(config.disable_gpu);
        assert_eq!(config.seed, Some(42));
        assert_eq!(config.context_size, 2048);
    }

    #[test]
    fn test_model_directory() {
        let dir = get_model_directory();
        assert!(dir.ends_with("models"));
        assert!(dir.to_string_lossy().contains(".humanifyjs"));
    }
}
