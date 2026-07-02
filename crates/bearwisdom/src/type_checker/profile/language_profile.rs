// =============================================================================
// type_checker/profile/language_profile.rs — LanguageProfile + sub-structs
//
// Spec: research/architecture/03-language-profile-spec.html
//
// All 17 axes (11 engine-side, 5 extractor-side, 1 hybrid). Profile data is
// 'static — every concrete profile is a `static const` value referenced by
// the registry. DEFAULT_PROFILE is the conservative fallback applied to
// languages with no bespoke profile yet.
// =============================================================================

use crate::types::{EdgeKind, SymbolKind, Visibility};

use super::super::core::types::PrimKind;

// ---------------------------------------------------------------------------
// Top-level struct
// ---------------------------------------------------------------------------

pub struct LanguageProfile {
    // === Identity ===
    pub id: &'static str,
    pub qname_separator: &'static str,
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
    /// Import-prefix decline, the import-set-keyed sibling of `namespace_decline`.
    /// When set and the target carries the `qname_separator` and its leading
    /// segment (sigil-stripped per `self_keywords` / a leading `$`) equals any
    /// of the file's import module paths, the engine declines before the
    /// strategy ladder — a qualified reference into a declared dependency module
    /// is external, not a project symbol, so no same-named local binds and
    /// external classification brands it after. `false` (the default) leaves it
    /// inert. This is the one decline shape `namespace_decline` can't express:
    /// the gate is the target's own leading namespace segment against the
    /// import set, not the resolving file's namespace.
    pub decline_qualified_when_prefix_imported: bool,
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
    /// Template-include import resolution. `Some` for languages whose
    /// `Imports` refs name another template FILE by relative path / stem
    /// (handlebars partials, EJS / Pug / Nunjucks includes, GSP renders,
    /// markdown relative links, YAML `uses`) rather than a symbol. `None`
    /// (the default) leaves the import-path strategy off, so a language's
    /// `Imports` refs flow through the regular ladder untouched. The data
    /// here fully describes the candidate-path generation and binding rule —
    /// see `ImportResolution`.
    pub import_resolution: Option<ImportResolution>,
    /// How the generic `build_file_context` default fills each
    /// `ImportEntry::module_path` from an `Imports` ref. `None` leaves it
    /// empty; `EchoTarget` mirrors the raw target into the module path
    /// (Nunjucks). Only consulted when no language hook builds the file
    /// context.
    pub import_module_path: ImportModulePath,
    /// Module-anchored binding for a ref whose extractor-set `module` is
    /// `Some`. `Off` (the default) leaves the strategy inert; `On(bind)`
    /// resolves the `module` to project symbols and binds `target` by the
    /// chosen rule. See `ModuleAnchor` / `ModuleAnchorBind`.
    pub module_anchor: ModuleAnchor,
    /// When a NON-`Imports` ref carries `module` and the module-anchor probe
    /// misses, terminate the ladder with `None` instead of falling through to
    /// the bare-name strategies. Guards an external / unindexed prefix from
    /// being hijacked by a same-named local homonym (Dart library prefixes).
    /// `false` (the default) lets the ladder continue after an anchor miss.
    pub module_anchor_terminal: bool,
    /// Which `module` specifiers the anchor treats as RELATIVE (resolved via
    /// `in_module_from`) versus ABSOLUTE (resolved via `ByNameUnderModuleDir`).
    /// `None` (the default) makes every module relative — always run the
    /// configured `ModuleAnchorBind`. `DotPrefix` / `DotSlashPrefix` split a
    /// dot- / dot-slash-prefixed module to the bind rule and route everything
    /// else through the directory-containment probe (Python `.foo` vs
    /// `models.X`).
    pub relative_marker: RelativeMarker,
    /// Import-scoped binding of a bare target to an EXTERNAL symbol. `None`
    /// (the default) keeps the engine off externals — the regular ladder binds
    /// only project symbols; `Some` opts a language in (Ruby gems). See
    /// `ExternalByImport`.
    pub external_by_import: Option<ExternalByImport>,
    /// How a candidate symbol's name is normalized before it is compared
    /// against a ref's target in the bare-name binding strategies (same-file
    /// sibling, scope-visible). `None` (the default) is the identity transform
    /// — a candidate binds only on a byte-for-byte name match, so every
    /// case-sensitive language is unaffected. `Spec` folds case and/or strips
    /// sigils, prefixes, and characters before comparing, so a case-insensitive
    /// language (Pascal, SQL, Fortran, VB) or a sigil-carrying one binds a
    /// reference written in a different surface form. See `NameNormalization`.
    pub name_normalization: NameNormalization,
    /// The module boundary a bare same-module reference binds within when no
    /// import and no chain root the target. `Off` (the default) leaves the
    /// rung inert. `SameDir` makes the file's parent directory the module
    /// (Odin/MATLAB same-package references — no `module` to anchor on),
    /// first-match. `SameDirUnique` is the same dir boundary but binds only on
    /// exactly one in-dir candidate (a unit split across sibling include files
    /// that also carries overload sets). `SourcesTargetSubtree` spans a whole
    /// `Sources/<Target>/` subtree (Swift whole-module compilation). The rung
    /// runs module-independently near the end of the ladder; the `SameDir` arm
    /// first-matches, the other arms bind only when EXACTLY ONE kind-compatible
    /// internal declaration is in-module.
    pub module_scope: ModuleScope,
    /// How `resolve_via_wildcard_import` decides a candidate sits under a
    /// wildcard import's module. `QnameUnder` (the default) keeps the current
    /// qname-prefix test (`{module}.{name}`, exactly one segment deeper).
    /// `FileStem` instead matches by the candidate's FILE — its basename-stem or
    /// a path dir-segment equals the import's module name — under
    /// `name_normalization` for the name comparison, with an optional
    /// `{stem}_`-prefixed include-file probe. See `WildcardMatch`.
    pub wildcard_match: WildcardMatch,
    /// How the import-scoped external bind (`resolve_via_external_by_import`)
    /// matches an external candidate's file against the file's imports.
    /// `PkgSegment` (the default) keys on the `ext:<lang>:<pkg>` package segment
    /// equalling an import root. `FileStemOrDir` keys on the external file's
    /// basename-stem / dir-segment matching an import leaf or package (Nim,
    /// whose externals are named by file rather than `ext:`-package). See
    /// `ExtMatch`. Only consulted when `external_by_import` is `Some`.
    pub ext_match: ExtMatch,
    /// Head-of-target alias binding. A dotted target's HEAD (the segment before
    /// the first `.`) names an in-file declaration the rest of the target reads
    /// against — an HCL provider-alias block (`google.compute_instance` where
    /// `google` is a `provider` block declared in the file). When set, the
    /// engine truncates the target at the first `.`, declines a head that is
    /// empty or carries a `_` (a provider RESOURCE type, not an alias), and
    /// binds the head to an in-file symbol of the configured kind.
    /// `Off` (the default) leaves the strategy inert. See `HeadAliasBind`.
    pub head_alias: HeadAliasBind,
    /// File-scoped import binding. An import whose `module_path` names a FILE
    /// (a `.robot` / `.resource` resource import, a Python library file) brings
    /// that file's members into bare-name scope; a bare target binds to a
    /// kind-compatible symbol in the imported file whose name matches under
    /// `name_normalization`. `Off` (the default) leaves the strategy inert.
    /// `On { wildcard_only }` opts a language in, optionally restricting the
    /// scan to wildcard imports. See `FileScopedImports`.
    pub file_scoped_imports: FileScopedImports,
    /// Whether a bare target equal to an import's bound name binds to the MODULE
    /// symbol whose qname IS the import's full module path. A namespace-qualified
    /// import (`alias MyApp.Foo`) brings the bare `Foo` into scope bound to the
    /// module `MyApp.Foo` itself, not a member under it. `false` (the default)
    /// leaves the strategy inert; `true` opts in namespace-qualified-import
    /// languages (Elixir, and any sibling whose aliases map a local name to a
    /// full module qname through the import table).
    pub alias_module_qname: bool,
    /// Alternate module-prefix candidates the `ByNameUnderModuleDir` anchor
    /// tries against a BARE specifier before the directory-containment probe.
    /// `Off` (the default) leaves the anchor's bare-specifier handling on the
    /// literal `{module}.{target}` qname plus directory containment. `On` runs
    /// a deterministic prefix-rewrite generator (DefinitelyTyped `@types/`
    /// rewrites + deep-import `/seg` peels) and, when set, declines the
    /// directory-containment fallback for bare specifiers so a bare package
    /// name (`react`) never directory-matches a same-named project file. See
    /// `ModulePrefixRewrites`.
    pub module_prefix_rewrites: ModulePrefixRewrites,
    /// Whether a bare import specifier that names a sibling WORKSPACE package
    /// scopes the bare target to that package's symbol set. `false` (the
    /// default) leaves the strategy inert. `true` opts a language in (TS/JS
    /// monorepos): the engine reads `SymbolLookup::workspace_package_id` /
    /// `symbols_in_package` / `is_workspace_declared_name` to bind the target —
    /// including deep imports (`@org/utils/sub/mod`) — at confidence 1.0.
    pub workspace_packages: bool,
    /// Basename stems (extension dropped) that count as a package's re-export
    /// barrels for `workspace_pkg_barrels` — the files whose `pub`/`export`
    /// re-exports the workspace-package rung follows. `["index"]` for JS/TS
    /// (`index.ts`); Rust uses its crate roots (`lib`, `main`). Empty leaves
    /// the rung's barrel discovery inert.
    pub reexport_barrel_stems: &'static [&'static str],
    /// The keyword a module specifier's leading segment uses to mean "this
    /// file's own package root", distinct from a declared sibling package
    /// name (Rust's `crate`, resolved against the ref's `file_package_id`
    /// rather than a `workspace_pkg_by_declared_name` lookup). `None` (the
    /// default) leaves every specifier resolved by declared name only.
    pub self_package_root: Option<&'static str>,
    /// Whether a bare name under a `*` wildcard import (`use pkg::*;`) that
    /// names a workspace package — sibling by declared name, or this file's
    /// own via `self_package_root` — brings that package's symbols into bare
    /// scope. `false` (the default) leaves the rung inert. `true` opts in a
    /// language whose glob imports genuinely open unqualified scope AND whose
    /// qualified names don't carry the module path a `wildcard_match` string
    /// test needs (Rust: `qualified_name` is file-nesting-only, so a name
    /// reached through a `pub use` re-export from a deeper submodule has no
    /// prefix in common with the glob's module path). Resolved the same way
    /// `workspace_packages` binds an explicit import — package id + optional
    /// sub-path file-substring match, never a `qualified_name` string test —
    /// and only when EXACTLY ONE candidate matches across the file's globs.
    pub wildcard_workspace_scope: bool,
    /// Whether the `ByNameUnderModuleDir` qname probe and `resolve_via_qname_exact`
    /// scan ALL same-qname overloads for the first kind-compatible one rather
    /// than taking `by_qualified_name`'s first-wins winner. `false` (the
    /// default) keeps first-wins. `true` opts a language in for declaration
    /// merging — TS exposes interface + variable under one qname (a decorator
    /// that is both a callable and a same-named interface), and a `Calls` ref must
    /// skip the interface to reach the callable variable. Generic correctness fix,
    /// scoped by the flag so first-wins languages are byte-identical.
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

    // === Syntax (extractor) ===
    pub constructor_patterns: &'static [ConstructorPattern],
    pub class_builder_specs: &'static [ClassBuilderSpec],
    pub decorator_syntax: Option<DecoratorSyntax>,
    pub doc_comment_kinds: &'static [&'static str],
    pub visibility_keywords: &'static [(&'static str, Visibility)],
}

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

