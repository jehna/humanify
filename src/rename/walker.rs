use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::{AstNodes, NodeId, Scoping, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, SourceType};
use oxc_str::Ident;

use super::{RenameError, Renamer};

pub fn rename_all_identifiers(
    source: &str,
    renamer: &mut dyn Renamer,
    context_size: usize,
) -> Result<String, RenameError> {
    if source.is_empty() {
        return Ok(String::new());
    }

    let allocator = Allocator::default();
    let parse_result = Parser::new(&allocator, source, SourceType::default()).parse();
    if !parse_result.errors.is_empty() {
        let msg = parse_result
            .errors
            .iter()
            .map(|e| e.message.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(RenameError::Parse(msg));
    }
    let program = parse_result.program;

    let semantic_result = SemanticBuilder::new().build(&program);
    if !semantic_result.errors.is_empty() {
        let msg = semantic_result
            .errors
            .iter()
            .map(|e| e.message.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(RenameError::Parse(msg));
    }
    let mut semantic = semantic_result.semantic;

    // Collect all symbols with their binding-scope span sizes for sorting.
    let mut entries: Vec<(SymbolId, u32, u32)> = {
        let scoping = semantic.scoping();
        let nodes = semantic.nodes();
        scoping
            .symbol_ids()
            .map(|sym_id| {
                let decl_node_id = scoping.symbol_declaration(sym_id);
                let sym_name = scoping.symbol_name(sym_id);
                let span = scoping.symbol_span(sym_id);
                let binding_scope = scoping.symbol_scope_id(sym_id);
                // Walk ancestors to find the scope-introducing ancestor node for context.
                let ctx_span = find_binding_ancestor_span(
                    nodes,
                    scoping,
                    decl_node_id,
                    sym_name,
                    source,
                    binding_scope,
                );
                let size = ctx_span.end.saturating_sub(ctx_span.start);
                (sym_id, size, span.start)
            })
            .collect()
    };

    // Sort: largest scope first; ties broken by source position (ascending).
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));

    let mut visited: HashSet<SymbolId> = HashSet::new();
    let mut taken: HashSet<String> = HashSet::new();

    for (sym_id, _, _) in &entries {
        let sym_id = *sym_id;
        let original_name = {
            let scoping = semantic.scoping();
            scoping.symbol_name(sym_id).to_string()
        };

        // Each binding (symbol_id) is processed independently — shadowed
        // variables with the same name in different scopes each get their
        // own rename call.
        if visited.contains(&sym_id) {
            continue;
        }
        visited.insert(sym_id);

        // Compute surrounding code context.
        let surrounding = {
            let scoping = semantic.scoping();
            let nodes = semantic.nodes();
            let decl_node_id = scoping.symbol_declaration(sym_id);
            let sym_span = scoping.symbol_span(sym_id);
            let binding_scope = scoping.symbol_scope_id(sym_id);
            let ctx_span = find_binding_ancestor_span(
                nodes,
                scoping,
                decl_node_id,
                &original_name,
                source,
                binding_scope,
            );
            compute_context_window(source, sym_span, ctx_span, context_size)
        };

        let new_name = renamer.rename(&original_name, &surrounding);

        if new_name == original_name {
            // No rename; short-circuit — skip safe-name pipeline.
            continue;
        }

        // Apply safe-name normalization.
        let mut safe = super::safe_name::to_identifier(&new_name);

        // Collision loop: prefix with '_' until name is free.
        let scope_id = semantic.scoping().symbol_scope_id(sym_id);
        loop {
            let already_taken = taken.contains(&safe);
            let pre_existing = {
                let scoping = semantic.scoping();
                let ident = Ident::from(safe.as_str());
                scoping.find_binding(scope_id, ident).is_some()
            };
            if !already_taken && !pre_existing {
                break;
            }
            safe = format!("_{safe}");
        }

        taken.insert(safe.clone());

        // Rename in the symbol table (codegen reads from here via with_scoping).
        let new_ident = Ident::from(allocator.alloc_str(&safe));
        semantic
            .scoping_mut()
            .rename_symbol(sym_id, scope_id, new_ident);
    }

    let scoping: Scoping = semantic.into_scoping();
    let output = Codegen::new()
        .with_scoping(Some(scoping))
        .build(&program)
        .code;

    Ok(output)
}

