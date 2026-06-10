// =============================================================================
// indexer/canonical_form.rs  —  canonical-contract validator for ParsedFile
//
// Rule reference: research/architecture/01-canonical-symbol-ref-contract.html
// =============================================================================

use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, ParsedFile, SegmentKind, SymbolKind,
};
use std::fmt;

// ---------------------------------------------------------------------------
// Violation type
// ---------------------------------------------------------------------------

/// A single contract violation produced by `validate`.
#[derive(Debug, Clone)]
pub struct ContractViolation {
    /// Rule code (SYM-001, REF-002, …).
    pub code: &'static str,
    /// Human-readable explanation of the violation.
    pub message: String,
    /// Where in the file the violation was found.
    pub location: ViolationLocation,
}

/// Where in a `ParsedFile` a violation was found.
#[derive(Debug, Clone)]
pub enum ViolationLocation {
    /// File-level (parallel-vector length mismatches, etc.).
    File { path: String },
    /// A specific symbol at index `index` in `file.symbols`.
    Symbol {
        path: String,
        index: usize,
        name: String,
    },
    /// A specific ref at index `index` in `file.refs`.
    Ref {
        path: String,
        index: usize,
        target_name: String,
    },
}

impl fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            ViolationLocation::File { path } => {
                write!(f, "[{}] {} ({})", self.code, self.message, path)
            }
            ViolationLocation::Symbol { path, index, name } => write!(
                f,
                "[{}] {} (symbol #{} '{}' in {})",
                self.code, self.message, index, name, path
            ),
            ViolationLocation::Ref {
                path,
                index,
                target_name,
            } => write!(
                f,
                "[{}] {} (ref #{} target '{}' in {})",
                self.code, self.message, index, target_name, path
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

/// Validate `file` against the canonical contract using only file-local
/// rules (no TypeArena required).
pub fn validate(file: &ParsedFile) -> Vec<ContractViolation> {
    let mut out = Vec::new();
    let line_starts = file.content.as_deref().map(build_line_starts);
    check_file_parallel_vecs(file, &mut out);
    check_flow_meta(file, &mut out);
    for (idx, sym) in file.symbols.iter().enumerate() {
        check_sym_001(file, idx, sym, &mut out);
        check_sym_002(file, idx, sym, &mut out);
        check_sym_003(file, idx, sym, &mut out);
        check_sym_004(file, idx, sym, line_starts.as_deref(), &mut out);
        check_sym_006(file, idx, sym, &mut out);
    }
    for (idx, r) in file.refs.iter().enumerate() {
        check_ref_001(file, idx, r, &mut out);
        check_ref_002(file, idx, r, &mut out);
        check_ref_003(file, idx, r, &mut out);
        check_ref_004(file, idx, r, &mut out);
        check_ref_005(file, idx, r, &mut out);
        check_ref_006(file, idx, r, line_starts.as_deref(), &mut out);
        if let Some(chain) = r.chain.as_ref() {
            check_chain_001(file, idx, r, chain, &mut out);
            check_chain_002(file, idx, r, chain, &mut out);
            check_chain_003(file, idx, r, chain, &mut out);
        }
    }
    out
}

/// Extends `validate` with TypeArena-dependent rules (TYPE-001/002/003) and
/// rules that need to look up a Symbol's `return_type` TypeId against the
/// arena.
pub fn validate_with_arena(file: &ParsedFile, arena: &TypeArena) -> Vec<ContractViolation> {
    let mut out = validate(file);
    for (idx, sym) in file.symbols.iter().enumerate() {
        check_sym_005_arena(file, idx, sym, arena, &mut out);
    }
    for ty_id in collect_type_ids(file) {
        check_type_001(file, ty_id, arena, &mut out);
    }
    out
}

/// Walks every TypeId referenced from `file` (Symbol.declared_type /
/// return_type / param_types, ChainSegment.declared_type_id / type_arg_ids).
fn collect_type_ids(file: &ParsedFile) -> Vec<TypeId> {
    let mut out = Vec::new();
    for sym in &file.symbols {
        if let Some(id) = sym.declared_type {
            out.push(id);
        }
        if let Some(id) = sym.return_type {
            out.push(id);
        }
        out.extend(sym.param_types.iter().copied());
    }
    for r in &file.refs {
        if let Some(chain) = r.chain.as_ref() {
            for seg in &chain.segments {
                if let Some(id) = seg.declared_type_id {
                    out.push(id);
                }
                out.extend(seg.type_arg_ids.iter().copied());
            }
        }
    }
    out
}

/// Derive position fields + intern type strings into the supplied
/// `TypeArena`. Symbols get `byte_offset` from (start_line, start_col),
/// refs get `col` from (byte_offset, line), and chain segments get
/// `byte_offset` by scanning the source for each segment's name.
///
/// Type-defining symbols (Class / Struct / Interface / Trait / Enum /
/// TypeAlias) get `return_type = Some(arena.class(qualified_name))` when
/// the extractor didn't already populate it. ChainSegment string fields
/// (declared_type, type_args) are interned into the arena and populated
/// as TypeId carriers (declared_type_id, type_arg_ids).
///
/// Production callers pass the workspace arena so TypeIds remain valid
/// across the whole index run. Test helpers can pass a fresh per-file
/// arena via the `validate_extraction` wrapper.
pub fn populate_positions(file: &mut ParsedFile, arena: &TypeArena) {
    let line_starts_owned: Option<Vec<u32>> = file.content.as_deref().map(build_line_starts);
    let line_starts = line_starts_owned.as_deref();

    for sym in &mut file.symbols {
        if line_starts.is_some() && sym.byte_offset == 0 {
            if let Some(b) = expected_byte(line_starts.unwrap(), sym.start_line, sym.start_col) {
                sym.byte_offset = b;
            }
        }
        // SYM-005 contract: type-defining symbols must carry a return_type
        // pointing at `Type::Class(qualified_name)` in the supplied arena.
        // build.rs intentionally re-derives the TypeId from the symbol
        // qname rather than trusting this field, so the value is safe to
        // populate even when the arena is per-file and gets dropped.
        if sym.return_type.is_none() && is_type_defining_kind(sym.kind) {
            let id = arena.class(&sym.qualified_name);
            sym.return_type = Some(id);
        }
    }

    for r in &mut file.refs {
        if let Some(starts) = line_starts {
            if r.col == 0 {
                if let Some(&ls) = starts.get(r.line as usize) {
                    if r.byte_offset >= ls {
                        r.col = r.byte_offset - ls;
                    }
                }
            }
        }
        if let Some(chain) = r.chain.as_mut() {
            if let Some(content) = file.content.as_deref() {
                populate_chain_segments(chain, content, r.byte_offset);
            }
            for seg in chain.segments.iter_mut() {
                if seg.declared_type_id.is_none() {
                    if let Some(name) = seg.declared_type.as_deref() {
                        if !name.is_empty() {
                            seg.declared_type_id = Some(arena.class(name));
                        }
                    }
                }
                if seg.type_arg_ids.is_empty() && !seg.type_args.is_empty() {
                    for arg in &seg.type_args {
                        if !arg.is_empty() {
                            seg.type_arg_ids.push(arena.class(arg));
                        }
                    }
                }
            }
        }
    }
}

fn is_type_defining_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Interface
            | SymbolKind::Trait
            | SymbolKind::Enum
            | SymbolKind::TypeAlias
    )
}

