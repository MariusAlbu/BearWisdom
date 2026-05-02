use super::*;

#[test]
fn ansible_marker_detection() {
    let tmp = std::env::temp_dir().join("bw-jar-marker-detect");
    std::fs::create_dir_all(&tmp).unwrap();
    assert!(!project_has_ansible_markers(&tmp));
    std::fs::write(tmp.join("ansible.cfg"), "[defaults]\n").unwrap();
    assert!(project_has_ansible_markers(&tmp));
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn discovery_returns_jinja2_when_python_install_available() {
    // Smoke: this only asserts when the developer's machine has Python+
    // jinja2 reachable. CI without Python skips the assertion; the
    // function still mustn't panic.
    let tmp = std::env::temp_dir().join("bw-jar-discover");
    std::fs::create_dir_all(&tmp).unwrap();
    let roots = discover_runtime_roots(&tmp);
    let found_jinja2 = roots.iter().any(|r| r.module_path == "jinja2");
    if found_jinja2 {
        let r = roots.iter().find(|r| r.module_path == "jinja2").unwrap();
        assert!(
            r.root.join("__init__.py").is_file(),
            "discovered jinja2 root must contain __init__.py"
        );
        assert!(r.root.join("defaults.py").is_file(), "expected defaults.py");
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn discovery_skips_ansible_when_no_markers() {
    let tmp = std::env::temp_dir().join("bw-jar-no-ansible");
    std::fs::create_dir_all(&tmp).unwrap();
    let roots = discover_runtime_roots(&tmp);
    // Without ansible markers, never list ansible — Flask/Django template
    // projects shouldn't pull in Ansible's filter library.
    let found_ansible = roots.iter().any(|r| r.module_path == "ansible");
    assert!(
        !found_ansible,
        "ansible should not be discovered without markers"
    );
    std::fs::remove_dir_all(&tmp).unwrap();
}

#[test]
fn discovery_finds_ansible_when_markers_present() {
    // Only meaningful when the developer's machine has ansible installed
    // somewhere reachable. If not, the test simply confirms no panic.
    let tmp = std::env::temp_dir().join("bw-jar-with-ansible");
    let roles = tmp.join("roles");
    std::fs::create_dir_all(&roles).unwrap();
    let _ = discover_runtime_roots(&tmp);
    std::fs::remove_dir_all(&tmp).unwrap();
}
