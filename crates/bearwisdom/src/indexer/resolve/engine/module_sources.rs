//! Lower exact source locations and configured dependencies to numeric modules.
use super::*;
use crate::ecosystem::manifest::module_config::{ModulePackage, TargetKind};

#[derive(Default)]
pub(super) struct SourceLinks {
    pub(super) providers: FxHashMap<String, ModuleId>,
    pub(super) redirects: FxHashMap<ModuleId, ModuleId>,
    files: FxHashMap<(ModuleId, usize), Target>,
    parents: FxHashMap<ModuleId, Target>,
    roots: FxHashMap<ModuleId, Target>,
    externals: FxHashMap<(ModuleId, ExportNameId), Target>,
}

impl SourceLinks {
    pub(super) fn parent_module(&self, module: ModuleId) -> Option<ModuleId> {
        match self.parents.get(&module) {
            Some(Target::Namespace(parent)) => Some(*parent),
            _ => None,
        }
    }
    pub(super) fn lower(
        &self,
        root: ModuleId,
        target: &InputTarget,
        names: &mut FxHashMap<String, ExportNameId>,
        units: &FxHashMap<(ModuleId, SourceModuleId), ModuleId>,
    ) -> Target {
        match target {
            InputTarget::SourceFile(id) => self
                .files
                .get(&(root, *id))
                .copied()
                .unwrap_or(Target::Incomplete),
            InputTarget::CrateRoot => self
                .roots
                .get(&root)
                .copied()
                .unwrap_or(Target::Unconfigured),
            InputTarget::ExternalRoot(name) => self
                .externals
                .get(&(root, intern(names, name)))
                .copied()
                .unwrap_or(Target::Unconfigured),
            InputTarget::Parent(unit, count) => {
                let Some(mut module) = units.get(&(root, *unit)).copied() else {
                    return Target::Missing;
                };
                for _ in 0..*count {
                    match self.parents.get(&module).copied() {
                        Some(Target::Namespace(parent)) => module = parent,
                        Some(failure) => return failure,
                        None => {
                            return if self.roots.contains_key(&root) {
                                Target::Missing
                            } else {
                                Target::Unconfigured
                            }
                        }
                    }
                }
                Target::Namespace(module)
            }
            _ => Target::Missing,
        }
    }
}

impl ModuleGraph {
    pub(in crate::indexer::resolve::engine) fn snapshot_configuration(
        &mut self,
        ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        self.programs.configuration = ctx.programs.clone();
        self.configuration = Some(
            ctx.manifests
                .values()
                .flat_map(|m| m.module_packages.iter().cloned())
                .collect(),
        );
    }

    pub(super) fn source_links(&mut self) -> SourceLinks {
        let packages = self.configuration.as_deref().unwrap_or(&[]);
        let configured: FxHashSet<_> = packages
            .iter()
            .flat_map(|p| p.targets.iter().map(|t| join(&p.root, &t.path)))
            .collect();
        let mut links = SourceLinks::default();
        let mut children: FxHashMap<ModuleId, Vec<ModuleId>> = FxHashMap::default();
        let mut owners: FxHashMap<ModuleId, Vec<(ModuleId, usize, ModuleId)>> =
            FxHashMap::default();
        for (key, input) in &self.inputs {
            if module_paths::normalize(key) != module_paths::normalize(&input.path) {
                continue;
            }
            let Some(&root) = self.paths.get(&input.path) else {
                continue;
            };
            for unit in &input.units {
                if let (Some(&id), Some(&parent)) = (
                    self.units.get(&(root, unit.id)),
                    self.units.get(&(root, unit.parent)),
                ) {
                    links.parents.insert(id, Target::Namespace(parent));
                    children.entry(parent).or_default().push(id);
                }
            }
            for (index, file) in input.source_files.iter().enumerate() {
                let candidates = candidates(input, file, configured.contains(&input.path));
                let found: Vec<_> = candidates
                    .iter()
                    .filter_map(|p| self.paths.get(p).copied())
                    .collect();
                let target = match found.as_slice() {
                    [id] => Target::Namespace(*id),
                    [] => Target::Incomplete,
                    _ => Target::Ambiguous,
                };
                links.files.insert((root, index), target);
                if let (Target::Namespace(child), Some(&parent)) =
                    (target, self.units.get(&(root, file.owner)))
                {
                    owners.entry(child).or_default().push((root, index, parent));
                }
            }
        }
        // One physical file is not yet several module instances. Conflicting
        // declarations (including a crate root used as a child) must abstain.
        let root_ids: FxHashSet<_> = configured
            .iter()
            .filter_map(|p| self.paths.get(p).copied())
            .collect();
        for (child, incoming) in owners {
            if incoming.len() != 1 || root_ids.contains(&child) {
                links.parents.insert(child, Target::Ambiguous);
                for (root, index, _) in incoming {
                    links.files.insert((root, index), Target::Ambiguous);
                }
            } else {
                let (_, _, parent) = incoming[0];
                links.parents.insert(child, Target::Namespace(parent));
                children.entry(parent).or_default().push(child);
            }
        }
        install_roots(
            packages,
            &self.paths,
            &children,
            &mut self.names,
            &mut links,
        );
        let invalid: FxHashMap<_, _> = links
            .roots
            .iter()
            .filter_map(|(&id, &target)| {
                matches!(target, Target::Ambiguous | Target::Incomplete).then_some((id, target))
            })
            .collect();
        for target in links
            .files
            .values_mut()
            .chain(links.externals.values_mut())
            .chain(links.parents.values_mut())
        {
            if let Target::Namespace(module) = *target {
                if let Some(&failure) = invalid.get(&module) {
                    *target = failure;
                }
            }
        }
        (links.providers, links.redirects) = self.install_provider_groups();
        links
    }
}

