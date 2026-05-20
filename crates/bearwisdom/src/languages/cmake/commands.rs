// =============================================================================
// languages/cmake/commands.rs  —  per-command extractors for normal_command
//
// Dispatches a single `normal_command` to the appropriate emitter (set, option,
// add_executable, find_package, …) and emits symbols + refs accordingly.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use super::arguments::{collect_arguments, collect_raw_arguments, command_identifier, nth_argument};
use super::extract::make_symbol;
use super::hooks::is_cmake_builtin;
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// normal_command dispatch
// ---------------------------------------------------------------------------

pub(super) fn extract_normal_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let cmd = match command_identifier(node, src) {
        Some(c) => c,
        None => return,
    };

    // Emit a Function symbol for every normal_command so coverage can match
    // the normal_command symbol_node_kind against an extracted symbol by line.
    let sym_idx = symbols.len();
    symbols.push(make_symbol(
        cmd.clone(),
        cmd.clone(),
        SymbolKind::Function,
        node,
        Some(format!("{}(...)", cmd)),
        None,
    ));

    // Only emit a Calls edge for non-builtin commands (user-defined functions/macros).
    // Builtin commands are resolved to external automatically; emitting Calls refs
    // for them produces unresolved noise against the project symbol index.
    if !is_cmake_builtin(&cmd) {
        refs.push(ExtractedRef {
            source_symbol_index: sym_idx,
            target_name: cmd.clone(),
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }

    let cmd_lower = cmd.to_lowercase();
    match cmd_lower.as_str() {
        "set" => extract_set_command(node, src, symbols),
        "option" => extract_option_command(node, src, symbols),
        "add_executable" | "add_library" | "add_custom_target" => {
            extract_target_command(node, src, symbols, refs)
        }
        "project" => extract_project_command(node, src, symbols),
        "include" => extract_include_command(node, src, refs),
        "find_package" => extract_find_package_command(node, src, symbols, refs),
        "add_subdirectory" => extract_add_subdirectory_command(node, src, refs),
        "target_link_libraries" => extract_target_link_libraries(node, src, symbols, refs),
        "string" => extract_string_output_var(node, src, symbols),
        "get_filename_component" | "cmake_path" => {
            extract_first_arg_output_var(node, src, symbols, &cmd_lower)
        }
        "math" => extract_math_output_var(node, src, symbols),
        "find_program" | "find_library" | "find_path" | "find_file" => {
            extract_first_arg_output_var(node, src, symbols, &cmd_lower)
        }
        "separate_arguments" => extract_first_arg_output_var(node, src, symbols, &cmd_lower),
        "file" => extract_file_output_var(node, src, symbols),
        "cmake_parse_arguments" => extract_cmake_parse_arguments(node, src, symbols),
        "execute_process" => extract_execute_process_outputs(node, src, symbols),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// foreach(<loop_var> ...) — first arg is the loop variable
// ---------------------------------------------------------------------------

pub(super) fn extract_foreach_loop_var(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let Some(name) = nth_argument(node, src, 0) else { return };
    if name.is_empty() || name.starts_with('$') {
        return;
    }
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some("foreach loop variable".to_string()),
        None,
    ));
}

// ---------------------------------------------------------------------------
// string(<MODE> ...) — output variable position depends on mode
// ---------------------------------------------------------------------------

fn extract_string_output_var(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let args = collect_arguments(node, src);
    let Some(mode) = args.first().map(|s| s.to_ascii_uppercase()) else { return };
    // For most string() modes, the output variable is either the second arg
    // (TOLOWER, TOUPPER, LENGTH, STRIP, ...) or the last arg (SUBSTRING, REGEX MATCH, ...).
    // Capture both candidate positions so we don't miss either pattern.
    let candidates: Vec<&String> = match mode.as_str() {
        // <out_var> in position 1
        "SHA1" | "SHA224" | "SHA256" | "SHA384" | "SHA512" | "MD5" => {
            args.get(1).into_iter().collect()
        }
        // input is arg 1, output is arg 2: string(MODE input output)
        "TOLOWER" | "TOUPPER" | "LENGTH" | "STRIP" | "ASCII" | "HEX" | "TIMESTAMP" => {
            args.get(2).into_iter().collect()
        }
        // string(APPEND <var> ...) and string(PREPEND <var> ...) modify <var> in-place;
        // the variable name is always the second arg (index 1 after mode).
        "APPEND" | "PREPEND" => {
            args.get(1).into_iter().collect()
        }
        // string(REPLACE <match> <replace> <out_var> <input...>) — out_var at index 3
        // string(FIND <str> <sub> <out_var> [REVERSE]) — out_var at index 3
        "REPLACE" | "FIND" => {
            args.get(3).into_iter().collect()
        }
        // string(CONCAT <out_var> [<input>...]) — out_var at index 1
        "CONCAT" => {
            args.get(1).into_iter().collect()
        }
        // string(JOIN <glue> <out_var> <input...>) — out_var at index 2
        "JOIN" => {
            args.get(2).into_iter().collect()
        }
        // string(SUBSTRING/REPEAT/REGEX/GENEX_STRIP ...) — out_var is the last arg
        "SUBSTRING" | "REPEAT" | "REGEX" | "GENEX_STRIP" => {
            args.last().into_iter().collect()
        }
        _ => return,
    };
    for cand in candidates {
        if cand.is_empty() || cand.starts_with('$') {
            continue;
        }
        let sig = format!("string({}) → {}", mode, cand);
        symbols.push(make_symbol(
            cand.clone(),
            cand.clone(),
            SymbolKind::Variable,
            node,
            Some(sig),
            None,
        ));
    }
}

// ---------------------------------------------------------------------------
// get_filename_component(<out> ...) and cmake_path(<MODE> <out> ...) —
// output variable in the first (or second, after MODE) argument position.
// ---------------------------------------------------------------------------

fn extract_first_arg_output_var(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    cmd: &str,
) {
    let args = collect_arguments(node, src);
    // get_filename_component: arg 0 is output. cmake_path: arg 0 is MODE, arg 1 is output.
    let out_idx = if cmd == "cmake_path" { 1 } else { 0 };
    let Some(name) = args.get(out_idx) else { return };
    if name.is_empty() || name.starts_with('$') {
        return;
    }
    let sig = format!("{}(... → {})", cmd, name);
    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// math(EXPR <out_var> "<expression>") — output variable is arg 1 (after EXPR)
// ---------------------------------------------------------------------------

fn extract_math_output_var(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let args = collect_arguments(node, src);
    if args.first().map(|s| s.eq_ignore_ascii_case("EXPR")) != Some(true) {
        return;
    }
    let Some(name) = args.get(1) else { return };
    if name.is_empty() || name.starts_with('$') {
        return;
    }
    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Variable,
        node,
        Some(format!("math(EXPR {} ...)", name)),
        None,
    ));
}

