//! Shared vocabulary for resolver instrumentation, not semantic identity.
//!
//! Every extracted reference receives one disposition. A `Resolved` outcome
//! is the engine's claim, not independent evidence that the target is correct.

use serde::{Deserialize, Serialize};

use crate::types::EdgeKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Resolved,
    Unresolved,
    Drained,
    Primitive,
    Duplicate,
    MissingSourceSymbol,
    MissingSourceId,
    UnsupportedLanguage,
}

/// A bounded aggregate of occurrences with the same measurement attributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OccurrenceBucket {
    /// Extraction origin, including embedded languages, not a guessed project language.
    pub language: String,
    pub kind: EdgeKind,
    pub from_snippet: bool,
    pub disposition: Disposition,
    pub count: u64,
}

/// Exhaustive partition: sum of these fields equals extracted references.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OccurrenceCounts {
    pub resolved: u64,
    pub unresolved: u64,
    pub drained: u64,
    pub primitive: u64,
    pub duplicate: u64,
    pub missing_source_symbol: u64,
    pub missing_source_id: u64,
    pub unsupported_language: u64,
}

impl OccurrenceCounts {
    pub fn add(&mut self, disposition: Disposition, count: u64) {
        let slot = match disposition {
            Disposition::Resolved => &mut self.resolved,
            Disposition::Unresolved => &mut self.unresolved,
            Disposition::Drained => &mut self.drained,
            Disposition::Primitive => &mut self.primitive,
            Disposition::Duplicate => &mut self.duplicate,
            Disposition::MissingSourceSymbol => &mut self.missing_source_symbol,
            Disposition::MissingSourceId => &mut self.missing_source_id,
            Disposition::UnsupportedLanguage => &mut self.unsupported_language,
        };
        *slot += count;
    }

    pub fn skipped(&self) -> u64 {
        self.missing_source_symbol + self.missing_source_id + self.unsupported_language
    }

    pub fn total(&self) -> u64 {
        self.resolved
            + self.unresolved
            + self.drained
            + self.primitive
            + self.duplicate
            + self.skipped()
    }

    /// Coverage among eligible extracted occurrences. Skips remain in the
    /// denominator. Empty input is unknown, never a synthetic 100%.
    pub fn binding_coverage_percent(&self) -> Option<f64> {
        let eligible = self.resolved + self.unresolved + self.skipped();
        (eligible > 0).then(|| self.resolved as f64 * 100.0 / eligible as f64)
    }
}

#[cfg(test)]
#[path = "occurrence_tests.rs"]
mod tests;
