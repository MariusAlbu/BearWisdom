//! Language-level atomic identities, distinct from unresolved engine evidence.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Intrinsic {
    Any,
    Unknown,
    Never,
    Void,
    Undefined,
    Null,
    Object,
    String,
    Number,
    Boolean,
    Symbol,
    BigInt,
}

impl Intrinsic {
    /// Display only. Semantic consumers carry the enum or its interned TypeId.
    pub fn display(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Unknown => "unknown",
            Self::Never => "never",
            Self::Void => "void",
            Self::Undefined => "undefined",
            Self::Null => "null",
            Self::Object => "object",
            Self::String => "string",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Symbol => "symbol",
            Self::BigInt => "bigint",
        }
    }
}

#[cfg(test)]
#[path = "intrinsics_tests.rs"]
mod tests;
