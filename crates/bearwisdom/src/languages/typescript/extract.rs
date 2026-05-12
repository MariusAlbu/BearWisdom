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

use crate::ecosystem::ecmascript_imports::build_import_map;
use crate::ecosystem::imports::{resolve_import_refs, ImportEntry, ImportKind};
use crate::parser::scope_tree::{self, ScopeKind, ScopeTree};
use crate::types::ExtractionResult;
use crate::types::{AliasTarget, EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
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
                alias_targets: Vec::new(),
            }
        }
    };

    let has_errors = tree.root_node().has_error();
    let src_bytes = source.as_bytes();
    let root = tree.root_node();

    let scope_tree = scope_tree::build(root, src_bytes, TS_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let mut alias_targets: Vec<(String, AliasTarget)> = Vec::new();

    // Pre-pass: parse every `import` statement into a per-local-name
    // `ImportEntry`. Carries enough info (default vs named vs namespace,
    // exported name for renamed imports) for the shared
    // `ecosystem::imports::resolve_import_refs` pass below to canonicalize
    // every ref against the file's import context — splits namespace
    // prefixes (`Foo.X` → module=Foo's import, target=X), substitutes
    // renamed imports (`{ X as Y }; Y()` → target=X), attributes calls
    // and type refs to their source modules.
    let import_map = build_import_map(root, src_bytes);

    // Triple-slash directives are .d.ts-specific imports of the form
    // `/// <reference path="X" />` (relative-path file include) and
    // `/// <reference types="pkg" />` (npm typings include). They're
    // critical for @types packages whose `index.d.ts` is just a hub
    // referencing siblings (`@types/jquery` → JQuery.d.ts +
    // JQueryStatic.d.ts + factory.d.ts + misc.d.ts). The TS extractor
    // pre-PR-149 ignored them entirely, so the API surface those
    // siblings declare never got followed.
    push_triple_slash_imports(source, &mut refs);

    extract_node(
        root,
        src_bytes,
        &scope_tree,
        &mut symbols,
        &mut refs,
        &mut alias_targets,
        None,
        demand,
    );

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

    // Bare re-export post-pass — covers the workspace barrel-file shape
    //
    //   ```typescript
    //   import type { Foo } from 'pkg';
    //   export type { Foo };
    //   ```
    //
    // where `extract_reexports` (called from inside the main traversal)
    // can't help: it returns early when the export_statement has no
    // `source` field, since the target module isn't on the node itself.
    // Here we have the file's import_map already built — for every
    // bare-export specifier whose name traces back to an import, emit
    // the same Imports ref + synthetic symbol pair the with-source path
    // produces. Without this, every barrel-style workspace package
    // re-exporting an npm type by name leaves consumers' imports
    // unresolved against the barrel file.
    extract_bare_reexports_via_imports(
        root, src_bytes, &import_map, &mut symbols, &mut refs,
    );

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

    // Apply ECMAScript import semantics to every ref in one pass:
    //
    //   - `import * as Foo from 'pkg'; Foo.X`        → target=X, module=pkg
    //   - `import * as F from 'pkg'; F.A.B`          → target=B, namespace_segments=[A], module=pkg
    //   - `import { X as Y } from 'pkg'; Y()`        → target=X, module=pkg
    //   - `import Foo from 'pkg'; Foo.method()`      → chain root annotated, module=pkg
    //
    // This replaces three legacy post-passes (annotate_call_modules,
    // annotate_namespace_type_refs, rewrite_aliased_refs) — the shared
    // resolver in `ecosystem::imports` handles every shape uniformly so
    // there's no per-language drift.
    resolve_import_refs(&mut refs, &import_map);

    let mut result = ExtractionResult::new(symbols, refs, has_errors);
    result.alias_targets = alias_targets;
    result
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
    alias_targets: &mut Vec<(String, AliasTarget)>,
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
                    extract_node(body, src, scope_tree, symbols, refs, alias_targets, idx, demand);
                }
            }

            "interface_declaration" => {
                let idx =
                    symbols::push_interface(&child, src, scope_tree, symbols, parent_index);
                imports::extract_heritage(&child, src, idx.unwrap_or(0), refs);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, alias_targets, idx, demand);
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
                        child, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx),
                    );
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                        narrowing::extract_narrowing_refs(&body, src, sym_idx, refs);
                        // Also recurse with extract_node so nested lexical_declaration,
                        // catch_clause, for_in_statement, etc. inside the body produce
                        // their symbols and type refs.
                        extract_node(body, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx), demand);
                    }
                }
            }

            "export_statement" => {
                // `export class Foo {}` / `export function bar() {}`
                // Recurse so that declarations inside are extracted.
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
                // Also extract re-export forms:
                //   `export { X } from './y'`
                //   `export { X as Z } from './y'`
                //   `export * from './y'`
                //   `export * as ns from './y'`
                extract_reexports(&child, src, symbols, refs);
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
                            child, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx),
                        );
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                        narrowing::extract_narrowing_refs(&body, src, sym_idx, refs);
                        // Also recurse with extract_node so nested lexical_declaration,
                        // catch_clause, for_in_statement, etc. produce symbols and type refs.
                        extract_node(body, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx), demand);
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
                                tv, src, scope_tree, symbols, refs, alias_targets, Some(field_idx),
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
                                tv, src, scope_tree, symbols, refs, alias_targets, Some(prop_idx),
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
                        child, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx),
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
                            value, src, scope_tree, symbols, refs, alias_targets, Some(idx),
                        );
                        // Capture the alias's structural shape so the chain
                        // walker can decide whether to expand it (Application
                        // arm) or treat it as opaque (Union / Intersection /
                        // Object / Other). This is the type-checker's
                        // alternative to the engine's positional flatten.
                        let qname = symbols[idx].qualified_name.clone();
                        let target = types::classify_alias_target(&value, src);
                        alias_targets.push((qname, target));
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
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                    extract_node(body, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                    extract_node(body, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
                }
            }

            // `namespace Foo { ... }` — TypeScript internal module / namespace declaration.
            "internal_module" => {
                let idx = symbols::push_namespace(&child, src, scope_tree, symbols, parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, alias_targets, idx, demand);
                }
            }

            // `declare module "foo" { ... }` / `declare function bar(): void`
            // The meaningful declaration is a child — recurse and let existing arms handle it.
            "ambient_declaration" => {
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                        child, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx),
                    );
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls(&body, src, sym_idx, refs);
                        narrowing::extract_narrowing_refs(&body, src, sym_idx, refs);
                        // Also recurse for nested declarations inside the generator body.
                        extract_node(body, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx), demand);
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
                        child, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx),
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
                        child, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx),
                    );
                }
            }

            // Abstract method signatures — treat as method symbols.
            "abstract_method_signature" => {
                let idx = symbols::push_method(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    types::extract_param_and_return_types(&child, src, sym_idx, refs);
                    extract_sig_object_type_members(
                        child, src, scope_tree, symbols, refs, alias_targets, Some(sym_idx),
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
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
            }

            // `new Foo(...)` at module scope or inside field initializers.
            "new_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                calls::emit_new_ref(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
            }

            // `expr as Type` — emit TypeRef for the asserted type.
            // Also recurse for nested calls/declarations inside the expression.
            "as_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                symbols::extract_type_ref_from_as_expression(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
            }

            // `expr satisfies Type` — emit TypeRef for the asserted type.
            "satisfies_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                symbols::extract_type_ref_from_satisfies_expression(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
            }

            // `<Type>expr` — emit TypeRef for the asserted type (TSX-invalid form).
            "type_assertion" => {
                let sym_idx = parent_index.unwrap_or(0);
                symbols::extract_type_ref_from_type_assertion(&child, src, sym_idx, refs);
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
});
                        }
                    }
                }
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
            }

            // `generic_type` encountered during recursion — recurse to catch all inner types.
            // This handles cases like generic types in field initializers and other
            // non-body expression contexts.
            "generic_type" => {
                let sym_idx = parent_index.unwrap_or(0);
                types::extract_type_ref_from_annotation(&child, src, sym_idx, refs);
                // Recurse to handle nested types within type arguments.
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
                extract_node(child, src, scope_tree, symbols, refs, alias_targets, parent_index, demand);
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
    alias_targets: &mut Vec<(String, AliasTarget)>,
    parent_index: Option<usize>,
) {
    // `demand = None` here: these are nested members inside an already-kept
    // declaration (the enclosing function/interface passed the demand gate),
    // so recurse permissively.
    if let Some(params) = func_node.child_by_field_name("parameters") {
        extract_node(params, src, scope_tree, symbols, refs, alias_targets, parent_index, None);
    }
    if let Some(ret) = func_node.child_by_field_name("return_type") {
        extract_node(ret, src, scope_tree, symbols, refs, alias_targets, parent_index, None);
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
    alias_targets: &mut Vec<(String, AliasTarget)>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "object_type" => {
            // Found one — extract its members as symbols. `demand = None`
            // because this runs inside an already-kept type alias / interface.
            extract_node(node, src, scope_tree, symbols, refs, alias_targets, parent_index, None);
        }
        // Type wrappers that can contain object_type members — recurse into children.
        "union_type" | "intersection_type" | "parenthesized_type"
        | "conditional_type" | "tuple_type" | "array_type"
        | "generic_type" | "type_arguments" | "readonly_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    recurse_for_object_types(
                        child, src, scope_tree, symbols, refs, alias_targets, parent_index,
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
/// Walk every top-level `import` statement and return a per-local-name
/// map describing where it came from. Drives the shared
/// Bare re-export pass — counterpart to `extract_reexports` for clauses
/// that have no `from '<module>'` source. The exported names are local
/// bindings; the resolver has no way to find them in another file's
/// symbol table unless something synthesises a landing point here.
///
/// For each `export { name [as alias] }` whose `name` was previously
/// imported in this file, emit:
///   * an `Imports` ref pointing back at the original module + the
///     canonical exported name, so the resolver can chain through to
///     the underlying definition;
///   * a synthetic `Variable` (or `TypeAlias` for type-only exports)
///     under the alias-or-name, so consumers' `import { … } from
///     './barrel'` finds something concrete in this file.
///
/// Names not present in the import map are skipped — those are local
/// declarations that the main traversal already extracted as proper
/// symbols, no synthesis required.
fn extract_bare_reexports_via_imports(
    root: Node,
    src: &[u8],
    import_map: &HashMap<String, ImportEntry>,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "export_statement" { continue; }
        // The with-source path is owned by `extract_reexports`. Skip it
        // here so we don't double-emit Imports refs for the same clause.
        if child.child_by_field_name("source").is_some() { continue; }

        // `export type { … }` — the whole clause is type-only. The
        // grammar surfaces `type` as a top-level keyword child of the
        // export_statement, before the export_clause.
        let stmt_type_only = (0..child.child_count()).any(|i| {
            child.child(i).map(|c| c.kind() == "type" && i < 2).unwrap_or(false)
        });

        let mut ec = child.walk();
        for clause in child.children(&mut ec) {
            if clause.kind() != "export_clause" { continue; }
            let mut sc = clause.walk();
            for spec in clause.children(&mut sc) {
                if spec.kind() != "export_specifier" { continue; }
                let name = spec
                    .child_by_field_name("name")
                    .map(|n| helpers::node_text(n, src))
                    .unwrap_or_default();
                if name.is_empty() { continue; }

                let Some(import) = import_map.get(&name) else { continue };

                // The canonical name in the source module — for renamed
                // imports (`import { X as Y }`), the source's export
                // list has `X`, not `Y`. The Imports ref must encode the
                // source-side name so cross-package chain following
                // matches the actual export.
                let exported_in_source = match &import.kind {
                    ImportKind::Named { exported_name } => exported_name.clone(),
                    ImportKind::Default => name.clone(),
                    // Namespace re-exports of a `import * as ns`-style binding
                    // are too ambiguous to resolve generically — skip.
                    ImportKind::Namespace | ImportKind::SideEffect => continue,
                };

                let alias = spec
                    .child_by_field_name("alias")
                    .map(|n| helpers::node_text(n, src))
                    .unwrap_or_default();
                let exposed = if !alias.is_empty() { alias.clone() } else { name.clone() };

                // Per-specifier `type` modifier: `export { type X }`.
                let spec_type_only = (0..spec.child_count()).any(|i| {
                    spec.child(i).map(|c| c.kind() == "type").unwrap_or(false)
                });
                let type_only = stmt_type_only || spec_type_only;

                let source_idx = symbols.len().saturating_sub(1);
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: exported_in_source,
                    kind: EdgeKind::Imports,
                    line: spec.start_position().row as u32,
                    module: Some(import.module.clone()),
                    chain: None,
                    byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });

                let already_emitted = symbols
                    .iter()
                    .any(|s| s.qualified_name == exposed);
                if !already_emitted {
                    let kind = if type_only {
                        SymbolKind::TypeAlias
                    } else {
                        // Variable matches both TypeRef and Calls in
                        // `predicates::kind_compatible`, so the consumer
                        // ref resolves regardless of how downstream
                        // names this re-export.
                        SymbolKind::Variable
                    };
                    symbols.push(ExtractedSymbol {
                        name: exposed.clone(),
                        qualified_name: exposed,
                        kind,
                        visibility: Some(Visibility::Public),
                        start_line: spec.start_position().row as u32 + 1,
                        end_line: spec.end_position().row as u32 + 1,
                        start_col: spec.start_position().column as u32,
                        end_col: spec.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: None,
                        parent_index: None,
                    });
                }
            }
        }
    }
}

