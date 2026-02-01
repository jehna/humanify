//! AST visitor for finding and renaming identifiers

use std::collections::{HashMap, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_codegen::{CodeGenerator, CodegenOptions};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

use crate::error::HumanifyError;
use crate::identifier::{is_reserved_keyword, to_identifier, BindingInfo};
use crate::renamer::{RenameContext, Renamer};
use crate::Result;

/// Visit all identifiers in JavaScript code and rename them using a callback
///
/// # Arguments
/// * `code` - The JavaScript source code to process
/// * `renamer` - A renamer that determines new names for identifiers
/// * `context_window_size` - Maximum size of surrounding code context to provide
/// * `on_progress` - Optional callback for progress updates (0.0 to 1.0)
///
/// # Returns
/// The modified JavaScript code with identifiers renamed
pub fn visit_all_identifiers<R, P>(
    code: &str,
    mut renamer: R,
    context_window_size: usize,
    mut on_progress: Option<P>,
) -> Result<String>
where
    R: Renamer,
    P: FnMut(f64),
{
    if code.is_empty() {
        return Ok(String::new());
    }

    let allocator = Allocator::default();
    let source_type = SourceType::default().with_module(true);
    let parser = Parser::new(&allocator, code, source_type);
    let parse_result = parser.parse();

    if !parse_result.errors.is_empty() {
        // Try parsing as script instead
        let parser = Parser::new(&allocator, code, SourceType::default());
        let parse_result = parser.parse();
        if !parse_result.errors.is_empty() {
            return Err(HumanifyError::ParseError(
                parse_result
                    .errors
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
    }

    // Collect all binding identifiers with their scope information
    let mut collector = BindingCollector::new(code.len());
    collector.visit_program(&parse_result.program);

    // Sort bindings by scope size (largest first), preserving DFS order for equal sizes
    let mut indexed_bindings: Vec<(usize, BindingInfo)> = collector
        .bindings
        .into_iter()
        .enumerate()
        .collect();

    // Use stable sort-like behavior by including original index in comparison
    indexed_bindings.sort_by(|(idx_a, a), (idx_b, b)| {
        match b.scope_size.cmp(&a.scope_size) {
            std::cmp::Ordering::Equal => idx_a.cmp(idx_b), // Preserve original DFS order
            other => other,
        }
    });

    let bindings: Vec<BindingInfo> = indexed_bindings.into_iter().map(|(_, b)| b).collect();

    let num_bindings = bindings.len();
    if num_bindings == 0 {
        if let Some(ref mut progress) = on_progress {
            progress(1.0);
        }
        // Return original code regenerated (for consistent formatting)
        let codegen = CodeGenerator::new()
            .with_options(CodegenOptions {
                single_quote: true,
                ..Default::default()
            })
            .build(&parse_result.program);
        return Ok(codegen.code);
    }

    // Track visited binding spans (not names, to handle shadowing)
    let mut visited_spans: HashSet<Span> = HashSet::new();
    // Track all names that exist in the global scope or have been assigned
    let mut used_names: HashSet<String> = HashSet::new();

    // Collect all existing binding names
    for binding in &bindings {
        used_names.insert(binding.name.clone());
    }

    // Map from binding span to all reference spans (including the binding itself)
    let mut binding_references: HashMap<Span, Vec<Span>> = HashMap::new();

    // Build a scope-based reference mapping, excluding shadowed scopes
    for binding in &bindings {
        // Find all bindings with the same name that might shadow this one
        let shadowing_scopes: Vec<Span> = bindings
            .iter()
            .filter(|b| {
                b.name == binding.name
                    && b.span != binding.span
                    && b.context_span.start >= binding.context_span.start
                    && b.context_span.end <= binding.context_span.end
                    && !(b.context_span.start == binding.context_span.start
                        && b.context_span.end == binding.context_span.end)
            })
            .map(|b| b.context_span)
            .collect();

        let refs = find_references_in_scope(
            code,
            &binding.name,
            binding.span,
            binding.context_span,
            &shadowing_scopes,
        );
        binding_references.insert(binding.span, refs);
    }

    // Collect all renames to apply
    let mut rename_operations: Vec<(Vec<Span>, String, String)> = Vec::new();

    // Process each binding
    for (idx, binding) in bindings.iter().enumerate() {
        // Skip if we've already visited this specific binding
        if visited_spans.contains(&binding.span) {
            continue;
        }

        // Get surrounding code context
        let surrounding_code = get_surrounding_code(code, binding, context_window_size);

        // Call renamer to get new name
        let new_name = renamer.rename(RenameContext {
            name: &binding.name,
            surrounding_code: &surrounding_code,
        });

        if new_name != binding.name {
            // Sanitize the new name
            let mut safe_name = to_identifier(&new_name);

            // Handle reserved keywords
            if is_reserved_keyword(&safe_name) {
                safe_name = format!("_{}", safe_name);
            }

            // Ensure uniqueness - check against all existing and assigned names
            while used_names.contains(&safe_name) && safe_name != binding.name {
                safe_name = format!("_{}", safe_name);
            }

            // Get all references to this binding
            let refs = binding_references
                .get(&binding.span)
                .cloned()
                .unwrap_or_else(|| vec![binding.span]);

            rename_operations.push((refs, binding.name.clone(), safe_name.clone()));

            // Update used_names: remove the old name (if no other binding uses it) and add the new
            used_names.insert(safe_name);
        }

        visited_spans.insert(binding.span);

        if let Some(ref mut progress) = on_progress {
            progress((idx + 1) as f64 / num_bindings as f64);
        }
    }

    if let Some(ref mut progress) = on_progress {
        progress(1.0);
    }

    // Apply all renames to the source code
    let result = apply_renames(code, &rename_operations);

    Ok(result)
}

/// Collector for binding identifiers (declarations)
struct BindingCollector<'a> {
    bindings: Vec<BindingInfo>,
    scope_stack: Vec<Span>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> BindingCollector<'a> {
    fn new(source_len: usize) -> Self {
        Self {
            bindings: Vec::new(),
            scope_stack: vec![Span::new(0, source_len as u32)],
            _marker: std::marker::PhantomData,
        }
    }

    fn current_scope(&self) -> Span {
        *self.scope_stack.last().unwrap()
    }

    fn add_binding(&mut self, name: &str, span: Span) {
        let context_span = self.current_scope();
        let scope_size = context_span.end - context_span.start;
        self.bindings.push(BindingInfo {
            name: name.to_string(),
            span,
            scope_size,
            context_span,
        });
    }
}

impl<'a> Visit<'a> for BindingCollector<'a> {
    fn visit_function(&mut self, func: &Function<'a>, _flags: ScopeFlags) {
        // Visit function name BEFORE pushing the function scope
        // Function name belongs to the outer (containing) scope
        if let Some(id) = &func.id {
            let outer_scope = self.current_scope();
            let scope_size = outer_scope.end - outer_scope.start;
            self.bindings.push(BindingInfo {
                name: id.name.to_string(),
                span: id.span,
                scope_size,
                context_span: outer_scope,
            });
        }

        // Push function scope for parameters and body
        self.scope_stack.push(func.span);

        // Visit parameters
        for param in &func.params.items {
            self.visit_formal_parameter(param);
        }

        // Visit body
        if let Some(body) = &func.body {
            self.visit_function_body(body);
        }

        self.scope_stack.pop();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.scope_stack.push(arrow.span);

        for param in &arrow.params.items {
            self.visit_formal_parameter(param);
        }

        self.visit_function_body(&arrow.body);

        self.scope_stack.pop();
    }

    fn visit_class(&mut self, class: &Class<'a>) {
        if let Some(id) = &class.id {
            self.add_binding(&id.name, id.span);
        }
        self.scope_stack.push(class.span);
        walk::walk_class(self, class);
        self.scope_stack.pop();
    }

    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        self.visit_binding_pattern(&decl.id);
        if let Some(init) = &decl.init {
            self.visit_expression(init);
        }
    }

    fn visit_binding_identifier(&mut self, ident: &BindingIdentifier<'a>) {
        self.add_binding(&ident.name, ident.span);
    }

    fn visit_catch_clause(&mut self, clause: &CatchClause<'a>) {
        self.scope_stack.push(clause.span);
        if let Some(param) = &clause.param {
            self.visit_catch_parameter(param);
        }
        self.visit_block_statement(&clause.body);
        self.scope_stack.pop();
    }
}

/// Find all references to a binding within its scope, excluding shadowed regions
fn find_references_in_scope(
    source: &str,
    name: &str,
    binding_span: Span,
    context_span: Span,
    shadowing_scopes: &[Span],
) -> Vec<Span> {
    let mut refs = vec![binding_span];

    // Simple text-based reference finding within the context
    let context_start = context_span.start as usize;
    let context_end = context_span.end as usize;
    let context = &source[context_start..context_end.min(source.len())];

    // Find all occurrences of the identifier
    let mut pos = 0;
    while let Some(idx) = context[pos..].find(name) {
        let abs_pos = context_start + pos + idx;
        let span = Span::new(abs_pos as u32, (abs_pos + name.len()) as u32);

        // Check if it's a valid identifier occurrence (not part of a larger word)
        let before_valid = if abs_pos == 0 {
            true
        } else {
            let c = source.chars().nth(abs_pos - 1).unwrap_or(' ');
            !c.is_ascii_alphanumeric() && c != '_' && c != '$'
        };

        let after_valid = {
            let end_pos = abs_pos + name.len();
            if end_pos >= source.len() {
                true
            } else {
                let c = source.chars().nth(end_pos).unwrap_or(' ');
                !c.is_ascii_alphanumeric() && c != '_' && c != '$'
            }
        };

        if before_valid && after_valid && span != binding_span {
            // Check it's not a property access (preceded by a dot)
            let is_property = abs_pos > 0 && source.chars().nth(abs_pos - 1) == Some('.');

            // Check it's not within a shadowing scope
            let in_shadow = shadowing_scopes.iter().any(|shadow| {
                abs_pos >= shadow.start as usize && abs_pos < shadow.end as usize
            });

            if !is_property && !in_shadow {
                refs.push(span);
            }
        }

        pos += idx + name.len();
    }

    refs
}

/// Get the surrounding code context for an identifier
fn get_surrounding_code(source: &str, binding: &BindingInfo, context_window_size: usize) -> String {
    let context_start = binding.context_span.start as usize;
    let context_end = (binding.context_span.end as usize).min(source.len());

    let context = &source[context_start..context_end];

    if context.len() <= context_window_size {
        return context.to_string();
    }

    // If context is too large, center around the identifier
    let binding_start = binding.span.start as usize;

    let relative_start = binding_start.saturating_sub(context_start);
    let half_window = context_window_size / 2;

    if relative_start < half_window {
        // Near the start of context
        context[..context_window_size.min(context.len())].to_string()
    } else if context.len() - relative_start < half_window {
        // Near the end of context
        let start = context.len().saturating_sub(context_window_size);
        context[start..].to_string()
    } else {
        // Center around the identifier
        let start = relative_start.saturating_sub(half_window);
        let end = (relative_start + half_window).min(context.len());
        context[start..end].to_string()
    }
}

/// Apply all rename operations to the source code
fn apply_renames(source: &str, operations: &[(Vec<Span>, String, String)]) -> String {
    // Collect all individual replacements
    let mut replacements: Vec<(Span, &str, &str)> = Vec::new();

    for (spans, old_name, new_name) in operations {
        for span in spans {
            replacements.push((*span, old_name, new_name));
        }
    }

    // Sort by position (reverse order for safe replacement)
    replacements.sort_by(|a, b| b.0.start.cmp(&a.0.start));

    let mut result = source.to_string();

    for (span, old_name, new_name) in replacements {
        let start = span.start as usize;
        let end = span.end as usize;

        // Verify the text at this span matches what we expect
        if end <= result.len() {
            let current = &result[start..end];
            if current == old_name {
                result.replace_range(start..end, new_name);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renamer::FnRenamer;

    #[test]
    fn test_noop_returns_same_code() {
        let code = "const a = 1;";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|name, _| name.to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, code);
    }

    #[test]
    fn test_noop_returns_same_empty_code() {
        let code = "";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|name, _| name.to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, code);
    }

    #[test]
    fn test_renames_simple_variable() {
        let code = "const a = 1;";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "b".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const b = 1;");
    }

    #[test]
    fn test_renames_variables_across_scopes() {
        let code = "const a = 1;\n(function () {\n  a = 2;\n});";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "b".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const b = 1;\n(function () {\n  b = 2;\n});");
    }

    #[test]
    fn test_handles_invalid_identifiers() {
        let code = "const a = 1";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "this.kLength".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const thisKLength = 1");
    }

    #[test]
    fn test_handles_space_in_identifier() {
        let code = "const a = 1";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "foo bar".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const fooBar = 1");
    }

    #[test]
    fn test_handles_reserved_identifiers() {
        let code = "const a = 1";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "static".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const _static = 1");
    }

    #[test]
    fn test_handles_multiple_identifiers_same_name() {
        let code = "const a = 1;\nconst b = 1;";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "foo".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const foo = 1;\nconst _foo = 1;");
    }

    #[test]
    fn test_renames_two_scopes_largest_first() {
        let code = "const a = 1;\n(function () {\n  const b = 2;\n});";
        let mut counter = 0;
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(move |_, _| {
                let names = ["c", "d"];
                let name = names[counter];
                counter += 1;
                name.to_string()
            }),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const c = 1;\n(function () {\n  const d = 2;\n});");
    }

    #[test]
    fn test_renames_shadowed_variables() {
        let code = "const a = 1;\n(function () {\n  const a = 2;\n});";
        let mut counter = 0;
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(move |_, _| {
                let names = ["c", "d"];
                let name = names[counter];
                counter += 1;
                name.to_string()
            }),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const c = 1;\n(function () {\n  const d = 2;\n});");
    }

    #[test]
    fn test_does_not_rename_class_methods() {
        let code = "class Foo {\n  bar() {}\n}";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|name, _| format!("_{}", name)),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "class _Foo {\n  bar() {}\n}");
    }

    #[test]
    fn test_scopes_renamed_largest_to_smallest() {
        let code = "function foo() {\n  function bar() {\n    function baz() {\n    }\n  }\n  function qux() {\n  }\n}";
        let mut names = Vec::new();
        let _ = visit_all_identifiers(
            code,
            FnRenamer::new(|name, _| {
                names.push(name.to_string());
                name.to_string()
            }),
            200,
            None::<fn(f64)>,
        )
        .unwrap();

        // Verify the important invariant: outer scopes are processed before inner scopes
        // foo (outermost) must come first
        assert_eq!(names[0], "foo");
        // bar (contains baz) must come before baz
        let bar_pos = names.iter().position(|n| n == "bar").unwrap();
        let baz_pos = names.iter().position(|n| n == "baz").unwrap();
        assert!(bar_pos < baz_pos, "bar should be processed before baz");
        // All 4 functions should be processed
        assert_eq!(names.len(), 4);
        assert!(names.contains(&"foo".to_string()));
        assert!(names.contains(&"bar".to_string()));
        assert!(names.contains(&"baz".to_string()));
        assert!(names.contains(&"qux".to_string()));
    }

    #[test]
    fn test_renames_each_variable_only_once() {
        let code = r#"function a(e, t) {
  var n = [];
  var r = e.length;
  var i = 0;
  for (; i < r; i += t) {
    if (i + t < r) {
      n.push(e.substring(i, i + t));
    } else {
      n.push(e.substring(i, r));
    }
  }
  return n;
}"#;
        let mut names = Vec::new();
        let _ = visit_all_identifiers(
            code,
            FnRenamer::new(|name, _| {
                names.push(name.to_string());
                format!("{}_changed", name)
            }),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(names, vec!["a", "e", "t", "n", "r", "i"]);
    }

    #[test]
    fn test_should_not_rename_object_properties() {
        let code = "const c = 2;\nconst a = {\n  b: c\n};\na.b;";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|name, _| {
                if name == "c" {
                    "d".to_string()
                } else if name == "a" {
                    "e".to_string()
                } else {
                    format!("_{}", name)
                }
            }),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const d = 2;\nconst e = {\n  b: d\n};\ne.b;");
    }

    #[test]
    fn test_handles_multiple_properties_with_same_name() {
        let code = "const foo = 1;\nconst bar = 2;";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "bar".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "const _bar = 1;\nconst bar = 2;");
    }

    #[test]
    fn test_should_not_crash_on_arguments_assigning() {
        let code = "function foo() {\n  arguments = '??';\n}";
        let result = visit_all_identifiers(
            code,
            FnRenamer::new(|_, _| "foobar".to_string()),
            200,
            None::<fn(f64)>,
        )
        .unwrap();
        assert_eq!(result, "function foobar() {\n  arguments = '??';\n}");
    }
}