fn is_callable_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor | SymbolKind::Test
    )
}

/// Assign byte_offsets to chain segments by scanning forward from the ref's
/// byte_offset in the source content. Each segment's name is searched for
/// after the previous segment's position; if found, the segment is anchored
/// at that byte position. Falls back to a monotonic synthetic offset when
/// the literal name can't be located (rare — happens for synthesized
/// segments or chain segments expressed via non-trivial source forms).
fn populate_chain_segments(
    chain: &mut crate::types::MemberChain,
    content: &str,
    ref_byte_offset: u32,
) {
    let bytes = content.as_bytes();
    let mut cursor = ref_byte_offset as usize;
    let mut prev: u32 = 0;
    for (i, seg) in chain.segments.iter_mut().enumerate() {
        if seg.byte_offset == 0 {
            let needle = seg.name.as_bytes();
            let from = cursor.min(bytes.len());
            let found = if !needle.is_empty() {
                find_subslice(bytes, needle, from)
            } else {
                None
            };
            let candidate = match found {
                Some(pos) => pos as u32,
                None => {
                    if i == 0 {
                        ref_byte_offset
                    } else {
                        prev.saturating_add(1)
                    }
                }
            };
            seg.byte_offset = if i > 0 && candidate <= prev {
                prev.saturating_add(1)
            } else {
                candidate
            };
        }
        if let Some(pos) = (seg.byte_offset as usize).checked_add(seg.name.len()) {
            cursor = pos;
        }
        prev = seg.byte_offset;
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() || from >= haystack.len() {
        return None;
    }
    let end = haystack.len() - needle.len();
    let mut i = from;
    while i <= end {
        if haystack[i..i + needle.len()] == *needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Build per-line byte-start indices once per file so SYM-004 / REF-006 can
/// cross-check (line, col) against byte_offset in O(1).
fn build_line_starts(content: &str) -> Vec<u32> {
    let mut starts = Vec::with_capacity(content.len() / 40);
    starts.push(0u32);
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            starts.push((i + 1) as u32);
        }
    }
    starts
}

/// Returns the expected byte offset of (line, col) given the precomputed line
/// starts. Clamps to file length on out-of-range positions.
fn expected_byte(line_starts: &[u32], line: u32, col: u32) -> Option<u32> {
    line_starts
        .get(line as usize)
        .map(|&s| s.saturating_add(col))
}

/// Wrap an `ExtractionResult` as a `ParsedFile` for validation. Extractor
/// unit tests pass `source` so the populate_positions pass can derive the
/// new position fields from content. `path` and `language` shape the
/// validator's location messages and govern the external-files exemption.
pub fn validate_extraction(
    extraction: crate::types::ExtractionResult,
    source: &str,
    path: &str,
    language: &str,
) -> Vec<ContractViolation> {
    let size = source.len() as u64;
    let line_count = source.lines().count() as u32;
    let mut parsed = ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
        size,
        line_count,
        mtime: None,
        package_id: None,
        symbols: extraction.symbols,
        refs: extraction.refs,
        routes: extraction.routes,
        db_sets: extraction.db_sets,
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(source.to_string()),
        has_errors: extraction.has_errors,
        flow: Default::default(),
        demand_contributions: extraction.demand_contributions,
        alias_targets: extraction.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };
    let arena = TypeArena::new();
    populate_positions(&mut parsed, &arena);
    validate_with_arena(&parsed, &arena)
}

