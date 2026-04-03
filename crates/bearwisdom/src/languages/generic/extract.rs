// =============================================================================
// parser/extractors/generic/mod.rs  —  universal grammar-based extractor
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



use super::helpers;
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

pub(super) struct ExtractionCtx<'src> {
    pub(super) src: &'src [u8],
    pub(super) symbols: Vec<ExtractedSymbol>,
    pub(super) refs: Vec<ExtractedRef>,
    /// Pre-built scope tree for qualified-name lookups.
    pub(super) scope_tree: ScopeTree,
    /// Stack of symbol indices for tracking parent_index on nested symbols.
    pub(super) parent_index_stack: Vec<usize>,
}

impl<'src> ExtractionCtx<'src> {
    /// The symbol index of the immediately enclosing scope, if any.
    pub(super) fn parent_symbol_index(&self) -> Option<usize> {
        self.parent_index_stack.last().copied()
    }

    /// Text of a node.
    pub(super) fn text(&self, node: Node) -> &str {
        node.utf8_text(self.src).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Node kind → SymbolKind mapping
// ---------------------------------------------------------------------------

/// Returns the SymbolKind for a node kind string, or None if the node kind
/// is not a declaration we want to extract.
pub(super) fn node_kind_to_symbol_kind(kind: &str) -> Option<SymbolKind> {
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
        // go_type_spec_kind().
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
// DFS walker
// ---------------------------------------------------------------------------

/// Recursively walk the CST.  `language` is threaded through for import dispatch.
fn walk_node<'src>(node: Node<'_>, ctx: &mut ExtractionCtx<'src>, language: &str) {
    let kind = node.kind();

    // ---- Import / use / require -------------------------------------------
    if is_import_node(kind) {
        let (module, target_name_opt) = helpers::extract_import_parts(node, ctx, language);

        if let Some(target_name) = target_name_opt {
            // Only emit the ref if we have a real target name.
            ctx.refs.push(ExtractedRef {
                source_symbol_index: ctx.symbols.len().saturating_sub(1).max(0),
                target_name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
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
            helpers::extract_inheritance_refs(node, ctx, kind, sym_idx);

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
    if helpers::is_call_node(kind) {
        if let Some(callee_name) = helpers::extract_call_target(node, ctx) {
            ctx.refs.push(ExtractedRef {
                source_symbol_index: ctx.symbols.len().saturating_sub(1),
                target_name: callee_name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        recurse_children(node, ctx, language);
        return;
    }

    // ---- Type identifier references ----------------------------------------
    if kind == "type_identifier" {
        if let Some(parent) = node.parent() {
            if !helpers::is_declaration_name_position(node, parent) {
                let name = ctx.text(node).trim();
                if !name.is_empty() && name != "void" && name != "var" {
                    ctx.refs.push(ExtractedRef {
                        source_symbol_index: ctx.symbols.len().saturating_sub(1),
                        target_name: name.to_string(),
                        kind: EdgeKind::TypeRef,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
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
// Tests
// ---------------------------------------------------------------------------

