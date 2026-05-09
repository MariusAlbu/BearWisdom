// =============================================================================
// c_lang/symbols.rs  —  Symbol pushers for C/C++
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, extract_declarator_name,
    find_child_by_kind, first_type_identifier, is_constructor_name, node_text,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
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
fn looks_like_attribute_macro(name: &str) -> bool {
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
fn find_real_specifier_name(node: &Node, src: &[u8]) -> Option<String> {
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

pub(super) fn push_function_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let decl_node = node.child_by_field_name("declarator")?;
    let (name, is_destructor) = extract_declarator_name(&decl_node, src);
    let name = name?;

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kind = if is_destructor {
        SymbolKind::Method
    } else if language != "c" && is_constructor_name(&name, scope) {
        SymbolKind::Constructor
    } else if scope.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let visibility = detect_visibility(node, src);
    let ret_type = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let params = decl_node
        .child_by_field_name("parameters")
        .or_else(|| find_child_by_kind(&decl_node, "parameter_list"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let signature = Some(format!("{ret_type} {name}{params}").trim().to_string());

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_specifier(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name = if let Some(name_node) = node.child_by_field_name("name") {
        let raw = node_text(name_node, src);
        // Tree-sitter-cpp doesn't expand macros, so a header pattern like
        // `class Q_WIDGETS_EXPORT QMessageBox : public QDialog` parses
        // with `Q_WIDGETS_EXPORT` bound to the `name` field and the real
        // class name as a sibling type_identifier. The same holds for
        // every Qt module export macro, every dllexport-style attribute
        // macro, and any project-defined visibility macro. Detect the
        // SCREAMING_SNAKE_CASE shape and look one level deeper for the
        // real identifier — purely structural, no macro name list.
        if looks_like_attribute_macro(&raw) {
            find_real_specifier_name(node, src).unwrap_or(raw)
        } else {
            raw
        }
    } else {
        // Anonymous struct/union/enum — emit with a synthetic name so the
        // coverage engine can match this node.
        let kw = match kind {
            SymbolKind::Class  => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum   => "enum",
            _                  => "struct",
        };
        format!("__anon_{kw}_{}", node.start_position().row)
    };

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = match kind {
        SymbolKind::Class  => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum   => "enum",
        _                  => "struct",
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_namespace(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// C++ `namespace alias = target;`. Tree-sitter exposes the alias under
/// the `name` field and the target as a sibling subtree containing one or
/// more `namespace_identifier` nodes (single-segment for `namespace Dc =
/// DeriveColors;`, multiple for nested forms like `namespace fs =
/// std::filesystem;`).
///
/// Emit a Namespace symbol for the alias so resolvers find it under
/// `same-file` lookup; emit TypeRef refs for each target identifier so
/// the alias→target relationship is preserved in the graph. Resolution of
/// `alias::member` to `target::member` is a follow-up — for now the
/// alias's own ref load (the largest single bucket on KeePassXC's
/// Phantom-style code) goes from `unresolved` to `resolved-same-file`,
/// and the target identifiers themselves get tracked refs.
pub(super) fn push_namespace_alias(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let Some(name_node) = node.child_by_field_name("name") else { return };
    let name = node_text(name_node, src);
    if name.is_empty() { return }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name} = ...")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Emit TypeRef for each `namespace_identifier` after the `=`. The first
    // child past the `=` token is the target; nested namespace targets
    // surface multiple identifiers we want to track.
    let mut cursor = node.walk();
    let mut past_equals = false;
    for child in node.children(&mut cursor) {
        if !past_equals {
            if child.kind() == "=" { past_equals = true; }
            continue;
        }
        emit_namespace_target_refs(&child, src, idx, refs);
    }
}

fn emit_namespace_target_refs(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if matches!(node.kind(), "namespace_identifier" | "type_identifier") {
        let name = node_text(*node, src);
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name,
                kind: EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
                byte_offset: 0,
                namespace_segments: Vec::new(),
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        emit_namespace_target_refs(&child, src, source_idx, refs);
    }
}

pub(super) fn push_typedef(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // For C/C++ `typedef <source_type> <new_name>;`, the grammar lays out
    // children left-to-right as:
    //   typedef  <source_type_node>  <declarator>  ;
    //
    // The declarator (the LAST eligible child, not the first) is the new
    // type alias being introduced.  However, C allows multiple declarators
    // in a single typedef, e.g.:
    //
    //   typedef struct { ... } TypeA, TypeB;
    //   typedef unsigned long size_t, ULONG_PTR;
    //
    // In this case there are TWO names and both should become TypeAlias
    // symbols.  The grammar represents them as two separate `type_identifier`
    // children after the struct/union/enum body.
    //
    // Strategy:
    //   1. Find all eligible declarator children (type_identifier,
    //      pointer_declarator, function_declarator).
    //   2. The FIRST one is potentially the source type (e.g. `HttpRequestPtr`
    //      in `typedef HttpRequestPtr Request;`) — only emit it if there is
    //      no second declarator (it IS the alias in that case).
    //   3. Always emit the LAST declarator as a TypeAlias.
    //   4. If there are more than two declarators, emit all but the first
    //      as TypeAlias symbols (the first is the source type).
    //
    // In practice this means:
    //   typedef T Foo;          → 2 children → emit last (Foo)
    //   typedef struct{} A, B;  → struct body + 2 type_identifiers → both A and B
    let mut cursor = node.walk();
    let mut declarators: Vec<Node> = Vec::new();
    // Check whether there is a struct/union/enum/class body child (anonymous
    // inline specifier).  If yes, ALL subsequent type_identifier nodes are new
    // aliases.  If no, only the LAST one is the new alias.
    let mut has_specifier_body = false;
    // Capture trailing ERROR's identifier when tree-sitter recovers from an
    // unknown macro inside the typedef. Real shapes hit:
    //   typedef __u32 __bitwise __le32;
    //     → [type_id __u32, type_id __bitwise, ERROR(__le32)] — ERROR holds the alias.
    //   typedef __u32 __attribute__((bitwise)) __le32;
    //     → [type_id __u32, function_declarator __attribute__, ERROR(__le32)].
    // Without this, the alias name `__le32` was silently dropped and 5K
    // call/type_ref edges in zig-compiler-fresh's vendored Linux types.h
    // could never resolve.
    let mut trailing_error_name: Option<String> = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Inline specifier with body → all following identifiers are aliases
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => {
                if child.child_by_field_name("body").is_some() {
                    has_specifier_body = true;
                }
                // Reset error tracking — the body legitimately captures its own state.
                trailing_error_name = None;
            }
            // Declarator variants that introduce a new alias name. The
            // canonical shape is `type_identifier`; `pointer_declarator`
            // wraps `typedef X *Y` (the alias name `Y` is inside);
            // `function_declarator` wraps `typedef X (*Fn)(Args)`.
            // **`array_declarator`** wraps `typedef X Y[N]` — common for
            // GMP-compatible types (`typedef bf_t mpz_t[1];`) and zlib
            // typedefs that swipl bundles. **`parenthesized_declarator`**
            // wraps GCC-attribute / `(*name)(...)` shapes that nest
            // around the real declarator.
            "type_identifier"
            | "pointer_declarator"
            | "function_declarator"
            | "array_declarator"
            | "parenthesized_declarator" => {
                declarators.push(child);
                trailing_error_name = None;
            }
            "ERROR" => {
                // Take the single identifier from inside the ERROR if there is one.
                let mut ec = child.walk();
                let idents: Vec<_> = child
                    .children(&mut ec)
                    .filter(|n| n.kind() == "identifier" || n.kind() == "type_identifier")
                    .collect();
                if idents.len() == 1 {
                    trailing_error_name = Some(node_text(idents[0], src));
                }
            }
            _ => {}
        }
    }

    // If the last meaningful node was an ERROR with one identifier, it almost
    // certainly holds the alias name — promote it as the last "declarator" and
    // skip the parsed-but-bogus declarators above it.
    if let Some(name) = trailing_error_name {
        let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
        let scope_path = scope_tree::scope_path(scope);
        let qualified_name = scope_tree::qualify(&name, scope);
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::TypeAlias,
            visibility: None,
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(format!("typedef {name}")),
            doc_comment: extract_doc_comment(node, src),
            scope_path,
            parent_index,
        });
        return;
    }

    if declarators.is_empty() {
        return;
    }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let scope_path = scope_tree::scope_path(scope);
    let doc = extract_doc_comment(node, src);

    // Determine which declarators represent new alias names.
    // - If there is an inline specifier body (e.g. `typedef struct{...} A, B`)
    //   ALL declarators in the list are alias names.
    // - Otherwise, only the last one is the alias (the earlier ones are the
    //   source type chain, e.g. `typedef const unsigned long * Foo`).
    let aliases_start = if has_specifier_body { 0 } else { declarators.len().saturating_sub(1) };

    for decl in &declarators[aliases_start..] {
        let Some(name) = first_type_identifier(decl, src) else { continue; };
        let qualified_name = scope_tree::qualify(&name, scope);
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::TypeAlias,
            visibility: None,
            start_line: node.start_position().row as u32,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: node.end_position().column as u32,
            signature: Some(format!("typedef {name}")),
            doc_comment: doc.clone(),
            scope_path: scope_path.clone(),
            parent_index,
        });
    }
}

