// Ruby / bundler externals — Phase 1.1

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Bundler / RubyGems cache → `discover_ruby_externals` + `walk_ruby_external_root`.
///
/// Ruby gems are distributed as source tarballs and extracted into one of
/// three locations:
///
///   1. Per-project vendored install: `./vendor/bundle/ruby/<ruby-ver>/gems/`.
///      Created by `bundle install --path vendor/bundle` or
///      `bundle config set --local path vendor/bundle`. Preferred location
///      when present — versioned with the project, reproducible.
///   2. User gem home: `~/.gem/ruby/<ruby-ver>/gems/`, or
///      `~/gems/gems/`, or whatever `gem env gemdir` reports. The default
///      when bundler isn't told to vendor.
///   3. System gem home: `$GEM_HOME/gems/`, `/usr/lib/ruby/gems/...`, etc.
///      Typical on Linux system Ruby installs.
///
/// For each declared gem in the Gemfile, we look in each candidate location
/// for a directory whose name begins with `<gem>-` and return the first hit.
/// That's close enough to correct for unversioned resolution; Gemfile.lock
/// version-matching is a later enhancement that requires a lockfile parser.
///
/// `walk_root` filters to `lib/**/*.rb` — bundler-installed gems conventionally
/// expose their public API in `lib/`, with `test/`, `spec/`, `bin/`, `ext/`,
/// `vendor/`, and `examples/` all skippable.
pub struct RubyExternalsLocator;

impl ExternalSourceLocator for RubyExternalsLocator {
    fn ecosystem(&self) -> &'static str { "ruby" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ruby_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ruby_external_root(dep)
    }
}

/// Discover external Ruby gem roots declared in a project's Gemfile.
///
/// Strategy:
///   1. Parse the project's Gemfile via `gemfile.rs` to get declared names.
///   2. For each name, search the candidate bundler install paths in order:
///        * `./vendor/bundle/ruby/<ver>/gems/<name>-*`         (vendored)
///        * `~/.gem/ruby/<ver>/gems/<name>-*`                   (user install)
///        * `$GEM_HOME/gems/<name>-*`                           (env override)
///        * `~/gems/gems/<name>-*`                              (Windows default)
///      The first existing directory wins. Ruby version segments and
///      version-suffixed gem dirs are matched by prefix, so the locator
///      doesn't need to know the exact installed version.
///   3. Return one `ExternalDepRoot` per resolved gem.
///
/// Missing bundler install = empty vec (not an error). The locator
/// degrades gracefully when tooling isn't available.
pub fn discover_ruby_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::gemfile::parse_gemfile_gems;

    let gemfile_path = project_root.join("Gemfile");
    if !gemfile_path.is_file() {
        return Vec::new();
    }
    let Ok(gemfile_content) = std::fs::read_to_string(&gemfile_path) else {
        return Vec::new();
    };
    let declared: Vec<String> = parse_gemfile_gems(&gemfile_content);
    if declared.is_empty() {
        return Vec::new();
    }

    let candidate_roots = ruby_candidate_gem_roots(project_root);
    if candidate_roots.is_empty() {
        debug!("No bundler gem install locations found for {}", project_root.display());
        return Vec::new();
    }

    let mut result = Vec::with_capacity(declared.len());
    let mut seen = std::collections::HashSet::new();
    for gem_name in &declared {
        if !seen.insert(gem_name.clone()) {
            continue;
        }
        if let Some(gem_root) = find_gem_dir(&candidate_roots, gem_name) {
            result.push(ExternalDepRoot {
                module_path: gem_name.clone(),
                version: gem_root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_prefix(&format!("{gem_name}-")))
                    .unwrap_or("")
                    .to_string(),
                root: gem_root,
                ecosystem: "ruby",
            });
        }
    }
    result
}

/// Build the ordered list of directories that might contain bundler-installed
/// gems. Each returned path points at a `gems/` subdir — gem installs live
/// inside as `<name>-<version>/`.
fn ruby_candidate_gem_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 1. Per-project vendored install: vendor/bundle/ruby/<ruby-ver>/gems/
    //    `<ruby-ver>` is e.g. `3.2.0` — we don't know which, so walk once.
    let vendor = project_root.join("vendor").join("bundle").join("ruby");
    if vendor.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&vendor) {
            for entry in entries.flatten() {
                let gems = entry.path().join("gems");
                if gems.is_dir() {
                    candidates.push(gems);
                }
            }
        }
    }

    // 2. Home-directory gem install: ~/.gem/ruby/<ver>/gems/
    if let Some(home) = dirs::home_dir() {
        let gem_dir = home.join(".gem").join("ruby");
        if gem_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&gem_dir) {
                for entry in entries.flatten() {
                    let gems = entry.path().join("gems");
                    if gems.is_dir() {
                        candidates.push(gems);
                    }
                }
            }
        }
        // Windows RubyInstaller default: ~/gems/gems/
        let win_default = home.join("gems").join("gems");
        if win_default.is_dir() {
            candidates.push(win_default);
        }
    }

    // 3. $GEM_HOME/gems/
    if let Ok(gem_home) = std::env::var("GEM_HOME") {
        let gems = PathBuf::from(gem_home).join("gems");
        if gems.is_dir() {
            candidates.push(gems);
        }
    }

    candidates
}

