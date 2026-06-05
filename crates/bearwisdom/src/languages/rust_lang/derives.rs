// =============================================================================
// rust_lang/derives.rs — synthesize the impls a #[derive(...)] generates
//
// `#[derive(Default, Clone, ...)]` makes the compiler generate trait impls
// whose methods never appear in source text, so `Config::default()`,
// `user.clone()`, `Wrapper::from(x)` go unresolved without synthesis. This
// recognizer reads the already-extracted struct/enum symbols plus the derive
// TypeRefs (the extractor emits each derive trait name as a `TypeRef` sourced
// at the struct/enum symbol) and emits those methods, parented to the owning
// type.
//
// Derive intent from refs: a derive trait name (`Default`, `Clone`, `From`,
// `serde::Serialize`, ...) arrives as a `TypeRef` whose `source_symbol_index`
// is a `Struct`/`Enum` symbol. A field's own type also arrives as a `TypeRef`
// sourced at the struct (the extractor attributes field types to the enclosing
// struct), but the derive-name map is the discriminator: a field is never
// typed by a derive trait name, so a non-derive `TypeRef` simply doesn't match
// the table and is ignored — the same shape as Lombok's `accessors_for`
// returning `None` for a non-Lombok name.
//
// Self-returning methods (`Default::default`, `Clone::clone`, `From::from`)
// carry a return-type `TypeRef` to the owning type's qname, so the chain walker
// types `Config::default().name` through to `Config`'s members. Methods whose
// return is not the type itself (`Debug::fmt`→Result, `PartialEq::eq`→bool,
// `Hash::hash`→(), `Ord::cmp`→Ordering) emit no ref — there is no chain value.
// =============================================================================

use crate::languages::Synthesized;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use std::collections::HashSet;

/// One synthesized member: its name, symbol kind, signature, and whether its
/// return is the owning type itself (Self) — which drives the return-type ref.
struct DeriveMethod {
    name: &'static str,
    kind: SymbolKind,
    /// `{ret}` placeholder is replaced with the owning type's simple name.
    signature: &'static str,
    returns_self: bool,
}

/// Map a bare derive trait name to the methods it generates. `&[]` for a
/// marker trait (`Copy`, `Eq`) or any derive that synthesizes nothing here.
///
/// These are Rust language-level trait names, not a library API list — the
/// derive→method shape is fixed by the standard library's derivable traits.
fn methods_for(derive: &str) -> &'static [DeriveMethod] {
    match derive {
        "Clone" => &[DeriveMethod { name: "clone", kind: SymbolKind::Method, signature: "fn clone(&self) -> {ret}", returns_self: true }],
        "Default" => &[DeriveMethod { name: "default", kind: SymbolKind::Function, signature: "fn default() -> {ret}", returns_self: true }],
        "From" => &[DeriveMethod { name: "from", kind: SymbolKind::Function, signature: "fn from(value: T) -> {ret}", returns_self: true }],
        "Into" => &[DeriveMethod { name: "into", kind: SymbolKind::Method, signature: "fn into(self) -> T", returns_self: false }],
        "Debug" | "Display" => &[DeriveMethod { name: "fmt", kind: SymbolKind::Method, signature: "fn fmt(&self, f: &mut Formatter) -> Result", returns_self: false }],
        "PartialEq" => &[
            DeriveMethod { name: "eq", kind: SymbolKind::Method, signature: "fn eq(&self, other: &{ret}) -> bool", returns_self: false },
            DeriveMethod { name: "ne", kind: SymbolKind::Method, signature: "fn ne(&self, other: &{ret}) -> bool", returns_self: false },
        ],
        "PartialOrd" => &[DeriveMethod { name: "partial_cmp", kind: SymbolKind::Method, signature: "fn partial_cmp(&self, other: &{ret}) -> Option", returns_self: false }],
        "Ord" => &[DeriveMethod { name: "cmp", kind: SymbolKind::Method, signature: "fn cmp(&self, other: &{ret}) -> Ordering", returns_self: false }],
        "Hash" => &[DeriveMethod { name: "hash", kind: SymbolKind::Method, signature: "fn hash(&self, state: &mut H)", returns_self: false }],
        "Serialize" => &[DeriveMethod { name: "serialize", kind: SymbolKind::Method, signature: "fn serialize(&self, serializer: S) -> Result", returns_self: false }],
        "Deserialize" => &[DeriveMethod { name: "deserialize", kind: SymbolKind::Function, signature: "fn deserialize(deserializer: D) -> Result", returns_self: false }],
        "AsRef" => &[DeriveMethod { name: "as_ref", kind: SymbolKind::Method, signature: "fn as_ref(&self) -> &T", returns_self: false }],
        "AsMut" => &[DeriveMethod { name: "as_mut", kind: SymbolKind::Method, signature: "fn as_mut(&mut self) -> &mut T", returns_self: false }],
        "Error" => &[
            DeriveMethod { name: "source", kind: SymbolKind::Method, signature: "fn source(&self) -> Option", returns_self: false },
            DeriveMethod { name: "description", kind: SymbolKind::Method, signature: "fn description(&self) -> &str", returns_self: false },
        ],
        _ => &[],
    }
}

