// =============================================================================
// erlang/primitives.rs — Erlang primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Erlang.
pub(crate) const PRIMITIVES: &[&str] = &[
    // BIFs
    "abs", "apply", "atom_to_binary", "atom_to_list",
    "binary_part", "binary_to_atom", "binary_to_float", "binary_to_integer",
    "binary_to_list", "binary_to_term",
    "bit_size", "byte_size", "ceil",
    "element", "erase", "error", "exit",
    "float", "float_to_binary", "float_to_list", "floor",
    "get", "get_keys", "group_leader", "halt", "hd",
    "integer_to_binary", "integer_to_list",
    "iolist_size", "iolist_to_binary",
    "is_alive", "is_atom", "is_binary", "is_bitstring", "is_boolean",
    "is_float", "is_function", "is_integer", "is_list", "is_map",
    "is_map_key", "is_number", "is_pid", "is_port", "is_process_alive",
    "is_record", "is_reference", "is_tuple",
    "length", "link",
    "list_to_atom", "list_to_binary", "list_to_bitstring",
    "list_to_float", "list_to_integer", "list_to_pid",
    "list_to_port", "list_to_ref", "list_to_tuple",
    "make_ref", "map_get", "map_size", "max", "min",
    "node", "nodes", "not", "now",
    "open_port", "pid_to_list",
    "port_close", "port_command", "port_connect", "port_control",
    "put", "ref_to_list", "register", "registered",
    "round", "self", "setelement", "size",
    "spawn", "spawn_link", "spawn_monitor", "spawn_opt",
    "split_binary", "statistics",
    "term_to_binary", "throw", "time", "tl", "trunc",
    "tuple_size", "tuple_to_list",
    "unlink", "unregister", "whereis",
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
    "node", "number", "string", "term", "tuple",
    // EUnit macros
    "assert", "assertNot", "assertEqual", "assertNotEqual",
    "assertMatch", "assertException", "assertError", "assertExit", "assertThrow",
    "ct", "ct.pal", "ct.log",
    "?assertEqual", "?assertMatch", "?assert", "?assertNot", "?assertException",
];
