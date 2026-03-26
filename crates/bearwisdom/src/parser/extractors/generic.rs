// =============================================================================
// parser/extractors/generic.rs  —  universal grammar-based extractor
//
// Works for any language that has a tree-sitter grammar registered in
// parser/languages.rs.  It does a single-pass DFS over the CST and maps
// well-known node-type names to SymbolKind values.
//
// What it extracts
// ----------------
// SYMBOLS:
//   Functions/methods, classes, structs, interfaces, enums, type aliases,
//   top-level constants/variables.
//
// REFERENCES:
//   Import / require / use statements → Imports edges.
//
// Limitations vs. dedicated extractors:
//   - No visibility inference — all symbols are emitted with None visibility.
//   - No signatures.
//
// When a dedicated extractor exists (csharp, typescript, tsx) the indexer will
// use it in preference to this module.
// =============================================================================

use crate::parser::languages;
use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

pub struct GenericExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

// ---------------------------------------------------------------------------
// Per-language scope configurations
// ---------------------------------------------------------------------------

/// Scope-opening node kinds for Python.
static PYTHON_SCOPE_CONFIG: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_definition",   name_field: "name" },
    ScopeKind { node_kind: "function_definition", name_field: "name" },
];

/// Scope-opening node kinds for Java.
static JAVA_SCOPE_CONFIG: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration",     name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",      name_field: "name" },
    ScopeKind { node_kind: "method_declaration",    name_field: "name" },
    ScopeKind { node_kind: "constructor_declaration", name_field: "name" },
];

/// Scope-opening node kinds for Go.
static GO_SCOPE_CONFIG: &[ScopeKind] = &[
    // type_spec holds the name of the type (struct, interface, alias).
    ScopeKind { node_kind: "type_spec",           name_field: "name" },
    ScopeKind { node_kind: "function_declaration", name_field: "name" },
    ScopeKind { node_kind: "method_declaration",  name_field: "name" },
];

/// Scope-opening node kinds for Rust.
static RUST_SCOPE_CONFIG: &[ScopeKind] = &[
    ScopeKind { node_kind: "mod_item",      name_field: "name" },
    ScopeKind { node_kind: "struct_item",   name_field: "name" },
    ScopeKind { node_kind: "enum_item",     name_field: "name" },
    ScopeKind { node_kind: "trait_item",    name_field: "name" },
    // impl_item uses "type" as the name field (e.g. `impl Point`).
    ScopeKind { node_kind: "impl_item",     name_field: "type" },
    ScopeKind { node_kind: "function_item", name_field: "name" },
];

/// Scope-opening node kinds for Ruby.
static RUBY_SCOPE_CONFIG: &[ScopeKind] = &[
    ScopeKind { node_kind: "class",            name_field: "name" },
    ScopeKind { node_kind: "module",           name_field: "name" },
    ScopeKind { node_kind: "method",           name_field: "name" },
    ScopeKind { node_kind: "singleton_method", name_field: "name" },
];

/// Scope-opening node kinds for PHP.
static PHP_SCOPE_CONFIG: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_definition", name_field: "name" },
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    ScopeKind { node_kind: "method_declaration",   name_field: "name" },
    ScopeKind { node_kind: "function_definition",  name_field: "name" },
];