// ---------------------------------------------------------------------------
// file(<MODE> ...) — output variable position depends on mode.
//   GLOB / GLOB_RECURSE: arg 1 (after MODE)
//   READ <path> <out>: arg 2
//   STRINGS <path> <out>: arg 2
//   RELATIVE_PATH <out> <dir> <path>: arg 1
//   TIMESTAMP <path> <out>: arg 2
//   SIZE <path> <out>: arg 2
//   MD5/SHA1/SHA256/SHA384/SHA512 <path> <out>: arg 2
//   REAL_PATH <path> <out>: arg 2
//   TO_CMAKE_PATH <path> <out>: arg 2
//   TO_NATIVE_PATH <path> <out>: arg 2
// ---------------------------------------------------------------------------

fn extract_file_output_var(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let args = collect_arguments(node, src);
    let Some(mode) = args.first().map(|s| s.to_ascii_uppercase()) else { return };
    let out_idx = match mode.as_str() {
        "GLOB" | "GLOB_RECURSE" | "RELATIVE_PATH" => 1,
        "READ" | "STRINGS" | "TIMESTAMP" | "SIZE" | "MD5"
        | "SHA1" | "SHA224" | "SHA256" | "SHA384" | "SHA512"
        | "REAL_PATH" | "TO_CMAKE_PATH" | "TO_NATIVE_PATH" => 2,
        _ => return,
    };
    let Some(name) = args.get(out_idx) else { return };
    if name.is_empty() || name.starts_with('$') {
        return;
    }
    symbols.push(make_symbol(
        name.clone(),
        name.clone(),
        SymbolKind::Variable,
        node,
        Some(format!("file({} ... → {})", mode, name)),
        None,
    ));
}

