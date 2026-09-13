// =============================================================================
// npm/specifiers.rs — the spelling of npm module specifiers and virtual paths
//
// Package-name extraction from a specifier, module-path validity, and the
// lexical normalization that keys one physical file under one virtual path.
// =============================================================================

use std::path::{Path, PathBuf};

/// Collapse embedded `/./` and `/../` segments and normalise backslashes in
/// a path fragment that's about to land in a virtual `ext:ts:<pkg>/<rel>`
/// URI. `resolve_relative_ts_path` joins specs like `./internal/foo` or
/// `../../foo` without normalising, so a single .d.ts can otherwise show up
/// under multiple virtual paths (`dist/types/Observable.d.ts`,
/// `dist/types/./internal/Observable.d.ts`,
/// `dist/types/internal/../Observable.d.ts`) and confuse downstream
/// dedupe + symbol prefixing.
pub(crate) fn normalize_virtual_rel(rel: &str) -> String {
    let s = rel.replace('\\', "/");
    let mut out: Vec<&str> = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|s| *s != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            _ => out.push(seg),
        }
    }
    out.join("/")
}

/// Collapse `.` and `..` components out of an absolute filesystem path
/// without touching disk (no symlink resolution, no existence check — the
/// path may name a file that doesn't exist yet at call time). Two relative
/// re-export hops that reach the same physical file by different routes
/// (`pkg/a/../b.d.ts` vs `pkg/b.d.ts`) must produce an identical `PathBuf`,
/// since every reachability-closure dedup (`HashSet<PathBuf>`) and
/// `(module, name) → PathBuf` first-writer-wins map is keyed on this
/// identity. A leading `..` that would pop past the start of the path is
/// kept literally rather than silently dropped, since there is no root
/// segment left to remove.
pub(crate) fn lexically_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Reject `dep.module_path` shapes that would produce malformed virtual
/// paths (`ext:ts:./xxx/...`, `ext:ts:F:/xxx/...`, `ext:ts:.ignored_xxx/...`).
///
/// Every walker formats `ext:ts:{module_path}/{rel_sub}` and downstream code
/// assumes a clean npm package shape — `name` or `@scope/name`. Anything
/// else (relative specifiers, drive letters, pnpm `.ignored_*` shadows,
/// `.pnpm/` store paths, hidden dirs) breaks `ts_package_from_virtual_path`,
/// which then either returns garbage prefixes (`F:`, `.`, `.ignored_xxx`)
/// or fails to identify the package at all — leaving the chain walker
/// unable to follow library types like `Observable.pipe()` or `HTMLElement.click()`.
///
/// Reduce a possibly-deep npm specifier to just its package name. Handles
/// scoped (`@scope/pkg/sub` → `@scope/pkg`), unscoped (`pkg/sub` → `pkg`),
/// and already-bare (`pkg` → `pkg`) forms. Returns the input unchanged when
/// the layout doesn't match either shape (callers re-validate).
pub(crate) fn npm_package_name_from_spec(spec: &str) -> &str {
    if let Some(rest) = spec.strip_prefix('@') {
        // Scoped: keep the first two slash-separated segments (`@scope/name`).
        let mut iter = rest.splitn(3, '/');
        let scope = iter.next().unwrap_or("");
        let name = iter.next().unwrap_or("");
        if !scope.is_empty() && !name.is_empty() {
            let end = 1 + scope.len() + 1 + name.len(); // '@' + scope + '/' + name
            return &spec[..end];
        }
        spec
    } else {
        // Unscoped: keep the leading segment.
        match spec.find('/') {
            Some(slash) => &spec[..slash],
            None => spec,
        }
    }
}

/// Used at every `ExternalDepRoot { module_path: … }` construction site to
/// gate which paths get into the index in the first place.
pub(crate) fn is_valid_npm_module_path(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.starts_with('.') {
        return false;
    } // ./, ../, .ignored_, .pnpm
    if name.contains(':') {
        return false;
    } // F:/Work/...
    if name.contains('\\') {
        return false;
    } // windows path leak
    if name.starts_with('@') {
        // Scoped: must be exactly `@scope/name`.
        let rest = &name[1..];
        let Some((scope, pkg)) = rest.split_once('/') else {
            return false;
        };
        if scope.is_empty() || pkg.is_empty() {
            return false;
        }
        if scope.starts_with('.') || pkg.starts_with('.') {
            return false;
        }
        if pkg.contains('/') {
            return false;
        } // no nested paths under @scope
        true
    } else {
        // Unscoped: single segment, no slashes.
        !name.contains('/')
    }
}

// ---------------------------------------------------------------------------
// Node builtins — appear in package.json declared deps but have no on-disk
// source under node_modules. Skipped during walk.
// ---------------------------------------------------------------------------

pub(super) fn node_builtins() -> std::collections::HashSet<&'static str> {
    [
        "assert",
        "buffer",
        "child_process",
        "cluster",
        "console",
        "crypto",
        "dgram",
        "dns",
        "domain",
        "events",
        "fs",
        "http",
        "http2",
        "https",
        "inspector",
        "module",
        "net",
        "node",
        "os",
        "path",
        "perf_hooks",
        "process",
        "punycode",
        "querystring",
        "readline",
        "repl",
        "stream",
        "string_decoder",
        "timers",
        "tls",
        "trace_events",
        "tty",
        "url",
        "util",
        "v8",
        "vm",
        "wasi",
        "worker_threads",
        "zlib",
    ]
    .into_iter()
    .collect()
}
