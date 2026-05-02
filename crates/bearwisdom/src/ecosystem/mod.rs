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

pub mod externals;
pub mod imports;
pub mod manifest;
pub mod symbol_index;

pub use symbol_index::SymbolLocationIndex;

pub mod android_sdk;
pub mod blazor_runtime;
pub mod jinja_ansible_runtime;
pub mod bicep_runtime;
pub mod cabal;
pub mod bazel_central_registry;
pub mod bash_completion_synthetics;
pub mod sdl_synthetics;
pub mod cargo;
pub mod clojure_core;
pub mod composer;
pub mod cpan;
pub mod cpython_stdlib;
pub mod cran;
pub mod dart_sdk;
pub mod dotnet_stdlib;
pub mod elixir_stdlib;
pub mod erlang_otp;
pub mod flutter_sdk;
pub mod gleam_stdlib;
pub mod go_mod;
pub mod go_platform;
pub mod go_stdlib;
pub mod godot_api;
pub mod groovy_stdlib;
pub mod hex;
pub mod jdk_src;
pub mod kotlin_stdlib;
pub mod luarocks;
pub mod matlab_runtime;
pub mod matlab_stdlib;
pub mod maven;
pub mod nimble;
pub mod node_builtins;
pub mod npm;
pub mod nvim_runtime;
pub mod nuget;
pub mod opam;
pub mod phoenix_stubs;
pub mod php_stubs;
pub mod posix_headers;
pub mod powershell_cmdlet_types;
pub mod powershell_stdlib;
pub mod psgallery;
pub mod pub_pkg;
pub mod puppet_forge;
pub mod puppet_stdlib;
pub mod pypi;
pub mod robot_builtin_synthetics;
pub mod robot_seleniumlibrary_synthetics;
pub mod robot_browser_synthetics;
pub mod ember_handlebars_helpers;
pub mod ruby_stdlib;
pub mod rubygems;
pub mod rust_stdlib;
pub mod scala_stdlib;
pub mod spm;
pub mod spring_stubs;
pub mod swift_foundation;
pub mod swift_pm_dsl_stubs;
pub mod tf_registry;
pub mod ts_lib_dom;
pub mod vba_typelibs;
pub mod zig_pkg;
pub mod zig_std;
pub use android_sdk::AndroidSdkEcosystem;
pub use bazel_central_registry::BazelCentralRegistryEcosystem;
pub use blazor_runtime::BlazorRuntimeEcosystem;
pub use jinja_ansible_runtime::JinjaAnsibleRuntimeEcosystem;
pub use bicep_runtime::BicepRuntimeEcosystem;
pub use bash_completion_synthetics::BashCompletionSyntheticsEcosystem;
pub use sdl_synthetics::SdlSyntheticsEcosystem;
pub use cabal::CabalEcosystem;
pub use cargo::CargoEcosystem;
pub use clojure_core::ClojureCoreEcosystem;
pub use composer::ComposerEcosystem;
pub use cpan::CpanEcosystem;
pub use cpython_stdlib::CpythonStdlibEcosystem;
pub use cran::CranEcosystem;
pub use dart_sdk::DartSdkEcosystem;
pub use dotnet_stdlib::DotnetStdlibEcosystem;
pub use elixir_stdlib::ElixirStdlibEcosystem;
pub use erlang_otp::ErlangOtpEcosystem;
pub use flutter_sdk::FlutterSdkEcosystem;
pub use gleam_stdlib::GleamStdlibEcosystem;
pub use go_mod::GoModEcosystem;
pub use go_stdlib::GoStdlibEcosystem;
pub use godot_api::GodotApiEcosystem;
pub use groovy_stdlib::GroovyStdlibEcosystem;
pub use hex::HexEcosystem;
pub use jdk_src::JdkSrcEcosystem;
pub use kotlin_stdlib::KotlinStdlibEcosystem;
pub use luarocks::LuarocksEcosystem;
pub use matlab_runtime::MatlabRuntimeEcosystem;
pub use matlab_stdlib::MatlabStdlibEcosystem;
pub use maven::MavenEcosystem;
pub use nimble::NimbleEcosystem;
pub use node_builtins::NodeBuiltinsEcosystem;
pub use npm::NpmEcosystem;
pub use nvim_runtime::NvimRuntimeEcosystem;
pub use nuget::NugetEcosystem;
pub use phoenix_stubs::PhoenixStubsEcosystem;
pub use opam::OpamEcosystem;
pub use php_stubs::PhpStubsEcosystem;
pub use posix_headers::{MsvcHeadersEcosystem, PosixHeadersEcosystem};
pub use powershell_stdlib::PowerShellStdlibEcosystem;
pub use psgallery::PsGalleryEcosystem;
pub use pub_pkg::PubEcosystem;
pub use puppet_forge::PuppetForgeEcosystem;
pub use puppet_stdlib::PuppetStdlibEcosystem;
pub use pypi::PypiEcosystem;
pub use robot_builtin_synthetics::RobotBuiltinEcosystem;
pub use robot_seleniumlibrary_synthetics::RobotSeleniumLibraryEcosystem;
pub use robot_browser_synthetics::RobotBrowserEcosystem;
pub use ember_handlebars_helpers::EmberHandlebarsHelpersEcosystem;
pub use ruby_stdlib::RubyStdlibEcosystem;
pub use rubygems::RubygemsEcosystem;
pub use rust_stdlib::RustStdlibEcosystem;
pub use scala_stdlib::ScalaStdlibEcosystem;
pub use spm::SpmEcosystem;
pub use spring_stubs::SpringStubsEcosystem;
pub use swift_foundation::SwiftFoundationEcosystem;
pub use swift_pm_dsl_stubs::SwiftPmDslStubsEcosystem;
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
    pub const fn new(s: &'static str) -> Self { Self(s) }
    pub fn as_str(&self) -> &'static str { self.0 }
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
    /// `posix-headers` (unix) and `msvc-headers` (Windows).
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
    fn manifest_specs(&self) -> &'static [ManifestSpec] { &[] }

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
    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] { &[] }

    /// `(extension, kind_label)` pairs (extensions include the leading dot)
    /// that mark workspace packages when matched against any file in a
    /// directory. Used by ecosystems where the manifest filename embeds
    /// the project name — `.NET` (`<Proj>.csproj`/`.fsproj`/`.vbproj`),
    /// Haskell (`<pkg>.cabal`), Nim (`<pkg>.nimble`). The detector treats
    /// each matched file as a distinct workspace package.
    ///
    /// Default: empty. Most ecosystems use exact filenames instead.
    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] { &[] }

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
    fn pruned_dir_names(&self) -> &'static [&'static str] { &[] }

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
    fn supports_reachability(&self) -> bool { false }

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
    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        fqn: &str,
    ) -> Vec<WalkedFile> {
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
    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
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
    fn uses_demand_driven_parse(&self) -> bool { false }

    /// Files to eagerly pull before Stage 2's demand loop starts, even
    /// for demand-driven ecosystems. Lets ecosystems whose "entry point"
    /// is a natural, bounded artefact (an npm package's types entry;
    /// a PyPI package's `__init__.py`; a JDK module's `module-info.java`)
    /// surface their public API on pass 1 without paying the cost of a
    /// full walk.
    ///
    /// Default returns empty — suited to ecosystems whose per-dep surface
    /// is large enough that even entry files are wasteful until demand
    /// names them (Go modules, where the "entry" is an entire flat
    /// directory of .go files).
    fn demand_pre_pull(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> Vec<crate::walker::WalkedFile> {
        let _ = dep_roots;
        Vec::new()
    }

    /// Per-file post-processing hook. npm uses this to prefix symbols
    /// with package name so the Tier-1 resolver matches
    /// `import { X } from 'pkg'` → `pkg.X`.
    fn post_process_parsed(&self, dep: &ExternalDepRoot, parsed: &mut ParsedFile) {
        let _ = (dep, parsed);
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Process-lifetime registry of every `Ecosystem` impl. Constructed once;
/// read-only thereafter. The default registry is populated in Phase 2 as
/// `Ecosystem` impls land; for now it's empty and the legacy
/// `ExternalSourceLocator` path carries all traffic.
#[derive(Default)]
pub struct EcosystemRegistry {
    ecosystems: Vec<Arc<dyn Ecosystem>>,
}

impl EcosystemRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, eco: Arc<dyn Ecosystem>) {
        self.ecosystems.push(eco);
    }

    pub fn get(&self, id: EcosystemId) -> Option<&Arc<dyn Ecosystem>> {
        self.ecosystems.iter().find(|e| e.id() == id)
    }

    pub fn all(&self) -> &[Arc<dyn Ecosystem>] { &self.ecosystems }

    /// Every registered ecosystem that declares `lang` in its `languages()`
    /// list. Used at resolve time to filter which ecosystems a given ref
    /// can reach.
    pub fn for_language(&self, lang: &str) -> Vec<&Arc<dyn Ecosystem>> {
        self.ecosystems
            .iter()
            .filter(|e| e.languages().iter().any(|l| *l == lang))
            .collect()
    }
}

/// Back-compat bridge to the legacy `ExternalSourceLocator` trait.
/// Used by the full indexer during the Phase 4 transition so the per-package
/// attribution overrides (TypeScript's hoisted-node_modules walk, Python's
/// venv ancestor probe) keep firing without adding a
/// `locate_roots_for_package` method to the new `Ecosystem` trait. Once all
/// call sites consume the new trait directly this helper goes away.
pub fn default_locator(
    id: EcosystemId,
) -> Option<Arc<dyn crate::ecosystem::externals::ExternalSourceLocator>> {
    match id.as_str() {
        "maven" => Some(Arc::new(MavenEcosystem)),
        "npm" => Some(Arc::new(NpmEcosystem)),
        "pypi" => Some(Arc::new(PypiEcosystem)),
        "cargo" => Some(Arc::new(CargoEcosystem)),
        "hex" => Some(Arc::new(HexEcosystem)),
        "nuget" => Some(Arc::new(NugetEcosystem)),
        "spm" => Some(Arc::new(SpmEcosystem)),
        "go-mod" => Some(Arc::new(GoModEcosystem)),
        "rubygems" => Some(Arc::new(RubygemsEcosystem)),
        "composer" => Some(Arc::new(ComposerEcosystem)),
        "cran" => Some(Arc::new(CranEcosystem)),
        "pub" => Some(Arc::new(PubEcosystem)),
        "cabal" => Some(Arc::new(CabalEcosystem)),
        "nimble" => Some(Arc::new(NimbleEcosystem)),
        "cpan" => Some(Arc::new(CpanEcosystem)),
        "opam" => Some(Arc::new(OpamEcosystem)),
        "luarocks" => Some(Arc::new(LuarocksEcosystem)),
        "zig-pkg" => Some(Arc::new(ZigPkgEcosystem)),
        "godot-api" => Some(Arc::new(GodotApiEcosystem)),
        "android-sdk" => Some(Arc::new(AndroidSdkEcosystem)),
        "kotlin-stdlib" => Some(Arc::new(KotlinStdlibEcosystem)),
        "rust-stdlib" => Some(Arc::new(RustStdlibEcosystem)),
        "go-stdlib" => Some(Arc::new(GoStdlibEcosystem)),
        "cpython-stdlib" => Some(Arc::new(CpythonStdlibEcosystem)),
        "jdk-src" => Some(Arc::new(JdkSrcEcosystem)),
        "ts-lib-dom" => Some(Arc::new(TsLibDomEcosystem)),
        "ruby-stdlib" => Some(Arc::new(RubyStdlibEcosystem)),
        "posix-headers" => Some(Arc::new(PosixHeadersEcosystem)),
        "msvc-headers" => Some(Arc::new(MsvcHeadersEcosystem)),
        "dotnet-stdlib" => Some(Arc::new(DotnetStdlibEcosystem)),
        "php-stubs" => Some(Arc::new(PhpStubsEcosystem)),
        "phoenix-stubs" => Some(Arc::new(PhoenixStubsEcosystem)),
        "scala-stdlib" => Some(Arc::new(ScalaStdlibEcosystem)),
        "groovy-stdlib" => Some(Arc::new(GroovyStdlibEcosystem)),
        "clojure-core" => Some(Arc::new(ClojureCoreEcosystem)),
        "erlang-otp" => Some(Arc::new(ErlangOtpEcosystem)),
        "elixir-stdlib" => Some(Arc::new(ElixirStdlibEcosystem)),
        "spring-stubs" => Some(Arc::new(SpringStubsEcosystem)),
        "blazor-runtime" => Some(Arc::new(BlazorRuntimeEcosystem)),
        "jinja-ansible-runtime" => Some(jinja_ansible_runtime::shared_locator()),
        "bicep-runtime" => Some(bicep_runtime::shared_locator()),
        "swift-foundation" => Some(Arc::new(SwiftFoundationEcosystem)),
        "swift-pm-dsl-stubs" => Some(Arc::new(SwiftPmDslStubsEcosystem)),
        "vba-typelibs" => Some(Arc::new(VbaTypelibsEcosystem)),
        "puppet-forge" => Some(puppet_forge::shared_locator()),
        "puppet-stdlib" => Some(puppet_stdlib::shared_locator()),
        "dart-sdk" => Some(Arc::new(DartSdkEcosystem)),
        "flutter-sdk" => Some(Arc::new(FlutterSdkEcosystem)),
        "psgallery" => Some(Arc::new(PsGalleryEcosystem)),
        "powershell-stdlib" => Some(Arc::new(PowerShellStdlibEcosystem)),
        "tf-registry" => Some(Arc::new(TfRegistryEcosystem)),
        "matlab-runtime" => Some(Arc::new(MatlabRuntimeEcosystem)),
        "matlab-stdlib" => Some(Arc::new(MatlabStdlibEcosystem)),
        "nvim-runtime" => Some(Arc::new(NvimRuntimeEcosystem)),
        "gleam-stdlib" => Some(Arc::new(GleamStdlibEcosystem)),
        "node-builtins" => Some(Arc::new(NodeBuiltinsEcosystem)),
        "bazel-central-registry" => Some(Arc::new(BazelCentralRegistryEcosystem)),
        "zig-std" => Some(Arc::new(ZigStdEcosystem)),
        "sdl-synthetics" => Some(Arc::new(SdlSyntheticsEcosystem)),
        "bash-completion-synthetics" => Some(Arc::new(BashCompletionSyntheticsEcosystem)),
        "robot-builtin" => Some(robot_builtin_synthetics::shared_locator()),
        "robot-seleniumlibrary" => Some(Arc::new(RobotSeleniumLibraryEcosystem)),
        "robot-browser" => Some(Arc::new(RobotBrowserEcosystem)),
        "ember-handlebars-helpers" => Some(ember_handlebars_helpers::shared_locator()),
        _ => None,
    }
}

/// Default registry. Populated with every shipped ecosystem: the 18 package
/// ecosystems migrated in Phase 2+3. Stdlib ecosystems (Phase 5) attach here
/// as they land.
///
/// `BEARWISDOM_DISABLE_ECOSYSTEMS` (env var) takes a comma-separated list of
/// ecosystem ids and skips their registration. Used for A/B testing whether
/// a synthetic still adds resolution after the underlying real-source path
/// has been fixed — index once with the synthetic, once without, compare
/// quality. Intended for local experimentation, not production gating.
pub fn default_registry() -> &'static EcosystemRegistry {
    use std::sync::OnceLock;
    static REG: OnceLock<EcosystemRegistry> = OnceLock::new();
    REG.get_or_init(|| {
        let disabled: std::collections::HashSet<String> =
            std::env::var("BEARWISDOM_DISABLE_ECOSYSTEMS")
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

        let mut reg = EcosystemRegistry::new();

        // Skip-aware register helper. Synthetics whose ids appear in the
        // disable set are silently dropped.
        macro_rules! reg_eco {
            ($eco:expr) => {{
                let arc = Arc::new($eco);
                let eco_id = arc.id().as_str();
                if !disabled.contains(eco_id) {
                    reg.register(arc);
                }
            }};
        }

        reg_eco!(MavenEcosystem);
        reg_eco!(NpmEcosystem);
        reg_eco!(HexEcosystem);
        reg_eco!(CargoEcosystem);
        reg_eco!(PypiEcosystem);
        reg_eco!(GoModEcosystem);
        reg_eco!(SpmEcosystem);
        reg_eco!(NugetEcosystem);
        reg_eco!(PubEcosystem);
        reg_eco!(RubygemsEcosystem);
        reg_eco!(CranEcosystem);
        reg_eco!(ComposerEcosystem);
        reg_eco!(CabalEcosystem);
        reg_eco!(NimbleEcosystem);
        reg_eco!(CpanEcosystem);
        reg_eco!(OpamEcosystem);
        reg_eco!(LuarocksEcosystem);
        reg_eco!(ZigPkgEcosystem);
        reg_eco!(GodotApiEcosystem);
        reg_eco!(PsGalleryEcosystem);
        reg_eco!(TfRegistryEcosystem);
        reg_eco!(BazelCentralRegistryEcosystem);
        // Stdlib ecosystems must register AFTER their base package ecosystem
        // (Maven) for TransitiveOn activation to resolve in a single pass.
        reg_eco!(KotlinStdlibEcosystem);
        reg_eco!(AndroidSdkEcosystem);
        reg_eco!(RustStdlibEcosystem);
        reg_eco!(GoStdlibEcosystem);
        reg_eco!(CpythonStdlibEcosystem);
        reg_eco!(JdkSrcEcosystem);
        reg_eco!(TsLibDomEcosystem);
        reg_eco!(RubyStdlibEcosystem);
        reg_eco!(PosixHeadersEcosystem);
        reg_eco!(MsvcHeadersEcosystem);
        reg_eco!(DotnetStdlibEcosystem);
        reg_eco!(PhpStubsEcosystem);
        reg_eco!(PhoenixStubsEcosystem);
        reg_eco!(ScalaStdlibEcosystem);
        reg_eco!(GroovyStdlibEcosystem);
        reg_eco!(ClojureCoreEcosystem);
        reg_eco!(ErlangOtpEcosystem);
        reg_eco!(ElixirStdlibEcosystem);
        reg_eco!(SpringStubsEcosystem);
        reg_eco!(BlazorRuntimeEcosystem);
        reg_eco!(JinjaAnsibleRuntimeEcosystem);
        reg_eco!(BicepRuntimeEcosystem);
        reg_eco!(SwiftFoundationEcosystem);
        reg_eco!(SwiftPmDslStubsEcosystem);
        reg_eco!(VbaTypelibsEcosystem);
        reg_eco!(PuppetForgeEcosystem);
        reg_eco!(PuppetStdlibEcosystem);
        reg_eco!(DartSdkEcosystem);
        reg_eco!(FlutterSdkEcosystem);
        reg_eco!(PowerShellStdlibEcosystem);
        reg_eco!(MatlabRuntimeEcosystem);
        reg_eco!(MatlabStdlibEcosystem);
        reg_eco!(NvimRuntimeEcosystem);
        reg_eco!(GleamStdlibEcosystem);
        reg_eco!(NodeBuiltinsEcosystem);
        reg_eco!(ZigStdEcosystem);
        reg_eco!(SdlSyntheticsEcosystem);
        reg_eco!(BashCompletionSyntheticsEcosystem);
        reg_eco!(RobotBuiltinEcosystem);
        reg_eco!(RobotSeleniumLibraryEcosystem);
        reg_eco!(RobotBrowserEcosystem);
        reg_eco!(EmberHandlebarsHelpersEcosystem);
        reg
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_id_roundtrip() {
        let id = EcosystemId::new("maven");
        assert_eq!(id.as_str(), "maven");
        assert_eq!(format!("{id}"), "maven");
    }

    #[test]
    fn default_registry_contains_package_ecosystems() {
        // Phase 4: all 18 package ecosystems are registered. Stdlib ecosystems
        // land in Phase 5 and will extend this count.
        let ids: Vec<&str> = default_registry()
            .all()
            .iter()
            .map(|e| e.id().as_str())
            .collect();
        for expected in [
            "maven", "npm", "hex", "cargo", "pypi", "go-mod", "spm", "nuget",
            "pub", "rubygems", "cran", "composer", "cabal", "nimble", "cpan",
            "opam", "luarocks", "zig-pkg",
        ] {
            assert!(
                ids.contains(&expected),
                "ecosystem {expected} missing from default_registry; got {ids:?}",
            );
        }
    }

    #[test]
    fn registry_lookup_by_id_and_language() {
        struct DummyEcosystem;
        impl Ecosystem for DummyEcosystem {
            fn id(&self) -> EcosystemId { EcosystemId::new("dummy") }
            fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
            fn languages(&self) -> &'static [&'static str] { &["fake-lang"] }
            fn activation(&self) -> EcosystemActivation { EcosystemActivation::Never }
            fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
                Vec::new()
            }
        }

        let mut reg = EcosystemRegistry::new();
        reg.register(Arc::new(DummyEcosystem));

        assert!(reg.get(EcosystemId::new("dummy")).is_some());
        assert!(reg.get(EcosystemId::new("other")).is_none());
        assert_eq!(reg.for_language("fake-lang").len(), 1);
        assert_eq!(reg.for_language("real-lang").len(), 0);
    }
}
