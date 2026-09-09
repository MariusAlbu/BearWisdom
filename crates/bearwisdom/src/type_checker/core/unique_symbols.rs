//! A selected program/source owns a unique-symbol declaration origin. Physical
//! navigation rows and rendered names are not required to preserve its identity.
use super::NominalContextId;
use crate::types::SourceSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct UniqueSymbol {
    pub(super) context: NominalContextId,
    source: usize,
    declaration: SourceSpan,
}

impl UniqueSymbol {
    pub(crate) fn new(context: NominalContextId, source: usize, declaration: SourceSpan) -> Self {
        Self {
            context,
            source,
            declaration,
        }
    }
}

#[cfg(test)]
#[path = "unique_symbols_tests.rs"]
mod tests;
