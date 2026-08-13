// =============================================================================
// nuget/assembly_cache.rs — weight-bounded memo for parsed-assembly lookups.
//
// Keyed by DLL path; each entry carries a caller-chosen eviction weight.
// Least-recently-used entries evict once the summed weight exceeds the
// budget — a corpus-scale run cracks types out of hundreds of distinct DLLs,
// and an unbounded memo pins every parsed assembly for the run's lifetime.
// An evicted DLL re-parses on next demand. The entry currently being
// inserted is never evicted, so a single DLL larger than the whole budget
// still caches while in use.
// `clear` runs at index-run boundaries so one run's demand set never pins
// memory into the next. Failed loads are remembered until `clear`, so a DLL
// that doesn't parse is attempted once per run, not once per demanded type.
// =============================================================================

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Summed entry weights. The worker weighs every assembly equally (weight 1),
/// so this is an entry count: a parsed assembly's heap cost is dominated by
/// fixed table overhead, not its DLL's file size, which makes on-disk bytes
/// a poor eviction proxy and equal weights a sound one.
pub(super) const DEFAULT_BUDGET: u64 = 128;

pub(super) struct AssemblyCache<V> {
    budget: u64,
    used: u64,
    loaded: HashMap<PathBuf, (V, u64)>,
    /// Recency order, front = least recently used.
    order: VecDeque<PathBuf>,
    failed: HashSet<PathBuf>,
}

impl<V: Clone> AssemblyCache<V> {
    pub(super) fn new(budget: u64) -> Self {
        Self {
            budget,
            used: 0,
            loaded: HashMap::new(),
            order: VecDeque::new(),
            failed: HashSet::new(),
        }
    }

    /// Returns the cached value for `path`, invoking `load` on a miss.
    /// `load` returns the value plus its eviction weight; `None` marks
    /// `path` failed until the next `clear`.
    pub(super) fn get_or_load(
        &mut self,
        path: &Path,
        load: impl FnOnce(&Path) -> Option<(V, u64)>,
    ) -> Option<V> {
        if self.failed.contains(path) {
            return None;
        }
        if let Some((v, _)) = self.loaded.get(path) {
            let v = v.clone();
            self.touch(path);
            return Some(v);
        }
        match load(path) {
            Some((v, weight)) => {
                self.loaded.insert(path.to_path_buf(), (v.clone(), weight));
                self.order.push_back(path.to_path_buf());
                self.used += weight;
                self.evict_to_budget();
                Some(v)
            }
            None => {
                self.failed.insert(path.to_path_buf());
                None
            }
        }
    }

    /// Drops every entry, successes and failures alike.
    pub(super) fn clear(&mut self) {
        self.loaded.clear();
        self.order.clear();
        self.failed.clear();
        self.used = 0;
    }

    fn touch(&mut self, path: &Path) {
        if let Some(pos) = self.order.iter().position(|p| p == path) {
            let key = self.order.remove(pos).expect("position came from iter");
            self.order.push_back(key);
        }
    }

    /// Evicts from the LRU end until the budget holds. The newest entry
    /// (back of `order`) survives even alone over budget: it is the one
    /// the caller is about to use.
    fn evict_to_budget(&mut self) {
        while self.used > self.budget && self.order.len() > 1 {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if let Some((_, weight)) = self.loaded.remove(&key) {
                self.used = self.used.saturating_sub(weight);
            }
        }
    }
}

#[cfg(test)]
#[path = "assembly_cache_tests.rs"]
mod tests;
