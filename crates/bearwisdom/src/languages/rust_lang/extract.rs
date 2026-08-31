// =============================================================================
// parser/extractors/rust/mod.rs  —  Rust symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Struct, Enum, EnumMember, Interface (trait), Method, Function,
//   TypeAlias, Variable (static), Namespace (mod), Test
//
// REFERENCES:
//   - `use` declarations     → Import edges (recursive use-tree walking)
//   - `call_expression`      → Calls edges
//
// Approach:
//   Single-pass recursive CST walk. No scope tree — qualified names are built
//   by threading a `qualified_prefix` string through the recursion. `impl`
//   blocks are not symbols themselves; they set the prefix for their methods.
// =============================================================================

use super::{calls, decorators, helpers, patterns, symbols};
use crate::types::ExtractionResult;
use crate::types::{AliasTarget, ExtractedRef, ExtractedSymbol};
use tree_sitter::{Node, Parser};

// Re-exports required by rust_tests.rs (`use super::*`).
pub(crate) use crate::types::{EdgeKind, SymbolKind, Visibility};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from Rust source code.
pub fn extract(source: &str) -> ExtractionResult {
    let language = tree_sitter_rust::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Rust grammar");

    // tree-sitter-rust 0.24 doesn't recognise the `const trait` syntax
    // introduced by the unstable `const_trait_impl` feature. The grammar
    // emits ERROR nodes around the entire trait_item, dropping it from
    // extraction. `core/src/cmp.rs` (PartialEq/Eq/Ord/PartialOrd) and
    // `core/src/convert.rs` (AsRef/AsMut) are the load-bearing cases.
    //
    // Rewrite `const trait` → `      trait` (same byte length so positions
    // stay valid) before parsing. Whitespace doesn't affect semantics and
    // keeps the parsed tree's byte ranges aligned with the source slice
    // the extractor reads identifiers from.
    //
    // tree-sitter-rust 0.24 also has no grammar node for the macro-2.0
    // (`decl_macro`) declarative-macro syntax — `[pub] macro NAME { ... }` /
    // `[pub] macro NAME(...) { ... }`, distinct from `macro_rules!`. Hitting
    // one derails the parser into a single ERROR node that swallows every
    // top-level item for the rest of the file: `core::macros::builtin` in
    // the shipped rust-src opens with `pub macro assert_matches { ... }`,
    // which blanks out `assert`, `debug_assert`, `matches`, `write`, and
    // everything after it. `rewrite_macro_2_0_defs` blanks each such
    // definition (visibility keyword through the matching closing `}`) to
    // whitespace, same byte-length-preserving technique as the const-trait
    // rewrite above, so the rest of the file parses cleanly.
    let owned;
    let needs_const_trait = memchr_const_trait(source);
    let needs_macro_2_0 = contains_macro_2_0_keyword(source);
    let parse_input: &str = if needs_const_trait || needs_macro_2_0 {
        let mut rewritten = if needs_const_trait {
            rewrite_const_trait(source)
        } else {
            source.to_string()
        };
        if needs_macro_2_0 {
            rewritten = rewrite_macro_2_0_defs(&rewritten);
        }
        owned = rewritten;
        owned.as_str()
    } else {
        source
    };

    let tree = match parser.parse(parse_input, None) {
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
                declared_modules: Vec::new(),
            }
        }
    };

    let mut syms = Vec::new();
    let mut refs = Vec::new();
    let mut alias_targets = Vec::new();

    let root = tree.root_node();

    extract_from_node(root, source, &mut syms, &mut refs, None, "", &mut alias_targets);

    // Second pass: scan the full CST for type_identifier and scoped_type_identifier
    // nodes, emitting TypeRef for each non-primitive type found anywhere in the file.
    if !syms.is_empty() {
        scan_all_type_identifiers(root, source, 0, &mut refs);
    }

    // Third pass: enrich Calls refs that have a qualified chain (≥2 segments)
    // but no module set.  Build an import map from the Imports refs already
    // emitted — `target_name → module` — then for each qualifying Calls ref
    // whose first chain segment matches an imported name, copy that module onto
    // the ref.  This lets the resolver trace `DbPool::new()` back to
    // `crate::db` because `DbPool` was imported via `use crate::db::DbPool`.
    {
        let import_map: rustc_hash::FxHashMap<String, String> = refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .filter_map(|r| {
                r.module
                    .as_ref()
                    .map(|m| (r.target_name.clone(), m.clone()))
            })
            .collect();

        for r in refs.iter_mut() {
            if r.kind == EdgeKind::Calls && r.module.is_none() {
                if let Some(chain) = &r.chain {
                    if chain.segments.len() >= 2 {
                        let first = &chain.segments[0].name;
                        if let Some(module) = import_map.get(first) {
                            // Copy the importing module path verbatim — the
                            // `::`-separated, crate-rooted form the `use` carried
                            // (`crate::db`, `lemmy_db_schema::source::person`).
                            // The engine's `ByNameUnderModuleDir` anchor maps the
                            // separators to a path fragment and falls back to the
                            // module leaf to locate the defining file.
                            r.module = Some(module.clone());
                        } else if let Some(prefix) = scoped_chain_prefix(chain) {
                            // A qualified call whose root matches no import is
                            // its own module evidence: crate and module paths
                            // are in scope without a `use`, so the verbatim
                            // `::` qualifier is the only module the resolver
                            // will ever see for this ref.
                            r.module = Some(prefix);
                        }
                    }
                }
            }
        }
    }

    // Fourth pass: drop refs whose `target_name` contains characters that
    // can never be part of a Rust identifier or path. The chain extractor
    // and call-expression fallback occasionally capture stray punctuation
    // from malformed CST nodes — `(config.normalize_type)(arg)` lands as
    // `normalize_type)`, `.collect::<Result<Vec<_>>>()` lands as
    // `Result<Vec<_>>>`, multi-line snippets land with embedded newlines.
    // None of these can ever resolve to a real symbol; keeping them just
    // pollutes the unresolved-refs table.
    refs.retain(|r| is_valid_rust_target_name(&r.target_name) && !is_generic_param_noise(r));

    let has_errors = tree.root_node().has_error();
    let mut result = ExtractionResult::new(syms, refs, has_errors);
    result.alias_targets = alias_targets;
    result
}

