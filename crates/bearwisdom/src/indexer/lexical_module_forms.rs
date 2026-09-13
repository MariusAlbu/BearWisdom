// =============================================================================
// indexer/lexical_module_forms — the module-surface syntax a language declares
//
// Node kinds, field names and tokens the generic module capture reads to find
// a file's imports, exports and re-exports. Data only; the capture that reads
// it lives beside it.
// =============================================================================

use tree_sitter::Node;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ImportForm {
    Default,
    Named,
    Namespace,
}
#[derive(Debug)]
pub(crate) struct ModuleForms {
    pub import_require: &'static str,
    pub import_alias: &'static str,
    pub assignment_token: &'static str,
    pub containers: &'static [&'static str],
    pub literal_names: &'static [&'static str],
    pub identifier_names: &'static [&'static str],
    pub declaration_wrappers: &'static [&'static str],
    pub augmentation: (&'static str, &'static str, &'static str),
    pub ambient_token: &'static str,
    pub import: &'static str,
    pub export: &'static str,
    pub import_forms: &'static [(&'static str, ImportForm)],
    pub import_containers: &'static [&'static str],
    pub declaration_lists: &'static [&'static str],
    pub export_clause: &'static str,
    pub export_specifier: &'static str,
    pub namespace_export: &'static str,
    /// Anonymous tokens of the export statement that publishes the module's
    /// export assignment under a global name for script consumers; it adds
    /// nothing to the module's own surface.
    pub global_alias_tokens: &'static [&'static str],
    pub selections: &'static [(&'static str, &'static str, &'static str, bool)],
    pub extensions: &'static [&'static str],
    pub substitutions: &'static [(&'static str, &'static [&'static str])],
    pub directory_entry: &'static str,
    pub wildcard_exclusions: &'static [&'static str],
    pub source_field: &'static str,
    pub type_token: &'static str,
    pub export_declaration_field: &'static str,
    pub export_value_field: &'static str,
    pub export_specifier_name_field: &'static str,
    pub export_specifier_alias_field: &'static str,
    pub import_specifier_name_field: &'static str,
    pub import_specifier_alias_field: &'static str,
    pub wildcard_token: &'static str,
    /// Node kinds whose interior is a declaration's own body or signature
    /// (`interface_body`, `statement_block`, `formal_parameters`, …). A
    /// parse error confined inside one of them changes what that
    /// declaration MEANS, not which names the module declares or forwards,
    /// so it leaves the module's export surface complete.
    pub error_containers: &'static [&'static str],
    pub default_token: &'static str,
    pub declaration_name_field: &'static str,
    pub container_name_field: &'static str,
    pub container_body_field: &'static str,
    pub literal_kind: &'static str,
    pub decode_literal: fn(&str) -> Option<String>,
    pub first_named_child: for<'a> fn(Node<'a>) -> Option<Node<'a>>,
    pub default_export_name: &'static str,
}
