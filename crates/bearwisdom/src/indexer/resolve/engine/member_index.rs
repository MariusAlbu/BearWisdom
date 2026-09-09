//! Structural ingestion owns spellings; nominal member queries own numeric keys.
use super::contract::Symbol;
use rustc_hash::FxHashMap;

/// Snapshot-local interned member name, never a declaration identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemberNameId(usize);

#[derive(Default)]
pub struct MemberIndex {
    names: FxHashMap<String, MemberNameId>,
    rows: FxHashMap<(i64, MemberNameId), Vec<i64>>,
}

impl MemberIndex {
    /// Preserve the source name arena while replacing ownership with one
    /// configured program's attested canonical rows. No member-name decoding.
    pub(crate) fn project(&self, canonical: &FxHashMap<i64, i64>) -> Self {
        let mut projected = Self {
            names: self.names.clone(),
            rows: FxHashMap::default(),
        };
        for (&(owner, name), members) in &self.rows {
            let Some(&owner) = canonical.get(&owner) else {
                continue;
            };
            let rows = projected.rows.entry((owner, name)).or_default();
            rows.extend(
                members
                    .iter()
                    .filter(|id| canonical.contains_key(id))
                    .copied(),
            );
            rows.sort_unstable();
            rows.dedup();
        }
        projected
    }
    pub fn record(&mut self, owner: i64, spelling: &str, declaration: i64) {
        let name = self.intern_name(spelling);
        let rows = self.rows.entry((owner, name)).or_default();
        if !rows.contains(&declaration) {
            rows.push(declaration);
        }
    }

    /// Restore declared rows in a projection of this source index, preserving
    /// the projection's appended source-name IDs across private proof rounds.
    pub(crate) fn restore_projection(&mut self, source: &Self, canonical: &FxHashMap<i64, i64>) {
        self.rows = source.project(canonical).rows;
    }

    /// Source-owned members can have a name without a physical navigation row.
    pub(crate) fn intern_name(&mut self, spelling: &str) -> MemberNameId {
        let next = MemberNameId(self.names.len());
        *self.names.entry(spelling.into()).or_insert(next)
    }

    /// Keep name IDs stable within this store while replacing all owner edges.
    pub(crate) fn rebuild(
        &mut self,
        members: &FxHashMap<i64, Vec<i64>>,
        symbols: &FxHashMap<i64, Symbol>,
    ) {
        self.rows.clear();
        for (&owner, rows) in members {
            for &id in rows {
                if let Some(symbol) = symbols.get(&id) {
                    self.record(owner, &symbol.name, id);
                }
            }
        }
    }

    pub fn name(&self, spelling: &str) -> Option<MemberNameId> {
        self.names.get(spelling).copied()
    }
    pub fn candidates(&self, owner: i64, name: MemberNameId) -> &[i64] {
        self.rows
            .get(&(owner, name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub(crate) fn declared(&self, owner: i64, name: MemberNameId) -> bool {
        self.rows.contains_key(&(owner, name))
    }

    /// Only a proved effective surface may project inherited navigation rows.
    /// An empty entry is an authoritative rowless declaration, not permission
    /// to borrow a deeper physical member with the same interned name.
    pub(crate) fn project_member(&mut self, owner: i64, name: MemberNameId, mut rows: Vec<i64>) {
        rows.sort_unstable();
        rows.dedup();
        self.rows.insert((owner, name), rows);
    }

    /// Rebind attested source rows without retaining their old display-name
    /// aliases. Unmigrated member forms keep their ingestion-owned rows.
    pub(crate) fn project_source_members(
        &mut self,
        sources: &FxHashMap<i64, FxHashMap<MemberNameId, Vec<i64>>>,
    ) {
        let replaced: FxHashMap<_, rustc_hash::FxHashSet<_>> = sources
            .iter()
            .map(|(&owner, members)| (owner, members.values().flatten().copied().collect()))
            .collect();
        // One pass over the index, not one full scan per source class.
        self.rows.retain(|&(parent, _), rows| {
            let Some(replaced) = replaced.get(&parent) else {
                return true;
            };
            if rows.is_empty() {
                return true;
            }
            rows.retain(|row| !replaced.contains(row));
            !rows.is_empty()
        });
        for (&owner, members) in sources {
            for (&name, rows) in members {
                self.project_member(owner, name, rows.clone());
            }
        }
    }
}

#[cfg(test)]
#[path = "member_index_tests.rs"]
mod tests;
