use std::collections::HashSet;

/// Runtime globals always external for OCaml.
/// Covers: Stdlib/Pervasives, option/result constructors, common module
/// functions used unqualified or as call heads, and Lwt core.
pub(crate) const EXTERNALS: &[&str] = &[
    // --- Option / Result constructors (used as function applications) ---
    "Some", "None", "Ok", "Error",

    // --- Stdlib / Pervasives ---
    "failwith", "invalid_arg", "raise", "raise_notrace",
    "not", "fst", "snd",
    "ref", "ignore", "incr", "decr",
    "succ", "pred", "abs",
    "min", "max", "compare",
    "print_endline", "print_string", "print_char",
    "print_int", "print_float", "print_newline",
    "prerr_endline", "prerr_string",
    "read_line", "read_int", "read_float",
    "exit", "at_exit",
    "string_of_int", "string_of_float", "string_of_bool",
    "int_of_string", "float_of_string", "bool_of_string",
    "int_of_char", "char_of_int",
    "truncate", "float_of_int", "int_of_float",
    "sqrt", "exp", "log", "log10",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "ceil", "floor",
    "mod_float", "ldexp", "frexp", "modf",
    "infinity", "nan", "epsilon_float",
    "min_int", "max_int", "min_float", "max_float",
    "format_of_string", "string_to_bytes", "bytes_to_string",
    "assert", "fun",

    // --- List module (commonly opened or called as List.f) ---
    "List.map", "List.iter", "List.filter", "List.fold_left", "List.fold_right",
    "List.rev", "List.length", "List.append", "List.concat", "List.concat_map",
    "List.for_all", "List.exists", "List.find", "List.find_opt",
    "List.assoc", "List.assoc_opt", "List.mem", "List.memq",
    "List.sort", "List.stable_sort", "List.fast_sort",
    "List.nth", "List.hd", "List.tl", "List.last",
    "List.flatten", "List.rev_map", "List.combine", "List.split",
    "List.to_seq", "List.of_seq",
    "List.init", "List.filteri", "List.mapi", "List.iteri",
    "List.fold_left_map",

    // --- Array module ---
    "Array.make", "Array.create_float", "Array.init",
    "Array.length", "Array.get", "Array.set",
    "Array.map", "Array.iter", "Array.fold_left", "Array.fold_right",
    "Array.of_list", "Array.to_list",
    "Array.copy", "Array.fill", "Array.blit",
    "Array.sort", "Array.stable_sort",

    // --- String module ---
    "String.concat", "String.sub", "String.length",
    "String.make", "String.init", "String.copy",
    "String.get", "String.index", "String.index_opt",
    "String.contains", "String.starts_with", "String.ends_with",
    "String.trim", "String.uppercase_ascii", "String.lowercase_ascii",
    "String.capitalize_ascii", "String.uncapitalize_ascii",
    "String.split_on_char", "String.to_seq",
    "String.escaped", "String.of_seq",

    // --- Hashtbl module ---
    "Hashtbl.create", "Hashtbl.add", "Hashtbl.find", "Hashtbl.find_opt",
    "Hashtbl.mem", "Hashtbl.remove", "Hashtbl.replace",
    "Hashtbl.iter", "Hashtbl.fold", "Hashtbl.length",
    "Hashtbl.clear", "Hashtbl.reset", "Hashtbl.copy",
    "Hashtbl.of_seq", "Hashtbl.to_seq",

    // --- Map / Set (functorised, accessed via module alias) ---
    "Map.empty", "Map.add", "Map.find", "Map.find_opt",
    "Map.mem", "Map.remove", "Map.iter", "Map.fold",
    "Map.map", "Map.filter", "Map.cardinal", "Map.bindings",
    "Set.empty", "Set.add", "Set.mem", "Set.remove",
    "Set.union", "Set.inter", "Set.diff", "Set.elements",

    // --- Printf / Format modules ---
    "Printf.printf", "Printf.sprintf", "Printf.eprintf", "Printf.fprintf",
    "Printf.sscanf", "Scanf.sscanf", "Scanf.scanf",
    "Format.printf", "Format.sprintf", "Format.eprintf", "Format.fprintf",
    "Format.pp_print_string", "Format.pp_print_int", "Format.pp_print_newline",
    "Fmt.str", "Fmt.pf", "Fmt.pr", "Fmt.epr", "Fmt.strf",
    "Fmt.string", "Fmt.int", "Fmt.float", "Fmt.bool", "Fmt.char",
    "Fmt.list", "Fmt.array", "Fmt.option", "Fmt.result",
    "Fmt.nop", "Fmt.cut", "Fmt.sp", "Fmt.semi",
    "Fmt.box", "Fmt.vbox", "Fmt.hbox", "Fmt.hvbox", "Fmt.hovbox",
    "Fmt.using", "Fmt.of_to_string",

    // --- Option module (OCaml 4.08+) ---
    "Option.get", "Option.value", "Option.bind", "Option.map",
    "Option.fold", "Option.iter", "Option.is_some", "Option.is_none",
    "Option.to_list", "Option.to_seq",

    // --- Result module (OCaml 4.08+) ---
    "Result.get_ok", "Result.get_error", "Result.bind", "Result.map",
    "Result.map_error", "Result.fold", "Result.iter", "Result.iter_error",
    "Result.is_ok", "Result.is_error", "Result.to_option",

    // --- Bytes module ---
    "Bytes.create", "Bytes.make", "Bytes.length", "Bytes.get", "Bytes.set",
    "Bytes.copy", "Bytes.fill", "Bytes.blit", "Bytes.to_string", "Bytes.of_string",

    // --- Lwt core (extremely common in OCaml ecosystem) ---
    "Lwt.return", "Lwt.bind", "Lwt.map", "Lwt.both", "Lwt.all",
    "Lwt.join", "Lwt.pick", "Lwt.choose",
    "Lwt.fail", "Lwt.fail_with", "Lwt.fail_invalid_arg",
    "Lwt.catch", "Lwt.try_bind", "Lwt.finalize",
    "Lwt.async", "Lwt.ignore_result",
    "Lwt.return_unit", "Lwt.return_none", "Lwt.return_nil",
    "Lwt.return_true", "Lwt.return_false",
    "Lwt_main.run", "Lwt_io.printf", "Lwt_io.print", "Lwt_io.printl",
    "Lwt_unix.sleep",

    // --- Sys / Unix ---
    "Sys.argv", "Sys.getenv", "Sys.file_exists",
    "Sys.getcwd", "Sys.chdir",
    "Unix.gettimeofday", "Unix.sleep", "Unix.getenv",
];

