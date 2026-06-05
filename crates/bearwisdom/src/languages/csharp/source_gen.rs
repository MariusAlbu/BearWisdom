// =============================================================================
// csharp/source_gen.rs — synthesize the members the C# compiler generates for
// positional records
//
// A positional record `record Point(int X, int Y)` makes the compiler generate
// a `Deconstruct(out int X, out int Y)` that never appears in source text, so a
// ref to `point.Deconstruct(out _, out _)` (or the positional pattern
// `var (a, b) = point`) goes unresolved without synthesis. This recognizer reads
// the already-extracted symbols and emits that Deconstruct, parented to the
// record.
//
// Detection: a record is extracted as `SymbolKind::Class` with signature
// `"class {Name}..."` — indistinguishable from a real class by kind or
// signature. The only discriminator at synthesis time (where we have `source` +
// flat `symbols`, not the CST) is source text: the class header line contains
// the `record` keyword as a whole word before the type name. This mirrors
// Kotlin's `is_data_class`, which scans for the `data` modifier the same way.
//
// Positional properties: the extractor emits each positional parameter AND each
// body property as a `SymbolKind::Property` with `signature: Some("{type}
// {name}")` and `scope_path == record_qname` — identical shape. The compiler
// generates Deconstruct ONLY for positional parameters, so the recognizer keeps
// only the properties whose source position falls inside the record's positional
// parameter list (the parenthesized group right after the type name, before any
// `{`). Body properties (`record Person { string Name {...} }`) sit after the
// `{` and are excluded — a record with no parameter list yields no Deconstruct.
// The recognizer reads these to build Deconstruct's `out` parameter list — it
// does NOT re-emit properties (the chain `dto.Category.Id` rides the property
// signature the extractor already produced).
//
// Deconstruct returns void, so it carries no return-type `TypeRef` — the same
// convention the Lombok/derive/data-class recognizers use for void/primitive
// returns.
// =============================================================================

use crate::languages::Synthesized;
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use std::collections::HashSet;

pub(super) fn synthesize_record_members(
    source: &str,
    symbols: &[ExtractedSymbol],
    _refs: &[ExtractedRef],
) -> Synthesized {
    let lines: Vec<&str> = source.lines().collect();

    let existing: HashSet<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut out_symbols: Vec<ExtractedSymbol> = Vec::new();

    for record_sym in symbols {
        if record_sym.kind != SymbolKind::Class || !is_record(record_sym, &lines) {
            continue;
        }
        let record_qname = record_sym.qualified_name.as_str();

        // The record's positional parameter-list span, or skip when absent
        // (a body-only record `record Person { ... }` generates no Deconstruct).
        let Some(span) = param_list_span(record_sym, &lines) else {
            continue;
        };

        // Positional properties of this record, in declaration order. The
        // extractor walks the parameter list left-to-right, so the symbol order
        // matches the source order. Keep only properties whose start position
        // falls inside the parameter list — body properties share the same kind
        // and scope_path but sit after the `{`.
        let props: Vec<&ExtractedSymbol> = symbols
            .iter()
            .filter(|s| {
                s.kind == SymbolKind::Property
                    && s.scope_path.as_deref() == Some(record_qname)
                    && span.contains(s.start_line, s.start_col, &lines)
            })
            .collect();
        if props.is_empty() {
            continue;
        }

        let qname = format!("{record_qname}.Deconstruct");
        if existing.contains(qname.as_str()) || !emitted.insert(qname.clone()) {
            continue;
        }

        // `void Deconstruct(out T1 P1, out T2 P2, ...)`.
        let params: Vec<String> = props
            .iter()
            .map(|p| format!("out {} {}", property_type(p), p.name))
            .collect();
        let signature = format!("void Deconstruct({})", params.join(", "));
        out_symbols.push(make_synth(
            "Deconstruct",
            SymbolKind::Method,
            signature,
            record_qname,
            record_sym.start_line,
        ));
    }

    Synthesized { symbols: out_symbols, refs: Vec::new() }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when `sym` (a `Class` symbol) is a `record` declaration.
///
/// Records and classes are both extracted as `SymbolKind::Class` with a
/// `"class {name}..."` signature, so the discriminator is source text. The
/// `record_declaration` node spans its attributes/modifiers, so the header may
/// start with `[Attr]` lines before the `record` keyword. Accumulate the header
/// from `start_line` up to and including the line holding the type name, then
/// check for a `record` token before that name. `record` is a contextual
/// keyword, so a whitespace-delimited word match excludes identifiers like
/// `RecordStore`.
fn is_record(sym: &ExtractedSymbol, lines: &[&str]) -> bool {
    let start = sym.start_line as usize;
    let end = (sym.end_line as usize).min(lines.len().saturating_sub(1));
    let name = sym.name.as_str();
    let mut header = String::new();
    for i in start..=end {
        let Some(line) = lines.get(i) else { break };
        header.push_str(line);
        header.push(' ');
        // The type name first appears once we reach the `record X` line.
        if header.contains(name) {
            break;
        }
    }
    // The discriminator is `record` appearing as a whole word before the name.
    let Some(name_pos) = header.find(name) else {
        return false;
    };
    header[..name_pos].split_whitespace().any(|w| w == "record")
}

/// The character span of a record's positional parameter list, as absolute
/// offsets into `source` (newlines counted as one char each).
struct ParamSpan {
    open: usize,  // offset just after the `(`
    close: usize, // offset of the matching `)`
}

impl ParamSpan {
    /// Whether a symbol at `(line, col)` starts inside the parameter list.
    fn contains(&self, line: u32, col: u32, lines: &[&str]) -> bool {
        let off = offset_of(line, col, lines);
        off >= self.open && off < self.close
    }
}

/// Locate the record's positional parameter list: the parenthesized group
/// immediately after the type name, before any body `{`. `None` for a record
/// with no parameter list (`record Person { ... }`). The scan begins at the
/// record name to skip any `(` in attributes/modifiers, and bails at the first
/// `{` so a body initializer can't be mistaken for a parameter list.
fn param_list_span(sym: &ExtractedSymbol, lines: &[&str]) -> Option<ParamSpan> {
    let chars: Vec<char> = lines.join("\n").chars().collect();
    let name_off = find_name_offset(&chars, sym.start_line, lines, sym.name.as_str())?;

    // First `(` after the name, with no intervening `{`.
    let mut i = name_off + sym.name.chars().count();
    let open = loop {
        let c = *chars.get(i)?;
        if c == '{' {
            return None;
        }
        if c == '(' {
            break i;
        }
        i += 1;
    };

    // Matching `)` by paren depth.
    let mut depth = 0usize;
    let mut j = open;
    let close = loop {
        match chars.get(j)? {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break j;
                }
            }
            _ => {}
        }
        j += 1;
    };

    Some(ParamSpan { open: open + 1, close })
}

