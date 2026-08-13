// =============================================================================
// ecosystem/ — first-class Ecosystem construct for package & stdlib sources
//
// An Ecosystem is a package world: one install location, one dep format, one
// artifact shape. One Ecosystem may serve several languages (Maven covers
// Java + Kotlin + Scala + Clojure + Groovy; npm covers JS + TS + Vue + Svelte;
// Hex covers Elixir + Erlang + Gleam). Stdlib sources (rust-stdlib, jdk-src,
// android-sdk, etc.) are modeled as Ecosystems with kind = Stdlib.
//
// This module is the architectural successor to `indexer/externals/` and
// `indexer/manifest/`. During the refactor both the old and new layers
// coexist: the new trait is additive; existing `ExternalSourceLocator` impls
// continue to work unchanged. Phase 2 migrates locators to full Ecosystem
// impls; Phase 3 folds manifest parsers in; Phase 4 wires the trait through
// the indexer and drops the legacy path.
//
// See REFACTOR_PLAN.md for the full phased migration.
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ecosystem::externals::ExternalDepRoot;
use crate::types::ParsedFile;
use crate::walker::WalkedFile;

pub mod ambient;
pub mod coursier_cache;
pub mod externals;
pub mod jvm_caches;
pub mod imports;
pub mod manifest;
pub mod symbol_index;

pub use symbol_index::SymbolLocationIndex;

pub mod alire;
pub mod android_sdk;
pub mod bazel_central_registry;
pub mod bicep_runtime;
pub mod cabal;
pub mod cargo;
pub mod cargo_build_scripts;
pub mod clojure_core;
pub mod compile_commands;
pub mod composer;
pub mod cpan;
pub mod cpython_stdlib;
pub mod cran;
pub mod dart_sdk;
pub mod dotnet_stdlib;
pub mod ecmascript_imports;
pub mod elixir_stdlib;
pub mod erlang_otp;
pub mod flutter_sdk;
pub mod freepascal_runtime;
pub mod gleam_stdlib;
pub mod gnat_project;
pub mod gnat_stdlib;
pub mod go_mod;
pub mod go_platform;
pub mod go_stdlib;
pub mod godot_api;
pub mod groovy_stdlib;
pub mod hex;
pub mod hexo_runtime;
pub mod jar_walker;
pub mod jdk_src;
pub mod jinja_ansible_runtime;
pub mod jupyter_dedup;
pub mod kotlin_stdlib;
pub mod lua_stdlib;
pub mod luarocks;
pub mod matlab_runtime;
pub mod maven;
pub mod maven_classes;
pub mod msvc_sdk;
pub mod nim_stdlib;
pub mod nimble;
pub mod npm;
pub mod nuget;
pub mod nuxt_runtime;
pub mod nvim_runtime;
pub mod ocaml_stdlib;
pub mod opam;
pub mod openapi_generated;
pub mod perl_stdlib;
pub mod php_stubs;
pub mod posix_headers;
pub mod powershell_cmdlet_types;
pub mod powershell_stdlib;
pub mod prisma_client;
pub mod prolog_runtime;
pub mod protoc_generated;
pub mod psgallery;
pub mod pub_pkg;
pub mod puppet_forge;
pub mod puppet_stdlib;
pub mod pypi;
pub mod qt_runtime;
pub mod r_stdlib;
pub mod ruby_stdlib;
pub mod rubygems;
pub mod rust_stdlib;
pub mod scala_stdlib;
pub mod sdl_synthetics;
pub mod spm;
pub mod swift_foundation;
pub mod swift_pm_dsl;
pub mod tf_registry;
pub mod toolchain_payload;
pub mod ts_lib_dom;
pub mod vba_typelibs;
pub mod vendored_self_declared;
pub mod vendored_submodules;
pub mod zig_pkg;
pub mod zig_std;

/// The checked-in vendor/generated-code classifier is shared with the
/// walker's git-state exclusion gate in `bearwisdom-profile`, which cannot
/// depend on this crate — so it lives there and is re-exported here.
pub use bearwisdom_profile::vendored_or_generated;