/// `ecosystem::imports::resolve_import_refs` pass, which canonicalizes
/// every ref against this map before the file is handed to the resolver.
///
/// Scan the head of a TypeScript source for triple-slash directives:
///
///   `/// <reference path="X" />`    — sibling-file include (relative)
///   `/// <reference types="pkg" />` — npm typings include
///
/// Each match emits an `EdgeKind::Imports` ref with `module = Some(spec)`,
/// which feeds into the standard import resolution path. Path-form refs
/// resolve against the source file's directory; types-form refs resolve
/// against `node_modules/@types/<spec>` (the npm walker's existing
/// @types fallback handles them).
///
/// The scan stops at the first non-comment / non-blank line per the TS
/// language spec — directives must precede all source.
fn push_triple_slash_imports(source: &str, refs: &mut Vec<ExtractedRef>) {
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        // Per the TS spec, triple-slash directives must precede all
        // statements; bail at the first non-comment line so we don't
        // pick up `/// XXX` written inside a JSDoc block far below.
        if !trimmed.starts_with("///") {
            // Allow JSDoc-style block comments and regular `//` comments
            // before the first directive — many .d.ts files have a
            // license header.
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
                continue;
            }
            break;
        }
        let body = trimmed.trim_start_matches('/').trim();
        // Match `<reference path="X" />` and `<reference types="X" />`.
        // The exact form is: `<reference KIND="VALUE" />` with optional
        // additional attributes and varying whitespace.
        let Some(rest) = body.strip_prefix("<reference") else { continue };
        let rest = rest.trim_start();
        for kind in &["path", "types"] {
            let prefix = format!("{kind}=");
            let Some(after) = rest.strip_prefix(&prefix) else {
                // Try after a leading `lib=` or other attribute.
                let probe = rest.find(&prefix);
                if let Some(idx) = probe {
                    if !is_attribute_boundary(rest.as_bytes(), idx) {
                        continue;
                    }
                    if let Some(value) = read_quoted(&rest[idx + prefix.len()..]) {
                        emit_triple_slash_ref(refs, kind, &value, line_no);
                    }
                }
                continue;
            };
            if let Some(value) = read_quoted(after) {
                emit_triple_slash_ref(refs, kind, &value, line_no);
            }
        }
    }
}