/// Per-language data for the generic template-include resolver. Every field
/// is a per-language delta of one shared algorithm: from an `Imports` ref
/// whose `target_name` is a relative-path / stem reference to another
/// template file, generate candidate project file paths and bind the first
/// one whose in-file symbol matches `bind_kind` under `stem_match`.
#[derive(Debug, Clone, Copy)]
pub struct ImportResolution {
    /// File extensions to try when the raw target carries none, in order.
    /// A target already ending in one of these is taken verbatim.
    pub extensions: &'static [&'static str],
    /// Where to look for the target relative to the source file's directory.
    pub candidate_dirs: CandidateDirs,
    /// Directory-index entry stems probed inside a `target`-named directory
    /// (`base.join(entry).join(ext)`): `[]`, `["index"]`,
    /// `["index", "README", ...]`, `["action"]`. Empty disables the probe.
    pub index_files: &'static [&'static str],
    /// Also probe the `_{stem}` sibling of each candidate (partial-file
    /// convention).
    pub underscore_variant: bool,
    /// Also probe a kebab-cased form of the target (`UserCard` → `user-card`).
    pub kebab_variant: bool,
    /// Decline a leading-slash target outright (views-root-relative with no
    /// anchor — guessing would mis-bind).
    pub decline_leading_slash: bool,
    /// How a candidate file's in-file symbol name is matched against the
    /// candidate path.
    pub stem_match: StemMatch,
    /// The symbol kind a candidate file's binding symbol must be.
    pub bind_kind: &'static str,
    /// The `Resolution::strategy` tag recorded on a hit (preserves the
    /// per-language diagnostic strings).
    pub strategy_tag: &'static str,
}

