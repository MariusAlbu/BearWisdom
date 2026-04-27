// =============================================================================
// type_checker/type_env.rs — Scoped generic type parameter environment
//
// Replaces ad-hoc `resolve_generic_type()` single-level lookups in chain
// resolvers with a proper scoped environment that handles nested generics,
// multi-param types, and chained generic propagation.
//
// Example: `repo: Repository<Map<string, User>>`
//   - Entering Repository<Map<string, User>>: bind T=Map<string, User>
//   - Following T to Map: push scope, bind K=string, V=User
//   - `map.get(k)` returns V → resolves to "User"
// =============================================================================

use rustc_hash::FxHashMap;

/// Maps generic type parameter names to their concrete types within a resolution context.
///
/// For `repo: Repository<User>` where `interface Repository<T>`:
///   env = { "T" => "User" }
///
/// For `map: Map<string, Handler>` where `interface Map<K, V>`:
///   env = { "K" => "string", "V" => "Handler" }
///
/// Scopes are stacked — inner scopes shadow outer ones.
#[derive(Debug, Default)]
pub struct TypeEnvironment {
    /// Stack of bindings — inner scopes shadow outer.
    /// Each entry maps param name → concrete type.
    bindings: Vec<FxHashMap<String, String>>,
}

impl TypeEnvironment {
    pub fn new() -> Self {
        Self {
            bindings: vec![FxHashMap::default()],
        }
    }

    /// Push a new scope (entering a generic context).
    pub fn push_scope(&mut self) {
        self.bindings.push(FxHashMap::default());
    }

    /// Pop a scope (leaving a generic context).
    pub fn pop_scope(&mut self) {
        // Never pop the root scope — that would leave bindings in an invalid state.
        if self.bindings.len() > 1 {
            self.bindings.pop();
        }
    }

    /// Bind a type parameter to a concrete type.
    pub fn bind(&mut self, param: &str, concrete: &str) {
        if let Some(scope) = self.bindings.last_mut() {
            scope.insert(param.to_string(), concrete.to_string());
        }
    }

    /// Bind multiple params from a type declaration to concrete args.
    /// params = ["T", "E"], args = ["User", "Error"] → T=User, E=Error
    pub fn bind_params(&mut self, params: &[String], args: &[String]) {
        for (param, arg) in params.iter().zip(args.iter()) {
            self.bind(param, arg);
        }
    }

    /// Resolve a type name through the environment.
    /// If `name` is a bound parameter, return its concrete type.
    /// Otherwise return the name unchanged.
    pub fn resolve(&self, name: &str) -> String {
        // Check scopes from innermost to outermost.
        for scope in self.bindings.iter().rev() {
            if let Some(concrete) = scope.get(name) {
                return concrete.clone();
            }
        }
        name.to_string()
    }

    /// Check if a name is a bound type parameter.
    pub fn is_bound(&self, name: &str) -> bool {
        self.bindings.iter().rev().any(|scope| scope.contains_key(name))
    }

    /// Enter a new generic context for `type_name` with the given concrete args.
    ///
    /// Looks up `type_name`'s declared parameter names via `params_lookup`, then
    /// pushes a new scope with each param bound to its corresponding concrete arg.
    /// Returns true if a new scope was pushed (i.e., params were found and bound).
    ///
    /// Caller must call `pop_scope()` when leaving this context.
    pub fn enter_generic_context(
        &mut self,
        type_name: &str,
        concrete_args: &[String],
        params_lookup: impl Fn(&str) -> Option<Vec<String>>,
    ) -> bool {
        if concrete_args.is_empty() {
            return false;
        }
        if let Some(params) = params_lookup(type_name) {
            if !params.is_empty() {
                self.push_scope();
                self.bind_params(&params, concrete_args);
                return true;
            }
        }
        false
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
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
}