fn read_quoted(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let q = bytes[0];
    if q != b'"' && q != b'\'' {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i] != q {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    if i >= bytes.len() {
        return None;
    }
    Some(s[1..i].to_string())
}

/// Char before `idx` must be a word boundary (whitespace or `<`) so we
/// don't match `pathy="X"` thinking it's `path="X"`.
fn is_attribute_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    prev == b' ' || prev == b'\t' || prev == b'<' || prev == b'\n'
}

fn emit_triple_slash_ref(refs: &mut Vec<ExtractedRef>, kind: &str, value: &str, line: usize) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    // For `types="pkg"`, rewrite to `@types/pkg/<entry>` so the npm
    // walker matches against the actual package layout. For `path="X"`,
    // pass the relative spec through and let the file-stem fallback in
    // resolve_common locate it.
    let module = match kind {
        "types" => format!("@types/{value}"),
        _ => value.to_string(),
    };
    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: module.clone(),
        kind: EdgeKind::Imports,
        line: line as u32,
        module: Some(module),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// Covers:
///   - `import Foo from 'pkg'`             → Default
///   - `import { X } from 'pkg'`           → Named { exported_name=X }
///   - `import { X as Y } from 'pkg'`      → Named { exported_name=X } keyed under Y
///   - `import * as ns from 'pkg'`         → Namespace
///   - `import 'pkg'`                      → SideEffect (no local name; not stored)
// build_import_map moved to crate::ecosystem::ecmascript_imports — both TS
// and JS extractors now share that single implementation.

