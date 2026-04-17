// =============================================================================
// ocaml/keywords.rs — OCaml primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for OCaml.
pub(crate) const KEYWORDS: &[&str] = &[
    // core constructors / types
    "Some", "None", "Ok", "Error",
    "true", "false", "unit",
    "int", "float", "bool", "char", "string", "bytes",
    "list", "array", "option", "result", "ref", "exn",
    "format", "in_channel", "out_channel",
    // I/O
    "print_string", "print_endline", "print_int", "print_float",
    "print_char", "print_newline",
    "prerr_string", "prerr_endline", "prerr_int",
    "read_line", "read_int",
    "input_line", "output_string", "flush",
    "open_in", "open_out", "close_in", "close_out",
    // exceptions / assertions
    "raise", "failwith", "invalid_arg", "assert", "ignore",
    // utility
    "fst", "snd", "not", "succ", "pred", "abs",
    "max_int", "min_int", "max_float", "min_float",
    "infinity", "nan", "epsilon_float",
    "compare", "min", "max", "incr", "decr",
    // conversions
    "string_of_int", "string_of_float", "string_of_bool",
    "int_of_string", "float_of_string", "bool_of_string",
    "char_of_int", "int_of_char",
    // String module
    "String.length", "String.get", "String.make", "String.sub",
    "String.concat", "String.contains", "String.trim",
    "String.lowercase_ascii", "String.uppercase_ascii",
    "String.split_on_char", "String.equal", "String.compare",
    // Bytes module
    "Bytes.create", "Bytes.length", "Bytes.get", "Bytes.set",
    "Bytes.copy", "Bytes.blit", "Bytes.sub",
    "Bytes.to_string", "Bytes.of_string",
    // List module
    "List.map", "List.mapi", "List.iter", "List.iteri",
    "List.fold_left", "List.fold_right",
    "List.filter", "List.filter_map",
    "List.find", "List.find_opt",
    "List.mem", "List.assoc", "List.assoc_opt",
    "List.split", "List.combine",
    "List.rev", "List.length", "List.hd", "List.tl",
    "List.nth", "List.nth_opt",
    "List.flatten", "List.sort", "List.stable_sort", "List.fast_sort",
    "List.exists", "List.for_all",
    "List.init", "List.concat", "List.append",
    // Array module
    "Array.make", "Array.create_float", "Array.init",
    "Array.length", "Array.get", "Array.set",
    "Array.copy", "Array.blit", "Array.sub",
    "Array.to_list", "Array.of_list",
    "Array.map", "Array.mapi", "Array.iter", "Array.iteri",
    "Array.fold_left", "Array.fold_right",
    "Array.sort", "Array.stable_sort",
    "Array.exists", "Array.for_all",
    // Hashtbl module
    "Hashtbl.create", "Hashtbl.add", "Hashtbl.find", "Hashtbl.find_opt",
    "Hashtbl.mem", "Hashtbl.remove", "Hashtbl.replace",
    "Hashtbl.iter", "Hashtbl.fold", "Hashtbl.length",
    // Buffer module
    "Buffer.create", "Buffer.add_string", "Buffer.add_char",
    "Buffer.contents", "Buffer.clear", "Buffer.length",
    // Printf / Format
    "Printf.printf", "Printf.sprintf", "Printf.fprintf", "Printf.eprintf",
    "Format.printf", "Format.sprintf", "Format.fprintf", "Format.asprintf",
    // Sys module
    "Sys.argv", "Sys.getenv", "Sys.getenv_opt",
    "Sys.file_exists", "Sys.is_directory", "Sys.command", "Sys.time",
    // Filename module
    "Filename.concat", "Filename.basename", "Filename.dirname",
    "Filename.extension", "Filename.remove_extension",
    "Filename.chop_extension", "Filename.temp_file",
    // Lazy
    "Lazy.force", "Lazy.from_fun", "Lazy.from_val",
    // Option module (OCaml 4.08+)
    "Option.map", "Option.bind", "Option.value", "Option.get",
    "Option.is_some", "Option.is_none", "Option.join", "Option.iter",
    // Result module (OCaml 4.08+)
    "Result.ok", "Result.error", "Result.map", "Result.bind",
    "Result.is_ok", "Result.is_error",
    "Result.get_ok", "Result.get_error", "Result.to_option",
    // Seq module
    "Seq.map", "Seq.filter", "Seq.fold_left", "Seq.iter",
    "Seq.empty", "Seq.return", "Seq.append", "Seq.concat", "Seq.flat_map",
    // Fun module
    "Fun.id", "Fun.const", "Fun.flip", "Fun.negate", "Fun.protect",
    // Int / Float modules
    "Int.equal", "Int.compare", "Int.to_string", "Int.of_string",
    "Float.equal", "Float.compare", "Float.to_string", "Float.of_string",
    // channels
    "In_channel.stdin", "Out_channel.stdout", "Out_channel.stderr",
    // Domain / Mutex / Condition (OCaml 5+)
    "Domain.spawn", "Domain.join",
    "Mutex.create", "Mutex.lock", "Mutex.unlock",
    "Condition.create", "Condition.wait", "Condition.signal", "Condition.broadcast",
    // Dune / common third-party (Pp, Path, Fiber, Memo)
    "Pp.text", "Pp.textf", "Pp.concat", "Pp.verbatim", "Pp.nop",
    "Pp.seq", "Pp.box", "Pp.vbox", "Pp.hbox", "Pp.hvbox", "Pp.hovbox",
    "Pp.tag", "Pp.cut", "Pp.space", "Pp.newline",
    "Path.build", "Path.relative", "Path.to_string",
    "Fiber.return", "Fiber.fork",
    "Memo.return", "Memo.exec",
    "Code_error.raise", "User_error.raise",
    "Import",
    // generic type params
    "T", "U", "K", "V", "a", "b", "c",
];
