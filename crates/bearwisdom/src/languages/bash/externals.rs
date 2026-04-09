/// Bash shell builtins and ubiquitous external commands that are never defined
/// inside a project — used to classify unresolved call refs as "external"
/// rather than "unknown".
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // POSIX shell builtins
    // -------------------------------------------------------------------------
    "echo", "printf", "read", "test", "true", "false",
    "exit", "return", "break", "continue",
    "shift", "set", "unset", "export",
    "local", "declare", "typeset", "readonly",
    "eval", "exec", "trap", "wait", "kill",
    "cd", "pwd", "pushd", "popd", "dirs",
    "source", "alias", "unalias",
    "type", "hash", "command", "builtin", "enable",
    "getopts", "let", "mapfile", "readarray",
    "caller", "compgen", "complete", "compopt",
    "shopt", "bind", "help", "logout", "times",
    "umask", "ulimit",
    "fg", "bg", "jobs", "disown", "suspend", "coproc",
    // -------------------------------------------------------------------------
    // Common external commands used in scripts
    // -------------------------------------------------------------------------
    "grep", "sed", "awk", "find", "sort", "uniq", "wc",
    "cut", "tr", "head", "tail", "cat",
    "cp", "mv", "rm", "mkdir", "rmdir",
    "chmod", "chown", "ln", "ls",
    "date", "sleep",
    "curl", "wget", "ssh", "scp",
    "tar", "gzip", "gunzip", "zip", "unzip",
    "diff", "patch",
    "git", "docker", "make",
    "pip", "npm", "node", "python", "ruby", "perl", "java",
    "jq", "xargs", "tee",
];