/// Find the span of the closest binding-introducing ancestor for a given declaration node.
/// Mirrors v1's `closestSurroundingContextPath`: walk parents until we find the node whose
/// outer bindings contain the symbol name (v1: `p.getOuterBindingIdentifiers()`), then
/// return the span of *that node's scope's block* (v1: `p.scope.path`).
///
/// In oxc: `scoping.get_node_id(ancestor.scope_id())` gives the node that created the scope
/// the ancestor lives in — that is the scope block node. We return its span.
fn find_binding_ancestor_span(
    nodes: &AstNodes<'_>,
    scoping: &Scoping,
    _decl_node_id: NodeId,
    _sym_name: &str,
    source: &str,
    binding_scope_id: oxc_semantic::ScopeId,
) -> oxc_span::Span {
    use oxc_span::Span;

    // The node that created the binding scope IS the scope block (v1's `scope.path`).
    // Use scoping.get_node_id(binding_scope_id) to find it directly.
    let scope_block_node_id = scoping.get_node_id(binding_scope_id);
    let scope_block = nodes.get_node(scope_block_node_id);
    let span = scope_block.kind().span();

    // Program node (or empty span) starts at 0; Program.span may not cover trailing
    // whitespace exactly, so fall back to the full source range.
    if span.start == 0 {
        return Span::new(0, source.len() as u32);
    }

    span
}

