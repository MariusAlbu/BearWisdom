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
use crate::indexer::flow_assignments::DestructureShape;
use crate::indexer::flow_cfg::{Fact, FactMap};
use tree_sitter::Node;

pub(crate) fn condition_true_guard(condition: Node, source: &[u8]) -> FactMap {
    let mut facts = FactMap::default();
    let node = unwrap_parentheses(condition);
    if node.kind() != "binary_expression" {
        return facts;
    }
    let operator = node
        .child_by_field_name("operator")
        .and_then(|operator| operator.utf8_text(source).ok())
        .unwrap_or("");
    let left = node.child_by_field_name("left");
    let right = node.child_by_field_name("right");
    if operator == "&&" {
        if let (Some(left), Some(right)) = (left, right) {
            let left = condition_true_guard(left, source);
            let right = condition_true_guard(right, source);
            for (name, fact) in left.0 {
                facts.insert(name, fact);
            }
            for (name, fact) in right.0 {
                facts.insert(name, fact);
            }
        }
        return facts;
    }
    if operator == "||" {
        if let (Some(left), Some(right)) = (left, right) {
            let mut left = condition_true_guard(left, source);
            left.join(&condition_true_guard(right, source));
            return left;
        }
        return facts;
    }
    if operator == "instanceof" {
        if let (Some(left), Some(right)) = (left, right) {
            if left.kind() == "identifier" && right.kind() == "identifier" {
                if let (Ok(name), Ok(ty)) = (left.utf8_text(source), right.utf8_text(source)) {
                    facts.insert(name.to_owned(), Fact::Single(ty.to_owned()));
                }
            }
        }
        return facts;
    }
    if !matches!(operator, "===" | "==") {
        return facts;
    }
    let (Some(left), Some(right)) = (left, right) else {
        return facts;
    };
    let left = unwrap_parentheses(left);
    if left.kind() != "unary_expression" {
        return facts;
    }
    let is_typeof = left
        .child_by_field_name("operator")
        .and_then(|operator| operator.utf8_text(source).ok())
        .is_some_and(|operator| operator == "typeof");
    let Some(argument) = left.child_by_field_name("argument") else {
        return facts;
    };
    if is_typeof && argument.kind() == "identifier" && right.kind() == "string" {
        if let (Ok(name), Ok(raw_type)) = (argument.utf8_text(source), right.utf8_text(source)) {
            if let Some(ty) = normalize_guard_type(raw_type) {
                facts.insert(name.to_owned(), Fact::Single(ty));
            }
        }
    }
    facts
}

fn unwrap_parentheses(mut node: Node) -> Node {
    while node.kind() == "parenthesized_expression" {
        let mut cursor = node.walk();
        let Some(inner) = node.named_children(&mut cursor).next() else {
            break;
        };
        node = inner;
    }
    node
}

pub(crate) fn normalize_guard_type(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let bytes = trimmed.as_bytes();
    let normalized = if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    (!normalized.is_empty()).then(|| normalized.to_owned())
}

pub(crate) fn discriminant_early_exit_scope(if_node: Node) -> Option<(u32, u32)> {
    const EXIT_KINDS: &[&str] = &[
        "return_statement",
        "throw_statement",
        "break_statement",
        "continue_statement",
    ];
    let consequence = if_node.child_by_field_name("consequence")?;
    let block_exits = if consequence.kind() == "statement_block" {
        let mut cursor = consequence.walk();
        let found = consequence
            .named_children(&mut cursor)
            .any(|child| EXIT_KINDS.contains(&child.kind()));
        found
    } else {
        false
    };
    let exits = EXIT_KINDS.contains(&consequence.kind()) || block_exits;
    if !exits {
        return None;
    }
    let start = if_node.end_byte() as u32;
    let end = if_node.parent()?.end_byte() as u32;
    (end > start).then_some((start, end))
}

