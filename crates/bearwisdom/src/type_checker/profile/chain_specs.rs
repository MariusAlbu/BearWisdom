// =============================================================================
// type_checker/profile/chain_specs.rs — spec vocabulary for the chain-walk axes: receiver
// dispatch, scope functions, supertype discovery, and kind compatibility.
// =============================================================================

use crate::types::{EdgeKind, SymbolKind};

/// What a scope function yields relative to its receiver, for the chain
/// walker's `scope_functions` miss-fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeYield {
    /// The call yields its own receiver type (Kotlin `apply` / `also`): the
    /// chain continues against the same receiver.
    Receiver,
    /// The call yields the type of its trailing lambda's body (Kotlin `let` /
    /// `run` / `with`). Not inferred generically — listed so the segment
    /// suppresses a chain-miss record instead of being treated as a hard miss.
    LambdaBody,
}

// ---------------------------------------------------------------------------
// Template-include import resolution (data for resolve_via_import_path)
// ---------------------------------------------------------------------------

/// How a delegate wrapper's generic arguments map onto the callback shape it
/// wraps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegateShape {
    /// Every generic argument is a callback PARAMETER (`Action<T1,T2>`).
    AllParams,
    /// The LAST generic argument is the callback's return; the rest are its
    /// parameters (`Func<T1,T2,R>`).
    LastIsReturn,
}

/// How the chain walker's root resolver types a bare `self`/`this` receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfReceiverDiscovery {
    /// The engine's `DefaultRootResolver` behavior: the source symbol's
    /// `scope_path` names the enclosing type, falling back to the file's
    /// single top-level type (SFC / page-class convention). The default for
    /// every language.
    ScopePathThenDefault,
    /// Reserved for frameworks whose `this` has an implicit, framework-declared
    /// type the extractor can't capture as a `scope_path` — seed a canonical
    /// member set and discover the receiver type by its members. Not consumed
    /// today; the variant reserves the axis so a framework can opt in without a
    /// per-language root-resolver hook.
    CanonicalMembers {
        seed: &'static str,
        siblings: &'static [&'static str],
    },
}

/// Component-selector resolution data for template refs (Angular). The engine
/// applies each `name_transform` to the ref target in turn, probes
/// `SymbolLookup::selector_qname` for a matching decorated class, and binds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectorResolution {
    /// Edge kinds whose targets are candidate selectors.
    pub edge_kinds: &'static [EdgeKind],
    /// Name transforms applied to the target, in order, each yielding one
    /// selector-key candidate. The raw target is always tried first.
    pub name_transforms: &'static [NameTransform],
}

/// A deterministic surface-form transform applied to a ref target to derive a
/// selector-key candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameTransform {
    /// `AppUserCard` → `app-user-card`: insert `-` at each interior uppercase
    /// boundary and lowercase. A single-segment lowercase/camelCase input with
    /// no interior uppercase returns unchanged.
    PascalToKebab,
}

/// A wildcard ambient-builtin family: an anchored target folds to one ambient
/// symbol. `prefix` is the literal stem; a target matches only when `prefix` is
/// immediately followed by an ASCII-uppercase character (the upstream
/// `^list[A-Z]` shape), so `listKeys`/`listFoo` match but `list`, `listener`,
/// and `listing` do not. `fold_to` is the ambient symbol name the matched
/// target resolves against (the vendored family base, e.g. `list`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WildcardBuiltin {
    pub prefix: &'static str,
    pub fold_to: &'static str,
}

impl WildcardBuiltin {
    /// `Some(fold_to)` when `target` is `prefix` immediately followed by an
    /// ASCII-uppercase character; `None` otherwise. The uppercase anchor is the
    /// upstream regex shape — `prefix` alone, or `prefix` followed by a
    /// lowercase letter, is a different identifier and does not fold.
    pub fn fold(&self, target: &str) -> Option<&'static str> {
        let rest = target.strip_prefix(self.prefix)?;
        match rest.chars().next() {
            Some(c) if c.is_ascii_uppercase() => Some(self.fold_to),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Name normalization (data for normalize_name in the bare-name strategies)
// ---------------------------------------------------------------------------

/// How a name is normalized before the bare-name binding strategies compare a
/// candidate symbol's name against a ref's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameNormalization {
    /// Identity. The comparison is byte-for-byte — a candidate binds only on
    /// an exact name match. The default for every case-sensitive language.
    None,
    /// Apply the `NormSpec` transform to both sides of the comparison.
    Spec(NormSpec),
}

/// The per-language name-normalization transform, applied identically to the
/// candidate's name and the ref's target before they are compared. Every field
/// is a delta off the identity transform; an all-default spec (`case_insensitive
/// = false` and empty slices) reduces to identity, so only the configured deltas
/// take effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormSpec {
    /// Fold ASCII case before comparing (Pascal, SQL, Fortran, VB).
    pub case_insensitive: bool,
    /// Characters removed anywhere in the name before comparing.
    pub strip_chars: &'static [char],
    /// Leading substrings removed (longest-match-first is the caller's job;
    /// the first that matches as a prefix is stripped).
    pub strip_prefixes: &'static [&'static str],
    /// `(prefix, suffix)` sigil pairs: when the name both starts with `prefix`
    /// and ends with `suffix`, both are stripped (e.g. an interpolation sigil
    /// wrapper). A pair with an empty suffix strips a bare leading sigil.
    pub strip_sigils: &'static [(&'static str, &'static str)],
}

