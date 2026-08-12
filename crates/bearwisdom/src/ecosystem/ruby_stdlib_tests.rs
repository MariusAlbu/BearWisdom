use super::*;

fn mkdep(root: PathBuf) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "ruby-stdlib".to_string(),
        version: String::new(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn ecosystem_identity() {
    let e = RubyStdlibEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["ruby"]);
}

#[test]
fn walk_stamps_ruby_stdlib_ecosystem_segment() {
    // Agrees with `ext_virtual_path::virtual_path_for_pulled`'s demand-pull
    // shape for the SAME layout, so a re-pulled stdlib file dedupes against
    // this walk's output.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("lib").join("ruby").join("3.3.0");
    std::fs::create_dir_all(root.join("net")).unwrap();
    std::fs::write(root.join("json.rb"), "module JSON; end\n").unwrap();
    std::fs::write(root.join("net").join("http.rb"), "module Net; end\n").unwrap();

    let dep = mkdep(root);
    let mut walked = walk_ruby_tree(&dep);
    walked.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    let paths: Vec<&str> = walked.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["ext:ruby-stdlib:json.rb", "ext:ruby-stdlib:net/http.rb"],
    );
}

#[test]
fn walk_excludes_test_and_spec_dirs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("lib").join("ruby").join("3.3.0");
    std::fs::create_dir_all(root.join("test")).unwrap();
    std::fs::write(root.join("set.rb"), "class Set; end\n").unwrap();
    std::fs::write(root.join("test").join("test_set.rb"), "# test\n").unwrap();

    let dep = mkdep(root);
    let walked = walk_ruby_tree(&dep);
    assert_eq!(walked.len(), 1);
    assert_eq!(walked[0].relative_path, "ext:ruby-stdlib:set.rb");
}
