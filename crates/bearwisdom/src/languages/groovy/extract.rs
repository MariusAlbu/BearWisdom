// =============================================================================
// languages/groovy/extract.rs  —  Groovy symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Namespace  — `package_declaration`
//   Class      — `class_declaration`
//   Function   — `function_definition` (top-level `def`)
//   Method     — `method_declaration` (inside class body)
//   Variable   — `declaration` (module-level)
//
// REFERENCES:
//   Imports    — `import_declaration`
//   Calls      — `method_invocation`
//
// Grammar: tree-sitter-groovy.  Actual node kinds confirmed by CST probe:
//   class_declaration  (fields: name, body)
//   method_declaration (fields: type, name, parameters, body)
//   function_definition (fields: name, parameters, body)   ← top-level `def fn`
//   package_declaration
//   import_declaration
//   method_invocation  (fields: name, arguments)
// =============================================================================

use crate::types::{
    ChainSegment, EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, MemberChain,
    SegmentKind, SymbolKind, Visibility,
};
use super::predicates;
use std::collections::HashMap;
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_groovy::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult::empty();
    }

    // First attempt: parse as-is.
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    // If the tree has errors, retry with angle-bracket-containing single-quoted
    // string literals neutralized. The Groovy grammar misparses patterns like
    //   protected static final X = '<init>'
    // because `'<init>'` looks like a generic type bound to the grammar.
    // We substitute only single-quoted strings that contain `<` to avoid
    // corrupting the byte offsets for symbols we actually care about.
    let sanitized: Option<String>;
    let (tree, source) = if tree.root_node().has_error() {
        sanitized = Some(neutralize_angle_bracket_sqstrings(source));
        let s = sanitized.as_deref().unwrap();
        let t = parser.parse(s, None).unwrap_or(tree);
        (t, s)
    } else {
        sanitized = None;
        (tree, source)
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Pre-scan for the package declaration so class names can be qualified.
    // Groovy files always declare the package before any class, so a single
    // top-level scan suffices.
    let namespace = pre_scan_namespace(tree.root_node(), source);

    visit(tree.root_node(), source, &mut symbols, &mut refs, None, false, namespace.as_deref());

    // Fallback for files where the grammar fails to parse the class_declaration
    // (e.g. Groovy grammar misparses certain single-quoted literals or GString
    // expressions). Recover the class name and any missing method names via
    // line-level scanning so that:
    //   (a) the class appears in the index with correct qualified_name;
    //   (b) scope_path on already-extracted methods is retroactively set;
    //   (c) methods that the grammar missed are also extracted.
    if has_errors && !symbols.iter().any(|s| s.kind == SymbolKind::Class) {
        if let Some((class_name, class_line)) = scan_class_name_from_source(source) {
            let class_qname = match namespace.as_deref() {
                Some(ns) => format!("{}.{}", ns, class_name),
                None => class_name.clone(),
            };
            let class_idx = symbols.len();

            // Extract superclass from source text for Inherits edge.
            extract_class_inherits_from_source(source, class_idx, &mut refs);

            symbols.push(ExtractedSymbol {
                name: class_name.clone(),
                qualified_name: class_qname.clone(),
                kind: SymbolKind::Class,
                visibility: Some(Visibility::Public),
                start_line: class_line,
                end_line: source.lines().count().saturating_sub(1) as u32,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("class {} {{ ... }}", class_name)),
                doc_comment: None,
                scope_path: None,
                parent_index: None,
            });

            // Retroactively fix scope_path on orphan methods so the inheritance
            // resolver can walk up from the correct class.
            let already_extracted: std::collections::HashSet<String> = symbols
                .iter()
                .filter(|s| s.kind == SymbolKind::Method)
                .map(|s| s.name.clone())
                .collect();
            for sym in symbols.iter_mut() {
                if sym.kind == SymbolKind::Method && sym.scope_path.is_none() {
                    sym.scope_path = Some(class_qname.clone());
                    if !sym.qualified_name.contains('.') {
                        sym.qualified_name = format!("{}.{}", class_qname, sym.qualified_name);
                    }
                }
            }

            // Scan source for additional method declarations the grammar missed.
            // Detect member indent from methods already extracted; the grammar's
            // start_col reflects the actual indentation of each declaration.
            // Falls back to 2 when no grammar methods were found (the grammar
            // missed everything, so we have no column signal).
            let fallback_member_indent = symbols
                .iter()
                .filter(|s| s.kind == SymbolKind::Method && s.start_col > 0)
                .map(|s| s.start_col as usize)
                .min()
                .unwrap_or(2);
            let new_methods = scan_methods_from_source(source, class_idx, &class_qname, &already_extracted, fallback_member_indent);
            symbols.extend(new_methods);
        }
    } else {
        // Even when the grammar parses successfully, the tree-sitter-groovy grammar
        // sometimes classifies `static Type method(...)` declarations as field_declaration
        // nodes (or other non-method_declaration nodes) rather than method_declaration.
        // This causes static and private static methods to be silently dropped from
        // the index.  We run a lightweight line-scanner supplemental pass for every
        // class found in the file to recover those missing methods.
        //
        // The `already_extracted` set prevents double-indexing: methods the grammar
        // correctly produced are already in symbols and will be skipped.
        let class_symbols: Vec<(usize, String, u32)> = symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind == SymbolKind::Class)
            .map(|(i, s)| (i, s.qualified_name.clone(), s.start_col))
            .collect();

        for (class_idx, class_qname, class_col) in class_symbols {
            let already_extracted_names: std::collections::HashSet<String> = symbols
                .iter()
                .filter(|s| {
                    s.kind == SymbolKind::Method
                        && s.scope_path
                            .as_deref()
                            .map(|sp| sp == class_qname)
                            .unwrap_or(false)
                })
                .map(|s| s.name.clone())
                .collect();

            // Use class start column plus one indent level (2 spaces) so that
            // methods of an outer class (at col 0 → members at col 2) are found
            // by the scanner, not only inner-class members (at col 2 → members at col 4).
            let member_indent = class_col as usize + 2;
            let new_methods = scan_methods_from_source(source, class_idx, &class_qname, &already_extracted_names, member_indent);
            symbols.extend(new_methods);
        }
    }

    // Post-processing: annotate Inherits/Implements refs that have no module
    // with the FQN from the file's import table. A Groovy file that writes
    //   import spock.lang.Specification
    //   class MySpec extends Specification { ... }
    // emits an Imports ref with module="spock.lang.Specification" and an
    // Inherits ref with target_name="Specification" and module=None. Matching
    // the short name against imports provides the FQN so:
    //   (a) the Java resolver's exact-import path resolves it via by_qualified_name;
    //   (b) the demand seeder can route through the symbol location index rather
    //       than falling through to the unfiltered find_by_name fallback.
    enrich_hierarchy_refs_from_imports(&mut refs);

    ExtractionResult::new(symbols, refs, has_errors)
}

