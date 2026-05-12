use super::*;
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// parse_requirements — Shape A (legacy flat list)
// ---------------------------------------------------------------------------

#[test]
fn requirements_yml_parses_shape_a_with_name() {
    let content = r#"---
- src: geerlingguy.nginx
  name: nginx
  version: 1.2.3
- src: geerlingguy.java
  name: java
"#;
    let deps = parse_requirements(content);
    assert!(deps.contains(&"nginx".to_string()));
    assert!(deps.contains(&"java".to_string()));
    // Each role emits the name plus the Galaxy-namespace src, so at least 2.
    assert!(deps.len() >= 2);
}

#[test]
fn requirements_yml_parses_shape_a_without_name_uses_src() {
    let content = r#"---
- src: geerlingguy.docker
  version: 8.0.0
"#;
    let deps = parse_requirements(content);
    // No `name:` — falls back to `src:`, which is a Galaxy name.
    assert!(deps.contains(&"geerlingguy.docker".to_string()));
}

#[test]
fn requirements_yml_parses_git_src_without_name() {
    let content = r#"---
- src: git+https://github.com/devture/com.devture.ansible.role.timesync.git
  version: v1.1.0-1
"#;
    let deps = parse_requirements(content);
    // Stripped URL + `.git` + no `ansible-role-` prefix in this name → last segment.
    assert!(deps.contains(&"com.devture.ansible.role.timesync".to_string()));
}

// ---------------------------------------------------------------------------
// parse_requirements — Shape B (keyed roles/collections)
// ---------------------------------------------------------------------------

#[test]
fn requirements_yml_parses_shape_b_roles() {
    let content = r#"---
roles:
  - src: https://github.com/devture/com.devture.ansible.role.systemd_docker_base.git
    name: systemd_docker_base
  - src: https://github.com/foo/ansible-role-timesync.git
    name: timesync
"#;
    let deps = parse_requirements(content);
    assert!(deps.contains(&"systemd_docker_base".to_string()));
    assert!(deps.contains(&"timesync".to_string()));
    // Each URL-based role also emits an org-prefixed composite.
    assert!(deps.contains(&"devture_systemd_docker_base".to_string()));
    assert!(deps.contains(&"foo_timesync".to_string()));
}

#[test]
fn requirements_yml_parses_shape_b_collections() {
    let content = r#"---
collections:
  - name: community.general
  - name: community.docker
"#;
    let deps = parse_requirements(content);
    assert!(deps.contains(&"community.general".to_string()));
    assert!(deps.contains(&"community.docker".to_string()));
}

#[test]
fn requirements_yml_parses_shape_b_mixed_roles_and_collections() {
    let content = r#"---
roles:
  - name: my_role
collections:
  - name: community.general
"#;
    let deps = parse_requirements(content);
    assert!(deps.contains(&"my_role".to_string()));
    assert!(deps.contains(&"community.general".to_string()));
}

// ---------------------------------------------------------------------------
// parse_requirements — real-world content shape from jinja-matrix-ansible
// ---------------------------------------------------------------------------

#[test]
fn requirements_yml_parses_matrix_ansible_format() {
    // Representative subset of the actual requirements.yml format used by
    // projects that combine `name:` with URL-based `src:` entries.
    let content = r#"---
- src: git+https://github.com/devture/com.devture.ansible.role.systemd_docker_base.git
  version: v1.5.0-0
  name: systemd_docker_base
- src: git+https://github.com/geerlingguy/ansible-role-docker
  version: 8.0.0
  name: docker
- src: git+https://github.com/mother-of-all-self-hosting/ansible-role-traefik.git
  version: v3.6.15-0
  name: traefik
"#;
    let deps = parse_requirements(content);
    assert!(deps.contains(&"systemd_docker_base".to_string()));
    assert!(deps.contains(&"docker".to_string()));
    assert!(deps.contains(&"traefik".to_string()));
    // org-prefixed composites are also emitted so `devture_systemd_docker_base_*`
    // variable refs are classified as external.
    assert!(deps.contains(&"devture_systemd_docker_base".to_string()));
    assert!(deps.contains(&"geerlingguy_docker".to_string()));
}

// ---------------------------------------------------------------------------
// meta_main_yml_dependencies_parsed — not applicable here (no meta/main.yml
// support in this reader; dependencies in meta/main.yml are a separate Ansible
// feature not currently parsed). This test documents that the reader does NOT
// scan meta/main.yml, which is correct for now.
// ---------------------------------------------------------------------------

#[test]
fn meta_main_yml_not_scanned() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // Write only a meta/main.yml — no requirements.yml.
    let meta_dir = root.join("roles/myrole/meta");
    fs::create_dir_all(&meta_dir).unwrap();
    fs::write(
        meta_dir.join("main.yml"),
        "dependencies:\n  - role: somereq\n",
    )
    .unwrap();
    let reader = AnsibleRequirementsManifest;
    // Without requirements.yml at root, reader returns None.
    assert!(reader.read(root).is_none());
}

// ---------------------------------------------------------------------------
// ManifestReader trait integration — read() via filesystem
// ---------------------------------------------------------------------------

#[test]
fn reader_returns_none_when_no_requirements_file() {
    let tmp = TempDir::new().unwrap();
    let reader = AnsibleRequirementsManifest;
    assert!(reader.read(tmp.path()).is_none());
}

#[test]
fn reader_parses_requirements_yml_at_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("requirements.yml"),
        "---\n- src: foo\n  name: myrole\n",
    )
    .unwrap();
    let reader = AnsibleRequirementsManifest;
    let data = reader.read(root).expect("should find requirements.yml");
    assert!(data.dependencies.contains("myrole"));
}

#[test]
fn reader_parses_requirements_yaml_alternate_extension() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("requirements.yaml"),
        "---\n- src: foo\n  name: otherrole\n",
    )
    .unwrap();
    let reader = AnsibleRequirementsManifest;
    let data = reader.read(root).expect("should find requirements.yaml");
    assert!(data.dependencies.contains("otherrole"));
}

// ---------------------------------------------------------------------------
// activation_prefix handling
// ---------------------------------------------------------------------------

#[test]
fn activation_prefix_takes_precedence_over_name_and_src() {
    let content = r#"---
- src: git+https://github.com/mother-of-all-self-hosting/ansible-role-coturn.git
  version: v4.9.0-1
  name: coturn
  activation_prefix: coturn_
"#;
    let deps = parse_requirements(content);
    // activation_prefix is authoritative; only the stripped form is emitted.
    assert!(deps.contains(&"coturn".to_string()));
    assert_eq!(deps.len(), 1);
}

// ---------------------------------------------------------------------------
// normalise_src edge cases
// ---------------------------------------------------------------------------

#[test]
fn normalise_src_strips_ansible_role_prefix() {
    let result = normalise_src("ansible-role-nginx");
    assert_eq!(result, "nginx");
}

#[test]
fn normalise_src_strips_git_url_and_git_suffix() {
    let result = normalise_src("git+https://github.com/geerlingguy/ansible-role-docker.git");
    assert_eq!(result, "docker");
}

#[test]
fn normalise_src_galaxy_style_unchanged() {
    let result = normalise_src("community.general");
    assert_eq!(result, "community.general");
}
