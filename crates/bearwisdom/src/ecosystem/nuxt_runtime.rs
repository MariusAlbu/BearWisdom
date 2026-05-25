// =============================================================================
// ecosystem/nuxt_runtime.rs — Nuxt auto-imports / auto-components discovery
//
// Nuxt 3 projects use auto-imports: `computed`, `ref`, `useRoute`,
// `definePageMeta`, `NuxtLink`, `useFetch`, and the project's own
// `app/components/**` are all callable in user code WITHOUT an explicit
// `import` statement. At build time Nuxt's CLI generates
//   * `.nuxt/imports.d.ts`     — composables + framework helpers
//   * `.nuxt/components.d.ts`  — auto-registered user components
// containing real TypeScript export declarations. These are normally
// pruned by the corpus walker as build output (`.nuxt/` is in every
// project's `.gitignore`).
//
// The walker re-includes those two files for Nuxt projects so the
// TypeScript extractor indexes them, and refs in `.vue` / `.ts` files
// resolve via the regular `default_*` strategies. No synthetic name list
// — the symbols come from the project's own generated declarations.
//
// **Activation:** `package.json` declares `nuxt` as a dependency or
// devDependency. Projects without `nuxt` skip this ecosystem entirely.
//
// **No-build case:** if the project hasn't run `nuxt prepare` / a Nuxt
// build, the `.nuxt/` directory is absent. The walker emits a
// diagnostic and returns empty — the user sees a clear signal that the
// build artefact is missing, not silently degraded resolution.
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("nuxt-runtime");
const ECOSYSTEM_TAG: &str = "nuxt-runtime";
const LANGUAGES: &[&str] = &["typescript", "javascript", "vue"];

const IMPORTS_DTS: &str = ".nuxt/imports.d.ts";
const COMPONENTS_DTS: &str = ".nuxt/components.d.ts";

pub struct NuxtRuntimeEcosystem;

impl Ecosystem for NuxtRuntimeEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/package.json",
                field_path: "dependencies",
                value: "nuxt",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/package.json",
                field_path: "devDependencies",
                value: "nuxt",
            },
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_nuxt_root(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        nuxt_auto_import_files(&dep.root)
    }
}

impl ExternalSourceLocator for NuxtRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str {
        ECOSYSTEM_TAG
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_nuxt_root(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        nuxt_auto_import_files(&dep.root)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NuxtRuntimeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NuxtRuntimeEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_nuxt_root(project_root: &Path) -> Vec<ExternalDepRoot> {
    let imports = project_root.join(IMPORTS_DTS);
    let components = project_root.join(COMPONENTS_DTS);
    if !imports.is_file() && !components.is_file() {
        tracing::warn!(
            "nuxt-runtime: project declares `nuxt` but `.nuxt/imports.d.ts` \
             and `.nuxt/components.d.ts` are absent at {} — run `nuxt prepare`",
            project_root.display()
        );
        return Vec::new();
    }
    tracing::info!(
        "nuxt-runtime: discovered .nuxt/ in {} (imports={} components={})",
        project_root.display(),
        imports.is_file(),
        components.is_file()
    );
    vec![ExternalDepRoot {
        module_path: "nuxt".to_string(),
        version: String::from("local"),
        root: project_root.to_path_buf(),
        ecosystem: ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn nuxt_auto_import_files(project_root: &Path) -> Vec<WalkedFile> {
    let mut out = Vec::with_capacity(2);
    for rel in [IMPORTS_DTS, COMPONENTS_DTS] {
        let abs = project_root.join(rel);
        if abs.is_file() {
            out.push(WalkedFile {
                relative_path: rel.to_string(),
                absolute_path: abs,
                language: "typescript",
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "nuxt_runtime_tests.rs"]
mod tests;