/// Return the scope config slice for a given language identifier.
/// Returns an empty slice for languages without scope config (C#, TS handled
/// by dedicated extractors; others fall back to no scope qualification).
pub fn scope_config_for(lang: &str) -> &'static [ScopeKind] {
    match lang {
        "python"     => PYTHON_SCOPE_CONFIG,
        "java"       => JAVA_SCOPE_CONFIG,
        "go"         => GO_SCOPE_CONFIG,
        "rust"       => RUST_SCOPE_CONFIG,
        "ruby"       => RUBY_SCOPE_CONFIG,
        "php"        => PHP_SCOPE_CONFIG,
        _            => &[],
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract symbols from any source file using tree-sitter heuristics.
///
/// Returns `None` if no grammar is registered for `language`.
pub fn extract(source: &str, language: &str) -> Option<GenericExtraction> {
    let ts_lang = languages::get_language(language)?;

    let mut parser = Parser::new();
    // set_language only fails if the language ABI version is incompatible;
    // our grammar loader already validates this at load time.
    parser.set_language(&ts_lang).ok()?;

    let tree = parser.parse(source.as_bytes(), None)?;
    let root = tree.root_node();
    let has_errors = root.has_error();
    let src = source.as_bytes();

    // Build the scope tree for this language before the DFS walk.
    let config = scope_config_for(language);
    let scope_tree = scope_tree::build(root, src, config);

    let mut ctx = ExtractionCtx {
        src,
        symbols: Vec::new(),
        refs: Vec::new(),
        scope_tree,
        // Stack for tracking parent symbol indices — independent of scope tree.
        parent_index_stack: Vec::new(),
    };

    walk_node(root, &mut ctx, language);

    Some(GenericExtraction {
        symbols: ctx.symbols,
        refs: ctx.refs,
        has_errors,
    })
}

// ---------------------------------------------------------------------------
// Internal context
// ---------------------------------------------------------------------------

struct ExtractionCtx<'src> {
    src: &'src [u8],
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
    /// Pre-built scope tree for qualified-name lookups.
    scope_tree: ScopeTree,
    /// Stack of symbol indices for tracking parent_index on nested symbols.
    /// Each entry is (symbol_index, pushes_scope) where pushes_scope means
    /// children should be parented to this symbol.
    parent_index_stack: Vec<usize>,
}

impl<'src> ExtractionCtx<'src> {
    /// The symbol index of the immediately enclosing scope, if any.
    fn parent_symbol_index(&self) -> Option<usize> {
        self.parent_index_stack.last().copied()
    }

    /// Text of a node.
    fn text(&self, node: Node) -> &str {
        node.utf8_text(self.src).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Node kind → SymbolKind mapping
// ---------------------------------------------------------------------------

/// Returns the SymbolKind for a node kind string, or None if the node kind
/// is not a declaration we want to extract.
fn node_kind_to_symbol_kind(kind: &str) -> Option<SymbolKind> {
    match kind {
        // Functions
        "function_definition"   // Python, C, C++, Go (func_literal)
        | "function_declaration"// C, C++
        | "function_item"       // Rust
        | "func_literal"        // Go
        | "method_spec"         // Go interface method spec
        => Some(SymbolKind::Function),

        // Methods (pure method declarations — function_item is handled above as
        // Function and contextually promoted to Method in walk_node)
        "method_definition"
        | "method_declaration"
        | "method"              // Ruby
        | "singleton_method"    // Ruby
        | "func_declaration"    // Go top-level func (also caught below via field check)
        => Some(SymbolKind::Method),

        // Classes
        "class_definition"      // Python
        | "class_declaration"   // Java, C#, TypeScript
        | "class_specifier"     // C++ class
        | "class"               // Ruby
        => Some(SymbolKind::Class),

        // Modules (Ruby, Elixir)
        "module"                // Ruby module
        | "defmodule"           // Elixir (heuristic — handled via name extraction)
        => Some(SymbolKind::Namespace),

        // Structs
        "struct_item"           // Rust
        | "struct_specifier"    // C/C++
        | "struct_declaration"
        => Some(SymbolKind::Struct),

        // Interfaces
        "interface_declaration" // Java, C#, TypeScript
        | "trait_item"          // Rust
        | "protocol_declaration"// Swift
        => Some(SymbolKind::Interface),

        // Enums
        "enum_declaration"      // Java, C#, TypeScript
        | "enum_item"           // Rust
        | "enum_definition"
        => Some(SymbolKind::Enum),

        // Type aliases
        "type_alias_declaration"
        | "type_item"           // Rust
        | "type_alias"
        => Some(SymbolKind::TypeAlias),

        // Go type declarations — type_spec holds the actual name; the body
        // determines the kind.  Handled specially in walk_node via
        // handle_go_type_spec().
        "type_spec" => Some(SymbolKind::TypeAlias), // fallback; walk_node overrides

        // Module-level constants/statics
        "const_item"   // Rust
        | "static_item"// Rust
        => Some(SymbolKind::Variable),

        _ => None,
    }
}

/// Determine the correct SymbolKind for a Go `type_spec` node by inspecting
/// the body child type.
fn go_type_spec_kind(node: Node, ctx: &ExtractionCtx) -> SymbolKind {
    // type_spec has fields: name, type_parameters (opt), value (the body)
    if let Some(value) = node.child_by_field_name("type") {
        match value.kind() {
            "struct_type" => return SymbolKind::Struct,
            "interface_type" => return SymbolKind::Interface,
            _ => {}
        }
    }
    // Also scan unnamed children for a body-type node.
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "struct_type" => return SymbolKind::Struct,
            "interface_type" => return SymbolKind::Interface,
            _ => {}
        }
    }
    let _ = ctx; // suppress unused warning
    SymbolKind::TypeAlias
}

