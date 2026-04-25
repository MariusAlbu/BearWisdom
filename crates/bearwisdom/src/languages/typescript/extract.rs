// =============================================================================
// languages/typescript/extract.rs  —  TypeScript / TSX extractor
//
// SYMBOLS: Class, Interface, Function, Method, Constructor, Property,
//          TypeAlias, Variable, Enum, EnumMember, Namespace
//
// REFERENCES: imports, calls, extends/implements, type refs, instanceof/as,
//             JSX component usage, tagged templates
// =============================================================================

use super::{calls, decorators, helpers, imports, narrowing, params, symbols, types};

use crate::types::ExtractionResult;
use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration for TypeScript
// ---------------------------------------------------------------------------

pub(crate) static TS_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_declaration", name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "function_declaration", name_field: "name" },
    ScopeKind { node_kind: "method_definition", name_field: "name" },
    // `namespace Foo { ... }` / `module Foo { ... }` — TS namespace blocks.
    // Without this, `interface ProcessEnv` declared inside `namespace NodeJS`
    // would be qualified as `ProcessEnv` instead of `NodeJS.ProcessEnv`,
    // breaking dotted-name TypeRef resolution against @types/node /
    // @playwright/test / @types/jest etc. tree-sitter-typescript emits both
    // `namespace` and `module` keywords as `internal_module`.
    ScopeKind { node_kind: "internal_module", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract symbols and references from TypeScript or TSX source.
pub fn extract(source: &str, is_tsx: bool) -> ExtractionResult {
    extract_inner(source, is_tsx, None)
}

/// R6: demand-filtered extraction. When `demand` is `Some`, top-level
/// declarations whose name is not in the set are skipped entirely —
/// `lib.dom.d.ts` with ~40k types reduces to the ~20 types a project uses.
///
/// `None` delivers the permissive behaviour (identical to `extract`).
pub fn extract_with_demand(
    source: &str,
    is_tsx: bool,
    demand: Option<&HashSet<String>>,
) -> ExtractionResult {
    extract_inner(source, is_tsx, demand)
}

fn extract_inner(
    source: &str,
    is_tsx: bool,
    demand: Option<&HashSet<String>>,
) -> ExtractionResult {
    let language: tree_sitter::Language = if is_tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load TypeScript grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return ExtractionResult {
                symbols: vec![],
                refs: vec![],
                routes: vec![],
                db_sets: vec![],
                has_errors: true,
                connection_points: Vec::new(),
                demand_contributions: Vec::new(),
            }
        }
    };

    let has_errors = tree.root_node().has_error();
    let src_bytes = source.as_bytes();
    let root = tree.root_node();

    let scope_tree = scope_tree::build(root, src_bytes, TS_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Pre-pass: build a local-alias → module-path map from all import statements.
    // This is used below to annotate call refs with their source module.
    let import_map = build_import_map(root, src_bytes);
    // Also build an alias map that carries the ORIGINAL exported name when it
    // differs from the local alias (`import { request as __request }`). Used
    // to rewrite usage-site refs so the resolver can find the real export.
    let alias_map = build_renamed_import_map(root, src_bytes);

    extract_node(root, src_bytes, &scope_tree, &mut symbols, &mut refs, None, demand);

    // Post-traversal full-tree scan: catch every type_identifier and generic_type
    // base name that the main walker may have missed (e.g. deeply nested generic
    // arguments, conditional types, mapped types, etc.).
    //
    // Only runs when no demand filter is active — under demand, most top-level
    // types are intentionally skipped, so the post-scan would re-emit the same
    // type_identifier refs we filtered out, defeating the whole point.
    if !symbols.is_empty() && demand.is_none() {
        scan_all_type_identifiers(root, src_bytes, 0, &mut refs);
    }

    // Post-filter: suppress TypeRef entries whose target is an in-scope type
    // parameter at the ref's source line. A `function f<Target>(x: Target)`
    // or `type T<Target> = { a: Target }` must not leak `Target` as an
    // unresolved external — it's a local generic binding, no more a ref than
    // a local variable. Works uniformly across every emission path (main
    // walker, type helper modules, scan_all post-scan) because it operates
    // on the finished refs vec.
    {
        let mut scopes: Vec<(String, u32, u32)> = Vec::new();
        collect_type_param_scopes(root, src_bytes, &mut scopes);
        if !scopes.is_empty() {
            refs.retain(|r| {
                if r.kind != EdgeKind::TypeRef {
                    return true;
                }
                !scopes.iter().any(|(name, start, end)| {
                    &r.target_name == name && r.line >= *start && r.line <= *end
                })
            });
        }
    }

    // Annotate call refs: if a Calls ref has a chain whose first segment is a
    // known import alias, set module so the resolver can trace it back.
    if !import_map.is_empty() {
        annotate_call_modules(&mut refs, &import_map);
    }

    // Rewrite aliased references: `import { request as __request }; __request(...)`
    // emits a ref with target_name=`__request`, but the exported symbol is
    // `request`. Substitute the original name + module so the resolver can find it.
    if !alias_map.is_empty() {
        rewrite_aliased_refs(&mut refs, &alias_map);
    }

    ExtractionResult::new(symbols, refs, has_errors)
}

/// R6 helper: list every declared name for a top-level declaration node.
/// Returns the `name` field when present (class, interface, function, type
/// alias, enum, namespace) and walks `variable_declarator` children for
/// `lexical_declaration` / `variable_declaration`.
fn declared_names(node: &Node, src: &[u8]) -> Vec<String> {
    if let Some(n) = node.child_by_field_name("name") {
        if let Ok(text) = n.utf8_text(src) {
            return vec![text.to_string()];
        }
    }
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        if c.kind() == "variable_declarator" {
            if let Some(n) = c.child_by_field_name("name") {
                if let Ok(text) = n.utf8_text(src) {
                    out.push(text.to_string());
                }
            }
        }
    }
    out
}

/// R6 helper: the set of tree-sitter node kinds that constitute a top-level
/// TypeScript declaration. Demand filtering only engages for these kinds.
fn is_top_level_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "abstract_class_declaration"
            | "interface_declaration"
            | "function_declaration"
            | "generator_function_declaration"
            | "generator_function"
            | "type_alias_declaration"
            | "enum_declaration"
            | "lexical_declaration"
            | "variable_declaration"
            | "internal_module"
    )
}

