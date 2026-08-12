// =============================================================================
// ecosystem/fpc_fragment_index.rs — FPC RTL symbol location index
//
// Builds the `(module, name) -> file` index the demand-driven resolver
// consults to locate FPC RTL declarations. FPC unit files (`system.pp`,
// `sysutils.pp`, ...) splice their real interface declarations in via
// `{$I fragment.inc}` directives rather than declaring them inline, so the
// scan treats `.inc` fragments as independently locatable declaration
// sources alongside `.pas`/`.pp` units: a fragment carries no `unit` header
// and no `interface` keyword of its own, so its entire body IS the
// declaration list from line 1.
//
// Header-only line scan, not a tree-sitter parse — see `symbol_index.rs` for
// the shape/query surface this builds. The real tree-sitter extraction runs
// later, when the demand-driven resolver pulls the located file; `.inc`
// fragments parse through the Pascal extractor's existing bare-fragment
// error-recovery path, which requires no unit wrapper.
// =============================================================================

use std::path::Path;

use tracing::debug;

use crate::ecosystem::externals::{ExternalDepRoot, MAX_WALK_DEPTH};
use crate::ecosystem::symbol_index::SymbolLocationIndex;

/// Build a `(module_path, name) → file` index over every Pascal source file
/// in `dep_roots` without a full tree-sitter parse. Two name shapes are
/// registered per file:
///
/// 1. The **unit name** extracted from the `unit <Name>;` declaration at the
///    top of each `.pas`/`.pp` — this is what a `uses SysUtils;` clause in
///    project code resolves against. Registered under both the unit name and
///    a lower-cased copy (Pascal identifiers are case-insensitive).
///
/// 2. Top-level declarations in the **interface section** (or, for `.inc`
///    fragments, the whole file): identifiers following `type`, `procedure`,
///    `function`, `var`, `const`, and `class` keywords on their own line.
///    These are the bare names a project uses after a `uses SysUtils`
///    (`Copy`, `Format`, `TStringList`, ...).
///
/// The scan reads each file line by line and stops at `implementation` so it
/// never descends into function bodies — keeping the scan O(interface size)
/// rather than O(file size).
pub(crate) fn build_pascal_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut idx = SymbolLocationIndex::new();
    for dep in dep_roots {
        collect_pascal_names_rec(&dep.root, dep, &mut idx, 0);
    }
    if !idx.is_empty() {
        debug!("freepascal: indexed {} Pascal symbol locations", idx.len());
    }
    idx
}

fn collect_pascal_names_rec(
    dir: &Path,
    dep: &ExternalDepRoot,
    idx: &mut SymbolLocationIndex,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "examples" | "demos" | "languages" | "images") {
                    continue;
                }
                if name.starts_with('.') { continue }
            }
            collect_pascal_names_rec(&path, dep, idx, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let lower = name.to_ascii_lowercase();
            // `.lpr` files are Lazarus project entry points, not units —
            // irrelevant for external RTL indexing. `.inc` fragments ARE
            // scanned: FPC's RTL units splice their real declarations in via
            // `{$I}`, so the fragment is where the interface actually lives.
            if !lower.ends_with(".pas") && !lower.ends_with(".pp") && !lower.ends_with(".inc") {
                continue;
            }
            scan_pascal_file(&path, dep, idx);
        }
    }
}

