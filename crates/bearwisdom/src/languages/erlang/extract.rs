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

use crate::types::{
    EdgeKind, ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol, ExtractionResult,
    SymbolKind, Visibility,
};
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

    // Pass 3: Cowboy router routes. `cowboy_router:compile([{Host, [{Path, Handler, _}]}])`
    // declares HTTP routes; walk the nested-list structure to emit one
    // ExtractedRoute per `{Path, Handler, ...}` triple.
    let mut routes: Vec<ExtractedRoute> = Vec::new();
    scan_cowboy_routes(tree.root_node(), source, &symbols, &mut routes);

    ExtractionResult::with_connectors(symbols, refs, routes, Vec::<ExtractedDbSet>::new(), has_errors)
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
            call_args: Vec::new(),
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
            call_args: Vec::new(),
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
            call_args: Vec::new(),
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
                    call_args: Vec::new(),
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
                                        call_args: Vec::new(),
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
                            call_args: Vec::new(),
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
                            call_args: Vec::new(),
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
                            call_args: Vec::new(),
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
                                                    call_args: Vec::new(),
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

// ---------------------------------------------------------------------------
// Cowboy route extraction
//
// Erlang Cowboy declares HTTP routes through a single setup call:
//
//   Dispatch = cowboy_router:compile([
//       {'_', [
//           {"/users", users_handler, []},
//           {"/users/:id", user_handler, []}
//       ]}
//   ]).
//
// Each inner triple `{Path, HandlerModule, InitArgs}` is one route. Cowboy
// dispatches all HTTP methods to the handler's `init/2` callback, so we
// record `http_method = ""` (Any) and let the pair matcher key on the URL.
// ---------------------------------------------------------------------------

pub(crate) fn scan_cowboy_routes(
    root: Node,
    src: &str,
    symbols: &[ExtractedSymbol],
    routes: &mut Vec<ExtractedRoute>,
) {
    // Two-track detection. The AST walker is preferred because it lets us
    // attribute the route to the right source line; if the grammar shape
    // changes and the walker misses a call, the text-fallback below
    // catches the routes anyway. Tracks `seen_at` byte offsets so the same
    // dispatch table isn't recorded twice when both tracks fire.
    let count_before = routes.len();
    visit_for_cowboy(&root, src, symbols, routes);
    if routes.len() > count_before {
        return;
    }
    // Text fallback — `cowboy_router:compile(...)` literal lookup.
    let needle = "cowboy_router:compile(";
    let mut start = 0usize;
    while let Some(rel) = src[start..].find(needle) {
        let pos = start + rel + needle.len();
        // pos points just after the `(`. Find the matching `)` to bound the
        // argument text, then parse triples inside it.
        if let Some(end) = find_matching_paren(src, pos - 1) {
            let inner = &src[pos..end];
            extract_cowboy_triples_from_text(inner, routes, symbols, 1);
            start = end + 1;
        } else {
            start = pos;
        }
    }
}

