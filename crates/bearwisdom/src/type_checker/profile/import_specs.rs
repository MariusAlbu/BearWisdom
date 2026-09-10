// =============================================================================
// type_checker/profile/import_specs.rs — spec vocabulary for the import/module resolution axes:
// how import statements bind, module paths anchor, and namespaces scope.
// =============================================================================

/// How strongly a language or ecosystem adapter constrains file-path matching
/// for one module specifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleMatchAuthority {
    /// The ordinary matcher may use its normal fallbacks after a miss.
    Heuristic,
    /// The adapted path is the complete candidate surface; a miss is final.
    Authoritative,
    /// The adapter recognized malformed or forbidden module syntax.
    Reject,
}

/// Language-owned evidence for resolving re-export source modules by path.
/// The generic re-export walker only joins paths, probes indexed files, and
/// applies these callbacks; source extensions and module spellings stay with
/// the language or ecosystem that defines them.
#[derive(Debug, Clone, Copy)]
pub struct SourceModulePathPolicy {
    /// Classifies source module text before the resolver decides whether to
    /// follow it as a project-relative path or a package/module name.
    pub classify_specifier: fn(&str) -> ModuleSpecifierClass,
    /// Candidate file paths for a joined relative source-module base.
    pub relative_candidate_paths: fn(&str) -> Vec<String>,
    /// Whether an indexed file path can supply a bare source-module spelling.
    pub bare_module_matches_file: fn(&str, &str) -> bool,
    /// Lower-cased file-stem/directory terms for external-import matching.
    pub external_import_match_terms: fn(&str) -> Vec<String>,
}

impl SourceModulePathPolicy {
    /// No source-module spelling is accepted until a language or ecosystem
    /// adapter explicitly supplies one.
    pub fn unsupported() -> Self {
        Self {
            classify_specifier: unsupported_module_specifier_class,
            relative_candidate_paths: no_relative_candidate_paths,
            bare_module_matches_file: never_matches_bare_module,
            external_import_match_terms: no_external_import_match_terms,
        }
    }

    pub fn classify(self, specifier: &str) -> ModuleSpecifierClass {
        (self.classify_specifier)(specifier)
    }

    pub fn is_relative(self, specifier: &str) -> bool {
        self.classify(specifier) == ModuleSpecifierClass::Relative
    }

    pub fn is_bare(self, specifier: &str) -> bool {
        self.classify(specifier) == ModuleSpecifierClass::Bare
    }
}

/// The normalized module-specifier class supplied to generic resolver rules.
/// `Unsupported` is deliberately fail-closed: rules that require either a
/// package name or a relative source do not infer syntax from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleSpecifierClass {
    Relative,
    Bare,
    Unsupported,
}

fn unsupported_module_specifier_class(_specifier: &str) -> ModuleSpecifierClass {
    ModuleSpecifierClass::Unsupported
}

fn no_relative_candidate_paths(_base: &str) -> Vec<String> {
    Vec::new()
}

fn never_matches_bare_module(_file_path: &str, _source_module: &str) -> bool {
    false
}

fn no_external_import_match_terms(_module: &str) -> Vec<String> {
    Vec::new()
}

/// Normalized, language-neutral evidence consumed by the generic file/module
/// matcher. Language and ecosystem modules build this value; resolver code
/// only applies its path, extension, prefix, and authority constraints.
#[derive(Debug, Clone)]
pub struct ModulePathMatch {
    pub module_path: String,
    /// Additional physical-path candidates derived by the active language
    /// profile. The generic matcher compares these verbatim and never infers
    /// source separator grammar itself.
    pub path_variants: Vec<String>,
    pub required_file_prefix: Option<&'static str>,
    pub compound_extensions: &'static [&'static str],
    pub authority: ModuleMatchAuthority,
    pub source_module_path_policy: SourceModulePathPolicy,
}

impl ModulePathMatch {
    pub fn heuristic(module_path: &str) -> Self {
        Self {
            module_path: module_path.to_string(),
            path_variants: Vec::new(),
            required_file_prefix: None,
            compound_extensions: &[],
            authority: ModuleMatchAuthority::Heuristic,
            source_module_path_policy: SourceModulePathPolicy::unsupported(),
        }
    }

