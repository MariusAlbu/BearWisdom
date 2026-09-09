//! Bounded lazy evaluation of source value-query dependencies. Forward reads
//! use source recipes; cycles never borrow a partially materialized signature.
use super::{
    computed_keys::{self, Reader, Sources, Values},
    Lookup, SourceInstanceId, View,
};
use crate::indexer::resolve::engine::{
    compilation::Compilation,
    contract::{generic_return, SymbolLookup},
    module_type_inputs::Slot,
    program_types::{self, source_signatures::SignatureId, Recipe},
};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::SourceSpan;
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};

#[path = "program_initializer_values.rs"]
mod initializers;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Query(SourceInstanceId, SourceSpan),
    Object(SourceInstanceId, SourceSpan),
    Field(i64),
    Signature(SourceInstanceId, SignatureId),
    Initializer(SourceInstanceId, SignatureId),
    Alias(TypeId),
}
#[derive(Clone, Copy)]
enum State {
    Active,
    Done(Option<TypeId>, usize),
}

struct Solver<'a> {
    view: &'a View,
    tree: &'a Compilation,
    arena: &'a TypeArena,
    values: Values,
    sources: FxHashMap<SourceInstanceId, (&'a str, &'a program_types::Input)>,
    fields: FxHashMap<i64, Vec<(SourceInstanceId, &'a Recipe)>>,
    aliases: FxHashMap<i64, Vec<(SourceInstanceId, &'a Recipe)>>,
    queries: FxHashMap<(SourceInstanceId, SourceSpan), Option<&'a computed_keys::Computed>>,
    signatures: FxHashMap<
        (SourceInstanceId, SignatureId),
        Option<&'a program_types::source_signatures::Input>,
    >,
    bases: FxHashMap<i64, Vec<(SourceInstanceId, &'a program_types::bases::Input)>>,
    interfaces: FxHashMap<i64, Vec<(SourceInstanceId, &'a program_types::interfaces::Input)>>,
    initializers: initializers::Inputs<'a>,
    constructor_calls:
        RefCell<FxHashMap<(SourceInstanceId, SignatureId), super::constructors::Call>>,
    memo: RefCell<FxHashMap<Key, State>>,
    heights: RefCell<Vec<usize>>,
    exhausted: Cell<u64>,
}

pub(super) fn bind(
    view: &mut View,
    inputs: &Sources,
    tree: &Compilation,
    arena: &TypeArena,
) -> bool {
    let values = computed_keys::prepare(view, inputs);
    let mut solver = Solver {
        view,
        tree,
        arena,
        values,
        sources: inputs
            .iter()
            .map(|(path, source, input)| (**source, (path.as_str(), *input)))
            .collect(),
        fields: Default::default(),
        aliases: Default::default(),
        queries: Default::default(),
        signatures: Default::default(),
        bases: Default::default(),
        interfaces: Default::default(),
        initializers: initializers::Inputs::new(inputs, view),
        constructor_calls: Default::default(),
        memo: Default::default(),
        heights: Default::default(),
        exhausted: Cell::new(0),
    };
    for (_, source, input) in inputs {
        for query in &input.computed.queries {
            solver
                .queries
                .entry((**source, query.site))
                .and_modify(|old| *old = None)
                .or_insert(Some(query));
        }
        for signature in &input.source_signatures {
            solver
                .signatures
                .entry((**source, signature.id))
                .and_modify(|old| *old = None)
                .or_insert(Some(signature));
        }
        for base in &input.bases {
            if let Some(&owner) = view.canonical.get(&base.owner) {
                solver
                    .bases
                    .entry(owner)
                    .or_default()
                    .push((**source, base));
            }
        }
        for interface in &input.interfaces {
            if let Some(&owner) = view.canonical.get(&interface.owner) {
                solver
                    .interfaces
                    .entry(owner)
                    .or_default()
                    .push((**source, interface));
            }
        }
        for signature in &input.signatures {
            let Some(&row) = view.canonical.get(&signature.declaration) else {
                continue;
            };
            let targets = match signature.slot {
                Slot::Field => &mut solver.fields,
                Slot::Alias => &mut solver.aliases,
                _ => continue,
            };
            for recipe in &signature.recipes {
                targets.entry(row).or_default().push((**source, recipe));
            }
        }
    }
    let mut queries = Vec::new();
    let mut keys = Vec::new();
    let mut initializers = Vec::new();
    for (path, &source, input) in inputs {
        for query in &input.computed.queries {
            queries.push((source, query.site, solver.query(source, query.site)));
        }
        for input in &input.initializers {
            let ty = solver.initializer(source, input.signature);
            let call = solver
                .constructor_calls
                .borrow()
                .get(&(source, input.signature))
                .filter(|call| Some(call.return_type) == ty)
                .cloned();
            initializers.push((source, input.signature, ty, call));
        }
        let lookup = solver.lookup(source);
        for key in &input.computed.keys {
            let ty = solver
                .values
                .value(&lookup, arena, path, source, key, &solver)
                .filter(|&ty| matches!(arena.get(ty), Type::UniqueSymbol(_)));
            keys.push((source, key.site, ty));
        }
    }
    drop(solver);
    let mut changed = false;
    for (source, site, ty) in queries {
        changed |= view
            .sources
            .get_mut(&source)
            .unwrap()
            .value_queries
            .insert(site, ty)
            != Some(ty);
    }
    for (source, site, ty) in keys {
        changed |= view
            .sources
            .get_mut(&source)
            .unwrap()
            .computed_keys
            .insert(site, ty)
            != Some(ty);
    }
    for (source, site, ty, call) in initializers {
        let source = view.sources.get_mut(&source).unwrap();
        changed |= source.initializers.insert(site, ty) != Some(ty);
        if source.constructor_calls.get(&site) != Some(&call) {
            changed = true;
            source.constructor_calls.insert(site, call);
        }
    }
    changed
}

impl Solver<'_> {
    fn lookup(&self, source: SourceInstanceId) -> Lookup<'_> {
        Lookup {
            tree: self.tree,
            view: self.view,
            source: self.view.sources.get(&source),
        }
    }
    fn memoized(&self, key: Key, evaluate: impl FnOnce() -> Option<TypeId>) -> Option<TypeId> {
        let depth = self.heights.borrow().len();
        match self.memo.borrow().get(&key).copied() {
            Some(State::Active) => return None,
            Some(State::Done(value, height)) => {
                self.note_height(height);
                if depth + height > 128 {
                    self.exhausted.set(self.exhausted.get() + 1);
                    return None;
                }
                return value;
            }
            None => {}
        }
        if depth >= 128 {
            self.exhausted.set(self.exhausted.get() + 1);
            return None;
        }
        let exhausted = self.exhausted.get();
        self.memo.borrow_mut().insert(key, State::Active);
        self.heights.borrow_mut().push(1);
        let value = evaluate();
        let height = self.heights.borrow_mut().pop().unwrap();
        self.note_height(height);
        if self.exhausted.get() != exhausted {
            self.memo.borrow_mut().remove(&key);
            return None;
        }
        self.memo
            .borrow_mut()
            .insert(key, State::Done(value, height));
        value
    }
    fn note_height(&self, height: usize) {
        if let Some(parent) = self.heights.borrow_mut().last_mut() {
            *parent = (*parent).max(height + 1);
        }
    }
    fn query(&self, source: SourceInstanceId, site: SourceSpan) -> Option<TypeId> {
        self.memoized(Key::Query(source, site), || {
            let (path, _) = self.sources.get(&source)?;
            let query = self.queries.get(&(source, site)).copied().flatten()?;
            self.values
                .value(&self.lookup(source), self.arena, path, source, query, self)
        })
    }
    fn definition(&self, targets: &[(SourceInstanceId, &Recipe)]) -> Option<TypeId> {
        match targets {
            [(source, recipe)] => Some(self.recipe(*source, recipe)),
            _ => None,
        }
    }

    fn interface_defaults(&self, ty: TypeId, owner: i64, mut args: Vec<TypeId>) -> Option<TypeId> {
        let Some(parts) = self.interfaces.get(&owner) else {
            return Some(ty);
        };
        let params = &self.view.info.get(&owner)?.generic_param_ids;
        if args.len() > params.len() {
            return None;
        }
        if args.len() == params.len() {
            return Some(ty);
        }
        let mut bindings = params.iter().copied().zip(args.iter().copied()).collect();
        while args.len() < params.len() {
            let mut defaults = Vec::new();
            for (source, part) in parts {
                let signature = self
                    .signatures
                    .get(&(*source, part.generic_signature?))
                    .copied()
                    .flatten()?;
                let default = signature.generics.get(args.len())?.default.as_ref()?;
                defaults.push(generic_return::substitute(
                    self.arena,
                    self.recipe(*source, default),
                    &bindings,
                ));
            }
            let &default = defaults.first()?;
            if defaults.iter().any(|&other| other != default) {
                return None;
            }
            bindings.insert(params[args.len()], default);
            args.push(default);
        }
        let base = match self.arena.get(ty) {
            Type::Apply { base, .. } => base,
            _ => ty,
        };
        Some(self.arena.intern(Type::Apply { base, args }))
    }
}

impl Reader for Solver<'_> {
    fn recipe(&self, source: SourceInstanceId, recipe: &Recipe) -> TypeId {
        let (path, _) = self.sources[&source];
        recipe.materialize_with_query(&self.lookup(source), self.arena, path, &|site| {
            self.query(source, site)
        })
    }
    fn field(&self, row: i64) -> Option<TypeId> {
        let row = *self.view.canonical.get(&row)?;
        self.memoized(Key::Field(row), || {
            if let Some(annotated) = self.fields.get(&row) {
                return self.definition(annotated);
            }
            let &(source, signature) = self.initializers.fields.get(&row)?.as_ref()?;
            self.initializer(source, signature)
        })
    }
    fn signature(&self, source: SourceInstanceId, signature: SignatureId) -> Option<TypeId> {
        self.memoized(Key::Signature(source, signature), || {
            let input = self
                .signatures
                .get(&(source, signature))
                .copied()
                .flatten()?;
            input
                .result
                .as_ref()
                .map(|recipe| self.recipe(source, recipe))
                .or_else(|| self.initializer(source, signature))
        })
    }
    fn expand(&self, ty: TypeId) -> Option<TypeId> {
        self.memoized(Key::Alias(ty), || {
            let (head, args) = match self.arena.get(ty) {
                Type::Apply { base, args } => (base, args),
                _ => (ty, vec![]),
            };
            let Type::Decl { symbol_id, .. } = self.arena.get(head) else {
                return Some(ty);
            };
            let owner = *self.view.canonical.get(&symbol_id)?;
            let Some(recipes) = self.aliases.get(&owner) else {
                return self.interface_defaults(ty, owner, args);
            };
            let params = &self.view.info.get(&owner)?.generic_param_ids;
            if params.len() != args.len() {
                return None;
            }
            let result = self.definition(recipes)?;
            let substitutions = params.iter().copied().zip(args).collect();
            self.expand(generic_return::substitute(
                self.arena,
                result,
                &substitutions,
            ))
        })
    }
    fn bases(&self, owner: i64) -> Option<Vec<TypeId>> {
        if let Some(parts) = self.interfaces.get(&owner) {
            let mut parts: Vec<_> = parts.iter().collect();
            parts.sort_by_key(|(source, _)| source.ordinal());
            let mut bases = Vec::new();
            for (source, part) in parts {
                if part.inherited_members
                    != crate::indexer::lexical::globals::InheritedMembers::DeclarationOrder
                {
                    return None;
                }
                for recipe in part.bases.as_ref()? {
                    bases.push(self.expand(self.recipe(*source, recipe))?);
                }
            }
            return Some(bases);
        }
        let Some(bases) = self.bases.get(&owner) else {
            return Some(vec![]);
        };
        let [(source, base)] = bases.as_slice() else {
            return None;
        };
        Some(vec![base.materialize_with_query(
            &self.lookup(*source),
            self.arena,
            self.sources[source].0,
            &|site| self.query(*source, site),
        )])
    }
}

#[cfg(test)]
#[path = "program_value_queries_tests.rs"]
mod tests;
