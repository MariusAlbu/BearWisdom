// Ruby / bundler externals — Phase 1.2 (Gemfile.lock-aware)

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Bundler / RubyGems cache → `discover_ruby_externals` + `walk_ruby_external_root`.
///
/// Ruby gems are distributed as source tarballs and extracted into one of
/// several locations searched in priority order:
///
///   1. Per-project vendored install: `./vendor/bundle/ruby/<ruby-ver>/gems/`.
///      Created by `bundle install --path vendor/bundle`.
///   2. `$BEARWISDOM_RUBY_GEM_HOME/gems/` — explicit env override (test support).
///   3. User gem home via `gem env gemdir`:
///        * `~/.local/share/gem/ruby/<ver>/gems/` (XDG layout — RubyInstaller 3.x+)
///        * `~/.gem/ruby/<ver>/gems/`              (classic layout)
///        * `~/gems/gems/`                          (legacy Windows RubyInstaller)
///   4. System gem home: `$GEM_HOME/gems/`.
///
/// Gem list is sourced from `Gemfile.lock` (GEM/specs: and GIT/specs: sections)
/// when present — this gives authoritative resolved names + versions. Falls back
/// to parsing `Gemfile` declarations when no lockfile exists.
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

/// A resolved gem entry — name + optional exact version from Gemfile.lock.
#[derive(Debug, Clone)]
struct GemEntry {
    name: String,
    /// Exact version from Gemfile.lock specs, e.g. "8.1.3". None when
    /// sourced from Gemfile declarations (no lockfile present).
    version: Option<String>,
}

/// Parse `Gemfile.lock` for the authoritative resolved gem list.
///
/// Handles three spec sections in Bundler lockfile format:
/// - `GEM` / `GIT` / `PATH` sections each contain an indented `specs:` block.
/// - Each gem entry is `    <name> (<version>)` (4-space indent).
///
/// Returns a vec of (name, version) pairs for every gem in all specs sections.
pub fn parse_gemfile_lock(content: &str) -> Vec<GemEntry> {
    let mut entries = Vec::new();
    let mut in_specs = false;

    for line in content.lines() {
        // Section headers reset context
        if line.trim().is_empty() {
            continue;
        }

        // Detect start of a specs block (inside GEM / GIT / PATH sections)
        if line == "  specs:" {
            in_specs = true;
            continue;
        }

        // Any non-indented or single-space line that isn't a sub-dep resets specs context
        // (e.g. PLATFORMS, DEPENDENCIES, BUNDLED WITH)
        if !line.starts_with("  ") {
            in_specs = false;
            continue;
        }

        if !in_specs {
            continue;
        }

        // Spec entries are indented exactly 4 spaces: `    <name> (<version>)`
        // Sub-dependency lines are indented 6+ spaces — skip them.
        if !line.starts_with("    ") || line.starts_with("      ") {
            continue;
        }

        let trimmed = line.trim();
        // Parse: `name (version)` — version may include platform suffix like
        // `ffi (1.17.4-x64-mingw-ucrt)`. We capture the full version string.
        if let Some(paren) = trimmed.find(" (") {
            let name = trimmed[..paren].trim().to_string();
            let rest = &trimmed[paren + 2..];
            let version = rest.trim_end_matches(')').trim().to_string();
            if !name.is_empty() && !version.is_empty() {
                entries.push(GemEntry { name, version: Some(version) });
            }
        }
    }
    entries
}