pub use alire::AlireEcosystem;
pub use android_sdk::AndroidSdkEcosystem;
pub use bazel_central_registry::BazelCentralRegistryEcosystem;
pub use bicep_runtime::BicepRuntimeEcosystem;
pub use cabal::CabalEcosystem;
pub use cargo::CargoEcosystem;
pub use cargo_build_scripts::CargoBuildScriptsEcosystem;
pub use clojure_core::ClojureCoreEcosystem;
pub use compile_commands::CompileCommandsEcosystem;
pub use composer::ComposerEcosystem;
pub use cpan::CpanEcosystem;
pub use cpython_stdlib::CpythonStdlibEcosystem;
pub use cran::CranEcosystem;
pub use dart_sdk::DartSdkEcosystem;
pub use dotnet_stdlib::DotnetStdlibEcosystem;
pub use elixir_stdlib::ElixirStdlibEcosystem;
pub use erlang_otp::ErlangOtpEcosystem;
pub use flutter_sdk::FlutterSdkEcosystem;
pub use freepascal_runtime::FreePascalRuntimeEcosystem;
pub use gleam_stdlib::GleamStdlibEcosystem;
pub use gnat_project::GnatProjectEcosystem;
pub use gnat_stdlib::GnatStdlibEcosystem;
pub use go_mod::GoModEcosystem;
pub use go_stdlib::GoStdlibEcosystem;
pub use godot_api::GodotApiEcosystem;
pub use groovy_stdlib::GroovyStdlibEcosystem;
pub use hex::HexEcosystem;
pub use hexo_runtime::HexoRuntimeEcosystem;
pub use jdk_src::JdkSrcEcosystem;
pub use jinja_ansible_runtime::JinjaAnsibleRuntimeEcosystem;
pub use kotlin_stdlib::KotlinStdlibEcosystem;
pub use lua_stdlib::LuaStdlibEcosystem;
pub use luarocks::LuarocksEcosystem;
pub use matlab_runtime::MatlabRuntimeEcosystem;
pub use maven::MavenEcosystem;
pub use maven_classes::MavenClassesEcosystem;
pub use msvc_sdk::MsvcSdkEcosystem;
pub use nim_stdlib::NimStdlibEcosystem;
pub use nimble::NimbleEcosystem;
pub use npm::NpmEcosystem;
pub use nuget::NugetEcosystem;
pub use nuxt_runtime::NuxtRuntimeEcosystem;
pub use nvim_runtime::NvimRuntimeEcosystem;
pub use ocaml_stdlib::OcamlStdlibEcosystem;
pub use opam::OpamEcosystem;
pub use openapi_generated::OpenApiGeneratedEcosystem;
pub use perl_stdlib::PerlStdlibEcosystem;
pub use php_stubs::PhpStubsEcosystem;
pub use posix_headers::{PosixHeadersEcosystem, VcpkgHeadersEcosystem};
pub use powershell_stdlib::PowerShellStdlibEcosystem;
pub use prisma_client::PrismaClientEcosystem;
pub use prolog_runtime::PrologRuntimeEcosystem;
pub use protoc_generated::ProtocGeneratedEcosystem;
pub use psgallery::PsGalleryEcosystem;
pub use pub_pkg::PubEcosystem;
pub use puppet_forge::PuppetForgeEcosystem;
pub use puppet_stdlib::PuppetStdlibEcosystem;
pub use pypi::PypiEcosystem;
pub use qt_runtime::QtRuntimeEcosystem;
pub use r_stdlib::RStdlibEcosystem;
pub use ruby_stdlib::RubyStdlibEcosystem;
pub use rubygems::RubygemsEcosystem;
pub use rust_stdlib::RustStdlibEcosystem;
pub use scala_stdlib::ScalaStdlibEcosystem;
pub use sdl_synthetics::SdlSyntheticsEcosystem;
pub use spm::SpmEcosystem;
pub use swift_foundation::SwiftFoundationEcosystem;
pub use swift_pm_dsl::SwiftPmDslEcosystem;
pub use tf_registry::TfRegistryEcosystem;
pub use ts_lib_dom::TsLibDomEcosystem;
pub use vba_typelibs::VbaTypelibsEcosystem;
pub use zig_pkg::ZigPkgEcosystem;
pub use zig_std::ZigStdEcosystem;

