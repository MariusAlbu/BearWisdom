use std::path::{Path, PathBuf};

use super::{canonical_dedup_path, dedup_roots};
use crate::ecosystem::externals::ExternalDepRoot;

#[cfg(unix)]
fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_dir_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

fn root(dir: PathBuf, package_id: Option<i64>) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: String::from("pkg"),
        version: String::from("1.0.0"),
        root: dir,
        ecosystem: "typescript",
        package_id,
        requested_imports: Vec::new(),
    }
}

/// Two symlinks pointing at the same physical directory must produce an
/// identical dedup key — the contract the dep-root dedup relies on to collapse
/// a pnpm-hoisted package's N per-consumer symlinks into one
/// `ExternalDepRoot`. Symlink creation needs elevation on Windows, so on a
/// denial the fixture can't be built and the test returns early rather than
/// failing — the contract still holds wherever symlinks are creatable (Unix,
/// elevated Windows).
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

/// Symlinked copies of one package collapse to a single walk whose declaring
/// package list carries every consumer that reached it.
#[test]
fn symlinked_copies_collapse_and_merge_package_ids() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let real = tmp.path().join("real_pkg");
    std::fs::create_dir(&real).unwrap();
    let link_a = tmp.path().join("a_pkg");
    let link_b = tmp.path().join("b_pkg");
    if create_dir_symlink(&real, &link_a).is_err() {
        eprintln!("skipping: symlink creation denied (needs elevation on this Windows host)");
        return;
    }
    create_dir_symlink(&real, &link_b).expect("second symlink");

    let deduped = dedup_roots(vec![root(link_a, Some(1)), root(link_b, Some(2))]);
    assert_eq!(deduped.len(), 1, "symlinked copies are one physical root");
    assert_eq!(deduped[0].1, vec![1, 2], "both declarers are recorded");
}

/// The same package reached twice from one directory records each declaring
/// package once.
#[test]
fn repeat_declarations_of_one_root_merge_without_duplicates() {
    let dir = PathBuf::from("/ws/node_modules/pkg");
    let deduped = dedup_roots(vec![
        root(dir.clone(), Some(7)),
        root(dir.clone(), Some(7)),
        root(dir, Some(9)),
    ]);
    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].1, vec![7, 9]);
}

/// Distinct physical copies of one module are separate walk targets — a
/// workspace package's own `node_modules` copy must not be folded into the
/// hoisted one.
#[test]
fn distinct_physical_copies_survive_as_separate_roots() {
    let deduped = dedup_roots(vec![
        root(PathBuf::from("/ws/node_modules/pkg"), Some(1)),
        root(PathBuf::from("/ws/apps/b/node_modules/pkg"), Some(2)),
    ]);
    assert_eq!(deduped.len(), 2, "two directories are two roots");
    assert_eq!(deduped[0].1, vec![1]);
    assert_eq!(deduped[1].1, vec![2]);
}