/// Dependency-gated framework globals for OCaml.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Alcotest — most common OCaml test framework
    for dep in ["alcotest", "alcotest-lwt", "alcotest-async"] {
        if deps.contains(dep) {
            globals.extend(ALCOTEST_GLOBALS);
            break;
        }
    }

    // OUnit / OUnit2
    for dep in ["ounit", "ounit2"] {
        if deps.contains(dep) {
            globals.extend(OUNIT_GLOBALS);
            break;
        }
    }

    // QCheck property testing
    for dep in ["qcheck", "qcheck-alcotest", "qcheck-ounit"] {
        if deps.contains(dep) {
            globals.extend(QCHECK_GLOBALS);
            break;
        }
    }

    globals
}

const ALCOTEST_GLOBALS: &[&str] = &[
    "check", "check_raises", "fail", "failf",
    "testcase", "test_case", "suite",
    "run", "run_with_args",
    "string", "int", "float", "bool", "unit",
    "char", "bytes", "list", "array", "option", "result",
    "pass", "reject",
];

const OUNIT_GLOBALS: &[&str] = &[
    "assert_equal", "assert_bool", "assert_string",
    "assert_failure", "assert_raises",
    "make_suite", "run_test_tt_main",
    ">::", ">:::","~:",
];

const QCHECK_GLOBALS: &[&str] = &[
    "Test.make", "Gen.int", "Gen.float", "Gen.bool",
    "Gen.string", "Gen.list", "Gen.array", "Gen.option",
    "Gen.oneof", "Gen.frequency",
    "assume", "collect", "stat",
];
