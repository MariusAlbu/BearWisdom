// =============================================================================
// type_checker/profile/language_profile.rs — LanguageProfile + sub-structs
//
// Spec: research/architecture/03-language-profile-spec.html
//
// All 17 axes (11 engine-side, 5 extractor-side, 1 hybrid). Profile data is
// 'static — every concrete profile is a `static const` value referenced by
// the registry. The conservative fallback lives in `default_profile.rs`. The
// import/module-resolution axes are composed under `imports: ImportAxes`
// (`import_axes.rs`); every other axis is a direct field on this struct.
// =============================================================================

use crate::types::{EdgeKind, SymbolKind, Visibility};

use super::super::core::types::PrimKind;

pub use super::chain_specs::*;
pub use super::default_profile::DEFAULT_PROFILE;
pub use super::import_axes::*;
pub use super::import_specs::*;
pub use super::syntax_specs::*;

// ---------------------------------------------------------------------------
// Top-level struct
// ---------------------------------------------------------------------------

/// How far apart two same-qname type declarations may sit and still be ONE
/// logical type (Roslyn-style declaration merging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeScope {
    /// No declaration merging — same-qname declarations are distinct types.
    None,
    /// Merge only within one file (TS `interface Foo` + `namespace Foo`;
    /// module scoping makes same-name declarations in other files distinct).
    SameFile,
    /// Merge across files within one package (C# partial classes — qnames
    /// are namespace-qualified, so one qname is one type per package).
    SamePackage,
}