/// Compute the context window slice from source, given the binding-ancestor span and the
/// symbol's own span. Mirrors v1's `scopeToString` truncation rules.
fn compute_context_window(
    source: &str,
    sym_span: oxc_span::Span,
    ctx_span: oxc_span::Span,
    context_size: usize,
) -> String {
    let ctx_start = ctx_span.start as usize;
    let ctx_end = (ctx_span.end as usize).min(source.len());
    let ctx_len = ctx_end.saturating_sub(ctx_start);

    if ctx_len <= context_size {
        return source[ctx_start..ctx_end].to_string();
    }

    // Is this a Program-level (full source) context?
    let is_program = ctx_start == 0 && ctx_end == source.len();

    if is_program {
        let sym_start = sym_span.start as usize;
        let sym_end = (sym_span.end as usize).min(source.len());
        let half = context_size / 2;
        if sym_end < half {
            return source[..context_size.min(source.len())].to_string();
        }
        if sym_start > source.len().saturating_sub(half) {
            let start = source.len().saturating_sub(context_size);
            return source[start..].to_string();
        }
        let start = sym_start.saturating_sub(half);
        let end = (sym_end + half).min(source.len());
        source[start..end].to_string()
    } else {
        // Inner scope: return first context_size bytes of the scope slice.
        let end = (ctx_start + context_size).min(ctx_end);
        source[ctx_start..end].to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::test_dsl::{fixed, identity, queue, recording, scenario, suffix};
    use super::*;

    #[test]
    fn no_op_returns_same_code() {
        scenario("const a = 1;")
            .renamed_with(identity())
            .yields("const a = 1;");
    }

    #[test]
    fn no_op_returns_same_empty_code() {
        let out = rename_all_identifiers("", &mut super::super::test_dsl::identity(), 500).unwrap();
        assert_eq!(out, "");
    }

    #[test]
    fn renames_simple_variable() {
        scenario("const a = 1;")
            .renamed_with(fixed("b"))
            .yields("const b = 1;");
    }

    #[test]
    fn renames_across_scopes() {
        let out = scenario("const a = 1; (function () { a = 2; });")
            .with_context_size(500)
            .renamed_with(fixed("b"));
        assert!(
            out.output().contains("const b = 1"),
            "expected 'const b': {}",
            out.output()
        );
        assert!(
            out.output().contains("b = 2"),
            "expected 'b = 2': {}",
            out.output()
        );
        assert!(
            !out.output().contains('a'),
            "should have no 'a' left: {}",
            out.output()
        );
    }

    #[test]
    fn renames_two_scopes_largest_first() {
        let out = scenario("const a = 1; (function () { const b = 2; });")
            .with_context_size(500)
            .renamed_with(queue(&["c", "d"]));
        assert!(
            out.output().contains("const c = 1"),
            "expected c: {}",
            out.output()
        );
        assert!(
            out.output().contains("const d = 2"),
            "expected d: {}",
            out.output()
        );
    }

    #[test]
    fn renames_shadowed_variables() {
        // Two independent bindings named 'a' — each gets its own rename call (symbol_id keyed).
        let out = scenario("const a = 1; (function () { const a = 2; });")
            .with_context_size(500)
            .renamed_with(queue(&["b", "c"]));
        assert!(
            out.output().contains("const b = 1"),
            "expected outer b: {}",
            out.output()
        );
        assert!(
            out.output().contains("const c = 2"),
            "expected inner c: {}",
            out.output()
        );
    }

    #[test]
    fn does_not_rename_class_methods() {
        let out = scenario("class Foo { bar() {} }")
            .with_context_size(500)
            .renamed_with(suffix("_changed"));
        assert!(
            out.output().contains("Foo_changed"),
            "expected Foo_changed: {}",
            out.output()
        );
        assert!(
            out.output().contains("bar"),
            "expected bar unchanged: {}",
            out.output()
        );
        assert!(
            !out.output().contains("bar_changed"),
            "bar should not be renamed: {}",
            out.output()
        );
    }

    const SCOPE_INPUT: &str = "const a = 1;\nfunction foo() {\n  const b = 2;\n\n  class Bar {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n};\n";

    #[test]
    fn passes_surrounding_scope_argument() {
        let (_, log) = scenario(SCOPE_INPUT)
            .with_context_size(500)
            .with_recording(recording("_changed"));
        assert_eq!(
            log.0.len(),
            5,
            "expected 5 calls, got {}: {:?}",
            log.0.len(),
            log.0
        );
    }

    #[test]
    fn passes_identifiers_in_scope_order() {
        let (_, log) = scenario(SCOPE_INPUT)
            .with_context_size(500)
            .with_recording(recording("_x"));
        assert_eq!(log.call_names(), &["a", "foo", "b", "Bar", "y"]);
    }

    #[test]
    fn scope_for_inner_binding_contains_enclosing_function() {
        let (_, log) = scenario(SCOPE_INPUT)
            .with_context_size(500)
            .with_recording(recording("_x"));
        assert!(log.scope_for("b").contains("const b = 2"));
        assert!(log.scope_for("b").contains("foo"));
    }

    #[test]
    fn scopes_renamed_largest_to_smallest() {
        let input = "function foo() { function bar() { function baz() { const qux = 1; } } }";
        let (_, log) = scenario(input)
            .with_context_size(500)
            .with_recording(recording("_x"));
        let names: Vec<&str> = log.0.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            &["foo", "bar", "baz", "qux"],
            "expected largest-first: {names:?}"
        );
    }

    #[test]
    fn each_variable_renamed_only_once() {
        let input = "function splitString(a, e, t, n, r, i) { return a + e + t + n + r + i; }";
        let (_, log) = scenario(input)
            .with_context_size(500)
            .with_recording(recording("_x"));
        let names: Vec<&str> = log.0.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"splitString"),
            "expected splitString: {names:?}"
        );
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "duplicate names in calls: {names:?}"
        );
    }

    #[test]
    fn scope_is_from_declaration_site() {
        let input = "function foo() { if (true) { if (true) { let a = 1; a.toString(); } } }";
        let (_, log) = scenario(input)
            .with_context_size(200)
            .with_recording(recording("_x"));
        let a_scope = log
            .0
            .iter()
            .find(|(n, _)| n == "a")
            .map(|(_, s)| s.as_str())
            .unwrap_or("");
        assert!(
            a_scope.contains("let a = 1"),
            "scope for 'a' should contain declaration: {a_scope}"
        );
        assert!(
            a_scope.contains("a.toString()"),
            "scope for 'a' should contain usage: {a_scope}"
        );
    }

    #[test]
    fn surrounding_code_for_program_level_binding_is_full_source() {
        let input = "const x = 1;";
        let (_, log) = scenario(input)
            .with_context_size(500)
            .with_recording(recording("_y"));
        let scope = log
            .0
            .iter()
            .find(|(n, _)| n == "x")
            .map(|(_, s)| s.as_str())
            .expect("expected call for 'x'");
        assert_eq!(
            scope, input,
            "surrounding_code for top-level binding should be the full source"
        );
    }

    #[test]
    fn surrounding_code_for_inner_binding_excludes_outer_code() {
        let input = "const outer = 99; function fn1() { const inner = 1; }";
        let (_, log) = scenario(input)
            .with_context_size(500)
            .with_recording(recording("_y"));
        let inner_scope = log
            .0
            .iter()
            .find(|(n, _)| n == "inner")
            .map(|(_, s)| s.as_str())
            .expect("expected call for 'inner'");
        assert!(
            inner_scope.contains("const inner = 1"),
            "inner scope should contain its own declaration: {inner_scope}"
        );
        assert!(
            !inner_scope.contains("const outer"),
            "inner scope should NOT contain outer variable: {inner_scope}"
        );
    }

    #[test]
    fn surrounding_code_is_truncated_by_context_size() {
        let input = "function big() { const x = 1; const y = 2; const z = 3; const w = 4; }";
        let (_, log) = scenario(input)
            .with_context_size(20)
            .with_recording(recording("_y"));
        let x_scope = log
            .0
            .iter()
            .find(|(n, _)| n == "x")
            .map(|(_, s)| s.as_str())
            .expect("expected call for 'x'");
        assert!(
            x_scope.len() < input.len(),
            "surrounding_code should be truncated, got {} bytes: {x_scope:?}",
            x_scope.len()
        );
        assert!(
            !x_scope.contains("const w = 4"),
            "truncated surrounding_code should not contain late declarations: {x_scope:?}"
        );
    }

    #[test]
    fn does_not_rename_object_properties() {
        let out = scenario("const c = 2; const a = { b: c }; a.b;")
            .with_context_size(500)
            .renamed_with(queue(&["d", "e"]));
        assert!(
            out.output().contains("b:") || out.output().contains("b :"),
            "property b should survive: {}",
            out.output()
        );
        assert!(
            !out.output().contains("const c"),
            "c should be renamed: {}",
            out.output()
        );
        assert!(
            !out.output().contains("const a"),
            "a should be renamed: {}",
            out.output()
        );
    }

    #[test]
    fn handles_invalid_identifiers() {
        scenario("const a = 1;")
            .with_context_size(500)
            .renamed_with(fixed("this.kLength"))
            .yields("const thisKLength = 1;");
    }

    #[test]
    fn handles_space_in_identifier() {
        scenario("const a = 1;")
            .with_context_size(500)
            .renamed_with(fixed("foo bar"))
            .yields("const fooBar = 1;");
    }

    #[test]
    fn handles_reserved_identifiers() {
        let out = scenario("const a = 1;")
            .with_context_size(500)
            .renamed_with(fixed("static"));
        assert!(
            out.output().contains("_static"),
            "expected _static: {}",
            out.output()
        );
    }

    #[test]
    fn handles_multiple_same_name() {
        let out = scenario("const a = 1; const b = 1;")
            .with_context_size(500)
            .renamed_with(fixed("foo"));
        assert!(
            out.output().contains("const foo = 1"),
            "expected foo: {}",
            out.output()
        );
        assert!(
            out.output().contains("const _foo = 1"),
            "expected _foo: {}",
            out.output()
        );
    }

    #[test]
    fn handles_multiple_props_same_name() {
        let out = scenario("const foo = 1; const bar = 2;")
            .with_context_size(500)
            .renamed_with(fixed("bar"));
        assert!(
            out.output().contains("_bar"),
            "expected _bar for renamed foo: {}",
            out.output()
        );
        assert!(
            out.output().contains("const bar = 2"),
            "expected bar to stay bar: {}",
            out.output()
        );
    }

    #[test]
    fn does_not_crash_on_arguments_assign() {
        let out = scenario("function foo() { arguments = '??'; }")
            .with_context_size(500)
            .renamed_with(fixed("foobar"));
        assert!(
            out.output().contains("foobar"),
            "expected foobar: {}",
            out.output()
        );
    }

    #[test]
    fn unicode_identifier() {
        let out = scenario("const café = 1;")
            .with_context_size(500)
            .renamed_with(fixed("x"));
        assert!(
            out.output().contains("const x = 1"),
            "expected x: {}",
            out.output()
        );
    }

    #[test]
    fn private_class_field() {
        let input = "class A { #x = 1; m() { return this.#x; } }";
        let out = scenario(input)
            .with_context_size(500)
            .renamed_with(identity());
        assert!(
            out.output().contains('A'),
            "expected class A in output: {}",
            out.output()
        );
        assert!(
            out.output().contains("#x"),
            "expected #x in output: {}",
            out.output()
        );
    }
}
