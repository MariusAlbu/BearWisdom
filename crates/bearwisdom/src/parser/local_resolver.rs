// =============================================================================
// parser/local_resolver.rs — tree-sitter locals.scm query-based scope resolver
//
// Uses the locals.scm query from a tree-sitter grammar to identify:
//   @local.scope      — where a new variable scope opens
//   @local.definition — where a name is introduced (variable, parameter, etc.)
//   @local.reference  — where a name is used
//
// Resolves references to definitions within the same or enclosing scope.
// Any reference NOT locally resolved is a genuine cross-file ref that the
// resolution engine should handle.
//
// Also supports the alternative capture naming convention used by some grammars:
//   @scope, @definition.var, @definition.function, @reference, etc.
// =============================================================================

use rustc_hash::FxHashSet;
use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator, Tree};

/// Compiled locals.scm query with capture indices resolved.
pub struct LocalResolver {
    query: Query,
    /// Capture indices that mark scope boundaries.
    scope_captures: Vec<u32>,
    /// Capture indices that mark definitions (variable introductions).
    definition_captures: Vec<u32>,
    /// Capture indices that mark references (name usages).
    reference_captures: Vec<u32>,
}

/// Result of running local resolution on a single file.
pub struct LocalResolution {
    /// Byte offsets of references that were resolved to a local definition.
    /// If a ref's start byte is in this set, it should NOT be emitted as
    /// an ExtractedRef — it's a local variable, not a cross-file reference.
    pub locally_resolved: FxHashSet<usize>,
}

impl LocalResolution {
    /// Check if an identifier at the given byte offset is locally resolved.
    pub fn is_local(&self, byte_offset: usize) -> bool {
        self.locally_resolved.contains(&byte_offset)
    }

    /// Number of locally resolved references.
    pub fn resolved_count(&self) -> usize {
        self.locally_resolved.len()
    }

    /// Empty resolution (no locals.scm available).
    pub fn empty() -> Self {
        Self {
            locally_resolved: FxHashSet::default(),
        }
    }
}

impl LocalResolver {
    /// Compile a locals.scm query for the given language.
    ///
    /// Returns `None` if the query string is empty or fails to compile.
    pub fn new(locals_scm: &str, language: Language) -> Option<Self> {
        if locals_scm.trim().is_empty() {
            return None;
        }

        let query = Query::new(&language, locals_scm).ok()?;

        let mut scope_captures = Vec::new();
        let mut definition_captures = Vec::new();
        let mut reference_captures = Vec::new();

        for (i, name) in query.capture_names().iter().enumerate() {
            let idx = i as u32;
            let n: &str = name;
            if n == "local.scope" || n == "scope" || n == "_scope" {
                scope_captures.push(idx);
            } else if n.starts_with("local.definition") || n.starts_with("definition") {
                definition_captures.push(idx);
            } else if n == "local.reference" || n == "reference" {
                reference_captures.push(idx);
            }
        }

        // Must have at least definitions and references to be useful.
        if definition_captures.is_empty() || reference_captures.is_empty() {
            return None;
        }

        Some(Self {
            query,
            scope_captures,
            definition_captures,
            reference_captures,
        })
    }

    /// Run local resolution on a parsed tree.
    ///
    /// Builds a scope tree from query matches, then resolves each reference
    /// to a definition in the same or enclosing scope.
    pub fn resolve(&self, tree: &Tree, source: &[u8]) -> LocalResolution {
        let mut cursor = QueryCursor::new();
        let root = tree.root_node();

        // Collect all captures in one pass.
        let mut scopes: Vec<ScopeSpan> = Vec::new();
        let mut definitions: Vec<DefEntry> = Vec::new();
        let mut references: Vec<RefEntry> = Vec::new();

        let mut matches = cursor.matches(&self.query, root, source);
        while let Some(m) = StreamingIterator::next(&mut matches) {
            for capture in m.captures {
                let node = capture.node;
                let idx = capture.index;

                if self.scope_captures.contains(&idx) {
                    scopes.push(ScopeSpan {
                        start: node.start_byte(),
                        end: node.end_byte(),
                    });
                } else if self.definition_captures.contains(&idx) {
                    let name = node_text(node, source);
                    if !name.is_empty() {
                        definitions.push(DefEntry {
                            name,
                            byte_offset: node.start_byte(),
                            scope_start: 0, // filled in below
                        });
                    }
                } else if self.reference_captures.contains(&idx) {
                    let name = node_text(node, source);
                    if !name.is_empty() {
                        references.push(RefEntry {
                            name,
                            byte_offset: node.start_byte(),
                        });
                    }
                }
            }
        }

        // Sort scopes by start position (outer scopes first for tie-breaking).
        scopes.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));

        // Assign each definition to its innermost enclosing scope.
        for def in &mut definitions {
            def.scope_start = innermost_scope(&scopes, def.byte_offset)
                .map(|s| s.start)
                .unwrap_or(0);
        }

        // Build a quick lookup: (scope_start, name) → exists
        let def_set: FxHashSet<(usize, &str)> = definitions
            .iter()
            .map(|d| (d.scope_start, d.name.as_str()))
            .collect();

        // Resolve references: walk up the scope chain looking for a definition.
        let mut locally_resolved = FxHashSet::default();

        for r in &references {
            // Skip if this reference IS a definition (same byte offset).
            if definitions.iter().any(|d| d.byte_offset == r.byte_offset) {
                locally_resolved.insert(r.byte_offset);
                continue;
            }

            // Walk enclosing scopes from innermost to outermost.
            if resolve_in_scopes(&scopes, &def_set, r.byte_offset, &r.name) {
                locally_resolved.insert(r.byte_offset);
            }
        }

        LocalResolution { locally_resolved }
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

