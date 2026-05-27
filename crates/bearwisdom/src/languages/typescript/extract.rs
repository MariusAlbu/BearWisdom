// =============================================================================
// languages/typescript/extract.rs  —  TypeScript / TSX extractor
//
// SYMBOLS: Class, Interface, Function, Method, Constructor, Property,
//          TypeAlias, Variable, Enum, EnumMember, Namespace
//
// REFERENCES: imports, calls, extends/implements, type refs, instanceof/as,
//             JSX component usage, tagged templates
// =============================================================================

use super::{alias_classify, calls, decorators, helpers, imports, narrowing, params, symbols, types};
use super::reexports::{
    extract_bare_reexports_via_imports, extract_reexports, push_triple_slash_imports,
};
use super::type_scan::{
    collect_type_param_scopes, is_ts_primitive, scan_all_type_identifiers,
};

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

    // SYM-002: scope_path equals symbols[parent_index].qualified_name. The
    // scope-tree path can drift for synthetic-name child symbols (index
    // signature parameters `[s]`, computed property keys `[Symbol.match]`,
    // mapped-type binder names) emitted inside method bodies — their scope
    // is the method, not the enclosing class. Re-derive from parent_index.
    for i in 0..symbols.len() {
        if let Some(p) = symbols[i].parent_index {
            let parent_qname = symbols[p].qualified_name.clone();
            symbols[i].scope_path = Some(parent_qname);
        }
    }

    // REF-001: every ref's source_symbol_index must be in bounds. Ambient
    // .d.ts files that are pure triple-slash reference hubs
    // (e.g. tsserverlibrary.d.ts) emit Imports refs but declare no symbols
    // under demand-driven filtering. Push a file-level sentinel symbol at
    // index 0 so those refs have a valid owner.
    if symbols.is_empty() && !refs.is_empty() {
        symbols.push(ExtractedSymbol {
            name: String::new(),
            qualified_name: String::new(),
            kind: crate::types::SymbolKind::Namespace,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: root.end_position().row as u32,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

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
                        let qname = symbols[idx].qualified_name.clone();
                        // A discriminated union of anonymous object types emits
                        // synthetic per-branch types + an Intersection target so
                        // a guard can narrow to one branch; every other shape
                        // takes the flat-flatten + classify path.
                        if let Some(target) = try_anonymous_discriminated_union(
                            value, src, scope_tree, symbols, refs, alias_targets, idx,
                        ) {
                            alias_targets.push((qname, target));
                        } else {
                            recurse_for_object_types(
                                value, src, scope_tree, symbols, refs, alias_targets, Some(idx),
                            );
                            // Capture the alias's structural shape so the chain
                            // walker can decide whether to expand it (Application
                            // arm) or treat it as opaque (Union / Intersection /
                            // Object / Other). This is the type-checker's
                            // alternative to the engine's positional flatten.
                            let target = alias_classify::classify_alias_target(&value, src);
                            alias_targets.push((qname, target));
                        }
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
                                col: 0,
                                module: None,
                                chain: None,
                                byte_offset: right.start_byte() as u32,
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
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
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
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
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
    // Explicit-stack walk. A recursive descent overflows on a pathologically
    // deep type-value node — e.g. a generated `.d.ts` whose alias is a union
    // of tens of thousands of string literals nests `union_type` that many
    // levels deep. Children are pushed in reverse so `object_type` members
    // are extracted in source order.
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "object_type" => {
                // Found one — extract its members as symbols. `demand = None`
                // because this runs inside an already-kept type alias / interface.
                extract_node(node, src, scope_tree, symbols, refs, alias_targets, parent_index, None);
            }
            // Type wrappers that can contain object_type members — descend into children.
            "union_type" | "intersection_type" | "parenthesized_type"
            | "conditional_type" | "tuple_type" | "array_type"
            | "generic_type" | "type_arguments" | "readonly_type" => {
                let mut cursor = node.walk();
                let children: Vec<tree_sitter::Node> =
                    node.children(&mut cursor).filter(|c| c.is_named()).collect();
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
            }
            // All other type nodes (type_identifier, primitive_type, function_type, etc.)
            // cannot contain object_type members — stop descent here.
            _ => {}
        }
    }
}

/// Unwrap `parenthesized_type` / `readonly_type` wrappers to the inner type.
fn unwrap_type_wrappers(node: Node) -> Option<Node> {
    let mut n = node;
    loop {
        match n.kind() {
            "parenthesized_type" | "readonly_type" => {
                let mut c = n.walk();
                let inner = n
                    .children(&mut c)
                    .find(|ch| ch.is_named() && ch.kind() != "readonly");
                drop(c);
                n = inner?;
            }
            _ => return Some(n),
        }
    }
}

/// Handle a discriminated union of anonymous object types
/// (`{kind:"a";x}|{kind:"b";y}`): emit a synthetic per-branch type (its members
/// parented under it) for each branch and return an
/// `AliasTarget::Intersection` of their qnames, instead of flattening every
/// branch's members under the alias.
///
/// The Intersection's member lookup is any-branch (`members.rs`), so with no
/// discriminant guard active every member still resolves (matching the prior
/// flat behavior — no regression); an active guard narrows the Intersection to
/// one branch (`narrow_union_by_discriminant`), giving branch precision.
///
/// Returns `None` for any other shape (single object type, mixed named/anonymous
/// union, primitive union) — the caller then takes the flat-object / classify
/// path. Synthetic branch qnames carry a `\u{1}` sentinel so they can't collide
/// with a real identifier.
fn try_anonymous_discriminated_union(
    value: Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    alias_targets: &mut Vec<(String, AliasTarget)>,
    alias_idx: usize,
) -> Option<AliasTarget> {
    let union = unwrap_type_wrappers(value)?;
    if union.kind() != "union_type" {
        return None;
    }
    let mut branch_nodes: Vec<Node> = Vec::new();
    let mut has_nameable = false;
    let mut cursor = union.walk();
    for child in union.children(&mut cursor) {
        match child.kind() {
            "|" => {}
            "object_type" => branch_nodes.push(child),
            _ if child.is_named() => has_nameable = true,
            _ => {}
        }
    }
    drop(cursor);
    // Only the pure-anonymous, multi-branch shape — otherwise let `classify`
    // produce its `Union` / `Object` as before.
    if has_nameable || branch_nodes.len() < 2 {
        return None;
    }

    let alias_qname = symbols[alias_idx].qualified_name.clone();
    let mut branch_qnames = Vec::with_capacity(branch_nodes.len());
    for (i, branch) in branch_nodes.into_iter().enumerate() {
        let branch_qname = format!("{alias_qname}\u{1}{i}");
        let branch_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: branch_qname.clone(),
            qualified_name: branch_qname.clone(),
            kind: SymbolKind::Interface,
            visibility: None,
            start_line: branch.start_position().row as u32,
            end_line: branch.end_position().row as u32,
            start_col: branch.start_position().column as u32,
            end_col: branch.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path: Some(alias_qname.clone()),
            parent_index: Some(alias_idx),
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
        // Emit the branch's members parented under the synthetic branch type.
        extract_node(
            branch, src, scope_tree, symbols, refs, alias_targets, Some(branch_idx), None,
        );
        branch_qnames.push(branch_qname);
    }
    Some(AliasTarget::Intersection(branch_qnames))
}