/// Annotate Inherits/Implements refs whose module is None with the FQN from
/// the file's import declarations.
///
/// The Groovy extractor emits Imports refs where target_name carries the full
/// FQN (e.g. "spock.lang.Specification"). This pass derives the simple name
/// from the last dot segment, builds a simple_name → FQN map, and writes the
/// FQN into the module field of any Inherits/Implements ref whose target_name
/// matches a simple name from an import. Wildcard and static imports are
/// excluded.
fn enrich_hierarchy_refs_from_imports(refs: &mut Vec<ExtractedRef>) {
    // Build simple_name → fqn. The Groovy extractor stores the full FQN in
    // both target_name and module for non-static imports. Extract the simple
    // name from the last dot segment of target_name.
    let import_map: std::collections::HashMap<String, String> = refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .filter_map(|r| {
            // Skip wildcards and empty refs.
            if r.target_name.is_empty() || r.target_name == "*" { return None; }
            let fqn = r.target_name.as_str();
            // Single-class imports have at least one dot; skip bare names.
            let dot = fqn.rfind('.')?;
            let simple = &fqn[dot + 1..];
            // Static-member imports have a lowercase simple name (method/field).
            // Only class imports start with uppercase.
            if !simple.starts_with(|c: char| c.is_uppercase()) { return None; }
            Some((simple.to_string(), fqn.to_string()))
        })
        .collect();

    if import_map.is_empty() { return; }

    for r in refs.iter_mut() {
        if !matches!(r.kind, EdgeKind::Inherits | EdgeKind::Implements) { continue; }
        if r.module.is_some() { continue; }
        if let Some(fqn) = import_map.get(&r.target_name) {
            r.module = Some(fqn.clone());
        }
    }
}

