// =============================================================================
// languages/erlang/extract.rs  —  Erlang symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — `module_attribute` (-module(name).)
//   Function   — `fun_decl` (name/arity, public if exported)
//   Struct     — `record_decl` (-record(name, {...}).)
//   TypeAlias  — `type_alias` (-type name() :: ...) and `opaque` (-opaque ...)
//   Method     — `callback` (-callback name(args) -> RetType.)
//   Variable   — `wild_attribute` (custom -name(value). attributes as metadata)
//
// REFERENCES:
//   Implements — `behaviour_attribute` (-behaviour(gen_server).)
//   Imports    — `import_attribute`, `pp_include`, `pp_include_lib`
//   Calls      — `call` nodes (local and remote)
//   Calls      — `internal_fun` (fun foo/2 references)
//   Calls      — `external_fun` (fun mod:foo/2 references)
//   Instantiates — `record_expr` (#record_name{...} constructions)
//
// Pass 1: collect exported function names from `export_attribute` nodes.
// Pass 2: extract symbols and refs; use export set for visibility.
// =============================================================================

use crate::types::{EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_erlang::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult::empty();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();

    // Pass 1: build export set
    let exported = collect_exports(tree.root_node(), source);

    // Pass 2: extract symbols and refs
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let mut cursor = tree.root_node().walk();
    for child in tree.root_node().children(&mut cursor) {
        match child.kind() {
            "module_attribute" => {
                extract_module(&child, source, &mut symbols);
            }
            "record_decl" => {
                extract_record(&child, source, &mut symbols);
            }
            "fun_decl" => {
                extract_function(&child, source, &exported, &mut symbols, &mut refs);
            }
            "behaviour_attribute" => {
                extract_behaviour(&child, source, symbols.len().saturating_sub(1), &mut refs);
            }
            "import_attribute" => {
                extract_import_attr(&child, source, symbols.len().saturating_sub(1), &mut refs);
            }
            "pp_include" | "pp_include_lib" => {
                extract_include(&child, source, symbols.len().saturating_sub(1), &mut refs);
            }
            "type_alias" | "opaque" => {
                extract_type_alias(&child, source, &mut symbols);
            }
            "callback" => {
                extract_callback(&child, source, &mut symbols);
            }
            "wild_attribute" => {
                extract_wild_attr(&child, source, &mut symbols);
            }
            // Spec declarations (-spec) carry type signatures, not call edges.
            // Atoms inside type signatures (e.g. `pid()`, `any()`, `binary()`)
            // look like zero-argument calls to the parser but are type
            // applications — emitting them as Calls refs produces false positives
            // that can never resolve to a function definition.
            "spec" => {}
            // Other top-level nodes (attributes, define macros, etc.) may contain
            // genuine call expressions — recurse to collect them.
            _ => {
                let sym_idx = symbols.len().saturating_sub(1);
                collect_calls(&child, source, sym_idx, &mut refs);
            }
        }
    }

    ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Pass 1: collect exported names
// ---------------------------------------------------------------------------

fn collect_exports(root: Node, src: &str) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "export_attribute" {
            // export_attribute → list of fa (fun/arity) nodes
            collect_fa_list(&child, src, &mut set);
        }
    }
    set
}

fn collect_fa_list(node: &Node, src: &str, set: &mut std::collections::HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "fa" {
            // fa has `fun` (atom) and `arity` (arity node) fields.
            let fun_name = child
                .child_by_field_name("fun")
                .map(|n| node_text(&n, src).to_string())
                .unwrap_or_default();
            let arity = child
                .child_by_field_name("arity")
                .map(|n| arity_value(&n, src).to_string())
                .unwrap_or_default();
            if !fun_name.is_empty() && !arity.is_empty() {
                set.insert(format!("{}/{}", fun_name, arity));
            }
        } else {
            collect_fa_list(&child, src, set);
        }
    }
}

// ---------------------------------------------------------------------------
// Module attribute
// ---------------------------------------------------------------------------

fn extract_module(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // -module(name).  The atom inside is the module name
    let text = node_text(node, src);
    // Extract atom from `-module(atom).`
    let name = extract_attr_value(&text, "module");
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32;
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("-module({}).", name)),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// Record
// ---------------------------------------------------------------------------

fn extract_record(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let text = node_text(node, src);
    let name = extract_attr_value(&text, "record");
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32;
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Struct,
        visibility: None,
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("-record({}, {{...}}).", name)),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
}

