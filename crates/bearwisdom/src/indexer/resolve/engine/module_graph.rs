//! Frozen ID-only export graph. Strings are consumed only while rebuilding it.
use super::{
    contract::SymbolLookup,
    module_input::{ExportDomain, InputTarget, ModuleInput, SourceModuleId},
    module_paths,
};
use crate::indexer::lexical::BindingId;
pub(super) use access::Site as ModuleSite;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ModuleId(usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ExportNameId(usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BindingResult {
    Missing,
    Bound(i64),
    Namespace(ModuleId),
    Overloads(usize),
    Entity {
        declaration: Option<i64>,
        overloads: Option<usize>,
        namespace: ModuleId,
    },
    Ambiguous,
    Incomplete,
    Unconfigured,
}
impl BindingResult {
    pub(super) fn declaration(self) -> Option<i64> {
        match self {
            Self::Bound(id) => Some(id),
            Self::Entity { declaration, .. } => declaration,
            _ => None,
        }
    }
    pub(super) fn namespace(self) -> Option<ModuleId> {
        match self {
            Self::Namespace(id) | Self::Entity { namespace: id, .. } => Some(id),
            _ => None,
        }
    }
    fn with_competitor(self, competing: bool) -> Self {
        if competing && matches!(self, Self::Missing | Self::Unconfigured) {
            Self::Incomplete
        } else {
            self
        }
    }
    fn merge(self, other: Self) -> Self {
        use BindingResult::*;
        match (self, other) {
            (Incomplete, _) | (_, Incomplete) => Incomplete,
            (Ambiguous, _) | (_, Ambiguous) => Ambiguous,
            (Unconfigured, Missing | Unconfigured) | (Missing, Unconfigured) => Unconfigured,
            (Unconfigured, _) | (_, Unconfigured) => Incomplete,
            (Missing, value) | (value, Missing) => value,
            (Bound(left), Bound(right)) if left == right => Bound(left),
            (Overloads(left), Overloads(right)) if left == right => Overloads(left),
            (Namespace(left), Namespace(right)) if left == right => Namespace(left),
            (left @ Entity { .. }, right @ Entity { .. }) if left == right => left,
            _ => Ambiguous,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Target {
    Declaration(i64),
    Namespace(ModuleId),
    Assigned(ModuleId),
    Entity(usize),
    Path(usize),
    ScopePath(usize),
    Access(usize),
    Overloads(usize),
    Export(ModuleId, ExportNameId),
    Binding(ModuleId, BindingId, ExportDomain),
    Missing,
    Incomplete,
    Ambiguous,
    Unconfigured,
}
#[derive(Default)]
struct TargetPool {
    entities: Vec<(Target, Target)>,
    keys: FxHashMap<Vec<i64>, usize>,
    groups: Vec<Vec<i64>>,
    paths: Vec<(Target, Vec<Selector>, Option<ModuleId>)>,
    path_keys: FxHashMap<(Target, Vec<Selector>, Option<ModuleId>), usize>,
    guards: Vec<access::Guard>,
    scope_paths: Vec<(Target, Vec<ExportNameId>)>,
    scope_keys: FxHashMap<(Target, Vec<ExportNameId>), usize>,
    scope_ids: Vec<Option<ModuleId>>,
}
type Selector = (ExportNameId, Option<ExportDomain>);
#[derive(Default)]
struct Module {
    assignments: FxHashMap<ExportDomain, Vec<Target>>,
    assignment_parts: Vec<ModuleId>,
    incomplete: bool,
    exports: FxHashMap<(ExportNameId, ExportDomain), Vec<Target>>,
    imports: FxHashMap<(BindingId, ExportDomain), Vec<Target>>,
    stars: Vec<(Target, ExportDomain)>,
    wildcard_exclusions: FxHashSet<ExportNameId>,
    parent: Option<ModuleId>,
}
#[derive(Default)]
pub(super) struct ModuleGraph {
    pub programs: super::program_graph::Graph,
    pub traits: super::trait_graph::Graph,
    /// Durable ingestion recipes; never inspected by the semantic traversal.
    pub inputs: BTreeMap<String, ModuleInput>,
    configuration: Option<Vec<crate::ecosystem::manifest::module_config::ModulePackage>>,
    providers: BTreeMap<String, program_modules::Provider>,
    augmentations: BTreeMap<String, program_modules::Provider>,
    paths: FxHashMap<String, ModuleId>,
    names: FxHashMap<String, ExportNameId>,
    modules: Vec<Module>,
    bindings: FxHashMap<(ModuleId, BindingId, ExportDomain), BindingResult>,
    units: FxHashMap<(ModuleId, SourceModuleId), ModuleId>,
    targets: TargetPool,
    visibility: FxHashMap<access::Entity, access::Scope>,
    declaration_origins: FxHashMap<i64, Option<ModuleId>>,
}

impl ModuleGraph {
    /// Ingestion boundary: translate a source name arena once, then select by ID.
    pub(super) fn export_name(&self, spelling: &str) -> Option<ExportNameId> {
        self.names.get(spelling).copied()
    }
    pub(super) fn select_export(&self, owner: ModuleId, name: ExportNameId) -> BindingResult {
        self.select_export_in(owner, name, ExportDomain::Value)
    }
    pub(super) fn select_export_in(
        &self,
        owner: ModuleId,
        name: ExportNameId,
        domain: ExportDomain,
    ) -> BindingResult {
        self.resolve_target(Target::Export(owner, name), domain, 0)
    }
    /// File-environment construction boundary. Reference resolution reads the
    /// installed BindingId facts, not this path table or export spellings.
    pub(super) fn binding(
        &self,
        file: &str,
        binding: BindingId,
        type_space: bool,
    ) -> BindingResult {
        self.binding_in(file, SourceModuleId(0), binding, type_space.into())
    }

    pub(super) fn binding_in(
        &self,
        file: &str,
        unit: SourceModuleId,
        binding: BindingId,
        domain: ExportDomain,
    ) -> BindingResult {
        self.paths
            .get(&module_paths::normalize(file))
            .and_then(|root| self.units.get(&(*root, unit)))
            .and_then(|module| self.bindings.get(&(*module, binding, domain)))
            .copied()
            .unwrap_or(BindingResult::Missing)
    }

    pub(super) fn overloads(&self, file: &str, binding: BindingId) -> &[i64] {
        match self.binding(file, binding, false) {
            BindingResult::Overloads(id)
            | BindingResult::Entity {
                overloads: Some(id),
                ..
            } => &self.targets.groups[id],
            _ => &[],
        }
    }

    pub(super) fn persist(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        self.programs.persist(conn)?;
        let config = serde_json::to_string(&self.configuration)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("INSERT OR REPLACE INTO _bearwisdom_meta (key,value) VALUES ('module_configuration_v1',?1)", [config])?;
        let payload = serde_json::to_string(&self.inputs.values().collect::<Vec<_>>())
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO _bearwisdom_meta (key,value) VALUES ('module_bindings_v1',?1)",
            [payload],
        )?;
        Ok(())
    }

    pub(super) fn load(&mut self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        self.programs.load(conn)?;
        use rusqlite::OptionalExtension;
        if self.configuration.is_none() {
            let config: Option<String> = conn
                .query_row(
                    "SELECT value FROM _bearwisdom_meta WHERE key='module_configuration_v1'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(config) = config {
                self.configuration = serde_json::from_str(&config).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
            }
        }
        let payload: Option<String> = conn
            .query_row(
                "SELECT value FROM _bearwisdom_meta WHERE key='module_bindings_v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(());
        };
        let stored: Vec<ModuleInput> = serde_json::from_str(&payload).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let mut statement = conn.prepare("SELECT path,hash FROM files")?;
        let live: FxHashSet<(String, String)> = statement
            .query_map([], |r| {
                Ok((module_paths::normalize(&r.get::<_, String>(0)?), r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        // A source fingerprint fences stale BindingIds, even when a barrel has
        // no declaration rows. Freshly parsed module environments always win.
        for input in stored {
            if input.binding_epoch == super::module_input::BINDING_EPOCH
                && live.contains(&(input.path.clone(), input.content_hash.clone()))
            {
                self.inputs.entry(input.path.clone()).or_insert(input);
            }
        }
        Ok(())
    }

    pub(super) fn rebuild(&mut self, lookup: &dyn SymbolLookup) {
        self.programs.rebuild(&self.inputs, lookup);
        self.paths = self.source_paths();
        self.names.clear();
        self.modules.clear();
        self.units.clear();
        self.bindings.clear();
        self.visibility.clear();
        self.declaration_origins.clear();
        self.targets = TargetPool::default();
        self.allocate_units();
        let sources = self.source_links();
        for (key, input) in &self.inputs {
            if module_paths::normalize(key) != module_paths::normalize(&input.path) {
                continue;
            }
            let Some(&root) = self.paths.get(&module_paths::normalize(&input.path)) else {
                continue;
            };
            let mut module = Module {
                parent: sources.parent_module(root),
                incomplete: input.source_complete == Some(false),
                ..Default::default()
            };
            for (target, domain) in &input.assignments {
                module.assignments.entry(*domain).or_default().push(lower(
                    &mut self.names,
                    &self.paths,
                    input,
                    target,
                    lookup,
                    &mut self.targets,
                    &self.units,
                    &sources,
                ));
            }
            for access in &input.declaration_access {
                lower(
                    &mut self.names,
                    &self.paths,
                    input,
                    access,
                    lookup,
                    &mut self.targets,
                    &self.units,
                    &sources,
                );
            }
            for export in &input.exports {
                let name = intern(&mut self.names, &export.name);
                let target = lower(
                    &mut self.names,
                    &self.paths,
                    input,
                    &export.target,
                    lookup,
                    &mut self.targets,
                    &self.units,
                    &sources,
                );
                module
                    .exports
                    .entry((name, export.domain))
                    .or_default()
                    .push(target);
            }
            for import in &input.imports {
                module
                    .imports
                    .entry((BindingId(import.binding), import.domain))
                    .or_default()
                    .push(lower(
                        &mut self.names,
                        &self.paths,
                        input,
                        &import.target,
                        lookup,
                        &mut self.targets,
                        &self.units,
                        &sources,
                    ));
            }
            for (spec, type_only) in &input.stars {
                let target = link(
                    &self.paths,
                    input,
                    spec,
                    lookup,
                    &sources.providers,
                    &sources.redirects,
                )
                .map(Target::Namespace)
                .unwrap_or(Target::Missing);
                module.stars.push((target, ExportDomain::Type));
                module.stars.push((target, ExportDomain::ValueQuery));
                if !type_only {
                    module.stars.push((target, ExportDomain::Value));
                }
            }
            module.wildcard_exclusions = input
                .wildcard_exclusions
                .iter()
                .map(|name| intern(&mut self.names, name))
                .collect();
            self.modules[root.0] = module;
            for unit in &input.units {
                if unit.id == SourceModuleId(0) {
                    continue;
                }
                let Some(&id) = self.units.get(&(root, unit.id)) else {
                    continue;
                };
                let mut module = Module {
                    parent: self.units.get(&(root, unit.parent)).copied(),
                    incomplete: unit
                        .source_scope
                        .as_ref()
                        .is_some_and(|scope| !scope.complete),
                    ..Default::default()
                };
                for (target, domain) in &unit.assignments {
                    module.assignments.entry(*domain).or_default().push(lower(
                        &mut self.names,
                        &self.paths,
                        input,
                        target,
                        lookup,
                        &mut self.targets,
                        &self.units,
                        &sources,
                    ));
                }
                for export in &unit.exports {
                    let name = intern(&mut self.names, &export.name);
                    let target = lower(
                        &mut self.names,
                        &self.paths,
                        input,
                        &export.target,
                        lookup,
                        &mut self.targets,
                        &self.units,
                        &sources,
                    );
                    module
                        .exports
                        .entry((name, export.domain))
                        .or_default()
                        .push(target);
                }
                for import in &unit.imports {
                    module
                        .imports
                        .entry((BindingId(import.binding), import.domain))
                        .or_default()
                        .push(lower(
                            &mut self.names,
                            &self.paths,
                            input,
                            &import.target,
                            lookup,
                            &mut self.targets,
                            &self.units,
                            &sources,
                        ));
                }
                module.stars = unit
                    .stars
                    .iter()
                    .map(|(target, domain)| {
                        (
                            lower(
                                &mut self.names,
                                &self.paths,
                                input,
                                target,
                                lookup,
                                &mut self.targets,
                                &self.units,
                                &sources,
                            ),
                            *domain,
                        )
                    })
                    .collect();
                module.wildcard_exclusions = unit
                    .wildcard_exclusions
                    .iter()
                    .map(|name| intern(&mut self.names, name))
                    .collect();
                self.modules[id.0] = module;
            }
        }
        self.capture_visibility();
        let mut exports = FxHashMap::default();
        for (index, module) in self.modules.iter().enumerate() {
            for (&(binding, domain), targets) in &module.imports {
                let mut value = if module.incomplete {
                    BindingResult::Incomplete
                } else {
                    BindingResult::Missing
                };
                for &target in targets {
                    let candidate = *exports
                        .entry((target, domain))
                        .or_insert_with(|| self.resolve_target(target, domain, 0));
                    value = value.merge(candidate.with_competitor(targets.len() > 1));
                }
                self.bindings
                    .insert((ModuleId(index), binding, domain), value);
            }
        }
    }
}

#[path = "module_access.rs"]
mod access;
#[path = "module_sources.rs"]
mod source_ids;
#[path = "module_units.rs"]
mod unit_ids;
#[path = "module_walk.rs"]
mod walk;

#[path = "program_modules.rs"]
mod program_modules;

fn intern(names: &mut FxHashMap<String, ExportNameId>, name: &str) -> ExportNameId {
    let next = ExportNameId(names.len());
    *names.entry(name.to_owned()).or_insert(next)
}

fn lower(
    names: &mut FxHashMap<String, ExportNameId>,
    paths: &FxHashMap<String, ModuleId>,
    input: &ModuleInput,
    target: &InputTarget,
    lookup: &dyn SymbolLookup,
    targets: &mut TargetPool,
    units: &FxHashMap<(ModuleId, SourceModuleId), ModuleId>,
    sources: &source_ids::SourceLinks,
) -> Target {
    match target {
        InputTarget::Incomplete => Target::Incomplete,
        InputTarget::Assigned { module } => link(
            paths,
            input,
            module,
            lookup,
            &sources.providers,
            &sources.redirects,
        )
        .map(Target::Assigned)
        .unwrap_or(Target::Missing),
        InputTarget::Entity {
            declaration,
            namespace,
        } => {
            let declaration = lower(
                names,
                paths,
                input,
                declaration,
                lookup,
                targets,
                units,
                sources,
            );
            let namespace = lower(
                names, paths, input, namespace, lookup, targets, units, sources,
            );
            let id = targets.entities.len();
            targets.entities.push((declaration, namespace));
            Target::Entity(id)
        }
        InputTarget::Declaration(id) => lookup
            .symbol_by_id(*id)
            .map(|s| Target::Declaration(lookup.canonical_decl_id(s.id)))
            .unwrap_or(Target::Missing),
        InputTarget::Declarations(rows) => {
            super::lexical_type_ids::agreed_declaration(rows.iter().copied(), lookup)
                .map(Target::Declaration)
                .unwrap_or(Target::Missing)
        }
        InputTarget::From { module, name } => link(
            paths,
            input,
            module,
            lookup,
            &sources.providers,
            &sources.redirects,
        )
        .map(|module| Target::Export(module, intern(names, name)))
        .unwrap_or(Target::Missing),
        InputTarget::Namespace { module } => link(
            paths,
            input,
            module,
            lookup,
            &sources.providers,
            &sources.redirects,
        )
        .map(Target::Namespace)
        .unwrap_or(Target::Missing),
        InputTarget::LocalNamespace(unit) => local_unit(paths, input, units, *unit)
            .map(Target::Namespace)
            .unwrap_or(Target::Missing),
        InputTarget::Binding {
            module,
            binding,
            domain,
        } => local_unit(paths, input, units, *module)
            .map(|id| Target::Binding(id, BindingId(*binding), *domain))
            .unwrap_or(Target::Missing),
        InputTarget::LocalExport { module, name } => local_unit(paths, input, units, *module)
            .map(|id| Target::Export(id, intern(names, name)))
            .unwrap_or(Target::Missing),
        InputTarget::Path {
            base,
            names: selectors,
        } => {
            let base = lower(names, paths, input, base, lookup, targets, units, sources);
            let selectors = selectors
                .iter()
                .map(|name| (intern(names, name), None))
                .collect();
            intern_path(targets, base, selectors, None)
        }
        InputTarget::Select { base, selectors } => {
            let base = lower(names, paths, input, base, lookup, targets, units, sources);
            let selectors = selectors
                .iter()
                .map(|(name, domain)| (intern(names, name), Some(*domain)))
                .collect();
            intern_path(targets, base, selectors, None)
        }
        InputTarget::ContextPath {
            base,
            selectors,
            origin,
        } => {
            let Some(origin) = local_unit(paths, input, units, *origin) else {
                return Target::Missing;
            };
            let base = lower(names, paths, input, base, lookup, targets, units, sources);
            let selectors = selectors
                .iter()
                .map(|(name, domain)| (intern(names, name), Some(*domain)))
                .collect();
            intern_path(targets, base, selectors, Some(origin))
        }
        InputTarget::DeclarationPath {
            base,
            names: selectors,
        } => {
            let base = lower(names, paths, input, base, lookup, targets, units, sources);
            let key = (
                base,
                selectors.iter().map(|name| intern(names, name)).collect(),
            );
            let next = targets.scope_paths.len();
            let id = *targets.scope_keys.entry(key.clone()).or_insert_with(|| {
                targets.scope_paths.push(key);
                next
            });
            Target::ScopePath(id)
        }
        InputTarget::Access {
            target,
            scope,
            origin,
            declaration,
        } => {
            let Some(origin) = local_unit(paths, input, units, *origin) else {
                return Target::Missing;
            };
            let target = lower(names, paths, input, target, lookup, targets, units, sources);
            let scope = scope
                .as_ref()
                .map(|scope| lower(names, paths, input, scope, lookup, targets, units, sources));
            let id = targets.guards.len();
            targets.guards.push(access::Guard {
                target,
                scope,
                origin,
                declaration: *declaration,
            });
            Target::Access(id)
        }
        InputTarget::CrateRoot
        | InputTarget::ExternalRoot(_)
        | InputTarget::Parent(_, _)
        | InputTarget::SourceFile(_) => paths
            .get(&module_paths::normalize(&input.path))
            .map(|&root| sources.lower(root, target, names, units))
            .unwrap_or(Target::Missing),
        InputTarget::Missing => Target::Missing,
        InputTarget::Overloads(rows) => {
            let ids: Option<Vec<_>> = rows
                .iter()
                .map(|&id| {
                    lookup
                        .symbol_by_id(id)
                        .filter(|s| s.kind == "function")
                        .map(|s| lookup.canonical_decl_id(s.id))
                })
                .collect();
            let Some(mut ids) = ids.filter(|ids| !ids.is_empty()) else {
                return Target::Missing;
            };
            ids.sort_unstable();
            ids.dedup();
            let next = targets.groups.len();
            let group = *targets.keys.entry(ids.clone()).or_insert_with(|| {
                targets.groups.push(ids);
                next
            });
            Target::Overloads(group)
        }
    }
}

fn local_unit(
    paths: &FxHashMap<String, ModuleId>,
    input: &ModuleInput,
    units: &FxHashMap<(ModuleId, SourceModuleId), ModuleId>,
    unit: SourceModuleId,
) -> Option<ModuleId> {
    units
        .get(&(*paths.get(&module_paths::normalize(&input.path))?, unit))
        .copied()
}

fn intern_path(
    targets: &mut TargetPool,
    base: Target,
    selectors: Vec<Selector>,
    origin: Option<ModuleId>,
) -> Target {
    let key = (base, selectors, origin);
    let next = targets.paths.len();
    let id = *targets.path_keys.entry(key.clone()).or_insert_with(|| {
        targets.paths.push(key);
        next
    });
    Target::Path(id)
}

fn link(
    paths: &FxHashMap<String, ModuleId>,
    input: &ModuleInput,
    spec: &str,
    lookup: &dyn SymbolLookup,
    providers: &FxHashMap<String, ModuleId>,
    redirects: &FxHashMap<ModuleId, ModuleId>,
) -> Option<ModuleId> {
    let redirect = |module| redirects.get(&module).copied().unwrap_or(module);
    if let Some(&module) = providers.get(spec) {
        return Some(module);
    }
    if let Some(base) = module_paths::relative_base(&input.path, spec) {
        return module_paths::find(&base, &input.paths, |path| paths.get(path).copied())
            .map(redirect);
    }
    // Only configured package entries/path aliases are candidates. The global
    // declaration-name and file-suffix indexes are not module evidence.
    if let Some(path) = lookup.resolve_module_from(&input.path, spec) {
        return paths
            .get(&module_paths::normalize(path))
            .copied()
            .map(redirect);
    }
    let alias = lookup.resolve_path_alias(lookup.package_id_for_file(&input.path), spec)?;
    module_paths::find(&module_paths::normalize(&alias), &input.paths, |path| {
        paths.get(path).copied()
    })
    .map(redirect)
}

#[cfg(test)]
#[path = "module_graph_tests.rs"]
mod tests;