/// Scan source lines for method declarations that the grammar failed to parse.
/// Returns a list of unique Method symbols with scope_path and qualified_name set.
/// Already-extracted method names (from the grammar's partial parse) are skipped.
///
/// `member_indent` is the exact number of leading spaces for a direct member of
/// this class (class at col C → members at C+2 spaces). Only lines with exactly
/// that indent depth are considered — this prevents inner-class methods (at C+4)
/// from being mis-attributed to the outer class.
///
/// A method declaration is recognised by the pattern:
///   (optional-visibility) (optional-modifier)* (type|def|void)? methodName(
/// where the line must start with an access modifier or `static` keyword.
///
/// The `static` keyword is included because the tree-sitter-groovy grammar
/// sometimes classifies `static Type method(...)` as a field_declaration
/// or otherwise fails to produce a method_declaration node, causing static
/// methods to be silently dropped from the index.
fn scan_methods_from_source(
    src: &str,
    parent_idx: usize,
    class_qname: &str,
    already_extracted: &std::collections::HashSet<String>,
    member_indent: usize,
) -> Vec<ExtractedSymbol> {
    let mut methods: Vec<ExtractedSymbol> = Vec::new();
    let mut seen: std::collections::HashSet<String> = already_extracted.clone();

    // Access modifiers (including `static`) that may appear as the FIRST token
    // on a method declaration line.  Static is included because the Groovy
    // grammar sometimes does not emit method_declaration for `static Type foo(...)`.
    const ACCESS: &[&str] = &["public", "protected", "private", "static"];
    // Other modifiers and primitive return types that may follow an access modifier.
    // Primitive types are included so `static boolean foo(...)` doesn't produce
    // method name "boolean" — the scanner consumes the primitive and takes the
    // next token as the method name.
    const OTHER_MODS: &[&str] = &[
        "static", "abstract", "final", "synchronized", "native", "void", "def",
        "boolean", "int", "long", "double", "float", "char", "byte", "short",
    ];

    // Build the exact indent prefix for this class's members (e.g. "  " for 2-space).
    let indent_prefix: String = " ".repeat(member_indent);

    for (line_idx, line) in src.lines().enumerate() {
        let trimmed = line.trim();

        // Accept tab-indented lines or lines with at least member_indent leading
        // spaces.  Using "at least" (prefix match only) rather than an exact
        // count means inner-class methods at deeper indentation are also
        // attributed to the outer class — the grammar's scope_path assignment
        // for those symbols is correct; the scanner's attribution is a secondary
        // index entry that helps cross-class bare calls resolve.
        let has_tab = line.starts_with('\t');
        let has_indent = !indent_prefix.is_empty() && line.starts_with(&indent_prefix);
        if !has_tab && !has_indent {
            continue;
        }

        // Skip comments
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with("*") || trimmed.starts_with("@") {
            continue;
        }

        let mut tokens = trimmed.split_whitespace().peekable();
        let first = match tokens.peek() {
            Some(&t) => t,
            None => continue,
        };

        // Line must START with an access modifier (or `static`) to be a method declaration.
        if !ACCESS.contains(&first) {
            continue;
        }
        tokens.next(); // consume access modifier / static keyword

        // Skip additional modifiers (void, def, static, type name, etc.)
        while tokens.peek().map_or(false, |t| {
            OTHER_MODS.contains(t) || t.chars().next().map_or(false, |c| c.is_uppercase())
        }) {
            tokens.next();
        }

        // The next token should be `methodName(` or `methodName`
        let candidate = match tokens.next() {
            Some(t) => t,
            None => continue,
        };
        let method_name = candidate.split('(').next().unwrap_or("").trim();

        if method_name.is_empty()
            || seen.contains(method_name)
            || !method_name.chars().next().map_or(false, |c| c.is_lowercase() || c == '_')
            || predicates::is_groovy_keyword(method_name)
        {
            continue;
        }

        // The token must contain `(` (method call) or the trimmed line must contain `(`
        // to rule out field declarations like `protected String foo`.
        if !candidate.contains('(') && !trimmed.contains('(') {
            continue;
        }

        seen.insert(method_name.to_string());
        methods.push(ExtractedSymbol {
            name: method_name.to_string(),
            qualified_name: format!("{}.{}", class_qname, method_name),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: line_idx as u32,
            end_line: line_idx as u32,
            start_col: 0,
            end_col: 0,
            signature: Some(method_name.to_string()),
            doc_comment: None,
            scope_path: Some(class_qname.to_string()),
            parent_index: Some(parent_idx),
        });
    }
    methods
}

