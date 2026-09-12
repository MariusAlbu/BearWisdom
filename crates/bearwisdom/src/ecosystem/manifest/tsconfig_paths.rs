// =============================================================================
// ecosystem/manifest/tsconfig_paths.rs — `compilerOptions.paths` as alias data
//
// A wildcard entry (`"@/*": ["src/*"]`) is a prefix rewrite; an exact entry
// (`"next-test-utils": ["./test/lib/next-test-utils"]`) rewrites one
// specifier and nothing that merely starts with it. The two are kept apart so
// no consumer has to infer exactness from spelling.
// =============================================================================

use std::path::{Path, PathBuf};

/// The alias rewrites one tsconfig (with its `extends` chain) declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsconfigAliases {
    /// `(alias_prefix, target_prefix)` with the trailing `*` stripped.
    pub prefixes: Vec<(String, String)>,
    /// `(specifier, target)` for entries without a wildcard.
    pub exact: Vec<(String, String)>,
}

impl TsconfigAliases {
    /// First writer wins per key: a more derived config's entry shadows the
    /// one an ancestor declares.
    fn absorb(&mut self, other: TsconfigAliases) {
        for entry in other.prefixes {
            if !self.prefixes.iter().any(|(k, _)| *k == entry.0) {
                self.prefixes.push(entry);
            }
        }
        for entry in other.exact {
            if !self.exact.iter().any(|(k, _)| *k == entry.0) {
                self.exact.push(entry);
            }
        }
    }
}

/// Parse `compilerOptions.paths` from one tsconfig's content. Strips `//` and
/// `/* */` comments first so JSONC configs parse. Does not follow `extends`.
/// The first target of each entry is taken; an entry whose wildcard is not
/// trailing on both sides is skipped.
pub fn parse_tsconfig_aliases(content: &str) -> TsconfigAliases {
    let stripped = super::strip_json_comments(content);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return TsconfigAliases::default();
    };
    let Some(paths) = value
        .get("compilerOptions")
        .and_then(|co| co.get("paths"))
        .and_then(|p| p.as_object())
    else {
        return TsconfigAliases::default();
    };
    let mut out = TsconfigAliases::default();
    for (key, targets) in paths {
        let Some(first) = targets
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        match (key.strip_suffix('*'), first.strip_suffix('*')) {
            (Some(alias_prefix), Some(target_prefix)) => {
                if !alias_prefix.is_empty() {
                    out.prefixes
                        .push((alias_prefix.to_string(), target_prefix.to_string()));
                }
            }
            (None, None) => {
                if !key.is_empty() && !key.contains('*') && !first.contains('*') {
                    out.exact.push((key.clone(), first.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

/// The prefix rewrites alone.
pub fn parse_tsconfig_paths(content: &str) -> Vec<(String, String)> {
    parse_tsconfig_aliases(content).prefixes
}

/// Like `parse_tsconfig_aliases` but follows the `extends` chain, so a
/// package that declares its `paths` in a shared base config (monorepos,
/// `@tsconfig/*` presets) still contributes aliases. Resolves relative
/// (`./base.json`, `../tsconfig.base.json`) and package
/// (`@org/cfg/web.json`, `@tsconfig/node18/tsconfig.json`) `extends` targets.
/// Child entries win over inherited ones on key conflict. Bounded depth with a
/// visited-set cycle guard.
pub fn parse_tsconfig_aliases_with_extends(tsconfig_path: &Path) -> TsconfigAliases {
    let mut out = TsconfigAliases::default();
    let mut seen = std::collections::HashSet::new();
    collect_tsconfig_paths(
        tsconfig_path,
        &|p| std::fs::read_to_string(p).ok(),
        &mut out,
        &mut seen,
        0,
    );
    out
}

/// The prefix rewrites of `parse_tsconfig_aliases_with_extends`.
pub fn parse_tsconfig_paths_with_extends(tsconfig_path: &Path) -> Vec<(String, String)> {
    parse_tsconfig_aliases_with_extends(tsconfig_path).prefixes
}

const MAX_TSCONFIG_EXTENDS_DEPTH: usize = 8;

/// Walk a tsconfig and its `extends` ancestors, accumulating aliases.
/// `read` returns a file's content or `None` when it doesn't exist — folding
/// existence and content into one closure keeps the extends-resolution logic
/// testable without touching the filesystem.
pub(super) fn collect_tsconfig_paths(
    path: &Path,
    read: &dyn Fn(&Path) -> Option<String>,
    out: &mut TsconfigAliases,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    if depth >= MAX_TSCONFIG_EXTENDS_DEPTH || !seen.insert(path.to_path_buf()) {
        return;
    }
    let Some(content) = read(path) else { return };
    // The current (more derived) config's aliases are absorbed before its
    // ancestors', so a child key shadows the parent's.
    out.absorb(parse_tsconfig_aliases(&content));
    let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
    for target in tsconfig_extends_targets(&content) {
        for candidate in extends_candidate_paths(&target, base_dir) {
            if read(&candidate).is_some() {
                collect_tsconfig_paths(&candidate, read, out, seen, depth + 1);
                break;
            }
        }
    }
}

/// Extract the `extends` field as a list of targets. TS 5.0+ allows an array;
/// older configs use a single string.
fn tsconfig_extends_targets(content: &str) -> Vec<String> {
    let stripped = super::strip_json_comments(content);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return Vec::new();
    };
    match value.get("extends") {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Candidate on-disk locations for an `extends` target, in priority order.
/// Relative targets resolve against `base_dir` (a missing `.json` is appended);
/// package targets resolve under `node_modules`, walking up from `base_dir`,
/// trying `<pkg>.json` then `<pkg>/tsconfig.json`.
fn extends_candidate_paths(target: &str, base_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if target.starts_with('.') {
        let rel = target.trim_start_matches("./");
        let joined = base_dir.join(rel);
        if joined.extension().is_some() {
            out.push(joined);
        } else {
            out.push(base_dir.join(format!("{rel}.json")));
        }
        return out;
    }
    let mut dir = Some(base_dir);
    while let Some(d) = dir {
        let nm = d.join("node_modules").join(target);
        if target.ends_with(".json") {
            out.push(nm);
        } else {
            out.push(nm.with_extension("json"));
            out.push(nm.join("tsconfig.json"));
        }
        dir = d.parent();
    }
    out
}

#[cfg(test)]
#[path = "tsconfig_paths_tests.rs"]
mod tests;