fn install_roots(
    packages: &[ModulePackage],
    paths: &FxHashMap<String, ModuleId>,
    children: &FxHashMap<ModuleId, Vec<ModuleId>>,
    names: &mut FxHashMap<String, ExportNameId>,
    links: &mut SourceLinks,
) {
    let mut seen: FxHashMap<ModuleId, usize> = FxHashMap::default();
    let mut instance = 0;
    for package in packages {
        for target in &package.targets {
            instance += 1;
            let Some(&root) = paths.get(&join(&package.root, &target.path)) else {
                continue;
            };
            let mut pending = vec![root];
            let mut visited = FxHashSet::default();
            let imports = configured_imports(package, target.kind, packages, paths, names);
            while let Some(module) = pending.pop() {
                if !visited.insert(module) {
                    continue;
                }
                let conflict = seen.insert(module, instance).is_some();
                links.roots.insert(
                    module,
                    if conflict {
                        Target::Ambiguous
                    } else if target.conditional {
                        Target::Incomplete
                    } else {
                        Target::Namespace(root)
                    },
                );
                if conflict {
                    for ((owner, _), value) in links.externals.iter_mut() {
                        if *owner == module {
                            *value = Target::Ambiguous;
                        }
                    }
                }
                for &(name, value) in &imports {
                    links.externals.insert(
                        (module, name),
                        if conflict {
                            Target::Ambiguous
                        } else if target.conditional {
                            Target::Incomplete
                        } else {
                            value
                        },
                    );
                }
                if let Some(children) = children.get(&module) {
                    pending.extend(children);
                }
            }
        }
    }
}

