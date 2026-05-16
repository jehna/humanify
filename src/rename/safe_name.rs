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
    use super::super::test_dsl::to_identifier_of;

    // --- to_identifier ---

    #[test]
    fn preserves_simple_name() {
        to_identifier_of("foo").is("foo");
    }

    #[test]
    fn strips_dot_and_camel_cases() {
        to_identifier_of("this.kLength").is("thisKLength");
    }

    #[test]
    fn strips_space_and_camel_cases() {
        to_identifier_of("foo bar").is("fooBar");
    }

    #[test]
    fn strips_dash_and_camel_cases() {
        to_identifier_of("foo-bar-baz").is("fooBarBaz");
    }

    #[test]
    fn collapses_consecutive_separators() {
        to_identifier_of("a..b").is("aB");
    }

    #[test]
    fn drops_unknown_punctuation() {
        to_identifier_of("foo!?bar").is("foobar");
    }

    #[test]
    fn empty_input_returns_underscore() {
        to_identifier_of("").is("_");
    }

    #[test]
    fn all_stripped_returns_underscore() {
        to_identifier_of("!!??").is("_");
    }

    #[test]
    fn leading_digit_prepends_underscore() {
        to_identifier_of("123abc").is("_123abc");
    }

    #[test]
    fn reserved_word_static() {
        to_identifier_of("static").is("_static");
    }

    #[test]
    fn reserved_word_class() {
        to_identifier_of("class").is("_class");
    }

    #[test]
    fn reserved_word_let() {
        to_identifier_of("let").is("_let");
    }

    #[test]
    fn underscore_passes_through() {
        to_identifier_of("_foo").is("_foo");
    }

    #[test]
    fn preexisting_underscore_static() {
        // `_static` is not in the reserved set → passes through unchanged.
        to_identifier_of("_static").is("_static");
    }

    #[test]
    fn unicode_xid_continue_preserved() {
        to_identifier_of("café").is("café");
    }

    #[test]
    fn dollar_sign_preserved() {
        to_identifier_of("$dollar").is("$dollar");
    }

    #[test]
    fn already_camel_preserved() {
        to_identifier_of("alreadyCamel").is("alreadyCamel");
    }

    // --- is_reserved_word ---

    #[test]
    fn true_for_static() {
        to_identifier_of("static").is_reserved();
    }

    #[test]
    fn true_for_class() {
        to_identifier_of("class").is_reserved();
    }

    #[test]
    fn true_for_let() {
        to_identifier_of("let").is_reserved();
    }

    #[test]
    fn false_for_foo() {
        to_identifier_of("foo").is_not_reserved();
    }

    #[test]
    fn false_for_underscore_static() {
        to_identifier_of("_static").is_not_reserved();
    }

    #[test]
    fn false_for_async() {
        // `async` is a contextual identifier we accept as a valid binding name.
        to_identifier_of("async").is_not_reserved();
    }
}
