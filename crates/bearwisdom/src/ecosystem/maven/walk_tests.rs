use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::walk_generic_jvm_root;
use crate::ecosystem::externals::ExternalDepRoot;

fn write(root: &Path, rel: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, "package p\n").unwrap();
}

fn dep(root: &Path) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "org.example:lib".to_string(),
        version: "1.0".to_string(),
        root: root.to_path_buf(),
        ecosystem: "maven",
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// A package directory named `test` is source; a `src/test` source set and
/// test-suffixed files are not.
#[test]
fn a_test_package_directory_is_walked_but_a_test_source_set_is_not() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "assertionsCommonMain/kotlin/test/Assertions.kt");
    write(tmp.path(), "commonMain/kotlin/tests/Helpers.kt");
    write(tmp.path(), "src/test/java/org/example/LibTest.java");
    write(tmp.path(), "src/test/java/org/example/Support.java");
    write(tmp.path(), "src/main/java/org/example/Lib.java");
    write(tmp.path(), "src/main/java/org/example/LibTest.java");
    write(tmp.path(), "META-INF/MANIFEST.MF");

    let mut walked: Vec<String> = walk_generic_jvm_root(&dep(tmp.path()))
        .into_iter()
        .map(|f| f.relative_path.replace('\\', "/"))
        .collect();
    walked.sort();
    let tail = |p: &str| walked.iter().any(|w| w.ends_with(p));
    assert!(
        tail("assertionsCommonMain/kotlin/test/Assertions.kt"),
        "{walked:?}"
    );
    assert!(tail("commonMain/kotlin/tests/Helpers.kt"), "{walked:?}");
    assert!(tail("src/main/java/org/example/Lib.java"), "{walked:?}");
    assert!(
        !tail("Support.java"),
        "the src/test source set is pruned: {walked:?}"
    );
    assert!(
        !tail("LibTest.java"),
        "test-suffixed files are skipped: {walked:?}"
    );
    assert_eq!(walked.len(), 3, "{walked:?}");
}
