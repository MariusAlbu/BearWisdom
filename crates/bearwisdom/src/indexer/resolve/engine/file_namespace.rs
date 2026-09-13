// =============================================================================
// engine/file_namespace — the namespace a parsed file declares into
//
// One question, answered from extractor output alone: which namespace do this
// file's top-level declarations belong to? Languages that emit no namespace
// symbol answer `None`, and every consumer treats that as "no namespace
// evidence" rather than a default.
// =============================================================================

use crate::types::{ParsedFile, SymbolKind};

/// The single namespace a parsed file declares its top-level symbols into.
/// `None` unless the file has exactly one top-level `Namespace` declaration
/// AND another symbol records that qname as its declaring scope — the
/// containment proof keeps a file that merely mentions a namespace from
/// claiming it.
pub(crate) fn declared_namespace(file: &ParsedFile) -> Option<&str> {
    let mut declared = file
        .symbols
        .iter()
        .filter(|s| s.parent_index.is_none() && s.kind == SymbolKind::Namespace);
    let ns = declared.next()?;
    if declared.next().is_some() {
        return None;
    }
    let contains_scoped_symbol = file
        .symbols
        .iter()
        .any(|s| s.scope_path.as_deref() == Some(ns.qualified_name.as_str()));
    contains_scoped_symbol.then_some(ns.qualified_name.as_str())
}

#[cfg(test)]
#[path = "file_namespace_tests.rs"]
mod tests;
