// =============================================================================
// languages/pascal/extract.rs  —  Pascal / Delphi extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — declProc / defProc (procedure_declaration / function_declaration)
//   Class     — declType wrapping declClass
//   Interface — declType wrapping declIntf
//   Enum      — declType wrapping declEnum
//   Struct    — declSection with record keyword (record_type)
//   Field     — declField inside declSection
//   Property  — declProp inside declSection
//   Variable  — declVar (module-level var section) / declConst
//   Namespace — unit (unit declaration)
//
// REFERENCES:
//   Imports   — declUses (uses clause)
//   Calls     — exprCall (function/method calls)
//   Inherits  — declClass parent typeref
//   TypeRef   — typeref nodes (type references in signatures)
//
// Grammar: tree-sitter-pascal 0.10.2 (tree-sitter-language ABI, LANGUAGE constant).
// Pascal uses '.' as namespace separator in unit names.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_pascal::LANGUAGE.into())
        .expect("Failed to load Pascal grammar");

    // Normalise source before parsing to prevent cascading parse errors.
    //
    // Known patterns that produce token sequences the tree-sitter-pascal grammar
    // cannot handle, causing it to wipe out all preceding declarations:
    //
    //   1. `{$ifdef FPC}object{$else}record{$endif}` — after pp stripping both
    //      `object` and `record` appear in the token stream. Collapsed to `record`.
    //
    //   2. `bitpacked record` — FPC extension not in grammar; tree-sitter sees
    //      an unknown identifier followed by `record`. Collapsed to `record`.
    //
    //   3. Blank line between `end;` and `);` in variant-record case arms —
    //      prevents tree-sitter from closing the anonymous nested record boundary.
    let normalised;
    let src = if source.contains("{$ifdef") || source.contains("{$if ")
        || source.contains("{$IF") || source.contains("bitpacked")
        || source.contains("end;") || source.contains('<')
    {
        normalised = normalise_source(source);
        normalised.as_str()
    } else {
        source
    };

    let tree = match parser.parse(src, None) {
        Some(t) => t,
        None => return ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    visit_root(tree.root_node(), src, &mut symbols, &mut refs);

    ExtractionResult::new(symbols, refs, has_errors)
}

/// Normalise Pascal source before handing to tree-sitter to prevent cascading
/// parse errors caused by constructs the grammar cannot represent:
///
///   - `{$ifdef COND}object{$else}record{$endif}`: pp stripping leaves both
///     `object` and `record` in the stream; collapsed to the else-branch keyword.
///   - `bitpacked record`: FPC extension; grammar only knows `packed`. The
///     `bitpacked` prefix is replaced by spaces so `record` stands alone.
///   - Blank lines between `end;` and `);` inside variant record case arms
///     prevent the grammar from closing the anonymous nested record correctly.
///     The blank line is collapsed so `end;` and `);` are adjacent.
#[cfg(test)]
pub(crate) fn normalise_source_for_test(src: &str) -> String {
    normalise_source(src)
}

fn normalise_source(src: &str) -> String {
    // Pass 1: collapse `bitpacked` → spaces.
    let after_bitpacked = if src.contains("bitpacked") {
        let mut s = src.to_string();
        // Replace case-insensitively.  Only the lowercase form appears in CGE
        // binding files, but guard against BITPACKED/Bitpacked as well.
        while let Some(pos) = s.to_ascii_lowercase().find("bitpacked") {
            // Replace with equal-length spaces to preserve byte offsets.
            s.replace_range(pos..pos + "bitpacked".len(), "         ");
        }
        s
    } else {
        src.to_string()
    };

    // Pass 2: collapse {$ifdef COND}TYPE_KW{$else}TYPE_KW{$endif} patterns.
    let after_ifdef = normalise_ifdef_type_keywords(&after_bitpacked);

    // Pass 3: collapse blank lines between `end;` and `);` in variant record case arms.
    // A blank line between a nested record's `end;` and the closing `);` of the case
    // alternative prevents tree-sitter from recognising the variant record boundary.
    // Removing the blank line makes the grammar parse the structure correctly.
    let after_variant = normalise_variant_record_end_paren(&after_ifdef);

    // Pass 4: strip generic type parameters from `specialize X<A,B,C>` forms.
    // FPC's generic specialization syntax `class(specialize X<T1,T2>)` is not
    // valid Delphi or standard Pascal; the grammar produces comparison-operator
    // expressions from `X<T1` which cascade into parse errors that wipe out
    // subsequent declarations.  Reduce `TypeName<…>` to `TypeName` (possibly
    // multiline) so the grammar sees a plain parenthesized type name.
    normalise_specialize_generics(&after_variant)
}