/// Returns true if `node` is or contains a `function_declarator` child,
/// indicating this declarator represents a function forward declaration.
fn has_function_declarator(node: &Node) -> bool {
    if node.kind() == "function_declarator" {
        return true;
    }
    // pointer_declarator and parenthesized_declarator can wrap a function_declarator,
    // e.g. `(*fp)(int)` or `virtual int area() = 0` which becomes
    // `pointer_declarator` → `function_declarator`.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if has_function_declarator(&child) {
            return true;
        }
    }
    false
}

pub(super) fn push_declaration(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let scope_path = scope_tree::scope_path(scope);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let name_opt = match child.kind() {
            // `identifier` — plain declarations and non-struct members
            // `field_identifier` — struct/union member names in C grammar
            "identifier" | "field_identifier" => Some(node_text(child, src)),
            // Declarator variants that wrap an identifier
            "init_declarator" | "pointer_declarator" | "reference_declarator"
            | "array_declarator" | "parenthesized_declarator"
            | "function_declarator" | "abstract_function_declarator" => {
                first_type_identifier(&child, src)
            }
            // C++17 structured bindings: `auto [a, b] = expr;`
            "structured_binding_declarator" => first_type_identifier(&child, src),
            _ => None,
        };
        if let Some(name) = name_opt {
            let qualified_name = scope_tree::qualify(&name, scope);
            // Forward declarations whose declarator is (or contains) a
            // function_declarator represent function/method signatures, not variables.
            let kind = if has_function_declarator(&child) {
                if scope.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                }
            } else {
                SymbolKind::Variable
            };
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind,
                visibility: detect_visibility(node, src),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: Some(format!("{type_str} {name}")),
                doc_comment: None,
                scope_path: scope_path.clone(),
                parent_index,
            });
        }
    }
}

