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
use crate::languages::typescript::flow::{
    TS_DISCRIMINANT_GUARD_QUERY, TS_LITERAL_TYPE_KINDS, TS_TYPE_GUARD_QUERY,
};

/// JavaScript flow-typing queries. Singleton — registered on the plugin via
/// `LanguagePlugin::flow_config()`.
///
/// The `"js"` strategy prefix routes the shared runner to `TS_CFG_KINDS`
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
