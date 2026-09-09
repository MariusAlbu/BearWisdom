//! Configured global environments. Runtime operations use snapshot-owned IDs;
//! physical declarations remain navigation targets, not workspace-wide owners.
use super::{contract::SymbolLookup, module_input::ModuleInput, module_paths};
use crate::indexer::{
    lexical::BindingId,
    programs::{Program, SourceScope},
};
use rustc_hash::FxHashMap;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

#[path = "program_merge_candidates.rs"]
pub(super) mod candidates;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ProgramId {
    snapshot: u64,
    index: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SourceInstanceId {
    program: ProgramId,
    index: usize,
}
impl SourceInstanceId {
    pub(super) fn ordinal(self) -> usize {
        self.index
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct GlobalNameId {
    program: ProgramId,
    index: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct GlobalBindingId {
    program: ProgramId,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Result {
    Missing,
    Bound(GlobalBindingId),
    Ambiguous,
    Incomplete,
    Unconfigured,
}

#[derive(Debug, Clone)]
pub(super) struct DeclarationPart {
    pub source: SourceInstanceId,
    pub declaration: i64,
    pub parameters: Vec<GlobalNameId>,
}
#[derive(Debug, Clone)]
pub(super) struct Group {
    pub parts: Vec<DeclarationPart>,
}
#[derive(Default, Clone)]
struct Source {
    isolated: Option<bool>,
    declarations: FxHashMap<(BindingId, bool), Result>,
}
#[derive(Default, Clone)]
struct Environment {
    pending: Vec<candidates::Pending>,
    rejected: Vec<i64>,
    context: Option<crate::type_checker::core::types::NominalContextId>,
    complete: bool,
    paths: FxHashMap<String, SourceInstanceId>,
    sources: Vec<Source>,
    names: FxHashMap<String, GlobalNameId>,
    globals: FxHashMap<(GlobalNameId, bool), Result>,
    groups: Vec<Group>,
}
#[derive(Default, Clone)]
pub(super) struct Graph {
    pub configuration: Option<Vec<Program>>,
    snapshot: u64,
    keys: FxHashMap<String, Vec<ProgramId>>,
    programs: Vec<Environment>,
}

impl Graph {
    pub(super) fn isolated(&self, source: SourceInstanceId) -> Option<bool> {
        self.environment(source.program)?
            .sources
            .get(source.index)?
            .isolated
    }
    pub(super) fn programs(&self) -> Vec<ProgramId> {
        (0..self.programs.len())
            .map(|index| ProgramId {
                snapshot: self.snapshot,
                index,
            })
            .collect()
    }
    pub(super) fn sources(&self, program: ProgramId) -> Vec<(String, SourceInstanceId)> {
        self.environment(program)
            .map(|env| {
                env.paths
                    .iter()
                    .map(|(path, &id)| (path.clone(), id))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn groups(&self, program: ProgramId) -> &[Group] {
        self.environment(program)
            .filter(|env| env.complete)
            .map(|env| env.groups.as_slice())
            .unwrap_or(&[])
    }
    pub(super) fn rejected(&self, program: ProgramId) -> &[i64] {
        self.environment(program)
            .map(|env| env.rejected.as_slice())
            .unwrap_or(&[])
    }
    pub(super) fn nominal_context(
        &self,
        program: ProgramId,
    ) -> Option<crate::type_checker::core::types::NominalContextId> {
        self.environment(program)
            .filter(|env| env.complete)?
            .context
    }
    pub(super) fn callable_policy(
        &self,
        program: ProgramId,
    ) -> Option<crate::indexer::programs::CallablePolicy> {
        self.environment(program).filter(|env| env.complete)?;
        self.configuration
            .as_ref()?
            .get(program.index)?
            .callable_policy
    }
    pub(super) fn compiler_intrinsics(
        &self,
        program: ProgramId,
    ) -> Option<crate::indexer::programs::CompilerIntrinsicPolicy> {
        self.environment(program).filter(|env| env.complete)?;
        self.configuration
            .as_ref()?
            .get(program.index)?
            .compiler_intrinsics
    }
    pub(super) fn source_binding_order(
        &self,
        program: ProgramId,
    ) -> Option<FxHashMap<SourceInstanceId, usize>> {
        let env = self.environment(program).filter(|env| env.complete)?;
        let order = self
            .configuration
            .as_ref()?
            .get(program.index)?
            .source_binding_order
            .as_ref()?;
        if order.len() != env.sources.len() {
            return None;
        }
        let mut bound = FxHashMap::default();
        for (rank, path) in order.iter().enumerate() {
            let source = *env.paths.get(&module_paths::normalize(path))?;
            if bound.insert(source, rank).is_some() {
                return None;
            }
        }
        Some(bound)
    }
    /// Ingestion/file-environment boundaries. No implicit program selection.
    pub(super) fn program(&self, key: &str) -> Option<ProgramId> {
        match self.keys.get(key)?.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }
    pub(super) fn source(&self, program: ProgramId, path: &str) -> Option<SourceInstanceId> {
        self.environment(program)?
            .paths
            .get(&module_paths::normalize(path))
            .copied()
    }
    pub(super) fn name(&self, program: ProgramId, spelling: &str) -> Option<GlobalNameId> {
        self.environment(program)?.names.get(spelling).copied()
    }
    pub(super) fn global(
        &self,
        program: ProgramId,
        name: GlobalNameId,
        type_space: bool,
    ) -> Result {
        let Some(env) = self.environment(program) else {
            return Result::Unconfigured;
        };
        if name.program != program {
            return Result::Unconfigured;
        }
        if !env.complete {
            return Result::Incomplete;
        }
        env.globals
            .get(&(name, type_space))
            .copied()
            .unwrap_or(Result::Missing)
    }
    pub(super) fn binding(
        &self,
        source: SourceInstanceId,
        binding: BindingId,
        type_space: bool,
    ) -> Result {
        let Some(env) = self.environment(source.program) else {
            return Result::Unconfigured;
        };
        if !env.complete {
            return Result::Incomplete;
        }
        env.sources
            .get(source.index)
            .and_then(|s| s.declarations.get(&(binding, type_space)))
            .copied()
            .unwrap_or(Result::Missing)
    }
    pub(super) fn group(&self, id: GlobalBindingId) -> Option<&Group> {
        self.environment(id.program)
            .filter(|env| env.complete)?
            .groups
            .get(id.index)
    }
    fn environment(&self, program: ProgramId) -> Option<&Environment> {
        (self.snapshot == program.snapshot)
            .then(|| self.programs.get(program.index))
            .flatten()
    }

    pub(super) fn rebuild(
        &mut self,
        inputs: &BTreeMap<String, ModuleInput>,
        lookup: &dyn SymbolLookup,
    ) {
        static NEXT_SNAPSHOT: AtomicU64 = AtomicU64::new(1);
        self.snapshot = NEXT_SNAPSHOT.fetch_add(1, Ordering::Relaxed);
        self.keys.clear();
        self.programs.clear();
        for config in self.configuration.as_deref().unwrap_or(&[]) {
            let id = ProgramId {
                snapshot: self.snapshot,
                index: self.programs.len(),
            };
            self.keys.entry(config.key.clone()).or_default().push(id);
            self.programs.push(build(id, config, inputs, lookup));
        }
        for ids in self.keys.values().filter(|ids| ids.len() != 1) {
            for id in ids {
                self.programs[id.index].complete = false;
            }
        }
    }

    pub(super) fn persist(&self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        let value = serde_json::to_string(&self.configuration)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("INSERT OR REPLACE INTO _bearwisdom_meta (key,value) VALUES ('program_configuration_v1',?1)", [value])?;
        Ok(())
    }
    pub(super) fn load(&mut self, conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        use rusqlite::OptionalExtension;
        if self.configuration.is_some() {
            return Ok(());
        }
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM _bearwisdom_meta WHERE key='program_configuration_v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(value) = value {
            self.configuration = serde_json::from_str(&value).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        }
        Ok(())
    }
}

struct Candidate<'a> {
    source: SourceInstanceId,
    part: &'a super::program_input::Part,
    rules: &'a [(crate::types::SymbolKind, crate::types::SymbolKind)],
    parameters: Vec<GlobalNameId>,
    members: Vec<GlobalNameId>,
    live: bool,
}

fn build(
    id: ProgramId,
    config: &Program,
    inputs: &BTreeMap<String, ModuleInput>,
    lookup: &dyn SymbolLookup,
) -> Environment {
    let mut env = Environment {
        context: Some(crate::type_checker::core::types::NominalContextId::fresh()),
        complete: config.complete && !config.key.is_empty() && !config.fingerprint.is_empty(),
        ..Default::default()
    };
    let mut candidates: FxHashMap<(GlobalNameId, bool), Vec<Candidate<'_>>> = FxHashMap::default();
    for source in &config.sources {
        let path = module_paths::normalize(&source.path);
        let instance = SourceInstanceId {
            program: id,
            index: env.sources.len(),
        };
        env.sources.push(Source::default());
        if env.paths.insert(path.clone(), instance).is_some() {
            env.complete = false;
        }
        let Some(input) = inputs.get(&path).filter(|input| {
            input.path == path
                && !source.content_hash.is_empty()
                && input.content_hash == source.content_hash
                && input.binding_epoch == super::module_input::BINDING_EPOCH
        }) else {
            env.complete = false;
            continue;
        };
        let Some(globals) = &input.globals else {
            env.complete = false;
            continue;
        };
        env.complete &= globals.complete && input.source_complete != Some(false);
        let isolated = match source.scope {
            SourceScope::Syntax => globals.isolated,
            SourceScope::Module => true,
            SourceScope::Unknown => {
                env.complete = false;
                continue;
            }
        };
        env.sources[instance.index].isolated = Some(isolated);
        env.complete &= contributions::valid(input, isolated);
        for part in globals
            .roots
            .iter()
            .filter(|_| !isolated)
            .chain(&globals.augmentations)
        {
            let name = intern(&mut env, id, &part.name);
            let parameters = part
                .parameters
                .iter()
                .map(|name| intern(&mut env, id, name))
                .collect();
            let members = part
                .members
                .iter()
                .map(|name| intern(&mut env, id, name))
                .collect();
            let live = part
                .declaration
                .and_then(|row| lookup.symbol_by_id(row))
                .is_some_and(|symbol| module_paths::normalize(&symbol.file_path) == path);
            candidates
                .entry((name, part.type_space))
                .or_default()
                .push(Candidate {
                    source: instance,
                    part,
                    rules: &globals.merge_rules,
                    parameters,
                    members,
                    live,
                });
        }
    }
    // Hash iteration order is not identity evidence. Deterministic group slots
    // simplify diagnostics; snapshot ownership still rejects all old handles.
    let mut candidates: Vec<_> = candidates.into_iter().collect();
    candidates.sort_unstable_by_key(|((name, domain), _)| (name.index, *domain));
    for (key, parts) in candidates {
        let result = bind_group(id, &parts, &mut env.groups);
        if !matches!(result, Result::Bound(_)) {
            env.rejected
                .extend(parts.iter().filter_map(|p| p.part.declaration));
            if result == Result::Incomplete {
                if let Some(candidate) = candidates::capture(key.0, &parts) {
                    env.pending.push(candidate);
                }
            }
        }
        env.globals.insert(key, result);
        for part in &parts {
            if let Some(binding) = part.part.binding {
                let entry = env.sources[part.source.index]
                    .declarations
                    .entry((BindingId(binding), key.1))
                    .or_insert(result);
                if *entry != result {
                    *entry = Result::Ambiguous;
                }
            }
        }
    }
    env
}

fn intern(env: &mut Environment, program: ProgramId, spelling: &str) -> GlobalNameId {
    let next = GlobalNameId {
        program,
        index: env.names.len(),
    };
    *env.names.entry(spelling.to_owned()).or_insert(next)
}

fn bind_group(program: ProgramId, parts: &[Candidate<'_>], groups: &mut Vec<Group>) -> Result {
    if parts.iter().any(|p| !p.live) {
        return Result::Incomplete;
    }
    if parts.len() > 1 {
        if parts
            .iter()
            .any(|p| !p.part.plain_parameters || !p.part.plain_merge)
        {
            return Result::Incomplete;
        }
        let mut kinds = FxHashMap::default();
        let mut members = rustc_hash::FxHashSet::default();
        for part in parts {
            if !part.part.type_space || part.parameters != parts[0].parameters {
                return Result::Ambiguous;
            }
            *kinds.entry(part.part.kind).or_insert(0usize) += 1;
            if part.members.iter().any(|member| !members.insert(*member)) {
                return Result::Incomplete;
            }
        }
        // Work scales with declarations times distinct kinds, not group size squared.
        for part in parts {
            for (&other, &count) in &kinds {
                if other == part.part.kind && count == 1 {
                    continue;
                }
                if !part.rules.iter().any(|&(a, b)| {
                    (a, b) == (part.part.kind, other) || (b, a) == (part.part.kind, other)
                }) {
                    return Result::Ambiguous;
                }
            }
        }
    }
    let id = GlobalBindingId {
        program,
        index: groups.len(),
    };
    groups.push(Group {
        parts: parts
            .iter()
            .map(|p| DeclarationPart {
                source: p.source,
                declaration: p.part.declaration.unwrap(),
                parameters: p.parameters.clone(),
            })
            .collect(),
    });
    Result::Bound(id)
}

#[cfg(test)]
#[path = "program_graph_tests.rs"]
mod tests;

#[path = "program_contributions.rs"]
mod contributions;
