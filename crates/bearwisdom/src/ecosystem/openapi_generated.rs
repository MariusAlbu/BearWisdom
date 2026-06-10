// =============================================================================
// ecosystem/openapi_generated.rs — OpenAPI / Swagger-codegen output discovery
//
// `openapi.yaml`/`openapi.json`/`swagger.yaml` define HTTP APIs;
// openapi-generator, swagger-codegen, tsoa, openapi-typescript, and a
// dozen language-specific generators emit client/server stubs. Output
// directories vary per generator + project config.
//
// **Discovery:** when any OpenAPI spec file is present, probe a set of
// common output dirs relative to the project root. Walked with the
// per-file extractor (TS/Java/Go/Python all extract their own generated
// API clients).
//
// Same probe-based pattern as `protoc_generated.rs` — pragmatic 80/20
// without parsing every generator's config schema.
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("openapi-generated");
const ECOSYSTEM_TAG: &str = "openapi-generated";
const LANGUAGES: &[&str] = &["typescript", "javascript", "java", "python", "go", "csharp"];

/// Conventional output dirs across openapi-generator, swagger-codegen,
/// tsoa, openapi-typescript, etc.
const OPENAPI_OUTPUT_DIRS: &[&str] = &[
    "generated/api",
    "generated/openapi",
    "src/generated",
    "src/generated/api",
    "gen/api",
    "openapi-generated",
    "api/generated",
    "client/generated",
    "frontend/generated",
    "build/generated/openapi",
    "target/generated-sources/openapi",
];

pub struct OpenApiGeneratedEcosystem;

impl Ecosystem for OpenApiGeneratedEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/openapi.yaml",
                field_path: "",
                value: "",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/openapi.json",
                field_path: "",
                value: "",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/swagger.yaml",
                field_path: "",
                value: "",
            },
            EcosystemActivation::ManifestFieldContains {
                manifest_glob: "**/swagger.json",
                field_path: "",
                value: "",
            },
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_openapi_output_dirs(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_generated_tree(&dep.root)
    }
}

impl ExternalSourceLocator for OpenApiGeneratedEcosystem {
    fn ecosystem(&self) -> &'static str {
        ECOSYSTEM_TAG
    }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_openapi_output_dirs(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_generated_tree(&dep.root)
    }
}

fn discover_openapi_output_dirs(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots = Vec::new();
    for relative in OPENAPI_OUTPUT_DIRS {
        let abs = project_root.join(relative);
        if abs.is_dir() && roots.iter().all(|r: &ExternalDepRoot| r.root != abs) {
            roots.push(ExternalDepRoot {
                module_path: format!("openapi:{relative}"),
                version: String::new(),
                root: abs,
                ecosystem: ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }
    roots
}

fn walk_generated_tree(dir: &Path) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(dir, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lang = if name.ends_with(".d.ts") || name.ends_with(".ts") {
                "typescript"
            } else if name.ends_with(".js") {
                "javascript"
            } else if name.ends_with(".java") {
                "java"
            } else if name.ends_with(".py") || name.ends_with(".pyi") {
                "python"
            } else if name.ends_with(".go") {
                "go"
            } else if name.ends_with(".cs") {
                "csharp"
            } else {
                continue;
            };
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:openapi:{display}"),
                absolute_path: path,
                language: lang,
            });
        }
    }
}