    /// Add the physical-path spelling of a source-qualified module. The
    /// caller supplies the active profile's separator, so this stays a
    /// profile-produced candidate rather than generic matcher policy.
    pub fn with_source_separator_path_variant(
        mut self,
        source_module: &str,
        separator: &str,
    ) -> Self {
        if !separator.is_empty() && separator != "/" {
            let path = source_module.replace(separator, "/");
            if path != self.module_path && !self.path_variants.contains(&path) {
                self.path_variants.push(path);
            }
        }
        self
    }
}

/// Language/ecosystem-owned conversion from source module spelling to the
/// generic path evidence used by the resolver.
pub type ModulePathAdapter = fn(&str) -> ModulePathMatch;

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

impl RelativeMarker {
    /// Profile-owned source spelling for module-anchor routing.
    pub fn is_relative(self, module: &str) -> bool {
        match self {
            Self::None => true,
            Self::DotPrefix => module.starts_with('.'),
            Self::DotSlashPrefix => module.starts_with("./") || module.starts_with("../"),
        }
    }
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
    /// The candidate's EXTERNAL file's `ext:<lang>:<pkg>/…` package segment
    /// equals the wildcard's module (`wildcard_package_segment`, the same
    /// `ext:<lang>:<pkg>` convention `ExtMatch::PkgSegment` reads), compared
    /// under the profile's `NameNormalization`. Unlike `FileStem`, this
    /// doesn't require the candidate's own file to name the wildcard's
    /// module — only to share its PACKAGE — so it reaches through a barrel
    /// re-export (`package:flutter/material.dart` re-exporting
    /// `src/widgets/framework.dart`) to the file that actually declares the
    /// member. Falls back to the same file-stem check `FileStem` uses for an
    /// internal candidate or a wildcard whose module carries no package
    /// identity (a relative import) — the extractor is responsible for
    /// reducing a language's real import syntax (a `package:` URI, a dotted
    /// namespace, …) to a bare package name at capture time; this variant
    /// does no URI parsing itself.
    PackageRoot,
    /// `QnameUnder` plus a language adapter that maps a wildcard module and
    /// target to the real files that can contribute the bare name. The
    /// resolver only compares those returned file paths and declines ambiguous
    /// hits; it does not know the language's relative-module keywords,
    /// source-root layout, or module-file conventions.
    QnameUnderWithPhysicalFiles {
        candidate_files: RelativeModuleCandidateFiles,
    },
}

/// Language-owned evidence for a wildcard module whose imports resolve by
/// physical file location. Returning no files leaves the generic wildcard
/// binder inert for that import. The callback receives the source target so it
/// can reject a language-qualified target before any file lookup occurs.
pub type RelativeModuleCandidateFiles = fn(&str, &str, &str) -> Vec<String>;

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

/// Module-prefix-rewrite policy for the `ByNameUnderModuleDir` anchor's
/// bare-specifier path. The engine consumes normalized candidates and a
/// decline decision without interpreting ecosystem naming conventions.
#[derive(Debug, Clone, Copy)]
pub enum ModulePrefixRewrites {
    /// No rewrites — the literal `{module}` is the only prefix tried, and the
    /// directory-containment fallback runs as usual. The default.
    Off,
    /// Generate qname prefixes for a bare specifier and retry each in
    /// adapter-defined order.
    On {
        /// Optional language/ecosystem adapter for file-path matching. The
        /// callback implementation lives outside the generic resolver.
        module_path_adapter: Option<ModulePathAdapter>,
        /// Produces qname prefix candidates in priority order. The adapter
        /// includes the literal module spelling when it is a valid candidate.
        candidate_prefixes: fn(&str) -> Vec<String>,
        /// True when directory containment would be unsound for this module.
        declines_directory_match: fn(&str) -> bool,
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
    /// A language-owned source-root layout where every file under one
    /// `<root>/<Target>/` subtree compiles into a module and sees every other
    /// top-level type in that subtree without import. `roots` is profile data;
    /// off-layout paths leave the rung inert.
    SourcesTargetSubtree { roots: &'static [&'static str] },
}
