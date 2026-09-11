// =============================================================================
// compilation_includes — include-graph visibility answered by the closure
// =============================================================================

use super::*;

impl super::super::contract::IncludeLookup for Compilation {
    fn include_reaches(&self, source_file: &str, candidate_file: &str) -> bool {
        self.include_closure.reaches(source_file, candidate_file)
    }

    fn include_spec_resolves(&self, source_file: &str, spec: &str) -> bool {
        self.include_closure.spec_resolves(source_file, spec)
    }
}
