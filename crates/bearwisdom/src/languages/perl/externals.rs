/// Perl built-in functions and widely-used core/CPAN modules that are never
/// defined inside a project.
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Built-in functions
    // -------------------------------------------------------------------------
    // string
    "chomp", "chop", "chr", "crypt", "hex", "index", "rindex",
    "lc", "lcfirst", "length", "oct", "ord", "pack", "unpack",
    "reverse", "sprintf", "substr", "uc", "ucfirst",
    "pos", "quotemeta", "split", "study",
    // numeric
    "abs", "atan2", "cos", "exp", "int", "log", "rand", "sin", "sqrt", "srand",
    // list / array
    "pop", "push", "shift", "unshift", "splice",
    "sort", "grep", "map", "join", "wantarray",
    "each", "keys", "values",
    // hash / reference
    "exists", "delete", "defined", "undef", "scalar", "ref",
    "bless", "tie", "untie", "tied",
    // flow / eval
    "eval", "die", "warn", "exit", "caller", "import",
    // declaration keywords
    "local", "my", "our", "state",
    // I/O
    "open", "close", "read", "write", "print", "say", "printf",
    "seek", "tell", "eof", "binmode", "truncate",
    // filesystem
    "stat", "rename", "unlink", "rmdir", "mkdir", "chmod", "chown",
    "chroot", "chdir", "glob", "link",
    "opendir", "readdir", "closedir",
    // process
    "fork", "exec", "system", "kill", "wait", "waitpid", "alarm", "sleep",
    // time
    "time", "localtime", "gmtime",
    // -------------------------------------------------------------------------
    // Core modules (used via `use Module;`)
    // -------------------------------------------------------------------------
    "Carp", "Data::Dumper",
    "File::Basename", "File::Copy", "File::Find", "File::Path",
    "File::Spec", "File::Temp",
    "FindBin", "Getopt::Long",
    "List::Util", "POSIX", "Scalar::Util", "Storable",
    "Test::More", "Test::Simple",
    // -------------------------------------------------------------------------
    // Popular CPAN modules
    // -------------------------------------------------------------------------
    "JSON", "YAML", "DBI",
    "LWP", "HTTP::Tiny",
    "Moo", "Moose", "Try::Tiny",
];