fn find_matching_paren(text: &str, open_idx: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if open_idx >= bytes.len() || bytes[open_idx] != b'(' {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'"' => {
                i = skip_string_literal(text, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn visit_for_cowboy(
    node: &Node,
    src: &str,
    symbols: &[ExtractedSymbol],
    routes: &mut Vec<ExtractedRoute>,
) {
    if node.kind() == "call" {
        if is_cowboy_compile_call(node, src) {
            // Capture from the args field. Walk the source text inside the
            // outermost square-bracketed list and extract triples.
            if let Some(args_node) = node.child_by_field_name("args") {
                let args_text = node_text(&args_node, src);
                extract_cowboy_triples_from_text(args_text, routes, symbols, node.start_position().row as u32 + 1);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_for_cowboy(&child, src, symbols, routes);
    }
}

fn is_cowboy_compile_call(node: &Node, src: &str) -> bool {
    let Some(expr) = node.child_by_field_name("expr") else { return false };
    if expr.kind() != "remote" {
        return false;
    }
    let module = expr
        .child_by_field_name("module")
        .map(|n| node_text(&n, src))
        .unwrap_or("");
    let fun = expr
        .child_by_field_name("fun")
        .map(|n| node_text(&n, src))
        .unwrap_or("");
    module == "cowboy_router" && fun == "compile"
}

/// Parse `[{Host, [{Path, Handler, _}, ...]}, ...]` from raw source text.
/// We brace-match `{...}` tuples and recognise `{Path, Handler, ...}` shape:
/// first child a `"..."` string starting with `/`, second child an atom.
pub(crate) fn extract_cowboy_triples_from_text(
    text: &str,
    routes: &mut Vec<ExtractedRoute>,
    symbols: &[ExtractedSymbol],
    fallback_line: u32,
) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Find matching closing brace, honoring string literals.
            let end = match find_matching_brace(text, i) {
                Some(e) => e,
                None => break,
            };
            let inner = &text[i + 1..end];
            if let Some(route) = parse_cowboy_triple(inner, fallback_line, symbols) {
                routes.push(route);
            } else {
                // Nested tuples — recurse into inner.
                extract_cowboy_triples_from_text(inner, routes, symbols, fallback_line);
            }
            i = end + 1;
        } else if bytes[i] == b'"' {
            // Skip over string literal contents.
            i = skip_string_literal(text, i);
        } else {
            i += 1;
        }
    }
}

/// Given the inside of `{...}`, return Some(route) if it parses as a
/// Cowboy route triple `{Path, HandlerAtom, _}`. Otherwise None.
fn parse_cowboy_triple(
    inner: &str,
    fallback_line: u32,
    symbols: &[ExtractedSymbol],
) -> Option<ExtractedRoute> {
    let parts = split_top_level_commas(inner);
    if parts.len() < 2 {
        return None;
    }
    let first = parts[0].trim();
    let second = parts[1].trim();
    // First arg must be a string literal starting with `"/`.
    let path = if first.starts_with('"') && first.ends_with('"') && first.len() >= 2 {
        let raw = &first[1..first.len() - 1];
        if !raw.starts_with('/') {
            return None;
        }
        normalize_cowboy_path(raw)
    } else {
        return None;
    };
    // Second arg must be an atom (lowercase identifier).
    let is_atom = second
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_lowercase() || c == '_')
        && second
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@');
    if !is_atom {
        return None;
    }
    let handler_symbol_index = symbols
        .iter()
        .position(|s| s.start_line == fallback_line)
        .unwrap_or(0);
    Some(ExtractedRoute {
        handler_symbol_index,
        http_method: String::new(),
        template: path,
    })
}

/// Convert Cowboy's `:name` bindings to `{name}` so the URL normalizer
/// aligns the path with Producer-side `/users/{id}` strings.
fn normalize_cowboy_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' {
            // Bind variable — consume identifier chars.
            let mut name = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_ascii_alphanumeric() || nc == '_' {
                    name.push(nc);
                    chars.next();
                } else {
                    break;
                }
            }
            if name.is_empty() {
                out.push(':');
            } else {
                out.push('{');
                out.push_str(&name);
                out.push('}');
            }
        } else if c == '[' {
            // Drop optional-segment brackets — `/users[/:id]` → `/users/:id`.
            // Cowboy uses `[]` for optional path segments.
        } else if c == ']' {
            // Same — drop.
        } else {
            out.push(c);
        }
    }
    out
}

/// Find the byte index of the `}` that closes the `{` at `open_idx`. Honors
/// nested braces and skips string-literal contents.
fn find_matching_brace(text: &str, open_idx: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    debug_assert_eq!(bytes[open_idx], b'{');
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'"' => {
                i = skip_string_literal(text, i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_string_literal(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    debug_assert_eq!(bytes[start], b'"');
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Split a comma-separated argument list while respecting nested
/// `{}`, `[]`, `()`, and `""`.
fn split_top_level_commas(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth_brace = 0i32;
    let mut depth_bracket = 0i32;
    let mut depth_paren = 0i32;
    let mut in_string = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            current.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                current.push(c);
            }
            '{' => {
                depth_brace += 1;
                current.push(c);
            }
            '}' => {
                depth_brace -= 1;
                current.push(c);
            }
            '[' => {
                depth_bracket += 1;
                current.push(c);
            }
            ']' => {
                depth_bracket -= 1;
                current.push(c);
            }
            '(' => {
                depth_paren += 1;
                current.push(c);
            }
            ')' => {
                depth_paren -= 1;
                current.push(c);
            }
            ',' if depth_brace == 0 && depth_bracket == 0 && depth_paren == 0 => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;
