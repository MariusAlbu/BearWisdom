//! Rust syntax data for the shared detached-body owner binder.
use crate::indexer::lexical::detached::{capture, Forms};
use crate::types::{AliasTarget, ExtractedRef, ExtractedSymbol};

pub(crate) const FORMS: Forms = Forms {
    scopes: &[
        "mod_item",
        "function_item",
        "block",
        "impl_item",
        "trait_item",
        "struct_item",
        "enum_item",
    ],
    isolated_scopes: &["mod_item"],
    opaque_bindings: &["extern_crate_declaration", "ERROR"],
    excluded_fields: &["trait"],
    imports: Some(&IMPORTS),
    name_prefixes: &["r#"],
    declarations: &[
        ("struct_item", true),
        ("enum_item", true),
        ("union_item", true),
        ("trait_item", false),
        ("type_item", false),
        ("type_parameter", false),
        ("mod_item", false),
    ],
    extension: "impl_item",
    target: "type",
    wrappers: &[("generic_type", "type")],
    identifiers: &["type_identifier"],
    members: &["function_item", "const_item", "type_item"],
};

pub(super) const IMPORTS: crate::indexer::lexical::import_names::Forms =
    crate::indexer::lexical::import_names::Forms {
        statements: &[("use_declaration", "argument")],
        paths: &[("scoped_identifier", "name", "path")],
        groups: &[("scoped_use_list", "path", "list")],
        lists: &["use_list"],
        renames: &[("use_as_clause", "alias")],
        identifiers: &["identifier"],
        self_leaf: "self",
        wildcards: &["use_wildcard"],
        discarded: &["_"],
        trivia: &["line_comment", "block_comment"],
    };

pub(super) fn extract_bound(
    root: tree_sitter::Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    aliases: &mut Vec<(String, AliasTarget)>,
) {
    super::extract::extract_from_node(root, source, symbols, refs, None, "", aliases);
    capture(root, source.as_bytes(), &FORMS, symbols);
    crate::indexer::namespaces::traits::classify_methods(symbols);
}

#[cfg(test)]
#[path = "owners_tests.rs"]
mod tests;
