// Sibling tests for `matlab_runtime.rs`. Verifies probe + walk shape against
// stub fixture directories — no installed MATLAB required.

use super::*;
use std::fs;
use tempfile::TempDir;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn make_install_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // Mimic `<MATLABROOT>/toolbox/<area>/...` layout.
    write_file(&root.join("toolbox/matlab/general/zeros.m"), "% builtin");
    write_file(&root.join("toolbox/matlab/general/ones.m"), "% builtin");
    write_file(&root.join("toolbox/stats/pdist2.m"), "% Stats Toolbox");
    write_file(&root.join("toolbox/stats/normcdf.m"), "% Stats Toolbox");
    write_file(&root.join("toolbox/nnet/deep/dlarray.m"), "% Deep Learning");
    write_file(&root.join("toolbox/matlab/uitools/uibutton.m"), "% App Designer");
    // Should be skipped:
    write_file(&root.join("toolbox/matlab/tests/test_zeros.m"), "% test");
    write_file(&root.join("toolbox/stats/private/helper.m"), "% private");
    write_file(&root.join("toolbox/matlab/general/Contents.m"), "% TOC");
    write_file(&root.join("toolbox/matlab/general/ja_JP/help.m"), "% i18n");
    write_file(&root.join("bin/matlab"), "");
    tmp
}

#[test]
fn probe_via_env_override_finds_install_root() {
    let fixture = make_install_fixture();
    let key = "BEARWISDOM_MATLAB_ROOT";
    std::env::set_var(key, fixture.path());
    let probed = probe_matlab_root();
    std::env::remove_var(key);
    assert_eq!(probed.as_deref(), Some(fixture.path()));
}

#[test]
fn discover_returns_toolbox_dir_when_install_has_one() {
    let fixture = make_install_fixture();
    let key = "BEARWISDOM_MATLAB_ROOT";
    std::env::set_var(key, fixture.path());
    let roots = discover_matlab_toolbox();
    std::env::remove_var(key);
    assert_eq!(roots.len(), 1);
    assert!(roots[0].root.ends_with("toolbox"));
}

#[test]
fn discover_returns_empty_when_install_missing_toolbox() {
    let tmp = TempDir::new().unwrap();
    write_file(&tmp.path().join("bin/matlab"), "");
    let key = "BEARWISDOM_MATLAB_ROOT";
    std::env::set_var(key, tmp.path());
    let roots = discover_matlab_toolbox();
    std::env::remove_var(key);
    assert!(roots.is_empty());
}

#[test]
fn walk_picks_up_toolbox_m_files_skipping_noise() {
    let fixture = make_install_fixture();
    let dep = ExternalDepRoot {
        module_path: "matlab-runtime".to_string(),
        version: String::new(),
        root: fixture.path().join("toolbox"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_toolbox_tree(&dep);
    let names: Vec<String> = walked
        .iter()
        .map(|w| {
            w.absolute_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    // Real toolbox functions surface.
    assert!(names.contains(&"zeros.m".to_string()));
    assert!(names.contains(&"ones.m".to_string()));
    assert!(names.contains(&"pdist2.m".to_string()));
    assert!(names.contains(&"normcdf.m".to_string()));
    assert!(names.contains(&"dlarray.m".to_string()));
    assert!(names.contains(&"uibutton.m".to_string()));
    // Noise filtered.
    assert!(!names.contains(&"test_zeros.m".to_string()));
    assert!(!names.contains(&"helper.m".to_string()));
    assert!(!names.contains(&"Contents.m".to_string()));
    assert!(!names.contains(&"help.m".to_string()));
}

#[test]
fn walked_files_carry_ext_matlab_virtual_prefix() {
    let fixture = make_install_fixture();
    let dep = ExternalDepRoot {
        module_path: "matlab-runtime".to_string(),
        version: String::new(),
        root: fixture.path().join("toolbox"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_toolbox_tree(&dep);
    assert!(!walked.is_empty());
    for w in &walked {
        assert!(
            w.relative_path.starts_with("ext:matlab:"),
            "unexpected virtual path: {}",
            w.relative_path
        );
        assert_eq!(w.language, "matlab");
    }
}

#[test]
fn release_dir_recognizer_accepts_canonical_layouts() {
    use std::path::PathBuf;
    assert!(looks_like_matlab_release_dir(&PathBuf::from("/x/R2024a")));
    assert!(looks_like_matlab_release_dir(&PathBuf::from("/x/R2023b")));
    assert!(looks_like_matlab_release_dir(&PathBuf::from("/x/MATLAB_R2024a.app")));
    assert!(!looks_like_matlab_release_dir(&PathBuf::from("/x/something")));
}

#[test]
fn ecosystem_identity() {
    let eco = MatlabRuntimeEcosystem;
    assert_eq!(eco.id().as_str(), "matlab-runtime");
    assert_eq!(eco.kind(), EcosystemKind::Stdlib);
    assert_eq!(eco.languages(), &["matlab"]);
}
