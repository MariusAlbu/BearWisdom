// =============================================================================
// erlang/keywords.rs — Erlang primitive and built-in types
// =============================================================================

// ERTS C BIFs — auto-imported from the `erlang` module, implemented in C
// inside the BEAM VM. No `.erl` source body exists for these; the OTP walker
// cannot discover them via source parsing. Listed here so the resolver can
// classify them as builtins without requiring an OTP index entry.
/// Primitive and built-in type/function names for Erlang.
pub(crate) const KEYWORDS: &[&str] = &[
    // Process / lifecycle
    "self", "spawn", "spawn_link", "spawn_monitor", "spawn_opt",
    "process_info", "process_flag",
    "exit", "error", "throw",
    "register", "unregister", "whereis",
    "link", "unlink",
    "monitor", "demonitor",
    "is_process_alive", "node", "nodes", "is_alive", "disconnect_node",
    "halt", "garbage_collect",
    "pid_to_list", "list_to_pid",
    "processes", "registered",
    // Type guard predicates
    "is_atom", "is_binary", "is_bitstring", "is_boolean",
    "is_float", "is_function", "is_integer", "is_list", "is_map",
    "is_map_key", "is_number", "is_pid", "is_port",
    "is_record", "is_reference", "is_tuple",
    // Size / length
    "length", "size", "byte_size", "bit_size", "tuple_size",
    "map_size", "iolist_size", "iolist_to_binary", "iolist_to_iovec",
    // Sequence ops
    "hd", "tl", "element", "setelement",
    "tuple_to_list", "list_to_tuple",
    "binary_to_list", "list_to_binary",
    "binary_to_term", "term_to_binary",
    "binary_to_atom", "atom_to_binary",
    "binary_to_integer", "integer_to_binary",
    "binary_to_float", "float_to_binary",
    "atom_to_list", "list_to_atom",
    "integer_to_list", "list_to_integer",
    "float_to_list", "list_to_float",
    "binary_part", "split_binary",
    "list_to_bitstring", "list_to_port", "list_to_ref",
    "ref_to_list",
    // Math
    "abs", "min", "max", "round", "trunc", "float", "ceil", "floor",
    // Time
    "now", "time", "date", "localtime", "universaltime",
    "monotonic_time", "system_time", "time_offset", "convert_time_unit",
    "start_timer", "cancel_timer", "read_timer", "send_after",
    // System
    "apply", "make_ref", "unique_integer",
    "phash", "phash2", "external_size",
    "system_flag", "system_info", "statistics",
    "port_close", "port_command", "port_connect", "port_control", "port_info",
    "open_port",
    "send",
    // Misc builtins
    "get", "get_keys", "erase", "put",
    "group_leader", "map_get",
    "not", "node",
    // OTP gen_server / supervisor callbacks
    "start_link", "init", "handle_call", "handle_cast", "handle_info",
    "terminate", "code_change", "format_status",
    "start", "stop", "call", "cast", "reply", "noreply",
    "gen_server", "gen_statem", "gen_event", "supervisor", "application",
    // io module
    "io.format", "io.fwrite",
    // lists module
    "lists.map", "lists.filter", "lists.foldl", "lists.foldr",
    "lists.foreach", "lists.flatten", "lists.reverse", "lists.sort",
    "lists.member", "lists.keyfind", "lists.keystore", "lists.keydelete",
    "lists.zip", "lists.unzip", "lists.seq", "lists.nth", "lists.last",
    "lists.append", "lists.concat", "lists.duplicate", "lists.subtract",
    "lists.all", "lists.any",
    // maps module
    "maps.get", "maps.find", "maps.put", "maps.new", "maps.from_list",
    "maps.to_list", "maps.keys", "maps.values", "maps.merge",
    "maps.update", "maps.remove", "maps.fold", "maps.map",
    "maps.filter", "maps.is_key", "maps.size",
    // string module
    "string.trim", "string.split", "string.join", "string.find",
    "string.replace", "string.lowercase", "string.uppercase",
    "string.to_integer", "string.to_float",
    // filename module
    "filename.join", "filename.basename", "filename.dirname",
    "filename.extension", "filename.rootname",
    // proplists module
    "proplists.get_value", "proplists.get_all_values",
    "proplists.delete", "proplists.is_defined",
    // ets module
    "ets.new", "ets.insert", "ets.lookup", "ets.delete",
    "ets.match", "ets.select", "ets.tab2list",
    // timer module
    "timer.sleep", "timer.send_after", "timer.apply_after", "timer.tc",
    // atom types
    "ok", "error", "undefined", "true", "false", "nil", "infinity",
    "timeout", "noreply", "stop", "ignore",
    // type names
    "pid", "port", "reference", "atom", "binary", "bitstring",
    "boolean", "byte", "char", "fun", "function", "integer",
    "iodata", "iolist", "list", "map", "mfa", "module",
    "neg_integer", "non_neg_integer", "pos_integer", "nonempty_list",
    "no_return", "node", "number", "string", "term", "tuple",
    // EUnit macros
    "assert", "assertNot", "assertEqual", "assertNotEqual",
    "assertMatch", "assertException", "assertError", "assertExit", "assertThrow",
    "ct", "ct.pal", "ct.log",
    "?assertEqual", "?assertMatch", "?assert", "?assertNot", "?assertException",
];
