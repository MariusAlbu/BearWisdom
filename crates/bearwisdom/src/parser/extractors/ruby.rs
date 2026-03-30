// =============================================================================
// parser/extractors/ruby.rs  —  Ruby symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class, Module, Method, Constructor (`initialize`), Property
//   (`attr_reader`/`attr_writer`/`attr_accessor` calls at class level)
//
// REFERENCES:
//   - `require` / `require_relative`   → Imports edges
//   - Method calls (`obj.method`)      → Calls edges
//   - `ClassName.new`                  → Instantiates edges
//   - Superclass after `<`             → Inherits edges
//   - Rails macros (`has_many`,
//     `belongs_to`, `has_one`, etc.)   → TypeRef edges
//
// Approach:
//   Single-pass recursive CST walk.  A `qualified_prefix` string and
//   `inside_class` flag are threaded through recursion, matching the
//   pattern of the Python extractor.  The scope_tree module is used to
//   build qualified names but the recursive prefix approach is sufficient
//   for Ruby's simpler nesting model (class/module only).
//
// Visibility convention:
//   Methods declared after `private` / `protected` keyword nodes are
//   not tracked at the AST level by tree-sitter-ruby in a way that is
//   easy to detect without a two-pass scan, so all methods default to
//   Public.  Methods starting with `_` are marked Private as a
//   best-effort heuristic.
// =============================================================================

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

