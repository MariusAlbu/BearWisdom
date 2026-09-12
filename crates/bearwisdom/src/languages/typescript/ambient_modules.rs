// =============================================================================
// languages/typescript/ambient_modules.rs — ambient `declare module` names
//
// Collects the string-literal names of top-level ambient module declarations:
// `declare module 'virtual:pwa' { ... }` and the shorthand
// `declare module 'my-shim';`. Identifier-named blocks (`declare module Foo`,
// `namespace Foo`) are namespaces, not module specifiers, and are excluded.
// The names feed `ParsedFile::declared_modules`, which the module-entry pass
// keys to the declaring file so imports of that specifier link to it.
// =============================================================================

use tree_sitter::Node;

/// Every distinct `declare module '<name>'` string literal declared at the
/// top level of `root` (bare `module '<name>'` too — `declare` is implicit
/// in a declaration file), in source order. A module file declares none:
/// its string-named blocks augment modules that exist elsewhere.
pub(crate) fn collect_declared_modules(root: Node, src: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if is_module_file(root) {
        return out;
    }
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "ambient_declaration" => {
                let mut ac = child.walk();
                for inner in child.children(&mut ac) {
                    if inner.kind() == "module" {
                        push_string_named(&inner, src, &mut out);
                    }
                }
            }
            "module" => push_string_named(&child, src, &mut out),
            _ => {}
        }
    }
    out
}

/// A file with a top-level import or export is a module; a string-named
/// `declare module` block inside it augments rather than declares.
pub(crate) fn is_module_file(root: Node) -> bool {
    let mut cursor = root.walk();
    let is_module = root
        .children(&mut cursor)
        .any(|child| matches!(child.kind(), "import_statement" | "export_statement"));
    is_module
}

/// Append `module_node`'s declared name when it is a string literal.
fn push_string_named(module_node: &Node, src: &[u8], out: &mut Vec<String>) {
    let Some(name_node) = module_node.child_by_field_name("name") else {
        return;
    };
    if name_node.kind() != "string" {
        return;
    }
    let Ok(raw) = name_node.utf8_text(src) else {
        return;
    };
    let name = strip_quotes(raw);
    if !name.is_empty() && !out.iter().any(|n| n == &name) {
        out.push(name);
    }
}

fn strip_quotes(s: &str) -> String {
    s.trim()
        .trim_start_matches(['"', '\''])
        .trim_end_matches(['"', '\''])
        .to_string()
}

#[cfg(test)]
#[path = "ambient_modules_tests.rs"]
mod tests;
