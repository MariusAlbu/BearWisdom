// =============================================================================
// engine/testkit/builders.rs — fluent fixture builders for the synthetic Lookup
// =============================================================================

use super::*;

impl Lookup {
    pub(crate) fn new() -> Self {
        Self {
            generics_of: FxHashMap::default(),
            parent_arg_ids_of: FxHashMap::default(),
            empty: Vec::new(),
            empty_pairs: Vec::new(),
            by_name: Default::default(),
            by_qname: Default::default(),
            by_qname_all: Default::default(),
            by_id: Default::default(),
            members: Default::default(),
            members_by_id: Default::default(),
            generics: Default::default(),
            field_types: Default::default(),
            field_type_ids: Default::default(),
            return_types: Default::default(),
            return_types_by_id: Default::default(),
            parents: Default::default(),
            parents_by_id: Default::default(),
            inherits_args: Default::default(),
            local_types: Default::default(),
            implicit_namespaces: Vec::new(),
            local_callable_heads: Default::default(),
            enclosing: Default::default(),
            aliases: Default::default(),
            aliases_by_id: Default::default(),
            reexports: Default::default(),
            ambient: Default::default(),
            by_package: Default::default(),
            workspace_pkgs: Default::default(),
            arena: TypeArena::new(),
        }
    }

    /// Register a symbol under both its simple name and its qualified name.
    pub(crate) fn with(mut self, sym: Symbol) -> Self {
        self.by_name
            .entry(sym.name.clone())
            .or_default()
            .push(sym.clone());
        self.by_qname_all
            .entry(sym.qualified_name.clone())
            .or_default()
            .push(sym.clone());
        self.by_id.insert(sym.id, sym.clone());
        self.by_qname.insert(sym.qualified_name.clone(), sym);
        self
    }

    /// Register a member symbol under `parent_qname`.
    pub(crate) fn with_member(mut self, parent_qname: &str, sym: Symbol) -> Self {
        self.members
            .entry(parent_qname.to_string())
            .or_default()
            .push(sym);
        self
    }

    /// Register a symbol that belongs to a workspace package. Indexes it under
    /// its simple/qualified name like `with`, AND records it under `package_id`
    /// (with `package_id` stamped on the row) so `symbols_in_package` and the
    /// import-scoped candidate pick can find it.
    pub(crate) fn with_in_package(mut self, package_id: i64, mut sym: Symbol) -> Self {
        sym.package_id = Some(package_id);
        self = self.with(sym.clone());
        self.by_package.entry(package_id).or_default().push(sym);
        self
    }

    /// Map a workspace-package specifier to its package id, backing
    /// `workspace_package_id` / `is_workspace_declared_name`.
    pub(crate) fn with_workspace_pkg(mut self, specifier: &str, package_id: i64) -> Self {
        self.workspace_pkgs.insert(specifier.to_string(), package_id);
        self
    }

    /// Register a member symbol under its parent's SYMBOL ID — the id-keyed
    /// counterpart of `with_member`, for tests that exercise the chain walker's
    /// identity path (two same-qname parents kept distinct by id).
    pub(crate) fn with_member_id(mut self, parent_id: i64, sym: Symbol) -> Self {
        self.members_by_id.entry(parent_id).or_default().push(sym);
        self
    }

    /// Register `child`'s direct parent by SYMBOL ID — the id-keyed counterpart
    /// of `with_parent`, driving the id-keyed supertype climb.
    pub(crate) fn with_generics_of(mut self, symbol_id: i64, params: &[&str]) -> Self {
        self.generics_of
            .insert(symbol_id, params.iter().map(|p| p.to_string()).collect());
        self
    }

    pub(crate) fn with_parent_arg_ids_of(
        mut self,
        child_id: i64,
        parent_id: i64,
        args: &[TypeId],
    ) -> Self {
        self.parent_arg_ids_of
            .insert((child_id, parent_id), args.to_vec());
        self
    }

    pub(crate) fn with_parent_id(mut self, child_id: i64, parent_id: i64) -> Self {
        self.parents_by_id.entry(child_id).or_default().push(parent_id);
        self
    }

    /// Register generic parameter names for a type qname.
    pub(crate) fn with_generics(mut self, qname: &str, params: &[&str]) -> Self {
        self.generics.insert(
            qname.to_string(),
            params.iter().map(|s| s.to_string()).collect(),
        );
        self
    }