// ---------------------------------------------------------------------------
// Identity & kind
// ---------------------------------------------------------------------------

/// Stable identifier for an `Ecosystem`. Used as the primary key in
/// `EcosystemRegistry` and as the tag stamped onto walked external files so
/// resolution can filter symbols by originating ecosystem.
///
/// Conventionally lower-kebab: `"maven"`, `"npm"`, `"pypi"`, `"cargo"`,
/// `"hex"`, `"rust-stdlib"`, `"android-sdk"`, `"ts-lib-dom"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EcosystemId(pub &'static str);

impl EcosystemId {
    pub const fn new(s: &'static str) -> Self {
        Self(s)
    }
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for EcosystemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Whether an ecosystem publishes third-party packages (npm, Cargo, Hex) or
/// ships language runtime source (rust-stdlib, jdk-src, android-sdk).
///
/// The trait surface is identical for both; `kind` drives caching policy
/// (stdlib caches keyed on toolchain version) and activation semantics
/// (stdlibs typically activate on language presence, not manifest match).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EcosystemKind {
    Package,
    Stdlib,
}

/// Host platform for `EcosystemActivation::AlwaysOnPlatform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Unix,       // Any non-Windows
    AnyDesktop, // Windows | MacOs | Linux
}

// ---------------------------------------------------------------------------
// Manifest specs
// ---------------------------------------------------------------------------

/// A filename-pattern + parser pair. Each `Ecosystem` declares the manifest
/// formats it recognizes; the project-level scan walks the repo once and
/// dispatches each match to the owning ecosystem.
///
/// This folds today's `indexer/manifest/` parsers into their respective
/// ecosystems. `parse` returns the same `ManifestData` type used elsewhere
/// in the codebase so downstream consumers don't need to change.
pub struct ManifestSpec {
    /// Glob pattern (repo-relative) matching manifest filenames for this
    /// ecosystem. Examples: `"**/package.json"`, `"**/Cargo.toml"`,
    /// `"**/pom.xml"`, `"**/build.gradle{,.kts}"`, `"**/go.mod"`.
    pub glob: &'static str,

    /// Parser invoked on each matched manifest file. Returns the normalized
    /// `ManifestData` payload (deps, module path, SDK info, etc.). Errors
    /// degrade gracefully to an empty `ManifestData`; they are logged but
    /// don't abort indexing.
    pub parse: fn(&Path) -> std::io::Result<crate::ecosystem::manifest::ManifestData>,
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

/// Predicate that decides whether an ecosystem is active for a given project.
/// Evaluated once per project during `ProjectContext` initialization.
///
/// Activation is how stdlibs avoid being indexed for projects that don't use
/// their language, how platform SDKs gate on `TransitiveOn(Maven)`, and how
/// the DOM stdlib activates only when `tsconfig.compilerOptions.lib`
/// contains `"DOM"`.
#[derive(Debug, Clone)]
pub enum EcosystemActivation {
    /// Active iff any manifest matching `manifest_specs()` is found in the
    /// project. The default for package ecosystems (npm, Cargo, Hex, ...).
    ManifestMatch,

