// Elixir / mix externals — Phase 1.2

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::Path;
use tracing::debug;

/// Mix project deps/ directory → `discover_elixir_externals` + `walk_elixir_external_root`.
///
/// Elixir's package manager `mix` is unusual — dependencies are fetched into
/// `<project>/deps/<package>/` rather than a global user cache. That makes
/// the locator shape simple: no path search, no version resolution. Every
/// entry in `deps/` is a package, and each entry has its source under
/// `deps/<package>/lib/`. Retiring the hardcoded Phoenix / Ecto / Plug /
/// ExUnit / Mox / ExMachina / Absinthe / Oban / Gettext blocks in
/// `elixir/externals.rs` depends on this locator running end-to-end with
/// `mix deps.get` already executed on the project.
pub struct ElixirExternalsLocator;

impl ExternalSourceLocator for ElixirExternalsLocator {
    fn ecosystem(&self) -> &'static str { "elixir" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_elixir_externals(project_root)
    }

    /// M3 override: in a workspace (e.g. an Elixir project that also has
    /// `assets/package.json` or `magefiles/`), the package list will contain
    /// the sub-packages but NOT the root Elixir project itself. The default
    /// implementation delegates to `locate_roots(package_abs_path)`, which
    /// only finds Elixir deps when `package_abs_path` == the mix project root.
    ///
    /// This override checks `package_abs_path` first, then falls back to
    /// `workspace_root`. That covers:
    ///   - Umbrella apps: each `apps/<child>` has its own mix.exs with deps.
    ///   - Root-level mix.exs alongside JS sub-packages: the root Elixir
    ///     project is discovered via `workspace_root` even when the per-package
    ///     loop is iterating over the JS and Go sub-packages.
    fn locate_roots_for_package(
        &self,
        workspace_root: &Path,
        package_abs_path: &Path,
        package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        // Try the specific package path first (covers umbrella app children).
        let mut roots = discover_elixir_externals(package_abs_path);

        // Fall back to workspace root when the package is a non-Elixir
        // sub-directory (e.g. `assets/`, `magefiles/`) inside an Elixir
        // project whose mix.exs lives at the workspace root.
        if roots.is_empty() && package_abs_path != workspace_root {
            roots = discover_elixir_externals(workspace_root);
        }

        for r in &mut roots {
            r.package_id = Some(package_id);
        }
        roots
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_elixir_external_root(dep)
    }
}

/// Discover external Elixir package roots for a project.
///
/// Strategy:
///   1. Require `mix.exs` at the project root — otherwise empty.
///   2. Walk `<project>/deps/`. Every direct-child directory placed there by
///      `mix deps.get` is a valid package (direct or transitive). All dirs
///      are included — `deps/` is exclusively managed by mix so stray dirs
///      are not a concern in practice.
///   3. For each package dir that contains a `lib/` subdirectory, emit an
///      `ExternalDepRoot`. `walk_elixir_external_root` restricts the walk
///      to `lib/**/*.ex` + `lib/**/*.exs`.
///   4. Read version from the package's mix.exs `@version` attribute for
///      logging. Not load-bearing.
///
/// Unlike Go/Java/Ruby, mix uses a project-local `deps/` directory rather
/// than a global user cache. Transitive deps are included alongside direct
/// deps — all appear as siblings under `deps/`.
pub fn discover_elixir_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mix_exs = project_root.join("mix.exs");
    if !mix_exs.is_file() {
        return Vec::new();
    }

    let deps_dir = project_root.join("deps");
    if !deps_dir.is_dir() {
        debug!(
            "No deps/ directory found for Elixir project at {} — run `mix deps.get`",
            project_root.display()
        );
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(&deps_dir) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Only include packages that have a lib/ directory — pure Erlang
        // deps without .ex source (e.g. gproc, hpack_erl, parse_trans)
        // would contribute no files and can be skipped cheaply.
        if !path.join("lib").is_dir() {
            continue;
        }
        // Version read from the package's mix.exs @version attribute when
        // available, otherwise blank. Not load-bearing; used only for logs.
        let version = read_mix_package_version(&path).unwrap_or_default();
        result.push(ExternalDepRoot {
            module_path: name.to_string(),
            version,
            root: path,
            ecosystem: "elixir",
            package_id: None,
        });
    }
    result
}

/// Best-effort read of `@version` from a package's mix.exs. Returns None
/// when the file is absent or the attribute isn't declared on a simple line.
fn read_mix_package_version(pkg_root: &Path) -> Option<String> {
    let mix_exs = pkg_root.join("mix.exs");
    let content = std::fs::read_to_string(&mix_exs).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("@version ") {
            let rest = rest.trim().trim_start_matches('=').trim();
            let ver = rest.trim_matches('"').trim_matches('\'');
            if !ver.is_empty() {
                return Some(ver.to_string());
            }
        }
    }
    None
}

/// Walk an Elixir package root and emit `WalkedFile` entries for every
/// `.ex` / `.exs` source file under `lib/`. Skips `test/`, `priv/`, `bin/`,
/// `config/`, `doc/`, `docs/`, `assets/`, and hidden directories. Virtual
/// paths use the `ext:elixir:<pkg>/<relative>` form.
pub fn walk_elixir_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let lib_dir = dep.root.join("lib");
    if !lib_dir.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk_elixir_dir(&lib_dir, &dep.root, dep, &mut out);
    out
}

fn walk_elixir_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_elixir_dir_bounded(dir, root, dep, out, 0);
}

