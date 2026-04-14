// Erlang / rebar3 externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::Path;
use tracing::debug;

/// Erlang rebar3 → `discover_erlang_externals` + `walk_erlang_external_root`.
///
/// rebar3 fetches deps into `_build/default/lib/<dep>/`. Declared deps
/// come from `rebar.config` `{deps, [...]}`. Walk: `src/**/*.erl`.
pub struct ErlangExternalsLocator;

impl ExternalSourceLocator for ErlangExternalsLocator {
    fn ecosystem(&self) -> &'static str { "erlang" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_erlang_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_erlang_external_root(dep)
    }
}

pub fn discover_erlang_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let rebar_config = project_root.join("rebar.config");
    if !rebar_config.is_file() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&rebar_config) else {
        return Vec::new();
    };
    let declared = parse_rebar_deps(&content);
    if declared.is_empty() {
        return Vec::new();
    }

    let deps_dir = project_root.join("_build").join("default").join("lib");
    if !deps_dir.is_dir() {
        return Vec::new();
    }

    let mut roots = Vec::new();
    for dep_name in &declared {
        let dep_dir = deps_dir.join(dep_name);
        if dep_dir.is_dir() {
            roots.push(ExternalDepRoot {
                module_path: dep_name.clone(),
                version: String::new(),
                root: dep_dir,
                ecosystem: "erlang",
                package_id: None,
            });
        }
    }
    debug!("Erlang: discovered {} external package roots", roots.len());
    roots
}

/// Parse dep names from rebar.config `{deps, [...]}` section.
pub fn parse_rebar_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("{deps,") else {
        return deps;
    };
    let rest = &content[start..];
    let Some(bracket_start) = rest.find('[') else {
        return deps;
    };
    let rest = &rest[bracket_start..];
    let Some(bracket_end) = rest.find(']') else {
        return deps;
    };
    let deps_block = &rest[1..bracket_end];

    // Top-level dep tuples: {atom, ...}. We track brace depth to only
    // match the first atom of depth-1 tuples, skipping nested {git,...}.
    let mut depth = 0u32;
    let mut in_atom = false;
    let mut atom_start = 0usize;
    for (i, ch) in deps_block.char_indices() {
        match ch {
            '{' => {
                depth += 1;
                if depth == 1 {
                    in_atom = true;
                    atom_start = i + 1;
                }
            }
            ',' | '}' if depth == 1 && in_atom => {
                let name = deps_block[atom_start..i].trim();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    deps.push(name.to_string());
                }
                in_atom = false;
                if ch == '}' { depth -= 1; }
            }
            '}' => { depth = depth.saturating_sub(1); }
            _ => {}
        }
    }
    deps
}

pub fn walk_erlang_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    let src_dir = dep.root.join("src");
    if src_dir.is_dir() {
        walk_erlang_dir(&src_dir, &dep.root, dep, &mut out);
    }
    // Also check include/ for header files
    let include_dir = dep.root.join("include");
    if include_dir.is_dir() {
        walk_erlang_dir(&include_dir, &dep.root, dep, &mut out);
    }
    out
}

fn walk_erlang_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_erlang_dir_bounded(dir, root, dep, out, 0);
}

fn walk_erlang_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue; };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "examples" | "doc") || name.starts_with('.') {
                    continue;
                }
            }
            walk_erlang_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !(name.ends_with(".erl") || name.ends_with(".hrl")) { continue; }
            if name.ends_with("_SUITE.erl") || name.ends_with("_tests.erl") { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:erlang:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "erlang",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erlang_parses_rebar_deps() {
        let content = r#"{deps, [
{cowlib,".*",{git,"https://github.com/ninenines/cowlib",{tag,"2.16.0"}}},{ranch,".*",{git,"https://github.com/ninenines/ranch",{tag,"1.8.1"}}}
]}."#;
        let deps = parse_rebar_deps(content);
        assert_eq!(deps, vec!["cowlib", "ranch"]);
    }

    #[test]
    fn erlang_discovers_rebar_deps() {
        let tmp = std::env::temp_dir().join("bw-test-erlang-discover");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("rebar.config"), r#"{deps, [{cowlib,".*",{git,"url",{tag,"1.0"}}},{ranch,".*",{git,"url",{tag,"1.0"}}}]}."#).unwrap();
        let deps_dir = tmp.join("_build").join("default").join("lib");
        let cowlib = deps_dir.join("cowlib").join("src");
        std::fs::create_dir_all(&cowlib).unwrap();
        std::fs::write(cowlib.join("cowlib.erl"), "-module(cowlib).\n").unwrap();

        let roots = discover_erlang_externals(&tmp);
        assert_eq!(roots.len(), 1); // only cowlib exists on disk
        assert_eq!(roots[0].module_path, "cowlib");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
