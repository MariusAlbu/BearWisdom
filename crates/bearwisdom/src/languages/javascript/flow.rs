// =============================================================================
// javascript/flow.rs — JavaScript FlowConfig
//
// The structural subset of the TypeScript flow configuration: forward
// inference from initializers, object-destructure seeding, await flagging,
// conditional narrowing, and literal wrapper typing. JavaScript has no type
// syntax, so the annotation-dependent parts of the TS config — declared-type
// capture (`type_annotation`), annotated parameters (`required_parameter` /
// `optional_parameter`), and call-site type arguments (`type_arguments`) —
// are absent: those node kinds do not exist in the JavaScript grammar and a
// query naming them fails to compile against it.
// =============================================================================

use crate::indexer::flow::FlowConfig;
use crate::indexer::flow_assignments::DestructureShape;
use crate::languages::typescript::flow::{
    TS_DISCRIMINANT_GUARD_QUERY, TS_LITERAL_TYPE_KINDS, TS_TYPE_GUARD_QUERY,
};

/// JavaScript flow-typing queries. Singleton — registered on the plugin via
/// `LanguagePlugin::flow_config()`.
///
/// The `"js"` strategy prefix routes the shared runner to `JS_CFG_KINDS`
/// (CFG-native narrowing lookup) and `TS_RETURN_QUERY` (body-based return-type
/// inference): the TypeScript grammar is a superset of the JavaScript grammar,
/// so those structural node kinds and queries are valid for both. The guard
/// queries and the literal-kind table are shared the same way.
pub static JS_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "js",

    // Matches `let/const/var x = <expr>`, `x = <expr>` reassignment, and
    // object/array-destructure declarations (`const { a, b: c } = f()`;
    // `const [first, second] = f()`). The
    // TypeScript query's annotation arms (the optional `type:` capture and
    // the parameter patterns) are omitted — JavaScript bindings carry no
    // declared type, so every binding types from its initializer.
    assignment_query: r#"
        (variable_declarator
            name: (identifier) @lhs
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)

        (variable_declarator
            name: (object_pattern
                [(shorthand_property_identifier_pattern) @destruct.bind
                 (pair_pattern
                    key: (property_identifier) @destruct.key
                    value: (identifier) @destruct.bind)])
            value: (_) @rhs)

        (variable_declarator
            name: (array_pattern
                (identifier) @destruct.bind)
            value: (_) @rhs)

        (formal_parameters
            (identifier) @lhs.param)
    "#,

    type_guard_query: TS_TYPE_GUARD_QUERY,

    discriminant_guard_query: TS_DISCRIMINANT_GUARD_QUERY,

    // JavaScript has no call-site type-argument syntax; empty opts out.
    type_args_query: "",

    literal_type_kinds: TS_LITERAL_TYPE_KINDS,
};

pub const JS_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
    crate::indexer::flow_cfg::CfgNodeKinds {
        function_kinds: &[
            "function_declaration",
            "function_expression",
            "method_definition",
            "arrow_function",
        ],
        block_kinds: &["statement_block"],
        if_kind: "if_statement",
        if_consequence_field: "consequence",
        if_consequence_body: None,
        if_alternative_field: "alternative",
        if_alternative_body: Some(crate::languages::typescript::flow::first_named_child),
        if_condition_field: "condition",
        assignment_kind: "assignment_expression",
        assignment_lhs_field: "left",
        declarator_kind: "variable_declarator",
        declarator_name_field: "name",
        binding_name_kinds: &["identifier"],
        definition_name_kinds: &["identifier"],
        bare_return_name_kinds: &["identifier"],
        function_name_fields: &["name"],
        loop_kinds: &[
            "while_statement",
            "for_statement",
            "for_in_statement",
            "do_statement",
        ],
        loop_body_field: "body",
        loop_condition_field: Some("condition"),
        switch_kinds: &["switch_statement"],
        switch_value_field: "value",
        switch_body_field: Some("body"),
        switch_case_kinds: &["switch_case"],
        switch_default_kinds: &["switch_default"],
        transparent_kinds: &[],
        implicit_return_candidate: None,
        condition_true_guard: Some(crate::languages::typescript::flow::condition_true_guard),
    };

pub(crate) fn return_object_members(node: tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    if node.kind() != "object" {
        return None;
    }
    let mut members = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let name = match child.kind() {
            "shorthand_property_identifier" | "property_identifier" => child.utf8_text(source).ok(),
            "pair" => child
                .child_by_field_name("key")
                .filter(|key| matches!(key.kind(), "property_identifier" | "identifier"))
                .and_then(|key| key.utf8_text(source).ok()),
            "method_definition" => child
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok()),
            _ => None,
        };
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            members.push(name.to_owned());
        }
    }
    Some(members)
}

pub(crate) fn destructure_shape(binding: tree_sitter::Node) -> DestructureShape {
    let Some(pattern) = binding.parent() else {
        return DestructureShape::NotPositional;
    };
    if pattern.kind() != "array_pattern" {
        let mut ancestor = Some(pattern);
        while let Some(parent) = ancestor {
            if parent.kind() == "array_pattern" {
                return DestructureShape::Unsupported;
            }
            ancestor = parent.parent();
        }
        return DestructureShape::NotPositional;
    }
    let mut cursor = pattern.walk();
    let direct = pattern.named_children(&mut cursor).collect::<Vec<_>>();
    if direct.iter().any(|child| child.kind() != "identifier") {
        return DestructureShape::Unsupported;
    }
    let mut ancestor = pattern.parent();
    while let Some(parent) = ancestor {
        if parent.kind() == "array_pattern" {
            return DestructureShape::Unsupported;
        }
        ancestor = parent.parent();
    }
    let mut separators = 0;
    for index in 0..pattern.child_count() {
        let Some(child) = pattern.child(index) else {
            return DestructureShape::Unsupported;
        };
        if child.id() == binding.id() {
            return DestructureShape::Slot(separators);
        }
        if child.kind() == "," {
            separators += 1;
        }
    }
    DestructureShape::Unsupported
}

pub(crate) fn is_await_rhs(node: tree_sitter::Node) -> bool {
    node.kind() == "await_expression"
}
