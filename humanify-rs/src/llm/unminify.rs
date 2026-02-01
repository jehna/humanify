//! Unminification functions for renaming variables using local LLM
//!
//! This module provides two-stage prompting to generate meaningful names
//! for minified variables, following the TypeScript implementation.

use super::grammar::GrammarBuilder;
use super::prompt::{Prompt, PromptError};
use crate::renamer::{RenameContext, Renamer};

/// Default context window size for code snippets
pub const DEFAULT_CONTEXT_WINDOW_SIZE: usize = 1000;

/// Padding characters used for filename inference
const PADDING_CHARS: usize = 200;

/// Unminify a variable name using two-stage LLM prompting
///
/// This function uses a two-stage approach:
/// 1. First, ask the LLM to describe the purpose of the variable
/// 2. Then, ask the LLM to suggest a good name based on the description
///
/// # Arguments
/// * `prompt` - The LLM prompt interface
/// * `variable_name` - The current (minified) variable name
/// * `filename` - The filename where the variable is defined
/// * `code` - The surrounding code context
///
/// # Returns
/// A new, more descriptive name for the variable
pub fn unminify_variable_name(
    prompt: &Prompt,
    variable_name: &str,
    filename: &str,
    code: &str,
) -> Result<String, PromptError> {
    // Stage 1: Get a description of the variable
    let description_grammar = GrammarBuilder::new()
        .literal(&format!("A good description for '{}' is: ", variable_name))
        .variable("[^\\r\\n\\x0b\\x0c\\x85.]+")
        .literal(".")
        .build();

    let description = prompt.prompt(
        &format!(
            "Your task is to read the code in file \"{}\" and write the purpose of variable, argument or function '{}' in one sentence. Use simple language so it's understandable by a junior programmer.",
            filename, variable_name
        ),
        code,
        &description_grammar,
    )?;

    // Stage 2: Get a name based on the description
    let name_grammar = GrammarBuilder::new()
        .literal("A good name would be '")
        .variable("[a-zA-Z][a-zA-Z0-9]{2,12}")
        .literal("'")
        .build();

    let result = prompt.prompt(
        "You are a Code Assistant.",
        &format!(
            "What would be a good name for the following function or a variable in Typescript? Don't mind the minified variable names.\n{}",
            description
        ),
        &name_grammar,
    )?;

    Ok(result)
}

/// Infer a filename from a code snippet using two-stage LLM prompting
///
/// This function uses a two-stage approach:
/// 1. First, ask the LLM to describe the purpose of the code
/// 2. Then, ask the LLM to suggest a good filename based on the description
///
/// # Arguments
/// * `prompt` - The LLM prompt interface
/// * `code` - The code snippet to analyze
///
/// # Returns
/// A suggested filename (with .js extension)
pub fn define_filename(prompt: &Prompt, code: &str) -> Result<String, PromptError> {
    // Stage 1: Get a description of the code
    let description_grammar = GrammarBuilder::new()
        .variable("[^\\r\\n\\x0b\\x0c\\x85.]+")
        .literal(".")
        .build();

    let description = prompt.prompt(
        "Simplify the code snippet's purpose into a concise explanation in one sentence. Don't use variable names or function names in your description. Use the present tense.",
        code,
        &description_grammar,
    )?;

    // Stage 2: Get a filename based on the description
    let filename_grammar = GrammarBuilder::new()
        .literal("Sure, a good name for your Javascript file would be '")
        .variable("[a-z][a-zA-Z0-9-]{2,12}")
        .literal(".js'")
        .build();

    let filename = prompt.prompt(
        "Create a name for a Javascript file for a code with the following description. Use lisp-case naming convention.",
        &description,
        &filename_grammar,
    )?;

    Ok(format!("{}.js", filename))
}

/// A renamer that uses local LLM inference to determine new variable names
///
/// This implements the `Renamer` trait and uses the same two-stage prompting
/// approach as the TypeScript implementation.
pub struct LocalRenamer<'a> {
    prompt: &'a Prompt,
    filename: String,
    context_window_size: usize,
}

impl<'a> LocalRenamer<'a> {
    /// Create a new LocalRenamer with the given prompt and filename
    ///
    /// # Arguments
    /// * `prompt` - The LLM prompt interface
    /// * `filename` - The filename to use for context (or use `with_inferred_filename`)
    pub fn new(prompt: &'a Prompt, filename: impl Into<String>) -> Self {
        Self {
            prompt,
            filename: filename.into(),
            context_window_size: DEFAULT_CONTEXT_WINDOW_SIZE,
        }
    }

