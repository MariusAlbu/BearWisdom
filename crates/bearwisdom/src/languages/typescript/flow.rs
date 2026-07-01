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

/// Return-expression query for body-based return-type inference (INFER-3).
/// Captures the returned expression of every `return <expr>`, plus an
/// arrow-function concise body (`() => expr`) via `@return.tail`. The consumer
/// (`flow::run_return_query`) resolves which function owns it by walking
/// ancestors to the nearest function node. That ancestor-walk attributes a
/// return nested in `if`/`for`/`switch` to its enclosing named function, and a
/// return inside a nested arrow/callback to the lambda (dropped when anonymous)
/// — so the widened capture never misattributes a callback return. The
/// `@return.tail` body field is skipped by the consumer when it is a
/// `statement_block` (its return value comes from the explicit `return` arm).
pub const TS_RETURN_QUERY: &str = r#"
    (return_statement (_) @return.expr)
    (arrow_function body: (_) @return.tail)
"#;

/// TypeScript flow-typing queries. Singleton — registered on the plugin via
/// `LanguagePlugin::flow_config()`.
pub static TS_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "ts",

    // Matches `let/const/var x = <expr>` and `x = <expr>` reassignment.
    // The optional `type: (type_annotation (_) @type)` capture records the
    // declared type for annotated locals (`const x: Array<T> = …`); the generic
    // runner strips generic args and writes `flow_binding_decl_type` so the
    // chain walker sees the receiver type even when the RHS has no resolvable ref.
    // The @rhs capture drives forward inference as before.
    //
    // The `required_parameter` / `optional_parameter` arms seed a declared
    // parameter type (`function f(text: string)`) the same way, so a member call
    // on the parameter (`text.replace(...)`) types its receiver from the
    // annotation. A parameter has no initializer, so only `@lhs` + `@type` are
    // captured (no `@rhs`). The accessibility-modifier shorthand
    // (`constructor(private db: Repo)`) is a `required_parameter` too, so it is
    // covered by the same arm.
    assignment_query: r#"
        (variable_declarator
            name: (identifier) @lhs
            type: (type_annotation (_) @type)?
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

        (required_parameter
            pattern: (identifier) @lhs
            (type_annotation (_) @type))

        (optional_parameter
            pattern: (identifier) @lhs
            (type_annotation (_) @type))
    "#,

    // Two block-scoped narrowing forms:
    //   if (x instanceof Foo) { ... }       — `x` narrows to the class `Foo`
    //   if (typeof x === "string") { ... }   — `x` narrows to the primitive
    // The @guard.body statement_block bounds the narrowed scope.
    //
    // Type predicates (`function isFoo(x): x is Foo`) stay out: they narrow at
    // the *call* site through which function was invoked, which is a cross-
    // function dataflow, not a lexical block a single query can capture.
    type_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (identifier) @guard.local
                    operator: "instanceof"
                    right: (identifier) @guard.type))
            consequence: (statement_block) @guard.body)

        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (unary_expression
                        operator: "typeof"
                        argument: (identifier) @guard.local)
                    operator: ["===" "=="]
                    right: (string) @guard.type))
            consequence: (statement_block) @guard.body)
    "#,

    // Discriminated-union guards. @guard.local is the receiver, @guard.prop the
    // discriminant property, @guard.literal the matched literal (with quotes),
    // @guard.body the scope in which `x` narrows to the branch whose `prop`
    // equals that literal. Two forms:
    //   if  (x.kind === "circle") { ... }   — `!==` intentionally unmatched
    //                                          (it narrows the else-branch)
    //   switch (x.kind) { case "circle": ... }  — @guard.body is the switch_case
    //                                          node, whose range covers the case
    discriminant_guard_query: r#"
        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (member_expression
                        object: (identifier) @guard.local
                        property: (property_identifier) @guard.prop)
                    operator: ["===" "=="]
                    right: (string) @guard.literal))
            consequence: (statement_block) @guard.body)

        (switch_statement
            value: (parenthesized_expression
                (member_expression
                    object: (identifier) @guard.local
                    property: (property_identifier) @guard.prop))
            body: (switch_body
                (switch_case
                    value: (string) @guard.literal) @guard.body))

        (if_statement
            condition: (parenthesized_expression
                (binary_expression
                    left: (member_expression
                        object: (identifier) @guard.local
                        property: (property_identifier) @guard.prop)
                    operator: ["!==" "!="]
                    right: (string) @guard.literal))) @guard.early_exit
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

        (call_expression
            function: (identifier) @call.method
            type_arguments: (type_arguments
                (_) @call.type_arg))
    "#,

    // Maps bare literal RHS node kinds to the wrapper type they imply when no
    // annotation and no resolvable ref are present. Mirrors the same mapping in
    // `languages/typescript/calls.rs` that stamps the wrapper at a literal
    // call site; this variant covers `const x = []; x.map()` where the local
    // is declared with a literal but called through an identifier.
    literal_type_kinds: &[
        ("array", "Array"),
        ("object", "Object"),
        ("string", "String"),
        ("template_string", "String"),
        ("number", "Number"),
        ("regex", "RegExp"),
    ],
};
