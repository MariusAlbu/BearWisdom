// =============================================================================
// rust_lang/builtins.rs — Rust builtin and helper predicates
// =============================================================================

use crate::types::{EdgeKind, ParsedFile};

use crate::indexer::resolve::engine::SymbolInfo;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "struct" | "interface" | "trait"),
        EdgeKind::Implements => matches!(sym_kind, "interface" | "trait"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class"
                | "struct"
                | "interface"
                | "enum"
                | "enum_member"
                | "type_alias"
                | "trait"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "struct" | "class"),
        _ => true,
    }
}

/// Extract the module path from a parsed Rust file.
/// The Rust extractor sets scope_path on top-level symbols to the module path,
/// e.g., "crate::models" or "crate::api::handlers".
pub(super) fn extract_module_path(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                // scope_path may use `::` or `.` separators — normalize to `.`
                let dot_path = sp.replace("::", ".");
                return Some(dot_path);
            }
        }
        // If no scope_path, check qualified_name prefix.
        if let Some(dot) = sym.qualified_name.rfind('.') {
            let prefix = &sym.qualified_name[..dot];
            if !prefix.is_empty() {
                return Some(prefix.to_string());
            }
        }
    }
    None
}

/// Normalize a Rust `::` path to the `.`-separated form used in the symbol index.
/// "crate::models::User" → "crate.models.User"
/// "serde::Deserialize"  → "serde.Deserialize"
pub(super) fn normalize_path(s: &str) -> String {
    s.replace("::", ".")
}

/// Extract the module prefix from a symbol's qualified_name.
/// "crate.models.User" → "crate.models"
pub(super) fn sym_module(sym: &SymbolInfo) -> &str {
    match sym.qualified_name.rfind('.') {
        Some(pos) => &sym.qualified_name[..pos],
        None => "",
    }
}

/// Return the directory portion of a file path.
pub(super) fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(pos) => &path[..pos],
        None => "",
    }
}

