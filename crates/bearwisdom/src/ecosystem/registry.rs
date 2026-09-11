// =============================================================================
// ecosystem/registry.rs — construction and lookup of the ecosystem set.
//
// `EcosystemRegistry` holds every `Ecosystem` impl; `default_registry` is the
// process-wide instance the indexer consults. `default_locator` bridges an
// `EcosystemId` to the legacy `ExternalSourceLocator` trait for call sites
// that still consume it.
// =============================================================================

use super::*;
use std::path::Path;

/// Process-lifetime registry of every `Ecosystem` impl. Constructed once;
/// read-only thereafter.
#[derive(Default)]
pub struct EcosystemRegistry {
    ecosystems: Vec<Arc<dyn Ecosystem>>,
}

impl EcosystemRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, eco: Arc<dyn Ecosystem>) {
        self.ecosystems.push(eco);
    }

    pub fn get(&self, id: EcosystemId) -> Option<&Arc<dyn Ecosystem>> {
        self.ecosystems.iter().find(|e| e.id() == id)
    }

    pub fn all(&self) -> &[Arc<dyn Ecosystem>] {
        &self.ecosystems
    }

    /// Every registered ecosystem that declares `lang` in its `languages()`
    /// list. Used at resolve time to filter which ecosystems a given ref
    /// can reach.
    pub fn for_language(&self, lang: &str) -> Vec<&Arc<dyn Ecosystem>> {
        self.ecosystems
            .iter()
            .filter(|e| e.languages().iter().any(|l| *l == lang))
            .collect()
    }

    /// Source-name aliases contributed by the ecosystem that owns a workspace
    /// package-kind label. The context remains agnostic of spelling rules.
    pub fn workspace_package_name_aliases(&self, kind: &str, declared_name: &str) -> Vec<String> {
        self.ecosystems
            .iter()
            .filter(|eco| {
                eco.workspace_package_files()
                    .iter()
                    .any(|(_, label)| *label == kind)
            })
            .flat_map(|eco| eco.workspace_package_name_aliases(declared_name))
            .collect()
    }

    /// Persisted dependency ecosystem for a normalized manifest kind.
    /// Selection is registry-owned, so discovery never matches manifest enum
    /// variants itself. Registration order is intentional first-wins policy.
    pub fn package_dependency_ecosystem(
        &self,
        kind: crate::ecosystem::manifest::ManifestKind,
    ) -> Option<&'static str> {
        self.ecosystems
            .iter()
            .find(|eco| eco.manifest_kinds().contains(&kind))
            .and_then(|eco| eco.package_dependency_ecosystem())
    }

    /// Convert a profile scanner's monorepo label into an adapter-owned
    /// workspace package kind.
    pub fn workspace_kind_for_monorepo_kind(&self, scanner_kind: &str) -> Option<&'static str> {
        self.ecosystems.iter().find_map(|eco| {
            eco.workspace_monorepo_kinds()
                .iter()
                .find_map(|(source, kind)| (*source == scanner_kind).then_some(*kind))
        })
    }

    /// Read package metadata through the ecosystem that owns `kind`.
    /// Unsupported kinds deliberately preserve the old neutral defaults.
    pub fn workspace_package_metadata(&self, dir: &Path, kind: &str) -> WorkspacePackageMetadata {
        self.ecosystems
            .iter()
            .filter(|eco| owns_workspace_kind(eco.as_ref(), kind))
            .find_map(|eco| eco.workspace_package_metadata(dir))
            .unwrap_or_else(|| WorkspacePackageMetadata {
                declared_name: None,
                is_publishable: true,
            })
    }

    /// Ask the owning adapter whether a workspace root is also a package.
    pub fn workspace_root_is_package(&self, root: &Path, kind: &str) -> bool {
        self.ecosystems
            .iter()
            .filter(|eco| owns_workspace_kind(eco.as_ref(), kind))
            .any(|eco| eco.workspace_root_is_package(root))
    }

    /// Find an owned package manifest in `dir`, returning its filename. Exact
    /// filename markers win over extension markers, matching the recursive
    /// scanner's precedence.
    pub fn workspace_manifest_filename(&self, dir: &Path, kind: &str) -> Option<String> {
        for eco in &self.ecosystems {
            for (filename, label) in eco.workspace_package_files() {
                if *label == kind && dir.join(filename).is_file() {
                    return Some((*filename).to_owned());
                }
            }
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            if self.ecosystems.iter().any(|eco| {
                eco.workspace_package_extensions()
                    .iter()
                    .any(|(extension, label)| *label == kind && filename.ends_with(extension))
            }) {
                return Some(filename);
            }
        }
        None
    }

    /// Whether a changed path names a workspace manifest owned by any
    /// registered ecosystem. Exact filenames and adapter-declared manifest
    /// extensions are both recognized; callers do not carry a package-manager
    /// inventory of their own.
    pub fn is_workspace_manifest_path(&self, path: &Path) -> bool {
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        self.ecosystems.iter().any(|eco| {
            eco.workspace_package_files()
                .iter()
                .any(|(candidate, _)| *candidate == filename)
                || eco
                    .workspace_package_extensions()
                    .iter()
                    .any(|(extension, _)| filename.ends_with(extension))
        })
    }
}