/// Where the import-path resolver looks for a target relative to the source
/// file's directory. The source-dir candidates are always emitted; this
/// governs whether a parent-walk over named subdirectories is added on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDirs {
    /// Source directory only.
    SelfDir,
    /// Walk parent directories up to `depth` levels (the source dir counts as
    /// level 0), joining each of `dirs` before the target variant — the
    /// handlebars partials-directory walk.
    WalkUp {
        dirs: &'static [&'static str],
        depth: usize,
    },
}

/// How a candidate file's binding symbol name is matched against the
/// candidate path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemMatch {
    /// `sym.name == file_stem` (filename without extension).
    StemExact,
    /// `sym.name == stem` OR `sym.name == stem.trim_start_matches('_')`.
    StemOrUnderscoreStripped,
    /// `sym.name == file_name` (basename including extension).
    BasenameWithExt,
    /// No name check — accept any `bind_kind` symbol in the candidate file
    /// (GSP: the partial's class is the only such symbol).
    AnyClassInFile,
}

/// How the generic `build_file_context` default fills an `ImportEntry`'s
/// `module_path` from a ref's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportModulePath {
    /// Leave `module_path` empty. Collects only `EdgeKind::Imports` refs.
    None,
    /// Mirror the raw target into `module_path`. Collects only `Imports` refs.
    EchoTarget,
    /// Harvest every ref that carries a `module` field — regardless of edge
    /// kind — into an `ImportEntry { imported_name: target, module_path:
    /// module }`. The TS/JS extractor emits one `TypeRef`-with-module ref per
    /// imported binding (`import { useState } from 'react'`), and the post-pass
    /// attaches a `module` to call refs (`UserService.findOne()` →
    /// `module="./user.service"`); both shapes are the file's import table.
    FromModuleField,
}

