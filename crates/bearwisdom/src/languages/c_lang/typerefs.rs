// =============================================================================
// c_lang/typerefs.rs  —  TypeRef emission helpers + base-class extraction
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Helpers — type reference emission
// ---------------------------------------------------------------------------

/// C/C++ reserved words, storage-class specifiers, and type qualifiers that
/// are sometimes captured as `type_identifier` nodes by tree-sitter-cpp when
/// they appear in template parameter lists or `using`/`class` aliases. They
/// must not become TypeRef target names — filtering them here cleans up
/// hundreds of false positives per C++ project without touching the tree-
/// walk logic.
const CPP_KEYWORD_BLOCKLIST: &[&str] = &[
    "class", "struct", "union", "enum", "typename", "using", "namespace",
    "public", "private", "protected", "virtual", "static", "extern",
    "const", "constexpr", "consteval", "constinit", "volatile", "mutable",
    "inline", "friend", "explicit", "operator", "template", "typedef",
    "final", "override", "noexcept", "throw",
    "true", "false", "nullptr", "this",
    "return", "if", "else", "for", "while", "do", "switch", "case", "default",
    "break", "continue", "goto",
    "sizeof", "alignof", "decltype", "new", "delete",
    "auto", "void",
    // Template type-parameter placeholder names the extractor emits but
    // which are NEVER resolvable — they're locally-scoped and should stay
    // inside the template definition.
    "T", "U", "V", "K", "Args",
];

fn is_cpp_keyword(name: &str) -> bool {
    CPP_KEYWORD_BLOCKLIST.contains(&name)
}

/// Detect names shaped like `SCREAMING_SNAKE_CASE` or `_LEADING_SCREAMING`,
/// which are conventionally macros in C/C++ (Qt's `Q_WIDGETS_EXPORT`,
/// MSVC's `__declspec`, project-defined visibility shims, etc.). When
/// tree-sitter-cpp can't expand a macro before a class/struct name it
/// binds the macro identifier to the `name` field; a structural rule
/// detects this without needing to know any specific macro names.
///
/// The rule:
///   * non-empty
///   * all chars are uppercase ASCII letters, digits, or `_`
///   * contains at least one `_` (so single-letter identifiers like `T`/`U`
///     and acronym-only names like `URL` aren't misclassified — those go
///     through the normal name path)
pub(super) fn looks_like_attribute_macro(name: &str) -> bool {
    if name.is_empty() { return false }
    let mut has_underscore = false;
    for ch in name.chars() {
        if ch == '_' {
            has_underscore = true;
            continue;
        }
        if !(ch.is_ascii_uppercase() || ch.is_ascii_digit()) {
            return false;
        }
    }
    has_underscore
}

/// When `push_specifier`'s `name` field returned a macro shape, search for
/// the real class/struct/enum/union identifier. Two parse shapes apply:
///
///   * **Self-contained** — tree-sitter kept the real name as a child of
///     the same `class_specifier` (rare; only happens for very short
///     standalone snippets).
///   * **Sibling-scattered** — when surrounding context (Q_PROPERTY
///     macros, Q_OBJECT, attribute clauses) confuses the parser, the real
///     identifier becomes a NEXT-SIBLING of the class_specifier under the
///     enclosing declaration_list / translation_unit. This is the shape
///     produced by every Qt class header in the wild.
///
/// We probe children first; then fall back to scanning forward across
/// siblings until we hit a body / brace / semicolon. The skip targets
/// (`field_declaration_list`, `compound_statement`, `;`) bound the
/// search so we never wander into another top-level declaration.
pub(super) fn find_real_specifier_name(node: &Node, src: &[u8]) -> Option<String> {
    // 1) Children of this class_specifier (self-contained shape).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "field_declaration_list"
                | "compound_statement"
                | "enumerator_list"
                | "base_class_clause"
        ) {
            break;
        }
        if matches!(child.kind(), "type_identifier" | "qualified_identifier") {
            let text = node_text(child, src);
            if !text.is_empty() && !looks_like_attribute_macro(&text) {
                return Some(text);
            }
        }
    }

    // 2) Next siblings (Qt's real-world shape).
    let mut sib = node.next_sibling();
    while let Some(s) = sib {
        match s.kind() {
            // Stop at the brace that opens the body, the semicolon that
            // ends the declaration, or any nested compound/declaration
            // structure — we're past the header by then.
            "{" | ";" | "compound_statement" | "field_declaration_list" => break,
            "identifier" | "type_identifier" | "qualified_identifier" => {
                let text = node_text(s, src);
                if !text.is_empty() && !looks_like_attribute_macro(&text) {
                    return Some(text);
                }
            }
            _ => {}
        }
        sib = s.next_sibling();
    }
    None
}

/// Emit a single TypeRef edge from `source_idx` to the type named by `name_node`.
fn push_typeref(name_node: Node, src: &[u8], source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let name = node_text(name_node, src);
    if name.is_empty()
        || is_cpp_keyword(&name)
        || super::predicates::is_c_compiler_intrinsic(&name)
    {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: source_idx,
        target_name: name,
        kind: EdgeKind::TypeRef,
        line: name_node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

/// Walk a `type_descriptor` (or any node) and emit TypeRef for every
/// `type_identifier` found.  Stops at leaf nodes — does not recurse into
/// sub-expressions to avoid false positives.
pub(super) fn emit_typerefs_for_type_descriptor(
    node: Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "type_identifier" => {
            push_typeref(node, src, source_idx, refs);
        }
        "primitive_type" | "auto" | "void" => {
            // primitives — skip
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                emit_typerefs_for_type_descriptor(child, src, source_idx, refs);
            }
        }
    }
}

pub(super) fn extract_bases(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_class_clause" {
            let mut bc = child.walk();
            for base in child.children(&mut bc) {
                match base.kind() {
                    "type_identifier" => {
                        let name = node_text(base, src);
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind: EdgeKind::Inherits,
                            line: base.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                    "base_class_specifier" => {
                        let mut ic = base.walk();
                        for inner in base.children(&mut ic) {
                            if inner.kind() == "type_identifier" {
                                let name = node_text(inner, src);
                                refs.push(ExtractedRef {
                                    source_symbol_index: source_idx,
                                    target_name: name,
                                    kind: EdgeKind::Inherits,
                                    line: inner.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
