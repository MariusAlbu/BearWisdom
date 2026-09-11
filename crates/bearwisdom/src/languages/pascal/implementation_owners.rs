// =============================================================================
// languages/pascal/implementation_owners — attach out-of-class bodies to types
//
// Pascal declares a member inside a type and defines it later in the
// implementation section as `procedure TWidget.Paint`. Tree-sitter places that
// definition under the unit, so the extraction walk cannot carry the declaring
// type as its structural parent. Repair that source-language shape here, before
// generic indexing derives enclosing-type identities from `parent_index`.
// =============================================================================

use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};

/// Reparent unit-level qualified routine implementations to the uniquely named
/// type declared in the same unit. The routine keeps its source-spelled name and
/// qname; only structural ownership and scope evidence change.
pub(super) fn attach(symbols: &mut [ExtractedSymbol], refs: &mut [ExtractedRef]) {
    let namespace_scopes: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter_map(|(index, symbol)| (symbol.kind == SymbolKind::Namespace).then_some(index))
        .collect();
    let type_owners: Vec<(usize, String, String, Option<usize>)> = symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| {
            matches!(
                symbol.kind,
                SymbolKind::Class | SymbolKind::Struct | SymbolKind::Interface
            )
        })
        .map(|(index, symbol)| {
            (
                index,
                symbol.name.clone(),
                symbol.qualified_name.clone(),
                symbol.parent_index,
            )
        })
        .collect();

    let mut attachments = Vec::new();
    for (symbol_index, symbol) in symbols.iter().enumerate() {
        if !matches!(
            symbol.kind,
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
        ) {
            continue;
        }
        let Some(structural_parent) = symbol.parent_index else {
            continue;
        };
        let (unit_index, body_owner, fragment_body) = if namespace_scopes
            .contains(&structural_parent)
        {
            (Some(structural_parent), None, false)
        } else {
            let wrapper = &symbols[structural_parent];
            if wrapper.name != "unknown"
                || !matches!(wrapper.kind, SymbolKind::Function | SymbolKind::Method)
                || symbols
                    .iter()
                    .filter(|candidate| {
                        candidate.parent_index == Some(structural_parent)
                            && matches!(
                                candidate.kind,
                                SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
                            )
                            && candidate.name.contains('.')
                    })
                    .count()
                    != 1
            {
                continue;
            }
            match wrapper.parent_index {
                Some(unit_index) if namespace_scopes.contains(&unit_index) => {
                    (Some(unit_index), Some(structural_parent), false)
                }
                None => (None, Some(structural_parent), true),
                _ => continue,
            }
        };
        let Some((owner_spelling, _)) = symbol.name.rsplit_once('.') else {
            continue;
        };
        if fragment_body {
            attachments.push((symbol_index, None, None, body_owner));
            continue;
        }
        let Some(unit_index) = unit_index else {
            continue;
        };
        let owner_leaf = owner_spelling.rsplit('.').next().unwrap_or(owner_spelling);
        let mut matches = type_owners.iter().filter(|(_, name, qname, parent)| {
            *parent == Some(unit_index)
                && (name.eq_ignore_ascii_case(owner_leaf)
                    || qname.eq_ignore_ascii_case(owner_spelling))
        });
        let Some((owner_index, _, owner_qname, _)) = matches.next() else {
            continue;
        };
        if matches.next().is_some() {
            continue;
        }
        attachments.push((
            symbol_index,
            Some(*owner_index),
            Some(owner_qname.clone()),
            body_owner,
        ));
    }

    for (symbol_index, owner_index, owner_qname, body_owner) in attachments {
        symbols[symbol_index].parent_index = owner_index;
        symbols[symbol_index].scope_path = owner_qname;
        if let Some(body_owner) = body_owner {
            for reference in refs.iter_mut() {
                if reference.source_symbol_index == body_owner {
                    reference.source_symbol_index = symbol_index;
                }
            }
        }
    }
}

/// Add the source-spelled owner for a qualified routine only after include
/// assembly has attested the standalone fragment. The generic assembler then
/// prefixes this scope with the including unit, yielding `Unit.TType` for the
/// cross-file containment pass.
pub(super) fn prepare_include_splice(symbols: &mut [ExtractedSymbol]) {
    for symbol in symbols {
        if symbol.parent_index.is_some()
            || symbol.scope_path.is_some()
            || !matches!(
                symbol.kind,
                SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
            )
        {
            continue;
        }
        let Some((owner, _)) = symbol.name.rsplit_once('.') else {
            continue;
        };
        if !owner.is_empty() {
            symbol.scope_path = Some(owner.to_string());
        }
    }
}

#[cfg(test)]
#[path = "implementation_owners_tests.rs"]
mod tests;