// ---------------------------------------------------------------------------
// Module-anchored binding (data for resolve_via_module_anchor)
// ---------------------------------------------------------------------------

/// Whether a ref's extractor-set `module` drives a module-anchored bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleAnchor {
    /// No module-anchored binding. The default for every language whose
    /// module-carrying refs resolve through the regular ladder.
    Off,
    /// Resolve `module` to project symbols and bind `target` by the rule.
    On(ModuleAnchorBind),
}

/// How `resolve_via_module_anchor` picks a binding symbol once it has the
/// module's symbols (or its directory).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleAnchorBind {
    /// `in_module_from(file, module)` → the symbol whose simple name equals
    /// `target` and whose kind is compatible. Python relative import, Dart
    /// library prefix.
    NameExactKind,
    /// `in_module_from(file, module)` → the same-named symbol
    /// (case-insensitive) when present, else the first symbol in the module.
    /// Anchors a cross-file edge when the require names a file, not a member
    /// (Ruby `require`).
    PreferNamedElseFirst,
    /// `by_name(target)` filtered to candidates whose `file_path` contains
    /// `module.replace('.', "/")`, plus the `{module}.{target}` qname probe.
    /// Maps a dotted module to a directory and accepts any kind-compatible
    /// file under it (Python `models.TextChoices` at `.../models/enums.py`).
    ByNameUnderModuleDir,
    /// `by_name(target)` filtered to candidates, kind-compatible, whose file
    /// basename-stem OR a path dir-segment equals the `module`'s leaf — the
    /// last `.`-separated segment, lowercased — under the `StemSource` rule.
    /// The module head is an alias root; its leaf is the file that owns the
    /// target. OCaml dotted-module references (`List.map` where the file
    /// `list.ml` declares `map`).
    ByFileStem { against: StemSource },
    /// `members_of(module)` where `module` names a TYPE: the member whose name
    /// equals `target` under the profile's `NameNormalization` and whose kind
    /// is compatible. Fortran derived-type-member references, where the
    /// extractor sets `module` to the type name and the member is
    /// case-insensitively matched.
    MemberOfModuleType,
}