/// Scan a single Pascal source file and register all unit-level names in `idx`.
///
/// Keyword matching is case-insensitive (Pascal convention). Identifiers are
/// registered under their as-declared form AND their lowercase form so
/// callers need not know the declaration casing.
///
/// `.inc` fragments carry no `unit <Name>;` header and no `interface`
/// keyword of their own — they're spliced into a parent unit via `{$I}`.
/// The scan treats the whole fragment body as interface-section content from
/// line 1, so it never waits for a `unit`/`interface` marker that will never
/// appear.
fn scan_pascal_file(path: &Path, dep: &ExternalDepRoot, idx: &mut SymbolLocationIndex) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let module = &dep.module_path;

    let is_fragment = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("inc"));

    let mut in_interface = is_fragment;
    let mut past_unit_decl = is_fragment;
    // When the previous interface line was a section keyword on its own
    // (`type`, `var`, `const`), record which keyword so the next
    // identifier-only line is treated as a declaration in that section.
    let mut pending_section: Option<&str> = None;

    for raw_line in content.lines() {
        let stripped = strip_pascal_line_comment(raw_line).trim();
        if stripped.is_empty() { continue }
        // Lower-case copy for keyword matching; the original `stripped` slice
        // preserves case for identifier registration.
        let lower = stripped.to_ascii_lowercase();

        // Extract the unit name from the `unit <Name>;` header.
        if !past_unit_decl {
            if let Some(rest) = lower.strip_prefix("unit ") {
                // Extract the original-case unit name by slicing `stripped`.
                let original_rest = &stripped["unit ".len()..];
                let unit_name = original_rest.trim_end_matches(';').trim();
                if !unit_name.is_empty() && is_pascal_ident(unit_name) {
                    idx.insert(module, unit_name, path);
                    let lc = unit_name.to_ascii_lowercase();
                    if lc != unit_name { idx.insert(module, &lc, path); }
                }
                // Consume the rest variable to avoid an unused-variable warning.
                let _ = rest;
                past_unit_decl = true;
            }
            continue;
        }

        if lower == "interface" {
            in_interface = true;
            pending_section = None;
            continue;
        }
        if lower == "implementation" {
            break;
        }

        if !in_interface { continue }

        // Detect bare section keywords (`type`, `var`, `const`) on their own
        // line — common Pascal style for a block of declarations. A line is
        // "bare" when the keyword is the entire content (no following ident).
        let is_bare_section = matches!(lower.as_str(), "type" | "var" | "const");
        if is_bare_section {
            pending_section = Some(match lower.as_str() {
                "type" => "type ",
                "var" => "var ",
                _ => "const ",
            });
            continue;
        }

        // A line that starts with `procedure`, `function`, or `class` (with a
        // space following, meaning it has an ident on the same line) belongs to
        // the top-level interface — clear any pending section context so these
        // are parsed via `extract_decl_ident` rather than the bare-ident path.
        let starts_new_decl = lower.starts_with("procedure ")
            || lower.starts_with("function ")
            || lower.starts_with("class ");
        if starts_new_decl {
            pending_section = None;
        }

        // Try to extract a declared identifier. Both branches recover the
        // original-case spelling from `stripped` using the byte offset
        // determined from the lowercase `lower` slice (byte lengths are
        // identical for ASCII identifiers).
        let ident: Option<&str> = if let Some(_section) = pending_section {
            // Line directly follows a bare section keyword (`type`, `var`,
            // `const`). The identifier starts at position 0 of the line.
            let end = lower
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(lower.len());
            let name_lower = &lower[..end];
            if !name_lower.is_empty() && is_pascal_ident(name_lower) {
                Some(&stripped[..end])
            } else {
                None
            }
        } else {
            extract_decl_ident(&lower).map(|found| {
                let offset = found.as_ptr() as usize - lower.as_ptr() as usize;
                &stripped[offset..offset + found.len()]
            })
        };

        if let Some(name) = ident {
            if !name.is_empty() && is_pascal_ident(name) {
                idx.insert(module, name, path);
                let lc = name.to_ascii_lowercase();
                if lc != name { idx.insert(module, &lc, path); }
            }
        }
    }
}

/// Return the declared identifier from a Pascal interface-section
/// declaration line, or `None` if the line doesn't match a recognized pattern.
///
/// Recognised forms (case-insensitive):
///   - `procedure Foo` / `procedure Foo(...)` → `"Foo"`
///   - `function Bar(...)` → `"Bar"`
///   - `type TMyClass` / `type TMyClass = ...` → `"TMyClass"`
///   - `var FField: TType` → `"FField"`
///   - `const MAX_SIZE = ...` → `"MAX_SIZE"`
///   - `class TFoo` → `"TFoo"`
fn extract_decl_ident(line: &str) -> Option<&str> {
    for kw in &["procedure ", "function ", "type ", "var ", "const ", "class "] {
        if let Some(rest) = line.strip_prefix(kw) {
            let rest = rest.trim_start();
            // Take the identifier up to the first non-identifier character.
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if !name.is_empty() && is_pascal_ident(name) {
                return Some(name);
            }
        }
    }
    None
}

/// True when `s` is a valid Pascal identifier: starts with a letter or
/// underscore, followed by letters, digits, or underscores.
fn is_pascal_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_alphabetic() && first != '_' { return false }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Remove a `//` or `{...}` line comment from `line`, returning the
/// text before the comment. Block comments `{...}` spanning a single line
/// are stripped; multi-line `{...}` blocks are not tracked here — they're
/// uncommon in interface-section declaration lines and treated as noise.
fn strip_pascal_line_comment(line: &str) -> &str {
    // Slash-slash line comment.
    if let Some(idx) = line.find("//") {
        return &line[..idx];
    }
    // Single-line brace comment: `{ ... }`.
    if let Some(open) = line.find('{') {
        if let Some(close) = line[open..].find('}') {
            let _ = close; // close position within the slice
            // Return everything before the `{`.
            return &line[..open];
        }
    }
    line
}

#[cfg(test)]
#[path = "fpc_fragment_index_tests.rs"]
mod tests;
