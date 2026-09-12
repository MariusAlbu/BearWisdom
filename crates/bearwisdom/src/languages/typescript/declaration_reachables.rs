// =============================================================================
// typescript/declaration_reachables.rs — the files a declaration file reaches
// without naming a binding
// =============================================================================

use tree_sitter::Node;

/// Relative specifiers a `.d.ts` file reaches other than through a named
/// import or re-export: its side-effect imports (`import './chunks/global.js'`,
/// the carrier of module augmentations and ambient declarations), and, for an
/// Angular NgModule declaration, the `.component`/`.directive` files it declares
/// — those are referenced only by selector, so nothing else demands them.
pub(super) fn reachables(file_path: &str, content: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !file_path.ends_with(".d.ts") {
        return out;
    }
    let Some(tree) = parse(content) else {
        return out;
    };
    push_side_effect_imports(tree.root_node(), content.as_bytes(), &mut out);
    if file_path.ends_with(".module.d.ts") && content.contains("ɵɵNgModuleDeclaration") {
        push_angular_declarations(content, &mut out);
    }
    out
}

fn parse(content: &str) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .ok()?;
    parser.parse(content, None)
}

/// Every top-level `import '<relative>'` without an import clause.
fn push_side_effect_imports(root: Node, src: &[u8], out: &mut Vec<String>) {
    let mut cursor = root.walk();
    for statement in root.children(&mut cursor) {
        if statement.kind() != "import_statement" {
            continue;
        }
        let mut children = statement.walk();
        if statement
            .children(&mut children)
            .any(|child| child.kind() == "import_clause")
        {
            continue;
        }
        let Some(source) = statement.child_by_field_name("source") else {
            continue;
        };
        let Ok(raw) = source.utf8_text(src) else {
            continue;
        };
        let spec = raw.trim_matches(|c| c == '\'' || c == '"' || c == '`');
        if spec.starts_with('.') && !out.iter().any(|s| s == spec) {
            out.push(spec.to_string());
        }
    }
}

/// The `.component`/`.directive` declaration files an NgModule `.d.ts`
/// imports or re-exports.
fn push_angular_declarations(content: &str, out: &mut Vec<String>) {
    for line in content.lines() {
        let t = line.trim();
        if !(t.starts_with("import ") || t.starts_with("export ")) {
            continue;
        }
        let Some(spec) = crate::ecosystem::npm::extract_quoted_after(t, " from ") else {
            continue;
        };
        if spec.starts_with('.')
            && (spec.contains(".component") || spec.contains(".directive"))
            && !out.iter().any(|s| s == spec)
        {
            out.push(spec.to_string());
        }
    }
}

#[cfg(test)]
#[path = "declaration_reachables_tests.rs"]
mod tests;