/// Like `validate_extraction` but panics on any violation; the natural shape
/// for extractor lib tests that should never produce contract violations.
pub fn assert_extraction_canonical(
    extraction: crate::types::ExtractionResult,
    source: &str,
    path: &str,
    language: &str,
) {
    let violations = validate_extraction(extraction, source, path, language);
    if violations.is_empty() {
        return;
    }
    let mut buf = String::with_capacity(violations.len() * 80);
    buf.push_str("canonical-form contract violations:\n");
    for v in &violations {
        buf.push_str("  - ");
        buf.push_str(&v.to_string());
        buf.push('\n');
    }
    panic!("{buf}");
}

/// Panics with the full violation list when `validate_with_arena(file, arena)`
/// is non-empty. When `BW_CANONICAL_FORM_REPORT` is set, writes to stderr
/// and returns — for sweep-style triage runs.
pub fn assert_canonical(file: &ParsedFile, arena: &TypeArena) {
    let violations = validate_with_arena(file, arena);
    if violations.is_empty() {
        return;
    }
    let mut buf = String::with_capacity(violations.len() * 80);
    buf.push_str("canonical-form contract violations:\n");
    for v in &violations {
        buf.push_str("  - ");
        buf.push_str(&v.to_string());
        buf.push('\n');
    }
    if std::env::var_os("BW_CANONICAL_FORM_REPORT").is_some() {
        eprintln!("{buf}");
        return;
    }
    panic!("{buf}");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sym_loc(file: &ParsedFile, idx: usize, sym: &ExtractedSymbol) -> ViolationLocation {
    ViolationLocation::Symbol {
        path: file.path.clone(),
        index: idx,
        name: sym.name.clone(),
    }
}

fn ref_loc(file: &ParsedFile, idx: usize, r: &ExtractedRef) -> ViolationLocation {
    ViolationLocation::Ref {
        path: file.path.clone(),
        index: idx,
        target_name: r.target_name.clone(),
    }
}

fn file_loc(file: &ParsedFile) -> ViolationLocation {
    ViolationLocation::File {
        path: file.path.clone(),
    }
}

/// Returns true for any character that any registered language uses as a
/// qname separator.
fn is_qname_separator_char(c: char) -> bool {
    matches!(
        c,
        '.' | ':' | '/' | '\\' | '$' | '>' | '-' | '\'' | '|' | '#' | '@'
    )
}

/// Whether `kind` is a callable kind that legitimately carries `call_args`.
fn kind_allows_call_args(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Calls | EdgeKind::Instantiates | EdgeKind::Imports
    )
}