pub(super) fn synthesize_derive_members(
    _source: &str,
    symbols: &[ExtractedSymbol],
    refs: &[ExtractedRef],
) -> Synthesized {
    // 1. Collect derive intent: which traits each struct/enum derives. A derive
    //    trait name arrives as a TypeRef sourced at the type symbol.
    let mut derives_by_type: Vec<(usize, &str)> = Vec::new(); // (type symbol index, bare derive)
    for r in refs {
        if r.kind != EdgeKind::TypeRef {
            continue;
        }
        let Some(sym) = symbols.get(r.source_symbol_index) else {
            continue;
        };
        if !is_type_decl(sym.kind) {
            continue;
        }
        let bare = r.target_name.rsplit("::").next().unwrap_or(r.target_name.as_str());
        if methods_for(bare).is_empty() {
            continue;
        }
        derives_by_type.push((r.source_symbol_index, bare));
    }

    if derives_by_type.is_empty() {
        return Synthesized::default();
    }

    // 2. For each derived trait on each type, emit its methods. A hand-written
    //    member of the same qname always wins; a Self-returning method carries
    //    a return-type ref to the owning type so a chain types through it.
    let existing: HashSet<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    let mut emitted: HashSet<String> = HashSet::new();
    let mut out_symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut out_refs: Vec<ExtractedRef> = Vec::new();

    for (type_idx, derive) in derives_by_type {
        let type_sym = &symbols[type_idx];
        let type_qname = type_sym.qualified_name.as_str();
        let type_simple = type_sym.name.as_str();
        let line = type_sym.start_line;

        for m in methods_for(derive) {
            let qname = format!("{type_qname}.{}", m.name);
            if existing.contains(qname.as_str()) || !emitted.insert(qname.clone()) {
                continue;
            }
            let signature = m.signature.replace("{ret}", type_simple);
            let sym = make_synth(m.name, m.kind, signature, type_qname, line);
            let idx = out_symbols.len();
            out_symbols.push(sym);
            if m.returns_self {
                out_refs.push(return_type_ref(idx, type_qname, line));
            }
        }
    }

    Synthesized { symbols: out_symbols, refs: out_refs }
}

fn is_type_decl(kind: SymbolKind) -> bool {
    matches!(kind, SymbolKind::Struct | SymbolKind::Enum)
}

/// Build a synthesized member named `name` under `scope_qname`.
fn make_synth(name: &str, kind: SymbolKind, signature: String, scope_qname: &str, line: u32) -> ExtractedSymbol {
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
/// synthesized symbol list — `parse_file` rebases it onto the file table).
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
    synthesize_derive_members(source, &r.symbols, &r.refs)
}
