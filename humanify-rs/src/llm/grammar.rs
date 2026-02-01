//! GBNF grammar builder for constraining LLM output
//!
//! GBNF (GGML BNF) is a format for defining formal grammars to constrain
//! model outputs in llama.cpp. This module provides a builder for creating
//! grammars and extracting the variable portion from responses.
//!
//! # Example
//!
//! ```
//! use humanify_rs::llm::GrammarBuilder;
//!
//! // Create a grammar that expects: "A good name would be 'someName'"
//! let grammar = GrammarBuilder::new()
//!     .literal("A good name would be '")
//!     .variable("[a-zA-Z][a-zA-Z0-9]{2,12}")
//!     .literal("'")
//!     .build();
//!
//! // The grammar rule for llama.cpp
//! assert!(grammar.rule().contains("root ::="));
//!
//! // Extract the variable from a response
//! let response = "A good name would be 'userName'";
//! assert_eq!(grammar.extract(response), Some("userName"));
//! ```

/// A GBNF grammar with extraction information
#[derive(Debug, Clone)]
pub struct Grammar {
    /// The GBNF rule string
    rule: String,
    /// Start position for extracting the variable
    extract_start: usize,
    /// End position for extracting the variable (offset from end, 0 = end of string)
    extract_end_offset: usize,
    /// Whether this grammar has a variable to extract
    has_variable: bool,
}

impl Grammar {
    /// Get the GBNF rule string for llama.cpp
    pub fn rule(&self) -> &str {
        &self.rule
    }

    /// Extract the variable portion from a response
    ///
    /// Returns `None` if the grammar has no variable or the response is too short
    pub fn extract<'a>(&self, response: &'a str) -> Option<&'a str> {
        if !self.has_variable {
            return None;
        }

        let end = if self.extract_end_offset == 0 {
            response.len()
        } else {
            response.len().saturating_sub(self.extract_end_offset)
        };

        if self.extract_start >= end || end > response.len() {
            return None;
        }

        Some(&response[self.extract_start..end])
    }

    /// Extract the variable and return an owned String
    pub fn extract_owned(&self, response: &str) -> Option<String> {
        self.extract(response).map(|s| s.to_string())
    }
}

impl std::fmt::Display for Grammar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rule)
    }
}

/// Builder for creating GBNF grammars
#[derive(Debug, Default)]
pub struct GrammarBuilder {
    parts: Vec<GrammarPart>,
}

#[derive(Debug, Clone)]
enum GrammarPart {
    Literal(String),
    Variable(String),
}

impl GrammarBuilder {
    /// Create a new grammar builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a literal string to the grammar
    ///
    /// The string will be escaped and quoted in the GBNF output.
    pub fn literal(mut self, text: &str) -> Self {
        self.parts.push(GrammarPart::Literal(text.to_string()));
        self
    }

    /// Add a variable pattern to the grammar
    ///
    /// The pattern should be a valid GBNF pattern (similar to regex).
    /// Only one variable per grammar is supported.
    ///
    /// # Common patterns
    /// - `[a-zA-Z]+` - one or more letters
    /// - `[a-zA-Z][a-zA-Z0-9]{2,12}` - identifier (letter followed by 2-12 alphanumeric)
    /// - `[^\r\n.]+` - any characters except newlines and periods
    pub fn variable(mut self, pattern: &str) -> Self {
        self.parts.push(GrammarPart::Variable(pattern.to_string()));
        self
    }

    /// Build the grammar
    ///
    /// # Panics
    /// Panics if more than one variable is added to the grammar.
    pub fn build(self) -> Grammar {
        let num_variables = self
            .parts
            .iter()
            .filter(|p| matches!(p, GrammarPart::Variable(_)))
            .count();

        if num_variables > 1 {
            panic!("Only one variable per grammar is supported");
        }

        let mut rule = String::from("root ::=");
        let mut extract_start = 0;
        let mut extract_end_offset = 0;
        let mut past_variable = false;

        for part in &self.parts {
            match part {
                GrammarPart::Literal(text) => {
                    let escaped = escape_gbnf_string(text);
                    rule.push_str(&format!(" \"{}\"", escaped));

                    if past_variable {
                        extract_end_offset += text.len();
                    } else {
                        extract_start += text.len();
                    }
                }
                GrammarPart::Variable(pattern) => {
                    rule.push_str(&format!(" {}", pattern));
                    past_variable = true;
                }
            }
        }

        Grammar {
            rule,
            extract_start,
            extract_end_offset,
            has_variable: num_variables == 1,
        }
    }
}

/// Escape a string for use in GBNF
fn escape_gbnf_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Convenience function to create a simple grammar with a prefix, variable pattern, and suffix
pub fn simple_grammar(prefix: &str, pattern: &str, suffix: &str) -> Grammar {
    GrammarBuilder::new()
        .literal(prefix)
        .variable(pattern)
        .literal(suffix)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_only_grammar() {
        let grammar = GrammarBuilder::new().literal("hello").build();

        assert_eq!(grammar.rule(), "root ::= \"hello\"");
        assert_eq!(grammar.extract("hello"), None);
    }

    #[test]
    fn test_variable_grammar() {
        let grammar = GrammarBuilder::new()
            .literal("Hello ")
            .variable("[a-z]+")
            .literal("!")
            .build();

        assert_eq!(grammar.rule(), "root ::= \"Hello \" [a-z]+ \"!\"");
        assert_eq!(grammar.extract("Hello world!"), Some("world"));
    }

    #[test]
    fn test_identifier_grammar() {
        let grammar = GrammarBuilder::new()
            .literal("A good name would be '")
            .variable("[a-zA-Z][a-zA-Z0-9]{2,12}")
            .literal("'")
            .build();

        assert_eq!(
            grammar.extract("A good name would be 'userName'"),
            Some("userName")
        );
        assert_eq!(
            grammar.extract("A good name would be 'foo'"),
            Some("foo")
        );
    }

    #[test]
    fn test_description_grammar() {
        let grammar = GrammarBuilder::new()
            .literal("A good description for 'x' is: ")
            .variable("[^\\r\\n.]+")
            .literal(".")
            .build();

        assert_eq!(
            grammar.extract("A good description for 'x' is: This variable stores the count."),
            Some("This variable stores the count")
        );
    }

    #[test]
    fn test_escape_quotes() {
        let grammar = GrammarBuilder::new()
            .literal("Say \"hello\"")
            .build();

        assert_eq!(grammar.rule(), "root ::= \"Say \\\"hello\\\"\"");
    }

    #[test]
    fn test_simple_grammar_helper() {
        let grammar = simple_grammar("Name: '", "[a-z]+", "'");

        assert_eq!(grammar.rule(), "root ::= \"Name: '\" [a-z]+ \"'\"");
        assert_eq!(grammar.extract("Name: 'test'"), Some("test"));
    }

    #[test]
    #[should_panic(expected = "Only one variable per grammar is supported")]
    fn test_multiple_variables_panics() {
        GrammarBuilder::new()
            .variable("[a-z]+")
            .variable("[0-9]+")
            .build();
    }

    #[test]
    fn test_extract_with_no_suffix() {
        let grammar = GrammarBuilder::new()
            .literal("Result: ")
            .variable("[a-z]+")
            .build();

        assert_eq!(grammar.extract("Result: hello"), Some("hello"));
    }

    #[test]
    fn test_extract_too_short() {
        let grammar = GrammarBuilder::new()
            .literal("Very long prefix ")
            .variable("[a-z]+")
            .build();

        assert_eq!(grammar.extract("Short"), None);
    }
}