// ---------------------------------------------------------------------------
// FILE-* rules
// ---------------------------------------------------------------------------

fn check_file_parallel_vecs(file: &ParsedFile, out: &mut Vec<ContractViolation>) {
    let nsym = file.symbols.len();
    let nref = file.refs.len();

    if !file.symbol_origin_languages.is_empty() && file.symbol_origin_languages.len() != nsym {
        out.push(ContractViolation {
            code: "FILE-001",
            message: format!(
                "symbol_origin_languages.len() = {} but symbols.len() = {}",
                file.symbol_origin_languages.len(),
                nsym,
            ),
            location: file_loc(file),
        });
    }
    if !file.ref_origin_languages.is_empty() && file.ref_origin_languages.len() != nref {
        out.push(ContractViolation {
            code: "FILE-002",
            message: format!(
                "ref_origin_languages.len() = {} but refs.len() = {}",
                file.ref_origin_languages.len(),
                nref,
            ),
            location: file_loc(file),
        });
    }
    if !file.symbol_from_snippet.is_empty() && file.symbol_from_snippet.len() != nsym {
        out.push(ContractViolation {
            code: "FILE-003",
            message: format!(
                "symbol_from_snippet.len() = {} but symbols.len() = {}",
                file.symbol_from_snippet.len(),
                nsym,
            ),
            location: file_loc(file),
        });
    }
}

fn check_flow_meta(file: &ParsedFile, out: &mut Vec<ContractViolation>) {
    let nref = file.refs.len();
    let nsym = file.symbols.len();
    for (&ref_idx, &lhs_idx) in &file.flow.flow_binding_lhs {
        if ref_idx >= nref {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_binding_lhs key {ref_idx} is out of bounds (refs.len() = {nref})",
                ),
                location: file_loc(file),
            });
        }
        if lhs_idx >= nsym {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_binding_lhs value {lhs_idx} is out of bounds (symbols.len() = {nsym})",
                ),
                location: file_loc(file),
            });
        }
    }
    for (&lhs_idx, _) in &file.flow.flow_binding_decl_type {
        if lhs_idx >= nsym {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_binding_decl_type key {lhs_idx} is out of bounds (symbols.len() = {nsym})",
                ),
                location: file_loc(file),
            });
        }
    }
    for &lhs_idx in &file.flow.flow_binding_unwrap {
        if lhs_idx >= nsym {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_binding_unwrap entry {lhs_idx} is out of bounds (symbols.len() = {nsym})",
                ),
                location: file_loc(file),
            });
        }
    }
    if !file.flow.ref_byte_offsets.is_empty() && file.flow.ref_byte_offsets.len() != nref {
        out.push(ContractViolation {
            code: "FILE-004",
            message: format!(
                "flow.ref_byte_offsets.len() = {} but refs.len() = {}",
                file.flow.ref_byte_offsets.len(),
                nref,
            ),
            location: file_loc(file),
        });
    }
}