/// What a `ModuleAnchorBind::ByFileStem` compares the candidate file-stem
/// against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemSource {
    /// The `module`'s last `.`-separated segment, lowercased — the leaf that
    /// names the file (OCaml `List.map` → leaf `list`).
    ModuleLeaf,
}

/// Which `module` specifiers the module-anchor treats as relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeMarker {
    /// Every module is relative — always run the configured `ModuleAnchorBind`
    /// via `in_module_from` (Dart, Ruby). The module resolver itself
    /// distinguishes `package:` / load-path / relative specifiers.
    None,
    /// A leading `.` marks a relative module (Python `.foo`, `..bar`);
    /// everything else is absolute and routes to `ByNameUnderModuleDir`.
    DotPrefix,
    /// A leading `./` or `../` marks a relative module; everything else is
    /// absolute and routes to `ByNameUnderModuleDir`.
    DotSlashPrefix,
}

/// Data for the file-namespace-gated decline. A ref declines before the
/// strategy ladder only when BOTH hold: the resolving file's `file_namespace`
/// equals `file_namespace`, AND `is_reserved` returns true for the target.
/// The two-key gate is why this can't fold into `builtin_skip` — that
/// predicate never sees the file namespace, so it would decline the reserved
/// names in every file rather than only the namespaced ones.
#[derive(Debug, Clone, Copy)]
pub struct NamespaceDecline {
    /// The `FileContext::file_namespace` sentinel that arms the decline.
    pub file_namespace: &'static str,
    /// True when the target names a reserved symbol of that namespace.
    pub is_reserved: fn(&str) -> bool,
}

/// Marker for `resolve_via_external_by_import`: an import-scoped bind of a bare
/// target to an EXTERNAL symbol. The external file's `ext:<lang>:<pkg>`
/// package segment must equal a non-relative import root from the file's
/// imports, or start with `{root}-` (Ruby gem families: `aws-sdk-s3` under
/// gem `aws`). The bind is structural — an import root names the package and
/// the package owns a matching symbol — so it resolves at `RESOLVED_CONFIDENCE`
/// like every other strategy; the unit field only opts a language in.
#[derive(Debug, Clone, Copy)]
pub struct ExternalByImport;

/// How `resolve_via_wildcard_import` tests whether a candidate sits under a
/// wildcard import's module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WildcardMatch {
    /// The candidate's qualified name is exactly one segment deeper than the
    /// wildcard's module path (`qname_directly_under`). The default — a member
    /// keyed under the imported namespace (Java static wildcard, Rust `use ::*`,
    /// Python `from m import *`, C++ `using namespace`).
    QnameUnder,
    /// The candidate's FILE names the wildcard's module: its basename-stem OR a
    /// path dir-segment equals the module name (`path_stem_matches`). The name
    /// comparison runs under the profile's `NameNormalization`, so a
    /// case-insensitive language binds a reference written in a different
    /// surface form. `underscore_prefix` additionally accepts a file whose stem
    /// is `{module}_…` — the include-file convention where a unit's symbols are
    /// split across `{unit}_part.inc` siblings (Pascal units / FPC includes).
    FileStem { underscore_prefix: bool },
}