/// Absolute char offset of `name`'s first occurrence at or after the start of
/// `start_line` in `chars` (which is `lines.join("\n")` as a char vector).
fn find_name_offset(chars: &[char], start_line: u32, lines: &[&str], name: &str) -> Option<usize> {
    let start = line_start_offset(start_line, lines);
    let needle: Vec<char> = name.chars().collect();
    (start..=chars.len().saturating_sub(needle.len()))
        .find(|&i| chars[i..i + needle.len()] == needle[..])
}

/// Absolute char offset of the start of `line` in `lines.join("\n")`.
fn line_start_offset(line: u32, lines: &[&str]) -> usize {
    lines
        .iter()
        .take(line as usize)
        .map(|l| l.chars().count() + 1)
        .sum()
}

/// Absolute char offset of `(line, col)`.
fn offset_of(line: u32, col: u32, lines: &[&str]) -> usize {
    line_start_offset(line, lines) + col as usize
}

/// A positional property's `signature` is `"{type} {name}"`; strip the trailing
/// ` {name}` to recover the declared type. `object` when absent/malformed (an
/// untyped `out` param still resolves as a member).
fn property_type(prop: &ExtractedSymbol) -> String {
    let Some(sig) = prop.signature.as_deref() else {
        return "object".to_string();
    };
    let suffix = format!(" {}", prop.name);
    let ty = sig.strip_suffix(&suffix).unwrap_or(sig).trim();
    if ty.is_empty() {
        "object".to_string()
    } else {
        ty.to_string()
    }
}

/// Build a synthesized member named `name` under `scope_qname`.
fn make_synth(
    name: &str,
    kind: SymbolKind,
    signature: String,
    scope_qname: &str,
    line: u32,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("{scope_qname}.{name}"),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature: Some(signature),
        doc_comment: None,
        scope_path: Some(scope_qname.to_string()),
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Test exposure
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(super) fn _test_synthesize(source: &str) -> Synthesized {
    let r = super::extract::extract(source);
    synthesize_record_members(source, &r.symbols, &r.refs)
}
