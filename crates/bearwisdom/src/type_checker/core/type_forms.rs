//! Canonical type forms; arena storage and operations live in the parent module.
use super::{
    Callable, GenericParamId, Indirection, Intrinsic, Lifetime, LitValue, Mutability,
    NominalContextId, PrimKind, SourceObject, TypeId, TypeOperator, UniqueSymbol,
};

/// The structured form every value, parameter, return, and field receives once
/// the engine has interned it. Strings appear only inside the nominal arms
/// (`Class`, `Decl`) and `Literal(LitValue::Str)`; every other type
/// relationship is by TypeId.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Type {
    /// Nominal reference identified by fully qualified name. The bootstrap
    /// nominal: extraction interns these before declaration rows exist, and
    /// ambient/external/synthesized names never get a row.
    Class(String),
    /// Bound nominal identity is `(context, symbol_id)`. The physical row stays
    /// a navigation address; one source can have different semantics in distinct
    /// configured programs. `qname` is display-only, never an interning key.
    Decl {
        symbol_id: i64,
        qname: String,
        #[serde(default)]
        context: Option<NominalContextId>,
    },
    /// Built-in scalar.
    Primitive(PrimKind),
    /// Source-language atoms, including top/dynamic/nullish types. Unknown
    /// here means the language type, not the engine's unresolved sentinel.
    Intrinsic(Intrinsic),
    UniqueSymbol(UniqueSymbol),
    /// Deferred source-owned operation; equality is not a completeness proof.
    Operator(TypeOperator<TypeId>),
    /// Callable signature.
    Function {
        params: Vec<TypeId>,
        return_: TypeId,
    },
    /// Source-owned callable, retaining axes erased by the legacy Function arm.
    Callable(Box<Callable<TypeId>>),
    /// Anonymous source-owned object with independently retained member origins.
    Object(Box<SourceObject<TypeId>>),
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
    /// Tagged generic lifetime argument, never a value/type parameter.
    Region(Lifetime),
    /// Exact reference/pointer structure. Unknown regions are not equality evidence.
    Indirect {
        kind: Indirection,
        mutability: Mutability,
        inner: TypeId,
    },
    /// Nullable wrapper. Engine looks through it for member resolution when
    /// `LanguageProfile::look_through_optional` is true.
    Optional(TypeId),
    /// Async wrapper. `await` unwraps to the inner type.
    AsyncWrapper(TypeId),
    /// Iterable wrapper. `for x in collection` binds `x` to the inner type.
    Iterator(TypeId),
    /// The type of a CLASS VALUE (`typeof C`) — the constructor, not an
    /// instance. Member lookup on it sees statics; a token-shaped parameter
    /// (`Type<T>`) unifies its instance out of `inner`.
    Constructor(TypeId),
    /// Singleton type (literal types).
    Literal(LitValue),
    /// Engine bailout — member lookup fails closed rather than guessing.
    Unknown,
}