/// How `resolve_via_external_by_import` matches an external candidate's file
/// against the resolving file's imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtMatch {
    /// The external file's `ext:<lang>:<pkg>` package segment equals an import
    /// root, or starts with `{root}-` (package families). The default.
    PkgSegment,
    /// The external file's basename-stem OR a path dir-segment equals an import
    /// LEAF (last path segment) or an import PACKAGE (first path segment) under
    /// `path_stem_matches`. For ecosystems whose externals are named by file
    /// rather than an `ext:`-package boundary (Nim: a `httpclient` import binds
    /// a symbol in `…/httpclient.nim`).
    FileStemOrDir,
}

/// Whether a dotted target's HEAD binds to an in-file alias declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadAliasBind {
    /// No head-alias binding. The default for every language whose dotted
    /// targets resolve through the regular ladder.
    Off,
    /// Truncate the target at its first `.` and bind the head to a same-file
    /// symbol. The head is declined when empty or when it carries a `_` — a
    /// `_`-bearing head is a provider resource TYPE (`aws_instance`), not an
    /// alias. `require_kind`, when `Some`, restricts the bound symbol to that
    /// kind (HCL provider-alias blocks are `class`); `None` accepts any
    /// kind-compatible in-file symbol.
    OnSameFile { require_kind: Option<&'static str> },
}

/// Whether a file-naming import brings its members into bare-name scope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileScopedImports {
    /// No file-scoped import binding. The default.
    Off,
    /// Bind a bare target to a kind-compatible symbol in an imported file
    /// (matched under `NameNormalization`).
    On {
        /// Scan only imports flagged `is_wildcard`. `true` for languages whose
        /// member-bearing file imports are marked wildcard (Robot resource /
        /// Python-library imports); `false` to scan every file-naming import.
        wildcard_only: bool,
        /// Alias-decoded entry binding. `None` (the default shape) matches a
        /// target only against a SYMBOL NAME in the imported file. `Some`
        /// additionally matches a target against an import entry's
        /// `imported_name` and, on a hit, binds the symbol named by that
        /// entry's `alias` — the import table itself carries the
        /// target-name → owning-symbol mapping. The entry's `alias` decodes as
        /// `{type}{separator}{member}`: a non-empty member binds the symbol of
        /// that name; else a non-empty type binds that type symbol; else the
        /// entry names neither and the first symbol of `fallback_kind` in the
        /// file binds. Robot dynamic-library keywords — a `@keyword("alias")`
        /// decorator routes a Robot keyword to a specific Python method, a
        /// `get_keyword_names` / `KEYWORDS` entry routes to the owning class, a
        /// module-level `KEYWORDS` dict falls back to the dispatch class.
        alias_decode: Option<AliasDecode>,
    },
}

/// Data for the alias-decoded file-scoped bind. See `FileScopedImports::On`.
/// The decode is ordered most-specific-first (named member, then named owning
/// type, then dispatch-class fallback); whichever rung binds resolves at
/// `RESOLVED_CONFIDENCE`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AliasDecode {
    /// Splits an entry's `alias` into `{type}{separator}{member}`.
    pub separator: &'static str,
    /// Symbol kind the no-type/no-member fallback binds to (the dispatch class
    /// for a module-level keyword table). `None` disables the fallback.
    pub fallback_kind: Option<&'static str>,
}

/// Module-prefix-rewrite generator for the `ByNameUnderModuleDir` anchor's
/// bare-specifier path. Each delta is pure data; the engine derives the
/// ordered candidate prefixes deterministically from the bare module string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulePrefixRewrites {
    /// No rewrites — the literal `{module}` is the only prefix tried, and the
    /// directory-containment fallback runs as usual. The default.
    Off,
    /// Generate alternate prefixes for a bare specifier and retry the qname
    /// probe under each, in order: `{module}` first, then the
    /// DefinitelyTyped rewrites, then the deep-import peels.
    On {
        /// Try the `@types/` DefinitelyTyped form of the specifier:
        /// `@scope/pkg` → `@types/scope__pkg`, `pkg` → `@types/pkg`. Skipped
        /// when the specifier already starts with `@types/`.
        definitely_typed: bool,
        /// Peel trailing `/seg` segments off a `/`-bearing specifier
        /// (`pkg/sub` → `pkg`), retrying the qname probe at each shorter prefix.
        /// Stops before a bare `@scope` (a scoped package always keeps its
        /// package segment: `@scope/pkg/sub` peels to `@scope/pkg`, never
        /// `@scope`).
        deep_import_peel: bool,
        /// Decline the directory-containment fallback for a bare specifier.
        /// `true` for TS/JS — a bare package name (`react`) must resolve
        /// through the qname rewrites or stay unresolved, never directory-match
        /// a same-named project file.
        decline_bare_directory_match: bool,
    },
}

