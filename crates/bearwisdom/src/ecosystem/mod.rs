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
pub mod manifest;
pub mod symbol_index;

pub use symbol_index::SymbolLocationIndex;

pub mod android_sdk;
pub mod cabal;
pub mod cargo;
pub mod clojure_core;
pub mod composer;
pub mod cpan;
pub mod cpython_stdlib;
pub mod cran;
pub mod dotnet_stdlib;
pub mod elixir_stdlib;
pub mod erlang_otp;
pub mod go_mod;
pub mod go_platform;
pub mod go_stdlib;
pub mod godot_api;
pub mod groovy_stdlib;
pub mod hex;
pub mod jdk_src;
pub mod kotlin_stdlib;
pub mod luarocks;
pub mod maven;
pub mod nimble;
pub mod npm;
pub mod nuget;
pub mod opam;
pub mod php_stubs;
pub mod posix_headers;
pub mod pub_pkg;
pub mod pypi;
pub mod ruby_stdlib;
pub mod rubygems;
pub mod rust_stdlib;
pub mod scala_stdlib;
pub mod spm;
pub mod swift_foundation;
pub mod ts_lib_dom;
pub mod vba_typelibs;
pub mod zig_pkg;
pub use android_sdk::AndroidSdkEcosystem;
pub use cabal::CabalEcosystem;
pub use cargo::CargoEcosystem;
pub use clojure_core::ClojureCoreEcosystem;
pub use composer::ComposerEcosystem;
pub use cpan::CpanEcosystem;
pub use cpython_stdlib::CpythonStdlibEcosystem;
pub use cran::CranEcosystem;
pub use dotnet_stdlib::DotnetStdlibEcosystem;
pub use elixir_stdlib::ElixirStdlibEcosystem;
pub use erlang_otp::ErlangOtpEcosystem;
pub use go_mod::GoModEcosystem;
pub use go_stdlib::GoStdlibEcosystem;
pub use godot_api::GodotApiEcosystem;
pub use groovy_stdlib::GroovyStdlibEcosystem;
pub use hex::HexEcosystem;
pub use jdk_src::JdkSrcEcosystem;
pub use kotlin_stdlib::KotlinStdlibEcosystem;
pub use luarocks::LuarocksEcosystem;
pub use maven::MavenEcosystem;
pub use nimble::NimbleEcosystem;
pub use npm::NpmEcosystem;
pub use nuget::NugetEcosystem;
pub use opam::OpamEcosystem;
pub use php_stubs::PhpStubsEcosystem;
pub use posix_headers::{MsvcHeadersEcosystem, PosixHeadersEcosystem};
pub use pub_pkg::PubEcosystem;
pub use pypi::PypiEcosystem;
pub use ruby_stdlib::RubyStdlibEcosystem;
pub use rubygems::RubygemsEcosystem;
pub use rust_stdlib::RustStdlibEcosystem;
pub use scala_stdlib::ScalaStdlibEcosystem;
pub use spm::SpmEcosystem;
pub use swift_foundation::SwiftFoundationEcosystem;
pub use ts_lib_dom::TsLibDomEcosystem;
pub use vba_typelibs::VbaTypelibsEcosystem;
pub use zig_pkg::ZigPkgEcosystem;

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
        "scala-stdlib" => Some(Arc::new(ScalaStdlibEcosystem)),
        "groovy-stdlib" => Some(Arc::new(GroovyStdlibEcosystem)),
        "clojure-core" => Some(Arc::new(ClojureCoreEcosystem)),
        "erlang-otp" => Some(Arc::new(ErlangOtpEcosystem)),
        "elixir-stdlib" => Some(Arc::new(ElixirStdlibEcosystem)),
        "swift-foundation" => Some(Arc::new(SwiftFoundationEcosystem)),
        "vba-typelibs" => Some(Arc::new(VbaTypelibsEcosystem)),
        _ => None,
    }
}

/// Default registry. Populated with every shipped ecosystem: the 18 package
/// ecosystems migrated in Phase 2+3. Stdlib ecosystems (Phase 5) attach here
/// as they land.
pub fn default_registry() -> &'static EcosystemRegistry {
    use std::sync::OnceLock;
    static REG: OnceLock<EcosystemRegistry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut reg = EcosystemRegistry::new();
        reg.register(Arc::new(MavenEcosystem));
        reg.register(Arc::new(NpmEcosystem));
        reg.register(Arc::new(HexEcosystem));
        reg.register(Arc::new(CargoEcosystem));
        reg.register(Arc::new(PypiEcosystem));
        reg.register(Arc::new(GoModEcosystem));
        reg.register(Arc::new(SpmEcosystem));
        reg.register(Arc::new(NugetEcosystem));
        reg.register(Arc::new(PubEcosystem));
        reg.register(Arc::new(RubygemsEcosystem));
        reg.register(Arc::new(CranEcosystem));
        reg.register(Arc::new(ComposerEcosystem));
        reg.register(Arc::new(CabalEcosystem));
        reg.register(Arc::new(NimbleEcosystem));
        reg.register(Arc::new(CpanEcosystem));
        reg.register(Arc::new(OpamEcosystem));
        reg.register(Arc::new(LuarocksEcosystem));
        reg.register(Arc::new(ZigPkgEcosystem));
        reg.register(Arc::new(GodotApiEcosystem));
        // Stdlib ecosystems must register AFTER their base package ecosystem
        // (Maven) for TransitiveOn activation to resolve in a single pass.
        reg.register(Arc::new(KotlinStdlibEcosystem));
        reg.register(Arc::new(AndroidSdkEcosystem));
        reg.register(Arc::new(RustStdlibEcosystem));
        reg.register(Arc::new(GoStdlibEcosystem));
        reg.register(Arc::new(CpythonStdlibEcosystem));
        reg.register(Arc::new(JdkSrcEcosystem));
        reg.register(Arc::new(TsLibDomEcosystem));
        reg.register(Arc::new(RubyStdlibEcosystem));
        reg.register(Arc::new(PosixHeadersEcosystem));
        reg.register(Arc::new(MsvcHeadersEcosystem));
        reg.register(Arc::new(DotnetStdlibEcosystem));
        reg.register(Arc::new(PhpStubsEcosystem));
        reg.register(Arc::new(ScalaStdlibEcosystem));
        reg.register(Arc::new(GroovyStdlibEcosystem));
        reg.register(Arc::new(ClojureCoreEcosystem));
        reg.register(Arc::new(ErlangOtpEcosystem));
        reg.register(Arc::new(ElixirStdlibEcosystem));
        reg.register(Arc::new(SwiftFoundationEcosystem));
        reg.register(Arc::new(VbaTypelibsEcosystem));
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
