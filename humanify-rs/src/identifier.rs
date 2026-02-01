//! Identifier tracking and utility functions

use oxc_span::Span;

/// Information about a binding identifier (variable/function declaration)
#[derive(Debug, Clone)]
pub struct BindingInfo {
    /// The identifier name
    pub name: String,
    /// Span of the identifier in the source
    pub span: Span,
    /// The scope size (used for sorting - larger scopes processed first)
    pub scope_size: u32,
    /// Span of the surrounding context (function, class, or program)
    pub context_span: Span,
}

/// Convert a potentially invalid identifier to a valid JavaScript identifier
///
/// Based on Babel's `toIdentifier` function:
/// - Removes leading digits
/// - Converts invalid characters to camelCase word boundaries
/// - Handles spaces by capitalizing next letter
pub fn to_identifier(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    let mut capitalize_next = false;
    let mut first_char = true;

    for c in name.chars() {
        if c == ' ' || c == '.' || c == '-' || c == '_' {
            // For space/dot/dash, capitalize the next letter (camelCase)
            if !result.is_empty() && c != '_' {
                capitalize_next = true;
            } else if c == '_' {
                result.push('_');
            }
            first_char = false;
            continue;
        }

        if first_char {
            // First character must be a letter or underscore or $
            if c.is_ascii_digit() {
                result.push('_');
                result.push(c);
            } else if c.is_ascii_alphabetic() || c == '_' || c == '$' {
                if capitalize_next {
                    result.extend(c.to_uppercase());
                    capitalize_next = false;
                } else {
                    result.push(c);
                }
            } else {
                // Invalid first char, skip
                continue;
            }
            first_char = false;
        } else {
            // Subsequent characters can be letters, digits, underscore, or $
            if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                if capitalize_next {
                    result.extend(c.to_uppercase());
                    capitalize_next = false;
                } else {
                    result.push(c);
                }
            }
            // Skip invalid characters
        }
    }

    result
}

/// Check if a name is a JavaScript reserved keyword
pub fn is_reserved_keyword(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "in"
            | "instanceof"
            | "new"
            | "return"
            | "switch"
            | "this"
            | "throw"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "class"
            | "const"
            | "enum"
            | "export"
            | "extends"
            | "import"
            | "super"
            | "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
            | "null"
            | "true"
            | "false"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_identifier_handles_spaces() {
        assert_eq!(to_identifier("foo bar"), "fooBar");
    }

    #[test]
    fn to_identifier_handles_dots() {
        assert_eq!(to_identifier("this.kLength"), "thisKLength");
    }

    #[test]
    fn to_identifier_handles_leading_digits() {
        assert_eq!(to_identifier("123abc"), "_123abc");
    }

    #[test]
    fn to_identifier_preserves_valid() {
        assert_eq!(to_identifier("validName"), "validName");
    }

    #[test]
    fn to_identifier_handles_underscores() {
        assert_eq!(to_identifier("_private"), "_private");
    }

    #[test]
    fn reserved_keywords_detected() {
        assert!(is_reserved_keyword("static"));
        assert!(is_reserved_keyword("const"));
        assert!(!is_reserved_keyword("myVariable"));
    }
}
