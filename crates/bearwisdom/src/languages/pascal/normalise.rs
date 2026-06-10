// =============================================================================
// languages/pascal/normalise.rs — source preprocessing for Pascal extractor
//
// Tree-sitter-pascal misparses certain idioms; this pass rewrites the source
// before the grammar runs. Covers variant record end-paren normalisation,
// IFDEF-wrapped type keyword recovery, and explicit-generic specialisation.
// =============================================================================

pub(crate) fn normalise_source_for_test(src: &str) -> String {
    normalise_source(src)
}

pub(super) fn normalise_source(src: &str) -> String {
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
            while j < n && lines[j].trim().is_empty() {
                j += 1;
            }

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
                        if tl.starts_with("record") || tl.starts_with("begin") {
                            depth += 1;
                        }
                        if tl == "end;" || tl == "end" {
                            depth -= 1;
                        }
                        if depth == 0 {
                            break;
                        }
                        k += 1;
                    }
                    // k now points to the `end;` line of the anonymous record.
                    // Skip past blank lines and the closing `)` or `);`.
                    let mut m = k + 1;
                    while m < n && lines[m].trim().is_empty() {
                        m += 1;
                    }
                    if m < n && (lines[m].trim() == ");" || lines[m].trim() == ")") {
                        // Replacement: keep the case arm prefix up to `(`, replace body.
                        let prefix_end = line.rfind('(').unwrap_or(line.len());
                        let prefix = &line[..prefix_end];
                        // Preserve the field name from the original (not lowercased).
                        let orig_inner = lines[j].trim();
                        let orig_field = if let Some(pos) = orig_inner
                            .find(": record")
                            .or_else(|| orig_inner.find(": RECORD"))
                            .or_else(|| orig_inner.find(":record"))
                        {
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
            None => {
                i += 1;
                continue;
            }
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
        while j < src_len
            && (out[j] == b' ' || out[j] == b'\t' || out[j] == b'\r' || out[j] == b'\n')
        {
            j += 1;
        }

        // Check if a type keyword starts here.
        let kw1 = TYPE_KWS.iter().find(|&&kw| {
            out[j..].starts_with(kw.as_bytes())
                && (j + kw.len() >= src_len || !out[j + kw.len()].is_ascii_alphanumeric())
        });

        let kw1 = match kw1 {
            Some(k) => k,
            None => {
                i = pp_end;
                continue;
            }
        };

        let kw1_end = j + kw1.len();

        // After kw1, consume whitespace, then expect `{$else}` or `{$ifend}`/`{$endif}`.
        let mut k = kw1_end;
        while k < src_len
            && (out[k] == b' ' || out[k] == b'\t' || out[k] == b'\r' || out[k] == b'\n')
        {
            k += 1;
        }

        if k >= src_len || out[k] != b'{' {
            i = pp_end;
            continue;
        }
        let pp2_start = k;
        let pp2_end = match out[k..].iter().position(|&b| b == b'}') {
            Some(p) => k + p + 1,
            None => {
                i = pp_end;
                continue;
            }
        };
        let pp2_inner = std::str::from_utf8(&out[pp2_start + 2..pp2_end - 1])
            .unwrap_or("")
            .trim_start()
            .to_ascii_lowercase();

        // Case 1: `{$else}` — has an else branch
        if pp2_inner.starts_with("else") {
            let mut m = pp2_end;
            while m < src_len
                && (out[m] == b' ' || out[m] == b'\t' || out[m] == b'\r' || out[m] == b'\n')
            {
                m += 1;
            }
            let kw2 = TYPE_KWS.iter().find(|&&kw| {
                out[m..].starts_with(kw.as_bytes())
                    && (m + kw.len() >= src_len || !out[m + kw.len()].is_ascii_alphanumeric())
            });
            let kw2 = match kw2 {
                Some(k) => k,
                None => {
                    i = pp_end;
                    continue;
                }
            };
            let kw2_end = m + kw2.len();

            // After kw2, expect `{$endif}` or `{$ifend}`
            let mut n = kw2_end;
            while n < src_len
                && (out[n] == b' ' || out[n] == b'\t' || out[n] == b'\r' || out[n] == b'\n')
            {
                n += 1;
            }
            if n >= src_len || out[n] != b'{' {
                i = pp_end;
                continue;
            }
            let pp3_end = match out[n..].iter().position(|&b| b == b'}') {
                Some(p) => n + p + 1,
                None => {
                    i = pp_end;
                    continue;
                }
            };
            let pp3_inner = std::str::from_utf8(&out[n + 2..pp3_end - 1])
                .unwrap_or("")
                .trim_start()
                .to_ascii_lowercase();
            if !pp3_inner.starts_with("endif") && !pp3_inner.starts_with("ifend") {
                i = pp_end;
                continue;
            }

            // Replace the span [pp_start..pp3_end] with kw2 + spaces.
            let span_len = pp3_end - pp_start;
            let replacement: Vec<u8> = {
                let mut v: Vec<u8> = kw2.bytes().collect();
                while v.len() < span_len {
                    v.push(b' ');
                }
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
                while v.len() < span_len {
                    v.push(b' ');
                }
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
                None => {
                    i += 1;
                    continue;
                }
            };
            // Check that this is an ifdef/ifndef opener.
            let pp1_inner = std::str::from_utf8(&out[i + 2..pp1_end - 1])
                .unwrap_or("")
                .trim_start()
                .to_ascii_lowercase();
            if !pp1_inner.starts_with("ifdef")
                && !pp1_inner.starts_with("ifndef")
                && !pp1_inner.starts_with("if ")
            {
                i = pp1_end;
                continue;
            }
            // Skip whitespace after the opener.
            let mut j = pp1_end;
            while j < len && matches!(out[j], b' ' | b'\t' | b'\r' | b'\n') {
                j += 1;
            }
            // Check for `specialize` keyword.
            if !out[j..].starts_with(b"specialize") {
                i = pp1_end;
                continue;
            }
            let spec_end = j + b"specialize".len();
            // Skip whitespace after `specialize`.
            let mut k = spec_end;
            while k < len && matches!(out[k], b' ' | b'\t' | b'\r' | b'\n') {
                k += 1;
            }
            // Expect `{$endif}` or `{$ifend}`.
            if k >= len || out[k] != b'{' {
                i = pp1_end;
                continue;
            }
            let pp2_end = match out[k..].iter().position(|&b| b == b'}') {
                Some(p) => k + p + 1,
                None => {
                    i = pp1_end;
                    continue;
                }
            };
            let pp2_inner = std::str::from_utf8(&out[k + 2..pp2_end - 1])
                .unwrap_or("")
                .trim_start()
                .to_ascii_lowercase();
            if !pp2_inner.starts_with("endif") && !pp2_inner.starts_with("ifend") {
                i = pp1_end;
                continue;
            }
            // Replace the entire `{$ifdef...}specialize{$endif}` span with spaces.
            for idx in i..pp2_end {
                if out[idx] != b'\n' && out[idx] != b'\r' {
                    out[idx] = b' ';
                }
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
                let next_ident = ((i + 1)..len)
                    .find_map(|k| {
                        let c = bytes[k];
                        if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
                            None
                        } else if c.is_ascii_uppercase() || c == b'_' {
                            Some(true)
                        } else {
                            Some(false)
                        }
                    })
                    .unwrap_or(false);

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
                        let closes_paren = ((j)..len)
                            .find_map(|k| {
                                let c = bytes[k];
                                if matches!(c, b' ' | b'\t' | b'\r' | b'\n') {
                                    None
                                } else if c == b')' {
                                    Some(true)
                                } else {
                                    Some(false)
                                }
                            })
                            .unwrap_or(false);
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