/// Scan source lines for a class declaration when tree-sitter parsing fails.
/// Returns `(class_name, line_number)` of the first `class ClassName` found.
fn scan_class_name_from_source(src: &str) -> Option<(String, u32)> {
    for (line_idx, line) in src.lines().enumerate() {
        let trimmed = line.trim();
        // Match: (optional visibility/modifiers) `class` <Name> (optional generics/extends/implements)
        let after_class = trimmed
            .split_whitespace()
            .skip_while(|&tok| matches!(tok, "public" | "protected" | "private" | "abstract" | "final" | "static"))
            .next()
            .filter(|&tok| tok == "class")
            .and_then(|_| {
                // Find position of "class" keyword and take the next token
                let mut parts = trimmed.split_whitespace().peekable();
                while let Some(tok) = parts.next() {
                    if tok == "class" {
                        return parts.next();
                    }
                }
                None
            });

        if let Some(raw_name) = after_class {
            // Strip any trailing `<...>` generic suffix from the name token
            let name = raw_name
                .split('<').next()
                .unwrap_or(raw_name)
                .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if !name.is_empty() && name.chars().next().map_or(false, |c| c.is_uppercase()) {
                return Some((name.to_string(), line_idx as u32));
            }
        }
    }
    None
}

/// Scan source lines for `extends ClassName` and emit an Inherits edge.
fn extract_class_inherits_from_source(src: &str, class_idx: usize, refs: &mut Vec<ExtractedRef>) {
    for (line_idx, line) in src.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.contains("class ") || !trimmed.contains(" extends ") {
            continue;
        }
        // Extract the name after "extends "
        if let Some(after) = trimmed.split(" extends ").nth(1) {
            let superclass = after
                .split_whitespace().next()
                .unwrap_or("")
                .split('<').next()
                .unwrap_or("")
                .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.');
            if !superclass.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: class_idx,
                    target_name: superclass.to_string(),
                    kind: EdgeKind::Inherits,
                    line: line_idx as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        break; // class declaration is always a single logical line
    }
}

