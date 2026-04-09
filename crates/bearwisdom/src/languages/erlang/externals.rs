use std::collections::HashSet;

/// Runtime globals always external for Erlang.
/// Covers: BIFs (built-in functions), OTP behaviour callbacks, process
/// dictionary primitives, common stdlib functions, and OTP module names.
pub(crate) const EXTERNALS: &[&str] = &[
    // ── Erlang BIFs (erlang module, always available without import) ──────────
    "self", "send", "spawn", "spawn_link", "spawn_monitor", "spawn_opt",
    "link", "unlink", "monitor", "demonitor", "monitor_node",
    "register", "unregister", "whereis", "registered",
    "is_atom", "is_binary", "is_bitstring", "is_boolean",
    "is_float", "is_function", "is_integer", "is_list",
    "is_map", "is_map_key", "is_number", "is_pid", "is_port",
    "is_process_alive", "is_record", "is_reference", "is_tuple",
    "abs", "ceil", "floor", "round", "trunc", "max", "min",
    "length", "hd", "tl", "element", "setelement",
    "tuple_size", "tuple_to_list", "list_to_tuple",
    "map_size", "map_get", "map_put", "map_remove", "map_update",
    "binary_to_list", "list_to_binary", "binary_to_atom", "binary_to_existing_atom",
    "atom_to_list", "list_to_atom", "list_to_existing_atom", "atom_to_binary",
    "integer_to_list", "list_to_integer", "integer_to_binary", "binary_to_integer",
    "float_to_list", "list_to_float", "float_to_binary", "binary_to_float",
    "term_to_binary", "binary_to_term", "term_to_iovec",
    "size", "bit_size", "byte_size", "iolist_size",
    "iolist_to_binary", "iolist_to_iovec",
    "error", "exit", "throw", "halt", "nif_error",
    "get", "put", "erase", "get_keys",
    "apply", "fun_info", "fun_to_list", "function_exported",
    "module_info", "make_ref", "unique_integer",
    "node", "nodes", "node_info",
    "now", "monotonic_time", "system_time", "time", "time_offset",
    "convert_time_unit", "timestamp",
    "process_info", "processes", "process_flag",
    "garbage_collect", "memory", "statistics",
    "open_port", "port_command", "port_connect", "port_close",
    "port_control", "port_info", "ports",
    "group_leader", "set_group_leader",
    "check_process_code", "purge_module", "load_module",
    "send_after", "start_timer", "cancel_timer", "read_timer",
    "phash", "phash2",
    "display", "display_nl", "display_string",
    "bump_reductions", "system_flag", "system_info",
    "hibernate", "yield",
    "raise",

    // ── OTP GenServer / GenStatem / GenEvent behaviour callbacks ─────────────
    "init", "terminate", "code_change",
    "handle_call", "handle_cast", "handle_info", "handle_continue",
    "handle_event", "handle_sync_event", "handle_common_event",
    "format_status",
    // GenServer / GenStatem start helpers
    "start_link", "start", "stop", "call", "cast", "reply",
    "multi_call", "abcast", "enter_loop",
    // GenStatem-specific
    "callback_mode", "state_enter",
    // Supervisor callbacks
    "child_spec", "init",

    // ── OTP module names (appear as qualified call targets) ───────────────────
    "gen_server", "gen_statem", "gen_event", "gen_fsm",
    "supervisor", "supervisor_bridge",
    "application", "application_controller",
    "proc_lib", "sys", "release_handler",
    "gen_tcp", "gen_udp", "gen_sctp", "ssl",
    "inet", "inet_res", "inet_parse",
    "file", "file_handle", "ram_file", "disk_log",
    "io", "io_lib", "string", "binary", "re", "unicode",
    "calendar", "timer",
    "ets", "dets", "mnesia",
    "crypto", "public_key", "ssh", "ssh_channel", "ssh_client_channel",
    "httpc", "httpd", "inets",
    "logger", "error_logger", "error_handler",
    "os", "code", "erlang",
    "beam_lib", "erts_debug",
    "gb_trees", "gb_sets", "ordsets", "orddict", "dict", "queue",
    "rand", "math", "sets",
    "uri_string", "http_uri",
    "proplists", "maps", "lists",
    "filename", "filelib",
    "zlib", "erl_tar",
    "pg", "pg2",
    "rpc", "erpc",
    "net_adm", "net_kernel", "net",
    "global", "global_group",
    "persistent_term", "atomics", "counters",
    "ets", "dets", "mnesia",
    "wx", "wx_object",
    "xmerl", "xmerl_scan", "xmerl_xs",
    "ssh_sftp", "ssh_scp",
    "snmp",
    "observer",

    // ── lists stdlib (most-used names, unqualified after import) ─────────────
    "keyfind", "keystore", "keydelete", "keymember", "keysort", "keymerge",
    "member", "reverse", "flatten", "append", "concat",
    "map", "filter", "foldl", "foldr", "foreach", "mapfoldl", "mapfoldr",
    "sort", "usort", "merge", "subtract", "umerge",
    "nth", "nthtail", "last", "sum",
    "splitwith", "partition", "zip", "unzip", "zip3",
    "seq", "duplicate", "delete", "dropwhile", "takewhile",
    "all", "any", "flatlength", "prefix", "suffix",
    "search", "enumerate", "uniq",

    // ── io / io_lib ──────────────────────────────────────────────────────────
    "format", "write", "read", "get_line", "get_chars",
    "fread", "fwrite", "nl", "columns", "rows",

    // ── maps stdlib ──────────────────────────────────────────────────────────
    "from_list", "to_list", "keys", "values",
    "find", "get", "put", "remove", "update", "merge", "merge_with",
    "fold", "map", "filter", "iterator", "next",
    "groups_from_list", "from_keys",
    "intersect", "intersect_with",

    // ── binary stdlib ────────────────────────────────────────────────────────
    "split", "copy", "part", "decode",
    "compile_pattern", "match", "matches", "replace",
    "encode_unsigned", "decode_unsigned",
    "longest_common_prefix", "longest_common_suffix",
    "bin_to_list", "list_to_bin",

    // ── string stdlib ────────────────────────────────────────────────────────
    "length", "to_upper", "to_lower", "titlecase", "casefold",
    "equal", "concat", "chr", "substr", "slice",
    "trim", "trim_leading", "trim_trailing",
    "split", "join", "replace", "find",
    "prefix", "lexemes", "tokens",
    "to_graphemes", "to_float", "to_integer",

    // ── json / jsone / thoas / jsx common top-level ───────────────────────────
    "encode", "decode",

    // ── common supervisor/application callbacks ───────────────────────────────
    "start", "stop",

    // ── receive sugar (not real function names but appear in analysis) ────────
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

    // Cowboy HTTP server
    if deps.contains("cowboy") {
        globals.extend(COWBOY_GLOBALS);
    }

    // Phoenix / Plug (via rebar3 deps)
    for dep in ["plug", "cowboy_plug"] {
        if deps.contains(dep) {
            globals.extend(PLUG_GLOBALS);
            break;
        }
    }

    globals
}

const EUNIT_GLOBALS: &[&str] = &[
    "test", "assert", "assertEqual", "assertNotEqual",
    "assertMatch", "assertException", "assertError",
    "assertThrow", "assertExit",
    "setup", "foreach",
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

const COWBOY_GLOBALS: &[&str] = &[
    "init", "terminate", "handle",
    "websocket_init", "websocket_handle", "websocket_info", "websocket_terminate",
    "rest_init", "allowed_methods", "content_types_provided", "content_types_accepted",
    "is_authorized", "forbidden", "resource_exists", "delete_resource",
];

const PLUG_GLOBALS: &[&str] = &[
    "init", "call",
    "put_private", "get_req_header", "resp", "send_resp",
];
