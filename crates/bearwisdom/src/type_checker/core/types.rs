// =============================================================================
// type_checker/core/types.rs — Type, TypeId, TypeArena, PrimKind, LitValue
//
// The arena owns every Type instance in the workspace. Consumers hold TypeIds
// (NonZeroU32) and dereference through TypeArena::get. Identity equality on
// types is integer equality on TypeIds.
//
// Phase 5 makes the arena interior-mutable: `intern` / `class` / `primitive`
// take `&self` so a single shared `Engine` can drive resolution across rayon
// workers. `get` returns owned `Type` (cheap clone — every prior call site
// already cloned the borrowed result).
//
// Spec: research/architecture/01-canonical-symbol-ref-contract.html
//       research/architecture/02-engine-internal-architecture.html § Layer 1
// =============================================================================

use rustc_hash::FxHashMap;
use std::num::NonZeroU32;
use std::sync::RwLock;

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

#[derive(Default)]
struct TypeArenaInner {
    types: Vec<Type>,
    intern: FxHashMap<Type, TypeId>,
    qname_to_class: FxHashMap<String, TypeId>,
    generic_params: Vec<GenericParamData>,
}

/// Per-workspace interning storage for `Type` values. Interior-mutable so a
/// single `&TypeArena` can be shared across rayon workers — every mutating
/// method (`intern`, `class`, `primitive`, `intern_generic`) takes `&self`
/// and acquires a write lock briefly; reads acquire a read lock and clone.
///
/// Cloning the result of `get` is cheap for the dominant case (Class /
/// Primitive / Literal / Unknown are single-word) and matches what every
/// pre-Phase-5 caller did already (`arena.get(ty).clone()`).
pub struct TypeArena {
    inner: RwLock<TypeArenaInner>,
}

impl Default for TypeArena {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TypeArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.read().unwrap();
        f.debug_struct("TypeArena")
            .field("types_len", &inner.types.len())
            .field("generic_params_len", &inner.generic_params.len())
            .finish()
    }
}

impl TypeArena {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(TypeArenaInner::default()),
        }
    }

    /// Intern `ty`, returning an existing TypeId on duplicate insert.
    /// Reads first for the common dedup case; upgrades to write only when
    /// a new entry is needed. Safe to call concurrently.
    pub fn intern(&self, ty: Type) -> TypeId {
        if let Some(&id) = self.inner.read().unwrap().intern.get(&ty) {
            return id;
        }
        let mut inner = self.inner.write().unwrap();
        // Re-check after acquiring write lock — another thread may have
        // interned the same Type while we were upgrading.
        if let Some(&id) = inner.intern.get(&ty) {
            return id;
        }
        let idx = inner.types.len();
        let id = TypeId(NonZeroU32::new((idx + 1) as u32).expect("arena index overflow"));
        inner.types.push(ty.clone());
        inner.intern.insert(ty, id);
        id
    }

    /// Look up an existing TypeId for a `Type` without inserting.
    pub fn lookup(&self, ty: &Type) -> Option<TypeId> {
        self.inner.read().unwrap().intern.get(ty).copied()
    }

    /// Intern a Class type by qualified name. Subsequent calls return the
    /// same TypeId.
    pub fn class(&self, qname: &str) -> TypeId {
        if let Some(&id) = self.inner.read().unwrap().qname_to_class.get(qname) {
            return id;
        }
        let id = self.intern(Type::Class(qname.to_string()));
        // Reacquire write to track in qname_to_class. Idempotent insert is
        // safe under concurrent callers.
        self.inner
            .write()
            .unwrap()
            .qname_to_class
            .insert(qname.to_string(), id);
        id
    }

    /// Look up the Class TypeId for `qname` without interning.
    pub fn class_lookup(&self, qname: &str) -> Option<TypeId> {
        self.inner.read().unwrap().qname_to_class.get(qname).copied()
    }

    /// Intern a primitive type.
    pub fn primitive(&self, p: PrimKind) -> TypeId {
        self.intern(Type::Primitive(p))
    }

    /// Resolve a TypeId to the underlying `Type`. Returns an owned clone so
    /// callers don't hold the read lock across operations. Cloning is
    /// cheap for the common variants and matches what every prior call
    /// site already did (`arena.get(ty).clone()`).
    ///
    /// Panics on out-of-range ids — TypeIds are only constructed through
    /// `intern`, so this should never fire in well-formed code.
    pub fn get(&self, id: TypeId) -> Type {
        self.inner.read().unwrap().types[id.index()].clone()
    }

    /// Number of distinct types currently interned.
    pub fn len(&self) -> usize {
        self.inner.read().unwrap().types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().types.is_empty()
    }

    /// Allocate a generic parameter slot.
    pub fn intern_generic(&self, data: GenericParamData) -> GenericParamId {
        let mut inner = self.inner.write().unwrap();
        let idx = inner.generic_params.len();
        let id = GenericParamId(NonZeroU32::new((idx + 1) as u32).expect("arena index overflow"));
        inner.generic_params.push(data);
        id
    }

    pub fn generic_param(&self, id: GenericParamId) -> GenericParamData {
        self.inner.read().unwrap().generic_params[id.index()].clone()
    }

    pub fn generic_param_count(&self) -> usize {
        self.inner.read().unwrap().generic_params.len()
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