pub(super) fn extract_enum_body(
    body: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let enum_qname = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enumerator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = if enum_qname.is_empty() {
                    name.clone()
                } else {
                    format!("{enum_qname}.{name}")
                };
                let scope = enclosing_scope(scope_tree, child.start_byte(), child.end_byte());
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::EnumMember,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: scope_tree::scope_path(scope),
                    parent_index,
                });
            }
        }
    }
}

pub(super) fn push_include(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_literal" | "system_lib_string" => {
                let raw = node_text(child, src);
                let path = raw.trim_matches('"').trim_matches('<').trim_matches('>');
                let target_name = path.rsplit('/').next().unwrap_or(path).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name,
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    module: Some(path.to_string()),
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// template_declaration — C++ `template<typename T> class/struct/fn { ... }`
// ---------------------------------------------------------------------------

/// Returns the inner declaration node (class/struct/function/etc) and its
/// optional symbol index after pushing it.  The caller is responsible for
/// recursing into the body.
///
/// We emit one TypeRef per type-parameter constraint when present (e.g.
/// `template<typename T, typename U = int>` → TypeRef to `int`).
pub(super) fn push_template_decl<'a>(
    node: &'a Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> (Option<usize>, Option<Node<'a>>) {
    // The inner declaration is the last named child that is not the template
    // parameter list.
    let mut inner: Option<Node<'a>> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "template_parameter_list" => {
                // emit TypeRef for default type arguments  e.g. `typename T = Foo`
                emit_template_param_typerefs(&child, src, symbols.len(), refs);
            }
            "class_specifier" | "struct_specifier" | "union_specifier"
            | "function_definition" | "alias_declaration" | "declaration"
            | "concept_definition" => {
                inner = Some(child);
            }
            _ => {}
        }
    }

    let inner_node = match inner {
        Some(n) => n,
        None => return (None, None),
    };

    // Push a symbol for the inner declaration.
    let idx = match inner_node.kind() {
        "class_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Class, symbols, parent_index)
        }
        "struct_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Struct, symbols, parent_index)
        }
        "union_specifier" => {
            push_specifier(&inner_node, src, scope_tree, SymbolKind::Struct, symbols, parent_index)
        }
        "function_definition" => {
            push_function_def(&inner_node, src, scope_tree, language, symbols, parent_index)
        }
        "concept_definition" => {
            push_concept_def(&inner_node, src, scope_tree, symbols, parent_index)
        }
        _ => None,
    };

    (idx, Some(inner_node))
}