fn owns_workspace_kind(eco: &dyn Ecosystem, kind: &str) -> bool {
    eco.workspace_package_files()
        .iter()
        .any(|(_, label)| *label == kind)
        || eco
            .workspace_package_extensions()
            .iter()
            .any(|(_, label)| *label == kind)
}

/// Bridge to the legacy `ExternalSourceLocator` trait, keeping the
/// per-package attribution overrides (TypeScript's hoisted-node_modules walk,
/// Python's venv ancestor probe) reachable without adding a
/// `locate_roots_for_package` method to the `Ecosystem` trait.
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
        "alire" => Some(Arc::new(AlireEcosystem)),
        "nimble" => Some(Arc::new(NimbleEcosystem)),
        "cpan" => Some(Arc::new(CpanEcosystem)),
        "opam" => Some(Arc::new(OpamEcosystem)),
        "luarocks" => Some(Arc::new(LuarocksEcosystem)),
        "zig-pkg" => Some(Arc::new(ZigPkgEcosystem)),
        "godot-api" => Some(Arc::new(GodotApiEcosystem)),
        "android-sdk" => Some(Arc::new(AndroidSdkEcosystem)),
        "freepascal-runtime" => Some(Arc::new(FreePascalRuntimeEcosystem)),
        "kotlin-stdlib" => Some(Arc::new(KotlinStdlibEcosystem)),
        "rust-stdlib" => Some(Arc::new(RustStdlibEcosystem)),
        "go-stdlib" => Some(Arc::new(GoStdlibEcosystem)),
        "cpython-stdlib" => Some(Arc::new(CpythonStdlibEcosystem)),
        "jdk-src" => Some(Arc::new(JdkSrcEcosystem)),
        "ts-lib-dom" => Some(Arc::new(TsLibDomEcosystem)),
        "ruby-stdlib" => Some(Arc::new(RubyStdlibEcosystem)),
        "r-stdlib" => Some(Arc::new(RStdlibEcosystem)),
        "lua-stdlib" => Some(Arc::new(LuaStdlibEcosystem)),
        "ocaml-stdlib" => Some(Arc::new(OcamlStdlibEcosystem)),
        "nim-stdlib" => Some(Arc::new(NimStdlibEcosystem)),
        "posix-headers" => Some(Arc::new(PosixHeadersEcosystem)),
        "msvc-sdk" => Some(Arc::new(MsvcSdkEcosystem)),
        "qt-runtime" => Some(Arc::new(QtRuntimeEcosystem)),
        "compile-commands" => Some(Arc::new(CompileCommandsEcosystem)),
        "vcpkg-headers" => Some(Arc::new(VcpkgHeadersEcosystem)),
        "dotnet-stdlib" => Some(Arc::new(DotnetStdlibEcosystem)),
        "php-stubs" => Some(Arc::new(PhpStubsEcosystem)),
        "scala-stdlib" => Some(Arc::new(ScalaStdlibEcosystem)),
        "groovy-stdlib" => Some(Arc::new(GroovyStdlibEcosystem)),
        "clojure-core" => Some(Arc::new(ClojureCoreEcosystem)),
        "erlang-otp" => Some(Arc::new(ErlangOtpEcosystem)),
        "elixir-stdlib" => Some(Arc::new(ElixirStdlibEcosystem)),
        "jinja-ansible-runtime" => Some(jinja_ansible_runtime::shared_locator()),
        "bicep-runtime" => Some(bicep_runtime::shared_locator()),
        "prolog-runtime" => Some(prolog_runtime::shared_locator()),
        "hexo-runtime" => Some(hexo_runtime::shared_locator()),
        "nuxt-runtime" => Some(nuxt_runtime::shared_locator()),
        "cargo-build-scripts" => Some(cargo_build_scripts::shared_locator()),
        "swift-foundation" => Some(Arc::new(SwiftFoundationEcosystem)),
        "vba-typelibs" => Some(Arc::new(VbaTypelibsEcosystem)),
        "puppet-forge" => Some(puppet_forge::shared_locator()),
        "dart-sdk" => Some(Arc::new(DartSdkEcosystem)),
        "flutter-sdk" => Some(Arc::new(FlutterSdkEcosystem)),
        "psgallery" => Some(Arc::new(PsGalleryEcosystem)),
        "powershell-stdlib" => Some(Arc::new(PowerShellStdlibEcosystem)),
        "tf-registry" => Some(Arc::new(TfRegistryEcosystem)),
        "matlab-runtime" => Some(Arc::new(MatlabRuntimeEcosystem)),
        "nvim-runtime" => Some(Arc::new(NvimRuntimeEcosystem)),
        "gleam-stdlib" => Some(Arc::new(GleamStdlibEcosystem)),
        "gnat-stdlib" => Some(Arc::new(GnatStdlibEcosystem)),
        "gnat-project" => Some(Arc::new(GnatProjectEcosystem)),
        "bazel-central-registry" => Some(Arc::new(BazelCentralRegistryEcosystem)),
        "zig-std" => Some(Arc::new(ZigStdEcosystem)),
        "sdl-synthetics" => Some(Arc::new(SdlSyntheticsEcosystem)),
        "maven-classes" => Some(Arc::new(MavenClassesEcosystem)),
        _ => None,
    }
}

