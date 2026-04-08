use std::collections::HashSet;

/// Runtime globals always external for Erlang.
/// Covers: BIFs (built-in functions), OTP behaviour callbacks, process
/// dictionary primitives, and common stdlib functions from lists/maps/io.
pub(crate) const EXTERNALS: &[&str] = &[
    // --- Erlang BIFs (erlang module, always available without import) ---
    "self", "send", "spawn", "spawn_link", "spawn_monitor",
    "link", "unlink", "monitor", "demonitor",
    "register", "unregister", "whereis", "registered",
    "is_atom", "is_binary", "is_bitstring", "is_boolean",
    "is_float", "is_function", "is_integer", "is_list",
    "is_map", "is_number", "is_pid", "is_port",
    "is_process_alive", "is_record", "is_reference", "is_tuple",
    "abs", "ceil", "floor", "round", "trunc", "max", "min",
    "length", "hd", "tl", "element", "setelement",
    "tuple_size", "tuple_to_list", "list_to_tuple",
    "map_size", "map_get", "map_put", "map_remove",
    "binary_to_list", "list_to_binary", "binary_to_atom",
    "atom_to_list", "list_to_atom", "atom_to_binary",
    "integer_to_list", "list_to_integer", "integer_to_binary",
    "float_to_list", "list_to_float", "float_to_binary",
    "term_to_binary", "binary_to_term",
    "size", "bit_size", "byte_size",
    "error", "exit", "throw", "halt",
    "get", "put", "erase", "get_keys",
    "apply", "fun_info", "function_exported",
    "module_info", "make_ref", "node", "nodes",
    "now", "monotonic_time", "system_time", "time",
    "process_info", "processes",
    "garbage_collect", "memory",
    "open_port", "port_command", "port_connect", "port_close",

    // --- OTP GenServer / GenEvent / Supervisor behaviour callbacks ---
    "init", "terminate", "code_change",
    "handle_call", "handle_cast", "handle_info", "handle_continue",
    "handle_event", "handle_sync_event",
    "format_status",
    // GenServer start helpers
    "start_link", "start", "stop", "child_spec",
    // Supervisor callback
    "init",

    // --- lists stdlib (most-used names, unqualified after import) ---
    "keyfind", "keystore", "keydelete", "keymember", "keysort",
    "member", "reverse", "flatten", "append", "concat",
    "map", "filter", "foldl", "foldr", "foreach",
    "sort", "usort", "merge", "subtract",
    "nth", "nthtail", "last", "sum", "max", "min",
    "splitwith", "partition", "zip", "unzip",
    "seq", "duplicate", "delete",

    // --- io / io_lib ---
    "format", "write", "read", "get_line",
    "fread", "fwrite",

    // --- maps stdlib ---
    "from_list", "to_list", "keys", "values",
    "find", "get", "put", "remove", "update", "merge",
    "fold", "map", "filter", "iterator", "next",

    // --- binary stdlib ---
    "split", "copy", "part", "decode",

    // --- json / jsone / thoas common top-level ---
    "encode", "decode",

    // --- common supervisor/application callbacks ---
    "start", "stop",

    // --- receive sugar (not real function names but appear in analysis) ---
    "recv", "receive",
];

/// Dependency-gated framework globals for Erlang.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // EUnit test framework
    for dep in ["eunit", "erlang/eunit"] {
        if deps.contains(dep) {
            globals.extend(EUNIT_GLOBALS);
            break;
        }
    }

    // Common Test framework
    if deps.contains("common_test") || deps.contains("ct") {
        globals.extend(CT_GLOBALS);
    }

    // PropEr / QuickCheck property testing
    for dep in ["proper", "eqc"] {
        if deps.contains(dep) {
            globals.extend(PROPER_GLOBALS);
            break;
        }
    }

    globals
}

const EUNIT_GLOBALS: &[&str] = &[
    "test", "assert", "assertEqual", "assertNotEqual",
    "assertMatch", "assertException", "assertError",
    "assertThrow", "assertExit",
    "setup", "foreach", "foreach",
];

const CT_GLOBALS: &[&str] = &[
    "all", "groups", "suite", "init_per_suite", "end_per_suite",
    "init_per_group", "end_per_group", "init_per_testcase", "end_per_testcase",
];

const PROPER_GLOBALS: &[&str] = &[
    "prop", "forall", "implies", "collect", "aggregate",
    "numtests", "verbose", "fails",
    "integer", "float", "atom", "binary", "list", "tuple",
    "oneof", "frequency", "elements",
];
