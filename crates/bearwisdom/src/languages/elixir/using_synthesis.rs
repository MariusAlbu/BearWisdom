// =============================================================================
// elixir/using_synthesis.rs — harvested `Def` facts → real member symbols.
//
// A module that (transitively) `use`s another module whose `__using__`
// quote block defines functions literally compiles those functions into
// ITSELF at runtime — `Plausible.Factory`'s `build/2` exists nowhere in
// Factory's own source text, only inside ExMachina's quote block. No
// per-file extractor can see this: the fact only exists once the whole
// project's `use`-injection map has been built and flattened. This module
// turns that flattened `Def` data into real `ExtractedSymbol` rows on every
// module whose `use` chain reaches it, via `LanguagePlugin::
// synthesize_project_symbols`.
// =============================================================================

use std::collections::HashSet;

use super::using_injection::{ElixirInjection, ElixirProjectState};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};

/// For every Elixir module in `parsed` whose flattened injection set
/// contains `Def` facts, synthesize one `ExtractedSymbol` per fact not
/// already declared by that module. Returns `(file_path, new_symbols)`
/// pairs — empty for files that contribute nothing.
pub(super) fn synthesize_def_members(
    project_state: &ElixirProjectState,
    parsed: &[ParsedFile],
) -> Vec<(String, Vec<ExtractedSymbol>)> {
    let mut out = Vec::new();
    for pf in parsed {
        if pf.language != "elixir" {
            continue;
        }
        let mut synthesized = Vec::new();
        let existing: HashSet<&str> =
            pf.symbols.iter().map(|s| s.qualified_name.as_str()).collect();
        let mut seen_qnames: HashSet<String> = HashSet::new();
        for module_qname in module_qnames_in_file(pf) {
            for inj in project_state.flattened_injections_for(&module_qname) {
                let ElixirInjection::Def { name, is_macro } = inj else {
                    continue;
                };
                let qualified_name = format!("{module_qname}.{name}");
                if existing.contains(qualified_name.as_str()) {
                    continue;
                }
                if !seen_qnames.insert(qualified_name.clone()) {
                    continue;
                }
                synthesized.push(synthetic_symbol(name, &qualified_name, &module_qname, *is_macro));
            }
        }
        if !synthesized.is_empty() {
            out.push((pf.path.clone(), synthesized));
        }
    }
    out
}

/// The qualified names of every `defmodule` declared in `pf`, read off the
/// already-extracted `Module`-kind symbols rather than re-parsing.
fn module_qnames_in_file(pf: &ParsedFile) -> Vec<String> {
    pf.symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Module)
        .map(|s| s.qualified_name.clone())
        .collect()
}

/// Build the synthesized member row. `parent_index` is left `None` —
/// `qualified_name`'s dotted prefix is how the resolve pass's structural
/// pass reconstructs containment for a member with no in-batch parent,
/// the same convention `LanguagePlugin::synthesize_symbols` documents for
/// its own per-file splice.
fn synthetic_symbol(
    name: &str,
    qualified_name: &str,
    module_qname: &str,
    is_macro: bool,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind: if is_macro { SymbolKind::Function } else { SymbolKind::Method },
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: Some(module_qname.to_string()),
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[cfg(test)]
#[path = "using_synthesis_tests.rs"]
mod tests;