static RUBY_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class",            name_field: "name" },
    ScopeKind { node_kind: "module",           name_field: "name" },
    ScopeKind { node_kind: "method",           name_field: "name" },
    ScopeKind { node_kind: "singleton_method", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Rails and ActiveRecord macros that produce TypeRef edges
// ---------------------------------------------------------------------------

static RAILS_ASSOC_MACROS: &[&str] =
    &["has_many", "belongs_to", "has_one", "has_and_belongs_to_many", "through"];

// attr_* macros that produce Property symbols
static ATTR_MACROS: &[&str] = &["attr_reader", "attr_writer", "attr_accessor"];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from Ruby source code.
pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_ruby::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load Ruby grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let src = source.as_bytes();
    let root = tree.root_node();

    // Build scope tree for qualified-name lookups (used in scope_path).
    let _scope_tree = scope_tree::build(root, src, RUBY_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_from_node(root, src, &mut symbols, &mut refs, None, "", false);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" => {
                extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "module" => {
                extract_module(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "method" => {
                extract_method(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }

            "singleton_method" => {
                extract_singleton_method(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            // `require 'foo'` and `require_relative './foo'`
            "call" => {
                extract_call_statement(
                    &child,
                    src,
                    refs,
                    symbols.len(),
                    parent_index,
                    inside_class,
                    symbols,
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Class
// ---------------------------------------------------------------------------

fn extract_class(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let signature = {
        // First line of the class definition (e.g. `class Foo < Bar`)
        let raw = node_text(node, src);
        raw.lines().next().map(|l| l.trim().to_string())
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Inheritance: `class Foo < Bar` — tree-sitter-ruby uses `superclass` field.
    // The `superclass` node contains `<` + `constant` children; we want the constant name.
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
        // Try to find the `constant` child (the actual class name).
        let super_name = {
            let mut found: Option<String> = None;
            let mut sc = superclass_node.walk();
            for c in superclass_node.children(&mut sc) {
                if c.kind() == "constant" || c.kind() == "scope_resolution" {
                    found = Some(node_text(&c, src));
                    break;
                }
            }
            found.unwrap_or_else(|| {
                // Fallback: strip the leading `< ` from the raw text.
                let raw = node_text(&superclass_node, src);
                raw.trim_start_matches('<').trim().to_string()
            })
        };
        refs.push(ExtractedRef {
            source_symbol_index: idx,
            target_name: super_name,
            kind: EdgeKind::Inherits,
            line: superclass_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }

    // Recurse into body
    if let Some(body) = node.child_by_field_name("body") {
        extract_from_node(body, src, symbols, refs, Some(idx), &new_prefix, true);
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

fn extract_module(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        // Ruby modules map to the Interface kind (closest semantic match —
        // they define a contract / namespace, no state).
        kind: SymbolKind::Interface,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("module {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        // Modules are not classes (no instances), but methods inside them are
        // extracted as methods for consistency.
        extract_from_node(body, src, symbols, refs, Some(idx), &new_prefix, false);
    }
}

// ---------------------------------------------------------------------------
// Method
// ---------------------------------------------------------------------------

fn extract_method(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = ruby_visibility(&name);

    let kind = if name == "initialize" {
        SymbolKind::Constructor
    } else if inside_class {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let signature = build_method_signature(node, src, &name);

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
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract call refs from the method body
    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Singleton method (`def self.foo` / `def ClassName.foo`)
// ---------------------------------------------------------------------------

fn extract_singleton_method(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let signature = build_method_signature(node, src, &name);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Call statements: require, require_relative, attr_*, Rails macros
// ---------------------------------------------------------------------------

/// Handle top-level or class-body call nodes.
///
/// - `require`/`require_relative` → Imports edge
/// - `attr_reader`/`attr_writer`/`attr_accessor` → Property symbols
/// - Rails association macros → TypeRef edges
/// - All other call expressions → Calls edges (if we have a containing symbol)
fn extract_call_statement(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    parent_index: Option<usize>,
    inside_class: bool,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    // The method name is in the `method` field for receiver calls, or the
    // entire `call` node's first `identifier` child for bare calls.
    let method_name = get_call_method_name(node, src);

    match method_name.as_deref() {
        Some("require") | Some("require_relative") => {
            extract_require(node, src, refs, current_symbol_count, method_name.as_deref());
        }

        Some(m) if ATTR_MACROS.contains(&m) => {
            if inside_class {
                extract_attr_macro(node, src, symbols, parent_index, m);
            }
        }

        Some(m) if RAILS_ASSOC_MACROS.contains(&m) => {
            // The first argument is the association name (a symbol literal).
            if let Some(args) = node.child_by_field_name("arguments") {
                let mut c = args.walk();
                for arg in args.children(&mut c) {
                    if arg.kind() == "simple_symbol" || arg.kind() == "symbol" {
                        let raw = node_text(&arg, src);
                        let assoc_name = raw.trim_start_matches(':').to_string();
                        refs.push(ExtractedRef {
                            source_symbol_index: current_symbol_count.saturating_sub(1),
                            target_name: assoc_name,
                            kind: EdgeKind::TypeRef,
                            line: arg.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                        break; // only first arg is the model name
                    }
                }
            }
        }

        _ => {
            // General call expression — emit a Calls edge if inside a method
            // (indicated by parent_index being set).
            if let Some(pidx) = parent_index {
                if let Some(recv) = node.child_by_field_name("receiver") {
                    // `obj.method(...)` — check for `ClassName.new`
                    let recv_text = node_text(&recv, src);
                    if let Some(mname) = method_name.as_deref() {
                        if mname == "new" {
                            refs.push(ExtractedRef {
                                source_symbol_index: pidx,
                                target_name: recv_text,
                                kind: EdgeKind::Instantiates,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        } else {
                            refs.push(ExtractedRef {
                                source_symbol_index: pidx,
                                target_name: mname.to_string(),
                                kind: EdgeKind::Calls,
                                line: node.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                } else if let Some(mname) = method_name.as_deref() {
                    refs.push(ExtractedRef {
                        source_symbol_index: pidx,
                        target_name: mname.to_string(),
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
    }
}

fn extract_require(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    method_name: Option<&str>,
) {
    let is_relative = method_name == Some("require_relative");
    if let Some(args) = node.child_by_field_name("arguments") {
        let mut c = args.walk();
        for arg in args.children(&mut c) {
            let kind = arg.kind();
            if kind == "string" || kind == "string_content" {
                let raw = node_text(&arg, src);
                // Strip surrounding quotes from string literals
                let path = raw
                    .trim_start_matches('"')
                    .trim_end_matches('"')
                    .trim_start_matches('\'')
                    .trim_end_matches('\'')
                    .to_string();
                // For require, the module is the path; for require_relative prefix it.
                let (target, module) = if is_relative {
                    let stem = path.rsplit('/').next().unwrap_or(&path);
                    (stem.to_string(), Some(path))
                } else {
                    let parts: Vec<&str> = path.split('/').collect();
                    let target = parts.last().unwrap_or(&path.as_str()).to_string();
                    let module = if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("/"))
                    } else {
                        None
                    };
                    (target, module)
                };
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: arg.start_position().row as u32,
                    module,
                    chain: None,
                });
                break;
            }
        }
    }
}

fn extract_attr_macro(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    _macro_name: &str,
) {
    let Some(args) = node.child_by_field_name("arguments") else { return };
    let mut c = args.walk();
    for arg in args.children(&mut c) {
        let kind = arg.kind();
        if kind == "simple_symbol" || kind == "symbol" {
            let raw = node_text(&arg, src);
            let name = raw.trim_start_matches(':').to_string();
            let qualified_name = name.clone(); // attr macros are always class-level
            symbols.push(ExtractedSymbol {
                name,
                qualified_name,
                kind: SymbolKind::Property,
                visibility: Some(Visibility::Public),
                start_line: arg.start_position().row as u32,
                end_line: arg.end_position().row as u32,
                start_col: arg.start_position().column as u32,
                end_col: arg.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Call extraction from a method body
// ---------------------------------------------------------------------------

fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(mname) = get_call_method_name(&child, src) {
                if mname == "new" {
                    // Emit Instantiates for `ClassName.new`
                    if let Some(recv) = child.child_by_field_name("receiver") {
                        let recv_text = node_text(&recv, src);
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: recv_text,
                            kind: EdgeKind::Instantiates,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                        // Don't also emit a Calls edge for `.new`.
                        extract_calls_from_body(&child, src, source_symbol_index, refs);
                        continue;
                    }
                }

                let chain = build_chain(&child, src);
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: mname,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain,
                });
            }
        }
        extract_calls_from_body(&child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured member access chain from a Ruby CST `call` node.
///
/// Ruby uses `call` with a `receiver` field and `method` field:
///
/// ```text
/// call
///   identifier / call @receiver
///   identifier @method "find_one"
///   argument_list
/// ```
/// produces: `[receiver, find_one]`
///
/// For `self.method_name`:
/// ```text
/// call
///   self @receiver
///   identifier @method "method_name"
/// ```
/// produces: `[self, method_name]`
fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "self" => {
            segments.push(ChainSegment {
                name: "self".to_string(),
                node_kind: "self".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" | "constant" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                optional_chaining: false,
            });
            Some(())
        }

        "call" => {
            // `receiver.method(...)` — recurse into receiver, then push method.
            if let Some(receiver) = node.child_by_field_name("receiver") {
                build_chain_inner(&receiver, src, segments)?;
                if let Some(method) = node.child_by_field_name("method") {
                    segments.push(ChainSegment {
                        name: node_text(&method, src),
                        node_kind: "call".to_string(),
                        kind: SegmentKind::Property,
                        declared_type: None,
                        optional_chaining: false,
                    });
                }
                Some(())
            } else {
                // Bare call (no receiver) — treat the method name as Identifier.
                if let Some(method) = node.child_by_field_name("method") {
                    segments.push(ChainSegment {
                        name: node_text(&method, src),
                        node_kind: "call".to_string(),
                        kind: SegmentKind::Identifier,
                        declared_type: None,
                        optional_chaining: false,
                    });
                    Some(())
                } else {
                    None
                }
            }
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: &Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}::{name}")
    }
}

fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() { None } else { Some(prefix.to_string()) }
}

fn ruby_visibility(name: &str) -> Option<Visibility> {
    if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    }
}

/// Extract the bare method name from a `call` node.
///
/// - `foo(...)` — first `identifier` child
/// - `obj.foo(...)` — `method` field
fn get_call_method_name(node: &Node, src: &[u8]) -> Option<String> {
    // Named field `method` is set for receiver calls: `obj.method`.
    if let Some(m) = node.child_by_field_name("method") {
        return Some(node_text(&m, src));
    }
    // Bare call: `foo(...)` — walk children for the leading identifier.
    let mut c = node.walk();
    for child in node.children(&mut c) {
        if child.kind() == "identifier" {
            return Some(node_text(&child, src));
        }
    }
    None
}

fn build_method_signature(node: &Node, src: &[u8], name: &str) -> Option<String> {
    // Parameters field contains the parenthesised param list.
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(&p, src))
        .unwrap_or_default();
    Some(format!("def {name}{params}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "ruby_tests.rs"]
mod tests;