// ---------------------------------------------------------------------------
// Type system axes
// ---------------------------------------------------------------------------

/// Names the trait + associated type a single-inner Deref wrapper uses, so the
/// chain walker can peel a user `impl Deref for C { type Target = Inner }`
/// receiver to its inner type. `trait_name` is the wrapper trait whose
/// supertype edge proves `C` implements it (Rust `"Deref"`); `target_assoc`
/// is the associated-type name whose indexed binding (`field_type["C.Target"]`)
/// names the inner type (Rust `"Target"`). Both are language facts, not member
/// tables — the inner type itself comes from the binding, never from a name
/// lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerefWrapper {
    pub trait_name: &'static str,
    pub target_assoc: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupertypeDiscovery {
    /// Inherits / Implements refs only. Most static-OO languages.
    Explicit,
    /// Structural match between method sets. Go.
    Structural,
    /// Both. TypeScript (interfaces are structural, classes are nominal).
    Both,
}

/// Order in which the arg-carrying member walk visits a type's ancestors.
/// The order decides which override wins when the same member is declared on
/// more than one ancestor: member lookup returns the FIRST kind-compatible
/// match, so an asymmetric multiple-inheritance diamond resolves differently
/// under each order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AncestorOrder {
    /// Breadth-first over the supertype graph. The default — direct parents
    /// before grandparents, left-to-right within a level. Correct for
    /// single-inheritance and linear hierarchies.
    Bfs,
    /// C3 linearization (Python's MRO). A monotonic merge of each parent's own
    /// C3 order with the local parent list, so a parent's full ancestor chain
    /// precedes the next sibling. Diverges from BFS only on asymmetric
    /// multiple-inheritance diamonds.
    C3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchAxis {
    /// Single dispatch on the receiver.
    Receiver,
    /// Multi-dispatch on argument types. R S4, Clojure, Common Lisp, Julia.
    MultiArg,
    /// Return-type dispatch. Haskell typeclass instance.
    ReturnType,
}

/// The structural shape of a built-in container, naming where its element data
/// lives in the receiver's `Apply` args. A `container_accessors` entry binds a
/// method name to one of these so the engine projects the right slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerShape {
    /// A linear collection — `Array<T>` / `ReadonlyArray<T>` / `Set<T>` — whose
    /// element is `args[0]`.
    Sequence,
    /// An associative collection — `Map<K, V>` — whose key is `args[0]` and
    /// value is `args[1]`.
    Map,
}

/// Which structural slot of a container a `container_accessors` method yields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessorSlot {
    /// `Sequence` element / the single iterated type — `args[0]`.
    Element,
    /// `Map` key — `args[0]`.
    Key,
    /// `Map` value — `args[1]`.
    Value,
}

/// How a bare (unqualified) receiver type encountered mid-chain is promoted to
/// its package-qualified qname before member lookup. Members are keyed under
/// the fully package-qualified qname (`com.foo.Repository.findOne`), while a
/// receiver typed by a simple name (`Repository`, or a method's same-package
/// return type `Entity`) carries only the bare head — the walker can't step
/// past it until the bare name is qualified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainQualification {
    /// No mid-chain qualification. The receiver's qname is used verbatim. The
    /// default for every language whose members are keyed under the same
    /// (bare or already-qualified) name the receiver type carries.
    None,
    /// Promote a bare receiver to its package-qualified qname via two
    /// deterministic sources, tried in order: (1) same-package — the previous
    /// receiver's package (a method's same-package return type) or, at the
    /// root, the file's own package; (2) the file's explicit non-wildcard
    /// imports (`import com.foo.Bar` makes a receiver typed `Bar` resolve under
    /// `com.foo.Bar`). Only promotes to a qname that owns a type or keys a
    /// member, so it can only widen resolution. Java / Groovy / C# / PHP.
    SamePackageAndImports,
    /// An import names a PACKAGE, not a type, and members are keyed under the
    /// import's short name (`import "github.com/gin-gonic/gin"` brings short
    /// name `gin`; the function lands as `gin.NewRouter`). A bare member ref
    /// whose qualifier the extractor dropped resolves under
    /// `{import.imported_name}.{target}` — or, for an aliased import, under
    /// `{last_path_segment}.{target}`. Distinct from `SamePackageAndImports`,
    /// where the import names the class itself. Go.
    PackageShortName,
}

/// Edge-kind × symbol-kind compatibility entries. An empty table means "any
/// EdgeKind accepts any SymbolKind."
pub type KindTable = &'static [(EdgeKind, &'static [SymbolKind])];

/// Permissive default for languages without bespoke rules.
pub const PERMISSIVE_KIND_TABLE: KindTable = &[];

/// Helper for callers that need to ask "is this symbol kind valid for this
/// edge kind?" against a KindTable.
pub struct KindCompatibility;

impl KindCompatibility {
    /// True when `sym_kind` is allowed as a resolution target for `edge_kind`
    /// under `table`. Empty tables accept everything.
    pub fn check(table: KindTable, edge_kind: EdgeKind, sym_kind: SymbolKind) -> bool {
        if table.is_empty() {
            return true;
        }
        for (ek, kinds) in table.iter() {
            if *ek == edge_kind {
                return kinds.iter().any(|k| *k == sym_kind);
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Syntax axes
// ---------------------------------------------------------------------------

