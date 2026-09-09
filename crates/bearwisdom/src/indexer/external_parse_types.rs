// =============================================================================
// indexer/external_parse_types.rs — arena-independent type serialization
//
// A run's `TypeId` / `GenericParamId` values index a per-workspace `TypeArena`
// and are meaningless in the next run's arena, so a persisted extraction
// cannot carry them raw. `CachedType` is the portable structural form of a
// `Type` (child ids expanded into the tree); `CachedGenericParam` entries form
// a file-local parameter table that preserves identity — every occurrence of
// one parameter re-interns to ONE `GenericParamId`, so generic substitution
// against `Apply` args still binds after a cache hit.
// =============================================================================

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::type_checker::core::types::{
    GenericParamData, GenericParamId, GenericParamKind, Indirection, Intrinsic, Lifetime, LitValue,
    Mutability, PrimKind, Type, TypeArena, TypeId,
};

#[cfg(test)]
#[path = "external_parse_types_tests.rs"]
mod tests;

/// Arena-independent structural form of a `Type`. Child `TypeId`s are expanded
/// into the tree; `Generic` refers into the owning payload's parameter table
/// by index so parameter identity survives the round trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum CachedType {
    Class(String),
    Primitive(PrimKind),
    Intrinsic(Intrinsic),
    Operator(Box<crate::type_checker::core::types::TypeOperator<Self>>),
    Function {
        params: Vec<CachedType>,
        return_: Box<CachedType>,
    },
    Tuple(Vec<CachedType>),
    Union(Vec<CachedType>),
    Intersection(Vec<CachedType>),
    Apply {
        base: Box<CachedType>,
        args: Vec<CachedType>,
    },
    Generic(usize),
    Region(CachedLifetime),
    Indirect {
        kind: CachedIndirection,
        mutability: Mutability,
        inner: Box<CachedType>,
    },
    Optional(Box<CachedType>),
    AsyncWrapper(Box<CachedType>),
    Iterator(Box<CachedType>),
    Constructor(Box<CachedType>),
    Literal(LitValue),
    Unknown,
}

/// One entry of the file-local generic-parameter table.
/// `owner_symbol_index` is an index into the same file's symbol vec, so it is
/// portable as long as symbol order is preserved (it is — symbols are cached
/// in extraction order).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CachedGenericParam {
    #[serde(default)]
    pub kind: GenericParamKind,
    pub name: String,
    pub owner_symbol_index: usize,
    pub bound: Option<CachedType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) enum CachedLifetime {
    Static,
    Parameter(usize),
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) enum CachedIndirection {
    Reference(CachedLifetime),
    Pointer,
}

/// Converts arena `TypeId`s into `CachedType` trees, accumulating the
/// generic-parameter table as parameters are first encountered.
pub(crate) struct TypeExporter<'a> {
    arena: &'a TypeArena,
    param_index: FxHashMap<GenericParamId, usize>,
    params: Vec<CachedGenericParam>,
}

impl<'a> TypeExporter<'a> {
    pub(crate) fn new(arena: &'a TypeArena) -> Self {
        Self {
            arena,
            param_index: FxHashMap::default(),
            params: Vec::new(),
        }
    }

    /// The accumulated parameter table, consumed by the payload after all
    /// types are exported.
    pub(crate) fn into_params(self) -> Vec<CachedGenericParam> {
        self.params
    }