    /// Active iff any file of the given language id is present in the
    /// project. Used by most stdlib ecosystems (rust-stdlib when any `.rs`
    /// file exists; cpython-stdlib when any `.py` file exists).
    LanguagePresent(&'static str),

    /// Active iff the named manifest has a specific field containing a
    /// value. Drives things like: `ts-lib-dom` active iff
    /// `tsconfig.json.compilerOptions.lib` contains `"DOM"`.
    ManifestFieldContains {
        manifest_glob: &'static str,
        field_path: &'static str,
        value: &'static str,
    },

    /// Active unconditionally on a given platform. Used by
    /// `posix-headers` (unix) and `msvc-sdk` (Windows, gated further).
    AlwaysOnPlatform(Platform),

    /// Active only when another ecosystem is active. Used by
    /// `android-sdk` (requires Maven), `kotlin-stdlib` (requires Maven or
    /// a `.kt` file).
    TransitiveOn(EcosystemId),

    /// Composite: all clauses must match.
    All(&'static [EcosystemActivation]),

    /// Composite: any clause matches.
    Any(&'static [EcosystemActivation]),

    /// Always active. Used sparingly; mainly for testing.
    Always,

    /// Never active. Effectively disables an ecosystem without unregistering.
    Never,
}

// ---------------------------------------------------------------------------
// Locate context
// ---------------------------------------------------------------------------

/// Read-only context passed to `Ecosystem::locate_roots`. Carries everything
/// the ecosystem needs to discover dep roots without reaching into the
/// broader `ProjectContext` (which may not yet be fully initialized at
/// locate time).
pub struct LocateContext<'a> {
    pub project_root: &'a Path,
    /// Manifests collected during the project scan, keyed by ecosystem id.
    /// An ecosystem reads its own entry; cross-ecosystem manifest access
    /// is allowed for edge cases (`build.gradle.kts` borrowing Kotlin
    /// parsing; deps.edn under Maven).
    pub manifests: &'a HashMap<EcosystemId, Vec<PathBuf>>,
    /// Ids of other ecosystems already judged active. Used by ecosystems
    /// whose discovery is conditional on another ecosystem being present
    /// (android-sdk discovers platform jars only when Maven is active).
    pub active_ecosystems: &'a [EcosystemId],
}

// ---------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------

/// A package or stdlib source provider.
///
/// Implementations are registered in `EcosystemRegistry` once per process.
/// During indexing, `ProjectContext::initialize` evaluates each ecosystem's
/// `activation()` against the project; active ecosystems then discover dep
/// roots via `locate_roots` and emit walked files via `walk_root`. The
/// indexer routes each walked file to the right `LanguagePlugin` via
/// per-file language detection — the ecosystem declares capability
/// (`languages()`), not hard assignment.
pub trait Ecosystem: Send + Sync {
    /// Stable id. Primary key in `EcosystemRegistry`.
    fn id(&self) -> EcosystemId;

    /// Package (third-party) vs Stdlib (language runtime). Drives caching
    /// and activation defaults.
    fn kind(&self) -> EcosystemKind;

    /// Capability declaration: which language ids appear in packages
    /// published to this ecosystem.
    ///
    /// Not an assignment. A Kotlin file does not intrinsically "belong to"
    /// Maven — the project's active ecosystem set plus per-file language
    /// detection drive actual routing. This list tells the walker which
    /// plugins may need to be invoked on packages from this ecosystem and
    /// tells the resolver which ecosystems a given ref-language can reach.
    fn languages(&self) -> &'static [&'static str];

    /// Manifest formats this ecosystem recognizes. Stdlib ecosystems
    /// typically return an empty slice (activation is probe-based).
    fn manifest_specs(&self) -> &'static [ManifestSpec] {
        &[]
    }