pub(crate) fn return_object_members(node: Node, source: &[u8]) -> Option<Vec<String>> {
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

pub(crate) fn destructure_shape(binding: Node) -> DestructureShape {
    positional_shape(binding, "array_pattern", false)
}

pub(crate) fn is_await_rhs(node: Node) -> bool {
    node.kind() == "await_expression"
}

fn positional_shape(
    binding: Node,
    pattern_kind: &str,
    requires_multiple_bindings: bool,
) -> DestructureShape {
    let Some(pattern) = binding.parent() else {
        return DestructureShape::NotPositional;
    };
    if pattern.kind() != pattern_kind {
        let mut ancestor = Some(pattern);
        while let Some(parent) = ancestor {
            if parent.kind() == pattern_kind {
                return DestructureShape::Unsupported;
            }
            ancestor = parent.parent();
        }
        return DestructureShape::NotPositional;
    }
    let mut cursor = pattern.walk();
    let direct = pattern.named_children(&mut cursor).collect::<Vec<_>>();
    if direct.iter().any(|child| child.kind() != "identifier")
        || (requires_multiple_bindings && direct.len() < 2)
    {
        return DestructureShape::Unsupported;
    }
    let mut ancestor = pattern.parent();
    while let Some(parent) = ancestor {
        if parent.kind() == pattern_kind {
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

/// Two block-scoped narrowing forms:
///   `if (x instanceof Foo) { ... }`      — `x` narrows to the class `Foo`
///   `if (typeof x === "string") { ... }` — `x` narrows to the primitive
/// The `@guard.body` statement_block bounds the narrowed scope. Purely
/// structural (no type syntax), so the same source compiles against the
/// TypeScript, TSX, and JavaScript grammars — `javascript::flow::JS_FLOW_CONFIG`
/// shares it.
///
/// Type predicates (`function isFoo(x): x is Foo`) stay out: they narrow at
/// the *call* site through which function was invoked, which is a cross-
/// function dataflow, not a lexical block a single query can capture.
pub const TS_TYPE_GUARD_QUERY: &str = r#"
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
"#;

/// Discriminated-union guards. `@guard.local` is the receiver, `@guard.prop`
/// the discriminant property, `@guard.literal` the matched literal (with
/// quotes), `@guard.body` the scope in which `x` narrows to the branch whose
/// `prop` equals that literal. Structural, so it also compiles against the
/// JavaScript grammar and is shared by `JS_FLOW_CONFIG`. Three forms:
///   if  (x.kind === "circle") { ... }        — positive guard
///   switch (x.kind) { case "circle": ... }   — @guard.body is the switch_case
///                                              node, whose range covers the case
///   if  (x.kind !== "circle") return;        — negated early-exit guard,
///                                              narrows the rest of the block
pub const TS_DISCRIMINANT_GUARD_QUERY: &str = r#"
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
"#;

/// Maps bare literal RHS node kinds to the wrapper type they imply when no
/// annotation and no resolvable ref are present. Mirrors the same mapping in
/// `languages/typescript/calls.rs` that stamps the wrapper at a literal
/// call site; this variant covers `const x = []; x.map()` where the local
/// is declared with a literal but called through an identifier. The node
/// kinds are identical in the JavaScript grammar, so `JS_FLOW_CONFIG` shares
/// the table.
pub const TS_LITERAL_TYPE_KINDS: &[(&str, &str)] = &[
    ("array", "Array"),
    ("object", "Object"),
    ("string", "String"),
    ("template_string", "String"),
    ("number", "Number"),
    ("regex", "RegExp"),
];

/// TypeScript flow-typing queries. Singleton — registered on the plugin via
/// `LanguagePlugin::flow_config()`.
pub static TS_FLOW_CONFIG: FlowConfig = FlowConfig {
    strategy_prefix: "ts",

    // Matches `let/const/var x = <expr>` and `x = <expr>` reassignment.
    // The optional `type: (type_annotation (_) @type)` capture records the
    // declared type text for annotated locals (`const x: Array<T> = …`) into
    // `flow_binding_decl_type` verbatim; the type interner decomposes the
    // generic application when the chain walker seeds it, so the receiver
    // types even when the RHS has no resolvable ref. The @rhs capture drives
    // forward inference as before.
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

        (variable_declarator
            name: (array_pattern
                (identifier) @destruct.bind)
            value: (_) @rhs)

        (required_parameter
            pattern: (identifier) @lhs.param
            (type_annotation (_) @type))

        (optional_parameter
            pattern: (identifier) @lhs.param
            (type_annotation (_) @type))

        (required_parameter
            pattern: (identifier) @lhs.param)
    "#,

    type_guard_query: TS_TYPE_GUARD_QUERY,

    discriminant_guard_query: TS_DISCRIMINANT_GUARD_QUERY,

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

    literal_type_kinds: TS_LITERAL_TYPE_KINDS,
};

/// TS/JS syntax data for the generic lexical-identity ingestion pass.
pub(crate) static TS_LEXICAL_SYNTAX: crate::indexer::lexical::LexicalSyntax =
    crate::indexer::lexical::LexicalSyntax {
        declaration_modifiers: &["ambient_declaration"],
        globals: crate::indexer::lexical::globals::Forms {
            inherited_members: crate::indexer::lexical::globals::InheritedMembers::DeclarationOrder,
            surface: &MEMBER_SURFACE,
            names: &["identifier", "type_identifier"],
            member_names: &["property_identifier", "identifier"],
            private_member: ("private_property_identifier", "static"),
            declaration_wrappers: &["expression_statement"],
            augmentation: ("ambient_declaration", "global", "statement_block"),
            ignored_roots: &["expression_statement", "empty_statement", "hash_bang_line"],
            unsupported_named: &["internal_module", "module", "enum_declaration"],
        },
        base_types: (
            &["class_declaration", "class", "abstract_class_declaration"],
            "class_heritage",
            "extends_clause",
            "value",
        ),
        base_head: ("identifier", "property", "type_arguments"),
        type_bases: (&["interface_declaration"], "extends_type_clause", "type"),
        modules: &crate::indexer::lexical::modules::ModuleForms {
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
            default_token: "default",
            declaration_name_field: "name",
            container_name_field: "name",
            container_body_field: "body",
            literal_kind: "string",
            decode_literal: decode_module_literal,
            first_named_child,
            default_export_name: "default",
        },
        functions: &[
            "function_declaration",
            "function_signature",
            "function_expression",
            "generator_function_declaration",
            "generator_function",
            "arrow_function",
            "method_definition",
        ],
        named_declarations: &[
            ("function_declaration", crate::types::SymbolKind::Function),
            ("function_signature", crate::types::SymbolKind::Function),
            (
                "generator_function_declaration",
                crate::types::SymbolKind::Function,
            ),
            (
                "abstract_class_declaration",
                crate::types::SymbolKind::Class,
            ),
            ("class_declaration", crate::types::SymbolKind::Class),
        ],
        named_expressions: &[
            ("function_expression", crate::types::SymbolKind::Function),
            ("generator_function", crate::types::SymbolKind::Function),
            ("class", crate::types::SymbolKind::Class),
        ],
        overload_declarations: &["function_signature"],
        type_declarations: &[
            ("interface_declaration", crate::types::SymbolKind::Interface),
            (
                "type_alias_declaration",
                crate::types::SymbolKind::TypeAlias,
            ),
        ],
        alias_values: &[("type_alias_declaration", "value")],
        compiler_intrinsics: &crate::indexer::lexical::type_syntax::compiler_intrinsics::Forms {
            keyword: "intrinsic",
            keyword_kind: "type_identifier",
            name_field: "name",
            parameters_field: "type_parameters",
            zero_parameter_aliases: &[(
                "BuiltinIteratorReturn",
                crate::indexer::lexical::type_syntax::compiler_intrinsics::Role::IteratorReturn,
            )],
        },
        dual_declarations: &[crate::types::SymbolKind::Class],
        merge_declarations: &[
            (
                crate::types::SymbolKind::Interface,
                crate::types::SymbolKind::Interface,
            ),
            (
                crate::types::SymbolKind::Class,
                crate::types::SymbolKind::Interface,
            ),
        ],
        type_scopes: &[
            "class_declaration",
            "abstract_class_declaration",
            "class",
            "interface_declaration",
            "type_alias_declaration",
            "function_type",
            "constructor_type",
            "method_signature",
            "abstract_method_signature",
            "call_signature",
            "construct_signature",
        ],
        type_forms: &[
            (
                "object_type",
                crate::indexer::lexical::type_syntax::TypeForm::Object(&STRUCTURAL_TYPES),
            ),
            (
                "predefined_type",
                crate::indexer::lexical::type_syntax::TypeForm::AtomicOrUnique(
                    &ATOMIC_TYPES,
                    &UNIQUE_SYMBOLS,
                ),
            ),
            (
                "literal_type",
                crate::indexer::lexical::type_syntax::TypeForm::Transparent,
            ),
            (
                "string",
                crate::indexer::lexical::type_syntax::TypeForm::Atomic(&ATOMIC_TYPES),
            ),
            (
                "number",
                crate::indexer::lexical::type_syntax::TypeForm::Atomic(&ATOMIC_TYPES),
            ),
            (
                "true",
                crate::indexer::lexical::type_syntax::TypeForm::Atomic(&ATOMIC_TYPES),
            ),
            (
                "false",
                crate::indexer::lexical::type_syntax::TypeForm::Atomic(&ATOMIC_TYPES),
            ),
            (
                "null",
                crate::indexer::lexical::type_syntax::TypeForm::Atomic(&ATOMIC_TYPES),
            ),
            (
                "undefined",
                crate::indexer::lexical::type_syntax::TypeForm::Atomic(&ATOMIC_TYPES),
            ),
            (
                "unary_expression",
                crate::indexer::lexical::type_syntax::TypeForm::Atomic(&ATOMIC_TYPES),
            ),
            (
                "type_identifier",
                crate::indexer::lexical::type_syntax::TypeForm::Name,
            ),
            (
                "type_query",
                crate::indexer::lexical::type_syntax::TypeForm::ValueQuery,
            ),
            (
                "type_annotation",
                crate::indexer::lexical::type_syntax::TypeForm::Transparent,
            ),
            (
                "parenthesized_type",
                crate::indexer::lexical::type_syntax::TypeForm::Transparent,
            ),
            (
                "constraint",
                crate::indexer::lexical::type_syntax::TypeForm::Transparent,
            ),
            (
                "default_type",
                crate::indexer::lexical::type_syntax::TypeForm::Transparent,
            ),
            (
                "generic_type",
                crate::indexer::lexical::type_syntax::TypeForm::Apply,
            ),
            (
                "function_type",
                crate::indexer::lexical::type_syntax::TypeForm::Function(&CALLABLE_TYPES),
            ),
            (
                "array_type",
                crate::indexer::lexical::type_syntax::TypeForm::Array(
                    &crate::indexer::lexical::type_syntax::arrays::Forms {
                        mutable: "Array",
                        readonly: "ReadonlyArray",
                    },
                ),
            ),
            (
                "tuple_type",
                crate::indexer::lexical::type_syntax::TypeForm::Tuple,
            ),
            (
                "union_type",
                crate::indexer::lexical::type_syntax::TypeForm::Union,
            ),
            (
                "intersection_type",
                crate::indexer::lexical::type_syntax::TypeForm::Intersection,
            ),
            (
                "optional_type",
                crate::indexer::lexical::type_syntax::TypeForm::Optional,
            ),
            (
                "index_type_query",
                crate::indexer::lexical::type_syntax::TypeForm::KeyOf,
            ),
            (
                "readonly_type",
                crate::indexer::lexical::type_syntax::TypeForm::Readonly,
            ),
            (
                "lookup_type",
                crate::indexer::lexical::type_syntax::TypeForm::IndexedAccess,
            ),
            (
                "infer_type",
                crate::indexer::lexical::type_syntax::TypeForm::Infer,
            ),
            (
                "conditional_type",
                crate::indexer::lexical::type_syntax::TypeForm::Conditional {
                    check: "left",
                    extends: "right",
                    when_true: "consequence",
                    when_false: "alternative",
                },
            ),
        ],
        call_roots: &[
            ("call_expression", "function"),
            ("new_expression", "constructor"),
        ],
        call_selectors: &[("member_expression", "property")],
        call_identifiers: &["identifier", "undefined"],
        initializer_forms: &crate::indexer::lexical::type_syntax::initializers::Forms {
            construct: ("new_expression", "constructor"),
            groups: &["parenthesized_expression"],
            atoms: &ATOMIC_TYPES,
            call: ("call_expression", "function"),
            objects: &crate::indexer::lexical::type_syntax::initializers::objects::Forms {
                object: "object",
                property: ("pair", "key", "value"),
                shorthand: "shorthand_property_identifier",
                declarations: &["lexical_declaration", "variable_declaration"],
            },
        },
        callback_forms: &CALLBACK_BODIES,
        reference_roots: &[
            ("member_expression", "object"),
            ("subscript_expression", "object"),
        ],
        reference_wrappers: &["parenthesized_expression", "await_expression"],
        blocks: &[
            "statement_block",
            "for_statement",
            "for_in_statement",
            "switch_body",
            "catch_clause",
        ],
        variables: &["variable_declarator"],
        assignments: &["assignment_expression"],
        bindings: crate::indexer::lexical::BindingForms {
            name_field: "name",
            function_parameters_field: Some("parameters"),
            function_parameter_field: Some("parameter"),
            variable_value_field: "value",
            binding_name_kinds: &["identifier"],
            assignment_left_field: "left",
            assignment_right_field: "right",
            catch_parameter: Some(("catch_clause", "parameter")),
            call_type_arguments_field: "type_arguments",
            call_callee_kinds: &["identifier", "type_identifier"],
            type_parameters_field: "type_parameters",
            annotation_field: "type",
            annotation_value: Some(annotation_value),
            direct_patterns: &["identifier", "shorthand_property_identifier_pattern"],
            pattern_fields: &[
                ("required_parameter", "pattern"),
                ("optional_parameter", "pattern"),
                ("assignment_pattern", "left"),
                ("object_assignment_pattern", "left"),
                ("pair_pattern", "value"),
            ],
            pattern_children: &["rest_pattern", "object_pattern", "array_pattern"],
        },
        function_scoped_declaration: "variable_declaration",
        literal_types: TS_LITERAL_TYPE_KINDS,
        preserve_non_union_type: true,
    };

use crate::indexer::lexical::globals::member_surface::{
    Forms as MemberForms, Kind as MemberKind, Modifier,
};
use crate::indexer::lexical::type_syntax::atoms::{Forms as AtomicForms, LiteralKind};
use crate::type_checker::core::types::Intrinsic;

pub(crate) fn first_named_child(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    node.named_child(0)
}

fn first_non_extra_named_child<'a>(node: tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    let child = node
        .named_children(&mut cursor)
        .find(|child| !child.is_extra());
    child
}

fn annotation_value(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    first_named_child(node)?
        .utf8_text(source)
        .ok()
        .map(str::to_owned)
}

/// Decode a TypeScript module-specifier literal at the language boundary.
/// Generic lexical module capture receives the already-normalized string.
fn decode_module_literal(raw: &str) -> Option<String> {
    let quote = raw.chars().next()?;
    if !matches!(quote, '\'' | '"') || !raw.ends_with(quote) || raw.len() < 2 {
        return None;
    }
    let mut chars = raw[1..raw.len() - 1].chars().peekable();
    let mut units = Vec::new();
    while let Some(ch) = chars.next() {
        if ch == quote || matches!(ch, '\n' | '\r') {
            return None;
        }
        let ch = if ch == '\\' {
            match chars.next()? {
                '\n' => continue,
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    continue;
                }
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'v' => '\u{b}',
                '0' if !chars.peek().is_some_and(|c| c.is_ascii_digit()) => '\0',
                '0'..='9' => return None,
                'x' => char::from_u32(module_literal_hex(&mut chars, 2)?)?,
                'u' => {
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let mut value = 0u32;
                        let mut count = 0;
                        loop {
                            let digit = chars.next()?;
                            if digit == '}' {
                                break;
                            }
                            value = value.checked_mul(16)?.checked_add(digit.to_digit(16)?)?;
                            count += 1;
                        }
                        if count == 0 {
                            return None;
                        }
                        char::from_u32(value)?
                    } else {
                        units.push(module_literal_hex(&mut chars, 4)? as u16);
                        continue;
                    }
                }
                escaped => escaped,
            }
        } else {
            ch
        };
        units.extend_from_slice(ch.encode_utf16(&mut [0; 2]));
    }
    String::from_utf16(&units).ok()
}

fn module_literal_hex(chars: &mut impl Iterator<Item = char>, count: usize) -> Option<u32> {
    (0..count).try_fold(0u32, |value, _| {
        Some(value * 16 + chars.next()?.to_digit(16)?)
    })
}

static CALLBACK_BODIES: crate::indexer::lexical::globals::call_arguments::callbacks::Forms =
    crate::indexer::lexical::globals::call_arguments::callbacks::Forms {
        functions: &["arrow_function", "function_expression"],
        wrappers: &["parenthesized_expression"],
        type_parameters: "type_parameters",
        parameters: "parameters",
        parameter: "parameter",
        pattern: "pattern",
        value: "value",
        body: "body",
        block: "statement_block",
        return_: "return_statement",
        forbidden_tokens: &["async", "*"],
        forbidden_expressions: &["optional_chain"],
        binary: "binary_expression",
        operator: "operator",
        left: "left",
        right: "right",
        argument: "argument",
        type_arguments: "type_arguments",
        arguments: "arguments",
        equality: &[("===", false), ("==", false), ("!==", true), ("!=", true)],
        typeof_: ("unary_expression", "typeof"),
        boolean_not: ("unary_expression", "!"),
        type_names: &[
            ("string", Intrinsic::String),
            ("number", Intrinsic::Number),
            ("boolean", Intrinsic::Boolean),
            ("bigint", Intrinsic::BigInt),
            ("symbol", Intrinsic::Symbol),
            ("undefined", Intrinsic::Undefined),
        ],
        return_value: first_named_child,
    };

pub(crate) static ATOMIC_TYPES: AtomicForms = AtomicForms {
    member_wrappers: &[
        (Intrinsic::String, "String"),
        (Intrinsic::Number, "Number"),
        (Intrinsic::Boolean, "Boolean"),
        (Intrinsic::Symbol, "Symbol"),
        (Intrinsic::BigInt, "BigInt"),
        (Intrinsic::Object, "Object"),
    ],
    intrinsics: &[
        ("any", Intrinsic::Any),
        ("unknown", Intrinsic::Unknown),
        ("never", Intrinsic::Never),
        ("void", Intrinsic::Void),
        ("undefined", Intrinsic::Undefined),
        ("null", Intrinsic::Null),
        ("object", Intrinsic::Object),
        ("string", Intrinsic::String),
        ("number", Intrinsic::Number),
        ("boolean", Intrinsic::Boolean),
        ("symbol", Intrinsic::Symbol),
        ("bigint", Intrinsic::BigInt),
    ],
    literals: &[
        ("string", LiteralKind::String),
        ("number", LiteralKind::Number),
        ("true", LiteralKind::Boolean(true)),
        ("false", LiteralKind::Boolean(false)),
    ],
    negative: ("unary_expression", "argument", "operator", "-"),
    numeric_separator: '_',
    bigint_suffix: "n",
    radix_prefixes: &[
        ("0x", 16),
        ("0X", 16),
        ("0b", 2),
        ("0B", 2),
        ("0o", 8),
        ("0O", 8),
    ],
    quotes: &['\'', '"'],
    escapes: &[
        ('n', 10),
        ('r', 13),
        ('t', 9),
        ('b', 8),
        ('f', 12),
        ('v', 11),
        ('\\', 92),
        ('\'', 39),
        ('"', 34),
    ],
    identity_escapes: true,
    unicode_escape: 'u',
    hex_escape: 'x',
    braced_unicode: true,
};

static CALLABLE_TYPES: crate::indexer::lexical::type_syntax::callables::Forms =
    crate::indexer::lexical::type_syntax::callables::Forms {
        predicate: "type_predicate",
        assertion: "asserts",
        identifier: &["identifier", "this"],
        parameters: "parameters",
        pattern: "pattern",
        return_type: "return_type",
        predicate_name: "name",
        predicate_type: "type",
        assertion_target: first_named_child,
    };

static STRUCTURAL_TYPES: crate::indexer::lexical::type_syntax::structural::Forms =
    crate::indexer::lexical::type_syntax::structural::Forms {
        property: "property_signature",
        index: "index_signature",
        mapped_clause: "mapped_type_clause",
        identifier: &["property_identifier", "identifier"],
        optional_annotations: &[
            (
                "type_annotation",
                crate::type_checker::core::types::MappedModifier::Preserve,
            ),
            (
                "opting_type_annotation",
                crate::type_checker::core::types::MappedModifier::Add,
            ),
            (
                "adding_type_annotation",
                crate::type_checker::core::types::MappedModifier::Add,
            ),
            (
                "omitting_type_annotation",
                crate::type_checker::core::types::MappedModifier::Remove,
            ),
        ],
    };

static UNIQUE_SYMBOLS: crate::indexer::lexical::type_syntax::unique_symbols::Forms =
    crate::indexer::lexical::type_syntax::unique_symbols::Forms {
        tokens: &["unique symbol", "unique symbol"],
        annotation: "type_annotation",
        transparent: &["parenthesized_type"],
        properties: &[
            ("property_signature", &["readonly"]),
            ("public_field_definition", &["static", "readonly"]),
        ],
        variable: (
            "variable_declarator",
            "lexical_declaration",
            "kind",
            "const",
        ),
        names: &[
            "identifier",
            "property_identifier",
            "string",
            "number",
            "computed_property_name",
            "private_property_identifier",
        ],
    };

static MEMBER_SURFACE: MemberForms = MemberForms {
    overload_order:
        crate::indexer::lexical::globals::member_surface::OverloadOrder::MergedGroupsLiteralFirst,
    specialized_parameter_types: &["literal_type"],
    kinds: &[
        ("property_signature", MemberKind::Property),
        ("public_field_definition", MemberKind::Property),
        ("method_signature", MemberKind::Method),
        ("method_definition", MemberKind::Method),
        ("abstract_method_signature", MemberKind::Method),
        ("call_signature", MemberKind::Call),
        ("construct_signature", MemberKind::Construct),
        ("index_signature", MemberKind::Index),
    ],
    modifiers: &[
        ("readonly", Modifier::Readonly),
        ("?", Modifier::Optional),
        ("static", Modifier::Static),
        ("abstract", Modifier::Abstract),
        ("public", Modifier::Public),
        ("protected", Modifier::Protected),
        ("private", Modifier::Private),
        ("override", Modifier::Override),
        ("declare", Modifier::Declare),
    ],
    accessor_tokens: ("get", "set"),
    constructor_name: "constructor",
    modifier_wrappers: &["accessibility_modifier", "override_modifier"],
    computed: "computed_property_name",
    literals: &["string", "number"],
    optional_parameter: "optional_parameter",
    rest_pattern: "rest_pattern",
    receiver_parameter: ("pattern", "this"),
    type_annotations: &[
        "type_annotation",
        "asserts_annotation",
        "type_predicate_annotation",
    ],
    // The grammar aliases both tokens of the sequence to the same token kind.
    unique_type: &["unique symbol", "unique symbol"],
    erased_containers: &["interface_body", "object_type"],
    erased_modifiers: &["abstract", "declare"],
    owner_body_field: "body",
    member_name_field: "name",
    signature_return_fields: &["return_type", "type"],
    signature_body_field: "body",
    signature_initializer_field: "value",
    type_parameters_field: "type_parameters",
    parameters_field: "parameters",
    single_parameter_field: "parameter",
    parameter_type_field: "type",
    parameter_initializer_field: "value",
    parameter_pattern_field: "pattern",
    index_name_field: "name",
    index_type_field: "index_type",
    type_parameter_name_field: "name",
    type_parameter_parts: &["name", "constraint", "value"],
    type_annotation_inner: first_named_child,
    computed_key_expression: first_non_extra_named_child,
};

pub const TS_CFG_KINDS: crate::indexer::flow_cfg::CfgNodeKinds =
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
        if_alternative_body: Some(first_named_child),
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
        condition_true_guard: Some(condition_true_guard),
    };