/// Default registry, populated with every shipped ecosystem.
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
        reg_eco!(AlireEcosystem);
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
        reg_eco!(FreePascalRuntimeEcosystem);
        reg_eco!(AndroidSdkEcosystem);
        reg_eco!(RustStdlibEcosystem);
        reg_eco!(GoStdlibEcosystem);
        reg_eco!(CpythonStdlibEcosystem);
        reg_eco!(JdkSrcEcosystem);
        reg_eco!(TsLibDomEcosystem);
        reg_eco!(RubyStdlibEcosystem);
        reg_eco!(RStdlibEcosystem);
        reg_eco!(LuaStdlibEcosystem);
        reg_eco!(OcamlStdlibEcosystem);
        reg_eco!(NimStdlibEcosystem);
        reg_eco!(PosixHeadersEcosystem);
        reg_eco!(MsvcSdkEcosystem);
        reg_eco!(QtRuntimeEcosystem);
        reg_eco!(CompileCommandsEcosystem);
        reg_eco!(VcpkgHeadersEcosystem);
        reg_eco!(DotnetStdlibEcosystem);
        reg_eco!(PhpStubsEcosystem);
        reg_eco!(PerlStdlibEcosystem);
        reg_eco!(ScalaStdlibEcosystem);
        reg_eco!(GroovyStdlibEcosystem);
        reg_eco!(ClojureCoreEcosystem);
        reg_eco!(ErlangOtpEcosystem);
        reg_eco!(ElixirStdlibEcosystem);
        reg_eco!(JinjaAnsibleRuntimeEcosystem);
        reg_eco!(BicepRuntimeEcosystem);
        reg_eco!(PrologRuntimeEcosystem);
        reg_eco!(HexoRuntimeEcosystem);
        reg_eco!(NuxtRuntimeEcosystem);
        reg_eco!(CargoBuildScriptsEcosystem);
        reg_eco!(SwiftFoundationEcosystem);
        reg_eco!(SwiftPmDslEcosystem);
        reg_eco!(VbaTypelibsEcosystem);
        reg_eco!(PuppetForgeEcosystem);
        reg_eco!(PuppetStdlibEcosystem);
        reg_eco!(DartSdkEcosystem);
        reg_eco!(FlutterSdkEcosystem);
        reg_eco!(PowerShellStdlibEcosystem);
        reg_eco!(MatlabRuntimeEcosystem);
        reg_eco!(NvimRuntimeEcosystem);
        reg_eco!(GleamStdlibEcosystem);
        reg_eco!(GnatStdlibEcosystem);
        reg_eco!(GnatProjectEcosystem);
        reg_eco!(ZigStdEcosystem);
        reg_eco!(SdlSyntheticsEcosystem);
        reg_eco!(PrismaClientEcosystem);
        reg_eco!(ProtocGeneratedEcosystem);
        reg_eco!(OpenApiGeneratedEcosystem);
        reg_eco!(MavenClassesEcosystem);
        reg
    })
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
