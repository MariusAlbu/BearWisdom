// =============================================================================
// ecosystem/go_platform.rs — Go GOOS/GOARCH filename-tag filter
//
// Go's build system implicitly excludes source files whose trailing `_GOOS`
// or `_GOOS_GOARCH` filename suffix doesn't match the build target. That
// exclusion rule is how projects like `modernc.org/sqlite` ship one 10MB
// `sqlite_windows.go`, one `sqlite_linux_amd64.go`, one `sqlite_darwin_arm64.go`,
// etc. — only the matching variant ever compiles.
//
// For indexing, walking every platform variant is wasteful and, for big CGo
// modules (`modernc.org/libc` ships 4MB `ccgo_linux_386.go` + eight more),
// pathological: tree-sitter ASTs across all variants blow host RAM past 7GB
// on go-pocketbase full reindex.
//
// This module replicates the subset of Go's `build.goodOSArchFile` rule that
// matters for filtering: trailing-underscore-segment matching against the
// known GOOS / GOARCH word lists. Build-tag comments (`//go:build darwin`)
// are ignored — filename-tag coverage alone drops the pathological fanout.
// =============================================================================

/// Return `true` when `filename` should be walked for the host platform.
///
/// Matches Go's filename-tag constraint: a file named `foo_GOOS.go`,
/// `foo_GOARCH.go`, or `foo_GOOS_GOARCH.go` is only built for the matching
/// host. Files without a platform suffix always pass.
pub fn file_matches_host(filename: &str) -> bool {
    let Some(stem) = strip_go_suffixes(filename) else { return false };
    file_matches(stem, host_goos(), host_goarch())
}

fn strip_go_suffixes(name: &str) -> Option<&str> {
    // Accept `.go` only. Ignore the `_test.go` branch — test files are dropped
    // earlier in the walkers.
    name.strip_suffix(".go")
}

fn file_matches(stem: &str, host_os: &str, host_arch: &str) -> bool {
    // Tail-segment rule:
    //   name_OOS_AARCH  → match both
    //   name_AARCH      → match arch
    //   name_OOS        → match os
    //   everything else → no constraint
    // A leading `name_` must exist; bare `amd64.go` is treated as a normal
    // filename with no tag (Go does the same).
    let segs: Vec<&str> = stem.split('_').collect();
    if segs.len() < 2 { return true }

    let last = segs[segs.len() - 1];
    let prev = if segs.len() >= 3 { Some(segs[segs.len() - 2]) } else { None };

    if is_known_arch(last) {
        if last != host_arch { return false }
        if let Some(p) = prev {
            if is_known_os(p) && p != host_os { return false }
        }
        return true;
    }
    if is_known_os(last) {
        return last == host_os;
    }
    true
}

fn host_goos() -> &'static str {
    // Map Rust's `std::env::consts::OS` onto Go's GOOS vocabulary.
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

fn host_goarch() -> &'static str {
    // Map Rust's `std::env::consts::ARCH` onto Go's GOARCH vocabulary.
    match std::env::consts::ARCH {
        "x86" => "386",
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "powerpc" => "ppc",
        "powerpc64" => "ppc64",
        "loongarch64" => "loong64",
        other => other,
    }
}

fn is_known_os(s: &str) -> bool {
    matches!(
        s,
        "aix" | "android" | "darwin" | "dragonfly" | "freebsd" | "hurd"
            | "illumos" | "ios" | "js" | "linux" | "nacl" | "netbsd"
            | "openbsd" | "plan9" | "solaris" | "wasip1" | "windows" | "zos"
    )
}

fn is_known_arch(s: &str) -> bool {
    matches!(
        s,
        "386" | "amd64" | "amd64p32" | "arm" | "armbe" | "arm64" | "arm64be"
            | "loong64" | "mips" | "mipsle" | "mips64" | "mips64le"
            | "mips64p32" | "mips64p32le" | "ppc" | "ppc64" | "ppc64le"
            | "riscv" | "riscv64" | "s390" | "s390x" | "sparc" | "sparc64"
            | "wasm"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_suffix_always_matches() {
        assert!(file_matches("print", "linux", "amd64"));
        assert!(file_matches("foo_bar", "linux", "amd64"));
    }

    #[test]
    fn os_only_suffix() {
        assert!(file_matches("sqlite_windows", "windows", "amd64"));
        assert!(!file_matches("sqlite_windows", "linux", "amd64"));
        assert!(file_matches("sqlite_linux", "linux", "amd64"));
        assert!(!file_matches("sqlite_linux", "darwin", "amd64"));
    }

    #[test]
    fn arch_only_suffix() {
        assert!(file_matches("rewrite_amd64", "linux", "amd64"));
        assert!(!file_matches("rewrite_amd64", "linux", "386"));
        assert!(file_matches("rewrite_arm64", "darwin", "arm64"));
    }

    #[test]
    fn os_and_arch_suffix() {
        assert!(file_matches("sqlite_linux_amd64", "linux", "amd64"));
        assert!(!file_matches("sqlite_linux_amd64", "windows", "amd64"));
        assert!(!file_matches("sqlite_linux_amd64", "linux", "386"));
        assert!(file_matches("sqlite_darwin_arm64", "darwin", "arm64"));
    }

    #[test]
    fn unknown_trailing_segment_passes() {
        // `foo_utils` is not a GOOS/GOARCH, so no constraint.
        assert!(file_matches("foo_utils", "linux", "amd64"));
        // `config_test` (not `_test.go`, just name) — `test` isn't a platform tag.
        assert!(file_matches("config_test", "linux", "amd64"));
    }

    #[test]
    fn bare_platform_word_is_not_a_tag() {
        // Go treats bare `amd64.go` / `linux.go` as name-only — single segment,
        // no underscore prefix. Our split-based rule needs 2+ segments to
        // trigger, which matches.
        assert!(file_matches("amd64", "linux", "386"));
        assert!(file_matches("linux", "windows", "amd64"));
    }

    #[test]
    fn end_to_end_wrapper() {
        // `file_matches_host` applies host mapping. Bail on non-.go files.
        assert!(!file_matches_host("README.md"));
        // Everything below rides on the host — verify the wrapper reaches
        // file_matches at all by asserting the neutral name passes.
        assert!(file_matches_host("print.go"));
    }
}
