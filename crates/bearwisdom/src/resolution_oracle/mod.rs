//! Independent, source-addressed correctness evaluation for labelled fixtures.
//!
//! Labels are authored BEFORE observing the engine. Names, confidence scores,
//! qnames and database row IDs do not participate in target equality. Source
//! addresses are valid only within a pinned corpus revision; this is an oracle
//! identity domain, not a replacement for compiler SymbolId/BindingId.

use crate::types::{EdgeKind, SymbolKind};
use serde::{Deserialize, Serialize};

mod callable_policy;
pub(crate) mod compiler_intrinsic_policy;
mod evaluate;
pub mod project;
pub use evaluate::{compare, evaluate};

/// Stable fixture-file identity assigned by the corpus manifest, not SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FixtureFileId(pub u32);

/// SHA-256 of source + independently authored labels + file manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusRevision(pub [u8; 32]);

/// Extractor reference anchor. Offsets are zero-based UTF-8 bytes, not columns.
/// Distinct co-located references cannot be guessed apart: duplicates are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReferenceSite {
    pub file: FixtureFileId,
    pub byte_offset: u32,
    pub kind: EdgeKind,
}

/// An independently labelled declaration's start position and syntactic kind.
/// Coordinates are zero-based UTF-8 byte columns, matching the parser contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeclarationSite {
    pub file: FixtureFileId,
    pub line: u32,
    pub col: u32,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedReference {
    pub site: ReferenceSite,
    /// None is an explicitly labelled negative, not an unlabelled reference.
    pub target: Option<DeclarationSite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedBinding {
    Resolved(DeclarationSite),
    Unresolved,
    Drained,
    /// The log claimed resolution, but the target no longer exists.
    DanglingTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub site: ReferenceSite,
    /// Extracted, but no resolution outcome was recorded (e.g. missing owner).
    pub binding: Option<ObservedBinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Correct,
    Incorrect,
    CorrectUnbound,
    Unresolved,
    NotExtracted,
    MissingResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluatedReference {
    pub expected: ExpectedReference,
    pub actual: Option<ObservedBinding>,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OracleCounts {
    pub labelled: u64,
    pub expected_bound: u64,
    pub correct: u64,
    pub incorrect: u64,
    pub correct_unbound: u64,
    pub unresolved: u64,
    pub not_extracted: u64,
    pub missing_resolution: u64,
    /// Extracted sites without ground truth. Never silently treated as correct.
    pub unlabelled_observations: u64,
    pub binding_precision_percent: Option<f64>,
    pub correct_binding_recall_percent: Option<f64>,
    pub extraction_coverage_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OracleReport {
    pub revision: CorpusRevision,
    pub counts: OracleCounts,
    pub references: Vec<EvaluatedReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleChange {
    pub site: ReferenceSite,
    pub before: EvaluatedReference,
    pub after: EvaluatedReference,
    /// True even when two incorrect targets exchange and totals stay equal.
    pub retargeted: bool,
    pub regressed: bool,
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
mod ambient_tests;
#[cfg(test)]
mod corpus_tests;
#[cfg(test)]
mod fixture_support;
#[cfg(test)]
mod module_tests;
#[cfg(test)]
mod rust_tests;
#[cfg(test)]
mod scope_tests;
