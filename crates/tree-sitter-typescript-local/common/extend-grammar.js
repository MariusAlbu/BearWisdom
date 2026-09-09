// Source syntax only: declaration legality and identities belong to the binder.
const defineGrammar = require('tree-sitter-typescript/common/define-grammar');

module.exports = dialect => grammar(defineGrammar(dialect), {
  name: dialect,
  precedences: ($, previous) => previous.concat([
    [$.lookup_type, $.array_type, 'type_operator', $.intersection_type, $.union_type, $.conditional_type, $.type],
  ]),
  rules: {
    export_statement: ($, previous) => choice(
      previous,
      seq('export', 'type', choice('*', $.namespace_export), $._from_clause, $._semicolon),
    ),
    predefined_type: ($, previous) => choice(previous, 'bigint'),
    _reserved_identifier: ($, previous) => choice(previous, 'global'),
    // Inside an ambient external module, a global augmentation omits `declare`.
    // Keep the same CST shape as explicit `declare global` for source ingestion.
    ambient_declaration: ($, previous) => choice(
      prec(1, previous),
      seq('global', $.statement_block),
    ),

    index_type_query: $ => prec.left('type_operator', seq('keyof', choice($.primary_type, $.readonly_type))),
    readonly_type: $ => prec.left('type_operator', seq('readonly', $.primary_type)),

    // Import-qualified heads participate in ordinary postfix type operations
    // (arrays/indexed access/keyof), not only a terminal annotation position.
    type: $ => choice($.primary_type, $.function_type, $.readonly_type, $.constructor_type, $.infer_type),
    primary_type: ($, previous) => choice(
      previous,
      prec(-1, alias($._type_query_member_expression_in_type_annotation, $.member_expression)),
      prec(-1, alias($._type_query_call_expression_in_type_annotation, $.call_expression)),
    ),

    // Import-qualified types are not runtime call instantiations. Preserve a
    // generic_type with a member_expression head and explicit type_arguments.
    generic_type: ($, previous) => choice(
      previous,
      prec('call', seq(
        field('name', alias($._type_query_member_expression_in_type_annotation, $.member_expression)),
        field('type_arguments', $.type_arguments),
      )),
    ),
  },
});
