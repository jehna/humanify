use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::{AstNodes, NodeId, Scoping, SemanticBuilder, SymbolId};
use oxc_span::{GetSpan, SourceType};
use oxc_str::Ident;

use super::collision::CollisionResolver;
use super::{NoopRenameObserver, RenameError, RenameObserver, Renamer};

pub fn rename_all_identifiers(
    source: &str,
    renamer: &mut dyn Renamer,
    context_size: usize,
) -> Result<String, RenameError> {
    rename_all_identifiers_with_observer(source, renamer, context_size, &mut NoopRenameObserver)
}

pub fn rename_all_identifiers_with_observer(
    source: &str,
    renamer: &mut dyn Renamer,
    context_size: usize,
    observer: &mut dyn RenameObserver,
) -> Result<String, RenameError> {
    if source.is_empty() {
        observer.identifiers_found(0);
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

    // Rename the smallest scopes first so names discovered for inner helpers and
    // locals can appear in the context used to name their enclosing scopes.
    // Within a scope, retain declaration order.
    entries.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));
    let total = entries.len();
    observer.identifiers_found(total);

    let mut visited: HashSet<SymbolId> = HashSet::new();
    let mut collisions = CollisionResolver::new(semantic.scoping(), semantic.nodes());

    for (index, (sym_id, _, _)) in entries.iter().enumerate() {
        let current = index + 1;
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
        observer.rename_started(current, total, &original_name);

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
            // Render small scopes through the live scoping so earlier renames
            // appear in later prompts. Keep raw source slicing as the bounded
            // fallback for large scopes and unsupported node kinds.
            let ctx_len = ctx_span.end.saturating_sub(ctx_span.start) as usize;
            if ctx_len > 0 && ctx_len <= context_size {
                let scope_node_id = scoping.get_node_id(binding_scope);
                let kind = nodes.get_node(scope_node_id).kind();
                let cloned_scoping = scoping.clone_in_with_semantic_ids_with_another_arena();
                super::render::codegen_scope_node(kind, cloned_scoping).unwrap_or_else(|| {
                    compute_context_window(source, sym_span, ctx_span, context_size)
                })
            } else {
                compute_context_window(source, sym_span, ctx_span, context_size)
            }
        };

        let new_name = renamer.rename(&original_name, &surrounding);

        if new_name == original_name {
            // No rename; short-circuit — skip safe-name pipeline.
            observer.rename_finished(current, total, &original_name, &original_name);
            continue;
        }

        // Apply safe-name normalization to get the base candidate.
        let base = super::safe_name::to_identifier(&new_name);

        // Resolve collisions in this scope chain with a numeric suffix instead
        // of the old underscore-prefix loop (which piled up `________index`
        // when a local model returned the same generic word many times). If the
        // model's name already ends in a number, that number is incremented
        // rather than a second one appended — so `...Iterator2` -> `...Iterator3`,
        // never `...Iterator22`.
        let scope_id = semantic.scoping().symbol_scope_id(sym_id);
        let (safe, _collided) = super::collision::dedupe(&base, |cand| {
            collisions.collides(semantic.scoping(), scope_id, sym_id, cand)
        });

        // Rename in the symbol table (codegen reads from here via with_scoping),
        // and keep the collision index in sync for later symbols.
        collisions.commit(scope_id, &original_name, &safe);
        let new_ident = Ident::from(allocator.alloc_str(&safe));
        semantic
            .scoping_mut()
            .rename_symbol(sym_id, scope_id, new_ident);
        observer.rename_finished(current, total, &original_name, &safe);
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
    let ctx_start = floor_char_boundary(source, ctx_span.start as usize);
    let ctx_end = ceil_char_boundary(source, ctx_span.end as usize);
    let ctx_len = ctx_end.saturating_sub(ctx_start);

    if ctx_len <= context_size {
        return source[ctx_start..ctx_end].to_string();
    }

    // Is this a Program-level (full source) context?
    let is_program = ctx_start == 0 && ctx_end == source.len();

    if is_program {
        let sym_start = (sym_span.start as usize).min(source.len());
        let sym_end = (sym_span.end as usize).min(source.len());
        let half = context_size / 2;
        if sym_end < half {
            let end = ceil_char_boundary(source, context_size);
            return source[..end].to_string();
        }
        if sym_start > source.len().saturating_sub(half) {
            let start = floor_char_boundary(source, source.len().saturating_sub(context_size));
            return source[start..].to_string();
        }
        let start = floor_char_boundary(source, sym_start.saturating_sub(half));
        let end = ceil_char_boundary(source, sym_end + half);
        source[start..end].to_string()
    } else {
        // Inner scope: return first context_size bytes of the scope slice.
        let end = ceil_char_boundary(source, ctx_start + context_size).min(ctx_end);
        source[ctx_start..end].to_string()
    }
}

