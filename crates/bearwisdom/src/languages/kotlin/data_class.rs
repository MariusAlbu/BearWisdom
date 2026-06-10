// =============================================================================
// kotlin/data_class.rs — synthesize the members Kotlin generates for data classes
//
// A `data class` has the compiler auto-generate: `copy(...)` returning the
// class type (the headline — enables chain typing), `componentN()` for each
// primary-constructor `val`/`var` property returning that property's declared
// type (enables destructuring), and the structural trio `equals`, `hashCode`,
// `toString`.  None of these appear in source text, so calls to them are
// unresolved without synthesis.
//
// Detection: `data` is a modifier inside the `modifiers` child of the
// `class_declaration` node.  The extractor does not carry the modifier forward
// into the symbol's `signature` (which is built as `"class {name}"`) or any
// other field — so the only reliable signal available at synthesis time (where
// we have `source` + flat `symbols`/`refs`, not the CST) is to locate the
// class's start line in `source` and check whether that line contains the
// `data` modifier before the `class` keyword.  See the `is_data_class` helper.
//
// Property discriminator: `extract_class_parameter` (symbols.rs) emits
// `SymbolKind::Property` for `val`/`var` primary-ctor params and
// `SymbolKind::Variable` for plain params.  Filtering `scope_path ==
// class_qname && kind == Property` recovers exactly the promoted properties
// in declaration order (the extractor walks params left-to-right).
//
// Property type: primary-ctor params are emitted with `signature: None`.
// The declared type is recovered by scanning the source line for
// `val {name}:` / `var {name}:` and extracting the text between `:` and the
// first `,`, `)`, or `=`.
// =============================================================================

use crate::languages::Synthesized;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use std::collections::HashSet;