/// The `::`-joined qualifier of a pure scoped-path call chain (`a::b::leaf` →
/// `a::b`), or `None` when any qualifying segment is not a plain path
/// identifier — a field/method hop, a `self`/`Self` head (resolved through the
/// enclosing type, not a module), a nested call, or bracketed type-argument
/// noise the chain builder couldn't flatten.
fn scoped_chain_prefix(chain: &crate::types::MemberChain) -> Option<String> {
    use crate::types::SegmentKind;
    let quals = &chain.segments[..chain.segments.len() - 1];
    if quals.is_empty() || chain.segments[0].name == "self" {
        return None;
    }
    let mut parts: Vec<&str> = Vec::with_capacity(quals.len());
    for seg in quals {
        if seg.node_kind != "scoped_identifier"
            || !matches!(seg.kind, SegmentKind::Identifier | SegmentKind::Property)
            || seg.is_call
            || seg.name.is_empty()
            || !seg.name.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return None;
        }
        parts.push(&seg.name);
    }
    Some(parts.join("::"))
}

/// True when a ref's target is a declared generic parameter rather than a real
/// symbol — type-argument noise the type-identifier scan and call fallback
/// occasionally capture. Two shapes, both never indexable:
///   - a single uppercase letter (`L`, `M`, `F`, `W`) in TypeRef position —
///     the convention for an unconstrained generic parameter; and
///   - `<Uppercase><digit>` (`P1`, `T2`) in any position — numbered generics.
/// Turbofish `<…>` targets are already dropped by `is_valid_rust_target_name`
/// (the angle brackets fail its identifier check), so they need no arm here.
fn is_generic_param_noise(r: &ExtractedRef) -> bool {
    let target = &r.target_name;
    if r.kind == EdgeKind::TypeRef {
        let bare = target.trim_start_matches("::");
        if bare.len() == 1
            && bare
                .chars()
                .next()
                .map_or(false, |c| c.is_ascii_uppercase())
        {
            return true;
        }
    }
    if target.len() == 2 {
        let mut chars = target.chars();
        let (a, b) = (chars.next().unwrap(), chars.next().unwrap());
        if a.is_ascii_uppercase() && b.is_ascii_digit() {
            return true;
        }
    }
    false
}