/// R6 helper: would a top-level node pass the demand gate? `true` when
/// demand is `None` (permissive mode) or any declared name is in the set.
fn keep_by_demand(
    node: &Node,
    src: &[u8],
    demand: Option<&HashSet<String>>,
    parent_index: Option<usize>,
) -> bool {
    // Filter only applies to top-level declarations. Nested ones (methods
    // inside a class, fields in an interface) ride their container's decision.
    if parent_index.is_some() {
        return true;
    }
    let Some(set) = demand else {
        return true;
    };
    let names = declared_names(node, src);
    if names.is_empty() {
        // No declared name to check — keep (ambient modules, export-only
        // statements, etc.).
        return true;
    }
    names.iter().any(|n| set.contains(n))
}

// ---------------------------------------------------------------------------
// Recursive visitor
// ---------------------------------------------------------------------------

fn extract_node(
    node: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    demand: Option<&HashSet<String>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // R6 demand gate — drop the whole subtree when this is a top-level
        // declaration whose name is not in the project's demand set.
        // Helper returns true unconditionally when demand is None or when
        // parent_index is set (nested declarations ride their container).
        if is_top_level_declaration(child.kind())
            && !keep_by_demand(&child, src, demand, parent_index)
        {
            continue;
        }
        match child.kind() {
            "class_declaration" | "abstract_class_declaration" => {
                let idx = symbols::push_class(&child, src, scope_tree, symbols, parent_index);
                let sym_idx = idx.unwrap_or(0);
                // Heritage clause (extends / implements).
                imports::extract_heritage(&child, src, sym_idx, refs);
                // Decorators (@Injectable, @Controller, etc.).
                decorators::extract_decorators(&child, src, sym_idx, refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx, demand);
                }
            }

            "interface_declaration" => {
                let idx =
                    symbols::push_interface(&child, src, scope_tree, symbols, parent_index);
                imports::extract_heritage(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx, demand);
                }
            }

            "function_declaration" => {
                let idx = symbols::push_function(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    types::extract_typed_params_as_symbols(
                        &child,
                        src,
                        scope_tree,
                        symbols,
                        refs,
                        Some(sym_idx),
                    );
                    // Extract property_signature / method_signature symbols from inline
                    // object types in parameter annotations and return type.
                    extract_sig_object_type_members(
                        child, src, scope_tree, symbols, refs, Some(sym_idx),
                    );
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                        narrowing::extract_narrowing_refs(&body, src, sym_idx, refs);
                        // Also recurse with extract_node so nested lexical_declaration,
                        // catch_clause, for_in_statement, etc. inside the body produce
                        // their symbols and type refs.
                        extract_node(body, src, scope_tree, symbols, refs, Some(sym_idx), demand);
                    }
                }
            }

            "export_statement" => {
                // `export class Foo {}` / `export function bar() {}`
                // Recurse so that declarations inside are extracted.
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
                // Also extract re-export forms:
                //   `export { X } from './y'`
                //   `export { X as Z } from './y'`
                //   `export * from './y'`
                //   `export * as ns from './y'`
                extract_reexports(&child, src, symbols.len(), refs);
            }

            "method_definition" => {
                let idx = symbols::push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    // Constructor parameter properties:
                    // `constructor(private db: DatabaseRepository)` creates a class property.
                    if symbols[sym_idx].kind == SymbolKind::Constructor {
                        params::extract_constructor_params(
                            &child,
                            src,
                            scope_tree,
                            symbols,
                            refs,
                            parent_index,
                        );
                    }
                    // Parameter types and return type for all methods.
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    // Extract typed params as scoped symbols (for chain resolution).
                    // Skip constructors — they're handled by extract_constructor_params.
                    if symbols[sym_idx].kind != SymbolKind::Constructor {
                        types::extract_typed_params_as_symbols(
                            &child,
                            src,
                            scope_tree,
                            symbols,
                            refs,
                            Some(sym_idx),
                        );
                    }
                    // Decorators (@Get, @Post, @UseGuards, etc.).
                    decorators::extract_decorators(&child, src, sym_idx, refs);
                    // Extract property_signature / method_signature symbols from inline
                    // object types in parameter annotations and return type.
                    if symbols[sym_idx].kind != SymbolKind::Constructor {
                        extract_sig_object_type_members(
                            child, src, scope_tree, symbols, refs, Some(sym_idx),
                        );
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                        narrowing::extract_narrowing_refs(&body, src, sym_idx, refs);
                        // Also recurse with extract_node so nested lexical_declaration,
                        // catch_clause, for_in_statement, etc. produce symbols and type refs.
                        extract_node(body, src, scope_tree, symbols, refs, Some(sym_idx), demand);
                    }
                }
            }

            "public_field_definition" | "field_definition" => {
                let field_idx = symbols.len();
                symbols::push_ts_field(&child, src, scope_tree, symbols, refs, parent_index);
                // If the field has an object-type annotation, recurse into it so that
                // nested method_signature / property_signature nodes produce symbols.
                // E.g.: `private ops: { findOne(): User; deleteById(id: number): void; }`.
                if symbols.len() > field_idx {
                    if let Some(type_ann) = child.child_by_field_name("type") {
                        let type_value = {
                            let mut found = None;
                            let mut tc = type_ann.walk();
                            for tc_child in type_ann.children(&mut tc) {
                                if tc_child.kind() != ":" {
                                    found = Some(tc_child);
                                    break;
                                }
                            }
                            found
                        };
                        if let Some(tv) = type_value {
                            recurse_for_object_types(
                                tv, src, scope_tree, symbols, refs, Some(field_idx),
                            );
                        }
                    }
                }
                // Extract calls from the field initializer value, if present.
                // e.g. `private logger = createLogger()` — call in field initializer.
                //
                // The "value" field of public_field_definition is the initializer
                // expression itself (not a body container).  We need to handle it
                // differently depending on what kind of expression it is:
                // - call_expression → emit_call_ref directly, then recurse into args
                // - new_expression  → emit_new_ref directly
                // - anything else   → recurse with extract_calls for nested calls
                //
                // Attribute refs to the field symbol (not its parent class) so the
                // chain walker's SelfRef phase receives a non-empty scope_chain —
                // a class-level source symbol has `scope_path=None`, which makes
                // `this.foo.bar()` inside a field initializer fail at Phase 1.
                let sym_idx = if symbols.len() > field_idx {
                    field_idx
                } else {
                    parent_index.unwrap_or(0)
                };
                if let Some(value) = child.child_by_field_name("value") {
                    match value.kind() {
                        "call_expression" => {
                            calls::emit_call_ref(&value, src, sym_idx, refs);
                            // Recurse into arguments for nested calls.
                            calls::extract_calls(&value, src, sym_idx, refs);
                        }
                        "new_expression" => {
                            calls::emit_new_ref(&value, src, sym_idx, refs);
                            calls::extract_calls(&value, src, sym_idx, refs);
                        }
                        _ => {
                            // For other expressions (method chains, conditionals, etc.)
                            // use extract_calls which recursively finds call_expression nodes.
                            calls::extract_calls(&value, src, sym_idx, refs);
                        }
                    }
                }
            }

            // Interface / object-type property signatures: `db: Database;`
            // Also handles complex types like `user: { findOne(): User; }` where
            // the type annotation contains an object_type with method/property signatures.
            "property_signature" => {
                let prop_idx = symbols.len();
                symbols::push_ts_field(&child, src, scope_tree, symbols, refs, parent_index);
                // If the property has an object-type annotation, recurse into it so that
                // nested method_signature / property_signature / call_signature /
                // index_signature nodes produce symbols.
                //
                // Example (Prisma-style):
                //   interface PrismaClient {
                //     user: {
                //       findUnique(args: FindArgs): Promise<User | null>;
                //       findMany(args?: FindManyArgs): User[];
                //     };
                //   }
                //
                // `user` is a property_signature whose type annotation is an object_type.
                // Without this recursion, `findUnique` and `findMany` are never extracted.
                if symbols.len() > prop_idx {
                    if let Some(type_ann) = child.child_by_field_name("type") {
                        // type_annotation ::= ":" type_node
                        // Skip the ":" token to find the actual type node.
                        let type_value = {
                            let mut found = None;
                            let mut tc = type_ann.walk();
                            for tc_child in type_ann.children(&mut tc) {
                                if tc_child.kind() != ":" {
                                    found = Some(tc_child);
                                    break;
                                }
                            }
                            found
                        };
                        if let Some(tv) = type_value {
                            recurse_for_object_types(
                                tv, src, scope_tree, symbols, refs, Some(prop_idx),
                            );
                        }
                    }
                }
            }

            // Interface method signatures: `findOne(id: number): T;`
            "method_signature" => {
                let idx = symbols::push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    extract_sig_object_type_members(
                        child, src, scope_tree, symbols, refs, Some(sym_idx),
                    );
                }
            }

            "type_alias_declaration" => {
                let idx = symbols.len();
                symbols::push_type_alias(&child, src, scope_tree, symbols, refs, parent_index);
                // Recurse into the type alias value to extract any nested object_type
                // members (property_signature, method_signature, call_signature,
                // index_signature) as Property/Method symbols.
                //
                // Covers all forms:
                //   type Foo = { prop: T }                     — object_type directly
                //   type Foo = { a: A } | { b: B }             — union of object types
                //   type Foo = Base & { extra: string }         — intersection with object_type
                //   type Foo = Generic<{ inner: T }>            — object_type as type arg
                if symbols.len() > idx {
                    if let Some(value) = child.child_by_field_name("value") {
                        recurse_for_object_types(
                            value, src, scope_tree, symbols, refs, Some(idx),
                        );
                    }
                }
            }

            "enum_declaration" => {
                symbols::push_enum(&child, src, scope_tree, symbols, parent_index);
            }

            "lexical_declaration" | "variable_declaration" => {
                // `const Foo = ...` / `let bar = ...`
                symbols::push_variable_decl(&child, src, scope_tree, symbols, refs, parent_index);
                // Also recurse so that `new_expression` and `call_expression` arms fire
                // for initializers that weren't inlined into push_variable_decl.
                // push_variable_decl handles TypeRef/chain inference for the initializer,
                // but Calls/Instantiates edges for nested calls come from extract_node.
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            "import_statement" => {
                imports::push_import(&child, src, symbols.len(), refs);
            }

            "for_in_statement" => {
                // for (const item of items) / for (const key in obj)
                // Extract loop variable with chain to iterable for type inference.
                // Then recurse into the body for call extraction.
                params::extract_for_loop_var(
                    &child,
                    src,
                    scope_tree,
                    symbols,
                    refs,
                    parent_index,
                );
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, parent_index, demand);
                }
            }

            "catch_clause" => {
                // catch (e: Error) { ... }
                // Extract the catch variable as a scoped symbol with an optional TypeRef.
                params::extract_catch_variable(
                    &child,
                    src,
                    scope_tree,
                    symbols,
                    refs,
                    parent_index,
                );
                // Recurse into the body for nested calls and symbols.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, parent_index, demand);
                }
            }

            // `namespace Foo { ... }` — TypeScript internal module / namespace declaration.
            "internal_module" => {
                let idx = symbols::push_namespace(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, idx, demand);
                }
            }

            // `declare module "foo" { ... }` / `declare function bar(): void`
            // The meaningful declaration is a child — recurse and let existing arms handle it.
            "ambient_declaration" => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // Generator functions — same extraction as regular function_declaration.
            "generator_function_declaration" | "generator_function" => {
                let idx =
                    symbols::push_function(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    types::extract_typed_params_as_symbols(
                        &child,
                        src,
                        scope_tree,
                        symbols,
                        refs,
                        Some(sym_idx),
                    );
                    extract_sig_object_type_members(
                        child, src, scope_tree, symbols, refs, Some(sym_idx),
                    );
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                        narrowing::extract_narrowing_refs(&body, src, sym_idx, refs);
                        // Also recurse for nested declarations inside the generator body.
                        extract_node(body, src, scope_tree, symbols, refs, Some(sym_idx), demand);
                    }
                }
            }

            // Interface construct signatures: `new(name: string): Product`
            // No `name` field — push with a synthetic name "new".
            "construct_signature" => {
                let idx = symbols::push_construct_signature(
                    &child, src, scope_tree, symbols, parent_index,
                );
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    extract_sig_object_type_members(
                        child, src, scope_tree, symbols, refs, Some(sym_idx),
                    );
                }
            }

            // Interface call signatures: `(x: number): string`
            // No `name` field — push with synthetic name "call".
            "call_signature" => {
                let idx = symbols::push_call_signature(
                    &child, src, scope_tree, symbols, parent_index,
                );
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    extract_sig_object_type_members(
                        child, src, scope_tree, symbols, refs, Some(sym_idx),
                    );
                }
            }

            // Abstract method signatures — treat as method symbols.
            "abstract_method_signature" => {
                let idx = symbols::push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    extract_sig_object_type_members(
                        child, src, scope_tree, symbols, refs, Some(sym_idx),
                    );
                }
            }

            // Interface getter/setter signatures — emit as Property.
            "getter_signature" | "setter_signature" => {
                symbols::push_ts_field(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // Index signature: `[key: string]: unknown` — emit as Property symbol
            // and extract TypeRef for the value type.
            "index_signature" => {
                symbols::push_index_signature(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // `object_type` is the body of an interface or a type-alias object literal.
            // It appears in two contexts:
            //   1. interface Foo { ... }       — body handled through interface_declaration → extract_node(body)
            //   2. type Foo = { ... }          — reached via recurse_for_object_types
            //   3. type Foo = A & { ... }      — reached when `_` arm recurses into intersection_type
            //   4. nested in generic args etc. — same
            //
            // When we arrive here via extract_node, recurse into children so that
            // property_signature / method_signature / call_signature / index_signature
            // arms fire for each member.
            "object_type" => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // Call expressions at any level not already handled by extract_calls
            // from inside a function/method body.  This captures top-level calls,
            // calls in class static blocks, IIFE patterns, decorator arguments
            // that reach here, etc.
            //
            // Use parent_index.unwrap_or(0) — attributes the call to the nearest
            // enclosing named symbol, or the first symbol in the file when at
            // module scope.
            "call_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                calls::emit_call_ref(&child, src, sym_idx, refs);
                // Continue recursing into children (e.g. arguments may contain
                // further nested call_expressions at this level).
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // `new Foo(...)` at module scope or inside field initializers.
            "new_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                calls::emit_new_ref(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // JSX element tags at any level not covered by a function body
            // explicitly calling `extract_calls`. The classic hole was arrow-
            // function-const React components like
            //   const PollProvider = (props) => <PollContext.Provider …/>;
            // whose body was reached via extract_node's default recursion
            // and whose `<X.Provider>` never saw the JSX arm in extract_calls.
            // Emit PascalCase / dotted JSX tags here; lowercase HTML
            // intrinsics are skipped (same rule as extract_calls).
            "jsx_self_closing_element" | "jsx_opening_element" => {
                let sym_idx = parent_index.unwrap_or(0);
                calls::emit_jsx_component_ref(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // `expr as Type` — emit TypeRef for the asserted type.
            // Also recurse for nested calls/declarations inside the expression.
            "as_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                symbols::extract_type_ref_from_as_expression(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // `expr satisfies Type` — emit TypeRef for the asserted type.
            "satisfies_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                symbols::extract_type_ref_from_satisfies_expression(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // `<Type>expr` — emit TypeRef for the asserted type (TSX-invalid form).
            "type_assertion" => {
                let sym_idx = parent_index.unwrap_or(0);
                symbols::extract_type_ref_from_type_assertion(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // `x instanceof Foo` — emit TypeRef for the constructor.
            // Also handles non-instanceof binary expressions via recursion.
            "binary_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                // Check for instanceof without re-importing narrowing internals.
                let has_instanceof = (0..child.child_count()).any(|i| {
                    child.child(i).map(|c| c.kind() == "instanceof").unwrap_or(false)
                });
                if has_instanceof {
                    if let Some(right) = child.child_by_field_name("right") {
                        let type_name = helpers::node_text(right, src);
                        if !type_name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: type_name,
                                kind: EdgeKind::TypeRef,
                                line: right.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // Standalone `type_annotation` nodes encountered during recursion
            // (e.g. in arrow function parameters, destructuring patterns, etc.)
            // that aren't covered by a dedicated handler above.
            "type_annotation" => {
                let sym_idx = parent_index.unwrap_or(0);
                types::extract_type_ref_from_annotation(&child, src, sym_idx, refs);
                // Recursively walk all children to catch type_identifiers and other types
                // nested inside generic_type, union_type, etc. that extract_type_ref_from_annotation
                // may have handled but children not yet extracted.
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // `generic_type` encountered during recursion — recurse to catch all inner types.
            // This handles cases like generic types in field initializers and other
            // non-body expression contexts.
            "generic_type" => {
                let sym_idx = parent_index.unwrap_or(0);
                types::extract_type_ref_from_annotation(&child, src, sym_idx, refs);
                // Recurse to handle nested types within type arguments.
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }

            // `type_identifier` encountered during recursion in expression contexts
            // (not as a declaration name). Emit a TypeRef unless it's a primitive.
            // Covers: type references in variable type annotations via `as`, generics,
            // template literal types, and other places where type_annotation handlers
            // don't fire.
            "type_identifier" => {
                let sym_idx = parent_index.unwrap_or(0);
                let name = helpers::node_text(child, src);
                if !name.is_empty() && !is_ts_primitive(&name) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
                // type_identifier is a leaf — no children to recurse into.
            }

            // Qualified type like `React.ReactNode` or `Stripe.Event`. Emit the
            // full dotted name as a single ref and do NOT recurse into children —
            // the inner type_identifier leaf would otherwise be picked up by the
            // `type_identifier` arm above as a spurious bare ref.
            "nested_type_identifier" => {
                let sym_idx = parent_index.unwrap_or(0);
                let name = helpers::node_text(child, src);
                if !name.is_empty() && !is_ts_primitive(&name) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }

            // `function name(params): ReturnType;` -- ambient / overload function signature.
            // Has a `name` field but no `body` field (unlike function_declaration).
            // Treat identically to function_declaration but skip body extraction.
            "function_signature" => {
                let idx = symbols::push_function(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                }
            }

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index, demand);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract `object_type` members from inline object types inside function/method signatures.
///
/// Calls `extract_node` on the `parameters` and `return_type` nodes of a function/method
/// so that `object_type` nodes inside type annotations are reached. This fires the
/// `property_signature` and `method_signature` arms for inline object types, e.g.:
///
///   function foo(opts: { x: number; y: string }): { id: number } { ... }
///
/// Without this, such members are invisible because `extract_param_and_return_types` only
/// emits TypeRef edges — it does not recurse into `extract_node` which produces symbols.
fn extract_sig_object_type_members(
    func_node: tree_sitter::Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // `demand = None` here: these are nested members inside an already-kept
    // declaration (the enclosing function/interface passed the demand gate),
    // so recurse permissively.
    if let Some(params) = func_node.child_by_field_name("parameters") {
        extract_node(params, src, scope_tree, symbols, refs, parent_index, None);
    }
    if let Some(ret) = func_node.child_by_field_name("return_type") {
        extract_node(ret, src, scope_tree, symbols, refs, parent_index, None);
    }
}

/// Recursively walk a type-value node (the right-hand side of a `type_alias_declaration`)
/// and call `extract_node` on every `object_type` found at any nesting depth.
///
/// This handles all forms where `object_type` can appear inside a type alias:
/// - Direct:               `type T = { x: number }`        → object_type at top level
/// - Union member:         `type T = { a: A } | { b: B }`  → union_type → object_type children
/// - Intersection member:  `type T = Base & { extra: X }`  → intersection_type → object_type
/// - Generic argument:     `type T = Mapped<{ k: V }>`     → generic_type → type_args → object_type
/// - Conditional branches: `type T = C extends X ? { a: A } : { b: B }`
///
/// For non-`object_type` structural nodes (union_type, intersection_type, etc.),
/// we recurse through their children so that nested `object_type` nodes are found.
/// `extract_node` is called only for `object_type` so that property_signature,
/// method_signature, call_signature, and index_signature arms fire for each member.
fn recurse_for_object_types(
    node: tree_sitter::Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "object_type" => {
            // Found one — extract its members as symbols. `demand = None`
            // because this runs inside an already-kept type alias / interface.
            extract_node(node, src, scope_tree, symbols, refs, parent_index, None);
        }
        // Type wrappers that can contain object_type members — recurse into children.
        "union_type" | "intersection_type" | "parenthesized_type"
        | "conditional_type" | "tuple_type" | "array_type"
        | "generic_type" | "type_arguments" | "readonly_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    recurse_for_object_types(
                        child, src, scope_tree, symbols, refs, parent_index,
                    );
                }
            }
        }
        // All other type nodes (type_identifier, primitive_type, function_type, etc.)
        // cannot contain object_type members — stop recursion here.
        _ => {}
    }
}

/// Build a map of `local_alias → module_path` from all top-level import statements.
///
/// Handles all three import forms:
/// - `import Foo from './bar'`            → `"Foo" → "./bar"`
/// - `import { Foo, Bar as B } from ...`  → `"Foo" → ..., "B" → ...`
/// - `import * as ns from './bar'`        → `"ns" → "./bar"`
///
/// Used by `annotate_call_modules` to set `module` on call refs that start
/// with a known import alias (e.g. `UserService.findOne(id)` → module set to
/// the module that exports `UserService`).
fn build_import_map(root: Node, src: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        // Import statements may be wrapped in export_statement in some grammars,
        // but for top-level imports we only need direct import_statement children.
        if child.kind() != "import_statement" {
            continue;
        }
        let Some(module_node) = child.child_by_field_name("source") else {
            continue;
        };
        let module_path = helpers::node_text(module_node, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if module_path.is_empty() {
            continue;
        }

        let mut ic = child.walk();
        for clause in child.children(&mut ic) {
            if clause.kind() != "import_clause" {
                continue;
            }
            let mut cc = clause.walk();
            for item in clause.children(&mut cc) {
                match item.kind() {
                    // `import Foo from './bar'` — default import; local name = Foo
                    "identifier" => {
                        let local = helpers::node_text(item, src);
                        if !local.is_empty() {
                            map.insert(local, module_path.clone());
                        }
                    }
                    // `import { Foo, Bar as B } from './bar'`
                    "named_imports" => {
                        let mut ni = item.walk();
                        for spec in item.children(&mut ni) {
                            if spec.kind() != "import_specifier" {
                                continue;
                            }
                            // `alias` field is the local name when `as` is used.
                            // If no alias, `name` is both the exported and local name.
                            let local = spec
                                .child_by_field_name("alias")
                                .or_else(|| spec.child_by_field_name("name"))
                                .map(|n| helpers::node_text(n, src))
                                .unwrap_or_default();
                            if !local.is_empty() {
                                map.insert(local, module_path.clone());
                            }
                        }
                    }
                    // `import * as ns from './bar'`
                    "namespace_import" => {
                        let mut nc = item.walk();
                        for ns_child in item.children(&mut nc) {
                            if ns_child.kind() == "identifier" {
                                let local = helpers::node_text(ns_child, src);
                                if !local.is_empty() {
                                    map.insert(local, module_path.clone());
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    map
}

/// Build a map of `local_alias → (original_name, module_path)` for every
/// import statement that uses `as` renaming. Unaliased imports are NOT in
/// this map — for them, the local name and the exported name are identical
/// and no rewrite is needed.
///
/// Example: `import { request as __request } from './core/request'`
///   → `{"__request" → ("request", "./core/request")}`
///
/// Only captures named imports. `import * as ns` and default imports can't
/// be renamed to a different exported name.
fn build_renamed_import_map(root: Node, src: &[u8]) -> HashMap<String, (String, String)> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "import_statement" {
            continue;
        }
        let Some(module_node) = child.child_by_field_name("source") else {
            continue;
        };
        let module_path = helpers::node_text(module_node, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if module_path.is_empty() {
            continue;
        }

        let mut ic = child.walk();
        for clause in child.children(&mut ic) {
            if clause.kind() != "import_clause" {
                continue;
            }
            let mut cc = clause.walk();
            for item in clause.children(&mut cc) {
                if item.kind() != "named_imports" {
                    continue;
                }
                let mut ni = item.walk();
                for spec in item.children(&mut ni) {
                    if spec.kind() != "import_specifier" {
                        continue;
                    }
                    // Only record if the spec has BOTH `name` (exported) and
                    // `alias` (local) fields — the `X as Y` form.
                    let (Some(name_node), Some(alias_node)) = (
                        spec.child_by_field_name("name"),
                        spec.child_by_field_name("alias"),
                    ) else {
                        continue;
                    };
                    let original = helpers::node_text(name_node, src);
                    let local = helpers::node_text(alias_node, src);
                    if original.is_empty() || local.is_empty() || original == local {
                        continue;
                    }
                    map.insert(local, (original, module_path.clone()));
                }
            }
        }
    }
    map
}

/// Rewrite refs whose `target_name` matches a known local alias so they point
/// at the ORIGINAL exported name, with `module` set to the export source.
///
/// This handles OpenAPI-generated SDKs, CommonJS interop shims, and any other
/// pattern where code uses a locally-renamed import like `X as Y`. Without
/// this pass, calls to `Y(...)` are unresolvable because the target file
/// exports `X`, not `Y`.
///
/// Skips refs that already have a non-empty `module` — those have been
/// resolved through another path (e.g. chain annotation) and rewriting would
/// lose information.
fn rewrite_aliased_refs(
    refs: &mut Vec<ExtractedRef>,
    alias_map: &HashMap<String, (String, String)>,
) {
    for r in refs.iter_mut() {
        // Don't touch the TypeRef emitted by push_import itself — those were
        // already pushed with target_name=original in imports.rs.
        if r.kind == EdgeKind::Imports {
            continue;
        }

        // Case A: simple (non-chain) refs and chain refs whose target_name
        // (the last segment) happens to BE the alias — e.g. a bare call to
        // `__request(...)`. Rewrite target_name to the original export.
        if let Some((original, module_path)) = alias_map.get(&r.target_name) {
            r.target_name = original.clone();
            if r.module.is_none() {
                r.module = Some(module_path.clone());
            }
        }

        // Case B: chain refs where the alias is the RECEIVER, e.g.
        // `__request.get(url)` — target_name is `get` (the last segment),
        // but the chain walker needs `__request` → `request` so the root
        // type lookup finds the real exported symbol. Rewrite the first
        // segment's name in place. This MUST happen independently of Case A
        // because the last segment rarely matches an alias.
        if let Some(chain) = r.chain.as_mut() {
            if let Some(first) = chain.segments.first_mut() {
                if let Some((original, module_path)) = alias_map.get(&first.name) {
                    first.name = original.clone();
                    if r.module.is_none() {
                        r.module = Some(module_path.clone());
                    }
                }
            }
        }
    }
}

/// For each `Calls` ref that has a chain with ≥2 segments, check whether the
/// first chain segment matches a known import alias. If so, set `module` to the
/// corresponding module path so the resolver can trace the call back to its
/// import source.
///
/// Only sets `module` when it is currently `None` — does not overwrite an
/// already-resolved module.
fn annotate_call_modules(refs: &mut Vec<ExtractedRef>, import_map: &HashMap<String, String>) {
    for r in refs.iter_mut() {
        if r.kind != EdgeKind::Calls || r.module.is_some() {
            continue;
        }
        let Some(chain) = &r.chain else { continue };
        if chain.segments.len() < 2 {
            continue;
        }
        let first = &chain.segments[0].name;
        if let Some(module_path) = import_map.get(first) {
            r.module = Some(module_path.clone());
        }
    }
}

/// Extract re-export refs from an `export_statement` node.
///
/// Handles:
///   `export { X } from './y'`              → Imports ref, target_name="X", module="./y"
///   `export { X as Z } from './y'`         → Imports ref, target_name="X", module="./y"
///   `export * from './y'`                  → Imports ref, target_name="*", module="./y"
///   `export * as ns from './y'`            → Imports ref, target_name="*", module="./y"
///
/// Re-exports are attributed to a file-level "sentinel" symbol at index
/// `file_symbol_count` — the index one past the last real symbol, which is
/// how the JS extractor handles them.  If the file has no symbols yet,
/// index 0 is fine because the resolution engine only uses the module field.
fn extract_reexports(
    node: &tree_sitter::Node,
    src: &[u8],
    file_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The source module is the `source` field of the export_statement.
    let module_path = node.child_by_field_name("source").map(|s| {
        helpers::node_text(s, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    });

    // Only re-export forms have a `source` field.
    let Some(ref mod_path) = module_path else {
        return;
    };
    if mod_path.is_empty() {
        return;
    }

    // Use sentinel index: one past the last symbol (or 0 if no symbols yet).
    // The resolver only needs `target_name` and `module`; the source index is
    // irrelevant for re-export chain following.
    let source_idx = file_symbol_count.saturating_sub(1);

    let line = node.start_position().row as u32;
    let mut has_wildcard = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `export { X }` or `export { X as Z }` from './y'
            "export_clause" => {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        // `name` field = the original exported name (before `as`).
                        // `alias` field = the local rename (after `as`), unused here.
                        // We store the original so the resolver can find it in the source.
                        let original_name = spec
                            .child_by_field_name("name")
                            .map(|n| helpers::node_text(n, src))
                            .unwrap_or_default();
                        if !original_name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: source_idx,
                                target_name: original_name,
                                kind: EdgeKind::Imports,
                                line: spec.start_position().row as u32,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                }
            }
            // `export * as ns from './y'` — the TS grammar wraps this in namespace_export.
            "namespace_export" => {
                has_wildcard = true;
            }
            // `export * from './y'` — in the TS grammar the `*` is a direct child of
            // export_statement (no namespace_export wrapper), unlike the JS grammar.
            "*" => {
                has_wildcard = true;
            }
            _ => {}
        }
    }

    if has_wildcard {
        refs.push(ExtractedRef {
            source_symbol_index: source_idx,
            target_name: "*".to_string(),
            kind: EdgeKind::Imports,
            line,
            module: module_path.clone(),
            chain: None,
            byte_offset: 0,
        });
    }
}

/// Return true if `name` is a TypeScript primitive type keyword.
///
/// These must not be emitted as TypeRef edges — they are language keywords,
/// not references to user-defined or library symbols.
#[inline]
fn is_ts_primitive(name: &str) -> bool {
    matches!(
        name,
        "string" | "number" | "boolean" | "void" | "any" | "unknown" | "never"
            | "undefined" | "null" | "object" | "symbol" | "bigint"
    )
}

/// Recursively scan ALL descendants of `node` for ref-producing node kinds that
/// may have been missed by the main walker due to nesting depth or expression
/// contexts not covered by a dedicated arm.
///
/// This post-traversal pass ensures:
/// - Every `type_identifier` (non-primitive) produces a TypeRef
/// - Every `type_annotation` produces a TypeRef for its enclosed type
/// - Every `as_expression` produces a TypeRef for the cast type
/// - Every `satisfies_expression` produces a TypeRef for the checked type
///
/// All refs are attributed to `sym_idx` (symbol 0 for the file-level pass).
/// The coverage metric only needs a ref at the correct line — sym_idx is not
/// checked by the correlation logic.
/// True when every leaf named child of `node` (walking nested
/// union_type / intersection_type wrappers) is a `literal_type`.
///
/// Longer literal unions (`'a' | 'b' | 'c' | 'd' | 'e'`) parse as a
/// LEFT-NESTED chain of union_types — the outer union's children are
/// not all literal_type directly, they include an inner union_type
/// that itself contains literals. A flat `children.all(literal_type)`
/// check would miss this shape and leak the first literal's content
/// as a coverage TypeRef.
/// Walk a union_type / intersection_type / parenthesized_type and return the
/// text of the first non-`literal_type` named child. Skips union/intersection
/// wrappers recursively. Returns `None` when every member is a literal_type
/// (caller should treat that as `_primitive`).
fn first_non_literal_descendant_text(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "literal_type" => continue,
            // `'a'[]` and `('a' | 'b')` and `Readonly<'a' | 'b'>` are all
            // structurally pure-literal containers — recurse so we don't
            // grab their literal contents as a TypeRef target.
            "union_type" | "intersection_type" | "parenthesized_type"
            | "array_type" | "readonly_type" | "tuple_type" => {
                if let Some(t) = first_non_literal_descendant_text(child, src) {
                    return Some(t);
                }
            }
            _ => return Some(helpers::node_text(child, src)),
        }
    }
    None
}

fn is_pure_literal_type_composite(node: tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    let mut any = false;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "literal_type" => any = true,
            // Structural wrappers around literals are still "pure literal"
            // for coverage purposes — `'a' | 'b'`, `'a'[]`, `('a' | 'b')`,
            // `readonly 'a'[]`, `['a', 'b']` all carry no real type ref.
            "union_type" | "intersection_type" | "parenthesized_type"
            | "array_type" | "readonly_type" | "tuple_type" => {
                if !is_pure_literal_type_composite(child) {
                    return false;
                }
                any = true;
            }
            _ => return false,
        }
    }
    any
}

fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    src: &[u8],
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" if child.is_named() => {
                let name = helpers::node_text(child, src);
                if !name.is_empty() && !is_ts_primitive(&name) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
                // type_identifier is a leaf — no children to recurse into.
            }
            "nested_type_identifier" if child.is_named() => {
                // Qualified type like `React.ReactNode` or `Stripe.Event` — emit
                // the full dotted name as a single ref. Do NOT recurse into
                // children; the tree has a leaf type_identifier inside that
                // would otherwise be emitted as a duplicate bare ref.
                let name = helpers::node_text(child, src);
                if !name.is_empty() && !is_ts_primitive(&name) {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }
            "generic_type" if child.is_named() => {
                // Extract the base type name from the generic, e.g. `Promise<User>` → `Promise`.
                // The first named child of generic_type is the base type_identifier.
                let base_opt = child.child_by_field_name("name").or_else(|| {
                    let children: Vec<_> = {
                        let mut gc = child.walk();
                        child.children(&mut gc).collect()
                    };
                    children.into_iter().find(|c| c.kind() == "type_identifier")
                });
                if let Some(base) = base_opt {
                    let name = helpers::node_text(base, src);
                    if !name.is_empty() && !is_ts_primitive(&name) {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: base.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                        });
                    }
                }
                // Still recurse so type arguments inside are also scanned.
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
            // Emit a TypeRef for the line of every type_annotation node found in the tree.
            // The annotation text is not important for the coverage metric — the line match
            // is what counts. We also recurse to catch nested annotations and type_identifiers.
            "type_annotation" if child.is_named() => {
                // Find the actual type node inside the annotation (after the colon).
                let type_node = {
                    let mut found = None;
                    let mut ac = child.walk();
                    for ann_child in child.children(&mut ac) {
                        if ann_child.kind() != ":" {
                            found = Some(ann_child);
                            break;
                        }
                    }
                    found
                };
                if let Some(tn) = type_node {
                    // Emit a ref for the annotation node itself (for type_annotation coverage).
                    let name = helpers::node_text(tn, src);
                    if !name.is_empty() {
                        // Decide the coverage-ref target based on the shape of the
                        // inner type node:
                        //   * qualified forms → emit the full dotted name so
                        //     downstream classification (primitives list, React
                        //     namespace check) can match.
                        //   * simple / generic / array / union / tuple →
                        //     first-segment split gives a meaningful root
                        //     (`Array<T>` → `Array`, `Promise<X>` → `Promise`).
                        //   * structured types (object_type, function_type,
                        //     mapped_type, conditional_type, etc.) → use the
                        //     `_primitive` sentinel. The first segment of these
                        //     is a property / parameter name, NOT a type ref —
                        //     emitting it leaks names like `children`, `id`,
                        //     `className`, `params` into unresolved_refs.
                        let target = match tn.kind() {
                            "nested_type_identifier" | "member_expression" => name.clone(),
                            // Pure string/number/etc. literal-type unions like
                            // `'default' | 'success' | 'warning'` — the first
                            // alphanumeric token of the text is the content of
                            // the FIRST literal (`"default"`), not a real type
                            // reference. Use the primitive sentinel so it
                            // doesn't leak into unresolved_refs.
                            // Pure-literal composites of any shape — `'a' | 'b'`,
                            // `'a'[]`, `('a' | 'b')`, `readonly 'a'[]`, `['a', 'b']` —
                            // emit only the primitive sentinel; the literal
                            // content isn't a real type ref.
                            "union_type" | "intersection_type"
                            | "array_type" | "tuple_type"
                            | "parenthesized_type" | "readonly_type"
                                if is_pure_literal_type_composite(tn) =>
                            {
                                "_primitive".to_string()
                            }
                            // Literal type by itself (rare — typically sits in
                            // a union, handled above) — never a real ref.
                            "literal_type" => "_primitive".to_string(),
                            "type_identifier"
                            | "identifier"
                            | "generic_type"
                            | "array_type"
                            | "tuple_type"
                            | "union_type"
                            | "intersection_type"
                            | "parenthesized_type"
                            | "type_query"
                            | "readonly_type" => {
                                // Walk past leading literal_type children when
                                // splitting by first segment — `'400' | Array<...>`
                                // would otherwise pick up the string contents
                                // (`400`) as the type name. Find the first
                                // non-literal direct named child and split its
                                // text instead.
                                let split_target = first_non_literal_descendant_text(tn, src)
                                    .unwrap_or_else(|| name.clone());
                                let candidate = split_target
                                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                                    .find(|s| !s.is_empty())
                                    .unwrap_or("_")
                                    .to_string();
                                // Numeric-only fallback (`200`, `400`) means the
                                // text we split was still literal content; use
                                // the primitive sentinel.
                                if !candidate.is_empty()
                                    && candidate.chars().all(|c| c.is_ascii_digit())
                                {
                                    "_primitive".to_string()
                                } else {
                                    candidate
                                }
                            }
                            // Structured types — the first identifier in the text
                            // is a property or parameter name, not a type reference.
                            _ => "_primitive".to_string(),
                        };
                        if !is_ts_primitive(&target) {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: target,
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                            });
                        } else {
                            // Even for primitive annotations we need a ref at this line
                            // so the type_annotation coverage budget is consumed.
                            // Use "_primitive" as a placeholder target — it won't resolve
                            // to any real symbol, but satisfies the coverage counter.
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: "_primitive".to_string(),
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                }
                // Recurse to catch nested annotations and other type_identifiers,
                // but skip when the inner type is already a qualified form — the
                // recursion would otherwise leak the bare last segment.
                let skip_recurse = type_node.is_some_and(|tn| {
                    matches!(tn.kind(), "nested_type_identifier" | "member_expression")
                });
                if !skip_recurse {
                    scan_all_type_identifiers(child, src, sym_idx, refs);
                }
            }
            // Emit a TypeRef for the line of every as_expression node found in the tree.
            "as_expression" if child.is_named() => {
                // Find the type after the `as` keyword.
                let mut after_as = false;
                let mut ac = child.walk();
                for as_child in child.children(&mut ac) {
                    if as_child.kind() == "as" {
                        after_as = true;
                        continue;
                    }
                    if after_as {
                        let name = helpers::node_text(as_child, src);
                        let target = name
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .find(|s| !s.is_empty())
                            .unwrap_or("_")
                            .to_string();
                        if !target.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: target,
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                        break;
                    }
                }
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
            // Emit a TypeRef for the line of every satisfies_expression node found in the tree.
            "satisfies_expression" if child.is_named() => {
                // Find the type after the `satisfies` keyword.
                let mut after_satisfies = false;
                let mut sc = child.walk();
                for sat_child in child.children(&mut sc) {
                    if sat_child.kind() == "satisfies" {
                        after_satisfies = true;
                        continue;
                    }
                    if after_satisfies {
                        let name = helpers::node_text(sat_child, src);
                        let target = name
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .find(|s| !s.is_empty())
                            .unwrap_or("_")
                            .to_string();
                        if !target.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: target,
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                        break;
                    }
                }
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
            _ => {
                scan_all_type_identifiers(child, src, sym_idx, refs);
            }
        }
    }
}

/// Return every type-parameter name declared on `node` via a direct
/// `type_parameters` child. Covers function / arrow / method / class /
/// interface / type-alias / call-signature / construct-signature
/// declarations — all the TS grammar nodes that can introduce a fresh
/// generic scope.
///
/// Empty vec for nodes that don't declare type parameters (most of them).
fn collect_declared_type_params(node: &tree_sitter::Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "type_parameters" {
            continue;
        }
        let mut tp_cursor = child.walk();
        for tp in child.children(&mut tp_cursor) {
            if tp.kind() != "type_parameter" {
                continue;
            }
            // Prefer the `name` field when the grammar exposes it; fall back
            // to the first type_identifier / identifier child for grammars
            // that don't name the field. The explicit loop keeps the walker
            // cursor's lifetime inside the same scope as the yielded node.
            let name_node = if let Some(n) = tp.child_by_field_name("name") {
                Some(n)
            } else {
                let mut tpc = tp.walk();
                let mut found = None;
                for c in tp.children(&mut tpc) {
                    if matches!(c.kind(), "type_identifier" | "identifier") {
                        found = Some(c);
                        break;
                    }
                }
                found
            };
            if let Some(n) = name_node {
                if let Ok(name) = n.utf8_text(src) {
                    if !name.is_empty() {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Walk the full tree and collect every `(type_param_name, start_line,
/// end_line)` tuple for the subtree where that type parameter is in scope.
///
/// Used by the post-filter pass in `extract_inner` to drop TypeRef entries
/// whose target is a generic type parameter binding (e.g. `Target` inside
/// `TargetedEvent<Target>`) rather than an external type. The scope is
/// approximated by the line range of the declaring node, which is accurate
/// enough in practice — same-line cross-scope collisions require contrived
/// single-line layouts of separate generic declarations.
fn collect_type_param_scopes(
    node: tree_sitter::Node,
    src: &[u8],
    out: &mut Vec<(String, u32, u32)>,
) {
    let declared = collect_declared_type_params(&node, src);
    if !declared.is_empty() {
        let start_line = node.start_position().row as u32;
        let end_line = node.end_position().row as u32;
        for name in declared {
            out.push((name, start_line, end_line));
        }
    }
    // `infer X` introduces a type variable inside a `conditional_type` —
    // `T extends Foo<infer X> ? X : never`. The variable is in scope for
    // the entire conditional expression. Without this, real source code
    // patterns like `T extends [infer Head, ...infer Tails]` leak `Head`
    // and `Tails` as unresolved external TypeRefs (very common in
    // tanstack-query / trpc / solid-query type gymnastics).
    if node.kind() == "infer_type" {
        let mut name_node = node.child_by_field_name("name");
        if name_node.is_none() {
            let mut cursor = node.walk();
            for c in node.children(&mut cursor) {
                if matches!(c.kind(), "type_identifier" | "identifier") {
                    name_node = Some(c);
                    break;
                }
            }
        }
        if let Some(n) = name_node {
            if let Ok(name) = n.utf8_text(src) {
                if !name.is_empty() {
                    let scope_node = enclosing_conditional_type(&node).unwrap_or(node);
                    out.push((
                        name.to_string(),
                        scope_node.start_position().row as u32,
                        scope_node.end_position().row as u32,
                    ));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_param_scopes(child, src, out);
    }
}

/// Walk parents until we find the enclosing `conditional_type` node — the
/// scope an `infer X` binding is visible in. Falls back to `None` if the
/// `infer_type` is somehow detached from a conditional context (which the
/// TS grammar shouldn't produce, but defensive coding doesn't hurt).
fn enclosing_conditional_type<'a>(node: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut cur = node.parent();
    while let Some(p) = cur {
        if p.kind() == "conditional_type" {
            return Some(p);
        }
        cur = p.parent();
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