    /// Register the declared type of a field/property qname.
    pub(crate) fn with_field_type(mut self, qname: &str, ty: &str) -> Self {
        self.field_types.insert(qname.to_string(), ty.to_string());
        self
    }

    /// Register a qname's field type as an already-interned TypeId — the form
    /// Phase A records for a declared annotation (`Ctor: typeof C`).
    pub(crate) fn with_field_type_id(mut self, qname: &str, id: TypeId) -> Self {
        self.field_type_ids.insert(qname.to_string(), id);
        self
    }

    /// Register the return type of a method/function qname.
    pub(crate) fn with_return_type(mut self, qname: &str, ty: &str) -> Self {
        self.return_types.insert(qname.to_string(), ty.to_string());
        self
    }

    /// Register a return type under the symbol's ID — the collision-free
    /// counterpart of `with_return_type`, for same-qname overload rows whose
    /// yields differ.
    pub(crate) fn with_return_type_of(mut self, id: i64, ty: &str) -> Self {
        self.return_types_by_id.insert(id, ty.to_string());
        self
    }

    /// Register `child`'s direct parent type qname (an `extends`/`implements` link).
    pub(crate) fn with_parent(mut self, child_qname: &str, parent_qname: &str) -> Self {
        self.parents
            .entry(child_qname.to_string())
            .or_default()
            .push(parent_qname.to_string());
        self
    }

    /// Register the generic args on `child`'s `extends`/`implements` edge to
    /// `parent_head`: `with_parent_args("Child", "Base", &["User"])` for
    /// `class Child extends Base<User>`. Backs `parent_class_args`.
    pub(crate) fn with_parent_args(mut self, child_head: &str, parent_head: &str, args: &[&str]) -> Self {
        self.inherits_args.insert(
            (child_head.to_string(), parent_head.to_string()),
            args.iter().map(|s| s.to_string()).collect(),
        );
        self
    }

    /// Register a local variable's forward-inferred type.
    pub(crate) fn with_local_type(mut self, name: &str, ty: &str) -> Self {
        self.local_types.insert(name.to_string(), ty.to_string());
        self
    }

    /// Register manifest-declared implicit namespace imports (the
    /// `<ImplicitUsings>` set), backing `implicit_wildcard_namespaces`.
    pub(crate) fn with_implicit_namespaces(mut self, namespaces: &[&str]) -> Self {
        self.implicit_namespaces = namespaces.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Register a local binding's callable-head pointer — the qname a
    /// destructured `$Ret` member's own declaration names, backing
    /// `local_callable_head`.
    pub(crate) fn with_local_callable_head(mut self, name: &str, qname: &str) -> Self {
        self.local_callable_heads
            .insert(name.to_string(), qname.to_string());
        self
    }

    /// Register the enclosing type qname for a source symbol qname.
    pub(crate) fn with_enclosing(mut self, source_qname: &str, type_qname: &str) -> Self {
        self.enclosing
            .insert(source_qname.to_string(), type_qname.to_string());
        self
    }

    /// Register a type alias's target. Accepts [`AliasTarget`] (the
    /// source-name form) and converts to [`AliasTargetIds`] at insert time,
    /// matching the behavior of the Compilation map so tests exercise the same
    /// id-keyed path the production engine uses.
    pub(crate) fn with_alias(mut self, name: &str, target: AliasTarget) -> Self {
        let interned = intern_alias_target(&self.arena, &target);
        self.aliases.insert(name.to_string(), interned);
        self
    }

    /// Register an alias target keyed by the declaration's symbol id — the
    /// collision-free path a use site uses when a bare name has several aliases.
    pub(crate) fn with_alias_id(mut self, id: i64, target: AliasTarget) -> Self {
        let interned = intern_alias_target(&self.arena, &target);
        self.aliases_by_id.insert(id, interned);
        self
    }

    /// Register a re-export entry from `file`: `original_name` (`"*"` for a
    /// wildcard `export * from 'module'`) re-exported from `module`.
    pub(crate) fn with_reexport(mut self, file: &str, original_name: &str, module: &str) -> Self {
        self.reexports
            .entry(file.to_string())
            .or_default()
            .push((original_name.to_string(), module.to_string()));
        self
    }

    /// Register a symbol as a member of ambient scope, keyed by its simple name.
    pub(crate) fn with_ambient(mut self, sym: Symbol) -> Self {
        self.ambient
            .entry(sym.name.clone())
            .or_default()
            .push(sym);
        self
    }
}
