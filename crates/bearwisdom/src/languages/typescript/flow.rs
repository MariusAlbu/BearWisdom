// =============================================================================
// typescript/flow.rs — R5 Sprint 2 TypeScript FlowConfig
//
// Provides tree-sitter queries for:
//   - Variable assignment (initial decl + reassignment) → flow_binding_lhs
//   - Type guards (instanceof, type predicates) → narrowings
//   - Call-site type arguments (`findOne<User>()`) → chain segment type_args
//
// The shared `indexer::flow::run_flow_queries` consumes these, correlates
// captures back to `ExtractedRef`s via `byte_offset`, and writes the results
// into `ParsedFile::flow`. The resolver / chain walkers read that metadata to
// drive forward type inference, conditional narrowing, and generics.
// =============================================================================

use crate::indexer::flow::FlowConfig;

/// TypeScript flow-typing queries. Singleton — registered on the plugin via
/// `LanguagePlugin::flow_config()`.
pub static TS_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "ts",

    // Matches `let/const/var x = <expr>` and `x = <expr>` reassignment.
    // The @rhs capture is the raw value expression; the flow runner finds the
    // chain/call ref whose byte_offset lies in [rhs.start_byte, rhs.end_byte)
    // and binds its resolved_yield_type to the symbol named by @lhs.
    assignment_query: r#"
        (variable_declarator
            name: (identifier) @lhs
            value: (_) @rhs)

        (assignment_expression
            left: (identifier) @lhs
            right: (_) @rhs)
    "#,

    // Matches `if (x instanceof Foo) { ... }` — the canonical narrowing form
    // in JS/TS. The @guard.body capture is the statement_block whose byte
    // range defines the narrowed scope; `x` inside that range is treated as
    // type `Foo`.
    //
    // Type predicates (`function isFoo(x): x is Foo`) and typeof-on-string
    // narrowings are left as future work — they require tracking which
    // function narrows which parameter, not just a lexical block scope.
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (identifier) @guard.local
                    right: (identifier) @guard.type))
            consequence: (statement_block) @guard.body)
    "#,

    // Matches call sites carrying explicit type arguments:
    //   obj.findOne<User>()
    //   repo.get<Item, Key>()
    //
    // The flow runner correlates @call.method with the MemberChain's last
    // segment and populates its `type_args` vec, which the chain walker then
    // binds via `TypeEnvironment::enter_generic_context`.
    type_args_query: r#"
        (call_expression
            function: (member_expression
                property: (property_identifier) @call.method)
            type_arguments: (type_arguments
                (type_identifier) @call.type_arg))
    "#,
};