fn walk_elixir_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "test" | "tests" | "priv" | "bin" | "config" | "doc" | "docs"
                        | "assets" | "examples" | "_build" | "cover"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_elixir_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !(name.ends_with(".ex") || name.ends_with(".exs")) {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:elixir:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "elixir",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capitalize(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }

    fn make_elixir_fixture(tmp: &Path, deps: &[&str]) {
        std::fs::create_dir_all(tmp).unwrap();
        // Minimal mix.exs declaring each dep.
        let mut mix = String::from("defmodule MyApp.MixProject do\n  use Mix.Project\n  defp deps do\n    [\n");
        for name in deps {
            mix.push_str(&format!("      {{:{name}, \"~> 1.0\"}},\n"));
        }
        mix.push_str("    ]\n  end\nend\n");
        std::fs::write(tmp.join("mix.exs"), mix).unwrap();

        // deps/<package>/lib/<package>.ex for each dep.
        for name in deps {
            let pkg = tmp.join("deps").join(name);
            let lib = pkg.join("lib");
            std::fs::create_dir_all(&lib).unwrap();
            std::fs::write(
                lib.join(format!("{name}.ex")),
                format!("defmodule {} do\n  def hello, do: :world\nend\n", capitalize(name)),
            )
            .unwrap();
            // Package's own mix.exs with @version — exercises read_mix_package_version.
            std::fs::write(
                pkg.join("mix.exs"),
                format!(
                    "defmodule {}.MixProject do\n  @version \"1.2.3\"\nend\n",
                    capitalize(name)
                ),
            )
            .unwrap();
            // Skippable test/ and priv/ siblings.
            std::fs::create_dir_all(pkg.join("test")).unwrap();
            std::fs::write(pkg.join("test").join("should_skip.exs"), "# test\n").unwrap();
            std::fs::create_dir_all(pkg.join("priv")).unwrap();
            std::fs::write(pkg.join("priv").join("seeds.exs"), "# priv\n").unwrap();
        }
    }

    #[test]
    fn elixir_locator_finds_deps_directories() {
        let tmp = std::env::temp_dir().join("bw-test-elixir-locator-find");
        let _ = std::fs::remove_dir_all(&tmp);
        make_elixir_fixture(&tmp, &["phoenix", "ecto", "plug"]);

        let roots = discover_elixir_externals(&tmp);
        assert_eq!(roots.len(), 3);
        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        assert!(names.contains("phoenix"));
        assert!(names.contains("ecto"));
        assert!(names.contains("plug"));

        // Version read from package mix.exs.
        assert!(roots.iter().all(|r| r.version == "1.2.3"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn elixir_walk_excludes_test_priv_and_config() {
        let tmp = std::env::temp_dir().join("bw-test-elixir-locator-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        make_elixir_fixture(&tmp, &["phoenix"]);

        let roots = discover_elixir_externals(&tmp);
        assert_eq!(roots.len(), 1);
        let walked = walk_elixir_external_root(&roots[0]);

        // Exactly one file: lib/phoenix.ex. The test/ and priv/ fixtures
        // under the package root must be excluded by walk_elixir_dir.
        assert_eq!(walked.len(), 1);
        let file = &walked[0];
        assert!(file.relative_path.starts_with("ext:elixir:phoenix/"));
        assert!(file.relative_path.ends_with("lib/phoenix.ex"));
        assert_eq!(file.language, "elixir");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn elixir_locator_returns_empty_without_mix_exs() {
        let tmp = std::env::temp_dir().join("bw-test-elixir-locator-no-manifest");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let roots = discover_elixir_externals(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn elixir_locator_returns_empty_when_deps_not_fetched() {
        let tmp = std::env::temp_dir().join("bw-test-elixir-locator-no-deps");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // mix.exs exists but no deps/ directory — simulates a fresh clone
        // that hasn't run `mix deps.get` yet.
        std::fs::write(
            tmp.join("mix.exs"),
            "defmodule MyApp.MixProject do\n  defp deps do\n    [{:phoenix, \"~> 1.7\"}]\n  end\nend\n",
        )
        .unwrap();
        let roots = discover_elixir_externals(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn elixir_locator_includes_transitive_deps_dirs() {
        let tmp = std::env::temp_dir().join("bw-test-elixir-locator-transitive");
        let _ = std::fs::remove_dir_all(&tmp);
        make_elixir_fixture(&tmp, &["phoenix"]);
        // Plant a transitive dep (cowboy) that isn't declared in mix.exs
        // but was fetched by mix. It should be discovered because it has a
        // lib/ directory — all dirs in deps/ are placed there by mix.
        let transitive = tmp.join("deps").join("cowboy");
        std::fs::create_dir_all(transitive.join("lib")).unwrap();
        std::fs::write(
            transitive.join("lib").join("cowboy.ex"),
            "defmodule Cowboy do\n  def start, do: :ok\nend\n",
        )
        .unwrap();
        // A dir WITHOUT lib/ (e.g. an Erlang-only dep like parse_trans that
        // ships no .ex source) should NOT appear.
        let no_lib = tmp.join("deps").join("parse_trans");
        std::fs::create_dir_all(&no_lib).unwrap();

        let roots = discover_elixir_externals(&tmp);
        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        // Both declared (phoenix) and transitive (cowboy) are included.
        assert!(names.contains("phoenix"), "missing phoenix");
        assert!(names.contains("cowboy"), "missing transitive dep cowboy");
        // parse_trans (no lib/) must be excluded.
        assert!(!names.contains("parse_trans"), "parse_trans (no lib/) should be excluded");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