fn configured_imports(
    package: &ModulePackage,
    kind: TargetKind,
    packages: &[ModulePackage],
    paths: &FxHashMap<String, ModuleId>,
    names: &mut FxHashMap<String, ExportNameId>,
) -> Vec<(ExportNameId, Target)> {
    let mut imports = FxHashMap::default();
    let mut denied = Vec::new();
    for target in package
        .targets
        .iter()
        .filter(|t| t.kind == TargetKind::Library)
    {
        let name = intern(names, &target.name);
        if matches!(kind, TargetKind::Executable | TargetKind::Development) {
            add_import(&mut imports, name, root_target(package, target, paths));
        } else {
            denied.push(name);
        }
    }
    for dep in &package.dependencies {
        let allowed = match dep.kind {
            TargetKind::Build => kind == TargetKind::Build,
            TargetKind::Development => kind == TargetKind::Development,
            _ => kind != TargetKind::Build,
        };
        // A registry name/version without a materialized manifest is still on the
        // legacy transition path. A declared path, however, is authoritative.
        let Some(dep_root) = &dep.root else {
            continue;
        };
        let candidates: Vec<_> = packages
            .iter()
            .filter(|p| {
                module_paths::normalize(&p.root) == module_paths::normalize(dep_root)
                    && p.name == dep.package
            })
            .collect();
        if !allowed {
            denied.push(intern(names, &dep.alias));
            if !dep.renamed {
                denied.extend(
                    candidates
                        .iter()
                        .flat_map(|p| p.targets.iter().filter(|t| t.kind == TargetKind::Library))
                        .map(|t| intern(names, &t.name)),
                );
            }
            continue;
        }
        if candidates.len() != 1 {
            add_import(
                &mut imports,
                intern(names, &dep.alias),
                if candidates.is_empty() {
                    Target::Incomplete
                } else {
                    Target::Ambiguous
                },
            );
            continue;
        }
        let provider = candidates[0];
        let libraries: Vec<_> = provider
            .targets
            .iter()
            .filter(|t| t.kind == TargetKind::Library)
            .collect();
        if libraries.is_empty() {
            add_import(&mut imports, intern(names, &dep.alias), Target::Incomplete);
        }
        for library in libraries {
            let alias = if dep.renamed {
                &dep.alias
            } else {
                &library.name
            };
            add_import(
                &mut imports,
                intern(names, alias),
                if dep.conditional {
                    Target::Incomplete
                } else {
                    root_target(provider, library, paths)
                },
            );
        }
    }
    for name in denied {
        imports.entry(name).or_insert(Target::Missing);
    }
    imports.into_iter().collect()
}

fn add_import(imports: &mut FxHashMap<ExportNameId, Target>, name: ExportNameId, value: Target) {
    imports
        .entry(name)
        .and_modify(|current| {
            if *current != value {
                *current = Target::Ambiguous;
            }
        })
        .or_insert(value);
}

fn root_target(
    package: &ModulePackage,
    target: &crate::ecosystem::manifest::module_config::ModuleTarget,
    paths: &FxHashMap<String, ModuleId>,
) -> Target {
    if target.conditional {
        return Target::Incomplete;
    }
    paths
        .get(&join(&package.root, &target.path))
        .copied()
        .map(Target::Namespace)
        .unwrap_or(Target::Incomplete)
}

fn candidates(
    input: &ModuleInput,
    file: &super::super::module_input::InputSourceFile,
    is_root: bool,
) -> Vec<String> {
    let Some(layout) = &input.file_layout else {
        return Vec::new();
    };
    let Some(directory) = directory(input, file.owner, is_root) else {
        return Vec::new();
    };
    if let Some(path) = &file.path {
        let base = if file.owner == SourceModuleId(0) {
            parent(&input.path)
        } else {
            &directory
        };
        return vec![join(base, path)];
    }
    let base = join(&directory, &file.name);
    vec![
        format!("{base}{}", layout.extension),
        join(&base, &layout.directory_entry),
    ]
}

fn directory(input: &ModuleInput, mut unit: SourceModuleId, is_root: bool) -> Option<String> {
    let layout = input.file_layout.as_ref()?;
    let basename = input.path.rsplit('/').next()?;
    let mut base: String = if is_root || basename == layout.directory_entry {
        parent(&input.path).into()
    } else {
        input.path.strip_suffix(&layout.extension)?.into()
    };
    let mut ancestry = Vec::new();
    let mut visited = FxHashSet::default();
    while unit != SourceModuleId(0) {
        if !visited.insert(unit) {
            return None;
        }
        let data = input.units.iter().find(|u| u.id == unit)?;
        ancestry.push(data);
        unit = data.parent;
    }
    for unit in ancestry.into_iter().rev() {
        base = if let Some(path) = &unit.source_path {
            join(
                if unit.parent == SourceModuleId(0) {
                    parent(&input.path)
                } else {
                    &base
                },
                path,
            )
        } else {
            join(&base, unit.source_name.as_deref()?)
        };
    }
    Some(base)
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}
fn join(base: &str, path: &str) -> String {
    if base.is_empty() || std::path::Path::new(path).is_absolute() {
        module_paths::normalize(path)
    } else {
        module_paths::normalize(&format!("{base}/{path}"))
    }
}

#[cfg(test)]
#[path = "module_sources_tests.rs"]
mod tests;
