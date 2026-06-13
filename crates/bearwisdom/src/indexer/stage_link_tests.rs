use super::virtual_path_for_pulled;
use std::path::Path;

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
