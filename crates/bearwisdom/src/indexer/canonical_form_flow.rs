// =============================================================================
// indexer/canonical_form_flow.rs  —  FILE-004 bounds rules for FlowMeta
//
// Every flow map keys a ref index or a symbol index of the SAME file. A stale
// index silently retargets an initializer onto an unrelated declaration, so
// each map is bounds-checked against its own file's vectors.
// =============================================================================

use super::canonical_form::{file_loc, ContractViolation};
use crate::types::ParsedFile;

pub(super) fn check_flow_meta(file: &ParsedFile, out: &mut Vec<ContractViolation>) {
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
    for (&ref_idx, &member_idx) in &file.flow.flow_member_init {
        if ref_idx >= nref {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_member_init key {ref_idx} is out of bounds (refs.len() = {nref})",
                ),
                location: file_loc(file),
            });
        }
        if member_idx >= nsym {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_member_init value {member_idx} is out of bounds (symbols.len() = {nsym})",
                ),
                location: file_loc(file),
            });
        }
    }
    for (&ref_idx, entries) in &file.flow.flow_binding_destructure {
        if ref_idx >= nref {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_binding_destructure key {ref_idx} is out of bounds (refs.len() = {nref})",
                ),
                location: file_loc(file),
            });
        }
        for (lhs_idx, _) in entries {
            if *lhs_idx >= nsym {
                out.push(ContractViolation {
                    code: "FILE-004",
                    message: format!(
                        "flow.flow_binding_destructure value {lhs_idx} is out of bounds (symbols.len() = {nsym})",
                    ),
                    location: file_loc(file),
                });
            }
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
    for &lhs_idx in &file.flow.flow_binding_await {
        if lhs_idx >= nsym {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_binding_await entry {lhs_idx} is out of bounds (symbols.len() = {nsym})",
                ),
                location: file_loc(file),
            });
        }
    }
    for &ref_idx in &file.flow.flow_binding_destructure_await {
        if ref_idx >= nref {
            out.push(ContractViolation {
                code: "FILE-004",
                message: format!(
                    "flow.flow_binding_destructure_await entry {ref_idx} is out of bounds (refs.len() = {nref})",
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

#[cfg(test)]
#[path = "canonical_form_flow_tests.rs"]
mod tests;
