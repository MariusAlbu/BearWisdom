// =============================================================================
// type_checker/type_env_tests.rs — TypeEnvironment unit tests
// =============================================================================

use super::*;

#[test]
fn test_new_has_one_scope() {
    let env = TypeEnvironment::new();
    assert!(!env.is_bound("T"));
}

#[test]
fn test_bind_and_resolve() {
    let mut env = TypeEnvironment::new();
    env.bind("T", "User");
    assert_eq!(env.resolve("T"), "User");
    assert_eq!(env.resolve("U"), "U"); // unbound → identity
}

#[test]
fn test_bind_params() {
    let mut env = TypeEnvironment::new();
    env.bind_params(
        &["K".to_string(), "V".to_string()],
        &["string".to_string(), "Handler".to_string()],
    );
    assert_eq!(env.resolve("K"), "string");
    assert_eq!(env.resolve("V"), "Handler");
    assert_eq!(env.resolve("T"), "T"); // unbound
}

#[test]
fn test_scope_shadowing() {
    let mut env = TypeEnvironment::new();
    env.bind("T", "User");

    env.push_scope();
    env.bind("T", "Order"); // shadows outer T

    assert_eq!(env.resolve("T"), "Order");

    env.pop_scope();
    assert_eq!(env.resolve("T"), "User"); // outer T restored
}

#[test]
fn test_inner_scope_sees_outer_bindings() {
    let mut env = TypeEnvironment::new();
    env.bind("T", "User");

    env.push_scope();
    env.bind("E", "Error");

    assert_eq!(env.resolve("T"), "User"); // visible from inner scope
    assert_eq!(env.resolve("E"), "Error");

    env.pop_scope();
    assert_eq!(env.resolve("T"), "User");
    assert!(!env.is_bound("E")); // gone after pop
}

#[test]
fn test_pop_scope_never_pops_root() {
    let mut env = TypeEnvironment::new();
    env.bind("T", "User");
    env.pop_scope(); // at root — should be a no-op
    env.pop_scope(); // again — still safe
    // The root binding should still be there.
    assert_eq!(env.resolve("T"), "User");
}

#[test]
fn test_is_bound() {
    let mut env = TypeEnvironment::new();
    assert!(!env.is_bound("T"));
    env.bind("T", "User");
    assert!(env.is_bound("T"));
    assert!(!env.is_bound("E"));
}

#[test]
fn test_enter_generic_context_no_params() {
    let mut env = TypeEnvironment::new();
    // If params_lookup returns None or empty, no scope is pushed.
    let pushed = env.enter_generic_context("String", &["User".to_string()], |_| None);
    assert!(!pushed);
    // Still at root level — resolve is identity.
    assert_eq!(env.resolve("T"), "T");
}

#[test]
fn test_enter_generic_context_binds_params() {
    let mut env = TypeEnvironment::new();
    let pushed = env.enter_generic_context(
        "Repository",
        &["User".to_string()],
        |name| {
            if name == "Repository" {
                Some(vec!["T".to_string()])
            } else {
                None
            }
        },
    );
    assert!(pushed);
    assert_eq!(env.resolve("T"), "User");
    env.pop_scope();
    assert!(!env.is_bound("T")); // T gone after pop
}

#[test]
fn test_nested_generic_context() {
    // Simulates: repo: Repository<Map<string, User>>
    // Repository<T> with T=Map<string, User>
    // Map<K, V> with K=string, V=User
    let mut env = TypeEnvironment::new();

    // Enter Repository<Map<string, User>>
    let pushed1 = env.enter_generic_context(
        "Repository",
        &["Map<string, User>".to_string()],
        |name| {
            if name == "Repository" {
                Some(vec!["T".to_string()])
            } else {
                None
            }
        },
    );
    assert!(pushed1);
    assert_eq!(env.resolve("T"), "Map<string, User>");

    // Enter Map<string, User>
    let pushed2 = env.enter_generic_context(
        "Map",
        &["string".to_string(), "User".to_string()],
        |name| {
            if name == "Map" {
                Some(vec!["K".to_string(), "V".to_string()])
            } else {
                None
            }
        },
    );
    assert!(pushed2);
    assert_eq!(env.resolve("K"), "string");
    assert_eq!(env.resolve("V"), "User");
    // T is still visible from outer scope.
    assert_eq!(env.resolve("T"), "Map<string, User>");

    env.pop_scope(); // leave Map context
    assert!(!env.is_bound("K"));
    assert_eq!(env.resolve("T"), "Map<string, User>"); // still bound

    env.pop_scope(); // leave Repository context
    assert!(!env.is_bound("T"));
}

#[test]
fn test_multi_param_partial_args() {
    // Fewer args than params — only bind what we have.
    let mut env = TypeEnvironment::new();
    env.bind_params(
        &["T".to_string(), "E".to_string(), "R".to_string()],
        &["User".to_string(), "Error".to_string()],
        // Only 2 args for 3 params — R stays unbound.
    );
    assert_eq!(env.resolve("T"), "User");
    assert_eq!(env.resolve("E"), "Error");
    assert_eq!(env.resolve("R"), "R"); // no arg provided → identity
}

#[test]
fn test_resolve_unbound_returns_identity() {
    let env = TypeEnvironment::new();
    assert_eq!(env.resolve("SomeConcreteType"), "SomeConcreteType");
    assert_eq!(env.resolve("T"), "T");
    assert_eq!(env.resolve(""), "");
}