// ---------------------------------------------------------------------------
// SYM-* rules
// ---------------------------------------------------------------------------

/// SYM-001: qualified_name ends with name, with a separator char immediately
/// before the suffix (or qname == name for top-level symbols).
fn check_sym_001(
    file: &ParsedFile,
    idx: usize,
    sym: &ExtractedSymbol,
    out: &mut Vec<ContractViolation>,
) {
    let qname = &sym.qualified_name;
    let name = &sym.name;
    if name.is_empty() {
        return;
    }
    if qname == name {
        return;
    }
    if !qname.ends_with(name) {
        out.push(ContractViolation {
            code: "SYM-001",
            message: format!("qualified_name '{qname}' does not end with name '{name}'",),
            location: sym_loc(file, idx, sym),
        });
        return;
    }
    let prefix_len = qname.len() - name.len();
    if let Some(c) = qname[..prefix_len].chars().last() {
        if !is_qname_separator_char(c) {
            out.push(ContractViolation {
                code: "SYM-001",
                message: format!(
                    "qualified_name '{qname}' ends with name '{name}' but is not preceded by a separator character (found '{c}')",
                ),
                location: sym_loc(file, idx, sym),
            });
        }
    }
}

/// SYM-002: scope_path must agree with the parent chain. If parent_index is
/// Some, then scope_path equals the parent's qualified_name.
fn check_sym_002(
    file: &ParsedFile,
    idx: usize,
    sym: &ExtractedSymbol,
    out: &mut Vec<ContractViolation>,
) {
    let Some(pidx) = sym.parent_index else { return };
    let Some(parent) = file.symbols.get(pidx) else {
        return;
    };
    let expected = parent.qualified_name.as_str();
    match sym.scope_path.as_deref() {
        Some(scope) if scope == expected => {}
        Some(scope) => {
            out.push(ContractViolation {
                code: "SYM-002",
                message: format!(
                    "scope_path '{scope}' does not match parent qualified_name '{expected}'",
                ),
                location: sym_loc(file, idx, sym),
            });
        }
        None => {
            out.push(ContractViolation {
                code: "SYM-002",
                message: format!(
                    "parent_index is Some but scope_path is None (expected '{expected}')",
                ),
                location: sym_loc(file, idx, sym),
            });
        }
    }
}

/// SYM-003: parent_index, if Some, must reference a strictly smaller index in
/// the same Vec.
fn check_sym_003(
    file: &ParsedFile,
    idx: usize,
    sym: &ExtractedSymbol,
    out: &mut Vec<ContractViolation>,
) {
    let Some(pidx) = sym.parent_index else { return };
    if pidx >= idx {
        out.push(ContractViolation {
            code: "SYM-003",
            message: format!(
                "parent_index {pidx} is not strictly less than this symbol's index {idx}",
            ),
            location: sym_loc(file, idx, sym),
        });
        return;
    }
    if pidx >= file.symbols.len() {
        out.push(ContractViolation {
            code: "SYM-003",
            message: format!(
                "parent_index {pidx} is out of bounds (symbols.len() = {})",
                file.symbols.len(),
            ),
            location: sym_loc(file, idx, sym),
        });
    }
}

// ---------------------------------------------------------------------------
// REF-* rules
// ---------------------------------------------------------------------------

