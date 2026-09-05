// =============================================================================
// indexer/ext_virtual_path — ecosystem-shaped virtual paths for pulled files
//
// Maps a demand-pulled external file's absolute path onto the same `ext:`
// virtual path its ecosystem's eager walker would emit, so `already_walked`
// dedup, per-locator `post_process_parsed` hooks, and `ExtMatch::PkgSegment`
// all treat pulled and walker-emitted files identically. One arm per
// ecosystem path layout; `None` means the path has no recognizable layout
// and the caller falls back to `ext:idx:`.
// =============================================================================

use std::path::Path;

mod pascal;

/// Derive the ecosystem-shaped virtual path for a file pulled through the
/// demand-driven path, so per-locator `post_process_parsed` hooks (notably
/// npm's `ext:ts:<pkg>/...` → prefix symbols with `<pkg>.`) recognize the
/// pulled file the same as a walker-emitted one.
pub(crate) fn virtual_path_for_pulled(abs: &Path, language: &str) -> Option<String> {
    let s = abs.to_string_lossy().replace('\\', "/");
    match language {
        "typescript" | "tsx" | "javascript" => {
            // Standard layout: `.../node_modules/<pkg>/...`.
            let after = if let Some(nm) = s.rfind("/node_modules/") {
                s[nm + "/node_modules/".len()..].to_string()
            } else {
                // Test/override layout: `BEARWISDOM_TS_NODE_MODULES` points
                // at a directory whose direct children are packages. Treat
                // that directory as the `node_modules` root.
                let env = std::env::var_os("BEARWISDOM_TS_NODE_MODULES")?;
                let root = env.to_string_lossy().replace('\\', "/");
                let root = root.trim_end_matches('/');
                s.strip_prefix(&format!("{root}/"))?.to_string()
            };
            let parts: Vec<&str> = after.splitn(4, '/').collect();
            if parts.is_empty() {
                return None;
            }
            let (pkg, rel) = if parts[0].starts_with('@') && parts.len() >= 3 {
                (format!("{}/{}", parts[0], parts[1]), parts[2..].join("/"))
            } else {
                (parts[0].to_string(), parts[1..].join("/"))
            };
            // Reject pnpm `.ignored_*` shadows and other dot-prefixed
            // directories that masquerade as packages — same gate npm.rs
            // applies at the dep-discovery side.
            if !crate::ecosystem::npm::is_valid_npm_module_path(&pkg) {
                return None;
            }
            Some(format!("ext:ts:{pkg}/{rel}"))
        }
        "rust" => {
            // Cargo registry layout: `.../registry/src/<index>/<crate>-<ver>/<rel>`.
            // Reconstruct the eager walker's `ext:rust:<crate>/<rel>` shape (the
            // crate dir name has the version stripped) so a demand-pulled crate
            // file matches the `already_walked` dedupe against walker output.
            let src_idx = s.find("/registry/src/")?;
            let after_src = &s[src_idx + "/registry/src/".len()..];
            // Skip the registry index directory segment.
            let (_index, after_index) = after_src.split_once('/')?;
            let (crate_dir, rel) = after_index.split_once('/')?;
            let (name, _version) = crate::ecosystem::cargo::split_crate_dir_name(crate_dir)?;
            if rel.is_empty() {
                return None;
            }
            Some(format!("ext:rust:{name}/{rel}"))
        }
        "go" => {
            // Module cache: `.../pkg/mod/<module>@<ver>/<rel>`.
            if let Some(mod_idx) = s.find("/pkg/mod/") {
                let after = &s[mod_idx + "/pkg/mod/".len()..];
                return Some(format!("ext:go/{after}"));
            }
            // GOROOT stdlib: `<goroot>/src/<pkg-path>/<file>.go` — string-
            // identical to the shape go_stdlib's per-package eager walker
            // emits, so pulled and walked stdlib files share one identity.
            let src_idx = s.find("/src/")?;
            let after = &s[src_idx + "/src/".len()..];
            if after.is_empty() {
                return None;
            }
            Some(format!("ext:go-stdlib/{after}"))
        }
        "ruby" => {
            // Bundler/RubyGems layout: `.../gems/<gem>-<version>[-<platform>]/<rel>`,
            // where `<rel>` starts with `lib/`. Reconstruct the eager walker's
            // `ext:ruby:<gem>/<rel>` shape (version stripped) so a demand-pulled
            // gem file matches the `already_walked` dedupe against walker output,
            // and so `ExtMatch::PkgSegment` can read the gem name back out of
            // the virtual path.
            if let Some(gems_idx) = s.rfind("/gems/") {
                let after_gems = &s[gems_idx + "/gems/".len()..];
                if let Some((gem_dir, rel)) = after_gems.split_once('/') {
                    if let Some((name, _version)) =
                        crate::ecosystem::cargo::split_crate_dir_name(gem_dir)
                    {
                        if !rel.is_empty() {
                            return Some(format!("ext:ruby:{name}/{rel}"));
                        }
                    }
                }
            }
            // Stdlib layout: `RbConfig::CONFIG['rubylibdir']`, universally
            // `<install>/lib/ruby/<version>/<rel>` (`json.rb`, `net/http.rb`,
            // ...) — no `/gems/` segment, since a `require` pulls these from
            // Ruby's own load path rather than a Bundler/RubyGems install. A
            // bare `require "json"` never re-tags the later `JSON` constant
            // reference with a module, so binding it depends on the
            // `ambient_scope` rung the same way FPC's implicit System unit
            // and Elixir's `Enum`/`Map` do: the `ruby-stdlib` ecosystem
            // segment is what `is_ambient_global_lib_path` classifies as
            // ambient scope.
            if let Some(rl_idx) = s.rfind("/lib/ruby/") {
                let after = &s[rl_idx + "/lib/ruby/".len()..];
                if let Some((_version, rel)) = after.split_once('/') {
                    if !rel.is_empty() {
                        return Some(format!("ext:ruby-stdlib:{rel}"));
                    }
                }
            }
            None
        }
        "dart" => {
            // Flutter SDK layout: `.../packages/<pkg>/lib/<rel>`. `ext:flutter-
            // sdk:<pkg>/<rel>` (rel WITHOUT the `lib/` segment) is the eager
            // walker's shape (flutter_sdk.rs's `dep.root` is the package's
            // `lib/` dir, so its own `rel` is already relative to `lib/`) — so
            // demand-pulled SDK files dedupe against walker output and
            // `ExtMatch::PkgSegment` reads a real package name instead of a
            // drive-letter fragment.
            if let Some(pk_idx) = s.rfind("/packages/") {
                let after = &s[pk_idx + "/packages/".len()..];
                if let Some((pkg, rest)) = after.split_once('/') {
                    if let Some(rel) = rest.strip_prefix("lib/") {
                        if !pkg.is_empty() && !rel.is_empty() {
                            return Some(format!("ext:flutter-sdk:{pkg}/{rel}"));
                        }
                    }
                }
            }
            // Pub-cache layout: `.../hosted/pub.dev/<pkg>-<ver>/lib/<rel>`.
            // `ext:dart:<pkg>/<rel>` is the eager `PubEcosystem` walker's shape
            // (pub_pkg/walk.rs — `dep.root` is the package's `lib/` dir, so its
            // own `rel` is already relative to `lib/`), version stripped, so a
            // demand-pulled package file dedupes against walker output.
            if let Some(hosted_idx) = s.rfind("/hosted/pub.dev/") {
                let after = &s[hosted_idx + "/hosted/pub.dev/".len()..];
                if let Some((pkg_dir, rest)) = after.split_once('/') {
                    if let Some(rel) = rest.strip_prefix("lib/") {
                        if let Some((pkg, _version)) =
                            crate::ecosystem::cargo::split_crate_dir_name(pkg_dir)
                        {
                            if !rel.is_empty() {
                                return Some(format!("ext:dart:{pkg}/{rel}"));
                            }
                        }
                    }
                }
            }
            // Dart-SDK layout: `.../lib/<lib>/<rel>`, `<lib>` one of the
            // recognized stdlib sub-libraries. `ext:dart-sdk:<lib>/<rel>` is
            // the eager `DartSdkEcosystem` walker's shape (dart_sdk.rs —
            // `dep.root` is the SDK's `lib/` dir), matched by SUB-LIBRARY name
            // rather than an install-root literal since the SDK's `lib/`
            // parent varies by platform and probe path (`dart-sdk/lib`,
            // `usr/lib/dart/lib`, a `FLUTTER_ROOT`-bundled cache, …).
            let mut search = 0;
            while let Some(i) = s[search..].find("/lib/") {
                let start = search + i + "/lib/".len();
                let after = &s[start..];
                if let Some((lib_name, rel)) = after.split_once('/') {
                    if crate::ecosystem::dart_sdk::DART_SDK_LIBS.contains(&lib_name)
                        && !rel.is_empty()
                    {
                        return Some(format!("ext:dart-sdk:{lib_name}/{rel}"));
                    }
                }
                search = start;
            }
            None
        }
        "elixir" => {
            // Mix vendors hex deps into the project: `.../deps/<pkg>/<rel>`.
            // `ext:elixir:<pkg>/<rel>` agrees with the hex walker's shape so
            // `ExtMatch::PkgSegment` reads the package name back out.
            if let Some(deps_idx) = s.rfind("/deps/") {
                let after = &s[deps_idx + "/deps/".len()..];
                if let Some((pkg, rel)) = after.split_once('/') {
                    if !pkg.is_empty() && !rel.is_empty() {
                        return Some(format!("ext:elixir:{pkg}/{rel}"));
                    }
                }
            }
            // OTP install layout: `.../lib/<app>/lib/<rel>` (app = elixir,
            // ex_unit, logger, mix, …). The `elixir-stdlib` ecosystem segment
            // is load-bearing: `is_ambient_global_lib_path` classifies
            // `ext:<eco>-stdlib:` files as ambient scope, which is what lets
            // bare stdlib module names (`Enum`, `Map`) bind without imports.
            // The DEEPEST valid `/lib/<app>/lib/` boundary wins — an install
            // prefix that itself ends in `lib/<name>` would otherwise parse as
            // the app and swallow the real boundary into the rel.
            let mut hit: Option<String> = None;
            let mut search = 0;
            while let Some(i) = s[search..].find("/lib/") {
                let start = search + i + "/lib/".len();
                let Some((app, rest)) = s[start..].split_once('/') else {
                    break;
                };
                if let Some(rel) = rest.strip_prefix("lib/") {
                    if !app.is_empty() && !rel.is_empty() {
                        hit = Some(format!("ext:elixir-stdlib:{app}/{rel}"));
                    }
                }
                search = start;
            }
            hit
        }
        "erlang" => {
            // OTP layout: `.../lib/<app>-<version>/src/<file>`. Extract the
            // app name (strip version suffix) and the file path relative to
            // the src/ directory so demand-pulled files match the virtual
            // paths emitted by erlang_otp demand_pre_pull.
            let lib_idx = s.rfind("/lib/")?;
            let after_lib = &s[lib_idx + "/lib/".len()..];
            let app_ver = after_lib.split('/').next()?;
            let app = app_ver.split('-').next()?;
            let src_idx = after_lib.find("/src/")?;
            let rel = &after_lib[src_idx + "/src/".len()..];
            Some(format!("ext:erlang:{app}/{rel}"))
        }
        "nim" => {
            // Nimble layout: `~/.nimble/pkgs2/<pkg>-<version>[-<hash>]/<rel>`.
            // Stdlib layout: `<nim-install>/lib/<rel>`.
            // Both produce `ext:nim:<pkg>/<rel>` matching the virtual paths
            // emitted by `walk_dir_bounded` and `nim_stdlib_pre_pull`.
            if let Some(pkgs_idx) = s.rfind("/pkgs2/") {
                let after = &s[pkgs_idx + "/pkgs2/".len()..];
                // `<pkg>-<version>-<hash>/...` — strip version+hash suffix.
                let slash = after.find('/')?;
                let dir_name = &after[..slash];
                let rel = &after[slash + 1..];
                // Package directory name may be `<pkg>-<version>[-<hash>]`
                // or just `<pkg>` (no version). Extract the bare pkg name as
                // the leading sequence of `-`-separated components up to (but
                // not including) the first component that starts with a digit
                // (the semver major). Hash components start with a hex char
                // that may not be a digit, so we stop at the semver component,
                // which always starts with a digit.
                let pkg: String = dir_name
                    .split('-')
                    .take_while(|part| part.chars().next().map_or(false, |c| !c.is_ascii_digit()))
                    .collect::<Vec<_>>()
                    .join("-");
                if pkg.is_empty() || rel.is_empty() {
                    return None;
                }
                return Some(format!("ext:nim:{pkg}/{rel}"));
            }
            // Stdlib: path contains `/lib/` and is under a Nim install.
            // The stdlib pre-pull uses `ext:nim:nim-stdlib/<rel>` where <rel>
            // is relative to `<install>/lib/`. Match files from both the
            // compiler's lib/ and the scoop shim at the same virtual root.
            if let Some(lib_idx) = s.rfind("/lib/") {
                let rel = &s[lib_idx + "/lib/".len()..];
                // Sanity-check: must end in .nim and not be nested under site-packages
                if rel.ends_with(".nim") && !s.contains("/site-packages/") {
                    return Some(format!("ext:nim:nim-stdlib/{rel}"));
                }
            }
            None
        }
        "python" => {
            // Installed-package layout: `.../site-packages/<pkg>/<rel>` —
            // dist-info and egg-link installs place the import root directly
            // under `site-packages/` too, so the top-level-dir rule covers
            // them. `ext:py:<pkg>/<rel>` is the eager pypi walker's shape
            // (pypi/walk.rs — `dep.root` is the package dir, single-file
            // modules key on the file stem), so a demand-pulled file dedupes
            // against walker output and `ExtMatch::PkgSegment` reads a real
            // package name. Checked before `/Lib/` — a Windows venv nests
            // `Lib/site-packages/`.
            if let Some(idx) = s.rfind("/site-packages/") {
                let after = &s[idx + "/site-packages/".len()..];
                return py_virtual_from_remainder(after);
            }
            // Stdlib layout: `<install>/Lib/<rel>`. The top module is the
            // first path segment (the file stem for single-file modules:
            // `typing.py` → `ext:py:typing/typing.py`).
            if let Some(idx) = s.rfind("/Lib/") {
                let after = &s[idx + "/Lib/".len()..];
                return py_virtual_from_remainder(after);
            }
            None
        }
        "pascal" => pascal::virtual_path_for_pulled_pascal(&s, abs),
        _ => None,
    }
}

/// Shape a path remainder under a python import root as `ext:py:<pkg>/<rel>`.
/// A remainder with a directory segment already leads with its top module
/// (`requests/api.py` → `ext:py:requests/api.py`); a bare single-file module
/// keys on its stem (`six.py` → `ext:py:six/six.py`).
fn py_virtual_from_remainder(rel: &str) -> Option<String> {
    if rel.is_empty() {
        return None;
    }
    if rel.contains('/') {
        return Some(format!("ext:py:{rel}"));
    }
    let stem = rel.strip_suffix(".pyi").or_else(|| rel.strip_suffix(".py"))?;
    if stem.is_empty() {
        return None;
    }
    Some(format!("ext:py:{stem}/{rel}"))
}

#[cfg(test)]
#[path = "ext_virtual_path_tests.rs"]
mod tests;

/// Whether `path` is a virtual external path (the `ext:<ecosystem>:...` scheme
/// every locator assigns to supply files) rather than a project-relative one.
pub(crate) fn is_virtual_external(path: &str) -> bool {
    path.starts_with("ext:")
}