// ---------------------------------------------------------------------------
// cmake_parse_arguments(<prefix> <options> <one_value> <multi_value> ...) —
// generates `<prefix>_<keyword>` variables for each option/oneval/multival
// keyword. Two call forms:
//   cmake_parse_arguments(<prefix> <options> <oneval> <multival> <args...>)
//   cmake_parse_arguments(PARSE_ARGV <n> <prefix> <options> <oneval> <multival>)
// Keyword strings are space- or semicolon-separated; the args themselves may
// be variable refs (`${oneArgs}`) which we cannot resolve statically — those
// cases are skipped.
// ---------------------------------------------------------------------------

fn extract_cmake_parse_arguments(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let args = collect_arguments(node, src);
    let (prefix, opts_idx) = if args.first().map(|s| s.eq_ignore_ascii_case("PARSE_ARGV")) == Some(true) {
        // PARSE_ARGV form: prefix is arg 2
        (args.get(2).cloned(), 3)
    } else {
        // Plain form: prefix is arg 0
        (args.first().cloned(), 1)
    };
    let Some(prefix) = prefix else { return };
    if prefix.is_empty() || prefix.starts_with('$') {
        return;
    }
    // Also emit the prefix itself as a Variable so `${PREFIX}` resolves —
    // although typically callers reference `${PREFIX_<KW>}` directly.
    symbols.push(make_symbol(
        prefix.clone(),
        prefix.clone(),
        SymbolKind::Variable,
        node,
        Some(format!("cmake_parse_arguments prefix {}", prefix)),
        None,
    ));
    // Parse options/oneval/multival keyword lists at indices opts_idx..opts_idx+3.
    for offset in 0..3 {
        let Some(kw_list) = args.get(opts_idx + offset) else { continue };
        // Strip surrounding quotes — `argument` nodes wrapping a quoted_argument
        // include the quote characters in their text.
        let kw_list = kw_list.trim().trim_matches('"').trim_matches('\'');
        if kw_list.is_empty() || kw_list.starts_with('$') {
            continue;
        }
        for kw in kw_list.split(|c: char| c == ';' || c.is_whitespace()) {
            let kw = kw.trim().trim_matches('"').trim_matches('\'');
            if kw.is_empty() {
                continue;
            }
            let var_name = format!("{}_{}", prefix, kw);
            symbols.push(make_symbol(
                var_name.clone(),
                var_name.clone(),
                SymbolKind::Variable,
                node,
                Some(format!("cmake_parse_arguments(... {} ...)", kw)),
                None,
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// execute_process(... [OUTPUT_VARIABLE <var>] [ERROR_VARIABLE <var>]
//                     [RESULT_VARIABLE <var>] [RESULTS_VARIABLE <var>])
// ---------------------------------------------------------------------------

fn extract_execute_process_outputs(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let args = collect_arguments(node, src);
    let mut i = 0;
    while i < args.len() {
        let kw = args[i].to_ascii_uppercase();
        let is_output_kw = matches!(
            kw.as_str(),
            "OUTPUT_VARIABLE" | "ERROR_VARIABLE" | "RESULT_VARIABLE" | "RESULTS_VARIABLE"
        );
        if is_output_kw {
            if let Some(name) = args.get(i + 1) {
                if !name.is_empty() && !name.starts_with('$') {
                    symbols.push(make_symbol(
                        name.clone(),
                        name.clone(),
                        SymbolKind::Variable,
                        node,
                        Some(format!("execute_process({} {})", kw, name)),
                        None,
                    ));
                }
            }
            i += 2;
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// set(<name> ...) → Variable
// ---------------------------------------------------------------------------

fn extract_set_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("set({} ...)", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// option(<name> "description" <default>) → Variable
// ---------------------------------------------------------------------------

fn extract_option_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("option({} ...)", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Variable,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// add_executable / add_library / add_custom_target → Function (build target)
// ---------------------------------------------------------------------------

fn extract_target_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let cmd = command_identifier(node, src).unwrap_or_default();
    let sig = format!("{}({} ...)", cmd, name);
    let idx = symbols.len();
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Function,
        node,
        Some(sig),
        None,
    ));
    // The command itself is already emitted as a Calls edge above in dispatch;
    // suppress a duplicate by not re-emitting here. The idx is stored for
    // target_link_libraries to reference.
    let _ = (idx, refs);
}

// ---------------------------------------------------------------------------
// project(<name> ...) → Namespace
// ---------------------------------------------------------------------------

fn extract_project_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };
    let sig = format!("project({})", name);
    symbols.push(make_symbol(
        name.clone(),
        name,
        SymbolKind::Namespace,
        node,
        Some(sig),
        None,
    ));
}

// ---------------------------------------------------------------------------
// include(<path>) → Imports
// ---------------------------------------------------------------------------

fn extract_include_command(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    let path = match nth_argument(node, src, 0) {
        Some(p) => p,
        None => return,
    };
    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: path.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(path),
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// find_package(<pkg> ...) → Imports + conventional output variable symbols
//
// CMake Find modules follow a naming convention: find_package(Foo) sets
// Foo_FOUND, FOO_FOUND, FOO_LIBRARIES, FOO_INCLUDE_DIRS, FOO_EXECUTABLE,
// FOO_VERSION, and FOO_DIRS. Emitting these as Variable symbols lets variable
// refs to these names resolve within the project that called find_package.
// ---------------------------------------------------------------------------

fn extract_find_package_command(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let pkg = match nth_argument(node, src, 0) {
        Some(p) => p,
        None => return,
    };
    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: pkg.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(pkg.clone()),
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
    emit_find_package_vars(node, &pkg, symbols);
}

/// Emit conventional Find-module output variable symbols for `find_package(<pkg>)`.
///
/// Covers both the original-case form (Protobuf_FOUND, Git_EXECUTABLE) and
/// the all-uppercase form (PROTOBUF_FOUND, GIT_EXECUTABLE) used by older CMake
/// Find modules.
fn emit_find_package_vars(
    node: &Node,
    pkg: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let upper = pkg.to_ascii_uppercase();
    let suffixes = [
        "_FOUND", "_LIBRARIES", "_LIBRARY", "_INCLUDE_DIRS", "_INCLUDE_DIR",
        "_EXECUTABLE", "_VERSION", "_DIRS", "_DIR",
    ];
    for &suffix in &suffixes {
        let mixed_name = format!("{pkg}{suffix}");
        symbols.push(make_symbol(
            mixed_name.clone(),
            mixed_name,
            SymbolKind::Variable,
            node,
            Some(format!("find_package({pkg}) output")),
            None,
        ));
        // Uppercase form only when pkg is not already uppercase
        if upper != pkg {
            let upper_name = format!("{upper}{suffix}");
            symbols.push(make_symbol(
                upper_name.clone(),
                upper_name,
                SymbolKind::Variable,
                node,
                Some(format!("find_package({pkg}) output")),
                None,
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// add_subdirectory(<dir>) → Imports + Calls
// ---------------------------------------------------------------------------

fn extract_add_subdirectory_command(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
) {
    let dir = match nth_argument(node, src, 0) {
        Some(d) => d,
        None => return,
    };
    refs.push(ExtractedRef {
        source_symbol_index: 0,
        target_name: dir.clone(),
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(dir),
        chain: None,
        byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// target_link_libraries(<target> <libs...>) → Calls from target to each lib
// ---------------------------------------------------------------------------

fn extract_target_link_libraries(
    node: &Node,
    src: &str,
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
) {
    // First argument is the target name; resolve to symbol index if possible.
    let target_name = match nth_argument(node, src, 0) {
        Some(n) => n,
        None => return,
    };

    // Find the target's symbol index, or default to 0.
    let target_idx = symbols
        .iter()
        .position(|s| s.name == target_name)
        .unwrap_or(0);

    // Remaining arguments are libraries (skip keywords like PUBLIC, PRIVATE, INTERFACE).
    // Arguments that came from `${VAR}` expansions are TypeRef (variable refs);
    // bare library names that resolve to known targets are Calls.
    let raw_args = collect_raw_arguments(node, src);
    let normalized = collect_arguments(node, src);

    // Zip raw vs normalized to know which args were variable refs.
    for (i, (raw, norm)) in raw_args.iter().zip(normalized.iter()).enumerate() {
        if i == 0 {
            continue; // Skip target name.
        }
        if norm.is_empty() {
            continue; // Generator expression or empty.
        }
        if is_cmake_builtin(norm) {
            continue; // Skip keywords (PRIVATE, PUBLIC, CACHE, etc.)
        }
        // Was this a variable ref in the original source?
        let was_var_ref = raw.trim_start().starts_with("${")
            || raw.trim_start().starts_with("$ENV{")
            || raw.trim_start().starts_with("$CACHE{");
        let kind = if was_var_ref { EdgeKind::TypeRef } else { EdgeKind::Calls };
        refs.push(ExtractedRef {
            source_symbol_index: target_idx,
            target_name: norm.clone(),
            kind,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }
}

/// Walk the entire tree and emit a Function symbol for every `normal_command` node
/// not already extracted (e.g., those inside function/macro bodies).
/// Also extracts variable symbols from `set()`, `option()`, `list(APPEND ...)`,
/// and `mark_as_advanced()` inside function bodies.
pub(super) fn collect_all_normal_commands(
    node: Node,
    src: &str,
    existing_lines: &std::collections::HashSet<u32>,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // foreach_command lives inside a foreach_loop wrapper, not as a normal_command.
    // Emit the loop variable here so `${LOOP_VAR}` references resolve.
    if node.kind() == "foreach_command" {
        extract_foreach_loop_var(&node, src, symbols);
        // Continue walking into the body for nested commands.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_all_normal_commands(child, src, existing_lines, symbols, refs);
        }
        return;
    }

    if node.kind() == "normal_command" {
        let line = node.start_position().row as u32;
        if !existing_lines.contains(&line) {
            let cmd = command_identifier(&node, src).unwrap_or_else(|| "cmd".to_string());
            let cmd_lower = cmd.to_ascii_lowercase();

            // For variable-defining commands, extract variable symbols even inside bodies.
            match cmd_lower.as_str() {
                "set" | "option" => {
                    extract_set_command(&node, src, symbols);
                }
                "foreach" => {
                    extract_foreach_loop_var(&node, src, symbols);
                }
                "string" => {
                    extract_string_output_var(&node, src, symbols);
                }
                "get_filename_component" | "cmake_path" => {
                    extract_first_arg_output_var(&node, src, symbols, &cmd_lower);
                }
                "math" => {
                    extract_math_output_var(&node, src, symbols);
                }
                "find_program" | "find_library" | "find_path" | "find_file" => {
                    extract_first_arg_output_var(&node, src, symbols, &cmd_lower);
                }
                "separate_arguments" => {
                    extract_first_arg_output_var(&node, src, symbols, &cmd_lower);
                }
                "file" => {
                    extract_file_output_var(&node, src, symbols);
                }
                "cmake_parse_arguments" => {
                    extract_cmake_parse_arguments(&node, src, symbols);
                }
                "execute_process" => {
                    extract_execute_process_outputs(&node, src, symbols);
                }
                "find_package" => {
                    if let Some(pkg) = nth_argument(&node, src, 0) {
                        emit_find_package_vars(&node, &pkg, symbols);
                    }
                }
                "list" => {
                    // list(APPEND|PREPEND|INSERT|REMOVE_DUPLICATES NAME ...) — index NAME
                    let args = collect_arguments(&node, src);
                    let subcommand = args.first().map(|s| s.to_ascii_uppercase());
                    if matches!(
                        subcommand.as_deref(),
                        Some("APPEND") | Some("PREPEND") | Some("INSERT") | Some("REMOVE_DUPLICATES") | Some("SORT") | Some("REVERSE") | Some("FILTER") | Some("TRANSFORM") | Some("GET") | Some("JOIN")
                    ) {
                        if let Some(var_name) = args.get(1) {
                            if !var_name.is_empty() && !var_name.starts_with('$') {
                                let sig = format!("list({} {} ...)", subcommand.as_deref().unwrap_or(""), var_name);
                                symbols.push(make_symbol(var_name.clone(), var_name.clone(), SymbolKind::Variable, &node, Some(sig), None));
                            }
                        }
                    }
                }
                "mark_as_advanced" => {
                    // mark_as_advanced(VAR1 VAR2 ...) — index all names
                    for var_name in collect_arguments(&node, src) {
                        if !var_name.is_empty() && !var_name.starts_with('$') {
                            let sig = format!("mark_as_advanced({})", var_name);
                            symbols.push(make_symbol(var_name.clone(), var_name, SymbolKind::Variable, &node, Some(sig), None));
                        }
                    }
                }
                _ => {}
            }

            let sym_idx = symbols.len();
            symbols.push(make_symbol(
                cmd.clone(),
                cmd.clone(),
                SymbolKind::Function,
                &node,
                Some(format!("{}(...)", cmd)),
                None,
            ));
            // Only emit Calls ref for user-defined (non-builtin) commands.
            if !is_cmake_builtin(&cmd) {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: cmd,
                    kind: EdgeKind::Calls,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
    col: 0,
});
            }
        }
        return; // Don't recurse inside normal_command
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_all_normal_commands(child, src, existing_lines, symbols, refs);
    }
}