/// Search every candidate gems root for a directory named `<gem_name>-*`.
/// When multiple versions are installed, the highest-version directory wins
/// (lexical sort — good enough for semver-style version strings).
fn find_gem_dir(candidates: &[PathBuf], gem_name: &str) -> Option<PathBuf> {
    let prefix = format!("{gem_name}-");
    for root in candidates {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut matches: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let name = p.file_name()?.to_str()?;
                if name.starts_with(&prefix) && p.is_dir() {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        if !matches.is_empty() {
            matches.sort();
            return matches.pop();
        }
    }
    None
}

/// Walk a discovered gem root and emit `WalkedFile` entries for every `.rb`
/// source file under `lib/`. Skips `test/`, `spec/`, `bin/`, `ext/`,
/// `vendor/`, `examples/`, and hidden directories. Virtual paths take the
/// form `ext:ruby:<gem_name>/<relative>` to mirror the TS convention.
pub fn walk_ruby_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let lib_dir = dep.root.join("lib");
    if !lib_dir.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk_ruby_dir(&lib_dir, &dep.root, dep, &mut out);
    out
}

fn walk_ruby_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_ruby_dir_bounded(dir, root, dep, out, 0);
}

fn walk_ruby_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
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
                    "test" | "tests" | "spec" | "specs" | "bin" | "ext" | "vendor" | "examples" | "docs"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_ruby_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".rb") {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:ruby:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "ruby",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ruby_fixture(tmp: &Path, gems: &[(&str, &str)]) {
        // Project root:
        //   Gemfile listing each gem
        //   vendor/bundle/ruby/3.2.0/gems/<gem>-<ver>/lib/<gem>.rb
        std::fs::create_dir_all(tmp).unwrap();
        let mut gemfile = String::from("source 'https://rubygems.org'\n");
        for (name, _) in gems {
            gemfile.push_str(&format!("gem '{name}'\n"));
        }
        std::fs::write(tmp.join("Gemfile"), gemfile).unwrap();

        let gems_root = tmp
            .join("vendor")
            .join("bundle")
            .join("ruby")
            .join("3.2.0")
            .join("gems");
        std::fs::create_dir_all(&gems_root).unwrap();
        for (name, version) in gems {
            let gem_root = gems_root.join(format!("{name}-{version}"));
            let lib = gem_root.join("lib");
            std::fs::create_dir_all(&lib).unwrap();
            std::fs::write(
                lib.join(format!("{name}.rb")),
                format!("module {} ; VERSION = '{}' ; end\n", capitalize(name), version),
            )
            .unwrap();
            // Skippable sibling directories that walk_ruby_dir must exclude.
            std::fs::create_dir_all(gem_root.join("test")).unwrap();
            std::fs::write(gem_root.join("test").join("should_skip.rb"), "# test\n").unwrap();
            std::fs::create_dir_all(gem_root.join("spec")).unwrap();
            std::fs::write(gem_root.join("spec").join("should_skip.rb"), "# spec\n").unwrap();
        }
    }

    fn capitalize(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }

    #[test]
    fn ruby_locator_finds_vendored_bundle_gems() {
        let tmp = std::env::temp_dir().join("bw-test-ruby-locator-find");
        let _ = std::fs::remove_dir_all(&tmp);
        make_ruby_fixture(&tmp, &[("devise", "4.9.3"), ("sidekiq", "7.1.0")]);

        let roots = discover_ruby_externals(&tmp);
        assert_eq!(roots.len(), 2, "expected one root per declared gem");
        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        assert!(names.contains("devise"));
        assert!(names.contains("sidekiq"));

        // Version string correctly stripped from the gem dir name.
        let devise = roots.iter().find(|r| r.module_path == "devise").unwrap();
        assert_eq!(devise.version, "4.9.3");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ruby_walk_excludes_test_and_spec_dirs() {
        let tmp = std::env::temp_dir().join("bw-test-ruby-locator-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        make_ruby_fixture(&tmp, &[("devise", "4.9.3")]);

        let roots = discover_ruby_externals(&tmp);
        assert_eq!(roots.len(), 1);
        let walked = walk_ruby_external_root(&roots[0]);

        // Exactly one file expected: lib/devise.rb. The test/ and spec/
        // fixtures under the gem root must be skipped by walk_ruby_dir.
        assert_eq!(walked.len(), 1, "walk_root should find only lib/devise.rb");
        let file = &walked[0];
        assert!(
            file.relative_path.starts_with("ext:ruby:devise/"),
            "virtual path should carry ext:ruby: prefix and gem name: got {}",
            file.relative_path
        );
        assert!(file.relative_path.ends_with("lib/devise.rb"));
        assert_eq!(file.language, "ruby");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ruby_locator_returns_empty_without_gemfile() {
        let tmp = std::env::temp_dir().join("bw-test-ruby-locator-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // No Gemfile, no vendor — should return empty, not error.
        let roots = discover_ruby_externals(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ruby_locator_returns_empty_when_gems_not_installed() {
        let tmp = std::env::temp_dir().join("bw-test-ruby-locator-no-cache");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // Gemfile declares gems but no install location has them.
        std::fs::write(
            tmp.join("Gemfile"),
            "source 'https://rubygems.org'\ngem 'rails'\n",
        )
        .unwrap();
        let roots = discover_ruby_externals(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