/// Normalise variant-record case arms that contain an anonymous nested record.
///
/// The Pascal grammar cannot parse `N : ( fieldname : record ... end; );` when
/// the closing `);` is on a separate line from `end;`, even with no blank lines
/// between them.  Replace the entire body of such case arms with a simple
/// `(fieldname : Pointer)` so the variant record structure is preserved for the
/// type declaration parser while the problematic nested body is elided.
///
/// Matched pattern (case-insensitive for keywords):
///   `<N> : (\n  <name> : record\n  ...\n  end;\n[blanks]\n);`
///
/// The replacement is:
///   `<N> : (<name>: Pointer);`
fn normalise_variant_record_end_paren(src: &str) -> String {
    // Quick exit: no anonymous records inside case arms.
    if !src.contains(": record") && !src.contains(":record") && !src.contains(": RECORD") {
        return src.to_string();
    }

    let lines: Vec<&str> = src.lines().collect();
    let n = lines.len();
    let mut out: Vec<String> = Vec::with_capacity(n);
    let mut i = 0;

    while i < n {
        let line = lines[i];
        // Detect a case arm opening: `<N> : (` at end of line (after trimming).
        // The line must contain `: (` and nothing after the `(` (only whitespace).
        let trimmed = line.trim();
        if trimmed.ends_with('(') && trimmed.contains(": (") {
            // Look ahead for `<name> : record` on the next non-blank line.
            let mut j = i + 1;
            while j < n && lines[j].trim().is_empty() { j += 1; }

            if j < n {
                let inner = lines[j].trim().to_ascii_lowercase();
                // Pattern: `fieldname : record` (with optional spaces around colon)
                let is_anon_record = inner.contains(": record") || inner.contains(":record");
                // Extract field name (everything before the `: record` part).
                let field_name = if let Some(pos) = inner.find(": record") {
                    inner[..pos].trim().to_string()
                } else if let Some(pos) = inner.find(":record") {
                    inner[..pos].trim().to_string()
                } else {
                    String::new()
                };

                if is_anon_record && !field_name.is_empty() {
                    // Find the closing `end;` for this anonymous record,
                    // then the `)` or `);` that closes the case arm.
                    let mut k = j + 1;
                    let mut depth = 1usize; // depth of nested record/begin/case
                    while k < n {
                        let tl = lines[k].trim().to_ascii_lowercase();
                        // Crude depth tracking: `record`, `begin`, `case` open; `end` closes.
                        if tl.starts_with("record") || tl.starts_with("begin") { depth += 1; }
                        if tl == "end;" || tl == "end" { depth -= 1; }
                        if depth == 0 { break; }
                        k += 1;
                    }
                    // k now points to the `end;` line of the anonymous record.
                    // Skip past blank lines and the closing `)` or `);`.
                    let mut m = k + 1;
                    while m < n && lines[m].trim().is_empty() { m += 1; }
                    if m < n && (lines[m].trim() == ");" || lines[m].trim() == ")") {
                        // Replacement: keep the case arm prefix up to `(`, replace body.
                        let prefix_end = line.rfind('(').unwrap_or(line.len());
                        let prefix = &line[..prefix_end];
                        // Preserve the field name from the original (not lowercased).
                        let orig_inner = lines[j].trim();
                        let orig_field = if let Some(pos) = orig_inner.find(": record").or_else(|| orig_inner.find(": RECORD")).or_else(|| orig_inner.find(":record")) {
                            orig_inner[..pos].trim()
                        } else {
                            &field_name
                        };
                        out.push(format!("{prefix}({orig_field}: Pointer);"));
                        i = m + 1;
                        continue;
                    }
                }
            }
        }

        out.push(line.to_string());
        i += 1;
    }

    let mut result = out.join("\n");
    if src.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Collapse `{$ifdef COND}object{$else}record{$endif}` (and permutations) to a
/// single keyword so tree-sitter never sees two competing type keywords in the
/// same token position.
///
/// The pattern: `{$if[def] COND}KW1{$else}KW2{$endif}` where KW1/KW2 are any
/// pair drawn from {object, record, class}. We always keep the else-branch
/// keyword and replace the whole span (including the pp tokens) with `KW2`
/// padded with spaces to preserve byte offsets as closely as possible.
///
/// We also handle `{$if[def] COND}KW1{$ifend}` (no else) by collapsing to KW1.
fn normalise_ifdef_type_keywords(src: &str) -> String {
    // Type keywords that can appear as the sole token in a conditional branch.
    const TYPE_KWS: &[&str] = &["object", "record", "class"];

    let fallback = src.to_string();
    // Single forward scan replacing in-place on a byte vec.  Replacements
    // pad with spaces to preserve byte offsets.
    let mut out = src.as_bytes().to_vec();
    let src_len = out.len();

    // Locate `{$ifdef ...}` or `{$if ...}` openings and try to match the
    // pattern `{$if[def] COND}KW1{$else}KW2{$end[if|ifend]}`.
    let mut i = 0;
    while i < src_len {
        // Quick scan: look for `{$`
        if out[i] != b'{' || i + 1 >= src_len || out[i + 1] != b'$' {
            i += 1;
            continue;
        }

        // Find the closing `}` of the opening pp token.
        let pp_start = i;
        let pp_end = match out[i..].iter().position(|&b| b == b'}') {
            Some(p) => i + p + 1,
            None => { i += 1; continue; }
        };

        // Extract the pp keyword (e.g. "ifdef", "if ", "ifndef", "ifopt")
        let pp_inner = std::str::from_utf8(&out[pp_start + 2..pp_end - 1])
            .unwrap_or("")
            .trim_start()
            .to_ascii_lowercase();
        let is_open = pp_inner.starts_with("ifdef")
            || pp_inner.starts_with("ifndef")
            || pp_inner.starts_with("if ");

        if !is_open {
            i = pp_end;
            continue;
        }

        // After the opening `{$ifdef COND}`, consume whitespace/newlines, then check
        // for a type keyword immediately followed by either `{$else}` or `{$ifend}`.
        let mut j = pp_end;
        while j < src_len && (out[j] == b' ' || out[j] == b'\t' || out[j] == b'\r' || out[j] == b'\n') {
            j += 1;
        }

        // Check if a type keyword starts here.
        let kw1 = TYPE_KWS.iter().find(|&&kw| {
            out[j..].starts_with(kw.as_bytes())
            && (j + kw.len() >= src_len
                || !out[j + kw.len()].is_ascii_alphanumeric())
        });

        let kw1 = match kw1 {
            Some(k) => k,
            None => { i = pp_end; continue; }
        };

        let kw1_end = j + kw1.len();

        // After kw1, consume whitespace, then expect `{$else}` or `{$ifend}`/`{$endif}`.
        let mut k = kw1_end;
        while k < src_len && (out[k] == b' ' || out[k] == b'\t' || out[k] == b'\r' || out[k] == b'\n') {
            k += 1;
        }

        if k >= src_len || out[k] != b'{' { i = pp_end; continue; }
        let pp2_start = k;
        let pp2_end = match out[k..].iter().position(|&b| b == b'}') {
            Some(p) => k + p + 1,
            None => { i = pp_end; continue; }
        };
        let pp2_inner = std::str::from_utf8(&out[pp2_start + 2..pp2_end - 1])
            .unwrap_or("")
            .trim_start()
            .to_ascii_lowercase();

        // Case 1: `{$else}` — has an else branch
        if pp2_inner.starts_with("else") {
            let mut m = pp2_end;
            while m < src_len && (out[m] == b' ' || out[m] == b'\t' || out[m] == b'\r' || out[m] == b'\n') {
                m += 1;
            }
            let kw2 = TYPE_KWS.iter().find(|&&kw| {
                out[m..].starts_with(kw.as_bytes())
                && (m + kw.len() >= src_len
                    || !out[m + kw.len()].is_ascii_alphanumeric())
            });
            let kw2 = match kw2 {
                Some(k) => k,
                None => { i = pp_end; continue; }
            };
            let kw2_end = m + kw2.len();

            // After kw2, expect `{$endif}` or `{$ifend}`
            let mut n = kw2_end;
            while n < src_len && (out[n] == b' ' || out[n] == b'\t' || out[n] == b'\r' || out[n] == b'\n') {
                n += 1;
            }
            if n >= src_len || out[n] != b'{' { i = pp_end; continue; }
            let pp3_end = match out[n..].iter().position(|&b| b == b'}') {
                Some(p) => n + p + 1,
                None => { i = pp_end; continue; }
            };
            let pp3_inner = std::str::from_utf8(&out[n + 2..pp3_end - 1])
                .unwrap_or("")
                .trim_start()
                .to_ascii_lowercase();
            if !pp3_inner.starts_with("endif") && !pp3_inner.starts_with("ifend") {
                i = pp_end; continue;
            }

            // Replace the span [pp_start..pp3_end] with kw2 + spaces.
            let span_len = pp3_end - pp_start;
            let replacement: Vec<u8> = {
                let mut v: Vec<u8> = kw2.bytes().collect();
                while v.len() < span_len { v.push(b' '); }
                v.truncate(span_len);
                v
            };
            out[pp_start..pp3_end].copy_from_slice(&replacement);
            // Don't advance i — the replacement is safe to skip over
            i = pp_start + kw2.len();
        } else if pp2_inner.starts_with("ifend") || pp2_inner.starts_with("endif") {
            // Case 2: `{$if...}KW1{$ifend}` — no else branch, keep kw1.
            let span_len = pp2_end - pp_start;
            let replacement: Vec<u8> = {
                let mut v: Vec<u8> = kw1.bytes().collect();
                while v.len() < span_len { v.push(b' '); }
                v.truncate(span_len);
                v
            };
            out[pp_start..pp2_end].copy_from_slice(&replacement);
            i = pp_start + kw1.len();
        } else {
            i = pp_end;
        }
    }

    match String::from_utf8(out) {
        Ok(s) => s,
        Err(_) => fallback, // fallback to original on encoding error
    }
}

/// Neutralise FPC generic specialization syntax so the standard Pascal grammar
/// does not mis-parse it as comparison operators.
///
/// FPC allows `class(specialize TypeName<T1, T2, T3>)` as a parent type,
/// where the parameter list may span several lines.  The tree-sitter-pascal
/// grammar parses `TypeName <` as a comparison expression, cascading into
/// errors that wipe out subsequent type declarations.
///
/// Strategy: find each `Identifier<…>)` span (uppercase-starting identifier
/// followed by `<`, first non-whitespace argument uppercase, matching `>` before
/// a `)`).  Replace the entire `<…>` span with spaces, including embedded
/// newlines, so the multi-line argument list collapses onto one logical line and
/// tree-sitter sees `class(TypeName )`.  Line positions for symbols after the
/// replaced span may shift; symbol names are the extraction goal.
fn normalise_specialize_generics(src: &str) -> String {
    if !src.contains('<') {
        return src.to_string();
    }

    // Pre-step: strip `{$ifdef...}specialize{$endif}` conditional blocks so the
    // bare `specialize` keyword does not appear in the token stream alongside
    // the type name after the `<...>` replacement.  Preserves byte count by
    // overwriting with spaces.
    let src_owned;
    let src = if src.contains("specialize") {
        let mut out = src.as_bytes().to_vec();
        let len = out.len();
        let mut i = 0;
        while i + 9 < len {
            // Look for `{$` opening of any preprocessor directive.
            if out[i] != b'{' || i + 1 >= len || out[i + 1] != b'$' {
                i += 1;
                continue;
            }
            // Find the closing `}`.
            let pp1_end = match out[i..].iter().position(|&b| b == b'}') {
                Some(p) => i + p + 1,
                None => { i += 1; continue; }
            };
            // Check that this is an ifdef/ifndef opener.
            let pp1_inner = std::str::from_utf8(&out[i + 2..pp1_end - 1])
                .unwrap_or("").trim_start().to_ascii_lowercase();
            if !pp1_inner.starts_with("ifdef") && !pp1_inner.starts_with("ifndef")
                && !pp1_inner.starts_with("if ")
            {
                i = pp1_end;
                continue;
            }
            // Skip whitespace after the opener.
            let mut j = pp1_end;
            while j < len && matches!(out[j], b' ' | b'\t' | b'\r' | b'\n') { j += 1; }
            // Check for `specialize` keyword.
            if !out[j..].starts_with(b"specialize") {
                i = pp1_end;
                continue;
            }
            let spec_end = j + b"specialize".len();
            // Skip whitespace after `specialize`.
            let mut k = spec_end;
            while k < len && matches!(out[k], b' ' | b'\t' | b'\r' | b'\n') { k += 1; }
            // Expect `{$endif}` or `{$ifend}`.
            if k >= len || out[k] != b'{' { i = pp1_end; continue; }
            let pp2_end = match out[k..].iter().position(|&b| b == b'}') {
                Some(p) => k + p + 1,
                None => { i = pp1_end; continue; }
            };
            let pp2_inner = std::str::from_utf8(&out[k + 2..pp2_end - 1])
                .unwrap_or("").trim_start().to_ascii_lowercase();
            if !pp2_inner.starts_with("endif") && !pp2_inner.starts_with("ifend") {
                i = pp1_end;
                continue;
            }
            // Replace the entire `{$ifdef...}specialize{$endif}` span with spaces.
            for idx in i..pp2_end {
                if out[idx] != b'\n' && out[idx] != b'\r' { out[idx] = b' '; }
            }
            i = pp2_end;
        }
        src_owned = String::from_utf8(out).unwrap_or_else(|_| src.to_string());
        &src_owned
    } else {
        src
    };

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out = src.to_string().into_bytes();

    let mut i = 0;
    while i < len {
        if bytes[i] == b'<' && i > 0 {
            // Only act on generics of the form `TypeName<` where:
            //  - The char before `<` is the last char of an identifier.
            //  - The identifier itself starts with an uppercase letter (Pascal
            //    type names are PascalCase; comparison operands are usually
            //    lowercase variables or numeric constants).
            //  - The first non-whitespace char after `<` is also an uppercase
            //    letter (the generic type argument is itself a Pascal type).
            let prev_ok = {
                let c = bytes[i - 1];
                c.is_ascii_alphanumeric() || c == b'_'
            };
            // Walk back to find the start of the identifier before `<`.
            let ident_starts_upper = if prev_ok {
                let mut k = i;
                while k > 0 && (bytes[k - 1].is_ascii_alphanumeric() || bytes[k - 1] == b'_') {
                    k -= 1;
                }
                bytes[k].is_ascii_uppercase()
            } else {
                false
            };
            if prev_ok && ident_starts_upper {
                // Require the first non-whitespace character after `<` to be an
                // UPPERCASE letter (generic type argument is a Pascal type name).
                let next_ident = ((i + 1)..len).find_map(|k| {
                    let c = bytes[k];
                    if matches!(c, b' ' | b'\t' | b'\r' | b'\n') { None }
                    else if c.is_ascii_uppercase() || c == b'_' { Some(true) }
                    else { Some(false) }
                }).unwrap_or(false);

                if next_ident {
                    // Locate the matching `>` tracking nesting depth.
                    let mut depth = 1usize;
                    let mut j = i + 1;
                    while j < len && depth > 0 {
                        match bytes[j] {
                            b'<' => depth += 1,
                            b'>' => depth -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                    if depth == 0 {
                        // Only erase when the matching `>` is immediately
                        // followed by `)` (possibly with intervening whitespace).
                        // This distinguishes generic type argument lists
                        // `TypeName<T1,T2>)` from comparison expressions where
                        // `>` is followed by an identifier, operator, or `;`.
                        let closes_paren = ((j)..len).find_map(|k| {
                            let c = bytes[k];
                            if matches!(c, b' ' | b'\t' | b'\r' | b'\n') { None }
                            else if c == b')' { Some(true) }
                            else { Some(false) }
                        }).unwrap_or(false);
                        if closes_paren {
                            // Replace the entire `<…>` span (positions i..j)
                            // with a single space at position i and fill the
                            // rest with spaces.  Newlines are also replaced so
                            // the multi-line type-argument list collapses onto
                            // one line, preventing tree-sitter from closing
                            // nodes prematurely at intermediate blank lines.
                            // Line-number accuracy for symbols inside `.inc`
                            // fragments is sacrificed deliberately: symbol
                            // names are the primary extraction goal.
                            for k in i..j {
                                out[k] = b' ';
                            }
                            i = j;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }

    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

// ---------------------------------------------------------------------------
// Root traversal
// ---------------------------------------------------------------------------

fn visit_root(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // For complete units/programs, the root has children we iterate normally.
    // For .inc fragments, three error-recovery variants apply:
    //
    //   Root is ERROR (Variants A/C): dispatch the root so try_extract_error_type_decl
    //   can see all children (name-error + body children) in one call.
    //
    //   Root is 'root' with ERROR children (Variant B: interface/class where the
    //   type keyword is inline in the same ERROR as identifier+kEq): collect all
    //   root children and call try_extract_error_type_decl with the whole set so
    //   the body siblings are accessible for member extraction.
    if node.kind() == "ERROR" {
        dispatch(node, src, symbols, refs, None);
        return;
    }

    let mut cursor = node.walk();
    let root_children: Vec<Node> = node.children(&mut cursor).collect();

    // Detect Variant B at the root level: at least one ERROR child starts with
    // [identifier, kEq, type_keyword].  If found, group all children into a
    // virtual container for try_extract_error_type_decl.
    if try_extract_root_type_decls(&root_children, src, symbols, refs) {
        return;
    }

    for child in root_children {
        dispatch(child, src, symbols, refs, None);
    }
}

fn dispatch(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    match node.kind() {
        "unit" => extract_unit(node, src, symbols, refs),
        "program" | "library" => extract_program(node, src, symbols, refs),
        "declProc" | "defProc" => extract_proc(node, src, symbols, refs, parent_index),
        // declType is the wrapper that carries the name for class/intf/enum/record bodies.
        "declType" => extract_decl_type(node, src, symbols, refs, parent_index),
        // declClass / declIntf dispatched directly (e.g. inside a unit body without declType)
        // are handled with name fallback via find_decl_type_name.
        "declClass" => extract_class(node, src, symbols, refs, parent_index, None),
        "declIntf" => extract_intf(node, src, symbols, refs, parent_index, None),
        "declSection" => extract_section(node, src, symbols, refs, parent_index),
        "declUses" => extract_uses(node, src, symbols, refs, parent_index),
        // declVars / declConsts — container nodes; dispatch each declVar / declConst child.
        "declVars" | "declConsts" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
        "declVar" => extract_var(node, src, symbols, refs, parent_index),
        "declConst" => {
            // Check whether error recovery folded a `TypeName = class(...)` declaration
            // into this constant node (the `type(typeref(Name)) defaultValue(= class(...))`
            // pattern produced when a generic class declaration follows an error-recovery
            // constant boundary).  When found, extract the embedded type declaration and
            // skip the spurious constant symbol.  Otherwise extract normally.
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            let embedded_type = children.windows(2).find(|w| {
                w[0].kind() == "type" && w[1].kind() == "defaultValue"
            });
            if let Some(w) = embedded_type {
                let type_node = w[0];
                let dv_node = w[1];
                if let Some(sym_kind) = infer_type_kind_from_default_value(dv_node, src) {
                    // Extract the name from the `type` node: it wraps a `typeref` which
                    // contains an `identifier`.
                    let name_opt = {
                        let mut tc = type_node.walk();
                        let type_children: Vec<Node> = type_node.children(&mut tc).collect();
                        type_children.iter().find_map(|c| {
                            if c.kind() == "typeref" {
                                let mut rc = c.walk();
                                let rc_ch: Vec<Node> = c.children(&mut rc).collect();
                                rc_ch.into_iter().find(|n| n.kind() == "identifier")
                            } else if c.kind() == "identifier" {
                                Some(*c)
                            } else {
                                None
                            }
                        })
                    };
                    if let Some(name_node) = name_opt {
                        let name = node_text(name_node, src);
                        if !name.is_empty() {
                            symbols.push(make_symbol(
                                name.clone(),
                                name,
                                sym_kind,
                                &type_node,
                                None,
                                parent_index,
                            ));
                            return;
                        }
                    }
                }
            }
            extract_const(node, src, symbols, refs, parent_index);
        }
        "exprCall" => {
            extract_call(node, src, refs, parent_index);
            // Recurse into arguments and nested sub-expressions so that
            // exprCall nodes inside arguments are also dispatched.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
        "typeref" => extract_typeref(node, src, refs, parent_index),
        "ERROR" => {
            // When a .inc file contains `TypeName = class ...` without the surrounding
            // `type` keyword, tree-sitter produces an ERROR node rather than declType →
            // declClass. Try to recover the declaration; fall back to generic recursion.
            if !try_extract_error_type_decl(node, src, symbols, refs, parent_index) {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    dispatch(child, src, symbols, refs, parent_index);
                }
            }
        }
        _ => {
            // Recurse into containers.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error-recovery helpers called from visit_root

/// Called when the tree root has kind 'root' (not ERROR). Checks if the root
/// children contain ERROR nodes that look like .inc-style type declarations.
/// Uses the same sibling-scan logic as try_extract_error_type_decl so that
/// multiple sequential declarations at the root level are all recovered.
///
/// Returns true when at least one type declaration was recovered, signalling
/// that normal child iteration should be skipped.
fn try_extract_root_type_decls(
    root_children: &[Node],
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) -> bool {
    // Only engage if at least one root child looks like a name-error (ERROR
    // starting with identifier + kEq) so we don't accidentally eat valid code.
    let has_name_err = root_children.iter().any(|c| {
        if c.kind() != "ERROR" { return false; }
        let mut cc = c.walk();
        let ch: Vec<Node> = c.children(&mut cc).collect();
        ch.len() >= 2 && ch[0].kind() == "identifier" && ch[1].kind() == "kEq"
    });
    if !has_name_err { return false; }

    recover_type_decls_from_siblings(root_children, src, symbols, refs, None) > 0
}

// ---------------------------------------------------------------------------
// Error-recovery: TypeName = class/record/interface without 'type' keyword
//
// Pascal .inc files are fragments inside a `type` block of the parent .pas.
// Parsed standalone, tree-sitter generates ERROR nodes.  The recovery
// strategy scans a flat list of sibling nodes (children of the root ERROR or
// the root 'root' node) for consecutive [name-ERROR, type-body] pairs and
// emits one symbol per pair.
//
// Node shapes seen in castle-fresh (post-preprocessor-strip):
//
//  A — single class, no guard:
//    ERROR                     ← root
//      ERROR(ident, kEq)       ← name
//      declProc(kClass, ...)   ← body
//
//  B — single interface (kInterface inline in name-ERROR):
//    root                      ← root
//      ERROR(ident, kEq, kInterface)   ← name + keyword fused
//      ERROR(body...)          ← body
//
//  C — multiple forward decls inside {$ifdef}/{$endif}:
//    ERROR                     ← root
//      pp
//      ERROR(ident, kEq)       ← name TypeA
//      declProc(kClass ;)      ← body TypeA (no-body forward decl)
//      ERROR(ident, kEq)       ← name TypeB
//      declProc(kClass ;)      ← body TypeB
//      ...
//      pp
//
// All variants reduce to: scan `children` sequentially, detect name-errors
// (ERROR starting with [identifier, kEq]) and consume their following
// sibling as the body (which provides the type keyword).
// ---------------------------------------------------------------------------

/// Scan a slice of sibling AST nodes for type declaration patterns and emit
/// one symbol per declaration found.
///
/// Handles two sibling layouts produced by tree-sitter for .inc-style fragments:
///
///  Layout 1 — one name-error, one body node:
///    ERROR(ident kEq)  declProc(kClass body...)
///
///  Layout 2 — fused body: the body ERROR absorbs the next decl's name.
///  Arises for consecutive forward declarations where each `class;` is
///  swallowed into the same body ERROR as the subsequent name:
///    ERROR(ident_A kEq)
///    ERROR(kClass ";" ident_B kEq)
///    ERROR(kClass ";" ident_C kEq)
///    ERROR(kClass ";" pp)
///
/// Returns the number of type declarations recovered.
fn recover_type_decls_from_siblings(
    siblings: &[Node],
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> usize {
    let mut count = 0;

    // Collect all tokens from the sibling list into a flat sequence so we can
    // slide a window over them.  Each (kind, node) pair is one token.
    // We process sequentially, tracking a pending name to be paired with
    // the next type keyword we encounter.
    let mut pending_name: Option<Node> = None;
    let mut pending_kind: Option<SymbolKind> = None;
    let mut body_children: Vec<Node> = Vec::new();

    // Flush any pending (name, kind) pair as a new symbol, recursing into body_children
    // for member extraction.  This is an inline macro-style block rather than a closure
    // to avoid multiple-mutable-borrow issues.
    macro_rules! emit_pending {
        ($anchor:expr) => {{
            if let (Some(nm), Some(kd)) = (pending_name.take(), pending_kind.take()) {
                let name = node_text(nm, src);
                if !name.is_empty() {
                    let sig = first_line_of($anchor, src);
                    let idx = symbols.len();
                    symbols.push(make_symbol(name.clone(), name, kd, &$anchor, Some(sig), parent_index));
                    count += 1;
                    let drained: Vec<Node> = body_children.drain(..).collect();
                    // First pass: dispatch any nodes that are class/record/interface bodies.
                    // Second pass: run recover_type_decls_from_siblings on the collected body
                    // to pick up further type declarations embedded in the body (e.g. a class
                    // declaration whose name is a typeref followed by a defaultValue node).
                    let has_embedded = drained.iter().any(|n| {
                        n.kind() == "typeref" || n.kind() == "identifier"
                    });
                    if has_embedded {
                        // Try sibling-scan first so nested type declarations are picked up.
                        let extra = recover_type_decls_from_siblings(&drained, src, symbols, refs, Some(idx));
                        if extra == 0 {
                            // No nested declarations found; dispatch bodies individually.
                            for bc in &drained {
                                dispatch_type_body(*bc, src, symbols, refs, Some(idx));
                            }
                        }
                    } else {
                        for bc in &drained {
                            dispatch_type_body(*bc, src, symbols, refs, Some(idx));
                        }
                    }
                }
            }
            body_children.clear();
        }};
    }

    let mut si_outer = 0;
    while si_outer < siblings.len() {
        let sibling = &siblings[si_outer];
        si_outer += 1;

        if matches!(sibling.kind(), "pp" | "comment") {
            continue;
        }

        // Top-level `identifier` + `kEq` pattern: a bare identifier followed by
        // `=` in a flat sibling list (e.g. children of an ERROR node passed
        // directly to this function).  The kind is inferred from the sibling
        // after `kEq` (kClass/kInterface/kRecord or defaultValue).
        if sibling.kind() == "identifier"
            && si_outer < siblings.len()
            && siblings[si_outer].kind() == "kEq"
        {
            // Look past the kEq to find the type keyword.
            let kind_opt = if si_outer + 1 < siblings.len() {
                let after_eq = siblings[si_outer + 1];
                match after_eq.kind() {
                    "kClass"     => Some(SymbolKind::Class),
                    "kInterface" => Some(SymbolKind::Interface),
                    "kRecord"    => Some(SymbolKind::Struct),
                    "defaultValue" => infer_type_kind_from_default_value(after_eq, src),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(sym_kind) = kind_opt {
                emit_pending!(*sibling);
                pending_name = Some(*sibling);
                pending_kind = Some(sym_kind);
                si_outer += 2; // skip kEq and the type keyword/defaultValue
                for later in &siblings[si_outer..] {
                    body_children.push(*later);
                }
                si_outer = siblings.len();
                continue;
            }
        }

        // Top-level `typeref` + `defaultValue` pattern: the type name was parsed as a
        // typeref node in the sibling list itself (not embedded inside another node).
        if sibling.kind() == "typeref" && si_outer < siblings.len()
            && siblings[si_outer].kind() == "defaultValue"
        {
            if let Some(sym_kind) = infer_type_kind_from_default_value(siblings[si_outer], src) {
                let mut tc = sibling.walk();
                let tc_ch: Vec<Node> = sibling.children(&mut tc).collect();
                if let Some(name_node) = tc_ch.iter().find(|n| n.kind() == "identifier") {
                    emit_pending!(*sibling);
                    pending_name = Some(*name_node);
                    pending_kind = Some(sym_kind);
                    let dv_node = siblings[si_outer];
                    // The defaultValue was consumed; skip it.
                    si_outer += 1;
                    // Push the interior of the defaultValue (non-kEq children) into
                    // body_children so any nested type declarations inside it are
                    // processed (e.g. an exprBinary representing a new type decl
                    // that was folded into the defaultValue by error recovery).
                    let mut dvc = dv_node.walk();
                    for dv_child in dv_node.children(&mut dvc) {
                        if dv_child.kind() != "kEq" {
                            body_children.push(dv_child);
                        }
                    }
                    // Remaining siblings become body_children.
                    for later in &siblings[si_outer..] {
                        body_children.push(*later);
                    }
                    si_outer = siblings.len(); // consume all remaining
                    continue;
                }
            }
        }

        if sibling.kind() != "ERROR" {
            // `exprBinary` produced by error recovery for `TypeName = class(...)`.
            // Children: identifier, operator(=), then the class/interface/record body.
            if sibling.kind() == "exprBinary" {
                let mut eb = sibling.walk();
                let eb_ch: Vec<Node> = sibling.children(&mut eb).collect();
                if eb_ch.len() >= 3 {
                    let name_ident = eb_ch.iter().find(|n| n.kind() == "identifier").copied();
                    let has_eq = eb_ch.iter().any(|n| {
                        n.kind() == "kEq"
                            || (n.kind() == "operator" && node_text(*n, src) == "=")
                    });
                    let kind_opt = eb_ch.iter().find_map(|n| {
                        match n.kind() {
                            "kClass"     => Some(SymbolKind::Class),
                            "kInterface" => Some(SymbolKind::Interface),
                            "kRecord"    => Some(SymbolKind::Struct),
                            "exprCall" | "ERROR" => infer_type_kind_from_default_value(*n, src),
                            _ => None,
                        }
                    });
                    if has_eq {
                        if let (Some(sym_kind), Some(name_node)) = (kind_opt, name_ident) {
                            emit_pending!(*sibling);
                            pending_name = Some(name_node);
                            pending_kind = Some(sym_kind);
                            continue;
                        }
                    }
                }
            }

            // Non-ERROR siblings: check if this is a type-body node (declProc/declSection
            // starting with kClass/kInterface/kRecord) when we have a pending name.
            // tree-sitter wraps the body of `TypeName = class ... end;` in a declProc
            // whose first child is kClass, with the full class body nested inside.
            if pending_name.is_some() {
                let type_kw = type_keyword_of_node(*sibling, src);
                if let Some(kd) = type_kw {
                    // This node IS the class/interface/record body.
                    if pending_kind.is_none() {
                        pending_kind = Some(kd);
                    }
                    body_children.push(*sibling);
                    continue;
                }
            }

            // Scan the direct children of this non-ERROR sibling for the pattern:
            //   `identifier("TypeName")  defaultValue(kEq exprCall("class"/"record"/...))`
            //
            // This is how tree-sitter's error recovery represents a new type declaration
            // embedded inside a `declProc` node belonging to the previous type's body.
            // When found: flush the current pending declaration and start a new one.
            {
                let mut sc = sibling.walk();
                let sib_children: Vec<Node> = sibling.children(&mut sc).collect();
                let mut found_embedded = false;
                for (si, sc_node) in sib_children.iter().enumerate() {
                    // Resolve the name node: either a plain `identifier` or a `typeref`
                    // wrapping a single identifier (tree-sitter may parse the class name
                    // as a type reference when it appears after a method signature tail).
                    let name_ident: Option<Node> = if sc_node.kind() == "identifier" {
                        Some(*sc_node)
                    } else if sc_node.kind() == "typeref" {
                        let mut tc = sc_node.walk();
                        let tc_children: Vec<Node> = sc_node.children(&mut tc).collect();
                        tc_children.into_iter().find(|n| n.kind() == "identifier")
                    } else {
                        None
                    };

                    if let Some(name_node) = name_ident {
                        if si + 1 < sib_children.len() {
                            let next = sib_children[si + 1];
                            // Look for `defaultValue` sibling that represents `= class(...)`.
                            if next.kind() == "defaultValue" {
                                if let Some(sym_kind) = infer_type_kind_from_default_value(next, src) {
                                    emit_pending!(*sibling);
                                    pending_name = Some(name_node);
                                    pending_kind = Some(sym_kind);
                                    for later in sib_children.iter().skip(si + 2) {
                                        body_children.push(*later);
                                    }
                                    found_embedded = true;
                                    break;
                                }
                            }
                        }
                    }
                    // When a child is an ERROR, its LAST identifier may be the name of the
                    // next type declaration.  Two sub-patterns:
                    //
                    //  a) ERROR + defaultValue: the `= class(...)` form.
                    //  b) ERROR + kClass/kInterface/kRecord: the bare `class(ParentType)`
                    //     form, where the type keyword stands alone as the next token.
                    if sc_node.kind() == "ERROR" && si + 1 < sib_children.len() {
                        let next = sib_children[si + 1];
                        let sym_kind_opt: Option<SymbolKind> = if next.kind() == "defaultValue" {
                            infer_type_kind_from_default_value(next, src)
                        } else {
                            match next.kind() {
                                "kClass" => Some(SymbolKind::Class),
                                "kInterface" => Some(SymbolKind::Interface),
                                "kRecord" => Some(SymbolKind::Struct),
                                _ => None,
                            }
                        };
                        if let Some(sym_kind) = sym_kind_opt {
                            // Grab the last identifier child of the ERROR as the type name.
                            let mut ec = sc_node.walk();
                            let err_ch: Vec<Node> = sc_node.children(&mut ec).collect();
                            let last_ident = err_ch.iter().rev()
                                .find(|n| n.kind() == "identifier");
                            if let Some(name_node) = last_ident {
                                emit_pending!(*sibling);
                                pending_name = Some(*name_node);
                                pending_kind = Some(sym_kind);
                                // Skip past the ERROR and the type keyword/defaultValue;
                                // subsequent children are the body of this new type.
                                let skip = if next.kind() == "defaultValue" { si + 2 } else { si + 2 };
                                for later in sib_children.iter().skip(skip) {
                                    body_children.push(*later);
                                }
                                found_embedded = true;
                                break;
                            }
                        }
                    }
                    // Skip `kClass`/`kInterface`/`kRecord` when it is a method modifier
                    // (`class function`/`class procedure` etc.).  Otherwise stop scanning —
                    // we've reached a type body that belongs to a pending declaration and
                    // no further embedded names follow (they'd have been handled above).
                    if matches!(sc_node.kind(), "kClass" | "kInterface" | "kRecord") {
                        let next_is_method_kw = sib_children.get(si + 1)
                            .map(|n| matches!(n.kind(), "kFunction" | "kProcedure" | "kConstructor" | "kDestructor"))
                            .unwrap_or(false);
                        if !next_is_method_kw {
                            break;
                        }
                    }
                }
                if found_embedded { continue; }
            }

            // No pending name, or not a type-body node → dispatch generically.
            if pending_name.is_some() && pending_kind.is_some() {
                body_children.push(*sibling);
            } else {
                dispatch(*sibling, src, symbols, refs, parent_index);
            }
            continue;
        }

        // Walk the children of this ERROR node token by token.
        let mut ec = sibling.walk();
        let err_children: Vec<Node> = sibling.children(&mut ec).collect();
        let mut j = 0;
        while j < err_children.len() {
            let tok = err_children[j];
            match tok.kind() {
                "identifier" => {
                    // If followed by kEq, this is a type name.
                    if j + 1 < err_children.len() && err_children[j + 1].kind() == "kEq" {
                        // Flush any pending declaration first.
                        emit_pending!(*sibling);
                        pending_name = Some(tok);
                        // Check if the type kind is embedded in kEq's children.
                        let keq_node = err_children[j + 1];
                        if let Some(kind) = infer_type_kind_from_eq_sibling(keq_node, src) {
                            pending_kind = Some(kind);
                        }
                        j += 2; // skip identifier and kEq
                        continue;
                    }
                    // If followed by a `defaultValue` node (alternative error-recovery form),
                    // check if it represents `= class(...)`.
                    if j + 1 < err_children.len() && err_children[j + 1].kind() == "defaultValue" {
                        if let Some(sym_kind) = infer_type_kind_from_default_value(err_children[j + 1], src) {
                            emit_pending!(*sibling);
                            pending_name = Some(tok);
                            pending_kind = Some(sym_kind);
                            j += 2; // skip identifier and defaultValue
                            continue;
                        }
                    }
                    // Not a name → body content.
                    if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "kClass" => {
                    if pending_name.is_some() && pending_kind.is_none() {
                        pending_kind = Some(SymbolKind::Class);
                    } else if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "kInterface" => {
                    if pending_name.is_some() && pending_kind.is_none() {
                        pending_kind = Some(SymbolKind::Interface);
                    } else if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "kRecord" => {
                    if pending_name.is_some() && pending_kind.is_none() {
                        pending_kind = Some(SymbolKind::Struct);
                    } else if pending_name.is_some() {
                        body_children.push(tok);
                    }
                }
                "pp" | "comment" => {} // skip preprocessor/comments in body
                _ => {
                    if pending_name.is_some() {
                        // When we have a name but no kind yet, try to infer the kind
                        // from the node.  `declClass` and `declIntf` are produced when
                        // tree-sitter successfully parses the class body after error
                        // recovery strips the generic params from the parent type.
                        if pending_kind.is_none() {
                            if let Some(kd) = type_keyword_of_node(tok, src) {
                                pending_kind = Some(kd);
                            } else if tok.kind() == "declClass" {
                                pending_kind = Some(SymbolKind::Class);
                            } else if tok.kind() == "declIntf" {
                                pending_kind = Some(SymbolKind::Interface);
                            }
                        }
                        body_children.push(tok);
                    }
                }
            }
            j += 1;
        }

        // Non-token children (e.g. typeref, declSection) that are not leaves.
        // Already handled above by the general arm.
    }

    // Flush any remaining pending declaration.
    let anchor_dummy = if let Some(last) = siblings.last() { *last } else { return count; };
    emit_pending!(anchor_dummy);

    count
}

/// Attempt to recover type declaration(s) from an ERROR node produced when
/// `TypeName = class/interface/record ...` appears without the surrounding
/// `type` keyword (typical in Pascal .inc fragment files).
///
/// Returns `true` when at least one symbol was recovered.
fn try_extract_error_type_decl(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    recover_type_decls_from_siblings(&children, src, symbols, refs, parent_index) > 0
}


// ---------------------------------------------------------------------------
// unit <Name>;  →  Namespace
// ---------------------------------------------------------------------------

fn extract_unit(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = find_identifier_child(node, src)
        .unwrap_or_else(|| "unit".to_string());
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Namespace,
        &node,
        None,
        None,
    ));

    // Recurse into unit body.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

fn extract_program(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = find_identifier_child(node, src)
        .unwrap_or_else(|| "program".to_string());
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Namespace,
        &node,
        None,
        None,
    ));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// procedure/function declarations  →  Function
// declProc = forward declaration header only
// defProc  = full definition with body
// ---------------------------------------------------------------------------

fn extract_proc(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = find_proc_name(node, src)
        .unwrap_or_else(|| "unknown".to_string());

    let sig = first_line_of(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        &node,
        Some(sig),
        parent_index,
    ));

    // Recurse into body for nested procs and calls.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

fn find_proc_name(node: Node, src: &str) -> Option<String> {
    // Pascal proc names: first identifier/operatorName child after kFunction/kProcedure
    let mut cursor = node.walk();
    let mut saw_keyword = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "kFunction" | "kProcedure" | "kConstructor" | "kDestructor" | "kOperator" => {
                saw_keyword = true;
            }
            "identifier" | "operatorName" if saw_keyword => {
                return Some(node_text(child, src));
            }
            // Qualified name: TypeName.MethodName
            "genericDot" | "exprDot" if saw_keyword => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    // Fallback: first identifier child.
    find_identifier_child(node, src)
}

// ---------------------------------------------------------------------------
// declType: type <Name> = <body>;
//
// The name is the first `identifier` child of `declType`.  The body is one of:
//   declClass, declIntf, declEnum — dispatched with the resolved name.
//   Other bodies (type alias, set, etc.) are recursed generically.
// ---------------------------------------------------------------------------

fn extract_decl_type(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // The name sits on the `identifier` child of `declType`, before `=`.
    let name = find_identifier_child(node, src);
    let mut emitted_primary = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "declClass" => {
                extract_class(child, src, symbols, refs, parent_index, name.clone());
                emitted_primary = true;
            }
            "declIntf" => {
                extract_intf(child, src, symbols, refs, parent_index, name.clone());
                emitted_primary = true;
            }
            "type" => {
                // The `type` child wraps the body expression (declEnum, typeref, etc.)
                if extract_decl_type_body(
                    child, src, symbols, refs, parent_index, name.clone(), &node,
                ) {
                    emitted_primary = true;
                }
            }
            _ => {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
    }

    // Plain type aliases (`PFoo = ^TFoo`, `TMask = set of TByte`,
    // `TIntArray = array of Integer`, `TByteHandler = procedure(b: Byte)`)
    // didn't fall into declClass / declIntf / declEnum, so no symbol got
    // emitted above. They're still named types — and the FFI binding
    // pattern `P<X> = ^T<X>` is the dominant unresolved-ref source in
    // Pascal projects with C-library bindings (GTK/GLib/OpenGL). Emit
    // a TypeAlias symbol so the resolver can find them.
    if !emitted_primary {
        if let Some(n) = name {
            symbols.push(make_symbol(
                n.clone(),
                n,
                SymbolKind::TypeAlias,
                &node,
                Some(first_line_of(node, src)),
                parent_index,
            ));
        }
    }
}

/// Dispatch the body of a `type` wrapper node inside `declType`.
///
/// Returns `true` when this body was itself a primary type kind (today:
/// `declEnum`) so the caller knows not to emit a fallback TypeAlias
/// symbol on top of the enum.
fn extract_decl_type_body(
    type_node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    name: Option<String>,
    decl_node: &Node,
) -> bool {
    let mut emitted_primary = false;
    let mut cursor = type_node.walk();
    for child in type_node.children(&mut cursor) {
        match child.kind() {
            "declEnum" => {
                let n = name.clone().unwrap_or_else(|| "unknown".to_string());
                let idx = symbols.len();
                symbols.push(make_symbol(
                    n.clone(),
                    n,
                    SymbolKind::Enum,
                    decl_node,
                    Some(first_line_of(*decl_node, src)),
                    parent_index,
                ));
                emitted_primary = true;
                // Recurse into enum for enum members if needed.
                let mut cur2 = child.walk();
                for ec in child.children(&mut cur2) {
                    dispatch(ec, src, symbols, refs, Some(idx));
                }
            }
            _ => {
                dispatch(child, src, symbols, refs, parent_index);
            }
        }
    }
    emitted_primary
}

// ---------------------------------------------------------------------------
// class type declarations  →  Class
// ---------------------------------------------------------------------------

fn extract_class(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    name_override: Option<String>,
) {
    let name = name_override
        .or_else(|| find_decl_type_name(node, src))
        .unwrap_or_else(|| "unknown".to_string());

    let sig = first_line_of(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Class,
        &node,
        Some(sig),
        parent_index,
    ));

    // Emit Inherits edge for parent class — the first `typeref` child directly
    // inside `declClass` (before any `declSection`) is the parent class.
    {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "typeref" {
                // This is the parent class typeref: class(ParentName)
                let mut tcur = child.walk();
                for tc in child.children(&mut tcur) {
                    match tc.kind() {
                        "identifier" => {
                            let parent_name = node_text(tc, src);
                            if !parent_name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: idx,
                                    target_name: parent_name,
                                    kind: EdgeKind::Inherits,
                                    line: child.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                            break;
                        }
                        "typerefDot" => {
                            let (member, qualifier) = split_dot_node(tc, src);
                            if !member.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: idx,
                                    target_name: member,
                                    kind: EdgeKind::Inherits,
                                    line: child.start_position().row as u32,
                                    module: qualifier,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                            break;
                        }
                        _ => {}
                    }
                }
                break; // only first typeref is the parent
            }
        }
    }

    // Recurse for nested members.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// interface type declarations  →  Interface
// ---------------------------------------------------------------------------

fn extract_intf(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    name_override: Option<String>,
) {
    let name = name_override
        .or_else(|| find_decl_type_name(node, src))
        .unwrap_or_else(|| "unknown".to_string());

    let sig = first_line_of(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Interface,
        &node,
        Some(sig),
        parent_index,
    ));

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// declSection: visibility/type/var/const sections inside a class or interface.
// Record sections emit a Struct symbol.  Other sections recurse their children,
// dispatching declField → Field and declProp → Property directly.
// ---------------------------------------------------------------------------

fn extract_section(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let has_record = has_keyword_child(node, "kRecord");

    if has_record {
        // Record type block: emit a Struct symbol for the record itself.
        let name = find_decl_type_name(node, src)
            .unwrap_or_else(|| "record".to_string());
        let sig = first_line_of(node, src);
        let idx = symbols.len();
        symbols.push(make_symbol(
            name.clone(),
            name,
            SymbolKind::Struct,
            &node,
            Some(sig),
            parent_index,
        ));
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            dispatch(child, src, symbols, refs, Some(idx));
        }
    } else {
        // Visibility section (private/public/protected/published) — no symbol emitted.
        // Recurse children, routing declField and declProp to dedicated extractors.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "declField" => extract_field(child, src, symbols, refs, parent_index),
                "declProp" => extract_prop(child, src, symbols, refs, parent_index),
                _ => dispatch(child, src, symbols, refs, parent_index),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// declField  →  Field
// ---------------------------------------------------------------------------

fn extract_field(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = find_identifier_child(node, src)
        .unwrap_or_else(|| "unknown".to_string());
    let sig = first_line_of(node, src);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Field,
        &node,
        Some(sig),
        parent_index,
    ));
}

// ---------------------------------------------------------------------------
// declProp  →  Property
// ---------------------------------------------------------------------------

fn extract_prop(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // declProp layout: kProperty identifier : type [read Getter] [write Setter] ;
    // The name is the identifier after kProperty.
    let mut cursor = node.walk();
    let mut saw_keyword = false;
    let mut name = None;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "kProperty" => { saw_keyword = true; }
            "identifier" if saw_keyword && name.is_none() => {
                name = Some(node_text(child, src));
            }
            _ => {}
        }
    }
    let name = name.unwrap_or_else(|| "unknown".to_string());
    let sig = first_line_of(node, src);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Property,
        &node,
        Some(sig),
        parent_index,
    ));
}

// ---------------------------------------------------------------------------
// uses <unit1>, <unit2>;  →  Symbol (Namespace) + Imports refs
// declUses appears in both symbol_node_kinds and ref_node_kinds, so we emit
// a symbol for the whole uses block AND a ref for every module listed.
// Grammar: declUses children are kUses + moduleName nodes.
// ---------------------------------------------------------------------------

fn extract_uses(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Emit a lightweight symbol so the symbol coverage checker is satisfied.
    let sym_idx = symbols.len();
    symbols.push(make_symbol(
        "uses".to_string(),
        "uses".to_string(),
        SymbolKind::Namespace,
        &node,
        None,
        parent_index,
    ));

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Grammar only has kUses (keyword) and moduleName children.
        if child.kind() == "moduleName" || child.kind() == "identifier" {
            let name = node_text(child, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(name),
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
    }
}

// ---------------------------------------------------------------------------
// declVar  →  Variable
// Grammar: declVar has identifier child(ren) + type child.
// ---------------------------------------------------------------------------

fn extract_var(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = find_identifier_child(node, src)
        .unwrap_or_else(|| "unknown".to_string());
    if name == "unknown" {
        return;
    }
    let sig = first_line_of(node, src);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        &node,
        Some(sig),
        parent_index,
    ));
    // Recurse to pick up typeref children (type references in the variable's type annotation).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dispatch(child, src, symbols, refs, Some(idx));
    }
}

// ---------------------------------------------------------------------------
// declConst  →  Variable (constants treated as variables for indexing purposes)
// Grammar: declConst has identifier + defaultValue children.
// ---------------------------------------------------------------------------

fn extract_const(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    _refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name = find_identifier_child(node, src)
        .unwrap_or_else(|| "unknown".to_string());
    if name == "unknown" {
        return;
    }
    let sig = first_line_of(node, src);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        &node,
        Some(sig),
        parent_index,
    ));
}

// ---------------------------------------------------------------------------
// typeref  →  TypeRef (type usage references)
// typeref children include identifier / typerefDot / typerefPtr / typerefTpl
// We extract the leading identifier as the referenced type name.
// ---------------------------------------------------------------------------

fn extract_typeref(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                return; // one ref per typeref is enough
            }
            "typerefDot" => {
                // Qualified type: Unit.Type — split into qualifier + member
                let (member, qualifier) = split_dot_node(child, src);
                if !member.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: member,
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: qualifier,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// exprCall  →  Calls
// ---------------------------------------------------------------------------

fn extract_call(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    // exprCall.entity is the callee.  Use the named field when available,
    // falling back to child(0) for grammars that omit the field name.
    let callee_opt = node.child_by_field_name("entity").or_else(|| node.child(0));
    if let Some(callee) = callee_opt {
        let (name, module) = resolve_call_target(callee, src);
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module,
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }
    }
}

/// Resolve a callee expression to `(target_name, module)`.
///
/// For qualified calls like `SysUtils.FreeAndNil`:
///   - `target_name` = "FreeAndNil"  (last segment)
///   - `module`      = Some("SysUtils")
///
/// For simple identifiers, `module` is `None`.
fn resolve_call_target(node: Node, src: &str) -> (String, Option<String>) {
    match node.kind() {
        "identifier" => (node_text(node, src), None),
        // exprDot / genericDot: children are identifier . identifier
        // Named children: [0] = qualifier, [1] = member
        "exprDot" | "genericDot" => split_dot_node(node, src),
        // Chained call: take the outer call's entity
        "exprCall" => {
            let inner = node.child_by_field_name("entity").or_else(|| node.child(0));
            inner.map(|n| resolve_call_target(n, src)).unwrap_or_default()
        }
        // Parenthesised expression — unwrap
        "exprParens" => {
            if let Some(inner) = node.named_child(0) {
                resolve_call_target(inner, src)
            } else {
                (String::new(), None)
            }
        }
        // Subscript / bracket access: take entity
        "exprBrackets" | "exprSubscript" => {
            let inner = node.child_by_field_name("entity").or_else(|| node.child(0));
            inner.map(|n| resolve_call_target(n, src)).unwrap_or_default()
        }
        // `inherited` keyword call: `inherited Create(...)` → use "inherited"
        "inherited" => ("inherited".to_string(), None),
        _ => {
            let t = node_text(node, src);
            if !t.is_empty() { (t, None) } else { (String::new(), None) }
        }
    }
}

/// Split an `exprDot` / `genericDot` / `typerefDot` node into `(member, Some(qualifier))`.
///
/// Grammar layout: identifier  kDot(.)  identifier
/// Named children (excluding anonymous punctuation) are the two identifier nodes.
/// named_child(0) = qualifier, named_child(1) = member.
fn split_dot_node(node: Node, src: &str) -> (String, Option<String>) {
    let count = node.named_child_count();
    if count >= 2 {
        let qualifier = node.named_child(0).map(|n| node_text(n, src)).unwrap_or_default();
        let member    = node.named_child(count - 1).map(|n| node_text(n, src)).unwrap_or_default();
        if !member.is_empty() {
            return (member, if qualifier.is_empty() { None } else { Some(qualifier) });
        }
    }
    // Fallback: return full text as target_name with no module
    (node_text(node, src), None)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_identifier_child(node: Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "moduleName") {
            return Some(node_text(child, src));
        }
    }
    None
}

/// For type declarations (class, interface, record): the name is typically
/// the identifier child of the containing `type` block. Walk up one level
/// or look for a varDef / declType wrapping node.
/// Simplified: look for first identifier child of the node itself.
fn find_decl_type_name(node: Node, src: &str) -> Option<String> {
    // Try named child "name" field first.
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(name_node, src));
    }
    find_identifier_child(node, src)
}

/// Dispatch a node that was identified as a type body in error-recovery context.
///
/// When tree-sitter wraps `TypeName = class ... end;` without the surrounding
/// `type` keyword, the class body ends up as a `declProc` whose first child is
/// `kClass`.  The body's contents are a mix of structured nodes (`declProc`,
/// `declSection`) and bare tokens (`kProcedure`, `identifier`, `;`) produced
/// by the grammar's error-recovery.  This function:
///
///   1. Skips the leading type keyword (kClass/kInterface/kRecord).
///   2. Dispatches structured children (declProc, declSection, etc.) normally.
///   3. For bare token sequences, scans for `kProcedure`/`kFunction` +
///      `identifier` patterns and emits a Function symbol for each.
fn dispatch_type_body(
    node: Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    if type_keyword_of_node(node, src).is_none() {
        dispatch(node, src, symbols, refs, parent_index);
        return;
    }

    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();

    let mut saw_type_keyword = false;
    let mut pending_proc_kw = false; // set after kProcedure/kFunction/etc.
    // Tracks whether the previous two tokens were `identifier kEq`, indicating
    // the start of a `TypeName = class/record` declaration embedded in the body
    // ERROR node by tree-sitter's error recovery.
    let mut pending_name_for_type: Option<Node> = None; // the identifier before kEq

    for (ci, child) in children.iter().enumerate() {
        match child.kind() {
            "kClass" | "kInterface" | "kRecord" if !saw_type_keyword => {
                saw_type_keyword = true; // skip the leading type keyword
                pending_name_for_type = None;
            }
            // When we see `kClass/kInterface/kRecord` after `identifier kEq`,
            // a new type declaration has been embedded in this body ERROR node.
            // Emit it as a Class/Interface/Struct and recurse for the rest.
            "kClass" | "kInterface" | "kRecord" if pending_name_for_type.is_some() => {
                let sym_kind = match child.kind() {
                    "kClass"     => SymbolKind::Class,
                    "kInterface" => SymbolKind::Interface,
                    _            => SymbolKind::Struct,
                };
                if let Some(name_node) = pending_name_for_type.take() {
                    let name = node_text(name_node, src);
                    if !name.is_empty() {
                        let new_idx = symbols.len();
                        symbols.push(make_symbol(
                            name.clone(),
                            name,
                            sym_kind,
                            child,
                            None,
                            parent_index,
                        ));
                        // Remaining children (from the type keyword onwards)
                        // belong to this new declaration.
                        recover_type_decls_from_siblings(
                            &children[ci..],
                            src,
                            symbols,
                            refs,
                            Some(new_idx),
                        );
                    }
                }
                return; // rest of children consumed by recover_type_decls_from_siblings
            }
            // `typeref` may be the name of a new type declaration when followed
            // by `defaultValue` (e.g. `TSFBool = class(TX3DSingleField)`).
            // In that case, intercept it here instead of dispatching as a ref.
            "typeref" if ci + 1 < children.len() && children[ci + 1].kind() == "defaultValue" => {
                if let Some(sym_kind) = infer_type_kind_from_default_value(children[ci + 1], src) {
                    let mut tc = child.walk();
                    let tc_ch: Vec<Node> = child.children(&mut tc).collect();
                    if let Some(name_node) = tc_ch.iter().find(|n| n.kind() == "identifier") {
                        let name = node_text(*name_node, src);
                        if !name.is_empty() {
                            let new_idx = symbols.len();
                            symbols.push(make_symbol(
                                name.clone(),
                                name,
                                sym_kind,
                                child,
                                None,
                                parent_index,
                            ));
                            // Remaining children from [ci+2] belong to this new declaration.
                            recover_type_decls_from_siblings(
                                &children[ci + 2..],
                                src,
                                symbols,
                                refs,
                                Some(new_idx),
                            );
                            return;
                        }
                    }
                }
                // Fallback: dispatch as a normal typeref.
                pending_proc_kw = false;
                pending_name_for_type = None;
                dispatch(*child, src, symbols, refs, parent_index);
            }
            // Structured children (including ERROR): dispatch normally.
            // ERROR nodes inside a type body may contain embedded type
            // declarations (e.g. class-of metaclass bodies that fold in the
            // following type declaration via error recovery).
            "declProc" | "defProc" | "declSection" | "declVars" | "declConsts"
            | "declUses" | "exprCall" | "typeref" | "ERROR" => {
                pending_proc_kw = false;
                pending_name_for_type = None;
                dispatch(*child, src, symbols, refs, parent_index);
            }
            "kProcedure" | "kFunction" | "kConstructor" | "kDestructor" | "kOperator" => {
                pending_proc_kw = true;
                pending_name_for_type = None;
            }
            "kEq" => {
                // `=` — if the previous child was an identifier, track it as
                // a potential type name (for `Name = class/record/interface`).
                if ci > 0 && children[ci - 1].kind() == "identifier" {
                    pending_name_for_type = Some(children[ci - 1]);
                } else {
                    pending_name_for_type = None;
                }
                pending_proc_kw = false;
            }
            "identifier" if pending_proc_kw => {
                // Bare `kProcedure identifier ;` inside the error-recovery body.
                let name = node_text(*child, src);
                if !name.is_empty() && name != "end" {
                    symbols.push(make_symbol(
                        name.clone(),
                        name,
                        SymbolKind::Function,
                        child,
                        None,
                        parent_index,
                    ));
                }
                pending_proc_kw = false;
                pending_name_for_type = None;
            }
            _ => {
                pending_proc_kw = false;
                pending_name_for_type = None;
            }
        }
    }
}

/// Returns the `SymbolKind` that corresponds to the type keyword that begins
/// this node (if any). Used to detect that a `declProc` or `declSection`
/// produced by error-recovery is actually a class/interface/record body.
///
/// The grammar wraps `TSoundAllocator = class ... end;` without the `type`
/// keyword into a `declProc` whose first non-pp child is `kClass`.
fn type_keyword_of_node(node: Node, _src: &str) -> Option<SymbolKind> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let mut iter = children.iter().peekable();
    while let Some(child) = iter.next() {
        match child.kind() {
            "pp" | "comment" => continue,
            "kClass" => {
                // `class function`/`class procedure`/`class constructor`/`class destructor`
                // is a method modifier, not a type body opener.
                let next_is_method = iter.peek()
                    .map(|n| matches!(n.kind(), "kFunction" | "kProcedure" | "kConstructor" | "kDestructor"))
                    .unwrap_or(false);
                if next_is_method {
                    return None;
                }
                return Some(SymbolKind::Class);
            }
            "kInterface" => return Some(SymbolKind::Interface),
            "kRecord" => return Some(SymbolKind::Struct),
            _ => return None,
        }
    }
    None
}

/// Infer the SymbolKind from a `defaultValue` node that represents `= class(...)`.
///
/// Tree-sitter's error recovery for `.inc` fragments sometimes produces:
///
///   `identifier("TypeName")  defaultValue(kEq  exprCall(identifier("class") ...))`
///
/// or the equivalent with "record", "object", "interface" as the first identifier
/// in the `exprCall` child of `defaultValue`.
fn infer_type_kind_from_default_value(node: Node, src: &str) -> Option<SymbolKind> {
    // node must be `defaultValue` or `kEq` — search for an identifier child
    // whose text is a Pascal type keyword.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "kClass"     => return Some(SymbolKind::Class),
            "kInterface" => return Some(SymbolKind::Interface),
            "kRecord"    => return Some(SymbolKind::Struct),
            "identifier" => {
                let text = node_text(child, src).to_ascii_lowercase();
                match text.as_str() {
                    "class" | "object" => return Some(SymbolKind::Class),
                    "interface"        => return Some(SymbolKind::Interface),
                    "record"           => return Some(SymbolKind::Struct),
                    _ => {}
                }
            }
            // Recurse into kEq, exprCall, and ERROR children.
            // ERROR wraps the class/record/interface keyword when tree-sitter's
            // error recovery groups the type opener with the surrounding context.
            "kEq" | "exprCall" | "ERROR" => {
                if let Some(k) = infer_type_kind_from_default_value(child, src) {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Infer the SymbolKind from a `kEq` node or an ERROR child whose first identifier
/// is a Pascal type keyword (for the case where `kEq` appears as a sibling in an
/// ERROR node's children: `ERROR { identifier "Name", kEq { identifier "class" } }`).
fn infer_type_kind_from_eq_sibling(keq_node: Node, src: &str) -> Option<SymbolKind> {
    infer_type_kind_from_default_value(keq_node, src)
}

fn has_keyword_child(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

fn first_line_of(node: Node, src: &str) -> String {
    let text = node_text(node, src);
    text.lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    node: &Node,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;
