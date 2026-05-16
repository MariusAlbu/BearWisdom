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
    EdgeKind, ExtractionResult, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility,
};
use super::predicates;
use super::ast_visit::visit;
use super::node_helpers::build_qualified_name;
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
                                    call_args: Vec::new(),
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
