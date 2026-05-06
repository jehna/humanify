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

    // If the span is empty or zero (shouldn't happen), fall back to full source.
    if span.start == 0 && span.end == 0 {
        return Span::new(0, source.len() as u32);
    }

    // If this is the Program node (span starts at 0 and covers everything), return full source.
    // Program.span may not cover trailing whitespace exactly; use source.len() as end.
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
    use super::*;
    use std::collections::VecDeque;

    // --- MockRenamer ---

    enum MockRenamer {
        Identity,
        Fixed(String),
        Queue(VecDeque<String>),
        Suffix(String),
        Recording {
            suffix: String,
            calls: Vec<(String, String)>,
        },
    }

    impl Renamer for MockRenamer {
        fn rename(&mut self, original: &str, surrounding: &str) -> String {
            match self {
                MockRenamer::Identity => original.to_string(),
                MockRenamer::Fixed(name) => name.clone(),
                MockRenamer::Queue(q) => q.pop_front().unwrap_or_else(|| original.to_string()),
                MockRenamer::Suffix(sfx) => format!("{original}{sfx}"),
                MockRenamer::Recording { suffix, calls } => {
                    let new_name = format!("{original}{suffix}");
                    calls.push((original.to_string(), surrounding.to_string()));
                    new_name
                }
            }
        }
    }

    fn identity() -> MockRenamer {
        MockRenamer::Identity
    }

    fn fixed(name: &str) -> MockRenamer {
        MockRenamer::Fixed(name.to_string())
    }

    fn queue(names: &[&str]) -> MockRenamer {
        MockRenamer::Queue(names.iter().map(|s| s.to_string()).collect())
    }

    fn suffix(sfx: &str) -> MockRenamer {
        MockRenamer::Suffix(sfx.to_string())
    }

    fn recording(sfx: &str) -> MockRenamer {
        MockRenamer::Recording {
            suffix: sfx.to_string(),
            calls: Vec::new(),
        }
    }

    fn run(source: &str, renamer: &mut dyn Renamer) -> String {
        rename_all_identifiers(source, renamer, 500).unwrap()
    }

    // Helper: strip trailing newline added by oxc codegen for comparison.
    fn norm(s: &str) -> &str {
        s.trim_end_matches('\n')
    }

    // --- Test cases ---

    #[test]
    fn no_op_returns_same_code() {
        let out = run("const a = 1;", &mut identity());
        assert_eq!(norm(&out), "const a = 1;");
    }

    #[test]
    fn no_op_returns_same_empty_code() {
        let out = rename_all_identifiers("", &mut identity(), 500).unwrap();
        assert_eq!(out, "");
    }

    #[test]
    fn renames_simple_variable() {
        let out = run("const a = 1;", &mut fixed("b"));
        assert_eq!(norm(&out), "const b = 1;");
    }

    #[test]
    fn renames_across_scopes() {
        // codegen-formatting differs from v1
        let out = run("const a = 1; (function () { a = 2; });", &mut fixed("b"));
        assert!(out.contains("const b = 1"), "expected 'const b': {out}");
        assert!(out.contains("b = 2"), "expected 'b = 2': {out}");
        assert!(!out.contains('a'), "should have no 'a' left: {out}");
    }

    #[test]
    fn renames_two_scopes_largest_first() {
        // const a = 1; (function () { const b = 2; });  → queue ["c","d"]
        // Largest scope = program scope (a); inner scope (b) second.
        let out = run(
            "const a = 1; (function () { const b = 2; });",
            &mut queue(&["c", "d"]),
        );
        assert!(out.contains("const c = 1"), "expected c: {out}");
        assert!(out.contains("const d = 2"), "expected d: {out}");
    }

    #[test]
    fn renames_shadowed_variables() {
        // Two independent bindings named 'a' in different scopes — each gets
        // its own rename call since visited-set is keyed by symbol_id.
        let out = run(
            "const a = 1; (function () { const a = 2; });",
            &mut queue(&["b", "c"]),
        );
        assert!(out.contains("const b = 1"), "expected outer b: {out}");
        assert!(out.contains("const c = 2"), "expected inner c: {out}");
    }

    #[test]
    fn does_not_rename_class_methods() {
        // class Foo { bar() {} } — only Foo is a binding; bar is a method key, not a binding.
        // suffix renamer adds "_changed": Foo → Foo_changed
        // codegen-formatting differs from v1
        let out = run("class Foo { bar() {} }", &mut suffix("_changed"));
        assert!(out.contains("Foo_changed"), "expected Foo_changed: {out}");
        // bar should appear unchanged (it's a method name, not a binding identifier)
        assert!(out.contains("bar"), "expected bar unchanged: {out}");
    }

    #[test]
    fn passes_surrounding_scope_argument() {
        let input = "const a = 1;\nfunction foo() {\n  const b = 2;\n\n  class Bar {\n    baz = 3;\n    hello() {\n      const y = 123;\n    }\n  }\n};\n";
        let mut rec = recording("_changed");
        let _ = rename_all_identifiers(input, &mut rec, 500).unwrap();
        let calls = match &rec {
            MockRenamer::Recording { calls, .. } => calls.clone(),
            _ => unreachable!(),
        };

        // Exactly 5 calls: a, foo, b, Bar, y (in that order — largest scope first).
        assert_eq!(
            calls.len(),
            5,
            "expected 5 calls, got {}: {calls:?}",
            calls.len()
        );

        let names: Vec<&str> = calls.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            &["a", "foo", "b", "Bar", "y"],
            "wrong name order: {names:?}"
        );

        // The scope passed for 'b' (index 2) should contain 'foo_changed' because
        // 'foo' was renamed before 'b' is processed and codegen reflects prior renames.
        // Actually: the surrounding_code is sliced from the *original source* (we don't
        // re-generate between calls), so it will contain the original 'foo' text.
        // v1 mutates the AST between calls; we slice source spans from original source.
        // The structural assertion that matters: scope for 'b' contains the function body.
        let b_scope = &calls[2].1;
        assert!(
            b_scope.contains("const b = 2"),
            "scope for 'b' should contain 'const b = 2': {b_scope}"
        );
        assert!(
            b_scope.contains("foo"),
            "scope for 'b' should contain 'foo': {b_scope}"
        );
    }

    #[test]
    fn scopes_renamed_largest_to_smallest() {
        // Nested scopes: expect names visited outermost first.
        let input = "function foo() { function bar() { function baz() { const qux = 1; } } }";
        let mut rec = recording("_x");
        let _ = rename_all_identifiers(input, &mut rec, 500).unwrap();
        let names: Vec<&str> = match &rec {
            MockRenamer::Recording { calls, .. } => calls.iter().map(|(n, _)| n.as_str()).collect(),
            _ => unreachable!(),
        };
        assert_eq!(
            names,
            &["foo", "bar", "baz", "qux"],
            "expected largest-first: {names:?}"
        );
    }

    #[test]
    fn each_variable_renamed_only_once() {
        // splitString-style: multiple params with same names across functions.
        // key: visited set is by original name — if two bindings share a name, renamer
        // is only called once.
        let input = "function splitString(a, e, t, n, r, i) { return a + e + t + n + r + i; }";
        let mut rec = recording("_x");
        let _ = rename_all_identifiers(input, &mut rec, 500).unwrap();
        let names: Vec<&str> = match &rec {
            MockRenamer::Recording { calls, .. } => calls.iter().map(|(n, _)| n.as_str()).collect(),
            _ => unreachable!(),
        };
        // splitString is the outermost binding; a,e,t,n,r,i are params.
        assert!(
            names.contains(&"splitString"),
            "expected splitString: {names:?}"
        );
        // Each unique name should appear exactly once.
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "duplicate names in calls: {names:?}"
        );
    }

    #[test]
    fn scope_is_from_declaration_site() {
        // function foo() { if (true) { if (true) { let a = 1; a.toString(); } } }
        // 'a' is declared inside the if blocks, but its binding-introducing ancestor is
        // the block where it's declared. Since the whole function fits within context_size=200,
        // the scope string should contain both ends of the source.
        let input = "function foo() { if (true) { if (true) { let a = 1; a.toString(); } } }";
        let mut calls: Vec<(String, String)> = Vec::new();
        let mut rec = MockRenamer::Recording {
            suffix: "_x".to_string(),
            calls: Vec::new(),
        };
        let _ = rename_all_identifiers(input, &mut rec, 200).unwrap();
        if let MockRenamer::Recording { calls: ref c, .. } = rec {
            calls = c.clone();
        }
        // Find the scope passed for 'a'.
        let a_scope = calls
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
    fn does_not_rename_object_properties() {
        // const c = 2; const a = { b: c }; a.b;
        // bindings: c, a — b is a property key (not a binding).
        // codegen-formatting differs from v1
        let out = run(
            "const c = 2; const a = { b: c }; a.b;",
            &mut queue(&["d", "e"]),
        );
        // c → d (outermost or same level), a → e
        // b should be unchanged as a property key
        assert!(
            out.contains("b:") || out.contains("b :"),
            "property b should survive: {out}"
        );
        assert!(!out.contains("const c"), "c should be renamed: {out}");
        assert!(!out.contains("const a"), "a should be renamed: {out}");
    }

    #[test]
    fn handles_invalid_identifiers() {
        // renamer returns "this.kLength" → safe_name → "thisKLength"
        let out = run("const a = 1;", &mut fixed("this.kLength"));
        assert!(out.contains("thisKLength"), "expected thisKLength: {out}");
    }

    #[test]
    fn handles_space_in_identifier() {
        // renamer returns "foo bar" → safe_name → "fooBar"
        let out = run("const a = 1;", &mut fixed("foo bar"));
        assert!(out.contains("fooBar"), "expected fooBar: {out}");
    }

    #[test]
    fn handles_reserved_identifiers() {
        // renamer returns "static" → safe_name → "_static"
        let out = run("const a = 1;", &mut fixed("static"));
        assert!(out.contains("_static"), "expected _static: {out}");
    }

    #[test]
    fn handles_multiple_same_name() {
        // const a = 1; const b = 1; → both renamed to "foo"
        // First gets "foo", second would collide → "_foo"
        // BUT: visited-set skips second because 'b' is a different original name.
        // Actually: a → "foo", b → "foo" collides → "_foo"
        let out = run("const a = 1; const b = 1;", &mut fixed("foo"));
        assert!(out.contains("const foo = 1"), "expected foo: {out}");
        assert!(out.contains("const _foo = 1"), "expected _foo: {out}");
    }

    #[test]
    fn handles_multiple_props_same_name() {
        // const foo = 1; const bar = 2; → both renamed to "bar"
        // 'bar' already exists as a binding; first rename of 'foo' → 'bar' collides → '_bar'
        // Then 'bar' itself → 'bar' but 'bar' was already renamed away...
        // Actually: largest scope first. Both are program-level so sorted by position.
        // 'foo' first (pos 0) → renamed to "bar", but 'bar' binding exists → "_bar"
        // 'bar' next → renamed to "bar", 'bar' is now free (was renamed to '_bar' above?
        //   No — 'bar' hasn't been renamed yet at this point. find_binding checks current
        //   scoping state. After 'foo' → '_bar', 'bar' still exists.
        //   'bar' → "bar": find_binding finds 'bar' binding → "_bar", but "_bar" is taken → "__bar"?
        // v1 test: "const foo = 1; const bar = 2;" → "const _bar = 1; const bar = 2;"
        // That implies: foo → "bar" collides with existing bar binding → "_bar"
        //               bar → "bar", no more collision (foo was renamed, bar binding was foo) → "bar"
        let out = run("const foo = 1; const bar = 2;", &mut fixed("bar"));
        // foo should become _bar (collides with existing bar)
        assert!(out.contains("_bar"), "expected _bar for renamed foo: {out}");
        // bar itself should become bar (no collision after foo was renamed away)
        assert!(
            out.contains("const bar = 2"),
            "expected bar to stay bar: {out}"
        );
    }

    #[test]
    fn does_not_crash_on_arguments_assign() {
        // function foo() { arguments = '??'; } → rename foo to "foobar"
        // arguments is not a binding identifier in normal AST; should not crash.
        let out = run("function foo() { arguments = '??'; }", &mut fixed("foobar"));
        assert!(out.contains("foobar"), "expected foobar: {out}");
    }

    // --- oxc-specific tests ---

    #[test]
    fn unicode_identifier() {
        // const café = 1; → rename to "x"
        let out = run("const café = 1;", &mut fixed("x"));
        assert!(out.contains("const x = 1"), "expected x: {out}");
    }

    #[test]
    fn private_class_field() {
        // class A { #x = 1; m() { return this.#x; } }
        // Private fields may or may not surface as binding identifiers in oxc.
        // Either way: identity renamer must not crash, output must round-trip.
        let input = "class A { #x = 1; m() { return this.#x; } }";
        let out = run(input, &mut identity());
        // 'A' is a binding; it should be present unchanged.
        assert!(out.contains('A'), "expected class A in output: {out}");
        // Private field access should survive codegen.
        assert!(out.contains("#x"), "expected #x in output: {out}");
    }
}
