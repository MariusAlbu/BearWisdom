// =============================================================================
// .NET binding inference for PowerShell
//
// Pre-pass that scans the raw source line-by-line for syntactic patterns
// that bind a `$variable` to a .NET type (`New-Object Foo.Bar`,
// `[Foo.Bar]::new()`, `[Foo.Bar]$x = ...`, cmdlet-result chains like
// `$x = (Get-Process).Name`). Each match emits a sentinel `Imports` ref with
// `target_name = DOTNET_BINDING_SENTINEL` and `module = Some(var_name)` that
// the resolver consumes in `build_file_context` to widen scope-local type
// information beyond the CST.
// =============================================================================

use super::extract::DOTNET_BINDING_SENTINEL;
use crate::ecosystem::powershell_cmdlet_types::{cmdlet_result_module_tag, cmdlet_return_type};
use crate::types::{EdgeKind, ExtractedRef};

pub(super) fn emit_dotnet_binding_sentinels(source: &str, refs: &mut Vec<ExtractedRef>) {
    let line_starts: Vec<u32> = {
        let mut offsets = vec![0u32];
        let mut pos: u32 = 0;
        for b in source.bytes() {
            pos += 1;
            if b == b'\n' { offsets.push(pos); }
        }
        offsets
    };
    // Track which registry var names and pipeline-var bindings we've already
    // emitted so we only push one sentinel per binding per file (dedup).
    let mut emitted_vars: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (line_no, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // ---- Pass 1: explicit .NET type assignment ----
        let binding = try_parse_new_object(line)
            .or_else(|| try_parse_type_new(line))
            .or_else(|| try_parse_typed_param(line));

        if let Some((var_name, dotnet_type)) = binding {
            if is_dotnet_type_name(&dotnet_type) && emitted_vars.insert(var_name.clone()) {
                refs.push(ExtractedRef {
                    source_symbol_index: 0,
                    target_name: DOTNET_BINDING_SENTINEL.to_string(),
                    kind: EdgeKind::Imports,
                    line: line_no as u32,
                    module: Some(var_name),
                    chain: None,
                    byte_offset: line_starts.get(line_no).copied().unwrap_or(0),
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }

        // ---- Part 1: hashtable-indexer registry variables ----
        // Detect patterns like `$sync["Key"].` or `$WPFApp["Key"].` and bind
        // the registry variable name to DependencyObject.
        for registry_var in HASHTABLE_REGISTRY_VARS {
            let pattern = format!("${registry_var}[");
            if line.contains(&pattern) || line.contains(&format!("${registry_var}.")) {
                let key = registry_var.to_string();
                if emitted_vars.insert(key.clone()) {
                    refs.push(ExtractedRef {
                        source_symbol_index: 0,
                        target_name: DOTNET_BINDING_SENTINEL.to_string(),
                        kind: EdgeKind::Imports,
                        line: line_no as u32,
                        module: Some(key),
                        chain: None,
                        byte_offset: line_starts.get(line_no).copied().unwrap_or(0),
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
        }

        // ---- Part 2: pipeline variable `$_` ----
        // If this line references `$_.` we emit a sentinel for `_` bound to
        // System.Windows.UIElement. One sentinel per file is enough.
        if line.contains("$_.") && emitted_vars.insert("_".to_string()) {
            refs.push(ExtractedRef {
                source_symbol_index: 0,
                target_name: DOTNET_BINDING_SENTINEL.to_string(),
                kind: EdgeKind::Imports,
                line: line_no as u32,
                module: Some("_".to_string()),
                chain: None,
                byte_offset: line_starts.get(line_no).copied().unwrap_or(0),
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }

        // ---- Part 3: cmdlet-result chains `(Get-Xxx).Member` ----
        // Scan for `(Get-Xxx).` patterns and emit a sentinel for the synthetic
        // module tag if `Get-Xxx` is in the cmdlet type table.
        if let Some(tag) = try_parse_cmdlet_result_chain(line) {
            if emitted_vars.insert(tag.clone()) {
                refs.push(ExtractedRef {
                    source_symbol_index: 0,
                    target_name: DOTNET_BINDING_SENTINEL.to_string(),
                    kind: EdgeKind::Imports,
                    line: line_no as u32,
                    module: Some(tag),
                    chain: None,
                    byte_offset: line_starts.get(line_no).copied().unwrap_or(0),
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }

        // ---- Part 4: propagation through member/index access ----
        // `$Tweaks = $sync.selectedTweaks` or `$val = $sync["Key"].Member` — if the
        // source variable is already bound to .NET (registry var, explicit type,
        // pipeline, cmdlet result, or an earlier propagation), inherit the binding.
        // The property/index read returns a .NET value in every realistic case
        // and `System.Windows.DependencyObject` is conservative enough to cover
        // it.  Registry vars are pre-seeded so `$Tweaks = $sync.foo` works even
        // on the very first line that touches `$sync`.
        if let Some((lhs, rhs_root)) = try_parse_propagation(line) {
            let bound = emitted_vars.contains(&rhs_root)
                || HASHTABLE_REGISTRY_VARS.iter().any(|v| *v == rhs_root.as_str());
            if bound && emitted_vars.insert(lhs.clone()) {
                refs.push(ExtractedRef {
                    source_symbol_index: 0,
                    target_name: DOTNET_BINDING_SENTINEL.to_string(),
                    kind: EdgeKind::Imports,
                    line: line_no as u32,
                    module: Some(lhs),
                    chain: None,
                    byte_offset: line_starts.get(line_no).copied().unwrap_or(0),
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
    }
}

/// Parse `$lhs = $rhs.<member>...` or `$lhs = $rhs[...]...` and return
/// `(lhs, rhs_root)` stripped of `$`. Returns `None` when the line isn't a
/// propagation assignment.
pub(crate) fn try_parse_propagation(line: &str) -> Option<(String, String)> {
    if !line.starts_with('$') {
        return None;
    }
    let eq_pos = line.find('=')?;
    // Skip compound assignments (`==`, `+=`, etc).
    let after_eq = line.get(eq_pos + 1..)?.chars().next();
    if matches!(after_eq, Some('=') | Some('~')) {
        return None;
    }
    let lhs_raw = line[..eq_pos].trim().trim_start_matches('$');
    let lhs = lhs_raw.split(':').next_back().unwrap_or(lhs_raw);
    if lhs.is_empty() || !lhs.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    let rhs = line[eq_pos + 1..].trim();
    if !rhs.starts_with('$') {
        return None;
    }
    // Skip the pipeline var `$_` — it already has its own sentinel.
    let rhs_name_start = &rhs[1..];
    let name_end = rhs_name_start
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rhs_name_start.len());
    if name_end == 0 {
        return None;
    }
    let rhs_root_raw = &rhs_name_start[..name_end];
    let rhs_root = rhs_root_raw.rsplit(':').next().unwrap_or(rhs_root_raw);
    // Require a `.` or `[` after the root so we know this is a member or
    // index read (not just `$a = $b` which carries no type information here).
    let after_root = rhs_name_start[name_end..].trim_start();
    if !(after_root.starts_with('.') || after_root.starts_with('[')) {
        return None;
    }

    Some((lhs.to_string(), rhs_root.to_string()))
}

/// Well-known WPF hashtable registry variable names used in PowerShell WPF
/// scripts. Accessing `$<name>["Key"]` returns a WPF element; members on
/// those elements are .NET framework members.
const HASHTABLE_REGISTRY_VARS: &[&str] = &["sync", "WPFApp", "script:sync"];

/// Try to parse a `(Get-Xxx).` pattern on the given line and return the
/// synthetic module tag for that cmdlet, if the cmdlet is in the type table.
///
/// Returns `None` if no known cmdlet result chain is detected on this line.
pub(crate) fn try_parse_cmdlet_result_chain(line: &str) -> Option<String> {
    // Look for `(Get-` sequence on the line (case-insensitive).
    let lower = line.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(paren_pos) = lower[search_from..].find("(get-") {
        let abs_pos = search_from + paren_pos;
        // Extract the cmdlet name: starts at abs_pos+1, ends at ')' or whitespace.
        let after_paren = &line[abs_pos + 1..];
        let cmdlet_end = after_paren
            .find(|c: char| c == ')' || c == ' ' || c == '\t' || c == '(')
            .unwrap_or(after_paren.len());
        let cmdlet_name = &after_paren[..cmdlet_end];
        if cmdlet_return_type(cmdlet_name).is_some() {
            return Some(cmdlet_result_module_tag(cmdlet_name));
        }
        search_from = abs_pos + 5; // advance past "(get-"
    }
    None
}

// ---------------------------------------------------------------------------
// .NET pattern parsers (shared with resolve.rs via pub(crate))
// ---------------------------------------------------------------------------

/// Try to parse `$var = New-Object [-TypeName] Type.Name [args...]`.
/// Returns `(var_name_without_dollar, type_name)` on success.
pub(crate) fn try_parse_new_object(line: &str) -> Option<(String, String)> {
    if !line.starts_with('$') {
        return None;
    }
    let eq_pos = line.find('=')?;
    let lhs = line[..eq_pos].trim();
    let rhs = line[eq_pos + 1..].trim();

    let var_raw = lhs.trim_start_matches('$');
    let var_name = if let Some(pos) = var_raw.find(':') {
        var_raw[pos + 1..].to_string()
    } else {
        var_raw.to_string()
    };
    if var_name.is_empty() {
        return None;
    }

    let rhs_lower = rhs.to_ascii_lowercase();
    if !rhs_lower.starts_with("new-object") {
        return None;
    }

    let after_cmd = rhs[10..].trim();
    let type_part = if after_cmd.to_ascii_lowercase().starts_with("-typename") {
        after_cmd[9..].trim()
    } else {
        after_cmd
    };

    // Type name is first whitespace-delimited token; strip trailing `(` for
    // patterns like `New-Object Windows.CornerRadius(10)`.
    let raw_token = type_part.split_ascii_whitespace().next()?;
    let type_name = raw_token
        .find('(')
        .map(|p| &raw_token[..p])
        .unwrap_or(raw_token)
        .to_string();
    if type_name.is_empty() {
        return None;
    }

    Some((var_name, type_name))
}

/// Try to parse `$var = [Type.Name]::new(...)`.
/// Returns `(var_name_without_dollar, type_name)` on success.
pub(crate) fn try_parse_type_new(line: &str) -> Option<(String, String)> {
    if !line.starts_with('$') {
        return None;
    }
    let eq_pos = line.find('=')?;
    let lhs = line[..eq_pos].trim();
    let rhs = line[eq_pos + 1..].trim();

    let var_raw = lhs.trim_start_matches('$');
    let var_name = if let Some(pos) = var_raw.find(':') {
        var_raw[pos + 1..].to_string()
    } else {
        var_raw.to_string()
    };
    if var_name.is_empty() {
        return None;
    }

    if !rhs.starts_with('[') {
        return None;
    }

    // Depth-counting scan to handle nested brackets in generic types:
    //   [System.Collections.Generic.List[string]]::new()
    //   [System.Collections.Hashtable]::new()
    // We count `[` depth to find the matching outer `]`.
    let close_bracket = find_matching_close_bracket(&rhs[1..])?;
    // close_bracket is relative to rhs[1..], so absolute index = close_bracket + 1
    let abs_close = close_bracket + 1;
    let raw_type = rhs[1..abs_close].trim();
    if raw_type.is_empty() {
        return None;
    }
    // Strip generic type arguments for the stored type name:
    //   "System.Collections.Generic.List[string]" → "System.Collections.Generic.List"
    let type_name = strip_type_args(raw_type);
    if type_name.is_empty() {
        return None;
    }

    let after_bracket = rhs[abs_close + 1..].trim_start();
    if !after_bracket.to_ascii_lowercase().starts_with("::new") {
        return None;
    }

    Some((var_name, type_name))
}

/// Try to parse `[Type.Name]$var` typed parameter / variable.
/// Returns `(var_name_without_dollar, type_name)` on success.
pub(crate) fn try_parse_typed_param(line: &str) -> Option<(String, String)> {
    if !line.starts_with('[') {
        return None;
    }

    let close_bracket = line.find(']')?;
    let type_name = line[1..close_bracket].trim().to_string();
    if type_name.is_empty() {
        return None;
    }

    let after_bracket = line[close_bracket + 1..].trim_start();
    if !after_bracket.starts_with('$') {
        return None;
    }

    let rest = &after_bracket[1..];
    let name_end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    let var_name = rest[..name_end].to_string();
    if var_name.is_empty() {
        return None;
    }

    Some((var_name, type_name))
}

/// Returns `true` if `type_name` looks like a .NET framework namespace path.
/// Must contain a `.` and start with a recognised top-level namespace segment.
/// Generic type args (e.g. `[string]`, `<int>`) are stripped before checking.
pub(crate) fn is_dotnet_type_name(type_name: &str) -> bool {
    let base = strip_type_args(type_name);
    if !base.contains('.') {
        return false;
    }
    let root = base.split('.').next().unwrap_or("");
    matches!(
        root,
        "System"
            | "Microsoft"
            | "Windows"
            | "WPF"
            | "PresentationFramework"
            | "PresentationCore"
    )
}

/// Strip PowerShell-style generic type arguments from a type name.
///
/// Examples:
///   "System.Collections.Generic.List[string]"  → "System.Collections.Generic.List"
///   "System.Collections.Hashtable"             → "System.Collections.Hashtable"
///   "System.Action[string,int]"                → "System.Action"
fn strip_type_args(s: &str) -> String {
    // Strip everything from the first `[` or `<` that follows an identifier char.
    let bracket_pos = s
        .char_indices()
        .find(|&(_, c)| c == '[' || c == '<')
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s[..bracket_pos].trim_end().to_string()
}

/// Depth-counting scan for the closing `]` that matches the opening `[`
/// which is assumed to appear just before `s` (i.e. `s` starts immediately
/// after the opening `[`).
///
/// Returns the index within `s` of the matching `]`, or `None` if not found.
fn find_matching_close_bracket(s: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