struct ScopeSpan {
    start: usize,
    end: usize,
}

struct DefEntry {
    name: String,
    byte_offset: usize,
    /// Start byte of the innermost scope containing this definition.
    scope_start: usize,
}

struct RefEntry {
    name: String,
    byte_offset: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the text of a tree-sitter node.
fn node_text(node: Node, source: &[u8]) -> String {
    let start = node.start_byte();
    let end = node.end_byte().min(source.len());
    if start >= end {
        return String::new();
    }
    String::from_utf8_lossy(&source[start..end]).to_string()
}

/// Find the innermost scope containing the given byte offset.
fn innermost_scope(scopes: &[ScopeSpan], byte_offset: usize) -> Option<&ScopeSpan> {
    let mut best: Option<&ScopeSpan> = None;
    for s in scopes {
        if s.start <= byte_offset && byte_offset < s.end {
            match best {
                None => best = Some(s),
                Some(prev) if (s.end - s.start) < (prev.end - prev.start) => best = Some(s),
                _ => {}
            }
        }
    }
    best
}

/// Check if a name is defined in the scope at `scope_start` or any enclosing scope.
fn resolve_in_scopes(
    scopes: &[ScopeSpan],
    def_set: &FxHashSet<(usize, &str)>,
    byte_offset: usize,
    name: &str,
) -> bool {
    // Check the file-level scope (scope_start = 0).
    if def_set.contains(&(0, name)) {
        return true;
    }

    // Walk from innermost scope outward.
    // Collect enclosing scopes sorted by size (smallest = innermost).
    let mut enclosing: Vec<&ScopeSpan> = scopes
        .iter()
        .filter(|s| s.start <= byte_offset && byte_offset < s.end)
        .collect();
    enclosing.sort_by_key(|s| s.end - s.start);

    for scope in enclosing {
        if def_set.contains(&(scope.start, name)) {
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn js_language() -> Language {
        tree_sitter_javascript::LANGUAGE.into()
    }

    fn parse_js(source: &[u8]) -> Tree {
        let lang = js_language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_javascript_locals() {
        let js_locals = r#"
            [(statement_block) (function_declaration) (arrow_function)] @local.scope
            (variable_declarator name: (identifier) @local.definition)
            (identifier) @local.reference
        "#;

        let resolver = LocalResolver::new(js_locals, js_language()).unwrap();

        let source = b"function foo(x) { let bar = 1; return bar + x; }";
        let tree = parse_js(source);
        let resolution = resolver.resolve(&tree, source);

        // "bar" at position of `return bar` should be locally resolved
        // (defined by `let bar = 1`).
        assert!(resolution.resolved_count() > 0);
    }

    #[test]
    fn test_empty_query_returns_none() {
        assert!(LocalResolver::new("", js_language()).is_none());
        assert!(LocalResolver::new("   \n  ", js_language()).is_none());
    }

    #[test]
    fn test_no_definitions_returns_none() {
        let query = "(identifier) @local.reference";
        // No @local.definition → resolver is useless
        assert!(LocalResolver::new(query, js_language()).is_none());
    }

    #[test]
    fn test_unresolved_ref_not_in_set() {
        let js_locals = r#"
            [(statement_block) (function_declaration)] @local.scope
            (variable_declarator name: (identifier) @local.definition)
            (identifier) @local.reference
        "#;

        let resolver = LocalResolver::new(js_locals, js_language()).unwrap();

        // `unknown` is never defined locally — should NOT be in resolved set.
        let source = b"function foo() { return unknown; }";
        let tree = parse_js(source);
        let resolution = resolver.resolve(&tree, source);

        // Find the byte offset of "unknown".
        let unknown_pos = source.windows(7).position(|w| w == b"unknown").unwrap();
        assert!(!resolution.is_local(unknown_pos));
    }
}
