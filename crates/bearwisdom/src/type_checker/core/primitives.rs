//! Scalar identity. Coarse inference categories are not exact source types.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum PrimKind {
    Int,
    Float,
    Str,
    Char,
    Bytes,
    Bool,
    Unit,
    Never,
    Symbol,
    Unknown,
    Signed(u16),
    Unsigned(u16),
    FloatWidth(u16),
    Isize,
    Usize,
}

#[cfg(test)]
#[path = "primitives_tests.rs"]
mod tests;
