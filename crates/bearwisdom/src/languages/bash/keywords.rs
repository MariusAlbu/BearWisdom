// =============================================================================
// bash/keywords.rs — Bash primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Bash.
pub(crate) const KEYWORDS: &[&str] = &[
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
    // Additional POSIX/coreutils + common shell-script tooling. The
    // pattern across `oh-my-bash`, `bash-it`, completion frameworks, and
    // dotfile repos is to lean on a wider command vocabulary than the
    // line above covered. Without these, every script using `id`,
    // `uname`, `infocmp`, `dig`, `hostname`, etc. surfaces them as
    // unresolved.
    "id", "uname", "hostname", "whoami", "tty", "logname",
    "infocmp", "tic", "stty", "reset", "clear",
    "env", "printenv", "locale", "iconv",
    "md5sum", "sha1sum", "sha256sum", "sha512sum", "cksum", "b2sum",
    "openssl", "gpg", "ssh-keygen", "ssh-add", "ssh-agent",
    "lsof", "fuser", "strace", "ltrace", "gdb",
    "which", "whereis", "locate", "updatedb",
    "uptime", "dmesg", "lscpu", "lsmem", "lspci", "lsusb",
    "vmstat", "iostat", "sar",
    "useradd", "groupadd", "groupmod", "groupdel",
    "tput", "tabs",
    "mkfifo", "mknod",
    "fold", "expand", "unexpand", "fmt", "nl", "pr",
    "od", "hexdump", "xxd",
    "split", "csplit",
    "bash", "sh", "zsh", "fish", "ksh",
    // External developer tooling commonly invoked from shell scripts —
    // not POSIX, but always external from the user project's POV.
    "vagrant", "vboxmanage", "virtualbox", "qemu",
    "docker-compose", "docker-machine", "podman", "kubectl", "helm", "minikube",
    "terraform", "ansible", "ansible-playbook", "puppet", "chef",
    "hg", "svn", "bzr", "fossil",
    "rbenv", "pyenv", "nvm", "nodenv", "asdf",
    "gem", "bundle", "composer", "yarn", "pnpm",
    "rake", "gulp", "grunt",
    "mvn", "gradle", "ant",
    "ag", "rg", "fzf", "fd", "bat", "tree",
    "vim", "nvim", "emacs", "nano", "less", "more",
];
