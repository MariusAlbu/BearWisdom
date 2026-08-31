// =============================================================================
// indexer/symbol_ids — symbol row identity for the resolve pipeline
// =============================================================================

use std::collections::HashMap;

use super::write::SymbolIdMap;

/// Symbol row identity for the resolve pipeline.
///
/// `by_row` is the identity spine: `by_row[path][i]` is the db id of
/// `pf.symbols[i]`, positionally exact, so two rows sharing a
/// `(path, qualified_name)` key — overloads, partial declarations, `impl`
/// blocks — each keep their own id. `by_key` is the qname-keyed view for
/// consumers that hold a qname but no row index (synthesized member qnames,
/// include-rewritten qnames); its inserts overwrite on duplicates, so it must
/// never be the source of identity where an index is available.
#[derive(Debug, Default, Clone)]
pub struct SymbolIds {
    by_key: SymbolIdMap,
    by_row: HashMap<String, Vec<i64>>,
}

impl SymbolIds {
    /// The db id of `pf.symbols[idx]` — positional when the file's row vector
    /// exists, `(path, qname)` fallback otherwise (synthetic/test inputs).
    pub fn id_of(&self, path: &str, idx: usize, qname: &str) -> Option<i64> {
        if let Some(rows) = self.by_row.get(path) {
            // 0 marks a row the writer skipped (survivor path); fall through.
            if let Some(&id) = rows.get(idx) {
                if id != 0 {
                    return Some(id);
                }
            }
        }
        self.by_key.get(&(path.to_string(), qname.to_string())).copied()
    }

    /// The qname-keyed view, for lookups that have no row index.
    pub fn by_key(&self) -> &SymbolIdMap {
        &self.by_key
    }

    /// Register a qname-keyed id (qname rewrites, synthesized keys).
    pub fn insert_key(&mut self, path: String, qname: String, id: i64) {
        self.by_key.insert((path, qname), id);
    }

    /// Register a file's positional row ids, aligned with its symbol vec.
    pub fn set_rows(&mut self, path: String, ids: Vec<i64>) {
        self.by_row.insert(path, ids);
    }

    /// Remove a qname-keyed id (qname rewrites re-key through this).
    pub fn remove_key(&mut self, path: &str, qname: &str) -> Option<i64> {
        self.by_key.remove(&(path.to_string(), qname.to_string()))
    }

    /// Insert a qname-keyed id only when the key is absent.
    pub fn insert_key_if_absent(&mut self, path: String, qname: String, id: i64) {
        self.by_key.entry((path, qname)).or_insert(id);
    }

    /// Rewrite ids in place after a merge pass replaced rows: every value in
    /// both views maps through `remapped` (deleted id -> canonical id).
    pub fn remap_ids(&mut self, remapped: &HashMap<i64, i64>) {
        for id in self.by_key.values_mut() {
            if let Some(&canonical) = remapped.get(id) {
                *id = canonical;
            }
        }
        for ids in self.by_row.values_mut() {
            for id in ids.iter_mut() {
                if let Some(&canonical) = remapped.get(id) {
                    *id = canonical;
                }
            }
        }
    }

    /// Absorb another batch's identity (demand-pulled externals, sub-walks).
    pub fn merge(&mut self, other: SymbolIds) {
        self.by_key.extend(other.by_key);
        self.by_row.extend(other.by_row);
    }

    /// Number of qname-keyed entries (diagnostic logging).
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

impl From<SymbolIdMap> for SymbolIds {
    fn from(by_key: SymbolIdMap) -> Self {
        Self { by_key, by_row: HashMap::new() }
    }
}