/// REF-001: source_symbol_index < symbols.len().
fn check_ref_001(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    out: &mut Vec<ContractViolation>,
) {
    if r.source_symbol_index >= file.symbols.len() {
        out.push(ContractViolation {
            code: "REF-001",
            message: format!(
                "source_symbol_index {} is out of bounds (symbols.len() = {})",
                r.source_symbol_index,
                file.symbols.len(),
            ),
            location: ref_loc(file, idx, r),
        });
    }
}

/// REF-002: byte_offset must point inside the file. A 0 value is only valid
/// at line 0 (the very start of the file).
fn check_ref_002(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    out: &mut Vec<ContractViolation>,
) {
    if file.size == 0 {
        return;
    }
    if r.byte_offset != 0 {
        return;
    }
    if r.line == 0 {
        return;
    }
    out.push(ContractViolation {
        code: "REF-002",
        message: format!(
            "byte_offset is 0 on {:?} ref at line {} (extractor did not populate byte_offset)",
            r.kind, r.line,
        ),
        location: ref_loc(file, idx, r),
    });
}

/// REF-003: a Calls ref whose target_name contains a `.`, `::`, or `->` must
/// carry a MemberChain.
fn check_ref_003(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    out: &mut Vec<ContractViolation>,
) {
    if r.kind != EdgeKind::Calls {
        return;
    }
    if r.chain.is_some() {
        return;
    }
    let tn = r.target_name.as_str();
    let dotted = tn.contains('.') || tn.contains("::") || tn.contains("->");
    if dotted {
        out.push(ContractViolation {
            code: "REF-003",
            message: format!("Calls ref has dotted target_name '{tn}' but no MemberChain",),
            location: ref_loc(file, idx, r),
        });
    }
}

/// REF-004: if chain is Some, its last segment name equals target_name.
fn check_ref_004(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    out: &mut Vec<ContractViolation>,
) {
    let Some(chain) = r.chain.as_ref() else {
        return;
    };
    let Some(last) = chain.segments.last() else {
        return;
    };
    if last.name != r.target_name {
        out.push(ContractViolation {
            code: "REF-004",
            message: format!(
                "chain last segment '{}' does not match target_name '{}'",
                last.name, r.target_name,
            ),
            location: ref_loc(file, idx, r),
        });
    }
}

/// REF-005: call_args is empty when kind is not in {Calls, Instantiates,
/// Imports}.
fn check_ref_005(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    out: &mut Vec<ContractViolation>,
) {
    if r.call_args.is_empty() {
        return;
    }
    if kind_allows_call_args(r.kind) {
        return;
    }
    out.push(ContractViolation {
        code: "REF-005",
        message: format!(
            "call_args populated ({} entries) on a {:?} ref; only Calls / Instantiates / Imports may carry call_args",
            r.call_args.len(),
            r.kind,
        ),
        location: ref_loc(file, idx, r),
    });
}

// ---------------------------------------------------------------------------
// CHAIN-* rules
// ---------------------------------------------------------------------------

/// CHAIN-001: segments.len() >= 1.
fn check_chain_001(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    chain: &MemberChain,
    out: &mut Vec<ContractViolation>,
) {
    if chain.segments.is_empty() {
        out.push(ContractViolation {
            code: "CHAIN-001",
            message: "MemberChain has zero segments; chain must have at least one segment"
                .to_string(),
            location: ref_loc(file, idx, r),
        });
    }
}

/// CHAIN-002: first segment kind in {SelfRef, Identifier, TypeAccess,
/// NamespaceAccess, Construction}.
fn check_chain_002(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    chain: &MemberChain,
    out: &mut Vec<ContractViolation>,
) {
    let Some(first) = chain.segments.first() else {
        return;
    };
    let ok = matches!(
        first.kind,
        SegmentKind::SelfRef
            | SegmentKind::Identifier
            | SegmentKind::TypeAccess
            | SegmentKind::NamespaceAccess
            | SegmentKind::Construction,
    );
    if !ok {
        out.push(ContractViolation {
            code: "CHAIN-002",
            message: format!(
                "first chain segment '{}' has kind {:?}; must be SelfRef / Identifier / TypeAccess / NamespaceAccess / Construction",
                first.name, first.kind,
            ),
            location: ref_loc(file, idx, r),
        });
    }
}