/// Discover external Ruby gem roots for a project.
///
/// Source priority:
///   1. `Gemfile.lock` — authoritative resolved list with exact versions.
///      When present, we use exact-version directory lookup first, then fall
///      back to prefix-match (handles minor platform variant suffixes).
///   2. `Gemfile` declarations — unversioned, prefix-match only.
///
/// For each gem, searches candidate gem roots in order and returns the first
/// found directory. Missing cache = graceful empty vec, never an error.
pub fn discover_ruby_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::gemfile::parse_gemfile_gems;

    // Prefer Gemfile.lock for authoritative resolved list.
    let lock_path = project_root.join("Gemfile.lock");
    let gems: Vec<GemEntry> = if lock_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&lock_path) {
            let parsed = parse_gemfile_lock(&content);
            if !parsed.is_empty() {
                parsed
            } else {
                // Lock file present but empty/unparseable — try Gemfile.
                let gemfile_path = project_root.join("Gemfile");
                if let Ok(gf) = std::fs::read_to_string(&gemfile_path) {
                    parse_gemfile_gems(&gf)
                        .into_iter()
                        .map(|name| GemEntry { name, version: None })
                        .collect()
                } else {
                    return Vec::new();
                }
            }
        } else {
            return Vec::new();
        }
    } else {
        // No lockfile — parse Gemfile declarations.
        let gemfile_path = project_root.join("Gemfile");
        if !gemfile_path.is_file() {
            return Vec::new();
        }
        let Ok(gf) = std::fs::read_to_string(&gemfile_path) else {
            return Vec::new();
        };
        parse_gemfile_gems(&gf)
            .into_iter()
            .map(|name| GemEntry { name, version: None })
            .collect()
    };

    if gems.is_empty() {
        return Vec::new();
    }

    let candidate_roots = ruby_candidate_gem_roots(project_root);
    if candidate_roots.is_empty() {
        debug!("No bundler gem install locations found for {}", project_root.display());
        return Vec::new();
    }

    let mut result = Vec::with_capacity(gems.len());
    let mut seen = std::collections::HashSet::new();
    for entry in &gems {
        // Deduplicate: Gemfile.lock lists sub-dependencies too — same gem name
        // may appear multiple times with different specs parents. Take the first.
        if !seen.insert(entry.name.clone()) {
            continue;
        }
        if let Some(gem_root) = find_gem_dir_entry(&candidate_roots, entry) {
            let version = gem_root
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix(&format!("{}-", entry.name)))
                .unwrap_or("")
                .to_string();
            result.push(ExternalDepRoot {
                module_path: entry.name.clone(),
                version,
                root: gem_root,
                ecosystem: "ruby",
                package_id: None,
            });
        }
    }
    result
}

/// Build the ordered list of directories that might contain bundler-installed
/// gems. Each returned path points at a `gems/` subdir — gem installs live
/// inside as `<name>-<version>/`.
///
/// Search order:
///   1. `BEARWISDOM_RUBY_GEM_HOME` env override (tests and CI).
///   2. Per-project vendored install: `vendor/bundle/ruby/<ruby-ver>/gems/`.
///   3. XDG user gem home: `~/.local/share/gem/ruby/<ver>/gems/`
///      (RubyInstaller 3.x+ on Windows and modern Linux/macOS).
///   4. Classic user gem home: `~/.gem/ruby/<ver>/gems/`.
///   5. Legacy Windows RubyInstaller default: `~/gems/gems/`.
///   6. `$GEM_HOME/gems/`.
fn ruby_candidate_gem_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 0. Explicit override for tests and CI environments.
    if let Ok(override_val) = std::env::var("BEARWISDOM_RUBY_GEM_HOME") {
        for seg in std::env::split_paths(&override_val) {
            let gems = seg.join("gems");
            if gems.is_dir() {
                candidates.push(gems);
            } else if seg.is_dir() {
                // Caller may have pointed directly at a gems/ dir.
                candidates.push(seg);
            }
        }
    }

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

    if let Some(home) = dirs::home_dir() {
        // 2. XDG layout (RubyInstaller 3.x+ on Windows, modern Linux/macOS):
        //    ~/.local/share/gem/ruby/<ver>/gems/
        let xdg_gem = home.join(".local").join("share").join("gem").join("ruby");
        if xdg_gem.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&xdg_gem) {
                for entry in entries.flatten() {
                    let gems = entry.path().join("gems");
                    if gems.is_dir() {
                        candidates.push(gems);
                    }
                }
            }
        }

        // 3. Classic user gem home: ~/.gem/ruby/<ver>/gems/
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

        // 4. Legacy Windows RubyInstaller default: ~/gems/gems/
        let win_default = home.join("gems").join("gems");
        if win_default.is_dir() {
            candidates.push(win_default);
        }
    }

    // 5. $GEM_HOME/gems/
    if let Ok(gem_home) = std::env::var("GEM_HOME") {
        if !gem_home.is_empty() {
            let gems = PathBuf::from(gem_home).join("gems");
            if gems.is_dir() {
                candidates.push(gems);
            }
        }
    }

    candidates
}

