// =============================================================================
// generic/helpers.rs  —  Import extraction, call detection, type helpers
// =============================================================================

use super::extract::ExtractionCtx;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Language-specific import module extraction
// ---------------------------------------------------------------------------

/// Extract import module and imported symbol name for a given language and
/// import node.
///
/// Returns `(module_path, imported_name)` where both may be `None`.
/// When `imported_name` is `None` the caller MUST skip the ref entirely.
pub(super) fn extract_import_parts<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
    language: &str,
) -> (Option<String>, Option<String>) {
    match language {
        "go" => extract_go_import(node, ctx),
        "rust" => extract_rust_import(node, ctx),
        "python" => extract_python_import(node, ctx),
        "java" => extract_java_import(node, ctx),
        "ruby" => extract_ruby_import(node, ctx),
        "php" => extract_php_import(node, ctx),
        _ => extract_generic_import(node, ctx),
    }
}

/// Go: `import_spec` → child `interpreted_string_literal`, strip quotes.
fn extract_go_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    let kind = node.kind();
    let spec_node = if kind == "import_spec" {
        node
    } else {
        let mut found = None;
        for i in 0..node.child_count() {
            let child = node.child(i).unwrap();
            if child.kind() == "import_spec" || child.kind() == "interpreted_string_literal" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(n) => n,
            None => return extract_generic_import(node, ctx),
        }
    };

    let raw: Option<&str> = if spec_node.kind() == "interpreted_string_literal" {
        Some(strip_quotes(ctx.text(spec_node)))
    } else {
        let mut found = None;
        for i in 0..spec_node.child_count() {
            let child = spec_node.child(i).unwrap();
            if child.kind() == "interpreted_string_literal" {
                found = Some(strip_quotes(ctx.text(child)));
                break;
            }
        }
        found
    };

    let module = raw.filter(|s| !s.is_empty()).map(|s| s.to_string());

    let target = module
        .as_deref()
        .and_then(|m| m.rsplit('/').next())
        .map(|s| s.to_string());

    (module, target)
}

/// Rust: `use_declaration` → text of `scoped_identifier` or `identifier` child.
fn extract_rust_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    let arg = node.child_by_field_name("argument");
    let path_node = arg.or_else(|| {
        for i in 0..node.child_count() {
            let child = node.child(i).unwrap();
            match child.kind() {
                "scoped_identifier" | "identifier" | "use_wildcard"
                | "use_as_clause" | "use_list" => return Some(child),
                _ => {}
            }
        }
        None
    });

    let module = path_node.map(|n| ctx.text(n).trim().to_string());
    let target = module.as_deref().and_then(|m| {
        let m = m.trim_end_matches("::*");
        m.rsplit("::").next().map(|s| s.trim_matches('{').trim_matches('}').trim().to_string())
    }).filter(|s| !s.is_empty());

    (module, target)
}

/// Python: `import_statement` → dotted_name child.
/// `import_from_statement` → module_name field + dotted_name/identifier children.
fn extract_python_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    let kind = node.kind();

    if kind == "import_from_statement" {
        let module = node
            .child_by_field_name("module_name")
            .map(|n| ctx.text(n).trim().to_string())
            .or_else(|| {
                for i in 0..node.child_count() {
                    let child = node.child(i).unwrap();
                    if child.kind() == "dotted_name" || child.kind() == "relative_import" {
                        return Some(ctx.text(child).trim().to_string());
                    }
                }
                None
            });

        let target = {
            let mut after_import = false;
            let mut found: Option<String> = None;
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if child.kind() == "import" {
                    after_import = true;
                    continue;
                }
                if after_import {
                    match child.kind() {
                        "identifier" | "dotted_name" => {
                            let t = ctx.text(child).trim().to_string();
                            if !t.is_empty() {
                                found = Some(t);
                                break;
                            }
                        }
                        "aliased_import" => {
                            if let Some(name_node) = child.child_by_field_name("name") {
                                let t = ctx.text(name_node).trim().to_string();
                                if !t.is_empty() {
                                    found = Some(t);
                                    break;
                                }
                            }
                        }
                        "wildcard_import" => {
                            found = Some("*".to_string());
                            break;
                        }
                        _ => {}
                    }
                }
            }
            found
        };

        return (module, target);
    }

    // `import os` or `import os as o`
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "dotted_name" | "identifier" => {
                let t = ctx.text(child).trim().to_string();
                if !t.is_empty() {
                    let target = t.split('.').next().map(|s| s.to_string());
                    return (Some(t), target);
                }
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let t = ctx.text(name_node).trim().to_string();
                    if !t.is_empty() {
                        let target = t.split('.').next().map(|s| s.to_string());
                        return (Some(t), target);
                    }
                }
            }
            _ => {}
        }
    }

    (None, None)
}