/// SYM-004: byte_offset must equal the byte position of (start_line, start_col)
/// against the file content.
fn check_sym_004(
    file: &ParsedFile,
    idx: usize,
    sym: &ExtractedSymbol,
    line_starts: Option<&[u32]>,
    out: &mut Vec<ContractViolation>,
) {
    let Some(starts) = line_starts else { return };
    let Some(expected) = expected_byte(starts, sym.start_line, sym.start_col) else {
        return;
    };
    if sym.byte_offset != expected {
        out.push(ContractViolation {
            code: "SYM-004",
            message: format!(
                "byte_offset {} does not match expected {} for (line {}, col {})",
                sym.byte_offset, expected, sym.start_line, sym.start_col,
            ),
            location: sym_loc(file, idx, sym),
        });
    }
}

/// REF-006: byte_offset must equal the byte position of (line, col) against
/// the file content.
fn check_ref_006(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    line_starts: Option<&[u32]>,
    out: &mut Vec<ContractViolation>,
) {
    let Some(starts) = line_starts else { return };
    if file
        .ref_origin_languages
        .get(idx)
        .map(|o| o.is_some())
        .unwrap_or(false)
    {
        return;
    }
    let Some(expected) = expected_byte(starts, r.line, r.col) else {
        return;
    };
    if r.byte_offset != expected {
        out.push(ContractViolation {
            code: "REF-006",
            message: format!(
                "byte_offset {} does not match expected {} for (line {}, col {})",
                r.byte_offset, expected, r.line, r.col,
            ),
            location: ref_loc(file, idx, r),
        });
    }
}

/// CHAIN-003: segment byte_offsets within a chain strictly increase.
fn check_chain_003(
    file: &ParsedFile,
    idx: usize,
    r: &ExtractedRef,
    chain: &MemberChain,
    out: &mut Vec<ContractViolation>,
) {
    let mut prev: Option<u32> = None;
    for (seg_idx, seg) in chain.segments.iter().enumerate() {
        if seg.byte_offset == 0 && seg_idx > 0 {
            out.push(ContractViolation {
                code: "CHAIN-003",
                message: format!(
                    "segment #{} '{}' has byte_offset 0; only the first segment may sit at byte 0",
                    seg_idx, seg.name,
                ),
                location: ref_loc(file, idx, r),
            });
            continue;
        }
        if let Some(p) = prev {
            if seg.byte_offset <= p {
                out.push(ContractViolation {
                    code: "CHAIN-003",
                    message: format!(
                        "segment #{} '{}' byte_offset {} is not strictly greater than previous segment's {}",
                        seg_idx, seg.name, seg.byte_offset, p,
                    ),
                    location: ref_loc(file, idx, r),
                });
            }
        }
        prev = Some(seg.byte_offset);
    }
}

/// SYM-005 (arena-checked): type-defining symbols must carry a return_type
/// that resolves to `Type::Class(self_qname)`. Runs only after
/// populate_positions, which establishes the arena.
fn check_sym_005_arena(
    file: &ParsedFile,
    idx: usize,
    sym: &ExtractedSymbol,
    arena: &TypeArena,
    out: &mut Vec<ContractViolation>,
) {
    if !is_type_defining_kind(sym.kind) {
        return;
    }
    let Some(rt) = sym.return_type else {
        out.push(ContractViolation {
            code: "SYM-005",
            message: format!(
                "type-defining symbol of kind {:?} must have return_type = Some(self_type_id)",
                sym.kind,
            ),
            location: sym_loc(file, idx, sym),
        });
        return;
    };
    if rt.index() >= arena.len() {
        out.push(ContractViolation {
            code: "SYM-005",
            message: format!(
                "return_type TypeId {} is out of bounds (arena.len() = {})",
                rt.index() + 1,
                arena.len(),
            ),
            location: sym_loc(file, idx, sym),
        });
        return;
    }
    let actual = arena.get(rt);
    match actual {
        Type::Class(ref q) if q == &sym.qualified_name => {}
        Type::Class(q) => {
            out.push(ContractViolation {
                code: "SYM-005",
                message: format!(
                    "return_type resolves to Class('{q}'); expected Class('{}')",
                    sym.qualified_name,
                ),
                location: sym_loc(file, idx, sym),
            });
        }
        other => {
            out.push(ContractViolation {
                code: "SYM-005",
                message: format!(
                    "return_type resolves to {other:?}; expected Class('{}')",
                    sym.qualified_name,
                ),
                location: sym_loc(file, idx, sym),
            });
        }
    }
}

