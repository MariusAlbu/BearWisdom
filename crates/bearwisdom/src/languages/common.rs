// =============================================================================
// languages/common.rs  —  shared extraction utilities used by multiple plugins
//
// Functions here are language-agnostic helpers that would otherwise be
// duplicated across per-language call extractors.  They live here rather than
// in `languages/mod.rs` to keep the plugin registry and trait definitions
// uncluttered.
// =============================================================================

/// When a call has a chain (e.g. `Foo::bar()`, `Foo.bar()`), emit a `TypeRef`
/// for the type prefix — the segment before the final method name — if it
/// looks like a type (starts with uppercase).
pub fn emit_chain_type_ref(
    chain: &Option<crate::types::MemberChain>,
    source_symbol_index: usize,
    func_node: &tree_sitter::Node,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    let c = match chain.as_ref() {
        Some(c) if c.segments.len() >= 2 => c,
        _ => return,
    };
    let type_seg = &c.segments[c.segments.len() - 2];
    if type_seg.name.chars().next().map_or(false, |ch| ch.is_uppercase()) {
        refs.push(crate::types::ExtractedRef {
            source_symbol_index,
            target_name: type_seg.name.clone(),
            kind: crate::types::EdgeKind::TypeRef,
            line: func_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}