/// Round `index` down to the nearest UTF-8 char boundary. Clamps to `source.len()`.
fn floor_char_boundary(source: &str, index: usize) -> usize {
    let mut idx = index.min(source.len());
    while !source.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Round `index` up to the nearest UTF-8 char boundary. Clamps to `source.len()`.
fn ceil_char_boundary(source: &str, index: usize) -> usize {
    let len = source.len();
    let mut idx = index.min(len);
    while idx < len && !source.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::test_dsl::{fixed, identity, mapping, queue, recording, scenario, suffix};
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
    fn renames_two_scopes_innermost_first() {
        let out = scenario("const a = 1; (function () { const b = 2; });")
            .with_context_size(500)
            .renamed_with(queue(&["c", "d"]));
        assert!(
            out.output().contains("const d = 1"),
            "expected outer a -> d: {}",
            out.output()
        );
        assert!(
            out.output().contains("const c = 2"),
            "expected inner b -> c: {}",
            out.output()
        );
    }

    #[test]
    fn renames_shadowed_variables() {
        // Two independent bindings named 'a' — each gets its own rename call (symbol_id keyed).
        // The inner binding is processed first.
        let out = scenario("const a = 1; (function () { const a = 2; });")
            .with_context_size(500)
            .renamed_with(queue(&["b", "c"]));
        assert!(
            out.output().contains("const c = 1"),
            "expected outer c: {}",
            out.output()
        );
        assert!(
            out.output().contains("const b = 2"),
            "expected inner b: {}",
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
        assert_eq!(log.call_names(), &["y", "b", "Bar", "a", "foo"]);
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
    fn scopes_renamed_smallest_to_largest() {
        let input = "function foo() { function bar() { function baz() { const qux = 1; } } }";
        let (_, log) = scenario(input)
            .with_context_size(500)
            .with_recording(recording("_x"));
        let names: Vec<&str> = log.0.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            &["qux", "baz", "foo", "bar"],
            "expected smallest-first: {names:?}"
        );
    }

    #[test]
    fn later_sibling_context_contains_earlier_rename() {
        let (_, log) = scenario("function f() { const a = 1; const b = a + 1; }")
            .with_context_size(500)
            .with_recording(recording("_named"));

        let b_scope = log.scope_for("b");
        assert!(
            b_scope.contains("a_named"),
            "later sibling context should contain the earlier rename: {b_scope}"
        );
    }

    #[test]
    fn outer_context_contains_inner_rename() {
        let (_, log) = scenario("const padding = 0; function a() { const b = 1; return b; }")
            .with_context_size(500)
            .with_recording(recording("_named"));

        let outer_scope = log.scope_for("a");
        assert!(
            outer_scope.contains("b_named"),
            "outer context should contain the inner rename: {outer_scope}"
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
            scope.trim_end(),
            input,
            "surrounding_code for top-level binding should render the full source"
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
    fn does_not_capture_unresolved_browser_global() {
        scenario("const a = 1; crypto.getRandomValues(new Uint8Array(a));")
            .with_context_size(500)
            .renamed_with(fixed("crypto"))
            .yields("const crypto2 = 1;\ncrypto.getRandomValues(new Uint8Array(crypto2));");
    }

    #[test]
    fn does_not_capture_unresolved_host_global() {
        scenario("const a = 1; hostApi.consume(a);")
            .with_context_size(500)
            .renamed_with(fixed("hostApi"))
            .yields("const hostApi2 = 1;\nhostApi.consume(hostApi2);");
    }

    #[test]
    fn does_not_capture_unresolved_global_in_descendant_scope() {
        let out = scenario("const a = 1; function f() { crypto.getRandomValues(a); }")
            .with_context_size(500)
            .renamed_with(mapping(&[("a", "crypto")]));
        assert!(
            out.output().contains("const crypto2 = 1"),
            "outer binding must not capture a descendant global reference: {}",
            out.output()
        );
    }

    #[test]
    fn sibling_scope_may_reuse_unresolved_global_name() {
        let out =
            scenario("function f() { hostApi.consume(); } function g() { const a = 1; return a; }")
                .with_context_size(500)
                .renamed_with(mapping(&[("a", "hostApi")]));
        assert!(
            out.output().contains("const hostApi = 1"),
            "sibling global reference should not force a suffix: {}",
            out.output()
        );
        assert!(
            !out.output().contains("hostApi2"),
            "sibling scopes cannot capture each other: {}",
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
            out.output().contains("const foo2 = 1"),
            "expected numeric-suffixed foo2: {}",
            out.output()
        );
    }

    #[test]
    fn handles_multiple_props_same_name() {
        let out = scenario("const foo = 1; const bar = 2;")
            .with_context_size(500)
            .renamed_with(fixed("bar"));
        assert!(
            out.output().contains("bar2"),
            "expected bar2 for renamed foo: {}",
            out.output()
        );
        assert!(
            out.output().contains("const bar = 2"),
            "expected bar to stay bar: {}",
            out.output()
        );
    }

    // --- scope-aware collision handling ---

    /// Counts occurrences of `name` as a whole identifier token (bounded by
    /// non-identifier characters), so `count_ident(s, "shared")` does not match
    /// inside `_shared`, and `count_ident(s, "_shared")` does not match inside
    /// `__shared`.
    fn count_ident(hay: &str, name: &str) -> usize {
        let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
        let bytes = hay.as_bytes();
        let mut n = 0;
        let mut start = 0;
        while let Some(pos) = hay[start..].find(name) {
            let i = start + pos;
            let before_ok = i == 0 || !is_word(hay[..i].chars().next_back().unwrap());
            let after = i + name.len();
            let after_ok = after >= bytes.len() || !is_word(hay[after..].chars().next().unwrap());
            if before_ok && after_ok {
                n += 1;
            }
            start = i + name.len();
        }
        n
    }

    #[test]
    fn sibling_scopes_may_reuse_the_same_name_without_suffix() {
        // Two locals the model names identically, in disjoint sibling functions.
        // JS allows this, so neither should accrue a suffix.
        let out = scenario("function f() { var a = 1; } function g() { var b = 2; }")
            .with_context_size(500)
            .renamed_with(mapping(&[("a", "shared"), ("b", "shared")]));
        let o = out.output();
        assert_eq!(
            count_ident(o, "shared"),
            2,
            "both sibling locals should be bare `shared`: {o}"
        );
        assert!(
            !o.contains("shared2"),
            "sibling reuse must not add a numeric suffix: {o}"
        );
    }

    #[test]
    fn same_scope_conflict_gets_numeric_suffix() {
        // Two bindings in the SAME scope cannot share a name; the second becomes
        // `shared2` rather than `_shared`.
        let out = scenario("const a = 1; const b = 2;")
            .with_context_size(500)
            .renamed_with(mapping(&[("a", "shared"), ("b", "shared")]));
        let o = out.output();
        assert_eq!(count_ident(o, "shared"), 1, "one bare `shared`: {o}");
        assert_eq!(count_ident(o, "shared2"), 1, "one `shared2`: {o}");
        assert!(!o.contains("shared3"), "only two colliding names: {o}");
        assert!(!o.contains("_shared"), "must not underscore-prefix: {o}");
    }

    #[test]
    fn shadowing_conflict_across_nesting_gets_numeric_suffix() {
        // An outer binding and a nested binding cannot both be `shared`, or a
        // reference would be captured. Exactly one becomes `shared2`, regardless
        // of traversal order.
        let out = scenario("var a = 1; function f() { var b = 2; return a; }")
            .with_context_size(500)
            .renamed_with(mapping(&[("a", "shared"), ("b", "shared")]));
        let o = out.output();
        assert!(o.contains("shared2"), "one conflict becomes `shared2`: {o}");
        assert!(!o.contains("shared3"), "only two colliding names: {o}");
        assert!(!o.contains("_shared"), "must not underscore-prefix: {o}");
    }

    #[test]
    fn no_suffix_pileup_across_many_disjoint_scopes() {
        // Regression for the `________index` pileup: many disjoint scopes each
        // naming their local `index` must all stay bare `index`.
        let mut src = String::new();
        for i in 0..10 {
            src.push_str(&format!("function f{i}() {{ var v{i} = {i}; }} "));
        }
        let pairs: Vec<(String, String)> = (0..10)
            .map(|i| (format!("v{i}"), "index".to_string()))
            .collect();
        let pair_refs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let out = scenario(&src)
            .with_context_size(500)
            .renamed_with(mapping(&pair_refs));
        let o = out.output();
        assert_eq!(
            count_ident(o, "index"),
            10,
            "all ten locals should be bare: {o}"
        );
        assert!(!o.contains("index2"), "no numeric-suffix pileup: {o}");
    }

    #[test]
    fn descendant_capture_is_prevented_with_numeric_suffix() {
        // Renaming the OUTER `a` to a name already bound in a nested scope would
        // capture the `return a` reference (it would resolve to the inner `x`).
        // The descendant check must force a suffix even though `find_binding`
        // (which only walks *up*) can't see the nested binding. Outer becomes
        // `x2`, and the inner `x` and the outer reference stay correct.
        let out = scenario("var a = 1; function f() { var x = 2; return a + x; }")
            .with_context_size(500)
            .renamed_with(mapping(&[("a", "x")]));
        let o = out.output();
        assert_eq!(
            count_ident(o, "x2"),
            2,
            "outer decl + its reference should both be `x2`: {o}"
        );
        assert_eq!(
            count_ident(o, "x"),
            2,
            "inner decl + its reference should stay bare `x`: {o}"
        );
        assert!(!o.contains("x3"), "only one conflict: {o}");
    }

    #[test]
    fn same_scope_numeric_names_increment_not_append() {
        // Three same-scope bindings the model all names `item2`: the suffix must
        // increment (`item2` -> `item3` -> `item4`), never append a second digit
        // (`item22`) or fall back to an underscore.
        let out = scenario("const a = 1; const b = 2; const c = 3;")
            .with_context_size(500)
            .renamed_with(mapping(&[("a", "item2"), ("b", "item2"), ("c", "item2")]));
        let o = out.output();
        assert_eq!(count_ident(o, "item2"), 1, "one `item2`: {o}");
        assert_eq!(count_ident(o, "item3"), 1, "one `item3`: {o}");
        assert_eq!(count_ident(o, "item4"), 1, "one `item4`: {o}");
        assert!(!o.contains("item22"), "must increment, not append: {o}");
        assert!(!o.contains("_item"), "must not underscore-prefix: {o}");
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
    fn truncation_window_falls_on_multibyte_char_boundary() {
        // Cyrillic 'о' is 2 bytes (0xD0 0xBE). With a long stretch of these
        // around the symbol, the truncation window edges land mid-character
        // unless we snap them to UTF-8 char boundaries.
        // Regression test for https://github.com/jehna/humanify/issues/747.
        let pad = "о".repeat(500);
        let input = format!("/* {pad} */ const x = 1; /* {pad} */");
        let out = rename_all_identifiers(&input, &mut identity(), 50).unwrap();
        assert!(out.contains("const x = 1"), "expected x unchanged: {out}");
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
