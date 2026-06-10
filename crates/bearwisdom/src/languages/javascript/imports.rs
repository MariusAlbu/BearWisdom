// =============================================================================
// javascript/imports.rs — import/export ref emission for the JS extractor
//
// Owns CommonJS (`require`, `module.exports`, `exports.X`), ES module
// (`import`/`export`), and ES5 prototype-method install detection. Emits
// `Imports` edges for module-boundary references and registers
// prototype-installed methods as Method symbols.
// =============================================================================

use super::calls::{callee_name, emit_call_ref_js, emit_new_ref_js, extract_calls};
use super::helpers::{extract_jsdoc, node_text};
use crate::parser::scope_tree::ScopeTree;
use crate::types::{EdgeKind, ExtractedRef as Ref, ExtractedSymbol as Sym, SymbolKind};
use tree_sitter::Node;

/// Emit refs for an `export_statement` node so the coverage system can match
/// at least one ref at the export statement's start line.
///
/// Handles:
/// - `export { foo, bar }` — emits Imports refs for each named specifier
/// - `export { foo as default } from './mod'` — same
/// - `export default expr` — emits Imports ref for the identifier/call name
/// - `export * from './mod'` — emits Imports ref for the module path
/// - `export const/function/class ...` — the inner decl handles symbols;
///   here we emit an Imports ref using the decl's name so the line is covered
/// - `export default { ... }` / `export default function() {}` — fallback ref
pub(super) fn push_export_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    let line = node.start_position().row as u32;
    let initial_ref_count = refs.len();

    let module_path = node.child_by_field_name("source").map(|s| {
        node_text(s, src)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    });

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `export { foo, bar }` or `export { foo } from './mod'`
            "export_clause" => {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        // The exported name (after `as`, or the original name).
                        let exported = spec
                            .child_by_field_name("alias")
                            .or_else(|| spec.child_by_field_name("name"))
                            .map(|n| node_text(n, src))
                            .unwrap_or_default();
                        if !exported.is_empty() {
                            refs.push(Ref {
                                is_import_binding: false,
                                is_reexport: false,
                                source_symbol_index,
                                target_name: exported,
                                kind: EdgeKind::Imports,
                                line: spec.start_position().row as u32,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: spec.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                                col: 0,
                            });
                        }
                    }
                }
            }

            // `export * from './mod'` — the `*` child is a namespace_export or literal
            "namespace_export" => {
                if let Some(mod_path) = &module_path {
                    refs.push(Ref {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: mod_path.clone(),
                        kind: EdgeKind::Imports,
                        line,
                        module: module_path.clone(),
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                        col: 0,
                    });
                }
            }

            // `export default <identifier>` — the exported identifier
            "identifier" => {
                let name = node_text(child, src);
                if name != "default" && name != "export" && !name.is_empty() {
                    refs.push(Ref {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                        col: 0,
                    });
                }
            }

            // `export default callExpr(...)` — emit a call ref for the callee
            "call_expression" => {
                emit_call_ref_js(&child, src, source_symbol_index, refs);
            }

            // `export default new Foo()` — emit a new ref
            "new_expression" => {
                emit_new_ref_js(&child, src, source_symbol_index, refs);
            }

            // `export const/let/var X = ...` and `export function foo()` etc.
            // Emit an Imports ref using the first declared name so the line is covered.
            "lexical_declaration" | "variable_declaration" => {
                let mut dc = child.walk();
                'outer_lex: for decl in child.children(&mut dc) {
                    if decl.kind() == "variable_declarator" {
                        if let Some(name_node) = decl.child_by_field_name("name") {
                            let name = node_text(name_node, src);
                            if !name.is_empty() {
                                refs.push(Ref {
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::Imports,
                                    line,
                                    module: None,
                                    chain: None,
                                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                    col: 0,
                                });
                                break 'outer_lex;
                            }
                        }
                    }
                }
            }

            "function_declaration" | "generator_function_declaration" => {
                // `export function foo() {}` / `export default function foo() {}`
                // Re-export form (`export { foo } from './mod'`) is handled in
                // the `export_clause` arm above; those land here only when an
                // anonymous default function is the export target, which we
                // skip — the inner function symbol already covers the line.
                //
                // For the NAMED form, emitting a self-targeted Imports ref is
                // pure noise: the function symbol exists in the same file,
                // there's no module path to resolve against, and the ref
                // always lands in `unresolved_refs` since name-collision with
                // `init` / `main` / etc. across unrelated files makes the
                // heuristic pick the wrong target (fluentui-blazor has 26+
                // unrelated `init` symbols across .cs / .razor.js files).
                //
                // Skip. Coverage comes from the function symbol itself.
                if module_path.is_some() {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if !name.is_empty() {
                            refs.push(Ref {
                                is_import_binding: false,
                                is_reexport: false,
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::Imports,
                                line,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: child.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                                col: 0,
                            });
                        }
                    }
                }
            }

            "class_declaration" | "class" => {
                // Same rationale as `function_declaration` above — only emit
                // for re-export forms where the module path gives the
                // resolver something to match against.
                if module_path.is_some() {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if !name.is_empty() {
                            refs.push(Ref {
                                is_import_binding: false,
                                is_reexport: false,
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::Imports,
                                line,
                                module: module_path.clone(),
                                chain: None,
                                byte_offset: child.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
                                col: 0,
                            });
                        }
                    }
                }
            }

            _ => {}
        }
    }

    // Fallback: if we emitted no ref yet (e.g. `export * from './mod'` without a
    // namespace_export child, `export default {}`, `export default function() {}`),
    // emit an Imports ref at the export line using the module path or a placeholder.
    if refs.len() == initial_ref_count {
        let target = module_path.clone().unwrap_or_else(|| "default".to_string());
        refs.push(Ref {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: target.clone(),
            kind: EdgeKind::Imports,
            line,
            module: module_path,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            col: 0,
        });
    }
}

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<Ref>,
) {
    crate::ecosystem::ecmascript_imports::push_import_refs(
        node,
        src,
        current_symbol_count,
        refs,
        crate::ecosystem::ecmascript_imports::PushImportOpts::JAVASCRIPT,
    );
}

