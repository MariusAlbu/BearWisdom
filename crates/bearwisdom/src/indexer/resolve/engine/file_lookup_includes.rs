// =============================================================================
// file_lookup_includes — include-graph visibility forwarded to the compilation
// =============================================================================

use super::*;

impl<'a> super::super::contract::IncludeLookup for FileLookup<'a> {
    fn include_reaches(&self, source_file: &str, candidate_file: &str) -> bool {
        self.tree.include_reaches(source_file, candidate_file)
    }

    fn include_spec_resolves(&self, source_file: &str, spec: &str) -> bool {
        self.tree.include_spec_resolves(source_file, spec)
    }
}