/// Java: `import_declaration` → text of scoped_identifier child.
fn extract_java_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "scoped_identifier" | "identifier" => {
                let t = ctx.text(child).trim().to_string();
                if !t.is_empty() {
                    let target = t.rsplit('.').next().map(|s| s.to_string());
                    let module = if t.contains('.') {
                        Some(t.rsplitn(2, '.').nth(1).unwrap_or(&t).to_string())
                    } else {
                        Some(t.clone())
                    };
                    return (module, target);
                }
            }
            _ => {}
        }
    }
    (None, None)
}

/// Ruby: `require` / `require_relative` calls — first string argument.
fn extract_ruby_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "string" || child.kind() == "string_content" {
            let t = strip_quotes(ctx.text(child));
            if !t.is_empty() {
                let target = t.rsplit('/').next().map(|s| s.to_string());
                return (Some(t.to_string()), target);
            }
        }
        if child.kind() == "argument_list" {
            for j in 0..child.child_count() {
                let grandchild = child.child(j).unwrap();
                if grandchild.kind() == "string" {
                    let t = strip_quotes(ctx.text(grandchild));
                    if !t.is_empty() {
                        let target = t.rsplit('/').next().map(|s| s.to_string());
                        return (Some(t.to_string()), target);
                    }
                }
            }
        }
    }
    (None, None)
}

/// PHP: `include_statement` / `require_once` — string argument.
fn extract_php_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "string" | "encapsed_string" => {
                let t = strip_quotes(ctx.text(child));
                if !t.is_empty() {
                    let target = t.rsplit('/').next().map(|s| s.to_string());
                    return (Some(t.to_string()), target);
                }
            }
            _ => {}
        }
    }
    (None, None)
}

/// Generic fallback: use field names and well-known literal kinds.
fn extract_generic_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    for field in &["source", "path", "module", "name"] {
        if let Some(n) = node.child_by_field_name(field) {
            let raw = ctx.text(n).trim();
            let text = raw.trim_matches('"').trim_matches('\'');
            if !text.is_empty() {
                let target = text.rsplit('/').next()
                    .or_else(|| text.rsplit('.').next())
                    .map(|s| s.to_string());
                return (Some(text.to_string()), target);
            }
        }
    }

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "string" | "string_literal" | "interpreted_string_literal" => {
                let t = strip_quotes(ctx.text(child));
                if !t.is_empty() {
                    let target = t.rsplit('/').next()
                        .or_else(|| t.rsplit('.').next())
                        .map(|s| s.to_string());
                    return (Some(t.to_string()), target);
                }
            }
            "dotted_name" | "scoped_identifier" | "module_path" => {
                let t = ctx.text(child).trim().to_string();
                if !t.is_empty() {
                    let target = t.rsplit('/').next()
                        .or_else(|| t.rsplit('.').next())
                        .map(|s| s.to_string());
                    return (Some(t), target);
                }
            }
            _ => {}
        }
    }

    let full = ctx.text(node).trim();
    if !full.is_empty() && full.len() <= 256 {
        let cleaned = full
            .trim_start_matches("import ")
            .trim_start_matches("use ")
            .trim_start_matches("require ")
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim_end_matches(';');
        if !cleaned.is_empty() {
            let target = cleaned.rsplit('/').next()
                .or_else(|| cleaned.rsplit('.').next())
                .map(|s| s.to_string());
            return (Some(cleaned.to_string()), target);
        }
    }

    (None, None)
}

/// Strip surrounding quotes from a string literal text.
pub(super) fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('`') && s.ends_with('`'))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Call-reference helpers
// ---------------------------------------------------------------------------

/// Returns true for node kinds that represent a function or method call.
pub(super) fn is_call_node(kind: &str) -> bool {
    matches!(
        kind,
        "call_expression"           // JS/TS, C, Go, Rust (macro calls are separate)
            | "method_call_expression"  // Rust
            | "call"                    // Ruby
            | "function_call"           // Lua, some others
            | "invocation_expression"   // C# — e.g. `foo.Bar()`
            | "method_invocation"       // Java — `object.method(args)`
    )
}