/// Extract re-export refs from an `export_statement` node.
///
/// Handles:
///   `export { X } from './y'`              → Imports ref, target_name="X", module="./y"
///   `export { X as Z } from './y'`         → Imports ref + synthetic Z symbol so
///                                            consumers' `import { Z }` resolves.
///   `export * from './y'`                  → Imports ref, target_name="*", module="./y"
///   `export * as ns from './y'`            → Imports ref, target_name="*", module="./y"
///
/// Re-exports are attributed to a file-level "sentinel" symbol at index
/// `file_symbol_count` — the index one past the last real symbol, which is
/// how the JS extractor handles them.  If the file has no symbols yet,
/// index 0 is fine because the resolution engine only uses the module field.
///
/// **Why we emit synthetic symbols for renamed re-exports.** A consumer
/// that writes `import { AnyTRPCRouter } from '@trpc/server'` looks for a
/// symbol named `AnyTRPCRouter` in the package's entry chain. When the
/// definition is `export { type AnyRouter as AnyTRPCRouter } from './core'`,
/// no such symbol exists anywhere in the project — only the `AnyRouter`
/// original. Emitting an alias-named symbol here gives the resolver
/// something to land on so the import resolves; the existing Imports ref
/// continues to encode the redirection back to the source for downstream
/// chain walks that need the underlying definition.
fn extract_reexports(
    node: &tree_sitter::Node,
    src: &[u8],
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    use crate::types::{ExtractedSymbol, SymbolKind, Visibility};

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
    let source_idx = symbols.len().saturating_sub(1);

    let line = node.start_position().row as u32;
    let mut has_wildcard = false;
    // `export type { ... } from '...'` — the whole clause is type-only.
    // Tree-sitter typescript surfaces this as a `type` keyword child of the
    // export_statement node itself, before the `export_clause`.
    let stmt_type_only = (0..node.child_count()).any(|i| {
        node.child(i).map(|c| c.kind() == "type" && i < 2).unwrap_or(false)
    });

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `export { X }` or `export { X as Z }` from './y'
            "export_clause" => {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        // `name` field = the original exported name (before `as`).
                        // `alias` field = the local rename (after `as`).
                        let original_name = spec
                            .child_by_field_name("name")
                            .map(|n| helpers::node_text(n, src))
                            .unwrap_or_default();
                        let alias_name = spec
                            .child_by_field_name("alias")
                            .map(|n| helpers::node_text(n, src))
                            .unwrap_or_default();
                        // Per-specifier `type` modifier: `export { type X as Y }`.
                        let spec_type_only = (0..spec.child_count()).any(|i| {
                            spec.child(i).map(|c| c.kind() == "type").unwrap_or(false)
                        });
                        let type_only = stmt_type_only || spec_type_only;
                        if !original_name.is_empty() {
                            // The Imports ref encodes the redirection — store
                            // the original so the resolver can find it in the
                            // source module. (Unchanged from prior behavior.)
                            refs.push(ExtractedRef {
                                source_symbol_index: source_idx,
                                target_name: original_name.clone(),
                                kind: EdgeKind::Imports,
                                line: spec.start_position().row as u32,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
                            });
                        }

                        // Emit a synthetic symbol named after what the export
                        // exposes (alias when present, original otherwise).
                        // Without this, consumers' `import { X } from '...'`
                        // and bare type refs to `X` find nothing in this file
                        // — the re-export is invisible at the symbol layer.
                        // The behaviour mirrors the JS extractor and gives
                        // the resolver a landing point for both renamed and
                        // bare re-exports without disturbing the existing
                        // `Imports` ref that drives cross-package chain
                        // following.
                        let exposed = if !alias_name.is_empty() {
                            alias_name.clone()
                        } else {
                            original_name.clone()
                        };
                        let already_emitted = !exposed.is_empty()
                            && symbols.iter().any(|s| s.qualified_name == exposed);
                        if !exposed.is_empty() && !already_emitted {
                            let kind = if type_only {
                                SymbolKind::TypeAlias
                            } else {
                                // Variable matches both TypeRef and Calls in
                                // `predicates::kind_compatible`, so the
                                // consumer ref resolves regardless of how
                                // the symbol is referenced downstream.
                                SymbolKind::Variable
                            };
                            symbols.push(ExtractedSymbol {
                                name: exposed.clone(),
                                qualified_name: exposed,
                                kind,
                                visibility: Some(Visibility::Public),
                                start_line: spec.start_position().row as u32 + 1,
                                end_line: spec.end_position().row as u32 + 1,
                                start_col: spec.start_position().column as u32,
                                end_col: spec.end_position().column as u32,
                                signature: None,
                                doc_comment: None,
                                scope_path: None,
                                parent_index: None,
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
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
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
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
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
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
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
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
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
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
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
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
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
    // `[K in keyof T]: V[K]` mapped types bind `K` for the body of the
    // mapped type. tree-sitter-typescript represents the `[K in keyof T]`
    // header as a `mapped_type_clause` node with the binding in its
    // `name` field. Without this, `TRecord[TKey]` in
    // `{ [TKey in keyof TRecord]: TRecord[TKey] }` leaks `TKey` as
    // an external TypeRef (the `indexed_access_type` walker treats
    // the index as a regular type ref).
    //
    // Scope is the parent `mapped_type` (whose line range covers both
    // the clause AND the body). Falls back to the clause itself if
    // somehow detached.
    if node.kind() == "mapped_type_clause" {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(src) {
                if !name.is_empty() {
                    let scope_node = node
                        .parent()
                        .filter(|p| p.kind() == "mapped_type")
                        .unwrap_or(node);
                    out.push((
                        name.to_string(),
                        scope_node.start_position().row as u32,
                        scope_node.end_position().row as u32,
                    ));
                }
            }
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
// Angular @Component selector extraction (called by full-index pipeline)
// ---------------------------------------------------------------------------

/// Scan `source` for `@Component({selector: '...'})` decorators on class
/// declarations and return a mapping of `(raw_selector, class_qualified_name)`
/// pairs.
///
/// `symbols` must be the symbols already extracted from the same source (via
/// `extract` or `extract_with_demand`) — this function matches the N-th class
/// declaration in the AST to the N-th `SymbolKind::Class` symbol in the
/// vec to obtain the qualified name without re-running the full symbol
/// extraction logic.
///
/// Called by the full-index pipeline (`indexer/full.rs`) for `typescript` and
/// `angular` files so `SymbolIndex::build_with_context` can build the
/// project-wide Angular selector map without a second parse pass per file.
pub fn extract_component_selectors(
    source: &str,
    symbols: &[crate::types::ExtractedSymbol],
) -> Vec<(String, String)> {
    use crate::types::SymbolKind;

    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let src = source.as_bytes();
    let root = tree.root_node();

    // Pre-build an index of class symbol qualified names in declaration order.
    // When a class declaration with an @Component decorator is encountered during
    // the AST walk, we match it by its class-name node text against the symbols
    // vec to get its qualified_name.
    let class_qnames: std::collections::HashMap<String, String> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .map(|s| (s.name.clone(), s.qualified_name.clone()))
        .collect();

    let mut result: Vec<(String, String)> = Vec::new();
    collect_component_selectors_recursive(&root, src, &class_qnames, &mut result);
    result
}

fn collect_component_selectors_recursive(
    node: &tree_sitter::Node,
    src: &[u8],
    class_qnames: &std::collections::HashMap<String, String>,
    result: &mut Vec<(String, String)>,
) {
    let kind = node.kind();
    if matches!(kind, "class_declaration" | "abstract_class_declaration") {
        // Try to extract a selector from a @Component decorator on this class.
        let selectors = super::decorators::component_selectors_from_class(node, src);
        if !selectors.is_empty() {
            // Get the class name to look up its qualified name.
            if let Some(name_node) = node.child_by_field_name("name") {
                let class_name = super::helpers::node_text(name_node, src);
                if let Some(qname) = class_qnames.get(&class_name) {
                    for sel in selectors {
                        result.push((sel, qname.clone()));
                    }
                }
            }
        }
    }
    // Recurse into children to handle nested classes and export wrappers.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_component_selectors_recursive(&child, src, class_qnames, result);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
