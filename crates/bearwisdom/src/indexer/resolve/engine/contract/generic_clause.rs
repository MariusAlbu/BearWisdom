// =============================================================================
// engine/contract/generic_clause — name-anchored generic-parameter clause
// location in a declaration signature
//
// A signature can contain several bracket groups (`Wrapper<Arg> Name(...)`
// return-type applications, parameter types, the declaration clause itself).
// The declaration's OWN clause is the one anchored on the declaration name:
// either glued to it (`Name<T>(...)`, `class Name<T>`) or free-standing just
// before it (`fun <T> name(...)`). A first-bracket scan cannot tell those
// apart and reads a return type's argument list as the declaration's
// parameters — a garbage parameter that later rewrites legitimate type
// mentions of the same name.
// =============================================================================

use super::chain_walker::{find_matching_bracket, merge_where_bounds, parse_generic_param_clause};

/// The generic parameters `sig` declares for the symbol named `name`, as
/// `(name, bound, default)` triples with `where`-clause bounds merged.
///
/// Anchoring, in order:
///   1. The LAST token-boundary occurrence of `name` in `sig` decides. A `<`
///      or `[` immediately after it is the declaration clause. Any other
///      following character means the declaration is non-generic — an earlier
///      bracket group is a type application, never the clause — UNLESS a
///      free-standing group precedes the name (rule 2).
///   2. A bracket group whose opener is NOT glued to an identifier character
///      and that closes before the name is a prefix declaration clause
///      (`fun <T> name(...)`); the last such group before the name wins.
///   3. When `name` never occurs at a token boundary, the signature does not
///      carry the declaration name (positional shapes); fall back to the
///      first bracket group that parses to a non-empty clause.
///
/// Empty when the anchored declaration carries no clause.
pub(crate) fn signature_generic_params(
    sig: &str,
    name: &str,
) -> Vec<(String, Option<String>, Option<String>)> {
    let Some(idx) = last_boundary_occurrence(sig, name) else {
        return first_bracket_generic_params(sig);
    };
    let after = idx + name.len();
    if let Some(open) = sig[after..].chars().next().filter(|c| *c == '<' || *c == '[') {
        return clause_params_at(sig, after, open);
    }
    match last_free_standing_group_before(sig, idx) {
        Some((start, open)) => clause_params_at(sig, start, open),
        None => Vec::new(),
    }
}

/// Parse the bracket group opening at byte `start` (whose opener is `open`)
/// into `(name, bound, default)` triples, merging any `where`-clause bounds
/// declared elsewhere in `sig`. Empty when the group never closes.
fn clause_params_at(
    sig: &str,
    start: usize,
    open: char,
) -> Vec<(String, Option<String>, Option<String>)> {
    let close = if open == '<' { '>' } else { ']' };
    let Some(rel_end) = find_matching_bracket(&sig[start..], open, close) else {
        return Vec::new();
    };
    let mut gparams = parse_generic_param_clause(&sig[start + 1..start + rel_end]);
    merge_where_bounds(&mut gparams, sig);
    gparams
}

/// Byte index of the last occurrence of `name` in `sig` at a token boundary:
/// the byte before is absent or a non-identifier byte, and so is the byte
/// after — so `Sync` never anchors inside `SyncAll`. `None` when no boundary
/// occurrence exists.
fn last_boundary_occurrence(sig: &str, name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    let bytes = sig.as_bytes();
    let mut upto = sig.len();
    while let Some(idx) = sig[..upto].rfind(name) {
        let after = idx + name.len();
        let before_ok = idx == 0 || !is_ident_byte(bytes[idx - 1]);
        let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
        if before_ok && after_ok {
            return Some(idx);
        }
        if idx == 0 {
            break;
        }
        upto = idx;
    }
    None
}

/// The last bracket group before byte `limit` whose opener is free-standing —
/// not glued to an identifier byte — and whose matching close also falls
/// before `limit`. Returns `(open_index, open_char)`. A glued opener
/// (`Wrapper<Arg>`) is a type application and never qualifies.
fn last_free_standing_group_before(sig: &str, limit: usize) -> Option<(usize, char)> {
    let bytes = sig.as_bytes();
    for (i, c) in sig[..limit].char_indices().rev() {
        if c != '<' && c != '[' {
            continue;
        }
        if i > 0 && is_ident_byte(bytes[i - 1]) {
            continue;
        }
        let close = if c == '<' { '>' } else { ']' };
        if let Some(rel_end) = find_matching_bracket(&sig[i..], c, close) {
            if i + rel_end < limit {
                return Some((i, c));
            }
        }
    }
    None
}

/// The first bracket group that parses to a non-empty clause — the anchor-less
/// fallback for signature shapes that do not repeat the declaration name.
fn first_bracket_generic_params(sig: &str) -> Vec<(String, Option<String>, Option<String>)> {
    for open in ['<', '['] {
        if let Some(start) = sig.find(open) {
            let gparams = clause_params_at(sig, start, open);
            if !gparams.is_empty() {
                return gparams;
            }
        }
    }
    Vec::new()
}

/// ASCII identifier byte: the token-boundary alphabet for signature text.
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
#[path = "generic_clause_tests.rs"]
mod tests;
