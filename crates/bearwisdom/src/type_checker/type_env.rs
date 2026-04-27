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

#[cfg(test)]
#[path = "type_env_tests.rs"]
mod tests;
