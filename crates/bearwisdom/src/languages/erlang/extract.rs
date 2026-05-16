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
    ExtractedDbSet, ExtractedRef, ExtractedRoute, ExtractedSymbol, ExtractionResult,
};
use tree_sitter::{Node, Parser};

use super::attributes::{
    collect_exports, extract_behaviour, extract_callback, extract_import_attr,
    extract_include, extract_module, extract_record, extract_type_alias, extract_wild_attr,
};
use super::cowboy::scan_cowboy_routes;
use super::functions::{collect_calls, extract_function};

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
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

/// Extract atom value from a `-name(value).` attribute text
pub(super) fn extract_attr_value(text: &str, attr: &str) -> String {
    let prefix = format!("-{}(", attr);
    if let Some(rest) = text.strip_prefix(&prefix) {
        let end = rest.find(|c| c == ')' || c == ',').unwrap_or(rest.len());
        return rest[..end].trim().trim_matches('\'').to_string();
    }
    String::new()
}

pub(super) fn extract_attr_value_str(text: &str, attr: &str) -> String {
    extract_attr_value(text, attr)
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;