    pub(crate) fn export(&mut self, id: TypeId) -> CachedType {
        match self.arena.get(id) {
            Type::Class(q) => CachedType::Class(q),
            // The parse cache is content-keyed and outlives any one index run;
            // symbol ids do not. An unconfigured nominal retains its legacy name.
            Type::Decl {
                qname,
                context: None,
                ..
            } => CachedType::Class(qname),
            // Program-owned nominals must be recaptured from source; their
            // display cannot become unconfigured name-resolution authority.
            Type::Decl {
                context: Some(_), ..
            } => CachedType::Unknown,
            Type::UniqueSymbol(_) | Type::Callable(_) | Type::Object(_) => CachedType::Unknown,
            Type::Primitive(p) => CachedType::Primitive(p),
            Type::Intrinsic(p) => CachedType::Intrinsic(p),
            Type::Operator(op) => CachedType::Operator(Box::new(op.map(|ty| self.export(*ty)))),
            Type::Function { params, return_ } => CachedType::Function {
                params: params.iter().map(|&p| self.export(p)).collect(),
                return_: Box::new(self.export(return_)),
            },
            Type::Tuple(items) => {
                CachedType::Tuple(items.iter().map(|&t| self.export(t)).collect())
            }
            Type::Union(branches) => {
                CachedType::Union(branches.iter().map(|&b| self.export(b)).collect())
            }
            Type::Intersection(branches) => {
                CachedType::Intersection(branches.iter().map(|&b| self.export(b)).collect())
            }
            Type::Apply { base, args } => CachedType::Apply {
                base: Box::new(self.export(base)),
                args: args.iter().map(|&a| self.export(a)).collect(),
            },
            Type::Generic { param } => CachedType::Generic(self.export_param(param)),
            Type::Region(region) => CachedType::Region(self.export_region(region)),
            Type::Indirect {
                kind,
                mutability,
                inner,
            } => CachedType::Indirect {
                kind: match kind {
                    Indirection::Reference(region) => {
                        CachedIndirection::Reference(self.export_region(region))
                    }
                    Indirection::Pointer => CachedIndirection::Pointer,
                },
                mutability,
                inner: Box::new(self.export(inner)),
            },
            Type::Optional(inner) => CachedType::Optional(Box::new(self.export(inner))),
            Type::AsyncWrapper(inner) => CachedType::AsyncWrapper(Box::new(self.export(inner))),
            Type::Iterator(inner) => CachedType::Iterator(Box::new(self.export(inner))),
            Type::Constructor(inner) => CachedType::Constructor(Box::new(self.export(inner))),
            Type::Literal(v) => CachedType::Literal(v),
            Type::Unknown => CachedType::Unknown,
        }
    }

    fn export_region(&mut self, region: Lifetime) -> CachedLifetime {
        // Content-only parse payloads have no declaration-ID remapper. Runtime
        // borrow variables must be recaptured from source, not exported as IDs.
        match region {
            Lifetime::Static => CachedLifetime::Static,
            Lifetime::Unknown | Lifetime::Inference { .. } => CachedLifetime::Unknown,
            Lifetime::Parameter(param) => CachedLifetime::Parameter(self.export_param(param)),
        }
    }

    /// Table index for a generic parameter. Reserve-then-fill: the slot is
    /// registered before the bound is exported, so a bound that references its
    /// own parameter terminates on the reserved index instead of recursing.
    pub(crate) fn export_param(&mut self, gp: GenericParamId) -> usize {
        if let Some(&i) = self.param_index.get(&gp) {
            return i;
        }
        let i = self.params.len();
        self.params.push(CachedGenericParam {
            kind: GenericParamKind::Type,
            name: String::new(),
            owner_symbol_index: 0,
            bound: None,
        });
        self.param_index.insert(gp, i);
        let data = self.arena.generic_param(gp);
        let bound = data.bound.map(|b| self.export(b));
        self.params[i] = CachedGenericParam {
            kind: data.kind,
            name: data.name,
            owner_symbol_index: data.owner_symbol_index,
            bound,
        };
        i
    }
}

/// Re-interns `CachedType` trees into the current run's arena. Each table
/// entry interns exactly once (`slots`), so every occurrence of one cached
/// parameter converges on one fresh `GenericParamId` — the same identity a
/// fresh extraction would produce.
pub(crate) struct TypeImporter<'a> {
    arena: &'a TypeArena,
    params: Vec<CachedGenericParam>,
    slots: Vec<Option<GenericParamId>>,
    importing: Vec<bool>,
}

impl<'a> TypeImporter<'a> {
    pub(crate) fn new(arena: &'a TypeArena, params: Vec<CachedGenericParam>) -> Self {
        let n = params.len();
        Self {
            arena,
            params,
            slots: vec![None; n],
            importing: vec![false; n],
        }
    }

