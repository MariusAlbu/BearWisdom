//! Ansible role-variable extraction for YAML files.
//!
//! Recognizes Ansible role-tree paths and emits one `Field` symbol per
//! top-level YAML key, qualified under the role/group/host scope derived
//! from the path. Symbols produced here flow through the standard indexing
//! pipeline so the Jinja resolver can find them via `SymbolIndex::by_name`
//! and `SymbolIndex::by_qname` without any extra cross-file state.
//!
//! Recognized path shapes (case-sensitive):
//!   `roles/<role>/defaults/main.{yml,yaml}`   → scope `<role>`
//!   `roles/<role>/vars/main.{yml,yaml}`        → scope `<role>`
//!   `group_vars/<name>.{yml,yaml}`             → scope `group_vars.<name>`
//!   `group_vars/<name>/<file>.{yml,yaml}`      → scope `group_vars.<name>`
//!   `host_vars/<name>.{yml,yaml}`              → scope `host_vars.<name>`
//!   `host_vars/<name>/<file>.{yml,yaml}`       → scope `host_vars.<name>`
//!   `inventory/group_vars/<name>.{yml,yaml}`   → scope `group_vars.<name>`
//!   `inventory/group_vars/<name>/<file>.yml`   → scope `group_vars.<name>`
//!   `inventory/host_vars/<name>.{yml,yaml}`    → scope `host_vars.<name>`
//!   `inventory/host_vars/<name>/<file>.yml`    → scope `host_vars.<name>`
//!
//! When the path matches none of these, `classify_ansible_path` returns
//! `None` and the caller falls through to the generic YAML extractor.

use crate::types::{ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

/// Classify a YAML file path as an Ansible role-variable source.
///
/// `file_path` should already be slash-normalised (`\` → `/`).
///
/// Returns `Some(scope)` when the path is an Ansible variable file, where
/// `scope` is the string to use as the qualified-name prefix for every
/// top-level key (e.g. `"matrix_base"` or `"group_vars.webservers"`).
/// Returns `None` for non-Ansible YAML files.
pub fn classify_ansible_path(file_path: &str) -> Option<String> {
    let p = file_path.replace('\\', "/");
    let segs: Vec<&str> = p.split('/').collect();
    let n = segs.len();
    if n < 2 {
        return None;
    }

    // Normalised file name (last segment), lower-cased for extension check.
    let last = segs[n - 1];
    if !last.ends_with(".yml") && !last.ends_with(".yaml") {
        return None;
    }

    // -----------------------------------------------------------------------
    // roles/<role>/defaults/main.{yml,yaml}
    // roles/<role>/vars/main.{yml,yaml}
    // -----------------------------------------------------------------------
    // Accept both flat (`roles/webserver/defaults/main.yml`) and nested under
    // an arbitrary intermediate directory (`roles/custom/<role>/defaults/main.yml`).
    // We scan backward from `defaults` or `vars` to find the role segment.
    for i in (1..n.saturating_sub(1)).rev() {
        if (segs[i] == "defaults" || segs[i] == "vars") && i + 1 == n - 1 {
            let file_stem = stem(last);
            if file_stem != "main" {
                break;
            }
            // The segment just before `defaults`/`vars` is the role name.
            if i > 0 {
                // Walk up to find a `roles` ancestor.
                for j in (0..i).rev() {
                    if segs[j] == "roles" {
                        let role = segs[i - 1];
                        if !role.is_empty() {
                            return Some(role.to_string());
                        }
                    }
                }
            }
            break;
        }
    }

    // -----------------------------------------------------------------------
    // group_vars/<name>.yml
    // group_vars/<name>/<any>.yml
    // inventory/group_vars/<name>.yml
    // inventory/group_vars/<name>/<any>.yml
    // -----------------------------------------------------------------------
    if let Some(scope) = scan_vars_dir(&segs, n, "group_vars", "group_vars") {
        return Some(scope);
    }

    // -----------------------------------------------------------------------
    // host_vars/<name>.yml
    // host_vars/<name>/<any>.yml
    // inventory/host_vars/<name>.yml
    // inventory/host_vars/<name>/<any>.yml
    // -----------------------------------------------------------------------
    if let Some(scope) = scan_vars_dir(&segs, n, "host_vars", "host_vars") {
        return Some(scope);
    }

    None
}

/// Scan for `<vars_dir>/<name>[/<file>].yml` at any depth in the path.
///
/// Returns `Some("<prefix>.<name>")` when found, where `prefix` is the
/// distinguishing namespace passed in (e.g. `"group_vars"`).
fn scan_vars_dir(segs: &[&str], n: usize, dir: &str, prefix: &str) -> Option<String> {
    for i in 0..n.saturating_sub(1) {
        if segs[i] != dir {
            continue;
        }
        // `group_vars/<name>.yml` — the name is the file stem.
        if i + 1 == n - 1 {
            let name = stem(segs[n - 1]);
            if !name.is_empty() {
                return Some(format!("{prefix}.{name}"));
            }
        }
        // `group_vars/<name>/<file>.yml` — the name is the directory.
        if i + 2 <= n - 1 {
            let name = segs[i + 1];
            if !name.is_empty() && !name.starts_with('.') {
                return Some(format!("{prefix}.{name}"));
            }
        }
    }
    None
}

/// Extract Ansible role-variable symbols from a YAML file.
///
/// Emits one `Field` per top-level key, qualified as `<scope>.<key>`.
/// The file-scope `Class` symbol uses `<scope>` as both name and
/// qualified name so cross-file `{% include %}` style refs can still
/// bind to the file. Skips comment lines, blank lines, and indented
/// (non-top-level) keys.
pub fn extract_ansible(source: &str, file_path: &str, scope: &str) -> ExtractionResult {
    let mut symbols = vec![ExtractedSymbol {
        name: scope.to_string(),
        qualified_name: scope.to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }];

    for (line_no, line) in source.lines().enumerate() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // Top-level keys start at column 0 (no indent), followed by `:`.
        if line.starts_with(|c: char| c.is_whitespace()) || line.starts_with('-') {
            continue;
        }
        // Skip YAML document markers.
        if line.starts_with("---") || line.starts_with("...") {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim();
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
            {
                symbols.push(ExtractedSymbol {
                    name: key.to_string(),
                    qualified_name: format!("{scope}.{key}"),
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
                    start_line: line_no as u32,
                    end_line: line_no as u32,
                    start_col: 0,
                    end_col: key.len() as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some(scope.to_string()),
                    parent_index: Some(0),
                });
            }
        }
    }

    let _ = file_path; // accepted for future diagnostics
    ExtractionResult::new(symbols, Vec::new(), false)
}

fn stem(file_name: &str) -> &str {
    let base = file_name
        .rsplit('/')
        .next()
        .unwrap_or(file_name);
    // Strip YAML extensions: `.yaml` or `.yml`.
    if let Some(s) = base.strip_suffix(".yaml") {
        return s;
    }
    if let Some(s) = base.strip_suffix(".yml") {
        return s;
    }
    base
}

// ---------------------------------------------------------------------------
// Tests live in sibling file.
// ---------------------------------------------------------------------------
#[cfg(test)]
#[path = "ansible_tests.rs"]
mod tests;
