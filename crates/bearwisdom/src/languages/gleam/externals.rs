/// Gleam standard library modules and OTP library names — always external
/// (never defined inside a project).
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // gleam/stdlib modules (short name = import alias)
    // -------------------------------------------------------------------------
    "gleam/io", "gleam/int", "gleam/float", "gleam/string",
    "gleam/list", "gleam/option", "gleam/result", "gleam/dict",
    "gleam/bool", "gleam/order", "gleam/dynamic",
    "gleam/bit_array", "gleam/bytes_builder",
    "gleam/iterator", "gleam/queue", "gleam/set",
    "gleam/uri", "gleam/regex", "gleam/yielder",
    // -------------------------------------------------------------------------
    // gleam_erlang / OTP
    // -------------------------------------------------------------------------
    "gleam/erlang", "gleam/erlang/process", "gleam/erlang/atom",
    "gleam/otp/actor", "gleam/otp/task", "gleam/otp/supervisor",
    // -------------------------------------------------------------------------
    // Short module aliases (after `import gleam/X` the alias is just `X`)
    // -------------------------------------------------------------------------
    "io", "int", "float", "string", "list", "option",
    "result", "dict", "bool", "order", "dynamic",
    "bit_array", "bytes_builder", "iterator", "queue",
    "set", "uri", "regex", "yielder",
    "process", "actor", "erlang",
];
