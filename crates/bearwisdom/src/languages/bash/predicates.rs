// =============================================================================
// bash/predicates.rs — Bash builtin and helper predicates
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

/// Bash builtins and common external commands that are never in the index.
pub(super) fn is_bash_builtin(name: &str) -> bool {
    matches!(
        name,
        // POSIX / bash builtins
        "echo"
            | "printf"
            | "read"
            | "test"
            | "true"
            | "false"
            | "exit"
            | "return"
            | "break"
            | "continue"
            | "shift"
            | "set"
            | "unset"
            | "export"
            | "local"
            | "declare"
            | "typeset"
            | "readonly"
            | "eval"
            | "exec"
            | "trap"
            | "wait"
            | "kill"
            | "cd"
            | "pwd"
            | "pushd"
            | "popd"
            | "dirs"
            | "source"
            | "alias"
            | "unalias"
            | "type"
            | "hash"
            | "command"
            | "builtin"
            | "enable"
            | "getopts"
            | "let"
            | "mapfile"
            | "readarray"
            | "caller"
            | "compgen"
            | "complete"
            | "compopt"
            | "shopt"
            | "bind"
            | "help"
            | "logout"
            | "times"
            | "umask"
            | "ulimit"
            | "fg"
            | "bg"
            | "jobs"
            | "disown"
            | "suspend"
            | "coproc"
            // common external commands that appear ubiquitously in shell scripts
            | "grep"
            | "sed"
            | "awk"
            | "find"
            | "sort"
            | "uniq"
            | "wc"
            | "cut"
            | "tr"
            | "head"
            | "tail"
            | "cat"
            | "cp"
            | "mv"
            | "rm"
            | "mkdir"
            | "rmdir"
            | "chmod"
            | "chown"
            | "ln"
            | "ls"
            | "date"
            | "sleep"
            | "curl"
            | "wget"
            | "ssh"
            | "scp"
            | "tar"
            | "gzip"
            | "gunzip"
            | "zip"
            | "unzip"
            | "diff"
            | "patch"
            | "git"
            | "docker"
            | "make"
            | "pip"
            | "npm"
            | "node"
            | "python"
            | "ruby"
            | "perl"
            | "java"
            | "jq"
            | "xargs"
            | "tee"
    )
}
