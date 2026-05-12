use std::collections::HashMap;

use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;

use super::infer_ansible_external;

fn ctx_with_roles(role_names: &[&str]) -> ProjectContext {
    let mut deps = std::collections::HashSet::new();
    for &name in role_names {
        deps.insert(name.to_string());
    }
    let data = ManifestData { dependencies: deps, ..Default::default() };
    let mut manifests = HashMap::new();
    manifests.insert(ManifestKind::AnsibleRequirements, data);
    ProjectContext {
        manifests,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// infer_ansible_external
// ---------------------------------------------------------------------------

#[test]
fn jinja_resolver_routes_declared_external_role_prefix_to_external() {
    let ctx = ctx_with_roles(&["systemd_docker_base"]);
    let ns = infer_ansible_external("systemd_docker_base_docker_service_name", Some(&ctx));
    assert_eq!(ns.as_deref(), Some("ansible.systemd_docker_base"));
}

#[test]
fn jinja_resolver_exact_role_name_match_is_external() {
    let ctx = ctx_with_roles(&["traefik"]);
    let ns = infer_ansible_external("traefik", Some(&ctx));
    assert_eq!(ns.as_deref(), Some("ansible.traefik"));
}

#[test]
fn jinja_resolver_no_false_positive_on_prefix_without_underscore() {
    // `docker` role must NOT match `dockerfile_path` — the name segment boundary
    // is enforced by requiring `<role>_`.
    let ctx = ctx_with_roles(&["docker"]);
    let ns = infer_ansible_external("dockerfile_path", Some(&ctx));
    // `dockerfile_path` does NOT start with `docker_`, so no match.
    assert!(ns.is_none());
}

#[test]
fn jinja_resolver_local_role_var_returns_none() {
    // A var whose role IS declared externally but the var prefix actually matches
    // the local role name — this should still be classified external.
    let ctx = ctx_with_roles(&["systemd_docker_base", "traefik"]);
    let ns = infer_ansible_external("traefik_enabled", Some(&ctx));
    assert_eq!(ns.as_deref(), Some("ansible.traefik"));
}

#[test]
fn jinja_resolver_no_match_returns_none() {
    let ctx = ctx_with_roles(&["systemd_docker_base"]);
    let ns = infer_ansible_external("matrix_base_enabled", Some(&ctx));
    assert!(ns.is_none());
}

#[test]
fn jinja_resolver_no_project_ctx_returns_none() {
    let ns = infer_ansible_external("systemd_docker_base_enabled", None);
    assert!(ns.is_none());
}

#[test]
fn jinja_resolver_empty_manifest_returns_none() {
    let ctx = ctx_with_roles(&[]);
    let ns = infer_ansible_external("anything", Some(&ctx));
    assert!(ns.is_none());
}
