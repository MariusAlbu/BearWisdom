// =============================================================================
// matlab/builtins.rs — MATLAB builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(sym_kind, "class" | "interface" | "enum" | "type_alias" | "function" | "variable"),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// MATLAB builtin functions and types always in scope.
pub(super) fn is_matlab_builtin(name: &str) -> bool {
    matches!(
        name,
        // I/O
        "disp"
            | "fprintf"
            | "sprintf"
            | "error"
            | "warning"
            | "assert"
            // Array info
            | "length"
            | "size"
            | "numel"
            | "ndims"
            | "reshape"
            // Array construction
            | "zeros"
            | "ones"
            | "eye"
            | "rand"
            | "randn"
            | "linspace"
            | "logspace"
            // Plotting
            | "plot"
            | "figure"
            | "subplot"
            | "hold"
            | "xlabel"
            | "ylabel"
            | "title"
            | "legend"
            | "grid"
            | "axis"
            | "xlim"
            | "ylim"
            | "close"
            // File I/O
            | "save"
            | "load"
            | "fopen"
            | "fclose"
            | "fread"
            | "fwrite"
            | "fscanf"
            | "fgets"
            | "fgetl"
            // Type checks
            | "exist"
            | "isempty"
            | "isnumeric"
            | "ischar"
            | "islogical"
            | "iscell"
            | "isstruct"
            | "class"
            | "typecast"
            | "cast"
            // Data structures
            | "cell"
            | "struct"
            | "fieldnames"
            | "rmfield"
            | "cellfun"
            | "arrayfun"
            | "structfun"
            // Set / search
            | "find"
            | "sort"
            | "unique"
            | "intersect"
            | "union"
            | "setdiff"
            | "ismember"
            // Stats
            | "min"
            | "max"
            | "sum"
            | "prod"
            | "mean"
            | "median"
            | "std"
            | "var"
            // Math
            | "abs"
            | "sqrt"
            | "exp"
            | "log"
            | "log10"
            | "log2"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
            | "ceil"
            | "floor"
            | "round"
            | "fix"
            | "mod"
            | "rem"
            | "power"
            // Linear algebra
            | "cross"
            | "dot"
            | "norm"
            | "det"
            | "inv"
            | "eig"
            | "svd"
            | "rank"
            | "pinv"
            | "null"
            | "orth"
            | "trace"
            | "transpose"
            | "ctranspose"
            // Array manipulation
            | "cat"
            | "horzcat"
            | "vertcat"
            | "repmat"
            | "kron"
            | "fliplr"
            | "flipud"
            | "rot90"
            | "squeeze"
            | "permute"
            | "ipermute"
    )
}