// ---------------------------------------------------------------------------
// Function declaration
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    exported: &std::collections::HashSet<String>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // fun_decl groups function_clause nodes
    // Get name from first function_clause → name field
    let name = get_function_name(node, src);
    if name.is_empty() {
        return;
    }

    // Compute arity from first clause argument count
    let arity = get_function_arity(node, src);
    let name_arity = format!("{}/{}", name, arity);
    let is_exported = exported.contains(&name_arity);

    let line = node.start_position().row as u32;
    let idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name_arity.clone(),
        qualified_name: name_arity.clone(),
        kind: SymbolKind::Function,
        visibility: Some(if is_exported { Visibility::Public } else { Visibility::Private }),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("{}", name_arity)),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    // Extract calls inside function body
    collect_calls(node, src, idx, refs);
}

fn get_function_name(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_clause" {
            if let Some(name_node) = child.child_by_field_name("name") {
                return node_text(&name_node, src).to_string();
            }
            // Fallback: first identifier child
            let mut c2 = child.walk();
            for n in child.children(&mut c2) {
                if n.kind() == "atom" || n.kind() == "identifier" {
                    return node_text(&n, src).to_string();
                }
            }
        }
    }
    String::new()
}

fn get_function_arity(node: &Node, src: &str) -> u32 {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_clause" {
            // Count argument nodes in `args` field
            if let Some(args) = child.child_by_field_name("args") {
                let count = args.child_count();
                // args typically wraps in parentheses; count non-punctuation children
                let non_punct = {
                    let mut c = args.walk();
                    args.children(&mut c).filter(|n| {
                        let k = n.kind();
                        k != "(" && k != ")" && k != ","
                    }).count()
                };
                return if non_punct == 0 && count == 2 { 0 } else { non_punct as u32 };
            }
            return 0;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Behaviour → Implements edge
// ---------------------------------------------------------------------------

fn extract_behaviour(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let text = node_text(node, src);
    let beh1 = extract_attr_value(&text, "behaviour");
    let behaviour = if beh1.is_empty() {
        extract_attr_value_str(&text, "behavior")
    } else {
        beh1
    };
    if behaviour.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: behaviour.clone(),
        kind: EdgeKind::Implements,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Import attribute
// ---------------------------------------------------------------------------

fn extract_import_attr(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // -import(module, [fun/1, ...]).
    // Use structured tree-sitter fields: `module` and `funs` (list of `fa` nodes).
    // Emit one Imports ref per imported function so the resolver can do exact
    // `name/arity → source_module` lookup at resolution time.
    let module_node = match node.child_by_field_name("module") {
        Some(n) => n,
        None => return,
    };
    let module_name = node_text(&module_node, src).trim_matches('\'').to_string();
    if module_name.is_empty() {
        return;
    }

    let line = node.start_position().row as u32;
    let mut cursor = node.walk();
    let mut emitted = false;
    for child in node.children(&mut cursor) {
        if child.kind() != "fa" {
            continue;
        }
        let fun_name = child
            .child_by_field_name("fun")
            .map(|n| node_text(&n, src).to_string())
            .unwrap_or_default();
        let arity_str = child
            .child_by_field_name("arity")
            .map(|n| arity_value(&n, src).to_string())
            .unwrap_or_default();
        if fun_name.is_empty() || arity_str.is_empty() {
            continue;
        }
        let target = format!("{}/{}", fun_name, arity_str);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Imports,
            line,
            module: Some(module_name.clone()),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
        });
        emitted = true;
    }

    // When the `funs` list is empty or not structured (parse error), fall back
    // to a single module-level import so the resolver can still wildcard-match.
    if !emitted {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: module_name.clone(),
            kind: EdgeKind::Imports,
            line,
            module: Some(module_name),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
        });
    }
}

// ---------------------------------------------------------------------------
// Include directives
// ---------------------------------------------------------------------------