    /// Create a new LocalRenamer with an inferred filename
    ///
    /// The filename is inferred from the beginning of the code using LLM.
    ///
    /// # Arguments
    /// * `prompt` - The LLM prompt interface
    /// * `code` - The code to infer the filename from
    pub fn with_inferred_filename(prompt: &'a Prompt, code: &str) -> Result<Self, PromptError> {
        // Use a snippet from the start of the code for filename inference
        let snippet_end = (PADDING_CHARS * 2).min(code.len());
        let snippet = &code[..snippet_end];

        let filename = define_filename(prompt, snippet)?;

        Ok(Self {
            prompt,
            filename,
            context_window_size: DEFAULT_CONTEXT_WINDOW_SIZE,
        })
    }

    /// Set the context window size for surrounding code
    pub fn with_context_window_size(mut self, size: usize) -> Self {
        self.context_window_size = size;
        self
    }

    /// Get the filename being used
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Get the context window size
    pub fn context_window_size(&self) -> usize {
        self.context_window_size
    }
}

impl Renamer for LocalRenamer<'_> {
    fn rename(&mut self, ctx: RenameContext<'_>) -> String {
        match unminify_variable_name(self.prompt, ctx.name, &self.filename, ctx.surrounding_code) {
            Ok(new_name) => new_name,
            Err(_) => {
                // On error, return the original name unchanged
                ctx.name.to_string()
            }
        }
    }
}

/// Process JavaScript code, renaming all identifiers using local LLM inference
///
/// This is the main entry point that combines filename inference and identifier renaming.
///
/// # Arguments
/// * `prompt` - The LLM prompt interface
/// * `code` - The JavaScript code to process
/// * `context_window_size` - Maximum size of surrounding code context
/// * `on_progress` - Optional callback for progress updates (0.0 to 1.0)
///
/// # Returns
/// The processed JavaScript code with renamed identifiers
pub fn local_rename<P>(
    prompt: &Prompt,
    code: &str,
    context_window_size: usize,
    on_progress: Option<P>,
) -> Result<String, LocalRenameError>
where
    P: FnMut(f64),
{
    // Infer filename from the code
    let renamer = LocalRenamer::with_inferred_filename(prompt, code)
        .map_err(LocalRenameError::Prompt)?
        .with_context_window_size(context_window_size);

    // Visit all identifiers and rename them
    crate::visit_all_identifiers(code, renamer, context_window_size, on_progress)
        .map_err(LocalRenameError::Humanify)
}

/// Errors that can occur during local rename
#[derive(Debug, thiserror::Error)]
pub enum LocalRenameError {
    /// Error from the LLM prompt
    #[error("Prompt error: {0}")]
    Prompt(#[from] PromptError),

    /// Error from the humanify processing
    #[error("Processing error: {0}")]
    Humanify(#[from] crate::HumanifyError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_description_grammar_extraction() {
        let grammar = GrammarBuilder::new()
            .literal("A good description for 'x' is: ")
            .variable("[^\\r\\n\\x0b\\x0c\\x85.]+")
            .literal(".")
            .build();

        assert_eq!(
            grammar.extract("A good description for 'x' is: This variable stores the user count."),
            Some("This variable stores the user count")
        );
    }

    #[test]
    fn test_name_grammar_extraction() {
        let grammar = GrammarBuilder::new()
            .literal("A good name would be '")
            .variable("[a-zA-Z][a-zA-Z0-9]{2,12}")
            .literal("'")
            .build();

        assert_eq!(
            grammar.extract("A good name would be 'userCount'"),
            Some("userCount")
        );
    }

    #[test]
    fn test_filename_grammar_extraction() {
        let grammar = GrammarBuilder::new()
            .literal("Sure, a good name for your Javascript file would be '")
            .variable("[a-z][a-zA-Z0-9-]{2,12}")
            .literal(".js'")
            .build();

        assert_eq!(
            grammar.extract("Sure, a good name for your Javascript file would be 'user-utils.js'"),
            Some("user-utils")
        );
    }

    #[test]
    fn test_local_renamer_configuration() {
        // Note: This test only checks configuration, not actual LLM calls
        // since we don't have a model loaded in tests

        // Test context window size
        assert_eq!(DEFAULT_CONTEXT_WINDOW_SIZE, 1000);

        // Test padding chars
        assert_eq!(PADDING_CHARS, 200);
    }
}