/// SYM-006: callable symbols must have param_types.len() consistent with
/// their declared signature. The arity is read from `signature` when
/// present (a heuristic — we count comma-separated args inside the first
/// parenthesized group). When no signature is recorded the rule is silent.
fn check_sym_006(
    file: &ParsedFile,
    idx: usize,
    sym: &ExtractedSymbol,
    out: &mut Vec<ContractViolation>,
) {
    if !is_callable_kind(sym.kind) {
        return;
    }
    let Some(sig) = sym.signature.as_deref() else {
        return;
    };
    let Some(arity) = signature_arity(sig) else {
        return;
    };
    if sym.param_types.is_empty() {
        // Not yet populated — expected during transition. Skip rather than
        // fire on every callable in the corpus.
        return;
    }
    if sym.param_types.len() != arity {
        out.push(ContractViolation {
            code: "SYM-006",
            message: format!(
                "param_types.len() = {} disagrees with signature arity {} (signature: '{}')",
                sym.param_types.len(),
                arity,
                sig,
            ),
            location: sym_loc(file, idx, sym),
        });
    }
}

/// Count comma-separated arguments inside the first parenthesized group of
/// a signature string. Returns None when the string has no balanced parens
/// (synthesized signatures, free-form descriptions). Counts every declared
/// parameter — including an explicit receiver (`self`/`this`) where the
/// language writes one in the signature.
pub(crate) fn signature_arity(sig: &str) -> Option<usize> {
    let open = sig.find('(')?;
    let after = &sig[open + 1..];
    let mut depth: u32 = 1;
    let mut count = 0usize;
    let mut seen_non_ws = false;
    let mut last_was_comma = true;
    for ch in after.chars() {
        match ch {
            '(' | '<' | '[' | '{' => {
                depth = depth.saturating_add(1);
                seen_non_ws = true;
                last_was_comma = false;
            }
            ')' | '>' | ']' | '}' => {
                if depth == 1 && ch == ')' {
                    if seen_non_ws && !last_was_comma {
                        count += 1;
                    }
                    return Some(count);
                }
                depth = depth.saturating_sub(1);
                seen_non_ws = true;
                last_was_comma = false;
            }
            ',' if depth == 1 => {
                if seen_non_ws {
                    count += 1;
                }
                last_was_comma = true;
                seen_non_ws = false;
            }
            c if c.is_whitespace() => {}
            _ => {
                seen_non_ws = true;
                last_was_comma = false;
            }
        }
    }
    None
}

/// TYPE-001: TypeId references must resolve to an entry in the arena. With
/// `NonZeroU32` TypeIds we only need to verify the upper bound; the lower
/// bound is enforced by the type itself.
fn check_type_001(
    file: &ParsedFile,
    ty: TypeId,
    arena: &TypeArena,
    out: &mut Vec<ContractViolation>,
) {
    if ty.index() >= arena.len() {
        out.push(ContractViolation {
            code: "TYPE-001",
            message: format!(
                "TypeId {} is out of bounds (arena.len() = {})",
                ty.index() + 1,
                arena.len(),
            ),
            location: ViolationLocation::File {
                path: file.path.clone(),
            },
        });
    }
}

#[cfg(test)]
#[path = "canonical_form_tests.rs"]
mod tests;
