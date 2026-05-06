// =============================================================================
// perl/keywords.rs — Perl language keywords + interpreter built-ins
//
// Names that are ALWAYS in scope without a `use` statement and are
// implemented inside the perl interpreter (C source, not walkable as
// Perl source). Core *modules* (Carp, Data::Dumper, File::Path, ...)
// are handled by the perl_stdlib walker — they live as .pm text files
// under <perl_root>/lib/<ver>/ and emit real symbols.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // I/O built-ins (interpreter ops)
    "print", "say", "printf", "warn", "die",
    // string built-ins
    "chomp", "chop", "split", "join", "length", "substr", "index", "rindex",
    "lc", "uc", "lcfirst", "ucfirst", "hex", "oct", "ord", "chr",
    "sprintf", "pos", "quotemeta", "reverse", "study",
    "pack", "unpack",
    // array / list built-ins
    "push", "pop", "shift", "unshift", "splice", "sort", "grep", "map",
    // hash built-ins
    "keys", "values", "each", "exists", "delete", "defined",
    // references / OO built-ins
    "undef", "ref", "bless", "tie", "untie", "tied", "scalar", "wantarray",
    // declaration keywords
    "my", "our", "local", "state", "use", "no", "require", "sub", "return",
    "package",
    // flow keywords
    "if", "elsif", "else", "unless", "while", "until",
    "for", "foreach", "do", "given", "when", "default",
    "last", "next", "redo",
    // eval / exception built-ins
    "eval", "die", "warn", "caller",
    // I/O file-handle built-ins
    "open", "close", "read", "write", "seek", "tell", "eof",
    "binmode", "truncate", "fileno", "select",
    // filesystem built-ins
    "stat", "lstat", "rename", "unlink", "mkdir", "rmdir", "chdir",
    "opendir", "readdir", "closedir", "glob",
    "chmod", "chown", "chroot", "link",
    // process built-ins
    "fork", "exec", "system", "kill", "wait", "waitpid", "alarm", "sleep",
    "exit", "import",
    // network built-ins
    "pipe", "socket", "connect", "bind", "listen", "accept",
    "shutdown", "send", "recv",
    // numeric / math built-ins
    "abs", "int", "sqrt", "sin", "cos", "atan2", "exp", "log",
    "rand", "srand",
    // time built-ins
    "time", "localtime", "gmtime",
    // pragmas (recognized at parse time, not walker-covered)
    "constant", "strict", "warnings", "utf8", "feature",
    // special variables / handles (extractor sometimes emits these as refs)
    "STDIN", "STDOUT", "STDERR", "ARGV", "ENV", "INC", "SIG",
    "@_", "$_", "$!", "$@", "$$", "$0",
];
