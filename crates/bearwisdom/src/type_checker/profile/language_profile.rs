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
    import_resolution: None,
    import_module_path: ImportModulePath::None,
    constructor_patterns: &[ConstructorPattern::CallableClass],
    class_builder_specs: &[],
    decorator_syntax: None,
    doc_comment_kinds: &[],
    visibility_keywords: &[],
};

#[cfg(test)]
#[path = "language_profile_tests.rs"]
mod tests;
