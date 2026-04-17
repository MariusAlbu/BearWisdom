// =============================================================================
// perl/predicates.rs — Perl builtin and helper predicates
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

/// Perl built-in functions that are never in the project index.
pub(super) fn is_perl_builtin(name: &str) -> bool {
    matches!(
        name,
        // string functions
        "chomp"
            | "chop"
            | "chr"
            | "crypt"
            | "hex"
            | "index"
            | "rindex"
            | "lc"
            | "lcfirst"
            | "length"
            | "oct"
            | "ord"
            | "pack"
            | "unpack"
            | "reverse"
            | "sprintf"
            | "substr"
            | "uc"
            | "ucfirst"
            | "pos"
            | "quotemeta"
            | "split"
            | "study"
            // numeric / math
            | "abs"
            | "atan2"
            | "cos"
            | "exp"
            | "int"
            | "log"
            | "rand"
            | "sin"
            | "sqrt"
            | "srand"
            // list / array
            | "pop"
            | "push"
            | "shift"
            | "unshift"
            | "splice"
            | "sort"
            | "grep"
            | "map"
            | "join"
            | "wantarray"
            | "each"
            | "keys"
            | "values"
            // hash / reference
            | "exists"
            | "delete"
            | "defined"
            | "undef"
            | "scalar"
            | "ref"
            | "bless"
            | "tie"
            | "untie"
            | "tied"
            // flow / eval
            | "eval"
            | "die"
            | "warn"
            | "exit"
            | "caller"
            | "import"
            // variable declaration keywords (parser emits these as calls)
            | "local"
            | "my"
            | "our"
            | "state"
            // I/O
            | "open"
            | "close"
            | "read"
            | "write"
            | "print"
            | "say"
            | "printf"
            | "seek"
            | "tell"
            | "eof"
            | "binmode"
            | "truncate"
            // filesystem
            | "stat"
            | "rename"
            | "unlink"
            | "rmdir"
            | "mkdir"
            | "chmod"
            | "chown"
            | "chroot"
            | "chdir"
            | "glob"
            | "link"
            | "opendir"
            | "readdir"
            | "closedir"
            // process
            | "fork"
            | "exec"
            | "system"
            | "kill"
            | "wait"
            | "waitpid"
            | "alarm"
            | "sleep"
            // time
            | "time"
            | "localtime"
            | "gmtime"
    )
}
