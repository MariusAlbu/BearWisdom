// =============================================================================
// dockerfile/primitives.rs — Dockerfile primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Dockerfile.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Dockerfile instructions
    "FROM", "RUN", "CMD", "ENTRYPOINT", "COPY", "ADD",
    "ENV", "ARG", "EXPOSE", "VOLUME", "WORKDIR", "USER",
    "LABEL", "STOPSIGNAL", "HEALTHCHECK", "SHELL",
    "ONBUILD", "MAINTAINER",
    // shell builtins available in RUN
    "echo", "printf", "read", "export", "unset", "set",
    "source", "eval", "exec", "trap",
    "return", "exit", "break", "continue", "shift",
    "cd", "pwd", "true", "false", "test", "[",
    "if", "then", "elif", "else", "fi",
    "for", "while", "until", "do", "done", "in",
    "case", "esac", "function",
    // common commands used in RUN layers
    "grep", "sed", "awk", "cut", "sort", "uniq", "wc",
    "head", "tail", "cat", "tee", "tr",
    "find", "xargs",
    "ls", "cp", "mv", "rm", "mkdir", "rmdir", "touch",
    "chmod", "chown", "ln", "stat",
    "dirname", "basename",
    "curl", "wget",
    "tar", "gzip", "gunzip", "zip", "unzip",
    "apt", "apt-get", "yum", "dnf", "apk", "pacman",
    "pip", "pip3", "npm", "yarn", "go",
    "make", "cmake",
    "useradd", "userdel", "groupadd", "passwd",
    "date", "sleep",
    "git",
    "jq",
];