fn extract_include(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let text = node_text(node, src);
    // -include("file.hrl"). or -include_lib("app/include/file.hrl").
    let file = if let Some(rest) = text.strip_prefix("-include_lib(") {
        rest.trim_end_matches(").").trim().trim_matches('"').to_string()
    } else if let Some(rest) = text.strip_prefix("-include(") {
        rest.trim_end_matches(").").trim().trim_matches('"').to_string()
    } else {
        return;
    };

    if !file.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: file.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: Some(file),
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// Type alias (-type / -opaque)
// ---------------------------------------------------------------------------

fn extract_type_alias(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // type_alias / opaque both have a `name` field → type_name → name field (atom)
    if let Some(type_name_node) = node.child_by_field_name("name") {
        let name = if let Some(inner) = type_name_node.child_by_field_name("name") {
            node_text(&inner, src).to_string()
        } else {
            node_text(&type_name_node, src).to_string()
        };
        if name.is_empty() {
            return;
        }
        let line = node.start_position().row as u32;
        let prefix = if node.kind() == "opaque" { "-opaque" } else { "-type" };
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name.clone(),
            kind: SymbolKind::TypeAlias,
            visibility: None,
            start_line: line,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("{}({}).", prefix, name)),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Callback (-callback)
// ---------------------------------------------------------------------------

fn extract_callback(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // callback has a `fun` field → _name (atom text)
    if let Some(fun_node) = node.child_by_field_name("fun") {
        let name = node_text(&fun_node, src).to_string();
        if name.is_empty() {
            return;
        }
        let line = node.start_position().row as u32;
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name.clone(),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: line,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("-callback {}(...).", name)),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Wild attribute (-name(value).) → Variable
// ---------------------------------------------------------------------------

fn extract_wild_attr(node: &Node, src: &str, symbols: &mut Vec<ExtractedSymbol>) {
    // wild_attribute has a `name` field → attr_name → name field (atom)
    // Skip well-known directives that are already handled by other arms or
    // that do not represent meaningful module-level metadata.
    if let Some(attr_name_node) = node.child_by_field_name("name") {
        let name = if let Some(inner) = attr_name_node.child_by_field_name("name") {
            node_text(&inner, src).to_string()
        } else {
            node_text(&attr_name_node, src).to_string()
        };
        // Skip known directives; only emit Variable for genuine custom attributes.
        const SKIP: &[&str] = &[
            "module", "export", "export_type", "import", "behaviour", "behavior",
            "record", "type", "opaque", "spec", "callback", "define",
            "include", "include_lib", "compile", "file", "on_load",
            "doc", "moduledoc", "deprecated", "feature", "vsn", "author",
        ];
        if name.is_empty() || SKIP.contains(&name.as_str()) {
            return;
        }
        let line = node.start_position().row as u32;
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name.clone(),
            kind: SymbolKind::Variable,
            visibility: None,
            start_line: line,
            end_line: node.end_position().row as u32,
            start_col: node.start_position().column as u32,
            end_col: 0,
            signature: Some(format!("-{}(...).", name)),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

// ---------------------------------------------------------------------------
// Collect call edges from a subtree
// ---------------------------------------------------------------------------

/// Attribute names that look like calls but are module-level directives.
/// `-doc "..."`, `-moduledoc "..."`, etc. (OTP 27+) get parsed such that the
/// atom `doc` / `moduledoc` can appear as a call target.  Skip them.
const ATTR_CALL_SKIP: &[&str] = &[
    "doc", "moduledoc", "feature", "deprecated", "dialyzer",
    "nifs", "on_load", "compile", "vsn", "author",
];

/// Count the number of arguments in an `expr_args` node.
///
/// `expr_args` holds a `multiple: true` `args` field whose entries are the
/// individual argument expressions. tree-sitter represents multiple-field
/// nodes as direct named children of `expr_args`; they are mixed with
/// comma/paren grammar tokens that are anonymous (not named). Count only
/// named children — each one is exactly one argument.
fn count_expr_args(expr_args: &Node) -> u32 {
    let mut c = expr_args.walk();
    expr_args
        .children(&mut c)
        .filter(|n| n.is_named())
        .count() as u32
}

/// Extract the integer text from an `arity` node.
///
/// An `arity` node in the grammar has the form `/N` where the leading slash
/// is anonymous punctuation. Its `value` field holds the integer node alone.
fn arity_value<'a>(arity_node: &Node, src: &'a str) -> &'a str {
    if let Some(v) = arity_node.child_by_field_name("value") {
        node_text(&v, src)
    } else {
        // Fallback: strip leading slash if present in the raw text.
        node_text(arity_node, src).trim_start_matches('/')
    }
}

fn collect_calls(node: &Node, src: &str, source_idx: usize, refs: &mut Vec<ExtractedRef>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                // call.expr — function expression; call.args — expr_args with arguments.
                // Always emit at least one ref per `call` node so the coverage budget
                // is satisfied. For named calls (atom or remote), emit `name/arity` as
                // the target_name so the resolver can do exact arity-aware lookup.
                let call_line = child.start_position().row as u32;
                let arg_count = child
                    .child_by_field_name("args")
                    .map(|a| count_expr_args(&a))
                    .unwrap_or(0);

                let target = if let Some(expr) = child.child_by_field_name("expr") {
                    match expr.kind() {
                        "atom" => {
                            let name = node_text(&expr, src);
                            format!("{}/{}", name, arg_count)
                        }
                        "remote" => {
                            // Module:function call.
                            if let Some(fun_node) = expr.child_by_field_name("fun") {
                                let fun_name = node_text(&fun_node, src).to_string();
                                let module = expr.child_by_field_name("module")
                                    .map(|n| node_text(&n, src).to_string());
                                if !fun_name.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: source_idx,
                                        target_name: format!("{}/{}", fun_name, arg_count),
                                        kind: EdgeKind::Calls,
                                        line: call_line,
                                        module,
                                        chain: None,
                                        byte_offset: 0,
                                        namespace_segments: Vec::new(),
                                    });
                                }
                                String::new()
                            } else {
                                node_text(&expr, src).to_string()
                            }
                        }
                        _ => node_text(&expr, src).to_string(),
                    }
                } else {
                    // No `expr` field — use first named child as fallback (no arity suffix).
                    let mut fallback = String::new();
                    for ci in 0..child.child_count() {
                        if let Some(c) = child.child(ci) {
                            if c.is_named() {
                                fallback = node_text(&c, src).to_string();
                                break;
                            }
                        }
                    }
                    fallback
                };
                if !target.is_empty() {
                    // Strip arity suffix for the ATTR_CALL_SKIP check so that
                    // `doc/0` is still recognised as the `doc` directive.
                    let bare = target.split('/').next().unwrap_or(&target);
                    if !ATTR_CALL_SKIP.contains(&bare) {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line: call_line,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                            namespace_segments: Vec::new(),
                        });
                    }
                }
                collect_calls(&child, src, source_idx, refs);
            }
            "internal_fun" => {
                // fun foo/2 — explicit arity in `arity` field; emit "name/N".
                let line = child.start_position().row as u32;
                if let Some(fun_node) = child.child_by_field_name("fun") {
                    let name = node_text(&fun_node, src).to_string();
                    let arity = child
                        .child_by_field_name("arity")
                        .map(|n| arity_value(&n, src).to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        let target = if arity.is_empty() {
                            name
                        } else {
                            format!("{}/{}", name, arity)
                        };
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                            namespace_segments: Vec::new(),
                        });
                    }
                }
            }
            "external_fun" => {
                // fun mod:foo/2 — explicit arity in `arity` field; emit "name/N".
                let line = child.start_position().row as u32;
                if let Some(fun_node) = child.child_by_field_name("fun") {
                    let fun_name = node_text(&fun_node, src).to_string();
                    let arity = child
                        .child_by_field_name("arity")
                        .map(|n| arity_value(&n, src).to_string())
                        .unwrap_or_default();
                    let module = child
                        .child_by_field_name("module")
                        .map(|n| node_text(&n, src).to_string());
                    if !fun_name.is_empty() {
                        let target = if arity.is_empty() {
                            fun_name
                        } else {
                            format!("{}/{}", fun_name, arity)
                        };
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: EdgeKind::Calls,
                            line,
                            module,
                            chain: None,
                            byte_offset: 0,
                            namespace_segments: Vec::new(),
                        });
                    }
                }
            }
            "record_expr" => {
                // #record_name{...} — record construction
                let line = child.start_position().row as u32;
                if let Some(name_node) = child.child_by_field_name("name") {
                    // record_name has a `name` field itself
                    let record_name = if let Some(inner) = name_node.child_by_field_name("name") {
                        node_text(&inner, src).to_string()
                    } else {
                        node_text(&name_node, src).to_string()
                    };
                    if !record_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: record_name,
                            kind: EdgeKind::Instantiates,
                            line,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
                collect_calls(&child, src, source_idx, refs);
            }
            _ => {
                collect_calls(&child, src, source_idx, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

/// Extract atom value from a `-name(value).` attribute text
fn extract_attr_value(text: &str, attr: &str) -> String {
    let prefix = format!("-{}(", attr);
    if let Some(rest) = text.strip_prefix(&prefix) {
        let end = rest.find(|c| c == ')' || c == ',').unwrap_or(rest.len());
        return rest[..end].trim().trim_matches('\'').to_string();
    }
    String::new()
}

fn extract_attr_value_str(text: &str, attr: &str) -> String {
    extract_attr_value(text, attr)
}
