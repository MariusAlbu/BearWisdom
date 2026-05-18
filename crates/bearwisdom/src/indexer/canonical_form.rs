// =============================================================================
// indexer/canonical_form.rs  —  canonical-contract validator for ParsedFile
//
// Rule reference: research/architecture/01-canonical-symbol-ref-contract.html
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, ParsedFile, SegmentKind,
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

/// Validate `file` against the canonical contract. Returns one entry per
/// violation; empty Vec means the file is contract-clean.
pub fn validate(file: &ParsedFile) -> Vec<ContractViolation> {
    let mut out = Vec::new();
    check_file_parallel_vecs(file, &mut out);
    check_flow_meta(file, &mut out);
    for (idx, sym) in file.symbols.iter().enumerate() {
        check_sym_001(file, idx, sym, &mut out);
        check_sym_002(file, idx, sym, &mut out);
        check_sym_003(file, idx, sym, &mut out);
    }
    for (idx, r) in file.refs.iter().enumerate() {
        check_ref_001(file, idx, r, &mut out);
        check_ref_002(file, idx, r, &mut out);
        check_ref_003(file, idx, r, &mut out);
        check_ref_004(file, idx, r, &mut out);
        check_ref_005(file, idx, r, &mut out);
        if let Some(chain) = r.chain.as_ref() {
            check_chain_001(file, idx, r, chain, &mut out);
            check_chain_002(file, idx, r, chain, &mut out);
        }
    }
    out
}

/// Panics with the full violation list when `validate(file)` is non-empty.
/// When the env var `BW_CANONICAL_FORM_REPORT` is set, the violations are
/// written to stderr instead of panicking — for sweep-style triage runs.
pub fn assert_canonical(file: &ParsedFile) {
    let violations = validate(file);
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
    matches!(c, '.' | ':' | '/' | '\\' | '$' | '>' | '-' | '\'' | '|' | '#' | '@')
}

/// Whether `kind` is a callable kind that legitimately carries `call_args`.
fn kind_allows_call_args(kind: EdgeKind) -> bool {
    matches!(kind, EdgeKind::Calls | EdgeKind::Instantiates | EdgeKind::Imports)
}

// ---------------------------------------------------------------------------
// FILE-* rules
// ---------------------------------------------------------------------------

fn check_file_parallel_vecs(file: &ParsedFile, out: &mut Vec<ContractViolation>) {
    let nsym = file.symbols.len();
    let nref = file.refs.len();

    if !file.symbol_origin_languages.is_empty()
        && file.symbol_origin_languages.len() != nsym
    {
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
    if !file.flow.ref_byte_offsets.is_empty()
        && file.flow.ref_byte_offsets.len() != nref
    {
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
            message: format!(
                "qualified_name '{qname}' does not end with name '{name}'",
            ),
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
    let Some(parent) = file.symbols.get(pidx) else { return };
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
            message: format!(
                "Calls ref has dotted target_name '{tn}' but no MemberChain",
            ),
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
    let Some(chain) = r.chain.as_ref() else { return };
    let Some(last) = chain.segments.last() else { return };
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
            message: "MemberChain has zero segments; chain must have at least one segment".to_string(),
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

#[cfg(test)]
#[path = "canonical_form_tests.rs"]
mod tests;
