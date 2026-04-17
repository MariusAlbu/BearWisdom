// =============================================================================
// perl/keywords.rs — Perl primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Perl.
pub(crate) const KEYWORDS: &[&str] = &[
    // I/O
    "print", "say", "warn", "die",
    // string
    "chomp", "chop", "split", "join", "length", "substr", "index", "rindex",
    "lc", "uc", "lcfirst", "ucfirst", "hex", "oct", "ord", "chr",
    "sprintf", "printf", "pos", "quotemeta",
    // array
    "push", "pop", "shift", "unshift", "reverse", "sort", "grep", "map",
    // hash
    "keys", "values", "each", "exists", "delete", "defined",
    // references / OO
    "undef", "ref", "bless", "tie", "untie",
    // eval / exec
    "eval", "local",
    // declarations
    "my", "our", "use", "require", "no", "sub", "return",
    // flow
    "last", "next", "redo",
    "if", "elsif", "else", "unless", "while", "until",
    "for", "foreach", "do", "given", "when", "default",
    // file I/O
    "open", "close", "read", "write", "seek", "tell", "eof",
    "binmode", "truncate", "stat", "lstat",
    "rename", "unlink", "mkdir", "rmdir", "chdir",
    "opendir", "readdir", "closedir", "glob",
    "chmod", "chown",
    // misc
    "scalar", "wantarray", "caller",
    "abs", "int", "sqrt", "sin", "cos", "atan2", "exp", "log",
    "rand", "srand", "time", "localtime", "gmtime",
    "sleep", "alarm", "system", "exec",
    "fork", "wait", "waitpid", "kill",
    "pipe", "socket", "connect", "bind", "listen", "accept",
    "shutdown", "send", "recv", "fileno", "select",
    // special variables / handles
    "STDIN", "STDOUT", "STDERR", "ARGV", "ENV", "INC", "SIG",
    "@_", "$_", "$!", "$@", "$$", "$0", "$1", "$2", "$&",
    // common modules
    "Carp", "Scalar::Util", "List::Util",
    "File::Spec", "File::Path", "File::Basename", "File::Find",
    "Getopt::Long", "Data::Dumper", "Storable",
    "JSON", "YAML", "DBI", "LWP", "HTTP::Tiny", "URI", "Encode",
    "POSIX", "IO::File", "IO::Socket",
    "MIME::Base64", "Digest::MD5", "Digest::SHA",
    "Test::More", "Test::Deep",
    "constant", "strict", "warnings", "utf8", "feature",
    "Exporter", "Moo", "Moose", "Type::Tiny",
];
