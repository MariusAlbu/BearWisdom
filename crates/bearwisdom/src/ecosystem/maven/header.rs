// =============================================================================
// maven/header.rs  —  the top-level declaration names of one JVM source file
//
// A shallow tree-sitter pass over a Java, Kotlin, Scala, Groovy or Clojure
// file that reads only its top-level declarations — the names a symbol
// location index keys the file under.
// =============================================================================

use tree_sitter::{Node, Parser};

/// Dispatch header-only scan by language id. Returns every top-level
/// declaration name the file publishes. Function/method/class bodies are
/// never walked.
pub(super) fn scan_jvm_header(source: &str, language: &str) -> Vec<String> {
    match language {
        "java" => scan_java_header(source),
        "kotlin" => scan_kotlin_header(source),
        "scala" => scan_scala_header(source),
        "clojure" => scan_clojure_header(source),
        "groovy" => scan_groovy_header(source),
        _ => Vec::new(),
    }
}

/// Header-only tree-sitter scan of a Java source file. Returns the names of
/// every top-level `class`, `interface`, `enum`, `record`, and
/// `annotation_type_declaration` the file declares. Nested types are
/// captured as well — the top level of the file also hosts member types
/// reachable by outer-qualified names (`Outer.Inner`). We record the bare
/// simple name so `find_by_name("Inner")` still hits.
pub(crate) fn scan_java_header(source: &str) -> Vec<String> {
    let language = tree_sitter_java::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_java_top_level_name(&child, bytes, &mut out);
    }
    out
}

pub(crate) fn collect_java_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "annotation_type_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        _ => {}
    }
}

/// Header-only tree-sitter scan of a Kotlin source file (using
/// `tree-sitter-kotlin-ng`, which is the grammar the crate's Kotlin plugin
/// uses). Returns top-level class / object / interface / type-alias /
/// function / property names.
pub(crate) fn scan_kotlin_header(source: &str) -> Vec<String> {
    let language = tree_sitter_kotlin_ng::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_kotlin_top_level_name(&child, bytes, &mut out);
    }
    out
}

pub(crate) fn collect_kotlin_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_declaration"
        | "object_declaration"
        | "interface_declaration"
        | "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "type_alias" => {
            // `typealias Foo = Bar` — field `type` (yes, really) holds the
            // identifier in kotlin-ng's grammar; fall back to any identifier
            // child for older grammar revs.
            if let Some(name_node) = node.child_by_field_name("type") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "property_declaration" => {
            // Top-level `val`/`var`. Name lives at
            // property_declaration → variable_declaration → simple_identifier.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                if inner.kind() == "variable_declaration" {
                    let mut ic = inner.walk();
                    for sub in inner.children(&mut ic) {
                        if matches!(sub.kind(), "simple_identifier" | "identifier") {
                            if let Ok(name) = sub.utf8_text(bytes) {
                                out.push(name.to_string());
                            }
                            break;
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Header-only tree-sitter scan of a Scala source file. Returns top-level
/// class / object / trait / case-class / function / val / var names.
pub(crate) fn scan_scala_header(source: &str) -> Vec<String> {
    let language = tree_sitter_scala::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_scala_top_level_name(&child, bytes, &mut out);
    }
    out
}

pub(crate) fn collect_scala_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_definition"
        | "object_definition"
        | "trait_definition"
        | "enum_definition"
        | "function_definition"
        | "function_declaration"
        | "type_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        "val_definition" | "var_definition" | "val_declaration" | "var_declaration" => {
            // Scala `val X = ...` / `val X: T = ...`. `pattern` field is the
            // canonical LHS (identifier or tuple pattern).
            let name_node = node
                .child_by_field_name("pattern")
                .or_else(|| node.child_by_field_name("name"));
            if let Some(name_node) = name_node {
                collect_scala_pattern_names(&name_node, bytes, out);
            }
        }
        "package_object" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.push(name.to_string());
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_scala_pattern_names(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "stable_identifier" => {
            if let Ok(name) = node.utf8_text(bytes) {
                out.push(name.to_string());
            }
        }
        _ => {
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                collect_scala_pattern_names(&inner, bytes, out);
            }
        }
    }
}

/// Header-only tree-sitter scan of a Clojure source file. Clojure's grammar
/// exposes every form as a `list_lit`; the first `sym_lit` child is the
/// declaration keyword (`def`, `defn`, `defn-`, `defmacro`, `defmulti`,
/// `defprotocol`, `defrecord`, `deftype`, `definterface`, `defmethod`,
/// `defonce`), and the second `sym_lit` is the declared name. Docstrings,
/// metadata, and the body don't affect the name's position.
pub(crate) fn scan_clojure_header(source: &str) -> Vec<String> {
    let language = tree_sitter_clojure::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_clojure_top_level_name(&child, bytes, &mut out);
    }
    out
}

pub(crate) fn collect_clojure_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    if node.kind() != "list_lit" {
        return;
    }
    // Find the first two `sym_lit` children. First is the form head (e.g.
    // `defn`), second is the declared name.
    let mut head: Option<String> = None;
    let mut name: Option<String> = None;
    let mut cursor = node.walk();
    for inner in node.children(&mut cursor) {
        if inner.kind() != "sym_lit" {
            continue;
        }
        let Ok(text) = inner.utf8_text(bytes) else {
            continue;
        };
        if head.is_none() {
            head = Some(text.to_string());
        } else {
            name = Some(text.to_string());
            break;
        }
    }
    let Some(head) = head else { return };
    let Some(name) = name else { return };
    if matches!(
        head.as_str(),
        "def"
            | "defn"
            | "defn-"
            | "defmacro"
            | "defmulti"
            | "defmethod"
            | "defprotocol"
            | "defrecord"
            | "deftype"
            | "definterface"
            | "defonce"
    ) {
        out.push(name);
    }
}

/// Header-only tree-sitter scan of a Groovy source file. Groovy jars shipped
/// by Gradle plugins and Spock test harnesses publish normal `class`,
/// `interface`, `enum` declarations — same `class_declaration` node kind as
/// Java's grammar uses here.
pub(crate) fn scan_groovy_header(source: &str) -> Vec<String> {
    let language = tree_sitter_groovy::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_groovy_top_level_name(&child, bytes, &mut out);
    }
    out
}

pub(crate) fn collect_groovy_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    if !matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "enum_declaration"
    ) {
        return;
    }
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(bytes) {
            out.push(name.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "header_tests.rs"]
mod tests;