/// Whether `name` could be a Rust identifier or path. Allows alphanumerics,
/// underscores, `::` separators, leading `!` (none — Rust forbids), trailing
/// `!` (macro invocation in some emit paths), `*` (wildcards), `?` is NOT
/// allowed — neither are angle brackets, parens, braces, brackets, quotes,
/// commas, whitespace, semicolons, or backslashes. Returns true for the
/// empty string so callers that explicitly accept empty targets aren't
/// disturbed; the main extract path already filters empty.
/// Cheap pre-scan to decide whether `rewrite_const_trait` is worth running.
/// Returns true only when both `const` and `trait` appear in the source.
fn memchr_const_trait(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut has_const = false;
    let mut has_trait = false;
    while i + 5 <= bytes.len() {
        let head = &bytes[i..];
        if !has_const && head.starts_with(b"const") {
            has_const = true
        }
        if !has_trait && head.starts_with(b"trait") {
            has_trait = true
        }
        if has_const && has_trait {
            return true;
        }
        i += 1;
    }
    false
}

/// Replace `const trait` and `[const]` super-trait projection notation
/// with whitespace, preserving byte offsets so tree-sitter spans still
/// line up with the original source for identifier extraction.
fn rewrite_const_trait(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out_bytes: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"const")
            && is_word_boundary_before(bytes, i)
            && is_word_boundary_after(bytes, i + 5)
        {
            let mut j = i + 5;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1
            }
            if bytes[j..].starts_with(b"trait") && is_word_boundary_after(bytes, j + 5) {
                out_bytes.extend_from_slice(b"     ");
                out_bytes.extend_from_slice(&bytes[i + 5..j]);
                i = j;
                continue;
            }
        }
        if bytes[i..].starts_with(b"[const]") {
            out_bytes.extend_from_slice(b"       ");
            i += 7;
            continue;
        }
        out_bytes.push(bytes[i]);
        i += 1;
    }
    // SAFETY: every byte either came verbatim from the original UTF-8
    // string or is an ASCII space — the result is still valid UTF-8.
    unsafe { String::from_utf8_unchecked(out_bytes) }
}

fn is_word_boundary_before(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let c = bytes[i - 1];
    !(c.is_ascii_alphanumeric() || c == b'_')
}

fn is_word_boundary_after(bytes: &[u8], i: usize) -> bool {
    if i >= bytes.len() {
        return true;
    }
    let c = bytes[i];
    !(c.is_ascii_alphanumeric() || c == b'_')
}

/// Cheap pre-scan for `rewrite_macro_2_0_defs`: true when the source contains
/// the bare `macro` keyword followed by whitespace (the shape the rewrite
/// targets). `macro_rules!` never matches — `_rules!` follows `macro`
/// directly, with no intervening whitespace.
fn contains_macro_2_0_keyword(source: &str) -> bool {
    source.contains("macro ") || source.contains("macro\t") || source.contains("macro\n")
}

/// Replace each `[pub[(...)]] macro NAME { ... }` / `... macro NAME(...) {
/// ... }` declarative-macro-2.0 definition with whitespace, preserving `\n`
/// bytes so line numbers of the surviving source stay aligned. Leaves
/// `macro_rules!` definitions (which the grammar parses natively) untouched.
fn rewrite_macro_2_0_defs(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if !(bytes[i..].starts_with(b"macro")
            && is_word_boundary_before(bytes, i)
            && is_word_boundary_after(bytes, i + 5))
        {
            i += 1;
            continue;
        }
        let mut j = i + 5;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let name_start = j;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        if j == name_start {
            i += 5;
            continue;
        }
        let mut k = j;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        // Optional `(params)` single-rule form precedes the body brace.
        if k < bytes.len() && bytes[k] == b'(' {
            let Some(after_parens) = skip_balanced(bytes, k, b'(', b')') else {
                i += 5;
                continue;
            };
            k = after_parens;
            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                k += 1;
            }
        }
        if k >= bytes.len() || bytes[k] != b'{' {
            i += 5;
            continue;
        }
        let Some(body_end) = skip_balanced(bytes, k, b'{', b'}') else {
            i += 5;
            continue;
        };
        // Extend the blanked span backward over a `pub` / `pub(...)`
        // visibility modifier so no dangling `pub` is left behind.
        let mut start = i;
        let mut back = i;
        while back > 0 && bytes[back - 1].is_ascii_whitespace() {
            back -= 1;
        }
        if back >= 3 && &bytes[back - 3..back] == b"pub" && is_word_boundary_before(bytes, back - 3)
        {
            start = back - 3;
        }
        for b in &mut out[start..body_end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        i = body_end;
    }
    // SAFETY: every byte is either verbatim from the original UTF-8 string or
    // an ASCII space substituted in place — the result is still valid UTF-8.
    unsafe { String::from_utf8_unchecked(out) }
}