/// Rust built-in names that are always in scope (prelude, macros, primitive ops,
/// Iterator/Option/Result adapters, Vec/str methods, and Diesel ORM methods).
pub(super) fn is_rust_builtin(name: &str) -> bool {
    // Strip leading `::` if present
    let name = name.trim_start_matches("::");
    // Also handle single-segment names in chains like "Some", "Ok"
    let simple = name.rsplit("::").next().unwrap_or(name);

    // Single uppercase letters are generic type parameters (T, U, V, K, L, M, F, W, …).
    // They are never real symbols in the index.
    if simple.len() == 1 && simple.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
        return true;
    }
    matches!(
        simple,
        // Prelude types / enums
        "Some"
            | "None"
            | "Ok"
            | "Err"
            | "Box"
            | "Vec"
            | "String"
            | "Option"
            | "Result"
            | "Arc"
            | "Rc"
            | "Cow"
            | "Pin"
            | "PhantomData"
            // Common trait methods (always available via auto-deref/prelude)
            | "clone"
            | "to_string"
            | "to_owned"
            | "into"
            | "from"
            | "default"
            | "fmt"
            | "eq"
            | "ne"
            | "cmp"
            | "partial_cmp"
            | "hash"
            // Iterator adapters / consumers
            | "map"
            | "filter"
            | "collect"
            | "iter"
            | "into_iter"
            | "unwrap"
            | "expect"
            | "ok"
            | "err"
            | "chain"
            | "enumerate"
            | "peekable"
            | "skip"
            | "take"
            | "zip"
            | "by_ref"
            | "rev"
            | "cycle"
            | "sum"
            | "product"
            | "all"
            | "any"
            | "find"
            | "find_map"
            | "filter_map"
            | "position"
            | "max_by_key"
            | "min_by_key"
            | "max_by"
            | "min_by"
            | "for_each"
            | "flat_map"
            | "inspect"
            | "partition"
            | "unzip"
            | "fold"
            | "scan"
            | "take_while"
            | "skip_while"
            | "count"
            | "last"
            | "next"
            | "nth"
            | "step_by"
            | "copied"
            | "cloned"
            | "fuse"
            | "map_while"
            // Option / Result combinators
            | "unwrap_or_default"
            | "unwrap_or"
            | "unwrap_or_else"
            | "ok_or"
            | "ok_or_else"
            | "and_then"
            | "or_else"
            | "map_or"
            | "map_or_else"
            | "as_ref"
            | "as_mut"
            | "is_some"
            | "is_none"
            | "is_ok"
            | "is_err"
            | "transpose"
            | "flatten"
            | "is_some_and"
            | "is_ok_and"
            // String / str methods
            | "as_str"
            | "as_bytes"
            | "to_lowercase"
            | "to_uppercase"
            | "trim"
            | "trim_start"
            | "trim_end"
            | "starts_with"
            | "ends_with"
            | "contains"
            | "replace"
            | "replacen"
            | "split"
            | "rsplit"
            | "splitn"
            | "split_once"
            | "rsplit_once"
            | "split_whitespace"
            | "split_ascii_whitespace"
            | "strip_prefix"
            | "strip_suffix"
            | "matches"
            | "match_indices"
            | "lines"
            | "chars"
            | "bytes"
            | "repeat"
            | "find"
            | "rfind"
            | "join"
            // Vec / slice methods
            | "len"
            | "is_empty"
            | "push"
            | "pop"
            | "insert"
            | "remove"
            | "extend"
            | "truncate"
            | "drain"
            | "retain"
            | "dedup"
            | "sort"
            | "sort_unstable"
            | "sort_by"
            | "sort_by_key"
            | "sort_unstable_by_key"
            | "binary_search"
            | "binary_search_by"
            | "windows"
            | "chunks"
            | "split_at"
            | "get"
            | "first"
            | "swap"
            | "reverse"
            | "resize"
            | "with_capacity"
            | "capacity"
            | "reserve"
            | "shrink_to_fit"
            | "clear"
            | "append"
            | "split_off"
            | "iter_mut"
            | "partition_point"
            // HashMap / BTreeMap / collections
            | "entry"
            | "or_default"
            | "or_insert"
            | "or_insert_with"
            | "and_modify"
            | "keys"
            | "values"
            | "values_mut"
            | "contains_key"
            | "get_mut"
            | "insert"
            | "remove_entry"
            // Display / Debug / format
            | "to_string"
            | "write_str"
            | "write_fmt"
            // Conversion traits
            | "as_slice"
            | "as_ptr"
            | "into_boxed_slice"
            | "into_string"
            | "into_bytes"
            | "into_vec"
            | "into_inner"
            | "try_into"
            | "try_from"
            // Sync / Send / concurrency
            | "lock"
            | "read"
            | "write"
            | "try_lock"
            | "send"
            | "recv"
            // Diesel ORM DSL methods
            | "select"
            | "left_join"
            | "inner_join"
            | "or"
            | "and"
            | "set"
            | "on"
            | "load"
            | "get_result"
            | "execute"
            | "gt"
            | "lt"
            // Macros
            | "println"
            | "eprintln"
            | "format"
            | "writeln"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "panic"
            | "todo"
            | "unimplemented"
            | "unreachable"
            | "vec"
            | "dbg"
            | "print"
            | "eprint"
            | "cfg"
            | "include_str"
            | "include_bytes"
            | "env"
            | "concat"
            | "stringify"
            | "file"
            | "line"
            | "column"
            | "module_path"
            // Rust attributes (emitted as TypeRef by extractor)
            | "derive"
            | "test"
            | "cfg"
            | "allow"
            | "deny"
            | "warn"
            | "inline"
            | "must_use"
            | "doc"
            | "repr"
            | "non_exhaustive"
            | "automatically_derived"
            // Common derive macros
            | "Debug"
            | "Clone"
            | "Copy"
            | "PartialEq"
            | "Eq"
            | "Hash"
            | "PartialOrd"
            | "Ord"
            | "Default"
            | "Serialize"
            | "Deserialize"
            | "Display"
            | "Error"
            | "From"
            | "Into"
            // wasm-bindgen ecosystem
            | "wasm_bindgen"
            | "wasm_bindgen_test"
            | "JsValue"
            | "JsError"
            // napi-rs ecosystem
            | "napi"
            | "napi_derive"
            | "Env"
            | "Reference"
            | "JsObject"
            | "JsString"
            | "JsNumber"
            | "JsBoolean"
            | "JsUndefined"
            | "JsNull"
            | "JsFunction"
            | "JsBuffer"
    )
}
