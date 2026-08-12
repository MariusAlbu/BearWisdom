// =============================================================================
// languages/pascal/main_unit.rs — {%MainUnit} fragment scope inheritance
//
// Castle-Game-Engine-style Pascal projects splice `.inc` fragments into a
// parent unit via `{$I fragment.inc}` and tag each fragment with a leading
// `{%MainUnit ParentFile.pas}` directive (the argument may carry a relative
// path, e.g. `{%MainUnit ../castleutils.pas}` for a fragment one directory
// below its parent). A fragment indexes as its own file with no `uses`
// clause of its own — the parent unit file carries the real `uses` list —
// while every declaration spliced across the unit's fragments shares ONE
// Pascal scope regardless of which physical file it sits in.
//
// This module builds, once per index pass, a fragment file path → list of
// wildcard-eligible unit names: the parent unit's own name (so sibling
// fragments spliced into the same unit resolve through
// `WildcardMatch::FileStem`'s underscore-prefix probe against the parent's
// name) plus every unit named in the parent's own `uses` clause.
// =============================================================================

use std::collections::HashMap;

use crate::types::{EdgeKind, ParsedFile, SymbolKind};

use super::include_directives::file_stem_of;

const MAIN_UNIT_EXTENSIONS: &[&str] = &[".pas", ".pp", ".dpr", ".dpk"];

/// Cross-file Pascal state: fragment file path → the wildcard-eligible unit
/// names its `{%MainUnit}` splice target should contribute.
#[derive(Debug, Default)]
pub struct PascalProjectState {
    fragment_wildcards: HashMap<String, Vec<String>>,
}

impl PascalProjectState {
    pub(crate) fn wildcards_for(&self, file_path: &str) -> &[String] {
        self.fragment_wildcards
            .get(file_path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[cfg(test)]
    pub(crate) fn from_map(fragment_wildcards: HashMap<String, Vec<String>>) -> Self {
        Self { fragment_wildcards }
    }
}

/// Build the fragment-path → wildcard-unit-names map across the whole
/// project. Internal-only: every input file this reads is available in the
/// eager pre-externals batch, so `populate_project_state` alone is enough —
/// no `_post_externals` rebuild is needed.
pub fn build_main_unit_state(
    parsed: &[ParsedFile],
    project_root: &std::path::Path,
) -> PascalProjectState {
    let mut by_stem: HashMap<String, &str> = HashMap::new();
    let mut by_path_lower: HashMap<String, &str> = HashMap::new();
    let mut uses_by_path: HashMap<&str, Vec<String>> = HashMap::new();
    for pf in parsed {
        if pf.language != "pascal" || !is_main_unit_file(&pf.path) {
            continue;
        }
        by_stem
            .entry(file_stem_lower(&pf.path))
            .or_insert(pf.path.as_str());
        by_path_lower.insert(pf.path.to_ascii_lowercase(), pf.path.as_str());
        uses_by_path.insert(pf.path.as_str(), uses_clause_units(pf));
    }

    let mut fragment_wildcards = HashMap::new();
    for pf in parsed {
        if pf.language != "pascal" || !is_fragment_file(&pf.path) {
            continue;
        }
        // The streaming index pipeline strips `ParsedFile.content` after each
        // per-file write, so an internal fragment usually arrives content-less
        // — its source is re-read from disk under `project_root`.
        let reread;
        let src = match pf.content.as_deref() {
            Some(s) => s,
            None if !pf.path.starts_with("ext:") => {
                match std::fs::read_to_string(project_root.join(&pf.path)) {
                    Ok(s) => {
                        reread = s;
                        &reread
                    }
                    Err(_) => continue,
                }
            }
            None => continue,
        };
        let Some(directive) = parse_main_unit_directive(src) else {
            continue;
        };
        let Some(main_stem) = file_stem_of(&directive) else {
            continue;
        };

        let resolved_path = resolve_relative(&pf.path, &directive)
            .and_then(|joined| by_path_lower.get(&joined.to_ascii_lowercase()).copied())
            .or_else(|| by_stem.get(&main_stem.to_ascii_lowercase()).copied());

        let mut wildcards = vec![main_stem];
        if let Some(main_path) = resolved_path {
            if let Some(uses) = uses_by_path.get(main_path) {
                wildcards.extend(uses.iter().cloned());
            }
        }
        fragment_wildcards.insert(pf.path.clone(), wildcards);
    }

    PascalProjectState { fragment_wildcards }
}

/// The unit names a main-unit file's own `uses` clause names — read off the
/// `Imports` refs `extract_uses` anchors to its dedicated `"uses"` Namespace
/// symbol. Excludes the `{$include}` directive's own `Imports` refs (same
/// `EdgeKind`, anchored to the unit/program root symbol instead), which
/// name fragment stems rather than real units.
fn uses_clause_units(pf: &ParsedFile) -> Vec<String> {
    pf.refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .filter_map(|r| {
            let sym = pf.symbols.get(r.source_symbol_index)?;
            if sym.kind == SymbolKind::Namespace && sym.name == "uses" {
                r.module.clone()
            } else {
                None
            }
        })
        .collect()
}

fn is_main_unit_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    MAIN_UNIT_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

fn is_fragment_file(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".inc")
}

fn file_stem_lower(path: &str) -> String {
    file_stem_of(path).unwrap_or_default().to_ascii_lowercase()
}

/// Scan a fragment's raw source for a `{%MainUnit ParentFile.pas}` directive
/// and return its raw filename argument (directory prefix and extension
/// intact). `None` when no such directive is present.
fn parse_main_unit_directive(src: &str) -> Option<String> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] != b'{' || i + 1 >= len || bytes[i + 1] != b'%' {
            i += 1;
            continue;
        }
        let close = match bytes[i..].iter().position(|&b| b == b'}') {
            Some(p) => i + p,
            None => break, // unterminated directive — nothing further to scan
        };
        let inner = &src[i + 2..close];
        let lower = inner.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("mainunit") {
            if rest.starts_with(|c: char| c.is_whitespace()) {
                let arg = inner["mainunit".len()..].trim();
                if !arg.is_empty() {
                    return Some(arg.to_string());
                }
            }
        }
        i = close + 1;
    }
    None
}

/// Join a `{%MainUnit}` argument against the directory of `fragment_path`,
/// normalizing `.`/`..` segments (both use `/`-separated paths, matching how
/// `ParsedFile::path` is stored). `None` when the argument's `..` segments
/// walk above the fragment's own root, or when the argument is absolute.
fn resolve_relative(fragment_path: &str, directive: &str) -> Option<String> {
    let directive = directive.replace('\\', "/");
    let dir = fragment_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut segments: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for part in directive.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            seg => segments.push(seg),
        }
    }
    Some(segments.join("/"))
}

#[cfg(test)]
#[path = "main_unit_tests.rs"]
mod tests;
