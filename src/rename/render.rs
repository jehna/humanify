use oxc_ast::AstKind;
use oxc_codegen::{Codegen, Context, Gen};
use oxc_semantic::Scoping;

/// Render a scope node through the current scoping so previously renamed
/// identifiers appear in the context passed to the renamer.
pub fn codegen_scope_node(kind: AstKind<'_>, scoping: Scoping) -> Option<String> {
    let mut codegen = Codegen::new().with_scoping(Some(scoping));
    let context = Context::default();

    match kind {
        AstKind::Program(node) => node.print(&mut codegen, context),
        AstKind::Function(node) => node.print(&mut codegen, context),
        AstKind::BlockStatement(node) => node.print(&mut codegen, context),
        AstKind::StaticBlock(node) => node.print(&mut codegen, context),
        AstKind::CatchClause(node) => node.print(&mut codegen, context),
        AstKind::Class(node) => node.print(&mut codegen, context),
        AstKind::ForStatement(node) => node.print(&mut codegen, context),
        AstKind::ForInStatement(node) => node.print(&mut codegen, context),
        AstKind::ForOfStatement(node) => node.print(&mut codegen, context),
        _ => return None,
    }

    Some(codegen.into_source_text())
}
