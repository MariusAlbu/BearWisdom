//! Explicit configuration inputs. Paths and fingerprints are ingestion data,
//! not semantic identities or a request to discover files heuristically.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Program {
    /// Stable caller-owned configuration address, distinct from its contents.
    pub key: String,
    /// Fingerprint of all configuration inputs, options and source selection.
    pub fingerprint: String,
    /// False when configuration discovery could have omitted a provider.
    pub complete: bool,
    /// Attested signature-assignment rules. Missing legacy inputs stay unknown.
    #[serde(default)]
    pub callable_policy: Option<CallablePolicy>,
    /// Effective compiler-provided alias semantics; missing evidence stays unknown.
    #[serde(default)]
    pub compiler_intrinsics: Option<CompilerIntrinsicPolicy>,
    /// Compiler-attested binding order, independent of file IDs or input sorting.
    /// Must contain every configured source exactly once; old callers stay unknown.
    #[serde(default)]
    pub source_binding_order: Option<Vec<String>>,
    pub sources: Vec<ProgramSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallablePolicy {
    pub strict_parameters: bool,
    pub strict_nulls: bool,
    /// Whether method declarations are exempt from strict parameter variance.
    /// Old callers lack this evidence; it is not inferred from display syntax.
    #[serde(default)]
    pub bivariant_methods: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerIntrinsicPolicy {
    pub strict_iterator_return: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramSource {
    /// Exact index source address, including any external source-instance prefix.
    pub path: String,
    pub content_hash: String,
    pub scope: SourceScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceScope {
    /// The caller has applied configuration-level module detection; use syntax.
    Syntax,
    /// Configuration forces module isolation, even without import/export syntax.
    Module,
    /// Configuration has not established whether root declarations are global.
    Unknown,
}

#[cfg(test)]
#[path = "programs_tests.rs"]
mod tests;