/// Extract the callee name from a call node.
pub(super) fn extract_call_target<'src>(node: Node<'_>, ctx: &ExtractionCtx<'src>) -> Option<String> {
    for field in &["function", "method", "name"] {
        if let Some(n) = node.child_by_field_name(field) {
            let name = match n.kind() {
                "member_expression" | "field_expression" | "scoped_identifier" => {
                    if let Some(prop) = n.child_by_field_name("property")
                        .or_else(|| n.child_by_field_name("field"))
                        .or_else(|| n.child_by_field_name("name"))
                    {
                        ctx.text(prop).trim().to_string()
                    } else {
                        let mut last = String::new();
                        for i in 0..n.child_count() {
                            let c = n.child(i).unwrap();
                            if c.kind() == "identifier" || c.kind() == "field_identifier" {
                                last = ctx.text(c).trim().to_string();
                            }
                        }
                        last
                    }
                }
                "identifier" | "field_identifier" | "simple_identifier" => {
                    ctx.text(n).trim().to_string()
                }
                _ => {
                    let t = ctx.text(n).trim().to_string();
                    if t.len() <= 64 && !t.contains('(') {
                        t
                    } else {
                        String::new()
                    }
                }
            };
            if !name.is_empty() {
                return Some(name);
            }
        }
    }

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "identifier" {
            let t = ctx.text(child).trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }

    None
}

/// Returns true when `node` (a `type_identifier`) is in the name position of a
/// declaration — i.e., it is the node that *defines* the type, not a *reference* to it.
pub(super) fn is_declaration_name_position(node: Node<'_>, parent: Node<'_>) -> bool {
    use super::extract::node_kind_to_symbol_kind;
    let parent_kind = parent.kind();

    let is_decl = node_kind_to_symbol_kind(parent_kind).is_some()
        || parent_kind == "type_spec"
        || parent_kind == "type_alias_declaration";
    if !is_decl {
        return false;
    }

    if let Some(name_child) = parent.child_by_field_name("name") {
        return name_child.id() == node.id();
    }

    for i in 0..parent.child_count() {
        let child = parent.child(i).unwrap();
        if child.kind() == "type_identifier" {
            return child.id() == node.id();
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Inheritance / implements extraction
// ---------------------------------------------------------------------------

/// Inspect a declaration node for base class and interface references.
pub(super) fn extract_inheritance_refs<'src>(
    node: Node<'_>,
    ctx: &mut ExtractionCtx<'src>,
    _kind: &str,
    source_idx: usize,
) {
    let line = node.start_position().row as u32;

    if let Some(sc) = node.child_by_field_name("superclasses") {
        for_each_type_child(sc, ctx, source_idx, line, EdgeKind::Inherits);
    }

    if let Some(sc) = node.child_by_field_name("superclass") {
        for_each_type_child(sc, ctx, source_idx, line, EdgeKind::Inherits);
    }

    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        let ck = child.kind();

        match ck {
            "superclass" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }
            "super_interfaces" | "implements_clause" | "class_interface_clause" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Implements);
            }
            "delegation_specifiers" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }
            "base_class_clause" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }
            "inheritance_clause" | "type_list" | "interfaces" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }
            "base_clause" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }
            "extends_clause" | "extends_type" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }
            "extends_interfaces" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }
            _ => {}
        }
    }
}

/// Extract a type name from a node.
pub(super) fn extract_type_name<'src>(node: Node<'_>, ctx: &ExtractionCtx<'src>) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "simple_identifier" | "constant" => {
            let t = ctx.text(node).trim().to_string();
            if !t.is_empty() { Some(t) } else { None }
        }
        "scoped_identifier" | "scope_resolution" | "member_expression" | "qualified_type" => {
            let t = ctx.text(node).trim().to_string();
            if !t.is_empty() && t.len() <= 128 { Some(t) } else { None }
        }
        "generic_type" | "parameterized_type" => {
            if let Some(base) = node.child(0) {
                extract_type_name(base, ctx)
            } else {
                None
            }
        }
        "type_list" => {
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if let Some(name) = extract_type_name(child, ctx) {
                    return Some(name);
                }
            }
            None
        }
        _ => {
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if matches!(child.kind(), "identifier" | "type_identifier" | "simple_identifier") {
                    let t = ctx.text(child).trim().to_string();
                    if !t.is_empty() { return Some(t); }
                }
            }
            None
        }
    }
}

/// Walk children of a clause node and emit a ref for each type found.
pub(super) fn for_each_type_child<'src>(
    clause: Node<'_>,
    ctx: &mut ExtractionCtx<'src>,
    source_idx: usize,
    line: u32,
    edge_kind: EdgeKind,
) {
    for i in 0..clause.child_count() {
        let child = clause.child(i).unwrap();
        if child.is_named() {
            if let Some(name) = extract_type_name(child, ctx) {
                ctx.refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: edge_kind,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
    }
}