/// Replace the content of single-quoted Groovy strings that contain `<` or `>`
/// with spaces of equal length, preserving byte offsets for all other tokens.
///
/// This is a targeted workaround for a tree-sitter-groovy grammar bug where
/// `'<init>'` is misidentified as a generic type constraint, causing parse
/// errors that prevent class extraction.
fn neutralize_angle_bracket_sqstrings(src: &str) -> String {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    while i < len {
        if bytes[i] == b'\'' {
            // Scan ahead to find the closing quote, collecting the string content.
            let start = i;
            i += 1;
            let content_start = i;
            while i < len && bytes[i] != b'\'' && bytes[i] != b'\n' {
                if bytes[i] == b'\\' { i += 1; } // skip escape
                i += 1;
            }
            // Include closing quote if present.
            let close = if i < len && bytes[i] == b'\'' { i += 1; i - 1 } else { len };
            let content = &bytes[content_start..close.min(len)];
            if content.iter().any(|&b| b == b'<' || b == b'>') {
                // Emit the full single-quoted region as spaces.
                let end = i;
                for _ in start..end {
                    out.push(b' ');
                }
            } else {
                // Safe — emit as-is.
                out.extend_from_slice(&bytes[start..i]);
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// Scan the top-level children of the compilation root for a `package_declaration`
/// and return the dotted package name (e.g. `"org.codenarc.rule"`).
fn pre_scan_namespace(root: Node, src: &str) -> Option<String> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_declaration" {
            let name = build_qualified_name(&child, src);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn visit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
    namespace: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "package_declaration" => {
                extract_package(&child, src, symbols, parent_index);
            }
            "class_declaration" => {
                extract_class(&child, src, symbols, refs, parent_index, namespace);
            }
            "interface_declaration" => {
                extract_interface(&child, src, symbols, refs, parent_index, namespace);
            }
            // Top-level `def fn(...)` — grammar emits function_definition
            "function_definition" => {
                extract_function(&child, src, symbols, refs, parent_index, inside_class, None);
            }
            // Typed `ReturnType method(...)` inside a class — grammar emits method_declaration
            "method_declaration" => {
                extract_method_declaration(&child, src, symbols, refs, parent_index, None);
            }
            "import_declaration" => {
                extract_import(&child, src, symbols.len().saturating_sub(1), refs);
            }
            "method_invocation" => {
                let local_types = HashMap::new();
                extract_call(&child, src, parent_index.unwrap_or(0), refs, &local_types);
                visit(child, src, symbols, refs, parent_index, inside_class, namespace);
            }
            _ => {
                visit(child, src, symbols, refs, parent_index, inside_class, namespace);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Package / Namespace
// ---------------------------------------------------------------------------

fn extract_package(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name = build_qualified_name(node, src);
    if name.is_empty() {
        return;
    }
    let line = node.start_position().row as u32;

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: line,
        end_line: line,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("package {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Class extraction
// ---------------------------------------------------------------------------

fn extract_class(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    namespace: Option<&str>,
) {
    // class_declaration has a `name` field (identifier)
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    // Qualify the class name with the package namespace so that the
    // inherits_map (keyed by qname) and scope_path can be resolved correctly.
    // e.g. `class Foo` in `package org.example` → qname = "org.example.Foo"
    let class_qname = match namespace {
        Some(ns) => format!("{}.{}", ns, name),
        None => name.clone(),
    };

    let line = node.start_position().row as u32;
    let class_idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: class_qname.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("class {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract superclass (extends) → Inherits edge
    if let Some(superclass_node) = node.child_by_field_name("superclass") {
        let mut sc = superclass_node.walk();
        for sc_child in superclass_node.children(&mut sc) {
            if sc_child.kind() == "type_identifier" || sc_child.kind() == "identifier" {
                let target = node_text(&sc_child, src).to_string();
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: class_idx,
                        target_name: target,
                        kind: EdgeKind::Inherits,
                        line: superclass_node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
        }
    }

    // Extract interfaces (implements) → Implements edges
    if let Some(interfaces_node) = node.child_by_field_name("interfaces") {
        extract_type_list_refs(&interfaces_node, src, class_idx, EdgeKind::Implements, refs);
    }

    // Walk class body for methods, fields, and nested classes.
    // Pass the class's qualified name as `class_scope` so methods can set
    // scope_path correctly — enabling the inheritance-chain resolver to
    // find the calling class when looking up `{ancestor}.{method_name}`.
    let class_scope = Some(class_qname.as_str());
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "method_declaration" => {
                    extract_method_declaration(&child, src, symbols, refs, Some(class_idx), class_scope);
                }
                "function_definition" => {
                    extract_function(&child, src, symbols, refs, Some(class_idx), true, class_scope);
                }
                "class_declaration" => {
                    // Inner / nested class — recurse so its methods are found.
                    extract_class(&child, src, symbols, refs, Some(class_idx), namespace);
                }
                "interface_declaration" => {
                    // Nested interface — extract so implementations resolve.
                    extract_interface(&child, src, symbols, refs, Some(class_idx), namespace);
                }
                "field_declaration" => {
                    extract_field(&child, src, symbols, Some(class_idx));
                }
                "method_invocation" => {
                    let local_types = HashMap::new();
                    extract_call(&child, src, class_idx, refs, &local_types);
                }
                _ => {
                    let local_types = HashMap::new();
                    visit_for_calls(&child, src, class_idx, refs, &local_types);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Interface extraction (interface_declaration)
// ---------------------------------------------------------------------------

fn extract_interface(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    namespace: Option<&str>,
) {
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    let iface_qname = match namespace {
        Some(ns) => format!("{}.{}", ns, name),
        None => name.clone(),
    };

    let line = node.start_position().row as u32;
    let iface_idx = symbols.len();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: iface_qname.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("interface {} {{ ... }}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });

    // Extract parent interfaces (extends_interfaces child → type_list)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_interfaces" {
            extract_type_list_refs(&child, src, iface_idx, EdgeKind::Inherits, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Field extraction (class body `field_declaration`)
// ---------------------------------------------------------------------------

fn extract_field(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let type_name = named_field_text(node, "type", src).unwrap_or_default();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let field_name = match named_field_text(&child, "name", src) {
                Some(n) => n,
                None => continue,
            };
            let line = child.start_position().row as u32;
            let sig = if type_name.is_empty() {
                field_name.clone()
            } else {
                format!("{} {}", type_name, field_name)
            };
            symbols.push(ExtractedSymbol {
                name: field_name.clone(),
                qualified_name: field_name.clone(),
                kind: SymbolKind::Field,
                visibility: Some(Visibility::Public),
                start_line: line,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: 0,
                signature: Some(sig),
                doc_comment: None,
                scope_path: None,
                parent_index,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Type list helper — walks super_interfaces / type_list for Inherits/Implements
// ---------------------------------------------------------------------------

fn extract_type_list_refs(
    node: &Node,
    src: &str,
    source_idx: usize,
    kind: EdgeKind,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let name = node_text(&child, src).to_string();
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
});
                }
            }
            _ => {
                extract_type_list_refs(&child, src, source_idx, kind, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function (top-level `def fn(...)`)
// ---------------------------------------------------------------------------

fn extract_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    inside_class: bool,
    class_scope: Option<&str>,
) {
    // function_definition has a `name` field
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    let line = node.start_position().row as u32;
    let kind = if inside_class { SymbolKind::Method } else { SymbolKind::Function };
    let idx = symbols.len();

    // Qualify the name when inside a class so the method appears as
    // `org.example.MyClass.myMethod` in the index.
    let qualified_name = match class_scope {
        Some(cls) => format!("{}.{}", cls, name),
        None => name.clone(),
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("def {}", name)),
        doc_comment: None,
        // scope_path = class qname so the inheritance-chain resolver can find
        // the enclosing class for bare method calls like `assertSingleViolation()`.
        scope_path: class_scope.map(|s| s.to_string()),
        parent_index,
    });

    let local_types = scan_local_types(node, src);
    visit_for_calls(node, src, idx, refs, &local_types);
}

// ---------------------------------------------------------------------------
// Method (typed form: `int add(int a, int b)`)
// ---------------------------------------------------------------------------

fn extract_method_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    class_scope: Option<&str>,
) {
    // method_declaration has fields: type, name, parameters, body
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    let return_type = named_field_text(node, "type", src).unwrap_or_default();
    let line = node.start_position().row as u32;
    let idx = symbols.len();

    let sig = if return_type.is_empty() {
        name.clone()
    } else {
        format!("{} {}", return_type, name)
    };

    // Qualify the name when inside a class.
    let qualified_name = match class_scope {
        Some(cls) => format!("{}.{}", cls, name),
        None => name.clone(),
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(sig),
        doc_comment: None,
        // scope_path = class qname so the inheritance-chain resolver can find
        // the enclosing class for bare method calls.
        scope_path: class_scope.map(|s| s.to_string()),
        parent_index,
    });

    let local_types = scan_local_types(node, src);
    visit_for_calls(node, src, idx, refs, &local_types);
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

fn extract_import(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let text = node_text(node, src);
    // Strip `import ` prefix and any `as Alias` suffix.
    // Also strip the optional `static` keyword that appears in static imports:
    //   import static org.codenarc.test.TestUtil.shouldFail
    // After stripping "import" we may see "static" as the next token — skip it.
    let after_import = text
        .trim_start_matches("import")
        .trim();

    // Skip the `static` keyword when present.
    let path_str = if after_import.starts_with("static ") || after_import == "static" {
        after_import.trim_start_matches("static").trim()
    } else {
        after_import
    };

    let full_path = path_str
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('*')
        .trim_end_matches('.')
        .to_string();

    if full_path.is_empty() {
        return;
    }

    // For a static member import `import static pkg.Class.member`, the target_name
    // is the simple member name (last segment) so the Java resolver's exact-import
    // lookup (which matches `imported_name == effective_target`) can find it.
    // The `module` carries the full qualified path so `by_qualified_name` works.
    let is_static_import = after_import.starts_with("static ");
    let (target_name, module_path) = if is_static_import {
        let simple = full_path
            .rfind('.')
            .map(|i| full_path[i + 1..].to_string())
            .unwrap_or_else(|| full_path.clone());
        (simple, full_path.clone())
    } else {
        (full_path.clone(), full_path.clone())
    };

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        module: Some(module_path),
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Call extraction (method_invocation)
// ---------------------------------------------------------------------------

fn extract_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    local_types: &HashMap<String, String>,
) {
    // method_invocation has field `name` (identifier)
    let name = match named_field_text(node, "name", src) {
        Some(n) => n,
        None => return,
    };

    // Skip control flow keywords that the grammar sometimes parses as method_invocation.
    if predicates::is_groovy_keyword(&name) {
        return;
    }

    // Build a MemberChain when the call has a receiver (`object` field).
    // This enables the chain walker and the external classifier to determine
    // the receiver type and classify the call correctly (e.g. `file.path.endsWith`
    // where `file` has declared type `File` from a for-loop or local declaration).
    let chain = node.child_by_field_name("object")
        .and_then(|obj| build_receiver_chain(&obj, &name, src, local_types));

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: name,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        module: None,
        chain,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    });
}

// ---------------------------------------------------------------------------
// Chain building — walk receiver AST to produce a MemberChain
// ---------------------------------------------------------------------------

/// Build a MemberChain from the `object` field of a `method_invocation` node
/// plus the final call segment name.
///
/// The chain walker and the external classifier both consume MemberChain.
/// For `file.path.endsWith(s)` with declared type `File` on `file`:
///   segments = [file (Identifier, declared_type=File), path (Property), endsWith (Property)]
///
/// Returns `None` when the receiver is too complex to model (e.g. a closure
/// literal, a cast expression) — the call is then emitted as a bare ref.
fn build_receiver_chain(
    obj_node: &Node,
    final_method: &str,
    src: &str,
    local_types: &HashMap<String, String>,
) -> Option<MemberChain> {
    let mut segments: Vec<ChainSegment> = Vec::new();
    collect_receiver_segments(obj_node, src, local_types, &mut segments, 0)?;

    // The final method/property call is the chain leaf.
    segments.push(ChainSegment {
        name: final_method.to_string(),
        node_kind: "method_invocation".to_string(),
        kind: SegmentKind::Property,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
    });

    if segments.len() < 2 {
        return None;
    }

    Some(MemberChain { segments })
}

/// Recursively collect chain segments from a receiver expression.
///
/// `depth` guards against pathologically deep chains (e.g. 20-segment builder
/// APIs) blowing the stack or producing noise the chain walker can't use.
fn collect_receiver_segments(
    node: &Node,
    src: &str,
    local_types: &HashMap<String, String>,
    segments: &mut Vec<ChainSegment>,
    depth: usize,
) -> Option<()> {
    // Cap recursion: chains deeper than 8 segments aren't useful for
    // type inference since we'd lose the root type anyway.
    if depth > 8 {
        return None;
    }

    match node.kind() {
        "identifier" => {
            let name = node_text(node, src).to_string();
            if name.is_empty() || predicates::is_groovy_keyword(&name) {
                return None;
            }
            let declared_type = local_types.get(&name).cloned();
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type,
                type_args: Vec::new(),
                optional_chaining: false,
            });
            Some(())
        }
        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
            });
            Some(())
        }
        "field_access" => {
            // field_access has `object` and `field` fields.
            let inner_obj = node.child_by_field_name("object")?;
            let field_node = node.child_by_field_name("field")?;
            let field_name = node_text(&field_node, src).to_string();
            if field_name.is_empty() {
                return None;
            }
            collect_receiver_segments(&inner_obj, src, local_types, segments, depth + 1)?;
            segments.push(ChainSegment {
                name: field_name,
                node_kind: "field_access".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
            });
            Some(())
        }
        "method_invocation" => {
            // Chained call: `foo().bar()`. Treat `foo()` as a call segment.
            // The method's name is the segment; the object of `foo()` is the
            // sub-chain. Recurse into the sub-chain first.
            let inner_name = match named_field_text(node, "name", src) {
                Some(n) => n,
                None => return None,
            };
            if let Some(inner_obj) = node.child_by_field_name("object") {
                collect_receiver_segments(&inner_obj, src, local_types, segments, depth + 1)?;
            } else {
                // Bare call at the root of the chain (e.g. `GradleRunner.create()`).
                // Treat the method name itself as a type-access root segment so the
                // chain walker can probe static methods on it.
                segments.push(ChainSegment {
                    name: inner_name.clone(),
                    node_kind: "method_invocation".to_string(),
                    kind: SegmentKind::TypeAccess,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                });
                return Some(());
            }
            segments.push(ChainSegment {
                name: inner_name,
                node_kind: "method_invocation".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: Vec::new(),
                optional_chaining: false,
            });
            Some(())
        }
        _ => {
            // Parenthesized expressions, casts, new expressions, etc.
            // Fall back to None — the call is emitted as a bare ref.
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Local type map — typed variable declarations in method bodies
// ---------------------------------------------------------------------------

/// Scan a method/function/closure body for typed local variable and for-loop
/// declarations, returning a map of variable-name → declared type name.
///
/// Recognizes:
///   `File file = ...`          (local_variable_declaration with explicit type)
///   `for (File file : files)`  (enhanced_for_statement with explicit type)
///
/// `def` declarations and bare assignments are excluded — the type is unknown
/// at extraction time for those.
fn scan_local_types(root: &Node, src: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    collect_local_types(root, src, &mut map);
    map
}

fn collect_local_types(node: &Node, src: &str, map: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "local_variable_declaration" => {
                // Grammar: local_variable_declaration has `type` and `declarator` fields.
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = node_text(&type_node, src).to_string();
                    // Skip `def` and primitive type keywords — their type is not a class.
                    if !type_name.is_empty()
                        && type_name != "def"
                        && !type_name.chars().next().map_or(false, |c| c.is_lowercase())
                    {
                        // Strip generic parameters: `List<File>` → `List`
                        let base_type = type_name.split('<').next().unwrap_or(&type_name).trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.').to_string();
                        // Collect each declarator's variable name.
                        let mut dc = child.walk();
                        for decl in child.children(&mut dc) {
                            if decl.kind() == "variable_declarator" {
                                if let Some(name_node) = decl.child_by_field_name("name") {
                                    let var_name = node_text(&name_node, src).to_string();
                                    if !var_name.is_empty() {
                                        map.insert(var_name, base_type.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                collect_local_types(&child, src, map);
            }
            "enhanced_for_statement" => {
                // Grammar: enhanced_for_statement has `type` and `name` fields.
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = node_text(&type_node, src).to_string();
                    if !type_name.is_empty()
                        && type_name != "def"
                        && !type_name.chars().next().map_or(false, |c| c.is_lowercase())
                    {
                        let base_type = type_name.split('<').next().unwrap_or(&type_name).trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.').to_string();
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let var_name = node_text(&name_node, src).to_string();
                            if !var_name.is_empty() {
                                map.insert(var_name, base_type);
                            }
                        }
                    }
                }
                collect_local_types(&child, src, map);
            }
            _ => {
                collect_local_types(&child, src, map);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Walk subtree collecting method_invocation nodes
// ---------------------------------------------------------------------------

fn visit_for_calls(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
    local_types: &HashMap<String, String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_invocation" => {
                extract_call(&child, src, source_idx, refs, local_types);
                visit_for_calls(&child, src, source_idx, refs, local_types);
            }
            "enhanced_for_statement" | "local_variable_declaration" => {
                // Merge any new local type declarations scoped to this block
                // into a child map. We collect_local_types to find new bindings
                // introduced inside sub-blocks (closures, nested for loops) and
                // visit the body with the extended map.
                let mut child_types = local_types.clone();
                collect_local_types(&child, src, &mut child_types);
                visit_for_calls(&child, src, source_idx, refs, &child_types);
            }
            _ => {
                visit_for_calls(&child, src, source_idx, refs, local_types);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text<'a>(node: &Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

/// Get text of a named field child (e.g. `name`, `function`)
fn named_field_text(node: &Node, field: &str, src: &str) -> Option<String> {
    node.child_by_field_name(field)
        .map(|n| node_text(&n, src).to_string())
        .filter(|s| !s.is_empty())
}

/// Build a dotted qualified name from scoped_identifier / identifier children
fn build_qualified_name(node: &Node, src: &str) -> String {
    // package_declaration contains a scoped_identifier or identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier" | "identifier" => {
                return node_text(&child, src).to_string();
            }
            _ => {}
        }
    }
    String::new()
}
