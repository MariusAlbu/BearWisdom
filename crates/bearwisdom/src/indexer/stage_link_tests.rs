use super::{canonical_dedup_path, virtual_path_for_pulled};
use std::path::Path;

#[cfg(unix)]
fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// Two symlinks pointing at the same physical directory must produce an
/// identical dedup key — the contract `parse_external_sources`'s dep-root
/// dedup relies on to collapse a pnpm-hoisted package's N per-consumer
/// symlinks into one `ExternalDepRoot`. Symlink creation needs elevation on
/// Windows, so on a denial the fixture can't be built and the test returns
/// early rather than failing — the contract still holds wherever symlinks
/// are creatable (Unix, elevated Windows).
#[test]
fn canonical_dedup_path_collapses_symlinks_to_same_target() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let real = tmp.path().join("real_pkg");
    std::fs::create_dir(&real).unwrap();

    let link_a = tmp.path().join("consumer_a_node_modules_pkg");
    let link_b = tmp.path().join("consumer_b_node_modules_pkg");
    if create_dir_symlink(&real, &link_a).is_err() {
        eprintln!("skipping: symlink creation denied (needs elevation on this Windows host)");
        return;
    }
    create_dir_symlink(&real, &link_b).expect("second symlink");

    assert_eq!(
        canonical_dedup_path(&link_a),
        canonical_dedup_path(&link_b),
        "two symlinks to the same physical directory must dedup to one key"
    );
}

/// Windows' `canonicalize` returns the `\\?\`-verbatim form; the dedup key
/// must not carry it, since the rest of the codebase compares/`strip_prefix`s
/// plain paths.
#[test]
fn canonical_dedup_path_strips_windows_verbatim_prefix() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let resolved = canonical_dedup_path(tmp.path());
    let s = resolved.to_string_lossy();
    assert!(!s.starts_with(r"\\?\"), "verbatim prefix leaked: {s}");
}

/// A path that doesn't exist on disk can't be canonicalized — the function
/// must fall back to the input unchanged rather than propagating the error,
/// since a missing-on-disk root is a legitimate (if unresolvable) dep root.
#[test]
fn canonical_dedup_path_falls_back_when_path_does_not_exist() {
    let missing = Path::new("/definitely/does/not/exist/on/this/machine");
    assert_eq!(canonical_dedup_path(missing), missing);
}

#[test]
fn rust_pulled_file_matches_eager_walker_shape() {
    // A demand-pulled cargo registry file must reconstruct the eager walker's
    // `ext:rust:<crate>/<rel-to-crate-root>` virtual path (version stripped),
    // so the `already_walked` dedupe recognizes a re-pulled file.
    let abs = Path::new(
        "/home/u/.cargo/registry/src/index.crates.io-abc/windows-0.61.3/src/Windows/Win32/mod.rs",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "rust").as_deref(),
        Some("ext:rust:windows/src/Windows/Win32/mod.rs"),
    );
}

#[test]
fn rust_pulled_file_handles_hyphenated_crate() {
    let abs = Path::new(
        "/home/u/.cargo/registry/src/index-1/proc-macro2-1.0.91/src/lib.rs",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "rust").as_deref(),
        Some("ext:rust:proc-macro2/src/lib.rs"),
    );
}

#[test]
fn rust_pulled_file_windows_separators() {
    let abs = Path::new(
        r"C:\Users\u\.cargo\registry\src\index-1\serde-1.0.200\src\de\mod.rs",
    );
    assert_eq!(
        virtual_path_for_pulled(abs, "rust").as_deref(),
        Some("ext:rust:serde/src/de/mod.rs"),
    );
}

#[test]
fn rust_pulled_file_non_registry_path_falls_through() {
    // Path outside the registry layout can't be shaped — caller falls back to
    // `ext:idx:`.
    let abs = Path::new("/some/where/else/foo.rs");
    assert_eq!(virtual_path_for_pulled(abs, "rust"), None);
}
