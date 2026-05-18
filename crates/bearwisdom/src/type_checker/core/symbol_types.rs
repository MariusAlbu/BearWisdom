// =============================================================================
// type_checker/core/symbol_types.rs — per-symbol type metadata
//
// Replaces the scattered string-keyed maps on SymbolIndex
// (field_type / return_type / param_types) with one map keyed by the DB
// symbol id. Built once per indexing run from canonical-form ExtractedSymbol
// fields after symbols are persisted.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 1
// =============================================================================

use super::types::{GenericParamId, TypeArena, TypeId};
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{ParsedFile, SymbolKind};
use rustc_hash::FxHashMap;

/// Maps `(file_path, parsed_file_symbol_index)` → durable DB symbol id. Built
/// by the indexer after symbols are persisted; consumed by
/// `SymbolTypeMap::build_from_parsed_files` so the type map keys by stable
/// id rather than per-run vector index.
pub type SymbolIdMap = FxHashMap<(String, usize), i64>;

/// Per-symbol type bundle. Every field is optional / empty when the symbol
/// kind does not carry it (e.g. a Class has return_type = Some(self_id) but
/// declared_type = None; a Method has param_types populated and
/// declared_type = None).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolTypeData {
    /// Annotated or inferred type for variables, fields, properties,
    /// parameters.
    pub declared_type: Option<TypeId>,
    /// Return type for functions, methods, constructors. For Class /
    /// Struct / Interface / Enum / TypeAlias symbols this is the symbol's
    /// own TypeId (classes-as-callable yield themselves).
    pub return_type: Option<TypeId>,
    /// Positional parameter types for callables. Empty for non-callable
    /// kinds.
    pub param_types: Vec<TypeId>,
    /// Generic parameter slots bound by this symbol (function, method, type).
    pub generic_params: Vec<GenericParamId>,
}

impl SymbolTypeData {
    pub fn is_empty(&self) -> bool {
        self.declared_type.is_none()
            && self.return_type.is_none()
            && self.param_types.is_empty()
            && self.generic_params.is_empty()
    }
}

/// Symbol id → SymbolTypeData lookup. The DB-side `symbols.id` is the key
/// because TypeIds and ExtractedSymbol indices are not durable across
/// indexing runs; symbol id is.
#[derive(Debug, Default)]
pub struct SymbolTypeMap {
    by_symbol_id: FxHashMap<i64, SymbolTypeData>,
}

impl SymbolTypeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            by_symbol_id: FxHashMap::with_capacity_and_hasher(cap, Default::default()),
        }
    }

    /// Number of symbols with non-empty type metadata recorded.
    pub fn len(&self) -> usize {
        self.by_symbol_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_symbol_id.is_empty()
    }

    /// Read the type bundle for a symbol id, if recorded.
    pub fn get(&self, sym_id: i64) -> Option<&SymbolTypeData> {
        self.by_symbol_id.get(&sym_id)
    }

    /// Insert / overwrite the bundle for `sym_id`. Empty bundles are
    /// discarded so `get` returns None for "no type info recorded."
    pub fn insert(&mut self, sym_id: i64, data: SymbolTypeData) {
        if data.is_empty() {
            self.by_symbol_id.remove(&sym_id);
            return;
        }
        self.by_symbol_id.insert(sym_id, data);
    }

    /// Mutable handle for incremental population (constructors-yield-self
    /// pass, decorator member synthesis, etc.). Creates an empty bundle on
    /// first access.
    pub fn entry(&mut self, sym_id: i64) -> &mut SymbolTypeData {
        self.by_symbol_id.entry(sym_id).or_default()
    }

    /// Read-only iterator over `(sym_id, data)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (i64, &SymbolTypeData)> + '_ {
        self.by_symbol_id.iter().map(|(k, v)| (*k, v))
    }

    /// Construct a SymbolTypeMap from per-file extraction output.
    ///
    /// Every type-defining symbol (Class, Struct, Interface, Trait, Enum,
    /// TypeAlias) receives `return_type = Some(self_type_id)` — the
    /// canonical "callable class yields itself" rule. The arena is extended
    /// with the corresponding Class type for each such symbol.
    ///
    /// Symbols whose extractor populated TypeId-bearing fields directly on
    /// ExtractedSymbol will have those carried in. Today's ExtractedSymbol
    /// does not yet carry TypeId fields; per-language migration will add
    /// them as extractors are reworked. Until then, only the self-yields-
    /// self entries are populated.
    pub fn build_from_parsed_files(
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        arena: &mut TypeArena,
        _profile: &LanguageProfile,
    ) -> Self {
        let mut map = SymbolTypeMap::new();
        for pf in parsed {
            for (idx, sym) in pf.symbols.iter().enumerate() {
                let Some(&sym_id) = sym_id_map.get(&(pf.path.clone(), idx)) else {
                    continue;
                };
                if is_type_defining(sym.kind) {
                    let class_id = arena.class(&sym.qualified_name);
                    let entry = map.entry(sym_id);
                    entry.return_type = Some(class_id);
                }
            }
        }
        map
    }
}

fn is_type_defining(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Interface
            | SymbolKind::Enum
            | SymbolKind::TypeAlias
    )
}

#[cfg(test)]
#[path = "symbol_types_tests.rs"]
mod tests;
