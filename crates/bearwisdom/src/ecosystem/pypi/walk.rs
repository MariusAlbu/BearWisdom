// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

use std::path::Path;

use crate::ecosystem::externals::{ExternalDepRoot, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub fn walk_python_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    if dep.root.is_file() {
        let file_name = dep
            .root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("module.py");
        let virtual_path = format!("ext:py:{}/{}", dep.module_path, file_name);
        out.push(WalkedFile {
            relative_path: virtual_path,
            absolute_path: dep.root.clone(),
            language: "python",
        });
    } else {
        walk_python_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    }
    out
}

fn walk_python_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // `tests` (plural) and `_test` are conventional Python
                // test-fixture directories that hold the package's own
                // `test_X.py` files — skip them so we don't index a
                // dep's own test suite.
                //
                // BUT: `test` (singular) is used by major packages —
                // Django's `django/test/` exposes `TestCase`, `Client`,
                // `RequestFactory`, `assertRaisesMessage`, etc. as
                // public API. unittest itself ships its public API
                // under the `unittest` package's `case.py`/`mock.py` —
                // already covered. Skipping `test` indiscriminately
                // hides every Django test-method ref. Keep `test`
                // walked; the per-file `test_*.py` / `_test.py` /
                // `conftest.py` filter below catches actual test
                // fixtures inside.
                if matches!(name, "__pycache__" | "tests" | ".git" | "_test") {
                    continue;
                }
                if name.ends_with(".dist-info") || name.ends_with(".egg-info") { continue }
            }
            walk_python_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".py") { continue }
            if name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py" {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:py:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "python",
            });
        }
    }
}
