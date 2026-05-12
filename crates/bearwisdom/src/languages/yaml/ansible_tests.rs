use super::*;
use crate::types::SymbolKind;

// ---------------------------------------------------------------------------
// Path classification
// ---------------------------------------------------------------------------

#[test]
fn role_defaults_main_yml_classified() {
    let scope = classify_ansible_path("roles/webserver/defaults/main.yml");
    assert_eq!(scope.as_deref(), Some("webserver"));
}

#[test]
fn role_defaults_main_yaml_classified() {
    let scope = classify_ansible_path("roles/webserver/defaults/main.yaml");
    assert_eq!(scope.as_deref(), Some("webserver"));
}

#[test]
fn role_vars_main_yml_classified() {
    let scope = classify_ansible_path("roles/db/vars/main.yml");
    assert_eq!(scope.as_deref(), Some("db"));
}

#[test]
fn custom_subdir_role_classified() {
    // roles/custom/<role>/defaults/main.yml — intermediate `custom` dir.
    let scope = classify_ansible_path("roles/custom/matrix-base/defaults/main.yml");
    assert_eq!(scope.as_deref(), Some("matrix-base"));
}

#[test]
fn group_vars_flat_yml_classified() {
    let scope = classify_ansible_path("group_vars/all.yml");
    assert_eq!(scope.as_deref(), Some("group_vars.all"));
}

#[test]
fn group_vars_directory_yml_classified() {
    let scope = classify_ansible_path("group_vars/webservers/vars.yml");
    assert_eq!(scope.as_deref(), Some("group_vars.webservers"));
}

#[test]
fn host_vars_flat_yml_classified() {
    let scope = classify_ansible_path("host_vars/server1.yml");
    assert_eq!(scope.as_deref(), Some("host_vars.server1"));
}

#[test]
fn host_vars_directory_yml_classified() {
    let scope = classify_ansible_path("host_vars/server1/main.yml");
    assert_eq!(scope.as_deref(), Some("host_vars.server1"));
}

#[test]
fn inventory_group_vars_path_recognized() {
    let scope = classify_ansible_path("inventory/sample/group_vars/all/all.yml");
    assert_eq!(scope.as_deref(), Some("group_vars.all"));
}

#[test]
fn inventory_host_vars_path_recognized() {
    let scope = classify_ansible_path("inventory/sample/host_vars/node1/main.yml");
    assert_eq!(scope.as_deref(), Some("host_vars.node1"));
}

#[test]
fn non_ansible_yaml_not_classified() {
    assert!(classify_ansible_path("src/config.yml").is_none());
    assert!(classify_ansible_path(".github/workflows/ci.yml").is_none());
    assert!(classify_ansible_path("roles/myapp/tasks/main.yml").is_none());
    assert!(classify_ansible_path("roles/myapp/handlers/main.yml").is_none());
}

#[test]
fn windows_path_separator_normalised() {
    let scope = classify_ansible_path(r"roles\webserver\defaults\main.yml");
    assert_eq!(scope.as_deref(), Some("webserver"));
}

// ---------------------------------------------------------------------------
// Symbol extraction
// ---------------------------------------------------------------------------

#[test]
fn defaults_main_yml_emits_field_symbol_per_top_level_key() {
    let source = r#"---
# Comment line — should be skipped
nginx_port: 80
nginx_worker_processes: 4
nginx_enabled: true
"#;
    let result = extract_ansible(source, "roles/webserver/defaults/main.yml", "webserver");
    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    // Class symbol + 3 field symbols.
    assert!(names.contains(&"webserver"), "class symbol missing");
    assert!(names.contains(&"nginx_port"));
    assert!(names.contains(&"nginx_worker_processes"));
    assert!(names.contains(&"nginx_enabled"));
    assert_eq!(result.symbols.len(), 4);
}

#[test]
fn group_vars_yml_qnames_correctly() {
    let source = "domain_name: example.com\nmax_connections: 100\n";
    let result = extract_ansible(source, "group_vars/all.yml", "group_vars.all");
    let qnames: Vec<&str> = result
        .symbols
        .iter()
        .map(|s| s.qualified_name.as_str())
        .collect();
    assert!(qnames.contains(&"group_vars.all"));
    assert!(qnames.contains(&"group_vars.all.domain_name"));
    assert!(qnames.contains(&"group_vars.all.max_connections"));
}

#[test]
fn host_vars_yml_qnames_correctly() {
    let source = "ansible_host: 10.0.0.1\nrole: primary\n";
    let result = extract_ansible(source, "host_vars/server1.yml", "host_vars.server1");
    let qnames: Vec<&str> = result
        .symbols
        .iter()
        .map(|s| s.qualified_name.as_str())
        .collect();
    assert!(qnames.contains(&"host_vars.server1"));
    assert!(qnames.contains(&"host_vars.server1.ansible_host"));
    assert!(qnames.contains(&"host_vars.server1.role"));
}

#[test]
fn field_symbols_have_correct_kind() {
    let source = "my_var: value\n";
    let result = extract_ansible(source, "roles/app/defaults/main.yml", "app");
    let field = result
        .symbols
        .iter()
        .find(|s| s.name == "my_var")
        .expect("field symbol missing");
    assert_eq!(field.kind, SymbolKind::Field);
}

#[test]
fn comment_and_blank_lines_skipped() {
    let source = "# comment\n\nreal_var: 1\n";
    let result = extract_ansible(source, "roles/app/defaults/main.yml", "app");
    assert_eq!(result.symbols.len(), 2); // class + 1 field
}

#[test]
fn indented_keys_not_emitted_as_top_level() {
    let source = "top_level:\n  nested_key: value\nanother_top: 2\n";
    let result = extract_ansible(source, "roles/app/defaults/main.yml", "app");
    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(!names.contains(&"nested_key"), "nested key should not be emitted");
    assert!(names.contains(&"top_level"));
    assert!(names.contains(&"another_top"));
}

#[test]
fn line_numbers_are_correct() {
    let source = "---\nfirst_var: 1\nsecond_var: 2\n";
    let result = extract_ansible(source, "roles/app/defaults/main.yml", "app");
    let first = result
        .symbols
        .iter()
        .find(|s| s.name == "first_var")
        .unwrap();
    assert_eq!(first.start_line, 1);
    let second = result
        .symbols
        .iter()
        .find(|s| s.name == "second_var")
        .unwrap();
    assert_eq!(second.start_line, 2);
}
