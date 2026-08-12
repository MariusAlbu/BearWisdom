// =============================================================================
// type_checker/profile/import_axes.rs — the import/module-resolution axes,
// composed as `LanguageProfile::imports`.
//
// Every field is a per-language delta of the module-resolution ladder: how
// import statements bind, module paths anchor, and namespaces scope. The
// enum/struct vocabulary these fields reference lives in `import_specs.rs`.
// =============================================================================

use super::import_specs::*;

pub struct ImportAxes {
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
    /// Whether a plain namespace import (`using System;`, `Imports System`,
    /// `open System`) brings every DIRECT member of the namespace into bare
    /// scope — the same admission a `*` wildcard import grants. When `true`,
    /// every non-binding Imports entry becomes a wildcard of its module path,
    /// and the wildcard rung also unions the manifest-declared implicit/global
    /// namespaces (`SymbolLookup::implicit_wildcard_namespaces`). `false` (the
    /// default) keeps wildcardness on the literal `*` target only.
    pub namespace_imports_are_wildcards: bool,
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
}