/// From `bytes[open_idx]` (which must equal `open`), scan forward tracking
/// nested `open`/`close` delimiter depth, skipping over string, char, and
/// comment contents so a delimiter inside one doesn't miscount. Returns the
/// index just past the matching `close`, or `None` if the source ends before
/// the delimiter is balanced.
fn skip_balanced(bytes: &[u8], open_idx: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b if b == open => {
                depth += 1;
                i += 1;
            }
            b if b == close => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' && i + 1 < bytes.len() { 2 } else { 1 };
                }
                i += 1;
            }
            b'\'' => {
                // Char literal vs. a lifetime (`'a`) — a lifetime never
                // closes with `'`, so only consume as a literal when a
                // closing `'` appears within a plausible char-literal span.
                let start = i;
                let mut lookahead = i + 1;
                while lookahead < bytes.len()
                    && lookahead < start + 8
                    && bytes[lookahead] != b'\''
                    && bytes[lookahead] != b'\n'
                {
                    lookahead += 1;
                }
                i = if lookahead < bytes.len() && bytes[lookahead] == b'\'' {
                    lookahead + 1
                } else {
                    start + 1
                };
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            _ => i += 1,
        }
    }
    None
}

fn is_valid_rust_target_name(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    for c in name.chars() {
        match c {
            '_' | ':' | '*' | '!' => continue,
            ch if ch.is_ascii_alphanumeric() => continue,
            // Emoji / non-ascii letters in identifiers are allowed in Rust;
            // gate on the broader unicode XID rule to keep CJK identifier
            // names out of the reject path.
            ch if ch.is_alphanumeric() => continue,
            _ => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    alias_targets: &mut Vec<(String, AliasTarget)>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" | "function_signature_item" => {
                if let Some(sym) =
                    symbols::extract_function(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let fn_qname = sym.qualified_name.clone();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRefs for parameter types and return type.
                    symbols::extract_fn_signature_type_refs(&child, source, idx, refs);
                    // Emit Variable symbols for callable-typed parameters
                    // (`impl Fn(...)`, `&dyn Fn(...)`, `fn(...)` etc.) so
                    // calls in the body that look like `cb(arg)` can resolve
                    // to the parameter via scope-chain lookup. Skip the
                    // common case (non-callable parameters) — adding every
                    // parameter as a Variable would explode the symbol
                    // table without helping resolution.
                    symbols::extract_callable_fn_params(&child, source, idx, &fn_qname, symbols);
                    // where-clause and type-parameter bounds → TypeRef edges.
                    // Iterate children by kind rather than field_name to avoid
                    // grammar-version sensitivity.
                    {
                        let mut wc = child.walk();
                        for gc in child.children(&mut wc) {
                            match gc.kind() {
                                "type_parameters" => {
                                    patterns::extract_type_param_bounds(&gc, source, idx, refs);
                                }
                                "where_clause" => {
                                    patterns::extract_where_clause(&gc, source, idx, refs);
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(body) = child.child_by_field_name("body") {
                        calls::extract_calls_from_body_with_symbols(
                            &body,
                            source,
                            idx,
                            refs,
                            Some(symbols),
                        );
                    }
                }
            }

            "struct_item" | "union_item" => {
                // `union_item` has the same field layout as `struct_item` in
                // tree-sitter-rust; reuse the struct extractor and emit Struct kind.
                if let Some(sym) =
                    symbols::extract_struct(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let struct_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Extract field symbols and TypeRefs for field types.
                    symbols::extract_struct_fields(
                        &child,
                        source,
                        idx,
                        &struct_prefix,
                        symbols,
                        refs,
                    );
                }
            }

            "enum_item" => {
                if let Some(sym) =
                    symbols::extract_enum(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        symbols::extract_enum_variants(
                            &body,
                            source,
                            Some(idx),
                            &new_prefix,
                            symbols,
                            refs,
                        );
                    }
                }
            }

            "trait_item" => {
                if let Some(sym) =
                    symbols::extract_trait(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Supertrait bounds: `trait Foo: Bar + Baz` -> Inherits edges.
                    patterns::extract_supertrait_bounds(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        // Extract associated types declared in the trait body.
                        symbols::extract_trait_associated_types(
                            &body,
                            source,
                            idx,
                            &new_prefix,
                            symbols,
                            refs,
                        );
                        extract_from_node(
                            body,
                            source,
                            symbols,
                            refs,
                            Some(idx),
                            &new_prefix,
                            alias_targets,
                        );
                    }
                }
            }

            "impl_item" => {
                calls::extract_impl(&child, source, symbols, refs, qualified_prefix);
            }

            "type_item" => {
                if let Some(sym) =
                    symbols::extract_type_alias(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRef for the RHS type (covers `type_identifier` nodes in
                    // the type alias body — e.g. `type Foo = SomeType<Bar>`).
                    if let Some(type_node) = child.child_by_field_name("type") {
                        symbols::extract_type_refs_from_type_node(&type_node, source, idx, refs);
                    }
                }
            }

            "const_item" => {
                if let Some(sym) =
                    symbols::extract_const(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRef for the type annotation.
                    if let Some(type_node) = child.child_by_field_name("type") {
                        symbols::extract_type_refs_from_type_node(&type_node, source, idx, refs);
                    }
                }
            }

            "static_item" => {
                if let Some(sym) =
                    symbols::extract_static(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    // Emit TypeRef for the type annotation.
                    if let Some(type_node) = child.child_by_field_name("type") {
                        symbols::extract_type_refs_from_type_node(&type_node, source, idx, refs);
                    }
                }
            }

            "mod_item" => {
                if let Some(sym) =
                    symbols::extract_mod(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    let new_prefix = helpers::qualify(&sym.name, qualified_prefix);
                    symbols.push(sym);
                    decorators::extract_decorators(&child, source, idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_from_node(
                            body,
                            source,
                            symbols,
                            refs,
                            Some(idx),
                            &new_prefix,
                            alias_targets,
                        );
                    }
                }
            }

            "use_declaration" => {
                let sym_count = symbols.len();
                calls::extract_use_names(
                    &child,
                    source,
                    refs,
                    symbols,
                    sym_count,
                    qualified_prefix,
                    alias_targets,
                );
            }

            // `extern "C" { fn malloc(size: usize) -> *mut u8; }`
            // Walk the declaration_list body and emit Function symbols for each
            // `foreign_item` function declaration.
            "foreign_mod_item" => {
                if let Some(body) = child.child_by_field_name("body") {
                    let mut bc = body.walk();
                    for decl in body.children(&mut bc) {
                        if decl.kind() == "function_item"
                            || decl.kind() == "function_signature_item"
                        {
                            if let Some(sym) = symbols::extract_function(
                                &decl,
                                source,
                                parent_index,
                                qualified_prefix,
                            ) {
                                let idx = symbols.len();
                                symbols.push(sym);
                                decorators::extract_decorators(&decl, source, idx, refs);
                            }
                        }
                    }
                }
            }

            // `extern crate foo;` — emit an Imports edge for the crate name.
            "extern_crate_declaration" => {
                calls::extract_extern_crate(&child, source, refs, symbols.len());
            }

            // `macro_rules! foo { ... }` — tree-sitter-rust 0.24 emits `macro_definition`.
            // Emit a Function symbol for the macro name.
            "macro_definition" => {
                if let Some(sym) =
                    symbols::extract_macro_rules(&child, source, parent_index, qualified_prefix)
                {
                    let idx = symbols.len();
                    symbols.push(sym);
                }
            }

            // Module-level macro invocations: `lazy_static! { ... }`, `global_allocator!(...)`.
            // Emit a Calls edge for the macro name (same as body-level macros).
            "macro_invocation" => {
                let source_idx = parent_index.unwrap_or(0);
                // Emit Calls ref for the macro name itself.
                if let Some(macro_node) = child.child_by_field_name("macro") {
                    let raw = helpers::node_text(&macro_node, source);
                    let raw = raw.trim_end_matches('!');
                    // Split `prefix::name!` into module + name so the
                    // resolver can route to the owning crate. Mirrors the
                    // body walker's macro_invocation arm in calls.rs.
                    let (module, target) = match raw.rsplit_once("::") {
                        Some((prefix, leaf)) if !prefix.is_empty() && !leaf.is_empty() => {
                            (Some(prefix.to_string()), leaf.to_string())
                        }
                        _ => (None, raw.to_string()),
                    };
                    if !target.is_empty() {
                        refs.push(crate::types::ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name: target,
                            kind: crate::types::EdgeKind::Calls,
                            line: macro_node.start_position().row as u32,
                            col: 0,
                            module,
                            chain: None,
                            byte_offset: macro_node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
                // Recurse into token-tree arguments for nested calls.
                calls::extract_calls_from_body_with_symbols(
                    &child,
                    source,
                    source_idx,
                    refs,
                    Some(symbols),
                );
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    alias_targets,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-tree type_identifier scan
// ---------------------------------------------------------------------------

/// Recursively scan the entire CST and emit a TypeRef for every `type_identifier`
/// or `scoped_type_identifier` node that is in a type-annotation position.
///
/// Nodes are skipped when they appear inside:
///   - The `value` field of a `let_declaration` — the RHS of a let binding is
///     an expression, not a type annotation.  `let x = Foo::new()` should not
///     emit TypeRef for `Foo` from this pass (calls.rs handles that separately).
///   - `attribute_item` subtrees — attributes are macro invocations whose names
///     and arguments are not symbol references (`#[derive(Debug)]`, `#[serde(...)]`).
fn scan_all_type_identifiers(
    node: tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip the value (RHS) of let declarations — type_identifier nodes there
        // are constructor/function names in expressions, not type annotations.
        if child.kind() == "let_declaration" {
            if let Some(value_node) = child.child_by_field_name("value") {
                // Only scan the type annotation subtree, not the value subtree.
                // The `type` field holds the explicit `: T` annotation.
                if let Some(type_node) = child.child_by_field_name("type") {
                    scan_all_type_identifiers(type_node, source, sym_idx, refs);
                }
                // Recurse into everything that is NOT the value subtree.
                let value_id = value_node.id();
                let mut lc = child.walk();
                for lc_child in child.children(&mut lc) {
                    if lc_child.id() == value_id {
                        continue;
                    }
                    // Also skip the type node — already handled above.
                    if child.child_by_field_name("type").map(|n| n.id()) == Some(lc_child.id()) {
                        continue;
                    }
                    scan_all_type_identifiers(lc_child, source, sym_idx, refs);
                }
                continue;
            }
            // No value field — scan all children normally.
        }

        // Skip attribute_item nodes entirely — their contents are macro invocations,
        // not type references.  `extract_decorators` handles them in the main pass
        // for structured attributes on top-level items.
        if child.kind() == "attribute_item" {
            continue;
        }

        match child.kind() {
            "type_identifier" if child.is_named() => {
                let name = helpers::node_text(&child, source);
                if !name.is_empty() && !symbols::is_rust_primitive(&name) {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
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
            "scoped_type_identifier" if child.is_named() => {
                // `foo::Bar` — extract the leaf name (last segment).
                let name = child
                    .child_by_field_name("name")
                    .map(|n| helpers::node_text(&n, source))
                    .unwrap_or_else(|| {
                        let text = helpers::node_text(&child, source);
                        text.rsplit("::").next().unwrap_or(&text).to_string()
                    });
                if !name.is_empty() && !symbols::is_rust_primitive(&name) {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
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
                // Don't recurse into scoped_type_identifier children — we already extracted the leaf.
                continue;
            }
            _ => {}
        }
        scan_all_type_identifiers(child, source, sym_idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