// ---------------------------------------------------------------------------
// concept_definition — C++20 `template<typename T> concept Foo = expr;`
// ---------------------------------------------------------------------------

pub(super) fn push_concept_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // concept_definition: `concept` `identifier` `=` expression
    let name_node = if let Some(n) = node.child_by_field_name("name") {
        n
    } else {
        let mut cursor = node.walk();
        let found = node.children(&mut cursor).find(|c| c.kind() == "identifier");
        found?
    };

    let name = node_text(name_node, src);
    if name.is_empty() {
        return None;
    }

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias, // concepts are type-constraint aliases
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("concept {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    Some(idx)
}

fn emit_template_param_typerefs(
    param_list: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        // `optional_type_parameter_declaration` is the node kind for
        // `typename T = SomeType` — it has a default type after `=`.
        // `type_parameter_declaration` is the plain `typename T` variant (no default).
        if child.kind() == "optional_type_parameter_declaration" {
            // Walk children after `=` and emit TypeRef for any named type.
            let mut after_eq = false;
            let mut ic = child.walk();
            for inner in child.children(&mut ic) {
                if inner.kind() == "=" {
                    after_eq = true;
                } else if after_eq {
                    // Could be `type_identifier`, `template_type`, `qualified_identifier`, etc.
                    emit_typerefs_for_type_descriptor(inner, src, source_idx, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// alias_declaration — C++ `using Alias = Type;`
// ---------------------------------------------------------------------------

pub(super) fn push_alias_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Structure: `using` <name> `=` `type_descriptor` `;`
    // The name can be: type_identifier, template_type (e.g. `using Foo<T> = Bar`)
    // or qualified_identifier. Skip non-identifier second children.
    let name_node = match node.child(1) {
        Some(n) if matches!(
            n.kind(),
            "type_identifier" | "identifier" | "template_type" | "qualified_identifier"
        ) => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("using {name} = ...")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // TypeRef for the aliased type.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_descriptor" {
            emit_typerefs_for_type_descriptor(child, src, idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// using_declaration — C++ `using std::vector;`  (namespace using, no `=`)
// ---------------------------------------------------------------------------

pub(super) fn push_using_decl(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The identifier after `using` is a qualified_identifier or identifier.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_identifier" | "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// preproc_def — `#define FOO value`  → Constant/Variable
// ---------------------------------------------------------------------------

pub(super) fn push_preproc_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Children: `#define`, `identifier`, optional `preproc_arg`
    let name_node = match node.child(1) {
        Some(n) if n.kind() == "identifier" => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let value = node
        .child(2)
        .filter(|n| n.kind() == "preproc_arg")
        .map(|n| node_text(n, src));
    let signature = Some(match &value {
        Some(v) => format!("#define {name} {v}"),
        None => format!("#define {name}"),
    });

    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// preproc_function_def — `#define MAX(a, b) ...`  → Function
// ---------------------------------------------------------------------------

pub(super) fn push_preproc_function_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Children: `#define`, `identifier`, `preproc_params`, optional `preproc_arg`
    let name_node = match node.child(1) {
        Some(n) if n.kind() == "identifier" => n,
        _ => return,
    };
    let name = node_text(name_node, src);
    let params = node
        .child(2)
        .filter(|n| n.kind() == "preproc_params")
        .map(|n| node_text(n, src))
        .unwrap_or_default();

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("#define {name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
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
