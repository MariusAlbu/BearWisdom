// =============================================================================
// type_checker/core/types.rs — Type, TypeId, TypeArena, PrimKind, LitValue
//
// The arena owns every Type instance in the workspace. Consumers hold TypeIds
// (NonZeroU32) and dereference through TypeArena::get. Identity equality on
// types is integer equality on TypeIds.
//
// Spec: research/architecture/01-canonical-symbol-ref-contract.html
//       research/architecture/02-engine-internal-architecture.html § Layer 1
// =============================================================================

use rustc_hash::FxHashMap;
use std::num::NonZeroU32;

/// Interned-type identifier. Nonzero so `Option<TypeId>` is one word.
/// Stable within a workspace build; not durable across indexing runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub NonZeroU32);

impl TypeId {
    pub fn index(self) -> usize {
        (self.0.get() as usize) - 1
    }
}

/// Generic-parameter identifier. Bound names live in TypeArena::generic_params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GenericParamId(pub NonZeroU32);

impl GenericParamId {
    pub fn index(self) -> usize {
        (self.0.get() as usize) - 1
    }
}

/// Canonical primitive categories. Width-specific integer / float variants
/// collapse to Int / Float; the engine does not type-check numeric precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PrimKind {
    Int,
    Float,
    Str,
    Char,
    Bytes,
    Bool,
    Unit,
    Never,
    Symbol,
    Unknown,
}

/// Singleton-type value carrier. `Type::Literal(LitValue::Str("foo"))` is the
/// type whose only inhabitant is the string `"foo"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LitValue {
    Str(String),
    Int(i64),
    Bool(bool),
}

/// Bound info for a generic parameter — captured at extraction time so the
/// engine can resolve `T` back to the declaration that introduced it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericParamData {
    /// Source name of the parameter (e.g. `T`, `K`, `V`).
    pub name: String,
    /// Index of the symbol that introduces this parameter (function, type,
    /// method). Used to delimit the parameter's scope.
    pub owner_symbol_index: usize,
    /// Optional upper-bound type (e.g. `T extends Animal` in TS).
    pub bound: Option<TypeId>,
}

/// The structured form every value, parameter, return, and field receives once
/// the engine has interned it. Strings appear only inside `Class(QName)` and
/// `Literal(LitValue::Str)`; every other type relationship is by TypeId.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    /// Nominal reference identified by fully qualified name.
    Class(String),
    /// Built-in scalar.
    Primitive(PrimKind),
    /// Callable signature.
    Function {
        params: Vec<TypeId>,
        return_: TypeId,
    },
    /// Positional fixed-length aggregate.
    Tuple(Vec<TypeId>),
    /// Sum of disjoint types. Member lookup returns the intersection of
    /// members across branches.
    Union(Vec<TypeId>),
    /// Combined member set. Member lookup returns the union of members
    /// across branches.
    Intersection(Vec<TypeId>),
    /// Generic application: `List<User>`, `Map<K, V>`, `Promise<Result>`.
    /// `base` resolves to a Class or TypeAlias.
    Apply {
        base: TypeId,
        args: Vec<TypeId>,
    },
    /// In-scope generic parameter.
    Generic {
        param: GenericParamId,
    },
    /// Nullable wrapper. Engine looks through it for member resolution when
    /// `LanguageProfile::look_through_optional` is true.
    Optional(TypeId),
    /// Async wrapper. `await` unwraps to the inner type.
    AsyncWrapper(TypeId),
    /// Iterable wrapper. `for x in collection` binds `x` to the inner type.
    Iterator(TypeId),
    /// Singleton type (literal types).
    Literal(LitValue),
    /// Engine bailout — member lookup fails closed rather than guessing.
    Unknown,
}

/// Per-workspace interning storage for `Type` values. Built once per indexing
/// run; populated by extractors then frozen as the engine consumes it.
#[derive(Debug, Default)]
pub struct TypeArena {
    types: Vec<Type>,
    intern: FxHashMap<Type, TypeId>,
    qname_to_class: FxHashMap<String, TypeId>,
    generic_params: Vec<GenericParamData>,
}

impl TypeArena {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `ty`, returning an existing TypeId on duplicate insert.
    pub fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(&id) = self.intern.get(&ty) {
            return id;
        }
        let idx = self.types.len();
        let id = TypeId(NonZeroU32::new((idx + 1) as u32).expect("arena index overflow"));
        self.types.push(ty.clone());
        self.intern.insert(ty, id);
        id
    }

    /// Look up an existing TypeId for a `Type` without inserting.
    pub fn lookup(&self, ty: &Type) -> Option<TypeId> {
        self.intern.get(ty).copied()
    }

    /// Intern a Class type by qualified name. Subsequent calls return the
    /// same TypeId.
    pub fn class(&mut self, qname: &str) -> TypeId {
        if let Some(&id) = self.qname_to_class.get(qname) {
            return id;
        }
        let id = self.intern(Type::Class(qname.to_string()));
        self.qname_to_class.insert(qname.to_string(), id);
        id
    }

    /// Look up the Class TypeId for `qname` without interning.
    pub fn class_lookup(&self, qname: &str) -> Option<TypeId> {
        self.qname_to_class.get(qname).copied()
    }

    /// Intern a primitive type.
    pub fn primitive(&mut self, p: PrimKind) -> TypeId {
        self.intern(Type::Primitive(p))
    }

    /// Resolve a TypeId to the underlying `Type`. Panics on out-of-range
    /// ids — TypeIds are only constructed through `intern`, so this should
    /// never fire in well-formed code.
    pub fn get(&self, id: TypeId) -> &Type {
        &self.types[id.index()]
    }

    /// Number of distinct types currently interned.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Allocate a generic parameter slot.
    pub fn intern_generic(&mut self, data: GenericParamData) -> GenericParamId {
        let idx = self.generic_params.len();
        let id = GenericParamId(NonZeroU32::new((idx + 1) as u32).expect("arena index overflow"));
        self.generic_params.push(data);
        id
    }

    pub fn generic_param(&self, id: GenericParamId) -> &GenericParamData {
        &self.generic_params[id.index()]
    }

    pub fn generic_param_count(&self) -> usize {
        self.generic_params.len()
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
