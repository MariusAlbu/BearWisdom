// =============================================================================
// gleam/builtins.rs — Gleam standard library and OTP module names
// =============================================================================

/// Gleam stdlib module names (the short name after the last `/`).
/// These are the identifiers that appear at call sites after the module alias,
/// e.g. `list.map` → module alias "list", function "map".
/// We mark the module-alias part as external here; individual function names
/// are caught via `is_gleam_stdlib_function`.
pub(super) fn is_gleam_stdlib_module(name: &str) -> bool {
    matches!(
        name,
        "io" | "int" | "float" | "string" | "list" | "option"
            | "result" | "dict" | "bool" | "order" | "dynamic"
            | "bit_array" | "bytes_builder" | "iterator" | "queue"
            | "set" | "uri" | "regex" | "yielder"
            // OTP / process library
            | "process" | "actor" | "erlang" | "otp"
    )
}

/// Gleam standard library function names (unqualified after module stripping).
pub(super) fn is_gleam_stdlib_function(name: &str) -> bool {
    matches!(
        name,
        // --- io ---
        "println" | "print" | "debug" | "eprintln" | "eprint"
            // --- int ---
            | "to_string" | "parse" | "add" | "subtract" | "multiply" | "divide"
            | "remainder" | "modulo" | "power" | "absolute_value"
            | "max" | "min" | "clamp" | "negate" | "square_root"
            | "is_even" | "is_odd" | "compare" | "digits" | "undigits"
            | "to_base_string" | "from_base_string" | "to_float"
            | "bitwise_and" | "bitwise_or" | "bitwise_exclusive_or"
            | "bitwise_shift_left" | "bitwise_shift_right" | "bitwise_not"
            // --- float ---
            | "round" | "ceiling" | "floor" | "truncate" | "loosely_equals"
            | "loosely_compare" | "to_precision"
            // --- string ---
            | "concat" | "contains" | "split" | "replace" | "length"
            | "trim" | "trim_start" | "trim_end" | "uppercase" | "lowercase"
            | "starts_with" | "ends_with" | "join" | "inspect"
            | "pad_start" | "pad_end" | "drop_start" | "drop_end"
            | "pop_grapheme" | "to_graphemes" | "from_graphemes"
            | "to_utf_codepoints" | "from_utf_codepoints"
            | "byte_size" | "slice" | "crop_start"
            // --- list ---
            | "map" | "filter" | "fold" | "each" | "find" | "first" | "rest"
            | "reverse" | "sort" | "contains" | "flatten" | "append"
            | "zip" | "range" | "repeat" | "chunk" | "group"
            | "count" | "unique" | "take" | "drop" | "last" | "index_of"
            | "index_map" | "flatten_map" | "try_map" | "filter_map"
            | "fold_right" | "reduce" | "pop" | "is_empty" | "sized_chunk"
            // --- option ---
            | "unwrap" | "is_some" | "is_none" | "then" | "or" | "lazy_or"
            | "lazy_unwrap" | "values" | "all" | "none"
            // --- result ---
            | "try" | "is_ok" | "is_error" | "replace" | "replace_error"
            | "nil_error" | "or" | "lazy_or" | "flatten" | "partition"
            | "unwrap_error" | "try_recover" | "map_error"
            // --- dict ---
            | "new" | "from_list" | "to_list" | "get" | "insert" | "delete"
            | "has_key" | "keys" | "values" | "size" | "merge" | "update"
            | "upsert" | "fold" | "filter" | "map_values" | "take"
            | "drop" | "combine"
            // --- bool ---
            | "and" | "or" | "negate" | "exclusive_or" | "guard"
            | "lazy_guard" | "to_string"
            // --- order ---
            | "compare" | "negate" | "to_int" | "break_tie"
            // --- dynamic ---
            | "from" | "classify" | "string" | "int" | "float" | "bool"
            | "list" | "field" | "element" | "optional_field"
            | "decode_error" | "any" | "dict" | "result"
            // --- process / actor (OTP) ---
            | "start" | "send" | "receive" | "sleep" | "sleep_forever"
            | "new_subject" | "self" | "pid" | "monitor" | "demonitor"
            | "link" | "unlink" | "kill" | "is_alive"
            | "receive_forever" | "selecting" | "map_subject"
            // --- erlang ---
            | "rescue" | "get_line" | "atom_from_dynamic"
    )
}
