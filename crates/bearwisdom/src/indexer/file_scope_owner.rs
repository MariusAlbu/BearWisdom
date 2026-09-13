// =============================================================================
// indexer/file_scope_owner.rs — the owner a declaration-less file needs
//
// A reference is attributed to the symbol that encloses it, and every stage
// after extraction reads that owner out of the file's symbol table. Languages
// that let executable code sit at top level produce files that carry
// references while declaring nothing — a spec suite written as nested blocks,
// a route table, a configuration DSL — and each of those references names a
// symbol index the file has no symbol for, so it is dropped.
//
// This pass gives such a file one owner: the file itself, as a module spanning
// the whole file and identified by its path.
// =============================================================================

use crate::languages::common::file_container::{
    file_stem, materialize_as_sole_owner, path_stem, FileContainer,
};
use crate::types::{ExtractionResult, SymbolKind, Visibility};

/// Give `result` a file-scope owner when the extractor emitted index links but
/// no symbol to own them. `relative_path` is the project-relative path the
/// owner takes its name and qualified name from; `line_count` is the file's
/// line count, which the owner spans.
///
/// Returns whether an owner was materialized.
pub(super) fn materialize(
    result: &mut ExtractionResult,
    relative_path: &str,
    line_count: u32,
) -> bool {
    let name = file_stem(relative_path);
    let qualified_name = path_stem(relative_path);
    materialize_as_sole_owner(
        result,
        FileContainer {
            name: &name,
            qualified_name: &qualified_name,
            kind: SymbolKind::Module,
            // An attribution site, not a declaration the file publishes: the
            // private marking keeps it out of the exported-API entry roots.
            visibility: Visibility::Private,
            end_line: line_count.saturating_sub(1),
        },
    )
}

#[cfg(test)]
#[path = "file_scope_owner_tests.rs"]
mod tests;
