// =============================================================================
// dart/imports.rs — Import / export / part directive ref extraction, and
// library-prefix binding for qualified references under a prefixed import.
// =============================================================================

use super::helpers::{first_child_text_of_kind, node_text};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Import / export directives
// ---------------------------------------------------------------------------

pub(super) fn extract_import_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_import_spec_recursive(node, src, current_symbol_count, refs);
}

/// `true` when `node` (an `import_specification`) carries an `as <alias>`
/// clause. A prefixed import never opens the bare scope — qualified
/// references route through `bind_prefixed_refs` instead.
fn has_alias_clause(node: &Node) -> bool {
    node.child_by_field_name("alias").is_some()
}

/// `true` when none of `node`'s `combinator` children is a `show` list. A
/// `show X, Y` combinator limits the brought-in names to its list — not a
/// wildcard. A `hide` combinator (or no combinator at all) still brings in
/// every OTHER declaration, approximated here as a full wildcard.
fn combinators_allow_wildcard(node: &Node, src: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "combinator" {
            continue;
        }
        let mut inner = child.walk();
        let is_show = child
            .children(&mut inner)
            .next()
            .is_some_and(|first| node_text(first, src) == "show");
        if is_show {
            return false;
        }
    }
    true
}

fn extract_import_spec_recursive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let k = node.kind();
    if k == "import_specification"
        || k == "library_import"
        || k == "import_or_export"
        || k == "library_export"
    {
        // A plain `import '...';` — no `as` prefix, no `show` combinator —
        // brings every declaration into unqualified scope. `export`
        // directives never open the declaring file's own scope, so they are
        // never wildcard-worthy regardless of their combinators.
        let wildcard_worthy = k == "import_specification"
            && !has_alias_clause(node)
            && combinators_allow_wildcard(node, src);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let ck = child.kind();
            if ck == "string_literal" || ck == "uri" || ck == "configurable_uri" {
                let raw = if ck == "configurable_uri" {
                    first_child_text_of_kind(&child, src, "string_literal")
                        .unwrap_or_else(|| node_text(child, src))
                } else {
                    node_text(child, src)
                };
                let module = raw.trim_matches('"').trim_matches('\'').to_string();
                let stem = module
                    .rsplit('/')
                    .next()
                    .unwrap_or(&module)
                    .trim_end_matches(".dart")
                    .to_string();
                // A wildcard entry's `module` carries the bare library STEM,
                // not the raw URI: `WildcardMatch::FileStem` compares it
                // against a candidate's declaring-file basename, which is
                // never `package:`/`dart:`-prefixed. A non-wildcard entry
                // keeps the raw URI for the module-anchor and reexport
                // rungs, which resolve it as a real specifier.
                let (target, ref_module) = if wildcard_worthy {
                    ("*".to_string(), stem)
                } else {
                    (stem, module)
                };
                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: Some(ref_module),
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            } else if ck == "import_specification"
                || ck == "library_import"
                || ck == "library_export"
            {
                extract_import_spec_recursive(&child, src, current_symbol_count, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Part directives
// ---------------------------------------------------------------------------

pub(super) fn extract_part_directive(
    node: &Node,
    src: &str,
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_literal" || child.kind() == "uri" {
            let raw = node_text(child, src);
            let module = raw.trim_matches('"').trim_matches('\'').to_string();
            let target = module
                .rsplit('/')
                .next()
                .unwrap_or(&module)
                .trim_end_matches(".dart")
                .to_string();
            refs.push(ExtractedRef {
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: child.start_position().row as u32,
                col: 0,
                module: Some(module),
                chain: None,
                byte_offset: child.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Library-prefix binding
// ---------------------------------------------------------------------------

/// Scan a Dart source for `import '<uri>' as <name>` directives, returning
/// a `<name> → <uri>` map of library prefixes to their source module.
///
/// Dart's `as <prefix>` only appears in `import` directives — it's the
/// library prefix for qualified references (`import 'foo.dart' as p;
/// p.SomeType`). Exports never take a prefix; `show`/`hide` lists never
/// rename. The regex is therefore safely scoped to imports.
///
/// Robust to multi-line directives:
///
/// ```dart
/// import 'package:foo/foo.dart'
///     as i1
///     show Bar;
/// ```
pub(super) fn collect_dart_import_aliases(
    source: &str,
) -> std::collections::HashMap<String, String> {
    use regex::Regex;
    use std::sync::OnceLock;
    static IMPORT_AS_RE: OnceLock<Regex> = OnceLock::new();
    let re = IMPORT_AS_RE.get_or_init(|| {
        // import '<uri>' ... as <ident> ... ; — capture the URI string and the
        // prefix. `\s` covers newlines for multi-line directives; `(?s)` lets
        // `.` cross newlines. The URI alternation handles both quote styles.
        Regex::new(
            r#"(?s)\bimport\b\s*(?:'([^']*)'|"([^"]*)")[^;]*?\bas\s+([A-Za-z_][A-Za-z0-9_]*)[^;]*?;"#,
        )
        .expect("static regex compiles")
    });
    let mut out = std::collections::HashMap::new();
    for cap in re.captures_iter(source) {
        let Some(prefix) = cap.get(3) else { continue };
        let uri = cap
            .get(1)
            .or_else(|| cap.get(2))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default();
        out.insert(prefix.as_str().to_string(), uri);
    }
    out
}

/// Carry the library prefix onto each qualified reference's
/// `namespace_segments[0]` (and its source module onto `module`), then drop
/// the bare prefix refs.
///
/// The grammar emits `i0.Value` as two adjacent sibling `type_identifier`
/// nodes, so the prefix ref and the qualified-name ref appear as separate
/// `ExtractedRef`s sharing a `source_symbol_index`, with the prefix's
/// `byte_offset` immediately before the name's. For each prefix ref the
/// qualified partner is the ref with the smallest `byte_offset` strictly
/// greater than the prefix's, same source symbol — the `name` after the `.`.
/// The nuclear type scan can emit duplicate refs at one byte offset, so every
/// ref at the partner offset is stamped. `Imports` refs are never prefixes.
pub(super) fn bind_prefixed_refs(
    refs: &mut Vec<ExtractedRef>,
    alias_uris: &std::collections::HashMap<String, String>,
) {
    // Phase 1 (immutable): collect each prefix occurrence and resolve its
    // qualified partner's byte offset — the nearest following ref in the same
    // source symbol that is not itself a prefix.
    struct Binding {
        partner_byte: u32,
        sym_idx: usize,
        prefix: String,
        uri: String,
    }
    let mut bindings: Vec<Binding> = Vec::new();
    for p in refs.iter() {
        if p.kind == EdgeKind::Imports {
            continue;
        }
        let Some(uri) = alias_uris.get(&p.target_name) else {
            continue;
        };
        let partner_byte = refs
            .iter()
            .filter(|r| {
                r.kind != EdgeKind::Imports
                    && r.source_symbol_index == p.source_symbol_index
                    && r.byte_offset > p.byte_offset
                    && !alias_uris.contains_key(&r.target_name)
            })
            .map(|r| r.byte_offset)
            .min();
        if let Some(partner_byte) = partner_byte {
            bindings.push(Binding {
                partner_byte,
                sym_idx: p.source_symbol_index,
                prefix: p.target_name.clone(),
                uri: uri.clone(),
            });
        }
    }

    // Phase 2 (mutable): stamp the prefix + module onto every ref at a partner
    // offset. The nuclear type scan can emit duplicate refs at one offset, so
    // all are stamped; an already-stamped ref (multi-prefix edge cases) is
    // left as-is.
    for b in &bindings {
        for r in refs.iter_mut() {
            if r.kind != EdgeKind::Imports
                && r.source_symbol_index == b.sym_idx
                && r.byte_offset == b.partner_byte
                && r.namespace_segments.is_empty()
            {
                r.namespace_segments = vec![b.prefix.clone()];
                r.module = Some(b.uri.clone());
            }
        }
    }

    // Phase 3: drop the bare prefix refs (every non-Imports ref whose name is
    // a known library prefix).
    refs.retain(|r| r.kind == EdgeKind::Imports || !alias_uris.contains_key(&r.target_name));
}

#[cfg(test)]
#[path = "imports_tests.rs"]
mod tests;