/// Returns true if this node kind is an import/use declaration.
fn is_import_node(kind: &str) -> bool {
    matches!(
        kind,
        "import_statement"
            | "import_declaration"
            | "import_from_statement" // Python
            | "using_directive"       // C#
            | "use_declaration"       // Rust
            | "include_statement"     // PHP
            | "require_call"          // Ruby (heuristic)
            | "load_statement"        // Starlark/Bazel
    )
}

// ---------------------------------------------------------------------------
// Name extraction helpers
// ---------------------------------------------------------------------------

/// Try to extract the name of a declaration node.
///
/// Strategy (in priority order):
///   1. The `name` named field.
///   2. The first `identifier` or `type_identifier` child.
///   3. Empty string (caller will skip).
fn extract_name<'src>(node: Node, ctx: &ExtractionCtx<'src>) -> Option<String> {
    // 1. Named `name` field (works for most grammars).
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = ctx.text(name_node).trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }

    // 2. First identifier-like child.
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        let kind = child.kind();
        if kind == "identifier"
            || kind == "type_identifier"
            || kind == "simple_identifier" // Kotlin/Swift
            || kind == "name_identifier"   // Kotlin
        {
            let text = ctx.text(child).trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Language-specific import module extraction
// ---------------------------------------------------------------------------

/// Extract import module and imported symbol name for a given language and
/// import node.
///
/// Returns `(module_path, imported_name)` where both may be `None`.
/// When `imported_name` is `None` the caller MUST skip the ref entirely.
fn extract_import_parts<'src>(
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
/// An `import_declaration` wraps one or more `import_spec` children.
fn extract_go_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    // import_declaration may contain import_spec children (grouped imports).
    // We handle single import_spec here — the walker recurse-stops at
    // import_declaration, so we must unwrap the block ourselves.
    let kind = node.kind();
    let spec_node = if kind == "import_spec" {
        node
    } else {
        // Find the first import_spec or interpreted_string_literal child.
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

    // Now extract the string literal from the spec (or directly if it IS the literal).
    let raw: Option<&str> = if spec_node.kind() == "interpreted_string_literal" {
        Some(strip_quotes(ctx.text(spec_node)))
    } else {
        // Look for interpreted_string_literal inside the spec.
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

    // For Go the imported "name" is the last path segment (e.g. "fmt" from "fmt").
    let target = module
        .as_deref()
        .and_then(|m| m.rsplit('/').next())
        .map(|s| s.to_string());

    (module, target)
}

/// Rust: `use_declaration` → text of `scoped_identifier` or `identifier` child.
/// Preserves `::` separators.
fn extract_rust_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    // use declarations have an `argument` field in the grammar.
    // Fallback: scan children for scoped_identifier, identifier, use_wildcard.
    let arg = node.child_by_field_name("argument");
    let path_node = arg.or_else(|| {
        // Find the first scoped_identifier, identifier, use_as_clause, or use_list.
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
    // For Rust `use`, the target is the last segment after `::`.
    let target = module.as_deref().and_then(|m| {
        // Strip trailing `::*` wildcard.
        let m = m.trim_end_matches("::*");
        m.rsplit("::").next().map(|s| s.trim_matches('{').trim_matches('}').trim().to_string())
    }).filter(|s| !s.is_empty());

    (module, target)
}

/// Python: `import_statement` → dotted_name child (module name).
/// `import_from_statement` → module_name field + dotted_name/identifier children.
fn extract_python_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    let kind = node.kind();

    if kind == "import_from_statement" {
        // `from os.path import join, exists`
        // module_name field holds the dotted module path.
        let module = node
            .child_by_field_name("module_name")
            .map(|n| ctx.text(n).trim().to_string())
            .or_else(|| {
                // Some grammar versions use dotted_name child instead.
                for i in 0..node.child_count() {
                    let child = node.child(i).unwrap();
                    if child.kind() == "dotted_name" || child.kind() == "relative_import" {
                        return Some(ctx.text(child).trim().to_string());
                    }
                }
                None
            });

        // The imported names: `name` children in the `name` field list,
        // or dotted_name / identifier children after the `import` keyword.
        // We pick the first imported symbol name.
        let target = {
            // Walk children looking for the first identifier/dotted_name that
            // comes after the `import` keyword.
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
                            // `join as j` — use the original name.
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

    // `import os` or `import os as o` — the module IS the target.
    // Walk children for dotted_name or identifier.
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        match child.kind() {
            "dotted_name" | "identifier" => {
                let t = ctx.text(child).trim().to_string();
                if !t.is_empty() {
                    // For bare `import X`, target is the top-level module name.
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
                    // The module is everything up to the last dot; target is the last segment.
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
        // Also check string children.
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
/// Returns `(module, None)` — no target derivation, caller skips ref if target is None.
/// Actually we try to derive target as last path segment.
fn extract_generic_import<'src>(
    node: Node,
    ctx: &ExtractionCtx<'src>,
) -> (Option<String>, Option<String>) {
    // Try named fields first.
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

    // Scan children for string literals and path identifiers.
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

    // Use the node's full text as a last resort — trim whitespace.
    let full = ctx.text(node).trim();
    if !full.is_empty() && full.len() <= 256 {
        // Strip leading keyword tokens.
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
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    // Handle `"..."`, `'...'`, and backtick strings.
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
// DFS walker
// ---------------------------------------------------------------------------

/// Recursively walk the CST.  `language` is threaded through for import
/// dispatch; `_parent_symbol_idx` is kept for API compatibility.
fn walk_node<'src>(node: Node<'_>, ctx: &mut ExtractionCtx<'src>, language: &str) {
    let kind = node.kind();

    // ---- Import / use / require -------------------------------------------
    if is_import_node(kind) {
        let (module, target_name_opt) = extract_import_parts(node, ctx, language);

        if let Some(target_name) = target_name_opt {
            // Only emit the ref if we have a real target name.
            ctx.refs.push(ExtractedRef {
                source_symbol_index: ctx.symbols.len().saturating_sub(1).max(0),
                target_name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
            });
        }
        // Don't recurse into imports — there's nothing useful inside.
        return;
    }

    // ---- Symbol declarations -----------------------------------------------
    let sym_kind_raw = node_kind_to_symbol_kind(kind);
    if let Some(mut sym_kind) = sym_kind_raw {
        // Go type_spec: resolve the actual kind from the body child.
        if kind == "type_spec" {
            sym_kind = go_type_spec_kind(node, ctx);
        }

        // Promote function_item/function_definition → Method when inside a
        // class/struct/impl/trait scope.  We use the scope tree to check if
        // the enclosing scope's node kind is a type-level construct.
        if sym_kind == SymbolKind::Function {
            let enclosing = scope_tree::find_enclosing_scope(
                &ctx.scope_tree, node.start_byte(), node.end_byte(),
            );
            if let Some(scope) = enclosing {
                match scope.node_kind {
                    "class_definition" | "class_declaration" | "class_specifier"
                    | "struct_item" | "impl_item" | "trait_item" | "class" | "module" => {
                        sym_kind = SymbolKind::Method;
                    }
                    _ => {}
                }
            }
        }

        if let Some(name) = extract_name(node, ctx) {
            // Use the enclosing scope (not the node's own scope) for qualified naming.
            let containing_scope = scope_tree::find_enclosing_scope(
                &ctx.scope_tree, node.start_byte(), node.end_byte(),
            );
            let qualified_name = scope_tree::qualify(&name, containing_scope);
            let sp = scope_tree::scope_path(containing_scope);

            let start = node.start_position();
            let end = node.end_position();
            let sym_idx = ctx.symbols.len();

            // Extract inheritance/implements BEFORE pushing the symbol,
            // using sym_idx as the source index (it will match after push).
            extract_inheritance_refs(node, ctx, kind, sym_idx);

            ctx.symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind: sym_kind,
                visibility: None,
                start_line: start.row as u32,
                end_line: end.row as u32,
                start_col: start.column as u32,
                end_col: end.column as u32,
                signature: None,
                doc_comment: None,
                scope_path: sp,
                parent_index: ctx.parent_symbol_index(),
            });

            // Push parent index for nested symbols, and recurse.
            let pushes_parent = matches!(
                sym_kind,
                SymbolKind::Class
                    | SymbolKind::Struct
                    | SymbolKind::Interface
                    | SymbolKind::Enum
                    | SymbolKind::Namespace
            );

            if pushes_parent {
                ctx.parent_index_stack.push(sym_idx);
                recurse_children(node, ctx, language);
                ctx.parent_index_stack.pop();
            } else {
                recurse_children(node, ctx, language);
            }
            return;
        }
    }

    // ---- Call references ---------------------------------------------------
    if is_call_node(kind) {
        if let Some(callee_name) = extract_call_target(node, ctx) {
            ctx.refs.push(ExtractedRef {
                source_symbol_index: ctx.symbols.len().saturating_sub(1),
                target_name: callee_name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
            });
        }
        recurse_children(node, ctx, language);
        return;
    }

    // ---- Type identifier references ----------------------------------------
    if kind == "type_identifier" {
        if let Some(parent) = node.parent() {
            if !is_declaration_name_position(node, parent) {
                let name = ctx.text(node).trim();
                if !name.is_empty() && name != "void" && name != "var" {
                    ctx.refs.push(ExtractedRef {
                        source_symbol_index: ctx.symbols.len().saturating_sub(1),
                        target_name: name.to_string(),
                        kind: EdgeKind::TypeRef,
                        line: node.start_position().row as u32,
                        module: None,
                    });
                }
            }
        }
        return;
    }

    // ---- Default: recurse into all children --------------------------------
    recurse_children(node, ctx, language);
}

fn recurse_children<'src>(node: Node<'_>, ctx: &mut ExtractionCtx<'src>, language: &str) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, ctx, language);
    }
}

// ---------------------------------------------------------------------------
// Call-reference helpers
// ---------------------------------------------------------------------------

/// Returns true for node kinds that represent a function or method call.
fn is_call_node(kind: &str) -> bool {
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
///
/// We try, in order:
///   1. The `function` named field (JS/TS `call_expression`)
///   2. The `method` named field (Rust `method_call_expression`, Java `method_invocation`)
///   3. The first `identifier` or `field_identifier` child
fn extract_call_target<'src>(node: Node<'_>, ctx: &ExtractionCtx<'src>) -> Option<String> {
    // Try named fields first.
    for field in &["function", "method", "name"] {
        if let Some(n) = node.child_by_field_name(field) {
            // For member expressions (`foo.bar()`), take only the last identifier.
            let name = match n.kind() {
                "member_expression" | "field_expression" | "scoped_identifier" => {
                    // Prefer the `property` / `field` sub-field, else last identifier child.
                    if let Some(prop) = n.child_by_field_name("property")
                        .or_else(|| n.child_by_field_name("field"))
                        .or_else(|| n.child_by_field_name("name"))
                    {
                        ctx.text(prop).trim().to_string()
                    } else {
                        // Walk children for the last identifier.
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

    // Fallback: first identifier child of the call node.
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
fn is_declaration_name_position(node: Node<'_>, parent: Node<'_>) -> bool {
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
///
/// Emits `EdgeKind::Inherits` for extends/superclass and `EdgeKind::Implements`
/// for implements/conformance clauses.  Uses the current symbol index as the
/// source so these refs get resolved during the cross-file resolution pass.
fn extract_inheritance_refs<'src>(node: Node<'_>, ctx: &mut ExtractionCtx<'src>, _kind: &str, source_idx: usize) {
    let line = node.start_position().row as u32;

    // Strategy: check named fields first, then walk children for clause nodes.
    // Each grammar uses different field names and child node kinds.

    // --- Named fields (grammar-specific) ---

    // Python: `superclasses` field → argument_list containing identifiers
    if let Some(sc) = node.child_by_field_name("superclasses") {
        for_each_type_child(sc, ctx, source_idx, line, EdgeKind::Inherits);
    }

    // Java: `superclass` field → superclass node wrapping a type
    if let Some(sc) = node.child_by_field_name("superclass") {
        // The superclass node wraps the actual type identifier
        for_each_type_child(sc, ctx, source_idx, line, EdgeKind::Inherits);
    }

    // Ruby: `superclass` field on class nodes
    // (same field name as Java but different structure)

    // --- Child node scanning (for clauses not exposed as named fields) ---
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        let ck = child.kind();

        match ck {
            // Java: superclass clause wrapping the extends type
            "superclass" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }

            // Java/Kotlin: super_interfaces, implements clause
            "super_interfaces" | "implements_clause" | "class_interface_clause" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Implements);
            }

            // Kotlin: delegation_specifiers
            "delegation_specifiers" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }

            // C++: base_class_clause
            "base_class_clause" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }

            // Swift/Dart: inheritance_clause, type_list
            "inheritance_clause" | "type_list" | "interfaces" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }

            // PHP: base_clause (extends)
            "base_clause" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }

            // Scala/generic: extends_clause
            "extends_clause" | "extends_type" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }

            // Java/Kotlin: extends_interfaces (for interface declarations)
            "extends_interfaces" => {
                for_each_type_child(child, ctx, source_idx, line, EdgeKind::Inherits);
            }

            _ => {}
        }
    }
}

/// Extract a type name from a node that might be an identifier, type_identifier,
/// scoped_identifier, member_expression, or similar.
fn extract_type_name<'src>(node: Node<'_>, ctx: &ExtractionCtx<'src>) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" | "simple_identifier" | "constant" => {
            let t = ctx.text(node).trim().to_string();
            if !t.is_empty() { Some(t) } else { None }
        }
        "scoped_identifier" | "scope_resolution" | "member_expression" | "qualified_type" => {
            // Full dotted/scoped name
            let t = ctx.text(node).trim().to_string();
            if !t.is_empty() && t.len() <= 128 { Some(t) } else { None }
        }
        "generic_type" | "parameterized_type" => {
            // Generic<T> — extract the base type name (first child)
            if let Some(base) = node.child(0) {
                extract_type_name(base, ctx)
            } else {
                None
            }
        }
        "type_list" => {
            // Return the first type in a type list
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                if let Some(name) = extract_type_name(child, ctx) {
                    return Some(name);
                }
            }
            None
        }
        _ => {
            // Fallback: try to find an identifier child
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
fn for_each_type_child<'src>(
    clause: Node<'_>,
    ctx: &mut ExtractionCtx<'src>,
    source_idx: usize,
    line: u32,
    edge_kind: EdgeKind,
) {
    for i in 0..clause.child_count() {
        let child = clause.child(i).unwrap();
        // Skip punctuation and keywords
        if child.is_named() {
            if let Some(name) = extract_type_name(child, ctx) {
                ctx.refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: name,
                    kind: edge_kind,
                    line,
                    module: None,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Compatibility shim: old public entry point threaded language through walk
// ---------------------------------------------------------------------------
//
// The original `walk_node` signature took `_parent_symbol_idx: Option<usize>`.
// We now thread `language: &str` instead.  The public `extract` function
// is the only caller so this is a purely internal change.

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "generic_tests.rs"]
mod tests;