pub struct LanguageProfile {
    // === Identity ===
    pub id: &'static str,
    pub qname_separator: &'static str,
    /// Declaration-merging reach for this language's type declarations.
    pub declaration_merging: MergeScope,
    pub self_keywords: &'static [&'static str],

    // === Type system (engine) ===
    pub supertype_discovery: SupertypeDiscovery,
    /// Order the arg-carrying member walk (`find_on_chain`) visits ancestors.
    /// `Bfs` (the default) keeps the breadth-first walk every language used
    /// before this axis; `C3` opts a multiple-inheritance language (Python)
    /// into C3 linearization so an asymmetric diamond resolves the same
    /// override the runtime would.
    pub ancestor_order: AncestorOrder,
    pub members_can_be_external: bool,
    pub dispatch_axis: DispatchAxis,
    pub has_generics: bool,
    pub has_sum_types: bool,
    pub look_through_optional: bool,
    pub literal_narrowing: bool,
    pub async_wrappers: &'static [&'static str],
    /// Built-in container accessor methods that type THROUGH the container's
    /// element/value. Each entry is `(method, shape, slot)`: the method name
    /// (`pop`, `get`), the container shape it applies to (`Sequence` / `Map`),
    /// and which structural slot the call yields (`Element`, `Key`, `Value`).
    /// The yielded type is projected from the receiver's `Apply` ARGS — a
    /// `Sequence` element is `args[0]`, a `Map` value is `args[1]` — so the
    /// element comes from structure, never from a hardcoded method→type table.
    /// `&[]` (the default) leaves container accessors as ordinary member
    /// lookups: with no entry they don't project, so a chain past one misses
    /// unless a real member of that name exists.
    pub container_accessors: &'static [(&'static str, ContainerShape, AccessorSlot)],
    /// Single-inner smart-pointer type heads whose member set is the pointed-to
    /// value's. The chain walker peels a `Type::Apply { base, args }` with one
    /// arg whose `base` simple-name is listed here to `args[0]` at the chain
    /// root, before member lookup — these wrappers `Deref` to their single
    /// inner, so peeling the structural `args[0]` IS that one Deref hop and the
    /// existing member walk then applies to the inner type. DATA, not a
    /// method→type table: the peel is the structural `args[0]` projection and
    /// the names only gate WHICH `Apply` is a single-inner wrapper. A real
    /// container (`Vec`, `HashMap`) must be ABSENT so its accessors stay on the
    /// container. `&[]` (the default) leaves every `Apply` receiver intact.
    pub single_inner_wrappers: &'static [&'static str],
    /// Container heads whose MISSED member lookups retry against a Deref-target
    /// head. Each entry maps a container type head to the head its built-in
    /// `Deref` impl exposes (`Vec` → `slice`, `String` → `str`, `PathBuf` →
    /// `Path`). NOT a peel: `single_inner_wrappers` REPLACES the type with its
    /// single argument, while this REHEADS — the applied type arguments are
    /// kept and only the head changes, so an element-generic member found on
    /// the target still substitutes the container's args (`Vec<T>.iter()`
    /// retries as `slice<T>.iter()` and yields `Iter<'_, T>` with `T` bound).
    /// The container's OWN members always win: the chain walker consults this
    /// map only after member lookup on the container has missed, so `Vec.push`
    /// never resolves to a `slice.push`. `&[]` (the default) leaves every
    /// member miss a miss.
    pub container_deref_targets: &'static [(&'static str, &'static str)],
    /// Type names whose members a FUNCTION VALUE carries — a method accessed
    /// without a call (`this.m.bind(this)`) resolves `bind`/`call`/`apply` on
    /// these declarations. The member SET comes from the indexed lib, never a
    /// hardcoded list; these names only say where a function value's prototype
    /// lives. `&[]` (the default) leaves an uncalled method's member misses as
    /// ordinary misses.
    pub function_prototype_types: &'static [&'static str],
    /// A user-defined single-inner Deref wrapper: a type `C` with an
    /// `impl Deref for C { type Target = Inner }` exposes Inner's member set on
    /// a `C` receiver (Rust autoderef). Unlike `single_inner_wrappers` — which
    /// peels a generic type ARG (`args[0]`) off a known std pointer head — this
    /// peels the Deref `Target` BINDING off an arbitrary user type. The chain
    /// walker fires it only when BOTH hold: `C` has a supertype edge reaching
    /// `trait_name` (the structural impl evidence), and the indexed
    /// `field_type["{C}.{target_assoc}"]` binding exists. The Target type is
    /// read from that already-indexed binding, never from a name table, and the
    /// edge requirement keeps it widening-only (no bare-name coincidence). The
    /// trait + assoc names are language DATA. `None` (the default) leaves every
    /// receiver intact.
    pub deref_wrapper: Option<DerefWrapper>,
    pub iterator_method: Option<&'static str>,
    pub primitive_mapping: &'static [(&'static str, PrimKind)],
    pub kind_compatible_table: KindTable,
    pub chain_qualification: ChainQualification,
    /// Names the generic resolver must treat as language builtins/primitives:
    /// scalar types (`int32`, `Int`), built-in functions (`subst`, `wildcard`),
    /// operators, and reserved namespace prefixes (`builtins.`, `cmake_`). When
    /// this returns true for a ref's target, the engine declines without running
    /// the strategy ladder and without recording a chain miss — the ref is left
    /// for external classification rather than bound to a project symbol that
    /// happens to share the name. `None` (the default) runs the ladder for every
    /// target, so languages with no reserved-name space are unaffected.
    pub builtin_skip: Option<fn(&str) -> bool>,
    /// File-namespace-gated decline, the two-key sibling of `builtin_skip`.
    /// `builtin_skip` keys only on the target string; this also requires the
    /// resolving file's `file_namespace` to equal a sentinel before declining.
    /// `Some` for languages where a name is reserved only inside a specific
    /// kind of file (R-package native C sources naming the R C API), so the
    /// same name binds normally everywhere else. `None` (the default) leaves
    /// it inert. Declines before the strategy ladder so no same-named project
    /// symbol binds; external classification brands the target afterward. See
    /// `NamespaceDecline`.
    pub namespace_decline: Option<NamespaceDecline>,
    /// Import/module-resolution axes: how import statements bind, module
    /// paths anchor, and namespaces scope. See `ImportAxes` (`import_axes.rs`).
    pub imports: ImportAxes,
    /// Module-string decline, the module-keyed sibling of `builtin_skip`.
    /// `builtin_skip` keys on the ref's TARGET string; this keys on the ref's
    /// extractor-set `module` string. When a ref carries a `module` and this
    /// returns true for it, the engine declines before the strategy ladder runs
    /// — the same pre-ladder decline as `builtin_skip`, so no same-named project
    /// symbol binds and external classification brands the ref afterward. `Some`
    /// for languages whose module specifiers name a non-project provider that
    /// the ladder must never bind through (SCSS `@use` of a Sass built-in module
    /// `sass:math`, or a synthesized CSS-function-call hint). `None` (the
    /// default) leaves every module-carrying ref on the ladder.
    pub module_skip: Option<fn(&str) -> bool>,
    /// Namespace-alias prefixes the engine strips from a dotted target before
    /// the ambient-package lookup. A target `{prefix}.{leaf}` whose `{prefix}`
    /// is one of these is rewritten to `{leaf}` for that one strategy, so a
    /// member exposed under an aliased namespace resolves against the bare
    /// ambient symbol (Bicep `sys.concat` / `az.resourceId` → `concat` /
    /// `resourceId`). `&[]` (the default) leaves every dotted target intact.
    pub ambient_namespace_prefixes: &'static [&'static str],
    /// Wildcard ambient builtins: an upstream runtime registers a family of
    /// builtins under one regex stem rather than a finite name set, so the
    /// concrete callees aren't enumerable as symbols. Each entry folds an
    /// anchored target to the single ambient symbol that stands for the family
    /// (Bicep's `az` `list*` regex overload → the vendored `list` symbol). The
    /// engine matches `{prefix}` immediately followed by an ASCII-uppercase
    /// char (`listConnectionStrings`, `listFoo` — never `list`, `listener`, or
    /// `listing`), then retries the ambient-package probe under `fold_to`. It
    /// fires after the bare ambient-package and namespace-alias-strip rungs, so
    /// a concrete same-named ambient symbol always binds first. `&[]` (the
    /// default) leaves the rung inert — a non-bicep language's `list*` call is
    /// unaffected. See `WildcardBuiltin`.
    pub wildcard_builtins: &'static [WildcardBuiltin],
    /// How a candidate symbol's name is normalized before it is compared
    /// against a ref's target in the bare-name binding strategies (same-file
    /// sibling, scope-visible). `None` (the default) is the identity transform
    /// — a candidate binds only on a byte-for-byte name match, so every
    /// case-sensitive language is unaffected. `Spec` folds case and/or strips
    /// sigils, prefixes, and characters before comparing, so a case-insensitive
    /// language (Pascal, SQL, Fortran, VB) or a sigil-carrying one binds a
    /// reference written in a different surface form. See `NameNormalization`.
    pub name_normalization: NameNormalization,
    /// Nominal DELEGATE wrappers whose generic arguments carry a callback's
    /// parameter types — `Action<T1,T2>` (every argument is a parameter),
    /// `Func<T1,R>` (the last argument is the return). The lambda seeder
    /// unwraps a callee parameter of this shape into the function type it
    /// wraps, so an un-annotated lambda argument's parameters seed from the
    /// delegate's arguments. Empty (the default) leaves nominal callee
    /// parameters opaque.
    pub delegate_wrappers: &'static [(&'static str, DelegateShape)],
    /// Simple names of the language's implicit root type — the base every
    /// declaration inherits without writing it (`Object`/`object` for the
    /// CLR). Closes both climbs at the root: the member walk resolves a missed
    /// member on the root's own declaration, and the extension-method receiver
    /// climb appends these names. Empty disables both probes.
    pub implicit_root_types: &'static [&'static str],
    /// Namespaces the compiler brings into scope without an explicit import.
    /// `ImplicitPreludeRule` binds a bare target that is a DIRECT member of one
    /// of these namespaces (an extra `qname_separator` segment beyond the
    /// namespace is a nested type/method and stays excluded — the compiler
    /// demands an explicit import to reach it). Two candidates under different
    /// listed namespaces both matching the same bare name is treated as
    /// ambiguous and declined rather than guessed. `&[]` (the default) leaves
    /// the rule inert for a language with no compiler-implicit namespace.
    pub implicit_prelude_namespaces: &'static [&'static str],
    /// Compiler-generated prefixes decorating a case/variant's declared name in
    /// the compiled form the index sees — F# discriminated-union cases compile
    /// to `New<Case>` static factory methods (`Ok` → `NewOk`). `ImplicitPreludeRule`
    /// probes `<prefix><target>` under `implicit_prelude_namespaces`, tried only
    /// after the bare target itself misses as a direct namespace member. `&[]`
    /// (the default) leaves the probe inert.
    pub compiled_name_prefixes: &'static [&'static str],
    /// Ambient npm/test-framework/core-lib globals probed for a bare
    /// single-identifier call/typeref/instantiation that no import binds.
    /// `Off` (the default) leaves the probe inert. `On` checks the synthetic
    /// `__npm_globals__.<name>` namespace and the bare qname when the defining
    /// file is an ambient-global lib file (test-runner globals, library globals
    /// installed under `$`, DOM constructors). See `AmbientGlobals`.
    pub ambient_globals: AmbientGlobals,
    /// How the chain walker's root resolver discovers the type a bare `self`/
    /// `this` receiver refers to. `ScopePathThenDefault` (the default) is the
    /// engine's `DefaultRootResolver` behavior — the source symbol's
    /// `scope_path`, then the file's single top-level type. `CanonicalMembers`
    /// is reserved for frameworks whose `this` has an implicit declared type.
    /// See `SelfReceiverDiscovery`.
    pub self_receiver_discovery: SelfReceiverDiscovery,
    /// Flat-global by-name binding for a language with NO imports, namespace, or
    /// scope structure. `Off` (the default) leaves the strategy inert. `Global`
    /// binds a bare target to the FIRST kind-compatible, project-internal symbol
    /// of the same name across the whole project. `DirectoryScoped` adds a
    /// sibling-directory filter: the candidate binds only when it lives in the
    /// same directory as the referencing file. Unlike
    /// `resolve_via_unique_internal_name`, which declines on more than one
    /// candidate, both variants first-match-bind — duplicate names are common and
    /// there is no structure to disambiguate. Runs LAST in the ladder, after
    /// every structural rung, so any structural evidence wins. See
    /// `NamespaceScope`.
    pub namespaceless_global_type_lookup: NamespaceScope,
    /// Explicit-member submodule import binding. `false` (the default) leaves
    /// the strategy inert. `true` opts in a language whose import statement can
    /// name a symbol AND its enclosing module together (Swift
    /// `import struct MyModule.Bar`): a bare ref to that symbol binds to the
    /// UNIQUE internal symbol of the same name and compatible kind, gated on an
    /// `ImportEntry` whose dotted `module_path` ends in the imported name. A
    /// plain whole-module import (`import Foundation`, no dot) does NOT arm it —
    /// that form has no project-symbol scope and is left for external
    /// classification. Runs just before `resolve_via_file_import`.
    pub explicit_member_import: bool,
    /// Component-selector resolution for template refs. `None` (the default)
    /// leaves it inert. `Some` binds a `Calls` ref whose target names a
    /// component/directive selector to the decorated class via
    /// `SymbolLookup::selector_qname`, applying the configured name transforms
    /// (e.g. `PascalToKebab` for `<app-user-card>` → `app-user-card`). See
    /// `SelectorResolution`.
    pub selector_resolution: Option<SelectorResolution>,
    /// Multi-candidate disambiguation by ranking. `false` (the default) leaves
    /// the rung inert — when several same-name, kind-compatible candidates
    /// survive every structural rung, the ladder declines rather than guess.
    /// `true` opts a language into `resolve_via_ranked_candidates`: the last
    /// ladder rung scores the candidate set on data the resolver already has
    /// (same workspace package, file-path proximity, visibility, ambient path)
    /// and binds the top only when it beats the runner-up by `RANK_MARGIN`,
    /// else declines on a tie. Sits AFTER every structural rung and after
    /// `resolve_via_unique_internal_name`, so any single-candidate or
    /// structural evidence wins first. For a language whose bare overload sets
    /// (same-name functions across a unit's include files, a type used
    /// function-style) have no import or scope signal to separate them.
    pub multi_candidate_ranking: bool,
    /// Receiver-threading scope functions: stdlib higher-order calls that yield
    /// a type structurally derived from their receiver rather than from a
    /// declared return. `&[]` (the default) leaves the chain walker's member
    /// lookup unchanged — every segment must resolve to a real member. A
    /// non-empty list opts a language in (Kotlin `apply`/`also`/`let`/`run`):
    /// these are unindexed stdlib extensions, so a mid-chain `x.apply { … }`
    /// segment misses ordinary member lookup and the chain dies before its real
    /// tail. When a CALL segment's name matches a `Receiver`-yielding entry and
    /// member lookup has missed, the walker keeps the receiver type and advances
    /// past the segment, so `x.apply { … }.realMember()` types `realMember`
    /// against `x`. Strictly a miss-fallback — a real member of the same name
    /// always wins first — so it can only widen. `LambdaBody`-yielding entries
    /// (`let`/`run`/`with`) are listed for completeness but only suppress a
    /// chain-miss record; the body type is not inferred generically.
    pub scope_functions: &'static [(&'static str, ScopeYield)],
    pub overload_pick_all: bool,
    /// Argument-dependent lookup. `false` (the default) leaves the probe inert.
    /// `true` opts a language in (C++): when the regular bare-name ladder
    /// declines a bare `Calls`/`Instantiates` ref, the engine resolves each
    /// call argument's type to a qname, takes its declaring-namespace prefix,
    /// and probes `{namespace}.{target}` for a kind-compatible callable — binding
    /// the unique survivor (after the BIND-4 arity/type filter when several
    /// namespaces contribute candidates). A bare call to a free function declared
    /// in the namespace of one of its argument types resolves even though no
    /// import / scope / using brings the function into scope. Strictly a fallback
    /// after the structural ladder, so scope/import evidence always wins.
    pub argument_dependent_lookup: bool,
    /// Associated-type projection (Rust `Self::Output` / `<C as Trait>::Item`).
    /// `false` (the default) leaves the projection inert — the raw `Self::Output`
    /// string interns as an opaque class with no members, so a chain cannot type
    /// past an associated-type-returning method. `true` opts a language in: in the
    /// string-fallback arm of `yield_type_of`, when a method/field's resolved type
    /// string is a `<head>::<Assoc>` qualified path whose head resolves to the
    /// current receiver's concrete qname C (a `self_keyword` head pins C; a
    /// `<C as Trait>` angle head extracts the inner C; a bare head equal to C),
    /// the engine projects `field_type_name("{C}.{Assoc}")` — the impl's real
    /// `type Assoc = Concrete` binding already in the index — and yields Concrete.
    /// A head that is neither a self-keyword nor C declines (a plain `module::Foo`
    /// path return is never hijacked), and a missing binding declines to the prior
    /// raw-intern behavior, so the projection is strictly widening.
    pub associated_type_projection: bool,
    /// Blanket-impl graph population (Rust `impl<U: Bound> Trait for U {}`).
    /// `false` (the default) leaves the supertype builder single-pass — a
    /// blanket impl's self-TypeRef names the impl's own generic param (`U`),
    /// which resolves to no concrete type, so no `C → Trait` edge ever forms and
    /// the blanket trait's default methods are unreachable. `true` opts a
    /// language into a second `build_explicit` pass: each concrete type C that
    /// provably satisfies Bound (its supertype graph reaches the resolved Bound
    /// trait) gains a `C → Trait` edge, so the existing member walk binds the
    /// trait's default methods on C. Widening-only — an edge forms ONLY when
    /// `walk_up(C)` reaches the bound; an unhydrated or unsatisfied bound
    /// declines (no edge), never a coincidental same-name bind. The two-pass
    /// split is inert when off: pass 2 runs only under this gate, so a language
    /// without it produces byte-identical edges.
    pub blanket_impl_resolution: bool,

    // === Syntax (extractor) ===
    pub constructor_patterns: &'static [ConstructorPattern],
    pub class_builder_specs: &'static [ClassBuilderSpec],
    pub decorator_syntax: Option<DecoratorSyntax>,
    pub doc_comment_kinds: &'static [&'static str],
    pub visibility_keywords: &'static [(&'static str, Visibility)],
}

#[cfg(test)]
#[path = "language_profile_tests.rs"]
mod tests;
