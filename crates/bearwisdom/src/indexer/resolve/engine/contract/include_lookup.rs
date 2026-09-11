// =============================================================================
// contract/include_lookup — include-graph visibility queries
// =============================================================================

/// What a lookup knows about a translation unit's `#include` graph. Closed by
/// default so other languages and synthetic stores never gain include-based
/// visibility; a compilation with an include closure answers both queries.
pub trait IncludeLookup {
    /// Whether `source_file` reaches `candidate_file` through its transitive,
    /// uniquely-resolved `#include` graph.
    fn include_reaches(&self, _source_file: &str, _candidate_file: &str) -> bool {
        false
    }

    /// Whether `source_file`'s include spelling `spec` names exactly one
    /// indexed file.
    fn include_spec_resolves(&self, _source_file: &str, _spec: &str) -> bool {
        false
    }
}
