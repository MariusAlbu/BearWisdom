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
    pub members_can_be_external: bool,
    pub dispatch_axis: DispatchAxis,
    pub has_generics: bool,
    pub has_sum_types: bool,
    pub look_through_optional: bool,
    pub literal_narrowing: bool,
    pub async_wrappers: &'static [&'static str],
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
    /// Import-scoped binding of a bare target to an EXTERNAL symbol at reduced
    /// confidence. `None` (the default) keeps the engine off externals below
    /// confidence 1.0; `Some` opts a language in (Ruby gems). See
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
    /// The source file's parent directory is itself the package: a bare target
    /// binds to any kind-compatible `by_name(target)` candidate whose immediate
    /// parent-dir basename equals the source file's immediate parent-dir
    /// basename. `false` (the default) leaves the strategy inert. `true` opts a
    /// language in (Odin same-package references — no `module` to anchor on, so
    /// this runs module-independently near the end of the ladder).
    pub package_by_directory: bool,
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

    // === Syntax (extractor) ===
    pub constructor_patterns: &'static [ConstructorPattern],
    pub class_builder_specs: &'static [ClassBuilderSpec],
    pub decorator_syntax: Option<DecoratorSyntax>,
    pub doc_comment_kinds: &'static [&'static str],
    pub visibility_keywords: &'static [(&'static str, Visibility)],
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
/// `module_path` from an `Imports` ref's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportModulePath {
    /// Leave `module_path` empty.
    None,
    /// Mirror the raw target into `module_path`.
    EchoTarget,
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

/// Data for `resolve_via_external_by_import`: an import-scoped bind of a bare
/// target to an EXTERNAL symbol. The external file's `ext:<lang>:<pkg>`
/// package segment must equal a non-relative import root from the file's
/// imports, or start with `{root}-` (Ruby gem families: `aws-sdk-s3` under
/// gem `aws`).
#[derive(Debug, Clone, Copy)]
pub struct ExternalByImport {
    /// Confidence recorded on a hit. Below 1.0 by design — this is the only
    /// strategy that intentionally binds to externals.
    pub confidence: f64,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupertypeDiscovery {
    /// Inherits / Implements refs only. Most static-OO languages.
    Explicit,
    /// Structural match between method sets. Go.
    Structural,
    /// Both. TypeScript (interfaces are structural, classes are nominal).
    Both,
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
    NamedFactory {
        fn_name_pattern: &'static str,
    },
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
    members_can_be_external: false,
    dispatch_axis: DispatchAxis::Receiver,
    has_generics: false,
    has_sum_types: false,
    look_through_optional: true,
    literal_narrowing: false,
    async_wrappers: &[],
    iterator_method: None,
    primitive_mapping: &[],
    kind_compatible_table: PERMISSIVE_KIND_TABLE,
    chain_qualification: ChainQualification::None,
    builtin_skip: None,
    namespace_decline: None,
    decline_qualified_when_prefix_imported: false,
    module_skip: None,
    ambient_namespace_prefixes: &[],
    import_resolution: None,
    import_module_path: ImportModulePath::None,
    module_anchor: ModuleAnchor::Off,
    module_anchor_terminal: false,
    relative_marker: RelativeMarker::None,
    external_by_import: None,
    name_normalization: NameNormalization::None,
    package_by_directory: false,
    wildcard_match: WildcardMatch::QnameUnder,
    ext_match: ExtMatch::PkgSegment,
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &[],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "language_profile_tests.rs"]
mod tests;