/// Search every candidate gems root for the best directory for the given entry.
///
/// When `entry.version` is `Some`, first tries an exact match
/// `<name>-<version>`, then falls back to prefix scan (handles platform-
/// variant suffixes like `ffi-1.17.4-x64-mingw-ucrt`). When no exact version
/// is known, falls back to picking the lexicographically largest match.
fn find_gem_dir_entry(candidates: &[PathBuf], entry: &GemEntry) -> Option<PathBuf> {
    let prefix = format!("{}-", entry.name);

    for root in candidates {
        // Exact-version fast path (Gemfile.lock case).
        if let Some(ref ver) = entry.version {
            let exact = root.join(format!("{}-{}", entry.name, ver));
            if exact.is_dir() {
                return Some(exact);
            }
            // Platform-variant: name-version-platform (e.g. ffi-1.17.4-x64-mingw-ucrt)
            // Try as a prefix of the exact versioned name.
            let ver_prefix = format!("{}-{}-", entry.name, ver);
            if let Ok(dir_entries) = std::fs::read_dir(root) {
                let mut platform_matches: Vec<PathBuf> = dir_entries
                    .flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let name = p.file_name()?.to_str()?;
                        if name.starts_with(&ver_prefix) && p.is_dir() {
                            Some(p)
                        } else {
                            None
                        }
                    })
                    .collect();
                if !platform_matches.is_empty() {
                    platform_matches.sort();
                    return platform_matches.pop();
                }
            }
        }

        // Prefix-scan fallback: pick highest-version directory.
        // Require the character immediately after `<name>-` to be a digit so
        // that `rails-dom-testing-2.3.0` does not match the gem `rails`.
        let Ok(dir_entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut matches: Vec<PathBuf> = dir_entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let name = p.file_name()?.to_str()?;
                if name.starts_with(&prefix) && p.is_dir() {
                    // The character after the `<gemname>-` separator must be
                    // a digit to be a version number, not another name segment.
                    let after = &name[prefix.len()..];
                    if after.starts_with(|c: char| c.is_ascii_digit()) {
                        Some(p)
                    } else {
                        None
                    }
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

/// Search every candidate gems root for a directory named `<gem_name>-*`.
/// When multiple versions are installed, the highest-version directory wins
/// (lexical sort — good enough for semver-style version strings).
///
/// This variant is kept for callers that don't have a `GemEntry` (tests, etc.).
#[allow(dead_code)]
fn find_gem_dir(candidates: &[PathBuf], gem_name: &str) -> Option<PathBuf> {
    let entry = GemEntry { name: gem_name.to_string(), version: None };
    find_gem_dir_entry(candidates, &entry)
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

    // -------------------------------------------------------------------------
    // Gemfile.lock parser tests
    // -------------------------------------------------------------------------

    #[test]
    fn parse_gemfile_lock_extracts_gem_specs() {
        let lock = r#"GEM
  remote: https://rubygems.org/
  specs:
    actionpack (8.1.3)
      activesupport (= 8.1.3)
    activesupport (8.1.3)
      concurrent-ruby (~> 1.0, >= 1.3.1)
    minitest (5.27.0)

PLATFORMS
  x86_64-linux

DEPENDENCIES
  rails (~> 8.1)
"#;
        let entries = parse_gemfile_lock(lock);
        assert_eq!(entries.len(), 3, "should parse top-level spec entries only");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"actionpack"), "missing actionpack");
        assert!(names.contains(&"activesupport"), "missing activesupport");
        assert!(names.contains(&"minitest"), "missing minitest");

        let ap = entries.iter().find(|e| e.name == "actionpack").unwrap();
        assert_eq!(ap.version.as_deref(), Some("8.1.3"));
        let mt = entries.iter().find(|e| e.name == "minitest").unwrap();
        assert_eq!(mt.version.as_deref(), Some("5.27.0"));
    }

    #[test]
    fn parse_gemfile_lock_handles_platform_variants() {
        let lock = r#"GEM
  remote: https://rubygems.org/
  specs:
    ffi (1.17.4-x64-mingw-ucrt)
    nokogiri (1.18.8-x86_64-linux)

PLATFORMS
  x86_64-linux
"#;
        let entries = parse_gemfile_lock(lock);
        let ffi = entries.iter().find(|e| e.name == "ffi").unwrap();
        // Full version string including platform suffix
        assert_eq!(ffi.version.as_deref(), Some("1.17.4-x64-mingw-ucrt"));
    }

    #[test]
    fn parse_gemfile_lock_handles_git_and_path_sections() {
        let lock = r#"GIT
  remote: https://github.com/example/gem.git
  revision: abc123
  specs:
    example_gem (0.10.0)
      some_dep (>= 1.0)

PATH
  remote: vendor/gems/dotenv-3.2.0
  specs:
    dotenv (3.2.0)

GEM
  remote: https://rubygems.org/
  specs:
    rails (8.1.3)
"#;
        let entries = parse_gemfile_lock(lock);
        assert_eq!(entries.len(), 3);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"example_gem"));
        assert!(names.contains(&"dotenv"));
        assert!(names.contains(&"rails"));
    }

    #[test]
    fn gemfile_lock_used_over_gemfile_when_present() {
        let tmp = std::env::temp_dir().join("bw-test-ruby-lockfile-preferred");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Gemfile declares "rails"; Gemfile.lock resolves to "minitest" + "devise".
        std::fs::write(tmp.join("Gemfile"), "source 'https://rubygems.org'\ngem 'rails'\n").unwrap();
        let lock = r#"GEM
  remote: https://rubygems.org/
  specs:
    minitest (5.27.0)
    devise (4.9.3)

PLATFORMS
  ruby
"#;
        std::fs::write(tmp.join("Gemfile.lock"), lock).unwrap();

        // Set up a fake gem cache via BEARWISDOM_RUBY_GEM_HOME.
        let gems_root = tmp.join("fake_gems");
        std::fs::create_dir_all(gems_root.join("gems").join("minitest-5.27.0").join("lib")).unwrap();
        std::fs::write(
            gems_root.join("gems").join("minitest-5.27.0").join("lib").join("minitest.rb"),
            "module Minitest; end\n",
        ).unwrap();
        std::fs::create_dir_all(gems_root.join("gems").join("devise-4.9.3").join("lib")).unwrap();
        std::fs::write(
            gems_root.join("gems").join("devise-4.9.3").join("lib").join("devise.rb"),
            "module Devise; end\n",
        ).unwrap();

        // Point the locator at our fake gem home.
        std::env::set_var("BEARWISDOM_RUBY_GEM_HOME", gems_root.join("gems").to_str().unwrap());
        let roots = discover_ruby_externals(&tmp);
        std::env::remove_var("BEARWISDOM_RUBY_GEM_HOME");

        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        // Should use lockfile gems (minitest, devise) — NOT Gemfile's "rails".
        assert!(names.contains("minitest"), "expected minitest from lockfile");
        assert!(names.contains("devise"), "expected devise from lockfile");
        assert!(!names.contains("rails"), "rails is in Gemfile but not lockfile");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn gemfile_lock_exact_version_match() {
        let tmp = std::env::temp_dir().join("bw-test-ruby-exact-version");
        let _ = std::fs::remove_dir_all(&tmp);

        // Two versions of the same gem installed; lockfile pins to 5.1.0.
        let gems_root = tmp.join("gems");
        for ver in &["5.0.0", "5.1.0", "5.2.0"] {
            std::fs::create_dir_all(gems_root.join(format!("minitest-{ver}")).join("lib")).unwrap();
            std::fs::write(
                gems_root.join(format!("minitest-{ver}")).join("lib").join("minitest.rb"),
                format!("module Minitest; VERSION='{}'; end\n", ver),
            ).unwrap();
        }

        // Project declares minitest 5.1.0 via lockfile.
        std::fs::create_dir_all(&tmp.join("project")).unwrap();
        std::fs::write(tmp.join("project").join("Gemfile"), "gem 'minitest'\n").unwrap();
        let lock = r#"GEM
  remote: https://rubygems.org/
  specs:
    minitest (5.1.0)

PLATFORMS
  ruby
"#;
        std::fs::write(tmp.join("project").join("Gemfile.lock"), lock).unwrap();

        std::env::set_var("BEARWISDOM_RUBY_GEM_HOME", gems_root.to_str().unwrap());
        let roots = discover_ruby_externals(&tmp.join("project"));
        std::env::remove_var("BEARWISDOM_RUBY_GEM_HOME");

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].version, "5.1.0", "should pick exact locked version");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
