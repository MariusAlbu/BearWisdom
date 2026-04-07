// =============================================================================
// bash/primitives.rs — Bash primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Bash.
pub(crate) const PRIMITIVES: &[&str] = &[
    // shell builtins
    "echo", "printf", "read", "declare", "local", "export", "unset",
    "set", "shopt", "alias", "unalias", "type", "command", "builtin",
    "enable", "hash", "source", "eval", "exec", "trap",
    "return", "exit", "break", "continue", "shift",
    "getopts", "wait", "jobs", "bg", "fg", "kill", "disown", "suspend",
    "cd", "pwd", "pushd", "popd", "dirs",
    "let", "true", "false", "test", "[", "[[", "((", ":",".",
    "case", "esac", "if", "then", "elif", "else", "fi",
    "for", "while", "until", "do", "done", "in", "select",
    "function", "time", "coproc",
    "mapfile", "readarray",
    "compgen", "complete", "compopt",
    "caller", "help", "history", "fc", "bind", "logout",
    "umask", "ulimit", "times",
    // commonly used external commands
    "grep", "sed", "awk", "cut", "sort", "uniq", "wc",
    "head", "tail", "cat", "tee", "tr", "rev", "paste",
    "join", "comm", "diff", "patch",
    "find", "xargs",
    "ls", "cp", "mv", "rm", "mkdir", "rmdir", "touch",
    "chmod", "chown", "chgrp", "ln", "stat", "file",
    "readlink", "dirname", "basename", "realpath", "mktemp",
    "tput", "date", "sleep",
    "curl", "wget", "ssh", "scp", "rsync",
    "tar", "gzip", "gunzip", "zip", "unzip",
    "git", "docker", "make", "cmake",
    "npm", "pip", "python", "ruby", "node", "java",
    "go", "cargo", "rustc", "gcc", "g++", "clang",
    "jq", "yq", "bc", "expr", "seq", "yes",
    "timeout", "watch", "nohup", "screen", "tmux",
    "systemctl", "journalctl",
    "ps", "top", "htop", "free", "df", "du",
    "mount", "umount", "lsblk", "fdisk", "mkfs",
    "ip", "ifconfig", "netstat", "ss",
    "ping", "traceroute", "dig", "nslookup", "host",
    "iptables", "ufw", "firewall-cmd",
    "useradd", "userdel", "usermod", "groupadd", "passwd", "su", "sudo",
    "apt", "apt-get", "yum", "dnf", "pacman", "brew",
];
