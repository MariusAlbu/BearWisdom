// =============================================================================
// kotlin/flow.rs — R5 Sprint 4 Kotlin FlowConfig
// =============================================================================

use crate::indexer::flow::FlowConfig;

pub static KOTLIN_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "kotlin",

    // Kotlin grammar node names differ across releases (tree-sitter-kotlin-ng);
    // v1 ships a minimal assignment query. Narrowing + type args stay empty
    // pending grammar verification — the resolver degrades to pre-R5 behavior
    // for those features.
    assignment_query: r#"
        (property_declaration
            (variable_declaration
                (identifier) @lhs)
            (_) @rhs)
    "#,

    type_guard_query: "",
    type_args_query: "",
};
