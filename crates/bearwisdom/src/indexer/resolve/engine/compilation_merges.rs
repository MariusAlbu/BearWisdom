//! Scoped declaration evidence takes precedence over legacy merge-name seeds.
use super::*;

impl Compilation {
    pub(super) fn capture_scoped_merges(&mut self, files: &[ParsedFile], ids: &SymbolIds) {
        for file in files {
            self.attest_scoped_merges(&super::super::module_input::scoped_declarations(file, ids));
        }
    }

    pub(super) fn attest_scoped_merges(&mut self, groups: &[Vec<i64>]) {
        for group in groups {
            let Some(&canonical) = group.first() else {
                continue;
            };
            self.merge_groups.attested.extend(
                group
                    .iter()
                    .copied()
                    .filter(|id| self.by_id.contains_key(id)),
            );
            if !group.iter().all(|id| self.by_id.contains_key(id)) {
                continue;
            }
            self.merge_groups.scoped.insert(canonical, group.clone());
        }
    }

    /// Merge membership is ID-only. Original member rows remain navigation
    /// targets; only their owning type's identity and member buckets are folded.
    pub(super) fn finish_identity_passes(&mut self) {
        self.merge_canonical = super::super::merge_canonical::compute(&self.merge_groups);
        super::super::merge_canonical::apply(
            &self.merge_canonical,
            &mut self.members_by_id,
            &mut self.inherits_by_id,
            &mut self.inherits_args_by_pair,
            &mut self.enclosing_type_by_id,
        );
        super::super::merge_canonical::fold_type_info(
            &self.merge_canonical,
            &mut self.type_info_by_id,
        );
        self.member_index.rebuild(&self.members_by_id, &self.by_id);
    }
}

#[cfg(test)]
#[path = "compilation_merges_tests.rs"]
mod tests;