    /// `(filename, kind_label)` pairs declaring exact workspace-package
    /// markers for this ecosystem. The detector registers one `PackageInfo`
    /// row per matched file with `packages.kind = kind_label`.
    ///
    /// One ecosystem may publish multiple filenames under one kind
    /// (`pyproject.toml` and `setup.py` → kind `python`) or several kinds
    /// (Maven's `pom.xml` → `maven`, `build.gradle{,.kts}` → `gradle`,
    /// `build.sbt` → `sbt`). The kind label is the user-visible ecosystem
    /// name, distinct from the internal `EcosystemId`.
    ///
    /// Used by the workspace package detector during a recursive tree
    /// walk. For every non-pruned directory it asks each registered
    /// ecosystem "do you own a manifest here?" and registers per match.
    /// Multiple ecosystems may match the same directory (Tauri repo root:
    /// `Cargo.toml` + `package.json`) — they coexist via the `(path, kind)`
    /// composite key on the `packages` table.
    ///
    /// Distinct from `manifest_specs()` which uses globs and drives
    /// project-wide manifest *parsing*. Workspace-package detection is the
    /// shallower question of "is this dir a package root?".
    ///
    /// Default: empty. Stdlib ecosystems and probe-based ones (which find
    /// roots via SDK discovery, not user-authored manifests) leave this
    /// at the default.
    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// `(extension, kind_label)` pairs (extensions include the leading dot)
    /// that mark workspace packages when matched against any file in a
    /// directory. Used by ecosystems where the manifest filename embeds
    /// the project name — `.NET` (`<Proj>.csproj`/`.fsproj`/`.vbproj`),
    /// Haskell (`<pkg>.cabal`), Nim (`<pkg>.nimble`). The detector treats
    /// each matched file as a distinct workspace package.
    ///
    /// Default: empty. Most ecosystems use exact filenames instead.
    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// Directory basenames this ecosystem creates that should be pruned
    /// from recursive package scans — dependency caches and build outputs,
    /// not user-authored workspace folders.
    ///
    /// Examples:
    ///   - npm: `node_modules`, `bower_components`
    ///   - pub: `.dart_tool`, `.pub-cache`
    ///   - cargo: `target`
    ///   - python: `__pycache__`, `.venv`, `.tox`, `.pytest_cache`
    ///   - maven/gradle: `.gradle`, `.mvn`
    ///   - cocoapods/spm: `Pods`, `DerivedData`
    ///
    /// The orchestrator unions these across every registered ecosystem and
    /// adds a small universal set (`.git`, `.hg`, `.svn`) outside any
    /// ecosystem's purview.
    ///
    /// Default: empty.
    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// When is this ecosystem active for a given project?
    fn activation(&self) -> EcosystemActivation;

    /// Discover on-disk dep roots. Called once per project for every
    /// active ecosystem. An empty vec means "nothing to index" — never
    /// an error (missing toolchains, absent caches, etc. all degrade
    /// to empty).
    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot>;

    /// Walk one dep root, yielding files to be parsed as external source.
    /// Each `WalkedFile` is downstream tagged with this ecosystem's id
    /// + per-file language detection routes to the right plugin.
    ///
    /// This is the eager/wholesale path. `Stdlib`-kind ecosystems typically
    /// use it (stdlib types are touched by nearly every file, so pre-warming
    /// pays off). `Package`-kind ecosystems should override
    /// `resolve_import` + `resolve_symbol` instead and leave this returning
    /// empty — the reachability loop drives them on demand.
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        let _ = dep;
        Vec::new()
    }

    /// Opt-in flag: has this ecosystem migrated to reachability-based
    /// loading? When false (default), the indexer ignores
    /// `resolve_import`/`resolve_symbol` and drives externals via
    /// `walk_root` as before. When true, the indexer calls the
    /// reachability methods and skips the eager walk. Set to true after
    /// overriding `resolve_import` with a real implementation.
    fn supports_reachability(&self) -> bool {
        false
    }

