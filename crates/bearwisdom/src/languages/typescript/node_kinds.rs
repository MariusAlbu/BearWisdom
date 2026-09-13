// =============================================================================
// typescript/node_kinds.rs — the grammar node kinds the extractor covers
//
// One entry per node kind that declares a symbol or carries a reference; the
// coverage tests enumerate these tables.
// =============================================================================

/// Node kinds that declare a symbol.
pub(super) const SYMBOL_NODE_KINDS: &[&str] = &[
    "class_declaration",
    "abstract_class_declaration",
    "interface_declaration",
    "function_declaration",
    "generator_function_declaration",
    "method_definition",
    "abstract_method_signature",
    "method_signature",
    "public_field_definition",
    "property_signature",
    "field_definition",
    "type_alias_declaration",
    "enum_declaration",
    "lexical_declaration",
    "variable_declaration",
    "internal_module",
    "construct_signature",
    "call_signature",
    "index_signature",
];

/// Node kinds that carry a reference.
pub(super) const REF_NODE_KINDS: &[&str] = &[
    "call_expression",
    "new_expression",
    "import_statement",
    // jsx_self_closing_element and jsx_opening_element are intentionally excluded:
    // we only emit refs for PascalCase component tags (~23% of occurrences),
    // not HTML intrinsics (div, span, etc.), so the 1:1 node→ref assumption breaks.
    "extends_clause",
    "implements_clause",
    "type_annotation",
    "type_identifier",
    "as_expression",
    "satisfies_expression",
    "tagged_template_expression",
];