    pub(crate) fn import(&mut self, t: &CachedType) -> TypeId {
        match t {
            // `class` (not a plain intern) so the arena's qname→class index is
            // populated the same way the fresh extraction path populates it.
            CachedType::Class(q) => self.arena.class(q),
            CachedType::Primitive(p) => self.arena.primitive(*p),
            CachedType::Intrinsic(p) => self.arena.intern(Type::Intrinsic(*p)),
            CachedType::Operator(op) => {
                let op = op.map(|ty| self.import(ty));
                self.arena.intern(Type::Operator(op))
            }
            CachedType::Function { params, return_ } => {
                let params = params.iter().map(|p| self.import(p)).collect();
                let return_ = self.import(return_);
                self.arena.intern(Type::Function { params, return_ })
            }
            CachedType::Tuple(items) => {
                let items = items.iter().map(|i| self.import(i)).collect();
                self.arena.intern(Type::Tuple(items))
            }
            CachedType::Union(branches) => {
                let branches = branches.iter().map(|b| self.import(b)).collect();
                self.arena.intern(Type::Union(branches))
            }
            CachedType::Intersection(branches) => {
                let branches = branches.iter().map(|b| self.import(b)).collect();
                self.arena.intern(Type::Intersection(branches))
            }
            CachedType::Apply { base, args } => {
                let base = self.import(base);
                let args = args.iter().map(|a| self.import(a)).collect();
                self.arena.intern(Type::Apply { base, args })
            }
            CachedType::Generic(i) => {
                let param = self.import_param(*i);
                self.arena.intern(Type::Generic { param })
            }
            CachedType::Optional(inner) => {
                let inner = self.import(inner);
                self.arena.intern(Type::Optional(inner))
            }
            CachedType::Region(region) => {
                let region = self.import_region(*region);
                self.arena.intern(Type::Region(region))
            }
            CachedType::Indirect {
                kind,
                mutability,
                inner,
            } => {
                let inner = self.import(inner);
                let kind = match kind {
                    CachedIndirection::Reference(region) => {
                        Indirection::Reference(self.import_region(*region))
                    }
                    CachedIndirection::Pointer => Indirection::Pointer,
                };
                self.arena.intern(Type::Indirect {
                    kind,
                    mutability: *mutability,
                    inner,
                })
            }
            CachedType::AsyncWrapper(inner) => {
                let inner = self.import(inner);
                self.arena.intern(Type::AsyncWrapper(inner))
            }
            CachedType::Iterator(inner) => {
                let inner = self.import(inner);
                self.arena.intern(Type::Iterator(inner))
            }
            CachedType::Constructor(inner) => {
                let inner = self.import(inner);
                self.arena.intern(Type::Constructor(inner))
            }
            CachedType::Literal(v) => self.arena.intern(Type::Literal(v.clone())),
            CachedType::Unknown => self.arena.intern(Type::Unknown),
        }
    }

    fn import_region(&mut self, region: CachedLifetime) -> Lifetime {
        match region {
            CachedLifetime::Static => Lifetime::Static,
            CachedLifetime::Unknown => Lifetime::Unknown,
            CachedLifetime::Parameter(index)
                if self
                    .params
                    .get(index)
                    .is_some_and(|p| p.kind == GenericParamKind::Lifetime) =>
            {
                Lifetime::Parameter(self.import_param(index))
            }
            CachedLifetime::Parameter(_) => Lifetime::Unknown,
        }
    }

    pub(crate) fn import_param(&mut self, i: usize) -> GenericParamId {
        if let Some(id) = self.slots.get(i).copied().flatten() {
            return id;
        }
        // Out-of-range indices cannot come from a payload this module wrote;
        // fail soft with a bound-less placeholder rather than panicking on a
        // corrupt cache row.
        if i >= self.params.len() {
            return self.arena.intern_generic(GenericParamData {
                kind: GenericParamKind::Type,
                name: String::new(),
                owner_symbol_index: 0,
                bound: None,
            });
        }
        // `GenericParamData` is immutable once interned, so a bound that
        // references its own parameter cannot round-trip in one intern call;
        // the re-entrant occurrence binds bound-free and both frames converge
        // on that id through the slot check below.
        let bound = if self.importing[i] {
            None
        } else {
            self.importing[i] = true;
            let bound_t = self.params[i].bound.clone();
            let b = bound_t.as_ref().map(|t| self.import(t));
            self.importing[i] = false;
            if let Some(id) = self.slots[i] {
                return id;
            }
            b
        };
        let id = self.arena.intern_generic(GenericParamData {
            kind: self.params[i].kind,
            name: self.params[i].name.clone(),
            owner_symbol_index: self.params[i].owner_symbol_index,
            bound,
        });
        self.slots[i] = Some(id);
        id
    }
}