// ---------------------------------------------------------------------------
// require()-side import emission
// ---------------------------------------------------------------------------

/// Extract the string argument from `require('foo')`.
/// Returns `None` if the call has no string literal argument.
pub(super) fn extract_require_path(call_node: &Node, src: &[u8]) -> Option<String> {
    extract_first_string_arg(call_node, src)
}

/// Extract the first string literal from a call's arguments node.
pub(super) fn extract_first_string_arg(call_node: &Node, src: &[u8]) -> Option<String> {
    let args = call_node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        match arg.kind() {
            "string" | "template_string" => {
                let raw = node_text(arg, src);
                let cleaned = raw
                    .trim_matches('`')
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                return Some(cleaned);
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// module.exports / exports.X detection
// ---------------------------------------------------------------------------

/// Walk the expression_statement for `module.exports = X` or `exports.Foo = X`
/// assignments and emit an Imports edge pointing to the assigned name, with
/// the target set to the RHS identifier (if simple) so the indexer can link
/// the export to the source symbol.
pub(super) fn extract_module_exports(
    stmt_node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<Ref>,
) {
    let mut cursor = stmt_node.walk();
    for child in stmt_node.children(&mut cursor) {
        if child.kind() != "assignment_expression" {
            continue;
        }
        let Some(left) = child.child_by_field_name("left") else {
            continue;
        };
        let Some(right) = child.child_by_field_name("right") else {
            continue;
        };

        let lhs = node_text(left, src);
        let is_module_exports = lhs == "module.exports" || lhs.starts_with("exports.");

        if !is_module_exports {
            continue;
        }

        // Determine what is being exported.
        let export_name = if lhs == "module.exports" {
            // `module.exports = SomeIdentifier` — link the export to that
            // symbol. For non-identifier RHS (`module.exports = factory()`,
            // `= { … }`, `= someCall()`) there's no concrete target to
            // point at; emitting the literal "module.exports" as Imports
            // target is just noise in `unresolved_refs`. Skip silently.
            match right.kind() {
                "identifier" => node_text(right, src),
                _ => continue,
            }
        } else {
            // `exports.Foo = bar` → export name is "Foo"
            left.child_by_field_name("property")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| lhs.clone())
        };

        refs.push(Ref {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index: current_symbol_count,
            target_name: export_name,
            kind: EdgeKind::Imports,
            line: child.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: child.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            col: 0,
        });
    }
}

// ---------------------------------------------------------------------------
// ES5 prototype-method installs (webpack / TS-to-ES5 transpile output)
// ---------------------------------------------------------------------------

/// Detect `X.prototype.Y = function (…) { … }` (or arrow) assignments and
/// emit `Y` as a Method under qualified name `X.Y`. This is how ES5-style
/// classes install instance methods — SignalR's `signalr.js`, any code that
/// targets `lib: ES5`, and most TypeScript `target: "es5"` transpile output
/// land on this pattern. Without it, the chain walker sees the constructor
/// function `X` but none of its methods, so `new X().foo()` leaves `foo`
/// unresolved.
pub(super) fn extract_prototype_method(
    stmt_node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<Sym>,
    refs: &mut Vec<Ref>,
    parent_index: Option<usize>,
) {
    use crate::parser::scope_tree;

    let mut cursor = stmt_node.walk();
    for child in stmt_node.children(&mut cursor) {
        if child.kind() != "assignment_expression" {
            continue;
        }
        let Some(left) = child.child_by_field_name("left") else {
            continue;
        };
        let Some(right) = child.child_by_field_name("right") else {
            continue;
        };

        // Left must be a member_expression: {object: member_expression, property: identifier}.
        if left.kind() != "member_expression" {
            continue;
        }
        let Some(outer_object) = left.child_by_field_name("object") else {
            continue;
        };
        let Some(outer_property) = left.child_by_field_name("property") else {
            continue;
        };
        if outer_property.kind() != "property_identifier" && outer_property.kind() != "identifier" {
            continue;
        }
        if outer_object.kind() != "member_expression" {
            continue;
        }
        // Inner member's property must be "prototype" and its object a plain identifier.
        let Some(inner_object) = outer_object.child_by_field_name("object") else {
            continue;
        };
        let Some(inner_property) = outer_object.child_by_field_name("property") else {
            continue;
        };
        if inner_object.kind() != "identifier" {
            continue;
        }
        if node_text(inner_property, src) != "prototype" {
            continue;
        }

        // Right must be a function-like expression. Bail on everything else
        // so we don't misattribute field initializers (`.prototype.x = []`).
        let is_callable = matches!(
            right.kind(),
            "function_expression" | "arrow_function" | "generator_function"
        );
        if !is_callable {
            continue;
        }

        let class_name = node_text(inner_object, src);
        let method_name = node_text(outer_property, src);
        if class_name.is_empty() || method_name.is_empty() {
            continue;
        }

        let parent_scope = if stmt_node.start_byte() > 0 {
            scope_tree::find_scope_at(scope_tree, stmt_node.start_byte() - 1)
        } else {
            None
        };
        let scope_path = scope_tree::scope_path(parent_scope);

        let kind = if method_name == "constructor" {
            SymbolKind::Constructor
        } else {
            SymbolKind::Method
        };

        let idx = symbols.len();
        symbols.push(Sym {
            name: method_name.clone(),
            qualified_name: format!("{class_name}.{method_name}"),
            kind,
            visibility: None,
            start_line: outer_property.start_position().row as u32,
            end_line: right.end_position().row as u32,
            start_col: outer_property.start_position().column as u32,
            end_col: right.end_position().column as u32,
            signature: Some(format!("{class_name}.prototype.{method_name} = function")),
            doc_comment: extract_jsdoc(stmt_node, src),
            scope_path,
            parent_index,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });

        // Harvest calls inside the function body so in-method refs attach
        // to the method symbol rather than the enclosing scope.
        if let Some(body) = right.child_by_field_name("body") {
            extract_calls(&body, src, idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// require() at the declarator level (outside a function body)
// ---------------------------------------------------------------------------

/// If `init_node` is `require('foo')`, push an Imports edge. Used for
/// top-level `const x = require('foo')` declarations.
pub(super) fn try_emit_require(
    init_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<Ref>,
) {
    if init_node.kind() != "call_expression" {
        return;
    }
    let Some(func) = init_node.child_by_field_name("function") else {
        return;
    };
    let name = callee_name(func, src);
    if name != "require" {
        return;
    }
    if let Some(module) = extract_require_path(init_node, src) {
        refs.push(Ref {
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: module.clone(),
            kind: EdgeKind::Imports,
            line: init_node.start_position().row as u32,
            module: Some(module),
            chain: None,
            byte_offset: init_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
            col: 0,
        });
    }
}
