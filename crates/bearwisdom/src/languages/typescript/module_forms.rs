// =============================================================================
// typescript/module_forms.rs — the TypeScript module-surface syntax
//
// Node kinds, field names and tokens the generic lexical module capture reads
// to find a TypeScript file's imports, exports and re-exports.
// =============================================================================

use super::flow::{decode_module_literal, first_named_child};
use crate::indexer::lexical::modules::ModuleForms;

pub(crate) static TS_MODULE_FORMS: ModuleForms = ModuleForms {
    import_require: "import_require_clause",
    import_alias: "import_alias",
    assignment_token: "=",
    containers: &["module", "internal_module"],
    literal_names: &["string"],
    identifier_names: &["identifier"],
    declaration_wrappers: &["expression_statement"],
    augmentation: ("ambient_declaration", "global", "statement_block"),
    ambient_token: "declare",
    import: "import_statement",
    export: "export_statement",
    import_forms: &[
        (
            "identifier",
            crate::indexer::lexical::modules::ImportForm::Default,
        ),
        (
            "import_specifier",
            crate::indexer::lexical::modules::ImportForm::Named,
        ),
        (
            "namespace_import",
            crate::indexer::lexical::modules::ImportForm::Namespace,
        ),
    ],
    import_containers: &["import_statement", "import_clause", "named_imports"],
    declaration_lists: &[
        "lexical_declaration",
        "variable_declaration",
        "ambient_declaration",
    ],
    export_clause: "export_clause",
    export_specifier: "export_specifier",
    namespace_export: "namespace_export",
    global_alias_tokens: &["as", "namespace"],
    selections: &[
        ("member_expression", "object", "property", false),
        ("nested_type_identifier", "module", "name", true),
        ("nested_identifier", "object", "property", true),
    ],
    extensions: &[".ts", ".tsx", ".d.ts", ".js", ".jsx"],
    substitutions: &[
        (".js", &[".ts", ".tsx", ".d.ts", ".js", ".jsx"]),
        (".mjs", &[".mts", ".d.mts", ".mjs"]),
        (".cjs", &[".cts", ".d.cts", ".cjs"]),
    ],
    directory_entry: "index",
    wildcard_exclusions: &["default"],
    source_field: "source",
    type_token: "type",
    export_declaration_field: "declaration",
    export_value_field: "value",
    export_specifier_name_field: "name",
    export_specifier_alias_field: "alias",
    import_specifier_name_field: "name",
    import_specifier_alias_field: "alias",
    wildcard_token: "*",
    error_containers: &[
        "interface_body",
        "class_body",
        "object_type",
        "enum_body",
        "statement_block",
        "formal_parameters",
        "type_parameters",
        "type_arguments",
        "arguments",
        "object",
        "array",
        "template_string",
        "parenthesized_expression",
        "parenthesized_type",
    ],
    default_token: "default",
    declaration_name_field: "name",
    container_name_field: "name",
    container_body_field: "body",
    literal_kind: "string",
    decode_literal: decode_module_literal,
    first_named_child,
    default_export_name: "default",
};