pub(super) fn synthesize_data_class_members(
    source: &str,
    symbols: &[ExtractedSymbol],
    _refs: &[ExtractedRef],
) -> Synthesized {
    let lines: Vec<&str> = source.lines().collect();

    let mut out_symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut out_refs: Vec<ExtractedRef> = Vec::new();

    // Collect the qnames already present so hand-written members always win.
    let existing: HashSet<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    // Track synthesized qnames to avoid duplicates within this pass.
    let mut emitted: HashSet<String> = HashSet::new();

    // Find every data class and synthesize its members.
    for class_sym in symbols {
        if class_sym.kind != SymbolKind::Class {
            continue;
        }
        if !is_data_class(class_sym, &lines) {
            continue;
        }

        let class_qname = class_sym.qualified_name.as_str();
        let class_simple = class_sym.name.as_str();
        let line = class_sym.start_line;

        // Collect the val/var primary-ctor properties in declaration order.
        // `extract_class_parameter` emits Property kind for val/var params and
        // leaves `signature: None` (it does not build a signature for
        // `class_parameter` nodes).  Body property declarations go through
        // `push_property_decl` which always sets a signature.  The combination
        // of `kind == Property`, `scope_path == class_qname`, and
        // `signature.is_none()` isolates primary-ctor promoted properties only.
        let properties: Vec<&ExtractedSymbol> = symbols
            .iter()
            .filter(|s| {
                s.kind == SymbolKind::Property
                    && s.scope_path.as_deref() == Some(class_qname)
                    && s.signature.is_none()
            })
            .collect();

        // `copy(...)` — returns the class itself; the return-type ref makes
        // `u.copy().name` type through to the class's members.
        let copy_qname = format!("{class_qname}.copy");
        if !existing.contains(copy_qname.as_str()) && emitted.insert(copy_qname.clone()) {
            let copy_sig = format!("fun copy(...): {class_simple}");
            let copy_sym = make_synth("copy", SymbolKind::Method, copy_sig, class_qname, line);
            let copy_idx = out_symbols.len();
            out_symbols.push(copy_sym);
            // Return-type ref → the class qname so the chain walker resolves
            // the result of copy() as an instance of the class.
            out_refs.push(return_type_ref(copy_idx, class_qname, line));
        }

        // `componentN()` — one per val/var primary-ctor property, in order.
        for (n, prop) in properties.iter().enumerate() {
            let method_name = format!("component{}", n + 1);
            let comp_qname = format!("{class_qname}.{method_name}");
            if existing.contains(comp_qname.as_str()) || !emitted.insert(comp_qname.clone()) {
                continue;
            }

            let prop_type = property_type_from_source(prop, &lines);
            let ret_ty = if prop_type.is_empty() {
                "Any".to_string()
            } else {
                prop_type.clone()
            };
            let sig = format!("fun {method_name}(): {ret_ty}");
            let comp_sym = make_synth(
                &method_name,
                SymbolKind::Method,
                sig,
                class_qname,
                prop.start_line,
            );
            let comp_idx = out_symbols.len();
            out_symbols.push(comp_sym);

            // Emit a return-type ref for non-primitive types so chains through
            // a componentN() result can bind to the property's type members.
            let head = type_head(&ret_ty);
            if !head.is_empty() && !is_kotlin_primitive(head) {
                out_refs.push(return_type_ref(comp_idx, head, prop.start_line));
            }
        }

        // `equals`, `hashCode`, `toString` — no return-type refs; their
        // returns are Boolean/Int/String (scalar stdlib types with no chain
        // value).  Emitting them as members lets calls like `.toString()`
        // resolve to a class member rather than going unresolved.
        let structural: [(&str, &str); 3] = [
            ("equals", "fun equals(other: Any?): Boolean"),
            ("hashCode", "fun hashCode(): Int"),
            ("toString", "fun toString(): String"),
        ];
        for (name, sig) in structural {
            let qname = format!("{class_qname}.{name}");
            if !existing.contains(qname.as_str()) && emitted.insert(qname.clone()) {
                out_symbols.push(make_synth(
                    name,
                    SymbolKind::Method,
                    sig.to_string(),
                    class_qname,
                    line,
                ));
            }
        }
    }

    Synthesized {
        symbols: out_symbols,
        refs: out_refs,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when `sym` (a `Class` symbol) is declared with the `data`
/// modifier.
///
/// The extractor builds the class signature as `"class {name}"` — the `data`
/// modifier is not reflected in the signature, so the discriminator is source
/// text. The `class_declaration` node spans its `modifiers` child, so its first
/// line is the first annotation/modifier, not necessarily the `class` keyword
/// (`@Serializable\ndata class User`). Accumulate the header from `start_line`
/// up to and including the line that holds the `class` keyword, then check for a
/// `data` token before it. `data` is a Kotlin modifier keyword, so a
/// whitespace-delimited word match excludes identifiers like `database`.
fn is_data_class(sym: &ExtractedSymbol, lines: &[&str]) -> bool {
    let start = sym.start_line as usize;
    let end = (sym.end_line as usize).min(lines.len().saturating_sub(1));
    let mut header = String::new();
    for i in start..=end {
        let Some(line) = lines.get(i) else { break };
        header.push_str(line);
        header.push(' ');
        if line.contains("class ") || line.contains("class(") {
            break;
        }
    }
    let Some(class_pos) = header.find("class") else {
        return false;
    };
    header[..class_pos].split_whitespace().any(|w| w == "data")
}

/// Extract the declared type of a primary-constructor property from the
/// source line.
///
/// Primary-ctor property symbols carry `signature: None` (the extractor does
/// not build a signature for class_parameter nodes). The type is recovered by
/// scanning the source from the property's own `start_col` (the `class_parameter`
/// node start) so an earlier param whose name is a prefix of this one
/// (`val myname` before `val name`) can't be matched first. After the name, the
/// declared type is the text between the next `:` and the first `,`, `)`, or `=`,
/// minus a trailing nullable `?`.
fn property_type_from_source(prop: &ExtractedSymbol, lines: &[&str]) -> String {
    let line = match lines.get(prop.start_line as usize) {
        Some(l) => *l,
        None => return String::new(),
    };
    // Anchor at the param's column; fall back to the whole line if the column
    // isn't a char boundary.
    let start = (prop.start_col as usize).min(line.len());
    let slice = line.get(start..).unwrap_or(line);

    // Locate the name, then the `:` that follows it (handles `name:` and the
    // legal `name :` spacing).
    let Some(name_pos) = slice.find(prop.name.as_str()) else {
        return String::new();
    };
    let after_name = &slice[name_pos + prop.name.len()..];
    let Some(colon_rel) = after_name.find(':') else {
        return String::new();
    };
    let after_colon = after_name[colon_rel + 1..].trim_start();

    // Take until the first terminator: `,`, `)`, `=`, or end of string.
    let end = after_colon
        .find([',', ')', '='])
        .unwrap_or(after_colon.len());
    let raw = after_colon[..end].trim();
    raw.trim_end_matches('?').trim().to_string()
}

/// The bare head type: strips generic args and array brackets.
/// `List<User>` → `List`, `Array<Int>` → `Array`.
fn type_head(t: &str) -> &str {
    let t = t.split('<').next().unwrap_or(t);
    let t = t.split('[').next().unwrap_or(t);
    t.trim()
}

/// Kotlin primitive and stdlib scalar types that produce no useful chain
/// member — emitting a return-type ref for these only creates unresolved refs.
fn is_kotlin_primitive(t: &str) -> bool {
    matches!(
        t,
        "Boolean"
            | "Byte"
            | "Short"
            | "Int"
            | "Long"
            | "Float"
            | "Double"
            | "Char"
            | "String"
            | "Unit"
            | "Nothing"
            | "Any"
            | "boolean"
            | "byte"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "char"
            | "void"
    )
}

/// Build a synthesized method symbol parented to `scope_qname`.
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

/// A return-type `TypeRef` sourced at `source_symbol_index` (relative to the
/// synthesized symbol list — `parse_file` rebases onto the file table).
fn return_type_ref(source_symbol_index: usize, type_name: &str, line: u32) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index,
        target_name: type_name.to_string(),
        kind: EdgeKind::TypeRef,
        line,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Test exposure
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(super) fn _test_synthesize(source: &str) -> Synthesized {
    let r = super::extract::extract(source);
    synthesize_data_class_members(source, &r.symbols, &r.refs)
}
