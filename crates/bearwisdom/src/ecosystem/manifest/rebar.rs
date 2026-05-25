// =============================================================================
// ecosystem/manifest/rebar.rs — Erlang `rebar.config` / `rebar3.config` reader
//
// rebar3 is the canonical Erlang build tool. `rebar.config` is an Erlang
// term file:
//
//   {deps, [
//       {jsx, "3.1.0"},
//       {cowboy, {git, "https://github.com/ninenines/cowboy", {tag, "2.10"}}},
//       cowlib
//   ]}.
//
// We extract atom dep names from the top-level `{deps, [...]}` tuple.
// Project layouts using erlang.mk or Makefile-based builds aren't
// covered here; they need a separate reader.
// =============================================================================

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct RebarManifest;

impl ManifestReader for RebarManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Rebar }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        for name in &["rebar.config", "rebar3.config"] {
            let path = project_root.join(name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let mut data = ManifestData::default();
                for dep in parse_rebar_deps(&content) {
                    data.dependencies.insert(dep);
                }
                return Some(data);
            }
        }
        None
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut out = Vec::new();
        collect_rebar_configs(project_root, &mut out, 0);
        out
    }
}

fn collect_rebar_configs(dir: &Path, out: &mut Vec<ReaderEntry>, depth: u32) {
    if depth > 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "_build" | "deps" | ".git" | ".bearwisdom" | "ebin") {
                continue;
            }
            collect_rebar_configs(&path, out, depth + 1);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == "rebar.config" || name == "rebar3.config" {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let mut data = ManifestData::default();
                    for dep in parse_rebar_deps(&content) {
                        data.dependencies.insert(dep);
                    }
                    out.push(ReaderEntry {
                        package_dir: path.parent().unwrap_or(dir).to_path_buf(),
                        manifest_path: path,
                        data,
                        name: None,
                    });
                }
            }
        }
    }
}

/// Extract dep atom names from the `{deps, [...]}` tuple. Best-effort —
/// matches `{name, _}` and bare `name` entries. Doesn't attempt full
/// Erlang term parsing.
pub fn parse_rebar_deps(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Find the `{deps,` opening then read the matched `[` block.
    let Some(deps_idx) = content.find("{deps,") else { return out };
    let after = &content[deps_idx + 6..];
    let Some(list_start) = after.find('[') else { return out };
    let list_body = &after[list_start + 1..];
    // Find matching `]`, respecting nested brackets.
    let mut depth: i32 = 1;
    let mut end = 0;
    for (i, c) in list_body.char_indices() {
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => {
                depth -= 1;
                if depth == 0 { end = i; break }
            }
            _ => {}
        }
    }
    if end == 0 { return out }
    let body = &list_body[..end];
    // Top-level commas split deps, but commas inside nested {...} mustn't.
    let entries = split_top_level(body, ',');
    for entry in entries {
        let trimmed = entry.trim().trim_start_matches(['\n', ' ', '\t']);
        // Strip line comments.
        let no_comment = trimmed.split('%').next().unwrap_or(trimmed).trim();
        if no_comment.is_empty() { continue }
        // `{name, ...}` shape — take name token before first comma inside.
        if let Some(inner) = no_comment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let name = inner.split(',').next().unwrap_or("").trim();
            if !name.is_empty() { out.push(name.to_string()) }
        } else {
            // Bare atom or quoted atom.
            let name = no_comment.trim_matches('\'').trim();
            if !name.is_empty()
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                out.push(name.to_string());
            }
        }
    }
    out
}

fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth: i32 = 0;
    let mut last = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth -= 1,
            _ if c == sep && depth == 0 => {
                parts.push(&s[last..i]);
                last = i + c.len_utf8();
            }
            _ => {}
        }
    }
    if last < s.len() { parts.push(&s[last..]) }
    parts
}

#[cfg(test)]
#[path = "rebar_tests.rs"]
mod tests;