    /// Reachability entry point: resolve a specific import statement.
    ///
    /// Given a package name and the symbols named in an `import { X, Y } from 'pkg'`
    /// statement (or language equivalent), return exactly the files needed to
    /// surface those symbols + any signature types they reference directly.
    /// The indexer parses the returned files like any other walked file and
    /// stops — no recursion into unrelated parts of the package.
    ///
    /// This is the preferred interception point for `Package`-kind
    /// ecosystems. Default delegates to `walk_root` so legacy eager
    /// behavior survives during the staged rollout; ecosystems override to
    /// emit a narrow, import-driven slice of WalkedFiles instead.
    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        package: &str,
        symbols: &[&str],
    ) -> Vec<WalkedFile> {
        let _ = (package, symbols);
        self.walk_root(dep)
    }

    /// Reachability chain step: pull a single qualified name on demand.
    ///
    /// Used by chain walkers (Tier 1.5 resolvers) when they encounter a ref
    /// whose target is known by fully-qualified name but hasn't been indexed
    /// yet. Return the file(s) defining `fqn` so the indexer can parse them
    /// and extend the symbol graph.
    ///
    /// Default delegates to `walk_root` so legacy eager behavior survives
    /// during the staged rollout. Reachability-capable ecosystems override
    /// to return just the file defining `fqn`.
    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        let _ = fqn;
        self.walk_root(dep)
    }

    /// Metadata-only extraction path. Used by NuGet (DLL metadata via
    /// dotscope) and potentially jdk-src (jmod) where no source is on disk.
    /// Mutually exclusive with `walk_root` for a given dep.
    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        let _ = dep;
        None
    }

    /// Build a cheap `(module, name) → file` index over the given dep roots
    /// using a header-only tree-sitter parse (top-level decls only, no
    /// function/method body descent). Consumed by Stage 2 of the refactored
    /// pipeline: for every symbol the demand set asks for, the indexer
    /// queries this handle to find the single file to parse.
    ///
    /// Default impl returns an empty index, which signals "this ecosystem
    /// has not migrated to demand-driven parsing yet; keep using the eager
    /// `walk_root` / `resolve_import` path." Ecosystems override once their
    /// scanner is wired.
    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        let _ = dep_roots;
        SymbolLocationIndex::new()
    }

    /// Opt-in flag: when `true`, the indexer skips this ecosystem's eager
    /// walk entirely, builds the symbol index up front, and parses only
    /// files the Stage 2 demand loop asks for. When `false` (default),
    /// the legacy `walk_root` / `resolve_import` eager path still runs.
    ///
    /// An ecosystem must implement `build_symbol_index` before flipping
    /// this on — returning empty from the index while skipping the eager
    /// walk would leave the ecosystem's deps entirely unindexed.
    fn uses_demand_driven_parse(&self) -> bool {
        false
    }

    /// Drop any process-lifetime caches backing this ecosystem's demand
    /// parsing. Invoked once at the start of every full index run so a prior
    /// run's demand set neither pins memory into this run nor influences its
    /// parse results. Default: no such caches exist.
    fn reset_demand_caches(&self) {}

    /// Per-file post-processing hook. npm uses this to prefix symbols
    /// with package name so the Tier-1 resolver matches
    /// `import { X } from 'pkg'` → `pkg.X`, requalifying extractor-set
    /// declared types against `arena` in the same pass.
    fn post_process_parsed(
        &self,
        dep: &ExternalDepRoot,
        parsed: &mut ParsedFile,
        arena: &crate::type_checker::core::types::TypeArena,
    ) {
        let _ = (dep, parsed, arena);
    }

    /// True if this ecosystem describes a workspace-level artefact that
    /// covers the entire build regardless of how workspace packages were
    /// detected. Such ecosystems bypass per-package activation narrowing —
    /// their activation is evaluated against the workspace-wide scope, and
    /// their `locate_roots_for_package` should delegate to
    /// `locate_roots(workspace_root)` rather than probing each package's
    /// directory.
    ///
    /// `compile_commands.json` is the canonical example: one file lists
    /// every translation unit's flags, so it is workspace-wide by
    /// construction. The C/C++ implicit toolchains (posix-headers,
    /// msvc-sdk, vcpkg-headers, qt-runtime) likewise describe
    /// workspace-level facts ("the OS / SDK provides these headers")
    /// rather than per-package dependencies.
    ///
    /// Default: `false` — package ecosystems (npm, cargo, maven, ...)
    /// stay narrow because per-package activation is genuinely correct
    /// for them (a frontend package's npm deps shouldn't activate for an
    /// unrelated backend package in the same monorepo).
    fn is_workspace_global(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

mod registry;
pub use registry::{default_locator, default_registry, EcosystemRegistry};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