/// Ambient-global probe for a bare single-identifier call that no import binds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AmbientGlobals {
    /// No ambient-global probe. The default.
    Off,
    /// Probe the synthetic `__npm_globals__.<name>` namespace and, on a miss,
    /// the bare qname when the candidate's defining file is an ambient-global
    /// lib file. Both probes are structural binds against the synthetic-globals
    /// namespace / ambient-lib files, so a hit resolves at `RESOLVED_CONFIDENCE`.
    On {
        /// Accept an ambient-lib `variable` candidate for an `Instantiates`
        /// ref. The core lib encodes constructors as `declare var X: { new():
        /// Y }` — recorded as a `variable` but constructible via `new X()`.
        instantiate_accepts_variable: bool,
    },
}

/// Flat-global by-name binding mode for the namespaceless-global rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NamespaceScope {
    /// No flat-global probe. The default.
    #[default]
    Off,
    /// First-match-bind against the FIRST kind-compatible, project-internal
    /// symbol of the same name anywhere in the project.
    Global,
    /// First-match-bind, but only among candidates that live in the same
    /// directory as the referencing file. A same-named symbol in another
    /// directory does not bind.
    DirectoryScoped,
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

/// The module boundary a bare same-module reference binds within when no
/// import and no chain root the target. Selects the generic
/// `resolve_via_module_scope` strategy's in-module candidate filter; the
/// unique-internal-name dedup then binds iff exactly one candidate survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleScope {
    /// No module-scope rung — the bare same-module bind never fires.
    Off,
    /// The source file's immediate parent directory IS the module (Odin,
    /// MATLAB): a candidate is in-module iff its immediate parent-dir
    /// basename equals the source's. Binds the FIRST kind-compatible same-dir
    /// candidate — a same-package same-name duplicate is a compile error in
    /// these languages, so the first match is the only match.
    SameDir,
    /// Like `SameDir` (the immediate parent dir is the module), but binds only
    /// when EXACTLY ONE kind-compatible internal candidate is in-dir, after the
    /// unique-internal-name dedup. For a language whose unit is split across
    /// sibling include files in one directory AND carries same-name overload
    /// sets in that directory: a single same-dir declaration binds; an overload
    /// set declines here and falls to argument-driven disambiguation, never a
    /// first-match guess.
    SameDirUnique,
    /// SwiftPM whole-module compilation: every file under one
    /// `Sources/<Target>/` (or `Tests/<Target>/`) subtree compiles into module
    /// `<Target>` and sees every other top-level type in that subtree without
    /// import. A candidate is in-module iff its path shares the source's
    /// `Sources/<seg>/` (or `Tests/<seg>/`) prefix; off-layout paths (no such
    /// prefix) leave the rung inert.
    SourcesTargetSubtree,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructorPattern {
    /// `new Foo()` — JS, TS, Java, C#, PHP, Dart.
    New,
    /// `Foo()` — Python, R6 (when invoked via constructor), most dynamic
    /// languages.
    CallableClass,
    /// `Foo.new(...)` — Ruby, Crystal.
    ClassDotNew,
    /// `Foo::new(...)` — Rust convention.
    TypeColonColonNew,
    /// `Foo::build(...)` and other named ::-prefixed factory conventions.
    TypeColonColonBuild,
    /// `Foo->new(...)` — Perl, valid in PHP.
    ArrowNew,
    /// `Foo$new(...)` — R R6 dispatch.
    R6DollarNew,
    /// `new("Foo", ...)` — R S4.
    S4New,
    /// `Foo:new(...)` — Lua method-call convention on metatable factories.
    LuaColonNew,
    /// Free-function call returning a known type. Pattern matched against
    /// the callee name.
    NamedFactory { fn_name_pattern: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoratorSyntax {
    /// Python `@dataclass`, TS `@Component`, Java `@Override`, Kotlin
    /// `@Suppress`.
    AtPrefix,
    /// Rust `#[derive(...)]`, `#[cfg(...)]`.
    HashBracket,
    /// C# `[Test]`, `[HttpGet]`.
    AttrBracket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKey {
    Named(&'static str),
    Positional(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassNameSource {
    /// First positional arg, e.g. `R6Class("Foo", ...)`.
    ArgIndex(usize),
    /// Name from the assignment LHS: `MyClass <- ...`.
    LhsIdentifier,
    /// Prefer LHS, fall back to positional arg at the given index.
    LhsThenArg(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketContainer {
    /// `list(a = function(...) ...)` — R / R6.
    ListCall(&'static str),
    /// `{ a = function(...) end }` — Lua table constructor.
    TableConstructor,
    /// `{ "a": fn ... }` — Python dict literal.
    DictLiteral,
    /// `do ... end` block — Ruby, Elixir.
    DoBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberShape {
    /// `name = function(args) body` — R / R6 / Lua-style.
    NameEqFunction,
    /// `function name(args) body end` — Lua.
    FunctionDefStmt,
    /// `def name(args): body` — Python.
    DefStmt,
    /// `(key, function)` pair in a dict / table literal.
    KeyValueFunc,
}

pub struct MethodBucket {
    pub arg: ArgKey,
    pub container: BucketContainer,
    pub member_shape: MemberShape,
    pub visibility: Visibility,
}

pub struct ClassBuilderSpec {
    /// Function or macro name that builds a class — `R6Class`, `ggproto`,
    /// `setClass`, `Struct.new`, `setmetatable`.
    pub callee: &'static str,
    /// Namespaces accepted as prefixes (`R6::R6Class` etc.).
    pub accepted_namespaces: &'static [&'static str],
    /// Where to find the class name in the call expression.
    pub class_name_source: ClassNameSource,
    /// Named or positional args holding method lists, with their shape.
    pub method_buckets: &'static [MethodBucket],
    /// Named or positional arg holding the superclass — emits Inherits ref
    /// when present.
    pub inherits_arg: Option<ArgKey>,
}

// ---------------------------------------------------------------------------
// Default profile
// ---------------------------------------------------------------------------

/// Conservative defaults applied to languages without a bespoke profile.
/// Every value is the safe fallback per doc 3's per-axis "Default" entry.
pub const DEFAULT_PROFILE: LanguageProfile = LanguageProfile {
    id: "default",
    qname_separator: ".",
    self_keywords: &[],
    supertype_discovery: SupertypeDiscovery::Explicit,
    ancestor_order: AncestorOrder::Bfs,
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    container_accessors: &[],
    single_inner_wrappers: &[],
    deref_wrapper: None,
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    wildcard_builtins: &[],
    import_resolution: None,
    import_module_path: ImportModulePath::None,
    module_anchor: ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: RelativeMarker::None,
    external_by_import: None,
    name_normalization: NameNormalization::None,
    module_scope: ModuleScope::Off,
    wildcard_match: WildcardMatch::QnameUnder,
    ext_match: ExtMatch::PkgSegment,
    head_alias: HeadAliasBind::Off,
    file_scoped_imports: FileScopedImports::Off,
    alias_module_qname: false,
    module_prefix_rewrites: ModulePrefixRewrites::Off,
    workspace_packages: false,
    reexport_barrel_stems: &["index"],
    self_package_root: None,
    wildcard_workspace_scope: false,
    overload_pick_all: false,
    argument_dependent_lookup: false,
    associated_type_projection: false,
    blanket_impl_resolution: false,
    ambient_globals: AmbientGlobals::Off,
    namespaceless_global_type_lookup: NamespaceScope::Off,
    explicit_member_import: false,
    self_receiver_discovery: SelfReceiverDiscovery::ScopePathThenDefault,
    selector_resolution: None,
    multi_candidate_ranking: false,
    scope_functions: &[],
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &[],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "language_profile_tests.rs"]
mod tests;
