// =============================================================================
// haskell/predicates.rs — Haskell builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Haskell Prelude functions and types always in scope without an import.
pub(super) fn is_haskell_builtin(name: &str) -> bool {
    matches!(
        name,
        // I/O
        "show"
            | "print"
            | "putStrLn"
            | "putStr"
            | "getLine"
            | "getContents"
            | "readLn"
            | "read"
            // Error / bottom
            | "error"
            | "undefined"
            | "seq"
            // Higher-order
            | "id"
            | "const"
            | "flip"
            // List operations
            | "map"
            | "filter"
            | "foldl"
            | "foldr"
            | "foldl'"
            | "scanl"
            | "scanr"
            | "zip"
            | "unzip"
            | "zipWith"
            | "head"
            | "tail"
            | "last"
            | "init"
            | "null"
            | "length"
            | "reverse"
            | "concat"
            | "concatMap"
            | "and"
            | "or"
            | "any"
            | "all"
            | "sum"
            | "product"
            | "maximum"
            | "minimum"
            | "take"
            | "drop"
            | "splitAt"
            | "span"
            | "break"
            | "elem"
            | "notElem"
            | "lookup"
            | "iterate"
            | "repeat"
            | "replicate"
            | "cycle"
            // Enum / Ord
            | "succ"
            | "pred"
            | "toEnum"
            | "fromEnum"
            | "minBound"
            | "maxBound"
            | "compare"
            | "max"
            | "min"
            // Numeric
            | "negate"
            | "abs"
            | "signum"
            | "fromInteger"
            | "toRational"
            | "fromRational"
            | "truncate"
            | "round"
            | "ceiling"
            | "floor"
            | "div"
            | "mod"
            | "quot"
            | "rem"
            | "divMod"
            | "quotRem"
            | "recip"
            | "pi"
            | "exp"
            | "log"
            | "sqrt"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            // Monad / Applicative
            | "return"
            | "pure"
            | "fmap"
            | "sequence"
            | "mapM"
            | "mapM_"
            | "forM"
            | "forM_"
            | "fail"
            | "mzero"
            | "mplus"
            | "guard"
            | "when"
            | "unless"
            | "join"
            | "forever"
            | "void"
            | "fix"
            | "on"
            // Operators (as names)
            | "$"
            | "."
            | "++"
            | ">>="
            | ">>"
            | "<$>"
            | "<*>"
            | "<|>"
            // Core types
            | "IO"
            | "Maybe"
            | "Just"
            | "Nothing"
            | "Either"
            | "Left"
            | "Right"
            | "Bool"
            | "True"
            | "False"
            | "Int"
            | "Integer"
            | "Float"
            | "Double"
            | "Char"
            | "String"
            | "Ordering"
            | "LT"
            | "EQ"
            | "GT"
    )
}
