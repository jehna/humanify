use unicode_ident::is_xid_continue;

/// Normalize an arbitrary string into a valid JavaScript identifier.
///
/// - Collapses separator runs (`.`, ` `, `-`) by uppercasing the next identifier-continue char.
/// - `_` is identifier-continue, NOT a separator.
/// - Strips chars that are neither identifier-continue nor separators.
/// - Prepends `_` if the result is empty, starts with a digit, or is a reserved word.
pub fn to_identifier(raw: &str) -> String {
    if raw.is_empty() {
        return "_".to_string();
    }

    let mut result = String::with_capacity(raw.len());
    let mut capitalize_next = false;

    for ch in raw.chars() {
        if ch == '.' || ch == ' ' || ch == '-' {
            // Separator: set flag only if we have content to continue from.
            if !result.is_empty() {
                capitalize_next = true;
            }
        } else if ch == '$' || ch == '_' || is_xid_continue(ch) {
            // Valid identifier-continue (including `_` and `$`).
            if capitalize_next {
                capitalize_next = false;
                for c in ch.to_uppercase() {
                    result.push(c);
                }
            } else {
                result.push(ch);
            }
        }
        // All other chars dropped silently.
    }

    if result.is_empty() {
        return "_".to_string();
    }

    // Prepend `_` if the first char is a digit.
    if result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, '_');
    }

    // Prepend `_` if the result is a reserved word.
    if is_reserved_word(&result) {
        result.insert(0, '_');
    }

    result
}

/// Returns true iff `name` is a JS reserved word that cannot be used as a binding name.
/// Includes strict-mode reserved words and contextual keywords (`let`, `static`, etc.).
/// Does NOT include `async` (contextual identifier we accept as a valid binding name).
pub fn is_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "let"
            | "static"
            | "implements"
            | "interface"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "await"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- to_identifier ---

    #[test]
    fn preserves_simple_name() {
        assert_eq!(to_identifier("foo"), "foo");
    }

    #[test]
    fn strips_dot_and_camel_cases() {
        assert_eq!(to_identifier("this.kLength"), "thisKLength");
    }

    #[test]
    fn strips_space_and_camel_cases() {
        assert_eq!(to_identifier("foo bar"), "fooBar");
    }

    #[test]
    fn strips_dash_and_camel_cases() {
        assert_eq!(to_identifier("foo-bar-baz"), "fooBarBaz");
    }

    #[test]
    fn collapses_consecutive_separators() {
        assert_eq!(to_identifier("a..b"), "aB");
    }

    #[test]
    fn drops_unknown_punctuation() {
        assert_eq!(to_identifier("foo!?bar"), "foobar");
    }

    #[test]
    fn empty_input_returns_underscore() {
        assert_eq!(to_identifier(""), "_");
    }

    #[test]
    fn all_stripped_returns_underscore() {
        assert_eq!(to_identifier("!!??"), "_");
    }

    #[test]
    fn leading_digit_prepends_underscore() {
        assert_eq!(to_identifier("123abc"), "_123abc");
    }

    #[test]
    fn reserved_word_static() {
        assert_eq!(to_identifier("static"), "_static");
    }

    #[test]
    fn reserved_word_class() {
        assert_eq!(to_identifier("class"), "_class");
    }

    #[test]
    fn reserved_word_let() {
        assert_eq!(to_identifier("let"), "_let");
    }

    #[test]
    fn underscore_passes_through() {
        assert_eq!(to_identifier("_foo"), "_foo");
    }

    #[test]
    fn preexisting_underscore_static() {
        // `_static` is not in the reserved set → passes through unchanged.
        assert_eq!(to_identifier("_static"), "_static");
    }

    #[test]
    fn unicode_xid_continue_preserved() {
        assert_eq!(to_identifier("café"), "café");
    }

    #[test]
    fn dollar_sign_preserved() {
        assert_eq!(to_identifier("$dollar"), "$dollar");
    }

    #[test]
    fn already_camel_preserved() {
        assert_eq!(to_identifier("alreadyCamel"), "alreadyCamel");
    }

    // --- is_reserved_word ---

    #[test]
    fn true_for_static() {
        assert!(is_reserved_word("static"));
    }

    #[test]
    fn true_for_class() {
        assert!(is_reserved_word("class"));
    }

    #[test]
    fn true_for_let() {
        assert!(is_reserved_word("let"));
    }

    #[test]
    fn false_for_foo() {
        assert!(!is_reserved_word("foo"));
    }

    #[test]
    fn false_for_underscore_static() {
        assert!(!is_reserved_word("_static"));
    }

    #[test]
    fn false_for_async() {
        // `async` is a contextual identifier we accept as a valid binding name.
        assert!(!is_reserved_word("async"));
    }
}
